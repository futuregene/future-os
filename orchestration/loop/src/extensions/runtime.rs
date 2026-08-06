//! Extension runtime (G-21) — LoopX `extensions/runtime.py`, natively.
//!
//! Lifecycle: `install → enable → (doctor-verified) → ready`, with
//! `disable`, `rollback`, and `status`. State persists in a JSON state file
//! (`extensions/state.json` under the runtime root) with the LoopX
//! `loopx_extension_state_v0` schema. Every install/upgrade keeps a bounded
//! revision history (`MAX_REVISIONS = 5`) so rollback always has a target.
//!
//! v1 is declarative: install validates the manifest + doctor readiness and
//! records the revision — it never executes extension code. The entrypoint
//! exists check lives in [`crate::extensions::readiness`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::manifest::{ExtensionManifest, LOOPX_EXTENSION_API_VERSION};
use super::readiness::{extension_doctor, DoctorStatus};

pub const EXTENSION_STATE_SCHEMA_VERSION: &str = "loopx_extension_state_v0";
pub const EXTENSION_OPERATION_SCHEMA_VERSION: &str = "loopx_extension_operation_v0";
/// LoopX MAX_REVISIONS.
pub const MAX_REVISIONS: usize = 5;

/// Default extension state file under a runtime root.
pub fn default_extension_state_file(runtime_root: &str) -> PathBuf {
    Path::new(runtime_root)
        .join("extensions")
        .join("state.json")
}

/// One retained revision of an extension (manifest snapshot).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionRevision {
    pub revision: String,
    pub version: String,
    pub manifest: ExtensionManifest,
}

/// One installed extension entry (LoopX runtime state entry).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionEntry {
    pub id: String,
    pub enabled: bool,
    pub active_revision: String,
    pub rollback_revision: Option<String>,
    pub doctor_verified_revision: Option<String>,
    pub revisions: Vec<ExtensionRevision>,
}

impl ExtensionEntry {
    pub fn manifest(&self) -> Option<&ExtensionManifest> {
        self.revisions
            .iter()
            .find(|r| r.revision == self.active_revision)
            .map(|r| &r.manifest)
    }

    pub fn rollback_available(&self) -> bool {
        self.rollback_revision.is_some()
    }

    /// Ready = enabled + doctor-verified active revision (LoopX
    /// `_verified_entrypoint`).
    pub fn ready(&self) -> bool {
        self.enabled
            && self.doctor_verified_revision.as_deref() == Some(self.active_revision.as_str())
    }
}

/// The persisted extension runtime state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionState {
    pub schema_version: String,
    pub extensions: BTreeMap<String, ExtensionEntry>,
}

impl ExtensionState {
    fn empty() -> Self {
        Self {
            schema_version: EXTENSION_STATE_SCHEMA_VERSION.to_string(),
            extensions: BTreeMap::new(),
        }
    }
}

/// The operation result (LoopX runtime operation return shape).
#[derive(Debug, Clone, Serialize)]
pub struct ExtensionOperation {
    pub ok: bool,
    pub schema_version: String,
    pub operation: String,
    pub dry_run: bool,
    pub changed: bool,
    pub extension_id: String,
    pub version: Option<String>,
    pub revision: Option<String>,
    pub previous_revision: Option<String>,
    pub enabled: Option<bool>,
    pub rollback_available: Option<bool>,
    pub doctor: Option<serde_json::Value>,
    pub error: Option<String>,
}

fn read_state(path: &Path) -> Result<ExtensionState, String> {
    if !path.exists() {
        return Ok(ExtensionState::empty());
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("extension runtime state is unreadable: {e}"))?;
    let state: ExtensionState = serde_json::from_str(&text)
        .map_err(|e| format!("extension runtime state is unreadable: {e}"))?;
    if state.schema_version != EXTENSION_STATE_SCHEMA_VERSION {
        return Err(format!(
            "extension runtime state must use {EXTENSION_STATE_SCHEMA_VERSION}"
        ));
    }
    Ok(state)
}

fn write_state(path: &Path, state: &ExtensionState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, format!("{json}\n")).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Revision = first 16 hex chars of the SHA-256 of the canonical manifest
/// (LoopX `_revision`).
pub fn manifest_revision(manifest: &ExtensionManifest) -> String {
    let serialized = serde_json::to_string(manifest).unwrap_or_default();
    let digest = crate::store::content_digest(serialized.as_bytes());
    digest.chars().take(16).collect()
}

