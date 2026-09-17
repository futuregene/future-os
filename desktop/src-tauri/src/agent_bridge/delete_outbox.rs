use super::*;

/// Deliver locally tombstoned session deletions after the Agent becomes
/// reachable. A delete is idempotent; `session not found` is success too.
pub async fn reconcile_delete_outbox() {
    let Ok(session_ids) = crate::store::pending_agent_session_deletes() else {
        return;
    };
    for session_id in session_ids {
        let result = async {
            let mut client = connect_agent().await?;
            let response = client
                .execute_command(delete_session_command(session_id.clone()))
                .await
                .map_err(|status| map_rpc_error("Agent delete delivery failed", status))?;
            let response = response.into_inner();
            if response.success || response.error.contains("session not found") {
                Ok(())
            } else {
                Err(crate::AppError::Message(response.error))
            }
        }
        .await;
        match result {
            Ok(()) => {
                let _ = crate::store::acknowledge_agent_session_delete(&session_id);
            }
            Err(error) => {
                let _ = crate::store::note_agent_session_delete_failure(
                    &session_id,
                    &error.to_string(),
                );
            }
        }
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
