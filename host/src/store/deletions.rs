//! Durable GUI-side delivery queue for idempotent Agent session deletion.

use rusqlite::{params, Connection};

use super::db::connect;
use super::util::now_millis;

pub(super) fn enqueue_agent_session_delete_in(
    conn: &Connection,
    session_id: &str,
) -> rusqlite::Result<()> {
    if session_id.trim().is_empty() {
        return Ok(());
    }
    const SQL: &str =
        "INSERT INTO agent_delete_outbox(session_id, requested_at, attempts, last_error)
         VALUES (?1, ?2, 0, NULL)
         ON CONFLICT(session_id) DO NOTHING";
    conn.execute(SQL, params![session_id, now_millis()])?;
    conn.execute(
        "INSERT INTO agent_delete_sources(session_id,database_id)
        SELECT ?1,MIN(database_id) FROM thread_sources WHERE session_id=?1
        HAVING count(DISTINCT database_id)=1
        ON CONFLICT(session_id) DO NOTHING",
        [session_id],
    )?;
    Ok(())
}

/// A tombstone is an admission fence for GUI import/discovery.  It remains
/// until the Agent acknowledges deletion, so a temporarily offline Agent can
/// never cause a locally deleted session to reappear in the sidebar.
pub fn is_agent_session_tombstoned(session_id: &str) -> Result<bool, crate::Error> {
    const SQL: &str = "SELECT EXISTS(SELECT 1 FROM agent_delete_outbox WHERE session_id = ?1)";
    let conn = connect()?;
    Ok(conn.query_row(SQL, [session_id], |row| row.get(0))?)
}

#[cfg(any(test, feature = "test-support"))]
pub fn pending_agent_session_deletes() -> Result<Vec<String>, crate::Error> {
    Ok(pending_agent_session_deletes_with_sources()?
        .into_iter()
        .map(|(session, _)| session)
        .collect())
}

pub fn pending_agent_session_deletes_with_sources(
) -> Result<Vec<(String, Option<String>)>, crate::Error> {
    let conn = connect()?;
    let mut stmt = conn
        .prepare("SELECT o.session_id,s.database_id FROM agent_delete_outbox o LEFT JOIN agent_delete_sources s ON s.session_id=o.session_id ORDER BY o.requested_at,o.session_id")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let session_ids = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(session_ids)
}

