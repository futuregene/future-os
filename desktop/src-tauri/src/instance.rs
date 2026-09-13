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
            format!("Another Desktop (GUI or headless) may be using this data directory: {e}")
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
        assert!(InstanceGuard::at(directory.path()).is_err());
        drop(guard);
        assert!(InstanceGuard::at(directory.path()).is_ok());
    }

    #[test]
    fn independent_data_directories_can_run_together() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let _a = InstanceGuard::at(first.path()).unwrap();
        let _b = InstanceGuard::at(second.path()).unwrap();
    }
}
