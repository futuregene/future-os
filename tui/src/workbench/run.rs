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
    /// An exit signal (SIGINT / SIGTERM / SIGHUP / SIGQUIT, see
    /// `Terminal::set_exit_signal_callback`). It is its own event and not an
    /// injected `ctrl+c` byte: that byte is the run's interrupt chord, so a
    /// signal routed through the keymap would stop a turn and leave the process
    /// running.
    Signal,
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
    update_rx: &'a mut Option<tokio::sync::oneshot::Receiver<Option<String>>>,
    /// The answers a read started by the surface comes back on (see
    /// [`show_conversation`] and `workbench::live::Loaded`): handed to `Live` so
    /// no door waits for the agent or for `git`, and drained by the loop that
    /// owns the surface.
    load_rx: tokio::sync::mpsc::UnboundedReceiver<super::live::Loaded>,
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
    }
}

/// Run the workbench until the reader quits. Returns an error only for a
/// failure that makes the surface impossible (no TTY, no agent).
pub async fn run(options: Options) -> Result<(), String> {
    let (mut settings, settings_warning) = load_settings_report(&options.settings_path);
    let mut theme_explicit = settings.theme_id.is_some();
    let (mut live, mut agent_events) = Live::attach(
        &options.addr,
        options.session.as_deref(),
        options.no_context_files,
        Some(&settings),
    )
    .await?;
    let mut workbench = Workbench::new();
    if let Some(error) = settings_warning {
        live.notice = Some(super::i18n::render(
            live.language(),
            super::i18n::Key::NoticeSettingsLoadFailed,
            &[&error],
        ));
    }
    let mouse_capture = settings.mouse_capture.unwrap_or(true);
    live.set_mouse_capture(mouse_capture);
    workbench.set_theme(&live.theme_id);
    workbench.set_language(live::persisted_language(&settings));
    workbench.set_preferences(layout_preferences(&settings, mouse_capture));
    // The folds are the rail's own memory, not the layout's, so they are applied
    // beside it: a rail the reader organised comes back organised.
    workbench.set_folded_workspaces(settings.folded_workspaces.clone().unwrap_or_default());
    workbench.keys = super::preferences::Keybindings::load(&super::preferences::keybindings_path(
        &options.settings_path,
    ));

    if options.continue_recent && options.session.is_none() {
        live.attach_most_recent().await;
    }
    // `cd project && future tui` starts that project's conversation: the
    // launchpad stages the working directory, not the app's scratch tree. A
    // launch from somewhere ambient (the home directory, a temp directory)
    // stages the workspace chosen last time instead (see `launch_workspace`).
    live.stage_launch_workspace(settings.last_workspace.as_deref());
    // …and whatever that staging came to is what the next start should reopen,
    // so a first launch from an ambient directory is the last one that lands
    // there by accident.
    persist_pending(&mut live, &mut settings, &options.settings_path);

    if let Some(prompt) = options
        .initial_prompt
        .filter(|text| !text.trim().is_empty())
    {
        live.apply(WbAction::Submit(prompt)).await;
    }

    // The startup update notice runs beside the surface. Waiting for the
    // network before the first frame made a slow or unreachable manifest look
    // like a terminal that had not started at all.
    let update_rx = if options.offline {
        None
    } else {
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = tx.send(crate::update::check(crate::version::VERSION).await);
        });
        Some(rx)
    };
    let mut update_rx = update_rx;

    let mut terminal = Terminal::new().map_err(|error| format!("terminal: {error}"))?;
    // The workbench is clickable, so it owns mouse reporting for as long as the
    // alternate screen is up: `start` writes the mode in, `stop` takes it out —
    // and both failures' paths (the exit failsafe and the crash hook) reset it
    // too, so the shell never inherits a captured mouse.
    terminal.set_mouse_mode(mouse_capture);

    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<TerminalEvent>();
    // The reads the surface starts answer here rather than inside the loop (see
    // `show_conversation` and `workbench::live::Loaded`): the screen paints what
    // it has and the answers arrive on their own wake-up.
    let (load_tx, load_rx) = mpsc::unbounded_channel::<super::live::Loaded>();
    live.set_load_sender(load_tx);
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
    // SIGINT/SIGTERM take the quit path so the terminal is restored by the
    // loop's own exit rather than by the signal handler's re-raise.
    {
        let signal_tx = input_tx.clone();
        terminal.set_exit_signal_callback(Some(Box::new(move || {
            let _ = signal_tx.send(TerminalEvent::Signal);
        })));
    }
    let mut notifications = crate::notifications::NotifyCenter::default();
    let outcome = event_loop(
        &mut DriverIo {
            terminal: &mut terminal,
            notifications: &mut notifications,
            theme_explicit: &mut theme_explicit,
            update_rx: &mut update_rx,
            load_rx,
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
    // The reading level is deliberately not written: `/reply-detail` expands
    // the transcript for this run only, so the next terminal opens folded.
    let preferences_saved = persist_preferences(&workbench, &mut settings, &options.settings_path);
    let keys_saved = workbench
        .keys
        .save(&super::preferences::keybindings_path(
            &options.settings_path,
        ))
        .map_err(|error| format!("save keybindings: {error}")); // i18n: process exit diagnostic, never drawn

    match (outcome, preferences_saved.and(keys_saved)) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
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
        update_rx,
        load_rx,
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
    // When the last step landed, so the next one can tell a tap from a repeat
    // (see [`STEP_TAP_GAP`]).
    let mut last_step: Option<Instant> = None;
    let mut painted = FrameCache::default();

    loop {
        // The frame's own metrics, computed once: the surface composes for
        // them, and the event path below hands the transcript the *same*
        // width. A second, different width (the old loop used `1` to read one
        // field) makes `ChatArea` discard its rendered rows and re-wrap the
        // whole transcript on every event.
        let frame = frame_of(terminal, workbench);
        if paint {
            draw(terminal, workbench, live, &frame, &mut painted)?;
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
                    // A signal leaves now, whatever an overlay is showing: the
                    // chord cannot be hard-coded (`ctrl+c` stops a run) and the
                    // reader may have rebound quit to something a fake byte
                    // could never reproduce.
                    TerminalEvent::Signal => return Ok(()),
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
                        Ok(TerminalEvent::Signal) => return Ok(()),
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
                        show_conversation(&open.id, terminal, workbench, live, &frame, &mut clock, &mut painted)?;
                    } else if let Some(id) =
                        resolve_step(&mut session_steps, workbench, live, &frame)
                    {
                        show_conversation(&id, terminal, workbench, live, &frame, &mut clock, &mut painted)?;
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
                    // it passed through (see [`OPEN_SETTLE`]). A tap is not a
                    // hold, and it does not pay that window: the read starts now
                    // and the window becomes time the agent has already spent.
                    let now = Instant::now();
                    let tapped = last_step.is_none_or(|last| now.duration_since(last) >= STEP_TAP_GAP);
                    last_step = Some(now);
                    if live.is_cached(&id) {
                        pending_open = None;
                        show_conversation(&id, terminal, workbench, live, &frame, &mut clock, &mut painted)?;
                    } else if tapped {
                        pending_open = Some(PendingOpen::now(id));
                    } else {
                        pending_open = Some(PendingOpen::stepped(id, now));
                    }
                }
                paint = true;
            }
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(
                    open_due.unwrap_or_else(Instant::now),
                )), if open_due.is_some() => {
                if let Some(open) = pending_open.take() {
                    show_conversation(&open.id, terminal, workbench, live, &frame, &mut clock, &mut painted)?;
                }
                paint = true;
            }
            Some(loaded) = load_rx.recv() => {
                // A read the surface started without waiting for it, installed
                // here so the client's own state and the frame that paints it
                // move together, and dropped when the reader has moved on (each
                // arm decides that: see `Live`'s install methods).
                match loaded {
                    super::live::Loaded::Switch(switch) => {
                        if live.install_switch_load(switch) {
                            let data = live.data(&frame);
                            workbench.follow_current(&data);
                            paint = true;
                        }
                    }
                    super::live::Loaded::Older(page) => {
                        if live.install_older(page) {
                            paint = true;
                        }
                    }
                    super::live::Loaded::State(state) => {
                        if live.install_state_refresh(state) {
                            if live.approval().is_some() && workbench.overlay.is_none() {
                                workbench.overlay = Some(super::Overlay::Approval);
                                workbench.overlay_scroll = 0;
                            }
                            paint = true;
                        }
                    }
                    super::live::Loaded::Git { rev, view } => {
                        if live.install_git(rev, view) {
                            workbench.workflow_index =
                                live.git().map(|git| git.selected).unwrap_or(0);
                            paint = true;
                        }
                    }
                }
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
                // only when someone presses ctrl+k. Both reads are started and
                // left to the loop's own arm (`Loaded::State`, `Loaded::Older`):
                // a turn boundary is not worth a frozen keyboard, and the
                // listing in particular can take seconds on a cold store.
                if live.take_state_refresh() {
                    live.begin_state_refresh();
                }
                // A run that started or stopped changes what the lists show
                // (a session's age, a title from its first message).
                if live.activity_kind() != was {
                    live.begin_sessions_refresh();
                }
                if !completion.is_empty() && live.queue_is_empty() {
                    let event = live.completion_notification();
                    notify(terminal, notifications, &workbench.notify,
                        &format!("complete:{completion}"), event);
                }
                paint = true;
            }
            _ = ticker.tick() => {
                // Connection edges arrive on a watch channel beside the stream;
                // the tick is the one wake-up that already exists, so the
                // header's dot is refreshed there instead of adding another
                // select arm that would have to disarm itself.
                let connected = live.connection_changed();
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
                let animating = activity_moving || rail_moving;
                if animating {
                    workbench.advance_tick();
                }
                // A run in another conversation is only visible here through
                // `list_sessions`, and the agent tells no push channel about
                // it: the list is re-read on this clock or the row's spinner
                // would freeze and its 活跃 stretch never empty.
                //
                // The listing itself runs off the loop
                // (`Live::begin_sessions_refresh`) and lands here a tick later:
                // awaiting it put the whole surface behind one agent answer,
                // which at 67 conversations was 6.9 s every five seconds.
                let changed = live.poll_sessions().unwrap_or(false);
                if frame.layout.sidebar.is_some()
                    && sessions_poll.elapsed() >= SESSIONS_POLL
                    && live.begin_sessions_refresh()
                {
                    sessions_poll = Instant::now();
                }
                // The workspace's branch and the `@` menu's files are read on
                // their own threads (see `Live::poll_local_reads`): a workspace
                // that moved pays no process spawn in the loop, and this clock
                // is what puts the answer on screen when it arrives.
                let arrived = live.poll_local_reads();
                let preview_arrived = live.poll_preview();
                if preview_arrived {
                    if let Some(siblings) = live.take_preview_siblings() {
                        workbench.set_preview_siblings(siblings);
                    }
                }
                paint = clock.tick(
                    animating,
                    workbench.spinner_frame(),
                    ages_on_screen(&frame),
                    Instant::now(),
                ) || changed
                    || arrived
                    || connected
                    || preview_arrived;
            }
            notice = update_notice(update_rx) => {
                if let Some(notice) = notice {
                    live.notice = Some(notice);
                }
                **update_rx = None;
                paint = true;
            }
        }
    }
}

