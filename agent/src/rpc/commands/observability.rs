//! Read-only session observability and export handlers: state, messages,
//! event replay, stats, and HTML export.

use std::sync::Arc;

use crate::rpc::{
    generate_session_html, get_state_internal, AppState, RpcCommand, RpcResponse, ServerSession,
    SseEvent,
};

/// Serialized-size budget for one paged `get_events_since` response. Every
/// event crosses the wire about three times (JSON `data` dual-write, typed
/// `ReplayEvent.data`, typed `EventPayload`), so this much journal-serialized
/// content stays well under the 32 MiB gRPC message cap.
pub(crate) const EVENTS_PAGE_BYTE_BUDGET: usize =
    crate::session::content::transport::REPLAY_PAGE_BYTES;

/// Per-event wire size beyond the `data` payload (type, run/event ids,
/// timestamp, idx…), approximating the journal line.
pub(crate) const EVENT_WIRE_OVERHEAD: usize =
    crate::session::content::transport::EVENT_WIRE_OVERHEAD;

/// Events read per sample window when the stored columns cannot decide whether a
/// run fits in the replay budget.
const REPLAY_SAMPLE_EVENTS: usize = 128;

/// Windows sampled from a run, at the middle of each equal slice.
///
/// One window measured 10 of the 21 refused runs on a real journal with no false
/// positive; a second and third window bought one more at 2.7× the cost, which is
/// the wrong trade in an open path — the estimate is only allowed to be wrong
/// towards waste.
const REPLAY_SAMPLE_WINDOWS: usize = 1;

/// Share of the budget a run's stored floor must reach before its events are
/// sampled at all.
///
/// Sampling costs a few page reads, and a run whose stored columns are a long way
/// below the budget cannot be refused however its events are sized — the floor is
/// what a replay charges as a minimum. Below this share the answer is "fits"
/// without reading anything, which keeps every ordinary open free.
const REPLAY_SAMPLE_TRIGGER_PERCENT: usize = 60;

/// Headroom over the extrapolated charge before a run is refused, as a percentage.
///
/// The sample is an estimate and the two answers are not symmetric: refusing a run
/// the replay would have served costs a snapshot, while missing one only costs the
/// replay the old path already paid. Measured over a real journal the windows land
/// within ±10% of the exact charge on the runs that matter, so 15% is the headroom
/// that keeps that error on the safe side of the line.
const REPLAY_ESTIMATE_SAFETY_PERCENT: usize = 115;

/// Messages in a `get_messages` page when the caller names an `offset` but no
/// `limit`. Remote clients name both; this only bounds the unusual request.
const DEFAULT_MESSAGE_PAGE: i64 = 100;

/// Cut `events` to one page for a paging caller (`max_events > 0`): at most
/// `max_events` entries, and at most [`EVENTS_PAGE_BYTE_BUDGET`] of estimated
/// serialized size, whichever comes first. The first event always goes out —
/// even when it alone exceeds the budget — so the caller's cursor always
/// advances. Returns the page plus whether a tail remains. `max_events <= 0`
/// is the legacy unlimited behavior: no cut, `has_more = false`.
pub(crate) fn page_events_tail(events: Vec<SseEvent>, max_events: i64) -> (Vec<SseEvent>, bool) {
    if max_events <= 0 || events.is_empty() {
        return (events, false);
    }
    let count_cap = usize::try_from(max_events).unwrap_or(usize::MAX);
    let mut bytes = 0usize;
    let mut cut = 0usize;
    for event in &events {
        if cut >= count_cap {
            break;
        }
        let size = event.data.len() + EVENT_WIRE_OVERHEAD;
        if cut > 0 && bytes + size > EVENTS_PAGE_BYTE_BUDGET {
            break;
        }
        bytes += size;
        cut += 1;
    }
    let has_more = cut < events.len();
    let mut page = events;
    page.truncate(cut);
    (page, has_more)
}

/// Base directory for HTML exports. Always `/tmp` in production; overridable
/// in tests (the setter is `cfg(test)`-only) so the write-failure arm can be
/// reached deterministically.
static EXPORT_DIR_OVERRIDE: parking_lot::Mutex<Option<std::path::PathBuf>> =
    parking_lot::Mutex::new(None);

fn export_output_path(session_id: &str) -> std::path::PathBuf {
    let base = EXPORT_DIR_OVERRIDE
        .lock()
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    base.join(format!(
        "future_agent_export_{}_{}.html",
        session_id,
        chrono::Local::now().format("%Y%m%d%H%M%S")
    ))
}

/// RAII guard for the test-only export-dir override. Holds the override for
/// its lifetime and restores `/tmp` on drop, so a panic can't leak a bad dir
/// into the parallel success-path export test.
#[cfg(test)]
pub(crate) struct ExportDirGuard;

#[cfg(test)]
impl ExportDirGuard {
    pub(crate) fn new(dir: std::path::PathBuf) -> Self {
        *EXPORT_DIR_OVERRIDE.lock() = Some(dir);
        ExportDirGuard
    }
}

#[cfg(test)]
impl Drop for ExportDirGuard {
    fn drop(&mut self) {
        *EXPORT_DIR_OVERRIDE.lock() = None;
    }
}

/// Serializes the two export tests, since the override is process-global.
#[cfg(test)]
pub(crate) static EXPORT_TEST_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

