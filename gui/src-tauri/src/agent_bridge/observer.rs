//! Per-session observers: the GUI's always-on tap into each agent session's
//! event stream. One observer task per known session, alive regardless of
//! which thread the UI shows — switching conversations only changes what the
//! frontend renders, never what observers receive.
//!
//! Each observer subscribes to its session's event stream and fans events out
//! to four sinks, per session, with no cross-session shared mutable state:
//!
//! 1. **Persistence** — for runs NOT owned by a prompt-pipeline collector
//!    (i.e. started by the TUI/CLI/another machine, or reanimated after a GUI
//!    restart), the observer creates the local run row (id == the agent's
//!    canonical run id, so the existing crash-reconcile machinery matches it)
//!    and projects events into the run-event log. Runs owned by a pipeline
//!    collector are persisted by that collector — exactly one writer per run:
//!    the pipeline registers its lease before the prompt ever reaches the
//!    agent, and `append_run_event`'s sequence guard absorbs any residual
//!    replay overlap.
//! 2. **NATS mirroring** — the sole publisher for the remote bridge. The
//!    collector deliberately does not publish; the observer's atomic-attach
//!    replay guarantees the mirrored sequence has no holes. Events are
//!    validated against the run cursor BEFORE any fan-out (persistence,
//!    webview forward, mirror publish), so the mirrored sequence stays
//!    in-order and duplicate-free across re-attaches.
//! 3. **Frontend invalidation** — `thread-runtime-updated` for persisted runs,
//!    plus whitelisted settings events (`agent-event`) for every session, so
//!    model/thinking/title changes land in the sidebar cache live.
//! 4. **Settlement** — observer-owned runs are settled on `agent_end`/`error`
//!    (the pipeline settles its own runs).
//!
//! Capacity: at most [`OBSERVER_MAX`] live observers. Only idle observers (no
//! active run) are evicted, least-recently-active first; an observer tracking
//! an active run is never evicted — the cap yields to overflow in that case.
//! An observer with no events for [`OBSERVER_IDLE_SLEEP`] and no active run
//! puts itself to sleep; discovery (streaming poll, thread open, prompt) wakes
//! it again via [`ensure_observer`].

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::Duration,
};

use tauri::Emitter;
use tokio::sync::oneshot;
use tonic::Code;

use super::{
    client::{base_command, get_state_command},
    connect_agent,
    run_control::{mark_run_completed_if_active, mark_run_failed_if_active},
    stream,
};
use crate::agent_proto::StreamRequest;

/// Process-wide cap on live observers (design default: 128).
const OBSERVER_MAX: usize = 128;
/// An observer with no events for this long and no active run exits (sleeps).
const OBSERVER_IDLE_SLEEP: Duration = Duration::from_secs(15 * 60);
/// How often a quiet stream is checked for the idle-sleep condition.
const IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// Event types forwarded to the webview as `agent-event`. Same whitelist as
/// the retired single-slot observer: settings changes applied by
/// `agentStateCache`, plus `user_message` (zero-latency user bubble in the
/// open thread). Per-token content (`text_chunk`, `thinking_delta`, `tool_*`)
/// is never forwarded — the frontend renders content from the persisted
/// run-event log.
const FORWARDED_EVENTS: &[&str] = &[
    "agent_start",
    "agent_end",
    "user_message",
    "model_changed",
    "thinking_level_changed",
    "permission_level_changed",
    "session_name_changed",
    "cwd_changed",
    "config_reloaded",
];

struct ObserverHandle {
    cancel: oneshot::Sender<()>,
    shared: Arc<ObserverShared>,
}

/// State shared between the manager and one observer task: eviction inputs
/// (activity, active-run flag), the session→thread mapping, and the
/// canonical→local run bindings the task builds as it projects runs.
struct ObserverShared {
    last_activity_ms: AtomicI64,
    has_active_run: AtomicBool,
    thread_id: Mutex<Option<String>>,
    run_bindings: Mutex<HashMap<String, String>>,
}

impl ObserverShared {
    fn new() -> Self {
        Self {
            last_activity_ms: AtomicI64::new(now_millis()),
            has_active_run: AtomicBool::new(false),
            thread_id: Mutex::new(None),
            run_bindings: Mutex::new(HashMap::new()),
        }
    }

    fn touch(&self) {
        self.last_activity_ms.store(now_millis(), Ordering::Relaxed);
    }

    /// Idle = no active run AND quiet for the full sleep window. Eviction
    /// treats any shorter quiet period as a valid victim; self-sleep requires
    /// the full window so a briefly-quiet session isn't churned.
    fn should_sleep(&self) -> bool {
        !self.has_active_run.load(Ordering::Relaxed)
            && now_millis() - self.last_activity_ms.load(Ordering::Relaxed)
                > OBSERVER_IDLE_SLEEP.as_millis() as i64
    }
}