/// Retain a bounded revision history, always keeping the required rollback
/// revision when it would otherwise fall off (LoopX `_retain_revisions`).
fn retain_revisions(
    revisions: Vec<ExtensionRevision>,
    new_revision: ExtensionRevision,
    required_revision: Option<&str>,
) -> Vec<ExtensionRevision> {
    let mut deduped: Vec<ExtensionRevision> = revisions
        .into_iter()
        .filter(|r| r.revision != new_revision.revision)
        .collect();
    let required_idx =
        required_revision.and_then(|req| deduped.iter().position(|r| r.revision == req));
    deduped.push(new_revision);
    if deduped.len() > MAX_REVISIONS {
        let mut retained: Vec<ExtensionRevision> =
            deduped[deduped.len() - MAX_REVISIONS..].to_vec();
        if let Some(idx) = required_idx {
            let required = deduped[idx].clone();
            if !retained.iter().any(|r| r.revision == required.revision) {
                retained.insert(0, required);
                retained.truncate(MAX_REVISIONS);
            }
        }
        retained
    } else {
        deduped
    }
}

/// `loopx extension install|upgrade --manifest PATH [--execute]`.
/// Dry-run validates + reports; `execute: true` persists.
pub fn install_extension(
    manifest: &ExtensionManifest,
    state_file: &Path,
    operation: &str,
    execute: bool,
) -> Result<ExtensionOperation, String> {
    if operation != "install" && operation != "upgrade" {
        return Err("extension operation must be install or upgrade".into());
    }
    let extension_id = manifest.provider.id.clone();
    let revision = manifest_revision(manifest);
    let doctor = extension_doctor(manifest);
    if execute && doctor.status != DoctorStatus::Ready.label() {
        return Err(format!(
            "extension `{extension_id}` doctor is not ready: {}",
            doctor.status
        ));
    }
    let mut state = read_state(state_file)?;
    let existing = state.extensions.get(&extension_id).cloned();
    if operation == "install" && existing.is_some() {
        return Err(format!("extension `{extension_id}` is already installed"));
    }
    if operation == "upgrade" && existing.is_none() {
        return Err(format!("extension `{extension_id}` is not installed"));
    }
    if existing
        .as_ref()
        .map(|e| e.active_revision == revision)
        .unwrap_or(false)
    {
        return Err(format!(
            "extension `{extension_id}` revision is already active"
        ));
    }
    let previous_revision = existing.as_ref().map(|e| e.active_revision.clone());

    let mut changed = false;
    if execute {
        let revisions = retain_revisions(
            existing
                .as_ref()
                .map(|e| e.revisions.clone())
                .unwrap_or_default(),
            ExtensionRevision {
                revision: revision.clone(),
                version: manifest.provider.version.clone(),
                manifest: manifest.clone(),
            },
            previous_revision.as_deref(),
        );
        state.extensions.insert(
            extension_id.clone(),
            ExtensionEntry {
                id: extension_id.clone(),
                enabled: true,
                active_revision: revision.clone(),
                rollback_revision: previous_revision.clone(),
                doctor_verified_revision: Some(revision.clone()),
                revisions,
            },
        );
        write_state(state_file, &state)?;
        changed = true;
    }
    Ok(ExtensionOperation {
        ok: true,
        schema_version: EXTENSION_OPERATION_SCHEMA_VERSION.to_string(),
        operation: operation.to_string(),
        dry_run: !execute,
        changed,
        extension_id,
        version: Some(manifest.provider.version.clone()),
        revision: Some(revision),
        previous_revision,
        enabled: Some(true),
        rollback_available: Some(false),
        doctor: Some(serde_json::to_value(&doctor).unwrap_or_default()),
        error: None,
    })
}

