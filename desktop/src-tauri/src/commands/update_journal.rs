//! Durable maintenance results. No Agent database or storage migration.
use super::state::Stage;
use crate::AppError;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Record {
    pub format_version: u32,
    pub operation_id: String,
    pub from_version: String,
    pub target_version: Option<String>,
    pub stage: String,
    pub outcome: Option<String>,
    pub diagnostic: Option<String>,
    pub restart_safe: bool,
    #[serde(default)]
    pub repair_acknowledged: bool,
}

// A test case's own journal directory, bound by [`CaseRoot`] on the thread that
// runs the case. The root used to come from the process-global `HOME` that every
// fixture rewrites, so a case's isolation depended on *every other test in this
// binary* leaving that variable alone for the whole of the case. Anything that did
// not — a fixture that restores `HOME` early, a detached task, a journal read
// issued from another module — could put two cases in one directory, and the
// second then reported `Another update transaction is active` about a lock its
// own sibling held. The override is per thread and test-only, so a case owns its
// root whatever the rest of the process does.
#[cfg(test)]
thread_local! {
    static CASE_ROOT: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// The updates directory: `<FutureOS home>/app/updates`, or the calling test
/// case's own directory (see [`CaseRoot`]).
///
/// The single derivation of it: the journal, the recovery fingerprints and the
/// platform adapters all live under this directory, and a test case points all
/// of them at one private directory.
pub(super) fn root() -> PathBuf {
    #[cfg(test)]
    if let Some(root) = CASE_ROOT.with(|root| root.borrow().clone()) {
        return root;
    }
    future_rpc::home::future_home().join("app/updates")
}

/// Binds the calling thread's journal to a private directory for one test case,
/// and unbinds it when the case ends.
///
/// This is the test-visible construction point the journal offers instead of the
/// environment: a case never shares `transaction.lock` with another case, so a
/// red run can only report a transaction *this* case started.
#[cfg(test)]
pub(super) struct CaseRoot {
    directory: tempfile::TempDir,
}

#[cfg(test)]
impl CaseRoot {
    pub(super) fn bind() -> Self {
        let directory = tempfile::Builder::new()
            .prefix("update-journal-case-")
            .tempdir()
            .expect("a private update-journal directory");
        CASE_ROOT.with(|root| *root.borrow_mut() = Some(directory.path().to_path_buf()));
        Self { directory }
    }

    pub(super) fn path(&self) -> &std::path::Path {
        self.directory.path()
    }
}

#[cfg(test)]
impl Drop for CaseRoot {
    fn drop(&mut self) {
        CASE_ROOT.with(|root| *root.borrow_mut() = None);
    }
}

fn read_latest() -> Result<Option<Record>, AppError> {
    let pointer = root().join("latest.json");
    let bytes = match std::fs::read(pointer) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let id: String = serde_json::from_slice(&bytes)?;
    if !id.starts_with("update-") || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(
            "Invalid update operation identifier; inspect app/updates before updating".into(),
        );
    }
    let record: Record =
        serde_json::from_slice(&std::fs::read(root().join(&id).join("result.json"))?)?;
    let known_stage = Stage::parse(&record.stage).is_some();
    let known_outcome = matches!(
        record.outcome.as_deref(),
        None | Some(
            "installed"
                | "pending-validation"
                | "cancelled"
                | "failed"
                | "interrupted"
                | "not-installed"
                | "failed-restored"
                | "repair-required"
                | "cleanup-failed"
        )
    );
    let safe_outcome = matches!(
        record.outcome.as_deref(),
        Some("installed" | "pending-validation" | "failed-restored" | "not-installed")
    );
    if record.format_version != 1
        || record.operation_id != id
        || !known_stage
        || !known_outcome
        || (record.restart_safe && !safe_outcome)
    {
        return Err(
            "Unsupported or inconsistent update record; inspect app/updates before updating".into(),
        );
    }
    Ok(Some(record))
}

/// The process lock survives page unmounts and distinguishes a live operation
/// from a record left behind by a crash (where Drop never runs).
fn transaction_lock() -> Result<std::fs::File, AppError> {
    std::fs::create_dir_all(root())?;
    Ok(std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root().join("transaction.lock"))?)
}

