//! Process-wide session lifecycle observer.
//!
//! Subscribes to the Agent's global control-plane stream for
//! `session_created` / `session_deleted` announcements, importing sessions
//! minted by other clients (TUI/CLI/channels) and dropping sessions they
//! removed within milliseconds — the discovery polls in `observer.rs` remain
//! the backstop for missed events and agents too old to emit the event.
//!
//! Sessions created by this Desktop installation (`creatorId == deviceId`) are
//! skipped: the GUI manages those threads and their session links itself, and
//! importing a stub here would race the prompt pipeline's own thread-row
//! update. Another Desktop has the same `createdBy` category but a different
//! creator id, so its sessions are imported normally.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::connect_agent;

static STARTED: AtomicBool = AtomicBool::new(false);

/// Re-arm the start guard so the "start it once" arm is reachable regardless of
/// what earlier tests in this process already started.
#[cfg(test)]
fn reset_started_for_test() {
    STARTED.store(false, Ordering::Release);
}

pub fn spawn_session_events_observer() {
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    crate::runtime::spawn(run());
}

async fn run() {
    let mut backoff = Duration::from_millis(250);
    loop {
        let result = observe_once(&mut backoff).await;
        if let Err(error) = result {
            eprintln!("FutureOS session events observer reconnecting: {error}");
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}

async fn observe_once(backoff: &mut Duration) -> Result<(), crate::AppError> {
    let mut client = connect_agent().await?;
    let mut stream = client
        .stream_events(crate::agent_proto::StreamRequest {
            event_types: vec!["session_created".to_string(), "session_deleted".to_string()],
            global_events: true,
            ..Default::default()
        })
        .await
        .map_err(|error| format!("global session events stream failed: {error}"))?
        .into_inner();
    // The global stream is an in-memory notification channel, not a durable
    // log. Once the subscription exists, reconcile the Agent's authoritative
    // session list before consuming queued notifications. This closes startup
    // and reconnect gaps without a one-second streaming-session poll.
    super::import::import_missing_sessions().await;
    // A successful attachment proves the Agent is healthy again; if this
    // stream later closes, reconnect promptly instead of retaining the
    // maximum delay accumulated during an earlier outage.
    *backoff = Duration::from_millis(250);

    while let Some(event) = stream
        .message()
        .await
        .map_err(|error| format!("global session events stream closed: {error}"))?
    {
        match event.r#type.as_str() {
            "session_created" => handle_session_created(&event.data).await,
            "session_deleted" => handle_session_deleted(&event.data),
            _ => {}
        }
    }
    Err(crate::AppError::Message(
        "global session events stream ended".to_string(),
    ))
}

/// Parse one `session_created` payload and import the session when it was
/// created by another client. Failures are logged, never propagated — one bad
/// event must not tear down the whole stream.
async fn handle_session_created(data: &str) {
    let payload: serde_json::Value = match serde_json::from_str(data) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("FutureOS: malformed session_created payload: {error}");
            return;
        }
    };
    let Some(session_id) = payload
        .get("sessionId")
        .and_then(|value| value.as_str())
        .filter(|id| !id.is_empty())
    else {
        eprintln!("FutureOS: session_created payload omitted sessionId");
        return;
    };
    let created_by = payload
        .get("createdBy")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let creator_id = payload
        .get("creatorId")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    // Compatibility with a pre-creatorId Agent: conservatively preserve the
    // old self-import guard. New Agents never take this branch, so two current
    // Desktop installations with distinct device ids still discover each
    // other correctly.
    if creator_id.is_empty() && created_by == "desktop" {
        return;
    }
    if !creator_id.is_empty() {
        match crate::device_identity::device_id() {
            Ok(device_id) if creator_id == device_id => {
                return; // This installation's prompt/fork path owns the local row.
            }
            Ok(_) => {}
            Err(error) => {
                // Importing while identity is unknown could steal the unique
                // binding from this installation's in-flight create. The
                // reconnect/periodic reconciliation will retry after the
                // original thread has had a chance to persist its binding.
                eprintln!(
                    "FutureOS deferred session {session_id} while device identity is unavailable: {error}"
                );
                return;
            }
        }
    }
    match super::observer::reconcile_discovered_session(session_id).await {
        Ok(true) => {
            eprintln!(
                "FutureOS imported session {session_id} announced by client {created_by:?} creator {creator_id:?}"
            );
        }
        Ok(false) => {}
        Err(error) => {
            eprintln!("FutureOS could not import announced session {session_id}: {error}")
        }
    }
}

