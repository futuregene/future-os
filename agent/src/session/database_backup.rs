//! Verified, independently readable SQLite upgrade snapshots shared by both stores.
use anyhow::{ensure, Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

pub fn create_database_snapshot(
    connection: &Connection,
    directory: &Path,
    prefix: &str,
) -> Result<PathBuf> {
    create_database_snapshot_cancellable(connection, directory, prefix, &Default::default())
}

pub(crate) fn create_database_snapshot_cancellable(
    connection: &Connection,
    directory: &Path,
    prefix: &str,
    cancellation: &crate::service::StartupCancellation,
) -> Result<PathBuf> {
    cancellation.check()?;
    let check: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    ensure!(
        check == "ok",
        "database integrity check failed; preserve the original database and WAL"
    );
    std::fs::create_dir_all(directory).context("create database snapshot directory")?;
    let destination = directory.join(format!("{prefix}-{}.db", uuid::Uuid::new_v4()));
    // Close the temporary file handle before SQLite opens that file.
    // tempfile creates it owner-only, and TempPath removes incomplete snapshots.
    let temporary = tempfile::Builder::new()
        .prefix(".sqlite-upgrade-")
        .tempfile_in(directory)?
        .into_temp_path();
    connection.execute(
        "VACUUM main INTO ?1",
        [temporary
            .to_str()
            .context("database snapshot path is not valid UTF-8")?],
    )?;
    cancellation.check()?;
    let snapshot = Connection::open_with_flags(&temporary, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    cancellation.monitor_sqlite(&snapshot)?;
    let checked = snapshot.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0));
    cancellation.check()?;
    let check = checked?;
    ensure!(check == "ok", "database snapshot integrity check failed");
    drop(snapshot);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&temporary)?
        .sync_all()?;
    cancellation.check()?;
    temporary
        .persist_noclobber(&destination)
        .context("publish verified database snapshot")?;
    #[cfg(unix)]
    {
        std::fs::File::open(directory)?.sync_all()?;
        if let Some(parent) = directory.parent() {
            std::fs::File::open(if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            })?
            .sync_all()?;
        }
    }
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancelled_upgrade_does_not_publish_an_incomplete_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let db = Connection::open(root.path().join("source.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE history(body TEXT); INSERT INTO history VALUES ('original');",
        )
        .unwrap();
        let cancellation = crate::service::StartupCancellation::default();
        cancellation.cancel();
        let error = create_database_snapshot_cancellable(
            &db,
            &root.path().join("backups"),
            "before",
            &cancellation,
        )
        .unwrap_err();
        assert!(error.is::<crate::service::StartupCancelled>());
        assert!(!root.path().join("backups").exists());
        assert_eq!(
            db.query_row("SELECT body FROM history", [], |row| row
                .get::<_, String>(0))
                .unwrap(),
            "original"
        );
    }

    #[test]
    fn snapshot_preserves_committed_wal_and_does_not_overwrite_prior_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let conn = Connection::open(root.path().join("source.db")).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.execute_batch(
            "CREATE TABLE history(body TEXT); INSERT INTO history VALUES('保留原文');",
        )
        .unwrap();
        let first = create_database_snapshot(&conn, root.path(), "before").unwrap();
        conn.execute("INSERT INTO history VALUES('new')", [])
            .unwrap();
        let second = create_database_snapshot(&conn, root.path(), "before").unwrap();
        assert_ne!(first, second);
        let backup = Connection::open(first).unwrap();
        let count: i64 = backup
            .query_row("SELECT count(*) FROM history", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(second).unwrap().permissions().mode() & 0o077,
                0
            );
        }
    }
}
