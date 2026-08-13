//! Consumes the Future Agent event stream for a single prompt: accumulates
//! assistant text, drives the approval wait-state, and forwards every event to
//! the persistence projection. Returns the assembled assistant text once the
//! agent signals `agent_end`.

use tokio::time::{sleep, timeout, Duration};
use tonic::Code;

use super::{connect_agent, persist::persist_run_event};
use crate::agent_proto::StreamRequest;

const AGENT_EVENT_STREAM_TIMEOUT_SECS: u64 = 600;
const STREAM_RECONNECT_ATTEMPTS: u32 = 6;

/// Idle timeout for one stream read while the run is not parked on an
/// approval. The 600s production budget is unexercisable in real time, so
/// tests override it via env (a cfg(test)-only seam).
fn agent_event_stream_timeout() -> Duration {
    #[cfg(test)]
    if let Some(ms) = std::env::var("FUTURE_TEST_AGENT_STREAM_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        return Duration::from_millis(ms);
    }
    Duration::from_secs(AGENT_EVENT_STREAM_TIMEOUT_SECS)
}

/// Outcome of collecting one canonical Agent run. `RunGone` is distinct from a
/// transient/`App` failure: the Agent explicitly does not recognize the run
/// (it rolled over, restarted, or already settled and dropped the ring), so the
/// caller must reconcile the local row from the Agent's journal (`get_state`)
/// instead of retrying the attach or marking the run failed.
#[derive(Debug)]
pub(super) enum CollectError {
    App(crate::AppError),
    RunGone(String),
}

impl From<crate::AppError> for CollectError {
    fn from(error: crate::AppError) -> Self {
        CollectError::App(error)
    }
}

impl From<String> for CollectError {
    fn from(error: String) -> Self {
        CollectError::App(error.into())
    }
}

impl From<CollectError> for crate::AppError {
    fn from(error: CollectError) -> Self {
        match error {
            CollectError::App(error) => error,
            CollectError::RunGone(reason) => {
                crate::AppError::from(format!("Future Agent run no longer active: {reason}"))
            }
        }
    }
}

/// Why an `atomic_attach` failed. Only `FailedPrecondition` / `NotFound` mean the
/// run is gone; everything else (connect failure, unknown code) is a transient
/// the reconnect loop should retry.
enum AttachFailure {
    Transient(String),
    RunGone(String),
}

/// Persist a run event on a blocking thread, so the synchronous SQLite write
/// (and the occasional `git` fork on write/artifact events) doesn't stall the
/// async event loop. Awaited to preserve event order; errors are logged inside
/// `persist_run_event`.
pub(super) async fn persist_run_event_off_thread(
    thread_id: &str,
    run_id: Option<&str>,
    event_type: String,
    data: String,
    sequence: i64,
) {
    let thread_id = thread_id.to_string();
    let run_id = run_id.map(str::to_string);
    let emitted_run_id = run_id.clone();
    let status = if event_type == "agent_end" {
        "finalizing"
    } else {
        "running"
    }
    .to_string();
    let _ = tokio::task::spawn_blocking(move || {
        persist_run_event(run_id.as_deref(), &event_type, &data, sequence);
        if let Some(run_id) = emitted_run_id {
            crate::emit_thread_runtime_updated(thread_id, run_id, status, false);
        }
    })
    .await;
}

pub(super) async fn replace_projection_off_thread(
    thread_id: &str,
    local_run_id: Option<&str>,
    events: Vec<(String, String, i64)>,
) {
    let thread_id = thread_id.to_string();
    let local_run_id = local_run_id.map(str::to_string);
    let emitted_run_id = local_run_id.clone();
    let terminal = events
        .iter()
        .any(|(event_type, _, _)| event_type == "agent_end");
    let _ = tokio::task::spawn_blocking(move || {
        if let Some(run_id) = local_run_id.as_deref() {
            // The Agent snapshot is already its canonical journal projection.
            // Fold it only into derived SQLite records; GUI raw-event JSONL is
            // legacy read-only and must never be rewritten here.
            for (event_type, data, sequence) in events {
                persist_run_event(Some(run_id), &event_type, &data, sequence);
            }
        }
        if let Some(run_id) = emitted_run_id {
            crate::emit_thread_runtime_updated(
                thread_id,
                run_id,
                if terminal { "finalizing" } else { "running" }.to_string(),
                true,
            );
        }
    })
    .await;
}