/// Await the startup update check, or never resolve when `--offline` removed
/// it. The arm is disabled after its one answer (`update_rx = None`).
async fn update_notice(
    rx: &mut Option<tokio::sync::oneshot::Receiver<Option<String>>>,
) -> Option<String> {
    match rx {
        Some(rx) => rx.await.ok().flatten(),
        None => std::future::pending().await,
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
/// one local call every five seconds — none of them a repaint (the poll
/// repaints only when the list it read differs from the one on screen).
///
/// The call is started, never awaited, in the loop (`Live::begin_sessions_refresh`):
/// its cost is the agent's to decide, and a listing that blocks the surface is
/// the one bill a clock this frequent cannot pay.
///
/// Measured 2026-10-30 on a 147-conversation store (`agent.db` at 1.7 GB), so
/// this clock is not the agent's own busy signal: **30–50 ms** steady — 0.6–1 %
/// of one of the two read workers — and **10 s on the first call of an agent
/// process whose pages have gone cold** (hours idle), which is a one-off: it
/// warms the same pages the next five seconds re-read. An incremental listing
/// (a revision token, a TTL memo) was therefore not built: it would trade a
/// staleness rule for a call that is already free when it matters. The long
/// tail is the cold read, and that one is the store's size, not the polling.
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
///
/// What the window defers is the *asking*, not the answering: the two reads go
/// out in a task ([`show_conversation`]), so the surface keeps painting, reading
/// the keyboard and streaming while the agent works, and the rows land on the
/// frame after. Only a burst pays the window; a single step pays it once.
const OPEN_SETTLE: Duration = Duration::from_millis(80);

/// How far apart two steps have to be to count as taps rather than a hold.
///
/// A tap and a repeat look the same to the surface — both are one
/// `shift+↑/↓` — and only their spacing tells them apart. A held key repeats
/// every 30–100 ms depending on the reader's keyboard settings, so anything
/// under two [`OPEN_SETTLE`] windows is read as a hold: the step keeps
/// deferring, and a burst of ten conversations is asked for once. A tap is
/// slower than that, and pays no window at all: the read starts the moment the
/// key is read, and the rows arrive as soon as the agent answers.
const STEP_TAP_GAP: Duration = Duration::from_millis(160);

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

    /// The reader tapped onto `id`: due at once, so the read goes out on this
    /// pass of the loop instead of a window later.
    fn now(id: String) -> Self {
        Self {
            id,
            due: Instant::now(),
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

/// Open one conversation, painting what the cache can show of it.
///
/// The agent's own half — the state and the tail page — is read in a task
/// ([`Live::start_step_switch`]), never here: a step must not put the surface
/// behind an agent answer. The rows the cache holds are painted immediately; a
/// conversation the client has not shown yet keeps the one on screen until its
/// page lands, which is the frame after.
///
/// Measured against a 146-conversation store (`agent.db` at 1.7 GB): the two
/// reads behind a step were 75 ms warm — 25 ms of state, 50 ms of page — and
/// seconds when the store was busy. Waiting for them in this loop froze the
/// rail, the stream and the keyboard for that long on *every* step, which is
/// what a held `shift+↑/↓` felt like.
fn show_conversation(
    id: &str,
    terminal: &mut Terminal,
    workbench: &mut Workbench,
    live: &mut Live,
    frame: &Frame,
    clock: &mut TickClock,
    painted: &mut FrameCache,
) -> Result<(), String> {
    if live.start_step_switch(id) {
        draw(terminal, workbench, live, frame, painted)?;
        clock.painted(workbench.spinner_frame(), Instant::now());
    }
    warm_neighbours(workbench, live, frame);
    Ok(())
}

/// Warm the two conversations the next step can land on.
///
/// A step reads a page's worth of history from the agent (measured: 120 ms
/// median on this store for a conversation the agent has not served recently),
/// and the rail makes the *next* step's destination predictable: it is the row
/// above or below the one the reader has just settled on. Reading those two now,
/// in the background, is what a reader's browsing rhythm can absorb — they are
/// looking at the conversation that just opened.
///
/// Fire and forget: nothing is installed, a lost race costs nothing but the
/// read, and a conversation the reader never opens costs the agent exactly one
/// page read that its own cache would have made anyway (see
/// [`Live::prefetch_page`]). Only a step warms anything — a click is not the
/// start of a walk.
fn warm_neighbours(workbench: &Workbench, live: &mut Live, frame: &Frame) {
    let data = live.data(frame);
    let mut wanted = Vec::with_capacity(2);
    for direction in [-1, 1] {
        if let Some(id) = workbench.session_beside(&data, direction) {
            wanted.push(id);
        }
    }
    drop(data);
    for id in wanted {
        live.prefetch_page(&id);
    }
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
                    let text = live.copy_last();
                    copy_to_clipboard(live, text, super::i18n::Key::CopyDone).await;
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
                Some(super::preferences::Action::Stop) => {
                    // Idle, this is a no-op (see `Live::abort`): `ctrl+c` means
                    // the turn in flight, and there is none.
                    live.abort().await;
                    return Ok(false);
                }
                Some(super::preferences::Action::NewSession) => {
                    return match live.apply(WbAction::Command("/new".into())).await {
                        Outcome::Quit => Ok(true),
                        Outcome::Continue => Ok(false),
                    };
                }
                Some(super::preferences::Action::Refresh) => {
                    live.begin_sessions_refresh();
                    live.begin_state_refresh();
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
                "/fork" => {
                    live.load_fork_entries().await;
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
                    open_git_panel(live, frame, workbench, section, filter, 0, true);
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
                    let text = live.copy_last();
                    copy_to_clipboard(live, text, super::i18n::Key::CopyDone).await;
                    continue;
                }
                "/paste" => {
                    // The typed door to the clipboard: `ctrl+v` is the front
                    // end's own paste on Windows (which carries text and cannot
                    // carry an image), so the chord there is `alt+v`; a reader
                    // who rebound it away, or is on a terminal that takes the
                    // key, still has this line.
                    live.paste_clipboard().await;
                    continue;
                }
                // Refreshing the view has no chord any more: it is a
                // once-in-a-while verb, so the palette (and this line) is its
                // door. Same three reads the action does.
                "/refresh" => {
                    live.begin_sessions_refresh();
                    live.begin_state_refresh();
                    live.refresh_tree();
                    continue;
                }
                "/mouse" => {
                    toggle_mouse(live, terminal, workbench, settings, settings_path);
                    continue;
                }
                // Pinning is the rail's verb and the app owns the result: `p`
                // is a letter in the draft, so `/pin` is the door, and its
                // action (`SetSessionPinned`) has to reach the driver the same
                // way the chord's did. The palette's row for the same verb
                // lands here too, so this arm is what applies and persists a pin
                // from either door.
                "/pin" => {
                    let actions = workbench.toggle_pin_selected(&live.data(frame));
                    for action in actions {
                        match action {
                            WbAction::SetSessionPinned { id, pinned } => {
                                apply_pin(live, &id, pinned, settings, settings_path)
                            }
                            other => {
                                if matches!(live.apply(other).await, Outcome::Quit) {
                                    return Ok(true);
                                }
                            }
                        }
                    }
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
        // The working-tree review: `d` in the repository browser, and its own
        // `r`. It has no palette verb — the 改动 tab of `/git` is where the same
        // question is asked, so the box is a *detail* of the browser rather than
        // a second door to it.
        if matches!(action, WbAction::OpenDiff) {
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
        // A committed palette is persisted now, not only at exit: a reader who
        // picks a theme and then loses the terminal keeps the choice.
        if let WbAction::SetTheme(id) = &action {
            workbench.set_theme(id);
            live.theme_id = id.clone();
            settings.theme_id = Some(id.clone());
            **theme_explicit = true;
            save_settings_notice(settings_path, settings, live);
            continue;
        }
        // Same rule for the language: the surface and the adapter both switch
        // now, and `settings.json` remembers it for the next start.
        if let WbAction::SetLanguage(preference) = action {
            workbench.set_language(preference);
            live.set_language(preference);
            settings.language = Some(preference.as_wire().to_string());
            save_settings_notice(settings_path, settings, live);
            continue;
        }
        // "Restore defaults" hands the palette back to the terminal: the stored
        // id goes away, so the next start follows the light/dark hint instead of
        // freezing whatever it said today.
        if let WbAction::ResetTheme = action {
            let palette = super::theme::resolve(None);
            workbench.set_theme(palette.id);
            live.theme_id = palette.id.to_string();
            settings.theme_id = None;
            **theme_explicit = false;
            save_settings_notice(settings_path, settings, live);
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
            if let Err(error) = persist_preferences(workbench, settings, settings_path) {
                settings_error_notice(live, &error);
            }
        }
        if matches!(action, WbAction::SetKeybindings) {
            if let Err(error) = workbench
                .keys
                .save(&super::preferences::keybindings_path(settings_path))
            {
                live.notice = Some(super::i18n::render(
                    live.language(),
                    super::i18n::Key::NoticeSettingsSaveFailed,
                    &[&error.to_string()],
                ));
            }
        }
        // A pin is a preference of this client's rail, so the surface's pin
        // lands in `settings.json` beside the layout — and the rail's groups are
        // rebuilt from it on the next frame.
        if let WbAction::SetSessionPinned { id, pinned } = &action {
            apply_pin(live, id, *pinned, settings, settings_path);
            continue;
        }
        // The preview box is the driver's to open: the adapter reads the bytes
        // and the surface paints them, but only this loop knows both — and which
        // list the box's `[` / `]` walk.
        if let WbAction::PreviewFile(path) = &action {
            open_preview_box(live, workbench, path, super::PreviewScope::Folder);
            continue;
        }
        if let WbAction::PreviewChatFile(path) = &action {
            open_preview_box(live, workbench, path, super::PreviewScope::Conversation);
            continue;
        }
        if matches!(action, WbAction::PreviewAttachment) {
            if live.load_attachment() {
                workbench.reset_preview();
                workbench.set_preview_siblings((Vec::new(), 0));
                workbench.open_preview(super::PreviewScope::Conversation);
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
        // `enter` on a `/git` worktree row: the panel's reads are rooted in the
        // session's workspace, so entering a worktree moves the session there
        // (the same `set_cwd` `/worktree` uses) and then re-reads the panel in
        // the new checkout — the rows on screen would otherwise still describe
        // the one the reader just left.
        if let WbAction::GitWorktree(path) = &action {
            let path = path.clone();
            live.select_worktree(&path).await;
            let (section, filter) = live
                .git()
                .map(|git| (git.section, git.filter.clone()))
                .unwrap_or((GitSection::Worktrees, None));
            open_git_panel(live, frame, workbench, section, filter, 0, true);
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
            open_git_panel(live, frame, workbench, section, filter, want, reset_scroll);
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
            let text = live.copy_last();
            copy_to_clipboard(live, text, super::i18n::Key::CopyDone).await;
            continue;
        }
        // The `[复制]` control under one message: the same clipboard road as
        // `/copy`, with the message the reader pointed at instead of the last
        // answer.
        if let WbAction::CopyMessage(row) = &action {
            let text = live.copy_message(*row);
            copy_to_clipboard(live, text, super::i18n::Key::CopiedMessage).await;
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
        // A switch that began and has not answered yet still names the
        // conversation being left on screen, so following "the conversation on
        // screen" here would put the rail's cursor back where the reader came
        // from — for a frame, and visibly. The answer's own arm (`switch_rx`)
        // does the follow when the rows land.
        let begun = live.take_switch_started();
        // A switch the rail did not make itself (a click on the launchpad's
        // recents, a `ctrl+r` pick) still has to move its cursor: the rail would
        // otherwise highlight a conversation the transcript is not showing.
        if moved && !begun {
            let data = live.data(frame);
            workbench.follow_current(&data);
        }
        // A model or level the reader chose is remembered as this terminal UI's
        // default: on the launchpad `live.apply` staged it (the agent cannot
        // hold it without a session), in a session it was applied to the agent;
        // either way `persist_pending` writes it to `settings.json` so the next
        // session starts with it.
        persist_pending(live, settings, settings_path);
    }
    Ok(false)
}

/// Write the reader's last chosen model, level and workspace into
/// `settings.json`, which is where the terminal UI keeps its defaults.
///
/// Called after every applied action and writes only on a change — a pick is
/// rare, and a launchpad chip that cycles through the list would otherwise
/// rewrite the file on each step.
fn persist_pending(live: &mut Live, settings: &mut TuiSettings, path: &Path) {
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
    // The workspace is drained rather than read: `None` is "nothing new", which
    // is also what a client in the middle of a conversation reports.
    if let Some(workspace) = live.take_workspace_to_remember() {
        if settings.last_workspace.as_deref() != Some(workspace.as_str()) {
            settings.last_workspace = Some(workspace);
            changed = true;
        }
    }
    if changed {
        save_settings_notice(path, settings, live);
    }
}

/// Persist a choice, leaving the failure visible on the notice row instead of
/// pretending a preference the next start will not see was saved.
fn save_settings_notice(path: &Path, settings: &TuiSettings, live: &mut Live) {
    if let Err(error) = save_settings(path, settings) {
        settings_error_notice(live, &error);
    }
}

/// Apply one pin and persist it: a pin is this client's own preference (the
/// agent has no such flag), so the rail's 置顶 block and `settings.json`
/// (`pinnedSessions`) move together. Both doors to the verb land here — the
/// draft's `/pin` and the palette's row for the same verb.
fn apply_pin(
    live: &mut Live,
    id: &str,
    pinned: bool,
    settings: &mut TuiSettings,
    settings_path: &Path,
) {
    live.notice = Some(live.set_session_pinned(id, pinned));
    settings.pinned_sessions = Some(live.pinned_sessions().to_vec());
    save_settings_notice(settings_path, settings, live);
}

fn settings_error_notice(live: &mut Live, error: &str) {
    live.notice = Some(super::i18n::render(
        live.language(),
        super::i18n::Key::NoticeSettingsSaveFailed,
        &[&error],
    ));
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
    // An escape sequence this build does not understand is *dropped*, never
    // typed. It used to fall through to the paste path, and the draft then held
    // `ESC[65292;1:3u` — a key release the terminal sent for the full-width
    // comma — which the frame wrote back out and the terminal executed: twelve
    // columns of the composer row vanished, the draft's tail with them, and the
    // caret was left past what the reader could see. Text arrives as text (no
    // ESC); anything carrying one is a sequence for some other layer.
    if data.contains('\x1b') {
        return Vec::new();
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
///
/// The size is asked of the terminal (`fresh_size`) rather than read from the
/// cache the resize notification fills: a `SIGWINCH` that never arrives leaves
/// that cache on the previous size, and every frame composed from it is then
/// the previous terminal's frame — a composer narrower than the window, with
/// the frame before it still readable to the right of the draft. Asking costs
/// one syscall per iteration and makes a missed notification a single stale
/// frame instead of a permanent one.
fn frame_of(terminal: &Terminal, workbench: &Workbench) -> Frame {
    let (width, height) = terminal.fresh_size();
    Frame::new(
        width,
        height,
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
fn open_git_panel(
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
    // The read runs beside the surface: measured on this repository, opening the
    // panel costs 132–287 ms for the commit log and its refs, and the loop used
    // to sit through it on every open, tab switch and filter. The panel paints
    // its previous rows until the new ones land (`Loaded::Git`).
    let started = live.begin_git_read(
        std::path::PathBuf::from(&cwd),
        section,
        filter.clone(),
        want,
    );
    if !started {
        // No driver behind this surface (never in the running app): read it here.
        let view = super::git_panel::read(
            std::path::Path::new(&cwd),
            section,
            filter,
            want,
            live.language(),
        );
        workbench.workflow_index = view.selected;
        live.set_git(Some(view));
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

/// Put `text` on the clipboard and say so on the notice row.
///
/// One road for every copy the conversation offers — `/copy`'s last answer, the
/// `[复制]` control under one message — so the two cannot drift in how they
/// report a clipboard that refused them. `done` is the sentence a successful
/// copy leaves behind.
async fn copy_to_clipboard(live: &mut Live, text: Result<String, String>, done: super::i18n::Key) {
    let text = match text {
        Ok(text) => text,
        Err(error) => {
            live.notice = Some(error);
            return;
        }
    };
    let result = tokio::task::spawn_blocking(move || crate::clipboard::copy_text(&text)).await;
    live.notice = Some(match result {
        Ok(Ok(())) => super::i18n::t(live.language(), done).into(),
        Ok(Err(error)) => {
            super::i18n::render(live.language(), super::i18n::Key::CopyFailed, &[&error])
        }
        Err(error) => super::i18n::render(
            live.language(),
            super::i18n::Key::CopyFailed,
            &[&error.to_string()],
        ),
    });
}

/// Open the preview box on `path`, stepping through `scope`'s pictures.
///
/// The pictures beside this one arrive with the decoded image, on the same
/// background thread — the box opens now with a loading row rather than freezing
/// the frame on a large file. Opened *over* whatever the reader was looking at:
/// `esc` from the box walks back to it (see [`Workbench::open_preview`]).
fn open_preview_box(
    live: &mut Live,
    workbench: &mut Workbench,
    path: &str,
    scope: super::PreviewScope,
) {
    match scope {
        super::PreviewScope::Folder => live.start_preview(path),
        super::PreviewScope::Conversation => live.start_picture(path),
    }
    workbench.reset_preview();
    workbench.set_preview_siblings((Vec::new(), 0));
    workbench.open_preview(scope);
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
    match save_settings(settings_path, settings) {
        Ok(()) => {
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
        Err(error) => settings_error_notice(live, &error),
    }
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
    painted: &mut FrameCache,
) -> Result<(), String> {
    let data = live.data(frame);
    let rows = workbench.render(&data, frame.width, frame.height);
    let canvas = canvas_rows(
        &rows,
        usize::from(frame.width),
        workbench.palette.fg,
        workbench.palette.bg,
    );
    let bytes = frame_bytes_since(
        &canvas,
        usize::from(frame.width),
        workbench.cursor(),
        painted,
        // A scroll moves whole terminal rows, so the transcript may only be
        // scrolled when those rows are its own: a rail shares them, and a picture
        // is anchored to the rows the scroll would move.
        if frame.layout.sidebar.is_none()
            && workbench.image_requests.is_empty()
            && workbench.painted_images.ids.is_empty()
            && workbench.painted_images.inline_shown.is_empty()
            && workbench.chat_window > 0
        {
            let top = usize::from(frame.layout.body.y);
            Some((top, top + workbench.chat_window - 1))
        } else {
            None
        },
    );
    if !bytes.is_empty() {
        terminal.write(&bytes);
    }
    paint_images(terminal, workbench, &painted.written);
    Ok(())
}

/// Paint the surface's own canvas onto every row — and its own default text
/// colour onto the rows that do not paint a surface of their own.
///
/// The surface owns its colours instead of borrowing the terminal profile's.
/// A workbench row is a string of styled spans and `styled` closes each span
/// with a reset (`\x1b[m`), which clears both colours — so setting them once per
/// row is not enough: every reset is followed by the palette's pair again, and
/// the row is padded out to the frame's width with them.
///
/// Without the background the canvas was whatever the reader's terminal painted,
/// which on a profile whose default is a mid slate (Ghostty ships `#282c34`)
/// washed the frozen palette out and dropped secondary text under 4.5:1 —
/// measured at 120x38, 98.6% of the cells carried no background of ours.
///
/// The foreground is applied only where the row left it to the terminal: a row
/// that paints a surface of its own (the user bubble, a tool block) chose its
/// colours, and a reader's own background pair is the one that reads on it. On
/// the canvas, though, the pair has to be ours: leaving plain text to the
/// profile is what put white-on-white light palettes and black-on-black dark ones
/// in front of readers whose profile is the other way round.
fn canvas_rows(rows: &[String], width: usize, fg: u8, bg: u8) -> Vec<String> {
    let background = format!("\x1b[48;5;{bg}m");
    rows.iter()
        .map(|row| {
            let paint = if row.contains("\x1b[48;5;") {
                background.clone()
            } else {
                format!("\x1b[38;5;{fg}m{background}")
            };
            let padding = width.saturating_sub(crate::utils::visible_width(
                &crate::utils::strip_ansi_codes(row),
            ));
            format!(
                "{paint}{}\x1b[0m{paint}{}\x1b[0m",
                repaint_after_resets(row, &paint),
                " ".repeat(padding)
            )
        })
        .collect()
}

/// Replace every `\x1b[m` / `\x1b[0m` with `\x1b[0m` + `paint`, so a span
/// boundary cannot drop the canvas.
fn repaint_after_resets(line: &str, paint: &str) -> String {
    let mut out = String::with_capacity(line.len() + 16);
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 2 < bytes.len() && bytes[i + 1] == b'[' {
            if bytes[i + 2] == b'm' {
                out.push_str("\x1b[0m");
                out.push_str(paint);
                i += 3;
                continue;
            }
            if bytes[i + 2] == b'0' && i + 3 < bytes.len() && bytes[i + 3] == b'm' {
                out.push_str("\x1b[0m");
                out.push_str(paint);
                i += 4;
                continue;
            }
        }
        let ch = line[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Paint (or remove) every picture the frame asked for.
///
/// A terminal bitmap is not a row of text: it is placed at a cell by the
/// terminal itself, above whatever the frame wrote there. So the surface only
/// *describes* what it wants ([`screen::PlacedPicture`]) and this — the one
/// place holding the terminal — writes them after the frame they belong to,
/// leaving the hardware cursor exactly where the frame put it.
///
/// The pixels are transmitted once per source (the file, and the crop of it a
/// zoomed box or a half-scrolled slot shows); every later frame only re-places
/// the image, which is a hundred bytes instead of a megabyte. Re-placing every
/// frame is what keeps a picture on screen: the frame under it is rewritten
/// each paint, and a placed image is not part of that text. A picture the frame
/// no longer asks for is deleted, which is how a thumbnail that scrolled out of
/// the window leaves the cells it used to cover.
fn paint_images(terminal: &mut Terminal, workbench: &mut Workbench, written_rows: &[usize]) {
    let requests = workbench.image_requests.clone();
    let protocol = crate::terminal_image::get_capabilities().images;
    if protocol == crate::terminal_image::ImageProtocol::None {
        if !workbench.painted_images.ids.is_empty()
            || !workbench.painted_images.inline_shown.is_empty()
        {
            workbench.painted_images.clear();
        }
        return;
    }
    let mut out = String::new();
    // Everything the frame dropped goes first: an image left behind sits on top
    // of the rows painted over it, and a scrolled thumbnail would stay where it
    // was.
    let wanted: Vec<u64> = requests
        .iter()
        .map(|placed| placed.request.source)
        .collect();
    let stale: Vec<u64> = workbench
        .painted_images
        .ids
        .keys()
        .copied()
        .filter(|source| !wanted.contains(source))
        .collect();
    for source in stale {
        if let Some(id) = workbench.painted_images.take(source) {
            out.push_str(&crate::terminal_image::delete_kitty_image(id));
        }
    }
    let mut inline_now: Vec<(u64, (u16, u16, u16, u16))> = Vec::new();
    for placed in &requests {
        let request = placed.request;
        let placement = (request.x, request.y, request.columns, request.rows);
        match protocol {
            // Both of these paint *into the text grid* instead of into a layer
            // of their own: a frame that rewrites any cell over the picture
            // erases it, so it is placed again only then — or when the shown
            // crop or its cells changed — and never once per frame, because
            // what would be repeated is the picture itself.
            crate::terminal_image::ImageProtocol::Iterm2
            | crate::terminal_image::ImageProtocol::Sixel => {
                let untouched = !written_rows.iter().any(|row| {
                    let row = *row as u16;
                    row >= request.y && row < request.y.saturating_add(request.rows)
                });
                let already = workbench
                    .painted_images
                    .inline_shown
                    .iter()
                    .any(|(source, at)| *source == request.source && *at == placement);
                if !(untouched && already) {
                    let sequence = match protocol {
                        crate::terminal_image::ImageProtocol::Iterm2 => {
                            let (rgb, w, h) = transmit_bitmap(
                                &placed.pixels,
                                request.crop,
                                request.columns as usize,
                                request.rows as usize,
                            );
                            crate::terminal_image::encode_iterm2_rgb_png(
                                &rgb,
                                w as u32,
                                h as u32,
                                request.columns as usize,
                                request.rows as usize,
                            )
                        }
                        // Sixel, the only other arm that reaches here: the
                        // terminal draws the bitmap at its own resolution, so it
                        // is sized by pixels rather than by cells.
                        _ => {
                            let (rgb, w, h) = sixel_bitmap(
                                &placed.pixels,
                                request.crop,
                                request.columns as usize,
                                request.rows as usize,
                            );
                            Some(crate::terminal_image::encode_sixel_rgb(&rgb, w, h))
                        }
                    };
                    if let Some(sequence) = sequence {
                        out.push_str(&format!(
                            "\x1b[{};{}H{sequence}",
                            request.y + 1,
                            request.x + 1
                        ));
                    }
                }
                inline_now.push((request.source, placement));
            }
            crate::terminal_image::ImageProtocol::Kitty => {
                match workbench.painted_images.ids.get(&request.source).copied() {
                    // Same bytes, new place: `a=p` places an already-transmitted
                    // image by id, and `p=1` keeps the placement one thing (a new
                    // `a=p` each frame would otherwise stack copies).
                    Some(id) => out.push_str(&format!(
                        "\x1b[{};{}H\x1b_Ga=p,i={},p=1,c={},r={},q=2,C=1\x1b\\",
                        request.y + 1,
                        request.x + 1,
                        id,
                        request.columns,
                        request.rows
                    )),
                    None => {
                        let id = crate::terminal_image::allocate_image_id();
                        // Transmit what the cells can show, not the source: the
                        // decoder keeps enough pixels for a zoomed window to
                        // stay sharp, but a whole 2048px picture across a
                        // hundred cells is fifteen megabytes over the pty that
                        // the terminal would immediately throw away.
                        let (rgb, w, h) = transmit_bitmap(
                            &placed.pixels,
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
                        for evicted in workbench.painted_images.remember(request.source, id) {
                            out.push_str(&crate::terminal_image::delete_kitty_image(evicted));
                        }
                    }
                }
            }
            crate::terminal_image::ImageProtocol::None => {}
        }
    }
    workbench.painted_images.inline_shown = inline_now;
    // Put the cursor back where the frame left it; the images do not move it
    // (kitty's `C=1`), but the addressing above did — and an in-line picture does
    // move it, which is the other reason this runs last.
    if let Some((x, y)) = workbench.cursor() {
        out.push_str(&format!("\x1b[{};{}H", y + 1, x + 1));
    }
    if !out.is_empty() {
        terminal.write(&out);
    }
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
    let (crop_w, crop_h) = crop_size(crop);
    let budget = (columns.max(1) * TRANSMIT_PX_PER_CELL).min(pixels.width.max(1));
    if crop_w <= budget || rows == 0 || columns == 0 {
        return (crop_rgb(pixels, crop), crop_w, crop_h);
    }
    let target_w = budget.max(1);
    let target_h = ((crop_h as f64) * (target_w as f64) / (crop_w as f64))
        .round()
        .max(1.0) as usize;
    let window = crop_window(pixels, crop, crop_w, crop_h);
    let scaled = image::imageops::resize(
        &window,
        target_w as u32,
        target_h as u32,
        image::imageops::FilterType::Triangle,
    );
    (rgb_bytes(&scaled), target_w, target_h)
}

/// The bitmap a sixel terminal gets for `crop`: the same window, scaled to the
/// pixel rectangle the cells it was given can hold.
///
/// Kitty and iTerm2 are told the cell rectangle (`c=`,`r=` / `width=`,`height=`)
/// and scale the bitmap themselves, which is why their transmission is *capped*
/// rather than sized. Sixel has no such parameter: the terminal paints the
/// pixels it is handed, one image pixel per screen pixel, from whatever cell the
/// cursor stands on. The size therefore has to be ours, and this is the one
/// place in the TUI that assumes how many pixels a cell is —
/// [`crate::terminal_image::get_cell_dimensions`], nine by eighteen unless a
/// terminal has reported its own. A profile with a bigger cell shows the picture
/// a little short of its box; one with a smaller cell shows it over the edge. A
/// preview either way, and far better than a grid of sixel blocks that has to be
/// scaled by a factor nobody chose.
fn sixel_bitmap(
    pixels: &super::image_view::ImagePixels,
    crop: (usize, usize, usize, usize),
    columns: usize,
    rows: usize,
) -> (Vec<u8>, usize, usize) {
    let cells = crate::terminal_image::get_cell_dimensions();
    let target_w = (columns.max(1) * cells.width_px).max(1);
    let target_h = (rows.max(1) * cells.height_px).max(1);
    let (crop_w, crop_h) = crop_size(crop);
    if (crop_w, crop_h) == (target_w, target_h) {
        return (crop_rgb(pixels, crop), crop_w, crop_h);
    }
    let window = crop_window(pixels, crop, crop_w, crop_h);
    let scaled = image::imageops::resize(
        &window,
        target_w as u32,
        target_h as u32,
        image::imageops::FilterType::Triangle,
    );
    (rgb_bytes(&scaled), target_w, target_h)
}

/// A crop's own size, never zero — a rectangle with no area still asks for one
/// pixel rather than an empty buffer.
fn crop_size(crop: (usize, usize, usize, usize)) -> (usize, usize) {
    (
        crop.2.saturating_sub(crop.0).max(1),
        crop.3.saturating_sub(crop.1).max(1),
    )
}

/// The crop as an image to resample, with anything outside the picture black.
fn crop_window(
    pixels: &super::image_view::ImagePixels,
    crop: (usize, usize, usize, usize),
    width: usize,
    height: usize,
) -> image::RgbaImage {
    let mut window = image::RgbaImage::new(width as u32, height as u32);
    for y in 0..height {
        for x in 0..width {
            let offset = ((crop.1 + y) * pixels.width + crop.0 + x) * 4;
            let pixel = pixels
                .rgba
                .get(offset..offset + 4)
                .map(|rgba| [rgba[0], rgba[1], rgba[2], rgba[3]])
                .unwrap_or([0, 0, 0, 0]);
            window.put_pixel(x as u32, y as u32, image::Rgba(pixel));
        }
    }
    window
}

/// An image's pixels as the row-major RGB the image protocols carry.
fn rgb_bytes(image: &image::RgbaImage) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(image.width() as usize * image.height() as usize * 3);
    for pixel in image.pixels() {
        rgb.extend_from_slice(&pixel.0[..3]);
    }
    rgb
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
#[cfg(test)]
fn frame_bytes(rows: &[String], cursor: Option<(u16, u16)>) -> String {
    let mut cache = FrameCache::default();
    frame_bytes_since(rows, 0, cursor, &mut cache, None)
}

/// The rows and cursor the terminal was last written, so a frame can send the
/// rows that changed instead of the whole screen.
#[derive(Default)]
struct FrameCache {
    rows: Vec<String>,
    width: usize,
    cursor: Option<(u16, u16)>,
    /// The rows the last call erased and rewrote. In-line images (iTerm2's, a
    /// sixel) live in the text grid rather than in a layer of their own, so a
    /// picture is only erased by a write to *its own* rows — a streamed reply
    /// repainting the tail must not cost a re-transmission of a thumbnail
    /// further up.
    written: Vec<usize>,
}

/// The bytes needed to turn `cache` into `rows` at `width`.
///
/// A width or height change is a full repaint: the frame's geometry moved, and
/// a row that happens to compare equal at a different width still occupies
/// different cells. Otherwise a frame that is exactly the one on screen scrolled
/// up inside `scroll`'s band is sent as one region scroll plus the band's new
/// bottom rows ([`scroll_of`]); everything else is addressed row by row.
///
/// `scroll` is the rows a transcript occupies, offered only when they are the
/// transcript's own (a terminal scroll moves whole rows, so a rail painted beside
/// it would be scrolled too) and when no in-line picture is on screen.
///
/// **Everything here is in physical rows.** `rows` is one string per screen row —
/// the transcript's logical lines were wrapped into rows before this function saw
/// them (`ChatArea::render_all_shared` hands out the wrapped lines) — and the band
/// indexes that same list. A wrapped line is therefore a run of consecutive
/// entries: the shift test compares them like any other row, so a scroll that
/// would break a wrapped line apart cannot pass it. Do not re-derive a band from
/// logical lines (message rows, transcript lines before wrapping): a logical
/// "one line up" is not a physical one whenever something wrapped, and the frames
/// would disagree by however many rows that line took.
fn frame_bytes_since(
    rows: &[String],
    width: usize,
    cursor: Option<(u16, u16)>,
    cache: &mut FrameCache,
    scroll: Option<(usize, usize)>,
) -> String {
    let full = cache.width != width || cache.rows.len() != rows.len();
    let mut frame = String::new();
    frame.push_str("\x1b[?25l");
    let mut written: Vec<usize> = Vec::new();
    let changed: Vec<usize> = if full {
        (0..rows.len()).collect()
    } else {
        (0..rows.len())
            .filter(|index| cache.rows.get(*index) != rows.get(*index))
            .collect()
    };
    let scroll = scroll_of(&cache.rows, rows, !full, scroll, changed.len());
    if let Some((top, bottom, n)) = scroll {
        // The region is the transcript's rows and nothing outside it changed
        // (both are checked before we get here), so one scroll sequence replaces
        // rewriting every row in it.
        frame.push_str(&format!("\x1b[{};{}r\x1b[{n}S", top + 1, bottom + 1));
        for (index, row) in rows.iter().enumerate().skip(bottom + 1 - n).take(n) {
            written.push(index);
            frame.push_str(&format!("\x1b[0m\x1b[{};1H", index + 1));
            frame.push_str("\x1b[K");
            frame.push_str(row);
        }
        frame.push_str("\x1b[r");
    } else {
        for index in changed {
            written.push(index);
            frame.push_str(&format!("\x1b[0m\x1b[{};1H", index + 1));
            frame.push_str("\x1b[K");
            frame.push_str(&rows[index]);
        }
    }
    if written.is_empty() && cursor == cache.cursor {
        cache.written.clear();
        return String::new();
    }
    cache.written = written;
    frame.push_str("\x1b[0m");
    if let Some((x, y)) = cursor {
        frame.push_str(&format!("\x1b[{};{}H\x1b[?25h", y + 1, x + 1));
    }
    cache.rows = rows.to_vec();
    cache.width = width;
    cache.cursor = cursor;
    frame
}

/// The band and distance of a transcript scroll, when this frame is exactly one.
///
/// Three things must hold, and each is an equality rather than a resemblance: the
/// geometry is unchanged, every row *outside* the band is identical, and inside
/// the band row `i` is row `i + n` shifted up for every row above the `n` that
/// came in at the bottom. Anything else — a reply that grew in the middle, a
/// different conversation, a rail, an overlay — answers `None` and the row diff
/// takes over, which is what keeps a "smarter" repaint from painting the wrong
/// screen.
///
/// The unit is the physical row (`shown` and `rows` are screen rows, wrapping
/// already applied): a logical line that wrapped to three rows is three entries
/// that must shift together, which the equality enforces for free.
///
/// It also has to pay: the scroll costs three sequences and `n` rows, and it is
/// only taken when the row diff would have written more rows than that.
fn scroll_of(
    shown: &[String],
    rows: &[String],
    same_geometry: bool,
    band: Option<(usize, usize)>,
    changed: usize,
) -> Option<(usize, usize, usize)> {
    let (top, bottom) = band?;
    if !same_geometry || shown.len() != rows.len() || top > bottom || bottom >= rows.len() {
        return None;
    }
    if (0..rows.len()).any(|index| (index < top || index > bottom) && rows[index] != shown[index]) {
        return None;
    }
    let height = bottom - top + 1;
    if height < 2 {
        return None;
    }
    for n in 1..height {
        if (0..height - n).all(|index| rows[top + index] == shown[top + index + n]) {
            return (1 + n < changed).then_some((top, bottom, n));
        }
    }
    None
}

fn persist_preferences(
    workbench: &Workbench,
    settings: &mut TuiSettings,
    path: &Path,
) -> Result<(), String> {
    let preferences = workbench.preferences();
    settings.sidebar_position = Some(preferences.sidebar_position.as_wire().into());
    settings.sidebar_width = Some(preferences.sidebar_width);
    settings.sidebar_visible = Some(preferences.sidebar_visible);
    settings.mouse_capture = Some(preferences.mouse_capture);
    settings.notify = Some(preferences.notify);
    // Which workspaces the reader folded shut survives a restart; the search text
    // does not (see `TuiSettings::folded_workspaces`).
    settings.folded_workspaces = Some(workbench.folded_workspaces());
    save_settings(path, settings)
}

#[cfg(test)]
pub(crate) fn load_settings(path: &Path) -> TuiSettings {
    load_settings_report(path).0
}

/// Read the settings with the reason a broken file was not used.
///
/// A corrupt file is not silently replaced in memory and then overwritten on
/// the next preference change: the caller gets the defaults *and* a sentence
/// that says the file could not be read.
pub(crate) fn load_settings_report(path: &Path) -> (TuiSettings, Option<String>) {
    match future_config::read_json_object(path) {
        Ok(value) => (TuiSettings::from_json(&value), None),
        Err(error) => (TuiSettings::default(), Some(error.to_string())),
    }
}

pub(crate) fn save_settings(path: &Path, settings: &TuiSettings) -> Result<(), String> {
    future_config::write_json_atomic(path, &settings.to_json(), false)
        .map_err(|error| error.to_string())
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

    /// The canvas is the palette's, not the reader's terminal profile.
    ///
    /// Every row is filled to the frame's width with `bg` and survives the resets
    /// `styled` writes at the end of each span — otherwise the colours are cleared
    /// mid-row and the terminal's own show through. That was the "background not
    /// black enough" state this replaced (98.6% of the cells carried no colour of
    /// ours, measured at 120x38 on Ghostty's `#282c34`).
    ///
    /// A row that paints a surface of its own keeps its colours and only gets the
    /// canvas as the *fallback* background; a row on the canvas gets the palette's
    /// text colour too, so neither a white-on-white light palette nor a
    /// black-on-black dark one depends on the reader's profile.
    #[test]
    fn canvas_rows_fill_every_column_and_survive_resets() {
        // A row of two spans, a plain row (no colour of its own) and an empty
        // row: the three shapes the compositor produces.
        let rows = vec![
            format!(
                "\x1b[38;5;252mgreeting\x1b[m{}hello\x1b[m",
                crate::theme::bold(" ")
            ),
            "short".to_string(),
            String::new(),
        ];
        let width = 20;
        let canvas = canvas_rows(&rows, width, 252, 232);
        for (index, row) in canvas.iter().enumerate() {
            assert!(
                row.starts_with("\x1b[38;5;252m\x1b[48;5;232m"),
                "row {index} must open on the palette's pair: {row:?}"
            );
            assert_eq!(
                crate::utils::visible_width(row),
                width,
                "row {index} must be filled to the frame width: {row:?}"
            );
            // A reset must never leave the terminal painting its own colours:
            // every piece after a reset re-opens with the palette's pair, and
            // only the row's own final reset may stand alone.
            let pieces: Vec<&str> = row.split("\x1b[0m").collect();
            for (piece, text) in pieces.iter().enumerate().skip(1) {
                if text.is_empty() && piece == pieces.len() - 1 {
                    continue;
                }
                assert!(
                    text.starts_with("\x1b[38;5;252m\x1b[48;5;232m"),
                    "row {index} drops the palette after a reset: {row:?}"
                );
            }
        }
        // Painting is a fill, not a rewrite: the text is still there.
        assert!(canvas[0].contains("hello"));
        assert!(canvas[1].contains("short"));

        // A row with a surface of its own keeps it: the bubble's background is
        // written over the canvas, and its text keeps the reader's own colour.
        let bubble = canvas_rows(
            &["\x1b[48;5;59m a message \x1b[0m".to_string()],
            20,
            252,
            232,
        );
        assert!(
            bubble[0].starts_with("\x1b[48;5;232m\x1b[48;5;59m"),
            "the bubble's own background wins: {:?}",
            bubble[0]
        );
        assert!(
            !bubble[0].contains("38;5;252"),
            "a row that paints a surface decides its own text colour: {:?}",
            bubble[0]
        );
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

    /// A key event the surface does not know is *dropped*, never typed.
    ///
    /// Kitty reports every key as `CSI codepoint;modifiers:kind u` once it is
    /// asked to, and a terminal with an IME sends the full-width comma that way:
    /// `ESC[65292;1:3u` (codepoint 65292, no modifiers, `:3` = release). That
    /// chunk arrived while the draft was being typed, was not recognised as a
    /// key, and fell through to the paste path — so the draft held those twelve
    /// characters, the frame wrote the escape back out, and the terminal ate
    /// twelve columns of the composer row: the reader's text lost its tail and
    /// the caret sat past what they could see.
    #[test]
    fn an_unrecognised_key_event_is_dropped_rather_than_typed() {
        // The release of a full-width comma: not a key press, nothing to insert.
        assert!(key_events("\x1b[65292;1:3u").is_empty());
        // Its press is that character, not an escape: with an IME the text and
        // the key report arrive together, and the character is the one to keep.
        assert_eq!(
            key_events("\x1b[65292u"),
            vec![WbEvent::Key("，".into())],
            "a printable codepoint is the character itself"
        );
        // A sequence from some other layer (a terminal reply, a mouse dialect)
        // is nobody's text: dropping it leaves the draft clean.
        for seq in ["\x1b[999z", "\x1b[1;2;3Q", "\x1b]52;c;Zm9v\x07"] {
            assert!(
                key_events(seq).is_empty(),
                "{seq:?} must not reach the draft"
            );
        }
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
            rgba: rgba.into(),
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
            rgba: rgba.into(),
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

    /// A sixel picture is sized in *pixels* — the terminal paints what it is
    /// given at its own resolution — so the crop is resampled into the pixel
    /// rectangle its cells can hold, with no cell parameter to lean on.
    #[test]
    fn a_sixel_picture_is_sized_to_the_pixels_its_cells_hold() {
        use crate::terminal_image::{get_cell_dimensions, set_cell_dimensions, CellDimensions};
        let saved = get_cell_dimensions();
        set_cell_dimensions(CellDimensions {
            width_px: 10,
            height_px: 20,
        });
        let mut rgba = Vec::new();
        for y in 0..40 {
            for x in 0..40 {
                rgba.extend_from_slice(&[(x * 6) as u8, (y * 6) as u8, 9, 255]);
            }
        }
        let pixels = super::super::image_view::ImagePixels {
            width: 40,
            height: 40,
            rgba: rgba.into(),
        };
        // Five cells at ten pixels across, three cells at twenty tall.
        let (rgb, w, h) = sixel_bitmap(&pixels, (0, 0, 40, 40), 5, 3);
        assert_eq!((w, h), (50, 60));
        assert_eq!(rgb.len(), 50 * 60 * 3);
        // A window already the size of its box is transmitted as it is: a
        // zoomed crop is sharp, and resampling it would only blur it.
        let (rgb, w, h) = sixel_bitmap(&pixels, (0, 0, 40, 40), 4, 2);
        assert_eq!((w, h), (40, 40));
        assert_eq!(rgb, crop_rgb(&pixels, (0, 0, 40, 40)));
        set_cell_dimensions(saved);
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
    /// not become an `escape` key, and its bytes must not be typed either. The
    /// surface holds a partial chunk for the next read (`stdin_buffer`'s
    /// deadline); what reaches the event path unfinished is dropped, because
    /// typing `ESC[1;5` into the draft puts an escape in the frame, where the
    /// terminal executes it instead of drawing it.
    #[test]
    fn an_incomplete_sequence_is_dropped_rather_than_typed() {
        // A lone trailing `\x1b` after a complete sequence.
        assert!(
            events_of("\x1b[B\x1b").is_empty(),
            "{:?}",
            events_of("\x1b[B\x1b")
        );
        // A truncated CSI likewise: it parses as nothing, so it is not a key.
        assert!(events_of("\x1b[1;5").is_empty());
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
            "\x1b[?25l\x1b[0m\x1b[1;1H\x1b[Ka\x1b[0m\x1b[2;1H\x1b[Kb\x1b[0m"
        );
    }

    /// A row is erased before it is written, so a terminal wider than the frame
    /// cannot keep the previous frame's cells in the columns beside it. That is
    /// the state a resize leaves behind for a frame or two — a composer narrower
    /// than the window, with the frame before it still legible to the right of
    /// the draft — and stale text where the draft continues reads as if it were
    /// typed.
    #[test]
    fn every_row_is_erased_before_it_is_painted() {
        let rows: Vec<String> = (0..4).map(|row| format!("r{row}")).collect();
        let frame = frame_bytes(&rows, None);
        for index in 1..=rows.len() {
            assert!(
                frame.contains(&format!("\x1b[0m\x1b[{index};1H\x1b[K")),
                "row {index} is not erased before its text: {frame:?}"
            );
        }
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

    /// Preferences are written atomically, and a corrupt file is reported
    /// instead of being read as an empty document with no explanation.
    #[test]
    fn settings_are_written_atomically_and_a_broken_file_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = TuiSettings {
            theme_id: Some("nord".into()),
            ..TuiSettings::default()
        };
        save_settings(&path, &settings).expect("atomic save");
        assert_eq!(load_settings(&path).theme_id.as_deref(), Some("nord"));
        assert!(
            !std::fs::read_dir(dir.path())
                .unwrap()
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().contains(".tmp.")),
            "the temporary sibling is renamed away"
        );

        std::fs::write(&path, "{ not json").unwrap();
        let (fallback, warning) = load_settings_report(&path);
        assert!(fallback.theme_id.is_none());
        assert!(
            warning.is_some(),
            "the reader is told why defaults are in use"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ not json",
            "loading must not rewrite the broken file behind the reader's back"
        );

        // A parent that is a file is a save failure, not a silently lost choice.
        let blocking = dir.path().join("not-a-dir");
        std::fs::write(&blocking, "x").unwrap();
        assert!(save_settings(&blocking.join("settings.json"), &settings).is_err());
    }

    /// Only changed rows cross the pty; a geometry change falls back to a full
    /// repaint, and an unchanged frame with the same cursor writes nothing.
    #[test]
    fn a_repaint_sends_only_rows_that_changed() {
        let first = vec!["one".to_string(), "two".to_string(), "three".to_string()];
        let mut cache = FrameCache::default();
        let full = frame_bytes_since(&first, 20, Some((3, 1)), &mut cache, None);
        assert!(full.contains("\x1b[1;1H") && full.contains("\x1b[2;1H"));

        let same = frame_bytes_since(&first, 20, Some((3, 1)), &mut cache, None);
        assert!(
            same.is_empty(),
            "an unchanged frame writes nothing: {same:?}"
        );

        let mut changed = first.clone();
        changed[1] = "TWO".into();
        let patch = frame_bytes_since(&changed, 20, Some((3, 1)), &mut cache, None);
        assert!(patch.contains("\x1b[2;1H") && patch.contains("TWO"));
        assert!(!patch.contains("\x1b[1;1H") && !patch.contains("\x1b[3;1H"));

        let resized = frame_bytes_since(&changed, 21, Some((3, 1)), &mut cache, None);
        assert!(
            resized.contains("\x1b[1;1H") && resized.contains("\x1b[3;1H"),
            "a width change repaints every row"
        );
    }

    /// A transcript scroll is one region scroll plus the lines that came in, not
    /// a rewrite of every row in the band.
    #[test]
    fn a_one_row_scroll_is_one_sequence_and_one_new_line() {
        let shown = scroll_frame(0);
        let next = scroll_frame(1);
        let mut cache = FrameCache::default();
        let first = frame_bytes_since(&shown, 180, None, &mut cache, Some((2, 37)));
        assert!(first.contains("line 0 "), "a first frame is a full paint");

        let bytes = frame_bytes_since(&next, 180, None, &mut cache, Some((2, 37)));
        assert!(
            bytes.contains("\x1b[3;38r\x1b[1S"),
            "one region scroll of one line: {bytes:?}"
        );
        assert!(
            bytes.contains("line 36 "),
            "the line that came in is written: {bytes:?}"
        );
        assert!(
            !bytes.contains("line 3 ") && !bytes.contains("line 20 "),
            "the rows above are moved, not repainted: {bytes:?}"
        );
        assert!(
            bytes.ends_with("\x1b[r") || bytes.contains("\x1b[r"),
            "{bytes:?}"
        );

        // What a terminal shows after those bytes: the band scrolled up by one
        // (row 2 becomes the old row 3, and so on), the last band row replaced by
        // the line that arrived.
        let mut grid = shown.clone();
        grid[2..=36].clone_from_slice(&shown[3..=37]);
        grid[37] = next[37].clone();
        assert_eq!(grid, next, "the scroll plus the new line is the frame");
    }

    /// Ten lines at once is one `CSI 10 S`, then ten rows — not ten scrolls.
    #[test]
    fn a_ten_row_scroll_is_one_sequence() {
        let shown = scroll_frame(0);
        let next = scroll_frame(10);
        let mut cache = FrameCache::default();
        frame_bytes_since(&shown, 180, None, &mut cache, Some((2, 37)));
        let bytes = frame_bytes_since(&next, 180, None, &mut cache, Some((2, 37)));
        assert!(bytes.contains("\x1b[10S"), "{bytes:?}");
        assert_eq!(bytes.matches('S').count(), 1, "one scroll: {bytes:?}");
    }

    /// Anything that is not exactly a scroll is repainted row by row: a row that
    /// changed in place, a frame that grew in the middle, a rail beside the
    /// transcript, a picture, and a change outside the band.
    #[test]
    fn only_an_exact_scroll_is_sent_as_a_scroll() {
        let shown = scroll_frame(0);
        let painted = || {
            let mut cache = FrameCache::default();
            frame_bytes_since(&shown, 180, None, &mut cache, Some((2, 37)));
            cache
        };

        // A row edited in place.
        let mut cache = painted();
        let mut edited = shown.clone();
        edited[10] = "\x1b[38;5;252medited\x1b[0m".into();
        let bytes = frame_bytes_since(&edited, 180, None, &mut cache, Some((2, 37)));
        assert!(
            !bytes.contains('S'),
            "a row edit is not a scroll: {bytes:?}"
        );

        // A frame that grew in the middle of the band.
        let mut cache = painted();
        let mut grown = shown.clone();
        grown[5] = "\x1b[38;5;252mgrown\x1b[0m".into();
        let bytes = frame_bytes_since(&grown, 180, None, &mut cache, Some((2, 37)));
        assert!(!bytes.contains('S'), "{bytes:?}");

        // A change outside the band (the header) disqualifies it, even when the
        // band itself shifted: the terminal scrolls whole rows and would take the
        // header with it.
        let mut cache = painted();
        let mut header_moved = scroll_frame(1);
        header_moved[0] = "\x1b[38;5;252manother header\x1b[0m".into();
        let bytes = frame_bytes_since(&header_moved, 180, None, &mut cache, Some((2, 37)));
        assert!(!bytes.contains('S'), "{bytes:?}");

        // No band offered (a rail shares the rows, or a picture is up).
        let mut cache = painted();
        let bytes = frame_bytes_since(&scroll_frame(1), 180, None, &mut cache, None);
        assert!(!bytes.contains('S'), "{bytes:?}");

        // A height change is a full repaint whatever the band says.
        let mut cache = painted();
        let taller = scroll_frame_rows(0, 41);
        let bytes = frame_bytes_since(&taller, 180, None, &mut cache, Some((2, 37)));
        assert!(bytes.contains("\x1b[1;1H") && bytes.contains("\x1b[41;1H"));
    }

    /// An unchanged frame is never "scrolled": that would move the screen for a
    /// frame that has nothing to move.
    #[test]
    fn an_unchanged_frame_is_never_scrolled() {
        let shown = scroll_frame(0);
        let mut cache = FrameCache::default();
        frame_bytes_since(&shown, 180, None, &mut cache, Some((2, 37)));
        let bytes = frame_bytes_since(&shown, 180, None, &mut cache, Some((2, 37)));
        assert!(bytes.is_empty(), "{bytes:?}");
    }

    /// A frame 40 rows tall: two header rows, a 36-row transcript band
    /// (`2..=37`) and a two-row composer. Row `first + n` is "line n", so a frame
    /// built from `first` is the one before it scrolled up by one.
    fn scroll_frame(first: usize) -> Vec<String> {
        scroll_frame_rows(first, 40)
    }

    fn scroll_frame_rows(first: usize, height: usize) -> Vec<String> {
        let band = height.saturating_sub(4);
        let mut rows: Vec<String> = (0..2).map(|i| format!("header {i}")).collect();
        rows.extend((0..band).map(|i| {
            let text = format!(
                "  line {} of the transcript, wrapped to the frame's width",
                first + i
            );
            let padding = 180usize.saturating_sub(text.chars().count());
            format!(
                "\x1b[38;5;252m{text}\x1b[0m\x1b[38;5;252m{}\x1b[0m",
                " ".repeat(padding)
            )
        }));
        rows.extend((0..2).map(|i| format!("composer {i}")));
        rows
    }

    /// A long line that wrapped occupies several *physical* rows, and a scroll
    /// moves all of them together — or none.
    ///
    /// This is the one case where "content moved up a line" and "the screen moved
    /// up a line" could disagree: if a band were measured in logical lines, a
    /// three-row wrapped line would hand the terminal a `CSI 1 S` and then write
    /// the wrong number of rows, shifting everything below it. The band here is
    /// physical rows, so the predicate sees the wrapped run as three equalities:
    /// a scroll that would tear it apart cannot pass.
    #[test]
    fn a_wrapped_line_moves_as_whole_physical_rows_or_not_at_all() {
        // One logical line, three physical rows, at band rows 5, 6 and 7.
        let frame_at = |first: usize| -> Vec<String> {
            let mut rows: Vec<String> = (0..2).map(|i| format!("header {i}")).collect();
            for index in 0..36 {
                let row = first + index;
                rows.push(match row {
                    5 => "\x1b[38;5;252m  line 5: a long message, part 1 of 3\x1b[0m".into(),
                    6 => "\x1b[38;5;252m  line 5: a long message, part 2 of 3\x1b[0m".into(),
                    7 => "\x1b[38;5;252m  line 5: a long message, part 3 of 3\x1b[0m".into(),
                    other => format!("\x1b[38;5;252m  line {other}\x1b[0m"),
                });
            }
            rows.extend((0..2).map(|i| format!("composer {i}")));
            rows
        };

        // Scrolled by one physical row: the wrapped run moved with everything else.
        let shown = frame_at(0);
        let next = frame_at(1);
        let mut cache = FrameCache::default();
        frame_bytes_since(&shown, 180, None, &mut cache, Some((2, 37)));
        let bytes = frame_bytes_since(&next, 180, None, &mut cache, Some((2, 37)));
        assert!(bytes.contains("\x1b[1S"), "{bytes:?}");
        // And the terminal ends up showing exactly the new frame: a wrapped line
        // that was torn would show part 2 where part 1 belongs.
        let mut grid = shown.clone();
        grid[2..=36].clone_from_slice(&shown[3..=37]);
        grid[37] = next[37].clone();
        assert_eq!(grid, next, "the wrapped run moved as one");

        // The wrap itself changed (the same logical line re-wrapped, so a row of
        // the run differs): that is not a shift, and it is repainted.
        let mut cache = FrameCache::default();
        frame_bytes_since(&shown, 180, None, &mut cache, Some((2, 37)));
        let mut rewrapped = frame_at(1);
        rewrapped[8] = "\x1b[38;5;252m  line 7: re-wrapped\x1b[0m".into();
        let bytes = frame_bytes_since(&rewrapped, 180, None, &mut cache, Some((2, 37)));
        assert!(!bytes.contains('S'), "a re-wrap is repainted: {bytes:?}");
    }

    /// The workspaces the reader folded shut survive a restart through the same
    /// file the layout uses.
    #[test]
    fn folded_workspaces_reach_the_settings_file_and_come_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let mut workbench = super::super::Workbench::new();
        workbench.set_folded_workspaces(["/src/future-os".to_string(), "/src/mobile".into()]);
        let mut settings = TuiSettings::default();
        persist_preferences(&workbench, &mut settings, &path).expect("settings save");

        // Read the file back, not the value the writer kept: the point is that a
        // *next start* sees the folds.
        let written = load_settings(&path);
        assert_eq!(
            written.folded_workspaces,
            Some(vec!["/src/future-os".into(), "/src/mobile".into()])
        );

        let mut restored = super::super::Workbench::new();
        restored.set_folded_workspaces(written.folded_workspaces.unwrap_or_default());
        assert_eq!(restored.folded_workspaces(), workbench.folded_workspaces());
    }

    /// The workspace a reader picked while a conversation was attached is what
    /// `settings.json` remembers, and what the next start opens on.
    ///
    /// Read back from the file rather than from the value the writer kept: the
    /// point is that a *next start* sees it.
    #[tokio::test]
    async fn the_chosen_workspace_reaches_the_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let workspace = tempfile::tempdir().unwrap();
        let chosen = workspace.path().to_string_lossy().to_string();

        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = crate::workbench::live::Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        // The launchpad's own road: nothing attached, so the directory is staged
        // for the session the first message creates.
        live.select_workspace(&chosen).await;

        let mut settings = TuiSettings::default();
        persist_pending(&mut live, &mut settings, &path);
        assert_eq!(
            load_settings(&path).last_workspace.as_deref(),
            Some(chosen.as_str())
        );

        // A second pass with nothing staged leaves the value alone — the write
        // happens on a change, not on every action.
        let mut settings = load_settings(&path);
        persist_pending(&mut live, &mut settings, &path);
        assert_eq!(
            load_settings(&path).last_workspace.as_deref(),
            Some(chosen.as_str())
        );
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

    fn rows_of(text: &[&str]) -> Vec<String> {
        text.iter().map(|row| (*row).to_string()).collect()
    }

    /// A scroll is a claim about the rows *inside* one band. A band of a single
    /// row makes no such claim (nothing inside it can move), and a band that
    /// runs off the frame describes rows that are not there — both are refused
    /// rather than sent as a scroll the terminal cannot honour.
    #[test]
    fn a_scroll_is_refused_for_a_band_that_cannot_move() {
        let shown = rows_of(&["a", "b", "c", "d"]);
        assert_eq!(
            scroll_of(&shown, &shown, true, Some((1, 1)), 4),
            None,
            "a one-row band was sent as a scroll"
        );
        assert_eq!(
            scroll_of(&shown, &shown, true, Some((2, 9)), 4),
            None,
            "a band past the last row was sent as a scroll"
        );
        // The positive control: the same geometry with the content moved up by
        // one row *is* a scroll, so the refusals above are about the band.
        let moved = rows_of(&["b", "c", "d", "q"]);
        assert_eq!(
            scroll_of(&shown, &moved, true, Some((0, 3)), 4),
            Some((0, 3, 1))
        );
    }

    /// A partial mouse report is held for the next read, not typed: half of
    /// `\x1b[<0;12;7M` in the draft would be the escape bytes.
    #[test]
    fn half_a_mouse_report_is_held_rather_than_typed() {
        assert!(events_of("\x1b[<0;12").is_empty());
        // …and the rest of it, arriving on its own, is one mouse event.
        let done = events_of("\x1b[<0;12;7M");
        assert!(
            matches!(done.as_slice(), [WbEvent::Mouse(_)]),
            "a complete report did not decode: {done:?}"
        );
    }

    /// Reading more of the terminal must not lose what arrived before the paste
    /// opened: the text ahead of it is typed, then the paste is held whole so
    /// the next read can close it.
    #[test]
    fn text_before_an_unterminated_paste_is_typed_first() {
        let events = events_of("ab\x1b[200~rest");
        assert_eq!(
            events,
            vec![
                WbEvent::Key("a".into()),
                WbEvent::Key("b".into()),
                WbEvent::Paste("rest".into()),
            ],
            "the keys before the paste were dropped or reordered"
        );
    }

    /// The rail's step chords are recognised before the key reaches `quit`, so a
    /// held key becomes one switch. Both directions are read here, and an
    /// overlay owns the keys it paints.
    #[test]
    fn the_step_chords_are_read_both_ways_and_only_without_an_overlay() {
        use crate::workbench::preferences::{Action, Keybindings};

        let mut keys = Keybindings::default();
        keys.bind(Action::PrevSession, "ctrl+u")
            .expect("a spare chord");
        keys.bind(Action::NextSession, "ctrl+d")
            .expect("a spare chord");
        let mut workbench = Workbench::new();
        workbench.keys = keys.clone();
        assert_eq!(
            session_step_of(&workbench, &WbEvent::Key("ctrl+u".into())),
            Some(-1),
            "the previous-session chord was not read as a step back"
        );
        assert_eq!(
            session_step_of(&workbench, &WbEvent::Key("ctrl+d".into())),
            Some(1)
        );
        assert_eq!(
            session_step_of(&workbench, &WbEvent::Key("ctrl+x".into())),
            None,
            "a chord that is not a step was read as one"
        );
        assert_eq!(
            session_step_of(&workbench, &WbEvent::Paste("ctrl+u".into())),
            None,
            "pasted text was read as a step chord"
        );
        workbench.overlay = Some(crate::workbench::Overlay::Help);
        assert_eq!(
            session_step_of(&workbench, &WbEvent::Key("ctrl+u".into())),
            None,
            "an overlay's key was taken as a session step"
        );
    }
}

#[cfg(test)]
mod driver_tests {
    //! The driver loop itself, driven in-process: what a key, an agent event, a
    //! loaded page and the clock each do to the frame.
    //!
    //! Everything here goes through the real `event_loop`, or through the real
    //! `quit` with the event the loop would have handed it. The loop's own wiring —
    //! which arm paints, which key lands a pending session step first, what the
    //! terminal is handed at exit — is exactly what the tests around it cannot see.
    //!
    //! The bytes a frame writes are captured (`terminal::capture`) instead of
    //! reaching fd 1, so a test can assert what the reader would have seen.

    use std::time::Duration;

    use tokio::sync::mpsc;

    use super::super::live::Live;
    use super::{event_loop, frame_of, quit, DriverIo, InputIo, TerminalEvent};
    use crate::rpc::types::AgentEvent;
    use crate::settings::TuiSettings;
    use crate::terminal::Terminal;
    use crate::workbench::{WbEvent, Workbench};

    /// The frame's text with the escape sequences taken out, so an assertion
    /// reads the way the reader did.
    pub(super) fn text_of(painted: &str) -> String {
        let ansi = regex::Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]").expect("regex");
        ansi.replace_all(painted, "").into_owned()
    }

    /// How many whole frames the capture holds: every frame starts by hiding the
    /// cursor and addressing the home cell.
    pub(super) fn frames(painted: &str) -> usize {
        painted.matches("\x1b[?25l").count()
    }

    /// How long a scripted run may take before the test gives up: every script
    /// ends with a `quit`/`Signal`, so anything near this bound is a loop that
    /// stopped reading its input.
    const RUN_BUDGET: Duration = Duration::from_secs(20);

    /// Held across a test that waits on a **product** read of the repository.
    ///
    /// `open_git_panel` spawns `git` by name, so the read resolves through
    /// `PATH` — and `PATH` is process-global. A peer test swaps it for a
    /// directory of opener stubs and an empty one, to prove that a missing
    /// opener is reported rather than swallowed
    /// (`live::tests::opening_a_link_spawns_the_opener_and_reports_a_missing_one`).
    /// A repository test running beside that window fails with
    /// `failed to spawn git: No such file or directory`, which says nothing
    /// about the code under test. Every test that mutates an environment
    /// variable holds this lock (that is what it is for, see
    /// `crate::test_env::lock`), so holding it here is what makes "the log
    /// installed" a fact about this test instead of about the scheduler.
    ///
    /// Only the tests whose *loop* spawns `git` take it: a fixture resolves
    /// its own git absolutely (`test_env::absolute_tool`) and needs none.
    ///
    /// The tests hold it across their awaits on purpose, which is what
    /// `clippy::await_holding_lock` warns about: the *loop* is what spawns
    /// the read, so the guard has to outlive those awaits, and nothing the
    /// loop runs takes this lock — the deadlock the lint is built for cannot
    /// happen here, while releasing it early reopens the race.
    fn a_peer_may_not_swap_the_environment() -> std::sync::MutexGuard<'static, ()> {
        crate::test_env::lock()
    }

    /// A workbench loop wired exactly as `run` wires it: a mock agent, a terminal,
    /// the two channels the loop reads, and the settings file it writes.
    pub(super) struct Driver {
        pub terminal: Terminal,
        pub notifications: crate::notifications::NotifyCenter,
        pub theme_explicit: bool,
        pub update_rx: Option<tokio::sync::oneshot::Receiver<Option<String>>>,
        pub load_rx: mpsc::UnboundedReceiver<super::super::live::Loaded>,
        pub load_tx: mpsc::UnboundedSender<super::super::live::Loaded>,
        pub workbench: Workbench,
        pub live: Live,
        pub agent_events: mpsc::UnboundedReceiver<AgentEvent>,
        pub agent_tx: mpsc::UnboundedSender<AgentEvent>,
        pub input_rx: mpsc::UnboundedReceiver<TerminalEvent>,
        pub input_tx: mpsc::UnboundedSender<TerminalEvent>,
        pub settings: TuiSettings,
        pub settings_path: std::path::PathBuf,
        /// Every command type the mock agent was asked, in order: what the driver
        /// actually sent, not what it meant to.
        pub seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        /// The same log, with the session each command named: what the driver asked
        /// *about*, not just what it asked.
        pub seen_sessions: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
        /// The stream `Live` attached to. Kept alive but not read by these tests:
        /// the agent events a script injects come through `agent_tx` instead, so a
        /// test can say exactly what the agent said and when.
        _agent_stream: mpsc::UnboundedReceiver<AgentEvent>,
        /// Kept so the directory outlives the driver (the settings file lives in
        /// it).
        _dir: tempfile::TempDir,
    }

    impl Driver {
        /// Attach to a mock agent that answers `pairs` (and nothing else), the way
        /// `run` attaches to a real one.
        pub async fn attach(pairs: &[(&str, &str)]) -> Self {
            let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(pairs);
            let seen_sessions = mock.seen_sessions.clone();
            let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
            let mut driver = Self::attach_at(&addr).await;
            driver.seen = seen;
            driver.seen_sessions = seen_sessions;
            driver
        }

        /// Attach with a rail of two conversations, the state a reader stepping
        /// through their recent work is in.
        pub async fn attach_sessions() -> Self {
            Self::attach(&[
                ("get_state", r#"{"sessionId":"s1","model":"future/old"}"#),
                (
                    "list_sessions",
                    r#"{"sessions":[
                    {"id":"s1","cwd":"/tmp","updatedAtMs":1700000002000,"model":"future/old"},
                    {"id":"s2","cwd":"/tmp","updatedAtMs":1700000001000,"model":"future/old"}
                ]}"#,
                ),
                ("get_session_entries", r#"{"entries":[]}"#),
                (
                    "list_models",
                    r#"{"models":[{"id":"old","label":"old","provider":"future",
                    "thinkingLevel":"default","thinkingLevels":["default"],
                    "defaultThinkingLevel":"default","isDefault":true}]}"#,
                ),
            ])
            .await
        }

        /// Attach to a conversation that is already current, which is the state a
        /// reader coming back to a session is in: the transcript is what the frame
        /// paints, and a prompt goes to the agent rather than starting a session.
        pub async fn attach_conversation() -> Self {
            // A workspace, because the surface reads it: `/git` and the files
            // panel are both rooted there, and a session with no `cwd` takes the
            // repository panel's "no workspace" door instead of a read.
            Self::attach_conversation_in(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).await
        }

        /// The same conversation with its workspace somewhere else. A read of the
        /// workspace (`/git`) answers from `cwd`, so a test that needs a
        /// particular repository points the session at it instead of borrowing
        /// whatever the sources were checked out into.
        pub async fn attach_conversation_in(cwd: &std::path::Path) -> Self {
            Self::attach_conversation_holding(cwd, r#"{"entries":[]}"#).await
        }

        /// The same conversation with a **transcript the agent already holds**.
        ///
        /// `/copy` reads `last_assistant_text` and the chat cursor walks the
        /// links of what was rendered, so both need messages to exist before the
        /// test types anything. A streamed `text_chunk` cannot stand in for
        /// them: `append_to_last_message` appends to an assistant message and,
        /// with no such message above it, drops the text (measured — the copy
        /// then reports "nothing to copy" however much was streamed).
        pub async fn attach_conversation_holding(cwd: &std::path::Path, entries: &str) -> Self {
            let cwd =
                serde_json::to_string(&cwd.to_string_lossy()).expect("a workspace path serialises");
            let state = format!(r#"{{"sessionId":"s1","model":"future/old","cwd":{cwd}}}"#);
            Self::attach(&[
                ("get_state", state.as_str()),
                ("get_session_entries", entries),
                (
                    "list_models",
                    r#"{"models":[{"id":"old","label":"old","provider":"future",
                        "thinkingLevel":"default","thinkingLevels":["default"],
                        "defaultThinkingLevel":"default","isDefault":true}]}"#,
                ),
                // The prompt ack is the one payload the wire keeps snake_case
                // (`payloads_ext::RunAck`); camelCase here is a *different*
                // answer, and one the client reports as unconfirmed. `run_epoch`
                // is not decoration: without it the ack fails to decode, the
                // run is never filed as this client's own, and every
                // `agent_end` about it reads as somebody else's run — which is
                // what the completion and failure notices gate on.
                (
                    "prompt",
                    r#"{"run_id":"r-1","run_epoch":1,"accepted_state":"running"}"#,
                ),
            ])
            .await
        }

        pub async fn attach_at(addr: &str) -> Self {
            let (mut live, agent_stream) = Live::attach(addr, None, false, None)
                .await
                .expect("attach to the mock agent");
            let (agent_tx, agent_events) = mpsc::unbounded_channel();
            let (input_tx, input_rx) = mpsc::unbounded_channel::<TerminalEvent>();
            let (load_tx, load_rx) = mpsc::unbounded_channel();
            // The driver's own wiring (`run`): the reads a step starts answer on
            // this channel instead of inside the loop. Without it a step opens
            // nothing, which is what made this driver's first version look like
            // the surface ignoring the key.
            live.set_load_sender(load_tx.clone());
            let dir = tempfile::tempdir().expect("tempdir");
            let settings_path = dir.path().join("settings.json");
            let mut terminal = crate::terminal::driver_terminal();
            terminal.set_mouse_mode(true);
            Self {
                terminal,
                notifications: crate::notifications::NotifyCenter::default(),
                theme_explicit: true,
                update_rx: None,
                load_rx,
                load_tx,
                workbench: Workbench::new(),
                live,
                agent_events,
                agent_tx,
                input_rx,
                input_tx,
                settings: TuiSettings::default(),
                settings_path,
                seen: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
                seen_sessions: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
                _agent_stream: agent_stream,
                _dir: dir,
            }
        }

        /// Queue raw terminal bytes; the loop parses them exactly as it parses a
        /// real read.
        pub fn data(&self, bytes: &str) {
            let _ = self.input_tx.send(TerminalEvent::Data(bytes.to_string()));
        }

        /// Queue a signal (the arm that leaves without looking at the surface).
        pub fn signal(&self) {
            let _ = self.input_tx.send(TerminalEvent::Signal);
        }

        pub fn frame(&self) -> super::super::layout::Frame {
            frame_of(&self.terminal, &self.workbench)
        }

        /// The conversation on screen, as the surface names it (`""` on the
        /// launchpad).
        pub fn session_id(&mut self) -> String {
            let frame = self.frame();
            self.live.data(&frame).session.id.clone()
        }

        /// Hand `quit` one key, the way the loop hands it one parsed event, and
        /// report whether it asked to leave.
        ///
        /// The event is built from the key's own spelling — what the action
        /// table is keyed on. Going through terminal bytes here would turn every
        /// chord this helper does not spell into a *paste*, and the sweep below
        /// over the whole binding table would then assert nothing about the
        /// chords it could not spell. The byte-to-key round trip is
        /// `events_of`'s business and is covered where it lives.
        pub async fn quit_key(&mut self, chord: &str) -> bool {
            self.quit(WbEvent::Key(chord.to_string()))
                .await
                .expect("the chord does not fail")
        }

        /// Run the loop until it returns, with what it painted. The script must end
        /// it (a `ctrl+x`, a `/quit`, a `Signal`).
        pub async fn run(&mut self) -> (Result<(), String>, String) {
            let outcome = self.step(RUN_BUDGET).await;
            (
                outcome.expect("the script must end the loop"),
                crate::terminal::capture::take(),
            )
        }

        /// Let the loop run for `budget`, returning what it painted. A script that
        /// injects several wakes (agent events, loaded pages, a resize) has no key
        /// to end on — each wake is one iteration, and the budget is what stops it
        /// after they are all handled.
        pub async fn pump(&mut self, budget: Duration) -> String {
            let _ = self.step(budget).await;
            crate::terminal::capture::take()
        }

        /// Run the loop in short slices until `done` holds, up to `budget`.
        ///
        /// A fixed sleep is a guess about how long a read takes, and under a
        /// loaded machine it is the wrong guess: this waits for the *effect*
        /// instead (a section installed, a notice set), which is what the test
        /// is about. Returns whether it arrived in time.
        pub async fn wait_for(
            &mut self,
            budget: Duration,
            mut done: impl FnMut(&Driver) -> bool,
        ) -> bool {
            let deadline = tokio::time::Instant::now() + budget;
            loop {
                if done(self) {
                    return true;
                }
                if tokio::time::Instant::now() >= deadline {
                    return false;
                }
                let _ = self.pump(Duration::from_millis(40)).await;
            }
        }

        /// One bounded run of the loop: `Some(outcome)` when the script ended it
        /// itself, `None` when the budget ran out first.
        async fn step(&mut self, budget: Duration) -> Option<Result<(), String>> {
            crate::terminal::capture::start();
            // `DriverIo` takes the loaded-page receiver by value, so the driver
            // lends it to the loop and takes it back afterwards. One channel for
            // the driver's whole life is the point: a read started before a run
            // that answers *during* it (or between two slices of `wait_for`) is
            // delivered, instead of landing in a receiver the previous run
            // dropped.
            let load_rx = std::mem::replace(&mut self.load_rx, mpsc::unbounded_channel().1);
            let mut io = DriverIo {
                terminal: &mut self.terminal,
                notifications: &mut self.notifications,
                theme_explicit: &mut self.theme_explicit,
                update_rx: &mut self.update_rx,
                load_rx,
            };
            let outcome = tokio::time::timeout(
                budget,
                event_loop(
                    &mut io,
                    &mut self.workbench,
                    &mut self.live,
                    &mut self.agent_events,
                    &mut self.input_rx,
                    &mut self.settings,
                    &self.settings_path,
                ),
            )
            .await
            .ok();
            // Take the receiver back: the next run (and anything the loop did
            // not drain) keeps flowing through the same channel.
            self.load_rx = io.load_rx;
            outcome
        }

        /// Hand `quit` one event, the way the loop does, and report whether it asked
        /// to leave. This is the action table without a loop around it: a test can
        /// ask about one chord without arranging a whole session first.
        pub async fn quit(&mut self, event: WbEvent) -> Result<bool, String> {
            let frame = self.frame();
            let mut io = InputIo {
                terminal: &mut self.terminal,
                settings: &mut self.settings,
                settings_path: &self.settings_path,
                theme_explicit: &mut self.theme_explicit,
            };
            quit(&mut self.workbench, &mut self.live, event, &frame, &mut io).await
        }

        /// Type `text` key by key through the driver: what a reader typing into the
        /// draft does, one terminal read at a time.
        pub async fn type_text(&mut self, text: &str) -> bool {
            let mut leave = false;
            for character in text.chars() {
                let mut buffer = [0u8; 4];
                let bytes = character.encode_utf8(&mut buffer);
                for event in super::events_of(bytes) {
                    leave |= self.quit(event).await.expect("a key does not fail");
                }
            }
            leave
        }

        /// Type a whole command (`"/files"`) and submit it. The line is pasted, not
        /// typed character by character: a bare `/` opens the command palette, so a
        /// typed line never reaches the draft — paste does, and the surface answers
        /// it on `enter` like any other line.
        pub async fn command(&mut self, command: &str) -> bool {
            let mut leave = false;
            self.quit(WbEvent::Paste(command.to_string()))
                .await
                .map(|asked| leave |= asked)
                .expect("a paste does not fail");
            let enter = super::events_of("\r").into_iter().next().expect("enter");
            leave || self.quit(enter).await.expect("enter does not fail")
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::workbench::live::Loaded;

        /// A frame is painted before the loop waits for anything, and `ctrl+x`
        /// leaves. The first assertion is the one the loop's own comment makes: the
        /// screen is never stale while the loop waits.
        #[tokio::test]
        async fn a_first_frame_is_painted_before_any_input_and_ctrl_x_leaves() {
            let mut driver = Driver::attach(&[]).await;
            driver.data("\u{18}");
            let (outcome, painted) = driver.run().await;
            assert!(outcome.is_ok(), "{outcome:?}");
            assert_eq!(frames(&painted), 1, "one frame, then the key left");
            let text = text_of(&painted);
            assert!(text.contains("FUTURE OS"), "{text:?}");
            assert!(text.contains("New Chat"), "{text:?}");
            assert!(
                painted.contains("\x1b[?25h"),
                "the frame puts the cursor back where the draft is: {painted:?}"
            );
        }

        /// A signal leaves without waiting for a key, and the loop does not report an
        /// error: the terminal is restored by `run`, which is not this path.
        #[tokio::test]
        async fn a_signal_ends_the_loop() {
            let mut driver = Driver::attach(&[]).await;
            driver.signal();
            let (outcome, _painted) = driver.run().await;
            assert!(outcome.is_ok(), "{outcome:?}");
        }

        /// The clock is a clock, not a paint: with nothing moving on screen the loop
        /// wakes on its ticker and paints nothing more. This is the property
        /// `TickClock`'s own test states, measured here through the loop that has to
        /// honour it.
        #[tokio::test]
        async fn an_idle_loop_wakes_on_its_ticker_without_painting_again() {
            let mut driver = Driver::attach(&[]).await;
            let painted = driver.pump(Duration::from_millis(400)).await;
            assert_eq!(
                frames(&painted),
                1,
                "a tick with nothing to show repaints the frame: {painted:?}"
            );
        }

        /// What the reader types is painted in the draft on the next frame, and the
        /// draft is what the loop asked the surface for — not a second copy of the
        /// text.
        #[tokio::test]
        async fn typed_text_is_painted_into_the_draft() {
            let mut driver = Driver::attach(&[]).await;
            driver.data("hi");
            let painted = driver.pump(Duration::from_millis(400)).await;
            let text = text_of(&painted);
            assert!(
                text.contains("hi"),
                "the draft did not take the keys: {text:?}"
            );
            assert!(frames(&painted) >= 2, "typing is a repaint: {painted:?}");
        }

        /// `enter` on a non-empty draft is a `prompt` to the agent — the command the
        /// driver exists to send.
        #[tokio::test]
        async fn enter_sends_the_draft_as_a_prompt() {
            let mut driver = Driver::attach_conversation().await;
            // Two reads, the way a terminal delivers typed keys: the text first,
            // then the `enter` that submits it. One read containing both is a
            // *paste*, and a paste is inserted, never submitted.
            driver.data("hello there");
            driver.data("\r");
            let painted = driver.pump(Duration::from_millis(700)).await;
            // The message is in the transcript the moment it is sent: the reader
            // sees their own text before the agent has answered.
            assert!(
                text_of(&painted).contains("hello there"),
                "the sent message was not painted: {painted:?}"
            );
            let seen = driver.seen.lock().expect("seen").clone();
            assert!(
                seen.iter().any(|kind| kind == "prompt"),
                "no prompt reached the agent: {seen:?}"
            );
        }

        /// An agent event the loop is woken for is applied and painted: a streamed
        /// text chunk is on screen after the wake-up, without a key press.
        #[tokio::test]
        async fn a_streamed_text_chunk_is_painted_when_it_arrives() {
            let mut driver = Driver::attach_conversation().await;
            let session = driver.session_id();
            for (kind, data) in [
                ("agent_start", serde_json::json!({})),
                ("text_chunk", serde_json::json!({"text": "streamed answer"})),
                ("agent_end", serde_json::json!({})),
            ] {
                let _ = driver.agent_tx.send(AgentEvent {
                    r#type: kind.into(),
                    data,
                    session_id: Some(session.clone()),
                    run_id: Some("run-1".into()),
                    ..Default::default()
                });
            }
            let painted = driver.pump(Duration::from_millis(700)).await;
            assert!(
                text_of(&painted).contains("streamed answer"),
                "the chunk never reached the screen: {painted:?}"
            );
            assert!(frames(&painted) >= 2);
        }

        /// A burst of agent events is drained into one frame: the loop applies the
        /// token deltas that arrived while it was busy and paints once, not once per
        /// token.
        #[tokio::test]
        async fn a_burst_of_agent_events_costs_one_frame() {
            let mut driver = Driver::attach_conversation().await;
            let session = driver.session_id();
            let _ = driver.agent_tx.send(AgentEvent {
                r#type: "agent_start".into(),
                session_id: Some(session.clone()),
                run_id: Some("run-1".into()),
                ..Default::default()
            });
            for index in 0..8 {
                let _ = driver.agent_tx.send(AgentEvent {
                    r#type: "text_chunk".into(),
                    data: serde_json::json!({"text": format!("chunk{index} ")}),
                    session_id: Some(session.clone()),
                    run_id: Some("run-1".into()),
                    ..Default::default()
                });
            }
            let _ = driver.agent_tx.send(AgentEvent {
                r#type: "agent_end".into(),
                session_id: Some(session.clone()),
                run_id: Some("run-1".into()),
                ..Default::default()
            });
            let painted = driver.pump(Duration::from_millis(700)).await;
            let text = text_of(&painted);
            assert!(
                text.contains("chunk7"),
                "the burst was not applied: {text:?}"
            );
        }

        /// A read the surface started without waiting for it is installed on its own
        /// wake-up, and a page that names another conversation is dropped instead of
        /// being prepended to the one on screen.
        ///
        /// Both answers go into one run: the first installs and repaints, the second
        /// is stale and must paint nothing — so the frame count is the count of
        /// answers that were actually used.
        #[tokio::test]
        async fn a_loaded_page_is_installed_and_a_stale_one_is_dropped() {
            let mut driver = Driver::attach_conversation().await;
            let current = driver.session_id();
            let _ = driver
                .load_tx
                .send(Loaded::State(Ok(crate::rpc::types::RpcSessionState {
                    session_id: "s2".into(),
                    ..Default::default()
                })));
            let _ = driver
                .load_tx
                .send(Loaded::Older(crate::workbench::live::OlderPage {
                    session_id: format!("{current}-other"),
                    before: 40,
                    page: Err("unused".into()),
                }));
            let painted = driver.pump(Duration::from_millis(500)).await;
            assert_eq!(
                frames(&painted),
                2,
                "the first frame, plus the one page that installed (the stale page owed nothing): {painted:?}"
            );
        }

        /// The startup update check answers whenever it answers; the loop paints the
        /// notice on its own arm, after the first frame.
        #[tokio::test]
        async fn the_update_check_arms_a_notice_and_disarms_itself() {
            let mut driver = Driver::attach(&[]).await;
            let (tx, rx) = tokio::sync::oneshot::channel();
            driver.update_rx = Some(rx);
            tx.send(Some("a newer future is out".into()))
                .expect("send the notice");
            let painted = driver.pump(Duration::from_millis(400)).await;
            assert!(driver.live.notice.is_some(), "no notice was armed");
            assert!(
                text_of(&painted).contains("a newer future is out"),
                "the notice was not painted: {painted:?}"
            );
            assert!(driver.update_rx.is_none(), "the arm must disarm itself");
        }

        /// A resize wake-up that leaves the frame identical paints nothing: the
        /// loop composes for the size it has now, and the incremental cache drops a
        /// frame that did not change. A resize look-alike is therefore free, which is
        /// what keeps a terminal that reports its size on every read from costing a
        /// full repaint a tick.
        #[tokio::test]
        async fn a_resize_that_changes_nothing_paints_nothing() {
            let mut driver = Driver::attach(&[]).await;
            let _ = driver.input_tx.send(TerminalEvent::Resize);
            let painted = driver.pump(Duration::from_millis(300)).await;
            assert_eq!(
                frames(&painted),
                1,
                "an unchanged frame was repainted for a resize: {painted:?}"
            );
        }

        /// `ctrl+x` is the one chord that leaves; every other chord is the surface's.
        #[tokio::test]
        async fn ctrl_x_asks_to_leave_and_another_chord_does_not() {
            let mut driver = Driver::attach(&[]).await;
            assert!(!driver.quit_key("ctrl+c").await, "idle ctrl+c just reports");
            assert!(driver.quit_key("ctrl+x").await, "ctrl+x leaves");
        }

        /// The rail's chord opens the panel and the same chord closes it: one key
        /// moves the surface in and out, and it never leaves the screen in a state
        /// the next key cannot undo.
        #[tokio::test]
        async fn the_rail_chord_opens_and_closes_the_same_panel() {
            let mut driver = Driver::attach(&[]).await;
            assert!(!driver.quit_key("ctrl+d").await, "ctrl+d does not leave");
            assert_eq!(
                driver.workbench.overlay,
                Some(crate::workbench::Overlay::PanelFull),
                "ctrl+d did not open the rail"
            );
            assert!(!driver.quit_key("escape").await, "escape does not leave");
            assert_eq!(
                driver.workbench.overlay, None,
                "the panel did not close on escape"
            );
        }

        /// The typed door to a panel: the command is answered by the surface, and
        /// the panel is open on the frame after it.
        #[tokio::test]
        async fn a_submitted_command_opens_its_panel_without_reaching_the_agent() {
            let mut driver = Driver::attach(&[]).await;
            let leave = driver.command("/files").await;
            assert!(!leave);
            assert_eq!(
                driver.workbench.overlay,
                Some(crate::workbench::Overlay::Files),
                "the files panel did not open"
            );
            let seen = driver.seen.lock().expect("seen").clone();
            assert!(
                !seen.iter().any(|kind| kind == "prompt"),
                "a panel command reached the agent as a prompt: {seen:?}"
            );
        }
        /// Every overlay the surface can put up is painted when it opens, and none
        /// of them takes the loop with it. One pass over the enum: an overlay that
        /// panics or paints nothing is a door the reader cannot come back from.
        #[tokio::test]
        async fn every_overlay_paints_a_frame_when_it_opens() {
            use crate::workbench::{picker::PickerKind, Overlay};

            // What the frame looks like with no overlay on it: an overlay that
            // painted this instead would be a door that opens nothing.
            let mut bare = Driver::attach_conversation().await;
            let baseline = bare.pump(Duration::from_millis(250)).await;

            let overlays = [
                Overlay::Help,
                Overlay::Palette,
                Overlay::PanelFull,
                Overlay::Picker(PickerKind::Model),
                Overlay::Queue,
                Overlay::Approval,
                Overlay::Diff,
                Overlay::Git,
                Overlay::Files,
                Overlay::Fork,
                Overlay::Worktree,
                Overlay::DeleteSession,
                Overlay::FilePreview,
                Overlay::Preferences,
                Overlay::Keybindings,
            ];
            for overlay in overlays {
                let mut driver = Driver::attach_conversation().await;
                driver.workbench.overlay = Some(overlay);
                let painted = driver.pump(Duration::from_millis(250)).await;
                assert!(
                    frames(&painted) >= 1,
                    "{overlay:?} painted nothing: {painted:?}"
                );
                assert_ne!(
                    painted, baseline,
                    "{overlay:?} paints the same screen as no overlay at all"
                );
            }
        }

        /// Only the quit chord leaves. Every other configurable action is the
        /// surface's business, and one that ended the process would be a key nobody
        /// dares press — this walks the whole binding table in one pass so a new
        /// action cannot quietly claim the quit path.
        #[tokio::test]
        async fn only_the_quit_chord_leaves_the_workbench() {
            use crate::workbench::preferences::{Action, Keybindings};

            let keys = Keybindings::default();
            for action in crate::workbench::preferences::ACTIONS {
                if action == Action::Quit {
                    continue;
                }
                let chord = keys.key(action);
                if chord.is_empty() {
                    continue;
                }
                let mut driver = Driver::attach_conversation().await;
                assert!(
                    !driver.quit_key(chord).await,
                    "{chord} ({}) left the workbench",
                    action.id()
                );
            }
        }

        /// The typed door to the surface's own verbs: each one is answered by the
        /// surface — it does not leave, and it does not become a prompt. `/quit` is
        /// the one verb that leaves, which is the other half of the claim.
        #[tokio::test]
        async fn a_surface_verb_is_answered_locally_and_only_quit_leaves() {
            use crate::workbench::{COMMANDS, TYPED_ONLY};

            let verbs: Vec<&str> = COMMANDS
                .iter()
                .map(|(command, _)| *command)
                .chain(TYPED_ONLY.iter().map(|(command, _)| *command))
                .collect();
            for verb in verbs {
                let mut driver = Driver::attach_conversation().await;
                let leave = driver.command(verb).await;
                if verb == "/quit" {
                    assert!(leave, "/quit did not leave");
                    continue;
                }
                assert!(!leave, "{verb} left the workbench");
                let seen = driver.seen.lock().expect("seen").clone();
                assert!(
                    !seen.iter().any(|kind| kind == "prompt"),
                    "{verb} reached the agent as a prompt: {seen:?}"
                );
            }
        }

        /// `ctrl+j` is a newline in the draft, `enter` is what sends: a two-line
        /// prompt stays one prompt with both lines in it, which is the property the
        /// composer's own hint row promises.
        #[tokio::test]
        async fn a_newline_chord_extends_the_draft_instead_of_sending_it() {
            let mut driver = Driver::attach_conversation().await;
            assert!(!driver.type_text("first line").await);
            assert!(!driver.type_text("\n").await);
            assert!(!driver.type_text("second line").await);
            let painted = driver.pump(Duration::from_millis(300)).await;
            let text = text_of(&painted);
            assert!(text.contains("first line"), "{text:?}");
            assert!(text.contains("second line"), "{text:?}");
            let seen = driver.seen.lock().expect("seen").clone();
            assert!(
                !seen.iter().any(|kind| kind == "prompt"),
                "a newline sent the prompt: {seen:?}"
            );
        }
    }
    /// A step key moves the rail's cursor and opens the conversation it lands
    /// on — through the settle window, because that read is not cached: the
    /// switch is what the loop's own timer arm does, not a side effect of the
    /// key press.
    #[tokio::test]
    async fn a_session_step_opens_the_conversation_it_stops_on() {
        let mut driver = Driver::attach_sessions().await;
        assert_eq!(driver.session_id(), "s1", "attached to the newest");
        // `shift+down`: one step towards the older conversation.
        driver.data("\x1b[1;2B");
        let painted = driver.pump(Duration::from_millis(600)).await;
        assert!(
            painted.contains("\x1b[?25l"),
            "the step painted nothing: {painted:?}"
        );
        // The switch is the read the step owes: the state and the tail page of
        // the conversation it landed on. (This mock answers `get_state` with the
        // conversation it started on for every caller, so the *request* is what
        // says which one the step opened.)
        let asked = driver.seen_sessions.lock().expect("seen").clone();
        assert!(
            asked
                .iter()
                .any(|(command, session)| command == "get_state" && session == "s2"),
            "the step never opened s2: {asked:?}"
        );
    }
    /// A rebound action still reaches the driver's own arm: the four verbs with
    /// no shipped chord (copy, queue, refresh, mouse) are bound here and each is
    /// answered — never by leaving the workbench.
    #[tokio::test]
    async fn a_rebound_action_reaches_its_own_arm() {
        use crate::workbench::preferences::{Action, Keybindings};

        let mut keys = Keybindings::default();
        for (action, chord) in [
            (Action::Copy, "ctrl+y"),
            (Action::Queue, "ctrl+e"),
            (Action::Refresh, "ctrl+b"),
            (Action::Mouse, "ctrl+w"),
        ] {
            keys.bind(action, chord).expect("a spare chord");
        }
        for chord in ["ctrl+y", "ctrl+e", "ctrl+b", "ctrl+w"] {
            let mut driver = Driver::attach_conversation().await;
            driver.workbench.keys = keys.clone();
            assert!(!driver.quit_key(chord).await, "{chord} left the workbench");
        }
        // The mouse verb is a toggle, and the surface is what remembers it.
        let mut driver = Driver::attach_conversation().await;
        driver.workbench.keys = keys;
        let before = driver.workbench.mouse_capture;
        assert!(!driver.quit_key("ctrl+w").await);
        assert_ne!(
            driver.workbench.mouse_capture, before,
            "the rebound mouse chord did not toggle mouse capture"
        );
        assert!(
            driver.settings.mouse_capture == Some(driver.workbench.mouse_capture),
            "the toggle was not written to the settings"
        );
    }

    /// A settings file that cannot be written is reported on the surface: a
    /// preference the reader just changed must not look saved when it was not.
    #[tokio::test]
    async fn an_unwritable_settings_file_is_reported_not_swallowed() {
        let mut driver = Driver::attach_conversation().await;
        // A regular file standing where the directory has to be: every write
        // below it fails with ENOTDIR, which is the one failure the writer
        // cannot create its way out of.
        let blocker = driver
            .settings_path
            .parent()
            .expect("a parent")
            .join("blocker");
        std::fs::write(&blocker, b"not a directory").expect("write the blocker");
        driver.settings_path = blocker.join("settings.json");

        // `/mouse` is the verb that both flips the preference and writes it, so
        // a failed write is reported on the notice row.
        assert!(!driver.command("/mouse").await, "/mouse does not leave");
        let notice = driver.live.notice.clone().expect("a notice");
        assert!(!notice.is_empty(), "the failed save was silent: {notice:?}");
    }

    mod overlay_navigation {
        use super::*;

        /// Navigation inside an overlay belongs to the overlay: the arrows, the paging
        /// keys and `home`/`end` move *inside* the box and none of them leaves it.
        ///
        /// This is the door a reader uses most and the one a stray `WbAction` would
        /// break silently — a key that closed the box would look like a mis-press and
        /// not like a bug.
        #[tokio::test]
        async fn navigation_inside_an_overlay_moves_without_closing_it() {
            use crate::workbench::{picker::PickerKind, Overlay};

            let overlays = [
                Overlay::Help,
                Overlay::Queue,
                Overlay::Git,
                Overlay::Files,
                Overlay::Worktree,
                Overlay::DeleteSession,
                Overlay::Fork,
                Overlay::PanelFull,
                Overlay::Picker(PickerKind::Model),
                Overlay::Picker(PickerKind::Sessions),
            ];
            for overlay in overlays {
                let mut driver = Driver::attach_conversation().await;
                driver.workbench.overlay = Some(overlay);
                for key in ["down", "up", "pageDown", "pageUp", "home", "end"] {
                    assert!(
                        !driver.quit_key(key).await,
                        "{key} left the workbench inside {overlay:?}"
                    );
                    assert_eq!(
                        driver.workbench.overlay,
                        Some(overlay),
                        "{key} closed {overlay:?}"
                    );
                }
            }
        }

        /// A picker is a list with a highlight of its own: `down` moves the row the
        /// box would commit, and `escape` leaves without committing anything.
        #[tokio::test]
        async fn a_picker_moves_its_highlight_and_escape_commits_nothing() {
            use crate::workbench::{picker::PickerKind, Overlay};

            let mut driver = Driver::attach(&[
                ("get_state", r#"{"sessionId":"s1","model":"future/old"}"#),
                ("get_session_entries", r#"{"entries":[]}"#),
                (
                    "list_models",
                    r#"{"models":[
                    {"id":"old","label":"old","provider":"future","thinkingLevel":"default",
                     "thinkingLevels":["default"],"defaultThinkingLevel":"default",
                     "isDefault":true},
                    {"id":"new","label":"new","provider":"future","thinkingLevel":"default",
                     "thinkingLevels":["default"],"defaultThinkingLevel":"default"}
                ]}"#,
                ),
            ])
            .await;
            let frame = driver.frame();
            let data = driver.live.data(&frame);
            driver.workbench.open_picker(PickerKind::Model, &data);
            assert_eq!(
                driver.workbench.overlay,
                Some(Overlay::Picker(PickerKind::Model))
            );
            let before = driver.workbench.picker.index;
            assert!(!driver.quit_key("down").await, "down does not leave");
            assert_ne!(
                driver.workbench.picker.index, before,
                "the highlight did not move down the list"
            );
            assert!(!driver.quit_key("escape").await, "escape does not leave");
            assert_eq!(
                driver.workbench.overlay, None,
                "escape did not close the box"
            );
            assert!(
                driver.live.pending_model().is_none(),
                "escape committed the highlighted row"
            );
        }
    }

    mod repository_keys {
        use super::*;
        use crate::workbench::run::driver_tests::workspace_panel::repo_with_two_commits;

        /// The repository browser's own keys: the tabs are numbered, `tab` moves
        /// between the list and the patch, and the horizontal pan is the pane's —
        /// the cursor does not move out from under the reader while they pan.
        #[allow(clippy::await_holding_lock)]
        #[tokio::test]
        async fn the_repository_tabs_focus_and_pan_are_its_own_keys() {
            let _env = a_peer_may_not_swap_the_environment();
            let mut driver = Driver::attach_conversation().await;
            assert!(!driver.quit_key("ctrl+g").await);
            assert!(
                driver
                    .wait_for(Duration::from_secs(20), |driver| driver
                        .live
                        .git()
                        .is_some())
                    .await,
                "the log never installed"
            );
            let log = driver.live.git().cloned().expect("the log installed");
            assert_eq!(
                log.section,
                crate::workbench::state::GitSection::Commits,
                "the panel opens on the log"
            );

            // `2` is the next tab, and the read that follows lands on it.
            assert!(!driver.quit_key("2").await, "a tab key does not leave");
            let want = crate::workbench::state::GitSection::ALL[1];
            assert!(
                driver
                    .wait_for(Duration::from_secs(20), |driver| driver
                        .live
                        .git()
                        .is_some_and(|view| view.section == want))
                    .await,
                "the refs never installed"
            );
            let refs = driver.live.git().cloned().expect("the refs installed");
            assert_ne!(
                refs.section, log.section,
                "the numbered tab did not switch the section"
            );

            // `tab` moves the keys between the list and the patch…
            let was = driver.workbench.git_detail_focus;
            assert!(!driver.quit_key("tab").await);
            assert_ne!(
                driver.workbench.git_detail_focus, was,
                "tab did not move the focus between the panes"
            );
            // …and with the focus on the patch, `down` never moves the list's
            // cursor: the arrows belong to the pane the keys are in.
            let cursor = driver.workbench.workflow_index;
            let scroll = driver.workbench.overlay_scroll;
            assert!(!driver.quit_key("down").await);
            assert_eq!(
                driver.workbench.workflow_index, cursor,
                "a panned pane moved the list's cursor"
            );
            assert!(
                driver.workbench.overlay_scroll >= scroll,
                "scrolling the patch moved it backwards"
            );

            // `1` is the log again, whichever tab the reader left open.
            assert!(!driver.quit_key("1").await);
            assert!(
                driver
                    .wait_for(Duration::from_secs(20), |driver| driver
                        .live
                        .git()
                        .is_some_and(
                            |view| view.section == crate::workbench::state::GitSection::Commits
                        ))
                    .await,
                "the first tab did not land back on the log"
            );

            // `r` re-reads the section on screen.
            assert!(!driver.quit_key("r").await, "refresh does not leave");
            let _ = driver.pump(Duration::from_millis(200)).await;
            assert!(
                driver.live.git().is_some(),
                "the refresh dropped the panel's view"
            );

            // `escape` closes it, and the next key belongs to the surface again.
            assert!(!driver.quit_key("escape").await);
            assert_eq!(driver.workbench.overlay, None, "escape did not close it");
        }

        /// A section with nothing to list still opens: the panel is a view of the
        /// repository, and "no stashes" is an answer, not an error.
        ///
        /// The repository is the fixture's own, not wherever the sources are
        /// checked out. It used to be `attach_conversation()` — a session rooted
        /// at `CARGO_MANIFEST_DIR` — which made the 改动 section's *rows* a fact
        /// about the machine: a clean checkout painted a box with nothing to
        /// list and a dirty one painted a working-tree patch, so two lines of
        /// `screen.rs` (`git_box`'s "select an entry" placeholder and the
        /// metadata colour of git's own lines) were covered by whichever state
        /// the tree happened to be in. Measured: with one tracked file edited,
        /// `screen.rs:2491` reads 1 from this test alone; clean, it reads 0.
        #[allow(clippy::await_holding_lock)]
        #[tokio::test]
        async fn every_repository_section_opens_without_an_error() {
            let _env = a_peer_may_not_swap_the_environment();
            let sections = crate::workbench::state::GitSection::ALL;
            for (index, section) in sections.iter().enumerate() {
                let repo = repo_with_two_commits();
                let mut driver = Driver::attach_conversation_in(repo.path()).await;
                assert!(!driver.quit_key("ctrl+g").await);
                assert!(
                    driver
                        .wait_for(Duration::from_secs(20), |driver| driver
                            .live
                            .git()
                            .is_some())
                        .await,
                    "the log never installed"
                );
                let key = (index + 1).to_string();
                assert!(!driver.quit_key(&key).await, "{key} left the workbench");
                assert!(
                    driver
                        .wait_for(Duration::from_secs(20), |driver| driver
                            .live
                            .git()
                            .is_some_and(|view| view.section == *section))
                        .await,
                    "tab {key} never landed on {section:?}"
                );
                let view = driver.live.git().cloned().expect("a view");
                assert_eq!(
                    view.section, *section,
                    "tab {key} did not land on {section:?}"
                );
                let name = section_name(*section);
                assert!(
                    view.error.is_none(),
                    "{name} could not be read in a repository: {:?}",
                    view.error
                );
            }
        }

        fn section_name(section: crate::workbench::state::GitSection) -> &'static str {
            match section {
                crate::workbench::state::GitSection::Commits => "the log",
                _ => "a repository section",
            }
        }
    }

    mod workspace_panel {
        use super::*;
        use crate::workbench::Overlay;

        /// A throwaway repository whose log has two commits, so a test that moves
        /// a cursor in it states its own premise instead of inheriting the
        /// checkout's history: CI clones one commit deep (`actions/checkout`
        /// without `fetch-depth: 0`) and a source tarball has no repository at
        /// all, so neither is a place to measure "the second row".
        pub(super) fn repo_with_two_commits() -> tempfile::TempDir {
            let dir = tempfile::tempdir().expect("a temp directory");
            // Pinned, not `git` from `PATH`: a sibling test in this binary swaps
            // the process-global `PATH` while it runs, and a spawn in that window
            // fails with ENOENT (`docs/testing/flaky-tui.md` §13.4, rule 2).
            let git_program = crate::test_env::absolute_tool("git");
            let git = |args: &[&str]| {
                let output = std::process::Command::new(&git_program)
                    .arg("-C")
                    .arg(dir.path())
                    .args(args)
                    .output()
                    .expect("run git");
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                assert!(output.status.success(), "git {args:?}: {stderr}");
            };
            git(&["init", "-q", "-b", "main"]);
            git(&["config", "user.email", "tui@example.invalid"]);
            git(&["config", "user.name", "TUI test"]);
            std::fs::write(dir.path().join("a.txt"), "one\n").expect("write the first file");
            git(&["add", "."]);
            git(&["commit", "-qm", "first commit"]);
            std::fs::write(dir.path().join("a.txt"), "two\n").expect("write the second file");
            git(&["commit", "-qam", "second commit"]);
            dir
        }

        /// `ctrl+g` opens the repository browser for the session's own workspace
        /// and the read lands on its own wake-up: the panel paints the rows the
        /// disk produced, not an empty list.
        ///
        /// The fixture's session carries a `cwd` — without one `open_git_panel`
        /// takes its "no workspace" door and never reads anything, which is why
        /// that whole path measured zero before this test. The repository is the
        /// fixture's own, so "the log of a repository with history" is a fact of
        /// the test rather than of the checkout it runs in.
        #[allow(clippy::await_holding_lock)]
        #[tokio::test]
        async fn the_repository_panel_reads_the_session_workspace() {
            let _env = a_peer_may_not_swap_the_environment();
            let repo = repo_with_two_commits();
            let mut driver = Driver::attach_conversation_in(repo.path()).await;
            assert!(!driver.quit_key("ctrl+g").await, "ctrl+g does not leave");
            assert_eq!(
                driver.workbench.overlay,
                Some(Overlay::Git),
                "ctrl+g did not open the repository panel"
            );
            assert!(
                driver
                    .wait_for(Duration::from_secs(20), |driver| driver
                        .live
                        .git()
                        .is_some())
                    .await,
                "the repository read never landed"
            );
            // The read landing owes a frame; paint it before reading the screen.
            let painted = driver.pump(Duration::from_millis(200)).await;
            let view = driver
                .live
                .git()
                .cloned()
                .expect("the read installed a view");
            assert!(
                view.error.is_none(),
                "the workspace is a repository: {:?}",
                view.error
            );
            assert!(
                !view.rows.is_empty(),
                "the commit log of a repository with history is empty"
            );
            assert!(
                !view.repo.is_empty() && text_of(&painted).contains(&view.repo),
                "the panel does not name the repository it read ({:?})",
                view.repo
            );
        }

        /// Moving inside the repository panel is a detail read: the row under the
        /// cursor is what the bottom half shows, and the surface decides which row
        /// that is.
        ///
        /// The repository is the fixture's own (`repo_with_two_commits`): the log
        /// has to have a second row to move onto, and that is a property of this
        /// test's repository rather than of wherever the sources were checked out.
        #[allow(clippy::await_holding_lock)]
        #[tokio::test]
        async fn moving_in_the_repository_panel_reads_the_row_under_the_cursor() {
            let _env = a_peer_may_not_swap_the_environment();
            let repo = repo_with_two_commits();
            let mut driver = Driver::attach_conversation_in(repo.path()).await;
            assert!(!driver.quit_key("ctrl+g").await);
            // The read is a spawned `git` walk, so how long it takes is the
            // machine's business: wait for the effect (a view installed) rather
            // than for a fixed number of milliseconds. 700ms was a guess that a
            // loaded Windows runner loses — the panel then had nothing to read a
            // row from, and the assertion below failed as "a view".
            assert!(
                driver
                    .wait_for(Duration::from_secs(20), |driver| driver
                        .live
                        .git()
                        .is_some())
                    .await,
                "the repository read never landed"
            );
            let first = driver.live.git().cloned().expect("a view");
            assert!(first.rows.len() > 1, "no second row to move to");

            // `down` inside the panel is a selection, not a draft key.
            assert!(!driver.quit_key("down").await, "down does not leave");
            // The row under the cursor is a second read of its own.
            assert!(
                driver
                    .wait_for(Duration::from_secs(20), |driver| driver
                        .live
                        .git()
                        .is_some_and(|view| view.selected != first.selected))
                    .await,
                "the cursor did not move off the first row"
            );
            assert!(
                driver
                    .wait_for(Duration::from_secs(20), |driver| driver
                        .live
                        .git()
                        .is_some_and(
                            |view| !view.detail.is_empty() || view.error.is_some()
                        ))
                    .await,
                "the row under the cursor was never read"
            );
            let painted = driver.pump(Duration::from_millis(300)).await;
            let moved = driver.live.git().cloned().expect("a view");
            assert_ne!(
                moved.selected, first.selected,
                "the cursor did not move off the first row"
            );
            assert!(
                !moved.detail.is_empty() || moved.error.is_some(),
                "the row under the cursor was never read"
            );
            assert!(frames(&painted) >= 1);
        }

        /// A parked approval owns `y`/`n`/`enter` on the base surface: `enter`
        /// opens the card rather than answering blind, and the letter is the
        /// answer the agent is told.
        #[tokio::test]
        async fn a_parked_approval_is_answered_by_its_own_keys() {
            let mut driver = Driver::attach(&[
                (
                    "get_state",
                    r#"{"sessionId":"s1","model":"future/old","pendingApprovals":[
                        {"approvalRequestId":"a-1","toolName":"bash","kind":"command","riskLevel":"high"}]}"#,
                ),
                ("get_session_entries", r#"{"entries":[]}"#),
                (
                    "list_models",
                    r#"{"models":[{"id":"old","label":"old","provider":"future",
                        "thinkingLevel":"default","thinkingLevels":["default"],
                        "defaultThinkingLevel":"default","isDefault":true}]}"#,
                ),
                ("approval_decision", r#"{"success":true}"#),
            ])
            .await;
            assert!(
                driver.live.approval().is_some(),
                "the state named a pending approval"
            );

            // `enter` opens the card instead of answering blind.
            assert!(!driver.quit_key("enter").await, "enter does not leave");
            assert_eq!(
                driver.workbench.overlay,
                Some(Overlay::Approval),
                "enter did not open the approval card"
            );

            // `y` on the base surface is the answer, and it clears the card.
            driver.workbench.overlay = None;
            assert!(!driver.quit_key("y").await, "y does not leave");
            assert!(
                driver.live.approval().is_none(),
                "an answered approval is still parked"
            );
            let seen = driver.seen.lock().expect("seen").clone();
            assert!(
                seen.iter().any(|kind| kind == "approval_decision"),
                "no decision reached the agent: {seen:?}"
            );
        }

        /// A session with no workspace takes the panel's "nothing to read" door:
        /// the box still opens, and it does not run git in an empty path.
        #[allow(clippy::await_holding_lock)]
        #[tokio::test]
        async fn the_repository_panel_without_a_workspace_says_so() {
            let _env = a_peer_may_not_swap_the_environment();
            let mut driver = Driver::attach(&[
                ("get_state", r#"{"sessionId":"s1","model":"future/old"}"#),
                ("get_session_entries", r#"{"entries":[]}"#),
            ])
            .await;
            assert!(!driver.quit_key("ctrl+g").await);
            let painted = driver.pump(Duration::from_millis(300)).await;
            assert_eq!(driver.workbench.overlay, Some(Overlay::Git));
            assert!(
                driver.live.git().is_none(),
                "a workspace-less session read a repository"
            );
            assert!(frames(&painted) >= 1);
        }

        /// A 储藏 entry is read as a whole patch — `stash show --patch` — so a
        /// rename in a stash arrives as git's own metadata: `similarity index`,
        /// `rename from`, `rename to`. Those are the lines the pane paints in its
        /// metadata colour, and they are only visible through a read that spans
        /// both sides of the rename (a one-path pathspec reports a new file).
        #[allow(clippy::await_holding_lock)]
        #[tokio::test]
        async fn the_stash_tab_paints_git_s_rename_metadata() {
            let _env = a_peer_may_not_swap_the_environment();
            let repo = tempfile::tempdir().expect("a temp directory");
            let git_program = crate::test_env::absolute_tool("git");
            let git = |args: &[&str]| {
                let output = std::process::Command::new(&git_program)
                    .arg("-C")
                    .arg(repo.path())
                    .args(args)
                    .output()
                    .expect("run git");
                assert!(output.status.success(), "git {args:?}: {output:?}");
            };
            git(&["init", "-q", "-b", "main"]);
            git(&["config", "user.email", "tui@example.invalid"]);
            git(&["config", "user.name", "TUI test"]);
            std::fs::write(repo.path().join("a.txt"), "one\n").expect("write a file");
            git(&["add", "."]);
            git(&["commit", "-qm", "first commit"]);
            git(&["mv", "a.txt", "renamed.txt"]);
            git(&["stash", "push", "-q", "-m", "a rename"]);

            let mut driver = Driver::attach_conversation_in(repo.path()).await;
            assert!(!driver.quit_key("ctrl+g").await, "ctrl+g left the app");
            assert!(
                driver
                    .wait_for(Duration::from_secs(20), |driver| driver
                        .live
                        .git()
                        .is_some())
                    .await,
                "the log never installed"
            );
            // `4` is the 储藏 tab: one entry, read as its own patch.
            assert!(!driver.quit_key("4").await, "4 left the app");
            assert!(
                driver
                    .wait_for(Duration::from_secs(20), |driver| driver
                        .live
                        .git()
                        .is_some_and(
                            |view| view.section == crate::workbench::state::GitSection::Stash
                        ))
                    .await,
                "the stash tab never landed"
            );
            let view = driver.live.git().cloned().expect("a view");
            assert!(
                view.detail
                    .iter()
                    .any(|line| line.starts_with("similarity index")),
                "the stash's patch is what this tab reads: {:?}",
                view.detail
            );
            // A narrow frame paints one pane at a time; `tab` gives the keys (and
            // the screen) to the patch.
            assert!(!driver.quit_key("tab").await, "tab left the app");
            let painted = driver.pump(Duration::from_millis(300)).await;
            let text = text_of(&painted);
            for line in [
                "similarity index",
                "rename from a.txt",
                "rename to renamed.txt",
            ] {
                assert!(text.contains(line), "the pane dropped `{line}`: {text}");
            }
        }

        /// The detail pane is a box that always says something: a selected row
        /// whose read produced no body paints "select an entry" rather than
        /// leaving the pane blank beside a row the reader can see.
        ///
        /// The empty body is a **defect worth writing down rather than a state to
        /// be proud of**: `diff_review::load` is handed the path
        /// `git status --porcelain` printed, which is relative to the
        /// *repository root*, and passes it to `git diff -- <path>` whose
        /// pathspec is relative to the *session's* directory. At the root the two
        /// agree; from a subdirectory nothing matches, so every 改动 row reads as
        /// "no patch" (measured below: one row listed, `detail` empty). The
        /// control is the same tab read from the root — same repository, same
        /// code — which paints the patch. Importing the path against the root
        /// turns this case into that one, and leaves the placeholder reachable
        /// only through a section whose rows carry no object at all; this test
        /// then fails on `detail.is_empty()`, which is the intent.
        #[allow(clippy::await_holding_lock)]
        #[tokio::test]
        async fn a_row_with_no_body_keeps_the_detail_pane_a_box() {
            let _env = a_peer_may_not_swap_the_environment();
            let repo = tempfile::tempdir().expect("a temp directory");
            let git_program = crate::test_env::absolute_tool("git");
            let git = |args: &[&str]| {
                let output = std::process::Command::new(&git_program)
                    .arg("-C")
                    .arg(repo.path())
                    .args(args)
                    .output()
                    .expect("run git");
                assert!(output.status.success(), "git {args:?}: {output:?}");
            };
            git(&["init", "-q", "-b", "main"]);
            git(&["config", "user.email", "tui@example.invalid"]);
            git(&["config", "user.name", "TUI test"]);
            std::fs::write(repo.path().join("a.txt"), "one\n").expect("write a file");
            std::fs::create_dir(repo.path().join("sub")).expect("a subdirectory");
            std::fs::write(repo.path().join("sub").join("b.txt"), "inner\n")
                .expect("write a file under the workspace");
            git(&["add", "."]);
            git(&["commit", "-qm", "first commit"]);
            std::fs::write(repo.path().join("a.txt"), "two\n").expect("edit the file");

            let mut driver = Driver::attach_conversation_in(&repo.path().join("sub")).await;
            assert!(!driver.quit_key("ctrl+g").await, "ctrl+g left the app");
            assert!(
                driver
                    .wait_for(Duration::from_secs(20), |driver| driver
                        .live
                        .git()
                        .is_some())
                    .await,
                "the log never installed"
            );
            assert!(!driver.quit_key("3").await, "3 left the app");
            assert!(
                driver
                    .wait_for(Duration::from_secs(20), |driver| driver
                        .live
                        .git()
                        .is_some_and(
                            |view| view.section == crate::workbench::state::GitSection::Status
                        ))
                    .await,
                "the working-tree tab never landed"
            );
            let view = driver.live.git().cloned().expect("a view");
            assert!(
                !view.rows.is_empty(),
                "the tab does not list the change: {:?}",
                view.rows
            );
            assert!(
                view.detail.is_empty(),
                "a patch was read from outside the workspace: {:?}",
                view.detail
            );
            // The list is what the reader has: the tab names the change.
            let listed = text_of(&driver.pump(Duration::from_millis(300)).await);
            assert!(
                listed.contains("a.txt"),
                "the row the pane describes is not on screen: {listed}"
            );
            // A narrow frame paints one pane at a time; `tab` shows the patch.
            assert!(!driver.quit_key("tab").await, "tab left the app");
            let painted = driver.pump(Duration::from_millis(300)).await;
            let text = text_of(&painted);
            let placeholder = crate::workbench::i18n::t(
                driver.workbench.lang,
                crate::workbench::i18n::Key::GitNoSelection,
            );
            assert!(
                text.contains(placeholder),
                "the detail pane painted nothing at all: {text}"
            );

            // The control: the same repository read from its root. There the
            // session's directory *is* the one the paths are relative to, so the
            // same tab paints the patch — which is what makes the empty body
            // above a mismatch of roots rather than a read that never happens.
            let mut root = Driver::attach_conversation_in(repo.path()).await;
            assert!(!root.quit_key("ctrl+g").await, "ctrl+g left the app");
            assert!(
                root.wait_for(Duration::from_secs(20), |driver| driver
                    .live
                    .git()
                    .is_some())
                    .await,
                "the log never installed"
            );
            assert!(!root.quit_key("3").await, "3 left the app");
            let patched = |driver: &Driver| {
                driver.live.git().is_some_and(|view| {
                    view.detail
                        .iter()
                        .any(|line| line.starts_with("diff --git"))
                })
            };
            let landed = root.wait_for(Duration::from_secs(20), patched).await;
            assert!(landed, "the same tab from the root read no patch");
            assert!(!root.quit_key("tab").await, "tab left the app");
            let painted = text_of(&root.pump(Duration::from_millis(300)).await);
            assert!(
                painted.contains("diff --git") && painted.contains("+two"),
                "the pane dropped the patch it read: {painted}"
            );
            assert!(
                !painted.contains(placeholder),
                "the pane asks for a selection with one selected: {painted}"
            );
        }
    }

    mod loop_edges {
        use super::*;
        use crate::workbench::Overlay;

        fn draft(driver: &mut Driver) -> String {
            let frame = driver.frame();
            driver.live.data(&frame).input.text.clone()
        }

        /// An agent event of `kind`, on `session`, carrying `data`.
        fn event(kind: &str, data: serde_json::Value, session: &str, run: &str) -> AgentEvent {
            AgentEvent {
                r#type: kind.into(),
                data,
                session_id: Some(session.into()),
                run_id: Some(run.into()),
                ..Default::default()
            }
        }

        /// A resize and a focus report can arrive in the same read as the keys
        /// around them. The burst drain unwraps each into its own event — the
        /// resize becomes the size the frame is composed for, and a focus report
        /// is answered by the notifier rather than typed into the draft.
        #[tokio::test]
        async fn a_resize_and_a_focus_report_inside_a_burst_are_not_text() {
            let mut driver = Driver::attach_conversation().await;
            driver.data("a");
            let _ = driver.input_tx.send(TerminalEvent::Resize);
            driver.data("\x1b[I");
            driver.data("\x1b[O");
            let _ = driver.pump(Duration::from_millis(400)).await;
            assert_eq!(
                draft(&mut driver),
                "a",
                "the keys around the resize were lost or the focus reports were typed"
            );
        }

        /// A conversation stepped away from is filed for the way back, and the
        /// step onto one is a step: it names the conversation it lands on and
        /// paints it.
        ///
        /// The rail's cursor starts on the first row while the conversation on
        /// screen is whatever the agent's state named, so the answer to
        /// `get_state` has to be moved by hand between the steps: a static one
        /// reports the conversation it started on and undoes the step that just
        /// landed.
        #[tokio::test]
        async fn stepping_away_files_the_conversation_for_the_way_back() {
            let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
                ("get_state", r#"{"sessionId":"s1","model":"future/old"}"#),
                (
                    "list_sessions",
                    r#"{"sessions":[
                    {"id":"s1","cwd":"/tmp","updatedAtMs":1700000002000,"model":"future/old"},
                    {"id":"s2","cwd":"/tmp","updatedAtMs":1700000001000,"model":"future/old"}
                ]}"#,
                ),
                ("get_session_entries", r#"{"entries":[]}"#),
                (
                    "list_models",
                    r#"{"models":[{"id":"old","label":"old","provider":"future",
                    "thinkingLevel":"default","thinkingLevels":["default"],
                    "defaultThinkingLevel":"default","isDefault":true}]}"#,
                ),
            ]);
            let answers = mock.data_by_type.clone();
            let seen_sessions = mock.seen_sessions.clone();
            let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
            let mut driver = Driver::attach_at(&addr).await;
            driver.seen = seen;
            driver.seen_sessions = seen_sessions;
            assert_eq!(
                driver.session_id(),
                "s1",
                "the fixture did not start on the rail's first row"
            );

            // One step down: `s2` is not cached, so the read starts now (a tap
            // does not pay the settle window) and the conversation it left
            // behind is filed when that answer lands.
            answers.lock().expect("answers").insert(
                "get_state".into(),
                r#"{"sessionId":"s2","model":"future/old"}"#.into(),
            );
            driver.data("\x1b[1;2B");
            assert!(
                driver
                    .wait_for(Duration::from_secs(20), |driver| driver
                        .live
                        .is_cached("s1"))
                    .await,
                "the conversation stepped away from was not kept: the way back is a read"
            );

            // …and the step back opens it, naming the conversation it lands on
            // and filing the one it leaves (the same bookkeeping, the other way).
            answers.lock().expect("answers").insert(
                "get_state".into(),
                r#"{"sessionId":"s1","model":"future/old"}"#.into(),
            );
            driver.data("\x1b[1;2A");
            assert!(
                driver
                    .wait_for(Duration::from_secs(20), |driver| driver
                        .live
                        .is_cached("s2"))
                    .await,
                "the step back never opened the conversation it landed on"
            );
            assert_eq!(
                driver.session_id(),
                "s1",
                "the step back landed on the wrong conversation"
            );
            assert!(
                driver
                    .seen_sessions
                    .lock()
                    .expect("seen")
                    .iter()
                    .any(|(command, session)| command == "get_state" && session == "s1"),
                "the step back asked the agent nothing about the conversation it opened"
            );
        }

        /// A key pressed while a step is still settling lands the step first, so
        /// the key belongs to the conversation the reader is now looking at.
        #[tokio::test]
        async fn a_key_pressed_while_a_step_settles_lands_the_step_first() {
            let mut driver = Driver::attach_sessions().await;
            driver.data("\x1b[1;2B");
            // Queued from beside the loop: it arrives inside `OPEN_SETTLE`, in a
            // later pass of the *same* run, which is the window the loop has to
            // close by landing the step.
            let tx = driver.input_tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                let _ = tx.send(TerminalEvent::Data("x".into()));
            });
            let _ = driver.pump(Duration::from_millis(700)).await;
            let asked = driver.seen_sessions.lock().expect("seen").clone();
            assert!(
                asked
                    .iter()
                    .any(|(command, session)| command == "get_state" && session == "s2"),
                "the key was applied without landing the step before it: {asked:?}"
            );
            assert_eq!(draft(&mut driver), "x", "the key did not reach the draft");
        }

        /// A step that follows another inside [`STEP_TAP_GAP`] is a *hold*: it
        /// waits out the settle window instead of opening at once, and the key
        /// that follows it lands the step first — so the key belongs to the
        /// conversation the reader is now looking at, not to the one they left.
        ///
        /// The second step is sent when the *first one's read is on the wire*
        /// rather than after a fixed sleep: a sleep long enough to be safe under
        /// load is longer than the gap, and would make the step a tap (which is
        /// the branch next door).
        #[tokio::test]
        async fn a_held_step_lands_before_the_key_that_follows_it() {
            let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
                ("get_state", r#"{"sessionId":"s1","model":"future/old"}"#),
                (
                    "list_sessions",
                    r#"{"sessions":[
                    {"id":"s1","cwd":"/tmp","updatedAtMs":1700000003000,"model":"future/old"},
                    {"id":"s2","cwd":"/tmp","updatedAtMs":1700000002000,"model":"future/old"},
                    {"id":"s3","cwd":"/tmp","updatedAtMs":1700000001000,"model":"future/old"}
                ]}"#,
                ),
                ("get_session_entries", r#"{"entries":[]}"#),
                (
                    "list_models",
                    r#"{"models":[{"id":"old","label":"old","provider":"future",
                    "thinkingLevel":"default","thinkingLevels":["default"],
                    "defaultThinkingLevel":"default","isDefault":true}]}"#,
                ),
            ]);
            // The answer to `get_state` names the conversation it was asked
            // about, so the table moves by hand between the steps: the rail's
            // cursor follows whatever the answer names, and a static `s1` would
            // put it back on the first row after every step.
            let answers = mock.data_by_type.clone();
            let seen_sessions = mock.seen_sessions.clone();
            let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
            let mut driver = Driver::attach_at(&addr).await;
            driver.seen = seen;
            driver.seen_sessions = seen_sessions;

            // The first step is a tap: it opens `s2` at once, which is what the
            // read named in the log means.
            answers.lock().expect("answers").insert(
                "get_state".into(),
                r#"{"sessionId":"s2","model":"future/old"}"#.into(),
            );
            driver.data("\x1b[1;2B");
            let steering_answers = answers.clone();
            let steering_seen = driver.seen_sessions.clone();
            let tx = driver.input_tx.clone();
            let steering = tokio::spawn(async move {
                let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
                while tokio::time::Instant::now() < deadline {
                    let opened = steering_seen
                        .lock()
                        .expect("seen")
                        .iter()
                        .any(|(command, session)| command == "get_state" && session == "s2");
                    if opened {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                // The repeat step, then a key: both inside one run of the loop,
                // because the settle window is state the loop holds. The answer
                // moves to `s3` first, the conversation this step lands on.
                steering_answers.lock().expect("answers").insert(
                    "get_state".into(),
                    r#"{"sessionId":"s3","model":"future/old"}"#.into(),
                );
                let _ = tx.send(TerminalEvent::Data("\x1b[1;2B".into()));
                tokio::time::sleep(Duration::from_millis(15)).await;
                let _ = tx.send(TerminalEvent::Data("x".into()));
            });
            let _ = driver.pump(Duration::from_millis(1500)).await;
            steering.await.expect("the steering task finishes");

            let asked = driver.seen_sessions.lock().expect("seen").clone();
            assert!(
                asked
                    .iter()
                    .any(|(command, session)| command == "get_state" && session == "s3"),
                "the held step never landed on the conversation it stopped at: {asked:?}"
            );
            // The key went to the conversation the held step landed on. Had it
            // been applied first it would be `s2`'s draft, and the box that
            // replaced it would be empty.
            assert_eq!(
                draft(&mut driver),
                "x",
                "the key was typed before the held step landed: it belongs to the conversation left behind"
            );
        }

        /// `n` is the decline half of a parked approval's `y`/`n`: the card is
        /// answered, so it stops being parked.
        ///
        /// There are two doors and they are different lines: with the card on
        /// screen the key goes to the card (a decision it hands back as an
        /// action), and with the card closed but the request still parked it is
        /// the *base surface's* fast path that answers — which is the pair this
        /// pins, because a reader who dismissed the card still has to be able to
        /// decline without reopening it.
        #[tokio::test]
        async fn a_parked_approval_is_declined_by_n() {
            let mut driver = Driver::attach_conversation().await;
            let session = driver.session_id();
            let _ = driver.agent_tx.send(event(
                "approval_request",
                serde_json::json!({"approval_request_id": "ap-1", "title": "run it?"}),
                &session,
                "run-1",
            ));
            let _ = driver.pump(Duration::from_millis(400)).await;
            assert!(
                driver.live.approval().is_some(),
                "the request never parked an approval"
            );
            // The card is on screen: this `n` is the card's.
            assert!(
                !driver.quit_key("n").await,
                "the decline left the workbench"
            );
            assert!(
                driver.live.approval().is_none(),
                "the decline did not answer the parked request"
            );

            // The same request, closed without answering: the next `n` is the
            // base surface's, and it must still answer it.
            let _ = driver.agent_tx.send(event(
                "approval_request",
                serde_json::json!({"approval_request_id": "ap-2", "title": "again?"}),
                &session,
                "run-1",
            ));
            let _ = driver.pump(Duration::from_millis(400)).await;
            assert!(
                driver.live.approval().is_some(),
                "the second request never parked"
            );
            assert!(!driver.quit_key("escape").await);
            assert_eq!(driver.workbench.overlay, None, "the card did not close");
            assert!(
                driver.live.approval().is_some(),
                "closing the card answered the request"
            );
            assert!(
                !driver.quit_key("n").await,
                "declining from the closed card left the workbench"
            );
            assert!(
                driver.live.approval().is_none(),
                "the base surface's decline was swallowed by the closed card"
            );
        }

        /// A run this client started, ending on its own, is announced; the
        /// transcript is complete and the activity row stops.
        #[tokio::test]
        async fn a_completed_run_is_announced_and_the_activity_row_stops() {
            let mut driver = Driver::attach_conversation().await;
            assert!(!driver.type_text("hello").await);
            assert!(!driver.quit_key("enter").await);
            let _ = driver.pump(Duration::from_millis(300)).await;
            // The ack is a `RunAck`, so the client treats the run as its own —
            // which is what decides whether its `agent_end` is announced. An
            // ack this fixture cannot decode is *reported* (the reader is told
            // the send was not confirmed) and files nothing, so this assertion
            // is the premise of everything below rather than a formality.
            assert!(
                driver.live.notice.is_none(),
                "the fixture's prompt ack did not decode: {:?}",
                driver.live.notice
            );
            let session = driver.session_id();
            let _ =
                driver
                    .agent_tx
                    .send(event("agent_start", serde_json::json!({}), &session, "r-1"));
            let _ = driver.pump(Duration::from_millis(200)).await;
            let _ = driver.agent_tx.send(event(
                "text_chunk",
                serde_json::json!({"text": "finished answer"}),
                &session,
                "r-1",
            ));
            // The end arrives **on its own** — an event that lands in the same
            // breath as the ones before it is swept up by the loop's burst drain,
            // which is a different call site for the same decision. This one
            // pins the single-event arm.
            let _ = driver.pump(Duration::from_millis(200)).await;
            let _ = driver.agent_tx.send(event(
                "agent_end",
                serde_json::json!({"state": "completed"}),
                &session,
                "r-1",
            ));
            let painted = driver.pump(Duration::from_millis(700)).await;
            assert!(
                text_of(&painted).contains("finished answer"),
                "the finished run's text is not on screen: {painted:?}"
            );
            assert!(
                !driver.live.activity_animating(),
                "a finished run still animates the activity row"
            );
            assert!(
                driver.live.queue_is_empty(),
                "the completion was raised with a queued follow-up on the agent"
            );
        }

        /// A run that ends in an error is announced as a failure, and the run is
        /// over either way: the activity row stops.
        ///
        /// The failure notice is not the completion one: a failed run is an
        /// *attention* alert (it rings even while the terminal is focused),
        /// which is the branch this pins.
        #[tokio::test]
        async fn a_failed_run_is_announced_as_failed() {
            let mut driver = Driver::attach_conversation().await;
            assert!(!driver.type_text("hello").await);
            assert!(!driver.quit_key("enter").await);
            let _ = driver.pump(Duration::from_millis(300)).await;
            assert!(
                driver.live.notice.is_none(),
                "the fixture's prompt ack did not decode: {:?}",
                driver.live.notice
            );
            let session = driver.session_id();
            let _ =
                driver
                    .agent_tx
                    .send(event("agent_start", serde_json::json!({}), &session, "r-1"));
            // The end arrives on its own: the loop's burst drain is the *other*
            // call site of the same decision, and
            // `a_burst_announces_what_is_in_it` is where that one is pinned.
            let _ = driver.pump(Duration::from_millis(200)).await;
            let _ = driver.agent_tx.send(event(
                "agent_end",
                serde_json::json!({"state": "failed"}),
                &session,
                "r-1",
            ));
            let painted = driver.pump(Duration::from_millis(700)).await;
            assert!(frames(&painted) >= 1, "the failure painted nothing");
            assert!(
                !driver.live.activity_animating(),
                "a failed run still animates the activity row"
            );
        }

        /// A burst is drained, not dropped: what arrives in the same breath as an
        /// event already being applied still gets its own handling — the failure
        /// of a run this client started is announced, and an `error` behind it is
        /// alerted and written to the transcript.
        #[tokio::test]
        async fn a_burst_announces_what_is_in_it() {
            // The ack's run id moves by hand between the submissions: a run is
            // only "this client's own" while the ack that opened it is the one
            // the client filed, and the two bursts below are two runs.
            let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
                ("get_state", r#"{"sessionId":"s1","model":"future/old"}"#),
                ("get_session_entries", r#"{"entries":[]}"#),
                (
                    "list_models",
                    r#"{"models":[{"id":"old","label":"old","provider":"future",
                    "thinkingLevel":"default","thinkingLevels":["default"],
                    "defaultThinkingLevel":"default","isDefault":true}]}"#,
                ),
                (
                    "prompt",
                    r#"{"run_id":"r-1","run_epoch":1,"accepted_state":"running"}"#,
                ),
            ]);
            let answers = mock.data_by_type.clone();
            let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
            let mut driver = Driver::attach_at(&addr).await;
            driver.seen = seen;
            assert!(!driver.type_text("hello").await);
            assert!(!driver.quit_key("enter").await);
            let _ = driver.pump(Duration::from_millis(300)).await;
            let session = driver.session_id();
            // Two events in one breath: the first is applied by the arm that
            // received it, the second by the drain behind it.
            let _ =
                driver
                    .agent_tx
                    .send(event("agent_start", serde_json::json!({}), &session, "r-1"));
            let _ = driver.agent_tx.send(event(
                "agent_end",
                serde_json::json!({"state": "failed"}),
                &session,
                "r-1",
            ));
            let _ = driver.pump(Duration::from_millis(500)).await;
            assert!(
                !driver.live.activity_animating(),
                "a failed run in a burst left the activity row spinning"
            );

            // The same shape for an alert: a plain `error` behind a start. The
            // key is `error` (the field the surface reads), and the text lands on
            // the notice row.
            let _ =
                driver
                    .agent_tx
                    .send(event("agent_start", serde_json::json!({}), &session, "r-1"));
            let _ = driver.agent_tx.send(event(
                "error",
                serde_json::json!({"error": "the provider refused"}),
                &session,
                "r-1",
            ));
            let _ = driver.pump(Duration::from_millis(500)).await;
            let notice = driver.live.notice.clone().unwrap_or_default();
            assert!(
                notice.contains("the provider refused"),
                "the error in the burst never reached the notice row: {notice:?}"
            );

            // And a *completion* behind a start: the drain asks the same
            // question of the second event, and the answer is the one the
            // single-event arm gave — the run is over and it was this client's.
            answers.lock().expect("answers").insert(
                "prompt".into(),
                r#"{"run_id":"r-2","run_epoch":1,"accepted_state":"running"}"#.into(),
            );
            assert!(!driver.type_text("again").await);
            assert!(!driver.quit_key("enter").await);
            let _ = driver.pump(Duration::from_millis(300)).await;
            let _ =
                driver
                    .agent_tx
                    .send(event("agent_start", serde_json::json!({}), &session, "r-2"));
            let _ = driver.agent_tx.send(event(
                "agent_end",
                serde_json::json!({"state": "completed"}),
                &session,
                "r-2",
            ));
            let _ = driver.pump(Duration::from_millis(500)).await;
            assert!(
                !driver.live.activity_animating(),
                "a completed run in a burst left the activity row spinning"
            );
            assert!(
                driver.live.queue_is_empty(),
                "the burst's completion was raised with a queued follow-up"
            );
        }

        /// An `error` event is an attention alert, not a completion: the loop
        /// raises it through the notifier and the transcript keeps its place.
        #[tokio::test]
        async fn an_error_event_is_alerted_rather_than_treated_as_an_answer() {
            let mut driver = Driver::attach_conversation().await;
            let session = driver.session_id();
            let _ = driver.agent_tx.send(event(
                "error",
                serde_json::json!({"message": "the provider refused"}),
                &session,
                "r-1",
            ));
            let painted = driver.pump(Duration::from_millis(500)).await;
            assert!(frames(&painted) >= 1, "the error painted no frame");
            assert!(
                !driver.live.activity_animating(),
                "an error left the activity row spinning"
            );
        }

        /// The approval card is re-opened by a state refresh: the request is
        /// still parked, so the box that shows it comes back rather than staying
        /// closed with nothing on screen saying why.
        #[tokio::test]
        async fn a_state_refresh_brings_the_parked_approval_back() {
            let mut driver = Driver::attach_conversation().await;
            let session = driver.session_id();
            let _ = driver.agent_tx.send(event(
                "approval_request",
                serde_json::json!({"approval_request_id": "ap-1", "title": "run it?"}),
                &session,
                "run-1",
            ));
            let _ = driver.pump(Duration::from_millis(400)).await;
            assert_eq!(driver.workbench.overlay, Some(Overlay::Approval));
            // The reader closes the card; the request is still parked.
            assert!(!driver.quit_key("escape").await);
            assert_eq!(driver.workbench.overlay, None);
            driver.live.begin_state_refresh();
            let _ = driver.pump(Duration::from_millis(600)).await;
            assert_eq!(
                driver.workbench.overlay,
                Some(Overlay::Approval),
                "a state refresh left a parked approval with no card on screen"
            );
        }

        /// The two failed reads of the repository panel that nothing else pins:
        /// the **row detail** (`SelectGit` → the gutter says what failed rather
        /// than staying empty) and the **file re-read** behind the review's
        /// `enter` (a notice, not a box that keeps the patch it had).
        ///
        /// The other three of this family — `/worktree`'s probe, `OpenDiff` and
        /// the clipboard's `y` — are `a_repository_read_that_failed_reaches_its_panel`,
        /// `a_review_of_a_workspace_that_went_away_says_so` and
        /// `a_copy_the_clipboard_refuses_is_reported`, which build a real
        /// repository so only the one tool they mean is missing. These two need a
        /// view that is already on screen, so they seed it and then take git away:
        /// a `PATH` with no `git` on it is this machine unable to answer.
        #[allow(clippy::await_holding_lock)]
        #[tokio::test]
        async fn a_detail_that_cannot_be_read_says_so_in_its_own_gutter() {
            let empty = tempfile::tempdir().expect("a directory to be PATH");
            let _env = a_peer_may_not_swap_the_environment();
            let _path = crate::test_env::EnvVarGuard::set("PATH", empty.path());

            // The panel's row detail: a view with a row is what the reader has
            // after a successful open, and the *re-read* is what fails here.
            let row = crate::workbench::state::GitRow {
                key: Some("abc1234".into()),
                columns: Vec::new(),
                prefix: "● ".into(),
                pieces: vec!["a commit".into()],
                kinds: Vec::new(),
            };
            let mut driver = Driver::attach_conversation().await;
            driver.live.set_git(Some(crate::workbench::state::GitView {
                repo: "gone".into(),
                section: crate::workbench::state::GitSection::Commits,
                rows: vec![row],
                selected: 0,
                detail: Vec::new(),
                filter: None,
                error: None,
            }));
            driver.workbench.overlay = Some(Overlay::Git);
            assert!(!driver.quit_key("j").await);
            let detail = driver
                .live
                .git()
                .map(|git| git.detail.clone())
                .unwrap_or_default();
            assert_eq!(
                detail.len(),
                1,
                "the gutter holds the failure and nothing else: {detail:?}"
            );
            assert!(
                detail[0].contains("git"),
                "the gutter does not say what could not be read: {detail:?}"
            );

            // The review's own `enter` re-reads one file. The view is on screen;
            // the read behind the next key is the one that fails.
            driver.live.set_diff(crate::workbench::state::DiffView {
                files: vec![crate::workbench::diff_review::DiffFile {
                    path: "a.txt".into(),
                    status: "M".into(),
                }],
                selected: 0,
                patch: String::new(),
                error: None,
            });
            driver.workbench.overlay = Some(Overlay::Diff);
            driver.live.notice = None;
            assert!(!driver.quit_key("enter").await);
            let notice = driver.live.notice.clone().unwrap_or_default();
            assert!(
                notice.contains("No such file"),
                "re-reading a file the tools cannot read said {notice:?}"
            );
        }

        /// A picture read from a panel steps through the pictures beside it: the
        /// list arrives with the decoded image, on the loop's own wake-up.
        #[tokio::test]
        async fn a_preview_publishes_the_pictures_beside_it() {
            let dir = tempfile::tempdir().expect("a temp directory");
            for name in ["one.png", "two.png"] {
                let image = image::RgbImage::from_pixel(4, 4, image::Rgb([200, 40, 40]));
                image.save(dir.path().join(name)).expect("write a picture");
            }
            let mut driver = Driver::attach_conversation_in(dir.path()).await;
            // The workspace listing lands on a tick; `/files` re-reads the tree
            // the client already has, so the box waits for it first.
            let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
            loop {
                let frame = driver.frame();
                if !driver.live.data(&frame).tree.is_empty() {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the workspace tree never arrived"
                );
                let _ = driver.pump(Duration::from_millis(300)).await;
            }
            assert!(!driver.command("/files").await, "/files left");
            let _ = driver.pump(Duration::from_millis(300)).await;
            assert!(!driver.quit_key("enter").await);
            let _ = driver.pump(Duration::from_millis(600)).await;
            assert_eq!(
                driver.workbench.preview_siblings.len(),
                2,
                "the picture's neighbours were not published to the box: {:?}",
                driver.workbench.preview_siblings
            );
        }
    }

    mod menus_and_verbs {
        use super::*;
        use crate::workbench::preferences::{Action, Keybindings};
        use crate::workbench::state::GitSection;
        use crate::workbench::{picker::PickerKind, Overlay};

        /// How long a background read (a repository section, a directory tree)
        /// may take before the test calls it stuck.
        const READ_BUDGET: Duration = Duration::from_secs(20);

        /// Give the driver a binding table with `pairs` pressed in, so a test
        /// names the verb it is testing rather than whatever chord ships today.
        fn bind(driver: &mut Driver, pairs: &[(Action, &str)]) {
            let mut keys = Keybindings::default();
            for (action, chord) in pairs {
                keys.bind(*action, chord).expect("a spare chord");
            }
            driver.workbench.keys = keys;
        }

        /// The preferences menu is a list of *writes*: every row it changes has
        /// to reach `settings.json` in the same gesture, or a reader who picks a
        /// width and loses the terminal finds it gone.
        #[tokio::test]
        async fn the_preferences_menu_writes_every_row_it_changes() {
            let mut driver = Driver::attach_conversation().await;
            bind(&mut driver, &[(Action::Preferences, "ctrl+o")]);
            assert!(!driver.quit_key("ctrl+o").await, "the menu left by itself");
            assert_eq!(driver.workbench.overlay, Some(Overlay::Preferences));

            // `宽度` is the fourth row: `right` widens the rail and the file
            // follows it.
            for _ in 0..3 {
                assert!(!driver.quit_key("down").await);
            }
            let was = driver.workbench.sidebar_w;
            assert!(!driver.quit_key("right").await);
            assert_ne!(
                driver.workbench.sidebar_w, was,
                "the width row did not widen the rail"
            );
            assert_eq!(
                super::super::load_settings(&driver.settings_path).sidebar_width,
                Some(driver.workbench.sidebar_w),
                "the width the menu showed was not what the next start will read"
            );

            // `鼠标` is the two doors' shared toggle: both answers are written,
            // and each says which state it left.
            for _ in 0..2 {
                assert!(!driver.quit_key("down").await);
            }
            let was = driver.workbench.mouse_capture;
            assert!(!driver.quit_key("enter").await);
            assert_ne!(
                driver.workbench.mouse_capture, was,
                "the mouse row did not flip the capture"
            );
            let off = driver.live.notice.clone().expect("the toggle said nothing");
            assert_eq!(
                super::super::load_settings(&driver.settings_path).mouse_capture,
                Some(driver.workbench.mouse_capture),
                "the toggle was not written"
            );
            assert!(!driver.quit_key("enter").await);
            assert_eq!(
                driver.workbench.mouse_capture, was,
                "the second toggle did not put the capture back"
            );
            let on = driver
                .live
                .notice
                .clone()
                .expect("the second toggle was silent");
            assert_ne!(on, off, "both directions reported the same state");

            // `快捷键` opens the binding table, and `r` there writes the
            // defaults back out.
            for _ in 0..3 {
                assert!(!driver.quit_key("down").await);
            }
            assert!(!driver.quit_key("enter").await);
            assert_eq!(
                driver.workbench.overlay,
                Some(Overlay::Keybindings),
                "the shortcuts row did not open the table"
            );
            assert!(!driver.quit_key("r").await);
            assert!(
                crate::workbench::preferences::keybindings_path(&driver.settings_path).exists(),
                "the reset bindings were not written"
            );

            // `恢复默认` is the whole menu above it: the layout, the language and
            // the palette that follows the terminal again, all in one gesture.
            assert!(!driver.quit_key("escape").await);
            assert_eq!(driver.workbench.overlay, Some(Overlay::Preferences));
            assert!(!driver.quit_key("down").await);
            assert!(!driver.quit_key("enter").await);
            let written = super::super::load_settings(&driver.settings_path);
            assert_eq!(
                written.theme_id, None,
                "the reset kept a palette the reader asked to forget"
            );
            assert_eq!(
                driver.workbench.palette.id,
                crate::workbench::theme::resolve(None).id,
                "the palette did not go back to following the terminal"
            );
            assert_eq!(
                written.language.as_deref(),
                Some("system"),
                "the reset did not put the language back to the system's"
            );
        }

        /// `/theme` and `/lang` are pickers, and committing one is a write: the
        /// next start paints the palette and speaks the language the reader
        /// chose.
        #[tokio::test]
        async fn a_theme_and_a_language_picked_from_the_menu_are_written() {
            let mut driver = Driver::attach_conversation().await;
            assert!(!driver.command("/theme").await, "/theme left");
            assert_eq!(
                driver.workbench.overlay,
                Some(Overlay::Picker(PickerKind::Theme))
            );
            // Off the current row, so the commit is a change the file can show.
            assert!(!driver.quit_key("down").await);
            assert!(!driver.quit_key("enter").await);
            assert_eq!(driver.workbench.overlay, None, "the picker stayed open");
            let theme = driver.workbench.palette.id.to_string();
            assert_eq!(
                driver.settings.theme_id.as_deref(),
                Some(theme.as_str()),
                "the committed theme was not staged for the settings file"
            );
            assert_eq!(
                super::super::load_settings(&driver.settings_path)
                    .theme_id
                    .as_deref(),
                Some(theme.as_str()),
                "the theme never reached settings.json"
            );

            assert!(!driver.command("/lang").await, "/lang left");
            assert_eq!(
                driver.workbench.overlay,
                Some(Overlay::Picker(PickerKind::Language))
            );
            assert!(!driver.quit_key("down").await);
            assert!(!driver.quit_key("enter").await);
            let wire = driver.workbench.language_pref.as_wire().to_string();
            assert_eq!(
                driver.settings.language.as_deref(),
                Some(wire.as_str()),
                "the picked language was not staged"
            );
            assert_eq!(
                super::super::load_settings(&driver.settings_path)
                    .language
                    .as_deref(),
                Some(wire.as_str()),
                "the language never reached settings.json"
            );
        }

        /// `/worktree` reads the repository beside the surface, `/worktree new`
        /// creates the checkout it names, and a second one by the same name is
        /// refused with a sentence instead of a half-created directory.
        #[tokio::test]
        async fn the_worktree_verbs_plan_create_and_refuse_a_second_copy() {
            let repo = super::workspace_panel::repo_with_two_commits();
            let mut driver = Driver::attach_conversation_in(repo.path()).await;
            assert!(!driver.command("/worktree").await, "/worktree left");
            assert_eq!(driver.workbench.overlay, Some(Overlay::Worktree));
            assert!(
                driver.live.notice.is_none(),
                "reading a repository reported {:?}",
                driver.live.notice
            );
            // The box owns the keys while it is open; the create is typed into
            // the draft, so the reader leaves the box first.
            assert!(!driver.quit_key("escape").await);
            assert_eq!(driver.workbench.overlay, None);

            assert!(!driver.command("/worktree new cov-branch").await);
            let created = repo.path().join(".worktrees").join("cov-branch");
            let created_shown = created.display().to_string();
            assert!(
                created.is_dir(),
                "the worktree was not created at {created_shown}: {:?}",
                driver.live.notice
            );

            // The same name again: the second create must refuse before it runs
            // git, and say so.
            assert!(!driver.command("/worktree new cov-branch").await);
            let notice = driver
                .live
                .notice
                .clone()
                .expect("a refused create said nothing");
            assert!(
                notice.contains("cov-branch"),
                "the refusal does not name what it refused: {notice:?}"
            );
        }

        /// The repository browser's verbs land on the driver: `y` copies the
        /// row, `d` opens the working-tree review, `enter` selects a file there
        /// and filters the log by a ref.
        #[allow(clippy::await_holding_lock)]
        #[tokio::test]
        async fn the_repository_verbs_are_answered_by_the_driver() {
            let _env = a_peer_may_not_swap_the_environment();
            let repo = super::workspace_panel::repo_with_two_commits();
            // A working-tree change, so the review has a file to show.
            std::fs::write(repo.path().join("a.txt"), "three\n").expect("write the change");
            let mut driver = Driver::attach_conversation_in(repo.path()).await;
            assert!(!driver.quit_key("ctrl+g").await);
            assert!(
                driver
                    .wait_for(READ_BUDGET, |driver| driver.live.git().is_some())
                    .await,
                "the log never installed"
            );

            // `y` copies the selected commit's id.
            assert!(!driver.quit_key("y").await, "copy left the panel");
            let notice = driver.live.notice.clone().expect("a copy said nothing");
            assert!(
                !notice.is_empty(),
                "the copy was reported as an empty… nothing"
            );

            // `d` opens the review of the working tree.
            assert!(!driver.quit_key("d").await, "the review key left");
            assert_eq!(driver.workbench.overlay, Some(Overlay::Diff));
            let frame = driver.frame();
            let data = driver.live.data(&frame);
            let view = data.diff.as_ref().expect("the review opened with no view");
            assert!(
                !view.files.is_empty(),
                "a modified file was not in the review: {:?}",
                view.error
            );

            // `enter` on a file re-reads just that file's patch.
            assert!(!driver.quit_key("enter").await);
            let frame = driver.frame();
            let data = driver.live.data(&frame);
            let selected = data.diff.as_ref().expect("the review closed");
            assert_eq!(selected.selected, 0, "the selection moved unexpectedly");

            // Back in the panel: `enter` on a ref filters the log by it, and
            // `backspace` clears the filter again.
            assert!(!driver.quit_key("escape").await);
            assert_eq!(driver.workbench.overlay, None);
            assert!(!driver.quit_key("ctrl+g").await);
            assert!(!driver.quit_key("2").await);
            assert!(
                driver
                    .wait_for(READ_BUDGET, |driver| driver
                        .live
                        .git()
                        .is_some_and(|view| view.section == GitSection::Refs))
                    .await,
                "the refs never installed"
            );
            assert!(!driver.quit_key("enter").await);
            let filtered = driver
                .wait_for(READ_BUDGET, |driver| {
                    driver.live.git().is_some_and(|view| view.filter.is_some())
                })
                .await;
            let filter_now = driver.live.git().map(|view| view.filter.clone());
            assert!(filtered, "the ref was not used as a filter: {filter_now:?}");
            assert!(!driver.quit_key("backspace").await);
            assert!(
                driver
                    .wait_for(READ_BUDGET, |driver| driver
                        .live
                        .git()
                        .is_some_and(|view| view.filter.is_none()))
                    .await,
                "backspace did not clear the filter"
            );
        }

        /// A worktree row is a place: `enter` moves the session there and the
        /// panel re-reads from the new checkout.
        #[allow(clippy::await_holding_lock)]
        #[tokio::test]
        async fn entering_a_worktree_moves_the_session_to_it() {
            let _env = a_peer_may_not_swap_the_environment();
            let repo = super::workspace_panel::repo_with_two_commits();
            let mut driver = Driver::attach_conversation_in(repo.path()).await;
            assert!(!driver.command("/worktree new cov-move").await);
            let created = repo.path().join(".worktrees").join("cov-move");
            assert!(created.is_dir(), "the fixture's worktree was not created");

            assert!(!driver.quit_key("ctrl+g").await);
            assert!(
                driver
                    .wait_for(READ_BUDGET, |driver| driver.live.git().is_some())
                    .await,
                "the log never installed"
            );
            // The worktrees tab lists the checkout the session is in first, so
            // the pointer moves to the created one before `enter`.
            assert!(!driver.quit_key("5").await);
            let listed = driver
                .wait_for(READ_BUDGET, |driver| {
                    driver.live.git().is_some_and(|view| {
                        view.section == GitSection::Worktrees && view.rows.len() > 1
                    })
                })
                .await;
            let rows_now = driver.live.git().map(|view| view.rows.len());
            assert!(
                listed,
                "the worktrees tab never listed both checkouts: {rows_now:?}"
            );
            assert!(!driver.quit_key("down").await);
            assert!(!driver.quit_key("enter").await);
            let seen = driver.seen.lock().expect("seen").clone();
            assert!(
                seen.iter().any(|kind| kind == "set_cwd"),
                "entering a worktree did not move the session: {seen:?}"
            );
            assert_eq!(driver.workbench.overlay, Some(Overlay::Git));
        }

        /// A file row opens the preview box, and a key for an attachment that
        /// is not there is a no-op rather than an error: the surface has nothing
        /// to show and nothing to say.
        #[tokio::test]
        async fn a_file_row_opens_the_preview_and_an_absent_attachment_is_quiet() {
            let dir = tempfile::tempdir().expect("a temp directory");
            std::fs::write(dir.path().join("readme.md"), "# hi\n").expect("write the file");
            let mut driver = Driver::attach_conversation_in(dir.path()).await;
            // The workspace listing starts with the session and lands on a tick,
            // which is why the box is opened after it: `/files` re-reads the
            // tree it already has, and a box opened on the empty default would
            // be waiting for a root it never learned.
            let deadline = tokio::time::Instant::now() + READ_BUDGET;
            loop {
                let frame = driver.frame();
                if !driver.live.data(&frame).tree.is_empty() {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the workspace tree never arrived"
                );
                // A tick is what polls the read, and a pump shorter than one
                // tick would return before it fires.
                let _ = driver.pump(Duration::from_millis(300)).await;
            }
            assert!(!driver.command("/files").await, "/files left");
            assert_eq!(driver.workbench.overlay, Some(Overlay::Files));
            let _ = driver.pump(Duration::from_millis(300)).await;
            assert!(!driver.quit_key("enter").await);
            assert_eq!(
                driver.workbench.overlay,
                Some(Overlay::FilePreview),
                "a file row did not open the box"
            );

            // Nothing attached: the key asks the adapter for an attachment the
            // session does not have, and the surface stays as it was.
            let mut driver = Driver::attach_conversation().await;
            bind(&mut driver, &[(Action::PreviewAttachment, "ctrl+o")]);
            assert!(!driver.quit_key("ctrl+o").await);
            assert_eq!(
                driver.workbench.overlay, None,
                "an absent attachment opened a box"
            );
        }

        /// The helper that waits for an effect is honest about the other answer:
        /// a budget that runs out reports "no", it does not claim the effect
        /// arrived.
        #[tokio::test]
        async fn a_budget_that_runs_out_reports_that_it_did_not_arrive() {
            let mut driver = Driver::attach_conversation().await;
            assert!(
                !driver.wait_for(Duration::ZERO, |_| false).await,
                "a wait that never saw its effect reported success"
            );
            assert!(
                driver.wait_for(Duration::ZERO, |_| true).await,
                "a condition that already holds was not read at once"
            );
        }
    }

    mod image_paint {
        use super::*;
        use crate::terminal_image::{CapabilitiesGuard, ImageProtocol};
        use crate::workbench::{demo, Overlay};

        /// The first 120 bytes of a capture: enough to see which escape sequence
        /// was written, without printing a picture into a failure message.
        fn head(painted: &str) -> &str {
            let end = painted.len().min(120);
            &painted[..end]
        }

        /// What a frame does with a picture on screen: the terminal is asked to
        /// place it, asked again only when the frame disturbed the cells it
        /// covers, and told to delete it when it goes away.
        ///
        /// `paint_images` measured zero because the capability probe answers
        /// "no pictures" on a host with no image protocol in its environment.
        /// The guard pins the answer a kitty terminal gives, and the frame here
        /// is the demo's own picture — the same fixture the preview box's own
        /// test uses.
        #[test]
        fn a_frame_places_the_picture_only_while_it_is_disturbed() {
            let _caps = CapabilitiesGuard::pin(ImageProtocol::Kitty);
            let mut data = demo::chat();
            data.preview = Some(demo::preview_image());
            let mut workbench = Workbench::new();
            workbench.overlay = Some(Overlay::FilePreview);
            let rows = workbench.render(&data, 120, 38);
            assert_eq!(rows.len(), 38, "the frame is the height it was asked for");
            assert!(
                !workbench.image_requests.is_empty(),
                "a kitty terminal is asked to place the picture"
            );
            // The terminal supplies the write path `paint_images` uses and
            // nothing else — as in the rest of this driver's fixtures, it must
            // not need a console (Windows' `Terminal::new` does, and a runner
            // that redirects stdio has none).
            let mut terminal = crate::terminal::driver_terminal();

            // A frame that rewrote the picture's rows asks for it again.
            let all_rows: Vec<usize> = (0..38).collect();
            crate::terminal::capture::start();
            super::super::paint_images(&mut terminal, &mut workbench, &all_rows);
            let placed = crate::terminal::capture::take();
            let shown = head(&placed);
            assert!(
                placed.contains("\x1b_Ga=T"),
                "the frame did not transmit the picture's pixels: {shown:?}"
            );
            assert!(
                !workbench.painted_images.ids.is_empty(),
                "the placement was not remembered, so the next frame would repeat it"
            );

            // The same frame again, with none of its rows rewritten: the picture
            // is already there, so nothing is sent (the measured cost of
            // repeating it is the picture itself, every frame).
            crate::terminal::capture::start();
            super::super::paint_images(&mut terminal, &mut workbench, &[]);
            let untouched = crate::terminal::capture::take();
            let shown = head(&untouched);
            assert!(
                !untouched.contains("a=T"),
                "an untouched picture had its pixels transmitted again: {shown:?}"
            );
            assert!(
                untouched.contains("a=p"),
                "the placement was dropped from an otherwise unchanged frame: {shown:?}"
            );

            // The box closes: what was placed is deleted rather than left on top
            // of the rows painted over it.
            workbench.overlay = None;
            let _ = workbench.render(&data, 120, 38);
            assert!(
                workbench.image_requests.is_empty(),
                "a closed box still asks for the picture"
            );
            crate::terminal::capture::start();
            super::super::paint_images(&mut terminal, &mut workbench, &[]);
            let cleared = crate::terminal::capture::take();
            let shown = head(&cleared);
            assert!(
                cleared.contains("a=d"),
                "the picture was not deleted when its box closed: {shown:?}"
            );
            assert!(
                workbench.painted_images.ids.is_empty(),
                "a deleted placement was kept in the bookkeeping"
            );
        }

        /// iTerm2 and sixel paint *into the text grid*, so a frame that rewrote
        /// the picture's rows erased it and the placement has to be repeated —
        /// and a frame that left those cells alone must not repeat the bytes.
        /// The kitty test above is a different road (a layer of its own, placed
        /// by id); this one is the road that has to compare rows.
        #[test]
        fn an_inline_protocol_is_asked_again_only_after_a_row_covered_it() {
            let _caps = CapabilitiesGuard::pin(ImageProtocol::Iterm2);
            let mut data = demo::chat();
            data.preview = Some(demo::preview_image());
            let mut workbench = Workbench::new();
            workbench.overlay = Some(Overlay::FilePreview);
            let _ = workbench.render(&data, 120, 38);
            let request = workbench
                .image_requests
                .first()
                .expect("the box asks for its picture")
                .request;
            let placement = (request.x, request.y, request.columns, request.rows);
            let mut terminal = Terminal::new().expect("terminal");

            // A frame that repainted the picture's own row: the picture is text
            // it just overwrote, so it goes back in.
            crate::terminal::capture::start();
            super::super::paint_images(&mut terminal, &mut workbench, &[usize::from(request.y)]);
            let placed = crate::terminal::capture::take();
            let shown = head(&placed);
            assert!(
                placed.contains("1337;File="),
                "an iTerm2 terminal was not asked to place the picture: {shown:?}"
            );
            assert_eq!(
                workbench.painted_images.inline_shown,
                vec![(request.source, placement)],
                "the placement was not remembered, so the next frame would repeat it"
            );

            // A frame that touched none of those rows: the picture is still on
            // screen, so the (large) image payload is not sent again.
            crate::terminal::capture::start();
            super::super::paint_images(&mut terminal, &mut workbench, &[]);
            let untouched = crate::terminal::capture::take();
            let shown = head(&untouched);
            assert!(
                !untouched.contains("1337;File="),
                "an untouched in-line picture was transmitted again: {shown:?}"
            );
            assert_eq!(
                workbench.painted_images.inline_shown,
                vec![(request.source, placement)],
                "the placement was forgotten by a frame that did not disturb it"
            );

            // …and the next disturbed frame places it once more.
            crate::terminal::capture::start();
            super::super::paint_images(&mut terminal, &mut workbench, &[usize::from(request.y)]);
            let again = crate::terminal::capture::take();
            let shown = head(&again);
            assert!(
                again.contains("1337;File="),
                "a rewritten picture was not placed again: {shown:?}"
            );
        }

        /// Sixel is the other in-line protocol, and its payload is a DCS
        /// sequence rather than an OSC one: the terminal asks for a bitmap and
        /// gets sixel, not a PNG.
        #[test]
        fn a_sixel_terminal_gets_the_bitmap_as_a_sixel_sequence() {
            let _caps = CapabilitiesGuard::pin(ImageProtocol::Sixel);
            let mut data = demo::chat();
            data.preview = Some(demo::preview_image());
            let mut workbench = Workbench::new();
            workbench.overlay = Some(Overlay::FilePreview);
            let _ = workbench.render(&data, 120, 38);
            assert!(
                !workbench.image_requests.is_empty(),
                "a sixel terminal is asked to place the picture"
            );
            let mut terminal = Terminal::new().expect("terminal");
            crate::terminal::capture::start();
            super::super::paint_images(&mut terminal, &mut workbench, &[0]);
            let painted = crate::terminal::capture::take();
            let shown = head(&painted);
            assert!(
                painted.contains("\x1bP0;1;0q"),
                "the picture was not encoded as sixel: {shown:?}"
            );
            assert!(
                painted.contains("\x1b\\"),
                "the sixel sequence was not terminated: {shown:?}"
            );
        }

        /// A terminal that cannot place pictures at all keeps no bookkeeping:
        /// the ids and in-line placements recorded by an earlier frame (a
        /// terminal multiplexer can lose the capability when it is re-attached)
        /// are dropped rather than left to describe images it is not holding.
        #[test]
        fn a_terminal_that_cannot_place_pictures_forgets_what_it_placed() {
            let _caps = CapabilitiesGuard::pin(ImageProtocol::None);
            let mut workbench = Workbench::new();
            workbench.painted_images.remember(7, 3);
            workbench
                .painted_images
                .inline_shown
                .push((8, (0, 0, 2, 2)));
            let mut terminal = Terminal::new().expect("terminal");
            crate::terminal::capture::start();
            super::super::paint_images(&mut terminal, &mut workbench, &[]);
            let painted = crate::terminal::capture::take();
            let shown = head(&painted);
            assert!(
                painted.is_empty(),
                "a picture-less terminal was written an image sequence: {shown:?}"
            );
            assert!(
                workbench.painted_images.ids.is_empty(),
                "the ids described pictures this terminal cannot hold"
            );
            assert!(
                workbench.painted_images.inline_shown.is_empty(),
                "the in-line placements were kept for a terminal with no pictures"
            );
        }

        /// The surface keeps a bounded number of transmitted pictures and gives
        /// the oldest up: an image the terminal is still holding would otherwise
        /// stay on screen with nothing left to place or delete it from.
        #[test]
        fn the_oldest_picture_is_deleted_when_the_surface_holds_too_many() {
            let _caps = CapabilitiesGuard::pin(ImageProtocol::Kitty);
            let pixels = demo::picture_pixels();
            let mut workbench = Workbench::new();
            for index in 0..=(crate::workbench::MAX_PAINTED_IMAGES as u64) {
                workbench
                    .image_requests
                    .push(crate::workbench::screen::PlacedPicture {
                        request: crate::workbench::screen::ImageRequest {
                            source: 1_000 + index,
                            x: 0,
                            y: 0,
                            columns: 4,
                            rows: 2,
                            crop: (0, 0, pixels.width, pixels.height),
                        },
                        pixels: pixels.clone(),
                    });
            }
            let mut terminal = Terminal::new().expect("terminal");
            crate::terminal::capture::start();
            super::super::paint_images(&mut terminal, &mut workbench, &[]);
            let painted = crate::terminal::capture::take();
            let shown = head(&painted);
            assert_eq!(
                workbench.painted_images.ids.len(),
                crate::workbench::MAX_PAINTED_IMAGES,
                "the surface kept more pictures than it is allowed to"
            );
            assert!(
                painted.contains("a=d,d=I,i="),
                "the evicted picture was not deleted, so it stays on screen: {shown:?}"
            );
        }
    }

    mod pin_copy_notify {
        use super::*;
        use crate::workbench::preferences::{Action, Keybindings};

        /// `/pin` is a preference of this client's rail: it lands in
        /// `settings.json` beside the layout, and the launchpad's staged model is
        /// remembered the same way — the driver's `persist_pending` writes only
        /// when something actually changed.
        #[tokio::test]
        async fn a_pin_and_a_staged_model_reach_the_settings_file() {
            let mut driver = Driver::attach_sessions().await;
            assert!(!driver.command("/pin").await, "/pin does not leave");
            let written = super::super::load_settings(&driver.settings_path);
            assert_eq!(
                written.pinned_sessions.as_deref().map(|pins| pins.len()),
                Some(1),
                "the pin never reached settings.json: {:?}",
                written.pinned_sessions
            );

            // The launchpad's model chip stages a choice and the driver writes it
            // as the next session's default (`persist_pending`). The typed
            // `/model` opens a picker instead, so the helper is driven here.
            let mut staged = Driver::attach(&[("get_state", r#"{"sessionId":"s1"}"#)]).await;
            staged
                .live
                .apply(crate::workbench::WbAction::SetPendingModel(
                    "future/new".into(),
                ))
                .await;
            staged
                .live
                .apply(crate::workbench::WbAction::SetPendingThinking(
                    "high".into(),
                ))
                .await;
            super::super::persist_pending(
                &mut staged.live,
                &mut staged.settings,
                &staged.settings_path,
            );
            let written = super::super::load_settings(&staged.settings_path);
            assert_eq!(
                written.default_model.as_deref(),
                Some("future/new"),
                "a staged model was not remembered for the next session"
            );
            assert_eq!(
                written.default_thinking_level.as_deref(),
                Some("high"),
                "a staged level was not remembered either"
            );

            // Nothing staged: the file is left exactly as it is.
            let before = std::fs::read(&staged.settings_path).expect("the file");
            super::super::persist_pending(
                &mut staged.live,
                &mut staged.settings,
                &staged.settings_path,
            );
            let after = std::fs::read(&staged.settings_path).expect("the file");
            assert_eq!(before, after, "an unchanged stage rewrote settings.json");
        }

        /// A copy asks the platform's clipboard for the last answer: a success and
        /// a refusal both reach the notice row, and neither ends the app.
        #[tokio::test]
        async fn copying_the_last_answer_reports_what_happened() {
            let mut keys = Keybindings::default();
            keys.bind(Action::Copy, "ctrl+y").expect("a spare chord");
            let mut driver = Driver::attach_conversation().await;
            driver.workbench.keys = keys;

            // Nothing said yet: `copy_last` has no text, and the notice says so.
            assert!(!driver.quit_key("ctrl+y").await, "copy does not leave");
            assert!(
                driver.live.notice.is_some(),
                "copying an empty conversation said nothing"
            );

            // With an answer on screen the copy goes to the clipboard: on a host
            // with no clipboard tool that is the failure sentence, which is the
            // sentence this arm owes the reader either way.
            {
                // An answer on screen: a streamed chunk is how one gets there
                // through the public surface.
                let mut answer = crate::rpc::types::AgentEvent {
                    r#type: "text_chunk".into(),
                    data: serde_json::json!({"text": "the build is green"}),
                    session_id: Some(driver.session_id()),
                    run_id: Some("run-1".into()),
                    ..Default::default()
                };
                driver.live.handle_event(answer.clone());
                answer.r#type = "agent_end".into();
                driver.live.handle_event(answer);
            }
            driver.live.notice = None;
            assert!(!driver.quit_key("ctrl+y").await);
            assert!(
                driver.live.notice.is_some(),
                "a copy with text reported nothing at all"
            );
        }

        /// The identity a notification is de-duplicated by: an approval uses the
        /// request id the agent minted, everything else its event id — so the same
        /// approval arriving twice is one notification and two different errors
        /// are two.
        #[test]
        fn a_notification_is_identified_by_the_events_own_id() {
            use crate::rpc::types::AgentEvent;

            let approval = AgentEvent {
                r#type: "approval_request".into(),
                data: serde_json::json!({"approval_request_id": "a-1"}),
                event_id: Some("e-1".into()),
                ..Default::default()
            };
            assert_eq!(
                super::super::alert_id(&approval),
                "approval_request:a-1",
                "an approval is identified by its request id"
            );

            let error = AgentEvent {
                r#type: "error".into(),
                data: serde_json::json!({}),
                event_id: Some("e-2".into()),
                ..Default::default()
            };
            assert_eq!(super::super::alert_id(&error), "error:e-2");

            // A payload with neither id: the kind alone, which still rings once
            // per kind rather than never.
            let bare = AgentEvent {
                r#type: "error".into(),
                data: serde_json::json!({}),
                ..Default::default()
            };
            assert_eq!(super::super::alert_id(&bare), "error:");
        }

        /// A file preview is the driver's box to open: the bytes are the
        /// adapter's, the box and its sibling list are the surface's.
        #[tokio::test]
        async fn opening_the_preview_box_resets_it_and_takes_the_scope() {
            let mut driver = Driver::attach_conversation().await;
            driver
                .workbench
                .set_preview_siblings((vec!["/tmp/other.png".into()], 1));
            super::super::open_preview_box(
                &mut driver.live,
                &mut driver.workbench,
                "/tmp/picture.png",
                crate::workbench::PreviewScope::Folder,
            );
            assert_eq!(
                driver.workbench.preview_scope,
                crate::workbench::PreviewScope::Folder,
                "the box did not take the scope it was opened with"
            );
            assert!(
                driver.workbench.preview_siblings.is_empty(),
                "the box kept the previous visit's sibling list: {:?}",
                driver.workbench.preview_siblings
            );
        }

        /// `/copy` with an answer on screen puts it on the clipboard, and the
        /// notice row says which way it went: the success sentence when a tool
        /// took the text, the refusal — naming the tool — when there is none.
        ///
        /// The answer comes from the agent's own entries. A streamed
        /// `text_chunk` cannot stand in for it: `append_to_last_message` appends
        /// to an assistant message and drops the text when the transcript has
        /// none, which is what made this arm look unreachable.
        #[allow(clippy::await_holding_lock)]
        #[tokio::test]
        async fn a_copy_of_the_answer_says_which_way_the_clipboard_went() {
            use crate::workbench::i18n::{t, Key};

            let entries = r#"{"entries":[
                {"id":"e2","role":"assistant","blocks":[{"kind":"text","text":"the build is green"}],
                 "createdAtMs":2}]}"#;
            let mut keys = Keybindings::default();
            keys.bind(Action::Copy, "ctrl+y").expect("a spare chord");
            let mut driver = Driver::attach_conversation_holding(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
                entries,
            )
            .await;
            driver.workbench.keys = keys;
            let language = driver.live.language();

            // A tool that takes the text: the row carries the success sentence.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;

                let _env = a_peer_may_not_swap_the_environment();
                let tools = tempfile::tempdir().expect("tempdir");
                // Every clipboard tool a unix host can pick, so the phase does
                // not depend on which one this machine's build reaches for.
                for name in ["pbcopy", "xclip", "wl-copy"] {
                    let stub = tools.path().join(name);
                    std::fs::write(&stub, "#!/bin/sh\ncat >/dev/null\nexit 0\n")
                        .expect("write the stub");
                    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                        .expect("chmod the stub");
                }
                let _path = crate::test_env::EnvVarGuard::set("PATH", tools.path());
                driver.live.notice = None;
                assert!(!driver.quit_key("ctrl+y").await, "copy left the app");
                let notice = driver
                    .live
                    .notice
                    .clone()
                    .expect("a copy a tool accepted said nothing");
                assert_eq!(
                    notice,
                    t(language, Key::CopyDone),
                    "the row does not carry the success sentence"
                );
            }

            // No tool at all: the refusal is a sentence that names it.
            {
                let _env = a_peer_may_not_swap_the_environment();
                let empty = tempfile::tempdir().expect("tempdir");
                let _path = crate::test_env::EnvVarGuard::set("PATH", empty.path());
                driver.live.notice = None;
                assert!(!driver.quit_key("ctrl+y").await, "copy left the app");
                let notice = driver
                    .live
                    .notice
                    .clone()
                    .expect("a refused copy said nothing");
                assert_ne!(
                    notice,
                    t(language, Key::CopyDone),
                    "a refused copy read as a success"
                );
                assert!(
                    ["pbcopy", "xclip", "wl-copy", "powershell"]
                        .iter()
                        .any(|tool| notice.contains(tool)),
                    "the refusal does not name the tool it could not start: {notice:?}"
                );
            }
        }

        /// A bindings file that cannot be written is a sentence on the notice
        /// row: the reader has just changed a key and must not believe it was
        /// saved. One road for every settings write, so the failure is the same
        /// `NoticeSettingsSaveFailed` the file's own write uses.
        #[tokio::test]
        async fn a_keybinding_that_cannot_be_saved_is_reported() {
            let mut driver = Driver::attach_conversation().await;
            // A regular file standing where the directory has to be: every write
            // below it fails with ENOTDIR, which the writer cannot create its
            // way out of.
            let blocker = driver
                .settings_path
                .parent()
                .expect("a parent")
                .join("blocker");
            std::fs::write(&blocker, b"not a directory").expect("write the blocker");
            driver.settings_path = blocker.join("settings.json");
            driver.workbench.overlay = Some(crate::workbench::Overlay::Keybindings);
            driver.live.notice = None;

            // `r` in the bindings table writes the defaults back out.
            assert!(!driver.quit_key("r").await, "the reset left the app");
            let notice = driver
                .live
                .notice
                .clone()
                .expect("a save that failed was swallowed");
            assert!(!notice.is_empty(), "the failed save left an empty notice");
            // The sentence is the settings writer's own, not a generic one: its
            // template is what every failed write reports.
            let template = crate::workbench::i18n::t(
                driver.live.language(),
                crate::workbench::i18n::Key::NoticeSettingsSaveFailed,
            );
            let prefix = template
                .split("{}")
                .next()
                .unwrap_or_default()
                .trim_end_matches([':', '：', ' ']);
            assert!(
                !prefix.is_empty() && notice.starts_with(prefix),
                "the failed save did not use the settings writer's sentence: {notice:?} vs {template:?}"
            );
            assert!(
                !crate::workbench::preferences::keybindings_path(&driver.settings_path).exists(),
                "the bindings file exists although its directory cannot hold one"
            );
        }

        /// The `[复制]` control under one message is the same clipboard road
        /// `/copy` takes — one message instead of the last answer — and the
        /// loop, not the surface, owns the clipboard.
        ///
        /// Reached the way a reader reaches it: `tab` walks the transcript's
        /// stops and `enter` presses the chip, so the case cannot pass by
        /// handing the loop the action it means to check. What the clipboard
        /// received is read back from the tool, which is the only proof that
        /// *that* message was copied rather than the last answer.
        #[cfg(unix)]
        #[allow(clippy::await_holding_lock)]
        #[tokio::test]
        async fn a_copy_of_one_message_takes_that_message_and_says_so() {
            use std::os::unix::fs::PermissionsExt as _;

            use crate::components::chat_area::RowAction;
            use crate::workbench::i18n::{t, Key};
            use crate::workbench::ChatStop;

            let entries = r#"{"entries":[
                {"id":"e1","role":"user","blocks":[{"kind":"text","text":"the question"}],
                 "createdAtMs":1},
                {"id":"e2","role":"assistant","blocks":[{"kind":"text","text":"the build is green"}],
                 "createdAtMs":2}]}"#;
            let mut driver = Driver::attach_conversation_holding(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
                entries,
            )
            .await;

            // A tool that keeps what it was handed: every clipboard a unix build
            // can pick, so the phase does not depend on which one this host has.
            let _env = a_peer_may_not_swap_the_environment();
            let tools = tempfile::tempdir().expect("tempdir");
            let taken = tools.path().join("clipboard.txt");
            for name in ["pbcopy", "xclip", "wl-copy"] {
                let stub = tools.path().join(name);
                // `cat` by absolute path: the stub runs with the same PATH the
                // test swapped, which carries no coreutils at all.
                std::fs::write(
                    &stub,
                    format!("#!/bin/sh\n/bin/cat > {}\n", taken.display()),
                )
                .expect("write the stub");
                std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                    .expect("chmod the stub");
            }
            let _path = crate::test_env::EnvVarGuard::set("PATH", tools.path());

            // Walk the transcript's stops until the cursor is on a copy chip.
            let mut on_a_copy = false;
            for _ in 0..12 {
                assert!(!driver.quit_key("tab").await, "tab left the app");
                let frame = driver.frame();
                let data = driver.live.data(&frame);
                let index = driver.workbench.chat_index;
                let stop =
                    index.and_then(|at| driver.workbench.chat_stops(&data).into_iter().nth(at));
                let copy = match stop {
                    Some(ChatStop::Row(row)) => matches!(row.action, RowAction::Copy { .. }),
                    _ => false,
                };
                if copy {
                    on_a_copy = true;
                    break;
                }
            }
            assert!(on_a_copy, "no copy chip among the transcript's stops");

            driver.live.notice = None;
            assert!(!driver.quit_key("enter").await, "enter left the app");
            let language = driver.live.language();
            let notice = driver
                .live
                .notice
                .clone()
                .expect("a copy of one message said nothing");
            assert_eq!(
                notice,
                t(language, Key::CopiedMessage),
                "the row does not carry the copy's own sentence"
            );
            let copied = std::fs::read_to_string(&taken)
                .expect("the clipboard tool kept nothing: the copy never reached it");
            assert_eq!(
                copied, "the build is green",
                "the clipboard holds something other than the message under the cursor"
            );
        }
    }

    mod preview_doors {
        use super::*;
        use crate::workbench::preferences::{Action, Keybindings};
        use crate::workbench::{Overlay, PreviewScope};

        /// The composer's own picture opens the box: the attachment the reader
        /// pasted is what `PreviewAttachment` hands to the adapter, and the box
        /// comes up on the conversation the picture belongs to.
        ///
        /// The path is **pasted**, not typed: a whole line is what the composer
        /// resolves into an attachment (`resolve_image_path` sniffs the bytes),
        /// and a path typed one byte at a time stays text.
        #[tokio::test]
        async fn an_attachment_preview_opens_the_box_on_the_draft_picture() {
            let dir = tempfile::tempdir().expect("tempdir");
            let shot = dir.path().join("shot.png");
            std::fs::write(&shot, b"\x89PNG\r\n\x1a\nrest of the file").expect("write the picture");

            let mut keys = Keybindings::default();
            // The action ships with no chord of its own (the composer's title is
            // its visible door), so a reader who wants a key binds one — which is
            // exactly what this presses.
            keys.bind(Action::PreviewAttachment, "ctrl+y")
                .expect("a spare chord");
            let mut driver = Driver::attach_conversation().await;
            driver.workbench.keys = keys;

            driver.data(&format!("\x1b[200~{}\x1b[201~", shot.display()));
            let _ = driver.pump(Duration::from_millis(300)).await;
            let frame = driver.frame();
            let draft = driver.live.data(&frame).input.editor.text().to_string();
            assert!(
                draft.contains("[Image #1]"),
                "the pasted picture did not become an attachment: {draft:?}"
            );

            assert!(!driver.quit_key("ctrl+y").await, "the chord left the app");
            assert_eq!(
                driver.workbench.overlay,
                Some(Overlay::FilePreview),
                "the attachment opened no box"
            );
            assert_eq!(
                driver.workbench.preview_scope,
                PreviewScope::Conversation,
                "the box stepped through the folder instead of the conversation"
            );
            assert!(
                driver.workbench.preview_siblings.is_empty(),
                "the box kept a list the attachment door never gave it: {:?}",
                driver.workbench.preview_siblings
            );
        }

        /// A file the conversation names is a picture in its own scope: `tab`
        /// walks the transcript's links, and `enter` on the file one opens the
        /// box over the conversation — not over the folder the path happens to
        /// sit in.
        #[tokio::test]
        async fn a_file_the_answer_links_opens_the_box_over_the_conversation() {
            let dir = tempfile::tempdir().expect("tempdir");
            let shot = dir.path().join("chart.png");
            std::fs::write(&shot, b"\x89PNG\r\n\x1a\nrest of the file").expect("write the picture");

            let text = format!("the build is green — see [the chart]({})", shot.display());
            let entries = serde_json::json!({"entries": [{
                "id": "e2",
                "role": "assistant",
                "blocks": [{"kind": "text", "text": text}],
                "createdAtMs": 2
            }]})
            .to_string();
            let mut driver = Driver::attach_conversation_holding(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
                &entries,
            )
            .await;

            assert!(!driver.quit_key("tab").await, "tab left the app");
            assert_eq!(
                driver.workbench.chat_index,
                Some(0),
                "the cursor never walked onto the answer's link"
            );
            assert!(!driver.quit_key("enter").await, "enter left the app");
            assert_eq!(
                driver.workbench.overlay,
                Some(Overlay::FilePreview),
                "the linked file opened no box"
            );
            assert_eq!(
                driver.workbench.preview_scope,
                PreviewScope::Conversation,
                "a conversation's own picture opened in the folder's scope"
            );
        }
    }

    mod bursts {
        use super::*;

        /// What the composer holds right now, read the way the surface reads it.
        fn draft_text(driver: &mut Driver) -> String {
            let frame = driver.frame();
            driver.live.data(&frame).input.editor.text().to_string()
        }

        /// A **signal** in the middle of a burst discards the rest of it: the
        /// loop leaves on the spot, so a keystroke read in the same burst is not
        /// applied. That is the measured behaviour — the burst is drained before
        /// it is processed, and the signal returns from inside the drain.
        #[tokio::test]
        async fn a_signal_inside_a_burst_leaves_before_the_keys_are_applied() {
            let mut driver = Driver::attach_conversation().await;
            driver.data("half typed");
            driver.signal();
            let (outcome, _painted) = driver.run().await;
            assert!(outcome.is_ok(), "a signal is a clean exit: {outcome:?}");
            assert_eq!(
                draft_text(&mut driver),
                "",
                "a key from a burst that ended in a signal was applied anyway"
            );
        }
    }

    mod doors_that_refuse {
        use super::*;
        use crate::workbench::Overlay;

        /// Two verbs read the repository through `git`, and when that read fails
        /// each one carries the refusal into its own surface: `/worktree` on the
        /// notice row with its box opened, `d` as the review box's own error.
        ///
        /// `git` is spawned **by name**, so the refusal is a `PATH` with no
        /// `git` on it — which is the `Ok(Err(..))` arm (the tool could not be
        /// started), not the `Err(..)` join arm that only a panicking closure
        /// reaches.
        #[allow(clippy::await_holding_lock)]
        #[tokio::test]
        async fn a_repository_read_that_failed_reaches_its_panel() {
            let _env = a_peer_may_not_swap_the_environment();
            let empty = tempfile::tempdir().expect("tempdir");
            let mut driver = Driver::attach_conversation().await;
            let _path = crate::test_env::EnvVarGuard::set("PATH", empty.path());

            assert!(!driver.command("/worktree").await, "/worktree left the app");
            let notice = driver
                .live
                .notice
                .clone()
                .expect("a probe that could not run said nothing");
            assert!(!notice.is_empty(), "the failed probe left an empty notice");
            assert_eq!(
                driver.workbench.overlay,
                Some(Overlay::Worktree),
                "the box the reader asked for did not open on the refusal"
            );
        }

        /// The working-tree review is the same read one level down: `d` in the
        /// repository browser asks git for the tree, and a workspace that went
        /// away under a loaded panel is that read failing — the box opens with
        /// the refusal in it instead of an empty review that looks clean.
        #[allow(clippy::await_holding_lock)]
        #[tokio::test]
        async fn a_review_of_a_workspace_that_went_away_says_so() {
            let _env = a_peer_may_not_swap_the_environment();
            let git = crate::test_env::absolute_tool("git");
            let repo = tempfile::tempdir().expect("tempdir");
            let created = std::process::Command::new(&git)
                .arg("-C")
                .arg(repo.path())
                .arg("init")
                .output()
                .expect("git init");
            assert!(created.status.success(), "git init: {created:?}");
            std::fs::write(repo.path().join("a.txt"), b"one\n").expect("write a file");
            for args in [
                vec!["add", "a.txt"],
                vec![
                    "-c",
                    "user.email=t@e",
                    "-c",
                    "user.name=t",
                    "commit",
                    "-m",
                    "one",
                ],
            ] {
                let done = std::process::Command::new(&git)
                    .arg("-C")
                    .arg(repo.path())
                    .args(&args)
                    .output()
                    .expect("git");
                assert!(done.status.success(), "git {args:?}: {done:?}");
            }

            let mut driver = Driver::attach_conversation_in(repo.path()).await;
            assert!(!driver.quit_key("ctrl+g").await, "ctrl+g left the app");
            assert!(
                driver
                    .wait_for(Duration::from_secs(10), |driver| driver
                        .live
                        .git()
                        .is_some())
                    .await,
                "the panel did not read the repository it was pointed at"
            );

            // The workspace the panel was read from is gone by the time the
            // review asks for it.
            std::fs::remove_dir_all(repo.path()).expect("remove the workspace");
            assert!(!driver.quit_key("d").await, "the review key left the app");
            assert_eq!(
                driver.workbench.overlay,
                Some(Overlay::Diff),
                "the review box did not open on the refusal"
            );
            let frame = driver.frame();
            let diff = driver
                .live
                .data(&frame)
                .diff
                .expect("the review box was never filled");
            assert!(
                diff.error.is_some(),
                "a workspace that is not there produced a review with no error: {diff:?}"
            );
        }

        /// A copy the platform clipboard refuses is a sentence on the notice row,
        /// not a silent no-op. The git panel's `y` is the door that hands a row
        /// to the platform tool, and a `PATH` that carries `git` but no clipboard
        /// tool is that refusal.
        #[cfg(unix)]
        #[allow(clippy::await_holding_lock)]
        #[tokio::test]
        async fn a_copy_the_clipboard_refuses_is_reported() {
            let _env = a_peer_may_not_swap_the_environment();
            // No tool of any kind on `PATH`: the only read this case needs is the
            // clipboard's, and that is the one that cannot run.
            let empty = tempfile::tempdir().expect("tempdir");
            let _path = crate::test_env::EnvVarGuard::set("PATH", empty.path());

            // The panel is seeded rather than read. `y` copies the row under the
            // cursor, and the *read* is a different measurement: it carries a
            // three-second query budget, so under an instrumented parallel suite
            // it can time out and leave a view with no row in it — which made this
            // case fail on "the copy said nothing" while the cause was one level
            // up (`timed out after 3s`, r14). The read's own failures are the
            // wave-6 cases; this one is about what happens after it succeeded.
            let mut driver = Driver::attach_conversation().await;
            driver.live.set_git(Some(crate::workbench::state::GitView {
                repo: "seeded".into(),
                section: crate::workbench::state::GitSection::Commits,
                rows: vec![crate::workbench::state::GitRow {
                    key: Some("abc1234".into()),
                    columns: Vec::new(),
                    prefix: "● ".into(),
                    pieces: vec!["a commit".into()],
                    kinds: Vec::new(),
                }],
                selected: 0,
                detail: Vec::new(),
                filter: None,
                error: None,
            }));
            driver.workbench.overlay = Some(Overlay::Git);

            assert!(!driver.quit_key("y").await, "the copy key left the app");
            let notice = driver
                .live
                .notice
                .clone()
                .expect("a refused copy said nothing at all");
            assert!(
                ["pbcopy", "pbpaste", "xclip", "wl-copy", "powershell"]
                    .iter()
                    .any(|tool| notice.contains(tool)),
                "the notice does not name the tool that refused: {notice:?}"
            );
        }

        /// The palette's `/pin` row is the second door to the rail's verb. The
        /// draft answers `/pin` in the surface; a committed palette row hands the
        /// same verb to the driver, and both must pin the conversation.
        #[tokio::test]
        async fn the_palettes_pin_reaches_the_rail_the_draft_answers_in_the_surface() {
            let mut driver = Driver::attach_sessions().await;
            assert!(!driver.type_text("/pin").await, "typing left the app");
            let selected = {
                let frame = driver.frame();
                let data = driver.live.data(&frame);
                driver.workbench.palette_selection(&data)
            };
            assert_eq!(
                selected.as_ref().map(|row| row.command()),
                Some("/pin".to_string()),
                "the palette's first row for `pin` is not the verb: {selected:?}"
            );
            assert!(
                !driver.quit_key("enter").await,
                "committing the row left the app"
            );
            let written = super::super::load_settings(&driver.settings_path);
            assert_eq!(
                written.pinned_sessions.as_deref().map(<[String]>::to_vec),
                Some(vec![driver.session_id()]),
                "the palette's pin never reached settings.json: {:?}",
                written.pinned_sessions
            );

            // The same verb with nothing to pin: the launchpad has no
            // conversation under the rail's cursor, so the answer is a sentence
            // and the loop stays.
            let mut fresh = Driver::attach(&[]).await;
            assert!(!fresh.type_text("/pin").await, "typing left the app");
            assert_eq!(
                fresh.workbench.overlay,
                Some(Overlay::Palette),
                "`/` did not open the palette on the launchpad"
            );
            assert!(!fresh.quit_key("enter").await, "enter left the app");
            let selected = {
                let frame = fresh.frame();
                let data = fresh.live.data(&frame);
                fresh
                    .workbench
                    .palette_selection(&data)
                    .map(|row| row.command())
            };
            assert_eq!(
                selected.as_deref(),
                Some("/pin"),
                "the launchpad's palette row for `pin`"
            );
            // What the launchpad's pin leaves behind: nothing pinned. The
            // sentence the surface answers with (`NoticePinNewSession`) does not
            // reach the notice row on this door — the palette's verb is handed to
            // `Live::apply`, whose `WbAction::Notice` arm is a no-op
            // (`live.rs:1979`), while the typed `/pin` is answered by the loop's
            // own `Notice` arm (`run.rs:1390`). Measured, not wished for: the
            // assertion is the invariant that survives the gap being closed, and
            // the gap itself is recorded in the ledger.
            let written = super::super::load_settings(&fresh.settings_path);
            assert_eq!(
                written.pinned_sessions, None,
                "a pin with nothing to pin still wrote a preference: {:?}",
                written.pinned_sessions
            );
        }
    }
}
