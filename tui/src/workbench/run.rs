//! The workbench's driver: terminal, event loop, and the keys that are not the
//! surface's own.
//!
//! `future tui` and `future-workbench` both end up here. The loop is async (the
//! agent, the terminal input and the animation tick are all wake-ups), and it
//! draws a frame *before* waiting: the screen is never stale while it waits for
//! something to happen.
//!
//! The split of responsibility is the same one the whole module tree follows:
//! [`Workbench`] decides what a key or a click *means* for the surface and
//! returns [`WbAction`]s; this loop decides what those actions mean for the
//! agent (via [`Live`]) and for the process (quit). Keys that need the agent
//! *before* the surface sees them — answering an approval, aborting a run —
//! are intercepted here, because the draft must not swallow them.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::app::TuiSettings;
use crate::components::chat_area::ReplyDetail;
use crate::rpc::types::AgentEvent;
use crate::terminal::Terminal;

use super::layout;
use super::live::{Live, Outcome};
use super::mouse;
use super::{WbAction, WbEvent, Workbench};

/// What a run of the workbench needs to know before it starts.
#[derive(Debug, Clone)]
pub struct Options {
    pub addr: String,
    /// A session to restore (`--session`).
    pub session: Option<String>,
    pub no_context_files: bool,
    /// A first message (`future tui "look at this"`), sent once attached.
    pub initial_prompt: Option<String>,
    /// `--continue`: attach to the most recently active session.
    pub continue_recent: bool,
    /// `~/.future/tui/settings.json` — the file the inline TUI also writes, so
    /// theme and reading level carry over between the two.
    pub settings_path: PathBuf,
}

enum TerminalEvent {
    Data(String),
    Resize,
}

/// Run the workbench until the reader quits. Returns an error only for a
/// failure that makes the surface impossible (no TTY, no agent).
pub async fn run(options: Options) -> Result<(), String> {
    let mut settings = load_settings(&options.settings_path);
    let (mut live, mut agent_events) = Live::attach(
        &options.addr,
        options.session.as_deref(),
        options.no_context_files,
        Some(&settings),
    )
    .await?;
    let mut workbench = Workbench::new();
    workbench.set_theme(&live.theme_id);

    if options.continue_recent && options.session.is_none() {
        live.attach_most_recent().await;
    }

    if let Some(prompt) = options
        .initial_prompt
        .filter(|text| !text.trim().is_empty())
    {
        live.apply(WbAction::Submit(prompt)).await;
    }

    let mut terminal = Terminal::new().map_err(|error| format!("terminal: {error}"))?;
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<TerminalEvent>();
    let data_tx = input_tx.clone();
    let resize_tx = input_tx.clone();
    terminal
        .start(
            Box::new(move |data| {
                let _ = data_tx.send(TerminalEvent::Data(data));
            }),
            Box::new(move || {
                let _ = resize_tx.send(TerminalEvent::Resize);
            }),
        )
        .map_err(|error| format!("terminal: {error}"))?;
    // SIGINT/SIGTERM take the same path as ctrl+c so the terminal is restored
    // by the loop's own exit rather than by the signal handler's re-raise.
    {
        let signal_tx = input_tx.clone();
        terminal.set_exit_signal_callback(Some(Box::new(move || {
            let _ = signal_tx.send(TerminalEvent::Data("\u{3}".into()));
        })));
    }
    terminal.write(mouse::MOUSE_ON);

    let outcome = event_loop(
        &mut terminal,
        &mut workbench,
        &mut live,
        &mut agent_events,
        &mut input_rx,
    )
    .await;

    // Leave the terminal exactly as it was found, whatever happened above.
    terminal.write(mouse::MOUSE_OFF);
    terminal.stop();

    // Persist what the reader chose, through the same file the inline TUI uses.
    if workbench.palette.id != settings.theme_id.as_deref().unwrap_or_default() {
        settings.theme_id = Some(workbench.palette.id.to_string());
    }
    let detail = live.detail().as_wire().to_string();
    if settings.reply_detail.as_deref() != Some(detail.as_str()) {
        settings.reply_detail = Some(detail);
    }
    save_settings(&options.settings_path, &settings);

    outcome
}