/// The assembled assistant text plus whether the stream reached a clean
/// `agent_end`. `complete == false` means the stream ended (server closed it,
/// agent restarted mid-reply) before signalling completion — the text is a
/// prefix, not the whole answer, and the caller must mark the run `failed`
/// rather than persist a silently truncated reply as `completed`.
#[derive(Debug)]
pub(super) struct AgentResponse {
    pub content: String,
    pub complete: bool,
}

pub(super) async fn collect_agent_response(
    local_run_id: Option<&str>,
    canonical_run_id: &str,
    session_id: &str,
    thread_id: &str,
) -> Result<AgentResponse, CollectError> {
    let mut content = String::new();
    let mut waiting_for_approval = false;
    // The collector only runs for freshly accepted prompts: the Agent journal
    // holds no earlier events for this run, so attach from the start. The GUI
    // keeps no durable event copy to resume from (the Agent journal is the
    // source of truth); a mid-collect reconnect resumes from `last_idx` below,
    // and a forced re-attach replays through the gap check.
    let mut last_idx: i64 = -1;
    let mut reconnect_attempt = 0_u32;

    let clean_end = 'attach: loop {
        let attach_result = async {
            let mut client = connect_agent()
                .await
                .map_err(|error| AttachFailure::Transient(error.to_string()))?;
            client
                .stream_events(StreamRequest {
                    event_types: vec![],
                    session_id: session_id.to_string(),
                    run_id: canonical_run_id.to_string(),
                    after_idx: last_idx,
                    atomic_attach: true,
                })
                .await
                .map(|response| response.into_inner())
                .map_err(|status| match status.code() {
                    // The Agent no longer has this run (rolled over, restarted,
                    // or settled and dropped the ring). Not retryable: the run is
                    // gone, so the caller must reconcile from the journal.
                    Code::FailedPrecondition | Code::NotFound => {
                        AttachFailure::RunGone(status.to_string())
                    }
                    _ => AttachFailure::Transient(status.to_string()),
                })
        }
        .await;

        let mut stream = match attach_result {
            Ok(stream) => stream,
            Err(AttachFailure::RunGone(reason)) => return Err(CollectError::RunGone(reason)),
            Err(AttachFailure::Transient(stream_error)) => {
                reconnect_attempt += 1;
                if reconnect_attempt > STREAM_RECONNECT_ATTEMPTS {
                    return Err(format!(
                        "Future Agent stream could not be resumed after {STREAM_RECONNECT_ATTEMPTS} attempts: {stream_error}"
                    )
                    .into());
                }
                sleep(reconnect_delay(reconnect_attempt)).await;
                continue;
            }
        };

        let stream_error = loop {
            let next_event = if waiting_for_approval {
                stream.message().await
            } else {
                match timeout(agent_event_stream_timeout(), stream.message()).await {
                    Ok(result) => result,
                    Err(_) => {
                        break "Future Agent response timed out.".to_string();
                    }
                }
            };

            let next_event = match next_event {
                Ok(event) => event,
                Err(error) => {
                    break format!("Future Agent event stream failed: {error}");
                }
            };

            let Some(event) = next_event else {
                break "Future Agent event stream closed before the run terminal.".to_string();
            };
            reconnect_attempt = 0;

            if event.run_id != canonical_run_id {
                return Err(format!(
                    "Future Agent sent event for run {}, expected {canonical_run_id}",
                    event.run_id
                )
                .into());
            }

            if event.projection_snapshot {
                if event.snapshot_cursor < last_idx {
                    continue;
                }
                let snapshot_events: Vec<(String, String, i64)> = event
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
                replace_projection_off_thread(thread_id, local_run_id, snapshot_events).await;

                content.clear();
                waiting_for_approval = false;
                let mut snapshot_terminal = None;
                for projected in &event.snapshot_events {
                    let projected_data = future_rpc::decode::projected_event_data_json(projected);
                    match fold_response_event(
                        &projected.r#type,
                        &projected_data,
                        &mut content,
                        &mut waiting_for_approval,
                    )? {
                        FoldOutcome::Continue => {}
                        FoldOutcome::Terminal { clean } => {
                            snapshot_terminal = Some(clean);
                        }
                    }
                }
                last_idx = event.snapshot_cursor;
                if let Some(clean) = snapshot_terminal {
                    break 'attach clean;
                }
                continue;
            }

            if event.r#type == "stream_gap" {
                break "Future Agent reported a legacy stream gap without a projection snapshot"
                    .to_string();
            }
            if event.idx <= last_idx {
                continue;
            }
            if event.idx != last_idx.saturating_add(1) {
                // Drop this stream and atomically reattach from the last
                // confirmed cursor. The Agent will return either the missing
                // tail or a full projection snapshot if the ring rolled over.
                break format!(
                    "Future Agent event gap for run {canonical_run_id}: expected {}, received {}",
                    last_idx.saturating_add(1),
                    event.idx
                );
            }
            last_idx = event.idx;

            // Canonical event payload (byte-stable while the agent dual-writes
            // `data`; the typed reconstruction takes over once `data` is
            // retired). Persistence and the content fold read the same string.
            let event_data = future_rpc::decode::event_data_json(&event);
            persist_run_event_off_thread(
                thread_id,
                local_run_id,
                event.r#type.clone(),
                event_data.clone(),
                event.idx,
            )
            .await;
            // Remote mirroring is owned by the session observer (sole NATS
            // publisher — its atomic-attach replay keeps the mirrored sequence
            // gap-free, which two independent publishers could not guarantee).
            match fold_response_event(
                &event.r#type,
                &event_data,
                &mut content,
                &mut waiting_for_approval,
            )? {
                FoldOutcome::Continue => {}
                FoldOutcome::Terminal { clean } => {
                    break 'attach clean;
                }
            }
        };

        reconnect_attempt += 1;
        if reconnect_attempt > STREAM_RECONNECT_ATTEMPTS {
            persist_run_event_off_thread(
                thread_id,
                local_run_id,
                "stream_disconnected".to_string(),
                serde_json::json!({"error": stream_error}).to_string(),
                last_idx.saturating_add(1),
            )
            .await;
            return Err(format!(
                "Future Agent stream could not be resumed after {STREAM_RECONNECT_ATTEMPTS} attempts: {stream_error}"
            )
            .into());
        }
        eprintln!(
            "FutureOS reattaching Agent run {canonical_run_id} after idx {last_idx} \
             (attempt {reconnect_attempt}): {stream_error}"
        );
        sleep(reconnect_delay(reconnect_attempt)).await;
    };

    Ok(AgentResponse {
        content,
        complete: clean_end,
    })
}

