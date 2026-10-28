//! Admission, independent upgrade snapshots and the versioned schema contract.
//! The historical migration chain keeps its released identities and transactions.
use crate::console;
use std::{collections::HashSet, path::PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use super::util::{create_id, now_millis};
use crate::Error;

const CONTRACT_SQL: &str = include_str!("app_schema_contract.sql");

const STEPS: &[(i64, &str)] = &[
    (1, CONTRACT_SQL),
    (2, include_str!("thread_sources.sql")),
    (3, include_str!("delete_sources.sql")),
    (4, include_str!("retire_reference_caches.sql")),
];

fn checksum(version: i64) -> String {
    let sql = STEPS
        .iter()
        .find(|step| step.0 == version)
        .expect("admitted contract version")
        .1;
    format!("{:x}", Sha256::digest(sql))
}

/// Runs before WAL configuration or any schema write, including on pooled opens.
pub(super) fn admit(conn: &Connection) -> Result<i64, Error> {
    future_app_settings::validate_database_header(conn)?;
    let version = conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))?;
    if version != 0 {
        let ledger: (i64, i64, i64) = conn.query_row(
            "SELECT count(*),min(version),max(version) FROM app_schema_migrations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if ledger != (version, 1, version) {
            return Err("Desktop database version header disagrees with its migration ledger; preserve it for recovery.".into());
        }
        for applied in 1..=version {
            let stored: Option<String> = conn
                .query_row(
                    "SELECT checksum FROM app_schema_migrations WHERE version=?1",
                    [applied],
                    |row| row.get(0),
                )
                .optional()?;
            if stored.as_deref() != Some(checksum(applied).as_str()) {
                return Err("Desktop database migration contract is missing or changed; preserve this database and restore a matching backup.".into());
            }
        }
        let identity: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM app_database_identity WHERE id='main' AND database_id!='' AND restore_generation!='')", [], |row| row.get(0))?;
        if !identity {
            return Err(
                "Desktop database identity is missing; preserve this database for recovery.".into(),
            );
        }
        let tables = table_names(conn)?;
        if ["messages", "run_events", "tool_calls", "tool_outputs"]
            .iter()
            .any(|table| tables.contains(*table))
        {
            return Err("Legacy GUI history appeared after upgrade; preserve both sources and restore a consistent backup.".into());
        }
        return Ok(version);
    }
    let tables = table_names(conn)?;
    if tables.contains("app_schema_migrations") {
        let highest: Option<i64> = conn.query_row(
            "SELECT max(version) FROM app_schema_migrations",
            [],
            |row| row.get(0),
        )?;
        if highest.is_some() {
            return Err("Desktop database version header disagrees with its migration ledger; preserve it for recovery.".into());
        }
    }
    if tables.is_empty() {
        return Ok(0);
    }
    // The CLI can create settings/workspaces before Desktop has ever run.
    // Recognize those partial installs, plus the oldest supported Desktop cores.
    for (table, columns) in [
        ("app_settings", &["key", "value", "updated_at"][..]),
        (
            "workspaces",
            &["id", "name", "kind", "path", "created_at", "updated_at"][..],
        ),
        (
            "threads",
            &[
                "id",
                "workspace_id",
                "mode",
                "title",
                "created_at",
                "updated_at",
            ][..],
        ),
    ] {
        if tables.contains(table) {
            let actual = column_names(conn, table)?;
            if columns.iter().any(|column| !actual.contains(*column)) {
                return Err(format!(
                    "Unrecognized Desktop database layout in {table}; no migration was started."
                )
                .into());
            }
        }
    }
    if !tables.contains("workspaces") && !tables.contains("app_settings") {
        return Err("Unrecognized Desktop database; no migration was started.".into());
    }
    Ok(0)
}