// How many times this thread built a bootstrap reply. The seam a size gate has
// to keep shut: a rejected run must answer without the projection payload ever
// being materialised. Thread-local so parallel tests cannot see each other.
#[cfg(test)]
thread_local! {
    static BOOTSTRAP_PAYLOAD_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    // How many times this thread asked the broadcaster to fold a run. The other
    // seam: a run the volume gate rejects must never reach the replay at all.
    static FOLD_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) fn handle_get_state(state: &AppState, cmd: &RpcCommand, id: &str) -> String {
    // The session-scoped guard in the dispatcher already resolved `session`, so
    // get_state_internal's only None path (unknown session) is unreachable here.
    let state_val = get_state_internal(
        state,
        &cmd.session_id,
        (!cmd.run_id.is_empty()).then_some(cmd.run_id.as_str()),
    )
    .expect("session-scoped guard guarantees a live session");
    RpcResponse::ok(id, "get_state", state_val)
}

/// `get_system_prompt` — the full system prompt the session's next run would
/// send, including the persistent append segment.
///
/// Kept out of `get_state` deliberately: the prompt is tens of kilobytes, and
/// `session status` polls `get_state`. Clients ask for the body only when a
/// human wants to read it.
pub(crate) fn handle_get_system_prompt(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    let sess = session.read();
    let prompt = sess.current_system_prompt();
    RpcResponse::ok(
        id,
        "get_system_prompt",
        serde_json::json!({
            "systemPrompt": prompt,
            "appendSystemPrompt": sess.append_system_prompt,
            "customSystemPrompt": sess.custom_system_prompt,
            "contextFiles": crate::prompt::load_project_context(&sess.cwd, !sess.no_context_files)
                .file_names(),
            "contextFilesEnabled": !sess.no_context_files,
        }),
    )
}

pub(crate) fn handle_get_messages(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    // A caller that names a page gets the page. Resolving `offset` to a row range
    // in `entries` and hydrating only those rows is what stops this command from
    // growing with the conversation: the whole transcript used to be mapped and
    // serialised for a reply carrying 40 messages, which cost 249 ms on a
    // 14,531-entry conversation and made a 25,000-entry one fail outright on the
    // 32 MiB frame limit. A caller that names no page keeps the whole in-memory
    // list, unchanged.
    if cmd.offset.is_some() || cmd.limit.is_some() {
        let offset = cmd.offset.unwrap_or(0).max(0).unsigned_abs() as usize;
        let limit = cmd
            .limit
            .filter(|limit| *limit > 0)
            .unwrap_or(DEFAULT_MESSAGE_PAGE) as usize;
        let (session_manager, session_id, supports_images) = {
            let current = session.read();
            let supports_images = current
                .model_registry
                .read()
                .request_model_accepts_images(&current.model);
            (
                current.session_manager.clone(),
                current.session_id.clone(),
                supports_images,
            )
        };
        let (entries, total) = match session_manager.message_page(&session_id, offset, limit) {
            Ok(page) => page,
            Err(error) => {
                return RpcResponse::build_fail(
                    id,
                    "get_messages",
                    &format!("Unable to read conversation messages: {error}"),
                )
            }
        };
        let messages = crate::session::entries_to_agent_messages(&entries, supports_images);
        let msgs: Vec<_> = messages.iter().map(context_message).collect();
        let kept = msgs.len();
        return RpcResponse::ok(
            id,
            "get_messages",
            serde_json::json!({
                "messages": msgs,
                "offset": offset,
                "nextOffset": offset + kept,
                "total": total,
                "hasMore": offset + kept < total,
            }),
        );
    }
    let session = session.read();
    let messages = session.messages.read();
    let msgs: Vec<_> = messages.iter().map(context_message).collect();
    RpcResponse::ok(id, "get_messages", serde_json::json!({"messages": msgs}))
}

/// One stored message as the RPC carries it.
fn context_message(message: &crate::types::AgentMessage) -> future_rpc::message::ContextMessage {
    future_rpc::message::ContextMessage {
        role: message.role.clone(),
        run_id: message
            .metadata
            .as_ref()
            .and_then(|m| m.get("run_id"))
            .and_then(|v| v.as_str())
            .map(str::to_owned),
        blocks: message
            .content
            .iter()
            .map(|block| {
                future_rpc::message::MessageBlock::from_model(
                    &serde_json::to_value(block).expect("model block"),
                )
            })
            .collect(),
        metadata: message.metadata.clone().and_then(|mut meta| {
            meta.remove("run_id");
            (!meta.is_empty()).then_some(serde_json::Value::Object(meta))
        }),
    }
}

