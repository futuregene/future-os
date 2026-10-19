//! Released schemas are frozen fixtures, never the current schema relabelled.
use super::*;
use rusqlite::params;
use std::path::PathBuf;

const RELEASED_SCHEMAS: &[(i64, &str)] = &[
    (2, include_str!("../../../tests/fixtures/database/v2.sql")),
    (3, include_str!("../../../tests/fixtures/database/v3.sql")),
    (4, include_str!("../../../tests/fixtures/database/v4.sql")),
    (5, include_str!("../../../tests/fixtures/database/v5.sql")),
    (6, include_str!("../../../tests/fixtures/database/v6.sql")),
];

fn seed_history(connection: &Connection, version: i64) {
    connection.execute_batch(
        "INSERT INTO sessions(id,revision,current_metadata_json) VALUES ('kept',3,'{\"session_name\":\"historical title\"}');
         INSERT INTO entries(session_id,position,entry_id,entry_type,role,metadata_json,content_json)
             VALUES ('kept',0,'entry','user','user','{\"future_field\":true}','\"historical text\"');
         INSERT INTO message_blocks(session_id,entry_position,ordinal,kind,text,metadata_json)
             VALUES ('kept',0,0,'text','historical text','{}');
         INSERT INTO runs(session_id,run_id,status,run_sequence,epoch,input_tokens)
             VALUES ('kept','run','completed',1,1,42);
         INSERT INTO run_events(sequence,session_id,run_id,epoch,idx,event_id,payload)
             VALUES (23,'kept','run',1,0,'event','{\"text\":\"historical event\"}');
         INSERT INTO legacy_imports(session_id,status,fingerprint) VALUES ('deleted','deleted','hash');
         INSERT INTO storage_meta(key,value) VALUES ('historical-key','historical-value');",
    ).unwrap();
    if version >= 3 {
        connection
            .execute(
                "INSERT INTO skills(name,deleted,updated_at_ms) VALUES ('removed-skill',1,123)",
                [],
            )
            .unwrap();
    }
    if version >= 4 {
        connection.execute("INSERT INTO skill_installations(location,name,scope,source,observed_at_ms) VALUES ('synthetic-location','installed-skill','global','managed',123)", []).unwrap();
    }
    if version >= 5 {
        connection.execute("INSERT INTO run_snapshots(session_id,run_id,version,cursor,payload,pricing_payloads) VALUES ('kept','run',1,22,'{\"history\":true}','[]')", []).unwrap();
    }
}