async fn event_loop(
    terminal: &mut Terminal,
    workbench: &mut Workbench,
    live: &mut Live,
    agent_events: &mut mpsc::UnboundedReceiver<AgentEvent>,
    input_rx: &mut mpsc::UnboundedReceiver<TerminalEvent>,
) -> Result<(), String> {
    let mut ticker = tokio::time::interval(Duration::from_millis(120));
    // The first tick fires immediately; consume it so the loop's first draw is
    // not followed by a burst.
    ticker.tick().await;

    loop {
        draw(terminal, workbench, live)?;

        tokio::select! {
            Some(input) = input_rx.recv() => {
                let events = match input {
                    TerminalEvent::Resize => vec![WbEvent::Resize(terminal.columns(), terminal.rows())],
                    TerminalEvent::Data(data) => events_of(&data),
                };
                for event in events {
                    if quit(workbench, live, event).await? {
                        return Ok(());
                    }
                }
            }
            Some(event) = agent_events.recv() => {
                let was = live.data(1).activity.kind;
                live.handle_event(event);
                // A run that started or stopped changes what the lists show
                // (a session's age, a title from its first message).
                if live.data(1).activity.kind != was {
                    live.refresh_sessions().await;
                }
            }
            _ = ticker.tick() => {}
        }
    }
}

/// Handle one event. Returns `true` when the reader asked to leave.
async fn quit(workbench: &mut Workbench, live: &mut Live, event: WbEvent) -> Result<bool, String> {
    // A parked approval owns `y` and `n`: the run is stopped until someone
    // answers, and the activity row is telling the reader exactly that.
    if let WbEvent::Key(key) = &event {
        if live.approval().is_some() {
            if matches!(key.as_str(), "y" | "Y") {
                live.decide(true).await;
                return Ok(false);
            }
            if matches!(key.as_str(), "n" | "N") {
                live.decide(false).await;
                return Ok(false);
            }
        }
        match key.as_str() {
            "ctrl+x" => {
                live.abort().await;
                return Ok(false);
            }
            "ctrl+e" => {
                let next = match live.detail() {
                    ReplyDetail::Default => ReplyDetail::Detailed,
                    ReplyDetail::Detailed => ReplyDetail::Default,
                };
                live.set_detail(next);
                return Ok(false);
            }
            "ctrl+n" => {
                // A new conversation: the surface's launchpad is not involved
                // here (the session is created straight away), so the reader
                // lands in an empty transcript, which is the same thing.
                return match live.apply(WbAction::Command("/new".into())).await {
                    Outcome::Quit => Ok(true),
                    Outcome::Continue => Ok(false),
                };
            }
            "ctrl+k" => {
                live.refresh_sessions().await;
                live.refresh_state().await;
                return Ok(false);
            }
            _ => {}
        }
    }

    let actions = workbench.handle(event, &live.data(1));
    for action in actions {
        if matches!(action, WbAction::Command(ref command) if command == "/theme") {
            workbench.cycle_theme();
            live.theme_id = workbench.palette.id.to_string();
            continue;
        }
        if matches!(action, WbAction::Submit(_)) {
            // The surface clears nothing itself: the adapter owns the draft.
            live.set_draft(String::new(), 0);
        }
        match live.apply(action).await {
            Outcome::Quit => return Ok(true),
            Outcome::Continue => {}
        }
    }
    Ok(false)
}

/// Split one chunk of terminal input into workbench events.
///
/// A bracketed paste arrives as one chunk; a chunk that opens one but does not
/// close it is completed by the next read, so the surface is handed the paste
/// as a unit rather than keystroke by keystroke.
fn events_of(data: &str) -> Vec<WbEvent> {
    let mut events = Vec::new();
    let mut rest = data;
    while let Some(start) = rest.find("\x1b[200~") {
        let after = &rest[start..];
        let Some(close) = after.find("\x1b[201~") else {
            // Unterminated: keep it whole so the caller's next read can finish
            // it. (The surface holds a paste as text; splitting one in half
            // would insert the escape bytes.)
            if start > 0 {
                events.extend(key_events(&rest[..start]));
            }
            events.push(WbEvent::Paste(
                after.trim_start_matches("\x1b[200~").to_string(),
            ));
            return events;
        };
        if start > 0 {
            events.extend(key_events(&rest[..start]));
        }
        let body = &after["\x1b[200~".len()..close];
        events.push(WbEvent::Paste(body.to_string()));
        rest = &after[close + "\x1b[201~".len()..];
    }
    events.extend(key_events(rest));
    events
}

