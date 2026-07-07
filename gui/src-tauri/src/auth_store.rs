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

use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::config_io;
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
    // Strictness (missing → empty, corrupt/non-object → error) is owned by the
    // shared config reader so auth.json and the other GUI configs stay identical;
    // `read_json_object` guarantees an object root, so the else arm is unreachable.
    match config_io::read_json_object(&auth_json_path()?)? {
        Value::Object(map) => Ok(map),
        _ => unreachable!("read_json_object returns an object root or an error"),
    }
}

/// Atomically write `auth.json` with `0600` permissions (unix). Delegates the
/// temp-file + rename + permission dance to the shared [`config_io`] helper so
/// all config writers stay consistent (owner-only here).
pub(crate) fn write(map: &Map<String, Value>) -> Result<(), AppError> {
    let path = auth_json_path()?;
    config_io::write_json_atomic(&path, &Value::Object(map.clone()), true)
}

/// Read `auth.json`, upsert the provider's entry (creating it / normalizing a
/// non-object to `{}`, defaulting `type` to `api_key`), let `mutate` set fields,
/// then write atomically. The whole read+write is serialized per-path so two
/// concurrent commands can't lose each other's update.
fn upsert_provider_entry(
    id: &str,
    mutate: impl FnOnce(&mut Map<String, Value>),
) -> Result<(), AppError> {
    let path = auth_json_path()?;
    config_io::with_config_lock(&path, || {
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
        mutate(object);
        write(&auth)
    })
}

/// Set a provider's API key, preserving any other fields on the entry (e.g. the
/// FutureGene entry's `base_url`). Defaults `type` to `api_key` when absent.
pub(crate) fn set_provider_key(id: &str, key: &str) -> Result<(), AppError> {
    upsert_provider_entry(id, |object| {
        object.insert("key".to_string(), Value::String(key.to_string()));
    })
}

/// Remove a provider's API key but keep the rest of the entry (e.g. FutureGene
/// logout retains `base_url`). Returns whether a key was present.
pub(crate) fn remove_provider_key(id: &str) -> Result<bool, AppError> {
    let path = auth_json_path()?;
    config_io::with_config_lock(&path, || {
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
    })
}

/// Remove a provider's whole auth entry (used when a custom provider is deleted).
/// Returns whether an entry was present.
pub(crate) fn remove_provider_entry(id: &str) -> Result<bool, AppError> {
    let path = auth_json_path()?;
    config_io::with_config_lock(&path, || {
        let mut auth = read()?;
        let removed = auth.remove(id).is_some();
        if removed {
            write(&auth)?;
        }
        Ok(removed)
    })
}

/// FutureGene login: store the device-flow API key and pin `base_url` under the
/// `future` entry. Mirrors the CLI's `saveAuth` (which writes
/// `base_url = {platform}/api`) so a GUI login and a CLI login leave identical
/// `auth.json` state — and the agent/Providers page resolve the same platform.
pub(crate) fn set_future_login(key: &str, base_url: &str) -> Result<(), AppError> {
    upsert_provider_entry(FUTURE_PROVIDER_ID, |object| {
        object.insert("key".to_string(), Value::String(key.to_string()));
        object.insert("base_url".to_string(), Value::String(base_url.to_string()));
    })
}

/// FutureGene logout: drop the key, keep `base_url`. Returns whether removed.
pub(crate) fn clear_future_key() -> Result<bool, AppError> {
    remove_provider_key(FUTURE_PROVIDER_ID)
}

/// Switch the FutureGene environment: pin `base_url` to `{platform}/api` (as the
/// CLI's `auth login --url` does) and drop the now-stale API key plus any
/// `platform_base_url`, so the resolved platform is unambiguous and the user
/// re-logs in against the target environment (credentials don't carry across
/// environments). Mirrors [`set_future_login`] minus the key.
pub(crate) fn set_future_base_url(base_url: &str) -> Result<(), AppError> {
    upsert_provider_entry(FUTURE_PROVIDER_ID, |object| {
        object.remove("key");
        object.remove("platform_base_url");
        object.insert("base_url".to_string(), Value::String(base_url.to_string()));
    })
}

/// Shared test fixture: points `HOME` at a fresh temp dir so config reads/writes
/// are isolated and never touch the developer's real `~/.future/`. Lives here (the
/// single owner of the auth file) so `auth_store` and `agent_providers` tests use
/// one copy. Serialized on [`crate::TEST_HOME_LOCK`] since `HOME` is process-global.
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::PathBuf;
    use std::sync::MutexGuard;

    pub(crate) struct HomeGuard {
        previous: Option<String>,
        dir: PathBuf,
        _lock: MutexGuard<'static, ()>,
    }

    impl HomeGuard {
        pub(crate) fn new(label: &str) -> Self {
            let lock = crate::TEST_HOME_LOCK
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let previous = std::env::var("HOME").ok();
            let dir = std::env::temp_dir().join(format!(
                "futureos-test-{}-{}",
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
}

#[cfg(test)]
mod tests {
    use super::test_support::HomeGuard;
    use super::*;

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
    fn set_future_login_writes_key_and_base_url() {
        let _home = HomeGuard::new("login");
        let path = auth_json_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"future":{"type":"api_key","base_url":"https://old.example.com/api"},"zai":{"type":"api_key","key":"keep"}}"#,
        )
        .unwrap();

        set_future_login("new-key", "https://future-os.cn/api").unwrap();

        let auth = read().unwrap();
        let future = auth["future"].as_object().unwrap();
        assert_eq!(future["key"], Value::String("new-key".to_string()));
        assert_eq!(future["type"], Value::String("api_key".to_string()));
        assert_eq!(
            future["base_url"],
            Value::String("https://future-os.cn/api".to_string()),
            "login pins base_url to the resolved platform (mirrors the CLI)"
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
    fn set_future_base_url_switches_env_and_drops_key() {
        let _home = HomeGuard::new("switch-env");
        let path = auth_json_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"future":{"type":"api_key","key":"old","base_url":"https://future-os.cn/api","platform_base_url":"https://future-os.cn"},"zai":{"type":"api_key","key":"keep"}}"#,
        )
        .unwrap();

        set_future_base_url("https://test.future-os.cn/api").unwrap();

        let auth = read().unwrap();
        let future = auth["future"].as_object().unwrap();
        assert_eq!(
            future["base_url"],
            json!("https://test.future-os.cn/api"),
            "base_url is pinned to the target environment"
        );
        assert!(future.get("key").is_none(), "stale key is dropped");
        assert!(
            future.get("platform_base_url").is_none(),
            "ambiguous platform_base_url is dropped"
        );
        assert_eq!(
            auth["zai"]["key"],
            json!("keep"),
            "other providers must be untouched"
        );
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