/// Live observers keyed by agent session id.
static OBSERVERS: LazyLock<Mutex<HashMap<String, ObserverHandle>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Register (or refresh) the observer for a session. User activity and an
/// active prompt use this entry point, so an existing observer is touched for
/// LRU purposes.
pub fn ensure_observer(session_id: &str) {
    ensure_observer_inner(session_id, true);
}

/// Ensure a passive observer without treating periodic discovery as user
/// activity. Otherwise the 60-second import loop would keep every idle entry
/// permanently hot and defeat the 128-observer LRU cap.
fn ensure_passive_observer(session_id: &str) {
    ensure_observer_inner(session_id, false);
}

fn ensure_observer_inner(session_id: &str, touch_existing: bool) {
    if session_id.trim().is_empty() {
        return;
    }
    let mut guard = OBSERVERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(handle) = guard.get(session_id) {
        if touch_existing {
            handle.shared.touch();
        }
        return;
    }
    evict_idle_if_over_cap(&mut guard);
    let shared = Arc::new(ObserverShared::new());
    let (cancel_tx, cancel_rx) = oneshot::channel();
    guard.insert(
        session_id.to_string(),
        ObserverHandle {
            cancel: cancel_tx,
            shared: shared.clone(),
        },
    );
    drop(guard);
    spawn_observer(session_id.to_string(), shared, cancel_rx);
}

/// Startup seed: one observer per thread that already has an agent session.
/// Runs synchronously but only spawns tasks — attach/retry happens inside
/// each observer, so a down agent never blocks the caller.
pub fn seed_observers_from_store() {
    let Ok(threads) = crate::store::list_threads() else {
        return;
    };
    for thread in threads {
        if let Some(session_id) = thread
            .agent_session_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            ensure_passive_observer(session_id);
        }
    }
}

/// Background discovery of conversations created outside the GUI (TUI, CLI,
/// channels, another machine). Two cadences: a 1s pass over the agent's
/// streaming sessions — a run started by another client appears in the
/// sidebar within ~1s — and a 60s full import for idle sessions plus
/// observer import. Idle observers are not re-touched here: opening a thread,
/// a prompt, or a newly streaming session wakes it instead.
pub fn spawn_session_discovery() {
    tauri::async_runtime::spawn(async move {
        let mut ticks = 0u64;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            ticks += 1;
            discover_streaming_sessions().await;
            if ticks.is_multiple_of(60) {
                if let Err(error) = super::import::import_missing_sessions().await {
                    eprintln!("FutureOS periodic session import failed: {error}");
                }
            }
        }
    });
}

/// The fast discovery pass: sessions the agent reports as streaming that have
/// no local thread get a thread stub plus an observer.
async fn discover_streaming_sessions() {
    let Ok(mut client) = connect_agent().await else {
        return;
    };
    let response = match client
        .execute_command(super::client::list_streaming_sessions_command())
        .await
    {
        Ok(response) => response.into_inner(),
        Err(_) => return,
    };
    if !response.success {
        return;
    }
    let session_ids: Vec<String> = serde_json::from_str::<serde_json::Value>(&response.data)
        .ok()
        .and_then(|value| value.get("sessionIds")?.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| {
            value
                .as_str()
                .filter(|id| !id.is_empty())
                .map(str::to_string)
        })
        .collect();
    for session_id in session_ids {
        let known = crate::store::find_thread_by_agent_session(&session_id)
            .ok()
            .flatten()
            .is_some();
        if !known {
            match super::import::import_streaming_session(&session_id).await {
                Ok(()) => eprintln!(
                    "FutureOS discovered streaming session {session_id} (created by another client)"
                ),
                Err(error) => {
                    eprintln!("FutureOS could not import discovered session {session_id}: {error}")
                }
            }
        }
        ensure_observer(&session_id);
    }
}

/// Drop the observer for a session going away (thread/session deleted).
pub fn drop_observer(session_id: &str) {
    if let Some(handle) = OBSERVERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(session_id)
    {
        let _ = handle.cancel.send(());
    }
}

/// Evict least-recently-active idle observers while at/over the cap. Active
/// runs are never evicted — if every observer tracks one, the cap overflows.
fn evict_idle_if_over_cap(guard: &mut HashMap<String, ObserverHandle>) {
    while guard.len() >= OBSERVER_MAX {
        let victim = guard
            .iter()
            .filter(|(_, handle)| !handle.shared.has_active_run.load(Ordering::Relaxed))
            .min_by_key(|(_, handle)| handle.shared.last_activity_ms.load(Ordering::Relaxed))
            .map(|(session_id, _)| session_id.clone());
        match victim {
            Some(session_id) => {
                if let Some(handle) = guard.remove(&session_id) {
                    let _ = handle.cancel.send(());
                    eprintln!(
                        "FutureOS observer for {session_id} evicted (LRU cap {OBSERVER_MAX})"
                    );
                }
            }
            None => {
                eprintln!(
                    "FutureOS observer cap {OBSERVER_MAX} exceeded: all observers track active runs"
                );
                return;
            }
        }
    }
}