pub(super) fn initialize(conn: &Connection) -> Result<(), Error> {
    let version = admit(conn)?;
    if version == future_app_settings::APP_DB_SCHEMA_VERSION {
        validate_core(conn)?;
        // Optional metadata can retry independently without replaying old cleanup.
        super::db::apply_versioned_migrations(conn).unwrap_or_else(|error| {
            console::eprintln!("FutureOS migration: optional metadata unavailable: {error}");
        });
        return Ok(());
    }
    let backup = snapshot_before_upgrade(conn)?;
    let result = (|| {
        if version == 0 {
            super::db::apply_schema(conn)?;
        }
        for (step, sql) in STEPS.iter().filter(|step| step.0 > version) {
            let tx = rusqlite::Transaction::new_unchecked(
                conn,
                rusqlite::TransactionBehavior::Immediate,
            )?;
            tx.execute_batch(sql)?;
            if *step == 1 {
                tx.execute("INSERT INTO app_database_identity(id,database_id,restore_generation) VALUES ('main',?1,?2)",params![create_id("database"),create_id("restore")])?;
            }
            if *step == future_app_settings::APP_DB_SCHEMA_VERSION {
                validate_core(&tx)?;
            }
            tx.execute(
                "INSERT INTO app_schema_migrations(version,checksum,applied_at) VALUES (?1,?2,?3)",
                params![step, checksum(*step), now_millis()],
            )?;
            tx.pragma_update(
                None,
                "application_id",
                future_app_settings::APP_DB_APPLICATION_ID,
            )?;
            tx.pragma_update(None, "user_version", step)?;
            tx.commit()?;
        }
        Ok(())
    })();
    result.map_err(|error: Error| {
        match backup {
            Some(path) => format!("Desktop database upgrade failed: {error}. Pre-upgrade snapshot: {}. Keep both databases; restart to retry the remaining migration steps.",path.display()).into(),
            None => error,
        }
    })
}

fn table_names(conn: &Connection) -> rusqlite::Result<HashSet<String>> {
    conn.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'")?
        .query_map([], |row| row.get(0))?
        .collect()
}

fn column_names(conn: &Connection, table: &str) -> rusqlite::Result<HashSet<String>> {
    conn.prepare("SELECT name FROM pragma_table_info(?1)")?
        .query_map([table], |row| row.get(0))?
        .collect()
}

/// Validate required structure, never interpret a missing source table as empty.
/// Row-level orphans are left addressable for explicit cancellation and recovery.
fn validate_core(conn: &Connection) -> Result<(), Error> {
    let fresh = Connection::open_in_memory()?;
    fresh.execute_batch(super::schema::SCHEMA)?;
    let actual_tables = table_names(conn)?;
    for table in table_names(&fresh)? {
        let actual = column_names(conn, &table)?;
        for column in column_names(&fresh, &table)? {
            if table == "runs" && column == "archived_at" {
                continue;
            }
            if !actual.contains(&column) {
                return Err(format!("Desktop database is incomplete: required {table}.{column} is missing. Preserve this database for recovery.").into());
            }
        }
        let mut statement = conn.prepare("SELECT \"table\" FROM pragma_foreign_key_list(?1)")?;
        let targets = statement
            .query_map([&table], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for target in targets {
            if !actual_tables.contains(&target) {
                return Err(format!(
                    "Desktop database has a missing foreign-key target: {table} -> {target}."
                )
                .into());
            }
        }
    }
    let unique: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_index_list('threads') WHERE name='idx_threads_agent_session_unique' AND \"unique\"=1)", [], |row| row.get(0))?;
    let sql: Option<String> = conn.query_row("SELECT sql FROM sqlite_schema WHERE type='index' AND name='idx_threads_agent_session_unique'", [], |row| row.get(0)).optional()?;
    let normalized = |sql: &str| {
        sql.split_whitespace()
            .collect::<String>()
            .to_lowercase()
            .replace("ifnotexists", "")
    };
    if !unique
        || sql.as_deref().map(normalized)
            != Some(normalized(super::schema::UNIQUE_AGENT_SESSION_INDEX))
    {
        return Err(
            "Desktop database is missing its valid unique conversation binding index.".into(),
        );
    }
    Ok(())
}

