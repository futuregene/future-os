//! Offline, coordinated history backups and resumable multi-file restoration.
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

use crate::Output;

pub const HELP: &str = "future data — offline history backup and recovery\n\nUsage:\n  future data backup <new-directory>\n  future data verify <backup-directory>\n  future data restore <backup-directory> --yes\n  future data resume\n  future data rollback\n\nStop Desktop, Agent and other writers first. Backups include both databases,\nmanaged attachments/workspaces/review data, legacy sources and Agent configuration\n(including credentials). Keep the backup private. Restore targets the original\nFutureOS home; paths and user workspaces retain their original meaning.\nIncomplete restores fence startup until resume or rollback. The previous files\nremain in the restore work directory even after success.";

const ROOTS: &[&str] = &[
    "agent/agent.db-wal",
    "agent/agent.db-shm",
    "agent/agent.db",
    "app/app.db-wal",
    "app/app.db-shm",
    "app/app.db",
    "app/images",
    "app/review",
    "workspaces/chat",
    "agent/sessions",
    "agent/run-events",
    "agent/session-backups",
    "agent/settings.json",
    "agent/models.json",
    "agent/auth.json",
];

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileRecord {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    format: String,
    version: u32,
    id: String,
    source_home: PathBuf,
    directories: Vec<String>,
    files: Vec<FileRecord>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreItem {
    path: String,
    had_original: bool,
    has_replacement: bool,
    #[serde(default)]
    original_digest: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreJournal {
    version: u32,
    id: String,
    rollback: bool,
    items: Vec<RestoreItem>,
}

pub fn data(command: Option<&str>, args: &[String], out: &Output) -> Result<(), String> {
    let run = || -> Result<()> {
        if args
            .iter()
            .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
        {
            out.log(HELP);
            return Ok(());
        }
        let home = future_rpc::home::future_home();
        match (command, args) {
            (None | Some("--help" | "-h"), []) => out.log(HELP),
            (Some("backup"), [destination]) => with_stopped_home(&home, || {
                future_rpc::home::ensure_restore_complete(&home)?;
                backup(&home, Path::new(destination))?;
                out.log(&format!("Verified backup: {destination}"));
                Ok(())
            })?,
            (Some("verify"), [source]) => {
                let manifest = verify(Path::new(source))?;
                out.log(&format!(
                    "Verified backup {}: {} files",
                    manifest.id,
                    manifest.files.len()
                ));
            }
            (Some("restore"), [source, yes]) if yes == "--yes" => with_stopped_home(&home, || {
                begin_restore(&home, Path::new(source))?;
                out.log("Restore completed. Previous files are retained for rollback inspection.");
                Ok(())
            })?,
            (Some("resume" | "rollback"), []) => with_stopped_home(&home, || {
                finish_restore(&home, command == Some("rollback"))?;
                out.log("Recovery operation completed; startup is available again.");
                Ok(())
            })?,
            _ => bail!("Invalid data command.\n{HELP}"),
        }
        Ok(())
    };
    run().map_err(|error| format!("{error:#}"))
}

fn with_stopped_home<T>(home: &Path, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    ensure!(home.is_dir(), "FutureOS home does not exist");
    fs::create_dir_all(home.join("app"))?;
    fs::create_dir_all(home.join("agent"))?;
    let open_lock = |path| {
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
    };
    let desktop_path = home.join("app/desktop.lock");
    let desktop = open_lock(&desktop_path)?;
    fs2::FileExt::try_lock_exclusive(&desktop).map_err(|error| {
        lock_failure(
            &error,
            &desktop_path,
            "Desktop is running; quit it before data maintenance",
        )
    })?;
    let agent_path = home.join("agent/agent-instance.lock");
    let mut agent = fd_lock::RwLock::new(open_lock(&agent_path)?);
    let _guard = agent.try_write().map_err(|error| {
        lock_failure(
            &error,
            &agent_path,
            "Agent is running; stop it before data maintenance",
        )
    })?;
    operation()
}

/// Why an exclusive lock could not be taken, in the terms the user can act on.
///
/// Two different failures arrive through the same `io::Error`. One is the process
/// this lock exists to detect — the Desktop or the Agent is up — and the fix is to
/// quit it. The other is the file system declining to lock at all: `ENOLCK` on a
/// network share without a lock manager, `ENOTSUP` on some FUSE mounts. Maintenance
/// is refused either way, because it must not run against a live process and this
/// is the only proof available, but reporting the second as "Desktop is running"
/// sends the user to look for a process that does not exist — and quitting it, over
/// and over, will never let the command through. `fs2` and `fd-lock` both surface
/// the platform's own errno here, so one comparison covers the two callers.
/// `host/src/instance.rs` draws the same line for the Desktop's own startup lock.
fn lock_failure(error: &std::io::Error, path: &Path, contended: &str) -> anyhow::Error {
    if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() {
        anyhow::anyhow!(contended.to_string())
    } else {
        anyhow::anyhow!(
            "this file system will not lock {} ({error}), so data maintenance cannot tell \
             whether a Desktop or an Agent is running against it. Move the FutureOS home to a \
             local disk (or one whose file system supports locks) and try again.",
            path.display()
        )
    }
}

fn relative(root: &Path, logical: &str) -> Result<PathBuf> {
    ensure!(
        !logical.is_empty() && !logical.contains(['\\', ':']),
        "invalid backup path"
    );
    let path = Path::new(logical);
    ensure!(
        path.components()
            .all(|part| matches!(part, Component::Normal(_))),
        "invalid backup path"
    );
    let directory_roots = [
        "app/images",
        "app/review",
        "workspaces/chat",
        "agent/sessions",
        "agent/run-events",
        "agent/session-backups",
    ];
    let allowed = ROOTS.contains(&logical)
        || directory_roots.iter().any(|base| {
            logical
                .strip_prefix(base)
                .is_some_and(|rest| rest.starts_with('/'))
        });
    ensure!(
        allowed
            && ![
                "agent/agent.db-wal",
                "agent/agent.db-shm",
                "app/app.db-wal",
                "app/app.db-shm"
            ]
            .contains(&logical),
        "unexpected backup path"
    );
    without_links(root, path)
}

fn without_links(root: &Path, path: &Path) -> Result<PathBuf> {
    let mut current = root.to_path_buf();
    for part in path.components() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(meta) => ensure!(
                !meta.file_type().is_symlink(),
                "backup path contains a symbolic link"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(current)
}

fn hash(path: &Path) -> Result<(u64, String)> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    let mut bytes = 0;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        digest.update(&buffer[..count]);
    }
    Ok((bytes, format!("{:x}", digest.finalize())))
}

/// A deterministic preimage check for rollback. Links in the old tree are
/// hashed as links, never followed; the original tree is moved intact.
fn root_digest(path: &Path) -> Result<Option<String>> {
    fn field(digest: &mut Sha256, bytes: &[u8]) {
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    fn visit(path: &Path, digest: &mut Sha256) -> Result<()> {
        let meta = fs::symlink_metadata(path)?;
        if meta.file_type().is_symlink() {
            field(digest, b"link");
            let target = fs::read_link(path)?;
            field(
                digest,
                target
                    .to_str()
                    .context("restore preimage link is not UTF-8")?
                    .as_bytes(),
            );
        } else if meta.is_dir() {
            field(digest, b"directory");
            let mut children = fs::read_dir(path)?
                .map(|entry| {
                    let entry = entry?;
                    let name = entry
                        .file_name()
                        .into_string()
                        .map_err(|_| anyhow::anyhow!("restore preimage name is not UTF-8"))?;
                    Ok((name, entry.path()))
                })
                .collect::<Result<Vec<_>>>()?;
            children.sort_by(|a, b| a.0.cmp(&b.0));
            field(digest, &(children.len() as u64).to_le_bytes());
            for (name, child) in children {
                field(digest, name.as_bytes());
                visit(&child, digest)?;
            }
        } else {
            ensure!(meta.is_file(), "unsupported restore preimage file type");
            field(digest, b"file");
            let (bytes, sha256) = hash(path)?;
            field(digest, &bytes.to_le_bytes());
            field(digest, sha256.as_bytes());
        }
        Ok(())
    }
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let mut digest = Sha256::new();
    visit(path, &mut digest)?;
    Ok(Some(format!("{:x}", digest.finalize())))
}

/// File contents are synced when copied. Sync nested directory entries before
/// publishing a complete tree or a restore fence that depends on that tree.
fn sync_tree(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                sync_tree(&entry.path())?;
            }
        }
        fs::File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn prepare_switch_parents(home: &Path, directory: &Path) -> Result<()> {
    ensure!(directory.is_dir(), "restore work directory is missing");
    for path in ROOTS {
        let parent = Path::new(path)
            .parent()
            .context("restore root has no parent")?;
        fs::create_dir_all(without_links(home, parent)?)?;
        fs::create_dir_all(without_links(
            directory,
            &Path::new("previous").join(parent),
        )?)?;
    }
    // Persist each destination's parent chain before an original is renamed.
    sync_tree(directory)?;
    sync_parent(directory)
}

fn copy_private(source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::fs::PermissionsExt;
        options.mode(0o600 | (fs::metadata(source)?.permissions().mode() & 0o100));
    }
    let mut target = options.open(destination)?;
    std::io::copy(&mut fs::File::open(source)?, &mut target)?;
    target.sync_all()?;
    Ok(())
}

