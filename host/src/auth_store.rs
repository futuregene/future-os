//! Strict, atomic, 0600 read/write for `~/.future/agent/auth.json`.
//!
//! Since audit item 2 the PRIMARY write path is the agent itself (the GUI sends
//! `set_auth` over gRPC and the agent writes its own file — see
//! `agent_bridge/config.rs`). This module remains the read path for the GUI and
//! the LOCAL FALLBACK writer for an unreachable or pre-item-2 agent, so it keeps
//! the same strict/atomic/0600 semantics as before. FutureGene login, logout,
//! and custom-provider API keys all still route through it in fallback — see
//! desktop/ER.md §6.9.
//!
//! Read semantics: a missing file is an empty object; a corrupt file or a
//! non-object root is an error (never silently dropped, so a write can't clobber
//! a hand-edited file). Write semantics: serialize to a sibling temp file with
//! `0600`, then atomically `rename` over the target.

use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::config_io;
use crate::Error as AppError;

/// The built-in FutureGene provider id. Shared so `agent_providers` doesn't keep
/// its own copy (see desktop/ER.md §6.9).
pub const FUTURE_PROVIDER_ID: &str = "future";

/// `~/.future/agent` — the agent config dir that owns `auth.json` and
/// `models.json`. Single source, shared with `agent_providers`.
pub fn agent_dir() -> Result<PathBuf, AppError> {
    if let Some(root) = future_rpc::home::future_home_override() {
        return Ok(root.join("agent"));
    }
    let home = crate::home_dir().ok_or("HOME/USERPROFILE environment variable is not set.")?;
    Ok(PathBuf::from(home).join(".future").join("agent"))
}

pub fn auth_json_path() -> Result<PathBuf, AppError> {
    Ok(agent_dir()?.join("auth.json"))
}

/// Read `auth.json` as a JSON object map.
///
/// Missing file → empty map. Corrupt JSON or a non-object root → error, so
/// callers never overwrite an unreadable file with partial state.
pub fn read() -> Result<Map<String, Value>, AppError> {
    // Strictness (missing → empty, corrupt/non-object → error) is owned by the
    // shared config reader so auth.json and the other GUI configs stay identical;
    // `read_json_object` guarantees an object root.
    let value = config_io::read_json_object(&auth_json_path()?)?;
    Ok(value
        .as_object()
        .cloned()
        .expect("read_json_object returns an object root or an error"))
}

