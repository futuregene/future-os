use super::*;

/// Deliver deletion intent in bounded batches over one shared client. Concurrent
/// GUI/background drains coalesce; partial or ambiguous replies keep tombstones.
pub async fn reconcile_delete_outbox() {
    use future_rpc::session_deletion::{DeleteSessionsResponse, MAX_DELETE_SESSIONS};
    static DRAIN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let Ok(_drain) = DRAIN.try_lock() else {
        return;
    };
    let Ok(session_ids) = crate::store::pending_agent_session_deletes() else {
        return;
    };
    if session_ids.is_empty() {
        return;
    }
    let mut client = match connect_agent().await {
        Ok(client) => client,
        Err(error) => {
            let outcomes = session_ids
                .into_iter()
                .map(|session_id| (session_id, Some(error.to_string())))
                .collect::<Vec<_>>();
            let _ = crate::store::settle_agent_session_deletes(&outcomes);
            return;
        }
    };
    for batch in session_ids.chunks(MAX_DELETE_SESSIONS) {
        let reply = async {
            let response = client
                .execute_command(client::delete_sessions_command(batch.to_vec()))
                .await
                .map_err(|status| map_rpc_error("Agent delete delivery failed", status))?
                .into_inner();
            if !response.success {
                return Err(crate::AppError::Message(response.error));
            }
            let payload = future_rpc::decode::response_data(&response);
            let decoded: DeleteSessionsResponse =
                serde_json::from_value(payload).map_err(|error| {
                    crate::AppError::Message(format!("Invalid Agent deletion response: {error}"))
                })?;
            let expected: std::collections::HashSet<_> = batch.iter().map(String::as_str).collect();
            let actual: std::collections::HashSet<_> = decoded
                .results
                .iter()
                .map(|result| result.session_id.as_str())
                .collect();
            // Never clear intent on a missing, duplicate or unrelated outcome.
            if decoded.results.len() != batch.len() || actual != expected {
                return Err(crate::AppError::Message(
                    "Incomplete Agent deletion response".to_owned(),
                ));
            }
            Ok(decoded
                .results
                .into_iter()
                .map(|result| {
                    let error = if result.deleted {
                        None
                    } else {
                        Some(if result.error.is_empty() {
                            "Agent session deletion failed".to_owned()
                        } else {
                            result.error
                        })
                    };
                    (result.session_id, error)
                })
                .collect::<Vec<_>>())
        }
        .await;
        let outcomes = match reply {
            Ok(outcomes) => outcomes,
            Err(error) => batch
                .iter()
                .map(|session_id| (session_id.clone(), Some(error.to_string())))
                .collect(),
        };
        let _ = crate::store::settle_agent_session_deletes(&outcomes);
    }
}

/// Keep retrying durable deletion intent for as long as this GUI process is
/// alive. Startup-only delivery loses convergence whenever the sidecar starts
/// late or an Agent refuses deletion while a run is draining.
pub fn spawn_delete_outbox_worker() {
    crate::runtime::spawn(async {
        loop {
            reconcile_delete_outbox().await;
            #[cfg(test)]
            if TEST_OUTBOX_STOP.swap(false, std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            tokio::time::sleep(delete_outbox_interval()).await;
        }
    });
}

#[cfg(test)]
pub(super) static TEST_OUTBOX_STOP: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Outbox retry interval; tests shrink it via env (a cfg(test)-only seam).
pub(super) fn delete_outbox_interval() -> std::time::Duration {
    #[cfg(test)]
    if let Some(ms) = std::env::var("FUTURE_TEST_OUTBOX_INTERVAL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        return std::time::Duration::from_millis(ms);
    }
    std::time::Duration::from_secs(5)
}
