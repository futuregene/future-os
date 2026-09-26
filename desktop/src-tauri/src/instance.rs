//! One Desktop backend per data directory, whether graphical or headless.
//! Keep the file (and OS lock) open for the process lifetime; never delete a
//! lock file on exit, which would allow a third process to lock a new inode.
use std::fs::{File, OpenOptions};
use std::path::Path;

pub(crate) struct InstanceGuard {
    _file: File,
}

impl InstanceGuard {
    pub(crate) fn acquire() -> Result<Self, crate::AppError> {
        let home = crate::home_dir().ok_or("HOME/USERPROFILE is not set")?;
        Self::at(&Path::new(&home).join(".future").join("app"))
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
        assert!(InstanceGuard::at(directory.path()).is_ok());
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
