//! Restart a failed non-Windows update only if the original program is intact.
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ProgramFingerprint {
    roots: Vec<PathBuf>,
    files: BTreeMap<PathBuf, Vec<u8>>,
}
impl ProgramFingerprint {
    pub(super) fn save(&self, record: &Path) -> Result<(), crate::AppError> {
        crate::config_io::write_json_atomic(
            &record.with_file_name("original-files.json"),
            &serde_json::to_value(self)?,
            true,
        )
    }
    pub(super) fn verify_saved(operation_id: &str) -> Result<(), crate::AppError> {
        let path = future_rpc::home::future_home()
            .join("app/updates")
            .join(operation_id)
            .join("original-files.json");
        let original: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        if !original.unchanged() {
            return Err(
                "Original program files have changed; repair the installation before restarting."
                    .into(),
            );
        }
        Ok(())
    }
    pub(super) fn capture() -> std::io::Result<Self> {
        Self::capture_under(std::env::current_exe()?, crate::cli_tool::resolve().ok())
    }
    /// `capture` with its two environment probes supplied by the caller: the
    /// executable whose installation is fingerprinted, and the resolved CLI (or
    /// `None` when there is none). Splitting them out is what lets a test take a
    /// fingerprint of a throwaway installation tree instead of the running build
    /// tree, which is what `capture` itself would hash. The production call site
    /// and the resulting fingerprint are unchanged.
    pub(super) fn capture_under(
        executable: PathBuf,
        cli: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        #[cfg(target_os = "macos")]
        let executable = tauri_plugin_updater::extract_path_from_executable(&executable)
            .map_err(std::io::Error::other)?;
        let mut roots = vec![executable];
        if let Some(cli) = cli {
            let canonical = cli.canonicalize()?;
            if canonical != cli {
                roots.push(canonical);
            }
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
    fn persisted_restoration_snapshot_is_rechecked_before_later_restart() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-original-snapshot");
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("program");
        std::fs::write(&file, b"original").unwrap();
        let original = ProgramFingerprint::from_roots(vec![file.clone()]).unwrap();
        let operation = super::super::journal::Operation::begin("old").unwrap();
        original.save(&operation.path()).unwrap();
        let id = super::super::journal::latest()
            .unwrap()
            .unwrap()
            .operation_id;
        assert!(ProgramFingerprint::verify_saved(&id).is_ok());
        std::fs::write(file, b"changed after the failure dialog").unwrap();
        assert!(ProgramFingerprint::verify_saved(&id).is_err());
    }

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

    /// A directory is fingerprinted by its mode and a symlink by the path it
    /// points at, so replacing a link after a failed install is a change.
    #[cfg(unix)]
    #[test]
    fn fingerprinting_records_directories_files_and_symlink_targets() {
        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("program");
        std::fs::write(&program, b"original").unwrap();
        let link = directory.path().join("current");
        std::os::unix::fs::symlink(&program, &link).unwrap();

        let mut files = BTreeMap::new();
        fingerprint(directory.path(), &mut files).unwrap();
        assert!(files.contains_key(directory.path()));
        assert!(files.contains_key(program.as_path()));
        assert_eq!(
            files.get(link.as_path()).expect("the symlink is recorded"),
            &program.as_os_str().as_encoded_bytes().to_vec()
        );
    }

    /// Anything in the program tree that is neither a file, a directory nor a
    /// symlink is unknown state: the fingerprint must fail closed rather than
    /// treat it as intact.
    #[cfg(unix)]
    #[test]
    fn an_unexpected_installation_file_type_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let pipe = directory.path().join("pipe");
        let status = std::process::Command::new("mkfifo")
            .arg(&pipe)
            .status()
            .expect("mkfifo");
        assert!(status.success());

        let error = match ProgramFingerprint::from_roots(vec![pipe]) {
            Ok(_) => panic!("a FIFO in the program tree is not a program file"),
            Err(error) => error,
        };
        assert!(error
            .to_string()
            .contains("Unexpected installation file type"));
    }

    /// `capture_under` fingerprints the installation it is *given* — the whole
    /// point of the parameter, since `capture`'s own tree is the running build
    /// directory. The captured fingerprint is then a real contract: it stays
    /// `unchanged()` until a byte of the program changes.
    #[test]
    fn capture_under_fingerprints_the_installation_it_is_given() {
        let directory = tempfile::tempdir().unwrap();
        // A macOS bundle shape, so the platform extraction resolves a root
        // *above* the executable rather than the executable itself.
        let macos = directory
            .path()
            .join("FutureOS.app")
            .join("Contents")
            .join("MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        let executable = macos.join("futureos");
        std::fs::write(&executable, b"the desktop binary").unwrap();
        let cli = directory.path().join("future-cli");
        std::fs::write(&cli, b"the cli binary").unwrap();

        let fingerprint =
            ProgramFingerprint::capture_under(executable.clone(), Some(cli.clone())).unwrap();
        assert!(
            fingerprint.files.contains_key(&executable),
            "the program itself is fingerprinted: {:?}",
            fingerprint.files.keys().collect::<Vec<_>>()
        );
        assert!(
            fingerprint
                .files
                .keys()
                .any(|root| executable.starts_with(root)),
            "the fingerprint carries the installation root, not just the file"
        );
        assert!(fingerprint.unchanged(), "a fresh capture is intact");

        std::fs::write(&executable, b"a tampered desktop binary").unwrap();
        assert!(
            !fingerprint.unchanged(),
            "a changed installation is no longer the captured one"
        );
    }

    /// A CLI probe that failed contributes no root: the fingerprint is only the
    /// program the updater is about to replace.
    #[test]
    fn capture_under_without_a_cli_fingerprints_only_the_program() {
        let directory = tempfile::tempdir().unwrap();
        let install = directory.path().join("install");
        std::fs::create_dir_all(&install).unwrap();
        let executable = install.join("futureos");
        std::fs::write(&executable, b"the desktop binary").unwrap();
        let absent_cli = directory.path().join("future-cli");
        std::fs::write(&absent_cli, b"the cli binary").unwrap();

        let fingerprint = ProgramFingerprint::capture_under(executable, None).unwrap();
        assert!(
            !fingerprint.files.contains_key(&absent_cli),
            "a probe that returned nothing is not a root"
        );
        assert!(fingerprint.unchanged());
    }
}
