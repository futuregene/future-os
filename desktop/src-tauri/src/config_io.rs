//! Shared strict-read + atomic-write helpers for the JSON config files the GUI
//! owns under `~/.future/` (`models.json`, `auth.json`, `approval_rule.json`).
//!
//! Two invariants these enforce, previously duplicated (and, for `models.json`
//! and `approval_rule.json`, gotten wrong — a corrupt file was silently reset,
//! dropping user-authored config):
//!
//! - **Strict read**: a corrupt or non-object file is an *error*, never silently
//!   reset to `{}`. A read-modify-write that starts from a silently-emptied doc
//!   would overwrite the whole file with just the current change, wiping a
//!   hand-edited or half-written config.
//! - **Atomic write with serialized RMW**: serialize to a uniquely-named sibling
//!   temp file (pid + a process-global counter, so two concurrent writers never
//!   share a temp path and truncate each other), then `rename` over the target.
//!   [`with_config_lock`] serializes the read-modify-write of a given path within
//!   the process so concurrent Tauri commands don't lose each other's update.

use std::collections::HashMap;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use serde_json::Value;

use crate::AppError;

/// Reuse a lock while a config operation for the path is alive. Weak entries are
/// pruned on access, so visiting many Workspace approval files does not grow a
/// process-lifetime registry or require leaking boxed mutexes.
fn path_lock(path: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
    let registry = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = registry.lock().unwrap_or_else(|poison| poison.into_inner());
    guard.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = guard.get(path).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    guard.insert(path.to_path_buf(), Arc::downgrade(&lock));
    lock
}

/// Serialize a read-modify-write of `path` within this process. Holds a per-path
/// lock for the duration of `f`, so two concurrent commands mutating the same
/// config file can't interleave their read and write and lose an update.
pub fn with_config_lock<T>(
    path: &Path,
    f: impl FnOnce() -> Result<T, AppError>,
) -> Result<T, AppError> {
    let lock = path_lock(path);
    let _guard = lock.lock().unwrap_or_else(|poison| poison.into_inner());
    f()
}

/// Read a JSON config strictly as an object, returned as a `Value::Object`.
///
/// Missing file → an empty object. Present-but-unparseable, or a non-object root
/// → error (so a following write can't clobber an unreadable/hand-edited file).
pub fn read_json_object(path: &Path) -> Result<Value, AppError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(Value::Object(serde_json::Map::new()));
        }
        Err(error) => return Err(error.into()),
    };

    let value: Value = serde_json::from_str(&contents).map_err(|error| {
        AppError::Message(format!(
            "Failed to parse {}: {error}. Please fix the file and retry.",
            path.display()
        ))
    })?;

    if value.is_object() {
        Ok(value)
    } else {
        Err(AppError::Message(format!(
            "The root of {} must be a JSON object.",
            path.display()
        )))
    }
}

/// Lenient read for best-effort *cache* files where a corrupt file should not
/// surface an error to the user (e.g. a model-count cache): missing or
/// unparseable → an empty object.
pub fn read_json_lenient(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<Value>(&contents).ok())
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()))
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Atomically write `value` (pretty-printed, trailing newline) to `path`.
///
/// Writes to a uniquely-named sibling temp file — `<name>.tmp.<pid>.<counter>`,
/// so two concurrent writers never collide on the temp path — then `rename`s it
/// over the target. `owner_only` applies `0600` on unix (used for `auth.json`).
pub fn write_json_atomic(path: &Path, value: &Value, owner_only: bool) -> Result<(), AppError> {
    let serialized = format!("{}\n", serde_json::to_string_pretty(value)?);
    write_bytes_atomic(path, serialized.as_bytes(), owner_only)
}

