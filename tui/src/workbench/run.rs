//! The workbench's driver: terminal, event loop, and the keys that are not the
//! surface's own.
//!
//! `future tui` ends up here. The loop is async (the agent, the terminal input
//! and the animation tick are all wake-ups), and it draws a frame *before*
//! waiting: the screen is never stale while it waits for something to happen.
//!
//! The split of responsibility is the same one the whole module tree follows:
//! [`Workbench`] decides what a key or a click *means* for the surface and
//! returns [`WbAction`]s; this loop decides what those actions mean for the
//! agent (via [`Live`]) and for the process (quit). Keys that need the agent
//! *before* the surface sees them — answering an approval, aborting a run —
//! are intercepted here, because the draft must not swallow them.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::components::chat_area::ReplyDetail;
use crate::rpc::types::AgentEvent;
use crate::settings::TuiSettings;
use crate::terminal::Terminal;

use super::layout::{self, Frame};
use super::live::{self, Live, Outcome};
use super::mouse;
use super::picker::PickerKind;
use super::state::PanelId;
use super::{VerbOutcome, WbAction, WbEvent, Workbench};

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
    /// `--offline`: skip the startup update check.
    pub offline: bool,
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
    workbench.set_language(live::persisted_language(&settings));
    workbench.set_preferences(super::preferences::LayoutPreferences {
        density: settings
            .density
            .as_deref()
            .map(super::preferences::Density::from_wire)
            .unwrap_or_default(),
        sidebar_position: settings
            .sidebar_position
            .as_deref()
            .map(super::preferences::SidebarPosition::from_wire)
            .unwrap_or_default(),
        sidebar_width: settings.sidebar_width.unwrap_or(layout::SIDEBAR_DEFAULT),
        sidebar_visible: settings.sidebar_visible.unwrap_or(true),
    });
    workbench.keys = super::preferences::Keybindings::load(&super::preferences::keybindings_path(
        &options.settings_path,
    ));

    if options.continue_recent && options.session.is_none() {
        live.attach_most_recent().await;
    }

    if let Some(prompt) = options
        .initial_prompt
        .filter(|text| !text.trim().is_empty())
    {
        live.apply(WbAction::Submit(prompt)).await;
    }

    // The startup update notice: best effort, never blocks the surface, and
    // skipped with `--offline`. It lands on the notice row, which is where a
    // reader looks for "something the surface wants to tell you".
    if !options.offline {
        if let Some(notice) = crate::update::check(crate::version::VERSION).await {
            live.notice = Some(notice);
        }
    }

    let mut terminal = Terminal::new().map_err(|error| format!("terminal: {error}"))?;
    // The workbench is clickable, so it owns mouse reporting for as long as the
    // alternate screen is up: `start` writes the mode in, `stop` takes it out —
    // and both failures' paths (the exit failsafe and the crash hook) reset it
    // too, so the shell never inherits a captured mouse.
    terminal.set_mouse_mode(true);
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
    let outcome = event_loop(
        &mut terminal,
        &mut workbench,
        &mut live,
        &mut agent_events,
        &mut input_rx,
        &mut settings,
        &options.settings_path,
    )
    .await;

    // Leave the terminal exactly as it was found, whatever happened above.
    terminal.stop();

    // Persist what the reader chose, through the same file the inline TUI uses.
    if workbench.palette.id != settings.theme_id.as_deref().unwrap_or_default() {
        settings.theme_id = Some(workbench.palette.id.to_string());
    }
    if settings.language.as_deref() != Some(workbench.language_pref.as_wire()) {
        settings.language = Some(workbench.language_pref.as_wire().to_string());
    }
    let detail = live.detail().as_wire().to_string();
    if settings.reply_detail.as_deref() != Some(detail.as_str()) {
        settings.reply_detail = Some(detail);
    }
    persist_preferences(&workbench, &mut settings, &options.settings_path);
    let _ = workbench.keys.save(&super::preferences::keybindings_path(
        &options.settings_path,
    ));

    outcome
}

