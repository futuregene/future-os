use serde::Serialize;
use std::ffi::OsString;
use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const REQUIRED_BWRAP_OPTIONS: &[&str] = &[
    "--new-session",
    "--die-with-parent",
    "--unshare-user",
    "--unshare-pid",
    "--unshare-ipc",
    "--cap-drop",
    "--ro-bind",
    "--dev",
    "--proc",
];

pub const BASELINE_BWRAP_ARGS: &[&str] = &[
    "--new-session",
    "--die-with-parent",
    "--unshare-user",
    "--unshare-pid",
    "--unshare-ipc",
    "--cap-drop",
    "ALL",
    "--ro-bind",
    "/",
    "/",
    "--dev",
    "/dev",
    "--proc",
    "/proc",
    "--",
    "/bin/true",
];

const PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinuxSandboxProbeCode {
    Available,
    PlatformNotLinux,
    BinaryMissing,
    PathRejected,
    BinaryInvalid,
    VersionUnreadable,
    VersionTooOld,
    RequiredFeatureMissing,
    UserNamespaceDisabled,
    ProcMountRestricted,
    ProbeTimeout,
    ProbeFailed,
    BinaryIdentityChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BwrapIdentity {
    pub device: u64,
    pub inode: u64,
    pub size: u64,
    pub modified_nanos: u128,
}

impl BwrapIdentity {
    fn from_metadata(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        #[cfg(unix)]
        let (device, inode) = (metadata.dev(), metadata.ino());
        #[cfg(not(unix))]
        let (device, inode) = (0, 0);
        let modified_nanos = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_nanos());
        Self {
            device,
            inode,
            size: metadata.len(),
            modified_nanos,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinuxSandboxCapabilities {
    pub user_namespace: bool,
    pub pid_namespace: bool,
    pub ipc_namespace: bool,
    pub read_only_root: bool,
    pub fresh_proc: bool,
    pub minimal_dev: bool,
    pub capability_drop: bool,
    pub network_isolation: bool,
    pub dynamic_glob_protection: bool,
}

impl LinuxSandboxCapabilities {
    fn baseline() -> Self {
        Self {
            user_namespace: true,
            pid_namespace: true,
            ipc_namespace: true,
            read_only_root: true,
            fresh_proc: true,
            minimal_dev: true,
            capability_drop: true,
            network_isolation: false,
            dynamic_glob_protection: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinuxSandboxProbe {
    pub available: bool,
    pub code: LinuxSandboxProbeCode,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub identity: Option<BwrapIdentity>,
    pub capabilities: Option<LinuxSandboxCapabilities>,
    pub expires_at_unix_ms: Option<u128>,
    #[serde(skip)]
    diagnostic: Option<String>,
}

impl LinuxSandboxProbe {
    fn unavailable(code: LinuxSandboxProbeCode, diagnostic: impl Into<String>) -> Self {
        Self {
            available: false,
            code,
            path: None,
            version: None,
            identity: None,
            capabilities: None,
            expires_at_unix_ms: None,
            diagnostic: Some(diagnostic.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProbeCommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

pub trait ProbeHost {
    fn metadata(&self, path: &Path) -> std::io::Result<Metadata>;
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf>;
    fn run(&self, program: &Path, args: &[&str], timeout: Duration) -> ProbeCommandOutput;
    fn now(&self) -> SystemTime;
}

struct SystemProbeHost;

impl ProbeHost for SystemProbeHost {
    fn metadata(&self, path: &Path) -> std::io::Result<Metadata> {
        std::fs::metadata(path)
    }

    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }

    fn run(&self, program: &Path, args: &[&str], timeout: Duration) -> ProbeCommandOutput {
        run_bounded(program, args, timeout)
    }

    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

#[derive(Debug, Default)]
pub struct LinuxProbeCache {
    success: Option<(LinuxSandboxProbe, SystemTime)>,
}

impl LinuxProbeCache {
    pub fn get_or_probe(
        &mut self,
        host: &dyn ProbeHost,
        path: Option<OsString>,
        workspace: &Path,
        cwd: &Path,
    ) -> LinuxSandboxProbe {
        if let Some((cached, expires_at)) = &self.success {
            if host.now() < *expires_at {
                if let (Some(path), Some(expected)) = (&cached.path, &cached.identity) {
                    if executable_identity(host, path).as_ref() == Some(expected) {
                        return cached.clone();
                    }
                }
                self.success = None;
            } else {
                self.success = None;
            }
        }

        let result = probe_with_host(host, path, workspace, cwd);
        if result.available {
            let expires_at = host.now() + CACHE_TTL;
            self.success = Some((result.clone(), expires_at));
        }
        result
    }
}

pub fn probe_linux_sandbox_host() -> LinuxSandboxProbe {
    #[cfg(not(target_os = "linux"))]
    {
        return LinuxSandboxProbe::unavailable(
            LinuxSandboxProbeCode::PlatformNotLinux,
            "bubblewrap is supported only on native Linux",
        );
    }
    #[cfg(target_os = "linux")]
    {
        static CACHE: std::sync::OnceLock<parking_lot::Mutex<LinuxProbeCache>> =
            std::sync::OnceLock::new();
        let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let cwd = workspace.clone();
        CACHE
            .get_or_init(|| parking_lot::Mutex::new(LinuxProbeCache::default()))
            .lock()
            .get_or_probe(&SystemProbeHost, std::env::var_os("PATH"), &workspace, &cwd)
    }
}

pub fn probe_with_host(
    host: &dyn ProbeHost,
    path: Option<OsString>,
    workspace: &Path,
    cwd: &Path,
) -> LinuxSandboxProbe {
    let (binary, identity) = match discover_bwrap(host, path, workspace, cwd) {
        Ok(found) => found,
        Err((code, message)) => return LinuxSandboxProbe::unavailable(code, message),
    };

    let version_output = host.run(&binary, &["--version"], PROBE_TIMEOUT);
    if version_output.timed_out {
        return LinuxSandboxProbe::unavailable(
            LinuxSandboxProbeCode::ProbeTimeout,
            "bwrap --version timed out",
        );
    }
    let version_text = format!("{}\n{}", version_output.stdout, version_output.stderr);
    let Some(version) = parse_bwrap_version(&version_text) else {
        return LinuxSandboxProbe::unavailable(
            LinuxSandboxProbeCode::VersionUnreadable,
            "bwrap version output was not recognized",
        );
    };
    if !version_output.success {
        return LinuxSandboxProbe::unavailable(
            LinuxSandboxProbeCode::VersionUnreadable,
            "bwrap --version failed",
        );
    }

    let help = host.run(&binary, &["--help"], PROBE_TIMEOUT);
    if help.timed_out {
        return LinuxSandboxProbe::unavailable(
            LinuxSandboxProbeCode::ProbeTimeout,
            "bwrap --help timed out",
        );
    }
    let help_text = format!("{}\n{}", help.stdout, help.stderr);
    if !help.success
        || REQUIRED_BWRAP_OPTIONS
            .iter()
            .any(|option| !help_text.contains(option))
    {
        return LinuxSandboxProbe::unavailable(
            LinuxSandboxProbeCode::RequiredFeatureMissing,
            "bwrap help does not advertise every required option",
        );
    }

    let baseline = host.run(&binary, BASELINE_BWRAP_ARGS, PROBE_TIMEOUT);
    if baseline.timed_out {
        return LinuxSandboxProbe::unavailable(
            LinuxSandboxProbeCode::ProbeTimeout,
            "bubblewrap baseline probe timed out",
        );
    }
    if !baseline.success {
        let diagnostic = format!("{}\n{}", baseline.stdout, baseline.stderr).to_lowercase();
        let code = if diagnostic.contains("user namespace")
            || diagnostic.contains("unprivileged_userns")
            || diagnostic.contains("operation not permitted")
        {
            LinuxSandboxProbeCode::UserNamespaceDisabled
        } else if diagnostic.contains("/proc") || diagnostic.contains("procfs") {
            LinuxSandboxProbeCode::ProcMountRestricted
        } else {
            LinuxSandboxProbeCode::ProbeFailed
        };
        return LinuxSandboxProbe::unavailable(code, "bubblewrap baseline probe failed");
    }

    let expires_at = host
        .now()
        .checked_add(CACHE_TTL)
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis());
    LinuxSandboxProbe {
        available: true,
        code: LinuxSandboxProbeCode::Available,
        path: Some(binary),
        version: Some(version),
        identity: Some(identity),
        capabilities: Some(LinuxSandboxCapabilities::baseline()),
        expires_at_unix_ms: expires_at,
        diagnostic: None,
    }
}

fn discover_bwrap(
    host: &dyn ProbeHost,
    path: Option<OsString>,
    workspace: &Path,
    cwd: &Path,
) -> Result<(PathBuf, BwrapIdentity), (LinuxSandboxProbeCode, String)> {
    let Some(path) = path else {
        return Err((LinuxSandboxProbeCode::BinaryMissing, "PATH is unset".into()));
    };
    let workspace = host
        .canonicalize(workspace)
        .unwrap_or_else(|_| workspace.to_path_buf());
    let cwd = host.canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let mut rejected = false;
    for directory in std::env::split_paths(&path) {
        if !directory.is_absolute() {
            rejected = true;
            continue;
        }
        let candidate = directory.join("bwrap");
        let Ok(canonical) = host.canonicalize(&candidate) else {
            continue;
        };
        if super::super::paths::path_within(&canonical, &workspace)
            || super::super::paths::path_within(&canonical, &cwd)
        {
            rejected = true;
            continue;
        }
        let Some(identity) = executable_identity(host, &canonical) else {
            rejected = true;
            continue;
        };
        return Ok((canonical, identity));
    }
    if rejected {
        Err((
            LinuxSandboxProbeCode::PathRejected,
            "PATH contained only unsafe or invalid bwrap candidates".into(),
        ))
    } else {
        Err((
            LinuxSandboxProbeCode::BinaryMissing,
            "no bwrap executable was found on safe absolute PATH entries".into(),
        ))
    }
}

fn executable_identity(host: &dyn ProbeHost, path: &Path) -> Option<BwrapIdentity> {
    let metadata = host.metadata(path).ok()?;
    if !metadata.is_file() || !is_executable(&metadata) {
        return None;
    }
    Some(BwrapIdentity::from_metadata(&metadata))
}

fn is_executable(metadata: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        true
    }
}

fn parse_bwrap_version(output: &str) -> Option<String> {
    output.split_whitespace().find_map(|token| {
        let candidate = token.trim_matches(|ch: char| !ch.is_ascii_digit() && ch != '.');
        let mut parts = candidate.split('.');
        let major = parts.next()?;
        let minor = parts.next()?;
        if major.chars().all(|ch| ch.is_ascii_digit())
            && minor.chars().all(|ch| ch.is_ascii_digit())
            && parts.all(|part| part.chars().all(|ch| ch.is_ascii_digit()))
        {
            Some(candidate.to_string())
        } else {
            None
        }
    })
}

fn run_bounded(program: &Path, args: &[&str], timeout: Duration) -> ProbeCommandOutput {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = child else {
        return ProbeCommandOutput {
            success: false,
            stdout: String::new(),
            stderr: "spawn failed".into(),
            timed_out: false,
        };
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child.wait_with_output();
                return match output {
                    Ok(output) => ProbeCommandOutput {
                        success: output.status.success(),
                        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                        timed_out: false,
                    },
                    Err(error) => ProbeCommandOutput {
                        success: false,
                        stdout: String::new(),
                        stderr: error.to_string(),
                        timed_out: false,
                    },
                };
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return ProbeCommandOutput {
                    success: false,
                    stdout: String::new(),
                    stderr: String::new(),
                    timed_out: true,
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct FakeHost {
        now: SystemTime,
        outputs: Mutex<VecDeque<ProbeCommandOutput>>,
    }

    impl ProbeHost for FakeHost {
        fn metadata(&self, path: &Path) -> std::io::Result<Metadata> {
            std::fs::metadata(path)
        }
        fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
            std::fs::canonicalize(path)
        }
        fn run(&self, _program: &Path, _args: &[&str], _timeout: Duration) -> ProbeCommandOutput {
            self.outputs.lock().unwrap().pop_front().unwrap()
        }
        fn now(&self) -> SystemTime {
            self.now
        }
    }

    fn output(success: bool, stdout: &str, stderr: &str) -> ProbeCommandOutput {
        ProbeCommandOutput {
            success,
            stdout: stdout.into(),
            stderr: stderr.into(),
            timed_out: false,
        }
    }

    fn executable(dir: &Path) -> PathBuf {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("bwrap");
        std::fs::write(&path, "fake").unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn success_outputs() -> VecDeque<ProbeCommandOutput> {
        VecDeque::from([
            output(true, "bubblewrap 0.11.1", ""),
            output(true, &REQUIRED_BWRAP_OPTIONS.join(" "), ""),
            output(true, "", ""),
        ])
    }

    #[test]
    fn rejects_relative_and_workspace_path_entries() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        executable(&workspace.join("bin"));
        let host = FakeHost {
            now: UNIX_EPOCH,
            outputs: Mutex::new(VecDeque::new()),
        };
        let joined =
            std::env::join_paths([PathBuf::from("relative"), workspace.join("bin")]).unwrap();
        let probe = probe_with_host(&host, Some(joined), &workspace, &workspace);
        assert_eq!(probe.code, LinuxSandboxProbeCode::PathRejected);
    }

    #[test]
    fn success_records_fixed_identity_capabilities_and_expiry() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let system = root.path().join("system");
        let binary = executable(&system);
        let host = FakeHost {
            now: UNIX_EPOCH + Duration::from_secs(10),
            outputs: Mutex::new(success_outputs()),
        };
        let probe = probe_with_host(
            &host,
            Some(std::env::join_paths([system]).unwrap()),
            &workspace,
            &workspace,
        );
        assert!(probe.available);
        assert_eq!(probe.path.as_deref(), Some(binary.as_path()));
        assert_eq!(probe.version.as_deref(), Some("0.11.1"));
        assert_eq!(probe.expires_at_unix_ms, Some(310_000));
        assert!(!probe.capabilities.unwrap().network_isolation);
    }

    #[test]
    fn version_help_timeout_and_runtime_failures_are_typed() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let system = root.path().join("system");
        executable(&system);
        let path = Some(std::env::join_paths([system]).unwrap());

        for (outputs, expected) in [
            (
                VecDeque::from([output(true, "not-a-version", "")]),
                LinuxSandboxProbeCode::VersionUnreadable,
            ),
            (
                VecDeque::from([output(true, "bwrap 1.0", ""), output(true, "--ro-bind", "")]),
                LinuxSandboxProbeCode::RequiredFeatureMissing,
            ),
            (
                VecDeque::from([
                    output(true, "bwrap 1.0", ""),
                    output(true, &REQUIRED_BWRAP_OPTIONS.join(" "), ""),
                    output(
                        false,
                        "",
                        "creating user namespace: Operation not permitted",
                    ),
                ]),
                LinuxSandboxProbeCode::UserNamespaceDisabled,
            ),
        ] {
            let host = FakeHost {
                now: UNIX_EPOCH,
                outputs: Mutex::new(outputs),
            };
            assert_eq!(
                probe_with_host(&host, path.clone(), &workspace, &workspace).code,
                expected
            );
        }
    }

    #[test]
    fn timeout_is_reported_before_running_later_probe_stages() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let system = root.path().join("system");
        executable(&system);
        let host = FakeHost {
            now: UNIX_EPOCH,
            outputs: Mutex::new(VecDeque::from([ProbeCommandOutput {
                success: false,
                stdout: String::new(),
                stderr: String::new(),
                timed_out: true,
            }])),
        };

        let probe = probe_with_host(
            &host,
            Some(std::env::join_paths([system]).unwrap()),
            &workspace,
            &workspace,
        );
        assert_eq!(probe.code, LinuxSandboxProbeCode::ProbeTimeout);
        assert!(!probe.available);
        assert!(host.outputs.lock().unwrap().is_empty());
    }

    #[test]
    fn cache_reuses_only_unexpired_matching_identity() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let system = root.path().join("system");
        let binary = executable(&system);
        let path = Some(std::env::join_paths([system]).unwrap());
        let host = FakeHost {
            now: UNIX_EPOCH,
            outputs: Mutex::new(success_outputs()),
        };
        let mut cache = LinuxProbeCache::default();
        assert!(
            cache
                .get_or_probe(&host, path.clone(), &workspace, &workspace)
                .available
        );
        assert!(
            cache
                .get_or_probe(&host, path.clone(), &workspace, &workspace)
                .available
        );
        assert!(host.outputs.lock().unwrap().is_empty());

        std::fs::write(&binary, "changed identity").unwrap();
        host.outputs
            .lock()
            .unwrap()
            .push_back(output(true, "not-a-version", ""));
        let failed = cache.get_or_probe(&host, path, &workspace, &workspace);
        assert!(!failed.available);
        assert_eq!(failed.code, LinuxSandboxProbeCode::VersionUnreadable);
    }

    #[test]
    fn production_argument_list_and_help_contract_cannot_drift() {
        for required in REQUIRED_BWRAP_OPTIONS {
            assert!(BASELINE_BWRAP_ARGS.contains(required), "{required}");
        }
        assert!(!BASELINE_BWRAP_ARGS.contains(&"--unshare-net"));
        assert!(!REQUIRED_BWRAP_OPTIONS.contains(&"--argv0"));
        assert!(!REQUIRED_BWRAP_OPTIONS.contains(&"--ro-bind-fd"));
    }
}