fn assert_history(connection: &Connection, version: i64) {
    for (query, expected) in [
        (
            "SELECT current_metadata_json FROM sessions WHERE id='kept'",
            "{\"session_name\":\"historical title\"}",
        ),
        (
            "SELECT metadata_json FROM entries WHERE session_id='kept'",
            "{\"future_field\":true}",
        ),
        (
            "SELECT content_json FROM entries WHERE session_id='kept'",
            "\"historical text\"",
        ),
        (
            "SELECT text FROM message_blocks WHERE session_id='kept'",
            "historical text",
        ),
        (
            "SELECT payload FROM run_events WHERE sequence=23",
            "{\"text\":\"historical event\"}",
        ),
        (
            "SELECT status FROM legacy_imports WHERE session_id='deleted'",
            "deleted",
        ),
        (
            "SELECT value FROM storage_meta WHERE key='historical-key'",
            "historical-value",
        ),
    ] {
        assert_eq!(
            connection
                .query_row(query, [], |r| r.get::<_, String>(0))
                .unwrap(),
            expected,
            "schema {version}: {query}"
        );
    }
    let run: (String, i64) = connection
        .query_row(
            "SELECT status,input_tokens FROM runs WHERE session_id='kept'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(run, ("completed".into(), 42));
    if version >= 3 {
        assert_eq!(
            connection
                .query_row(
                    "SELECT deleted FROM skills WHERE name='removed-skill'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }
    if version >= 4 {
        assert_eq!(
            connection
                .query_row(
                    "SELECT name FROM skill_installations WHERE location='synthetic-location'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "installed-skill"
        );
    }
    if version >= 5 {
        let snapshot: (String, String) = connection
            .query_row(
                "SELECT payload,pricing_payloads FROM run_snapshots WHERE session_id='kept'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(snapshot, ("{\"history\":true}".into(), "[]".into()));
    }
}

fn backups(directory: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(directory.join("backups"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect()
}

#[test]
fn every_released_schema_upgrades_with_history_backup_and_idempotent_reopen() {
    for &(version, ddl) in RELEASED_SCHEMAS {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent.db");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(ddl).unwrap();
        seed_history(&connection, version);
        drop(connection);

        let database =
            Database::open(&path).unwrap_or_else(|error| panic!("schema {version}: {error:#}"));
        database
            .call(move |connection| {
                assert_history(connection, version);
                assert_eq!(
                    connection.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?,
                    CURRENT_SCHEMA_VERSION
                );
                assert_eq!(
                    connection.query_row(
                        "SELECT value FROM storage_meta WHERE key='event_sequence'",
                        [],
                        |r| r.get::<_, String>(0)
                    )?,
                    "23"
                );
                assert_eq!(
                    connection.query_row(
                        "SELECT count(*) FROM session_tombstones WHERE session_id='deleted'",
                        [],
                        |r| r.get::<_, i64>(0)
                    )?,
                    1
                );
                // The upgraded store accepts new writes alongside preserved history.
                connection.execute("INSERT INTO sessions(id) VALUES ('new')", [])?;
                Ok(())
            })
            .unwrap();
        drop(database);

        let snapshots = backups(directory.path());
        assert_eq!(snapshots.len(), 1);
        let snapshot =
            Connection::open_with_flags(&snapshots[0], rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        assert_eq!(
            snapshot
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            version
        );
        assert_history(&snapshot, version);
        drop(snapshot);

        let database = Database::open(&path).unwrap();
        database
            .call(move |connection| {
                assert_history(connection, version);
                assert_eq!(
                    connection
                        .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))?,
                    2
                );
                Ok(())
            })
            .unwrap();
        assert_eq!(backups(directory.path()), snapshots);
    }
}

#[test]
fn missing_released_history_is_rejected_without_recreating_tables() {
    for &(version, ddl) in RELEASED_SCHEMAS {
        for table in ["entries", "run_events", "run_snapshots"] {
            if table == "run_snapshots" && version < 5 {
                continue;
            }
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("agent.db");
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch(ddl).unwrap();
            connection
                .execute("INSERT INTO sessions(id) VALUES ('keep')", [])
                .unwrap();
            connection
                .execute_batch(&format!("DROP TABLE {table}"))
                .unwrap();
            drop(connection);

            let error = match Database::open(&path) {
                Ok(_) => panic!("schema {version}: missing {table} must fail"),
                Err(error) => error,
            };
            assert!(
                format!("{error:#}").contains("history structure is incomplete")
                    || error.to_string().contains("unsupported pre-release")
            );
            let connection = Connection::open(&path).unwrap();
            assert_eq!(
                connection
                    .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                    .unwrap(),
                version
            );
            assert_eq!(
                connection
                    .query_row(
                        "SELECT count(*) FROM sqlite_schema WHERE name=?1",
                        [table],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                0
            );
            assert_eq!(
                connection
                    .query_row("SELECT count(*) FROM sessions WHERE id='keep'", [], |r| r
                        .get::<_, i64>(
                        0
                    ))
                    .unwrap(),
                1
            );
        }
    }
}

#[test]
fn snapshots_in_an_old_version_are_validated_when_present() {
    for &(version, ddl) in &RELEASED_SCHEMAS[..3] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent.db");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(ddl).unwrap();
        connection
            .execute_batch("CREATE TABLE run_snapshots(session_id TEXT, payload TEXT)")
            .unwrap();
        drop(connection);
        let error = match Database::open(&path) {
            Ok(_) => panic!("schema {version}: malformed existing snapshots must fail"),
            Err(error) => error,
        };
        assert!(format!("{error:#}").contains("history structure is incomplete"));
    }
}

#[test]
fn skills_only_databases_upgrade_without_losing_uninstall_tombstones() {
    let v3 = include_str!("../../../tests/fixtures/database/skills-v3.sql");
    let v4 = include_str!("../../../tests/fixtures/database/skills-v4.sql");
    for (version, ddl) in [(0, v3), (3, v3), (4, v4)] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent.db");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(ddl).unwrap();
        connection
            .pragma_update(None, "application_id", 0x46555452)
            .unwrap();
        connection
            .pragma_update(None, "user_version", version)
            .unwrap();
        connection
            .execute(
                "INSERT INTO skills(name,deleted,updated_at_ms) VALUES (?1,1,123)",
                params!["removed-skill"],
            )
            .unwrap();
        drop(connection);
        let database = Database::open(&path).unwrap();
        database
            .call(|connection| {
                assert_eq!(
                    connection.query_row(
                        "SELECT deleted FROM skills WHERE name='removed-skill'",
                        [],
                        |r| r.get::<_, i64>(0)
                    )?,
                    1
                );
                assert_eq!(
                    connection
                        .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))?,
                    0
                );
                Ok(())
            })
            .unwrap();
    }
}