fn key_events(data: &str) -> Vec<WbEvent> {
    if data.is_empty() {
        return Vec::new();
    }
    if let Some(event) = mouse::parse(data) {
        return vec![WbEvent::Mouse(event)];
    }
    if mouse::is_partial(data) {
        return Vec::new();
    }
    if let Some(key) = crate::keys::parse_key(data) {
        return vec![WbEvent::Key(key)];
    }
    // Anything the key parser did not name (a multi-byte character, for
    // instance) is text, and belongs in the draft as-is.
    vec![WbEvent::Paste(data.to_string())]
}

fn draw(terminal: &mut Terminal, workbench: &mut Workbench, live: &mut Live) -> Result<(), String> {
    let width = terminal.columns().max(1);
    let height = terminal.rows().max(1);
    let frame_layout = layout::layout(
        width,
        height,
        if workbench.sidebar_visible {
            workbench.sidebar_w
        } else {
            0
        },
    );
    let inner = frame_layout
        .body
        .w
        .saturating_sub(layout::CHAT_PADDING * 2)
        .max(1) as usize;
    let data = live.data(inner);
    let rows = workbench.render(&data, width, height);

    let mut frame = String::with_capacity(rows.len() * (width as usize + 8));
    // Hide the cursor while the frame is written (it would otherwise run down
    // the screen), then put it where the prompt is.
    frame.push_str("\x1b[?25l\x1b[H");
    for row in &rows {
        frame.push_str(row);
        frame.push_str("\x1b[0m\r\n");
    }
    if let Some((x, y)) = workbench.cursor() {
        frame.push_str(&format!("\x1b[{};{}H\x1b[?25h", y + 1, x + 1));
    }
    terminal.write(&frame);
    Ok(())
}

/// The inline TUI's settings file, read the same way it reads it (so a theme
/// chosen in one surface is the theme of the other).
pub fn settings_path() -> PathBuf {
    crate::home::future_state_dir()
        .join("tui")
        .join("settings.json")
}

pub(crate) fn load_settings(path: &Path) -> TuiSettings {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    TuiSettings::from_json(&value)
}

pub(crate) fn save_settings(path: &Path, settings: &TuiSettings) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let value = serde_json::to_string_pretty(&settings.to_json()).unwrap_or_default();
    let _ = std::fs::write(path, value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_key_becomes_one_key_event() {
        assert_eq!(events_of("a"), vec![WbEvent::Key("a".into())]);
        assert_eq!(events_of("\u{3}"), vec![WbEvent::Key("ctrl+c".into())]);
    }

    #[test]
    fn a_bracketed_paste_is_one_event_without_its_marks() {
        let events = events_of("\x1b[200~hello\nworld\x1b[201~");
        assert_eq!(events, vec![WbEvent::Paste("hello\nworld".into())]);
    }

    #[test]
    fn a_paste_with_text_on_both_sides_keeps_the_order() {
        let events = events_of("a\x1b[200~pasted\x1b[201~b");
        assert_eq!(
            events,
            vec![
                WbEvent::Key("a".into()),
                WbEvent::Paste("pasted".into()),
                WbEvent::Key("b".into()),
            ]
        );
    }

    #[test]
    fn an_unterminated_paste_is_held_rather_than_split() {
        let events = events_of("\x1b[200~half a paste");
        assert_eq!(events, vec![WbEvent::Paste("half a paste".into())]);
    }

    #[test]
    fn a_mouse_report_is_decoded_not_typed() {
        let events = events_of("\x1b[<0;11;4M");
        assert!(
            matches!(events.as_slice(), [WbEvent::Mouse(_)]),
            "{events:?}"
        );
    }

    #[test]
    fn a_multibyte_character_is_text() {
        assert_eq!(events_of("中"), vec![WbEvent::Paste("中".into())]);
    }
}
