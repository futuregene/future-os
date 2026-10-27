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
use super::state::GitSection;
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

struct InputIo<'a> {
    terminal: &'a mut Terminal,
    settings: &'a mut TuiSettings,
    settings_path: &'a Path,
    theme_explicit: &'a mut bool,
}

struct DriverIo<'a> {
    terminal: &'a mut Terminal,
    notifications: &'a mut crate::notifications::NotifyCenter,
    theme_explicit: &'a mut bool,
}

/// The layout a stored [`TuiSettings`] stands for, before the first frame.
///
/// An absent key is the shipped default, not an error: a reader who never
/// touched a setting (a first start, or a `settings.json` written by an older
/// build) gets the same layout every time. The rail starts hidden — the body
/// is the conversation, and the rail is a thing the reader opens on purpose
/// (`ctrl+s`, or `shift+←`/`shift+→` to step its tabs) rather than
/// something to close before typing.
fn layout_preferences(
    settings: &TuiSettings,
    mouse_capture: bool,
) -> super::preferences::LayoutPreferences {
    super::preferences::LayoutPreferences {
        sidebar_position: settings
            .sidebar_position
            .as_deref()
            .map(super::preferences::SidebarPosition::from_wire)
            .unwrap_or_default(),
        sidebar_width: settings.sidebar_width.unwrap_or(layout::SIDEBAR_DEFAULT),
        sidebar_visible: settings.sidebar_visible.unwrap_or(false),
        mouse_capture,
        notify: settings.notify.unwrap_or_default(),
        reduced_motion: settings.reduced_motion.unwrap_or(false),
    }
}

