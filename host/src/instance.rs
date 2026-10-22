//! One Desktop backend per data directory, whether graphical or headless.
//! Keep the file (and OS lock) open for the process lifetime; never delete a
//! lock file on exit, which would allow a third process to lock a new inode.
use std::fs::{File, OpenOptions};
use std::path::Path;

pub struct InstanceGuard {
    _file: File,
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // A concurrently forked child can retain this open file description
        // until exec, even with CLOEXEC. Unlock explicitly when its owner ends
        // rather than waiting for the last inherited descriptor to close.
        let _ = fs2::FileExt::unlock(&self._file);
    }
}

/// Namespace the native single-instance plugin by the same physical root as
/// the file lock. The default identifier remains byte-for-byte compatible.
pub fn gui_identifier(base: &str) -> Result<String, crate::Error> {
    if future_rpc::home::future_home_override().is_none() {
        return Ok(base.into());
    }
    let directory = future_app_settings::app_dir().map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let selected_root = crate::future_home_root()
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if crate::home_dir().is_some_and(|home| {
        std::path::PathBuf::from(home)
            .join(".future")
            .canonicalize()
            .is_ok_and(|default| default == selected_root)
    }) {
        return Ok(base.into());
    }
    let root_id = future_rpc::connection::data_root_id(&crate::future_home_root())
        .map_err(|error| error.to_string())?;
    Ok(format!("{base}.home{}", &root_id[..16]))
}

impl InstanceGuard {
    pub fn acquire() -> Result<Self, crate::Error> {
        let directory = future_app_settings::app_dir().map_err(|error| error.to_string())?;
        Self::at(&directory)
    }

