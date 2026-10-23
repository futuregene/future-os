//! Agent-authoritative cleanup coordination. SQL and CAS writes live in host.
use std::collections::HashSet;

/// Inspect missing Agent bindings without deleting conversation history.
/// An incomplete restore or a different Agent database is indistinguishable
/// from an offline deletion. Only an explicit deletion event or user action
/// authorizes removing the local conversation and its recovery copies.
pub async fn reconcile_orphan_sessions() -> Result<usize, crate::Error> {
    // The bundled agent may still be booting when this pass runs; retry
    // briefly before concluding it is down.
    let mut live_sessions: Option<HashSet<String>> = None;
    for _ in 0..3 {
        match crate::agent_bridge::list_agent_session_ids().await {
            Ok(ids) => {
                live_sessions = Some(ids);
                break;
            }
            Err(crate::Error::AgentUnavailable(_)) => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Err(error) => {
                eprintln!("FutureOS: orphan-session reconcile skipped: {error}");
                return Ok(0);
            }
        }
    }
    let Some(live_sessions) = live_sessions else {
        eprintln!("FutureOS: orphan-session reconcile skipped: agent unreachable");
        return Ok(0);
    };

    let candidates = super::session_reconcile_candidates(&live_sessions)?;
    let orphans = candidates.missing_sources;
    let bindings = candidates.bindings;
    let mut removed = 0;
    for batch in bindings.chunks(future_rpc::session_deletion::MAX_DELETE_SESSIONS) {
        let ids = batch.iter().map(|(_, session)| session.clone()).collect();
        let presence = match crate::agent_bridge::session_presence(ids).await {
            Ok(presence) => presence,
            Err(error) => {
                eprintln!("FutureOS: durable session reconcile deferred: {error}");
                break;
            }
        };
        for (thread, session) in batch {
            let status = presence
                .sessions
                .iter()
                .find(|item| item.session_id == *session)
                .map(|item| item.status.as_str());
            match status {
                Some("present") => {
                    if let Err(error) = super::record_thread_source(
                        thread,
                        session,
                        &presence.database_id,
                        &presence.restore_generation,
                    ) {
                        eprintln!("FutureOS: conversation source binding retained: {error}");
                    }
                }
                Some("deleted")
                    if super::thread_source(thread)?
                        .is_some_and(|source| source.0 == presence.database_id) =>
                {
                    removed +=
                        usize::from(crate::agent_bridge::reconcile_deleted_session(session)?);
                }
                _ => {}
            }
        }
    }
    if orphans != 0 && removed == 0 {
        eprintln!("FutureOS: {} conversation source(s) are missing; preserving local history for recovery", orphans);
    }
    Ok(removed)
}

/// Release deletion intents the GUI can never deliver, so a conversation that
/// still exists in the Agent database stops being fenced out of every import.
///
/// A user delete queues an intent and deletes the local conversation; the fence
/// then blocks import until the Agent acknowledges. That is correct while the
/// intent can still be delivered. When no ownership evidence survives (see
/// [`super::undeliverable_agent_session_deletes`]) the delivery pass refuses the
/// intent forever, so the fence becomes a permanent leak: the session stays in
/// the Agent and no client-mirroring pass may show it again.
///
/// Only intents refused at least once and holding no evidence are candidates,
/// and only those whose session the Agent still reports *present* here are
/// released. An intent with evidence keeps its fence — that delivery can still
/// happen, and it belongs to a user delete this pass must not undo. An
/// unreachable Agent, or one that no longer holds the session, releases nothing.
pub async fn reconcile_undeliverable_delete_intents() -> Result<usize, crate::Error> {
    let candidates = super::undeliverable_agent_session_deletes()?;
    let mut released = 0;
    for batch in candidates.chunks(future_rpc::session_deletion::MAX_DELETE_SESSIONS) {
        let presence = match crate::agent_bridge::session_presence(batch.to_vec()).await {
            Ok(presence) => presence,
            Err(error) => {
                // Nothing in this pass can be judged without the Agent's own
                // catalogue; the next one re-runs it.
                eprintln!("FutureOS: undeliverable delete reconcile deferred: {error}");
                break;
            }
        };
        let present: Vec<String> = presence
            .sessions
            .iter()
            .filter(|item| item.status == "present")
            .map(|item| item.session_id.clone())
            .collect();
        if present.is_empty() {
            continue;
        }
        released += super::release_undeliverable_agent_session_deletes(&present)?;
    }
    if released > 0 {
        eprintln!(
            "FutureOS: released {released} undeliverable deletion intent(s); the next import may mirror those conversations again"
        );
    }
    Ok(released)
}

