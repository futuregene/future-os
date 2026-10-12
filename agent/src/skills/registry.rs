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
/// the schema versions this build knows (0, 2, 3, 4, 5 and the current Agent schema).
///
/// A file created here is stamped `application_id` + `user_version = 4`; an
/// existing v3 database moves to v4 after the new tables are created.
pub(crate) fn open_registry(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create SQLite directory")?;
    }
    let mut connection = Connection::open(path).context("open Agent SQLite database")?;
    connection.busy_timeout(Duration::from_secs(5))?;
    let initial = {
        // Ownership and version must also come from one preflight snapshot;
        // a peer can initialize the file between two independent PRAGMA reads.
        let preflight = connection.transaction()?;
        let state = inspect_registry_database(&preflight)?;
        preflight.commit()?;
        state
    };
    if initial.fresh {
        // Configure a fresh file before WAL initializes its header. The
        // recognition/version checks are repeated under the write lock below.
        connection.pragma_update(None, "auto_vacuum", 2)?;
    }
    connection.pragma_update(None, "journal_mode", "WAL")?;
    // Settled registries are also opened by list queries. Do not reserve the
    // writer lock unless tables/indexes or the v3 stamp actually need updating.
    if initial.fresh || initial.version == 3 || !initial.schema_ready {
        initialize_registry_schema(&mut connection)?;
    }
    Ok(connection)
}

struct RegistryDatabaseState {
    fresh: bool,
    version: i64,
    schema_ready: bool,
}

fn inspect_registry_database(connection: &Connection) -> Result<RegistryDatabaseState> {
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
    } else if version != 0
        && version != 2
        && version != 3
        && version != 4
        && version != 5
        && version != crate::session::database::CURRENT_SCHEMA_VERSION
    {
        bail!("unsupported Agent database schema version {version}");
    }
    let schema_ready = connection.query_row(
        "SELECT count(*) = 5 FROM sqlite_schema
         WHERE (type='table' AND name IN
             ('skills','skills_meta','skill_installations','skill_operations'))
            OR (type='index' AND name='skill_installations_name')",
        [],
        |row| row.get(0),
    )?;
    Ok(RegistryDatabaseState {
        fresh,
        version,
        schema_ready,
    })
}

fn initialize_registry_schema(connection: &mut Connection) -> Result<()> {
    // Another process may have initialized/upgraded the file after preflight.
    // Read ownership and version only after reserving the write lock: a stale
    // fresh/v3 observation must never overwrite a newer supported schema.
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let current = inspect_registry_database(&tx)?;
    tx.execute_batch(SKILLS_TABLE_SQL)?;
    if current.fresh {
        tx.execute_batch(&format!(
            "PRAGMA application_id = {APPLICATION_ID};
             PRAGMA user_version = 4;"
        ))?;
    } else if current.version == 3 {
        tx.execute_batch("PRAGMA user_version = 4;")?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "registry_race_tests.rs"]
mod registry_race_tests;