pub(crate) fn handle_get_events_since(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    let broadcaster = session.read().broadcaster.clone();
    let available = match broadcaster.replay_watermark(&cmd.run_id) {
        Ok(cursor) => cursor,
        Err(error) => return RpcResponse::build_fail(id, "get_events_since", &error.to_string()),
    };
    if cmd
        .replay_until_idx
        .is_some_and(|until| until < -1 || available.is_some_and(|available| until > available))
    {
        return RpcResponse::build_fail(id, "get_events_since", "replay_window_changed");
    }
    let watermark = cmd.replay_until_idx.or(available);
    let limit = (cmd.max_events > 0).then(|| {
        usize::try_from(cmd.max_events)
            .unwrap_or(usize::MAX)
            .saturating_add(1)
    });
    let replay = broadcaster.events_page(&cmd.run_id, cmd.since_idx, limit);
    let (run_id, events, _min_idx, projection) = match replay {
        Ok(replay) => replay,
        Err(error) => {
            return RpcResponse::build_fail(id, "get_events_since", &error.to_string());
        }
    };
    let events = events
        .into_iter()
        .filter(|event| watermark.is_none_or(|until| event.idx <= until))
        .collect();
    if projection
        .as_ref()
        .is_some_and(|projection| watermark.is_some_and(|until| projection.cursor > until))
    {
        return RpcResponse::build_fail(id, "get_events_since", "replay_window_changed");
    }
    // A cursor older than what this Agent can still serve is not a window: the
    // events that fell outside it are gone, and the consumer must be told so it
    // can rebuild from a full read instead of folding the surviving tail into
    // the projection it already has. Two shapes reach here — a compressed
    // projection standing in for a dropped tail, and a raw window whose head the
    // ring or the journal no longer holds (a first event above the cursor).
    let head_lost = cmd.since_idx >= 0
        && match broadcaster.earliest_cursor(&cmd.run_id, cmd.since_idx) {
            Ok(first) => first.is_some_and(|first| cmd.since_idx.saturating_add(1) < first),
            // The replay itself already answered; a head probe that cannot read
            // is not worth failing the read over.
            Err(_) => false,
        };
    let truncated = projection.is_some() || head_lost;
    // Paging (proto max_events): a long run's journal far exceeds the
    // gRPC message cap when returned whole, so a paging caller gets the
    // tail cut to its page size (bounded by a serialized-size budget)
    // and re-requests from the last idx while has_more is set.
    let (events, has_more) = page_events_tail(events, cmd.max_events);
    // Typed payload (audit item 1): ReplayEventPayload / EventsSincePayload.
    let events = events
        .iter()
        .map(crate::rpc::replay_event_payload)
        .collect::<Vec<_>>();
    let projection = projection.map(|snapshot| crate::rpc::payloads::ProjectionPayload {
        run_id: snapshot.run_id,
        cursor: snapshot.cursor,
        events: snapshot
            .events
            .iter()
            .map(crate::rpc::replay_event_payload)
            .collect(),
    });
    let payload = crate::rpc::payloads::EventsSincePayload {
        run_id,
        events,
        truncated,
        projection,
        has_more,
        watermark,
    };
    RpcResponse::ok(
        id,
        "get_events_since",
        serde_json::to_value(payload).unwrap_or_default(),
    )
}

/// Opt-in bootstrap for clients that understand a projection + cursor. Existing
/// replay requests retain their raw event contract. This additive command uses
/// the generic JSON response carrier; no protobuf fields are added/reused.
pub(crate) fn handle_get_run_snapshot(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    // Decide a run that cannot fit from its stored volume, before the journal is
    // replayed. The fold is O(events) and its result is thrown away, so without
    // this every open of a long run paid it only to be told `too_large`.
    if run_journal_over_replay_budget(session, &cmd.run_id) {
        return replay_too_large(id);
    }
    let broadcaster = session.read().broadcaster.clone();
    #[cfg(test)]
    FOLD_CALLS.with(|calls| calls.set(calls.get() + 1));
    let snapshot = match broadcaster.run_snapshot(&cmd.run_id) {
        Ok(snapshot) => snapshot,
        Err(error)
            if matches!(
                error.to_string().as_str(),
                "run snapshot exceeds memory budget"
                    | "run replay exceeds memory budget; use paged get_events_since"
            ) =>
        {
            return RpcResponse::build_fail_code(
                id,
                "get_run_snapshot",
                "run_snapshot_too_large",
                &error.to_string(),
                serde_json::json!({}),
            )
        }
        Err(error)
            if matches!(
                error.to_string().as_str(),
                "run snapshot prefix is incomplete" | "run snapshot is empty"
            ) =>
        {
            return RpcResponse::build_fail_code(
                id,
                "get_run_snapshot",
                "run_snapshot_unavailable",
                &error.to_string(),
                serde_json::json!({}),
            );
        }
        Err(error) => return RpcResponse::build_fail(id, "get_run_snapshot", &error.to_string()),
    };
    // No visible semantic events yet: the legacy raw read is small and handles
    // this boundary without manufacturing a restorable projection.
    if snapshot.events.is_empty() {
        return RpcResponse::build_fail_code(
            id,
            "get_run_snapshot",
            "run_snapshot_unavailable",
            "run snapshot has no semantic events yet",
            serde_json::json!({}),
        );
    }
    // Leave room below both the Agent's gRPC and Desktop's immutable snapshot
    // budgets. Exceptional huge runs fall back explicitly to paged replay,
    // never a silently truncated projection with a falsely advanced cursor.
    //
    // The floor is checked before the reply exists: materialising the projection
    // clones every event's `data` into a JSON tree, and for a run that cannot fit
    // that whole tree is built and thrown away. The floor is a lower bound, so it
    // only answers the overshoots the exact check below would have rejected
    // anyway; a run it passes still meets the byte-exact test.
    if bootstrap_reply_floor(&snapshot.events) > EVENTS_PAGE_BYTE_BUDGET {
        return bootstrap_too_large(id);
    }
    #[cfg(test)]
    BOOTSTRAP_PAYLOAD_BUILDS.with(|builds| builds.set(builds.get() + 1));
    let payload = serde_json::json!({
        "runSnapshot": true,
        "events": [],
        "watermark": snapshot.cursor,
        "nextSinceIdx": snapshot.cursor,
        "hasMore": false,
        "projection": crate::rpc::payloads::ProjectionPayload {
            run_id: snapshot.run_id, cursor: snapshot.cursor,
            events: snapshot.events.iter().map(crate::rpc::replay_event_payload).collect(),
        },
    });
    if bootstrap_reply_bytes(&payload) > EVENTS_PAGE_BYTE_BUDGET {
        return bootstrap_too_large(id);
    }
    RpcResponse::ok(id, "get_run_snapshot", payload)
}

/// The one answer an oversize bootstrap gets, from either size check: the same
/// token and text, so a caller cannot tell which check caught it (the host maps
/// the token to `None` and pages instead).
fn bootstrap_too_large(id: &str) -> String {
    RpcResponse::build_fail_code(
        id,
        "get_run_snapshot",
        "run_snapshot_too_large",
        "run snapshot exceeds the bootstrap byte budget",
        serde_json::json!({}),
    )
}