fn collect(root: &Path, logical: &str, destination: &Path, manifest: &mut Manifest) -> Result<()> {
    let source = relative(root, logical)?;
    let metadata = match fs::symlink_metadata(&source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_dir() {
        manifest.directories.push(logical.into());
        fs::create_dir_all(relative(destination, logical)?)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("backup file name is not UTF-8"))?;
            collect(root, &format!("{logical}/{name}"), destination, manifest)?;
        }
    } else {
        ensure!(metadata.is_file(), "unsupported managed file type");
        let target = relative(destination, logical)?;
        copy_private(&source, &target)?;
        let (bytes, sha256) = hash(&target)?;
        ensure!(
            hash(&source)? == (bytes, sha256.clone()),
            "managed file changed during backup"
        );
        manifest.files.push(FileRecord {
            path: logical.into(),
            bytes,
            sha256,
        });
    }
    Ok(())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    use std::io::Write;
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().context("metadata has no parent")?)?;
    temporary.write_all(&serde_json::to_vec_pretty(value)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    sync_parent(path)?;
    Ok(())
}

fn sync_parent(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(path.parent().context("path has no parent")?)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn rename_durable(source: &Path, target: &Path) -> Result<()> {
    fs::rename(source, target)?;
    sync_parent(source)?;
    sync_parent(target)?;
    Ok(())
}

fn backup(home: &Path, destination: &Path) -> Result<()> {
    ensure!(!destination.exists(), "backup destination already exists");
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let home = home.canonicalize()?;
    let parent = parent.canonicalize()?;
    ensure!(
        !parent.starts_with(&home),
        "place complete backups outside the FutureOS home"
    );
    let staging = tempfile::Builder::new()
        .prefix(".future-backup-")
        .tempdir_in(parent)?;
    let mut manifest = Manifest {
        format: "futureos-history-backup".into(),
        version: 1,
        id: future_app_workspaces::new_id("backup"),
        source_home: home.clone(),
        directories: Vec::new(),
        files: Vec::new(),
    };
    let mut writers = Vec::new();
    // Reserve both databases before taking either snapshot. Other CLI SQLite
    // writers cannot commit between the two snapshots or managed-file inventory.
    for logical in ["agent/agent.db", "app/app.db"] {
        let source = without_links(&home, Path::new(logical))?;
        if source.exists() {
            let conn = Connection::open_with_flags(&source, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
            conn.busy_timeout(std::time::Duration::from_secs(5))?;
            conn.execute_batch("BEGIN IMMEDIATE")?;
            writers.push((logical, conn));
        }
    }
    for (logical, _) in &writers {
        let source =
            Connection::open_with_flags(home.join(logical), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let folder = staging.path().join(
            Path::new(logical)
                .parent()
                .context("database has no parent")?,
        );
        let snapshot =
            future_agent::session::create_database_snapshot(&source, &folder, "snapshot")?;
        let target = staging.path().join(logical);
        rename_durable(&snapshot, &target)?;
        let (bytes, sha256) = hash(&target)?;
        manifest.files.push(FileRecord {
            path: (*logical).into(),
            bytes,
            sha256,
        });
    }
    for logical in ROOTS.iter().filter(|logical| !logical.contains(".db")) {
        collect(&home, logical, staging.path(), &mut manifest)?;
    }
    manifest.files.sort_by(|a, b| a.path.cmp(&b.path));
    manifest.directories.sort();
    write_json(&staging.path().join("manifest.json"), &manifest)?;
    verify(staging.path())?;
    sync_tree(staging.path())?;
    ensure!(
        !destination.exists(),
        "backup destination appeared during backup"
    );
    rename_durable(staging.path(), destination)?;
    Ok(())
}

fn verify(source: &Path) -> Result<Manifest> {
    let manifest: Manifest = serde_json::from_slice(&fs::read(source.join("manifest.json"))?)?;
    ensure!(
        manifest.format == "futureos-history-backup" && manifest.version == 1,
        "unsupported backup format"
    );
    verify_files(source, &manifest)?;
    Ok(manifest)
}

fn verify_files(source: &Path, manifest: &Manifest) -> Result<()> {
    verify_files_at(manifest, ROOTS, |logical| relative(source, logical))
}

fn verify_files_at(
    manifest: &Manifest,
    roots: &[&str],
    resolve: impl Fn(&str) -> Result<PathBuf>,
) -> Result<()> {
    let mut files = HashSet::new();
    let mut directories = HashSet::new();
    for directory in &manifest.directories {
        ensure!(
            directories.insert(directory.clone()),
            "duplicate backup directory"
        );
        ensure!(resolve(directory)?.is_dir(), "backup directory is missing");
    }
    for file in &manifest.files {
        ensure!(files.insert(file.path.clone()), "duplicate backup file");
        let path = resolve(&file.path)?;
        ensure!(
            path.is_file() && hash(&path)? == (file.bytes, file.sha256.clone()),
            "backup file verification failed: {}",
            file.path
        );
        if file.path == "agent/agent.db" || file.path == "app/app.db" {
            // A backup is a standalone SQLite snapshot. Unmanifested sidecars
            // must not override its hashed contents during verification.
            for suffix in ["-wal", "-shm", "-journal"] {
                let sidecar = path.with_file_name(format!(
                    "{}{suffix}",
                    path.file_name()
                        .context("database has no filename")?
                        .to_string_lossy()
                ));
                ensure!(
                    fs::symlink_metadata(sidecar)
                        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
                    "backup database contains an unexpected SQLite sidecar"
                );
            }
            let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            let check: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
            ensure!(check == "ok", "backup database integrity check failed");
            let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
            if file.path == "app/app.db" {
                future_app_settings::validate_database_header(&conn)?;
            } else {
                let application: i64 =
                    conn.pragma_query_value(None, "application_id", |row| row.get(0))?;
                ensure!(
                    application == 0x46555452 && [0, 2, 3, 4, 5, 6, 7].contains(&version),
                    "unsupported Agent backup database"
                );
            }
        }
    }
    let mut actual_files = HashSet::new();
    let mut actual_directories = HashSet::new();
    for root in roots
        .iter()
        .filter(|root| !root.ends_with("-wal") && !root.ends_with("-shm"))
    {
        inventory(
            &resolve(root)?,
            root,
            &mut actual_files,
            &mut actual_directories,
        )?;
    }
    ensure!(
        actual_files == files && actual_directories == directories,
        "backup inventory disagrees with its manifest; no files may be silently omitted"
    );
    Ok(())
}

/// Compare the actual managed tree as well as each listed hash. An omitted
/// record must not turn a present attachment/history source into absent data.
fn inventory(
    path: &Path,
    logical: &str,
    files: &mut HashSet<String>,
    directories: &mut HashSet<String>,
) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        !metadata.file_type().is_symlink(),
        "backup path contains a symbolic link"
    );
    if metadata.is_dir() {
        directories.insert(logical.into());
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("backup file name is not UTF-8"))?;
            inventory(
                &entry.path(),
                &format!("{logical}/{name}"),
                files,
                directories,
            )?;
        }
    } else {
        ensure!(metadata.is_file(), "unsupported managed file type");
        files.insert(logical.into());
    }
    Ok(())
}

