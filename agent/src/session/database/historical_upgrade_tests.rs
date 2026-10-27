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
    (7, include_str!("../../../tests/fixtures/database/v7.sql")),
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
    if version >= 7 {
        connection.execute("INSERT OR IGNORE INTO storage_meta(key,value) VALUES ('database_id','original-identity'),('restore_generation','original-generation')", []).unwrap();
    }
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

#[test]
fn upgraded_legacy_sessions_continue_beside_new_compact_streams() {
    use crate::session::{
        content::{self, journal::Writer},
        sqlite_store::SqliteStore,
    };
    use serde_json::json;
    for &(version, ddl) in RELEASED_SCHEMAS {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent.db");
        let source = Connection::open(&path).unwrap();
        source.execute_batch(ddl).unwrap();
        seed_history(&source, version);
        drop(source);
        let store = SqliteStore::open(&path).unwrap();
        assert_eq!(store.storage_format("kept").unwrap(), 1);
        store.append("kept",vec![json!({"id":"followup","type":"user","role":"user","timestamp":"2026-10-27T00:00:00Z","content":"continue historical conversation"})]).unwrap();
        let event = json!({"session_id":"kept","run_id":"continued","epoch":2,"idx":0,"session_idx":-1,"event_type":"text_chunk","timestamp":"2026-10-27T00:00:00Z","data":"{\"text\":\"legacy continuation\"}"});
        store.append_event("kept", event).unwrap();
        assert_eq!(store.events("kept", "continued").unwrap().len(), 1);
        let entries = store.entries("kept").unwrap();
        assert!(entries.iter().any(|e| e["id"] == "followup"));
        store.bind_events("compact-new").unwrap();
        assert_eq!(store.storage_format("compact-new").unwrap(), 2);
        let event = json!({"session_id":"compact-new","run_id":"new-run","epoch":1,"idx":0,"session_idx":-1,"event_type":"usage","timestamp":"2026-10-27T00:00:00.123456789Z","data":json!({"input_tokens":7,"unknown":false}).to_string()});
        Writer::default()
            .persist(&store, "compact-new", &[event])
            .unwrap();
        let page = store
            .segmented_replay("compact-new", "new-run", -1, false, Some(8))
            .unwrap();
        assert_eq!(page.events.len(), 1);
        store.db.read(move |db| {
            assert_history(db,version);
            let (raw,compact):(i64,i64)=db.query_row("SELECT (SELECT count(*) FROM run_events WHERE session_id='compact-new'),(SELECT count(*) FROM stream_changes)",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
            assert_eq!((raw,compact),(0,1));content::compact::validate_layout(db)?;Ok(())
        }).unwrap();
        drop(store);
        let reopened = SqliteStore::open(&path).unwrap();
        assert_eq!(reopened.storage_format("kept").unwrap(), 1);
        assert_eq!(reopened.storage_format("compact-new").unwrap(), 2);
        assert_eq!(backups(directory.path()).len(), 1);
        println!("COMPACT_MIGRATION_CASE={{\"sourceSchema\":{version},\"legacyContinuation\":true,\"compactNewWrite\":true,\"sourcePreserved\":true,\"reopen\":true}}");
    }
}

#[test]
fn old_unpublished_layout_is_rejected_before_wal_or_schema_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("agent.db");
    let source = Connection::open(&path).unwrap();
    source.execute_batch(include_str!("schema.sql")).unwrap();
    source
        .execute_batch(include_str!(
            "../../../tests/fixtures/database/unreleased-segmented-v8.sql"
        ))
        .unwrap();
    source.execute_batch("PRAGMA application_id=1179997266; PRAGMA user_version=8; INSERT INTO storage_meta(key,value) VALUES ('database_id','old-draft'),('restore_generation','kept'); INSERT INTO sessions(id) VALUES ('preserve-me');").unwrap();
    drop(source);
    let bytes = std::fs::read(&path).unwrap();
    let error = match Database::open(&path) {
        Ok(_) => panic!("old draft must not be adopted"),
        Err(e) => format!("{e:#}"),
    };
    assert!(error.contains("unsupported unpublished"), "{error}");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(!path.with_extension("db-wal").exists());
    assert!(!directory.path().join("backups").exists());
}

#[test]
fn failed_target_installation_rolls_back_and_retries_from_released_source() {
    for &(version, ddl) in RELEASED_SCHEMAS {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent.db");
        let source = Connection::open(&path).unwrap();
        source.execute_batch(ddl).unwrap();
        seed_history(&source, version);
        // Conflicting target name makes installation fail after earlier target
        // statements. It must not leave a partially upgraded authoritative DB.
        source
            .execute_batch("CREATE VIEW stream_contexts AS SELECT 'conflict' AS value")
            .unwrap();
        drop(source);
        assert!(Database::open(&path).is_err());
        let source = Connection::open(&path).unwrap();
        assert_history(&source, version);
        assert_eq!(
            source
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            version
        );
        assert_eq!(source.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('stream_layout','content_objects','session_formats')",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        source.execute_batch("DROP VIEW stream_contexts").unwrap();
        drop(source);
        let upgraded = Database::open(&path).unwrap();
        upgraded
            .call(move |db| {
                assert_history(db, version);
                crate::session::content::compact::validate_layout(db)?;
                Ok(())
            })
            .unwrap();
    }
}

#[test]
fn damaged_current_layout_is_refused_without_empty_repair() {
    for removal in [
        "DROP INDEX stream_changes_body",
        "DROP TRIGGER stream_direct_removed",
        "DELETE FROM stream_layout",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent.db");
        let database = Database::open(&path).unwrap();
        drop(database);
        let source = Connection::open(&path).unwrap();
        source.execute_batch(removal).unwrap();
        source
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        drop(source);
        let bytes = std::fs::read(&path).unwrap();
        assert!(Database::open(&path).is_err(), "{removal}");
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
