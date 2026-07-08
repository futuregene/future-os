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
use std::sync::{Mutex, OnceLock};

use serde_json::Value;

use crate::AppError;

/// Intern a `&'static Mutex<()>` per config path. The set of config paths is tiny
/// and fixed, so the leaked boxes are bounded.
fn path_lock(path: &Path) -> &'static Mutex<()> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, &'static Mutex<()>>>> = OnceLock::new();
    let registry = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = registry.lock().unwrap_or_else(|poison| poison.into_inner());
    guard
        .entry(path.to_path_buf())
        .or_insert_with(|| &*Box::leak(Box::new(Mutex::new(()))))
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let serialized = format!("{}\n", serde_json::to_string_pretty(value)?);
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
        file.write_all(serialized.as_bytes())?;
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
}
