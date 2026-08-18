//! Process-wide provider/auth configuration observer.
//!
//! Unlike run streams this subscription is not attached to a chat. It bridges
//! the Agent's committed config revision to the WebView and remote clients so
//! every endpoint invalidates provider/model state from the same completion.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::Emitter;

use super::connect_agent;

static STARTED: AtomicBool = AtomicBool::new(false);

pub fn spawn_provider_config_observer() {
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    tauri::async_runtime::spawn(run());
}

async fn run() {
    let mut last_revision: Option<i64> = None;
    let mut backoff = Duration::from_millis(250);
    loop {
        let result = observe_once(&mut last_revision, &mut backoff).await;
        if let Err(error) = result {
            eprintln!("FutureOS provider config observer reconnecting: {error}");
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}

async fn observe_once(
    last_revision: &mut Option<i64>,
    backoff: &mut Duration,
) -> Result<(), crate::AppError> {
    let mut client = connect_agent().await?;
    let mut stream = client
        .stream_events(crate::agent_proto::StreamRequest {
            event_types: vec!["ping".to_string(), "provider_config_changed".to_string()],
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
        let data = payload.to_string();
        if let Some(handle) = crate::APP_HANDLE.get() {
            let _ = handle.emit("provider-config-changed", &payload);
        }
        crate::remote::publish_event(
            "_global",
            "provider_config_changed",
            &data,
            "",
            revision,
            0,
            &format!("provider-config-{revision}"),
            "",
            revision,
            -1,
        );
    }
    Err(crate::AppError::Message(
        "global config stream ended".to_string(),
    ))
}
