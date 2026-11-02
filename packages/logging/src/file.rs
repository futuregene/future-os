use super::Profile;
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) struct RollingFile {
    path: PathBuf,
    file: Option<File>,
    bytes: u64,
    lines: usize,
    max_lines: usize,
    max_bytes: u64,
    backups: usize,
    // Last field: close the data handle before releasing ownership on Windows.
    _lock: File,
}

impl RollingFile {
    pub(super) fn open(path: &Path, profile: Profile, max_lines: usize) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(PathBuf::from(lock_path))?;
        fs2::FileExt::try_lock_exclusive(&lock)?;
        let backups = if profile == Profile::Detailed { 11 } else { 2 };
        let max_age = Duration::from_secs(86400 * if profile == Profile::Detailed { 7 } else { 3 });
        let mut writer = Self {
            path: path.into(),
            _lock: lock,
            file: None,
            bytes: 0,
            lines: 0,
            max_lines,
            max_bytes: 8 * 1024 * 1024,
            backups,
        };
        for index in 1..=11 {
            let backup = writer.backup(index);
            if index > backups
                || std::fs::metadata(&backup)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|time| time.elapsed().ok())
                    .is_some_and(|age| age > max_age)
            {
                match std::fs::remove_file(backup) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
        // A new launch starts a clean JSONL file, also preserving pre-upgrade
        // text logs in a backup instead of mixing text with structured records.
        writer.rotate()?;
        Ok(writer)
    }

    fn backup(&self, index: usize) -> PathBuf {
        let mut name = self.path.as_os_str().to_os_string();
        name.push(format!(".{index}"));
        name.into()
    }

    fn rotate(&mut self) -> io::Result<()> {
        // Close before rename: Windows does not permit renaming an open file.
        self.file.take();
        for index in (1..=self.backups).rev() {
            let to = self.backup(index);
            if to.exists() {
                std::fs::remove_file(&to)?;
            }
            let from = if index == 1 {
                self.path.clone()
            } else {
                self.backup(index - 1)
            };
            match std::fs::rename(from, to) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        self.file = Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?,
        );
        self.bytes = 0;
        self.lines = 0;
        Ok(())
    }
}

impl Write for RollingFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes > 0
            && (self.bytes.saturating_add(bytes.len() as u64) > self.max_bytes
                || (self.max_lines > 0 && self.lines >= self.max_lines))
        {
            self.rotate()?;
        }
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("log file is closed"))?
            .write_all(bytes)?;
        self.bytes += bytes.len() as u64;
        self.lines += bytes.iter().filter(|byte| **byte == b'\n').count();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_retains_whole_records_and_a_bounded_number_of_backups() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("component.log");
        let mut writer = RollingFile::open(&path, Profile::Normal, 0).unwrap();
        writer.max_bytes = 16;
        for index in 0..6 {
            writer
                .write_all(format!("{{\"entry\":{index}}}\n").as_bytes())
                .unwrap();
        }
        writer.flush().unwrap();
        drop(writer);
        for index in 0..=2 {
            let file = if index == 0 {
                path.clone()
            } else {
                path.with_extension(format!("log.{index}"))
            };
            let contents = std::fs::read_to_string(file).unwrap();
            for line in contents.lines() {
                serde_json::from_str::<serde_json::Value>(line).unwrap();
            }
        }
        assert!(!path.with_extension("log.3").exists());
        assert!(std::fs::read_to_string(path).unwrap().contains("5"));
    }
}
