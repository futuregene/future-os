use super::probe::LinuxSandboxProbe;
use super::request::{HelperPhase, LinuxSandboxRequest, MountKind, MountRequest, REQUEST_VERSION};
use crate::sandbox::backend::{PreparedShell, SandboxBoundary, ShellBackend};
use crate::sandbox::linux::plan::LinuxSandboxPlan;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum LinuxSandboxRunnerError {
    #[error("Linux sandbox probe did not provide a verified backend receipt")]
    InvalidProbeReceipt,
    #[error("Linux sandbox plan still contains unexpanded glob rules")]
    UnexpandedGlob,
    #[error("Linux sandbox plan contains missing protected paths")]
    MissingPathUnsupported,
    #[error("current FutureOS executable could not be resolved: {0}")]
    CurrentExecutable(std::io::Error),
    #[error(transparent)]
    InvalidRequest(#[from] super::request::RequestError),
}

pub fn prepare(
    probe: &LinuxSandboxProbe,
    plan: LinuxSandboxPlan,
    command: &str,
    cwd: &Path,
) -> Result<PreparedShell, LinuxSandboxRunnerError> {
    let (Some(bwrap_path), Some(bwrap_identity)) = (&probe.path, &probe.identity) else {
        return Err(LinuxSandboxRunnerError::InvalidProbeReceipt);
    };
    if !probe.available {
        return Err(LinuxSandboxRunnerError::InvalidProbeReceipt);
    }
    // L3 expands these before execution. Until that compiler lands, refusing
    // the command is safer than silently dropping a protection rule.
    if !plan.unsupported_dynamic_globs.is_empty() {
        return Err(LinuxSandboxRunnerError::UnexpandedGlob);
    }
    if !plan.missing_protected_paths.is_empty() {
        return Err(LinuxSandboxRunnerError::MissingPathUnsupported);
    }

    let mut mounts = Vec::new();
    extend_mounts(&mut mounts, &plan.writable_roots, MountKind::Writable);
    extend_mounts(&mut mounts, &plan.read_only_paths, MountKind::ReadOnly);
    extend_mounts(&mut mounts, &plan.unreadable_paths, MountKind::Unreadable);
    // Reopens deliberately come last so a narrow allow wins over a wider
    // lower-priority protection mount.
    extend_mounts(&mut mounts, &plan.reopened_paths, MountKind::Writable);

    let request = LinuxSandboxRequest {
        version: REQUEST_VERSION,
        phase: HelperPhase::Outer,
        bwrap_path: bwrap_path.clone(),
        bwrap_identity: bwrap_identity.clone(),
        cwd: cwd.to_path_buf(),
        argv: shell_argv(command),
        mounts,
        policy_digest: plan.policy_digest.clone(),
    };
    let encoded = request.encode()?;
    let executable = std::env::current_exe().map_err(LinuxSandboxRunnerError::CurrentExecutable)?;
    let args = helper_args(&executable, encoded);
    Ok(PreparedShell {
        program: executable.to_string_lossy().into_owned(),
        args,
        env_delta: std::collections::BTreeMap::new(),
        boundary: SandboxBoundary {
            backend: ShellBackend::LinuxBubblewrap,
            policy_digest: Some(plan.policy_digest),
        },
    })
}

fn extend_mounts(mounts: &mut Vec<MountRequest>, paths: &[PathBuf], kind: MountKind) {
    mounts.extend(paths.iter().cloned().map(|path| MountRequest {
        source: path.clone(),
        target: path,
        kind,
        expected: None,
        source_fd: None,
    }));
}

pub(crate) fn helper_args(executable: &Path, encoded: String) -> Vec<String> {
    let mut args = Vec::new();
    let is_unified = executable
        .file_stem()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("future"));
    if is_unified {
        args.push("agent".into());
    }
    args.push("--linux-sandbox-helper".into());
    args.push(encoded);
    args
}

fn shell_argv(command: &str) -> Vec<String> {
    let (program, args) = crate::sandbox::shell_invocation(command);
    std::iter::once(program.to_string()).chain(args).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::linux::probe::{BwrapIdentity, LinuxSandboxProbeCode};

    fn probe() -> LinuxSandboxProbe {
        LinuxSandboxProbe {
            available: true,
            code: LinuxSandboxProbeCode::Available,
            path: Some(PathBuf::from("/usr/bin/bwrap")),
            version: Some("1.0.0".into()),
            identity: Some(BwrapIdentity {
                device: 1,
                inode: 2,
                size: 3,
                modified_nanos: 4,
            }),
            capabilities: None,
            expires_at_unix_ms: None,
            diagnostic: None,
        }
    }

    fn plan() -> LinuxSandboxPlan {
        LinuxSandboxPlan {
            writable_roots: vec![PathBuf::from("/tmp/work")],
            read_only_paths: Vec::new(),
            unreadable_paths: Vec::new(),
            reopened_paths: Vec::new(),
            missing_protected_paths: Vec::new(),
            unsupported_dynamic_globs: Vec::new(),
            policy_digest: "a".repeat(64),
        }
    }

    #[test]
    fn preparation_is_structured_and_fail_closed_for_unfinished_policy_features() {
        let prepared = prepare(&probe(), plan(), "echo ok", Path::new("/tmp/work")).unwrap();
        assert_eq!(prepared.boundary.backend, ShellBackend::LinuxBubblewrap);
        assert_eq!(
            prepared.boundary.policy_digest.as_deref(),
            Some(&*"a".repeat(64))
        );
        assert!(prepared
            .args
            .iter()
            .any(|arg| arg == "--linux-sandbox-helper"));

        let mut glob = plan();
        glob.unsupported_dynamic_globs.push("/tmp/**/*.pem".into());
        assert!(matches!(
            prepare(&probe(), glob, "true", Path::new("/tmp/work")),
            Err(LinuxSandboxRunnerError::UnexpandedGlob)
        ));
        let mut missing = plan();
        missing
            .missing_protected_paths
            .push(PathBuf::from("/tmp/missing"));
        assert!(matches!(
            prepare(&probe(), missing, "true", Path::new("/tmp/work")),
            Err(LinuxSandboxRunnerError::MissingPathUnsupported)
        ));
    }

    #[test]
    fn helper_dispatch_supports_unified_and_standalone_binaries() {
        assert_eq!(
            helper_args(Path::new("/opt/future"), "x".into()),
            ["agent", "--linux-sandbox-helper", "x"]
        );
        assert_eq!(
            helper_args(Path::new("/opt/future-agent"), "x".into()),
            ["--linux-sandbox-helper", "x"]
        );
    }
}