/// Parse one `session_deleted` payload and drop Desktop's mirror of that
/// session. Deliberately unfiltered by creator: the Agent refuses to delete a
/// session while a run is draining, so a deletion that did happen is
/// authoritative for every client, including this installation's own (whose
/// row is already gone → no-op). Failures are logged, never propagated — one
/// bad event must not tear down the whole stream.
fn handle_session_deleted(data: &str) {
    let payload: serde_json::Value = match serde_json::from_str(data) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("FutureOS: malformed session_deleted payload: {error}");
            return;
        }
    };
    let Some(session_id) = payload
        .get("sessionId")
        .and_then(|value| value.as_str())
        .filter(|id| !id.is_empty())
    else {
        eprintln!("FutureOS: session_deleted payload omitted sessionId");
        return;
    };
    match super::observer::reconcile_deleted_session(session_id) {
        Ok(true) => {
            eprintln!("FutureOS: removed session {session_id} deleted outside the GUI");
            crate::emit_threads_updated();
        }
        Ok(false) => {}
        Err(error) => {
            eprintln!("FutureOS: could not remove deleted session {session_id}: {error}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{
        break_home, mock_agent, restore_home, seed_thread, seed_workspace, stream_event, Reply,
        TestHome,
    };
    use super::*;

    #[tokio::test]
    async fn handle_session_created_skips_own_creator_and_malformed_payloads() {
        let _home = TestHome::new("session-events-skip");
        let _mock = mock_agent();
        let creator_id = crate::device_identity::device_id().expect("device id");

        // This installation's sessions are its own bookkeeping — no import,
        // and no get_state RPC (an unserved command would fail the test).
        handle_session_created(
            &serde_json::json!({
                "sessionId": "s-mine",
                "createdBy": "desktop",
                "creatorId": creator_id,
                "cwd": "/ws"
            })
            .to_string(),
        )
        .await;
        assert!(crate::store::find_thread_by_agent_session("s-mine")
            .expect("find")
            .is_none());

        // Malformed payloads and missing session ids are logged and skipped.
        handle_session_created("not json").await;
        handle_session_created(r#"{"createdBy":"tui"}"#).await;

        // A pre-creatorId Agent announces this installation's own session with
        // only `createdBy`: the legacy self-import guard still skips it, so no
        // stub is minted for a conversation the GUI itself is creating.
        handle_session_created(r#"{"sessionId":"s-legacy","createdBy":"desktop"}"#).await;
        assert!(crate::store::find_thread_by_agent_session("s-legacy")
            .expect("find")
            .is_none());
    }

    /// While device identity cannot be resolved, an announcement from another
    /// client is **deferred**, not imported: the local row for a session this
    /// installation is itself creating must keep its binding, and the periodic
    /// reconciliation retries once identity exists.
    #[tokio::test]
    async fn an_announcement_is_deferred_while_device_identity_is_unavailable() {
        let _home = TestHome::new("session-events-no-identity");
        let _mock = mock_agent();
        let previous = break_home();

        handle_session_created(
            r#"{"sessionId":"s-other","createdBy":"tui","creatorId":"desktop_other"}"#,
        )
        .await;

        restore_home(previous);
        assert!(
            crate::store::find_thread_by_agent_session("s-other")
                .expect("find")
                .is_none(),
            "a deferred session must not be mirrored"
        );
    }

    /// A deletion announcement whose local reconciliation fails is reported, not
    /// propagated: the stream stays up so a later event can still be applied.
    #[tokio::test]
    async fn a_deletion_that_cannot_be_reconciled_is_logged() {
        let home = TestHome::new("session-events-delete-error");
        let _mock = mock_agent();
        let conn =
            rusqlite::Connection::open(home.path().join(".future/app/app.db")).expect("open db");
        conn.execute_batch("DROP TABLE threads;")
            .expect("drop threads");
        drop(conn);

        // The lookup itself now fails, which is the arm under test.
        handle_session_deleted(r#"{"sessionId":"s-gone"}"#);
    }

    /// A failed import is logged and dropped, never propagated: one hostile
    /// event must not tear down the process-wide stream.
    #[tokio::test]
    async fn a_failed_import_is_logged_and_the_announcement_is_dropped() {
        let _home = TestHome::new("session-events-import-failure");
        let mock = mock_agent();
        mock.push(
            "get_session_entries",
            Reply::Reject("agent refused the page".into()),
        );

        handle_session_created(r#"{"sessionId":"s-refused","createdBy":"tui"}"#).await;
        assert!(crate::store::find_thread_by_agent_session("s-refused")
            .expect("find")
            .is_none());
    }

    /// The start guard is what keeps one Agent stream per process. Started from
    /// a fixture because the guard is process-global: two calls, one loop.
    #[tokio::test]
    async fn spawning_the_observer_twice_starts_it_once() {
        let _home = TestHome::new("session-events-single-start");
        let _mock = mock_agent();
        reset_started_for_test();

        spawn_session_events_observer();
        spawn_session_events_observer();

        // Let the spawned loop reach its first attachment (and the mock answer
        // it) so the test proves the loop ran, not merely that it was queued.
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    /// `run` is the process-lifetime loop: it never returns, so what the test
    /// can prove is that a failed attachment is logged and retried on a longer
    /// backoff instead of ending the observer. The timeout is the assertion.
    #[tokio::test]
    async fn the_observer_loop_survives_a_failed_attachment() {
        let _home = TestHome::new("session-events-loop-error");
        let previous = std::env::var("FUTURE_AGENT_GRPC_ADDR").ok();
        // A malformed endpoint, the documented way to make `connect_agent`
        // fail without touching the real Agent.
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");

        let _ = tokio::time::timeout(Duration::from_millis(600), run()).await;

        match previous {
            Some(value) => std::env::set_var("FUTURE_AGENT_GRPC_ADDR", value),
            None => std::env::remove_var("FUTURE_AGENT_GRPC_ADDR"),
        }
    }

    #[tokio::test]
    async fn handle_session_created_imports_external_sessions() {
        let _home = TestHome::new("session-events-import");
        let mock = mock_agent();

        // The announced session holds a message: a session still empty at
        // announcement time is deliberately left to the discovery pass.
        mock.push_data(
            "get_session_entries",
            serde_json::json!({"entries": [
                {"id":"u1","role":"user","kind":"user","blocks":[],"createdAtMs":1000}
            ]}),
        );
        mock.push_data(
            "get_state",
            serde_json::json!({
                "sessionId": "s-tui",
                "sessionName": "From TUI",
                "cwd": "",
                "model": "future/k3"
            }),
        );
        handle_session_created(
            r#"{"sessionId":"s-tui","createdBy":"desktop","creatorId":"desktop_other","cwd":"/tmp"}"#,
        )
        .await;
        let thread = crate::store::find_thread_by_agent_session("s-tui")
            .expect("find")
            .expect("imported stub");
        assert_eq!(thread.title, "From TUI");
        assert!(super::super::observer::OBSERVERS
            .lock()
            .unwrap()
            .contains_key("s-tui"));

        // A repeat announcement is a no-op (import_discovered_session is
        // idempotent), and no additional RPC is scripted.
        handle_session_created(
            r#"{"sessionId":"s-tui","createdBy":"desktop","creatorId":"desktop_other","cwd":"/tmp"}"#,
        )
        .await;
    }

    /// An announcement that arrives while this installation's device identity is
    /// unreadable is deferred, not imported: importing blind could steal the
    /// unique session binding from this installation's own in-flight create. The
    /// periodic reconciliation retries it, so nothing is lost by waiting.
    #[tokio::test]
    async fn an_announcement_while_identity_is_unavailable_is_deferred() {
        let _home = TestHome::new("session-events-identity-unavailable");
        let _mock = mock_agent();
        let previous = break_home();

        handle_session_created(
            r#"{"sessionId":"s-deferred","createdBy":"desktop","creatorId":"another-installation"}"#,
        )
        .await;

        restore_home(previous);
        assert!(
            crate::store::find_thread_by_agent_session("s-deferred")
                .expect("find")
                .is_none(),
            "a deferred announcement must not have minted a stub"
        );
    }

    /// The delete side is the same contract as the create side: a reconcile that
    /// cannot reach the store is logged and dropped, and the conversation mirror
    /// it could not inspect is left exactly as it was.
    #[tokio::test]
    async fn a_deletion_that_cannot_be_reconciled_leaves_the_mirror_alone() {
        let home = TestHome::new("session-events-delete-failure");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("s-gone"));
        let previous = break_home();

        handle_session_deleted(r#"{"sessionId":"s-gone"}"#);

        restore_home(previous);
        assert!(
            crate::store::get_thread(&thread.id)
                .expect("get_thread")
                .is_some(),
            "a deletion that could not be reconciled leaves the mirror alone"
        );
    }

    #[tokio::test]
    async fn handle_session_deleted_removes_the_local_conversation() {
        let home = TestHome::new("session-events-deleted");
        let _mock = mock_agent();
        let workspace = super::super::test_support::seed_workspace(home.path(), "ws-deleted");
        let thread = super::super::test_support::seed_thread(&workspace.id, Some("s-gone"));
        let run = super::super::test_support::seed_run(&thread.id);
        assert!(crate::store::get_thread(&thread.id).expect("get").is_some());

        // Malformed / half-formed announcements are logged and ignored.
        handle_session_deleted("not json");
        handle_session_deleted(r#"{"createdBy":"tui"}"#);
        assert!(crate::store::get_thread(&thread.id).expect("get").is_some());

        handle_session_deleted(r#"{"sessionId":"s-gone"}"#);
        assert!(crate::store::get_thread(&thread.id).expect("get").is_none());
        assert!(
            crate::store::is_agent_session_tombstoned("s-gone").expect("tombstone"),
            "the deleted session is fenced against re-import"
        );
        assert!(
            crate::store::get_run(&run.id).expect("get run").is_none(),
            "the conversation's runs go with it"
        );

        // A repeat announcement (or one for a session this GUI never had) is a
        // no-op.
        handle_session_deleted(r#"{"sessionId":"s-gone"}"#);
        handle_session_deleted(r#"{"sessionId":"s-never-seen"}"#);
    }

    #[tokio::test]
    async fn observe_once_applies_a_deleted_announcement() {
        let _home = TestHome::new("session-events-stream-deleted");
        let mock = mock_agent();
        mock.push_typed_data("list_sessions", serde_json::json!({"sessions": []}));
        mock.push_data(
            "get_session_entries",
            serde_json::json!({"entries": [
                {"id":"u1","role":"user","kind":"user","blocks":[],"createdAtMs":1000}
            ]}),
        );
        mock.push_data(
            "get_state",
            serde_json::json!({
                "sessionId": "s-live",
                "sessionName": "Live",
                "cwd": "",
                "model": "future/k3"
            }),
        );
        mock.push_plain_stream(super::super::test_support::StreamScript::Events(
            vec![
                stream_event(
                    "",
                    0,
                    "session_created",
                    r#"{"sessionId":"s-live","createdBy":"tui"}"#,
                ),
                stream_event("", 1, "session_deleted", r#"{"sessionId":"s-live"}"#),
            ],
            None,
        ));
        let mut backoff = Duration::from_secs(5);
        let result = observe_once(&mut backoff).await;
        assert!(result.is_err(), "stream ends → Err");
        assert!(
            crate::store::find_thread_by_agent_session("s-live")
                .expect("find")
                .is_none(),
            "a session announced and then deleted outside the GUI leaves no thread"
        );
    }

    #[tokio::test]
    async fn observe_once_consumes_the_stream_until_it_ends() {
        let _home = TestHome::new("session-events-stream");
        let mock = mock_agent();
        mock.push_typed_data("list_sessions", serde_json::json!({"sessions": []}));
        mock.push_data(
            "get_session_entries",
            serde_json::json!({"entries": [
                {"id":"u1","role":"user","kind":"user","blocks":[],"createdAtMs":1000}
            ]}),
        );
        mock.push_data(
            "get_state",
            serde_json::json!({
                "sessionId": "s-live",
                "sessionName": "Live",
                "cwd": "",
                "model": "future/k3"
            }),
        );
        mock.push_plain_stream(super::super::test_support::StreamScript::Events(
            vec![
                stream_event(
                    "",
                    0,
                    "session_created",
                    r#"{"sessionId":"s-live","createdBy":"tui"}"#,
                ),
                // Unknown types are ignored; the stream must survive them.
                stream_event("", 1, "ping", r#"{"configRevision":3}"#),
            ],
            None,
        ));
        let mut backoff = Duration::from_secs(5);
        let result = observe_once(&mut backoff).await;
        assert!(result.is_err(), "stream ends → Err");
        assert_eq!(backoff, Duration::from_millis(250));
        assert!(crate::store::find_thread_by_agent_session("s-live")
            .expect("find")
            .is_some());
    }

    #[tokio::test]
    async fn observe_once_surfaces_attach_failure() {
        let _mock = mock_agent();
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock sets the endpoint");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        let mut backoff = Duration::from_millis(250);
        let result = observe_once(&mut backoff).await;
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        assert!(result.is_err());
    }
}
