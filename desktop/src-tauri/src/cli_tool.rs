//! CLI tool discovery is separate from Agent ownership. Never execute an empty
//! externalBin placeholder, search the current directory or invoke a shell.
use std::path::{Path, PathBuf};
use std::time::Duration;

fn usable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !path.is_absolute() || !metadata.is_file() || metadata.len() == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return false;
        }
    }
    true
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Descriptor {
    pub path: PathBuf,
    pub source: String,
    pub version: String,
    pub capabilities: future_rpc::cli_capabilities::CliCapabilities,
}

#[derive(Clone, Debug, PartialEq)]
struct Identity {
    path: PathBuf,
    length: u64,
    modified: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
    #[cfg(unix)]
    inode: (u64, u64, i64, i64),
}
impl Identity {
    fn read(path: &Path) -> std::io::Result<Self> {
        let metadata = path.metadata()?;
        Ok(Self {
            path: path.canonicalize()?,
            length: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            #[cfg(unix)]
            inode: {
                use std::os::unix::fs::MetadataExt;
                (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                )
            },
        })
    }
}
struct CachedProbe {
    identity: Identity,
    checked_at: std::time::Instant,
    result: Result<Descriptor, String>,
}
static CACHE: std::sync::Mutex<Option<CachedProbe>> = std::sync::Mutex::new(None);

pub(crate) fn resolve() -> Result<PathBuf, crate::AppError> {
    discover(None).map(|tool| tool.path)
}

pub(crate) fn resolve_for(capability: &str) -> Result<PathBuf, crate::AppError> {
    discover(Some(capability)).map(|tool| tool.path)
}

pub(crate) fn discover(capability: Option<&str>) -> Result<Descriptor, crate::AppError> {
    let executable = std::env::current_exe()?;
    let parent = executable
        .parent()
        .ok_or("Desktop executable has no parent directory")?;
    let name = if cfg!(windows) {
        "future.exe"
    } else {
        "future"
    };
    let path = resolve_from(parent, name, std::env::var_os("PATH").as_deref())
        .ok_or("No usable future CLI found. Install the matching CLI on PATH or beside Desktop; CLI-dependent skills and maintenance are unavailable.")?;
    let identity = Identity::read(&path)?;
    let mut cache = CACHE.lock().unwrap_or_else(|error| error.into_inner());
    let source = if path == parent.join(name) {
        "bundled"
    } else if path.parent().is_some_and(|p| {
        std::env::var_os("PATH")
            .is_some_and(|value| std::env::split_paths(&value).any(|entry| entry == p))
    }) {
        "path"
    } else {
        "development"
    };
    if cache.as_ref().is_none_or(|cached| {
        cached.identity != identity
            || (cached.result.is_err()
                && cached.checked_at.elapsed() >= std::time::Duration::from_secs(5))
    }) {
        let result = probe(&path, source).and_then(|descriptor| {
            if Identity::read(&path).ok().as_ref() != Some(&identity) {
                return Err(
                    "CLI changed during discovery; retry after the build or installation completes"
                        .into(),
                );
            }
            Ok(descriptor)
        });
        *cache = Some(CachedProbe {
            identity,
            checked_at: std::time::Instant::now(),
            result,
        });
    }
    let descriptor = cache.as_ref().unwrap().result.clone().map_err(|error| format!("CLI {}: {error}. Install a matching CLI beside Desktop or correct PATH; no fallback CLI was executed.", path.display()))?;
    descriptor
        .capabilities
        .validate(capability)
        .map_err(|error| {
            format!(
                "CLI {} ({}): {error}. Install the matching CLI.",
                descriptor.path.display(),
                descriptor.version
            )
        })?;
    Ok(descriptor)
}

fn decode_probe(path: &Path, source: &str, bytes: &[u8]) -> Result<Descriptor, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Reply {
        version: String,
        desktop_capabilities: future_rpc::cli_capabilities::CliCapabilities,
    }
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("Unsupported version/capability response: {error}"))?;
    let reported = value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let reply: Reply = serde_json::from_value(value)
        .map_err(|error| format!("version {reported}: unsupported capability response: {error}"))?;
    if reply.version.trim().is_empty() {
        return Err("CLI returned an empty display version".into());
    }
    reply
        .desktop_capabilities
        .validate(None)
        .map_err(|error| format!("version {}: {error}", reply.version))?;
    Ok(Descriptor {
        path: path.into(),
        source: source.into(),
        version: reply.version,
        capabilities: reply.desktop_capabilities,
    })
}

