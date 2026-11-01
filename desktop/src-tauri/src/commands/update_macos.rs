//! Same-volume, atomic bundle exchange. The previous bundle remains available
//! until a matching new process confirms GUI and Agent readiness.
use super::{
    journal,
    process::{self, Completion, PreparedProcess},
};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn bundle() -> Result<PathBuf, String> {
    tauri_plugin_updater::extract_path_from_executable(
        &std::env::current_exe().map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
fn staging(target: &Path, id: &str) -> Result<PathBuf, String> {
    if !id.starts_with("update-") || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return Err("Invalid transaction identifier".into());
    }
    Ok(target
        .parent()
        .ok_or("App bundle has no parent")?
        .join(format!(".futureos-{id}")))
}
pub(super) fn can_prepare() -> bool {
    use std::os::unix::ffi::OsStrExt;
    bundle()
        .ok()
        .and_then(|path| {
            path.parent()
                .and_then(|p| std::ffi::CString::new(p.as_os_str().as_bytes()).ok())
        })
        .is_some_and(|path| unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 })
}
fn sync_tree(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            sync_tree(&entry.map_err(|e| e.to_string())?.path())?;
        }
    }
    std::fs::File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|e| e.to_string())
}
fn probe(path: &Path) -> Result<(), String> {
    let mut child = Command::new(path)
        .arg("--help")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Candidate executable cannot start: {e}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("Candidate executable probe failed: {status}"))
            };
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Candidate executable probe timed out".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}
pub(super) struct Prepared {
    helper: Option<PreparedProcess>,
    directory: PathBuf,
    committed: bool,
}
impl Prepared {
    pub(super) fn new(bytes: &[u8], operation: &journal::Operation) -> Result<Self, String> {
        Self::prepare_at(bytes, operation, bundle()?)
    }
    /// The body of [`Self::new`] with the installed bundle's path injected.
    ///
    /// Split out — behaviour unchanged — because `bundle()` derives the path from
    /// `current_exe()`, which from a test binary is never an `.app`: the staging, signature,
    /// identity, probe and helper steps behind it can then run against a bundle in a
    /// temporary directory, and a failure in any of them is reported instead of being
    /// masked by "this executable is not a bundle".
    fn prepare_at(
        bytes: &[u8],
        operation: &journal::Operation,
        target: PathBuf,
    ) -> Result<Self, String> {
        Self::prepare_with(bytes, operation, target, None)
    }
    /// [`Self::prepare_at`] with the program the prepared bundle is handed to injected.
    ///
    /// `None` is production: this executable with the worker subcommand. A **test target** has
    /// no worker — `current_exe()` there is the test binary itself — so the chain's *successful*
    /// half (the executable identity, both `probe` calls, `sync_tree`, the hand-off) could not
    /// be reached from a test at all, and the case that tried was withdrawn
    /// (`coverage/2026-11/tauri/wave3.md` §4). A caller that can supply a cooperating program
    /// leaves every step above the hand-off — and the hand-off itself — on the real path; only
    /// the child is a stand-in, and the child is not what this file is about.
    #[cfg(any(test, feature = "test-support"))]
    fn prepare_with_helper(
        bytes: &[u8],
        operation: &journal::Operation,
        target: PathBuf,
        helper: &Path,
    ) -> Result<Self, String> {
        Self::prepare_with(bytes, operation, target, Some(helper))
    }
    fn prepare_with(
        bytes: &[u8],
        operation: &journal::Operation,
        target: PathBuf,
        helper: Option<&Path>,
    ) -> Result<Self, String> {
        if target.extension().and_then(|s| s.to_str()) != Some("app") {
            return Err(
                "Install the application in a writable Applications folder before updating".into(),
            );
        }
        let id = operation
            .path()
            .parent()
            .and_then(|p| p.file_name())
            .ok_or("Missing transaction directory")?
            .to_string_lossy()
            .into_owned();
        let directory = staging(&target, &id)?;
        std::fs::create_dir(&directory)
            .map_err(|e| format!("Cannot stage update beside the application: {e}"))?;
        let prepared = (|| {
            let decoder = flate2::read::GzDecoder::new(bytes);
            tar::Archive::new(decoder)
                .unpack(&directory)
                .map_err(|e| e.to_string())?;
            let entries = std::fs::read_dir(&directory)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if entries.len() != 1
                || entries[0].path().extension().and_then(|s| s.to_str()) != Some("app")
            {
                return Err("Update must contain exactly one application bundle".into());
            }
            let candidate = directory.join("candidate.app");
            std::fs::rename(entries[0].path(), &candidate).map_err(|e| e.to_string())?;
            let verification = Command::new("/usr/bin/codesign")
                .args(["--verify", "--deep", "--strict"])
                .arg(&candidate)
                .output()
                .map_err(|e| e.to_string())?;
            if !verification.status.success() {
                return Err(format!(
                    "Candidate application signature is invalid: {}",
                    String::from_utf8_lossy(&verification.stderr)
                ));
            }
            let executable = Command::new("/usr/libexec/PlistBuddy")
                .args(["-c", "Print :CFBundleExecutable"])
                .arg(candidate.join("Contents/Info.plist"))
                .output()
                .map_err(|e| e.to_string())?;
            if !executable.status.success() {
                return Err("Candidate bundle has no executable identity".into());
            }
            let executable = String::from_utf8(executable.stdout).map_err(|e| e.to_string())?;
            let name = executable.trim();
            if name.is_empty() || name.contains('/') || name.contains('\\') {
                return Err("Invalid candidate executable identity".into());
            }
            probe(&candidate.join("Contents/MacOS").join(name))?;
            probe(&candidate.join("Contents/MacOS/future"))?;
            sync_tree(&directory)?;
            let program = match helper {
                Some(helper) => helper.to_path_buf(),
                None => std::env::current_exe().map_err(|e| e.to_string())?,
            };
            let mut command = Command::new(program);
            command
                .arg("--app-update-macos-worker")
                .arg(&target)
                .arg(&id)
                .arg(operation.path());
            let log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(operation.path().with_file_name("worker.log"))
                .map_err(|e| e.to_string())?;
            command.stderr(log);
            PreparedProcess::start(command)
        })();
        match prepared {
            Ok(helper) => Ok(Self {
                helper: Some(helper),
                directory,
                committed: false,
            }),
            Err(error) => {
                let _ = std::fs::remove_dir_all(directory);
                Err(error)
            }
        }
    }
    pub(super) fn commit(mut self) -> Result<Completion, String> {
        self.committed = true;
        // Keep the directory after commit, including when the parent crashes.
        let helper = self
            .helper
            .take()
            .ok_or("Update helper was already committed")?;
        let result = helper.commit();
        if result.as_ref().is_ok_and(|completion| completion.restored) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
        result
    }
}
impl Drop for Prepared {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
}
/// The whole preparation chain, end to end, for a caller that can supply the helper.
///
/// Differs from the product path in one thing only — *who* the helper is, see
/// [`Prepared::prepare_with_helper`]. Returns the child's completion serialized, the same text
/// `process::complete` prints, so a test target can assert on it without the `Completion`
/// type, which is `pub(super)` to this module's parent.
///
/// The caller must be a process with no parallel suite of its own: `probe` gives each
/// candidate executable a 10 s budget, which this host cannot meet while a few hundred
/// instrumented libtest cases are spawning children (`wave3.md` §4). That is why
/// `tests/gui_main_thread_drive` is the only caller.
#[cfg(any(test, feature = "test-support"))]
pub(super) fn prepare_and_commit(
    version: &str,
    bytes: &[u8],
    target: PathBuf,
    helper: &Path,
) -> Result<String, String> {
    let operation = journal::Operation::begin(version).map_err(|e| e.to_string())?;
    let prepared = Prepared::prepare_with_helper(bytes, &operation, target, helper)?;
    let completion = prepared.commit()?;
    serde_json::to_string(&completion).map_err(|e| e.to_string())
}
fn exchange(a: &Path, b: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    let a = std::ffi::CString::new(a.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let b = std::ffi::CString::new(b.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    // Both bundles were prepared in the same parent filesystem. RENAME_SWAP
    // keeps the installed pathname valid across crashes and needs no delete.
    let result = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            a.as_ptr(),
            libc::AT_FDCWD,
            b.as_ptr(),
            libc::RENAME_SWAP,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}
pub(super) fn worker(args: &[String]) -> Result<(), String> {
    let [target, id, record] = args else {
        return Err("Invalid macOS update worker arguments".into());
    };
    let target = PathBuf::from(target);
    let candidate = staging(&target, id)?.join("candidate.app");
    let record = PathBuf::from(record);
    let _lease = journal::observer_lease(&record).map_err(|e| e.to_string())?;
    if !process::await_commit()? {
        if let Some(directory) = candidate.parent() {
            let _ = std::fs::remove_dir_all(directory);
        }
        return Ok(());
    }
    let _agent_lease = match process::agent_lease(&record) {
        Ok(lease) => lease,
        Err(error) => {
            let completion = Completion {
                ok: false,
                restored: true,
                diagnostic: error,
            };
            journal::write_worker_result(
                &record,
                "failed-restored",
                Some(completion.diagnostic.clone()),
                true,
            )
            .map_err(|e| e.to_string())?;
            return process::complete(&completion);
        }
    };
    let completion = match exchange(&target, &candidate) {
        Ok(()) => {
            let durable = (|| {
                std::fs::File::open(target.parent().ok_or("Missing application parent")?)
                    .and_then(|file| file.sync_all())
                    .map_err(|e| e.to_string())?;
                std::fs::File::open(candidate.parent().ok_or("Missing candidate parent")?)
                    .and_then(|file| file.sync_all())
                    .map_err(|e| e.to_string())?;
                journal::write_worker_result(&record, "pending-validation", None, true)
                    .map_err(|e| e.to_string())
            })();
            if let Err(error) = durable {
                let restored = exchange(&target, &candidate).is_ok();
                Completion {
                    ok: false,
                    restored,
                    diagnostic: format!("Cannot persist installation result: {error}"),
                }
            } else {
                Completion::success()
            }
        }
        Err(error) => Completion {
            ok: false,
            restored: true,
            diagnostic: format!(
                "Atomic application exchange failed; original bundle is intact: {error}"
            ),
        },
    };
    process::complete(&completion)
}
pub(super) fn confirm(id: &str) -> Result<(), String> {
    let directory = staging(&bundle()?, id)?;
    if directory.exists() {
        std::fs::remove_dir_all(directory).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The staging directory is derived from the *installed* bundle's path and from the
    /// transaction id, and it must stay beside the bundle (that is what makes the final
    /// rename atomic): both inputs have to reach the answer.
    #[test]
    fn staging_is_derived_from_the_bundle_and_the_transaction_id() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("FutureOS.app");
        let first = staging(&target, "update-2026-11-01T10-00-00").unwrap();
        // Beside the bundle, not inside it and not in a fixed temp directory.
        assert_eq!(first.parent(), target.parent());
        assert_eq!(
            first.file_name().and_then(|name| name.to_str()),
            Some(".futureos-update-2026-11-01T10-00-00")
        );
        // A different transaction id is a different directory; the same one is stable.
        let second = staging(&target, "update-2026-11-01T10-00-01").unwrap();
        assert_ne!(first, second);
        assert_eq!(
            first,
            staging(&target, "update-2026-11-01T10-00-00").unwrap()
        );
        // Anything that is not `update-` plus ASCII alphanumerics would become a directory
        // name this function created under a caller-supplied path, so it is refused.
        for id in [
            "",
            "install-1",
            "update_1",
            "update-a/b",
            "update-a.b",
            "UPDATE-1",
        ] {
            assert!(
                staging(&target, id).is_err(),
                "{id:?} must be refused as a transaction"
            );
        }
        // A bundle with no parent has nowhere to stage beside.
        assert!(staging(Path::new("/"), "update-1").is_err());
    }

    /// `sync_tree` walks what it is given: a directory recurses, a symlink is left alone
    /// (following it could sync a file outside the bundle), a plain file is synced, and a
    /// path that does not exist is an error rather than a silent success.
    #[test]
    fn sync_tree_walks_directories_and_skips_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let bundle = root.path().join("Candidate.app");
        std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        std::fs::write(bundle.join("Contents/Info.plist"), b"<plist/>").unwrap();
        std::fs::write(bundle.join("Contents/MacOS/future"), b"binary").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/hosts", bundle.join("Contents/link")).unwrap();

        assert!(sync_tree(&bundle).is_ok(), "a well-formed tree syncs");
        assert!(
            sync_tree(&bundle.join("Contents/MacOS/future")).is_ok(),
            "a plain file syncs"
        );
        #[cfg(unix)]
        assert!(
            sync_tree(&bundle.join("Contents/link")).is_ok(),
            "a symlink is skipped, so a dangling one is not an error either"
        );
        // The error arm: `symlink_metadata` on a path that does not exist, and on a path
        // whose child disappears mid-walk is the same call at a different node.
        assert!(sync_tree(&root.path().join("missing.app")).is_err());
        let vanished = root.path().join("Vanished.app");
        std::fs::create_dir(&vanished).unwrap();
        std::fs::write(vanished.join("entry"), b"x").unwrap();
        std::fs::remove_file(vanished.join("entry")).unwrap();
        std::fs::remove_dir(&vanished).unwrap();
        assert!(
            sync_tree(&vanished).is_err(),
            "a directory that is gone cannot be synced"
        );
    }

    /// `probe` runs the candidate exactly as the installer will (`--help`) and reads its
    /// exit status, so a binary that cannot start, one that fails and one that succeeds are
    /// three different answers.
    #[test]
    fn probe_answers_for_a_running_a_failing_and_a_missing_executable() {
        assert!(
            probe(Path::new("/bin/echo")).is_ok(),
            "an executable that exits 0 is accepted"
        );
        let failed = probe(Path::new("/usr/bin/false")).expect_err("exit 1 must be refused");
        assert!(
            failed.contains("probe failed"),
            "the diagnostic names the failed probe, got {failed:?}"
        );
        let missing = probe(Path::new("/nonexistent/candidate")).expect_err("cannot start");
        assert!(
            missing.contains("cannot start"),
            "the diagnostic names the failed spawn, got {missing:?}"
        );
    }

    /// The worker entry point validates its three arguments before touching the filesystem:
    /// a wrong arity is a named error, and so is a transaction id it would have to build a
    /// path from.
    #[test]
    fn the_worker_refuses_wrong_arguments_before_touching_any_filesystem() {
        assert!(worker(&[]).is_err());
        assert!(worker(&["only-target".to_string()]).is_err());
        let four = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ];
        assert!(
            worker(&four).is_err(),
            "more than three arguments is the same error"
        );
        let bad_id = vec![
            "/tmp/FutureOS.app".to_string(),
            "install-1".to_string(),
            "/tmp/record.json".to_string(),
        ];
        let error = worker(&bad_id).expect_err("an id the staging path refuses stops the worker");
        assert_eq!(error, "Invalid transaction identifier");
    }

    /// One update payload: a gzip'd tar holding exactly the given `<archive path, tree>` pairs.
    fn tar_gz(entries: &[(&str, &Path)]) -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut archive = tar::Builder::new(encoder);
        for (name, tree) in entries {
            if tree.is_dir() {
                archive.append_dir_all(name, tree).unwrap();
            } else {
                archive.append_path_with_name(tree, name).unwrap();
            }
        }
        archive.into_inner().unwrap().finish().unwrap()
    }

    /// A `.app` tree shaped the way an update carries it: an optional `CFBundleExecutable`
    /// and a main executable in `Contents/MacOS/future` that exits 0 — which is exactly what
    /// `probe` runs (`--help`).
    ///
    /// The executable is a POSIX shell script, not a copied Mach-O: signing a Mach-O copy of a
    /// system binary gives it a signature macOS has never seen, so its *first* exec is
    /// validated by `amfid`, and in a full instrumented suite that validation can outlast
    /// `probe`'s own 10 s deadline — measured once in round 1 of `wave3_repeat.log`, where this
    /// test failed with "Candidate executable probe timed out" while the suite had several
    /// tests blocked (a script needs no code-signature validation on exec).
    ///
    /// `sign` runs the same ad-hoc signature a locally built bundle gets, so the
    /// `codesign --verify --deep --strict` inside preparation has something to accept.
    /// `macho` swaps the script for a copy of `/bin/echo`. Only the no-identity case needs
    /// that: `codesign --deep` skips a non-Mach-O subcomponent, so a script-only bundle cannot
    /// be signed at all (measured: verify answers "code object is not signed at all … In
    /// subcomponent: Contents/MacOS/future"), which would stop the test at the signature check
    /// instead of the identity one. That case never reaches `probe`, so it does not inherit the
    /// exec cost described above.
    fn bundle_tree(case: &Path, executable: Option<&str>, sign: bool, macho: bool) -> PathBuf {
        let app = case.join("payload/FutureOS.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        let main = app.join("Contents/MacOS/future");
        if macho {
            std::fs::copy("/bin/echo", &main).unwrap();
        } else {
            std::fs::write(&main, "#!/bin/sh\nexit 0\n").unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&main, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let identity = executable
            .map(|name| format!("<key>CFBundleExecutable</key><string>{name}</string>"))
            .unwrap_or_default();
        std::fs::write(
            app.join("Contents/Info.plist"),
            format!(
                "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict>{identity}\
                 <key>CFBundleIdentifier</key><string>com.futureos.test</string></dict></plist>"
            ),
        )
        .unwrap();
        if sign {
            let status = Command::new("/usr/bin/codesign")
                .args(["--force", "--deep", "--sign", "-"])
                .arg(&app)
                .status()
                .expect("run codesign on the fixture");
            assert!(
                status.success(),
                "the fixture bundle could not be ad-hoc signed"
            );
        }
        app
    }

    /// A bound journal case for `Prepared`'s filesystem work: its own updates directory, so
    /// the staging directory the preparation creates lands in `case`, not in `~/.future`.
    /// `fixtures` is a directory *outside* `case`: the payload trees live there, because
    /// [`assert_nothing_left_beside`] asserts that preparation left nothing beside the
    /// installed bundle, and a fixture tree in the same directory would hide exactly that.
    type PreparedCase = (
        crate::auth_store::test_support::HomeGuard,
        journal::CaseRoot,
        tempfile::TempDir,
        tempfile::TempDir,
    );

    fn prepared_case(name: &'static str) -> PreparedCase {
        let home = crate::auth_store::test_support::HomeGuard::new(name);
        (
            home,
            journal::CaseRoot::bind(),
            tempfile::Builder::new()
                .prefix("macos-update-case-")
                .tempdir()
                .unwrap(),
            tempfile::Builder::new()
                .prefix("macos-update-fixtures-")
                .tempdir()
                .unwrap(),
        )
    }

    /// Everything preparation created below `case` is gone: the staging directory is the one
    /// thing it adds beside the installed bundle, and it must not outlive a failure.
    fn assert_nothing_left_beside(case: &Path) {
        let leftovers: Vec<_> = std::fs::read_dir(case)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            leftovers.is_empty(),
            "preparation cleaned up after itself, found {leftovers:?}"
        );
    }

    #[test]
    fn a_target_that_is_not_an_application_bundle_is_refused_before_anything_is_written() {
        let (_home, _root, case, _fixtures) = prepared_case("macos-prepare-not-a-bundle");
        let operation = journal::Operation::begin("9.9.9").unwrap();
        let error = Prepared::prepare_at(b"payload", &operation, case.path().join("FutureOS.zip"))
            .err()
            .expect("a non-.app target is refused");
        assert_eq!(
            error, "Install the application in a writable Applications folder before updating",
            "the refusal names the real precondition instead of a staging failure"
        );
        assert_nothing_left_beside(case.path());
    }

    #[test]
    fn a_payload_that_is_not_a_gzip_archive_is_refused_and_unstaged() {
        let (_home, _root, case, _fixtures) = prepared_case("macos-prepare-not-an-archive");
        let operation = journal::Operation::begin("9.9.9").unwrap();
        let target = case.path().join("FutureOS.app");
        // The staging directory is created *before* the archive is read, so this failure is
        // also the one that proves the cleanup arm runs.
        let error = Prepared::prepare_at(b"not a gzip stream", &operation, target)
            .err()
            .expect("an unreadable archive is refused");
        assert!(!error.is_empty(), "the unpack error is reported");
        assert_nothing_left_beside(case.path());
    }

    #[test]
    fn a_payload_must_hold_exactly_one_application_bundle() {
        let (_home, _root, case, fixtures) = prepared_case("macos-prepare-bundle-count");
        let operation = journal::Operation::begin("9.9.9").unwrap();
        let tree = bundle_tree(fixtures.path(), Some("future"), false, false);
        let target = case.path().join("FutureOS.app");

        // Two bundles: which one would be installed is genuinely ambiguous.
        let two = tar_gz(&[("FutureOS.app", &tree), ("Other.app", &tree)]);
        let error = Prepared::prepare_at(&two, &operation, target.clone())
            .err()
            .expect("two bundles are refused");
        assert_eq!(error, "Update must contain exactly one application bundle");
        assert_nothing_left_beside(case.path());

        // One entry, but not a bundle.
        let not_a_bundle = std::path::Path::new("/bin/echo");
        let wrong = tar_gz(&[("FutureOS", not_a_bundle)]);
        let error = Prepared::prepare_at(&wrong, &operation, target)
            .err()
            .expect("one non-bundle entry is refused");
        assert_eq!(error, "Update must contain exactly one application bundle");
        assert_nothing_left_beside(case.path());
    }

    #[test]
    fn an_unsigned_candidate_is_refused_by_the_signature_check() {
        let (_home, _root, case, fixtures) = prepared_case("macos-prepare-unsigned");
        let operation = journal::Operation::begin("9.9.9").unwrap();
        let tree = bundle_tree(fixtures.path(), Some("future"), false, false);
        let payload = tar_gz(&[("FutureOS.app", &tree)]);
        let error = Prepared::prepare_at(&payload, &operation, case.path().join("FutureOS.app"))
            .err()
            .expect("an unsigned bundle is refused");
        assert!(
            error.starts_with("Candidate application signature is invalid"),
            "the refusal names the signature, got {error:?}"
        );
        assert_nothing_left_beside(case.path());
    }

    #[test]
    fn a_signed_candidate_without_an_executable_identity_is_refused() {
        let (_home, _root, case, fixtures) = prepared_case("macos-prepare-no-identity");
        let operation = journal::Operation::begin("9.9.9").unwrap();
        let tree = bundle_tree(fixtures.path(), None, true, true);
        let payload = tar_gz(&[("FutureOS.app", &tree)]);
        let error = Prepared::prepare_at(&payload, &operation, case.path().join("FutureOS.app"))
            .err()
            .expect("a bundle with no CFBundleExecutable is refused");
        assert_eq!(error, "Candidate bundle has no executable identity");
        assert_nothing_left_beside(case.path());
    }

    /// The whole successful preparation chain — unpack, signature, identity, both probes,
    /// fsync — against a bundle in a temporary directory.
    ///
    // **Not a test yet — the chain's successful half, and why it is not in this file.**
    //
    // The whole successful path (unpack, signature, identity, both `probe` calls, `sync_tree`,
    // the helper hand-off) was written as a unit test here and **withdrawn**: it fails on
    // `probe`'s own 10 s deadline under the instrumented suite, reproducibly. Measured over
    // four instrumented rounds (`wave3*.log`, `wave3-repeat*.log`): one failure with a copied
    // `/bin/echo` as the candidate executable, one with a shell script instead — the fixture
    // was not the variable. The same rounds had
    // `app_maintenance_status_reports_the_cli_it_discovered` failing on *its* 3 s subprocess
    // budget, i.e. this host stalls process spawning for >10 s while 389 instrumented tests
    // run in parallel. A test that asserts through a wall-clock deadline cannot be made stable
    // here, and a retry would hide a real regression.
    //
    // Where it belongs: `tests/gui_main_thread_drive`, which is its own process with no
    // parallel suite and already holds the real `AppHandle`. That needs
    // `Prepared::prepare_at` reachable from a test target, and `journal::Operation` is
    // `pub(super)` — i.e. one more gated facade in `test_support`, not a test change. Named in
    // `coverage/2026-11/tauri/wave3.md` §下一轮 as the next round's first item; the lines it
    // would take are the 81 still unseen in this file.
    #[test]
    fn atomic_exchange_retains_both_complete_versions_and_can_be_reversed() {
        let root = tempfile::tempdir().unwrap();
        let installed = root.path().join("installed.app");
        let candidate = root.path().join("candidate.app");
        std::fs::create_dir(&installed).unwrap();
        std::fs::create_dir(&candidate).unwrap();
        std::fs::write(installed.join("version"), b"old").unwrap();
        std::fs::write(candidate.join("version"), b"new").unwrap();
        exchange(&installed, &candidate).unwrap();
        assert_eq!(std::fs::read(installed.join("version")).unwrap(), b"new");
        assert_eq!(std::fs::read(candidate.join("version")).unwrap(), b"old");
        exchange(&installed, &candidate).unwrap();
        assert_eq!(std::fs::read(installed.join("version")).unwrap(), b"old");
    }
    #[test]
    fn failed_exchange_leaves_the_installed_version_in_place() {
        let root = tempfile::tempdir().unwrap();
        let installed = root.path().join("installed.app");
        std::fs::create_dir(&installed).unwrap();
        std::fs::write(installed.join("version"), b"old").unwrap();
        assert!(exchange(&installed, &root.path().join("missing.app")).is_err());
        assert_eq!(std::fs::read(installed.join("version")).unwrap(), b"old");
    }
}