/// Run the workbench until the reader quits. Returns an error only for a
/// failure that makes the surface impossible (no TTY, no agent).
pub async fn run(options: Options) -> Result<(), String> {
    let mut settings = load_settings(&options.settings_path);
    let mut theme_explicit = settings.theme_id.is_some();
    let (mut live, mut agent_events) = Live::attach(
        &options.addr,
        options.session.as_deref(),
        options.no_context_files,
        Some(&settings),
    )
    .await?;
    let mut workbench = Workbench::new();
    let mouse_capture = settings.mouse_capture.unwrap_or(true);
    live.set_mouse_capture(mouse_capture);
    workbench.set_theme(&live.theme_id);
    workbench.set_language(live::persisted_language(&settings));
    workbench.set_preferences(layout_preferences(&settings, mouse_capture));
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
    terminal.set_mouse_mode(mouse_capture);

    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<TerminalEvent>();
    let data_tx = input_tx.clone();
    let resize_tx = input_tx.clone();
    Terminal::start(
        &mut terminal,
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
    let mut notifications = crate::notifications::NotifyCenter::default();
    let outcome = event_loop(
        &mut DriverIo {
            terminal: &mut terminal,
            notifications: &mut notifications,
            theme_explicit: &mut theme_explicit,
        },
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
    if theme_explicit && workbench.palette.id != settings.theme_id.as_deref().unwrap_or_default() {
        settings.theme_id = Some(workbench.palette.id.to_string());
    }
    if settings.language.as_deref() != Some(workbench.language_pref.as_wire()) {
        settings.language = Some(workbench.language_pref.as_wire().to_string());
    }
    // The reading level is deliberately not written: `ctrl+e` expands the
    // transcript for this run only, so the next terminal opens folded.
    persist_preferences(&workbench, &mut settings, &options.settings_path);
    let _ = workbench.keys.save(&super::preferences::keybindings_path(
        &options.settings_path,
    ));

    outcome
}

async fn event_loop(
    io: &mut DriverIo<'_>,
    workbench: &mut Workbench,
    live: &mut Live,
    agent_events: &mut mpsc::UnboundedReceiver<AgentEvent>,
    input_rx: &mut mpsc::UnboundedReceiver<TerminalEvent>,
    settings: &mut TuiSettings,
    settings_path: &Path,
) -> Result<(), String> {
    let DriverIo {
        terminal,
        notifications,
        theme_explicit,
    } = io;
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
    // The rail's list is re-read on its own clock (see `SESSIONS_POLL`), starting
    // now: the first frame paints what the attach already read.
    let mut sessions_poll = Instant::now();
    // Something is on screen to paint from the first iteration — and after
    // that only when an event or the clock changed what the frame shows. A
    // ticker that repaints an unchanged frame is not free: composing it means
    // rebuilding `WorkbenchData`, which asks the transcript for every row it
    // has rendered (see `Live::data`).
    let mut paint = true;
    // A stepped conversation the reader has not stopped on yet (see
    // [`OPEN_SETTLE`]).
    let mut pending_open: Option<PendingOpen> = None;

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
        let open_due = pending_open.as_ref().map(|open| open.due);

        // `paint` belongs to the *next* frame: every arm below assigns it — an
        // input or agent event paints, and the ticker asks the clock. Assign,
        // never OR into it: the value it holds on entry is the frame that was
        // just drawn, so `paint |= …` keeps a frame owed forever and repaints
        // every tick (measured: an animating rail went from two frames a second
        // to eight before this was an assignment).
        tokio::select! {
            Some(input) = input_rx.recv() => {
                let mut events = match &input {
                    TerminalEvent::Resize => vec![WbEvent::Resize(terminal.columns(), terminal.rows())],
                    TerminalEvent::Data(data) => events_of(data),
                };
                // Read the rest of the burst before doing anything with it. A
                // held key arrives as one terminal event per repeat, and every
                // repeat used to cost its own round trip, transcript load and
                // full repaint — which is what made a held `shift+↓` look stuck.
                // A batch is still handled in order; what changes is that the
                // session steps in it collapse into one switch, and the frame is
                // painted once at the end instead of once per keystroke.
                while events.len() < INPUT_BURST_LIMIT {
                    match input_rx.try_recv() {
                        Ok(TerminalEvent::Resize) => events.push(WbEvent::Resize(
                            terminal.columns(),
                            terminal.rows(),
                        )),
                        Ok(TerminalEvent::Data(data)) => events.extend(events_of(&data)),
                        Err(_) => break,
                    }
                }
                let mut session_steps = 0i32;
                for event in events {
                    if let WbEvent::Focus(focused) = event {
                        notifications.set_focused(focused);
                        continue;
                    }
                    // The rail's cursor moves now; the conversation it lands on
                    // is opened once the keys go quiet, below.
                    if let Some(delta) = session_step_of(workbench, &event) {
                        session_steps += delta;
                        continue;
                    }
                    // Anything else is the reader working *in* the conversation:
                    // land the step first, or `enter` would submit to the one
                    // they have already left. A step from this same burst counts
                    // — the cursor is already on it.
                    if let Some(open) = pending_open.take() {
                        show_conversation(&open.id, terminal, workbench, live, &frame, &mut clock)
                            .await?;
                    } else if let Some(id) =
                        resolve_step(&mut session_steps, workbench, live, &frame)
                    {
                        show_conversation(&id, terminal, workbench, live, &frame, &mut clock)
                            .await?;
                    }
                    if quit(workbench, live, event, &frame, &mut InputIo { terminal, settings, settings_path, theme_explicit }).await? {
                        return Ok(());
                    }
                }
                if let Some(id) = resolve_step(&mut session_steps, workbench, live, &frame) {
                    // A conversation the cache can paint goes up now — the
                    // reader asked for it and it costs nothing to show. One the
                    // client would have to fetch waits for the keys to go quiet
                    // instead: a held `shift+↓` used to open every conversation
                    // it passed through (see [`OPEN_SETTLE`]).
                    if live.is_cached(&id) {
                        pending_open = None;
                        show_conversation(&id, terminal, workbench, live, &frame, &mut clock)
                            .await?;
                    } else {
                        pending_open = Some(PendingOpen::stepped(id, Instant::now()));
                    }
                }
                paint = true;
            }
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(
                    open_due.unwrap_or_else(Instant::now),
                )), if open_due.is_some() => {
                if let Some(open) = pending_open.take() {
                    show_conversation(&open.id, terminal, workbench, live, &frame, &mut clock)
                        .await?;
                }
                paint = true;
            }
            Some(event) = agent_events.recv() => {
                let was = live.activity_kind();
                // State refresh below re-reads the authoritative queue before
                // this candidate can emit a notification.
                let mut completion = if live.take_completion_notification(&event) {
                    event.run_id.clone().unwrap_or_default()
                } else {
                    String::new()
                };
                if let Some(alert) = live.alert_for(&event) {
                    let id = alert_id(&event);
                    notify(terminal, notifications, &workbench.notify, &id, alert);
                }
                if live.take_failure_notification(&event) {
                    notify(terminal, notifications, &workbench.notify,
                        &format!("failed:{}", event.run_id.as_deref().unwrap_or("")),
                        crate::notifications::NotifyEvent::for_kind(
                            crate::notifications::NotifyKind::Failed));
                }
                live.handle_event(event);
                if live.approval().is_some() && workbench.overlay.is_none() {
                    workbench.overlay = Some(super::Overlay::Approval);
                    workbench.overlay_scroll = 0;
                }
                // What arrived while this event was being applied is applied
                // now, before the frame: a streaming turn emits deltas at tens
                // per second and a frame per token is work no one can see (the
                // screen only ever shows the last one).
                drain(agent_events, |event| {
                    if live.take_completion_notification(&event) {
                        completion = event.run_id.clone().unwrap_or_default();
                    }
                    if let Some(alert) = live.alert_for(&event) {
                        let id = alert_id(&event);
                        notify(terminal, notifications, &workbench.notify, &id, alert);
                    }
                    if live.take_failure_notification(&event) {
                        notify(terminal, notifications, &workbench.notify,
                            &format!("failed:{}", event.run_id.as_deref().unwrap_or("")),
                            crate::notifications::NotifyEvent::for_kind(
                                crate::notifications::NotifyKind::Failed));
                    }
                    live.handle_event(event);
                });
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
                if !completion.is_empty() && live.queue_is_empty() {
                    let event = live.completion_notification();
                    notify(terminal, notifications, &workbench.notify,
                        &format!("complete:{completion}"), event);
                }
                paint = true;
            }
            _ = ticker.tick() => {
                // The two spinners are the only animation a tick owns — the
                // activity row's, and the one a streaming conversation draws in
                // the rail. They turn on the same frame, so one counter drives
                // both. Nothing else in the frame is finer than a minute.
                // A short terminal lays out no activity row at all
                // (`compact`), and the rail can be closed, so each side only
                // animates while it is on screen to draw it.
                let activity_moving =
                    live.activity_animating() && frame.layout.activity.is_some();
                let rail_moving = live.sessions_streaming() && frame.layout.sidebar.is_some();
                let animating = workbench.motion_enabled() && (activity_moving || rail_moving);
                if animating {
                    workbench.advance_tick();
                }
                // A run in another conversation is only visible here through
                // `list_sessions`, and the agent tells no push channel about
                // it: the list is re-read on this clock or the row's spinner
                // would freeze and its 活跃 stretch never empty.
                let mut changed = false;
                if frame.layout.sidebar.is_some() && sessions_poll.elapsed() >= SESSIONS_POLL {
                    sessions_poll = Instant::now();
                    changed = live.refresh_sessions().await;
                }
                paint = clock.tick(
                    animating,
                    workbench.spinner_frame(),
                    ages_on_screen(&frame),
                    Instant::now(),
                ) || changed;
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

/// How often the rail's conversation list is re-read while it is on screen.
///
/// The list is the only view of *other* conversations this surface has: the
/// agent pushes events for the session attached here and nothing about the rest,
/// so a run another client started, and the moment it ends, reach the rail only
/// by asking (`list_sessions`). Without this clock a row's spinner would freeze
/// mid-turn and 最近's 活跃 stretch would never empty.
///
/// Five seconds is the trade the two uses agree on: a run that starts elsewhere
/// shows up in the time it takes to look at the rail, and an idle terminal pays
/// one cheap local call every five seconds — none of them a repaint (the poll
/// repaints only when the list it read differs from the one on screen).
const SESSIONS_POLL: Duration = Duration::from_secs(5);

/// How many already-queued agent events one iteration applies before painting.
///
/// The limit exists so a producer that outruns the surface for a long time
/// cannot keep the loop away from the keyboard and from painting; it is far
/// above a normal burst's length (a streamed token run between two frames is
/// hundreds of deltas at most, not thousands), so in practice a whole run is
/// coalesced into one frame and the cap is only ever reached when something has
/// gone wrong upstream.
const COALESCE_LIMIT: usize = 4096;

/// How many terminal events one batch may hold before the loop acts on them.
///
/// A held key arrives as one event per repeat, and each one used to cost its own
/// round trip, transcript load and repaint. Reading the burst first lets the
/// session steps in it collapse into one switch and the frame paint once. The
/// cap keeps a flood (a very fast paste, a repeating terminal report) from
/// starving the agent events that share this loop.
const INPUT_BURST_LIMIT: usize = 64;

/// How long a stepped conversation waits for the keys to go quiet.
///
/// A held `shift+↑/↓` arrives as one step per terminal repeat. Every repeat
/// used to open the conversation it landed on — a round trip, a transcript page
/// and a full repaint each, all replaced by the next repeat. Measured on a
/// 180×50 terminal: eight repeats wrote 178 KB in six frames and took 412 ms to
/// settle, so the transcript trailed the reader's fingers by a tenth of a second
/// and every conversation in between was loaded only to be thrown away.
///
/// The window sits between the two costs it trades: shorter than a repeat
/// interval, so a held key keeps deferring, and shorter than the pause a reader
/// notices, so letting go lands the conversation at once. A conversation the
/// cache can paint skips the wait entirely — there is nothing to fetch.
const OPEN_SETTLE: Duration = Duration::from_millis(80);

/// A conversation the rail's cursor has landed on whose rows are not on screen
/// yet, and when to open it.
struct PendingOpen {
    id: String,
    /// The last step's arrival plus [`OPEN_SETTLE`]: every repeat pushes it out.
    due: Instant,
}

impl PendingOpen {
    /// The reader stepped onto `id`: the window restarts from `now`, which is
    /// what makes a held key one open instead of one per repeat.
    fn stepped(id: String, now: Instant) -> Self {
        Self {
            id,
            due: now + OPEN_SETTLE,
        }
    }
}

/// The net step a *session* key asks for, if this event is one.
///
/// The driver reads this before the key reaches `quit`, so a held `shift+↑` /
/// `shift+↓` becomes one switch instead of one per repeat. The guards are the
/// same ones that decide whether any global shortcut fires: an overlay owns the
/// keys it paints.
fn session_step_of(workbench: &Workbench, event: &WbEvent) -> Option<i32> {
    let WbEvent::Key(key) = event else {
        return None;
    };
    if workbench.overlay.is_some() {
        return None;
    }
    match workbench.keys.action(key) {
        Some(super::preferences::Action::PrevSession) => Some(-1),
        Some(super::preferences::Action::NextSession) => Some(1),
        _ => None,
    }
}

/// The conversation a burst's accumulated steps land on, after moving the
/// surface's cursor that many rows.
///
/// `step_session` owns both halves — the rail's cursor, which is what the reader
/// watches move, and the conversation the cursor is now on. A step that finds
/// nowhere to go (either end of the list) answers `None`, and nothing opens.
fn resolve_step(
    steps: &mut i32,
    workbench: &mut Workbench,
    live: &mut Live,
    frame: &Frame,
) -> Option<String> {
    let wanted = std::mem::take(steps);
    if wanted == 0 {
        return None;
    }
    let data = live.data(frame);
    workbench
        .step_session(wanted, &data)
        .into_iter()
        .find_map(|action| match action {
            WbAction::SwitchSession(id) => Some(id),
            _ => None,
        })
}

/// Open one conversation, painting what the cache can show of it before the
/// round trip the agent still owes.
///
/// The mid-switch paint is the payoff of keeping cached rows: the reader sees
/// the conversation they landed on immediately, and the reconcile that follows
/// only replaces the rows when that conversation moved on while they were away
/// (see `Live::finish_switch`).
async fn show_conversation(
    id: &str,
    terminal: &mut Terminal,
    workbench: &mut Workbench,
    live: &mut Live,
    frame: &Frame,
    clock: &mut TickClock,
) -> Result<(), String> {
    if live.already_showing(id) {
        return Ok(());
    }
    if live.begin_switch(id) {
        draw(terminal, workbench, live, frame)?;
        clock.painted(workbench.spinner_frame(), Instant::now());
    }
    live.finish_switch(id).await;
    let data = live.data(frame);
    workbench.follow_current(&data);
    Ok(())
}

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

/// Whether the sidebar is showing the clock at all: both tabs print how fresh a
/// conversation is (`5m`, `3h`), so a rail on screen is a rail that ages.
fn ages_on_screen(frame: &Frame) -> bool {
    // The frame's own layout says whether a rail was drawn (a hidden or
    // too-narrow sidebar has none), so this asks what is on screen rather than
    // what the reader would like to be.
    frame.layout.sidebar.is_some()
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
    io: &mut InputIo<'_>,
) -> Result<bool, String> {
    let InputIo {
        terminal,
        settings,
        settings_path,
        theme_explicit,
    } = io;
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
            match workbench.keys.action(key) {
                Some(super::preferences::Action::Copy) => {
                    copy_last(live).await;
                    return Ok(false);
                }
                Some(super::preferences::Action::Mouse) => {
                    toggle_mouse(live, terminal, workbench, settings, settings_path);
                    return Ok(false);
                }
                Some(super::preferences::Action::Queue) => {
                    live.refresh_state().await;
                    let _ = workbench.surface_command("/queue", &live.data(frame));
                    return Ok(false);
                }
                Some(super::preferences::Action::ReduceMotion) => {
                    let _ = workbench.handle(WbEvent::Key(key.clone()), &live.data(frame));
                    persist_preferences(workbench, settings, settings_path);
                    return Ok(false);
                }
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
                    // A view toggle, not a preference: nothing is written back,
                    // so the next start opens folded again whatever the reader
                    // peeked at here (see `ReplyDetail`).
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
                    // The tree is a live view of a directory a build may have
                    // written to since it was read.
                    live.refresh_tree();
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
                // The repository browser is the surface's own panel, like
                // `/diff` and `/worktree`: the agent has no such verb, so the
                // draft's `/git` must not reach it. Reopening resumes the tab
                // and filter the reader left.
                "/git" => {
                    let (section, filter) = live
                        .git()
                        .map(|git| (git.section, git.filter.clone()))
                        .unwrap_or((GitSection::Commits, None));
                    open_git_panel(live, frame, workbench, section, filter, 0, true).await;
                    continue;
                }
                // The files panel: the conversation's own workspace, read afresh
                // — a run may have written into it since the tree was last
                // scanned, and a panel that opened on yesterday's listing would
                // be the one thing on screen that is not live.
                "/files" => {
                    live.refresh_tree();
                    workbench.overlay = Some(super::Overlay::Files);
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
                "/paste" => {
                    // The typed door to the clipboard, for terminals that take
                    // `ctrl+v` for themselves (Windows Terminal pastes with it,
                    // so the key never arrives here).
                    live.paste_clipboard().await;
                    continue;
                }
                // Refreshing the view has no chord any more: it is a
                // once-in-a-while verb, so the palette (and this line) is its
                // door. Same three reads the action does.
                "/refresh" => {
                    live.refresh_sessions().await;
                    live.refresh_state().await;
                    live.refresh_tree();
                    continue;
                }
                // Previewing the newest attachment has no chord (the surface's
                // chords are reserved for the actions pressed while working), so
                // this verb is its door.
                "/preview" => {
                    if live.load_attachment() {
                        workbench.reset_preview();
                        workbench.overlay = Some(super::Overlay::FilePreview);
                    }
                    continue;
                }
                "/mouse" => {
                    toggle_mouse(live, terminal, workbench, settings, settings_path);
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
            **theme_explicit = true;
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
            let mouse = workbench.preferences().mouse_capture;
            if live.mouse_capture() != mouse {
                live.set_mouse_capture(mouse);
                terminal.set_mouse_mode(mouse);
            }
            persist_preferences(workbench, settings, settings_path);
        }
        if matches!(action, WbAction::SetKeybindings) {
            let _ = workbench
                .keys
                .save(&super::preferences::keybindings_path(settings_path));
        }
        // A pinned workspace is a preference of this client's rail, so the
        // surface's pin lands in `settings.json` beside the layout — and the
        // rail's groups are rebuilt from it on the next frame.
        if let WbAction::SetWorkspacePinned { path, pinned } = &action {
            live.notice = Some(live.set_workspace_pinned(path, *pinned));
            settings.pinned_workspaces = Some(live.pinned_workspaces().to_vec());
            save_settings(settings_path, settings);
            continue;
        }
        // The preview box is the driver's to open: the adapter reads the bytes
        // and the surface paints them, but only this loop knows both.
        if let WbAction::PreviewFile(path) = &action {
            live.load_preview(path).await;
            workbench.reset_preview();
            // The pictures beside this one, so the box can step through the
            // folder with its own buttons — the reader opened a row, and the
            // rows around it are the neighbours they expect.
            workbench.set_preview_siblings(super::files::image_siblings(path));
            workbench.overlay = Some(super::Overlay::FilePreview);
            continue;
        }
        if matches!(action, WbAction::PreviewAttachment) {
            if live.load_attachment() {
                workbench.reset_preview();
                workbench.overlay = Some(super::Overlay::FilePreview);
            }
            continue;
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
        if matches!(action, WbAction::OpenFiles) {
            live.refresh_tree();
            workbench.overlay = Some(super::Overlay::Files);
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
        // `/git`: the panel's data is the driver's to read (the surface owns no
        // I/O), so every git verb lands here — the key, the palette entry, a tab
        // switch, a cursor move and `r`.
        if matches!(
            action,
            WbAction::OpenGit
                | WbAction::GitSection(_)
                | WbAction::GitFilter(_)
                | WbAction::GitRefresh
                | WbAction::SelectGit(_)
        ) {
            if let WbAction::SelectGit(index) = action {
                open_git_detail(live, frame, workbench, index).await;
                continue;
            }
            let current = live.git().cloned();
            let (section, filter, want, reset_scroll) = match &action {
                WbAction::OpenGit => (
                    current
                        .as_ref()
                        .map(|git| git.section)
                        .unwrap_or(GitSection::Commits),
                    current.as_ref().and_then(|git| git.filter.clone()),
                    0,
                    true,
                ),
                WbAction::GitSection(section) => (
                    *section,
                    current.as_ref().and_then(|git| git.filter.clone()),
                    0,
                    true,
                ),
                WbAction::GitFilter(filter) => (GitSection::Commits, filter.clone(), 0, true),
                WbAction::GitRefresh => (
                    current
                        .as_ref()
                        .map(|git| git.section)
                        .unwrap_or(GitSection::Commits),
                    current.as_ref().and_then(|git| git.filter.clone()),
                    workbench.workflow_index,
                    false,
                ),
                _ => (GitSection::Commits, None, 0, true),
            };
            open_git_panel(live, frame, workbench, section, filter, want, reset_scroll).await;
            continue;
        }
        if let WbAction::CopyText(text) = &action {
            let text = text.clone();
            let show = text.clone();
            let copied =
                tokio::task::spawn_blocking(move || crate::clipboard::copy_text(&text)).await;
            live.notice = Some(match copied {
                Ok(Ok(())) => {
                    super::i18n::render(live.language(), super::i18n::Key::GitCopied, &[&show])
                }
                Ok(Err(error)) => {
                    super::i18n::render(live.language(), super::i18n::Key::CopyFailed, &[&error])
                }
                Err(error) => super::i18n::render(
                    live.language(),
                    super::i18n::Key::CopyFailed,
                    &[&error.to_string()],
                ),
            });
            continue;
        }
        if matches!(action, WbAction::CopyLast) {
            copy_last(live).await;
            continue;
        }
        if matches!(action, WbAction::ToggleMouse) {
            toggle_mouse(live, terminal, workbench, settings, settings_path);
            continue;
        }
        if let WbAction::Notice(notice) = &action {
            live.notice = Some(notice.clone());
            continue;
        }
        // The draft is *not* cleared here: `submit` reads the prompt, its folded
        // pastes and its attachments out of it, and clears it itself once the
        // prompt is actually on its way. Clearing first here made every submit
        // read an empty draft and return without reaching the agent, and it also
        // threw away the attachments — and when the session cannot be started
        // the reader's text has to survive so it can be sent again.
        //
        // `SessionStep` is the rail's cursor moving between conversations: the
        // surface deferred the switch to us, so it is resolved here against the
        // data of *this* moment (the driver hands a whole burst of them over as
        // one net step, see `event_loop`).
        let action = match action {
            WbAction::SessionStep(delta) => {
                let data = live.data(frame);
                match workbench.step_session(delta, &data).into_iter().next() {
                    Some(next) => next,
                    None => continue,
                }
            }
            other => other,
        };
        // A delete moves the transcript too: the row that took the deleted
        // conversation's place is what the screen is now showing.
        let moved = matches!(
            action,
            WbAction::SwitchSession(_) | WbAction::DeleteSession(_)
        );
        match live.apply(action).await {
            Outcome::Quit => return Ok(true),
            Outcome::Continue => {}
        }
        // A switch the rail did not make itself (a click on the launchpad's
        // recents, a `ctrl+r` pick) still has to move its cursor: the rail would
        // otherwise highlight a conversation the transcript is not showing.
        if moved {
            let data = live.data(frame);
            workbench.follow_current(&data);
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
    match data {
        "\x1b[I" => return vec![WbEvent::Focus(true)],
        "\x1b[O" => return vec![WbEvent::Focus(false)],
        _ => {}
    }
    if let Some(event) = mouse::parse(data) {
        return vec![WbEvent::Mouse(event)];
    }
    if mouse::is_partial(data) {
        return Vec::new();
    }
    if let Some(key) = crate::keys::parse_key(data) {
        // A *release* is not a press. The kitty protocol is enabled with the
        // `report event types` flag (`\x1b[>7u`), so every key this surface is
        // handed arrives twice — `\x1b[104u` on the way down and `\x1b[104;1:3u`
        // on the way up — and both parse to the same key id. Handling the
        // second one typed every character twice (`hi` arrived as `hhii`) and
        // moved every list two rows per press. The repeat event (`:2`, a held
        // key) is a press and stays.
        if crate::keys::is_key_release(data) {
            return Vec::new();
        }
        return vec![WbEvent::Key(key)];
    }
    // A chunk the parser does not recognise as one key is one of two things:
    // several control chords arriving in the same read (a held key, a fast
    // `ctrl+y ctrl+y`), or text. The parser folds a trailing control byte into a
    // chord even when text precedes it — `b\x13` parses as `ctrl+s` — so the
    // split is only made for a chunk that is *entirely* control bytes. That keeps
    // both halves working: two chords become two keys, and a run of text is never
    // re-split into characters the parser would misread.
    let control_only = data
        .chars()
        .all(|ch| (ch as u32) < 0x20 || ch as u32 == 0x7f);
    if control_only {
        let keys: Vec<WbEvent> = data
            .chars()
            .filter_map(|ch| crate::keys::parse_key(&ch.to_string()).map(WbEvent::Key))
            .collect();
        if keys.len() == data.chars().count() {
            return keys;
        }
    }
    // The same read can hold several *escape* sequences — holding `↓` in a list,
    // or an arrow arriving in the same read as a chord. Each one is a complete
    // key on its own; the chunk is not, and pasting it would put `\x1b[B` in the
    // draft. Split only when every segment parses, so an unfinished sequence (a
    // truncated `\x1b[1;5A`, a lone trailing `\x1b`) stays whole for the next
    // read rather than being guessed at.
    if let Some(keys) = split_escape_keys(data) {
        return keys;
    }
    // Printable text that is not a single ASCII character: a CJK character an
    // IME hands over (`中` is one `str` of three bytes, which `parse_key` only
    // reads when it is one byte), or several characters arriving in one read.
    // Each becomes a key of its own, which is exactly what latin typing
    // produces — so the picker's incremental search can be typed in Chinese and
    // a filter line sees the same character-by-character input either way.
    if !data.contains('\x1b') && !data.chars().any(char::is_control) {
        let keys: Vec<WbEvent> = data
            .chars()
            .map(|ch| {
                // `parse_key` still owns the spelling for the characters it
                // knows (` ` is `space`, `#` is `#`); the rest is itself.
                let id = crate::keys::parse_key(&ch.to_string()).unwrap_or_else(|| ch.to_string());
                WbEvent::Key(id)
            })
            .collect();
        if !keys.is_empty() {
            return keys;
        }
    }
    // Anything else is text, and belongs in the draft as it stands.
    vec![WbEvent::Paste(data.to_string())]
}

/// Split a chunk of several complete escape sequences into one key each.
///
/// `None` when the chunk is not a run of complete sequences: the caller then
/// treats it as text, which is the honest answer for anything this does not
/// recognise. A trailing bare `\x1b` is the start of a sequence whose rest has
/// not arrived, so it is not split either — a guess there would turn half of one
/// key into an `escape`.
fn split_escape_keys(data: &str) -> Option<Vec<WbEvent>> {
    if !data.starts_with('\x1b') || data.len() < 2 {
        return None;
    }
    let mut keys = Vec::new();
    let mut segments = 0usize;
    let mut rest = data;
    while !rest.is_empty() {
        let end = rest[1..]
            .find('\x1b')
            .map(|at| at + 1)
            .unwrap_or(rest.len());
        let segment = &rest[..end];
        if segment == "\x1b" {
            return None;
        }
        let key = crate::keys::parse_key(segment)?;
        segments += 1;
        // One sequence per key, and a release is not a key (see `key_events`).
        if !crate::keys::is_key_release(segment) {
            keys.push(WbEvent::Key(key));
        }
        rest = &rest[end..];
    }
    // Several complete sequences: the chunk is a run of keys, and it is
    // answered as such even when only a release was in it (which answers
    // *nothing*, not the raw escape bytes as text).
    (segments > 1).then_some(keys)
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
        workbench.sidebar_position,
    )
}

/// Read the `/git` panel for the session's workspace and open it.
///
/// The surface owns no I/O, so the read happens here — one section plus the
/// selected row's detail, on the blocking pool because a large repository on a
/// slow disk must not stall the frame that asked for it.
async fn open_git_panel(
    live: &mut Live,
    frame: &Frame,
    workbench: &mut Workbench,
    section: GitSection,
    filter: Option<String>,
    want: usize,
    reset_scroll: bool,
) {
    let cwd = live.data(frame).session.cwd;
    // No workspace (the launchpad) has no repository to read: the panel says so
    // with its own copy rather than showing git's "cannot change to ''".
    if cwd.trim().is_empty() {
        live.set_git(None);
        if reset_scroll {
            workbench.overlay_scroll = 0;
        }
        workbench.overlay = Some(super::Overlay::Git);
        return;
    }
    let lang = live.language();
    let loaded = tokio::task::spawn_blocking(move || {
        super::git_panel::read(std::path::Path::new(&cwd), section, filter, want, lang)
    })
    .await;
    match loaded {
        Ok(view) => {
            // The surface's cursor follows the view's selection: a section with
            // a graph-only first line must not open with the arrow on a row that
            // cannot be entered.
            workbench.workflow_index = view.selected;
            live.set_git(Some(view));
        }
        Err(error) => live.notice = Some(error.to_string()),
    }
    if reset_scroll {
        workbench.overlay_scroll = 0;
    }
    workbench.overlay = Some(super::Overlay::Git);
}

/// Load one row's detail for the `/git` panel. Only the detail is re-read: the
/// rows on screen are already the ones the reader is looking at.
async fn open_git_detail(live: &mut Live, frame: &Frame, workbench: &mut Workbench, index: usize) {
    let cwd = live.data(frame).session.cwd;
    let Some(mut git) = live.git().cloned() else {
        workbench.overlay = Some(super::Overlay::Git);
        return;
    };
    let Some(key) = git.rows.get(index).and_then(|row| row.key.clone()) else {
        workbench.overlay = Some(super::Overlay::Git);
        return;
    };
    let section = git.section;
    let lang = live.language();
    let loaded = tokio::task::spawn_blocking(move || {
        super::git_panel::detail(std::path::Path::new(&cwd), section, &key, lang)
    })
    .await;
    git.selected = index;
    match loaded {
        Ok(Ok(lines)) => git.detail = lines,
        Ok(Err(error)) => git.detail = vec![error],
        Err(error) => git.detail = vec![error.to_string()],
    }
    live.set_git(Some(git));
    workbench.overlay = Some(super::Overlay::Git);
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

fn toggle_mouse(
    live: &mut Live,
    terminal: &mut Terminal,
    workbench: &mut Workbench,
    settings: &mut TuiSettings,
    settings_path: &Path,
) {
    let enabled = live.toggle_mouse();
    workbench.mouse_capture = enabled;
    terminal.set_mouse_mode(enabled);
    settings.mouse_capture = Some(enabled);
    save_settings(settings_path, settings);
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

fn alert_id(event: &AgentEvent) -> String {
    let identity = if event.r#type == "approval_request" {
        event
            .data
            .get("approval_request_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
    } else {
        event.event_id.as_deref().unwrap_or("")
    };
    format!("{}:{identity}", event.r#type)
}

fn notify(
    terminal: &Terminal,
    center: &mut crate::notifications::NotifyCenter,
    config: &crate::notifications::NotifyConfig,
    id: &str,
    event: crate::notifications::NotifyEvent,
) {
    for sequence in center.push(id, event, config, terminal.is_tty()) {
        terminal.write(&sequence);
    }
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
    paint_image(terminal, workbench, &data);
    Ok(())
}

/// Paint (or remove) the picture the frame asked for.
///
/// A terminal bitmap is not a row of text: it is placed at a cell by the
/// terminal itself, above whatever the frame wrote there. So the preview box
/// only *describes* what it wants ([`screen::ImageRequest`]) and this — the one
/// place holding the terminal — writes it after the frame it belongs to,
/// leaving the hardware cursor exactly where the frame put it.
///
/// The pixels are transmitted once per source (the file, and the crop of it a
/// zoomed box shows); every later frame only re-places the image, which is a
/// hundred bytes instead of a megabyte. Re-placing every frame is what keeps a
/// picture on screen: the frame under it is rewritten each paint, and a placed
/// image is not part of that text.
fn paint_image(terminal: &mut Terminal, workbench: &mut Workbench, data: &super::WorkbenchData) {
    let request = workbench.image_request;
    let Some(request) = request else {
        if let Some(painted) = workbench.painted_image.take() {
            terminal.write(&crate::terminal_image::delete_kitty_image(painted.id));
        }
        return;
    };
    // The pixels are the box's own preview: the same image the frame drew.
    let Some(super::state::PreviewView::Image { pixels, .. }) = data.preview.as_ref() else {
        return;
    };
    let mut out = String::new();
    match workbench.painted_image {
        Some(painted) if painted.source == request.source => {
            // Same bytes, new place: `a=p` places an already-transmitted image
            // by id, and `p=1` keeps the placement one thing (a new `a=p` each
            // frame would otherwise stack copies).
            out.push_str(&format!(
                "\x1b[{};{}H\x1b_Ga=p,i={},p=1,c={},r={},q=2,C=1\x1b\\",
                request.y + 1,
                request.x + 1,
                painted.id,
                request.columns,
                request.rows
            ));
        }
        other => {
            if let Some(painted) = other {
                out.push_str(&crate::terminal_image::delete_kitty_image(painted.id));
            }
            let id = crate::terminal_image::allocate_image_id();
            // Transmit what the cells can show, not the source: the decoder
            // keeps enough pixels for a zoomed window to stay sharp, but a
            // whole 2048px picture across a hundred cells is fifteen megabytes
            // over the pty that the terminal would immediately throw away.
            let (rgb, w, h) = transmit_bitmap(
                pixels,
                request.crop,
                request.columns as usize,
                request.rows as usize,
            );
            out.push_str(&format!("\x1b[{};{}H", request.y + 1, request.x + 1));
            out.push_str(&crate::terminal_image::encode_kitty_rgb(
                &rgb,
                w,
                h,
                request.columns as usize,
                request.rows as usize,
                id,
            ));
            workbench.painted_image = Some(super::PaintedImage {
                source: request.source,
                id,
            });
        }
    }
    // Put the cursor back where the frame left it; the image itself does not
    // move it (`C=1`), but the addressing above did.
    if let Some((x, y)) = workbench.cursor() {
        out.push_str(&format!("\x1b[{};{}H", y + 1, x + 1));
    }
    terminal.write(&out);
}

/// How many source pixels per placed cell are worth transmitting.
///
/// A terminal cell is roughly 8–10 physical pixels wide on a HiDPI screen, and
/// the terminal scales whatever it is given down to the cell box. Ten pixels a
/// cell is therefore "at or above what the display can show" without carrying a
/// full-resolution photograph across the pty.
const TRANSMIT_PX_PER_CELL: usize = 10;

/// The bitmap to transmit for `crop`, placed over `columns` × `rows` cells.
///
/// Returns RGB bytes and the bitmap's own size. A crop no wider than the cells
/// can show is transmitted as it is — that is the zoomed case, where the window
/// is a few hundred pixels and sharpness is the whole point. A larger crop (the
/// picture fitted into the box) is downsampled first: the bytes cross a pipe and
/// are then scaled again by the terminal, so sending a 2048px source for a
/// 100-cell box is latency with nothing to show for it.
fn transmit_bitmap(
    pixels: &super::image_view::ImagePixels,
    crop: (usize, usize, usize, usize),
    columns: usize,
    rows: usize,
) -> (Vec<u8>, usize, usize) {
    let crop_w = crop.2.saturating_sub(crop.0).max(1);
    let crop_h = crop.3.saturating_sub(crop.1).max(1);
    let budget = (columns.max(1) * TRANSMIT_PX_PER_CELL).min(pixels.width.max(1));
    if crop_w <= budget || rows == 0 || columns == 0 {
        return (crop_rgb(pixels, crop), crop_w, crop_h);
    }
    let target_w = budget.max(1);
    let target_h = ((crop_h as f64) * (target_w as f64) / (crop_w as f64))
        .round()
        .max(1.0) as usize;
    let mut window = image::RgbaImage::new(crop_w as u32, crop_h as u32);
    for y in 0..crop_h {
        for x in 0..crop_w {
            let offset = ((crop.1 + y) * pixels.width + crop.0 + x) * 4;
            let pixel = pixels
                .rgba
                .get(offset..offset + 4)
                .map(|rgba| [rgba[0], rgba[1], rgba[2], rgba[3]])
                .unwrap_or([0, 0, 0, 0]);
            window.put_pixel(x as u32, y as u32, image::Rgba(pixel));
        }
    }
    let scaled = image::imageops::resize(
        &window,
        target_w as u32,
        target_h as u32,
        image::imageops::FilterType::Triangle,
    );
    let mut rgb = Vec::with_capacity(target_w * target_h * 3);
    for pixel in scaled.pixels() {
        rgb.extend_from_slice(&pixel.0[..3]);
    }
    (rgb, target_w, target_h)
}

/// The RGB bytes of a crop of `pixels`, row-major — what kitty's `f=24` wants.
fn crop_rgb(
    pixels: &super::image_view::ImagePixels,
    crop: (usize, usize, usize, usize),
) -> Vec<u8> {
    let (x0, y0, x1, y1) = crop;
    let mut out = Vec::with_capacity((x1 - x0) * (y1 - y0) * 3);
    for y in y0..y1 {
        for x in x0..x1 {
            let offset = (y * pixels.width + x) * 4;
            match pixels.rgba.get(offset..offset + 3) {
                Some(rgb) => out.extend_from_slice(rgb),
                None => out.extend_from_slice(&[0, 0, 0]),
            }
        }
    }
    out
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
    settings.sidebar_position = Some(preferences.sidebar_position.as_wire().into());
    settings.sidebar_width = Some(preferences.sidebar_width);
    settings.sidebar_visible = Some(preferences.sidebar_visible);
    settings.mouse_capture = Some(preferences.mouse_capture);
    settings.notify = Some(preferences.notify);
    settings.reduced_motion = Some(preferences.reduced_motion);
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
    fn focus_reports_are_not_inserted_into_the_draft() {
        assert_eq!(key_events("\x1b[I"), vec![WbEvent::Focus(true)]);
        assert_eq!(key_events("\x1b[O"), vec![WbEvent::Focus(false)]);
    }

    #[test]
    fn a_plain_key_becomes_one_key_event() {
        assert_eq!(events_of("a"), vec![WbEvent::Key("a".into())]);
        assert_eq!(events_of("\u{3}"), vec![WbEvent::Key("ctrl+c".into())]);
    }
    /// The RGB bytes of a crop, row-major: what `f=24` transmits, and what a
    /// zoomed, panned picture shows.
    #[test]
    fn a_crop_is_transmitted_row_major_and_inside_the_picture() {
        use super::super::image_view::ImagePixels;
        // 3x2: each pixel's red channel is its index.
        let mut rgba = Vec::new();
        for index in 0..6u8 {
            rgba.extend_from_slice(&[index, 0, 0, 255]);
        }
        let pixels = ImagePixels {
            width: 3,
            height: 2,
            rgba,
        };
        // The middle column only.
        let rgb = crop_rgb(&pixels, (1, 0, 2, 2));
        assert_eq!(rgb, vec![1, 0, 0, 4, 0, 0]);
        // A crop that asks past the edge is clamped by the caller's own maths;
        // a missing pixel is black rather than a panic.
        let rgb = crop_rgb(&pixels, (2, 1, 4, 3));
        assert_eq!(rgb, vec![5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    /// A picture fitted into the box is transmitted at the cells' resolution,
    /// not the source's; a zoomed window, which is already small, is not
    /// resampled at all.
    #[test]
    fn the_transmitted_bitmap_is_sized_to_the_cells() {
        let mut rgba = Vec::new();
        for y in 0..400 {
            for x in 0..400 {
                rgba.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 0, 255]);
            }
        }
        let pixels = super::super::image_view::ImagePixels {
            width: 400,
            height: 400,
            rgba,
        };
        // Fitted into 20 cells at ten pixels a cell: 200 px, half the source.
        let (rgb, w, h) = transmit_bitmap(&pixels, (0, 0, 400, 400), 20, 20);
        assert_eq!((w, h), (200, 200));
        assert_eq!(rgb.len(), w * h * 3);
        // A zoomed window already fits the budget: no resample, the bytes are
        // the crop itself.
        let (rgb, w, h) = transmit_bitmap(&pixels, (10, 10, 210, 210), 40, 40);
        assert_eq!((w, h), (200, 200));
        assert_eq!(rgb.len(), 200 * 200 * 3);
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

    /// A CJK character is *typed*, not pasted: one key event, exactly like
    /// `a`, so the pickers' incremental search can be given Chinese and the
    /// draft does not treat an IME's character as an attachment-sized paste.
    #[test]
    fn a_multibyte_character_is_one_typed_key() {
        assert_eq!(events_of("中"), vec![WbEvent::Key("中".into())]);
        assert_eq!(
            events_of("你好"),
            vec![WbEvent::Key("你".into()), WbEvent::Key("好".into())]
        );
    }

    /// The kitty protocol is enabled with `report event types`, so each key
    /// arrives twice: once as a press and once as a release carrying `:3`. The
    /// release must not be handled — it used to type every character twice and
    /// move every list two rows per press. A repeat (`:2`, a held key) is a
    /// press and still arrives.
    #[test]
    fn a_key_release_is_never_handled_as_a_press() {
        // Press then release of `h`, in the order the terminal sends them.
        assert_eq!(events_of("\x1b[104u"), vec![WbEvent::Key("h".into())]);
        assert_eq!(events_of("\x1b[104;1:3u"), Vec::new());
        // shift+up: `\x1b[1;2A` down, `\x1b[1;2:3A` up.
        assert_eq!(
            events_of("\x1b[1;2A"),
            vec![WbEvent::Key("shift+up".into())]
        );
        assert_eq!(events_of("\x1b[1;2:3A"), Vec::new());
        // A held key repeats (`:2`) — that is still a press.
        assert_eq!(events_of("\x1b[104;1:2u"), vec![WbEvent::Key("h".into())]);
        // Two events in one read: the press survives, the release does not.
        assert_eq!(
            events_of("\x1b[104u\x1b[104;1:3u"),
            vec![WbEvent::Key("h".into())]
        );
    }

    /// Two control chords in one read are two events, not one paste: the raw
    /// bytes must never reach the draft, and neither chord may be swallowed.
    #[test]
    fn a_chunk_of_several_keys_is_split_into_events() {
        let events = events_of("\u{19}\u{19}");
        assert_eq!(events, vec![WbEvent::Key("ctrl+y".into()); 2], "{events:?}");
        let arrows = events_of("\x1b[B\x1b[B");
        assert_eq!(arrows, vec![WbEvent::Key("down".into()); 2], "{arrows:?}");
    }

    /// A run of real text stays one paste even when a control byte follows it:
    /// the parser folds a trailing byte into a chord (`b\x13` reads as
    /// `ctrl+s`), so re-splitting text would corrupt it.
    #[test]
    fn a_text_run_is_never_split_into_chords() {
        let events = events_of("ab\u{13}");
        assert_eq!(
            events,
            vec![WbEvent::Paste("ab\u{13}".into())],
            "{events:?}"
        );
        let mixed = events_of("ab\tcd");
        assert_eq!(
            mixed,
            vec![WbEvent::Paste("ab\tcd".into())],
            "a tab between letters is text the draft keeps: {mixed:?}"
        );
    }

    /// An unfinished sequence is not guessed at: half an escape sequence must
    /// wait for the rest of its bytes rather than becoming an `escape` key now.
    #[test]
    fn an_incomplete_sequence_is_not_split() {
        // A lone trailing `\x1b` after a complete sequence: the next read
        // finishes it, so this one stays whole.
        assert_eq!(
            events_of("\x1b[B\x1b"),
            vec![WbEvent::Paste("\x1b[B\x1b".into())]
        );
        // A truncated CSI likewise: it parses as nothing, so it is not a key.
        assert_eq!(
            events_of("\x1b[1;5"),
            vec![WbEvent::Paste("\x1b[1;5".into())]
        );
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

    /// A first start — no `settings.json`, or one whose `sidebarVisible` an
    /// older build never wrote — opens on the conversation, not on the rail.
    /// The choice is a preference once it is made: whoever hid or showed the
    /// rail keeps exactly that on the next start.
    #[test]
    fn a_first_start_hides_the_rail_and_a_choice_survives() {
        let fresh = layout_preferences(&TuiSettings::default(), true);
        assert!(!fresh.sidebar_visible, "a fresh start hides the rail");
        // A file with other keys but no `sidebarVisible` is the same start.
        let older = TuiSettings::from_json(&serde_json::json!({"mouseCapture": false}));
        assert!(!layout_preferences(&older, true).sidebar_visible);

        let shown = TuiSettings::from_json(&serde_json::json!({"sidebarVisible": true}));
        assert!(layout_preferences(&shown, true).sidebar_visible);
        let hidden = TuiSettings::from_json(&serde_json::json!({"sidebarVisible": false}));
        assert!(!layout_preferences(&hidden, true).sidebar_visible);
    }

    /// A held `shift+↑/↓` opens one conversation, the one the reader stops on.
    ///
    /// Each repeat restarts the settle window, so nothing opens while the key
    /// keeps producing steps — the conversations passed through are never
    /// loaded at all (measured before this: eight repeats wrote 178 KB in six
    /// frames and took 412 ms to settle, each frame a conversation that was
    /// replaced 30 ms later). The two constants have to bracket a repeat
    /// interval and stay under the pause a reader notices.
    #[test]
    fn a_held_session_key_opens_the_conversation_it_stops_on() {
        let start = Instant::now();
        let mut open = PendingOpen::stepped("s-1".into(), start);
        assert_eq!(open.due, start + OPEN_SETTLE);

        // Repeats at the fastest rate a terminal uses (~30/s): every one pushes
        // the deadline past the next, so it never expires under the key.
        let mut held = start;
        for step in 1..=30u32 {
            assert!(held < open.due, "the window expired under a held key");
            held = start + Duration::from_millis(33) * step;
            open = PendingOpen::stepped(format!("s-{step}"), held);
        }
        assert_eq!(open.id, "s-30", "the last step is the one that opens");
        // Still pending at the moment of that last repeat…
        assert!(held < open.due);
        // …and due one settle after it.
        assert_eq!(open.due, held + OPEN_SETTLE);
    }

    /// The settle window's own bounds, so a future tweak cannot quietly break
    /// the property the test above relies on.
    #[test]
    fn the_settle_window_outlasts_a_repeat_and_beats_a_noticeable_pause() {
        // Slower than a repeat interval (30–40 ms), or a held key would still
        // open every conversation it passed through.
        assert!(OPEN_SETTLE > Duration::from_millis(40));
        // …and under the ~100 ms at which a pause reads as a delay.
        assert!(OPEN_SETTLE <= Duration::from_millis(100));
    }
}