/// How long `probe` may run before it kills the child.
///
/// `Some(budget)` is the production rule: a CLI that never answers must not
/// block Desktop startup forever. `None` is a test-only seam (see
/// [`PROBE_DEADLINE_OVERRIDE`]): no deadline, so the child's own end decides.
///
/// The seam exists for the tests that assert what the probe *captured*. With a
/// deadline in play those assertions are decided by a race between a busy host's
/// scheduler and this clock, not by the code under test:
/// `docs/testing/flaky-tauri.md` records the measurement (and the invocation the
/// campaign's inner loop has to use for the race to be visible at all). The arm
/// that asserts the *kill* keeps the production budget -
/// `a_silent_cli_is_killed_and_reported_as_a_timeout` - and that test is what
/// pins this constant to 3 s.
const PROBE_DEADLINE: Duration = Duration::from_secs(3);

fn probe_deadline() -> Option<Duration> {
    #[cfg(test)]
    if let Some(overridden) = PROBE_DEADLINE_OVERRIDE.with(|cell| *cell.borrow()) {
        return overridden;
    }
    Some(PROBE_DEADLINE)
}

// Test-only override for `probe_deadline` (a plain comment: rustdoc does not
// document macro invocations, and `unused_doc_comments` is denied here).
// `Some(None)` = no deadline (the capture tests), `Some(Some(d))` = a `d`
// deadline (both arms of the control in
// `a_reply_that_arrives_after_the_deadline_is_captured_only_without_one`). It is
// deliberately **thread-local** rather than a process-wide static: libtest runs
// each test on its own thread, so a value that outlives the test that set it
// cannot reach another test's probe - the leak class this file already had to fix
// once (`EnvGuard`, `docs/testing/flaky-tauri.md` item 27).
#[cfg(test)]
thread_local! {
    static PROBE_DEADLINE_OVERRIDE: std::cell::RefCell<Option<Option<Duration>>> =
        const { std::cell::RefCell::new(None) };
}

/// Arms [`PROBE_DEADLINE_OVERRIDE`] and clears it on drop, so even a panicking
/// test cannot hand its deadline to the next test on the same thread.
#[cfg(test)]
struct ProbeDeadlineGuard;

#[cfg(test)]
impl ProbeDeadlineGuard {
    fn set(deadline: Option<Duration>) -> Self {
        PROBE_DEADLINE_OVERRIDE.with(|cell| *cell.borrow_mut() = Some(deadline));
        ProbeDeadlineGuard
    }
}

#[cfg(test)]
impl Drop for ProbeDeadlineGuard {
    fn drop(&mut self) {
        PROBE_DEADLINE_OVERRIDE.with(|cell| *cell.borrow_mut() = None);
    }
}