/// VACUUM INTO creates a coherent SQLite snapshot, including committed WAL data.
/// Never copy just app.db or overwrite a previous snapshot on an upgrade retry.
fn snapshot_before_upgrade(conn: &Connection) -> Result<Option<PathBuf>, Error> {
    if table_names(conn)?.is_empty() {
        return Ok(None);
    }
    let Some(source) = conn.path().filter(|path| !path.is_empty()) else {
        return Ok(None);
    };
    let directory = std::path::Path::new(source)
        .parent()
        .ok_or("Desktop database has no parent directory")?
        .join("backups");
    let path = future_agent::session::create_database_snapshot(conn, &directory, "app-before-schema-upgrade")
        .map_err(|error| Error::from(format!("Cannot create a verified Desktop pre-upgrade snapshot: {error}; migration was not started.")))?;
    console::eprintln!(
        "FutureOS: preserved Desktop pre-upgrade snapshot at {}",
        path.display()
    );
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reference_cache_retirement_is_atomic_retryable_and_preserves_live_history() {
        let conn = Connection::open_in_memory().unwrap();
        legacy(&conn);
        initialize(&conn).unwrap();
        // Reinstall the exact retired DDL to reconstruct the contract-3 boundary.
        conn.execute_batch(include_str!("fixtures/retired-reference-tables.sql"))
            .unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;
            INSERT INTO workspace_files VALUES('file','w','/file','file',NULL,1,1,1,1);
            INSERT INTO reference_targets VALUES('target','artifact','artifact','workspace','w','Cached',NULL,NULL,1,1);
            INSERT INTO object_references VALUES('reference','message','u','target',1);
            INSERT INTO runs(id,thread_id,status,created_at,updated_at) VALUES('run','t','completed',1,1);
            INSERT INTO review_snapshots(id,workspace_id,thread_id,run_id,phase,status,created_at) VALUES('snapshot','w','t','run','before','complete',1);
            INSERT INTO artifacts(id,workspace_id,thread_id,title,artifact_type,content,created_at,updated_at) VALUES('artifact','w','t','Keep','markdown','original artifact',1,1);
            DELETE FROM app_schema_migrations WHERE version=4; PRAGMA user_version=3;
            CREATE TRIGGER refuse_v4 BEFORE INSERT ON app_schema_migrations WHEN NEW.version=4 BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        let identity: String = conn
            .query_row("SELECT database_id FROM app_database_identity", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(initialize(&conn).is_err());
        assert_eq!(admit(&conn).unwrap(), 3);
        for table in ["workspace_files", "reference_targets", "object_references"] {
            assert_eq!(
                conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
        }
        assert!(conn
            .prepare("SELECT 1 FROM sqlite_schema WHERE name='idx_review_snapshots_run'")
            .unwrap()
            .exists([])
            .unwrap());
        conn.execute_batch("DROP TRIGGER refuse_v4").unwrap();
        initialize(&conn).unwrap();
        initialize(&conn).unwrap();
        assert_eq!(
            admit(&conn).unwrap(),
            future_app_settings::APP_DB_SCHEMA_VERSION
        );
        for table in ["workspace_files", "reference_targets", "object_references"] {
            assert!(!table_names(&conn).unwrap().contains(table));
        }
        for (sql, expected) in [
            (
                "SELECT content FROM legacy_messages WHERE id='u'",
                "保留历史",
            ),
            (
                "SELECT content FROM artifacts WHERE id='artifact'",
                "original artifact",
            ),
            ("SELECT status FROM runs WHERE id='run'", "completed"),
            (
                "SELECT status FROM review_snapshots WHERE id='snapshot'",
                "complete",
            ),
        ] {
            assert_eq!(
                conn.query_row(sql, [], |r| r.get::<_, String>(0)).unwrap(),
                expected
            );
        }
        assert_eq!(
            conn.query_row("SELECT database_id FROM app_database_identity", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            identity
        );
        assert!(!conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap());
        let plan: String = conn.query_row(
            "EXPLAIN QUERY PLAN SELECT id FROM review_snapshots WHERE run_id='run' AND phase='before'", [], |r| r.get(3),
        ).unwrap();
        assert!(
            plan.contains("sqlite_autoindex_review_snapshots_"),
            "{plan}"
        );
        assert!(!conn
            .prepare("SELECT 1 FROM sqlite_schema WHERE name='idx_review_snapshots_run'")
            .unwrap()
            .exists([])
            .unwrap());
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM app_schema_migrations WHERE version=4",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }

    #[test]
    fn fresh_schema_has_no_retired_caches_and_current_open_does_not_replay_retirement() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        for table in ["workspace_files", "reference_targets", "object_references"] {
            assert!(!table_names(&conn).unwrap().contains(table));
        }
        // A later restore must not make every pooled open repeat destructive SQL.
        conn.execute_batch("CREATE TABLE workspace_files(restored TEXT); INSERT INTO workspace_files VALUES('keep');").unwrap();
        initialize(&conn).unwrap();
        assert_eq!(
            conn.query_row("SELECT restored FROM workspace_files", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "keep"
        );
    }

    #[test]
    fn version_two_qualifies_only_proven_deletion_sources() {
        let conn = Connection::open_in_memory().unwrap();
        legacy(&conn);
        initialize(&conn).unwrap();
        conn.execute_batch("UPDATE threads SET agent_session_id='session' WHERE id='t';
            INSERT INTO thread_sources VALUES('t','session','original-agent','generation');
            INSERT INTO agent_delete_outbox(session_id,requested_at) VALUES('session',1),('unknown',2);
            DROP TABLE agent_delete_sources;
            DELETE FROM app_schema_migrations WHERE version>=3; PRAGMA user_version=2;
            CREATE TRIGGER refuse_v3 BEFORE INSERT ON app_schema_migrations WHEN NEW.version=3 BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(initialize(&conn).is_err());
        assert_eq!(admit(&conn).unwrap(), 2);
        assert!(!table_names(&conn).unwrap().contains("agent_delete_sources"));
        conn.execute_batch("DROP TRIGGER refuse_v3").unwrap();
        initialize(&conn).unwrap();
        let rows: Vec<(String, String)> = conn
            .prepare("SELECT session_id,database_id FROM agent_delete_sources")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(rows, vec![("session".into(), "original-agent".into())]);
        let archive: String = conn
            .query_row(
                "SELECT content FROM legacy_messages WHERE id='u'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(archive, "保留历史");
        assert_eq!(
            admit(&conn).unwrap(),
            future_app_settings::APP_DB_SCHEMA_VERSION
        );
    }

    #[test]
    fn version_one_upgrades_in_order_and_preserves_database_identity() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        let identity: String = conn
            .query_row("SELECT database_id FROM app_database_identity", [], |row| {
                row.get(0)
            })
            .unwrap();
        conn.execute_batch("DROP TABLE agent_delete_sources; DROP TABLE thread_sources; DELETE FROM app_schema_migrations WHERE version>=2; PRAGMA user_version=1;
            CREATE TRIGGER refuse_v2 BEFORE INSERT ON app_schema_migrations WHEN NEW.version=2 BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(initialize(&conn).is_err());
        assert_eq!(admit(&conn).unwrap(), 1);
        assert!(!table_names(&conn).unwrap().contains("thread_sources"));
        conn.execute_batch("DROP TRIGGER refuse_v2").unwrap();
        initialize(&conn).unwrap();
        let after: String = conn
            .query_row("SELECT database_id FROM app_database_identity", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(identity, after);
        assert_eq!(
            admit(&conn).unwrap(),
            future_app_settings::APP_DB_SCHEMA_VERSION
        );
    }

    fn legacy(conn: &Connection) {
        conn.execute_batch(include_str!("fixtures/pre-agent.sql"))
            .unwrap();
        conn.execute_batch("INSERT INTO workspaces(id,name,kind,path,created_at,updated_at) VALUES('w','W','user','.',1,1);
            INSERT INTO threads(id,workspace_id,mode,title,created_at,updated_at) VALUES('t','w','chat','History',1,1);
            INSERT INTO messages(id,thread_id,role,content,created_at,updated_at) VALUES('u','t','user','保留历史',1,1);").unwrap();
    }

    #[test]
    fn fresh_and_released_layouts_converge_without_replaying_cleanup() {
        for fixture in [
            None,
            Some(include_str!("fixtures/v1.2.1.sql")),
            Some(include_str!("fixtures/pre-agent.sql")),
        ] {
            let conn = Connection::open_in_memory().unwrap();
            if let Some(fixture) = fixture {
                conn.execute_batch(fixture).unwrap();
            }
            initialize(&conn).unwrap();
            let identity: String = conn
                .query_row("SELECT database_id FROM app_database_identity", [], |row| {
                    row.get(0)
                })
                .unwrap();
            initialize(&conn).unwrap();
            validate_core(&conn).unwrap();
            assert_eq!(
                admit(&conn).unwrap(),
                future_app_settings::APP_DB_SCHEMA_VERSION
            );
            let after: String = conn
                .query_row("SELECT database_id FROM app_database_identity", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(identity, after);
            let markers: i64 = conn
                .query_row("SELECT count(*) FROM app_schema_migrations", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(markers, future_app_settings::APP_DB_SCHEMA_VERSION);
        }
    }

    #[test]
    fn unsupported_databases_are_rejected_before_any_schema_write() {
        for sql in [
            "CREATE TABLE foreign_data(value TEXT); INSERT INTO foreign_data VALUES('keep');",
            "PRAGMA application_id=123;",
            "PRAGMA application_id=1179992400; PRAGMA user_version=99;",
            "CREATE TABLE workspaces(id TEXT);",
        ] {
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch(sql).unwrap();
            let before = table_names(&conn).unwrap();
            assert!(initialize(&conn).is_err(), "{sql}");
            assert_eq!(table_names(&conn).unwrap(), before);
        }
    }

    #[test]
    fn cli_bootstrap_is_a_supported_legacy_layout() {
        let conn = Connection::open_in_memory().unwrap();
        future_app_settings::ensure_table(&conn).unwrap();
        future_app_workspaces::ensure_table(&conn).unwrap();
        initialize(&conn).unwrap();
        validate_core(&conn).unwrap();
    }

    #[test]
    fn wal_snapshot_contains_original_history_and_is_not_repeated_after_success() {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join("app.db")).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        legacy(&conn);
        initialize(&conn).unwrap();
        initialize(&conn).unwrap();
        let files = fs::read_dir(dir.path().join("backups"))
            .unwrap()
            .map(|file| file.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(files.len(), 1);
        let backup =
            Connection::open_with_flags(&files[0], rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let body: String = backup
            .query_row("SELECT content FROM messages WHERE id='u'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(body, "保留历史");
        let archived: String = conn
            .query_row(
                "SELECT content FROM legacy_messages WHERE id='u'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(archived, body);
        assert_eq!(
            backup
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn backup_failure_prevents_migration() {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join("app.db")).unwrap();
        legacy(&conn);
        fs::write(dir.path().join("backups"), b"blocked").unwrap();
        assert!(initialize(&conn).is_err());
        assert!(table_names(&conn).unwrap().contains("messages"));
        assert!(!table_names(&conn).unwrap().contains("legacy_messages"));
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn failed_contract_commit_is_retryable_without_losing_archives() {
        let conn = Connection::open_in_memory().unwrap();
        legacy(&conn);
        conn.execute_batch(CONTRACT_SQL).unwrap();
        conn.execute_batch("CREATE TRIGGER refuse_contract BEFORE INSERT ON app_schema_migrations BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(initialize(&conn).is_err());
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
        let body: String = conn
            .query_row(
                "SELECT content FROM legacy_messages WHERE id='u'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(body, "保留历史");
        conn.execute_batch("DROP TRIGGER refuse_contract").unwrap();
        initialize(&conn).unwrap();
        assert_eq!(
            admit(&conn).unwrap(),
            future_app_settings::APP_DB_SCHEMA_VERSION
        );
    }

    #[test]
    fn inconsistent_nonzero_headers_never_replay_contract_steps() {
        for extra_version in [0, future_app_settings::APP_DB_SCHEMA_VERSION + 1] {
            let conn = Connection::open_in_memory().unwrap();
            legacy(&conn);
            initialize(&conn).unwrap();
            conn.execute(
                "INSERT INTO app_schema_migrations VALUES(?1,'unknown',1)",
                [extra_version],
            )
            .unwrap();
            assert!(initialize(&conn).is_err());
            let body: String = conn
                .query_row(
                    "SELECT content FROM legacy_messages WHERE id='u'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(body, "保留历史");
        }
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();
        assert!(admit(&conn).is_err());
        assert!(table_names(&conn).unwrap().contains("agent_delete_sources"));
    }

    #[test]
    fn damaged_current_contract_does_not_recreate_or_erase_authoritative_tables() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        conn.execute_batch("UPDATE app_schema_migrations SET checksum='changed' WHERE version=1")
            .unwrap();
        assert!(initialize(&conn).is_err());
        conn.execute(
            "UPDATE app_schema_migrations SET checksum=?1 WHERE version=1",
            [checksum(1)],
        )
        .unwrap();
        conn.execute_batch("DROP TABLE artifacts").unwrap();
        assert!(initialize(&conn).is_err());
        assert!(!table_names(&conn).unwrap().contains("artifacts"));
    }

    #[test]
    fn restored_original_tables_are_retained_and_rejected() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        conn.execute_batch(
            "CREATE TABLE messages(content TEXT); INSERT INTO messages VALUES('restored');",
        )
        .unwrap();
        assert!(initialize(&conn).is_err());
        let body: String = conn
            .query_row("SELECT content FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(body, "restored");
    }
}
