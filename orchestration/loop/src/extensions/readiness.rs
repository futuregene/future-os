//! Extension readiness (G-21) — LoopX `extensions/readiness.py`, natively.
//!
//! v1 readiness is DECLARATIVE: the doctor resolves the declared runtime
//! entrypoint (a PATH-resolvable command or a `python3 -m <module>` pair) and
//! verifies it exists with a stable identity. It does NOT execute the
//! extension (no probe subprocess in v1 — that is the P4 process runtime).
//! `doctor_args` presence records that a probe is *configured*; the verified
//! flag is granted when the entrypoint identity resolves.

use std::path::PathBuf;

use serde::Serialize;

use super::manifest::ManifestRuntime;

pub const EXTENSION_DOCTOR_SCHEMA_VERSION: &str = "loopx_extension_doctor_v0";

/// LoopX extension_doctor status values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoctorStatus {
    Ready,
    EntrypointMissing,
    DoctorNotConfigured,
    ProbeRequired,
    ProviderUnavailable,
}

impl DoctorStatus {
    pub fn label(&self) -> &'static str {
        match self {
            DoctorStatus::Ready => "ready",
            DoctorStatus::EntrypointMissing => "entrypoint_missing",
            DoctorStatus::DoctorNotConfigured => "doctor_not_configured",
            DoctorStatus::ProbeRequired => "probe_required",
            DoctorStatus::ProviderUnavailable => "provider_unavailable",
        }
    }
}

/// The resolved runtime entrypoint (LoopX ResolvedRuntimeEntrypoint).
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedRuntimeEntrypoint {
    pub argv_prefix: Vec<String>,
    pub identity: String,
}

/// Resolve a command name against PATH (no execution). Returns the absolute
/// path when found.
fn which(command: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(command);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    // Absolute/relative path form.
    let path = std::path::Path::new(command);
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    None
}

/// A stable identity for a resolved entrypoint: absolute path + content
/// digest prefix (LoopX `_file_identity` digest).
fn file_identity(path: &std::path::Path) -> Option<String> {
    let abs = path.canonicalize().ok()?;
    let content = std::fs::read(&abs).ok()?;
    let digest = crate::store::content_digest(&content);
    Some(format!("{}@{}", abs.display(), &digest[..16]))
}

/// Resolve the runtime entrypoint (entrypoint command or python_module).
pub fn resolve_runtime_entrypoint(runtime: &ManifestRuntime) -> Option<ResolvedRuntimeEntrypoint> {
    if let Some(entrypoint) = &runtime.entrypoint {
        let path = which(entrypoint)?;
        let identity = file_identity(&path)?;
        return Some(ResolvedRuntimeEntrypoint {
            argv_prefix: vec![path.to_string_lossy().into_owned()],
            identity,
        });
    }
    if let Some(module) = &runtime.python_module {
        let interpreter = which("python3").or_else(|| which("python"))?;
        let identity = file_identity(&interpreter)?;
        return Some(ResolvedRuntimeEntrypoint {
            argv_prefix: vec![
                interpreter.to_string_lossy().into_owned(),
                "-m".to_string(),
                module.clone(),
            ],
            identity: format!("python_module:{}@{}", module, identity),
        });
    }
    None
}

/// Doctor report for one manifest (LoopX extension_doctor).
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub schema_version: String,
    pub extension_id: String,
    pub version: String,
    pub status: String,
    pub available: bool,
    pub verified: bool,
    pub entrypoint_identity: Option<String>,
    pub failure_kind: Option<String>,
    pub external_writes_performed: bool,
}