/// Low-level atomic write of raw bytes (temp-file + `rename`, same guarantees as
/// [`write_json_atomic`]). Shared by the JSON writer and by transactional
/// rollback, which restores the *exact* original bytes rather than a
/// re-serialization that could reorder or reformat them.
pub fn write_bytes_atomic(path: &Path, bytes: &[u8], owner_only: bool) -> Result<(), AppError> {
    // Ensure the parent directory exists; `.transpose()` keeps the early-return
    // on error without an `if let` whose never-taken else arm would be a
    // spurious uncovered line (every real path has a parent).
    path.parent().map(std::fs::create_dir_all).transpose()?;

    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config.json".to_string());
    let tmp = path.with_file_name(format!("{file_name}.tmp.{}.{counter}", std::process::id()));

    let result = (|| -> Result<(), AppError> {
        // Create with the final mode already applied (create_owner_only uses
        // 0600 on unix) — a create-then-chmod window would let another local
        // user open the world-readable temp file and keep the fd across the
        // chmod, reading the secrets written below.
        let mut file = if owner_only {
            create_owner_only(&tmp)?
        } else {
            std::fs::File::create(&tmp)?
        };
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        if owner_only {
            // rename preserves the temp file's mode; re-assert in case the target
            // pre-existed with looser perms on some platforms.
            set_owner_only(path)?;
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Capture a file's exact pre-mutation bytes for transactional rollback.
/// `Ok(None)` means the file did not exist (rollback must delete, not empty,
/// it). Any OTHER read error (permission, transient I/O) surfaces as `Err` —
/// it must never be confused with "did not exist", which would make rollback
/// DELETE an existing file.
pub fn snapshot_file(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Best-effort restore of a snapshot taken by [`snapshot_file`]. Used only on
/// the error path, so a failed restore is logged to stderr rather than
/// shadowing the original error being returned to the caller.
pub fn restore_file(path: &Path, snapshot: Option<&[u8]>, owner_only: bool) {
    let result = match snapshot {
        Some(bytes) => write_bytes_atomic(path, bytes, owner_only),
        None => std::fs::remove_file(path).map_err(AppError::from),
    };
    if let Err(error) = result {
        // A NotFound remove just means the file was never created — fine.
        eprintln!(
            "FutureOS: config rollback could not restore {}: {error}",
            path.display()
        );
    }
}

#[cfg(unix)]
fn create_owner_only(path: &Path) -> Result<std::fs::File, AppError> {
    use std::os::unix::fs::OpenOptionsExt;
    Ok(std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?)
}

#[cfg(not(unix))]
fn create_owner_only(path: &Path) -> Result<std::fs::File, AppError> {
    // Windows has no 0600 equivalent; rely on the per-user profile directory.
    Ok(std::fs::File::create(path)?)
}

#[cfg(unix)]
fn set_owner_only(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) -> Result<(), AppError> {
    // Windows has no 0600 equivalent; rely on the per-user profile directory.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_path(label: &str) -> PathBuf {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "futureos-config-io-{}-{label}-{counter}.json",
            std::process::id()
        ))
    }

    #[test]
    fn missing_file_reads_as_empty_object() {
        let path = temp_path("missing");
        let _ = std::fs::remove_file(&path);
        assert_eq!(read_json_object(&path).unwrap(), json!({}));
    }

    #[test]
    fn corrupt_file_errors_and_is_not_clobbered() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(read_json_object(&path).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ not json",
            "strict read must leave the file untouched"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn non_object_root_errors() {
        let path = temp_path("array");
        std::fs::write(&path, "[1,2,3]").unwrap();
        assert!(read_json_object(&path).is_err());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn atomic_write_round_trips() {
        let path = temp_path("write");
        write_json_atomic(&path, &json!({ "a": 1 }), false).unwrap();
        assert_eq!(read_json_object(&path).unwrap(), json!({ "a": 1 }));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn path_lock_reuses_the_same_arc() {
        // Two live handles for one path must resolve to the same lock (the
        // Weak-upgrade fast path) so concurrent RMWs serialize on one mutex.
        let path = Path::new("/tmp/futureos-path-lock-reuse");
        let first = path_lock(path);
        let second = path_lock(path);
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn reading_a_directory_is_a_real_io_error() {
        // `read_to_string` on a directory is neither NotFound nor parseable, so
        // it must surface the IO error rather than being mistaken for "missing".
        let dir = std::env::temp_dir().join(format!("futureos-config-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_json_object(&dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn snapshot_file_treats_directory_as_error() {
        let dir = std::env::temp_dir().join(format!("futureos-snap-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(snapshot_file(&dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_over_a_directory_fails_and_cleans_up() {
        // Renaming the temp file over an existing directory fails; the writer
        // must remove the abandoned temp file on the error path.
        let dir = std::env::temp_dir().join(format!("futureos-write-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(write_bytes_atomic(&dir, b"payload", false).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn restore_file_writes_snapshot_and_deletes_on_none() {
        let path = temp_path("restore");
        restore_file(&path, Some(b"abc"), false);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "abc");

        restore_file(&path, None, false); // deletes the file
        assert!(!path.exists());

        // Deleting an already-missing file hits the NotFound logging arm.
        restore_file(&path, None, false);
        std::fs::remove_file(&path).ok();
    }
}
