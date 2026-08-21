//! W6 assembly of capability records, handle-audited ACLs, restricted token
//! and the suspended W3 process driver. Product availability remains disabled
//! until the remaining reset/uninstall and host-probe release work is complete.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use std::fs::File;
use std::os::windows::io::AsRawHandle;
use windows_sys::Win32::Foundation::ERROR_LOCK_VIOLATION;
use windows_sys::Win32::Storage::FileSystem::{
    LockFileEx, UnlockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
};
use windows_sys::Win32::System::IO::OVERLAPPED;

use super::acl::{ensure_write_deny, ensure_write_file, ensure_write_root, revoke_capability};
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
static ACTIVE_CAPABILITIES: OnceLock<Mutex<ActiveCapabilities>> = OnceLock::new();

#[derive(Default)]
struct ActiveCapabilities {
    names: HashMap<String, usize>,
    process_lock: Option<CapabilityFileLock>,
}

struct CapabilityFileLock {
    file: File,
    overlapped: OVERLAPPED,
}

// The lock is attached to a file handle, not to the acquiring thread. Windows
// permits `UnlockFileEx` from another thread as long as the same handle and
// OVERLAPPED byte range are supplied.
unsafe impl Send for CapabilityFileLock {}

impl CapabilityFileLock {
    fn acquire(state_path: &Path) -> io::Result<Self> {
        let parent = state_path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows capability state path has no parent",
            )
        })?;
        std::fs::create_dir_all(parent)?;
        let lock_path = parent.join("windows-capabilities.lock");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        // A non-blocking exclusive byte-range lock is visible across FutureOS
        // processes and is automatically released by Windows if one crashes.
        // This closes the gap where an NSIS maintenance process could revoke
        // ACEs still used by a separately running agent.
        let ok = unsafe {
            LockFileEx(
                file.as_raw_handle(),
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut overlapped,
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    format!("Windows sandbox permissions are in use: {error}"),
                ));
            }
            return Err(error);
        }
        Ok(Self { file, overlapped })
    }
}

impl Drop for CapabilityFileLock {
    fn drop(&mut self) {
        unsafe {
            UnlockFileEx(self.file.as_raw_handle(), 0, 1, 0, &mut self.overlapped);
        }
    }
}

/// Keeps capability generations alive while their restricted process tree can
/// still use inherited ACEs. GC only revokes a SID when no lease references it.
pub(crate) struct CapabilityLease {
    names: Vec<String>,
}

impl CapabilityLease {
    fn acquire(records: &[CapabilityRecord], state_path: &Path) -> io::Result<Self> {
        let mut names = records
            .iter()
            .map(|record| record.name.clone())
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        if names.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows sandbox has no capability records",
            ));
        }
        let mut active = active_capabilities()
            .lock()
            .map_err(|_| io::Error::other("active capability lock is poisoned"))?;
        if active.names.is_empty() {
            debug_assert!(active.process_lock.is_none());
            active.process_lock = Some(CapabilityFileLock::acquire(state_path)?);
        }
        for name in &names {
            *active.names.entry(name.clone()).or_default() += 1;
        }
        Ok(Self { names })
    }
}

impl Drop for CapabilityLease {
    fn drop(&mut self) {
        let Ok(mut active) = active_capabilities().lock() else {
            return;
        };
        for name in &self.names {
            let remove = match active.names.get_mut(name) {
                Some(count) if *count > 1 => {
                    *count -= 1;
                    false
                }
                Some(_) => true,
                None => false,
            };
            if remove {
                active.names.remove(name);
            }
        }
        if active.names.is_empty() {
            active.process_lock.take();
        }
    }
}

fn active_capabilities() -> &'static Mutex<ActiveCapabilities> {
    ACTIVE_CAPABILITIES.get_or_init(|| Mutex::new(ActiveCapabilities::default()))
}

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
    let lease = CapabilityLease::acquire(&records, state_path)?;
    let mut state = persist_records(&records, state_path)?;
    apply_acl_plan(plan, &records)?;
    reconcile_stale_records(&records, &mut state, state_path)?;
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
    let mut child = RestrictedChild::spawn(&token, OsStr::new(program), &args, cwd, env_overrides)?;
    child.attach_capability_lease(lease);
    Ok(child)
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

fn persist_records(records: &[CapabilityRecord], state_path: &Path) -> io::Result<CapabilityState> {
    let mut state = match CapabilityState::load(state_path) {
        Ok(state) => state,
        Err(error) if error.kind() == io::ErrorKind::NotFound => CapabilityState::default(),
        Err(error) => return Err(error),
    };
    state.merge(records.iter().cloned())?;
    state.save_atomic(state_path)?;
    Ok(state)
}

fn reconcile_stale_records(
    records: &[CapabilityRecord],
    state: &mut CapabilityState,
    state_path: &Path,
) -> io::Result<()> {
    let current = records
        .iter()
        .map(|record| record.name.as_str())
        .collect::<HashSet<_>>();
    let active = active_capabilities()
        .lock()
        .map_err(|_| io::Error::other("active capability lock is poisoned"))?
        .names
        .keys()
        .cloned()
        .collect::<HashSet<_>>();
    // A failed revoke leaves both the ACE and metadata intact so a later run
    // can retry. Removing metadata first would turn recoverable cleanup into an
    // undiscoverable persistent ACE.
    let before = state.records.len();
    state.records.retain(|record| {
        current.contains(record.name.as_str())
            || active.contains(&record.name)
            || revoke_record(record).is_err()
    });
    if state.records.len() != before {
        state.save_atomic(state_path)?;
    }
    Ok(())
}

