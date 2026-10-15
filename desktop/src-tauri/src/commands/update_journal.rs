//! Durable maintenance results. No Agent database or storage migration.
use crate::AppError;
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

fn root() -> PathBuf {
    future_rpc::home::future_home().join("app/updates")
}

pub(super) fn latest() -> Result<Option<Record>, AppError> {
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
    let known_stage = matches!(
        record.stage.as_str(),
        "checking"
            | "downloading"
            | "verified"
            | "cleanup"
            | "awaiting-exit"
            | "installing"
            | "finished"
    );
    let known_outcome = matches!(
        record.outcome.as_deref(),
        None | Some(
            "installed"
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
        Some("installed" | "failed-restored" | "not-installed")
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

struct Inner {
    path: PathBuf,
    record: Record,
    handed_off: bool,
}
impl Inner {
    fn save(&self) -> Result<(), AppError> {
        crate::config_io::write_json_atomic(&self.path, &serde_json::to_value(&self.record)?, true)
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        if self.record.outcome.is_none() && !self.handed_off {
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
impl Operation {
    pub(super) fn begin(version: &str) -> Result<Self, AppError> {
        if latest()?.is_some_and(|record| {
            !record.repair_acknowledged
                && (record.outcome.is_none()
                    || record.outcome.as_deref() == Some("repair-required")
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
        inner.record.stage = stage.into();
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
        inner.record.stage = "finished".into();
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
    fn results_survive_operation_drop_and_record_failure_diagnostics() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-journal-result");
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
        let op = Operation::begin("old").unwrap();
        assert!(Operation::begin("other").is_err());
        drop(op);
        assert_eq!(
            latest().unwrap().unwrap().outcome.as_deref(),
            Some("interrupted")
        );
    }
}