/// Sweep conversations the GUI mirrors for a session that has no message at all.
///
/// Sessions are announced when they are *created*, so builds that imported every
/// announcement ended up with a row for sessions nobody had prompted yet: an
/// empty conversation in every list, plus the throw-away temp workspace the
/// mirror allocated for it. Import no longer does that; this heals the rows that
/// are already stored, so an existing list converges without hand deletion.
///
/// Only threads that never produced a run are candidates — a thread with runs
/// always had something to show — and only when the Agent itself answers that
/// the session holds no message. An unreachable Agent, an unreadable journal, or
/// a partially read one all leave the row alone (see
/// `session_has_messages`), and a thread created within
/// the existing grace interval is skipped so a conversation being started
/// right now is never caught mid-bind. Returns the number of threads deleted.
pub async fn reconcile_empty_conversations() -> Result<usize, crate::Error> {
    let candidates = super::empty_conversation_candidates()?;
    let mut removed = 0;
    for (thread_id, session_id) in candidates {
        if super::has_legacy_history(&thread_id)? {
            continue;
        }
        match crate::agent_bridge::session_has_messages(&session_id).await {
            Ok(true) => {}
            Ok(false) => {
                // The Agent keeps the session: this drops only the local
                // projection, without queuing an Agent delete or tombstoning
                // the id the next discovery pass has to re-import.
                crate::store::forget_thread_mirror(&thread_id)?;
                removed += 1;
            }
            Err(crate::Error::AgentUnavailable(error)) => {
                // Nothing left in this pass can be judged while the Agent is
                // away; the next one re-runs it.
                eprintln!("FutureOS: empty-conversation reconcile stopped: {error}");
                break;
            }
            Err(error) => {
                eprintln!("FutureOS: empty-conversation probe failed for {session_id}: {error}")
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use crate::store::db::test_support::guarded_conn;
    use crate::store::test_support::{seed_full, seed_message_less_threads};
    use crate::store::*;
    use crate::test_support::agent::mock_agent;
    #[tokio::test]
    async fn orphaned_runs_remain_discoverable_and_can_be_terminated_without_guessing_a_session() {
        let (_home, conn) = guarded_conn("orphan_run_abort");
        seed_full(&conn);
        conn.execute_batch("PRAGMA foreign_keys=OFF; DELETE FROM threads WHERE id='t_blank'; PRAGMA foreign_keys=ON;").unwrap();
        drop(conn);
        assert!(list_active_runs()
            .unwrap()
            .iter()
            .any(|run| run.run_id == "r_run"));
        let cancelled = crate::agent_bridge::abort_run("t_blank".into(), "r_run".into())
            .await
            .unwrap();
        assert_eq!(cancelled.status, "cancelled");
        assert!(!list_active_runs()
            .unwrap()
            .iter()
            .any(|run| run.run_id == "r_run"));
    }

    #[tokio::test]
    async fn reconcile_preserves_history_when_an_agent_database_is_missing() {
        let (_home, conn) = guarded_conn("reconcile_ok");
        seed_full(&conn);
        drop(conn);
        let mock = mock_agent();
        mock.default_data("list_session_ids", serde_json::json!({"ids":["sess_live"]}));

        // Absence from the Agent catalogue can mean a partial restore; even
        // completed conversations and their run records must survive.
        let deleted = reconcile_orphan_sessions().await.expect("reconcile");
        assert_eq!(deleted, 0);
        assert!(crate::store::get_thread("t_none").expect("get").is_some());
        assert!(crate::store::get_thread("t_sess").expect("get").is_some());
    }

    #[tokio::test]
    async fn reconcile_skips_when_enumeration_fails() {
        let (_home, conn) = guarded_conn("reconcile_fail");
        seed_full(&conn);
        drop(conn);
        let mock = mock_agent();
        mock.reject_default("list_session_ids", "mock enumeration failure");

        let deleted = reconcile_orphan_sessions().await.expect("reconcile");
        assert_eq!(deleted, 0, "a failed enumeration deletes nothing");
        assert!(crate::store::get_thread("t_none").expect("get").is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reconcile_skips_when_the_agent_is_unreachable() {
        let (_home, conn) = guarded_conn("reconcile_down");
        seed_full(&conn);
        drop(conn);
        let mock = mock_agent();
        // Down mode: every RPC answers Unavailable, so connect (or the first
        // command on the latched channel) surfaces AgentUnavailable and the
        // pass retries, then skips.
        mock.set_down(true);

        let deleted = reconcile_orphan_sessions().await.expect("reconcile");
        assert_eq!(deleted, 0, "an unreachable agent deletes nothing");
        assert!(crate::store::get_thread("t_none").expect("get").is_some());
    }

    #[tokio::test]
    async fn reconcile_releases_a_deletion_intent_the_gui_can_never_deliver() {
        let (_home, conn) = guarded_conn("reconcile_undeliverable");
        conn.execute_batch(
            "INSERT INTO agent_delete_outbox (session_id, requested_at, attempts, last_error)
             VALUES ('sess_stuck', 1, 1, 'Deletion source is unverified')",
        )
        .expect("seed a refused intent that holds no ownership evidence");
        drop(conn);
        let mock = mock_agent();
        mock.default_data(
            "get_session_presence",
            serde_json::json!({"databaseId":"database-live","restoreGeneration":"generation",
                "sessions":[{"sessionId":"sess_stuck","status":"present"}]}),
        );

        assert_eq!(
            reconcile_undeliverable_delete_intents()
                .await
                .expect("reconcile"),
            1
        );
        assert!(
            !crate::store::is_agent_session_tombstoned("sess_stuck").expect("fence"),
            "the session is still in the Agent, so the import used to be fenced out of it forever"
        );
    }

    #[tokio::test]
    async fn reconcile_keeps_the_fence_of_an_intent_that_still_owns_its_session() {
        let (_home, conn) = guarded_conn("reconcile_evidenced");
        conn.execute_batch(
            "INSERT INTO agent_delete_outbox (session_id, requested_at, attempts, last_error)
             VALUES ('sess_evidenced', 1, 1, 'Deletion belongs to a different Agent database');
             INSERT INTO agent_delete_sources (session_id, database_id)
             VALUES ('sess_evidenced', 'database-other');",
        )
        .expect("seed an intent with ownership evidence");
        drop(conn);
        let mock = mock_agent();
        mock.default_data(
            "get_session_presence",
            serde_json::json!({"databaseId":"database-live","restoreGeneration":"generation",
                "sessions":[{"sessionId":"sess_evidenced","status":"present"}]}),
        );

        // A user delete that named its database is deliverable, so the session
        // must never be mirrored again. Dropping the evidence condition from
        // `undeliverable_agent_session_deletes` turns this red.
        assert_eq!(
            reconcile_undeliverable_delete_intents()
                .await
                .expect("reconcile"),
            0
        );
        assert!(
            crate::store::is_agent_session_tombstoned("sess_evidenced").expect("fence"),
            "a conversation the user deleted must not come back"
        );
    }

    #[tokio::test]
    async fn reconcile_keeps_the_fence_when_the_agent_no_longer_holds_the_session() {
        let (_home, conn) = guarded_conn("reconcile_session_gone");
        conn.execute_batch(
            "INSERT INTO agent_delete_outbox (session_id, requested_at, attempts, last_error)
             VALUES ('sess_gone', 1, 1, 'Deletion source is unverified')",
        )
        .expect("seed");
        drop(conn);
        let mock = mock_agent();
        mock.default_data(
            "get_session_presence",
            serde_json::json!({"databaseId":"database-live","restoreGeneration":"generation",
                "sessions":[{"sessionId":"sess_gone","status":"missing"}]}),
        );

        assert_eq!(
            reconcile_undeliverable_delete_intents()
                .await
                .expect("reconcile"),
            0,
            "a session the Agent does not hold is not a conversation to restore"
        );
        assert!(crate::store::is_agent_session_tombstoned("sess_gone").expect("fence"));
    }

    #[tokio::test]
    async fn reconcile_releases_nothing_while_the_agent_is_unreachable() {
        let (_home, conn) = guarded_conn("reconcile_undeliverable_down");
        conn.execute_batch(
            "INSERT INTO agent_delete_outbox (session_id, requested_at, attempts, last_error)
             VALUES ('sess_stuck', 1, 1, 'Deletion source is unverified')",
        )
        .expect("seed");
        drop(conn);
        let mock = mock_agent();
        mock.set_down(true);

        assert_eq!(
            reconcile_undeliverable_delete_intents()
                .await
                .expect("reconcile"),
            0
        );
        assert!(crate::store::is_agent_session_tombstoned("sess_stuck").expect("fence"));
    }

    #[tokio::test]
    async fn reconcile_empty_conversations_removes_a_session_without_messages() {
        let (_home, conn) = guarded_conn("reconcile_empty");
        seed_message_less_threads(&conn, &[("t_empty", "sess_empty", 1)]);
        drop(conn);
        let mock = mock_agent();
        mock.default_data(
            "get_session_entries",
            serde_json::json!({"entries": [], "hasMore": false}),
        );

        assert_eq!(reconcile_empty_conversations().await.expect("reconcile"), 1);
        assert!(crate::store::get_thread("t_empty").expect("get").is_none());
        assert!(
            !crate::store::is_agent_session_tombstoned("sess_empty").expect("tombstone"),
            "the sweep owns no Agent session: it stays importable"
        );
        assert!(
            crate::store::pending_agent_session_deletes()
                .expect("outbox")
                .is_empty(),
            "an empty mirror is not a user delete"
        );
    }

    #[tokio::test]
    async fn reconcile_empty_conversations_keeps_a_session_with_a_message() {
        let (_home, conn) = guarded_conn("reconcile_empty_msg");
        seed_message_less_threads(&conn, &[("t_full", "sess_full", 1)]);
        drop(conn);
        let mock = mock_agent();
        mock.default_data(
            "get_session_entries",
            serde_json::json!({"entries": [
                {"id":"u1","role":"user","kind":"user","blocks":[],"createdAtMs":1000}
            ]}),
        );

        assert_eq!(reconcile_empty_conversations().await.expect("reconcile"), 0);
        assert!(crate::store::get_thread("t_full").expect("get").is_some());
    }

    /// A page that stops mid-journal proves nothing about the entries it left
    /// out, so the row must survive.
    #[tokio::test]
    async fn reconcile_empty_conversations_keeps_a_partial_page() {
        let (_home, conn) = guarded_conn("reconcile_empty_more");
        seed_message_less_threads(&conn, &[("t_more", "sess_more", 1)]);
        drop(conn);
        let mock = mock_agent();
        mock.default_data(
            "get_session_entries",
            serde_json::json!({"entries": [], "hasMore": true}),
        );

        assert_eq!(reconcile_empty_conversations().await.expect("reconcile"), 0);
        assert!(crate::store::get_thread("t_more").expect("get").is_some());
    }

    /// An unreadable journal is not an empty one: the pass keeps the row.
    #[tokio::test]
    async fn reconcile_empty_conversations_keeps_rows_it_cannot_read() {
        let (_home, conn) = guarded_conn("reconcile_empty_err");
        seed_message_less_threads(&conn, &[("t_err", "sess_err", 1)]);
        drop(conn);
        let mock = mock_agent();
        mock.reject_default("get_session_entries", "journal unreadable");

        assert_eq!(reconcile_empty_conversations().await.expect("reconcile"), 0);
        assert!(crate::store::get_thread("t_err").expect("get").is_some());
    }

    #[tokio::test]
    async fn reconcile_empty_conversations_skips_when_the_agent_is_unreachable() {
        let (_home, conn) = guarded_conn("reconcile_empty_down");
        seed_message_less_threads(&conn, &[("t_down", "sess_down", 1)]);
        drop(conn);
        let mock = mock_agent();
        mock.set_down(true);

        assert_eq!(reconcile_empty_conversations().await.expect("reconcile"), 0);
        assert!(crate::store::get_thread("t_down").expect("get").is_some());
    }
}