/// Takes the transaction lease, giving a lock that only a just-forked child still
/// holds the moment it needs to exec the chance to disappear first (see
/// [`LEASE_SETTLE_STEP`]). A lease another process really is holding is still
/// refused — after the wait, and with the same message.
fn acquire_transaction_lease() -> Result<std::fs::File, AppError> {
    let lease = transaction_lock()?;
    let mut waited = 0;
    loop {
        match lease.try_lock_exclusive() {
            Ok(()) => return Ok(lease),
            Err(_) if waited < LEASE_SETTLE_ATTEMPTS => {
                waited += 1;
                std::thread::sleep(LEASE_SETTLE_STEP);
            }
            Err(_) => return Err(AppError::from("Another update transaction is active")),
        }
    }
}

pub(super) fn latest() -> Result<Option<Record>, AppError> {
    let lock = transaction_lock()?;
    if lock.try_lock_exclusive().is_err() {
        return read_latest();
    }
    let Some(mut record) = read_latest()? else {
        return Ok(None);
    };
    if record.outcome.is_none() {
        let observer = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root().join(&record.operation_id).join("observer.lock"))
        {
            Ok(file) => file,
            Err(error)
                if error.kind() == std::io::ErrorKind::PermissionDenied
                    || error.raw_os_error() == Some(32) =>
            {
                return Ok(Some(record))
            }
            Err(error) => return Err(error.into()),
        };
        // Windows' FileShare.None and Unix's advisory lease protect handoff.
        if observer.try_lock_exclusive().is_ok() {
            let stage = Stage::parse(&record.stage).ok_or("Unknown update stage")?;
            if stage.program_untouched() {
                record.outcome = Some("not-installed".into());
                record.restart_safe = true;
                record.diagnostic = Some("Update preparation was interrupted; the installed version was not modified. You can retry.".into());
            } else {
                record.outcome = Some("interrupted".into());
                record.restart_safe = false;
                record.diagnostic = Some("Update was interrupted after shutdown began; inspect the retained transaction before restarting.".into());
            }
            save_record(&record)?;
        }
    }
    Ok(Some(record))
}

fn save_record(record: &Record) -> Result<(), AppError> {
    crate::config_io::write_json_atomic(
        &root().join(&record.operation_id).join("result.json"),
        &serde_json::to_value(record)?,
        true,
    )
}