async fn event_loop(
    terminal: &mut Terminal,
    workbench: &mut Workbench,
    live: &mut Live,
    agent_events: &mut mpsc::UnboundedReceiver<AgentEvent>,
    input_rx: &mut mpsc::UnboundedReceiver<TerminalEvent>,
    settings: &mut TuiSettings,
    settings_path: &Path,
) -> Result<(), String> {
    // The first frame is composed for the size the terminal has *now*. The
    // cached size dates from `Terminal::new()`/`start()`, and a pane a terminal
    // multiplexer resized while the agent was attaching is still stale there
    // until a resize notification reaches the reader thread — which is how one
    // instance painted a first frame a row short and repaired it on the next
    // redraw. One refresh here, before the loop's first `draw`, is what makes
    // the first frame the steady-state frame: `draw` re-reads the size and
    // `compose` rebuilds the layout *and* the hit map from the same numbers, so
    // the metrics a mouse report is resolved against belong to the frame on
    // screen from the first paint.
    terminal.refresh_size();

    let mut ticker = tokio::time::interval(TICK);
    // The first tick fires immediately; consume it so the loop's first draw is
    // not followed by a burst.
    ticker.tick().await;
    let mut clock = TickClock::new(workbench.spinner_frame(), Instant::now());
    // Something is on screen to paint from the first iteration — and after
    // that only when an event or the clock changed what the frame shows. A
    // ticker that repaints an unchanged frame is not free: composing it means
    // rebuilding `WorkbenchData`, which asks the transcript for every row it
    // has rendered (see `Live::data`).
    let mut paint = true;

    loop {
        // The frame's own metrics, computed once: the surface composes for
        // them, and the event path below hands the transcript the *same*
        // width. A second, different width (the old loop used `1` to read one
        // field) makes `ChatArea` discard its rendered rows and re-wrap the
        // whole transcript on every event.
        let frame = frame_of(terminal, workbench);
        if paint {
            draw(terminal, workbench, live, &frame)?;
            clock.painted(workbench.spinner_frame(), Instant::now());
        }

        // `paint` belongs to the *next* frame: every arm below assigns it — an
        // input or agent event paints, and the ticker asks the clock.
        tokio::select! {
            Some(input) = input_rx.recv() => {
                let events = match &input {
                    TerminalEvent::Resize => vec![WbEvent::Resize(terminal.columns(), terminal.rows())],
                    TerminalEvent::Data(data) => events_of(data),
                };
                for event in events {
                    if quit(workbench, live, event, &frame, terminal, settings, settings_path).await? {
                        return Ok(());
                    }
                }
                paint = true;
            }
            Some(event) = agent_events.recv() => {
                let was = live.activity_kind();
                live.handle_event(event);
                if live.approval().is_some() && workbench.overlay.is_none() {
                    workbench.overlay = Some(super::Overlay::Approval);
                    workbench.overlay_scroll = 0;
                }
                // What arrived while this event was being applied is applied
                // now, before the frame: a streaming turn emits deltas at tens
                // per second and a frame per token is work no one can see (the
                // screen only ever shows the last one).
                drain(agent_events, |event| live.handle_event(event));
                // A `usage` or `agent_end` event means the agent knows more
                // about what the turn spent than the deltas do: ask it, so the
                // footer's ctx/cost/cache move at the end of a turn instead of
                // only when someone presses ctrl+k.
                if live.take_state_refresh() {
                    live.refresh_state().await;
                    if live.approval().is_some() && workbench.overlay.is_none() {
                        workbench.overlay = Some(super::Overlay::Approval);
                        workbench.overlay_scroll = 0;
                    }
                }
                // A run that started or stopped changes what the lists show
                // (a session's age, a title from its first message).
                if live.activity_kind() != was {
                    live.refresh_sessions().await;
                }
                paint = true;
            }
            _ = ticker.tick() => {
                // The spinner is the only animation a tick owns; it advances
                // with the ticker and owes a frame when its drawn frame
                // changes. Nothing else in the frame is finer than a minute.
                // A short terminal lays out no activity row at all
                // (`compact`), so there is nothing on screen to animate.
                let animating = live.activity_animating() && frame.layout.activity.is_some();
                if animating {
                    workbench.advance_tick();
                }
                paint = clock.tick(
                    animating,
                    workbench.spinner_frame(),
                    ages_on_screen(workbench, &frame),
                    Instant::now(),
                );
            }
        }
    }
}

/// How often the loop wakes up. The ticker is a clock, not a paint: what it
/// animates is the activity row's spinner (`SPINNER_TICKS` ticks a frame).
const TICK: Duration = Duration::from_millis(120);

