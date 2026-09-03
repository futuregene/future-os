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
//!    agent, and the collector's cursor-ordering absorbs any residual replay
//!    overlap.
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
//! it again via [`ensure_observer_for_thread`].

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

/// Initial attach backoff. Tests shrink it so the self-heal retry loop runs
/// without real waits (the doubling stays, capped at 10s); the env override
/// lets individual tests pick a long window to cancel mid-backoff.
fn observer_backoff() -> Duration {
    #[cfg(test)]
    if let Some(ms) = std::env::var("FUTURE_TEST_OBSERVER_BACKOFF_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        return Duration::from_millis(ms);
    }
    #[cfg(test)]
    const INITIAL: Duration = Duration::from_millis(5);
    #[cfg(not(test))]
    const INITIAL: Duration = Duration::from_millis(500);
    INITIAL
}

/// How often a quiet stream is checked for the idle-sleep condition. Tests
/// shrink it via env so the quiet-window branch can be exercised without a
/// 30-second wait.
fn idle_check_interval() -> Duration {
    #[cfg(test)]
    if let Some(ms) = std::env::var("FUTURE_TEST_IDLE_CHECK_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        return Duration::from_millis(ms);
    }
    IDLE_CHECK_INTERVAL
}

/// Event types forwarded to the webview as `agent-event`. Same whitelist as
/// the retired single-slot observer: settings changes applied by
/// `agentStateCache`, `user_message` (zero-latency user bubble in the open
/// thread), and compaction lifecycle signals (including standalone/manual
/// compaction). Per-token content (`text_chunk`, `thinking_delta`, `tool_*`)
/// is never forwarded — the frontend renders content from the persisted
/// run-event log.
const FORWARDED_EVENTS: &[&str] = &[
    "agent_start",
    "agent_end",
    "compaction_started",
    "compaction_committed",
    "compaction_failed",
    "user_message",
    "model_changed",
    "thinking_level_changed",
    "permission_level_changed",
    "session_name_changed",
    "cwd_changed",
    "config_reloaded",
];

pub(super) struct ObserverHandle {
    cancel: oneshot::Sender<()>,
    pub(super) shared: Arc<ObserverShared>,
}

/// State shared between the manager and one observer task: eviction inputs
/// (activity, active-run flag), the immutable session→thread owner, and the
/// canonical→local run bindings the task builds as it projects runs.
pub(super) struct ObserverShared {
    pub(super) last_activity_ms: AtomicI64,
    pub(super) has_active_run: AtomicBool,
    pub(super) thread_id: String,
    run_bindings: Mutex<HashMap<String, String>>,
}