/// Run the declarative doctor: resolve the entrypoint; with no doctor_args
/// configured, record `doctor_not_configured`; otherwise `ready` when the
/// identity resolves.
pub fn extension_doctor(manifest: &super::manifest::ExtensionManifest) -> DoctorReport {
    let extension_id = manifest.provider.id.clone();
    let version = manifest.provider.version.clone();
    let Some(runtime) = &manifest.runtime else {
        // Declarative-only extension (no executable runtime): nothing to
        // doctor — ready by declaration.
        return DoctorReport {
            schema_version: EXTENSION_DOCTOR_SCHEMA_VERSION.to_string(),
            extension_id,
            version,
            status: DoctorStatus::Ready.label().to_string(),
            available: true,
            verified: true,
            entrypoint_identity: None,
            failure_kind: None,
            external_writes_performed: false,
        };
    };
    let identity_before = resolve_runtime_entrypoint(runtime);
    let available = identity_before.is_some();
    let doctor_args = runtime.doctor_args.clone();
    if !doctor_args.is_empty() && !available {
        return DoctorReport {
            schema_version: EXTENSION_DOCTOR_SCHEMA_VERSION.to_string(),
            extension_id,
            version,
            status: DoctorStatus::EntrypointMissing.label().to_string(),
            available: false,
            verified: false,
            entrypoint_identity: None,
            failure_kind: Some("entrypoint_missing".to_string()),
            external_writes_performed: false,
        };
    }
    if doctor_args.is_empty() {
        // v1: no probe is executed; an entrypoint without a configured doctor
        // is still verifiable by identity resolution alone.
        let (status, verified) = if available {
            (DoctorStatus::Ready, true)
        } else {
            (DoctorStatus::EntrypointMissing, false)
        };
        return DoctorReport {
            schema_version: EXTENSION_DOCTOR_SCHEMA_VERSION.to_string(),
            extension_id,
            version,
            status: status.label().to_string(),
            available,
            verified,
            entrypoint_identity: identity_before.map(|e| e.identity),
            failure_kind: if available {
                None
            } else {
                Some("entrypoint_missing".to_string())
            },
            external_writes_performed: false,
        };
    }
    // doctor_args configured but no probe execution in v1 — the identity
    // resolution is the readiness check.
    DoctorReport {
        schema_version: EXTENSION_DOCTOR_SCHEMA_VERSION.to_string(),
        extension_id,
        version,
        status: if available {
            DoctorStatus::Ready.label().to_string()
        } else {
            DoctorStatus::EntrypointMissing.label().to_string()
        },
        available,
        verified: available,
        entrypoint_identity: identity_before.map(|e| e.identity),
        failure_kind: if available {
            None
        } else {
            Some("entrypoint_missing".to_string())
        },
        external_writes_performed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::manifest::{validate_manifest_value, ExtensionManifest};

    fn manifest_with(
        entrypoint: Option<&str>,
        module: Option<&str>,
        doctor_args: Vec<&str>,
    ) -> ExtensionManifest {
        let mut runtime = serde_json::json!({
            "protocol": "command_json_v0",
            "args": [],
            "doctor_args": doctor_args,
            "required_permissions": ["shell"],
            "timeout_seconds": 30
        });
        if let Some(ep) = entrypoint {
            runtime["entrypoint"] = serde_json::json!(ep);
        }
        if let Some(m) = module {
            runtime["python_module"] = serde_json::json!(m);
        }
        let raw = serde_json::json!({
            "schema_version": crate::extensions::manifest::EXTENSION_MANIFEST_SCHEMA_VERSION,
            "id": "ext-readiness",
            "version": "1.0.0",
            "requires_loopx_api": ">=1",
            "permissions": ["shell"],
            "runtime": runtime,
            "provides": [{"id": "ext-readiness_cap", "kind": "domain_rule", "visibility": "public"}],
            "implements": [{"capability_id": "ext-readiness_cap", "protocol": "command_json_v0"}]
        });
        validate_manifest_value(&raw, "test").unwrap()
    }

    #[test]
    fn missing_entrypoint_is_not_ready() {
        let m = manifest_with(
            Some("/definitely/not/a/real/command-xyz"),
            None,
            vec!["--probe"],
        );
        let report = extension_doctor(&m);
        assert_eq!(report.status, DoctorStatus::EntrypointMissing.label());
        assert!(!report.verified);
        assert!(!report.available);
    }

    #[test]
    fn shell_entrypoint_is_ready() {
        let m = manifest_with(Some("sh"), None, vec!["-c", "true"]);
        let report = extension_doctor(&m);
        assert_eq!(report.status, DoctorStatus::Ready.label());
        assert!(report.verified);
        assert!(report.entrypoint_identity.is_some());
    }

    #[test]
    fn declarative_only_extension_is_ready_by_declaration() {
        let raw = serde_json::json!({
            "schema_version": crate::extensions::manifest::EXTENSION_MANIFEST_SCHEMA_VERSION,
            "id": "ext-decl",
            "version": "1.0.0",
            "requires_loopx_api": ">=1",
            "permissions": [],
            "provides": [{"id": "ext-decl_cap", "kind": "domain_rule", "visibility": "public"}]
        });
        let m = validate_manifest_value(&raw, "test").unwrap();
        let report = extension_doctor(&m);
        assert_eq!(report.status, DoctorStatus::Ready.label());
        assert!(report.verified);
    }

    #[test]
    fn python_module_resolves_interpreter() {
        let m = manifest_with(None, Some("json"), vec![]);
        let report = extension_doctor(&m);
        assert!(report.verified, "python3 -m json should resolve");
        assert!(report
            .entrypoint_identity
            .unwrap()
            .starts_with("python_module:"));
    }
}
