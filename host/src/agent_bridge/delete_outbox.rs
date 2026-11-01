use super::*;

/// Deliver deletion intent in bounded batches over one shared client. Concurrent
/// GUI/background drains coalesce; partial or ambiguous replies keep tombstones.
pub async fn reconcile_delete_outbox() {
    use future_rpc::session_deletion::{DeleteSessionsResponse, MAX_DELETE_SESSIONS};
    static DRAIN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let Ok(_drain) = DRAIN.try_lock() else {
        return;
    };
    let Ok(pending) = crate::store::pending_agent_session_deletes_with_sources() else {
        return;
    };
    if pending.is_empty() {
        return;
    }
    let mut client = match connect_agent().await {
        Ok(client) => client,
        Err(error) => {
            let outcomes = pending
                .into_iter()
                .map(|(session_id, _)| (session_id, Some(error.to_string())))
                .collect::<Vec<_>>();
            let _ = crate::store::settle_agent_session_deletes(&outcomes);
            return;
        }
    };
    let identity = match session_presence_with_client(&mut client, Vec::new()).await {
        Ok(identity) => identity,
        Err(error) => {
            let outcomes = pending
                .into_iter()
                .map(|(session, _)| (session, Some(error.to_string())))
                .collect::<Vec<_>>();
            let _ = crate::store::settle_agent_session_deletes(&outcomes);
            return;
        }
    };
    let mut session_ids = Vec::new();
    let mut deferred = Vec::new();
    for (session, source) in pending {
        if source.as_deref() == Some(identity.database_id.as_str()) {
            session_ids.push(session);
        } else {
            let reason = if source.is_some() {
                "Deletion belongs to a different Agent database; intent retained."
            } else {
                "Deletion source is unverified; intent and Agent history retained."
            };
            deferred.push((session, Some(reason.to_owned())));
        }
    }
    if !deferred.is_empty() {
        let _ = crate::store::settle_agent_session_deletes(&deferred);
    }
    for batch in session_ids.chunks(MAX_DELETE_SESSIONS) {
        let reply = async {
            let response = client
                .execute_command(client::delete_sessions_command(batch.to_vec()))
                .await
                .map_err(|status| map_rpc_error("Agent delete delivery failed", status))?
                .into_inner();
            if !response.success {
                return Err(crate::Error::Message(response.error));
            }
            let payload = future_rpc::decode::response_data(&response);
            let decoded: DeleteSessionsResponse =
                serde_json::from_value(payload).map_err(|error| {
                    crate::Error::Message(format!("Invalid Agent deletion response: {error}"))
                })?;
            let expected: std::collections::HashSet<_> = batch.iter().map(String::as_str).collect();
            let actual: std::collections::HashSet<_> = decoded
                .results
                .iter()
                .map(|result| result.session_id.as_str())
                .collect();
            // Never clear intent on a missing, duplicate or unrelated outcome.
            if decoded.results.len() != batch.len() || actual != expected {
                return Err(crate::Error::Message(
                    "Incomplete Agent deletion response".to_owned(),
                ));
            }
            Ok(decoded
                .results
                .into_iter()
                .map(|result| {
                    let error = if result.deleted
                        || client::is_session_not_found_error(&result.error_code, &result.error)
                    {
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
/// alive. Startup-only delivery loses convergence whenever the Agent service starts
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

#[cfg(test)]
mod presence_failure_tests {
    #![allow(clippy::await_holding_lock)]
    use super::reconcile_delete_outbox;
    use crate::test_support::command_agent::{mock_agent_lock, script_mock_agent, MockScript};
    use std::collections::HashMap;

    /// The catalogue read is what decides *which* Agent owns a pending deletion.
    /// When it fails the intent is not dropped and not delivered blind: every
    /// pending session is settled with that reason, so the tombstone stays up and
    /// the retry carries the cause the user (and the log) can read.
    #[tokio::test]
    async fn a_catalogue_failure_is_recorded_against_every_pending_delete() {
        let _lock = mock_agent_lock();
        let _home = crate::auth_store::test_support::HomeGuard::new("outbox-presence-failure");
        crate::store::initialize_app_store().expect("init store");
        crate::test_support::command_agent::ensure_mock_agent();

        // A pending intent, inserted the way the GUI's delete does.
        let home = std::env::var("HOME").expect("test home");
        let db = std::path::Path::new(&home).join(".future/app/app.db");
        let conn = rusqlite::Connection::open(&db).expect("open app db");
        conn.execute(
            "INSERT INTO agent_delete_outbox(session_id, requested_at, attempts, last_error) \
             VALUES ('sess_orphan', 1, 0, NULL)",
            [],
        )
        .expect("queue a pending delete");
        drop(conn);

        script_mock_agent(MockScript {
            errors: HashMap::from([(
                "get_session_presence".to_string(),
                "the catalogue is unavailable".to_string(),
            )]),
            ..Default::default()
        });
        reconcile_delete_outbox().await;

        // Still pending (a failure must not settle the intent) and carrying the
        // Agent's own sentence, with one attempt recorded.
        assert_eq!(
            crate::store::pending_agent_session_deletes().expect("pending"),
            vec!["sess_orphan".to_string()],
            "a failed catalogue read must not drop the deletion intent"
        );
        let conn = rusqlite::Connection::open(&db).expect("open app db");
        let (attempts, last_error): (i64, Option<String>) = conn
            .query_row(
                "SELECT attempts, last_error FROM agent_delete_outbox WHERE session_id='sess_orphan'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("the row");
        assert_eq!(attempts, 1, "one delivery attempt was recorded");
        assert!(
            last_error
                .as_deref()
                .is_some_and(|text| text.contains("catalogue")),
            "{last_error:?}"
        );
        script_mock_agent(MockScript::default());
    }
}
