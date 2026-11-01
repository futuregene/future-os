//! Connection ownership, layout validation and durability settings.
use super::reclamation::configure_incremental_auto_vacuum;
use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use std::{path::Path, time::Duration};

#[cfg(test)]
pub(super) fn open_connection(path: &Path) -> Result<Connection> {
    open_connection_cancellable(path, &crate::service::StartupCancellation::default())
}

pub(super) fn open_connection_cancellable(
    path: &Path,
    cancellation: &crate::service::StartupCancellation,
) -> Result<Connection> {
    cancellation.check()?;
    if let Some(home) = path.parent().and_then(Path::parent) {
        future_rpc::home::ensure_restore_complete(home)?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create SQLite directory")?;
    }
    let mut connection = Connection::open(path).context("open Agent SQLite database")?;
    cancellation.monitor_sqlite(&connection)?;
    let result = initialize_connection(&mut connection, path, cancellation);
    // The startup token must not interrupt ordinary writes after readiness.
    connection.progress_handler(0, None::<fn() -> bool>)?;
    cancellation.check()?;
    result?;
    Ok(connection)
}

fn initialize_connection(
    connection: &mut Connection,
    path: &Path,
    cancellation: &crate::service::StartupCancellation,
) -> Result<()> {
    crate::session::content::install_functions(connection)?;
    // Compiling this schema's read paths is dominated by the deep views
    // (`entry_records`, `entry_header_records`, `history_shapes`): measured on
    // this connection, `SELECT … FROM entry_records` costs **26 ms to prepare
    // and 0.2 ms to execute** — a 130:1 ratio — and a trivial table read costs
    // 20 µs. rusqlite's cache holds 16 statements; this crate prepares ~130
    // distinct ones, so an LRU of 16 evicts the hot views between calls and
    // pays the 26 ms again. The cache is per connection and per statement, so
    // the ceiling only has to cover one connection's working set.
    connection.set_prepared_statement_cache_capacity(256);
    connection.busy_timeout(Duration::from_secs(5))?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let application: i64 =
        connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    if !super::supports_schema_version(version) {
        bail!("unsupported Agent database schema version {version}");
    }
    if application != 0 && application != 0x46555452 {
        bail!("database belongs to a different application");
    }
    if application == 0 {
        let tables: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if version != 0 || tables != 0 {
            bail!("refusing to initialize an unrecognized database");
        }
    }
    if application == 0x46555452 && version == 0 {
        let unknown: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table'
             AND name NOT LIKE 'sqlite_%' AND name NOT IN
             ('skills','skills_meta','skill_installations','skill_operations'))",
            [],
            |row| row.get(0),
        )?;
        if unknown {
            bail!("Agent database has an incomplete schema identity; preserve it for recovery");
        }
    }
    if version == 2 {
        let current_layout: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('entries') WHERE name='metadata_json') AND EXISTS(SELECT 1 FROM pragma_table_info('runs') WHERE name='run_sequence') AND EXISTS(SELECT 1 FROM pragma_table_info('runs') WHERE name='status') AND NOT EXISTS(SELECT 1 FROM pragma_table_info('runs') WHERE name='payload')",
            [],
            |row| row.get(0),
        )?;
        if !current_layout {
            bail!("unsupported pre-release Agent database layout; preserve agent.db and restore a supported backup");
        }
    }
    if version == super::CURRENT_SCHEMA_VERSION {
        // CREATE IF NOT EXISTS must never mask missing authoritative history.
        super::schema::validate_core(connection)?;
    } else if application != 0 {
        let bootstrap: bool = connection.query_row(
            "SELECT NOT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table'
             AND name NOT LIKE 'sqlite_%' AND name NOT IN
             ('skills','skills_meta','skill_installations','skill_operations'))",
            [],
            |row| row.get(0),
        )?;
        if !(bootstrap && matches!(version, 0 | 3 | 4)) {
            super::schema::validate_authoritative_history(connection, version)?;
        }
        let directory = path
            .parent()
            .context("Agent database has no parent")?
            .join("backups");
        cancellation.report(crate::service::StartupStage::BackingUpStorage);
        let snapshot = crate::session::database_backup::create_database_snapshot_cancellable(
            connection,
            &directory,
            "agent-before-schema-upgrade",
            cancellation,
        )?;
        tracing::info!(path = %snapshot.display(), "preserved Agent pre-upgrade snapshot");
    }
    // WAL initialization writes the database header. Configure a fresh file's
    // pointer-map layout before that first write, without a full VACUUM.
    cancellation.check()?;
    configure_incremental_auto_vacuum(connection)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    // FULL durability stays on the writer; PASSIVE checkpoints have their own
    // connection/thread so crossing a page count does not expand COMMIT work.
    connection.pragma_update(None, "wal_autocheckpoint", 0)?;
    connection.pragma_update(None, "journal_size_limit", 8 * 1024 * 1024)?;
    cancellation.check()?;
    if version != super::CURRENT_SCHEMA_VERSION && application != 0 {
        cancellation.report(crate::service::StartupStage::UpgradingStorage);
    }
    super::schema::initialize_schema(connection, cancellation)?;
    cancellation.check()?;
    Ok(())
}
