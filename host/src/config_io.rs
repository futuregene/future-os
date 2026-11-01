//! Config-file helpers for the JSON files the application host owns under
//! `~/.future/` (`models.json`, `auth.json`, `approval_rule.json`).
//!
//! The strict-read and atomic-write primitives live in `future-config`: the TUI
//! writes its own `~/.future/tui/settings.json` with the same two guarantees
//! (a corrupt file is an error, never a silent reset; a write is a temp file
//! plus `rename`, serialized per path) and must not link this backend to get
//! them. What stays here is the host's [`Error`] type on the boundary, the
//! lenient read for best-effort caches, and the snapshot/rollback pair used by
//! transactional config updates.

#[cfg(any(test, feature = "test-support"))]
use crate::console;
use std::path::Path;

use serde_json::Value;

use crate::Error;

/// Strict read of a config file as a JSON object (see `future-config`).
pub fn read_json_object(path: &Path) -> Result<Value, Error> {
    future_config::read_json_object(path).map_err(into_host)
}

/// Atomic write of a JSON config file (see `future-config`).
pub fn write_json_atomic(path: &Path, value: &Value, owner_only: bool) -> Result<(), Error> {
    future_config::write_json_atomic(path, value, owner_only).map_err(into_host)
}

/// Atomic write of raw bytes, with the same temp-file + `rename` guarantees.
pub fn write_bytes_atomic(path: &Path, bytes: &[u8], owner_only: bool) -> Result<(), Error> {
    future_config::write_bytes_atomic(path, bytes, owner_only).map_err(into_host)
}

/// Serialize a read-modify-write of `path` within this process.
pub fn with_config_lock<T>(path: &Path, f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    future_config::with_config_lock(path, f)
}

/// `future-config` expresses the same three failure kinds this crate's [`Error`]
/// already carries, so the conversion keeps the message the caller would have
/// seen when the primitives lived here.
fn into_host(error: future_config::Error) -> Error {
    match error {
        future_config::Error::Io(error) => Error::Io(error),
        future_config::Error::Json(error) => Error::Json(error),
        future_config::Error::Message(message) => Error::Message(message),
    }
}

/// Lenient read for best-effort *cache* files where a corrupt file should not
/// surface an error to the user (e.g. a model-count cache): missing or
/// unparseable → an empty object.
#[cfg(any(test, feature = "test-support"))]
pub fn read_json_lenient(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<Value>(&contents).ok())
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()))
}

/// Capture a file's exact pre-mutation bytes for transactional rollback.
/// `Ok(None)` means the file did not exist (rollback must delete, not empty,
/// it). Any OTHER read error (permission, transient I/O) surfaces as `Err` —
/// it must never be confused with "did not exist", which would make rollback
/// DELETE an existing file.
#[cfg(any(test, feature = "test-support"))]
pub fn snapshot_file(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Best-effort restore of a snapshot taken by [`snapshot_file`]. Used only on
/// the error path, so a failed restore is logged to stderr rather than
/// shadowing the original error being returned to the caller.
#[cfg(any(test, feature = "test-support"))]
pub fn restore_file(path: &Path, snapshot: Option<&[u8]>, owner_only: bool) {
    let result = match snapshot {
        Some(bytes) => write_bytes_atomic(path, bytes, owner_only),
        None => std::fs::remove_file(path).map_err(Error::from),
    };
    if let Err(error) = result {
        // A NotFound remove just means the file was never created — fine.
        console::eprintln!(
            "FutureOS: config rollback could not restore {}: {error}",
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn temp_path(label: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let counter = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "futureos-host-config-{}-{label}-{counter}.json",
            std::process::id()
        ))
    }

    #[test]
    fn host_boundary_keeps_a_parse_error_as_an_error() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "{ not json").unwrap();
        let error = read_json_object(&path).unwrap_err();
        assert!(matches!(error, Error::Message(_)), "{error:?}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ not json",
            "strict read must leave the file untouched"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn lenient_read_treats_a_corrupt_cache_as_empty() {
        let path = temp_path("cache");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(read_json_lenient(&path), json!({}));
        std::fs::remove_file(&path).ok();
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

    /// The three failure kinds keep their identity across the boundary: a caller
    /// that matches `Error::Json` (as the settings writers do) must not receive a
    /// generic message because the primitive moved into another crate.
    #[test]
    fn a_config_failure_keeps_its_kind() {
        let io = into_host(future_config::Error::Io(std::io::Error::other("disk")));
        assert!(matches!(io, Error::Io(_)), "{io:?}");

        let invalid = serde_json::from_str::<i32>("not json").expect_err("invalid json");
        let json = into_host(future_config::Error::Json(invalid));
        assert!(matches!(json, Error::Json(_)), "{json:?}");

        let message = into_host(future_config::Error::Message("no room".to_string()));
        assert_eq!(message.to_string(), "no room");
    }
}