fn revoke_record(record: &CapabilityRecord) -> io::Result<()> {
    let sid = derive_capability_sid(&record.name)?;
    let mut paths = std::iter::once(&record.writable_root)
        .chain(record.write_carveouts.iter())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    for path in paths {
        let target = match FrozenPath::open_local_ntfs(path) {
            Ok(target) => target,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        revoke_capability(&target, &sid)?;
    }
    Ok(())
}

fn capability_state_path() -> io::Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "user home is unavailable"))?;
    Ok(home.join(".future/windows-capabilities.json"))
}

/// Exercise the complete unelevated backend in a disposable NTFS fixture.
/// This intentionally probes more than `CreateRestrictedToken`: a host is only
/// usable when the real shell/private-desktop pipeline can write the granted
/// root while an adjacent user-writable path remains denied.
pub(crate) fn probe_host() -> io::Result<crate::sandbox::WindowsSandboxProbe> {
    let fixture = tempfile::tempdir()?;
    let writable_root = fixture.path().join("allowed");
    std::fs::create_dir(&writable_root)?;
    let allowed_marker = writable_root.join("allowed.txt");
    let denied_marker = fixture.path().join("denied.txt");
    let state_path = fixture.path().join("capabilities.json");
    let plan = WindowsSandboxPlan {
        writable_roots: vec![writable_root.clone()],
        ..WindowsSandboxPlan::default()
    };
    let env = [
        (
            OsString::from("FUTUREOS_SANDBOX_PROBE_ALLOW"),
            allowed_marker.as_os_str().to_owned(),
        ),
        (
            OsString::from("FUTUREOS_SANDBOX_PROBE_DENY"),
            denied_marker.as_os_str().to_owned(),
        ),
    ];
    let command = "$ErrorActionPreference = 'Stop'; \
        Set-Content -LiteralPath $env:FUTUREOS_SANDBOX_PROBE_ALLOW -Value 'ok'; \
        try { Set-Content -LiteralPath $env:FUTUREOS_SANDBOX_PROBE_DENY -Value 'bad'; exit 42 } \
        catch { exit 0 }";
    let child = match spawn_with_plan(&plan, command, &writable_root, &env, None, &state_path) {
        Ok(child) => child,
        Err(error) => {
            let _ = reset_capabilities_at(&state_path);
            return Ok(crate::sandbox::WindowsSandboxProbe::unavailable(
                "backend_initialization_failed",
                error,
            ));
        }
    };
    let exit_code = child.wait_blocking();
    drop(child);
    let cleanup = reset_capabilities_at(&state_path);
    if let Err(error) = cleanup {
        return Err(io::Error::other(format!(
            "Windows sandbox probe cleanup failed: {error}"
        )));
    }
    match exit_code {
        Ok(0) if allowed_marker.is_file() && !denied_marker.exists() => {
            Ok(crate::sandbox::WindowsSandboxProbe::available())
        }
        Ok(42) | Ok(0) if denied_marker.exists() => Ok(
            crate::sandbox::WindowsSandboxProbe::unavailable_without_error("write_boundary_failed"),
        ),
        Ok(_) => Ok(
            crate::sandbox::WindowsSandboxProbe::unavailable_without_error(
                "restricted_shell_failed",
            ),
        ),
        Err(error) => Ok(crate::sandbox::WindowsSandboxProbe::unavailable(
            "restricted_shell_failed",
            error,
        )),
    }
}

/// Remove all persisted FutureOS capability ACEs when no restricted process
/// tree is active. This is the backend primitive for Settings reset/uninstall;
/// callers must surface `WouldBlock` rather than terminating user commands.
pub(crate) fn reset_capabilities() -> io::Result<usize> {
    reset_capabilities_at(&capability_state_path()?)
}

fn reset_capabilities_at(state_path: &Path) -> io::Result<usize> {
    let _guard = PREPARE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| io::Error::other("Windows capability preparation lock is poisoned"))?;
    if !active_capabilities()
        .lock()
        .map_err(|_| io::Error::other("active capability lock is poisoned"))?
        .names
        .is_empty()
    {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Windows sandbox permissions are in use by an active command",
        ));
    }
    let _process_lock = CapabilityFileLock::acquire(state_path)?;
    let mut state = match CapabilityState::load(state_path) {
        Ok(state) => state,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let before = state.records.len();
    let mut first_error = None;
    state.records.retain(|record| match revoke_record(record) {
        Ok(()) => false,
        Err(error) => {
            if first_error.is_none() {
                first_error = Some(error);
            }
            true
        }
    });
    if state.records.is_empty() {
        match std::fs::remove_file(state_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    } else {
        state.save_atomic(state_path)?;
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(before)
}

#[cfg(test)]
pub(crate) fn reset_capabilities_for_test(state_path: &Path) -> io::Result<usize> {
    reset_capabilities_at(state_path)
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
    fn capability_file_lock_blocks_other_process_handles_and_recovers() {
        let fixture = tempfile::tempdir().unwrap();
        let state_path = fixture.path().join("capabilities.json");
        let first = CapabilityFileLock::acquire(&state_path).unwrap();
        let error = match CapabilityFileLock::acquire(&state_path) {
            Ok(_) => panic!("second lock handle unexpectedly acquired the active range"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        drop(first);
        CapabilityFileLock::acquire(&state_path).unwrap();
    }

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
