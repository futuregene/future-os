//! Process-wide provider/auth configuration observer.
//!
//! Unlike run streams this subscription is not attached to a chat. It bridges
//! the Agent's committed config revision to the WebView and remote clients so
//! every endpoint invalidates provider/model state from the same completion.

use crate::console;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::agent_client::connect_agent;

static STARTED: AtomicBool = AtomicBool::new(false);

pub fn spawn_provider_config_observer() {
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    crate::runtime::spawn(run());
}

async fn run() {
    let mut last_revision: Option<i64> = None;
    let mut backoff = Duration::from_millis(250);
    while !crate::request_scope::is_stopping() {
        let result = observe_once(&mut last_revision, &mut backoff).await;
        // Agent shutdown closes these streams before background tasks drain.
        // Exit normally instead of logging a disconnect or reattaching to it.
        if crate::request_scope::is_stopping() {
            break;
        }
        if let Err(error) = result {
            console::eprintln!("FutureOS provider config observer reconnecting: {error}");
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}

async fn observe_once(
    last_revision: &mut Option<i64>,
    backoff: &mut Duration,
) -> Result<(), crate::Error> {
    let mut client = connect_agent().await?;
    let instance = client.readiness().agent_instance_id.clone();
    let mut stream = client
        .stream_events(future_rpc::proto::StreamRequest {
            event_types: vec![
                "ping".to_string(),
                "provider_config_changed".to_string(),
                "fusion_settings_changed".to_string(),
            ],
            global_events: true,
            ..Default::default()
        })
        .await
        .map_err(|error| format!("global config stream failed: {error}"))?
        .into_inner();
    // A successful attachment proves the Agent is healthy again. If this
    // stream later closes, reconnect promptly instead of retaining the
    // maximum delay accumulated during an earlier outage.
    *backoff = Duration::from_millis(250);

    while let Some(event) = stream
        .message()
        .await
        .map_err(|error| format!("global config stream closed: {error}"))?
    {
        if event.r#type == "fusion_settings_changed" || event.r#type == "ping" {
            crate::agent_events::publish_invalidation("app_settings_changed");
            if event.r#type == "fusion_settings_changed" {
                continue;
            }
        }
        let mut payload = serde_json::from_str::<serde_json::Value>(&event.data)
            .unwrap_or_else(|_| serde_json::json!({}));
        let revision = payload
            .get("revision")
            .or_else(|| payload.get("configRevision"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        if event.r#type == "ping" {
            // Always invalidate on a newly attached stream. Agent revisions
            // are process-local and reset after restart, so equality alone
            // cannot prove the underlying config/runtime generation is the
            // same one we observed before the disconnect.
            payload = serde_json::json!({
                "revision": revision,
                "providerId": "*",
                "operation": "snapshot",
                "authChanged": true,
                "modelsChanged": true,
            });
        } else if last_revision.is_some_and(|seen| revision <= seen) {
            // subscribe-before-snapshot can legitimately queue revision N and
            // then emit a ping snapshot at N. The snapshot already invalidated
            // every consumer, so do not trigger a duplicate reload.
            continue;
        }
        if event.r#type != "provider_config_changed" && event.r#type != "ping" {
            continue;
        }
        *last_revision = Some(revision);
        if let Some(object) = payload.as_object_mut() {
            object.insert("agentInstanceId".into(), serde_json::json!(instance));
        }
        let data = payload.to_string();
        crate::events::notify("provider-config-changed", payload.clone());
        crate::agent_events::publish_event(
            "_global",
            "provider_config_changed",
            &data,
            "",
            revision,
            0,
            &format!("provider-config-{instance}-{revision}"),
            "",
            revision,
            -1,
        );
    }
    Err(crate::Error::Message(
        "global config stream ended".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{mock_agent, stream_event, StreamScript};
    use super::*;

    #[tokio::test]
    async fn shutdown_does_not_start_a_subscription() {
        let _home = crate::test_support::agent::TestHome::new("config_observer-already-stopping");
        let mock = mock_agent();
        let _stopping = crate::request_scope::close_admission_guard_for_test();
        tokio::time::timeout(Duration::from_millis(100), run())
            .await
            .expect("an observer must exit when the host is already stopping");
        assert!(mock.stream_requests().is_empty());
    }

    #[tokio::test]
    async fn shutdown_during_backoff_prevents_another_subscription() {
        let _home = crate::test_support::agent::TestHome::new("config_observer-stop-backoff");
        let mock = mock_agent();
        mock.push_plain_stream(crate::test_support::agent::StreamScript::AttachError(
            tonic::Code::Unavailable,
            "subscription ended",
        ));
        let observer = run();
        tokio::pin!(observer);
        // Drive the observer only until its first attach reaches the server.
        // Manual polls prevent a slow CI scheduler from running a retry first.
        tokio::time::timeout(Duration::from_secs(2), async {
            while mock.stream_requests().is_empty() {
                assert!(futures::poll!(&mut observer).is_pending());
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the first subscription must reach the mock");
        let _stopping = crate::request_scope::close_admission_guard_for_test();
        tokio::time::timeout(Duration::from_secs(1), &mut observer)
            .await
            .expect("shutdown must end the retry loop");
        assert_eq!(mock.stream_requests().len(), 1);
    }

    #[tokio::test]
    async fn observe_once_ping_snapshot_invalidates_and_ends() {
        let mock = mock_agent();
        mock.push_plain_stream(StreamScript::Events(
            vec![stream_event("", 0, "ping", r#"{"revision":7}"#)],
            None,
        ));
        let mut last_revision: Option<i64> = None;
        let mut backoff = Duration::from_secs(5);
        let result = observe_once(&mut last_revision, &mut backoff).await;
        assert!(result.is_err(), "stream ends → Err");
        assert_eq!(last_revision, Some(7));
        assert_eq!(backoff, Duration::from_millis(250));
    }

    #[tokio::test]
    async fn observe_once_tracks_revisions_and_skips_stale_and_unknown() {
        let mock = mock_agent();
        mock.push_plain_stream(StreamScript::Events(
            vec![
                stream_event("", 0, "provider_config_changed", r#"{"configRevision":3}"#),
                stream_event("", 1, "provider_config_changed", r#"{"revision":3}"#),
                stream_event("", 2, "provider_config_changed", r#"{"revision":5}"#),
                stream_event("", 3, "thread_run", r#"{"revision":9}"#),
                stream_event("", 4, "provider_config_changed", "not-json"),
            ],
            None,
        ));
        let mut last_revision: Option<i64> = None;
        let mut backoff = Duration::from_millis(250);
        let result = observe_once(&mut last_revision, &mut backoff).await;
        assert!(result.is_err());
        assert_eq!(last_revision, Some(5));
    }

    #[tokio::test]
    async fn observe_once_surfaces_attach_failure() {
        let mock = mock_agent();
        mock.push_plain_stream(StreamScript::AttachError(
            tonic::Code::Unavailable,
            "stream attach failed",
        ));
        let mut last_revision: Option<i64> = None;
        let mut backoff = Duration::from_millis(250);
        let error = observe_once(&mut last_revision, &mut backoff)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("global config stream failed"));
    }

    #[tokio::test]
    async fn observe_once_surfaces_stream_close_error() {
        let mock = mock_agent();
        mock.push_plain_stream(StreamScript::Events(
            vec![stream_event("", 0, "ping", r#"{"revision":1}"#)],
            Some((tonic::Code::Unavailable, "stream dropped")),
        ));
        let mut last_revision: Option<i64> = None;
        let mut backoff = Duration::from_millis(250);
        let error = observe_once(&mut last_revision, &mut backoff)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("global config stream closed"));
    }

    #[tokio::test]
    async fn observe_once_connect_failure_propagates() {
        let _mock = mock_agent();
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock sets the endpoint");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        let mut last_revision: Option<i64> = None;
        let mut backoff = Duration::from_millis(250);
        let result = observe_once(&mut last_revision, &mut backoff).await;
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn run_loop_reconnects_and_backs_off() {
        let _mock = mock_agent();
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock sets the endpoint");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        let task = tokio::spawn(run());
        // One full iteration: connect fails → eprintln → 250ms backoff sleep →
        // backoff doubles. Abort before the second iteration's sleep completes.
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
        task.abort();
        let _ = task.await;
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
    }

    #[test]
    fn spawn_provider_config_observer_runs_once() {
        let _mock = mock_agent();
        spawn_provider_config_observer();
        spawn_provider_config_observer();
    }
}