fn reconnect_delay(attempt: u32) -> Duration {
    Duration::from_millis((100_u64 << attempt.min(4)).min(2_000))
}

enum FoldOutcome {
    Continue,
    Terminal { clean: bool },
}

fn fold_response_event(
    event_type: &str,
    data: &str,
    content: &mut String,
    waiting_for_approval: &mut bool,
) -> Result<FoldOutcome, crate::AppError> {
    match event_type {
        "approval_request" => {
            *waiting_for_approval = true;
        }
        "approval_decision" => {
            *waiting_for_approval = false;
        }
        "text_chunk" => {
            if let Some(text) = event_text(data) {
                content.push_str(&text);
            }
        }
        "agent_end" => {
            return Ok(FoldOutcome::Terminal {
                clean: !agent_end_incomplete(data),
            });
        }
        "error" => {
            return Err(event_error(data)
                .unwrap_or_else(|| "Future Agent returned an error event.".to_string())
                .into());
        }
        _ => {}
    }
    Ok(FoldOutcome::Continue)
}

/// Returns true when an `agent_end` event's data marks the reply as incomplete —
/// i.e. the LLM stream was truncated before a genuine finish. Such a reply is a
/// prefix and must not be persisted as a clean completion.
pub(super) fn agent_end_incomplete(data: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(data)
        .ok()
        .and_then(|value| {
            value
                .get("reason")
                .and_then(|reason| reason.as_str())
                .map(str::to_string)
        })
        .is_some_and(|reason| reason == "incomplete")
}

