use super::*;

/// The registry lives at `$HOME/.future/agent/agent.db` — the same file
/// the session Manager owns for the default sessions directory.
#[test]
fn registry_db_path_matches_the_session_database() {
    let _home = crate::test_support::TestHome::new();
    assert_eq!(
        registry_db_path(),
        crate::utils::default_session_dir("")
            .parent()
            .unwrap()
            .join("agent.db")
    );
}

#[test]
fn install_upgrade_and_uninstall_roundtrip() {
    let _home = crate::test_support::TestHome::new();
    record_skill_installed("future-x", Some("1.0.0")).unwrap();
    let records = list_skill_records().unwrap();
    assert_eq!(
        records,
        vec![SkillRecord {
            name: "future-x".to_string(),
            version: Some("1.0.0".to_string()),
            deleted: false,
            installed_at_ms: records[0].installed_at_ms,
            updated_at_ms: records[0].updated_at_ms,
        }]
    );
    let first_installed_at = records[0].installed_at_ms;

    // Upgrade: version moves, installed_at keeps the first install time.
    record_skill_installed("future-x", Some("2.0.0")).unwrap();
    let record = &list_skill_records().unwrap()[0];
    assert_eq!(record.version.as_deref(), Some("2.0.0"));
    assert_eq!(record.installed_at_ms, first_installed_at);
    assert!(!record.deleted);

    // Uninstall: tombstone, last-known version kept.
    record_skill_uninstalled("future-x").unwrap();
    let record = &list_skill_records().unwrap()[0];
    assert!(record.deleted);
    assert_eq!(record.version.as_deref(), Some("2.0.0"));
    assert_eq!(
        deleted_skill_names().unwrap(),
        ["future-x".to_string()].into()
    );

    // Reinstall clears the tombstone.
    record_skill_installed("future-x", Some("3.0.0")).unwrap();
    assert!(deleted_skill_names().unwrap().is_empty());
    assert_eq!(
        list_skill_records().unwrap()[0].version.as_deref(),
        Some("3.0.0")
    );
}

#[test]
fn uninstalling_an_unknown_skill_creates_a_tombstone() {
    let _home = crate::test_support::TestHome::new();
    record_skill_uninstalled("never-installed").unwrap();
    let record = &list_skill_records().unwrap()[0];
    assert_eq!(record.name, "never-installed");
    assert!(record.deleted);
    assert_eq!(record.version, None);
}

/// A database created by the registry (no agent has run yet) must be
/// accepted by the session store, which fills in the remaining tables.
#[test]
fn registry_created_database_is_accepted_by_the_session_store() {
    let _home = crate::test_support::TestHome::new();
    record_skill_installed("future-x", Some("1.0.0")).unwrap();
    let path = registry_db_path();

    let check = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        check
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        check
            .pragma_query_value(None, "application_id", |row| row.get::<_, i64>(0))
            .unwrap(),
        APPLICATION_ID
    );

    // The session Manager opens agent.db through the strict schema path;
    // a registry-created file must pass it and gain the session tables.
    let manager = crate::session::Manager::new(path.parent().unwrap().join("sessions"));
    manager.initialize().unwrap();
    assert!(manager.import_records().unwrap().is_empty());
    let check = rusqlite::Connection::open(&path).unwrap();
    let sessions: i64 = check
        .query_row("SELECT count(*) FROM sessions", [], |row| row.get(0))
        .unwrap();
    let skills: i64 = check
        .query_row("SELECT count(*) FROM skills", [], |row| row.get(0))
        .unwrap();
    assert_eq!((sessions, skills), (0, 1));
}

/// A v2 database (previous agent release) gains the skills table without
/// its declared version being touched; the agent's schema batch owns the
/// v2 → v3 migration.
#[test]
fn open_registry_upgrades_v2_without_bumping_the_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY NOT NULL);
             PRAGMA application_id = 1179997266;
             PRAGMA user_version = 2;",
        )
        .unwrap();
    drop(connection);

    open_registry(&path).unwrap();
    let check = Connection::open(&path).unwrap();
    let version: i64 = check
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 2);
    let skills: i64 = check
        .query_row("SELECT count(*) FROM skills", [], |row| row.get(0))
        .unwrap();
    assert_eq!(skills, 0);
}

#[test]
fn open_registry_refuses_foreign_databases() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let connection = Connection::open(&path).unwrap();
    connection
        // Must stay within i32: SQLite silently ignores out-of-range
        // application_id values, which would leave the field at 0.
        .pragma_update(None, "application_id", 0x1122_3344i64)
        .unwrap();
    drop(connection);
    assert!(open_registry(&path).is_err());
}

#[test]
fn open_registry_refuses_unrecognized_databases() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let connection = Connection::open(&path).unwrap();
    connection.execute("CREATE TABLE mystery (x)", []).unwrap();
    drop(connection);
    assert!(open_registry(&path).is_err());
}

#[test]
fn open_registry_refuses_future_schema_versions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(&format!(
            "PRAGMA application_id = {APPLICATION_ID};
             PRAGMA user_version = 99;"
        ))
        .unwrap();
    drop(connection);
    assert!(open_registry(&path).is_err());
}

/// The registry writes beside a live session store (whose worker holds
/// agent.db open) — WAL allows the two connections to coexist.
#[test]
fn registry_writes_coexist_with_an_open_session_store() {
    let _home = crate::test_support::TestHome::new();
    let path = registry_db_path();
    let manager = crate::session::Manager::new(path.parent().unwrap().join("sessions"));
    manager.initialize().unwrap();

    record_skill_installed("future-x", Some("1.0.0")).unwrap();
    assert_eq!(list_skill_records().unwrap().len(), 1);

    let check = Connection::open(&path).unwrap();
    assert_eq!(
        check
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        crate::session::database::CURRENT_SCHEMA_VERSION,
        "registry writes must preserve the Agent schema version"
    );

    // The session store's connection still serves requests afterwards.
    assert!(manager.import_records().unwrap().is_empty());
}

/// A v3 database is adopted in place: its tables are kept, and only the
/// schema version moves — opening it must never drop or recreate anything.
#[test]
fn a_v3_database_is_migrated_in_place_without_losing_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch(SKILLS_TABLE_SQL).unwrap();
    connection
        .execute_batch(&format!(
            "PRAGMA application_id = {APPLICATION_ID};\n PRAGMA user_version = 3;"
        ))
        .unwrap();
    connection
        .execute(
            "INSERT INTO skills(name,version,deleted,installed_at_ms,updated_at_ms)
             VALUES('legacy','0.9.0',0,1,1)",
            [],
        )
        .unwrap();
    drop(connection);

    let migrated = open_registry(&path).unwrap();
    let version: i64 = migrated
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 4);
    let name: String = migrated
        .query_row("SELECT name FROM skills", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "legacy", "the upgrade must keep the existing rows");

    // And a v4 database is accepted as-is on the next open.
    drop(migrated);
    assert!(open_registry(&path).is_ok());
}