/// Pending deletion intents this GUI can never deliver, because no ownership
/// evidence for the session they name survives.
///
/// [`super::pending_agent_session_deletes_with_sources`] is fed to the delivery
/// pass, which refuses every intent whose source is unverified; a source is only
/// ever recorded from `thread_sources` (see [`enqueue_agent_session_delete_in`]).
/// An intent holding neither a recorded source nor a `thread_sources` row can
/// therefore never leave this queue — and while it sits there it keeps
/// [`is_agent_session_tombstoned`] true, which fences the session out of every
/// import. The conversation stays in the Agent database and is invisible to the
/// GUI forever, which no later pass can repair on its own.
///
/// `attempts > 0` keeps the rule to intents that were actually refused: a fresh
/// row is waiting its first delivery attempt, not stuck. A row a delivery
/// settled is gone from this queue by construction
/// ([`settle_agent_session_deletes`] deletes it), so nothing returned here ever
/// reached the Agent.
pub fn undeliverable_agent_session_deletes() -> Result<Vec<String>, crate::Error> {
    const SQL: &str = "SELECT o.session_id FROM agent_delete_outbox o
         WHERE o.attempts > 0
           AND NOT EXISTS (SELECT 1 FROM agent_delete_sources s WHERE s.session_id = o.session_id)
           AND NOT EXISTS (SELECT 1 FROM thread_sources t WHERE t.session_id = o.session_id)
         ORDER BY o.requested_at, o.session_id";
    let conn = connect()?;
    let mut stmt = conn.prepare(SQL)?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Drop the import fence for intents [`undeliverable_agent_session_deletes`]
/// selected, so the next discovery pass may mirror the conversation again.
///
/// The caller owns the Agent half of the judgement: it must have confirmed the
/// session is still *present* in the Agent database. Releasing an intent whose
/// session is gone would only forget a deletion that already happened.
///
/// The predicate is re-checked inside the write transaction, so an intent that
/// became deliverable (or was settled by a delivery) since the probe is left
/// alone. Returns how many fences were dropped.
pub fn release_undeliverable_agent_session_deletes(
    session_ids: &[String],
) -> Result<usize, crate::Error> {
    const SQL: &str = "DELETE FROM agent_delete_outbox WHERE session_id = ?1
           AND attempts > 0
           AND NOT EXISTS (SELECT 1 FROM agent_delete_sources s
                           WHERE s.session_id = agent_delete_outbox.session_id)
           AND NOT EXISTS (SELECT 1 FROM thread_sources t
                           WHERE t.session_id = agent_delete_outbox.session_id)";
    let mut conn = connect()?;
    let tx = conn.transaction()?;
    let mut released = 0;
    for session_id in session_ids {
        released += tx.execute(SQL, [session_id])?;
    }
    tx.commit()?;
    Ok(released)
}

#[cfg(test)]
fn acknowledge_agent_session_delete(session_id: &str) -> Result<(), crate::Error> {
    settle_agent_session_deletes(&[(session_id.to_owned(), None)])
}

#[cfg(test)]
fn note_agent_session_delete_failure(session_id: &str, error: &str) -> Result<(), crate::Error> {
    settle_agent_session_deletes(&[(session_id.to_owned(), Some(error.to_owned()))])
}

/// Acknowledge only confirmed outcomes, keeping failures fenced for retry.
/// All outcomes from one Agent batch share one local durability boundary.
pub fn settle_agent_session_deletes(
    outcomes: &[(String, Option<String>)],
) -> Result<(), crate::Error> {
    let mut conn = connect()?;
    let tx = conn.transaction()?;
    for (session_id, error) in outcomes {
        if let Some(error) = error {
            tx.execute(
                "UPDATE agent_delete_outbox SET attempts = attempts + 1, last_error = ?2 WHERE session_id = ?1",
                params![session_id, error],
            )?;
        } else {
            tx.execute(
                "DELETE FROM agent_delete_outbox WHERE session_id = ?1",
                [session_id],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Preserve deletion intent across Debug ▸ Reset.  The GUI database may be
/// cleared, but Agent-owned canonical sessions must still be reclaimed.
pub(super) fn enqueue_all_agent_session_deletes_in(conn: &Connection) -> rusqlite::Result<()> {
    let now = now_millis();
    // INSERT OR IGNORE, not `ON CONFLICT DO NOTHING`: the upsert clause is a
    // syntax error on INSERT…SELECT in SQLite's grammar (the ON reads as a
    // join constraint), and this path failing would break Debug ▸ Reset.
    const SQL: &str =
        "INSERT OR IGNORE INTO agent_delete_outbox(session_id, requested_at, attempts, last_error)
         SELECT DISTINCT COALESCE(NULLIF(TRIM(agent_session_id), ''), id), ?1, 0, NULL
         FROM threads";
    conn.execute(SQL, [now])?;
    conn.execute_batch(include_str!("delete_sources.sql"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::test_support::{guarded_conn, memory_conn};

    /// An intent with no ownership evidence and a refused first delivery is
    /// structurally undeliverable: the fence it holds can never lift by itself.
    #[test]
    fn only_refused_intents_without_any_ownership_evidence_are_undeliverable() {
        let (_home, conn) = guarded_conn("deletions_undeliverable");
        conn.execute_batch(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws1', 'WS', 'temporary', '/tmp/ws1', 'active', 1, 1);
             INSERT INTO threads (
                 id, workspace_id, mode, title, agent_session_id, created_at, updated_at
             ) VALUES ('t_src', 'ws1', 'chat', 'T', 'sess_thread_source', 1, 1);
             INSERT INTO agent_delete_outbox (session_id, requested_at, attempts, last_error)
             VALUES ('sess_stuck', 1, 1, 'Deletion source is unverified'),
                    ('sess_fresh', 2, 0, NULL),
                    ('sess_evidence', 3, 1, 'Deletion belongs to a different Agent database'),
                    ('sess_thread_source', 4, 1, 'Deletion source is unverified');
             INSERT INTO agent_delete_sources (session_id, database_id)
             VALUES ('sess_evidence', 'database-other');
             INSERT INTO thread_sources (thread_id, session_id, database_id, restore_generation)
             VALUES ('t_src', 'sess_thread_source', 'database-x', 'generation');",
        )
        .expect("seed outbox shapes");
        drop(conn);

        assert_eq!(
            undeliverable_agent_session_deletes().expect("candidates"),
            vec!["sess_stuck".to_string()],
            "an intent holding ownership evidence is deliverable and must stay fenced, \
             and a fresh intent has not had its delivery attempt yet"
        );
    }

    #[test]
    fn releasing_an_undeliverable_intent_lifts_only_its_fence() {
        let (_home, conn) = guarded_conn("deletions_release");
        conn.execute_batch(
            "INSERT INTO agent_delete_outbox (session_id, requested_at, attempts, last_error)
             VALUES ('sess_stuck', 1, 1, 'Deletion source is unverified'),
                    ('sess_evidence', 2, 1, 'Deletion belongs to a different Agent database');
             INSERT INTO agent_delete_sources (session_id, database_id)
             VALUES ('sess_evidence', 'database-other');",
        )
        .expect("seed");
        drop(conn);

        let released = release_undeliverable_agent_session_deletes(&[
            "sess_stuck".to_string(),
            "sess_evidence".to_string(),
        ])
        .expect("release");
        assert_eq!(released, 1, "the evidenced intent is never released");
        assert!(!is_agent_session_tombstoned("sess_stuck").expect("stuck"));
        assert!(
            is_agent_session_tombstoned("sess_evidence").expect("evidenced"),
            "a delivery that has ownership evidence owns the session: releasing it would \
             mirror a conversation the user deleted"
        );
    }

    #[test]
    fn enqueue_ignores_blank_and_duplicate_session_ids() {
        let conn = memory_conn();
        enqueue_agent_session_delete_in(&conn, "   ").expect("blank is a no-op");
        enqueue_agent_session_delete_in(&conn, "sess_1").expect("enqueue");
        enqueue_agent_session_delete_in(&conn, "sess_1").expect("conflict tolerated");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM agent_delete_outbox", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(count, 1);
    }

    #[test]
    fn outbox_lifecycle_against_the_real_db() {
        let (_home, conn) = guarded_conn("deletions_lifecycle");
        enqueue_agent_session_delete_in(&conn, "sess_b").expect("enqueue b");
        std::thread::sleep(std::time::Duration::from_millis(2));
        enqueue_agent_session_delete_in(&conn, "sess_a").expect("enqueue a");
        drop(conn);

        assert!(is_agent_session_tombstoned("sess_a").expect("tombstoned"));
        assert!(!is_agent_session_tombstoned("sess_z").expect("not tombstoned"));

        // Pending deletes come out in request order.
        assert_eq!(
            pending_agent_session_deletes().expect("pending"),
            vec!["sess_b".to_string(), "sess_a".to_string()]
        );

        note_agent_session_delete_failure("sess_a", "offline").expect("note failure");
        assert!(
            is_agent_session_tombstoned("sess_a").expect("still tombstoned"),
            "a failed delivery keeps the tombstone"
        );

        acknowledge_agent_session_delete("sess_a").expect("ack");
        assert!(!is_agent_session_tombstoned("sess_a").expect("acknowledged"));
        assert_eq!(
            pending_agent_session_deletes().expect("pending"),
            vec!["sess_b".to_string()]
        );
    }

    #[test]
    fn failure_notes_accumulate_attempts_and_the_error() {
        let (_home, conn) = guarded_conn("deletions_failure");
        enqueue_agent_session_delete_in(&conn, "sess_f").expect("enqueue");
        note_agent_session_delete_failure("sess_f", "first").expect("note 1");
        note_agent_session_delete_failure("sess_f", "second").expect("note 2");

        let (attempts, last_error): (i64, String) = conn
            .query_row(
                "SELECT attempts, last_error FROM agent_delete_outbox WHERE session_id = 'sess_f'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("load outbox row");
        assert_eq!(attempts, 2);
        assert_eq!(last_error, "second");
    }

    #[test]
    fn enqueue_all_collects_distinct_session_ids_with_thread_id_fallback() {
        let conn = memory_conn();
        conn.execute_batch(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws1', 'WS', 'temporary', '/tmp/ws1', 'active', 1, 1);
             INSERT INTO threads (
                 id, workspace_id, mode, title, agent_session_id, created_at, updated_at
             ) VALUES
                 ('t1', 'ws1', 'chat', 'T1', 'sess_1', 1, 1),
                 ('t2', 'ws1', 'chat', 'T2', 'sess_2', 1, 1),
                 ('t3', 'ws1', 'chat', 'T3', '  ', 1, 1),
                 ('t4', 'ws1', 'chat', 'T4', NULL, 1, 1);",
        )
        .expect("seed threads");

        enqueue_all_agent_session_deletes_in(&conn).expect("enqueue all");

        let mut stmt = conn
            .prepare("SELECT session_id FROM agent_delete_outbox ORDER BY session_id")
            .expect("prepare");
        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect");
        // Bound session ids are unique; blank and NULL ids fall back to the
        // owning thread id.
        assert_eq!(ids, vec!["sess_1", "sess_2", "t3", "t4"]);
    }
}