fn event_text(data: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(data)
        .ok()
        .and_then(|value| {
            value
                .get("text")
                .and_then(|text| text.as_str())
                .map(str::to_string)
        })
}

fn event_error(data: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(data)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .or_else(|| value.get("message"))
                .and_then(|error| error.as_str())
                .map(str::to_string)
        })
}

#[cfg(test)]
mod tests {
    use super::{
        agent_end_incomplete, fold_response_event, reconnect_delay, FoldOutcome,
        STREAM_RECONNECT_ATTEMPTS,
    };

    #[test]
    fn incomplete_reason_marks_truncated() {
        // Truncated stream: run loop emits agent_end reason "incomplete".
        assert!(agent_end_incomplete(r#"{"reason":"incomplete"}"#));
        assert!(agent_end_incomplete(
            r#"{"reason":"incomplete","stop_reason":"truncated"}"#
        ));
    }

    #[test]
    fn clean_reasons_are_not_truncated() {
        assert!(!agent_end_incomplete(r#"{"reason":"complete"}"#));
        assert!(!agent_end_incomplete(r#"{"reason":"stop_condition"}"#));
        assert!(!agent_end_incomplete(r#"{"reason":"interrupted"}"#));
        // Missing / malformed reason must default to clean, not truncated.
        assert!(!agent_end_incomplete(r#"{"usage":{}}"#));
        assert!(!agent_end_incomplete("not json"));
    }

    #[test]
    fn projection_events_rebuild_response_and_terminal_state() {
        let mut content = String::new();
        let mut waiting = false;
        assert!(matches!(
            fold_response_event(
                "text_chunk",
                r#"{"text":"restored answer"}"#,
                &mut content,
                &mut waiting
            )
            .unwrap(),
            FoldOutcome::Continue
        ));
        assert_eq!(content, "restored answer");
        assert!(matches!(
            fold_response_event(
                "agent_end",
                r#"{"reason":"complete"}"#,
                &mut content,
                &mut waiting
            )
            .unwrap(),
            FoldOutcome::Terminal { clean: true }
        ));
    }

    #[test]
    fn reconnect_backoff_is_bounded() {
        assert!(reconnect_delay(1) < reconnect_delay(2));
        assert_eq!(
            reconnect_delay(STREAM_RECONNECT_ATTEMPTS),
            std::time::Duration::from_millis(1_600)
        );
        assert_eq!(
            reconnect_delay(u32::MAX),
            std::time::Duration::from_millis(1_600)
        );
    }

    // ── collect_agent_response against the scripted mock ────────────────

    use super::super::test_support::{
        mock_agent, seed_run, seed_thread, seed_workspace, stream_event, StreamScript, TestHome,
    };
    use super::{collect_agent_response, CollectError};
    use crate::agent_proto::ProjectedRunEvent;

    fn projected(event_type: &str, data: &str, idx: i64) -> ProjectedRunEvent {
        ProjectedRunEvent {
            r#type: event_type.to_string(),
            data: data.to_string(),
            idx,
            payload: None,
        }
    }

    fn snapshot_event(
        run_id: &str,
        cursor: i64,
        events: Vec<ProjectedRunEvent>,
    ) -> crate::agent_proto::StreamEvent {
        crate::agent_proto::StreamEvent {
            projection_snapshot: true,
            snapshot_cursor: cursor,
            snapshot_events: events,
            run_id: run_id.to_string(),
            ..Default::default()
        }
    }

    /// A clean one-shot run: text then a complete agent_end.
    #[tokio::test]
    async fn collect_clean_run_assembles_content() {
        let mock = mock_agent();
        mock.push_stream(StreamScript::Events(
            vec![
                stream_event("run-1", 0, "text_chunk", r#"{"text":"hello"}"#),
                stream_event("run-1", 1, "text_chunk", r#"{"text":" world"}"#),
                stream_event("run-1", 2, "agent_end", r#"{"reason":"complete"}"#),
            ],
            None,
        ));
        let response = collect_agent_response(None, "run-1", "sess-1", "thread-1")
            .await
            .expect("collect");
        assert_eq!(response.content, "hello world");
        assert!(response.complete);
        // The collector attaches atomically from the start.
        let attaches = mock.stream_requests();
        assert_eq!(attaches.len(), 1);
        assert!(attaches[0].atomic_attach);
        assert_eq!(attaches[0].after_idx, -1);
    }

    /// The lease-coupled collector path (replica.rs collect) end to end.
    #[tokio::test]
    async fn collect_through_replica_lease_binds_and_releases() {
        let home = TestHome::new("stream-lease");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);

        mock.push_stream(StreamScript::Events(
            vec![
                stream_event(&run.id, 0, "text_chunk", r#"{"text":"via lease"}"#),
                stream_event(&run.id, 1, "agent_end", r#"{"reason":"complete"}"#),
            ],
            None,
        ));
        let manager = &super::super::replica::AGENT_REPLICAS;
        let lease = manager.acquire(&run.id).expect("lease");
        let response = lease
            .collect(Some(&run.id), &run.id, "sess-1", &thread.id)
            .await
            .expect("collect");
        assert_eq!(response.content, "via lease");
        assert!(
            manager.canonical_for_local(&run.id).is_none(),
            "the binding drops with the lease"
        );
        assert!(
            manager.is_owned_or_recently_released(&run.id),
            "the grace window marks the run recently released"
        );
        // The agent_end off-thread persist flipped the run to finalizing
        // bookkeeping without a pipeline settle; the row stays non-terminal
        // until the caller settles it.
    }

    #[tokio::test]
    async fn collect_marks_incomplete_agent_end() {
        let mock = mock_agent();
        mock.push_stream(StreamScript::Events(
            vec![
                stream_event("run-1", 0, "text_chunk", r#"{"text":"prefix"}"#),
                stream_event("run-1", 1, "agent_end", r#"{"reason":"incomplete"}"#),
            ],
            None,
        ));
        let response = collect_agent_response(None, "run-1", "sess-1", "thread-1")
            .await
            .expect("collect");
        assert_eq!(response.content, "prefix");
        assert!(!response.complete);
    }

    #[tokio::test]
    async fn collect_error_event_aborts_with_its_message() {
        let mock = mock_agent();
        mock.push_stream(StreamScript::Events(
            vec![stream_event(
                "run-1",
                0,
                "error",
                r#"{"error":"model exploded"}"#,
            )],
            None,
        ));
        let error = collect_agent_response(None, "run-1", "sess-1", "thread-1")
            .await
            .expect_err("error event");
        assert!(matches!(error, CollectError::App(_)));
        assert_eq!(crate::AppError::from(error).to_string(), "model exploded");

        // No error/message field → generic message.
        mock.push_stream(StreamScript::Events(
            vec![stream_event("run-2", 0, "error", r#"{"what":"unknown"}"#)],
            None,
        ));
        let error = collect_agent_response(None, "run-2", "sess-1", "thread-1")
            .await
            .expect_err("error event");
        assert_eq!(
            crate::AppError::from(error).to_string(),
            "Future Agent returned an error event."
        );
    }

    #[tokio::test]
    async fn collect_rejects_events_for_a_different_run() {
        let mock = mock_agent();
        mock.push_stream(StreamScript::Events(
            vec![stream_event(
                "run-other",
                0,
                "text_chunk",
                r#"{"text":"x"}"#,
            )],
            None,
        ));
        let error = collect_agent_response(None, "run-1", "sess-1", "thread-1")
            .await
            .expect_err("wrong run");
        assert_eq!(
            crate::AppError::from(error).to_string(),
            "Future Agent sent event for run run-other, expected run-1"
        );
    }

    #[tokio::test]
    async fn collect_skips_replay_overlap_and_duplicate_idx() {
        let mock = mock_agent();
        mock.push_stream(StreamScript::Events(
            vec![
                stream_event("run-1", 0, "text_chunk", r#"{"text":"a"}"#),
                stream_event("run-1", 0, "text_chunk", r#"{"text":"dup"}"#),
                stream_event("run-1", 1, "agent_end", r#"{"reason":"complete"}"#),
            ],
            None,
        ));
        let response = collect_agent_response(None, "run-1", "sess-1", "thread-1")
            .await
            .expect("collect");
        assert_eq!(response.content, "a", "the idx-0 replay was deduped");
    }

    #[tokio::test]
    async fn collect_reattaches_after_a_mid_stream_close() {
        let mock = mock_agent();
        // First stream closes without a terminal event.
        mock.push_stream(StreamScript::Events(
            vec![stream_event("run-1", 0, "text_chunk", r#"{"text":"a"}"#)],
            None,
        ));
        // Reattach resumes from the cursor and finishes.
        mock.push_stream(StreamScript::Events(
            vec![
                stream_event("run-1", 1, "text_chunk", r#"{"text":"b"}"#),
                stream_event("run-1", 2, "agent_end", r#"{"reason":"complete"}"#),
            ],
            None,
        ));
        let response = collect_agent_response(None, "run-1", "sess-1", "thread-1")
            .await
            .expect("collect");
        assert_eq!(response.content, "ab");
        assert!(response.complete);
        let attaches = mock.stream_requests();
        assert_eq!(attaches.len(), 2);
        assert_eq!(attaches[1].after_idx, 0, "reattach resumes from the cursor");
    }

    #[tokio::test]
    async fn collect_reattaches_after_a_mid_stream_transport_error() {
        let mock = mock_agent();
        mock.push_stream(StreamScript::Events(
            vec![stream_event("run-1", 0, "text_chunk", r#"{"text":"a"}"#)],
            Some((tonic::Code::DataLoss, "frame exploded")),
        ));
        mock.push_stream(StreamScript::Events(
            vec![stream_event(
                "run-1",
                1,
                "agent_end",
                r#"{"reason":"complete"}"#,
            )],
            None,
        ));
        let response = collect_agent_response(None, "run-1", "sess-1", "thread-1")
            .await
            .expect("collect");
        assert!(response.complete);
    }

    #[tokio::test]
    async fn collect_idx_gap_forces_reattach_and_run_gone_surfaces() {
        let mock = mock_agent();
        // idx skips 1 → gap → drop the stream and reattach.
        mock.push_stream(StreamScript::Events(
            vec![
                stream_event("run-1", 0, "text_chunk", r#"{"text":"a"}"#),
                stream_event("run-1", 2, "text_chunk", r#"{"text":"c"}"#),
            ],
            None,
        ));
        // The agent no longer knows the run on reattach.
        mock.push_stream(StreamScript::AttachError(
            tonic::Code::FailedPrecondition,
            "unknown run",
        ));
        let error = collect_agent_response(None, "run-1", "sess-1", "thread-1")
            .await
            .expect_err("run gone");
        match error {
            CollectError::RunGone(reason) => {
                assert!(reason.contains("unknown run"), "reason: {reason}");
                let app_error = crate::AppError::from(CollectError::RunGone(reason));
                assert!(app_error
                    .to_string()
                    .contains("Future Agent run no longer active"));
            }
            CollectError::App(other) => panic!("expected RunGone, got {other}"),
        }
    }

    #[tokio::test]
    async fn collect_treats_legacy_stream_gap_as_reattach_signal() {
        let mock = mock_agent();
        mock.push_stream(StreamScript::Events(
            vec![
                stream_event("run-1", 0, "text_chunk", r#"{"text":"a"}"#),
                stream_event("run-1", 1, "stream_gap", "{}"),
            ],
            None,
        ));
        mock.push_stream(StreamScript::Events(
            vec![
                stream_event("run-1", 1, "text_chunk", r#"{"text":"b"}"#),
                stream_event("run-1", 2, "agent_end", r#"{"reason":"complete"}"#),
            ],
            None,
        ));
        let response = collect_agent_response(None, "run-1", "sess-1", "thread-1")
            .await
            .expect("collect");
        assert_eq!(response.content, "ab");
    }

    #[tokio::test]
    async fn collect_gives_up_after_six_transient_attach_failures() {
        let mock = mock_agent();
        for _ in 0..=STREAM_RECONNECT_ATTEMPTS {
            mock.push_stream(StreamScript::AttachError(
                tonic::Code::Unavailable,
                "still down",
            ));
        }
        let error = collect_agent_response(None, "run-1", "sess-1", "thread-1")
            .await
            .expect_err("attach failures");
        let message = crate::AppError::from(error).to_string();
        assert!(
            message.contains("could not be resumed after 6 attempts"),
            "message: {message}"
        );
        assert!(message.contains("still down"), "message: {message}");
    }

    #[tokio::test]
    async fn collect_stream_timeout_reattaches() {
        let mock = mock_agent();
        std::env::set_var("FUTURE_TEST_AGENT_STREAM_TIMEOUT_MS", "50");
        // First attach hangs (no events within the idle budget)...
        mock.push_stream(StreamScript::Hang);
        // ...the reattach finishes the run.
        mock.push_stream(StreamScript::Events(
            vec![stream_event(
                "run-1",
                0,
                "agent_end",
                r#"{"reason":"complete"}"#,
            )],
            None,
        ));
        let response = collect_agent_response(None, "run-1", "sess-1", "thread-1")
            .await
            .expect("collect");
        std::env::remove_var("FUTURE_TEST_AGENT_STREAM_TIMEOUT_MS");
        assert!(response.complete);
        assert_eq!(mock.stream_requests().len(), 2);
    }

    #[tokio::test]
    async fn collect_approval_wait_disables_the_idle_timeout() {
        let mock = mock_agent();
        mock.push_stream(StreamScript::Events(
            vec![
                stream_event(
                    "run-1",
                    0,
                    "approval_request",
                    r#"{"approval_request_id":"a1"}"#,
                ),
                stream_event(
                    "run-1",
                    1,
                    "approval_decision",
                    r#"{"approval_request_id":"a1","status":"approved"}"#,
                ),
                stream_event("run-1", 2, "text_chunk", r#"{"text":"done"}"#),
                stream_event("run-1", 3, "agent_end", r#"{"reason":"complete"}"#),
            ],
            None,
        ));
        let response = collect_agent_response(None, "run-1", "sess-1", "thread-1")
            .await
            .expect("collect");
        assert_eq!(response.content, "done");
        assert!(response.complete);
    }

    #[tokio::test]
    async fn collect_applies_a_terminal_projection_snapshot() {
        let home = TestHome::new("stream-snapshot");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);

        mock.push_stream(StreamScript::Events(
            vec![snapshot_event(
                &run.id,
                2,
                vec![
                    projected("text_chunk", r#"{"text":"from snapshot "}"#, 0),
                    projected("text_chunk", r#"{"text":"tail"}"#, 1),
                    projected("agent_end", r#"{"reason":"complete"}"#, 2),
                ],
            )],
            None,
        ));
        let response = collect_agent_response(Some(&run.id), &run.id, "sess-1", &thread.id)
            .await
            .expect("collect");
        assert_eq!(response.content, "from snapshot tail");
        assert!(response.complete);
    }

    #[tokio::test]
    async fn collect_skips_a_stale_snapshot_and_folds_a_fresh_one() {
        let mock = mock_agent();
        mock.push_stream(StreamScript::Events(
            vec![
                stream_event("run-1", 0, "text_chunk", r#"{"text":"live-a"}"#),
                stream_event("run-1", 1, "text_chunk", r#"{"text":"live-b"}"#),
                // Stale snapshot (cursor below the stream position): skipped.
                snapshot_event(
                    "run-1",
                    0,
                    vec![projected("text_chunk", r#"{"text":"stale"}"#, 0)],
                ),
                // Fresh snapshot replaces the accumulated content wholesale.
                snapshot_event(
                    "run-1",
                    3,
                    vec![
                        projected("text_chunk", r#"{"text":"snap"}"#, 2),
                        projected("approval_request", r#"{"approval_request_id":"a1"}"#, 3),
                    ],
                ),
                // The stream continues after the snapshot cursor.
                stream_event(
                    "run-1",
                    4,
                    "approval_decision",
                    r#"{"approval_request_id":"a1"}"#,
                ),
                stream_event("run-1", 5, "agent_end", r#"{"reason":"complete"}"#),
            ],
            None,
        ));
        let response = collect_agent_response(None, "run-1", "sess-1", "thread-1")
            .await
            .expect("collect");
        assert_eq!(
            response.content, "snap",
            "snapshot replaced the live prefix"
        );
        assert!(response.complete);
    }

    #[tokio::test]
    async fn collect_propagates_a_projection_snapshot_error() {
        let mock = mock_agent();
        mock.push_stream(StreamScript::Events(
            vec![snapshot_event(
                "run-1",
                1,
                vec![
                    projected("text_chunk", r#"{"text":"prefix"}"#, 0),
                    projected("error", r#"{"error":"snapshot error"}"#, 1),
                ],
            )],
            None,
        ));
        let error = collect_agent_response(None, "run-1", "sess-1", "thread-1")
            .await
            .expect_err("error inside a projection snapshot");
        assert_eq!(crate::AppError::from(error).to_string(), "snapshot error");
    }

    #[tokio::test]
    async fn collect_gives_up_after_six_stream_breaks() {
        let mock = mock_agent();
        // Seven streams that each close without a terminal event (and without
        // ever resetting `reconnect_attempt` via a valid event): the seventh
        // break exhausts the budget and surfaces the reconnect error.
        for _ in 0..=STREAM_RECONNECT_ATTEMPTS {
            mock.push_stream(StreamScript::Events(vec![], None));
        }
        let error = collect_agent_response(None, "run-1", "sess-1", "thread-1")
            .await
            .expect_err("repeated stream breaks");
        let message = crate::AppError::from(error).to_string();
        assert!(
            message.contains("could not be resumed after 6 attempts"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn collect_persists_events_off_thread_for_local_runs() {
        let home = TestHome::new("stream-persist");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);

        mock.push_stream(StreamScript::Events(
            vec![
                stream_event(
                    &run.id,
                    0,
                    "tool_start",
                    r#"{"tool_id":"tc1","tool_name":"shell","tool_args":"{}"}"#,
                ),
                stream_event(&run.id, 1, "tool_end", r#"{"tool_id":"tc1","text":"ok"}"#),
                stream_event(&run.id, 2, "agent_end", r#"{"reason":"complete"}"#),
            ],
            None,
        ));
        let response = collect_agent_response(Some(&run.id), &run.id, "sess-1", &thread.id)
            .await
            .expect("collect");
        assert!(response.complete);
        // The off-thread persistence is awaited per event, so the tool
        // projection is warm by the time collect returns.
        let input = crate::store::get_tool_call_input(&run.id, "tc1").expect("projection");
        assert_eq!(input.as_deref(), Some("{}"));
    }
}