fn pending(home: &Path) -> PathBuf {
    home.join(future_rpc::home::RESTORE_PENDING_FILE)
}
fn work(home: &Path, id: &str) -> Result<PathBuf> {
    ensure!(
        !id.is_empty()
            && id.len() <= 100
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
        "invalid restore identity"
    );
    let directory = home.join(format!(".restore-work-{id}"));
    match fs::symlink_metadata(&directory) {
        Ok(meta) => ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "invalid restore work directory"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(directory)
}

fn begin_restore(home: &Path, source: &Path) -> Result<()> {
    prepare_restore(home, source)?;
    finish_restore(home, false)
}

fn prepare_restore(home: &Path, source: &Path) -> Result<()> {
    future_rpc::home::ensure_restore_complete(home)?;
    let mut manifest = verify(source)?;
    ensure!(manifest.source_home == home.canonicalize()?, "restore must target the original FutureOS home; managed working-directory paths are retained");
    let id = future_app_workspaces::new_id("restore");
    let directory = work(home, &id)?;
    fs::create_dir(&directory)?;
    let incoming = directory.join("incoming");
    fs::create_dir(&incoming)?;
    for directory in &manifest.directories {
        fs::create_dir_all(relative(&incoming, directory)?)?;
    }
    for file in &manifest.files {
        copy_private(
            &relative(source, &file.path)?,
            &relative(&incoming, &file.path)?,
        )?;
    }
    verify_files(&incoming, &manifest)?;
    // Rotate only the restore identity, never message/run ids or original bodies.
    let app = incoming.join("app/app.db");
    if app.exists() {
        let conn = Connection::open(app)?;
        let identity: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='app_database_identity')",
            [],
            |row| row.get(0),
        )?;
        if identity {
            conn.execute(
                "UPDATE app_database_identity SET restore_generation=?1",
                [&id],
            )?;
        }
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    }
    let agent = incoming.join("agent/agent.db");
    if agent.exists() {
        let conn = Connection::open(agent)?;
        let metadata: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='storage_meta')",
            [],
            |row| row.get(0),
        )?;
        if metadata {
            conn.execute("INSERT INTO storage_meta(key,value) VALUES ('restore_generation',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[&id])?;
        }
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    }
    for record in &mut manifest.files {
        if record.path.ends_with(".db") {
            (record.bytes, record.sha256) = hash(&relative(&incoming, &record.path)?)?;
        }
    }
    write_json(&directory.join("restore-manifest.json"), &manifest)?;
    let mut items = Vec::new();
    for path in ROOTS {
        // Only enumerated managed roots are switched; application directories,
        // singleton locks and user-selected workspaces never move.
        let original = without_links(home, Path::new(path))?;
        let replacement = incoming.join(path);
        if let Ok(meta) = fs::symlink_metadata(&original) {
            ensure!(
                !meta.file_type().is_symlink(),
                "managed restore root is a symbolic link"
            );
        }
        items.push(RestoreItem {
            path: (*path).into(),
            had_original: original.exists(),
            has_replacement: replacement.exists(),
            original_digest: root_digest(&original)?,
        });
    }
    let journal = RestoreJournal {
        version: 2,
        id,
        rollback: false,
        items,
    };
    prepare_switch_parents(home, &directory)?;
    write_json(&pending(home), &journal)?;
    Ok(())
}

