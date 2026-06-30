//! Strict, atomic, 0600 read/write for `~/.future/agent/auth.json`.
//!
//! This is the single write path for the agent auth file (FutureGene login,
//! logout, and custom-provider API keys all route through it) so permissions
//! and parse strictness stay consistent — see gui/ER.md §6.9.
//!
//! Read semantics: a missing file is an empty object; a corrupt file or a
//! non-object root is an error (never silently dropped, so a write can't clobber
//! a hand-edited file). Write semantics: serialize to a sibling temp file with
//! `0600`, then atomically `rename` over the target.

use std::io::{ErrorKind, Write};
use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::AppError;

/// The built-in FutureGene provider id. Shared so `agent_providers` doesn't keep
/// its own copy (see gui/ER.md §6.9).
pub(crate) const FUTURE_PROVIDER_ID: &str = "future";

/// `~/.future/agent` — the agent config dir that owns `auth.json` and
/// `models.json`. Single source, shared with `agent_providers`.
pub(crate) fn agent_dir() -> Result<PathBuf, AppError> {
    let home = crate::home_dir().ok_or("HOME/USERPROFILE environment variable is not set.")?;
    Ok(PathBuf::from(home).join(".future").join("agent"))
}

pub(crate) fn auth_json_path() -> Result<PathBuf, AppError> {
    Ok(agent_dir()?.join("auth.json"))
}

/// Read `auth.json` as a JSON object map.
///
/// Missing file → empty map. Corrupt JSON or a non-object root → error, so
/// callers never overwrite an unreadable file with partial state.
pub(crate) fn read() -> Result<Map<String, Value>, AppError> {
    let path = auth_json_path()?;
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(error.into()),
    };

    let value: Value = serde_json::from_str(&contents).map_err(|error| {
        AppError::Message(format!(
            "{} 解析失败：{error}。请修复该文件后重试。",
            path.display()
        ))
    })?;

    match value {
        Value::Object(map) => Ok(map),
        _ => Err(AppError::Message(format!(
            "{} 的根节点必须是 JSON 对象。",
            path.display()
        ))),
    }
}