/// `loopx extension enable --id X [--execute]`.
pub fn enable_extension(
    extension_id: &str,
    state_file: &Path,
    execute: bool,
) -> Result<ExtensionOperation, String> {
    let state = read_state(state_file)?;
    let (active_revision, already_enabled, manifest) = {
        let entry = state
            .extensions
            .get(extension_id)
            .ok_or_else(|| format!("extension `{extension_id}` is not installed"))?;
        let active_revision = entry.active_revision.clone();
        let manifest = entry
            .manifest()
            .cloned()
            .ok_or_else(|| "extension active manifest is invalid".to_string())?;
        (active_revision, entry.enabled, manifest)
    };
    let doctor = extension_doctor(&manifest);
    if execute && doctor.status != DoctorStatus::Ready.label() {
        return Err(format!(
            "extension `{extension_id}` enable doctor is not ready: {}",
            doctor.status
        ));
    }
    let mut changed = false;
    if execute {
        let mut state = read_state(state_file)?;
        let entry = state
            .extensions
            .get_mut(extension_id)
            .ok_or_else(|| format!("extension `{extension_id}` is not installed"))?;
        entry.enabled = true;
        entry.doctor_verified_revision = Some(active_revision.clone());
        changed = !already_enabled;
        write_state(state_file, &state)?;
    }
    Ok(ExtensionOperation {
        ok: true,
        schema_version: EXTENSION_OPERATION_SCHEMA_VERSION.to_string(),
        operation: "enable".to_string(),
        dry_run: !execute,
        changed,
        extension_id: extension_id.to_string(),
        version: Some(manifest.provider.version.clone()),
        revision: Some(active_revision),
        previous_revision: None,
        enabled: Some(already_enabled || execute),
        rollback_available: None,
        doctor: Some(serde_json::to_value(&doctor).unwrap_or_default()),
        error: None,
    })
}

/// `loopx extension disable --id X [--execute]`.
pub fn disable_extension(
    extension_id: &str,
    state_file: &Path,
    execute: bool,
) -> Result<ExtensionOperation, String> {
    let state = read_state(state_file)?;
    let (was_enabled, active_revision, rollback_available) = {
        let entry = state
            .extensions
            .get(extension_id)
            .ok_or_else(|| format!("extension `{extension_id}` is not installed"))?;
        (
            entry.enabled,
            entry.active_revision.clone(),
            entry.rollback_available(),
        )
    };
    let mut changed = false;
    if execute && was_enabled {
        let mut state = read_state(state_file)?;
        let entry = state
            .extensions
            .get_mut(extension_id)
            .ok_or_else(|| format!("extension `{extension_id}` is not installed"))?;
        entry.enabled = false;
        entry.doctor_verified_revision = None;
        changed = true;
        write_state(state_file, &state)?;
    }
    Ok(ExtensionOperation {
        ok: true,
        schema_version: EXTENSION_OPERATION_SCHEMA_VERSION.to_string(),
        operation: "disable".to_string(),
        dry_run: !execute,
        changed,
        extension_id: extension_id.to_string(),
        version: None,
        revision: Some(active_revision),
        previous_revision: None,
        enabled: Some(false),
        rollback_available: Some(rollback_available),
        doctor: None,
        error: None,
    })
}

/// `loopx extension rollback --id X [--execute]` — swap active/rollback
/// revisions (LoopX rollback_extension).
pub fn rollback_extension(
    extension_id: &str,
    state_file: &Path,
    execute: bool,
) -> Result<ExtensionOperation, String> {
    let mut state = read_state(state_file)?;
    let entry = state
        .extensions
        .get_mut(extension_id)
        .ok_or_else(|| format!("extension `{extension_id}` is not installed"))?;
    let target_revision = entry
        .rollback_revision
        .clone()
        .ok_or_else(|| format!("extension `{extension_id}` has no rollback revision"))?;
    let target = entry
        .revisions
        .iter()
        .find(|r| r.revision == target_revision)
        .cloned()
        .ok_or_else(|| "extension rollback manifest is invalid".to_string())?;
    let doctor = extension_doctor(&target.manifest);
    if execute && doctor.status != DoctorStatus::Ready.label() {
        return Err(format!(
            "extension `{extension_id}` rollback doctor is not ready: {}",
            doctor.status
        ));
    }
    let previous_revision = entry.active_revision.clone();
    let mut changed = false;
    if execute {
        entry.active_revision = target_revision.clone();
        entry.rollback_revision = Some(previous_revision.clone());
        entry.doctor_verified_revision = Some(target_revision.clone());
        entry.enabled = true;
        changed = true;
        write_state(state_file, &state)?;
    }
    Ok(ExtensionOperation {
        ok: true,
        schema_version: EXTENSION_OPERATION_SCHEMA_VERSION.to_string(),
        operation: "rollback".to_string(),
        dry_run: !execute,
        changed,
        extension_id: extension_id.to_string(),
        version: Some(target.version),
        revision: Some(target_revision),
        previous_revision: Some(previous_revision),
        enabled: Some(true),
        rollback_available: Some(true),
        doctor: Some(serde_json::to_value(&doctor).unwrap_or_default()),
        error: None,
    })
}

