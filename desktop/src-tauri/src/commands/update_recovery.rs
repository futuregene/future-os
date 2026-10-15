//! Restart a failed non-Windows update only if the original program is intact.
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};

pub(super) struct ProgramFingerprint {
    roots: Vec<PathBuf>,
    files: BTreeMap<PathBuf, Vec<u8>>,
}
impl ProgramFingerprint {
    pub(super) fn capture() -> std::io::Result<Self> {
        let executable = std::env::current_exe()?;
        #[cfg(target_os = "macos")]
        let executable = tauri_plugin_updater::extract_path_from_executable(&executable)
            .map_err(std::io::Error::other)?;
        let mut roots = vec![executable];
        if let Ok(cli) = crate::cli_tool::resolve() {
            roots.push(cli);
        }
        Self::from_roots(roots)
    }
    fn from_roots(roots: Vec<PathBuf>) -> std::io::Result<Self> {
        let mut files = BTreeMap::new();
        for root in &roots {
            fingerprint(root, &mut files)?;
        }
        Ok(Self { roots, files })
    }
    pub(super) fn unchanged(&self) -> bool {
        Self::from_roots(self.roots.clone()).is_ok_and(|current| current.files == self.files)
    }
}
fn fingerprint(path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    use std::os::unix::fs::PermissionsExt;
    let mode = metadata.permissions().mode().to_le_bytes();
    if metadata.is_symlink() {
        files.insert(
            path.to_owned(),
            std::fs::read_link(path)?
                .as_os_str()
                .as_encoded_bytes()
                .to_vec(),
        );
    } else if metadata.is_dir() {
        files.insert(path.to_owned(), mode.to_vec());
        for entry in std::fs::read_dir(path)? {
            fingerprint(&entry?.path(), files)?;
        }
    } else if metadata.is_file() {
        let mut file = std::fs::File::open(path)?;
        let mut hash = Sha256::new();
        hash.update(mode);
        let mut buffer = [0; 16384];
        loop {
            let size = file.read(&mut buffer)?;
            if size == 0 {
                break;
            }
            hash.update(&buffer[..size]);
        }
        files.insert(path.to_owned(), hash.finalize().to_vec());
    } else {
        return Err(std::io::Error::other("Unexpected installation file type"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changed_or_missing_bundle_files_disable_relaunch() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("program");
        std::fs::write(&file, b"original").unwrap();
        let fingerprint =
            ProgramFingerprint::from_roots(vec![directory.path().to_owned()]).unwrap();
        assert!(fingerprint.unchanged());
        std::fs::write(&file, b"partial update").unwrap();
        assert!(!fingerprint.unchanged());
        std::fs::remove_file(file).unwrap();
        assert!(!fingerprint.unchanged());
    }
}
