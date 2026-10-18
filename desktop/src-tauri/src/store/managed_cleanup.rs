//! Missing references are recovery candidates, never deletion authorization.
use rusqlite::{params, Connection};

pub(super) const SQL: &str = include_str!("managed_cleanup.sql");
const VERSION: &str = "v1.2.4-managed-directory-cleanup";

pub(super) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    tx.execute_batch(SQL)?;
    tx.execute(
        "INSERT INTO schema_migrations(version,applied_at) VALUES (?1,?2) ON CONFLICT DO NOTHING",
        params![VERSION, super::now_millis()],
    )?;
    tx.commit()
}

/// Called inside the explicit delete transaction, before ownership rows vanish.
pub(super) fn request_thread_in(conn: &Connection, thread_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO managed_directory_cleanup(kind,directory_id,first_seen_at,delete_requested_at)
         SELECT 'images',COALESCE(NULLIF(asset_root_id,''),id),?2,?2 FROM threads WHERE id=?1
         UNION SELECT 'chat',id,?2,?2 FROM threads WHERE id=?1
         UNION SELECT 'chat',agent_session_id,?2,?2 FROM threads
             WHERE id=?1 AND COALESCE(agent_session_id,'')!=''
         UNION SELECT 'chat',session_id,?2,?2 FROM legacy_thread_bindings WHERE thread_id=?1
         ON CONFLICT(kind,directory_id) DO UPDATE SET delete_requested_at=excluded.delete_requested_at",
        params![thread_id, super::now_millis()],
    )?;
    Ok(())
}

pub(super) fn request_workspace_in(conn: &Connection, workspace_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO managed_directory_cleanup(kind,directory_id,first_seen_at,delete_requested_at)
         VALUES ('review',?1,?2,?2)
         ON CONFLICT(kind,directory_id) DO UPDATE SET delete_requested_at=excluded.delete_requested_at",
        params![workspace_id, super::now_millis()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_marker_rolls_back_and_can_be_retried() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_migrations(version TEXT PRIMARY KEY,applied_at INTEGER);
            CREATE TRIGGER refuse_marker BEFORE INSERT ON schema_migrations
            BEGIN SELECT RAISE(ABORT,'injected'); END;",
        )
        .unwrap();
        assert!(migrate(&conn).is_err());
        let tables: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name='managed_directory_cleanup'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0);
        conn.execute_batch("DROP TRIGGER refuse_marker").unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let markers: i64 = conn
            .query_row(
                "SELECT count(*) FROM schema_migrations WHERE version=?1",
                [VERSION],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(markers, 1);
    }

    #[test]
    fn deletion_intent_rolls_back_with_owner_deletion() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(super::super::schema::SCHEMA).unwrap();
        conn.execute_batch("INSERT INTO workspaces(id,name,kind,path,created_at,updated_at) VALUES('w','W','user','.',1,1);
            INSERT INTO threads(id,workspace_id,mode,title,agent_session_id,asset_root_id,created_at,updated_at)
            VALUES('t','w','chat','T','session','shared-assets',1,1);
            INSERT INTO legacy_thread_bindings VALUES('t','legacy-session');").unwrap();
        {
            let tx = conn.transaction().unwrap();
            request_thread_in(&tx, "t").unwrap();
            request_workspace_in(&tx, "w").unwrap();
            tx.execute("DELETE FROM threads WHERE id='t'", []).unwrap();
        }
        let intents: i64 = conn
            .query_row(
                "SELECT count(*) FROM managed_directory_cleanup",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(intents, 0);
        let tx = conn.transaction().unwrap();
        request_thread_in(&tx, "t").unwrap();
        request_thread_in(&tx, "t").unwrap();
        request_workspace_in(&tx, "w").unwrap();
        tx.commit().unwrap();
        let intents: i64 = conn.query_row("SELECT count(*) FROM managed_directory_cleanup WHERE delete_requested_at IS NOT NULL", [], |row| row.get(0)).unwrap();
        assert_eq!(
            intents, 5,
            "one asset root, three chat identities, one review repo"
        );
    }
}