/// `loopx extension status [--id X]` — compact runtime state rows (LoopX
/// extension_status).
#[derive(Debug, Clone, Serialize)]
pub struct ExtensionStatusRow {
    pub id: String,
    pub enabled: bool,
    pub active_revision: String,
    pub rollback_available: bool,
    pub doctor_verified: bool,
    pub revision_count: usize,
}

pub fn extension_status(
    state_file: &Path,
    extension_id: Option<&str>,
) -> Result<Vec<ExtensionStatusRow>, String> {
    let state = read_state(state_file)?;
    let mut rows = vec![];
    for (id, entry) in &state.extensions {
        if let Some(wanted) = extension_id {
            if id != wanted {
                continue;
            }
        }
        rows.push(ExtensionStatusRow {
            id: id.clone(),
            enabled: entry.enabled,
            active_revision: entry.active_revision.clone(),
            rollback_available: entry.rollback_available(),
            doctor_verified: entry.ready(),
            revision_count: entry.revisions.len(),
        });
    }
    if let Some(wanted) = extension_id {
        if rows.is_empty() {
            return Err(format!("extension `{wanted}` is not installed"));
        }
    }
    Ok(rows)
}

/// Compose declared manifests with installed runtime lifecycle state
/// (LoopX extension_catalog_entries).
#[derive(Debug, Clone, Serialize)]
pub struct ExtensionCatalogEntry {
    pub id: String,
    pub version: String,
    pub origin: String,
    pub lifecycle: crate::capabilities::lifecycle::ProviderLifecycle,
    pub active_revision: Option<String>,
    pub provides: Vec<String>,
    pub implements: Vec<String>,
}

/// Queryable catalog of ready extensions implementing a capability/protocol
/// pair (G-22 resolution input).
pub fn extension_catalog_entries(state_file: &Path) -> Result<Vec<ExtensionCatalogEntry>, String> {
    let state = read_state(state_file)?;
    let mut entries = vec![];
    for (id, entry) in &state.extensions {
        let Some(manifest) = entry.manifest() else {
            continue;
        };
        let lifecycle = crate::capabilities::lifecycle::ProviderLifecycle::new(
            true,
            true,
            entry.enabled,
            entry.ready(),
        )
        .unwrap_or(crate::capabilities::lifecycle::ProviderLifecycle {
            declared: true,
            installed: true,
            enabled: entry.enabled,
            ready: false,
        });
        entries.push(ExtensionCatalogEntry {
            id: id.clone(),
            version: manifest.provider.version.clone(),
            origin: "extension".to_string(),
            lifecycle,
            active_revision: Some(entry.active_revision.clone()),
            provides: manifest.capabilities.iter().map(|c| c.id.clone()).collect(),
            implements: manifest
                .implementations
                .iter()
                .map(|i| i.capability_id.clone())
                .collect(),
        });
    }
    Ok(entries)
}

