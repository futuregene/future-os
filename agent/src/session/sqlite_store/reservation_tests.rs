use super::*;

#[test]
fn stream_binding_selects_only_genuinely_new_identities() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&directory.path().join("agent.db")).unwrap();
    store
        .db
        .call(|db| {
            db.execute_batch(
                "INSERT INTO sessions(id,revision) VALUES ('old',-1);
            INSERT INTO legacy_imports(session_id,status,fingerprint) VALUES
                ('imported','imported','fixture'), ('skipped','skipped','fixture'),
                ('deleted-import','deleted','fixture');
            INSERT INTO session_tombstones(session_id,deleted_at_ms) VALUES ('deleted',0);",
            )?;
            Ok(())
        })
        .unwrap();
    for session in ["fresh", "old", "imported"] {
        store.bind_events_with_format(session, 2).unwrap();
    }
    assert_eq!(store.storage_format("fresh").unwrap(), 2);
    for session in ["old", "imported"] {
        assert_eq!(store.storage_format(session).unwrap(), 1);
    }
    // Rebinding with a different default cannot change a stored decision.
    store.bind_events_with_format("fresh", 1).unwrap();
    store.bind_events_with_format("old", 2).unwrap();
    assert_eq!(store.storage_format("fresh").unwrap(), 2);
    assert_eq!(store.storage_format("old").unwrap(), 1);
    for session in ["skipped", "deleted", "deleted-import"] {
        assert!(store.bind_events_with_format(session, 2).is_err());
    }
    store
        .db
        .read(|db| {
            let count: i64 = db.query_row(
                "SELECT COUNT(*) FROM sessions WHERE id IN ('skipped','deleted','deleted-import')",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(count, 0);
            Ok(())
        })
        .unwrap();
}

#[test]
fn reservation_and_format_are_one_transaction() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&directory.path().join("agent.db")).unwrap();
    store
        .db
        .call(|db| {
            db.execute_batch(
                "CREATE TRIGGER reject_format BEFORE INSERT ON session_formats
            BEGIN SELECT RAISE(ABORT,'injected reservation failure'); END;",
            )?;
            Ok(())
        })
        .unwrap();
    assert!(store.bind_events_with_format("fresh", 2).is_err());
    store
        .db
        .read(|db| {
            let exists: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE id='fresh')",
                [],
                |row| row.get(0),
            )?;
            assert!(!exists);
            Ok(())
        })
        .unwrap();
}

#[test]
fn production_binding_uses_the_selected_default() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&directory.path().join("agent.db")).unwrap();
    store.bind_events("fresh").unwrap();
    assert_eq!(
        store.storage_format("fresh").unwrap(),
        crate::session::content::NEW_SESSION_FORMAT
    );
}
