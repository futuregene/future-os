//! W6 assembly of capability records, handle-audited ACLs, restricted token
//! and the suspended W3 process driver. Product availability remains disabled
//! until the native Windows matrix has passed.

#![allow(dead_code)]

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::acl::{ensure_write_deny, ensure_write_file, ensure_write_root};
use super::audit::FrozenPath;
use super::process::RestrictedChild;
use super::token::{derive_capability_sid, RestrictedToken};
use crate::sandbox::windows_capability::{
    policy_records, request_records, CapabilityRecord, CapabilityState,
};
use crate::sandbox::windows_plan::{build_plan, WindowsSandboxPlan};
use crate::sandbox::windows_request::{ApprovedWriteCapability, WriteScope};
use crate::sandbox::{shell_invocation, ResolvedSandbox};

static PREPARE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

enum CompatibilitySidMode {
    Production,
    #[cfg(test)]
    Diagnostic {
        include_logon: bool,
        include_everyone: bool,
    },
}

pub(crate) fn spawn(
    sandbox: &ResolvedSandbox,
    command: &str,
    cwd: &Path,
    env_overrides: &[(OsString, OsString)],
    approval: Option<&ApprovedWriteCapability>,
) -> io::Result<RestrictedChild> {
    let plan = build_plan(sandbox);
    let state_path = capability_state_path()?;
    spawn_with_plan(&plan, command, cwd, env_overrides, approval, &state_path)
}

fn spawn_with_plan(
    plan: &WindowsSandboxPlan,
    command: &str,
    cwd: &Path,
    env_overrides: &[(OsString, OsString)],
    approval: Option<&ApprovedWriteCapability>,
    state_path: &Path,
) -> io::Result<RestrictedChild> {
    spawn_with_plan_inner(
        plan,
        command,
        cwd,
        env_overrides,
        approval,
        state_path,
        CompatibilitySidMode::Production,
    )
}

fn spawn_with_plan_inner(
    plan: &WindowsSandboxPlan,
    command: &str,
    cwd: &Path,
    env_overrides: &[(OsString, OsString)],
    approval: Option<&ApprovedWriteCapability>,
    state_path: &Path,
    compatibility: CompatibilitySidMode,
) -> io::Result<RestrictedChild> {
    let _guard = PREPARE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| io::Error::other("Windows capability preparation lock is poisoned"))?;

    let records = records_for(plan, approval)?;
    persist_records(&records, state_path)?;
    apply_acl_plan(plan, &records)?;
    let token = match compatibility {
        CompatibilitySidMode::Production => RestrictedToken::from_capabilities(&records)?,
        #[cfg(test)]
        CompatibilitySidMode::Diagnostic {
            include_logon,
            include_everyone,
        } => {
            RestrictedToken::from_capabilities_for_test(&records, include_logon, include_everyone)?
        }
    };
    let (program, args) = shell_invocation(command);
    let args = args.into_iter().map(OsString::from).collect::<Vec<_>>();
    RestrictedChild::spawn(&token, OsStr::new(program), &args, cwd, env_overrides)
}

/// Native integration tests use the exact production assembly while keeping
/// capability metadata and ACL mutations inside their disposable fixture.
#[cfg(test)]
pub(crate) fn spawn_with_plan_for_test(
    plan: &WindowsSandboxPlan,
    command: &str,
    cwd: &Path,
    env_overrides: &[(OsString, OsString)],
    approval: Option<&ApprovedWriteCapability>,
    state_path: &Path,
) -> io::Result<RestrictedChild> {
    spawn_with_plan(plan, command, cwd, env_overrides, approval, state_path)
}

