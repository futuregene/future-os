//! Process-wide session lifecycle observer.
//!
//! Subscribes to the Agent's global control-plane stream for
//! `session_created` announcements and imports sessions minted by other
//! clients (TUI/CLI/channels) within milliseconds — the discovery polls in
//! `observer.rs` remain the backstop for missed events and agents too old to
//! emit the event.
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

pub fn spawn_session_events_observer() {
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    tauri::async_runtime::spawn(run());
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
            event_types: vec!["session_created".to_string()],
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
        if event.r#type != "session_created" {
            continue;
        }
        handle_session_created(&event.data).await;
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

#[cfg(test)]
mod tests {
    use super::super::test_support::{mock_agent, stream_event, TestHome};
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
    }

    #[tokio::test]
    async fn handle_session_created_imports_external_sessions() {
        let _home = TestHome::new("session-events-import");
        let mock = mock_agent();

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

    #[tokio::test]
    async fn observe_once_consumes_the_stream_until_it_ends() {
        let _home = TestHome::new("session-events-stream");
        let mock = mock_agent();
        mock.push_typed_data("list_sessions", serde_json::json!({"sessions": []}));
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
