//! G-21/G-22 extension contract tests: the declarative extension lifecycle
//! closed loop (install → enable → disable → rollback → status) with
//! revision retention, API compatibility checks, doctor readiness, and the
//! capability→extension resolution hook (G-22).

use std::path::PathBuf;

use future_loop::extensions::manifest::{validate_manifest_value, ExtensionManifest};
use future_loop::extensions::readiness::{extension_doctor, DoctorStatus};
use future_loop::extensions::runtime::{
    disable_extension, enable_extension, extension_catalog_entries, extension_status,
    install_extension, manifest_revision, rollback_extension, ExtensionState,
};

fn tmp_state(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "future-loop-p3-ext-contract-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("state.json")
}

fn manifest(id: &str, version: &str, entrypoint: &str) -> ExtensionManifest {
    let raw = serde_json::json!({
        "schema_version": future_loop::extensions::manifest::EXTENSION_MANIFEST_SCHEMA_VERSION,
        "id": id,
        "version": version,
        "requires_future_loop_api": ">=1,<3",
        "permissions": ["shell"],
        "runtime": {
            "protocol": "command_json_v0",
            "entrypoint": entrypoint,
            "args": [],
            "doctor_args": ["-c", "true"],
            "required_permissions": ["shell"],
            "timeout_seconds": 30
        },
        "provides": [{"id": format!("{id}_cap"), "kind": "domain_rule", "visibility": "public"}],
        "implements": [{"capability_id": format!("{id}_cap"), "protocol": "command_json_v0"}]
    });
    validate_manifest_value(&raw, "test").unwrap()
}

/// ── P3 acceptance: extension install/enable/disable/rollback/status loop ─
#[test]
fn extension_lifecycle_closed_loop() {
    let path = tmp_state("loop");
    let m = manifest("ext-a", "1.0.0", "sh");
    // Dry-run does not persist.
    let op = install_extension(&m, &path, "install", false).unwrap();
    assert!(op.dry_run && !op.changed);
    assert!(extension_status(&path, None).unwrap().is_empty());
    // Execute persists + enables.
    let op = install_extension(&m, &path, "install", true).unwrap();
    assert!(!op.dry_run && op.changed);
    assert_eq!(op.enabled, Some(true));
    let rows = extension_status(&path, Some("ext-a")).unwrap();
    assert!(rows[0].enabled && rows[0].doctor_verified);
    // Install is idempotent-fail (duplicate rejected).
    assert!(install_extension(&m, &path, "install", true).is_err());
    // Disable → not doctor-verified; enable → back.
    disable_extension("ext-a", &path, true).unwrap();
    assert!(!extension_status(&path, Some("ext-a")).unwrap()[0].enabled);
    enable_extension("ext-a", &path, true).unwrap();
    assert!(extension_status(&path, Some("ext-a")).unwrap()[0].enabled);
}

/// ── Upgrade keeps a bounded revision history + rollback target ────────────
#[test]
fn upgrade_retains_bounded_revisions_and_rollback() {
    let path = tmp_state("upgrade");
    install_extension(&manifest("ext-b", "1.0.0", "sh"), &path, "install", true).unwrap();
    for v in ["1.0.1", "1.0.2", "1.0.3", "1.0.4", "1.0.5"] {
        install_extension(&manifest("ext-b", v, "sh"), &path, "upgrade", true).unwrap();
    }
    let rows = extension_status(&path, Some("ext-b")).unwrap();
    assert_eq!(rows[0].revision_count, 5, "MAX_REVISIONS retention");
    assert!(rows[0].rollback_available);
    // Rollback swaps active/rollback revisions.
    let active_before = rows[0].active_revision.clone();
    let op = rollback_extension("ext-b", &path, true).unwrap();
    assert!(op.changed);
    assert_ne!(op.revision.as_deref(), Some(active_before.as_str()));
    // Active is now the previous revision; rollback target is the newer one.
    let state: ExtensionState =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let entry = &state.extensions["ext-b"];
    assert_eq!(
        entry.active_revision,
        manifest_revision(&manifest("ext-b", "1.0.4", "sh"))
    );
    assert_eq!(
        entry.rollback_revision.as_deref(),
        Some(manifest_revision(&manifest("ext-b", "1.0.5", "sh")).as_str())
    );
}

/// ── API compatibility fails closed ────────────────────────────────────────
#[test]
fn incompatible_future_loop_api_is_rejected() {
    let mut raw = serde_json::json!({
        "schema_version": future_loop::extensions::manifest::EXTENSION_MANIFEST_SCHEMA_VERSION,
        "id": "ext-bad",
        "version": "1.0.0",
        "requires_future_loop_api": ">=99",
        "permissions": [],
        "provides": [{"id": "c", "kind": "domain_rule", "visibility": "public"}]
    });
    assert!(validate_manifest_value(&raw, "test").is_err());
    raw["requires_future_loop_api"] = serde_json::json!(">=1,<2");
    assert!(validate_manifest_value(&raw, "test").is_ok());
}