/// The extension API version this runtime provides (help/CLI surface).
pub fn extension_api_version() -> u32 {
    LOOPX_EXTENSION_API_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::manifest::validate_manifest_value;

    fn tmp_file(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "loopx-p3-ext-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("state.json")
    }

    fn manifest(id: &str, version: &str) -> ExtensionManifest {
        let raw = serde_json::json!({
            "schema_version": crate::extensions::manifest::EXTENSION_MANIFEST_SCHEMA_VERSION,
            "id": id,
            "version": version,
            "requires_loopx_api": ">=1",
            "permissions": ["shell"],
            "runtime": {
                "protocol": "command_json_v0",
                "entrypoint": "echo",
                "args": [],
                "doctor_args": [],
                "required_permissions": ["shell"],
                "timeout_seconds": 30
            },
            "provides": [{"id": format!("{id}_cap"), "kind": "domain_rule", "visibility": "public"}],
            "implements": [{"capability_id": format!("{id}_cap"), "protocol": "command_json_v0"}]
        });
        validate_manifest_value(&raw, "test").unwrap()
    }

    #[test]
    fn install_enable_status_loop_closes() {
        let path = tmp_file("loop");
        let m = manifest("ext-a", "1.0.0");
        let op = install_extension(&m, &path, "install", true).unwrap();
        assert!(op.ok && op.changed && !op.dry_run);
        assert_eq!(op.extension_id, "ext-a");

        let rows = extension_status(&path, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].enabled && rows[0].doctor_verified);
        assert!(!rows[0].rollback_available);

        // disable
        let op = disable_extension("ext-a", &path, true).unwrap();
        assert!(op.changed);
        let rows = extension_status(&path, None).unwrap();
        assert!(!rows[0].enabled);
        // enable
        let op = enable_extension("ext-a", &path, true).unwrap();
        assert!(op.changed);
        let rows = extension_status(&path, None).unwrap();
        assert!(rows[0].enabled);
    }

    #[test]
    fn duplicate_install_fails_closed() {
        let path = tmp_file("dup");
        let m = manifest("ext-b", "1.0.0");
        install_extension(&m, &path, "install", true).unwrap();
        let err = install_extension(&m, &path, "install", true).unwrap_err();
        assert!(err.contains("already installed"));
    }

    #[test]
    fn upgrade_keeps_rollback_revision_bounded() {
        let path = tmp_file("upg");
        let m1 = manifest("ext-c", "1.0.0");
        install_extension(&m1, &path, "install", true).unwrap();
        for v in ["1.0.1", "1.0.2", "1.0.3", "1.0.4", "1.0.5"] {
            let m = manifest("ext-c", v);
            install_extension(&m, &path, "upgrade", true).unwrap();
        }
        let rows = extension_status(&path, Some("ext-c")).unwrap();
        assert!(rows[0].rollback_available);
        // MAX_REVISIONS retention: 6 revisions total → 5 retained
        assert_eq!(rows[0].revision_count, MAX_REVISIONS);
        // active is the newest
        let entry = read_state(&path).unwrap().extensions["ext-c"].clone();
        assert_eq!(
            entry.active_revision,
            manifest_revision(&manifest("ext-c", "1.0.5"))
        );
        // rollback target is 1.0.4
        assert_eq!(
            entry.rollback_revision.as_deref(),
            Some(manifest_revision(&manifest("ext-c", "1.0.4")).as_str())
        );
    }

    #[test]
    fn rollback_swaps_active_and_rollback() {
        let path = tmp_file("rb");
        let m1 = manifest("ext-d", "1.0.0");
        install_extension(&m1, &path, "install", true).unwrap();
        let m2 = manifest("ext-d", "1.0.1");
        install_extension(&m2, &path, "upgrade", true).unwrap();
        let op = rollback_extension("ext-d", &path, true).unwrap();
        assert!(op.changed);
        assert_eq!(
            op.revision.as_deref(),
            Some(manifest_revision(&m1).as_str())
        );
        assert_eq!(
            op.previous_revision.as_deref(),
            Some(manifest_revision(&m2).as_str())
        );
        // After rollback, rollback target is the 1.0.1 revision again.
        let entry = read_state(&path).unwrap().extensions["ext-d"].clone();
        assert_eq!(entry.active_revision, manifest_revision(&m1));
        assert_eq!(
            entry.rollback_revision.as_deref(),
            Some(manifest_revision(&m2).as_str())
        );
    }

    #[test]
    fn rollback_without_target_fails() {
        let path = tmp_file("norb");
        let m = manifest("ext-e", "1.0.0");
        install_extension(&m, &path, "install", true).unwrap();
        let err = rollback_extension("ext-e", &path, true).unwrap_err();
        assert!(err.contains("no rollback revision"));
    }

    #[test]
    fn dry_run_does_not_persist() {
        let path = tmp_file("dry");
        let m = manifest("ext-f", "1.0.0");
        let op = install_extension(&m, &path, "install", false).unwrap();
        assert!(op.dry_run && !op.changed);
        assert!(extension_status(&path, None).unwrap().is_empty());
    }

    #[test]
    fn catalog_entries_reflect_lifecycle() {
        let path = tmp_file("cat");
        let m = manifest("ext-g", "1.0.0");
        install_extension(&m, &path, "install", true).unwrap();
        disable_extension("ext-g", &path, true).unwrap();
        let entries = extension_catalog_entries(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].lifecycle.ready);
        assert_eq!(entries[0].provides, vec!["ext-g_cap"]);
    }
}