fn unregister(session_id: &str, shared: &Arc<ObserverShared>) {
    let mut guard = OBSERVERS.lock().unwrap_or_else(|e| e.into_inner());
    if guard
        .get(session_id)
        .is_some_and(|handle| Arc::ptr_eq(&handle.shared, shared))
    {
        guard.remove(session_id);
    }
}

/// Get-or-create the local run row for a canonical agent run, recording the
/// binding for the observer. The row id IS the canonical run id — crash
/// recovery (`check_and_reanimate_run`) matches local rows against the agent's
/// `activeRun.runId`, so synthetic rows must use the agent's id to reanimate.
/// Race-safe: a concurrent creator loses the INSERT and reads back the row.
pub(super) fn ensure_run_binding(
    session_id: &str,
    canonical_run_id: &str,
    thread_id: &str,
) -> Option<String> {
    {
        let shared = OBSERVERS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(local) = shared.get(session_id).and_then(|handle| {
            handle
                .shared
                .run_bindings
                .lock()
                .ok()?
                .get(canonical_run_id)
                .cloned()
        }) {
            return Some(local);
        }
    }
    if let Ok(Some(run)) = crate::store::get_run(canonical_run_id) {
        bind_run(session_id, canonical_run_id, &run.id);
        return Some(run.id);
    }
    let run = match crate::store::create_run(crate::store::CreateRunInput {
        id: Some(canonical_run_id.to_string()),
        thread_id: thread_id.to_string(),
        trigger_message_id: None,
        model_provider: None,
        model_id: None,
    }) {
        Ok(run) => run,
        Err(_) => crate::store::get_run(canonical_run_id).ok()??,
    };
    bind_run(session_id, canonical_run_id, &run.id);
    Some(run.id)
}

fn bind_run(session_id: &str, canonical_run_id: &str, local_run_id: &str) {
    let shared = OBSERVERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(handle) = shared.get(session_id) {
        if let Ok(mut bindings) = handle.shared.run_bindings.lock() {
            bindings.insert(canonical_run_id.to_string(), local_run_id.to_string());
        }
    }
}

// ── Observer task ──────────────────────────────────────────────────────────

/// Task-local stream bookkeeping: per-run event cursors (idx dedup + gap
/// detection), the session's currently-active run (if any), and the most
/// recently settled run — the broadcaster keeps stamping its identity onto
/// between-runs settings events until the next run starts.
struct ObserverState {
    cursors: HashMap<String, i64>,
    session_cursor: i64,
    active_run: Option<String>,
    last_settled_run: Option<String>,
}

impl Default for ObserverState {
    fn default() -> Self {
        Self {
            cursors: HashMap::new(),
            session_cursor: -1,
            active_run: None,
            last_settled_run: None,
        }
    }
}

fn spawn_observer(session_id: String, shared: Arc<ObserverShared>, cancel: oneshot::Receiver<()>) {
    tauri::async_runtime::spawn(async move {
        run_observer(&session_id, &shared, cancel).await;
        unregister(&session_id, &shared);
    });
}

