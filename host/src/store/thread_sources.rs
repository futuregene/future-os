use rusqlite::{params, OptionalExtension};

pub fn thread_source(thread_id: &str) -> Result<Option<(String, String)>, crate::Error> {
    let conn = super::db::connect()?;
    Ok(conn
        .query_row(
            "SELECT database_id,restore_generation FROM thread_sources WHERE thread_id=?1",
            [thread_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

/// A present session proves initial ownership. A different database never
/// silently claims an existing binding, even at the same filesystem path.
pub fn record_thread_source(
    thread_id: &str,
    session_id: &str,
    database_id: &str,
    generation: &str,
) -> Result<(), crate::Error> {
    if database_id.is_empty() || generation.is_empty() {
        return Err("Agent database identity is missing.".into());
    }
    let mut connection = super::db::connect()?;
    let conn = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let previous: Option<String> = conn
        .query_row(
            "SELECT database_id FROM thread_sources WHERE thread_id=?1",
            [thread_id],
            |row| row.get(0),
        )
        .optional()?;
    if previous
        .as_deref()
        .is_some_and(|previous| previous != database_id)
    {
        return Err("Conversation belongs to a different Agent database. Restore its matching data before continuing.".into());
    }
    let changed = conn.execute("INSERT INTO thread_sources(thread_id,session_id,database_id,restore_generation)
        SELECT id,?2,?3,?4 FROM threads WHERE id=?1 AND agent_session_id=?2
        ON CONFLICT(thread_id) DO UPDATE SET restore_generation=excluded.restore_generation
        WHERE thread_sources.database_id=excluded.database_id AND thread_sources.session_id=excluded.session_id",
        params![thread_id,session_id,database_id,generation])?;
    if changed == 0 {
        return Err("Conversation source binding changed; reload before continuing.".into());
    }
    conn.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::test_support::HomeGuard as TestHome;
    use crate::store::test_support::{seed_thread, seed_workspace};

    #[test]
    fn ownership_is_stable_across_restore_and_rejects_foreign_sources() {
        let home = TestHome::new("source-ownership");
        crate::store::initialize_app_store().expect("init store");
        let workspace = seed_workspace(home.path(), "source");
        let thread = seed_thread(&workspace.id, Some("session"));
        record_thread_source(&thread.id, "session", "database-a", "restore-1").unwrap();
        record_thread_source(&thread.id, "session", "database-a", "restore-2").unwrap();
        assert!(record_thread_source(&thread.id, "session", "database-b", "restore-3").is_err());
        assert!(
            record_thread_source(&thread.id, "different-session", "database-a", "restore-3")
                .is_err()
        );
        assert_eq!(
            thread_source(&thread.id).unwrap(),
            Some(("database-a".into(), "restore-2".into()))
        );
    }

    /// A binding without an identity is refused rather than written: the pair
    /// (database, generation) is what later lets a reader tell "same data" from
    /// "same path, different database", and an empty pair would match anything.
    #[test]
    fn a_binding_without_a_database_identity_is_refused() {
        let _home = crate::auth_store::test_support::HomeGuard::new("thread-source-empty-id");
        crate::store::initialize_app_store().expect("init store");

        for (database_id, generation) in [("", "gen"), ("db", ""), ("", "")] {
            let error = record_thread_source("thread_1", "sess_1", database_id, generation)
                .expect_err("an empty identity is not a binding");
            assert_eq!(
                error.to_string(),
                "Agent database identity is missing.",
                "({database_id:?}, {generation:?})"
            );
        }
        assert!(
            thread_source("thread_1").expect("read back").is_none(),
            "nothing was written for a refused binding"
        );
    }
}