/// Atomically write `auth.json` with `0600` permissions (unix).
pub(crate) fn write(map: &Map<String, Value>) -> Result<(), AppError> {
    let path = auth_json_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let serialized = format!(
        "{}\n",
        serde_json::to_string_pretty(&Value::Object(map.clone()))?
    );

    // Unique temp name in the same directory so the final `rename` is atomic on
    // the same filesystem; the pid keeps concurrent writers from colliding.
    let tmp = path.with_file_name(format!("auth.json.tmp.{}", std::process::id()));

    let result = (|| -> Result<(), AppError> {
        let mut file = std::fs::File::create(&tmp)?;
        set_owner_only(&tmp)?;
        file.write_all(serialized.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, &path)?;
        // rename preserves the temp file's mode; re-assert in case the target
        // pre-existed with looser perms on some platforms.
        set_owner_only(&path)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(unix)]
fn set_owner_only(path: &PathBuf) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_only(_path: &PathBuf) -> Result<(), AppError> {
    // Windows has no 0600 equivalent; rely on the per-user profile directory.
    Ok(())
}

/// Set a provider's API key, preserving any other fields on the entry (e.g. the
/// FutureGene entry's `base_url`). Defaults `type` to `api_key` when absent.
pub(crate) fn set_provider_key(id: &str, key: &str) -> Result<(), AppError> {
    let mut auth = read()?;
    let entry = auth.entry(id.to_string()).or_insert_with(|| json!({}));
    if !entry.is_object() {
        *entry = json!({});
    }
    let object = entry
        .as_object_mut()
        .expect("entry was just normalized to an object");
    object
        .entry("type".to_string())
        .or_insert_with(|| Value::String("api_key".to_string()));
    object.insert("key".to_string(), Value::String(key.to_string()));
    write(&auth)
}

/// Remove a provider's API key but keep the rest of the entry (e.g. FutureGene
/// logout retains `base_url`). Returns whether a key was present.
pub(crate) fn remove_provider_key(id: &str) -> Result<bool, AppError> {
    let mut auth = read()?;
    let removed = auth
        .get_mut(id)
        .and_then(Value::as_object_mut)
        .map(|entry| entry.remove("key").is_some())
        .unwrap_or(false);
    if removed {
        write(&auth)?;
    }
    Ok(removed)
}

/// Remove a provider's whole auth entry (used when a custom provider is deleted).
/// Returns whether an entry was present.
pub(crate) fn remove_provider_entry(id: &str) -> Result<bool, AppError> {
    let mut auth = read()?;
    let removed = auth.remove(id).is_some();
    if removed {
        write(&auth)?;
    }
    Ok(removed)
}

/// FutureGene login: store the device-flow API key under the `future` entry.
pub(crate) fn set_future_key(key: &str) -> Result<(), AppError> {
    set_provider_key(FUTURE_PROVIDER_ID, key)
}

/// FutureGene logout: drop the key, keep `base_url`. Returns whether removed.
pub(crate) fn clear_future_key() -> Result<bool, AppError> {
    remove_provider_key(FUTURE_PROVIDER_ID)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::MutexGuard;

    // Each test points HOME at a fresh temp dir so reads/writes are isolated and
    // never touch the developer's real ~/.future/agent/auth.json.
    struct HomeGuard {
        previous: Option<String>,
        dir: std::path::PathBuf,
        _lock: MutexGuard<'static, ()>,
    }

    impl HomeGuard {
        fn new(label: &str) -> Self {
            let lock = crate::TEST_HOME_LOCK
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let previous = std::env::var("HOME").ok();
            let dir = std::env::temp_dir().join(format!(
                "futureos-auth-test-{}-{}",
                std::process::id(),
                label
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("HOME", &dir);
            HomeGuard {
                previous,
                dir,
                _lock: lock,
            }
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn read_missing_file_is_empty() {
        let _home = HomeGuard::new("missing");
        assert!(read().unwrap().is_empty());
    }

    #[test]
    fn corrupt_json_errors_and_does_not_clobber() {
        let _home = HomeGuard::new("corrupt");
        let path = auth_json_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not valid json").unwrap();

        assert!(read().is_err());
        // set_provider_key reads first, so it must surface the error rather than
        // overwrite the unreadable file.
        assert!(set_provider_key("future", "k").is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ not valid json",
            "corrupt file must be left untouched"
        );
    }

    #[test]
    fn non_object_root_errors() {
        let _home = HomeGuard::new("nonobject");
        let path = auth_json_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[1, 2, 3]").unwrap();
        assert!(read().is_err());
    }

    #[test]
    fn set_future_key_preserves_other_fields() {
        let _home = HomeGuard::new("preserve");
        let path = auth_json_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"future":{"type":"api_key","base_url":"https://example.com"},"zai":{"type":"api_key","key":"keep"}}"#,
        )
        .unwrap();

        set_future_key("new-key").unwrap();

        let auth = read().unwrap();
        let future = auth["future"].as_object().unwrap();
        assert_eq!(future["key"], Value::String("new-key".to_string()));
        assert_eq!(
            future["base_url"],
            Value::String("https://example.com".to_string()),
            "base_url must be preserved on login"
        );
        assert_eq!(
            auth["zai"]["key"],
            Value::String("keep".to_string()),
            "other providers must be untouched"
        );
    }

    #[test]
    fn clear_future_key_keeps_base_url() {
        let _home = HomeGuard::new("logout");
        set_provider_key("future", "k").unwrap();
        // Seed a base_url alongside the key.
        let mut auth = read().unwrap();
        auth["future"]
            .as_object_mut()
            .unwrap()
            .insert("base_url".to_string(), json!("https://example.com"));
        write(&auth).unwrap();

        assert!(clear_future_key().unwrap());
        let auth = read().unwrap();
        let future = auth["future"].as_object().unwrap();
        assert!(future.get("key").is_none(), "key removed");
        assert_eq!(future["base_url"], json!("https://example.com"));
        // Idempotent: clearing again reports nothing removed.
        assert!(!clear_future_key().unwrap());
    }

    #[test]
    fn remove_provider_entry_drops_whole_entry() {
        let _home = HomeGuard::new("delete");
        set_provider_key("dashscope", "k").unwrap();
        assert!(remove_provider_entry("dashscope").unwrap());
        assert!(read().unwrap().get("dashscope").is_none());
        assert!(!remove_provider_entry("dashscope").unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn write_sets_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let _home = HomeGuard::new("perms");
        set_provider_key("future", "k").unwrap();
        let mode = std::fs::metadata(auth_json_path().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
