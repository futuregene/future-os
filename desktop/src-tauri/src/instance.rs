//! One Desktop backend per data directory, whether graphical or headless.
//! Keep the file (and OS lock) open for the process lifetime; never delete a
//! lock file on exit, which would allow a third process to lock a new inode.
use std::fs::{File, OpenOptions};
use std::path::Path;

pub(crate) struct InstanceGuard {
    _file: File,
}

/// Namespace the native single-instance plugin by the same physical root as
/// the file lock. The default identifier remains byte-for-byte compatible.
#[cfg(any(feature = "gui", test))]
pub(crate) fn gui_identifier(base: &str) -> Result<String, crate::AppError> {
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
    pub(crate) fn acquire() -> Result<Self, crate::AppError> {
        let directory = future_app_settings::app_dir().map_err(|error| error.to_string())?;
        Self::at(&directory)
    }

    fn at(directory: &Path) -> Result<Self, crate::AppError> {
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
                     Please quit the running Desktop first (Ctrl+C for futureos-headless), \
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
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(error.contains("futureos-headless"), "{error}");
        assert!(
            error.contains(&directory.path().display().to_string()),
            "{error}"
        );
        assert!(!error.contains("os error"), "{error}");
        drop(guard);
        // Retry briefly before calling the lock leaked: a sibling test's
        // `Command::spawn` forks while this guard is open, and the child shares
        // the open file description until it execs. `flock` ownership follows
        // the description rather than the fd, so the lock can still read as held
        // for the length of that window -- an artifact of running the suite in
        // parallel, not a lock that was never released. A lock that really is
        // never released still fails here, just after the deadline.
        //
        // Report the reason rather than a bare `is_ok()`: this line failed
        // intermittently under full-suite load (3 times in ~20 runs) with no way
        // to tell a lock that was not released from an unrelated `open` failure,
        // which is the difference between a product bug and a test-environment
        // one. It has never reproduced in isolation (150 single runs, 320
        // concurrent runs of this test, and several full-suite runs all pass).
        //
        // fd exhaustion was measured and ruled out: this test holds no fds (all
        // three of its cases pass under `ulimit -n 128`, where 284 other tests
        // fail with EMFILE), and the suite's peak demand stays below 256 against
        // a 2560 soft limit.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match InstanceGuard::at(directory.path()) {
                Ok(_) => break,
                Err(_) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => {
                    panic!("re-acquiring the lock after release failed: {error}")
                }
            }
        }
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
        let reacquired = InstanceGuard::acquire().expect("reacquire after release");
        drop(reacquired);
        drop(home);
    }
}