fn finish_restore(home: &Path, rollback: bool) -> Result<()> {
    finish_restore_with(home, rollback, || Ok(()))
}

fn finish_restore_with(
    home: &Path,
    rollback: bool,
    mut after_move: impl FnMut() -> Result<()>,
) -> Result<()> {
    let mut journal: RestoreJournal = serde_json::from_slice(&fs::read(pending(home))?)?;
    ensure!(
        matches!(journal.version, 1 | 2) && journal.items.len() == ROOTS.len(),
        "invalid restore journal"
    );
    for (item, path) in journal.items.iter().zip(ROOTS) {
        ensure!(item.path == *path, "invalid restore target");
        if journal.version == 2 {
            ensure!(
                item.had_original == item.original_digest.is_some(),
                "restore preimage evidence is missing"
            );
        }
    }
    let directory = work(home, &journal.id)?;
    for item in &journal.items {
        without_links(home, Path::new(&item.path))?;
        without_links(&directory, &Path::new("previous").join(&item.path))?;
        without_links(&directory, &Path::new("incoming").join(&item.path))?;
    }
    // Check every retained preimage before performing another rename. A lost
    // previous root must never be mistaken for a completed rollback.
    if journal.version == 2 {
        for item in journal.items.iter().filter(|item| item.had_original) {
            let previous = directory.join("previous").join(&item.path);
            let candidate = if previous.exists() {
                previous
            } else {
                home.join(&item.path)
            };
            ensure!(
                root_digest(&candidate)? == item.original_digest,
                "restore preimage changed or is missing: {}; startup remains fenced",
                item.path
            );
        }
    }
    if !rollback && !journal.rollback {
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(directory.join("restore-manifest.json"))?)?;
        for item in &journal.items {
            let listed = manifest
                .files
                .iter()
                .map(|file| file.path.as_str())
                .chain(manifest.directories.iter().map(String::as_str))
                .any(|path| {
                    path == item.path
                        || path
                            .strip_prefix(&item.path)
                            .is_some_and(|tail| tail.starts_with('/'))
                });
            ensure!(
                listed == item.has_replacement,
                "restore replacement disagrees with its manifest"
            );
            if item.has_replacement
                && item.had_original
                && !directory.join("previous").join(&item.path).exists()
            {
                ensure!(
                    directory.join("incoming").join(&item.path).exists(),
                    "restore replacement is missing; startup remains fenced"
                );
            }
        }
        let roots: Vec<_> = journal
            .items
            .iter()
            .filter(|item| item.has_replacement)
            .map(|item| item.path.as_str())
            .collect();
        // A crash may have installed some roots already. Verify every remaining
        // or installed replacement before moving any more originals.
        verify_files_at(&manifest, &roots, |logical| {
            let item = journal
                .items
                .iter()
                .find(|item| {
                    logical == item.path
                        || logical
                            .strip_prefix(&item.path)
                            .is_some_and(|tail| tail.starts_with('/'))
                })
                .context("unexpected restore replacement path")?;
            let base = if directory.join("incoming").join(&item.path).exists() {
                directory.join("incoming")
            } else {
                home.to_path_buf()
            };
            relative(&base, logical)
        })?;
    }
    prepare_switch_parents(home, &directory)?;
    if rollback && !journal.rollback {
        journal.rollback = true;
        write_json(&pending(home), &journal)?;
    }
    for item in &journal.items {
        let original = home.join(&item.path);
        let previous = directory.join("previous").join(&item.path);
        let incoming = directory.join("incoming").join(&item.path);
        if journal.rollback {
            if item.has_replacement && original.exists() && !incoming.exists() {
                if let Some(parent) = incoming.parent() {
                    fs::create_dir_all(parent)?;
                }
                rename_durable(&original, &incoming)?;
                after_move()?;
            }
            if item.had_original && previous.exists() {
                if let Some(parent) = original.parent() {
                    fs::create_dir_all(parent)?;
                }
                rename_durable(&previous, &original)?;
                after_move()?;
            }
        } else {
            if item.had_original && !previous.exists() {
                if let Some(parent) = previous.parent() {
                    fs::create_dir_all(parent)?;
                }
                rename_durable(&original, &previous)?;
                after_move()?;
            }
            if item.has_replacement && incoming.exists() {
                if let Some(parent) = original.parent() {
                    fs::create_dir_all(parent)?;
                }
                ensure!(
                    !original.exists(),
                    "restore target changed while maintenance was pending"
                );
                rename_durable(&incoming, &original)?;
                after_move()?;
            }
            ensure!(
                !item.has_replacement || original.exists(),
                "restored replacement is missing"
            );
        }
    }
    if journal.rollback {
        for item in &journal.items {
            let original = home.join(&item.path);
            ensure!(
                original.exists() == item.had_original,
                "rollback is incomplete: {}; startup remains fenced",
                item.path
            );
            if journal.version == 2 {
                ensure!(
                    root_digest(&original)? == item.original_digest,
                    "rollback preimage verification failed: {}; startup remains fenced",
                    item.path
                );
            }
        }
    } else {
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(directory.join("restore-manifest.json"))?)?;
        verify_files(home, &manifest)?;
    }
    rename_durable(
        &pending(home),
        &directory.join(if journal.rollback {
            "rolled-back.json"
        } else {
            "completed.json"
        }),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &Path) -> PathBuf {
        let home = root.join("home");
        fs::create_dir_all(home.join("app/images/t")).unwrap();
        fs::create_dir_all(home.join("workspaces/chat/t")).unwrap();
        fs::write(home.join("app/images/t/original.png"), b"old image").unwrap();
        fs::write(home.join("workspaces/chat/t/note.txt"), b"old workspace").unwrap();
        let manager = future_agent::session::Manager::new(home.join("agent/sessions"));
        manager.initialize().unwrap();
        drop(manager);
        let app = Connection::open(home.join("app/app.db")).unwrap();
        app.execute_batch(include_str!("../../../host/src/store/fixtures/v1.2.1.sql"))
            .unwrap();
        app.execute_batch("INSERT INTO workspaces(id,name,kind,path,created_at,updated_at) VALUES('w','W','user','.',1,1);
            INSERT INTO threads(id,workspace_id,mode,title,created_at,updated_at) VALUES('t','w','chat','old title',1,1);").unwrap();
        home
    }

    fn change_current(home: &Path) {
        fs::write(home.join("app/images/t/original.png"), b"new image").unwrap();
        Connection::open(home.join("app/app.db"))
            .unwrap()
            .execute("UPDATE threads SET title='new title'", [])
            .unwrap();
    }

    fn assert_title(home: &Path, expected: &str) {
        let actual: String = Connection::open(home.join("app/app.db"))
            .unwrap()
            .query_row("SELECT title FROM threads WHERE id='t'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn omitted_records_and_unlisted_managed_files_refuse_restore() {
        for omit in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let home = fixture(root.path());
            let archive = root.path().join("archive");
            backup(&home, &archive).unwrap();
            if omit {
                let mut manifest = verify(&archive).unwrap();
                manifest
                    .files
                    .retain(|file| file.path != "app/images/t/original.png");
                write_json(&archive.join("manifest.json"), &manifest).unwrap();
            } else {
                fs::create_dir_all(archive.join("agent/sessions")).unwrap();
                fs::write(
                    archive.join("agent/sessions/unlisted.jsonl"),
                    b"historical input",
                )
                .unwrap();
            }
            assert!(verify(&archive)
                .unwrap_err()
                .to_string()
                .contains("inventory"));
            assert!(begin_restore(&home, &archive).is_err());
            assert!(!pending(&home).exists());
            assert_title(&home, "old title");
        }
    }

    #[test]
    fn damaged_replacement_is_rejected_before_another_original_moves() {
        let root = tempfile::tempdir().unwrap();
        let home = fixture(root.path());
        let archive = root.path().join("archive");
        backup(&home, &archive).unwrap();
        change_current(&home);
        prepare_restore(&home, &archive).unwrap();
        assert!(finish_restore_with(&home, false, || bail!("interrupted")).is_err());
        let journal: RestoreJournal =
            serde_json::from_slice(&fs::read(pending(&home)).unwrap()).unwrap();
        let directory = work(&home, &journal.id).unwrap();
        fs::write(
            directory.join("incoming/app/images/t/original.png"),
            b"damaged",
        )
        .unwrap();
        let mut moves = 0;
        assert!(finish_restore_with(&home, false, || {
            moves += 1;
            Ok(())
        })
        .is_err());
        assert_eq!(moves, 0);
        assert_title(&home, "new title");
        assert!(pending(&home).exists());
        // Rollback depends on the verified originals, not a damaged replacement.
        finish_restore(&home, true).unwrap();
        assert_title(&home, "new title");
        assert_eq!(
            fs::read(home.join("app/images/t/original.png")).unwrap(),
            b"new image"
        );
    }

    #[test]
    fn rollback_can_itself_be_interrupted_at_every_move() {
        for stop_after in 1..=ROOTS.len() * 2 {
            let root = tempfile::tempdir().unwrap();
            let home = fixture(root.path());
            let archive = root.path().join("archive");
            backup(&home, &archive).unwrap();
            change_current(&home);
            prepare_restore(&home, &archive).unwrap();
            // Leave the forward switch complete but its fence unacknowledged.
            let journal: RestoreJournal =
                serde_json::from_slice(&fs::read(pending(&home)).unwrap()).unwrap();
            let total: usize = journal
                .items
                .iter()
                .map(|item| usize::from(item.had_original) + usize::from(item.has_replacement))
                .sum();
            if stop_after > total {
                break;
            }
            let mut forward = 0;
            assert!(finish_restore_with(&home, false, || {
                forward += 1;
                if forward == total {
                    bail!("exit before completion acknowledgement");
                }
                Ok(())
            })
            .is_err());
            let mut backward = 0;
            assert!(finish_restore_with(&home, true, || {
                backward += 1;
                if backward == stop_after {
                    bail!("exit during rollback");
                }
                Ok(())
            })
            .is_err());
            assert!(pending(&home).exists());
            // Resume must honor the persisted rollback direction.
            finish_restore(&home, false).unwrap();
            assert!(!pending(&home).exists());
            assert_title(&home, "new title");
            assert_eq!(
                fs::read(home.join("app/images/t/original.png")).unwrap(),
                b"new image"
            );
        }
    }

    #[test]
    fn unmanifested_sqlite_sidecars_are_not_accepted_as_backup_contents() {
        let root = tempfile::tempdir().unwrap();
        let home = fixture(root.path());
        let archive = root.path().join("archive");
        backup(&home, &archive).unwrap();
        fs::write(
            archive.join("agent/agent.db-wal"),
            b"unexpected committed pages",
        )
        .unwrap();
        assert!(verify(&archive)
            .unwrap_err()
            .to_string()
            .contains("sidecar"));
        assert!(begin_restore(&home, &archive).is_err());
        assert!(!pending(&home).exists());
        assert_title(&home, "old title");
    }

    #[cfg(unix)]
    #[test]
    fn linked_restore_parents_never_move_an_external_workspace() {
        let root = tempfile::tempdir().unwrap();
        let home = fixture(root.path());
        let archive = root.path().join("archive");
        backup(&home, &archive).unwrap();
        let external = root.path().join("external");
        fs::create_dir_all(external.join("chat")).unwrap();
        fs::write(external.join("chat/keep.txt"), b"external data").unwrap();
        fs::rename(
            home.join("workspaces"),
            root.path().join("original-workspaces"),
        )
        .unwrap();
        std::os::unix::fs::symlink(&external, home.join("workspaces")).unwrap();
        assert!(prepare_restore(&home, &archive).is_err());
        assert!(!pending(&home).exists());
        assert_eq!(
            fs::read(external.join("chat/keep.txt")).unwrap(),
            b"external data"
        );
    }

    #[test]
    fn missing_preimage_never_clears_the_restore_fence() {
        let root = tempfile::tempdir().unwrap();
        let home = fixture(root.path());
        let archive = root.path().join("archive");
        backup(&home, &archive).unwrap();
        change_current(&home);
        prepare_restore(&home, &archive).unwrap();
        assert!(finish_restore_with(&home, false, || bail!("exit after first rename")).is_err());
        let journal: RestoreJournal =
            serde_json::from_slice(&fs::read(pending(&home)).unwrap()).unwrap();
        let previous = work(&home, &journal.id)
            .unwrap()
            .join("previous/agent/agent.db");
        let bytes = fs::read(&previous).unwrap();
        fs::remove_file(&previous).unwrap();
        assert!(finish_restore(&home, true)
            .unwrap_err()
            .to_string()
            .contains("preimage"));
        assert!(future_rpc::home::ensure_restore_complete(&home).is_err());
        assert_title(&home, "new title");
        fs::write(&previous, bytes).unwrap();
        finish_restore(&home, true).unwrap();
        future_rpc::home::ensure_restore_complete(&home).unwrap();
        assert_title(&home, "new title");
    }

    #[test]
    fn version_one_pending_restores_remain_resumable() {
        for rollback in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let home = fixture(root.path());
            let archive = root.path().join("archive");
            backup(&home, &archive).unwrap();
            change_current(&home);
            prepare_restore(&home, &archive).unwrap();
            let mut journal: serde_json::Value =
                serde_json::from_slice(&fs::read(pending(&home)).unwrap()).unwrap();
            journal["version"] = 1.into();
            for item in journal["items"].as_array_mut().unwrap() {
                item.as_object_mut().unwrap().remove("originalDigest");
            }
            write_json(&pending(&home), &journal).unwrap();
            assert!(finish_restore_with(&home, false, || bail!("exit after rename")).is_err());
            finish_restore(&home, rollback).unwrap();
            assert_title(&home, if rollback { "new title" } else { "old title" });
        }
    }

    #[test]
    fn complete_backup_restores_both_databases_and_managed_files() {
        let root = tempfile::tempdir().unwrap();
        let home = fixture(root.path());
        let archive = root.path().join("archive");
        with_stopped_home(&home, || backup(&home, &archive)).unwrap();
        let manifest = verify(&archive).unwrap();
        assert!(manifest
            .files
            .iter()
            .any(|file| file.path == "agent/agent.db"));
        assert!(manifest.files.iter().any(|file| file.path == "app/app.db"));
        change_current(&home);
        with_stopped_home(&home, || begin_restore(&home, &archive)).unwrap();
        assert_title(&home, "old title");
        assert_eq!(
            fs::read(home.join("app/images/t/original.png")).unwrap(),
            b"old image"
        );
        future_rpc::home::ensure_restore_complete(&home).unwrap();
    }

    #[test]
    fn every_move_boundary_can_resume_or_roll_back_without_losing_previous_data() {
        for rollback in [false, true] {
            // Fault after every old/new rename, before any acknowledgement.
            for stop_after in 1..=ROOTS.len() * 2 {
                let root = tempfile::tempdir().unwrap();
                let home = fixture(root.path());
                let archive = root.path().join("archive");
                backup(&home, &archive).unwrap();
                change_current(&home);
                prepare_restore(&home, &archive).unwrap();
                let journal: RestoreJournal =
                    serde_json::from_slice(&fs::read(pending(&home)).unwrap()).unwrap();
                let total_moves: usize = journal
                    .items
                    .iter()
                    .map(|item| usize::from(item.had_original) + usize::from(item.has_replacement))
                    .sum();
                if stop_after > total_moves {
                    break;
                }
                let mut moves = 0;
                assert!(finish_restore_with(&home, false, || {
                    moves += 1;
                    if moves == stop_after {
                        bail!("injected process exit");
                    }
                    Ok(())
                })
                .is_err());
                assert!(future_rpc::home::ensure_restore_complete(&home).is_err());
                finish_restore(&home, rollback).unwrap();
                assert_title(&home, if rollback { "new title" } else { "old title" });
                assert_eq!(
                    fs::read(home.join("app/images/t/original.png")).unwrap(),
                    if rollback { b"new image" } else { b"old image" }
                );
                future_rpc::home::ensure_restore_complete(&home).unwrap();
            }
        }
    }

    #[test]
    fn corruption_and_path_traversal_fail_before_touching_current_data() {
        let root = tempfile::tempdir().unwrap();
        let home = fixture(root.path());
        let archive = root.path().join("archive");
        backup(&home, &archive).unwrap();
        fs::write(archive.join("app/images/t/original.png"), b"damaged").unwrap();
        assert!(begin_restore(&home, &archive).is_err());
        assert!(!pending(&home).exists());
        assert_title(&home, "old title");
        for path in [
            "../outside",
            "app/images/../../outside",
            "/absolute",
            "C:\\outside",
        ] {
            assert!(relative(&archive, path).is_err());
        }
    }

    #[test]
    fn active_desktop_and_agent_ownership_prevent_data_maintenance() {
        let root = tempfile::tempdir().unwrap();
        let home = fixture(root.path());
        let desktop = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(home.join("app/desktop.lock"))
            .unwrap();
        fs2::FileExt::try_lock_exclusive(&desktop).unwrap();
        assert!(with_stopped_home(&home, || Ok(())).is_err());
        fs2::FileExt::unlock(&desktop).unwrap();
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(home.join("agent/agent-instance.lock"))
            .unwrap();
        let mut agent = fd_lock::RwLock::new(file);
        let _guard = agent.try_write().unwrap();
        assert!(with_stopped_home(&home, || Ok(())).is_err());
    }

    /// The two reasons a lock cannot be taken have different fixes, and the one
    /// that is *not* a running process must not send the reader to quit one.
    #[test]
    fn a_lock_the_file_system_refuses_is_not_reported_as_a_running_process() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("desktop.lock");

        let contended = lock_failure(
            &fs2::lock_contended_error(),
            &path,
            "Desktop is running; quit it before data maintenance",
        );
        assert_eq!(
            contended.to_string(),
            "Desktop is running; quit it before data maintenance"
        );

        let refused = lock_failure(
            &std::io::Error::from_raw_os_error(libc::EOPNOTSUPP),
            &path,
            "Desktop is running; quit it before data maintenance",
        );
        let text = refused.to_string();
        assert!(!text.contains("Desktop is running"), "{text}");
        assert!(text.contains("will not lock"), "{text}");
        assert!(text.contains(&path.display().to_string()), "{text}");
    }
}