/// ── Doctor readiness gates install/enable ─────────────────────────────────
#[test]
fn doctor_readiness_gates_lifecycle() {
    let path = tmp_state("doctor");
    // Missing entrypoint → doctor not ready → install with execute fails.
    let bad = manifest("ext-c", "1.0.0", "/definitely/not/a/real/command-xyz");
    let report = extension_doctor(&bad);
    assert_eq!(report.status, DoctorStatus::EntrypointMissing.label());
    assert!(!report.verified);
    let err = install_extension(&bad, &path, "install", true).unwrap_err();
    assert!(err.contains("doctor is not ready"));
    // Declarative-only extension (no runtime) is ready by declaration.
    let raw = serde_json::json!({
        "schema_version": future_loop::extensions::manifest::EXTENSION_MANIFEST_SCHEMA_VERSION,
        "id": "ext-decl",
        "version": "1.0.0",
        "requires_future_loop_api": ">=1",
        "permissions": [],
        "provides": [{"id": "ext-decl_cap", "kind": "domain_rule", "visibility": "public"}]
    });
    let decl = validate_manifest_value(&raw, "test").unwrap();
    assert!(extension_doctor(&decl).verified);
}

/// ── G-22: capability → extension resolution ──────────────────────────────
#[test]
fn resolve_capability_extension_id_single_ready_implementation() {
    let path = tmp_state("resolve");
    install_extension(&manifest("ext-d", "1.0.0", "sh"), &path, "install", true).unwrap();
    let resolved = future_loop::capabilities::resolver::resolve_capability_extension_id(
        &path,
        "ext-d_cap",
        "command_json_v0",
    )
    .unwrap();
    assert_eq!(resolved, "ext-d");
    // Disabled → not resolvable.
    disable_extension("ext-d", &path, true).unwrap();
    let err = future_loop::capabilities::resolver::resolve_capability_extension_id(
        &path,
        "ext-d_cap",
        "command_json_v0",
    )
    .unwrap_err();
    assert!(err.contains("no enabled, doctor-ready extension"));
}

#[test]
fn resolve_ambiguous_implementations_fails_closed() {
    let path = tmp_state("ambiguous");
    install_extension(&manifest("ext-e1", "1.0.0", "sh"), &path, "install", true).unwrap();
    install_extension(&manifest("ext-e2", "1.0.0", "sh"), &path, "install", true).unwrap();
    // Both extensions implement the SAME capability id → ambiguous resolution.
    let raw1 = serde_json::json!({
        "schema_version": future_loop::extensions::manifest::EXTENSION_MANIFEST_SCHEMA_VERSION,
        "id": "ext-f1",
        "version": "1.0.0",
        "requires_future_loop_api": ">=1",
        "permissions": ["shell"],
        "runtime": {"protocol": "command_json_v0", "entrypoint": "sh", "args": [], "doctor_args": ["-c", "true"], "required_permissions": ["shell"], "timeout_seconds": 30},
        "provides": [{"id": "shared_cap", "kind": "domain_rule", "visibility": "public"}],
        "implements": [{"capability_id": "shared_cap", "protocol": "command_json_v0"}]
    });
    let mut raw2 = raw1.clone();
    raw2["id"] = serde_json::json!("ext-f2");
    let m1 = validate_manifest_value(&raw1, "t").unwrap();
    let m2 = validate_manifest_value(&raw2, "t").unwrap();
    install_extension(&m1, &path, "install", true).unwrap();
    install_extension(&m2, &path, "install", true).unwrap();
    let err = future_loop::capabilities::resolver::resolve_capability_extension_id(
        &path,
        "shared_cap",
        "command_json_v0",
    )
    .unwrap_err();
    assert!(err.contains("multiple enabled, doctor-ready extensions"));
}

#[test]
fn catalog_entries_compose_lifecycle() {
    let path = tmp_state("catalog");
    install_extension(&manifest("ext-g", "1.0.0", "sh"), &path, "install", true).unwrap();
    let entries = extension_catalog_entries(&path).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].lifecycle.ready);
    assert_eq!(entries[0].provides, vec!["ext-g_cap"]);
    assert_eq!(entries[0].implements, vec!["ext-g_cap"]);
    disable_extension("ext-g", &path, true).unwrap();
    let entries = extension_catalog_entries(&path).unwrap();
    assert!(!entries[0].lifecycle.ready);
}
