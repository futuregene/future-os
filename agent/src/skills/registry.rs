//! Skill inventory, history, and operation journal in agent.db.
//!
//! The host-local SkillManager owns filesystem mutation and these tables.
//! Desktop and mobile use Agent RPC; one-shot CLI calls the same manager.
//!
//! The row for an uninstalled skill is deliberately kept as a tombstone
//! (`deleted = 1`): an unseen builtin may be installed automatically, while a
//! skill the user removed must stay removed.
//!
//! The table is created by the agent's schema batch ([`SKILLS_TABLE_SQL`] is
//! part of it) and, when a mutator runs before any agent has started, by
//! [`open_registry`] with the same `application_id`/`user_version` stamps so
//! the agent accepts the file afterwards.

use anyhow::{bail, Context, Result};
#[cfg(test)]
use rusqlite::params;
use rusqlite::Connection;
#[cfg(test)]
use std::collections::HashSet;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::time::Duration;

/// `application_id` shared with the session schema (`session::database`) —
/// 'FUTR'. Marks a database as ours, including ones a mutator created before
/// the agent first ran.
const APPLICATION_ID: i64 = 0x46555452;

/// The `skills` table DDL — the canonical copy. `session::database` includes
/// it in the agent's schema batch.
pub(crate) const SKILLS_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS skills (
    name TEXT PRIMARY KEY NOT NULL,
    version TEXT,
    deleted INTEGER NOT NULL DEFAULT 0 CHECK(deleted IN (0,1)),
    installed_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS skills_meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS skill_installations (
    location TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    scope TEXT NOT NULL CHECK(scope IN ('app','global')),
    source TEXT NOT NULL CHECK(source IN ('managed','external')),
    version TEXT,
    package_sha256 TEXT,
    observed_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS skill_installations_name ON skill_installations(name);
CREATE TABLE IF NOT EXISTS skill_operations (
    name TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('install','uninstall')),
    version TEXT,
    phase TEXT NOT NULL CHECK(phase IN ('prepared','replaced')),
    started_at_ms INTEGER NOT NULL
);";

/// One registry row: a skill's name (the install directory name, equal to the
/// catalogue id and the SKILL.md `name`), the last version recorded for it,
/// and whether it was uninstalled (tombstone).
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRecord {
    pub name: String,
    pub version: Option<String>,
    pub deleted: bool,
    pub installed_at_ms: Option<i64>,
    pub updated_at_ms: i64,
}

/// agent.db, next to the default sessions directory (`~/.future/agent`). The
/// same file the session [`Manager`](crate::session::Manager) owns.
#[cfg(test)]
pub fn registry_db_path() -> PathBuf {
    crate::utils::future_home().join("agent").join("agent.db")
}

/// Record an install (or upgrade — same path) of `name` at `version`,
/// clearing any uninstall tombstone.
#[cfg(test)]
pub fn record_skill_installed(name: &str, version: Option<&str>) -> Result<()> {
    let connection = open_registry(&registry_db_path())?;
    let now = now_ms();
    // installed_at_ms keeps the first-ever install time; every later change
    // only moves version/updated_at_ms.
    connection
        .execute(
            "INSERT INTO skills(name, version, deleted, installed_at_ms, updated_at_ms)
             VALUES (?1, ?2, 0, ?3, ?3)
             ON CONFLICT(name) DO UPDATE SET
                 version = excluded.version,
                 deleted = 0,
                 updated_at_ms = excluded.updated_at_ms",
            params![name, version, now],
        )
        .with_context(|| format!("record install of skill {name:?}"))?;
    Ok(())
}

/// Record an uninstall of `name`: keep the row (and its last-known version)
/// as a tombstone so bootstrap does not re-install the skill later. An
/// explicit install clears the tombstone again.
#[cfg(test)]
pub fn record_skill_uninstalled(name: &str) -> Result<()> {
    let connection = open_registry(&registry_db_path())?;
    let now = now_ms();
    connection
        .execute(
            "INSERT INTO skills(name, version, deleted, installed_at_ms, updated_at_ms)
             VALUES (?1, NULL, 1, NULL, ?2)
             ON CONFLICT(name) DO UPDATE SET
                 deleted = 1,
                 updated_at_ms = excluded.updated_at_ms",
            params![name, now],
        )
        .with_context(|| format!("record uninstall of skill {name:?}"))?;
    Ok(())
}

/// Names with an uninstall tombstone — the set builtin bootstrap must not
/// auto-install.
#[cfg(test)]
pub fn deleted_skill_names() -> Result<HashSet<String>> {
    let connection = open_registry(&registry_db_path())?;
    let mut statement = connection
        .prepare("SELECT name FROM skills WHERE deleted = 1")
        .context("list tombstoned skills")?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<HashSet<_>, _>>()
        .context("list tombstoned skills")?;
    Ok(names)
}

/// Every registry row, ordered by name. Diagnostics/tests; the interesting
/// readers use [`deleted_skill_names`].
#[cfg(test)]
pub fn list_skill_records() -> Result<Vec<SkillRecord>> {
    let connection = open_registry(&registry_db_path())?;
    let mut statement = connection
        .prepare(
            "SELECT name, version, deleted, installed_at_ms, updated_at_ms
             FROM skills ORDER BY name",
        )
        .context("list skill records")?;
    let records = statement
        .query_map([], |row| {
            Ok(SkillRecord {
                name: row.get(0)?,
                version: row.get(1)?,
                deleted: row.get::<_, i64>(2)? != 0,
                installed_at_ms: row.get(3)?,
                updated_at_ms: row.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("list skill records")?;
    Ok(records)
}

/// Open agent.db for a registry write, creating the `skills` table when
/// missing. Runs in mutator processes (CLI, desktop) beside a possibly
/// running agent, so it follows the same recognition rules as the session
/// schema's `open_connection`: refuse foreign or unrecognized files, accept
/// the schema versions this build knows (0, 2, 3, 4).
///
/// A file created here is stamped `application_id` + `user_version = 4`; an
/// existing v3 database moves to v4 after the new tables are created.
pub(crate) fn open_registry(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create SQLite directory")?;
    }
    let mut connection = Connection::open(path).context("open Agent SQLite database")?;
    connection.busy_timeout(Duration::from_secs(5))?;
    let application: i64 =
        connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if application != 0 && application != APPLICATION_ID {
        bail!("database belongs to a different application");
    }
    let fresh = application == 0;
    if fresh {
        let tables: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if version != 0 || tables != 0 {
            bail!("refusing to initialize an unrecognized database");
        }
    } else if version != 0 && version != 2 && version != 3 && version != 4 {
        bail!("unsupported Agent database schema version {version}");
    }
    connection.pragma_update(None, "journal_mode", "WAL")?;
    let tx = connection.transaction()?;
    tx.execute_batch(SKILLS_TABLE_SQL)?;
    if fresh {
        tx.execute_batch(&format!(
            "PRAGMA application_id = {APPLICATION_ID};
             PRAGMA user_version = 4;"
        ))?;
    } else if version == 3 {
        tx.execute_batch("PRAGMA user_version = 4;")?;
    }
    tx.commit()?;
    Ok(connection)
}

#[cfg(test)]
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
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
}