/// The answer the replay itself gives when it crosses its byte budget. A run the
/// volume gate rejects answers with this exact token and text, so the cheap
/// decision and the expensive one are indistinguishable on the wire.
fn replay_too_large(id: &str) -> String {
    RpcResponse::build_fail_code(
        id,
        "get_run_snapshot",
        "run_snapshot_too_large",
        "run replay exceeds memory budget; use paged get_events_since",
        serde_json::json!({}),
    )
}

/// Whether replaying `run_id` must cross its byte budget, decided from the stored
/// journal's row count and body bytes instead of from a replay.
///
/// Only a settled run on the canonical format is judged, and that is what makes
/// the answer safe to give without looking:
///
/// * A live run is served from the in-memory projection, whose own budget
///   `run_snapshot` already checks in constant time — the journal's size says
///   nothing about what that run can serve.
/// * An unknown run must reach `run_snapshot`, which owns the `not known` answer.
/// * The legacy format has no replay byte budget; its answer depends on the folded
///   payload, so the volume cannot decide it.
///
/// For the rest, `rows * overhead + body_bytes` is a lower bound on the replay's
/// own accounting, so crossing it means the replay would have crossed its budget
/// and answered `too_large` — the same answer, one journal read earlier.
fn run_journal_over_replay_budget(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    run_id: &str,
) -> bool {
    let (broadcaster, session_id, manager) = {
        let session = session.read();
        (
            session.broadcaster.clone(),
            session.session_id.clone(),
            session.session_manager.clone(),
        )
    };
    if broadcaster.current_run_id() == run_id {
        return false;
    }
    // The probe also fences the journal writer, so the counters below see every
    // append that preceded this read. A run this Agent cannot name is left to
    // `run_snapshot`, which owns that answer.
    match broadcaster.replay_watermark(run_id) {
        Ok(Some(_)) => {}
        _ => return false,
    }
    let Ok(store) = manager.storage() else {
        return false;
    };
    let Ok(Some((rows, body_bytes, metadata_bytes))) = store.run_replay_volume(&session_id, run_id)
    else {
        return false;
    };
    // The stored columns alone are a floor, and an exact decision when they cross:
    // every row costs its wire overhead and its own content, so a run this heavy
    // cannot be replayed inside the budget whatever its events say.
    let stored = rows
        .saturating_mul(EVENT_WIRE_OVERHEAD as i64)
        .saturating_add(body_bytes);
    let stored = usize::try_from(stored).unwrap_or(usize::MAX);
    if stored > EVENTS_PAGE_BYTE_BUDGET {
        return true;
    }
    // Otherwise the answer is in the events themselves, not in the columns: a
    // replayed event's data is the *reconstructed* JSON — entry ids, content keys
    // and offsets that live in the shared context tables, not in the row — so on
    // real runs it is ~340 B even when the row's own body is 40 B. Measure that
    // density from a bounded sample instead of replaying the run to find out.
    let floor = usize::try_from(rows)
        .unwrap_or(usize::MAX)
        .saturating_mul(EVENT_WIRE_OVERHEAD)
        .saturating_add(usize::try_from(body_bytes).unwrap_or(usize::MAX))
        .saturating_add(usize::try_from(metadata_bytes).unwrap_or(usize::MAX));
    if floor.saturating_mul(100)
        <= EVENTS_PAGE_BYTE_BUDGET.saturating_mul(REPLAY_SAMPLE_TRIGGER_PERCENT)
    {
        return false;
    }
    let (sampled_charge, sampled_events) = sample_replay_charge(store, &session_id, run_id, rows);
    replay_estimate_over_budget(
        rows,
        body_bytes,
        metadata_bytes,
        sampled_charge,
        sampled_events,
    )
}

/// The gate's arithmetic, on its own so both boundaries can be tested exactly.
///
/// The stored columns give a floor; when that floor is enough the answer is exact.
/// Otherwise the sampled density is extrapolated over the run's rows and compared
/// against the budget with [`REPLAY_ESTIMATE_SAFETY_PERCENT`] of headroom, because
/// an estimate may only ever be wrong towards waste — never towards refusing a run
/// the replay would have served.
fn replay_estimate_over_budget(
    rows: i64,
    body_bytes: i64,
    metadata_bytes: i64,
    sampled_charge: usize,
    sampled_events: usize,
) -> bool {
    let rows = usize::try_from(rows).unwrap_or(usize::MAX);
    let body_bytes = usize::try_from(body_bytes).unwrap_or(usize::MAX);
    let metadata_bytes = usize::try_from(metadata_bytes).unwrap_or(usize::MAX);
    let floor = rows
        .saturating_mul(EVENT_WIRE_OVERHEAD)
        .saturating_add(body_bytes)
        .saturating_add(metadata_bytes);
    if floor > EVENTS_PAGE_BYTE_BUDGET {
        return true;
    }
    if sampled_events == 0 {
        return false;
    }
    // Mean charge per event over the sampled windows, extrapolated to the run.
    let estimate = sampled_charge.saturating_mul(rows) / sampled_events;
    estimate.saturating_mul(100)
        > EVENTS_PAGE_BYTE_BUDGET.saturating_mul(REPLAY_ESTIMATE_SAFETY_PERCENT)
}