/// How long an idle frame may go unpainted while the only thing on screen that
/// changes with time is the sidebar's session ages ("now", "5m", "3h"…) — one
/// minute, the finest step that column has. An idle loop that repainted on the
/// tick instead would re-compose a frame eight times a second to draw the same
/// bytes.
const IDLE_REFRESH: Duration = Duration::from_secs(60);

/// How many already-queued agent events one iteration applies before painting.
///
/// The limit exists so a producer that outruns the surface for a long time
/// cannot keep the loop away from the keyboard and from painting; it is far
/// above a normal burst's length (a streamed token run between two frames is
/// hundreds of deltas at most, not thousands), so in practice a whole run is
/// coalesced into one frame and the cap is only ever reached when something has
/// gone wrong upstream.
const COALESCE_LIMIT: usize = 4096;

/// Apply every agent event that is already queued, stopping at
/// [`COALESCE_LIMIT`] so the loop always gets back to painting and reading the
/// keyboard. The events are applied in order, so the transcript the next frame
/// paints is the same one a frame per event would have produced.
///
/// Returns how many it applied — the count is what pins the bound in the test
/// below; the loop itself does not need it.
fn drain(
    agent_events: &mut mpsc::UnboundedReceiver<AgentEvent>,
    mut apply: impl FnMut(AgentEvent),
) -> usize {
    let mut applied = 0;
    while applied < COALESCE_LIMIT {
        match agent_events.try_recv() {
            Ok(event) => {
                apply(event);
                applied += 1;
            }
            Err(_) => break,
        }
    }
    applied
}

/// Whether the sidebar is showing the one panel that reads the clock: the
/// session list's ages. Any other panel is static while nothing happens.
fn ages_on_screen(workbench: &Workbench, frame: &Frame) -> bool {
    // The frame's own layout says whether a rail was drawn (a hidden or
    // too-narrow sidebar has none), so this asks what is on screen rather than
    // what the reader would like to be.
    frame.layout.sidebar.is_some() && workbench.panel == PanelId::Sessions
}

/// Whether a 120 ms tick owes the screen a frame.
///
/// Two clocks reach the frame and neither is worth a repaint of the whole
/// surface: the activity row's spinner and elapsed counter (sub-second, only
/// while a run is in flight) and the sidebar's session ages (minute
/// granularity, only while that panel is on screen). Everything else in a frame
/// changes only when an event changes it, and an event paints anyway.
///
/// Split out from the loop so the "an idle tick paints nothing" contract is a
/// unit test rather than something to believe about a `select!` arm.
struct TickClock {
    /// The spinner frame the last paint drew.
    spinner: usize,
    /// When the frame was last painted, by any cause.
    painted_at: Instant,
}

impl TickClock {
    fn new(spinner: usize, now: Instant) -> Self {
        Self {
            spinner,
            painted_at: now,
        }
    }

    /// A frame was painted for another reason (an event, the first frame).
    fn painted(&mut self, spinner: usize, now: Instant) {
        self.spinner = spinner;
        self.painted_at = now;
    }

    /// The ticker fired with nothing else to report: is a frame owed?
    ///
    /// `animating` — the activity row is drawing the spinner or the elapsed
    /// counter; `spinner` — the frame that counter is on now; `ages` — the
    /// session list is on screen, so its minute-granularity ages are too.
    fn tick(&mut self, animating: bool, spinner: usize, ages: bool, now: Instant) -> bool {
        if animating {
            // The spinner's own step bounds how fast an animating row can
            // change (the elapsed counter is coarser), so it is exactly when a
            // frame is due — two a second instead of eight.
            if spinner == self.spinner {
                return false;
            }
            self.painted(spinner, now);
            return true;
        }
        if !ages {
            return false;
        }
        if now.duration_since(self.painted_at) < IDLE_REFRESH {
            return false;
        }
        self.painted(spinner, now);
        true
    }
}

