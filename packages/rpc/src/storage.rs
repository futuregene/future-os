//! Process-wide storage policy, selected before any database is opened.
//! NFS mode requires working server-side locks and stable writes. It does not
//! make an unreliable network filesystem safe, and never weakens durability.
use std::{fmt, str::FromStr};

pub const STORAGE_MODE_ENV: &str = "FUTURE_STORAGE_MODE";
pub const STORAGE_MODE_HEADER: &str = "x-future-storage-mode";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StorageMode {
    #[default]
    Wal,
    Nfs,
}
impl StorageMode {
    pub fn from_env() -> Result<Self, String> {
        match std::env::var(STORAGE_MODE_ENV) {
            Ok(value) => value.parse(),
            Err(std::env::VarError::NotPresent) => Self::read_home(&crate::home::future_home()),
            Err(error) => Err(format!("{STORAGE_MODE_ENV}: {error}")),
        }
    }
    pub fn read_home(home: &std::path::Path) -> Result<Self, String> {
        match std::fs::read_to_string(home.join("storage-mode")) {
            Ok(value) => value.trim().parse(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::Wal),
            Err(error) => Err(format!("read storage-mode: {error}")),
        }
    }
    /// The Agent instance lock must be held. Publish before opening databases
    /// so later CLI commands use the same mode without an environment override.
    pub fn record_selection(home: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        if std::env::var_os(STORAGE_MODE_ENV).is_none() {
            return Ok(());
        }
        let mode = Self::from_env().map_err(std::io::Error::other)?;
        if Self::read_home(home).ok() == Some(mode) && home.join("storage-mode").exists() {
            return Ok(());
        }
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temporary = home.join(format!(".storage-mode-{}-{nonce}", std::process::id()));
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            writeln!(file, "{mode}")?;
            file.sync_all()?;
            std::fs::rename(&temporary, home.join("storage-mode"))
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }
    /// Call only during entry-point initialization, before starting threads.
    pub fn apply(self) {
        std::env::set_var(STORAGE_MODE_ENV, self.as_str());
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wal => "wal",
            Self::Nfs => "nfs",
        }
    }
    pub fn journal_mode(self) -> &'static str {
        match self {
            Self::Wal => "WAL",
            Self::Nfs => "PERSIST",
        }
    }
    /// Shared by the Host, task store and other SQLite owners.
    pub fn pragmas(self) -> &'static str {
        match self {
            Self::Wal => "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
            Self::Nfs => "PRAGMA journal_mode=PERSIST; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
        }
    }
}
impl FromStr for StorageMode {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "wal" => Ok(Self::Wal),
            "nfs" => Ok(Self::Nfs),
            _ => Err(format!(
                "invalid storage mode {value:?}; expected wal or nfs"
            )),
        }
    }
}
impl fmt::Display for StorageMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_policy_defaults_to_wal_and_rejects_invalid_content() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            StorageMode::read_home(directory.path()).unwrap(),
            StorageMode::Wal
        );
        std::fs::write(directory.path().join("storage-mode"), "nfs\n").unwrap();
        assert_eq!(
            StorageMode::read_home(directory.path()).unwrap(),
            StorageMode::Nfs
        );
        std::fs::write(directory.path().join("storage-mode"), "unknown").unwrap();
        assert!(StorageMode::read_home(directory.path()).is_err());
    }

    #[test]
    fn selection_preserves_durability_and_rejects_unknown_modes() {
        assert_eq!(StorageMode::default(), StorageMode::Wal);
        assert_eq!("nfs".parse::<StorageMode>().unwrap(), StorageMode::Nfs);
        for value in ["", "auto", "off", "NFS"] {
            assert!(value.parse::<StorageMode>().is_err());
        }
        assert!(StorageMode::Nfs.pragmas().contains("synchronous=FULL"));
        assert!(!StorageMode::Nfs.pragmas().contains("WAL"));
    }
}