/// Total charge and event count of [`REPLAY_SAMPLE_EVENTS`] events taken at
/// [`REPLAY_SAMPLE_WINDOWS`] points spread across `run_id`.
///
/// Read straight from the store rather than through the broadcaster's replay: a
/// settled run's journal is complete, so there is nothing to fence, and the
/// broadcaster's own read would flush the writer on every window — several
/// milliseconds to learn what a page answers in tens of microseconds. A sample is
/// not the run: the windows are there to average out a run that opens with a huge
/// tool result or ends in a burst of text, and the row count read separately is
/// what carries the estimate to the whole.
fn sample_replay_charge(
    store: &crate::session::sqlite_store::SqliteStore,
    session: &str,
    run_id: &str,
    rows: i64,
) -> (usize, usize) {
    let mut charge = 0usize;
    let mut events = 0usize;
    for window in 0..REPLAY_SAMPLE_WINDOWS as i64 {
        // Windows sit at the middles of equal slices, never on the opening event:
        // a run starts with the prompt, which is not what the rest of it costs.
        let offset = rows.saturating_mul(window * 2 + 1) / (2 * REPLAY_SAMPLE_WINDOWS as i64);
        let Ok(page) =
            store.replay_page(session, run_id, offset, false, Some(REPLAY_SAMPLE_EVENTS))
        else {
            continue;
        };
        for event in page.events {
            charge = charge.saturating_add(event.data.len() + EVENT_WIRE_OVERHEAD);
            events += 1;
        }
    }
    (charge, events)
}

/// Smallest number of bytes one folded event adds to the bootstrap reply beyond
/// its `data` string: the camelCase field names, the quoted run/session/event
/// ids and timestamp, the numerals, and the comma before the next entry. Only
/// ever used as a lower bound, so an underestimate is safe — it costs a payload
/// materialisation the byte-exact check then rejects. Counted on a real
/// projection event it is ~115 B of scaffolding before the ids; 64 leaves room.
const REPLAY_EVENT_MIN_JSON_BYTES: usize = 64;

/// A lower bound on the serialized size of the bootstrap reply's events.
///
/// `data` reaches the reply as JSON text, so it can only grow (escaping), and
/// every event carries at least [`REPLAY_EVENT_MIN_JSON_BYTES`] of envelope.
/// That makes `Σ(data.len()) + n ∗ 64` a floor no reply can serialize below.
fn bootstrap_reply_floor(events: &[SseEvent]) -> usize {
    events.iter().fold(0usize, |bytes, event| {
        bytes
            .saturating_add(event.data.len())
            .saturating_add(REPLAY_EVENT_MIN_JSON_BYTES)
    })
}

/// A `Write` sink that keeps nothing. The bootstrap reply is serialized here
/// only to learn how large it would be; the RPC layer serializes it for real,
/// so measuring it with `to_vec` would allocate a second copy of the whole
/// reply — the fat end of an already fat payload — just to drop it.
#[derive(Default)]
struct ByteCount(usize);

impl std::io::Write for ByteCount {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The reply's exact serialized size, counted without keeping the bytes.
fn bootstrap_reply_bytes(payload: &serde_json::Value) -> usize {
    let mut count = ByteCount::default();
    serde_json::to_writer(&mut count, payload).expect("snapshot JSON");
    count.0
}

pub(crate) fn handle_get_session_events_since(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    let replay = session
        .read()
        .broadcaster
        .session_events_since(cmd.since_idx);
    match replay {
        Ok(events) => RpcResponse::ok(
            id,
            "get_session_events_since",
            serde_json::json!({
                "events": events.into_iter().map(|event| serde_json::json!({
                    "type": event.event_type,
                    "data": event.data,
                    "sessionId": event.session_id,
                    "sessionIdx": event.session_idx,
                    "eventId": event.event_id,
                    "timestamp": event.timestamp,
                })).collect::<Vec<_>>()
            }),
        ),
        Err(error) => RpcResponse::build_fail(id, "get_session_events_since", &error.to_string()),
    }
}

pub(crate) fn handle_get_session_stats(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    let stats = session.read().get_session_stats();
    RpcResponse::ok(id, "get_session_stats", stats)
}

pub(crate) fn handle_get_runtime_metrics(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    let metrics = session.read().get_runtime_metrics();
    RpcResponse::ok(id, "get_runtime_metrics", metrics)
}

pub(crate) fn handle_get_last_assistant_text(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    let (text, last_tool) = {
        let session = session.read();
        (session.get_last_assistant_text(), session.last_tool_name())
    };
    // `lastTool` lets a caller that is about to delete the conversation record
    // one clue about how far the run got, instead of only an empty answer.
    RpcResponse::ok(
        id,
        "get_last_assistant_text",
        serde_json::json!({
            "text": if text.is_empty() { None } else { Some(text) },
            "lastTool": last_tool,
        }),
    )
}

pub(crate) fn handle_export_html(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    // Export session to HTML file
    let sess = session.read();
    let session_id = sess.session_id();
    let model = sess.model.clone();
    let cwd = sess.cwd.clone();
    // The session's own block history, not the LLM projection: thinking, a
    // tool call and its result keep the places the export folds them into.
    let messages = sess.messages.read().clone();
    drop(sess);

    // Generate HTML
    let html = generate_session_html(&session_id, &model, &cwd, &messages);

    // Write to a unique temp file to avoid clobbering concurrent exports.
    let output_path = export_output_path(&session_id);
    let output_path_str = output_path.to_string_lossy().to_string();
    if let Err(e) = std::fs::write(&output_path, html) {
        return RpcResponse::build_fail(id, "export_html", &format!("failed to write file: {}", e));
    }

    RpcResponse::ok(
        id,
        "export_html",
        serde_json::json!({"path": output_path_str}),
    )
}

#[cfg(test)]
mod bootstrap_budget_tests {
    use super::*;
    use crate::rpc::commands::test_support::*;
    use crate::rpc::{handle_command_internal, SseBroadcaster};

    fn read(state: &AppState, run: &str) -> serde_json::Value {
        let mut cmd = make_cmd("get_run_snapshot");
        cmd.run_id = run.into();
        parse_response(&handle_command_internal(state, cmd))
    }