pub(super) fn confirm_startup(version: &str) -> Result<(), AppError> {
    let lock = transaction_lock()?;
    if lock.try_lock_exclusive().is_err() {
        return Ok(());
    }
    if let Some(mut record) = read_latest()? {
        if (record.outcome.as_deref() == Some("pending-validation")
            || (matches!(record.outcome.as_deref(), None | Some("interrupted"))
                && matches!(record.stage.as_str(), "installing" | "awaiting-validation")))
            && record.target_version.as_deref() == Some(version)
        {
            let observer = match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(root().join(&record.operation_id).join("observer.lock"))
            {
                Ok(file) => file,
                Err(error) if error.raw_os_error() == Some(32) => return Ok(()),
                Err(error) => return Err(error.into()),
            };
            if observer.try_lock_exclusive().is_err() {
                return Ok(());
            }
            super::adapters::validate_startup(&record.operation_id)?;
            record.stage = "finished".into();
            record.outcome = Some("installed".into());
            record.diagnostic = None;
            save_record(&record)?;
            if let Err(error) = super::adapters::cleanup_confirmed(&record.operation_id) {
                crate::host_environment::report(&format!(
                    "Update confirmed; old backup retained because cleanup failed: {error}"
                ));
            }
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub(super) fn observer_lease(record: &std::path::Path) -> Result<std::fs::File, AppError> {
    let lease = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(record.with_file_name("observer.lock"))?;
    lease.try_lock_exclusive()?;
    Ok(lease)
}

#[cfg(not(target_os = "windows"))]
pub(super) fn write_worker_result(
    path: &std::path::Path,
    outcome: &str,
    diagnostic: Option<String>,
    safe: bool,
) -> Result<(), AppError> {
    let mut record: Record = serde_json::from_slice(&std::fs::read(path)?)?;
    record.stage = if outcome == "pending-validation" {
        "awaiting-validation"
    } else {
        "finished"
    }
    .into();
    record.outcome = Some(outcome.into());
    record.diagnostic = diagnostic;
    record.restart_safe = safe;
    crate::config_io::write_json_atomic(path, &serde_json::to_value(record)?, true)
}

struct Inner {
    path: PathBuf,
    record: Record,
    handed_off: bool,
    _lease: std::fs::File,
}
impl Inner {
    fn save(&self) -> Result<(), AppError> {
        crate::config_io::write_json_atomic(&self.path, &serde_json::to_value(&self.record)?, true)
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        if self.record.outcome.is_none() && !self.handed_off {
            if std::fs::read(&self.path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Record>(&bytes).ok())
                .is_some_and(|record| record.outcome.is_some())
            {
                return;
            }
            self.record.outcome = Some("interrupted".into());
            self.record.diagnostic = Some("Update operation was interrupted. No successful installation was confirmed; check program files before restarting.".into());
            if let Err(error) = self.save() {
                crate::host_environment::report(&format!(
                    "Cannot persist interrupted update: {error}"
                ));
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct Operation(Arc<Mutex<Inner>>);

/// How long `begin` waits for a lease that is only *transiently* held.
///
/// The lease is a file lock, and this process spawns children constantly — the
/// CLI runner, the installer, terminal shells. A child forked by another thread
/// keeps a copy of every descriptor that is open at that instant, including this
/// lease, until it execs; until then the lock reads as held by nobody in
/// particular. That window was reported as
/// `Another update transaction is active` — measured on CI: a red desktop run
/// whose own case had started no update at all (run 39802801818).
///
/// Waiting before refusing costs a genuinely concurrent installation nothing: it
/// holds the lease for the length of that installation, which is minutes, so the
/// outcome is the same refusal either way. The window being waited out is the
/// fork-to-exec gap, which is microseconds.
const LEASE_SETTLE_STEP: std::time::Duration = std::time::Duration::from_millis(10);
const LEASE_SETTLE_ATTEMPTS: usize = 25;

impl Operation {
    pub(super) fn begin(version: &str) -> Result<Self, AppError> {
        // Recover an abandoned pre-install record before reserving a new one.
        let _ = latest()?;
        let lease = acquire_transaction_lease()?;
        if read_latest()?.is_some_and(|record| {
            !record.repair_acknowledged
                && (record.outcome.is_none()
                    || matches!(
                        record.outcome.as_deref(),
                        Some("repair-required" | "pending-validation")
                    )
                    || (record.outcome.as_deref() == Some("interrupted")
                        && matches!(record.stage.as_str(), "installing" | "awaiting-exit")))
        }) {
            return Err("A previous update has no confirmed result. Inspect app/updates and app/logs/update-recovery.log; finish or repair that installation before updating again.".into());
        }
        std::fs::create_dir_all(root())?;
        let directory = tempfile::Builder::new()
            .prefix("update-")
            .tempdir_in(root())?;
        let id = directory
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let path = directory.keep().join("result.json");
        let inner = Inner {
            path,
            record: Record {
                format_version: 1,
                operation_id: id.clone(),
                from_version: version.into(),
                target_version: None,
                stage: "checking".into(),
                outcome: None,
                diagnostic: None,
                restart_safe: false,
                repair_acknowledged: false,
            },
            handed_off: false,
            _lease: lease,
        };
        inner.save()?;
        crate::config_io::write_json_atomic(
            &root().join("latest.json"),
            &serde_json::to_value(id)?,
            true,
        )?;
        Ok(Self(Arc::new(Mutex::new(inner))))
    }

    pub(super) fn stage(&self, stage: &str, target: Option<&str>) -> Result<(), AppError> {
        let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let next = Stage::parse(stage).ok_or("Unknown update stage")?;
        let previous = Stage::parse(&inner.record.stage).ok_or("Unknown update stage")?;
        if !previous.permits(next) {
            return Err("Invalid update state transition".into());
        }
        inner.record.stage = next.as_str().into();
        if let Some(target) = target {
            inner.record.target_version = Some(target.into());
        }
        inner.save()
    }

    pub(super) fn finish(
        &self,
        outcome: &str,
        diagnostic: Option<String>,
        restart_safe: bool,
    ) -> Result<(), AppError> {
        let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if inner.handed_off || inner.record.outcome.is_some() {
            return Ok(());
        }
        let persisted: Record = serde_json::from_slice(&std::fs::read(&inner.path)?)?;
        if persisted.outcome.is_some() {
            inner.record = persisted;
            return Ok(());
        }
        inner.record.stage = if outcome == "pending-validation" {
            "awaiting-validation"
        } else {
            "finished"
        }
        .into();
        inner.record.outcome = Some(outcome.into());
        inner.record.diagnostic = diagnostic;
        inner.record.restart_safe = restart_safe;
        inner.save()
    }

    pub(super) fn path(&self) -> PathBuf {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .path
            .clone()
    }

    #[cfg(windows)]
    pub(super) fn hand_off(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).handed_off = true;
    }
}

pub(super) fn acknowledge_repair(operation_id: &str) -> Result<(), AppError> {
    let mut record = latest()?.ok_or("There is no update recovery record")?;
    if record.operation_id != operation_id {
        return Err("Update operation changed; refresh before acknowledging repair".into());
    }
    let path = root().join(operation_id).join("result.json");
    // On Windows the observer holds this file with FileShare.None until its
    // entire recovery is finished. Never acknowledge a live observer's result.
    let _lease = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_file_name("observer.lock"))?;
    record.repair_acknowledged = true;
    record.diagnostic = Some(format!("{}; user confirmed manual installation repair; previous outcome was not treated as installation success", record.diagnostic.as_deref().unwrap_or("Unconfirmed update result")));
    record.restart_safe = false;
    crate::config_io::write_json_atomic(&path, &serde_json::to_value(record)?, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abandoned_download_is_retryable_without_manual_repair() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-crash-before-install");
        let _root = CaseRoot::bind();
        let operation = Operation::begin("old").unwrap();
        operation.stage("downloading", Some("new")).unwrap();
        let path = operation.path();
        drop(operation);
        // Model an OS kill: unlike a normal Drop, no outcome was written.
        let mut record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        record["outcome"] = serde_json::Value::Null;
        crate::config_io::write_json_atomic(&path, &record, true).unwrap();
        let recovered = latest().unwrap().unwrap();
        assert_eq!(recovered.outcome.as_deref(), Some("not-installed"));
        assert!(recovered.restart_safe);
        assert!(!recovered.repair_acknowledged);
        assert!(Operation::begin("old").is_ok());
    }

    #[test]
    fn replacing_files_cannot_be_reclassified_as_a_safe_download() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-crash-during-install");
        let _root = CaseRoot::bind();
        let operation = Operation::begin("old").unwrap();
        operation.stage("installing", Some("new")).unwrap();
        assert!(operation.stage("prepared", None).is_err());
        drop(operation);
        assert!(Operation::begin("old").is_err());
    }

    #[test]
    fn old_process_cannot_confirm_the_new_version() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-health-version");
        let _root = CaseRoot::bind();
        let operation = Operation::begin("old").unwrap();
        operation.stage("installing", Some("new")).unwrap();
        operation.finish("pending-validation", None, true).unwrap();
        drop(operation);
        confirm_startup("old").unwrap();
        assert_eq!(
            latest().unwrap().unwrap().outcome.as_deref(),
            Some("pending-validation")
        );
        assert!(Operation::begin("old").is_err());
    }

    #[test]
    fn a_worker_result_survives_a_later_parent_error_and_drop() {
        let _home =
            crate::auth_store::test_support::HomeGuard::new("update-worker-result-authority");
        let _root = CaseRoot::bind();
        let operation = Operation::begin("old").unwrap();
        operation.stage("installing", Some("new")).unwrap();
        let mut record = read_latest().unwrap().unwrap();
        record.stage = "awaiting-validation".into();
        record.outcome = Some("pending-validation".into());
        record.restart_safe = true;
        save_record(&record).unwrap();
        operation
            .finish(
                "repair-required",
                Some("parent lost the response".into()),
                false,
            )
            .unwrap();
        drop(operation);
        let record = latest().unwrap().unwrap();
        assert_eq!(record.outcome.as_deref(), Some("pending-validation"));
        assert!(record.restart_safe);
    }

    #[test]
    fn results_survive_operation_drop_and_record_failure_diagnostics() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-journal-result");
        let _root = CaseRoot::bind();
        let op = Operation::begin("old").unwrap();
        op.stage("installing", Some("new")).unwrap();
        op.finish("repair-required", Some("mixed program files".into()), false)
            .unwrap();
        drop(op);
        let record = latest().unwrap().unwrap();
        assert_eq!(record.target_version.as_deref(), Some("new"));
        assert_eq!(record.outcome.as_deref(), Some("repair-required"));
        assert!(!record.restart_safe);
        assert!(record.diagnostic.unwrap().contains("mixed"));
    }

    #[test]
    fn repair_acknowledgement_preserves_unknown_outcome_and_never_grants_restart() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-journal-repair");
        let _root = CaseRoot::bind();
        let op = Operation::begin("old").unwrap();
        op.stage("installing", Some("new")).unwrap();
        let id = latest().unwrap().unwrap().operation_id;
        drop(op);
        assert!(Operation::begin("new").is_err());
        acknowledge_repair(&id).unwrap();
        let record = latest().unwrap().unwrap();
        assert_eq!(record.outcome.as_deref(), Some("interrupted"));
        assert!(record.repair_acknowledged);
        assert!(!record.restart_safe);
        assert!(Operation::begin("new").is_ok());
    }

    #[test]
    fn corrupt_record_fails_closed_and_terminal_failure_is_not_overwritten() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-journal-corrupt");
        let _root = CaseRoot::bind();
        let op = Operation::begin("old").unwrap();
        op.finish("repair-required", Some("original error".into()), false)
            .unwrap();
        op.finish("installed", None, true).unwrap();
        assert_eq!(
            latest().unwrap().unwrap().outcome.as_deref(),
            Some("repair-required")
        );
        assert!(Operation::begin("new").is_err());
        std::fs::write(op.path(), b"invalid JSON").unwrap();
        assert!(latest().is_err());
        assert!(Operation::begin("new").is_err());
    }

    #[test]
    fn unfinished_operation_blocks_reentry_and_dropped_operation_is_not_success() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-journal-interrupted");
        let _root = CaseRoot::bind();
        let op = Operation::begin("old").unwrap();
        assert!(Operation::begin("other").is_err());
        drop(op);
        assert_eq!(
            latest().unwrap().unwrap().outcome.as_deref(),
            Some("interrupted")
        );
    }

    /// Cases that run at the same time must not share a journal directory.
    ///
    /// This is the regression the module's fixtures were rewritten for: the root
    /// used to come from the process-global `HOME`, so two cases running side by
    /// side resolved one directory and the second one's very first
    /// `Operation::begin` reported `Another update transaction is active` — about
    /// a lease its own sibling held, not about anything the case did. Falsified by
    /// dropping the `CASE_ROOT` lookup from `root()`: every thread then resolves
    /// the one `HOME`-based directory, so `root()` stops matching the case's own
    /// path and the collected paths collapse to one.
    ///
    /// The check is deliberately about the *root*, not about holding a live
    /// transaction here: a lease is a file lock, and a child process spawned by
    /// another test between its `fork` and `exec` inherits it, so holding four of
    /// them while the whole module runs would measure that other hazard instead.
    /// The lock's own protection is covered by
    /// [`Operation::begin`]'s callers below — `replacing_files_cannot_be_reclassified…`
    /// and `unfinished_operation_blocks_reentry…` both keep asserting the refusal.
    #[test]
    fn cases_running_at_the_same_time_never_share_a_journal() {
        let roots = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let cases: Vec<_> = (0..4)
            .map(|_| {
                let roots = roots.clone();
                std::thread::spawn(move || {
                    let case = CaseRoot::bind();
                    assert_eq!(
                        root(),
                        case.path(),
                        "a case resolves its own journal, not the process environment"
                    );
                    roots.lock().unwrap().push(case.path().to_path_buf());
                })
            })
            .collect();
        for case in cases {
            case.join().expect("a case thread must not panic");
        }
        let roots = roots.lock().unwrap().clone();
        let unique: std::collections::HashSet<_> = roots.iter().collect();
        assert_eq!(
            unique.len(),
            roots.len(),
            "cases must not share a journal root: {roots:?}"
        );
    }

    /// `latest()` fails closed on every damaged shape of its own state: an
    /// absent pointer is "no record", a pointer that is not an operation id is
    /// refused, and any other read failure is an error rather than silence.
    #[test]
    fn latest_fails_closed_on_damaged_state() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-journal-latest");
        let _root = CaseRoot::bind();
        let root = root();
        std::fs::create_dir_all(&root).unwrap();
        assert!(latest().unwrap().is_none(), "no pointer means no record");

        std::fs::write(root.join("latest.json"), b"\"../outside\"").unwrap();
        let error = latest().unwrap_err();
        assert!(error
            .to_string()
            .contains("Invalid update operation identifier"));

        std::fs::remove_file(root.join("latest.json")).unwrap();
        std::fs::create_dir(root.join("latest.json")).unwrap();
        assert!(latest().is_err(), "a read error is not an absent record");
    }

    /// A record whose stage or outcome is unknown, or that claims to be
    /// restart-safe on an outcome that is not, is refused. The cleanup-failure
    /// outcome is a known one and must be accepted.
    #[test]
    fn latest_rejects_an_inconsistent_record() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-journal-record");
        let _root = CaseRoot::bind();
        let id = "update-abc";
        let directory = root().join(id);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(root().join("latest.json"), format!("\"{id}\"")).unwrap();
        let record = |outcome: &str, restart_safe: bool| {
            serde_json::json!({
                "formatVersion": 1,
                "operationId": id,
                "fromVersion": "old",
                "targetVersion": null,
                "stage": "finished",
                "outcome": outcome,
                "diagnostic": null,
                "restartSafe": restart_safe,
            })
        };
        let write = |value: &serde_json::Value| {
            std::fs::write(directory.join("result.json"), value.to_string()).unwrap();
        };

        write(&record("cleanup-failed", false));
        assert_eq!(
            latest().unwrap().unwrap().outcome.as_deref(),
            Some("cleanup-failed")
        );

        let mut unknown_stage = record("cleanup-failed", false);
        unknown_stage["stage"] = serde_json::json!("halfway");
        write(&unknown_stage);
        assert!(latest().is_err());

        let mut unsafe_claim = record("repair-required", false);
        unsafe_claim["restartSafe"] = serde_json::json!(true);
        write(&unsafe_claim);
        assert!(latest().is_err());
    }

    /// A repair acknowledgement is bound to the operation it was shown for.
    #[test]
    fn repair_acknowledgement_refuses_a_stale_operation_id() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-journal-stale-ack");
        let _root = CaseRoot::bind();
        let op = Operation::begin("old").unwrap();
        let id = latest().unwrap().unwrap().operation_id;
        drop(op);
        let error = acknowledge_repair(&format!("{id}-other")).unwrap_err();
        assert!(error.to_string().contains("Update operation changed"));
    }

    /// The drop guard's last resort: a record that cannot be persisted when the
    /// operation is abandoned is *reported*, not panicked. `result.json` is a
    /// directory here, so the atomic rename cannot land.
    #[test]
    fn an_unpersistable_interrupted_record_is_reported_without_panicking() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-journal-drop-failure");
        let _root = CaseRoot::bind();
        let op = Operation::begin("old").unwrap();
        let path = op.path();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();

        drop(op);
    }
}