    fn at(directory: &Path) -> Result<Self, crate::Error> {
        if let Some(home) = directory.parent() {
            future_rpc::home::ensure_restore_complete(home)?;
        }
        std::fs::create_dir_all(directory).map_err(|e| format!("Create Desktop directory: {e}"))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("desktop.lock"))
            .map_err(|e| format!("Open Desktop lock: {e}"))?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|e| {
            if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() {
                format!(
                    "Desktop is already running for data directory {}. \
                     GUI and headless mode cannot use the same data directory at the same time. \
                     Please quit the running Desktop first (Ctrl+C for future host), \
                     then try again.",
                    directory.display()
                )
            } else {
                format!(
                    "Could not lock Desktop data directory {}: {e}",
                    directory.display()
                )
            }
        })?;
        if let Some(home) = directory.parent() {
            future_rpc::home::ensure_restore_complete(home)?;
        }
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn distinct_future_homes_isolate_lock_namespace_and_storage_without_moving_legacy_data() {
        let _home = crate::auth_store::test_support::HomeGuard::new("instance-future-home");
        let default = future_app_settings::app_dir().unwrap();
        std::fs::create_dir_all(&default).unwrap();
        std::fs::write(default.join("app.db"), b"legacy bytes").unwrap();
        assert_eq!(
            gui_identifier("com.futureos.desktop").unwrap(),
            "com.futureos.desktop"
        );
        std::env::set_var("FUTURE_HOME", default.parent().unwrap());
        assert_eq!(
            gui_identifier("com.futureos.desktop").unwrap(),
            "com.futureos.desktop",
            "an explicit default root must activate the same GUI instance"
        );
        let parent = tempfile::tempdir().unwrap();
        let first = parent.path().join("first");
        let second = parent.path().join("second");
        std::env::set_var("FUTURE_HOME", &first);
        let a = InstanceGuard::acquire().unwrap();
        let first_id = gui_identifier("com.futureos.desktop").unwrap();
        assert!(first.join("app/desktop.lock").is_file());
        assert_eq!(crate::store::future_dir().unwrap(), first);
        assert_eq!(
            future_app_settings::app_db_path().unwrap(),
            first.join("app/app.db")
        );
        assert_eq!(
            future_app_workspaces::chat_workspaces_root().unwrap(),
            first.join("workspaces/chat")
        );
        assert!(future_app_workspaces::is_chat_directory(
            &first.join("workspaces/chat/session").to_string_lossy()
        ));
        assert!(!future_app_workspaces::is_chat_directory(
            "~/.future/workspaces/chat/session"
        ));
        std::env::set_var("FUTURE_HOME", &second);
        let b = InstanceGuard::acquire().unwrap();
        assert_ne!(gui_identifier("com.futureos.desktop").unwrap(), first_id);
        assert!(second.join("app/desktop.lock").is_file());
        assert_eq!(
            std::fs::read(default.join("app.db")).unwrap(),
            b"legacy bytes"
        );
        assert!(!first.join("app/app.db").exists());
        drop((a, b));
    }

    #[test]
    fn excludes_second_owner_and_releases_on_drop() {
        let directory = tempfile::tempdir().unwrap();
        let guard = InstanceGuard::at(directory.path()).unwrap();
        let error = InstanceGuard::at(directory.path())
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("Desktop is already running"), "{error}");
        assert!(error.contains("future host"), "{error}");
        assert!(
            error.contains(&directory.path().display().to_string()),
            "{error}"
        );
        assert!(!error.contains("os error"), "{error}");
        drop(guard);
        // Releasing the only owner makes the same directory available again.
        assert_release_is_visible_in_a_fresh_process(directory.path());
    }

    /// The *failure* half of the release check, and the non-contended lock error.
    ///
    /// `at()` maps two different `flock` failures through one `map_err`: taking a
    /// lock that someone else holds (covered by the tests above) and a lock that
    /// cannot be taken for any other reason. The second must not be reported as a
    /// running Desktop — the user would go hunting for a process that does not
    /// exist. A FIFO is how this host produces that failure: `open(O_RDWR)` on one
    /// succeeds, while `flock` on it returns ENOTSUP (probed: `/dev/null` *accepts*
    /// the lock, so it cannot be used). Gated to macOS because the errno is a
    /// property of the platform, not of this code.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_lock_failure_that_is_not_contention_is_not_reported_as_another_instance() {
        let directory = tempfile::tempdir().unwrap();
        let lock_path = directory.path().join("desktop.lock");
        let fifo = std::ffi::CString::new(lock_path.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: mkfifo only reads the path built above; a failure is asserted.
        let created = unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) };
        // Hoisted out of the assertion: `assert_eq!` only evaluates its format
        // arguments on failure, so a passing run would never execute them.
        let mkfifo_error = format!(
            "mkfifo {}: {}",
            lock_path.display(),
            std::io::Error::last_os_error()
        );
        assert_eq!(created, 0, "{mkfifo_error}");

        let error = InstanceGuard::at(directory.path())
            .err()
            .expect("this lock cannot be taken")
            .to_string();
        assert!(!error.contains("already running"), "{error}");
        assert!(
            error.contains("Could not lock Desktop data directory"),
            "{error}"
        );
        assert!(
            error.contains(&directory.path().display().to_string()),
            "the message must still name the directory: {error}"
        );
    }

    #[test]
    fn directory_errors_are_not_reported_as_another_instance() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let error = InstanceGuard::at(file.path()).err().unwrap().to_string();
        assert!(error.contains("Create Desktop directory"), "{error}");
        assert!(!error.contains("already running"), "{error}");
    }

    #[test]
    fn independent_data_directories_can_run_together() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let _a = InstanceGuard::at(first.path()).unwrap();
        let _b = InstanceGuard::at(second.path()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn releasing_the_owner_unlocks_a_description_still_held_by_a_child() {
        let directory = tempfile::tempdir().unwrap();
        let guard = InstanceGuard::at(directory.path()).unwrap();
        // dup shares the same open file description as an inherited fork fd,
        // without needing to fork from the multithreaded test runner.
        let inherited = guard._file.try_clone().unwrap();
        assert!(InstanceGuard::at(directory.path()).is_err());
        drop(guard);
        let next = InstanceGuard::at(directory.path())
            .expect("the old owner's inherited description must not retain its lock");
        drop(inherited);
        assert!(
            InstanceGuard::at(directory.path()).is_err(),
            "the new owner keeps its lock"
        );
        drop(next);
        assert!(InstanceGuard::at(directory.path()).is_ok());
    }

    /// `acquire()` resolves the data directory from the process-global HOME, so
    /// this is the only test that needs the same fixture the store tests use.
    #[test]
    fn acquire_resolves_the_home_directory_and_keeps_its_lock() {
        let home = crate::auth_store::test_support::HomeGuard::new("instance-acquire");
        let root = std::env::var("HOME").expect("HomeGuard publishes HOME");
        let data_directory = Path::new(&root).join(".future").join("app");
        let guard = InstanceGuard::acquire().expect("acquire under the override HOME");
        assert!(
            data_directory.join("desktop.lock").is_file(),
            "acquire must create the lock file under HOME/.future/app"
        );

        let error = InstanceGuard::acquire()
            .err()
            .expect("the data directory is already owned")
            .to_string();
        assert!(error.contains("Desktop is already running"), "{error}");
        assert!(
            error.contains(&data_directory.display().to_string()),
            "the message must name the contended directory: {error}"
        );

        drop(guard);
        // Releasing the only owner makes the same directory available again.
        assert_release_is_visible_in_a_fresh_process(&data_directory);
        drop(home);
    }

    /// Environment naming the directory a [`reacquire_fixture`] child checks.
    /// Absent in an ordinary suite run, where the fixture is a no-op.
    const REACQUIRE_FIXTURE_DIR: &str = "FUTURE_HOST_INSTANCE_REACQUIRE_DIR";

    /// Assert that `drop`'s release is visible to another process.
    ///
    /// The acquire/release/acquire pair this checks cannot be asserted from the
    /// test process itself: `flock` ownership follows the open file description,
    /// so a sibling test's `Command::spawn` forks a child that keeps *this*
    /// process's lock alive until it execs, and no amount of care in this
    /// process makes the re-lock deterministic then. A retry loop hides that
    /// window instead of removing it, and turns a real leak into a slow failure.
    ///
    /// The child shares nothing but the directory: it is the only process that
    /// touches this lock, so its single attempt at each acquire is
    /// deterministic. Measured while eight threads spawned 27,565 children: 3,098
    /// rounds, no failure -- against 109 lost re-locks in 400 spawn-then-release
    /// rounds in a process that held the lock across the spawn. A lock that is
    /// really never released still fails here, once.
    fn assert_release_is_visible_in_a_fresh_process(directory: &Path) {
        let output = Command::new(std::env::current_exe().expect("the running test binary"))
            .args([
                "--exact",
                "instance::tests::reacquire_fixture",
                "--test-threads=1",
            ])
            // The directory, not `HOME`: the child takes the lock through the
            // same `at()` the caller used, so the check does not depend on a
            // process-global environment other tests rewrite concurrently.
            .env(REACQUIRE_FIXTURE_DIR, directory)
            .stdin(Stdio::null())
            .output()
            .expect("run the re-acquire fixture");
        let report = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.status.success(),
            "re-acquiring the released directory failed: {report}"
        );
        // A filter that matches nothing also exits successfully, so without this
        // a rename would leave the check above vacuous.
        assert!(
            report.contains("1 passed"),
            "the fixture did not run: {report}"
        );
    }

    /// The child half of [`assert_release_is_visible_in_a_fresh_process`].
    #[test]
    fn reacquire_fixture() {
        let Some(directory) = std::env::var_os(REACQUIRE_FIXTURE_DIR) else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        let guard = InstanceGuard::at(&directory).expect("the released directory is available");
        drop(guard);
        // A guard that leaked its descriptor instead of closing it still fails
        // here, on this one attempt.
        let reacquired = InstanceGuard::at(&directory).expect("a second acquire after release");
        drop(reacquired);
    }
}