fn probe(path: &Path, source: &str) -> Result<Descriptor, String> {
    use std::{
        io::{Read, Seek},
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    const LIMIT: u64 = 64 * 1024;
    // File-backed capture avoids a blocked pipe or an unbounded reader thread
    // if a broken CLI inherits its output handles into another process.
    #[cfg(windows)]
    {
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut header = [0; 64];
        file.read_exact(&mut header)
            .map_err(|_| "CLI is not a Windows PE executable")?;
        let offset = u32::from_le_bytes(header[60..64].try_into().unwrap()) as u64;
        file.seek(std::io::SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        let mut signature = [0; 4];
        file.read_exact(&mut signature)
            .map_err(|_| "CLI has no PE header")?;
        if &header[..2] != b"MZ" || &signature != b"PE\0\0" {
            return Err("CLI is not a Windows PE executable".into());
        }
    }
    let mut stdout = tempfile::tempfile().map_err(|e| e.to_string())?;
    let stderr = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut command = Command::new(path);
    command
        .args(["version", "--json"])
        .stdin(Stdio::null())
        .stdout(stdout.try_clone().map_err(|e| e.to_string())?)
        .stderr(stderr.try_clone().map_err(|e| e.to_string())?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot probe CLI: {e}"))?;
    // `None` means no deadline (test seam): the loop can then only end on the
    // child's own exit or on the output limit.
    let deadline = probe_deadline().map(|budget| Instant::now() + budget);
    let status = loop {
        let oversized = stdout.metadata().map(|m| m.len() > LIMIT).unwrap_or(true)
            || stderr.metadata().map(|m| m.len() > LIMIT).unwrap_or(true);
        if oversized || deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if oversized {
                "CLI discovery exceeded its output limit"
            } else {
                "CLI discovery timed out"
            }
            .into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        }
    };
    if stderr.metadata().map_err(|e| e.to_string())?.len() > LIMIT {
        return Err("CLI discovery exceeded its output limit".into());
    }
    if !status.success() {
        return Err(format!("CLI version discovery exited with {status}"));
    }
    stdout.rewind().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    stdout
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("CLI discovery exceeded its output limit".into());
    }
    decode_probe(path, source, &bytes)
}

fn resolve_from(parent: &Path, name: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let mut candidates = vec![parent.join(name)];
    // Compile-time checkout paths are development-only, never a packaged fallback.
    #[cfg(all(debug_assertions, not(test)))]
    {
        if let Some(target) = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            candidates.push(target.join("debug").join(name));
        }
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        candidates.push(manifest.join("../../target/debug").join(name));
        candidates.push(manifest.join("../../target/release").join(name));
        let ext = if cfg!(windows) { ".exe" } else { "" };
        candidates.push(
            manifest
                .join("binaries")
                .join(format!("future-{}{ext}", env!("FUTURE_TARGET_TRIPLE"))),
        );
    }
    if let Some(path) = path {
        candidates.extend(
            std::env::split_paths(path)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(name)),
        );
    }
    candidates.into_iter().find(|p| usable(p))
}

/// Called only before the host starts threads, after login-shell hydration.
pub(crate) fn prepare_path() -> Result<(), crate::AppError> {
    let cli = match resolve() {
        Ok(cli) => cli,
        Err(error) => {
            eprintln!("FutureOS: {error}");
            return Ok(());
        }
    };
    let parent = cli
        .parent()
        .ok_or("CLI executable has no parent directory")?;
    let mut directories = vec![parent.to_path_buf()];
    if let Some(path) = std::env::var_os("PATH") {
        directories.extend(std::env::split_paths(&path).filter(|p| p != parent));
    }
    std::env::set_var(
        "PATH",
        std::env::join_paths(directories).map_err(|e| e.to_string())?,
    );
    Ok(())
}

/// Restores a process-global environment variable when it is dropped, including
/// while the test that replaced it is unwinding.
///
/// `cargo test` runs every test of one target in ONE process, so a value left
/// behind by a panicking assertion is inherited by unrelated tests: replacing
/// `PATH` with a fixture directory and panicking before the restore made 25
/// later tests in `commands::*`/`git_review::*` fail with `NotFound` while
/// spawning `git` (FLAKES.md item 27, measured in `docs/testing/flaky-tauri.md`).
/// Holding a `Mutex` is not a substitute — a lock prevents a race between tests,
/// it does not restore a leaked value — so a global-state test needs both.
///
/// ```text
/// let _path = EnvGuard::set("PATH", directory.as_os_str());
/// ```
#[cfg(test)]
#[must_use = "the previous value is restored when the guard is dropped; bind it to a variable"]
pub(crate) struct EnvGuard {
    key: &'static str,
    saved: Option<std::ffi::OsString>,
}

#[cfg(test)]
impl EnvGuard {
    /// Replaces the variable `key` and remembers what to put back on drop.
    pub(crate) fn set(key: &'static str, value: &std::ffi::OsStr) -> Self {
        let saved = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, saved }
    }
}