impl ObserverShared {
    fn new(thread_id: impl Into<String>) -> Self {
        Self {
            last_activity_ms: AtomicI64::new(now_millis()),
            has_active_run: AtomicBool::new(false),
            thread_id: thread_id.into(),
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
pub(super) static OBSERVERS: LazyLock<Mutex<HashMap<String, ObserverHandle>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Register (or refresh) the sole observer for an Agent session, bound to one
/// immutable GUI-thread owner for its lifetime. A legacy database may contain
/// more than one thread referencing the same Agent session; that data remains
/// readable, but a second thread cannot silently take over the live observer.
pub fn ensure_observer_for_thread(session_id: &str, thread_id: &str) -> Result<(), String> {
    ensure_observer_inner(session_id, thread_id, true)
}

/// Ensure a passive observer without treating periodic discovery as user
/// activity. Otherwise the 60-second import loop would keep every idle entry
/// permanently hot and defeat the 128-observer LRU cap.
fn ensure_passive_observer(session_id: &str, thread_id: &str) -> Result<(), String> {
    ensure_observer_inner(session_id, thread_id, false)
}

fn ensure_observer_inner(
    session_id: &str,
    thread_id: &str,
    touch_existing: bool,
) -> Result<(), String> {
    let session_id = session_id.trim();
    let thread_id = thread_id.trim();
    if session_id.is_empty() || thread_id.is_empty() {
        return Err("Observer requires both thread_id and session_id".to_string());
    }
    let thread = crate::store::get_thread(thread_id)
        .map_err(|error| format!("get observer owner: {error}"))?
        .ok_or_else(|| format!("Observer owner thread {thread_id} does not exist"))?;
    let mapped_session = thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    if mapped_session != Some(session_id) {
        return Err(format!(
            "observer_binding_mismatch: thread {thread_id} is not bound to session {session_id}"
        ));
    }
    let mut guard = OBSERVERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(handle) = guard.get(session_id) {
        verify_observer_owner(session_id, handle, thread_id)?;
        if touch_existing {
            handle.shared.touch();
        }
        return Ok(());
    }
    evict_idle_if_over_cap(&mut guard);
    let shared = Arc::new(ObserverShared::new(thread_id));
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
    Ok(())
}

fn verify_observer_owner(
    session_id: &str,
    handle: &ObserverHandle,
    requested_thread_id: &str,
) -> Result<(), String> {
    if handle.shared.thread_id == requested_thread_id {
        return Ok(());
    }
    Err(format!(
        "observer_owner_conflict: session {session_id} is already observed by thread {}",
        handle.shared.thread_id
    ))
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
            if let Err(error) = ensure_passive_observer(session_id, &thread.id) {
                eprintln!("FutureOS could not seed observer for session {session_id}: {error}");
            }
        }
    }
}

/// Background discovery of conversations created outside the GUI (TUI, CLI,
/// channels, another machine). The `session_created` push observer
/// (`session_events.rs`) imports most sessions within milliseconds; this poll
/// remains the backstop for missed events and agents too old to emit the
/// event. Two cadences: a 1s pass over the agent's streaming sessions — a run
/// started by another client appears in the sidebar within ~1s — and a 60s
/// full import for idle sessions plus observer import. Idle observers are not
/// re-touched here: opening a thread, a prompt, or a newly streaming session
/// wakes it instead.
#[cfg(test)]
static TEST_DISCOVERY_STOP: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn spawn_session_discovery() {
    tauri::async_runtime::spawn(async move {
        let mut ticks = 0u64;
        loop {
            tokio::time::sleep(discovery_interval()).await;
            #[cfg(test)]
            if TEST_DISCOVERY_STOP.swap(false, std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            ticks += 1;
            discovery_tick(ticks).await;
        }
    });
}

/// Discovery tick interval; tests shrink it via env (a cfg(test)-only seam).
fn discovery_interval() -> std::time::Duration {
    #[cfg(test)]
    if let Some(ms) = std::env::var("FUTURE_TEST_DISCOVERY_INTERVAL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        return std::time::Duration::from_millis(ms);
    }
    std::time::Duration::from_secs(1)
}

/// One discovery tick: the fast streaming-session pass, plus the full
/// idle-session import every 60 ticks.
async fn discovery_tick(ticks: u64) {
    discover_streaming_sessions().await;
    if ticks.is_multiple_of(60) {
        super::import::import_missing_sessions().await;
    }
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
    let session_ids: Vec<String> = future_rpc::decode::response_data(&response)
        .get("sessionIds")
        .and_then(|value| value.as_array().cloned())
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
            match super::import::import_discovered_session(&session_id).await {
                Ok(true) => eprintln!(
                    "FutureOS discovered streaming session {session_id} (created by another client)"
                ),
                Ok(false) => {}
                Err(error) => {
                    eprintln!("FutureOS could not import discovered session {session_id}: {error}")
                }
            }
        }
        if let Ok(Some(thread)) = crate::store::find_thread_by_agent_session(&session_id) {
            if let Err(error) = ensure_observer_for_thread(&session_id, &thread.id) {
                eprintln!("FutureOS could not observe discovered session {session_id}: {error}");
            }
        }
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

/// Cancel and deregister every live observer. Test-only: a finished test's
/// spawned observers outlive it (they run on the ambient runtime, not the test
/// runtime) and keep re-attaching — their `get_state`/`stream_events` calls
/// would otherwise consume the next test's scripted replies. The mock guard
/// calls this on acquisition so each test starts from a clean observer slate.
#[cfg(test)]
pub(crate) fn cancel_all_observers() {
    let handles: Vec<ObserverHandle> = OBSERVERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain()
        .map(|(_, handle)| handle)
        .collect();
    for handle in handles {
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
        if run.thread_id != thread_id {
            eprintln!(
                "FutureOS observer refused cross-thread run binding: session={session_id} thread={thread_id} run={canonical_run_id} belongs_to={}",
                run.thread_id
            );
            return None;
        }
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
    let mut backoff = observer_backoff();

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
            // would only duplicate the NATS mirror. A fresh attach (startup,
            // first sight) has no durable GUI-side cursor — the Agent journal
            // is the source of truth — so it replays from the start (-1);
            // replayed events are deduped by the cursor check before fan-out.
            let cursor = match state.cursors.get(&run_id) {
                Some(&cursor) => cursor,
                None => -1,
            };
            state.cursors.insert(run_id.clone(), cursor);
            match client
                .stream_events(StreamRequest {
                    event_types: vec![],
                    session_id: session_id.to_string(),
                    run_id: run_id.clone(),
                    after_idx: cursor,
                    atomic_attach: true,
                    global_events: false,
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
        backoff = observer_backoff();

        // ── Streaming ──────────────────────────────────────────────
        loop {
            let message = tokio::select! {
                _ = &mut cancel => return,
                result = tokio::time::timeout(idle_check_interval(), stream.message()) => result,
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
    let state: serde_json::Value = future_rpc::decode::response_data(&response);
    state
        .get("activeRun")?
        .get("runId")?
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_string)
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
        let event_data = future_rpc::decode::event_data_json(&event);
        if FORWARDED_EVENTS.contains(&event_type) {
            forward_settings_event(
                crate::APP_HANDLE.get(),
                session_id,
                &shared.thread_id,
                event_type,
                &event_data,
            );
        }
        crate::remote::publish_event(
            session_id,
            event_type,
            &event_data,
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
                .map(future_rpc::decode::projected_event_data_json);
            let events: Vec<(String, String, i64)> = event
                .snapshot_events
                .iter()
                .map(|projected| {
                    (
                        projected.r#type.clone(),
                        future_rpc::decode::projected_event_data_json(projected),
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
                    let event_data = future_rpc::decode::event_data_json(&event);
                    if FORWARDED_EVENTS.contains(&event_type) {
                        forward_settings_event(
                            crate::APP_HANDLE.get(),
                            session_id,
                            &shared.thread_id,
                            event_type,
                            &event_data,
                        );
                    }
                    crate::remote::publish_event(
                        session_id,
                        event_type,
                        &event_data,
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

    // Canonical event payload (byte-stable while the agent dual-writes `data`;
    // the typed reconstruction takes over once `data` is retired). Persistence,
    // terminal detection, forwarding and the NATS mirror all read it.
    let event_data = future_rpc::decode::event_data_json(&event);

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
            event_data.clone(),
            event.idx,
        )
        .await;

        match event_type {
            "agent_end" => {
                if stream::agent_end_incomplete(&event_data) {
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
        forward_settings_event(
            crate::APP_HANDLE.get(),
            session_id,
            &shared.thread_id,
            event_type,
            &event_data,
        );
    }
    crate::remote::publish_event(
        session_id,
        event_type,
        &event_data,
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
    let events = future_rpc::decode::response_data(&response)
        .get("events")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    for value in events {
        let event_type = value
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let event_data = value
            .get("data")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        // Reconstructed events carry the typed payload like live ones.
        let payload = future_rpc::encode::event_payload(&event_type, &event_data);
        let event = crate::agent_proto::StreamEvent {
            r#type: event_type,
            data: event_data,
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
            payload,
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
    let thread_id = shared.thread_id.clone();
    let local_run_id = ensure_run_binding(session_id, canonical_run_id, &thread_id)?;
    Some((thread_id, local_run_id))
}

/// Forward a whitelisted event to the webview as `agent-event`. Both owner
/// identities are injected so the frontend can reject stale listeners during
/// a thread switch instead of trusting session identity alone.
fn forward_settings_event<R: tauri::Runtime>(
    app_handle: Option<&tauri::AppHandle<R>>,
    session_id: &str,
    thread_id: &str,
    event_type: &str,
    data: &str,
) {
    if let Some(app_handle) = app_handle {
        emit_settings_event(app_handle, session_id, thread_id, event_type, data);
    }
}

/// Emit the enriched payload via an injectable `Emitter`. Generic over
/// `Runtime` so unit tests can drive the emit body with a mock app handle
/// (the process-global `APP_HANDLE` is `None` outside a running Tauri app).
fn emit_settings_event<R: tauri::Runtime>(
    app_handle: &tauri::AppHandle<R>,
    session_id: &str,
    thread_id: &str,
    event_type: &str,
    data: &str,
) {
    if let Some(payload) = settings_event_payload(session_id, thread_id, event_type, data) {
        let _ = app_handle.emit("agent-event", &payload);
    }
}

/// Build the enriched webview payload for a forwarded settings event, or None
/// when `data` is not a JSON object. Pure so it can be unit-tested without a
/// live Tauri AppHandle.
fn settings_event_payload(
    session_id: &str,
    thread_id: &str,
    event_type: &str,
    data: &str,
) -> Option<serde_json::Value> {
    let mut payload = serde_json::from_str::<serde_json::Value>(data).ok()?;
    if let serde_json::Value::Object(ref mut map) = payload {
        map.insert(
            "sessionId".to_string(),
            serde_json::Value::String(session_id.to_string()),
        );
        map.insert(
            "threadId".to_string(),
            serde_json::Value::String(thread_id.to_string()),
        );
        map.insert(
            "_eventType".to_string(),
            serde_json::Value::String(event_type.to_string()),
        );
    }
    Some(payload)
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
                ..ObserverShared::new("thread-owner")
            }),
        }
    }

    #[test]
    fn observer_owner_is_idempotent_but_cannot_be_rebound() {
        let handle = handle_with(0, false);
        assert!(verify_observer_owner("session-a", &handle, "thread-owner").is_ok());
        let error = verify_observer_owner("session-a", &handle, "thread-other").unwrap_err();
        assert!(error.starts_with("observer_owner_conflict:"));
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
        let fresh = ObserverShared::new("thread-fresh");
        assert!(
            !fresh.should_sleep(),
            "just-registered observers stay awake"
        );

        let quiet_active = ObserverShared {
            last_activity_ms: AtomicI64::new(0),
            has_active_run: AtomicBool::new(true),
            ..ObserverShared::new("thread-active")
        };
        assert!(
            !quiet_active.should_sleep(),
            "an active run keeps the observer awake indefinitely"
        );

        let quiet_idle = ObserverShared {
            last_activity_ms: AtomicI64::new(0),
            has_active_run: AtomicBool::new(false),
            ..ObserverShared::new("thread-idle")
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
            payload: None,
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
        let shared = Arc::new(ObserverShared::new("thread-order"));
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
        let shared = Arc::new(ObserverShared::new("thread-order"));
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
        let shared = Arc::new(ObserverShared::new("thread-order"));
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
        let shared = Arc::new(ObserverShared::new("thread-order"));
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

    // ── manager + task helpers ────────────────────────────────────────

    use super::super::test_support::{
        break_home, mock_agent, restore_home, seed_thread, seed_workspace, Reply, StreamScript,
        TestHome,
    };

    #[tokio::test]
    async fn ensure_observer_rejects_invalid_or_missing_owners() {
        let _home = TestHome::new("observer-ensure-err");
        let _mock = mock_agent();

        assert_eq!(
            ensure_observer_for_thread("", "thread-1").unwrap_err(),
            "Observer requires both thread_id and session_id"
        );
        assert_eq!(
            ensure_observer_for_thread("sess-1", "").unwrap_err(),
            "Observer requires both thread_id and session_id"
        );
        assert_eq!(
            ensure_observer_for_thread("sess-1", "no-such-thread").unwrap_err(),
            "Observer owner thread no-such-thread does not exist"
        );

        let prev = break_home();
        let err = ensure_observer_for_thread("sess-1", "thread-1").unwrap_err();
        restore_home(prev);
        assert!(err.starts_with("get observer owner:"), "{err}");
    }

    #[tokio::test]
    async fn ensure_observer_rejects_a_binding_mismatch() {
        let home = TestHome::new("observer-ensure-mismatch");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-other"));
        let err = ensure_observer_for_thread("sess-wrong", &thread.id).unwrap_err();
        assert!(err.starts_with("observer_binding_mismatch:"), "{err}");
    }

    #[tokio::test]
    async fn ensure_observer_touches_an_existing_observer() {
        let home = TestHome::new("observer-touch");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-touch"));

        let (cancel, _rx) = oneshot::channel();
        let shared = Arc::new(ObserverShared::new(thread.id.clone()));
        OBSERVERS
            .lock()
            .unwrap()
            .insert("sess-touch".to_string(), ObserverHandle { cancel, shared });

        ensure_observer_for_thread("sess-touch", &thread.id).expect("existing touch");
        ensure_passive_observer("sess-touch", &thread.id).expect("existing passive");

        OBSERVERS.lock().unwrap().remove("sess-touch");
    }

    #[test]
    fn seed_observers_handles_empty_and_broken_store() {
        let home = TestHome::new("observer-seed-empty");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        // A thread with no agent session → the `if let Some` else path.
        seed_thread(&workspace.id, None);
        seed_observers_from_store();
        // Broken store → silent return.
        let prev = break_home();
        seed_observers_from_store();
        restore_home(prev);
    }

    #[test]
    fn seed_observers_creates_and_logs_conflicts() {
        let home = TestHome::new("observer-seed-conflict");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let t1 = seed_thread(&workspace.id, Some("sess-shared"));
        let _t2 = seed_thread(&workspace.id, Some("sess-shared"));
        seed_observers_from_store();
        assert!(OBSERVERS.lock().unwrap().contains_key("sess-shared"));
        drop_observer("sess-shared");
        let _ = t1;
    }

    #[tokio::test]
    async fn drop_observer_cancels_a_registered_observer() {
        let (cancel, rx) = oneshot::channel();
        let shared = Arc::new(ObserverShared::new("thread-drop"));
        OBSERVERS
            .lock()
            .unwrap()
            .insert("sess-drop".to_string(), ObserverHandle { cancel, shared });
        drop_observer("sess-drop");
        assert!(rx.await.is_ok(), "cancel was sent");
        assert!(!OBSERVERS.lock().unwrap().contains_key("sess-drop"));
    }

    #[test]
    fn unregister_removes_only_the_matching_shared() {
        let (cancel1, _rx1) = oneshot::channel();
        let shared1 = Arc::new(ObserverShared::new("t1"));
        let shared1_clone = shared1.clone();
        let shared2 = Arc::new(ObserverShared::new("t2"));
        OBSERVERS.lock().unwrap().insert(
            "sess-u".to_string(),
            ObserverHandle {
                cancel: cancel1,
                shared: shared1,
            },
        );
        // A mismatched shared does NOT remove the entry.
        unregister("sess-u", &shared2);
        assert!(OBSERVERS.lock().unwrap().contains_key("sess-u"));
        // The matching shared removes it.
        unregister("sess-u", &shared1_clone);
        assert!(!OBSERVERS.lock().unwrap().contains_key("sess-u"));
    }

    #[tokio::test]
    async fn ensure_run_binding_cached_cross_thread_own_and_fallback() {
        let home = TestHome::new("observer-run-binding");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-rb"));

        // Cached binding returns without touching the store.
        let (cancel, _rx) = oneshot::channel();
        let shared = Arc::new(ObserverShared::new(thread.id.clone()));
        shared
            .run_bindings
            .lock()
            .unwrap()
            .insert("run-cached".to_string(), "local-cached".to_string());
        OBSERVERS
            .lock()
            .unwrap()
            .insert("sess-rb".to_string(), ObserverHandle { cancel, shared });
        assert_eq!(
            ensure_run_binding("sess-rb", "run-cached", &thread.id).as_deref(),
            Some("local-cached")
        );

        // A run owned by another thread → refused.
        let other = seed_thread(&workspace.id, Some("sess-other"));
        crate::store::create_run(crate::store::CreateRunInput {
            id: Some("run-foreign".to_string()),
            thread_id: other.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("foreign run");
        assert!(ensure_run_binding("sess-rb", "run-foreign", &thread.id).is_none());

        // A run on THIS thread → bound + returned.
        crate::store::create_run(crate::store::CreateRunInput {
            id: Some("run-own".to_string()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("own run");
        assert_eq!(
            ensure_run_binding("sess-rb", "run-own", &thread.id).as_deref(),
            Some("run-own")
        );

        // A fresh canonical id → created + bound.
        assert_eq!(
            ensure_run_binding("sess-rb", "run-fresh", &thread.id).as_deref(),
            Some("run-fresh")
        );

        // Broken store → create_run fails and the fallback read also fails.
        let prev = break_home();
        assert!(ensure_run_binding("sess-rb", "run-fresh-2", &thread.id).is_none());
        restore_home(prev);
        OBSERVERS.lock().unwrap().remove("sess-rb");
    }

    #[tokio::test]
    async fn bind_run_records_into_a_registered_observer_only() {
        // No observer registered → bind_run is a no-op.
        bind_run("sess-no-observer", "run-1", "local-1");

        let home = TestHome::new("observer-bind-run");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-bind"));
        let (cancel, _rx) = oneshot::channel();
        let shared = Arc::new(ObserverShared::new(thread.id.clone()));
        OBSERVERS
            .lock()
            .unwrap()
            .insert("sess-bind".to_string(), ObserverHandle { cancel, shared });
        bind_run("sess-bind", "run-1", "local-1");
        assert_eq!(
            OBSERVERS.lock().unwrap().get("sess-bind").and_then(|h| h
                .shared
                .run_bindings
                .lock()
                .ok()?
                .get("run-1")
                .cloned()),
            Some("local-1".to_string())
        );
        OBSERVERS.lock().unwrap().remove("sess-bind");
    }

    #[tokio::test]
    async fn probe_active_run_resolves_or_declines() {
        let mock = mock_agent();
        let mut client = super::super::client::connect_agent()
            .await
            .expect("connect");

        // No activeRun → None.
        mock.push_data("get_state", serde_json::json!({"isStreaming": false}));
        assert!(probe_active_run(&mut client, "sess").await.is_none());

        // activeRun with a runId → Some.
        mock.push_data(
            "get_state",
            serde_json::json!({"activeRun": {"runId": "run-1"}}),
        );
        assert_eq!(
            probe_active_run(&mut client, "sess").await.as_deref(),
            Some("run-1")
        );

        // activeRun with an empty runId → None.
        mock.push_data("get_state", serde_json::json!({"activeRun": {"runId": ""}}));
        assert!(probe_active_run(&mut client, "sess").await.is_none());

        // Reject → None.
        mock.push("get_state", Reply::Reject("nope".to_string()));
        assert!(probe_active_run(&mut client, "sess").await.is_none());

        // Transport error → None.
        mock.push("get_state", Reply::Status(tonic::Code::Internal, "down"));
        assert!(probe_active_run(&mut client, "sess").await.is_none());
    }

    #[tokio::test]
    async fn sleep_or_cancel_returns_true_only_on_cancel() {
        let (_tx, mut rx) = oneshot::channel::<()>();
        assert!(!sleep_or_cancel(&mut rx, Duration::from_millis(5)).await);

        let (tx, mut rx) = oneshot::channel::<()>();
        tx.send(()).unwrap();
        assert!(sleep_or_cancel(&mut rx, Duration::from_secs(10)).await);
    }

    #[tokio::test]
    async fn replay_session_events_reconstructs_and_detects_gaps() {
        let mock = mock_agent();
        let mut client = super::super::client::connect_agent()
            .await
            .expect("connect");
        let shared = Arc::new(ObserverShared::new("thread-rp"));
        let mut state = ObserverState::default();

        // Transport error → false.
        mock.push(
            "get_session_events_since",
            Reply::Status(tonic::Code::Internal, "down"),
        );
        assert!(!replay_session_events(&mut client, "sess-rp", &shared, &mut state).await);

        // Reject → false.
        mock.push(
            "get_session_events_since",
            Reply::Reject("nope".to_string()),
        );
        assert!(!replay_session_events(&mut client, "sess-rp", &shared, &mut state).await);

        // No events → true.
        mock.push_data(
            "get_session_events_since",
            serde_json::json!({"events": []}),
        );
        assert!(replay_session_events(&mut client, "sess-rp", &shared, &mut state).await);

        // One session-level event → true, cursor advanced.
        mock.push_data(
            "get_session_events_since",
            serde_json::json!({"events": [{"type": "model_changed", "data": "{}", "sessionIdx": 0}]}),
        );
        assert!(replay_session_events(&mut client, "sess-rp", &shared, &mut state).await);
        assert_eq!(state.session_cursor, 0);

        // A gap → handle_event false → replay false.
        mock.push_data(
            "get_session_events_since",
            serde_json::json!({"events": [{"type": "model_changed", "data": "{}", "sessionIdx": 2}]}),
        );
        assert!(!replay_session_events(&mut client, "sess-rp", &shared, &mut state).await);
    }

    #[test]
    fn forward_settings_event_emits_or_skips_without_a_handle() {
        // No handle → no emit (the `if let Some` body is skipped).
        forward_settings_event(
            None::<&tauri::AppHandle<tauri::test::MockRuntime>>,
            "sess",
            "thread",
            "model_changed",
            "{}",
        );
        // A mock handle → the emit body runs (valid + malformed payloads).
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        let handle = app.handle();
        forward_settings_event(
            Some(handle),
            "sess",
            "thread",
            "model_changed",
            r#"{"model":"k3"}"#,
        );
        forward_settings_event(Some(handle), "sess", "thread", "model_changed", "not json");
    }

    #[test]
    fn settings_event_payload_enriches_objects_and_rejects_malformed() {
        let payload =
            settings_event_payload("sess", "thread", "model_changed", r#"{"model":"k3"}"#)
                .expect("object payload");
        assert_eq!(payload["sessionId"], "sess");
        assert_eq!(payload["threadId"], "thread");
        assert_eq!(payload["_eventType"], "model_changed");
        assert_eq!(payload["model"], "k3");

        // Non-object JSON → the map-insert is skipped but Some is returned.
        let array = settings_event_payload("s", "t", "e", "[1,2]").expect("array payload");
        assert!(array.is_array());

        // Malformed JSON → None.
        assert!(settings_event_payload("s", "t", "e", "not json").is_none());
    }

    #[tokio::test]
    async fn discover_streaming_sessions_paths() {
        let home = TestHome::new("observer-discover");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        seed_thread(&workspace.id, Some("sess-known"));

        // Empty sessionIds → no-op.
        mock.push_data(
            "list_streaming_sessions",
            serde_json::json!({"sessionIds": []}),
        );
        discover_streaming_sessions().await;

        // A known session → only ensure_observer.
        mock.push_data(
            "list_streaming_sessions",
            serde_json::json!({"sessionIds": ["sess-known"]}),
        );
        discover_streaming_sessions().await;
        assert!(OBSERVERS.lock().unwrap().contains_key("sess-known"));
        drop_observer("sess-known");

        // An unknown session → import + observe.
        mock.push_data(
            "list_streaming_sessions",
            serde_json::json!({"sessionIds": ["sess-new"]}),
        );
        mock.push_data(
            "get_state",
            serde_json::json!({"sessionId": "sess-new", "sessionName": "New", "cwd": "", "model": "future/k3"}),
        );
        discover_streaming_sessions().await;
        assert!(OBSERVERS.lock().unwrap().contains_key("sess-new"));
        drop_observer("sess-new");

        // Reject (success=false) → return.
        mock.push("list_streaming_sessions", Reply::Reject("nope".to_string()));
        discover_streaming_sessions().await;

        // Transport error → return.
        mock.push(
            "list_streaming_sessions",
            Reply::Status(tonic::Code::Internal, "down"),
        );
        discover_streaming_sessions().await;

        // Connect failure → return.
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        discover_streaming_sessions().await;
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
    }

    #[tokio::test]
    async fn spawn_observer_unregisters_on_cancel() {
        let home = TestHome::new("observer-cancel");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-cancel"));

        ensure_observer_for_thread("sess-cancel", &thread.id).expect("spawn");
        assert!(OBSERVERS.lock().unwrap().contains_key("sess-cancel"));

        // Let the observer reach the streaming loop, then cancel and wait for
        // run_observer to return and unregister.
        tokio::time::sleep(Duration::from_millis(120)).await;
        drop_observer("sess-cancel");
        for _ in 0..50 {
            if !OBSERVERS.lock().unwrap().contains_key("sess-cancel") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!OBSERVERS.lock().unwrap().contains_key("sess-cancel"));
    }

    /// A run-scoped event owned by the observer (not pipeline-owned) is
    /// persisted and settled end-to-end.
    #[tokio::test]
    async fn handle_event_projects_and_settles_an_owned_run() {
        let home = TestHome::new("observer-owned-run");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-owned"));
        let shared = Arc::new(ObserverShared::new(thread.id.clone()));
        let mut state = ObserverState::default();

        assert!(
            handle_event(
                "sess-owned",
                &shared,
                &mut state,
                stream_event("agent_start", "run-owned", 0)
            )
            .await
        );
        assert_eq!(state.active_run.as_deref(), Some("run-owned"));
        assert_eq!(
            crate::store::get_run("run-owned")
                .expect("get")
                .expect("some")
                .thread_id,
            thread.id
        );

        assert!(
            handle_event(
                "sess-owned",
                &shared,
                &mut state,
                stream_event("text_chunk", "run-owned", 1)
            )
            .await
        );

        // Clean agent_end → completed + settled.
        let mut end = stream_event("agent_end", "run-owned", 2);
        end.data = r#"{"reason":"complete"}"#.to_string();
        assert!(handle_event("sess-owned", &shared, &mut state, end).await);
        assert_eq!(
            crate::store::get_run("run-owned")
                .expect("get")
                .expect("some")
                .status,
            "completed"
        );
        assert!(state.active_run.is_none());
        assert_eq!(state.last_settled_run.as_deref(), Some("run-owned"));
    }

    /// Incomplete `agent_end` and `error` terminals both settle the owned run
    /// to failed.
    #[tokio::test]
    async fn handle_event_settles_failed_terminals() {
        let home = TestHome::new("observer-failed");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-failed"));
        let shared = Arc::new(ObserverShared::new(thread.id.clone()));
        let mut state = ObserverState::default();

        assert!(
            handle_event(
                "sess-failed",
                &shared,
                &mut state,
                stream_event("agent_start", "run-inc", 0)
            )
            .await
        );
        let mut end = stream_event("agent_end", "run-inc", 1);
        end.data = r#"{"reason":"incomplete"}"#.to_string();
        assert!(handle_event("sess-failed", &shared, &mut state, end).await);
        assert_eq!(
            crate::store::get_run("run-inc")
                .expect("get")
                .expect("some")
                .status,
            "failed"
        );

        // A fresh run that errors.
        assert!(
            handle_event(
                "sess-failed",
                &shared,
                &mut state,
                stream_event("agent_start", "run-err", 0)
            )
            .await
        );
        assert!(
            handle_event(
                "sess-failed",
                &shared,
                &mut state,
                stream_event("error", "run-err", 1)
            )
            .await
        );
        assert_eq!(
            crate::store::get_run("run-err")
                .expect("get")
                .expect("some")
                .status,
            "failed"
        );
    }

    /// Projection snapshots replace the local replica wholesale and settle a
    /// terminal snapshot.
    #[tokio::test]
    async fn handle_event_applies_a_projection_snapshot() {
        let home = TestHome::new("observer-snapshot");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-snap"));
        let shared = Arc::new(ObserverShared::new(thread.id.clone()));
        let mut state = ObserverState::default();

        let snapshot = crate::agent_proto::StreamEvent {
            r#type: "projection_snapshot".to_string(),
            data: "{}".to_string(),
            run_id: "run-snap".to_string(),
            idx: -1,
            projection_snapshot: true,
            snapshot_cursor: 5,
            snapshot_events: vec![crate::agent_proto::ProjectedRunEvent {
                r#type: "text_chunk".to_string(),
                data: "hello".to_string(),
                idx: 0,
                ..Default::default()
            }],
            session_id: "sess-snap".to_string(),
            epoch: 1,
            event_id: String::new(),
            timestamp: String::new(),
            session_idx: -1,
            run_sequence: 1,
            payload: None,
        };
        assert!(
            handle_event("sess-snap", &shared, &mut state, snapshot).await,
            "a projection snapshot is applied"
        );
        assert_eq!(state.cursors.get("run-snap"), Some(&5));
    }

    /// A projection snapshot for a pipeline-owned run (lease held) skips the
    /// local replace but still mirrors and records the cursor.
    #[tokio::test]
    async fn handle_event_snapshot_for_a_pipeline_owned_run() {
        let _lease = lease_run("run-snap-owned");
        let shared = Arc::new(ObserverShared::new("thread-order"));
        let mut state = ObserverState::default();
        let snapshot = crate::agent_proto::StreamEvent {
            r#type: "projection_snapshot".to_string(),
            data: "{}".to_string(),
            run_id: "run-snap-owned".to_string(),
            idx: -1,
            projection_snapshot: true,
            snapshot_cursor: 9,
            snapshot_events: vec![crate::agent_proto::ProjectedRunEvent {
                r#type: "text_chunk".to_string(),
                data: "hi".to_string(),
                idx: 0,
                ..Default::default()
            }],
            session_id: "sess-order".to_string(),
            epoch: 1,
            event_id: String::new(),
            timestamp: String::new(),
            session_idx: -1,
            run_sequence: 1,
            payload: None,
        };
        assert!(handle_event("sess-order", &shared, &mut state, snapshot).await);
        assert_eq!(state.cursors.get("run-snap-owned"), Some(&9));
    }

    /// Session-level replay overlap and a negative session index both keep
    /// the session cursor stable.
    #[tokio::test]
    async fn handle_event_session_replay_and_negative_idx() {
        let shared = Arc::new(ObserverShared::new("thread-order"));
        let mut state = ObserverState::default();
        // Negative session_idx → no cursor bookkeeping.
        assert!(
            handle_event(
                "sess-order",
                &shared,
                &mut state,
                session_event("model_changed", -1)
            )
            .await
        );
        assert!(
            handle_event(
                "sess-order",
                &shared,
                &mut state,
                session_event("model_changed", 0)
            )
            .await
        );
        assert_eq!(state.session_cursor, 0);
        // Replay overlap (idx <= cursor) → accepted, no change.
        assert!(
            handle_event(
                "sess-order",
                &shared,
                &mut state,
                session_event("model_changed", 0)
            )
            .await
        );
        assert_eq!(state.session_cursor, 0);
    }

    /// A projection snapshot whose folded events include a terminal `agent_end`
    /// settles the run (incomplete → failed, clean → completed).
    #[tokio::test]
    async fn handle_event_snapshot_with_terminal_agent_end() {
        let home = TestHome::new("observer-snap-terminal");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-snap-term"));
        let shared = Arc::new(ObserverShared::new(thread.id.clone()));
        let mut state = ObserverState::default();

        let incomplete = crate::agent_proto::StreamEvent {
            r#type: "projection_snapshot".to_string(),
            data: "{}".to_string(),
            run_id: "run-snap-inc".to_string(),
            idx: -1,
            projection_snapshot: true,
            snapshot_cursor: 3,
            snapshot_events: vec![crate::agent_proto::ProjectedRunEvent {
                r#type: "agent_end".to_string(),
                data: r#"{"reason":"incomplete"}"#.to_string(),
                idx: 3,
                ..Default::default()
            }],
            session_id: "sess-snap-term".to_string(),
            epoch: 1,
            event_id: String::new(),
            timestamp: String::new(),
            session_idx: -1,
            run_sequence: 1,
            payload: None,
        };
        assert!(handle_event("sess-snap-term", &shared, &mut state, incomplete).await);
        assert_eq!(
            crate::store::get_run("run-snap-inc")
                .expect("get")
                .expect("some")
                .status,
            "failed"
        );

        let complete = crate::agent_proto::StreamEvent {
            r#type: "projection_snapshot".to_string(),
            data: "{}".to_string(),
            run_id: "run-snap-ok".to_string(),
            idx: -1,
            projection_snapshot: true,
            snapshot_cursor: 4,
            snapshot_events: vec![crate::agent_proto::ProjectedRunEvent {
                r#type: "agent_end".to_string(),
                data: r#"{"reason":"complete"}"#.to_string(),
                idx: 4,
                ..Default::default()
            }],
            session_id: "sess-snap-term".to_string(),
            epoch: 1,
            event_id: String::new(),
            timestamp: String::new(),
            session_idx: -1,
            run_sequence: 1,
            payload: None,
        };
        assert!(handle_event("sess-snap-term", &shared, &mut state, complete).await);
        assert_eq!(
            crate::store::get_run("run-snap-ok")
                .expect("get")
                .expect("some")
                .status,
            "completed"
        );
    }

    /// A pipeline-owned terminal event settles without store bookkeeping.
    #[tokio::test]
    async fn handle_event_pipeline_owned_terminal_settles() {
        let _lease = lease_run("run-owned-term");
        let shared = Arc::new(ObserverShared::new("thread-order"));
        let mut state = ObserverState::default();
        assert!(
            handle_event(
                "sess-order",
                &shared,
                &mut state,
                stream_event("agent_end", "run-owned-term", 0)
            )
            .await
        );
        assert_eq!(state.last_settled_run.as_deref(), Some("run-owned-term"));
    }

    /// Discovery logs import and observe failures without surfacing them.
    #[tokio::test]
    async fn discover_streaming_sessions_logs_import_and_observe_errors() {
        let home = TestHome::new("observer-discover-err");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-conflict"));

        // Unknown session whose import fails (get_state reject).
        mock.push_data(
            "list_streaming_sessions",
            serde_json::json!({"sessionIds": ["sess-bad"]}),
        );
        mock.push("get_state", Reply::Reject("gone".to_string()));
        discover_streaming_sessions().await;

        // Known session already observed by a phantom thread → owner conflict.
        let (cancel, _rx) = oneshot::channel();
        let shared = Arc::new(ObserverShared::new("ghost-thread"));
        OBSERVERS.lock().unwrap().insert(
            "sess-conflict".to_string(),
            ObserverHandle { cancel, shared },
        );
        mock.push_data(
            "list_streaming_sessions",
            serde_json::json!({"sessionIds": ["sess-conflict"]}),
        );
        discover_streaming_sessions().await;
        OBSERVERS.lock().unwrap().remove("sess-conflict");
        let _ = thread;
    }

    /// The discovery loop ticks, then stops on the test seam; the default
    /// (unseamed) interval is 1s.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn discovery_loop_ticks_and_stops() {
        let _home = TestHome::new("observer-discovery-loop");
        let mock = mock_agent();
        std::env::set_var("FUTURE_TEST_DISCOVERY_INTERVAL_MS", "10");
        spawn_session_discovery();
        tokio::time::sleep(Duration::from_millis(60)).await;
        TEST_DISCOVERY_STOP.store(true, std::sync::atomic::Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(30)).await;
        std::env::remove_var("FUTURE_TEST_DISCOVERY_INTERVAL_MS");
        assert_eq!(discovery_interval(), Duration::from_secs(1));
        let _ = mock;
    }

    /// The 60th discovery tick runs the full import; other ticks do not.
    #[tokio::test]
    async fn discovery_tick_imports_on_the_60th_tick() {
        let _home = TestHome::new("observer-discovery-tick");
        let mock = mock_agent();
        mock.push_data(
            "list_streaming_sessions",
            serde_json::json!({"sessionIds": []}),
        );
        discovery_tick(1).await;
        mock.push_data(
            "list_streaming_sessions",
            serde_json::json!({"sessionIds": []}),
        );
        discovery_tick(60).await;
    }

    // ── run_observer self-heal paths ─────────────────────────────────

    /// Spawn `run_observer` directly on the test runtime so its retry/idle
    /// arms can be driven deterministically (unlike `spawn_observer`, which
    /// runs on Tauri's ambient runtime).
    fn spawn_run_observer(
        session_id: &str,
        shared: Arc<ObserverShared>,
    ) -> (oneshot::Sender<()>, tokio::task::JoinHandle<()>) {
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let sid = session_id.to_string();
        let handle = tokio::spawn(async move {
            run_observer(&sid, &shared, cancel_rx).await;
        });
        (cancel_tx, handle)
    }

    fn quiet_shared(thread_id: &str) -> Arc<ObserverShared> {
        Arc::new(ObserverShared {
            last_activity_ms: AtomicI64::new(0),
            has_active_run: AtomicBool::new(false),
            ..ObserverShared::new(thread_id)
        })
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn run_observer_retries_connect_failures() {
        // Serialize with the remote mock family too: breaking
        // `FUTURE_AGENT_GRPC_ADDR` must not race a bridge/commands test's
        // `connect_agent`. Acquire MOCK_SER before MOCK_LOCK so the lock
        // ordering stays acyclic (bridge: MOCK_SER→TEST_HOME_LOCK; TestHome
        // observer: TEST_HOME_LOCK→MOCK_LOCK; here: MOCK_SER→MOCK_LOCK).
        let _remote = crate::remote::test_support::mock_agent_lock();
        let _mock = mock_agent();
        std::env::set_var("FUTURE_TEST_OBSERVER_BACKOFF_MS", "40");
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");

        let shared = Arc::new(ObserverShared::new("thread-cf"));
        let (cancel_tx, handle) = spawn_run_observer("sess-cf", shared);
        // Cycle 1: connect fails → 40ms backoff completes → double + continue.
        // Cycle 2: connect fails → 80ms backoff; cancel lands mid-sleep → return.
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel_tx.send(()).unwrap();
        handle.await.unwrap();

        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        std::env::remove_var("FUTURE_TEST_OBSERVER_BACKOFF_MS");
    }

    #[tokio::test]
    async fn run_observer_retries_replay_failures() {
        let _home = TestHome::new("observer-replay-retry");
        let mock = mock_agent();
        std::env::set_var("FUTURE_TEST_OBSERVER_BACKOFF_MS", "50");
        mock.push(
            "get_session_events_since",
            Reply::Reject("nope".to_string()),
        );
        mock.push(
            "get_session_events_since",
            Reply::Reject("nope".to_string()),
        );

        let shared = Arc::new(ObserverShared::new("thread-rr"));
        let (cancel_tx, handle) = spawn_run_observer("sess-rr", shared);
        // Cycle 1: replay fails → 50ms backoff completes → continue (no doubling).
        // Cycle 2: replay fails → 50ms backoff; cancel mid-sleep → return.
        tokio::time::sleep(Duration::from_millis(75)).await;
        cancel_tx.send(()).unwrap();
        handle.await.unwrap();
        std::env::remove_var("FUTURE_TEST_OBSERVER_BACKOFF_MS");
    }

    #[tokio::test]
    async fn run_observer_attaches_atomically_to_an_active_run() {
        let _home = TestHome::new("observer-atomic");
        let mock = mock_agent();
        std::env::set_var("FUTURE_TEST_OBSERVER_BACKOFF_MS", "200");
        mock.push_data(
            "get_session_events_since",
            serde_json::json!({"events": []}),
        );
        mock.push_data(
            "get_state",
            serde_json::json!({"activeRun": {"runId": "run-aa"}}),
        );
        mock.push_stream(StreamScript::Events(
            vec![stream_event("agent_start", "run-aa", 0)],
            None,
        ));

        let shared = Arc::new(ObserverShared::new("thread-aa"));
        let (cancel_tx, handle) = spawn_run_observer("sess-aa", shared);
        // The atomic attach streams one event then closes; the post-stream
        // 200ms backoff sleep is where cancel lands → return (post-stream arm).
        tokio::time::sleep(Duration::from_millis(80)).await;
        cancel_tx.send(()).unwrap();
        handle.await.unwrap();
        std::env::remove_var("FUTURE_TEST_OBSERVER_BACKOFF_MS");
    }

    /// A re-attach to a run already seen resumes from the in-memory cursor
    /// (`Some` arm) instead of replaying from the start.
    #[tokio::test]
    async fn run_observer_reattaches_with_a_cached_cursor() {
        let _home = TestHome::new("observer-reattach");
        let mock = mock_agent();
        std::env::set_var("FUTURE_TEST_OBSERVER_BACKOFF_MS", "50");
        // Two full attach cycles (replay empty → probe active → atomic stream
        // yields one event then closes).
        for _ in 0..2 {
            mock.push_data(
                "get_session_events_since",
                serde_json::json!({"events": []}),
            );
            mock.push_data(
                "get_state",
                serde_json::json!({"activeRun": {"runId": "run-rc"}}),
            );
            mock.push_stream(StreamScript::Events(
                vec![stream_event("agent_start", "run-rc", 0)],
                None,
            ));
        }

        let shared = Arc::new(ObserverShared::new("thread-rc"));
        let (cancel_tx, handle) = spawn_run_observer("sess-rc", shared);
        // Cycle 1 attaches fresh (cursor None → -1) and re-attaches; cycle 2
        // attaches with a cached cursor (Some arm).
        tokio::time::sleep(Duration::from_millis(140)).await;
        cancel_tx.send(()).unwrap();
        handle.await.unwrap();
        std::env::remove_var("FUTURE_TEST_OBSERVER_BACKOFF_MS");
    }

    #[tokio::test]
    async fn run_observer_falls_back_to_plain_subscribe_after_atomic_error() {
        let _home = TestHome::new("observer-atomic-fallback");
        let mock = mock_agent();
        std::env::set_var("FUTURE_TEST_OBSERVER_BACKOFF_MS", "50");
        mock.push_data(
            "get_session_events_since",
            serde_json::json!({"events": []}),
        );
        mock.push_data(
            "get_state",
            serde_json::json!({"activeRun": {"runId": "run-fb"}}),
        );
        // Atomic attach fails with FailedPrecondition → plain subscribe; the
        // plain subscribe also fails → sleep_or_cancel → cancel mid-sleep.
        mock.push_stream(StreamScript::AttachError(
            tonic::Code::FailedPrecondition,
            "ended",
        ));
        mock.push_plain_stream(StreamScript::AttachError(tonic::Code::Internal, "down"));

        let shared = Arc::new(ObserverShared::new("thread-fb"));
        let (cancel_tx, handle) = spawn_run_observer("sess-fb", shared);
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancel_tx.send(()).unwrap();
        handle.await.unwrap();
        std::env::remove_var("FUTURE_TEST_OBSERVER_BACKOFF_MS");
    }

    /// When the atomic attach fails with `FailedPrecondition`/`NotFound` but the
    /// fallback plain subscribe works, the observer streams from it (`Some` arm).
    #[tokio::test]
    async fn run_observer_falls_back_to_a_working_plain_subscribe() {
        let _home = TestHome::new("observer-fallback-ok");
        let mock = mock_agent();
        mock.push_data(
            "get_session_events_since",
            serde_json::json!({"events": []}),
        );
        mock.push_data(
            "get_state",
            serde_json::json!({"activeRun": {"runId": "run-fbok"}}),
        );
        mock.push_stream(StreamScript::AttachError(tonic::Code::NotFound, "ended"));
        // Plain subscribe succeeds (yields one event then closes).
        mock.push_plain_stream(StreamScript::Events(
            vec![stream_event("model_changed", "", -1)],
            None,
        ));

        let shared = Arc::new(ObserverShared::new("thread-fbok"));
        let (cancel_tx, handle) = spawn_run_observer("sess-fbok", shared);
        tokio::time::sleep(Duration::from_millis(60)).await;
        cancel_tx.send(()).unwrap();
        handle.await.unwrap();
        std::env::remove_var("FUTURE_TEST_OBSERVER_BACKOFF_MS");
    }

    /// When the fallback plain subscribe also fails, the observer sleeps the
    /// backoff and re-attaches (the `continue 'attach` arm).
    #[tokio::test]
    async fn run_observer_continues_after_plain_subscribe_failure() {
        let _home = TestHome::new("observer-fallback-retry");
        let mock = mock_agent();
        std::env::set_var("FUTURE_TEST_OBSERVER_BACKOFF_MS", "50");
        for _ in 0..2 {
            mock.push_data(
                "get_session_events_since",
                serde_json::json!({"events": []}),
            );
            mock.push_data(
                "get_state",
                serde_json::json!({"activeRun": {"runId": "run-fbr"}}),
            );
            mock.push_stream(StreamScript::AttachError(
                tonic::Code::FailedPrecondition,
                "ended",
            ));
            mock.push_plain_stream(StreamScript::AttachError(tonic::Code::Internal, "down"));
        }

        let shared = Arc::new(ObserverShared::new("thread-fbr"));
        let (cancel_tx, handle) = spawn_run_observer("sess-fbr", shared);
        // Cycle 1's backoff completes → continue 'attach; cycle 2's backoff is
        // where cancel lands (the `return` arm) after both arms are exercised.
        tokio::time::sleep(Duration::from_millis(75)).await;
        cancel_tx.send(()).unwrap();
        handle.await.unwrap();
        std::env::remove_var("FUTURE_TEST_OBSERVER_BACKOFF_MS");
    }

    #[tokio::test]
    async fn run_observer_retries_atomic_attach_errors() {
        let _home = TestHome::new("observer-atomic-error");
        let mock = mock_agent();
        std::env::set_var("FUTURE_TEST_OBSERVER_BACKOFF_MS", "40");
        mock.push_data(
            "get_session_events_since",
            serde_json::json!({"events": []}),
        );
        mock.push_data(
            "get_state",
            serde_json::json!({"activeRun": {"runId": "run-ae"}}),
        );
        mock.push_data(
            "get_session_events_since",
            serde_json::json!({"events": []}),
        );
        mock.push_data(
            "get_state",
            serde_json::json!({"activeRun": {"runId": "run-ae"}}),
        );
        // Both atomic attaches fail with a non-NotFound status → settle + backoff.
        mock.push_stream(StreamScript::AttachError(tonic::Code::Internal, "down"));
        mock.push_stream(StreamScript::AttachError(tonic::Code::Internal, "down"));

        let shared = Arc::new(ObserverShared::new("thread-ae"));
        let (cancel_tx, handle) = spawn_run_observer("sess-ae", shared);
        // Cycle 1: 40ms backoff completes → double + continue. Cycle 2: 80ms
        // backoff; cancel mid-sleep → return.
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel_tx.send(()).unwrap();
        handle.await.unwrap();
        std::env::remove_var("FUTURE_TEST_OBSERVER_BACKOFF_MS");
    }

    #[tokio::test]
    async fn run_observer_retries_plain_subscribe_failure() {
        let _home = TestHome::new("observer-plain-retry");
        let mock = mock_agent();
        std::env::set_var("FUTURE_TEST_OBSERVER_BACKOFF_MS", "40");
        // No active run → plain subscribe; it fails twice so both the continue
        // and the cancel-during-sleep arms execute.
        mock.push_data(
            "get_session_events_since",
            serde_json::json!({"events": []}),
        );
        mock.push_data("get_state", serde_json::json!({"isStreaming": false}));
        mock.push_plain_stream(StreamScript::AttachError(tonic::Code::Internal, "down"));
        mock.push_data(
            "get_session_events_since",
            serde_json::json!({"events": []}),
        );
        mock.push_data("get_state", serde_json::json!({"isStreaming": false}));
        mock.push_plain_stream(StreamScript::AttachError(tonic::Code::Internal, "down"));

        let shared = Arc::new(ObserverShared::new("thread-pr"));
        let (cancel_tx, handle) = spawn_run_observer("sess-pr", shared);
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel_tx.send(()).unwrap();
        handle.await.unwrap();
        std::env::remove_var("FUTURE_TEST_OBSERVER_BACKOFF_MS");
    }

    #[tokio::test]
    async fn run_observer_gap_breaks_the_stream() {
        let _home = TestHome::new("observer-gap");
        let mock = mock_agent();
        std::env::set_var("FUTURE_TEST_OBSERVER_BACKOFF_MS", "40");
        mock.push_data(
            "get_session_events_since",
            serde_json::json!({"events": []}),
        );
        // No active run → plain subscribe; the stream yields a gap event first.
        mock.push_data("get_state", serde_json::json!({"isStreaming": false}));
        mock.push_plain_stream(StreamScript::Events(
            vec![stream_event("text_chunk", "run-gap-stream", 3)],
            None,
        ));

        let shared = Arc::new(ObserverShared::new("thread-gap"));
        let (cancel_tx, handle) = spawn_run_observer("sess-gap", shared);
        // handle_event rejects the gap (idx 3 with no cursor) → break → backoff.
        tokio::time::sleep(Duration::from_millis(60)).await;
        cancel_tx.send(()).unwrap();
        handle.await.unwrap();
        std::env::remove_var("FUTURE_TEST_OBSERVER_BACKOFF_MS");
    }

    #[tokio::test]
    async fn run_observer_quiet_window_sleeps() {
        let _home = TestHome::new("observer-quiet");
        let mock = mock_agent();
        std::env::set_var("FUTURE_TEST_IDLE_CHECK_MS", "10");
        mock.push_data(
            "get_session_events_since",
            serde_json::json!({"events": []}),
        );
        mock.push_data("get_state", serde_json::json!({"isStreaming": false}));
        // Plain subscribe hangs; the quiet-window timeout fires and, with an
        // ancient last-activity timestamp and no active run, the observer sleeps.

        let shared = quiet_shared("thread-quiet");
        let (_cancel_tx, handle) = spawn_run_observer("sess-quiet", shared);
        // The observer should self-terminate via the quiet-window return; give
        // it a bounded window to do so.
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("observer sleeps")
            .expect("observer joins cleanly");
        std::env::remove_var("FUTURE_TEST_IDLE_CHECK_MS");
    }

    #[tokio::test]
    async fn run_observer_keeps_waiting_through_idle_with_an_active_run() {
        let _home = TestHome::new("observer-idle-active");
        let mock = mock_agent();
        std::env::set_var("FUTURE_TEST_IDLE_CHECK_MS", "10");
        mock.push_data(
            "get_session_events_since",
            serde_json::json!({"events": []}),
        );
        mock.push_data("get_state", serde_json::json!({"isStreaming": false}));
        // Plain subscribe hangs; the idle timeout fires but `has_active_run`
        // keeps `should_sleep()` false, so the observer hits the keep-waiting
        // (`continue`) arm instead of self-terminating.
        let shared = Arc::new(ObserverShared {
            last_activity_ms: AtomicI64::new(0),
            has_active_run: AtomicBool::new(true),
            ..ObserverShared::new("thread-idle-active")
        });
        let (cancel_tx, handle) = spawn_run_observer("sess-idle-active", shared);
        // A few idle cycles, then cancel so the loop exits deterministically.
        tokio::time::sleep(Duration::from_millis(40)).await;
        cancel_tx.send(()).unwrap();
        handle.await.unwrap();
        std::env::remove_var("FUTURE_TEST_IDLE_CHECK_MS");
    }

    #[test]
    fn emit_settings_event_emits_through_a_mock_handle() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        let handle = app.handle();
        // Valid object payload → emits; malformed JSON → no emit.
        emit_settings_event(
            handle,
            "sess",
            "thread",
            "model_changed",
            r#"{"model":"k3"}"#,
        );
        emit_settings_event(handle, "sess", "thread", "model_changed", "not json");
    }
}
