use super::*;

#[test]
fn fresh_registry_keeps_incremental_reclamation_without_rebuilding() {
    let dir = tempfile::tempdir().unwrap();
    let connection = open_registry(&dir.path().join("agent.db")).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "auto_vacuum", |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn registry_rechecks_a_peer_upgrade_after_fresh_or_v3_preflight() {
    for (initial_version, upgraded_version) in [0, 3].into_iter().flat_map(|initial| {
        [5, crate::session::database::CURRENT_SCHEMA_VERSION].map(|version| (initial, version))
    }) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.db");
        let mut connection = Connection::open(&path).unwrap();
        if initial_version == 3 {
            connection
                .execute_batch(&format!(
                    "PRAGMA application_id={APPLICATION_ID}; PRAGMA user_version=3;"
                ))
                .unwrap();
        }
        let initial = inspect_registry_database(&connection).unwrap();
        assert_eq!(initial.version, initial_version);
        assert_eq!(initial.fresh, initial_version == 0);

        // Deterministic interleaving: a peer commits a supported newer schema after preflight
        // but before this mutator acquires its schema write transaction.
        let peer = Connection::open(&path).unwrap();
        peer.execute_batch(&format!(
            "PRAGMA application_id={APPLICATION_ID}; PRAGMA user_version={upgraded_version};
             CREATE TABLE upgrade_marker(value TEXT);
             INSERT INTO upgrade_marker VALUES('kept');"
        ))
        .unwrap();
        initialize_registry_schema(&mut connection).unwrap();
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            upgraded_version
        );
        assert_eq!(
            connection
                .query_row("SELECT value FROM upgrade_marker", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "kept"
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM skills", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn registry_rejects_a_future_or_foreign_schema_committed_after_preflight() {
    for (application, version) in [(APPLICATION_ID, 99), (12345, 5)] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.db");
        let mut connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(&format!(
                "PRAGMA application_id={APPLICATION_ID}; PRAGMA user_version=3;"
            ))
            .unwrap();
        inspect_registry_database(&connection).unwrap();
        let peer = Connection::open(&path).unwrap();
        peer.execute_batch(&format!(
            "PRAGMA application_id={application}; PRAGMA user_version={version};"
        ))
        .unwrap();
        assert!(initialize_registry_schema(&mut connection).is_err());
        assert!(
            connection.is_autocommit(),
            "failed initialization must release its transaction"
        );
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            version
        );
        assert_eq!(
            connection
                .pragma_query_value(None, "application_id", |r| r.get::<_, i64>(0))
                .unwrap(),
            application
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM sqlite_schema WHERE name='skills'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
}