#[cfg(test)]
impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.saved.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_discovery_rejects_legacy_and_incompatible_tools() {
        let path = Path::new("/fixture/future");
        assert!(decode_probe(path, "bundled", br#"{"version":"old"}"#).is_err());
        assert!(decode_probe(path, "bundled", b"not JSON").is_err());
        let mut capabilities = future_rpc::cli_capabilities::CliCapabilities::default();
        let response =
            serde_json::json!({"version": "display-version", "desktopCapabilities": capabilities});
        let tool = decode_probe(path, "bundled", &serde_json::to_vec(&response).unwrap()).unwrap();
        assert_eq!(tool.path, path);
        assert_eq!(tool.source, "bundled");
        capabilities.storage_schema += 1;
        let response =
            serde_json::json!({"version": "old-version", "desktopCapabilities": capabilities});
        let error =
            decode_probe(path, "bundled", &serde_json::to_vec(&response).unwrap()).unwrap_err();
        assert!(error.contains("old-version"));
        assert!(error.contains("incompatible"));
    }

    // These fixtures are inspected as files, never executed (including Windows).
    fn tool(directory: &Path) -> PathBuf {
        std::fs::create_dir_all(directory).unwrap();
        let path = directory.join("future");
        std::fs::write(&path, b"CLI discovery fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[test]
    fn bundled_cli_is_preferred_over_path_cli() {
        let root = tempfile::tempdir().unwrap();
        let bundled = tool(&root.path().join("bundle"));
        let installed = tool(&root.path().join("installed"));
        let path = std::env::join_paths([installed.parent().unwrap()]).unwrap();
        assert_eq!(
            resolve_from(bundled.parent().unwrap(), "future", Some(&path)),
            Some(bundled)
        );
    }

    #[test]
    fn placeholders_and_directories_do_not_hide_a_usable_path_cli() {
        let root = tempfile::tempdir().unwrap();
        let bundle = root.path().join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("future"), []).unwrap();
        let directory = root.path().join("directory");
        std::fs::create_dir_all(directory.join("future")).unwrap();
        let installed = tool(&root.path().join("installed"));
        let invalid_path = std::env::join_paths([directory.as_path()]).unwrap();
        assert_eq!(resolve_from(&bundle, "future", Some(&invalid_path)), None);
        let path =
            std::env::join_paths([directory.as_path(), installed.parent().unwrap()]).unwrap();
        assert_eq!(
            resolve_from(&bundle, "future", Some(&path)),
            Some(installed)
        );
    }

    #[test]
    fn relative_path_entries_are_never_searched() {
        let root = tempfile::tempdir().unwrap();
        let installed = tool(&root.path().join("installed"));
        let relative = std::env::join_paths([Path::new("."), Path::new("installed")]).unwrap();
        assert_eq!(resolve_from(root.path(), "future", Some(&relative)), None);
        let path = std::env::join_paths([Path::new("."), installed.parent().unwrap()]).unwrap();
        assert_eq!(
            resolve_from(root.path(), "future", Some(&path)),
            Some(installed)
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_executable_bundle_does_not_hide_a_usable_path_cli() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let bundled = tool(&root.path().join("bundle"));
        std::fs::set_permissions(&bundled, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            resolve_from(bundled.parent().unwrap(), "future", None),
            None
        );
        let installed = tool(&root.path().join("installed"));
        let path = std::env::join_paths([installed.parent().unwrap()]).unwrap();
        assert_eq!(
            resolve_from(bundled.parent().unwrap(), "future", Some(&path)),
            Some(installed)
        );
    }

    /// A *runnable* CLI fixture. The tests above only inspect the file; the
    /// discovery tests have to execute it, because `probe` reads the version and
    /// capability contract from a real child process.
    #[cfg(unix)]
    fn script_cli(directory: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(directory).unwrap();
        let path = directory.join("future");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// The probe reply a compatible CLI prints: a display version plus the
    /// capability set this Desktop expects.
    fn reply(commands: &[&str]) -> String {
        let capabilities = future_rpc::cli_capabilities::CliCapabilities {
            commands: commands.iter().map(|c| c.to_string()).collect(),
            ..Default::default()
        };
        serde_json::json!({ "version": "1.2.3", "desktopCapabilities": capabilities }).to_string()
    }

    fn print_reply(commands: &[&str]) -> String {
        format!("printf '%s' '{}'", reply(commands))
    }

    /// Serialize the tests that replace process-global `PATH`.
    fn path_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Discovery runs the CLI it found, reads the version/capability contract
    /// from it, and reports that the binary came from `PATH` — the source a
    /// user-installed CLI must be labelled with.
    #[cfg(unix)]
    #[test]
    fn a_runnable_cli_on_path_is_discovered_and_probed() {
        let _lock = path_lock();
        // No deadline: the reply is captured because the fixture ends, not
        // because a busy host managed to schedule it inside 3 s.
        let _deadline = ProbeDeadlineGuard::set(None);
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("bin");
        let cli = script_cli(&directory, &print_reply(&["skills.init"]));
        let _path = EnvGuard::set("PATH", directory.as_os_str());

        let descriptor = discover(Some("skills.init")).expect("a compatible CLI must resolve");
        assert_eq!(descriptor.path, cli);
        assert_eq!(descriptor.source, "path");
        assert_eq!(descriptor.version, "1.2.3");
        assert_eq!(
            descriptor.capabilities.commands,
            vec!["skills.init".to_string()]
        );
        assert_eq!(resolve_for("skills.init").unwrap(), cli);
    }

    /// A CLI that does not advertise the capability the caller needs is refused
    /// *by name*, with its identity, instead of being used anyway.
    #[cfg(unix)]
    #[test]
    fn a_missing_capability_is_reported_with_the_cli_identity() {
        let _lock = path_lock();
        let _deadline = ProbeDeadlineGuard::set(None);
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("bin");
        script_cli(&directory, &print_reply(&["version"]));
        let _path = EnvGuard::set("PATH", directory.as_os_str());

        let error = discover(Some("skills.init")).unwrap_err().to_string();
        assert!(
            error.contains("does not advertise capability skills.init"),
            "{error}"
        );
        assert!(
            error.contains("1.2.3"),
            "the CLI's identity must be named: {error}"
        );
        assert!(error.contains("Install the matching CLI"), "{error}");
    }

    /// A CLI that changes while it is being probed is rejected: caching a
    /// descriptor that no longer describes the binary on disk is exactly how a
    /// half-finished install would be used.
    #[cfg(unix)]
    #[test]
    fn a_cli_that_changes_during_discovery_is_rejected() {
        let _lock = path_lock();
        let _deadline = ProbeDeadlineGuard::set(None);
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("bin");
        // Prints a valid reply and then grows itself, so the identity re-read
        // after the probe sees a different file. The appended byte is a newline:
        // a shell command word would make the *next* run of the same script fail
        // (`x: command not found`) and hide what this test measures.
        script_cli(
            &directory,
            &format!(
                "printf '%s' '{}'; printf '\\n' >> \"$0\"",
                reply(&["skills.init"])
            ),
        );
        let _path = EnvGuard::set("PATH", directory.as_os_str());

        let error = discover(Some("skills.init")).unwrap_err().to_string();
        assert!(error.contains("CLI changed during discovery"), "{error}");
    }

    /// `prepare_path` puts the discovered CLI's directory first, so every
    /// maintenance command this process spawns resolves the same binary the
    /// Desktop validated.
    #[cfg(unix)]
    #[test]
    fn prepare_path_prepends_the_discovered_cli() {
        let _lock = path_lock();
        let _deadline = ProbeDeadlineGuard::set(None);
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("bin");
        script_cli(&directory, &print_reply(&["skills.init"]));
        let _path = EnvGuard::set("PATH", directory.as_os_str());

        prepare_path().expect("a discovered CLI is prepended to PATH");
        let prepared = std::env::var_os("PATH").expect("PATH is set");
        let entries: Vec<PathBuf> = std::env::split_paths(&prepared).collect();
        assert_eq!(
            entries.first().map(PathBuf::as_path),
            Some(directory.as_path())
        );
    }

    /// A display version of whitespace is not a version: it would be rendered as
    /// an empty string in the UI and compared against nothing.
    #[test]
    fn a_blank_display_version_is_rejected() {
        let capabilities = future_rpc::cli_capabilities::CliCapabilities::default();
        let response = serde_json::json!({ "version": "   ", "desktopCapabilities": capabilities });
        let error = decode_probe(
            Path::new("/fixture/future"),
            "bundled",
            &serde_json::to_vec(&response).unwrap(),
        )
        .unwrap_err();
        assert!(error.contains("empty display version"), "{error}");
    }

    /// A CLI that never answers is killed and reported as a timeout rather than
    /// blocking Desktop startup forever.
    #[cfg(unix)]
    #[test]
    fn a_silent_cli_is_killed_and_reported_as_a_timeout() {
        let root = tempfile::tempdir().unwrap();
        let cli = script_cli(&root.path().join("bin"), "sleep 30");
        let started = std::time::Instant::now();
        let error = probe(&cli, "path").unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(
            started.elapsed() >= std::time::Duration::from_secs(3),
            "the probe deadline must be honoured: {:?}",
            started.elapsed()
        );
    }

    /// A CLI that floods its output is killed as soon as the limit is crossed:
    /// the captured file must not grow with the child.
    #[cfg(unix)]
    #[test]
    fn a_flooding_cli_is_killed_at_the_output_limit() {
        // The limit, not the clock, is what this test is about.
        let _deadline = ProbeDeadlineGuard::set(None);
        let root = tempfile::tempdir().unwrap();
        let cli = script_cli(
            &root.path().join("bin"),
            "dd if=/dev/zero bs=1024 count=200 2>/dev/null",
        );
        let error = probe(&cli, "path").unwrap_err();
        assert!(error.contains("output limit"), "{error}");
    }

    /// A non-zero exit is reported with its status instead of being parsed as an
    /// empty reply.
    #[cfg(unix)]
    #[test]
    fn a_failing_cli_is_reported_with_its_exit_status() {
        let _deadline = ProbeDeadlineGuard::set(None);
        let root = tempfile::tempdir().unwrap();
        let cli = script_cli(&root.path().join("bin"), "exit 3");
        let error = probe(&cli, "path").unwrap_err();
        assert!(error.contains("exited with"), "{error}");
    }

    /// The seam must not be able to change what Desktop ships: with no override
    /// armed, and regardless of what a previous test on this thread armed, the
    /// deadline is the production budget and nothing else.
    #[test]
    fn the_production_probe_deadline_is_unchanged_without_an_override() {
        assert_eq!(PROBE_DEADLINE, Duration::from_secs(3));
        assert_eq!(probe_deadline(), Some(PROBE_DEADLINE));
        // A guard that has been dropped must leave nothing behind, which is what
        // makes the seam safe to arm inside a test at all.
        let armed = ProbeDeadlineGuard::set(None);
        assert_eq!(probe_deadline(), None);
        drop(armed);
        assert_eq!(probe_deadline(), Some(PROBE_DEADLINE));
    }

    /// The defect the seam removes, as a deterministic control: no load, no
    /// timing luck. A fixture whose *correct* reply arrives after the deadline is
    /// captured or lost depending only on whether a deadline is in play.
    ///
    /// The first arm is the production rule, so it also documents the real
    /// behaviour a busy host can produce (a slow but correct CLI is killed and
    /// reported as a timeout); the second arm is the state the capture tests run
    /// in, which is what removes the race from their assertions.
    ///
    /// The delay is `/bin/sleep`, not `sleep`: this test is about the deadline,
    /// and a lookup through `PATH` would make it answer a second question (the
    /// concurrent-`PATH` race, NEW-C in `docs/testing/flaky-tauri.md`) and report
    /// the wrong mechanism when it fails.
    #[cfg(unix)]
    #[test]
    fn a_reply_that_arrives_after_the_deadline_is_captured_only_without_one() {
        let root = tempfile::tempdir().unwrap();
        let cli = script_cli(
            &root.path().join("bin"),
            &format!(
                "/bin/sleep 4
{}",
                print_reply(&["skills.init"])
            ),
        );

        {
            let _deadline = ProbeDeadlineGuard::set(Some(PROBE_DEADLINE));
            let error = probe(&cli, "path").unwrap_err();
            assert!(
                error.contains("timed out"),
                "the production budget must lose a reply that arrives after it: {error}"
            );
        }

        let _deadline = ProbeDeadlineGuard::set(None);
        let descriptor = probe(&cli, "path").unwrap_or_else(|error| {
            panic!("without a deadline the fixture's reply must be captured: {error}")
        });
        assert_eq!(descriptor.version, "1.2.3");
        assert_eq!(
            descriptor.capabilities.commands,
            vec!["skills.init".to_string()]
        );
    }
}
