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
                     Please quit the running Desktop first (Ctrl+C if it is running with --headless), \
                     then try again.",
                    directory.display()
                )
            } else {
                format!("Could not lock Desktop data directory {}: {e}", directory.display())
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
        assert!(error.contains("--headless"), "{error}");
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
}