async fn run_observer(
    session_id: &str,
    shared: &Arc<ObserverShared>,
    mut cancel: oneshot::Receiver<()>,
) {
    let mut state = ObserverState::default();
    let mut backoff = Duration::from_millis(500);

    'attach: loop {
        let mut client = match connect_agent().await {
            Ok(client) => client,
            Err(_) => {
                if sleep_or_cancel(&mut cancel, backoff).await {
                    return;
                }
                backoff = (backoff * 2).min(Duration::from_secs(10));
                continue;
            }
        };

        if !replay_session_events(&mut client, session_id, shared, &mut state).await {
            if sleep_or_cancel(&mut cancel, backoff).await {
                return;
            }
            continue;
        }

        // Probe: an active run gets an atomic attach from the local cursor
        // (replaying everything the ring still holds); otherwise a plain live
        // subscription — a run starting later arrives from idx 0 anyway, and a
        // run starting in the probe→subscribe gap is caught by gap detection.
        let active_run = probe_active_run(&mut client, session_id).await;
        let mut stream = if let Some(run_id) = active_run {
            // Mark this before attaching, not on the first streamed event. A
            // quiet active run (for example waiting on an approval) must never
            // be evicted in the probe→first-event window.
            note_run_active(shared, &mut state, &run_id);
            // Resume from the in-memory cursor on re-attach: events at or
            // below it were already persisted and mirrored, so replaying them
            // would only duplicate the NATS mirror. The persisted-store cursor
            // is the resume point for a fresh attach (startup, first sight).
            let cursor = match state.cursors.get(&run_id) {
                Some(&cursor) => cursor,
                None => attach_cursor(session_id, &run_id).await,
            };
            state.cursors.insert(run_id.clone(), cursor);
            match client
                .stream_events(StreamRequest {
                    event_types: vec![],
                    session_id: session_id.to_string(),
                    run_id: run_id.clone(),
                    after_idx: cursor,
                    atomic_attach: true,
                })
                .await
            {
                Ok(response) => response.into_inner(),
                Err(status)
                    if matches!(status.code(), Code::FailedPrecondition | Code::NotFound) =>
                {
                    // The run ended between probe and attach — plain subscribe.
                    state.cursors.remove(&run_id);
                    note_run_settled(shared, &mut state, &run_id);
                    match plain_subscribe(&mut client, session_id).await {
                        Some(stream) => stream,
                        None => {
                            if sleep_or_cancel(&mut cancel, backoff).await {
                                return;
                            }
                            continue 'attach;
                        }
                    }
                }
                Err(_) => {
                    note_run_settled(shared, &mut state, &run_id);
                    if sleep_or_cancel(&mut cancel, backoff).await {
                        return;
                    }
                    backoff = (backoff * 2).min(Duration::from_secs(10));
                    continue 'attach;
                }
            }
        } else {
            match plain_subscribe(&mut client, session_id).await {
                Some(stream) => stream,
                None => {
                    if sleep_or_cancel(&mut cancel, backoff).await {
                        return;
                    }
                    backoff = (backoff * 2).min(Duration::from_secs(10));
                    continue 'attach;
                }
            }
        };
        backoff = Duration::from_millis(500);

        // ── Streaming ──────────────────────────────────────────────
        loop {
            let message = tokio::select! {
                _ = &mut cancel => return,
                result = tokio::time::timeout(IDLE_CHECK_INTERVAL, stream.message()) => result,
            };
            let event = match message {
                Ok(Ok(Some(event))) => event,
                Ok(Ok(None)) | Ok(Err(_)) => break, // closed / data_loss — reattach
                Err(_) => {
                    // Quiet window: sleep check, then keep waiting.
                    if shared.should_sleep() {
                        return;
                    }
                    continue;
                }
            };
            shared.touch();
            if !handle_event(session_id, shared, &mut state, event).await {
                break; // idx gap — reattach for replay/snapshot healing
            }
            if shared.should_sleep() {
                return;
            }
        }

        if sleep_or_cancel(&mut cancel, backoff).await {
            return;
        }
        backoff = (backoff * 2).min(Duration::from_secs(10));
    }
}

async fn plain_subscribe(
    client: &mut crate::agent_proto::FutureAgentClient<tonic::transport::Channel>,
    session_id: &str,
) -> Option<tonic::Streaming<crate::agent_proto::StreamEvent>> {
    client
        .stream_events(StreamRequest {
            event_types: vec![],
            session_id: session_id.to_string(),
            ..Default::default()
        })
        .await
        .map(|response| response.into_inner())
        .ok()
}

async fn sleep_or_cancel(cancel: &mut oneshot::Receiver<()>, duration: Duration) -> bool {
    tokio::select! {
        _ = cancel => true,
        _ = tokio::time::sleep(duration) => false,
    }
}

