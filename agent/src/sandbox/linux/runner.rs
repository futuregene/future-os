use super::probe::LinuxSandboxProbe;
use super::request::{HelperPhase, LinuxSandboxRequest, MountKind, MountRequest, REQUEST_VERSION};
use crate::sandbox::backend::{PreparedShell, SandboxBoundary, ShellBackend};
use crate::sandbox::linux::plan::LinuxSandboxPlan;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum LinuxSandboxRunnerError {
    #[error("Linux sandbox probe did not provide a verified backend receipt")]
    InvalidProbeReceipt,
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
    let mut mounts = Vec::new();
    extend_mounts(&mut mounts, &plan.writable_roots, MountKind::Writable);
    extend_mounts(&mut mounts, &plan.read_only_paths, MountKind::ReadOnly);
    extend_mounts(&mut mounts, &plan.unreadable_paths, MountKind::Unreadable);
    extend_mounts(
        &mut mounts,
        &plan.missing_protected_paths,
        MountKind::MissingProtected,
    );
    extend_mounts(
        &mut mounts,
        &plan.reopened_read_only_paths,
        MountKind::ReadOnly,
    );
    extend_mounts(&mut mounts, &plan.reopened_paths, MountKind::Writable);
    // Bubblewrap applies mounts in argv order. Broad mounts must precede
    // narrow mounts so alternating deny/allow descendants remain visible;
    // for the same target, protection precedes the effective reopen.
    // `sort_by_key` is stable, so equal targets retain the construction order:
    // writable root, write protection, read protection, then effective reopen.
    mounts.sort_by_key(|mount| mount.target.components().count());

    let request = LinuxSandboxRequest {
        version: REQUEST_VERSION,
        phase: HelperPhase::Outer,
        bwrap_path: bwrap_path.clone(),
        bwrap_identity: bwrap_identity.clone(),
        cwd: cwd.to_path_buf(),
        argv: shell_argv(command),
        mounts,
        glob_snapshots: plan.glob_snapshots,
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
            reopened_read_only_paths: Vec::new(),
            missing_protected_paths: Vec::new(),
            unsupported_dynamic_globs: Vec::new(),
            glob_snapshots: Vec::new(),
            policy_digest: "a".repeat(64),
        }
    }

    #[test]
    fn preparation_is_structured_and_includes_complete_policy() {
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

        let mut complete = plan();
        complete
            .missing_protected_paths
            .push(PathBuf::from("/tmp/missing"));
        let prepared = prepare(&probe(), complete, "true", Path::new("/tmp/work")).unwrap();
        assert!(prepared
            .args
            .iter()
            .any(|arg| arg == "--linux-sandbox-helper"));
    }

    #[test]
    fn mount_order_preserves_alternating_broad_and_narrow_rules() {
        let mut policy = plan();
        policy.read_only_paths = vec![PathBuf::from("/tmp/work/vendor")];
        policy.reopened_paths = vec![PathBuf::from("/tmp/work/vendor/ok")];
        policy.unreadable_paths = vec![PathBuf::from("/tmp/work/vendor/ok/secret")];
        let prepared = prepare(&probe(), policy, "true", Path::new("/tmp/work")).unwrap();
        let encoded = prepared.args.last().unwrap();
        let request = LinuxSandboxRequest::decode(encoded).unwrap();
        let targets: Vec<_> = request.mounts.iter().map(|mount| &mount.target).collect();
        let broad = targets
            .iter()
            .position(|path| path.as_path() == Path::new("/tmp/work/vendor"))
            .unwrap();
        let reopen = targets
            .iter()
            .position(|path| path.as_path() == Path::new("/tmp/work/vendor/ok"))
            .unwrap();
        let narrow = targets
            .iter()
            .position(|path| path.as_path() == Path::new("/tmp/work/vendor/ok/secret"))
            .unwrap();
        assert!(broad < reopen && reopen < narrow);

        let mut policy = plan();
        let same = PathBuf::from("/tmp/work/secret");
        policy.read_only_paths = vec![same.clone()];
        policy.unreadable_paths = vec![same.clone()];
        let prepared = prepare(&probe(), policy, "true", Path::new("/tmp/work")).unwrap();
        let request = LinuxSandboxRequest::decode(prepared.args.last().unwrap()).unwrap();
        let same_target: Vec<_> = request
            .mounts
            .iter()
            .filter(|mount| mount.target == same)
            .map(|mount| mount.kind)
            .collect();
        assert_eq!(same_target, [MountKind::ReadOnly, MountKind::Unreadable]);
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
