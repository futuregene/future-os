//! Skill-registry writes into agent.db — the desktop's mirror of the agent
//! crate's `skills::registry`.
//!
//! The desktop must not link the agent crate (it talks to the agent over
//! gRPC), so this module duplicates the tiny write path: the `skills` table
//! DDL and the application_id / user_version guard protocol are copies of
//! `agent/src/skills/registry.rs` — keep the two in sync. Reads (the
//! tombstone set for builtin bootstrap) are not mirrored: bootstrap runs
//! through the bundled CLI (`future init`), which links the agent crate.
//!
//! Why write agent.db directly instead of asking the agent: skill
//! install/uninstall must also work while no local agent is running (remote
//! setups), and the filesystem mutations are the desktop's own — the agent
//! only discovers skills. SQLite WAL lets the two processes share the file.

use rusqlite::{params, Connection};
use std::path::Path;
use std::time::Duration;

use crate::AppError;

/// 'FUTR' — the agent.db `application_id`, shared with the agent's session
/// schema. Mirror of the agent crate's registry constant.
const APPLICATION_ID: i64 = 0x46555452;

/// Mirror of the agent crate's `skills::registry::SKILLS_TABLE_SQL`.
const SKILLS_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS skills (
    name TEXT PRIMARY KEY NOT NULL,
    version TEXT,
    deleted INTEGER NOT NULL DEFAULT 0 CHECK(deleted IN (0,1)),
    installed_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL
);";

/// agent.db beside the app-scope skills directory (`~/.future/agent`).
fn registry_db_path() -> Result<std::path::PathBuf, AppError> {
    Ok(crate::auth_store::agent_dir()?.join("agent.db"))
}

/// Record an install (or upgrade) of `id` at `version`, clearing any
/// uninstall tombstone.
pub(crate) fn record_installed(id: &str, version: &str) -> Result<(), AppError> {
    let connection = open_registry(&registry_db_path()?)?;
    let now = now_ms();
    connection.execute(
        "INSERT INTO skills(name, version, deleted, installed_at_ms, updated_at_ms)
         VALUES (?1, ?2, 0, ?3, ?3)
         ON CONFLICT(name) DO UPDATE SET
             version = excluded.version,
             deleted = 0,
             updated_at_ms = excluded.updated_at_ms",
        params![id, version, now],
    )?;
    Ok(())
}

/// Record an uninstall of `id`: keep the row (and its last-known version) as
/// a tombstone so the CLI's builtin bootstrap does not re-install the skill.
pub(crate) fn record_uninstalled(id: &str) -> Result<(), AppError> {
    let connection = open_registry(&registry_db_path()?)?;
    let now = now_ms();
    connection.execute(
        "INSERT INTO skills(name, version, deleted, installed_at_ms, updated_at_ms)
         VALUES (?1, NULL, 1, NULL, ?2)
         ON CONFLICT(name) DO UPDATE SET
             deleted = 1,
             updated_at_ms = excluded.updated_at_ms",
        params![id, now],
    )?;
    Ok(())
}

/// Open agent.db for a registry write, creating the `skills` table when
/// missing. Mirrors the agent's recognition rules: refuse foreign or
/// unrecognized files; accept the schema versions this build knows (0, 2, 3).
/// A file created here is stamped so the agent accepts it afterwards; an
/// existing database keeps its declared version (the agent's schema batch
/// owns migrations).
fn open_registry(path: &Path) -> Result<Connection, AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    let application: i64 =
        connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if application != 0 && application != APPLICATION_ID {
        return Err(AppError::Message(
            "database belongs to a different application".to_string(),
        ));
    }
    let fresh = application == 0;
    if fresh {
        let tables: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if version != 0 || tables != 0 {
            return Err(AppError::Message(
                "refusing to initialize an unrecognized database".to_string(),
            ));
        }
    } else if version != 0 && version != 2 && version != 3 {
        return Err(AppError::Message(format!(
            "unsupported Agent database schema version {version}"
        )));
    }
    connection.pragma_update(None, "journal_mode", "WAL")?;
    let tx = connection.transaction()?;
    tx.execute_batch(SKILLS_TABLE_SQL)?;
    if fresh {
        tx.execute_batch(&format!(
            "PRAGMA application_id = {APPLICATION_ID};
             PRAGMA user_version = 3;"
        ))?;
    }
    tx.commit()?;
    Ok(connection)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry_row(id: &str) -> (Option<String>, bool) {
        let connection = Connection::open(registry_db_path().unwrap()).unwrap();
        connection
            .query_row(
                "SELECT version, deleted FROM skills WHERE name = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get::<_, i64>(1)? != 0)),
            )
            .unwrap()
    }

    #[test]
    fn record_installed_writes_and_upgrades() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-registry-install");
        record_installed("future-x", "1.0.0").unwrap();
        assert_eq!(registry_row("future-x"), (Some("1.0.0".to_string()), false));

        // Upgrade: version moves, still installed.
        record_installed("future-x", "2.0.0").unwrap();
        assert_eq!(registry_row("future-x"), (Some("2.0.0".to_string()), false));

        // A desktop-created database is stamped for the agent to accept.
        let check = Connection::open(registry_db_path().unwrap()).unwrap();
        assert_eq!(
            check
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            3
        );
        assert_eq!(
            check
                .pragma_query_value(None, "application_id", |row| row.get::<_, i64>(0))
                .unwrap(),
            APPLICATION_ID
        );
    }

    #[test]
    fn record_uninstalled_tombstones_and_reinstall_clears() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-registry-uninstall");
        record_installed("future-x", "1.0.0").unwrap();
        record_uninstalled("future-x").unwrap();
        // Tombstone keeps the last-known version.
        assert_eq!(registry_row("future-x"), (Some("1.0.0".to_string()), true));

        record_installed("future-x", "2.0.0").unwrap();
        assert_eq!(registry_row("future-x"), (Some("2.0.0".to_string()), false));
    }

    #[test]
    fn record_uninstalled_of_an_unknown_skill_creates_a_tombstone() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-registry-unknown");
        record_uninstalled("never-installed").unwrap();
        assert_eq!(registry_row("never-installed"), (None, true));
    }

    #[test]
    fn foreign_databases_are_refused() {
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
    fn unrecognized_databases_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.db");
        let connection = Connection::open(&path).unwrap();
        connection.execute("CREATE TABLE mystery (x)", []).unwrap();
        drop(connection);
        assert!(open_registry(&path).is_err());
    }

    #[test]
    fn future_schema_versions_are_refused() {
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
}