/// Atomically write `auth.json` with `0600` permissions (unix). Delegates the
/// temp-file + rename + permission dance to the shared [`config_io`] helper so
/// all config writers stay consistent (owner-only here).
pub fn write(map: &Map<String, Value>) -> Result<(), AppError> {
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
#[cfg(any(test, feature = "test-support"))]
pub fn set_provider_key(id: &str, key: &str) -> Result<(), AppError> {
    upsert_provider_entry(id, |object| {
        object.insert("key".to_string(), Value::String(key.to_string()));
    })
}

/// Remove a provider's API key but keep the rest of the entry (e.g. FutureGene
/// logout retains `base_url`). Returns whether a key was present.
#[cfg(any(test, feature = "test-support"))]
pub fn remove_provider_key(id: &str) -> Result<bool, AppError> {
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
/// FutureGene login: store the device-flow API key and pin `base_url` under the
/// `future` entry. Mirrors the CLI's `saveAuth` (which writes
/// `base_url = {platform}/api`) so a GUI login and a CLI login leave identical
/// `auth.json` state — and the agent/Providers page resolve the same platform.
#[cfg(any(test, feature = "test-support"))]
pub fn set_future_login(key: &str, base_url: &str) -> Result<(), AppError> {
    upsert_provider_entry(FUTURE_PROVIDER_ID, |object| {
        object.insert("key".to_string(), Value::String(key.to_string()));
        object.insert("base_url".to_string(), Value::String(base_url.to_string()));
    })
}

/// FutureGene logout: drop the key, keep `base_url`. Returns whether removed.
#[cfg(any(test, feature = "test-support"))]
pub fn clear_future_key() -> Result<bool, AppError> {
    remove_provider_key(FUTURE_PROVIDER_ID)
}

/// Switch the FutureGene environment: pin `base_url` to `{platform}/api` (as the
/// CLI's `auth login --url` does) and drop the now-stale API key plus any
/// `platform_base_url`, so the resolved platform is unambiguous and the user
/// re-logs in against the target environment (credentials don't carry across
/// environments). Mirrors `set_future_login` minus the key.
pub fn set_future_base_url(base_url: &str) -> Result<(), AppError> {
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
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use std::path::PathBuf;
    use std::sync::MutexGuard;

    /// Drain the shared Host state before an entry fixture changes HOME.
    pub fn cleanup_host() {
        crate::shutdown::reset_for_test();
        crate::shutdown::clear_failure_for_test();
        crate::runtime::cancel_test_tasks();
        crate::agent_bridge::cancel_all_observers();
        crate::request_scope::reset_for_test();
        crate::tasks::reset_for_test();
        crate::remote_peer::runtime::reset_after_shutdown_for_test();
        // A *stopped* host role, not merely a re-armed closed one:
        // `reset_after_shutdown_for_test` only resets a server that was shut
        // down, so a suspension with no resume — the shape
        // `handle_system_suspend_stops_the_running_generation` leaves behind
        // when it clears `start_requested` at the end — kept `suspended` set
        // for the whole process. Every later `start` then took
        // `start_generation`'s `!start_requested || suspended` arm and
        // answered an *empty* status: no bridge, no invitation, and the next
        // fixture's failure read as "the host shows no invitation" instead of
        // the reason. Draining here is the same rule the lines above follow —
        // nothing process-global outlives the fixture that set it.
        crate::remote::stop();
        crate::remote::reset_after_shutdown_for_test();
        crate::store::reset_run_event_writer_for_test();
        crate::store::close_pool();
    }

    pub struct HomeGuard {
        previous: Option<String>,
        previous_userprofile: Option<std::ffi::OsString>,
        previous_future_home: Option<std::ffi::OsString>,
        dir: PathBuf,
        _lock: MutexGuard<'static, ()>,
        cleanup: fn(),
        timing: crate::test_support::timing::FixtureTiming,
    }

    impl HomeGuard {
        pub fn new(label: &str) -> Self {
            Self::with_cleanup(label, cleanup_host)
        }

        /// Entry fixtures drain their remaining services and close database pools
        /// before replacing HOME. The guard owns the shared environment lock.
        pub fn with_cleanup(label: &str, cleanup: fn()) -> Self {
            let mut timing = crate::test_support::timing::FixtureTiming::start("HomeGuard", label);
            crate::remote::enable_fixture_mode();
            let lock = crate::TEST_HOME_LOCK
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            timing.mark("lock_wait");
            // `HOME` and the SQLite pool are process-global. A task started by
            // an earlier test may outlive its caller runtime, so it must be
            // cancelled and observed before this fixture publishes a new path.
            cleanup();
            timing.mark("pre_cleanup");
            let previous = std::env::var("HOME").ok();
            let previous_userprofile = std::env::var_os("USERPROFILE");
            let previous_future_home = std::env::var_os("FUTURE_HOME");
            let dir = std::env::temp_dir().join(format!(
                "futureos-test-{}-{}",
                std::process::id(),
                label
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("HOME", &dir);
            // The FutureOS root has to move with this fixture, not merely be
            // forgotten. Removing the override still puts every *file* under
            // `<dir>/.future`, but the local endpoint is named from the override
            // alone (an absent one resolves to the untagged default socket/pipe),
            // so a fixture that removed it reached the developer's running Agent
            // while its own state sat in a temp dir. `<dir>/.future` is exactly
            // what "override absent + this HOME" already resolved to, so no path
            // assertion changes — only the endpoint becomes this fixture's own.
            std::env::set_var("FUTURE_HOME", dir.join(".future"));
            timing.mark("setup");
            HomeGuard {
                previous,
                previous_userprofile,
                previous_future_home,
                dir,
                _lock: lock,
                cleanup,
                timing,
            }
        }

        pub fn path(&self) -> &std::path::Path {
            &self.dir
        }

        /// Withdraw every way the home root can be resolved — `HOME`,
        /// `USERPROFILE` and the `FUTURE_HOME` root this guard published — for
        /// the rest of the guard's life, so the code under test runs against a
        /// process that has no home at all.
        ///
        /// All three are needed. The two user-home variables are one way to
        /// name the root and `FUTURE_HOME` is the other, so a test that dropped
        /// only `HOME` would keep resolving this fixture's root through the
        /// override and assert about a home it believes does not exist.
        /// [`restore`](Self::restore) puts every one of them back.
        pub fn withdraw_home(&self) {
            std::env::remove_var("HOME");
            std::env::remove_var("USERPROFILE");
            std::env::remove_var("FUTURE_HOME");
        }

        /// Restore the process-global `HOME` to its pre-guard state and remove
        /// the temp dir. Split from [`Drop`] so the absent-`HOME` arm is
        /// directly observable without racing the global `TEST_HOME_LOCK`.
        fn restore(&mut self) {
            self.timing.mark("body");
            // Keep the fixture path valid until every process-lifetime task
            // started under it has stopped. Restoring HOME first would let a
            // late task resolve another test's database instead.
            (self.cleanup)();
            self.timing.mark("cleanup");
            match &self.previous {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match &self.previous_userprofile {
                Some(value) => std::env::set_var("USERPROFILE", value),
                None => std::env::remove_var("USERPROFILE"),
            }
            match &self.previous_future_home {
                Some(value) => std::env::set_var("FUTURE_HOME", value),
                None => std::env::remove_var("FUTURE_HOME"),
            }
            // The entry cleanup above must release open resources before deletion;
            // Windows cannot unlink a database held by a connection pool.
            let _ = std::fs::remove_dir_all(&self.dir);
            self.timing.finish();
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            self.restore();
        }
    }

    #[test]
    fn home_guard_restores_absent_home() {
        let lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let previous = std::env::var("HOME").ok();
        let dir =
            std::env::temp_dir().join(format!("futureos-test-{}-absent-home", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A guard that believes HOME was absent at creation: its restore must
        // remove HOME rather than overwrite it.
        let mut guard = HomeGuard {
            previous: None,
            previous_userprofile: std::env::var_os("USERPROFILE"),
            previous_future_home: std::env::var_os("FUTURE_HOME"),
            dir: dir.clone(),
            _lock: lock,
            cleanup: cleanup_host,
            timing: crate::test_support::timing::FixtureTiming::start("HomeGuard", "absent-home"),
        };
        guard.restore();
        assert!(
            std::env::var("HOME").is_err(),
            "absent HOME must stay absent"
        );
        // Re-arm the guard with the observed previous value so its eventual
        // Drop restores the process-global HOME to exactly what it found.
        guard.previous = previous;
    }

    /// `FUTURE_HOME` is the documented way to relocate the whole FutureOS root
    /// (an embedded host sets it). `restore` puts the caller's value back, the
    /// same way it does HOME.
    #[test]
    fn home_guard_restores_a_previously_set_future_home() {
        let lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let previous = std::env::var("HOME").ok();
        let dir =
            std::env::temp_dir().join(format!("futureos-test-{}-future-home", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut guard = HomeGuard {
            previous,
            previous_userprofile: std::env::var_os("USERPROFILE"),
            previous_future_home: Some(std::ffi::OsString::from("/opt/pre-existing-future-home")),
            dir: dir.clone(),
            _lock: lock,
            cleanup: cleanup_host,
            timing: crate::test_support::timing::FixtureTiming::start("HomeGuard", "future-home"),
        };
        // While the fixture is live the variable names the fixture's own root,
        // so the code under test cannot resolve the caller's one.
        std::env::set_var("FUTURE_HOME", "/opt/temporary-future-home");

        guard.restore();

        assert_eq!(
            std::env::var_os("FUTURE_HOME"),
            Some(std::ffi::OsString::from("/opt/pre-existing-future-home")),
            "restore must put the caller's FUTURE_HOME back"
        );
        assert!(!dir.exists(), "the fixture directory is removed too");
        // Re-arm with the observed (absent) previous value *and* drop the
        // saved FUTURE_HOME, so the guard's Drop leaves the process-global
        // environment exactly as this test found it instead of re-setting it.
        guard.previous_future_home = None;
        std::env::remove_var("FUTURE_HOME");
    }

    /// A live guard must redirect the FutureOS root itself, not just `HOME`.
    ///
    /// Both variables decide where state lands — `HOME` for the files, the
    /// override for the local endpoint (the untagged default socket/pipe when it
    /// is absent) — so a fixture that only moved `HOME` kept its own files while
    /// still reaching the developer's running Agent. `<dir>/.future` is the root
    /// a `HOME`-only resolution produced anyway, so this pins that the two move
    /// together without changing any path.
    #[test]
    fn a_live_home_guard_redirects_the_future_home_root_it_created() {
        let home = HomeGuard::new("future-home-live");
        let dir = home.path().to_path_buf();
        assert_eq!(
            future_rpc::home::future_home_override(),
            Some(dir.join(".future")),
            "the guard must publish its own FutureOS root"
        );
        assert_eq!(
            future_rpc::home::future_home(),
            dir.join(".future"),
            "files resolve to the fixture root, exactly as a HOME-only resolution did"
        );
    }

    /// The premise [`HomeGuard::withdraw_home`] exists for: "no home" is not
    /// `HOME` alone. With the override still published, a test that removed
    /// only `HOME` kept resolving this fixture's root and asserted about a home
    /// it believed did not exist — so the withdrawal has to take all three
    /// variables, and `restore` has to bring all three back.
    #[test]
    fn withdrawing_the_home_leaves_no_root_to_resolve_and_restore_puts_it_back() {
        let mut guard = HomeGuard::new("withdraw-home");
        let saved_home = guard.previous.clone().map(std::ffi::OsString::from);
        let saved_userprofile = guard.previous_userprofile.clone();
        let saved_future_home = guard.previous_future_home.clone();

        guard.withdraw_home();

        assert_eq!(std::env::var_os("HOME"), None, "HOME withdrawn");
        assert_eq!(
            std::env::var_os("USERPROFILE"),
            None,
            "USERPROFILE is the Windows spelling of the same thing"
        );
        assert_eq!(
            future_rpc::home::future_home_override(),
            None,
            "the fixture's own root must go too: it is a second way to name one"
        );
        assert!(
            crate::store::future_dir().is_err(),
            "with no home and no override there is no FutureOS root to resolve"
        );

        guard.restore();

        assert_eq!(std::env::var_os("HOME"), saved_home);
        assert_eq!(std::env::var_os("USERPROFILE"), saved_userprofile);
        assert_eq!(std::env::var_os("FUTURE_HOME"), saved_future_home);
    }
    #[test]
    fn home_guard_removes_its_directory_even_with_a_live_pooled_connection() {
        // Regression: the guard used to delete its temp home while the
        // connection pool still held the database open. Windows cannot unlink
        // an open file, so the delete failed silently and every run leaked its
        // home directory (~19k dirs / 33 GB accumulated before this was found),
        // which eventually filled the disk and surfaced as spurious SQLite
        // `DatabaseBusy`/`StorageFull` failures in unrelated tests.
        let dir = {
            let guard = HomeGuard::new("leak-regression");
            let dir = guard.path().to_path_buf();
            // Leave a pooled connection to this home's database open across the
            // guard's drop: that handle is what used to make the delete fail.
            crate::store::initialize_app_store().expect("store");
            dir
        };
        assert!(!dir.exists(), "guard must remove {} on drop", dir.display());
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
        let path = auth_json_path().unwrap();
        config_io::with_config_lock(&path, || {
            let mut auth = read()?;
            let removed = auth.remove("dashscope").is_some();
            assert!(removed);
            write(&auth)?;
            Ok(removed)
        })
        .unwrap();
        assert!(read().unwrap().get("dashscope").is_none());
    }

    #[test]
    fn non_object_provider_entry_is_normalized() {
        let _home = HomeGuard::new("nonobj-entry");
        let path = auth_json_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"future": "bogus"}"#).unwrap();

        set_provider_key("future", "k").unwrap();

        let auth = read().unwrap();
        assert_eq!(auth["future"]["key"], "k");
        assert_eq!(auth["future"]["type"], "api_key");
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