/// Handle one event. Returns `true` when the reader asked to leave.
async fn quit(
    workbench: &mut Workbench,
    live: &mut Live,
    event: WbEvent,
    frame: &Frame,
    terminal: &mut Terminal,
    settings: &mut TuiSettings,
    settings_path: &Path,
) -> Result<bool, String> {
    // A parked approval owns y/n only on the base surface. Inside its card
    // these keys belong to the card, and other overlays never approve blindly.
    if let WbEvent::Key(key) = &event {
        if live.approval().is_some() && workbench.overlay.is_none() {
            if matches!(key.as_str(), "y" | "Y") {
                live.decide(true).await;
                return Ok(false);
            }
            if matches!(key.as_str(), "n" | "N") {
                live.decide(false).await;
                return Ok(false);
            }
            if matches!(key.as_str(), "enter") {
                workbench.overlay = Some(super::Overlay::Approval);
                workbench.overlay_scroll = 0;
                return Ok(false);
            }
        }
        // Configurable global actions are handled below, after the approval
        // gate. Keeping them out of this literal-chord fast path makes the
        // displayed keybinding and the one that actually fires agree.
    }

    // Global actions that need the adapter use the same configurable map as
    // the surface. Overlay keys still belong to the overlay.
    if let WbEvent::Key(key) = &event {
        if workbench.overlay.is_none() {
            match key.as_str() {
                "f6" => {
                    copy_last(live).await;
                    return Ok(false);
                }
                "f7" => {
                    toggle_mouse(live, terminal);
                    return Ok(false);
                }
                "ctrl+g" => {
                    live.refresh_state().await;
                    let _ = workbench.surface_command("/queue", &live.data(frame));
                    return Ok(false);
                }
                _ => {}
            }
            match workbench.keys.action(key) {
                Some(super::preferences::Action::Stop) => {
                    live.abort().await;
                    return Ok(false);
                }
                Some(super::preferences::Action::ReplyDetail) => {
                    let next = match live.detail() {
                        ReplyDetail::Default => ReplyDetail::Detailed,
                        ReplyDetail::Detailed => ReplyDetail::Default,
                    };
                    live.set_detail(next);
                    return Ok(false);
                }
                Some(super::preferences::Action::NewSession) => {
                    return match live.apply(WbAction::Command("/new".into())).await {
                        Outcome::Quit => Ok(true),
                        Outcome::Continue => Ok(false),
                    };
                }
                Some(super::preferences::Action::Refresh) => {
                    live.refresh_sessions().await;
                    live.refresh_state().await;
                    return Ok(false);
                }
                _ => {}
            }
        }
    }
    // The frame's own width, not a probe: see `Frame` and `Live::data`.
    let actions = workbench.handle(event, &live.data(frame));
    for action in actions {
        // The three list menus belong to the surface: the app owns the agent,
        // not the picker. Opening it here (rather than in `live.command`) is
        // what lets the palette and the draft both reach it.
        if let WbAction::Command(command) = &action {
            match command.trim() {
                "/approval" => {
                    live.refresh_state().await;
                    workbench.overlay = Some(super::Overlay::Approval);
                    workbench.overlay_scroll = 0;
                    continue;
                }
                "/diff" => {
                    let selected = workbench.workflow_index;
                    let cwd = live.data(frame).session.cwd;
                    let previous = live.selected_diff_path(selected);
                    let result = tokio::task::spawn_blocking(move || {
                        let mut view = super::state::DiffView::default();
                        match super::diff_review::load(std::path::Path::new(&cwd), &previous) {
                            Ok(review) => {
                                view.selected = review
                                    .files
                                    .iter()
                                    .position(|file| file.path == previous)
                                    .unwrap_or(0);
                                view.files = review.files;
                                view.patch = super::diff_review::bounded_patch(&review.patch);
                            }
                            Err(error) => view.error = Some(error),
                        }
                        view
                    })
                    .await;
                    match result {
                        Ok(view) => live.set_diff(view),
                        Err(error) => live.notice = Some(error.to_string()),
                    }
                    workbench.overlay = Some(super::Overlay::Diff);
                    workbench.workflow_index = selected;
                    workbench.overlay_scroll = 0;
                    continue;
                }
                "/history" | "/tree" => {
                    live.refresh_sessions().await;
                    workbench.overlay = Some(if command.trim() == "/history" {
                        super::Overlay::History
                    } else {
                        super::Overlay::Tree
                    });
                    workbench.workflow_index = 0;
                    workbench.overlay_scroll = 0;
                    continue;
                }
                "/fork" => {
                    live.load_fork_points().await;
                    workbench.overlay = Some(super::Overlay::Fork);
                    workbench.workflow_index = 0;
                    continue;
                }
                "/worktree" | "/worktree " => {
                    let cwd = live.data(frame).session.cwd;
                    let result = tokio::task::spawn_blocking(move || {
                        crate::worktree::probe_repo(
                            &crate::worktree::GitCli::default(),
                            &cwd,
                            std::env::current_dir().ok().as_deref(),
                        )
                    })
                    .await;
                    match result {
                        Ok(Ok(rows)) => live.set_worktrees(rows),
                        Ok(Err(error)) => {
                            live.notice = Some(super::i18n::render(
                                live.language(),
                                super::i18n::Key::WorkflowFailed,
                                &[&error],
                            ))
                        }
                        Err(error) => live.notice = Some(error.to_string()),
                    }
                    workbench.overlay = Some(super::Overlay::Worktree);
                    workbench.workflow_index = 0;
                    continue;
                }
                "/copy" => {
                    copy_last(live).await;
                    continue;
                }
                "/mouse" => {
                    toggle_mouse(live, terminal);
                    continue;
                }
                _ => {}
            }
            if let Some(branch) = command.trim().strip_prefix("/worktree new ") {
                let cwd = live.data(frame).session.cwd;
                let branch = branch.to_string();
                let created = tokio::task::spawn_blocking(move || {
                    crate::worktree::create_worktree(
                        &crate::worktree::GitCli::default(),
                        &cwd,
                        std::env::current_dir().ok().as_deref(),
                        &branch,
                    )
                })
                .await;
                match created {
                    Ok(created) => live.create_worktree_with(|| created).await,
                    Err(error) => live.notice = Some(error.to_string()),
                }
                continue;
            }
            if let Some(kind) = PickerKind::from_command(command) {
                let data = live.data(frame);
                workbench.open_picker(kind, &data);
                continue;
            }
            // A verb that names something on screen is answered by the surface
            // (a panel to show, a workspace to explain). The palette advertises
            // these, so they must not reach the agent: it has no such commands
            // and answered `Unknown command` for all five.
            if let VerbOutcome::Done { notice } =
                workbench.surface_command(command, &live.data(frame))
            {
                live.notice = notice;
                continue;
            }
        }
        // A committed palette is persisted now, not only at exit: a reader who
        // picks a theme and then loses the terminal keeps the choice.
        if let WbAction::SetTheme(id) = &action {
            workbench.set_theme(id);
            live.theme_id = id.clone();
            settings.theme_id = Some(id.clone());
            save_settings(settings_path, settings);
            continue;
        }
        // Same rule for the language: the surface and the adapter both switch
        // now, and `settings.json` remembers it for the next start.
        if let WbAction::SetLanguage(preference) = action {
            workbench.set_language(preference);
            live.set_language(preference);
            settings.language = Some(preference.as_wire().to_string());
            save_settings(settings_path, settings);
            continue;
        }
        if matches!(
            action,
            WbAction::SetPreferences | WbAction::ResizeSidebar(_) | WbAction::PersistSidebar
        ) {
            persist_preferences(workbench, settings, settings_path);
        }
        if matches!(action, WbAction::SetKeybindings) {
            let _ = workbench
                .keys
                .save(&super::preferences::keybindings_path(settings_path));
        }
        if matches!(action, WbAction::OpenQueue) {
            live.refresh_state().await;
            if let VerbOutcome::Done { notice } =
                workbench.surface_command("/queue", &live.data(frame))
            {
                live.notice = notice;
            }
            continue;
        }
        if matches!(action, WbAction::SelectDiff(_)) {
            let selected = match action {
                WbAction::SelectDiff(index) => index,
                _ => 0,
            };
            let data = live.data(frame);
            let path = data
                .diff
                .as_ref()
                .and_then(|diff| diff.files.get(selected))
                .map(|file| file.path.clone());
            if let Some(path) = path {
                let cwd = data.session.cwd;
                let result = tokio::task::spawn_blocking(move || {
                    super::diff_review::load(std::path::Path::new(&cwd), &path)
                })
                .await;
                match result {
                    Ok(Ok(review)) => live.set_diff(super::state::DiffView {
                        files: review.files,
                        selected,
                        patch: super::diff_review::bounded_patch(&review.patch),
                        error: None,
                    }),
                    Ok(Err(error)) => live.notice = Some(error),
                    Err(error) => live.notice = Some(error.to_string()),
                }
            }
            workbench.overlay = Some(super::Overlay::Diff);
            continue;
        }
        if matches!(action, WbAction::OpenApproval) {
            workbench.overlay = Some(super::Overlay::Approval);
            continue;
        }
        if matches!(action, WbAction::CopyLast) {
            copy_last(live).await;
            continue;
        }
        if matches!(action, WbAction::ToggleMouse) {
            toggle_mouse(live, terminal);
            continue;
        }
        if let WbAction::Notice(notice) = &action {
            live.notice = Some(notice.clone());
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
        // A model or level chosen with no session attached is nothing the agent
        // can hold: `live.apply` staged it on the adapter, which asks for it
        // when the session is created, and `persist_pending` remembers it as
        // this terminal UI's default. One rule, so the picker's row and the
        // launchpad's own chips land in the same place.
        persist_pending(live, settings, settings_path);
    }
    Ok(false)
}

/// Write the launchpad's staged model and level into `settings.json`, which is
/// where the terminal UI keeps its defaults.
///
/// Called after every applied action and writes only on a change — a staging
/// action is rare, and a launchpad chip that cycles through the list would
/// otherwise rewrite the file on each step.
fn persist_pending(live: &Live, settings: &mut TuiSettings, path: &Path) {
    let mut changed = false;
    if let Some(model) = live.pending_model() {
        if settings.default_model.as_deref() != Some(model) {
            settings.default_model = Some(model.to_string());
            changed = true;
        }
    }
    if let Some(effort) = live.pending_effort() {
        if settings.default_thinking_level.as_deref() != Some(effort) {
            settings.default_thinking_level = Some(effort.to_string());
            changed = true;
        }
    }
    if changed {
        save_settings(path, settings);
    }
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

/// The frame the surface is composed for right now, computed once per loop
/// iteration and handed to both halves of it — the composer and the event path
/// — so the width the transcript wraps to is the width it is drawn at.
fn frame_of(terminal: &Terminal, workbench: &Workbench) -> Frame {
    Frame::new(
        terminal.columns(),
        terminal.rows(),
        if workbench.sidebar_visible {
            workbench.sidebar_w
        } else {
            0
        },
        workbench.density,
        workbench.sidebar_position,
    )
}

async fn copy_last(live: &mut Live) {
    match live.copy_last() {
        Ok(text) => {
            let result =
                tokio::task::spawn_blocking(move || crate::clipboard::copy_text(&text)).await;
            live.notice = Some(match result {
                Ok(Ok(())) => super::i18n::t(live.language(), super::i18n::Key::CopyDone).into(),
                Ok(Err(error)) => {
                    super::i18n::render(live.language(), super::i18n::Key::CopyFailed, &[&error])
                }
                Err(error) => {
                    super::i18n::render(live.language(), super::i18n::Key::CopyFailed, &[&error])
                }
            });
        }
        Err(error) => live.notice = Some(error),
    }
}

fn toggle_mouse(live: &mut Live, terminal: &mut Terminal) {
    let enabled = live.toggle_mouse();
    terminal.set_mouse_mode(enabled);
    live.notice = Some(
        super::i18n::t(
            live.language(),
            if enabled {
                super::i18n::Key::MouseOn
            } else {
                super::i18n::Key::MouseOff
            },
        )
        .into(),
    );
}

fn draw(
    terminal: &mut Terminal,
    workbench: &mut Workbench,
    live: &mut Live,
    frame: &Frame,
) -> Result<(), String> {
    let data = live.data(frame);
    let rows = workbench.render(&data, frame.width, frame.height);
    terminal.write(&frame_bytes(&rows, workbench.cursor()));
    Ok(())
}

/// The bytes one frame becomes: cursor hidden, each row placed at its own
/// absolute line, and the cursor put where the prompt is.
///
/// **Every row is addressed, never advanced to with a line feed.** A full-screen
/// surface may not assume its idea of the height matches the terminal's: the
/// pty's size reaches the process through SIGWINCH, so between a resize and the
/// next frame — or on any terminal that reported a size the program has not yet
/// observed — a frame written with `\r\n` runs past the last row and the
/// terminal scrolls. The result is not a clipped frame but a *shifted* one: the
/// header leaves the screen, every band sits some cells above the row the layout
/// assigned it, and an SGR mouse report (which names a fixed cell) then lands on
/// the wrong thing. Measured in a ttyd-rendered terminal at 1000x700: two frames
/// after start, only the last seven rows were still visible.
///
/// Absolute addressing makes a size mismatch harmless: rows beyond the terminal
/// are simply not drawn (`CUP` to a row past the end leaves the cursor at the
/// last row and writes nothing else), and no write ever scrolls the screen.
fn frame_bytes(rows: &[String], cursor: Option<(u16, u16)>) -> String {
    let mut frame = String::with_capacity(rows.len() * 64);
    frame.push_str("\x1b[?25l");
    for (index, row) in rows.iter().enumerate() {
        // `CSI row;1H` — absolute, 1-based. The reset first so a row's styling
        // cannot bleed into the positioning sequence.
        frame.push_str(&format!("\x1b[0m\x1b[{};1H", index + 1));
        frame.push_str(row);
    }
    frame.push_str("\x1b[0m");
    if let Some((x, y)) = cursor {
        frame.push_str(&format!("\x1b[{};{}H\x1b[?25h", y + 1, x + 1));
    }
    frame
}

fn persist_preferences(workbench: &Workbench, settings: &mut TuiSettings, path: &Path) {
    let preferences = workbench.preferences();
    settings.density = Some(preferences.density.as_wire().into());
    settings.sidebar_position = Some(preferences.sidebar_position.as_wire().into());
    settings.sidebar_width = Some(preferences.sidebar_width);
    settings.sidebar_visible = Some(preferences.sidebar_visible);
    save_settings(path, settings);
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
    use crate::workbench::SPINNER_TICKS;

    /// The ticker is a clock, not a paint. With nothing on screen that reads it
    /// — no run in flight, no session list — a minute of ticks (500 of them at
    /// the 120 ms cadence) owes the screen exactly nothing, which is the whole
    /// difference between an idle `future tui` and one that re-composes its
    /// transcript eight times a second.
    #[test]
    fn an_idle_tick_with_no_clock_on_screen_paints_nothing() {
        let start = Instant::now();
        let mut clock = TickClock::new(0, start);
        for tick in 1..=500u32 {
            let now = start + TICK * tick;
            assert!(!clock.tick(false, 0, false, now), "tick {tick} painted");
        }
    }

    /// With the session list on screen the only thing that changes is that
    /// column's ages, which read in whole minutes: one frame a minute, not 500.
    #[test]
    fn an_idle_tick_paints_the_session_ages_once_a_minute() {
        let start = Instant::now();
        let mut clock = TickClock::new(0, start);
        let mut painted = Vec::new();
        for tick in 1..=1000u32 {
            // Two minutes of ticks.
            let now = start + TICK * tick;
            assert!(now - start <= Duration::from_secs(120));
            if clock.tick(false, 0, true, now) {
                painted.push(now - start);
            }
        }
        assert_eq!(
            painted,
            vec![Duration::from_secs(60), Duration::from_secs(120)],
            "an idle frame is at most one minute stale, and never fresher"
        );
    }

    /// A run in flight animates the spinner, and the spinner holds a frame for
    /// [`SPINNER_TICKS`] ticks: two frames a second rather than one per tick.
    #[test]
    fn a_run_in_flight_paints_once_per_spinner_frame() {
        let start = Instant::now();
        let mut clock = TickClock::new(0, start);
        let mut painted = 0;
        for tick in 1..=40usize {
            let now = start + TICK * tick as u32;
            // What the loop does with the same tick: step, then ask.
            let spinner = tick / SPINNER_TICKS;
            if clock.tick(true, spinner, false, now) {
                painted += 1;
            }
        }
        // Four ticks make a spinner frame, so 40 ticks are ten frames — and the
        // spinner frame, not the tick, is what bounds an animating repaint.
        assert_eq!(painted, 40 / SPINNER_TICKS);
    }

    /// A paint for any other reason restarts the idle clock: the frame on
    /// screen is fresh, so the minute is counted from it.
    #[test]
    fn a_paint_for_another_reason_restarts_the_idle_clock() {
        let start = Instant::now();
        let mut clock = TickClock::new(0, start);
        // Half a minute, then an event paints.
        assert!(!clock.tick(false, 0, true, start + TICK * 250));
        let event_at = start + TICK * 250;
        clock.painted(0, event_at);
        // The minute is counted from the event, not from the last tick.
        assert!(!clock.tick(false, 0, true, event_at + Duration::from_secs(59)));
        assert!(clock.tick(false, 0, true, event_at + Duration::from_secs(60)));
    }

    /// A queued burst is applied in the order it arrived, and one iteration
    /// never swallows more than [`COALESCE_LIMIT`] of it — the cap is what keeps
    /// a producer that outruns the surface from starving the keyboard.
    #[test]
    fn a_queued_burst_is_applied_in_order_and_bounded() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        for i in 0..5 {
            tx.send(AgentEvent {
                r#type: format!("t{i}"),
                ..Default::default()
            })
            .unwrap();
        }
        let mut seen = Vec::new();
        assert_eq!(drain(&mut rx, |event| seen.push(event.r#type)), 5);
        assert_eq!(seen, ["t0", "t1", "t2", "t3", "t4"]);
        // An empty queue is not a wait: it returns at once, with nothing done.
        assert_eq!(drain(&mut rx, |_| unreachable!()), 0);

        // The cap holds even when more is queued than one iteration allows.
        for i in 0..COALESCE_LIMIT + 7 {
            tx.send(AgentEvent {
                r#type: format!("q{i}"),
                ..Default::default()
            })
            .unwrap();
        }
        let mut count = 0;
        assert_eq!(drain(&mut rx, |_| count += 1), COALESCE_LIMIT);
        assert_eq!(count, COALESCE_LIMIT);
        // …and the rest is still there for the next iteration, not dropped.
        assert_eq!(drain(&mut rx, |_| count += 1), 7);
        assert_eq!(count, COALESCE_LIMIT + 7);
    }

    /// A frame must never advance with a line feed: on a terminal shorter than
    /// the frame (or one whose resize has not reached the process yet) that
    /// scrolls the alternate screen and every row lands above its coordinate.
    #[test]
    fn a_frame_addresses_every_row_and_never_scrolls() {
        let rows = vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string(),
        ];
        let frame = frame_bytes(&rows, Some((4, 1)));
        assert!(
            !frame.contains('\n'),
            "no line feed in a full-screen frame: {frame:?}"
        );
        assert!(
            !frame.contains("\r"),
            "no carriage return either: {frame:?}"
        );
        // Each row is placed at its own 1-based line.
        for index in 0..rows.len() {
            assert!(
                frame.contains(&format!("\x1b[{};1H", index + 1)),
                "row {} is not addressed: {frame:?}",
                index + 1
            );
        }
        assert!(
            frame.contains("\x1b[2;5H"),
            "the cursor sits at its cell: {frame:?}"
        );
        // The rows appear in order, each after its own positioning sequence.
        let first = frame.find("first").unwrap();
        let second = frame.find("second").unwrap();
        let third = frame.find("third").unwrap();
        assert!(first < second && second < third, "{frame:?}");
    }

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

    /// The old shape wrote rows with `\r\n` and only kept the *last* one free
    /// of a line feed. That is not enough: a frame taller than the terminal
    /// scrolls anyway, and a scrolled frame puts every painted row above the
    /// coordinate the hit map (and therefore the mouse) believes it occupies.
    /// The frame now never emits a line feed at all — pinned above.
    #[test]
    fn a_frame_ends_with_a_reset_and_no_advance() {
        let rows: Vec<String> = (0..38).map(|row| format!("row {row}")).collect();
        let frame = frame_bytes(&rows, Some((4, 31)));
        assert!(!frame.contains("\r\n"));
        assert!(frame.contains("row 37"));
        assert!(frame.ends_with("\x1b[0m\x1b[32;5H\x1b[?25h"), "{frame:?}");
    }

    /// Without a cursor position the frame still ends with the reset.
    #[test]
    fn a_frame_without_a_cursor_ends_with_the_reset() {
        let rows = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            frame_bytes(&rows, None),
            "\x1b[?25l\x1b[0m\x1b[1;1Ha\x1b[0m\x1b[2;1Hb\x1b[0m"
        );
    }
}