    fn reply_builds() -> usize {
        BOOTSTRAP_PAYLOAD_BUILDS.with(|builds| builds.get())
    }

    fn fold_calls() -> usize {
        FOLD_CALLS.with(|calls| calls.get())
    }

    /// Publish `count` events into `run`, more than the journal queue holds at
    /// once. The writer's queue is bounded (4,096): a tight loop of tens of
    /// thousands of `try_send`s fills it and interrupts the run, so the chunks
    /// are fenced with a watermark probe, which waits for the write to land.
    fn publish_in_chunks(
        broadcaster: &SseBroadcaster,
        run: &str,
        count: usize,
        event: impl Fn(usize) -> SseEvent,
    ) {
        const CHUNK: usize = 2_048;
        let mut start = 0;
        while start < count {
            let end = (start + CHUNK).min(count);
            for index in start..end {
                broadcaster.broadcast(event(index));
            }
            broadcaster
                .replay_watermark(run)
                .expect("fence the journal writer");
            start = end;
        }
    }

    /// A run the bootstrap can carry still returns the snapshot it always did:
    /// same fields, same cursor, same folded events.
    #[test]
    fn a_run_that_fits_still_returns_its_snapshot() {
        let state = make_app_state();
        let session = state.get_session("default").expect("default session");
        let broadcaster = session.read().broadcaster.clone();
        broadcaster.start_run("fitting".into(), 1);
        broadcaster.broadcast(SseEvent::new(
            "agent_start",
            serde_json::json!({"started_at_ms":1}),
        ));
        for _ in 0..2_000 {
            broadcaster.broadcast(SseEvent::new("text_chunk", serde_json::json!({"text":"x"})));
        }

        let before = reply_builds();
        let reply = read(&state, "fitting");
        assert_eq!(reply_builds() - before, 1, "a fitting run builds one reply");

        assert_eq!(reply["success"], true);
        assert_eq!(reply["data"]["runSnapshot"], true);
        assert_eq!(reply["data"]["events"], serde_json::json!([]));
        assert_eq!(reply["data"]["hasMore"], false);
        assert_eq!(reply["data"]["watermark"], 2_000);
        assert_eq!(reply["data"]["nextSinceIdx"], 2_000);
        assert_eq!(reply["data"]["projection"]["runId"], "fitting");
        assert_eq!(reply["data"]["projection"]["cursor"], 2_000);
        // The two thousand chunks fold into accumulated text, not one entry each.
        let events = reply["data"]["projection"]["events"]
            .as_array()
            .expect("projection events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["idx"], 2_000);
    }

    /// The same run read back after it settled answers identically — the volume
    /// gate let it through and the journal replay did the work exactly once.
    #[test]
    fn a_settled_run_that_fits_returns_the_same_snapshot() {
        let state = make_app_state();
        let session = state.get_session("default").expect("default session");
        let broadcaster = session.read().broadcaster.clone();
        broadcaster.start_run("settling".into(), 1);
        broadcaster.broadcast(SseEvent::new(
            "agent_start",
            serde_json::json!({"started_at_ms":1}),
        ));
        for _ in 0..2_000 {
            broadcaster.broadcast(SseEvent::new("text_chunk", serde_json::json!({"text":"x"})));
        }
        // Settle it: the answer now has to come from the journal, not the ring.
        broadcaster.start_run("later".into(), 1);

        let folds = fold_calls();
        let reply = read(&state, "settling");
        assert_eq!(fold_calls() - folds, 1, "a fitting run is replayed once");

        assert_eq!(reply["success"], true);
        assert_eq!(reply["data"]["runSnapshot"], true);
        assert_eq!(reply["data"]["watermark"], 2_000);
        assert_eq!(reply["data"]["nextSinceIdx"], 2_000);
        assert_eq!(reply["data"]["projection"]["runId"], "settling");
        assert_eq!(reply["data"]["projection"]["cursor"], 2_000);
        assert_eq!(
            reply["data"]["projection"]["events"]
                .as_array()
                .expect("projection events")
                .len(),
            2
        );
    }

    /// A settled run whose journal alone is past the replay budget answers with
    /// the replay's own token and text, and never reaches the replay at all: the
    /// stored volume decides it. This is the whole point — without it, every open
    /// read and folded the run only to throw the fold away.
    #[test]
    fn a_settled_run_past_the_replay_budget_answers_without_folding() {
        let state = make_app_state();
        let session = state.get_session("default").expect("default session");
        let broadcaster = session.read().broadcaster.clone();
        // A row per tool result, each carrying a 2 KiB body: 3,600 rows put the
        // journal past the 8 MiB replay budget without a 26,000-event fixture.
        const EVENTS: usize = 3_600;
        broadcaster.start_run("long".into(), 1);
        broadcaster.broadcast(SseEvent::new(
            "agent_start",
            serde_json::json!({"started_at_ms":1}),
        ));
        publish_in_chunks(&broadcaster, "long", EVENTS, |index| {
            SseEvent::new(
                "tool_result",
                serde_json::json!({
                    "tool_id": format!("call-{index}"),
                    "text": "y".repeat(2 * 1024),
                    "exit_code": 0,
                }),
            )
        });
        broadcaster.start_run("later".into(), 1);

        let volume = state
            .session_manager
            .storage()
            .expect("session storage")
            .run_replay_volume("default", "long")
            .expect("stored volume")
            .expect("the settled run has a journal");
        // One row per event, plus the run's own `agent_start`.
        assert_eq!(volume.0, EVENTS as i64 + 1);
        // The stored columns alone decide this one; no sample is needed.
        assert!(replay_estimate_over_budget(
            volume.0, volume.1, volume.2, 0, 0
        ));

        let (folds, builds) = (fold_calls(), reply_builds());
        let reply = read(&state, "long");
        assert_eq!(
            fold_calls() - folds,
            0,
            "the replay must not run for a run the stored volume already rejects"
        );
        assert_eq!(reply_builds() - builds, 0);
        assert_eq!(reply["success"], false);
        assert_eq!(reply["error_code"], "run_snapshot_too_large");
        assert_eq!(
            reply["error"],
            "run replay exceeds memory budget; use paged get_events_since"
        );
    }

    /// A long delta run is the case a count gate gets wrong: tens of thousands of
    /// chunks collapse into a handful of journal rows and one accumulated text, so
    /// the snapshot both fits and is cheap, and must still be served.
    #[test]
    fn a_long_delta_run_that_fits_still_returns_its_snapshot() {
        let state = make_app_state();
        let session = state.get_session("default").expect("default session");
        let broadcaster = session.read().broadcaster.clone();
        broadcaster.start_run("streaming".into(), 1);
        broadcaster.broadcast(SseEvent::new(
            "agent_start",
            serde_json::json!({"started_at_ms":1}),
        ));
        let chunks = 17_000;
        publish_in_chunks(&broadcaster, "streaming", chunks, |_| {
            SseEvent::new("text_chunk", serde_json::json!({"text":"x"}))
        });
        broadcaster.start_run("later".into(), 1);

        // The count is past any per-event gate, yet the journal holds far fewer
        // rows: this is why an event count cannot decide the budget.
        let (rows, body, metadata) = state
            .session_manager
            .storage()
            .expect("session storage")
            .run_replay_volume("default", "streaming")
            .expect("stored volume")
            .expect("the settled run has a journal");
        assert!(
            rows * 4 < chunks as i64,
            "deltas coalesce into rows, {rows} of {chunks}"
        );
        // Small events: even the sampled charge stays far under the budget.
        let sampled = rows.saturating_mul(EVENT_WIRE_OVERHEAD as i64) as usize;
        assert!(!replay_estimate_over_budget(
            rows,
            body,
            metadata,
            sampled,
            rows as usize
        ));

        let folds = fold_calls();
        let reply = read(&state, "streaming");
        assert_eq!(
            fold_calls() - folds,
            1,
            "a run that fits is served, not pre-rejected"
        );
        assert_eq!(reply["success"], true);
        assert_eq!(reply["data"]["runSnapshot"], true);
        let cursor = reply["data"]["watermark"].as_i64().expect("watermark");
        assert_eq!(reply["data"]["nextSinceIdx"], cursor);
        assert_eq!(reply["data"]["projection"]["cursor"], cursor);
        assert_eq!(reply["data"]["projection"]["runId"], "streaming");
        // camelCase only: the payload keeps the field names it always had.
        assert_eq!(
            reply["data"]["projection"]["run_id"],
            serde_json::Value::Null
        );
        assert!(!reply["data"]["projection"]["events"]
            .as_array()
            .expect("projection events")
            .is_empty());
    }

    /// Real runs from a live journal, largest first: rows, body bytes, metadata
    /// bytes, the charge a bounded sample measured (`sampled_charge` over
    /// `sampled_events` of them), and what the old path actually answered when the
    /// whole run was replayed (`"refused"` = `run_snapshot_too_large`).
    ///
    /// Captured with `store.run_replay_volume` and a full paged replay of every
    /// run in `~/.future/agent/agent.db`'s largest 29 canonical runs (2026-11-04).
    const REAL_RUNS: &[(i64, i64, i64, usize, usize, &str)] = &[
        (14951, 646116, 1196993, 244490, 384, "refused"),
        (14499, 477027, 1118015, 255776, 384, "refused"),
        (12143, 758922, 1011163, 273005, 384, "served"),
        (15279, 595346, 1179214, 266883, 384, "refused"),
        (12393, 379443, 886070, 253541, 384, "served"),
        (15380, 641583, 1252864, 261571, 384, "refused"),
        (13430, 711487, 1138973, 266907, 384, "refused"),
        (13946, 806069, 1171441, 255022, 384, "refused"),
        (16446, 781400, 1327634, 261518, 384, "refused"),
        (15924, 734702, 1305485, 254791, 384, "refused"),
        (14079, 613023, 1141385, 241957, 384, "refused"),
        (12702, 470044, 956609, 253954, 384, "served"),
        (12295, 659174, 1000727, 250942, 384, "served"),
        (18160, 1005186, 1456063, 247633, 384, "refused"),
        (14224, 573510, 1127848, 253670, 384, "refused"),
        (17910, 818989, 1512481, 272844, 384, "refused"),
        (12624, 820321, 1012664, 253021, 384, "refused"),
        (15868, 1000053, 1283526, 233375, 384, "refused"),
        (19340, 769636, 1487923, 253934, 384, "refused"),
        (12087, 583334, 968720, 259736, 384, "served"),
        (14343, 743855, 1163593, 253195, 384, "refused"),
        (17291, 863816, 1360673, 277870, 384, "refused"),
        (12796, 569811, 979526, 263639, 384, "served"),
        (13565, 1018597, 1125236, 268993, 384, "refused"),
        (14443, 732149, 1181571, 247893, 384, "refused"),
        (32195, 1462914, 2592910, 249010, 384, "refused"),
        (12750, 831826, 1039253, 255086, 384, "refused"),
        (12367, 427771, 947268, 245550, 384, "served"),
        (12075, 715484, 975054, 273564, 384, "served"),
    ];

    /// The correctness bar, on real runs: a run the old path served must still be
    /// served, byte for byte — so the gate may not refuse a single one of them.
    #[test]
    fn no_real_run_that_returned_a_snapshot_is_refused() {
        let mut refused = 0;
        for (rows, body, metadata, charge, events, verdict) in REAL_RUNS {
            let gated = replay_estimate_over_budget(*rows, *body, *metadata, *charge, *events);
            if *verdict == "served" {
                assert!(
                    !gated,
                    "run of {rows} rows returned a snapshot before this gate existed"
                );
            } else if gated {
                refused += 1;
            }
        }
        // Ten of the twenty-one runs the old path refused are now refused before the
        // replay. The other eleven sit within the estimate's headroom and still pay
        // it: refusing those on a sampled estimate is the false refusal this test
        // exists to prevent, and the numbers say the two sets are 1.3% apart.
        assert_eq!(refused, 10, "gated real runs");
    }

    /// Both lines, exactly: the stored floor and the sampled estimate each hold one
    /// row/percent under and one over the budget.
    #[test]
    fn the_gate_brackets_the_stored_floor_and_the_sampled_estimate() {
        // Stored floor: rows alone, then bodies, then metadata.
        let rows_at_budget = (EVENTS_PAGE_BYTE_BUDGET / EVENT_WIRE_OVERHEAD) as i64;
        assert!(!replay_estimate_over_budget(rows_at_budget, 0, 0, 0, 0));
        assert!(replay_estimate_over_budget(rows_at_budget + 1, 0, 0, 0, 0));
        let body = EVENTS_PAGE_BYTE_BUDGET - EVENT_WIRE_OVERHEAD;
        assert!(!replay_estimate_over_budget(1, body as i64, 0, 0, 0));
        assert!(replay_estimate_over_budget(1, body as i64 + 1, 0, 0, 0));
        let metadata = EVENTS_PAGE_BYTE_BUDGET - EVENT_WIRE_OVERHEAD;
        assert!(!replay_estimate_over_budget(1, 0, metadata as i64, 0, 0));
        assert!(replay_estimate_over_budget(1, 0, metadata as i64 + 1, 0, 0));

        // Sampled estimate: the same run, judged from its own events. One event of
        // the budget is served; the safety headroom keeps the exact line above it.
        let rows = 12_000i64;
        let at = EVENTS_PAGE_BYTE_BUDGET / rows as usize;
        assert!(!replay_estimate_over_budget(
            rows,
            0,
            0,
            at * rows as usize,
            rows as usize
        ));
        // 3% over the budget is still inside the headroom the estimate is refused by.
        let over = EVENTS_PAGE_BYTE_BUDGET * REPLAY_ESTIMATE_SAFETY_PERCENT / (100 * rows as usize);
        assert!(!replay_estimate_over_budget(
            rows,
            0,
            0,
            over * rows as usize,
            rows as usize
        ));
        assert!(replay_estimate_over_budget(
            rows,
            0,
            0,
            (over + 2) * rows as usize,
            rows as usize
        ));
    }

    /// A run past the byte budget answers the same token and message it always
    /// did — and does so without ever building the reply it would discard.
    #[test]
    fn an_oversize_run_answers_before_the_reply_is_built() {
        let state = make_app_state();
        let session = state.get_session("default").expect("default session");
        // One event whose own text fills the budget: cheap to build, impossible
        // to serve.
        let broadcaster = Arc::new(SseBroadcaster::new());
        session.write().broadcaster = broadcaster.clone();
        broadcaster.start_run("large".into(), 1);
        broadcaster.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text":"x".repeat(EVENTS_PAGE_BYTE_BUDGET)}),
        ));