/// Manual Windows-only diagnostic used to determine the minimum compatibility
/// SID set that can initialize PowerShell on real hosts. This is not reachable
/// from the product runner.
#[cfg(test)]
pub(crate) fn spawn_with_compatibility_sids_for_test(
    plan: &WindowsSandboxPlan,
    command: &str,
    cwd: &Path,
    state_path: &Path,
    include_logon: bool,
    include_everyone: bool,
) -> io::Result<RestrictedChild> {
    spawn_with_plan_inner(
        plan,
        command,
        cwd,
        &[],
        None,
        state_path,
        CompatibilitySidMode::Diagnostic {
            include_logon,
            include_everyone,
        },
    )
}

fn records_for(
    plan: &WindowsSandboxPlan,
    approval: Option<&ApprovedWriteCapability>,
) -> io::Result<Vec<CapabilityRecord>> {
    match approval {
        None => Ok(policy_records(plan)),
        Some(approval) => {
            if approval.targets.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "approved Windows capability has no targets",
                ));
            }
            let targets = approval
                .targets
                .iter()
                .map(|target| (PathBuf::from(&target.path), target.scope))
                .collect::<Vec<_>>();
            request_records(plan, &approval.request_id, &targets)
        }
    }
}

fn persist_records(records: &[CapabilityRecord], state_path: &Path) -> io::Result<()> {
    let mut state = match CapabilityState::load(state_path) {
        Ok(state) => state,
        Err(error) if error.kind() == io::ErrorKind::NotFound => CapabilityState::default(),
        Err(error) => return Err(error),
    };
    state.merge(records.iter().cloned())?;
    state.save_atomic(state_path)
}

fn capability_state_path() -> io::Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "user home is unavailable"))?;
    Ok(home.join(".future/windows-capabilities.json"))
}

fn apply_acl_plan(plan: &WindowsSandboxPlan, records: &[CapabilityRecord]) -> io::Result<()> {
    for record in records {
        let sid = derive_capability_sid(&record.name)?;
        let root = FrozenPath::open_local_ntfs(&record.writable_root)?;
        match approved_scope(record, &record.writable_root) {
            Some(WriteScope::File) => ensure_write_file(&root, &sid)?,
            Some(WriteScope::Subtree) | None => ensure_write_root(&root, &sid)?,
        }

        for carveout in &plan.write_carveouts {
            // A DACL cannot attach to a future name. Existing literal objects
            // get hardening; missing objects remain part of the documented
            // workspace-internal limitation rather than being replaced by a
            // broader parent deny.
            let frozen = match FrozenPath::open_local_ntfs(&carveout.path) {
                Ok(frozen) => frozen,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            // Windows deny ACEs always win. An explicit ask carveout therefore
            // cannot be reopened by adding a narrower request SID while any
            // broader writable-root SID remains in the token. Preflight rejects
            // that unsupported request; every carveout reaching assembly stays
            // fail-closed here.
            ensure_write_deny(&frozen, &sid)?;
        }
    }
    Ok(())
}

fn approved_scope(record: &CapabilityRecord, path: &Path) -> Option<WriteScope> {
    record
        .approved_targets
        .iter()
        .find(|target| target.path == path)
        .map(|target| target.scope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::windows_request::ApprovalTarget;

    #[test]
    fn approved_records_keep_exact_targets_and_scope() {
        let root = std::env::current_dir().unwrap().join("runner-workspace");
        let target = std::env::current_dir().unwrap().join("runner-release");
        let plan = WindowsSandboxPlan {
            writable_roots: vec![root.clone()],
            ..WindowsSandboxPlan::default()
        };
        let approval = ApprovedWriteCapability {
            request_id: "approval-runner".to_string(),
            command_hash: "hash".to_string(),
            targets: vec![ApprovalTarget {
                path: target.to_string_lossy().into_owned(),
                scope: WriteScope::File,
            }],
        };
        let records = records_for(&plan, Some(&approval)).unwrap();
        assert!(records.iter().any(|record| record.writable_root == root));
        let exact = records
            .iter()
            .find(|record| record.writable_root == target)
            .unwrap();
        assert_eq!(
            approved_scope(exact, &exact.writable_root),
            Some(WriteScope::File)
        );
    }
}