/// The session's active run per `get_state`, or None when idle/unreachable.
async fn probe_active_run(
    client: &mut crate::agent_proto::FutureAgentClient<tonic::transport::Channel>,
    session_id: &str,
) -> Option<String> {
    let response = client
        .execute_command(get_state_command(session_id.to_string()))
        .await
        .ok()?
        .into_inner();
    if !response.success {
        return None;
    }
    let state: serde_json::Value = serde_json::from_str(&response.data).ok()?;
    state
        .get("activeRun")?
        .get("runId")?
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// Resume cursor for an atomic attach: the last persisted sequence of the
/// run's local events, or -1 (full ring replay) when nothing is stored yet.
async fn attach_cursor(session_id: &str, canonical_run_id: &str) -> i64 {
    let local = {
        let guard = OBSERVERS.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .get(session_id)
            .and_then(|handle| {
                handle
                    .shared
                    .run_bindings
                    .lock()
                    .ok()?
                    .get(canonical_run_id)
                    .cloned()
            })
            .or_else(|| {
                crate::store::get_run(canonical_run_id)
                    .ok()
                    .flatten()
                    .map(|run| run.id)
            })
    };
    let Some(local_run_id) = local else {
        return -1;
    };
    tokio::task::spawn_blocking(move || {
        crate::store::list_run_events(&local_run_id)
            .ok()
            .and_then(|events| events.into_iter().map(|event| event.sequence).max())
            .unwrap_or(-1)
    })
    .await
    .unwrap_or(-1)
}

/// Process one event. Returns false when the run's idx sequence broke — the
/// caller re-attaches, and the replay/projection snapshot heals the gap.
///
/// Ordering rule: an event is validated against its run cursor BEFORE any
/// fan-out (persistence, webview forward, NATS mirror). The observer is the
/// sole NATS publisher, so nothing unvalidated may be mirrored — otherwise a
/// gap-triggered re-attach would re-publish replayed events and remote
/// clients would see the run out of order and duplicated.
async fn handle_event(
    session_id: &str,
    shared: &Arc<ObserverShared>,
    state: &mut ObserverState,
    event: crate::agent_proto::StreamEvent,
) -> bool {
    let event_type = event.r#type.as_str();
    let run_id = event.run_id.as_str();

    // Session-level event (no run scope): validate its independent session
    // cursor before forwarding. A hole forces re-attach and journal replay.
    if run_id.is_empty() {
        if event.session_idx >= 0 {
            if event.session_idx <= state.session_cursor {
                return true;
            }
            if event.session_idx != state.session_cursor.saturating_add(1) {
                return false;
            }
            state.session_cursor = event.session_idx;
        }
        if FORWARDED_EVENTS.contains(&event_type) {
            forward_settings_event(session_id, event_type, &event.data);
        }
        crate::remote::publish_event(
            session_id,
            event_type,
            &event.data,
            run_id,
            event.idx,
            event.epoch,
            &event.event_id,
            &event.timestamp,
            event.session_idx,
            event.run_sequence,
        );
        return true;
    }

    // Projection snapshots replace the run's local replica wholesale. The
    // snapshot IS the healing mechanism, so it bypasses the continuity check:
    // its cursor is the new position, locally and on the mirror.
    if event.projection_snapshot {
        state
            .cursors
            .insert(run_id.to_string(), event.snapshot_cursor);
        note_run_active(shared, state, run_id);
        if let Some((thread_id, local_run_id)) = observer_run(session_id, shared, run_id).await {
            let terminal_agent_end = event
                .snapshot_events
                .iter()
                .find(|projected| projected.r#type == "agent_end")
                .map(|projected| projected.data.clone());
            let events: Vec<(String, String, i64)> = event
                .snapshot_events
                .iter()
                .map(|projected| {
                    (
                        projected.r#type.clone(),
                        projected.data.clone(),
                        projected.idx,
                    )
                })
                .collect();
            stream::replace_projection_off_thread(&thread_id, Some(&local_run_id), events).await;
            if let Some(agent_end_data) = terminal_agent_end {
                if stream::agent_end_incomplete(&agent_end_data) {
                    mark_run_failed_if_active(
                        Some(&local_run_id),
                        "Future Agent response ended before a clean terminal.",
                    );
                } else {
                    mark_run_completed_if_active(Some(&local_run_id));
                }
                crate::store::clear_run_event_buffer(&local_run_id);
                note_run_settled(shared, state, run_id);
            }
        }
        // Mirror the snapshot AFTER the local replace has landed — a remote
        // client healing from this signal reads back the store we just wrote.
        // Folded events cannot be applied incrementally, so this goes out as a
        // wholesale-replacement signal, not as individual events.
        crate::remote::publish_snapshot(
            session_id,
            run_id,
            event.snapshot_cursor,
            &event.snapshot_events,
            event.run_sequence,
        );
        return true;
    }

    // Run-scoped event: validate ordering first. Replays (idx <= cursor) and
    // gaps (idx skips ahead) are settled here — neither may reach the stores,
    // the webview, or the mirror.
    match state.cursors.get(run_id).copied() {
        Some(last) => {
            if event.idx <= last {
                return true; // replay overlap — already persisted and mirrored
            }
            if event.idx != last.saturating_add(1) {
                return false; // mid-run hole — re-attach for replay/snapshot healing
            }
        }
        None => {
            if event.idx != 0 {
                if state.last_settled_run.as_deref() == Some(run_id) {
                    // The broadcaster keeps the settled run's identity until
                    // the next `start_run`, so between-runs settings changes
                    // (model, thinking level, …) arrive stamped with it at the
                    // idx the run ended on. They are session fan-out, not run
                    // content: forward and mirror without cursor bookkeeping —
                    // treating them as gaps would force a pointless re-attach
                    // on every settings change.
                    if FORWARDED_EVENTS.contains(&event_type) {
                        forward_settings_event(session_id, event_type, &event.data);
                    }
                    crate::remote::publish_event(
                        session_id,
                        event_type,
                        &event.data,
                        run_id,
                        event.idx,
                        event.epoch,
                        &event.event_id,
                        &event.timestamp,
                        event.session_idx,
                        event.run_sequence,
                    );
                    return true;
                }
                return false; // missed head on first sight — re-attach for replay
            }
        }
    }
    state.cursors.insert(run_id.to_string(), event.idx);
    note_run_active(shared, state, run_id);

    // Terminal bookkeeping applies regardless of ownership — otherwise a
    // pipeline-owned run's end would wedge has_active_run forever.
    let is_terminal = matches!(event_type, "agent_end" | "error");

    // Pipeline-owned runs are persisted by their collector; the observer only
    // mirrors/forwards those. Everything else is ours to project — persisted
    // BEFORE fan-out, so the mirror never announces what the store lacks.
    if let Some((thread_id, local_run_id)) = observer_run(session_id, shared, run_id).await {
        stream::persist_run_event_off_thread(
            &thread_id,
            Some(&local_run_id),
            event_type.to_string(),
            event.data.clone(),
            event.idx,
        )
        .await;

        match event_type {
            "agent_end" => {
                if stream::agent_end_incomplete(&event.data) {
                    mark_run_failed_if_active(
                        Some(&local_run_id),
                        "Future Agent response ended before a clean terminal.",
                    );
                } else {
                    mark_run_completed_if_active(Some(&local_run_id));
                }
                crate::store::clear_run_event_buffer(&local_run_id);
                note_run_settled(shared, state, run_id);
            }
            "error" => {
                mark_run_failed_if_active(Some(&local_run_id), "Future Agent reported an error.");
                note_run_settled(shared, state, run_id);
            }
            _ => {}
        }
    } else if is_terminal {
        note_run_settled(shared, state, run_id);
    }

    if FORWARDED_EVENTS.contains(&event_type) {
        forward_settings_event(session_id, event_type, &event.data);
    }
    crate::remote::publish_event(
        session_id,
        event_type,
        &event.data,
        run_id,
        event.idx,
        event.epoch,
        &event.event_id,
        &event.timestamp,
        event.session_idx,
        event.run_sequence,
    );
    true
}

async fn replay_session_events(
    client: &mut crate::agent_proto::FutureAgentClient<tonic::transport::Channel>,
    session_id: &str,
    shared: &Arc<ObserverShared>,
    state: &mut ObserverState,
) -> bool {
    let command = crate::agent_proto::RpcCommand {
        since_idx: state.session_cursor,
        ..base_command("get_session_events_since", session_id.to_string())
    };
    let Ok(response) = client.execute_command(command).await else {
        return false;
    };
    let response = response.into_inner();
    if !response.success {
        return false;
    }
    let events = serde_json::from_str::<serde_json::Value>(&response.data)
        .ok()
        .and_then(|value| value.get("events")?.as_array().cloned())
        .unwrap_or_default();
    for value in events {
        let event = crate::agent_proto::StreamEvent {
            r#type: value
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            data: value
                .get("data")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            session_id: session_id.to_string(),
            session_idx: value
                .get("sessionIdx")
                .and_then(|value| value.as_i64())
                .unwrap_or(-1),
            event_id: value
                .get("eventId")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            timestamp: value
                .get("timestamp")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            idx: -1,
            run_sequence: -1,
            ..Default::default()
        };
        if !handle_event(session_id, shared, state, event).await {
            return false;
        }
    }
    true
}

fn note_run_active(shared: &Arc<ObserverShared>, state: &mut ObserverState, run_id: &str) {
    state.active_run = Some(run_id.to_string());
    shared.has_active_run.store(true, Ordering::Relaxed);
}

fn note_run_settled(shared: &Arc<ObserverShared>, state: &mut ObserverState, run_id: &str) {
    if state.active_run.as_deref() == Some(run_id) {
        state.active_run = None;
        shared.has_active_run.store(false, Ordering::Relaxed);
    }
    state.cursors.remove(run_id);
    state.last_settled_run = Some(run_id.to_string());
    if let Ok(mut bindings) = shared.run_bindings.lock() {
        bindings.remove(run_id);
    }
}

/// The thread this session belongs to, resolved once and cached. The mapping
/// can appear after the observer starts (session import creates thread stubs
/// asynchronously), so a miss is re-resolved on the next event.
async fn resolve_thread_id(session_id: &str, shared: &Arc<ObserverShared>) -> Option<String> {
    if let Some(thread_id) = shared.thread_id.lock().ok()?.clone() {
        return Some(thread_id);
    }
    let session = session_id.to_string();
    let found = tokio::task::spawn_blocking(move || {
        crate::store::find_thread_by_agent_session(&session)
            .ok()
            .flatten()
            .map(|thread| thread.id)
    })
    .await
    .ok()
    .flatten();
    if let Some(thread_id) = &found {
        if let Ok(mut cache) = shared.thread_id.lock() {
            *cache = Some(thread_id.clone());
        }
    }
    found
}

/// The thread + local run row this observer projects `run_id` into, or None
/// when a prompt-pipeline collector owns the run — now or within the lease
/// grace window (single-writer rule; the collector persists its terminal
/// event before releasing, so a grace check is what prevents a duplicate
/// `agent_end` in the log).
async fn observer_run(
    session_id: &str,
    shared: &Arc<ObserverShared>,
    canonical_run_id: &str,
) -> Option<(String, String)> {
    if super::replica::AGENT_REPLICAS.is_owned_or_recently_released(canonical_run_id) {
        return None;
    }
    let thread_id = resolve_thread_id(session_id, shared).await?;
    let local_run_id = ensure_run_binding(session_id, canonical_run_id, &thread_id)?;
    Some((thread_id, local_run_id))
}

/// Forward a whitelisted event to the webview as `agent-event` (same envelope
/// the retired single-slot observer used: sessionId + _eventType injected).
fn forward_settings_event(session_id: &str, event_type: &str, data: &str) {
    let Some(app_handle) = crate::APP_HANDLE.get() else {
        return;
    };
    if let Ok(mut payload) = serde_json::from_str::<serde_json::Value>(data) {
        if let serde_json::Value::Object(ref mut map) = payload {
            map.insert(
                "sessionId".to_string(),
                serde_json::Value::String(session_id.to_string()),
            );
            map.insert(
                "_eventType".to_string(),
                serde_json::Value::String(event_type.to_string()),
            );
        }
        let _ = app_handle.emit("agent-event", &payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle_with(last_activity_ms: i64, has_active_run: bool) -> ObserverHandle {
        let (cancel, _rx) = oneshot::channel();
        ObserverHandle {
            cancel,
            shared: Arc::new(ObserverShared {
                last_activity_ms: AtomicI64::new(last_activity_ms),
                has_active_run: AtomicBool::new(has_active_run),
                ..ObserverShared::new()
            }),
        }
    }

    #[test]
    fn eviction_picks_oldest_idle_and_spares_active_runs() {
        let mut map = HashMap::new();
        for i in 0..OBSERVER_MAX {
            map.insert(format!("sess-{i}"), handle_with(i as i64, false));
        }
        // The oldest observer tracks an active run — it must survive even
        // though it is the least-recently-active entry.
        map.insert("sess-active".to_string(), handle_with(-1_000_000, true));

        evict_idle_if_over_cap(&mut map);

        assert!(
            map.contains_key("sess-active"),
            "an observer tracking an active run is never evicted"
        );
        assert!(!map.contains_key("sess-0"), "oldest idle evicted first");
        assert!(
            !map.contains_key("sess-1"),
            "second-oldest idle evicted next"
        );
        assert_eq!(map.len(), OBSERVER_MAX - 1, "evicted down below the cap");
    }

    #[test]
    fn eviction_overflows_when_every_observer_is_active() {
        let mut map = HashMap::new();
        for i in 0..=OBSERVER_MAX {
            map.insert(format!("sess-{i}"), handle_with(i as i64, true));
        }
        evict_idle_if_over_cap(&mut map);
        assert_eq!(
            map.len(),
            OBSERVER_MAX + 1,
            "no idle victim anywhere — the cap yields to overflow"
        );
    }

    #[test]
    fn sleep_requires_quiet_and_no_active_run() {
        let fresh = ObserverShared::new();
        assert!(
            !fresh.should_sleep(),
            "just-registered observers stay awake"
        );

        let quiet_active = ObserverShared {
            last_activity_ms: AtomicI64::new(0),
            has_active_run: AtomicBool::new(true),
            ..ObserverShared::new()
        };
        assert!(
            !quiet_active.should_sleep(),
            "an active run keeps the observer awake indefinitely"
        );

        let quiet_idle = ObserverShared {
            last_activity_ms: AtomicI64::new(0),
            has_active_run: AtomicBool::new(false),
            ..ObserverShared::new()
        };
        assert!(
            quiet_idle.should_sleep(),
            "long-quiet + no active run → sleep"
        );
    }

    // ── Ordering: validation before fan-out ─────────────────────────────

    fn stream_event(event_type: &str, run_id: &str, idx: i64) -> crate::agent_proto::StreamEvent {
        crate::agent_proto::StreamEvent {
            r#type: event_type.to_string(),
            data: "{}".to_string(),
            run_id: run_id.to_string(),
            idx,
            projection_snapshot: false,
            snapshot_events: vec![],
            snapshot_cursor: 0,
            session_id: "sess-order".to_string(),
            epoch: 1,
            event_id: String::new(),
            timestamp: String::new(),
            session_idx: -1,
            run_sequence: 1,
        }
    }

    fn session_event(event_type: &str, session_idx: i64) -> crate::agent_proto::StreamEvent {
        let mut event = stream_event(event_type, "", -1);
        event.session_idx = session_idx;
        event.run_sequence = -1;
        event
    }

    /// Lease the run so `observer_run` short-circuits as pipeline-owned and
    /// `handle_event` never touches the store. Fan-out ends at no-op sinks
    /// (no APP_HANDLE, no remote pairing) — what these tests assert is the
    /// cursor bookkeeping that gates it.
    fn lease_run(run_id: &str) -> super::super::replica::ReplicaLease {
        super::super::replica::AGENT_REPLICAS
            .acquire(run_id)
            .expect("lease")
    }

    #[tokio::test]
    async fn gap_event_is_rejected_without_advancing_the_cursor() {
        let _lease = lease_run("run-gap");
        let shared = Arc::new(ObserverShared::new());
        let mut state = ObserverState::default();

        assert!(
            handle_event(
                "sess-order",
                &shared,
                &mut state,
                stream_event("text_chunk", "run-gap", 0)
            )
            .await,
            "in-order first event accepted"
        );
        assert!(
            handle_event(
                "sess-order",
                &shared,
                &mut state,
                stream_event("text_chunk", "run-gap", 0)
            )
            .await,
            "replay overlap is a no-op, not a gap"
        );
        assert!(
            !handle_event(
                "sess-order",
                &shared,
                &mut state,
                stream_event("text_chunk", "run-gap", 2)
            )
            .await,
            "idx skipping ahead breaks the stream for re-attach"
        );
        assert_eq!(
            state.cursors.get("run-gap"),
            Some(&0),
            "a rejected gap event must not advance the cursor — the re-attach replay covers it"
        );
        assert!(
            handle_event(
                "sess-order",
                &shared,
                &mut state,
                stream_event("text_chunk", "run-gap", 1)
            )
            .await,
            "the replayed continuation validates against the untouched cursor"
        );
        assert_eq!(state.cursors.get("run-gap"), Some(&1));
    }

    #[tokio::test]
    async fn session_gap_uses_the_independent_session_cursor() {
        let shared = Arc::new(ObserverShared::new());
        let mut state = ObserverState::default();

        assert!(
            handle_event(
                "sess-order",
                &shared,
                &mut state,
                session_event("model_changed", 0)
            )
            .await
        );
        assert!(
            !handle_event(
                "sess-order",
                &shared,
                &mut state,
                session_event("model_changed", 2)
            )
            .await,
            "a missing session-scoped event must force replay"
        );
        assert_eq!(state.session_cursor, 0);
        assert!(
            handle_event(
                "sess-order",
                &shared,
                &mut state,
                session_event("model_changed", 1)
            )
            .await
        );
        assert_eq!(state.session_cursor, 1);
    }

    #[tokio::test]
    async fn missed_head_on_first_sight_forces_reattach() {
        let shared = Arc::new(ObserverShared::new());
        let mut state = ObserverState::default();

        assert!(
            !handle_event(
                "sess-order",
                &shared,
                &mut state,
                stream_event("text_chunk", "run-late", 3)
            )
            .await,
            "a run first seen above idx 0 means the head was missed — re-attach"
        );
        assert!(
            !state.cursors.contains_key("run-late"),
            "no cursor bookkeeping before the replay heals the head"
        );
    }

    #[tokio::test]
    async fn settled_run_stragglers_fan_out_without_bookkeeping() {
        let _lease = lease_run("run-fresh");
        let shared = Arc::new(ObserverShared::new());
        let mut state = ObserverState {
            last_settled_run: Some("run-settled".to_string()),
            ..ObserverState::default()
        };
        // Between-runs settings events keep the settled run's stamped identity;
        // they must forward/mirror without cursor bookkeeping or a re-attach.
        assert!(
            handle_event(
                "sess-order",
                &shared,
                &mut state,
                stream_event("model_changed", "run-settled", 47)
            )
            .await
        );
        assert!(
            !state.cursors.contains_key("run-settled"),
            "a straggler must not open cursor bookkeeping for the settled run"
        );
        // A genuinely new run still starts at idx 0.
        assert!(
            handle_event(
                "sess-order",
                &shared,
                &mut state,
                stream_event("agent_start", "run-fresh", 0)
            )
            .await
        );
        assert_eq!(state.cursors.get("run-fresh"), Some(&0));
    }
}