        let before = reply_builds();
        let reply = read(&state, "large");
        assert_eq!(
            reply_builds() - before,
            0,
            "the folded reply must not be materialised only to be discarded"
        );
        assert_eq!(reply["success"], false);
        assert_eq!(reply["error_code"], "run_snapshot_too_large");
        assert_eq!(
            reply["error"],
            "run snapshot exceeds the bootstrap byte budget"
        );
    }

    /// The floor the gate decides on sits exactly on the per-event budget, and
    /// it never overstates a reply it is only guessing at.
    #[test]
    fn the_reply_floor_brackets_the_budget_without_overstating_it() {
        let with_data = |len: usize| {
            let mut event = SseEvent::new("text_chunk", serde_json::Value::Null);
            event.data = "x".repeat(len);
            event
        };

        // Just under: one event's data can fill the budget minus its envelope
        // and still leave the gate open.
        let under = [with_data(
            EVENTS_PAGE_BYTE_BUDGET - REPLAY_EVENT_MIN_JSON_BYTES,
        )];
        assert_eq!(bootstrap_reply_floor(&under), EVENTS_PAGE_BYTE_BUDGET);
        // Just over: one more byte of data and no reply carrying it can fit.
        let over = [with_data(
            EVENTS_PAGE_BYTE_BUDGET - REPLAY_EVENT_MIN_JSON_BYTES + 1,
        )];
        assert!(bootstrap_reply_floor(&over) > EVENTS_PAGE_BYTE_BUDGET);

        // And it is a floor, not an estimate: a reply carrying these events
        // serializes to strictly more than the floor says it will.
        let events: Vec<SseEvent> = (0..3).map(|_| with_data(128)).collect();
        let payload = serde_json::json!({
            "runSnapshot": true,
            "events": [],
            "watermark": 2,
            "nextSinceIdx": 2,
            "hasMore": false,
            "projection": crate::rpc::payloads::ProjectionPayload {
                run_id: "run".into(),
                cursor: 2,
                events: events.iter().map(crate::rpc::replay_event_payload).collect(),
            },
        });
        assert!(bootstrap_reply_bytes(&payload) > bootstrap_reply_floor(&events));
        assert!(bootstrap_reply_floor(&events) <= bootstrap_reply_bytes(&payload));
    }
}
