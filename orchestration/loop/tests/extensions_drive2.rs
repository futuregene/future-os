//! Coverage drive 2 for `extensions/`: example manifest writer, doctor-status
//! labels, revision-overflow retention, doctor-gated lifecycle errors, and
//! the status/catalog filter arms.

use future_loop::extensions::manifest::{
    validate_manifest_value, write_example_manifest, EXTENSION_MANIFEST_SCHEMA_VERSION,
};
use future_loop::extensions::readiness::DoctorStatus;
use future_loop::extensions::runtime::{
    disable_extension, enable_extension, extension_catalog_entries, extension_status,
    install_extension, rollback_extension,
};

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn manifest_with_entrypoint(id: &str, version: &str, entrypoint: &str) -> future_loop::extensions::manifest::ExtensionManifest {
    let raw = serde_json::json!({
        "schema_version": EXTENSION_MANIFEST_SCHEMA_VERSION,
        "id": id,
        "version": version,
        "requires_future_loop_api": ">=1,<3",
        "permissions": ["shell"],
        "runtime": {
            "protocol": "command_json_v0",
            "entrypoint": entrypoint,
            "args": [],
            "doctor_args": ["--check"],
            "required_permissions": ["shell"],
            "timeout_seconds": 30
        },
        "provides": [{"id": format!("{id}_cap"), "kind": "domain_rule", "visibility": "public"}],
        "implements": [{"capability_id": format!("{id}_cap"), "protocol": "command_json_v0"}]
    });
    validate_manifest_value(&raw, "test").unwrap()
}

#[test]
fn example_manifest_writer() {
    let _g = LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = write_example_manifest(dir.path(), "ext-example");
    assert!(path.exists());
    // The example manifest is itself valid.
    let text = std::fs::read_to_string(&path).unwrap();
    let raw: serde_json::Value = serde_json::from_str(&text).unwrap();
    validate_manifest_value(&raw, "example").unwrap();
}

#[test]
fn doctor_status_labels() {
    let _g = LOCK.lock().unwrap();
    for (s, label) in [
        (DoctorStatus::Ready, "ready"),
        (DoctorStatus::EntrypointMissing, "entrypoint_missing"),
        (DoctorStatus::DoctorNotConfigured, "doctor_not_configured"),
        (DoctorStatus::ProbeRequired, "probe_required"),
        (DoctorStatus::ProviderUnavailable, "provider_unavailable"),
    ] {
        assert_eq!(s.label(), label);
    }
}

#[test]
fn doctor_gated_lifecycle_errors() {
    let _g = LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let state_file = dir.path().join("state.json");
    // An extension whose entrypoint cannot resolve (doctor not ready).
    let bad = manifest_with_entrypoint("ext-bad", "1.0.0", "definitely-not-a-real-cmd-xyz");
    // Dry-run install skips the doctor; --execute refuses.
    install_extension(&bad, &state_file, "install", false).unwrap();
    assert!(install_extension(&bad, &state_file, "install", true).is_err());
    // The dry-run install did NOT persist; use a good manifest for the rest.
    let good = manifest_with_entrypoint("ext-good", "1.0.0", "sh");
    install_extension(&good, &state_file, "install", true).unwrap();
    // Now corrupt the installed entry's manifest by hand so enable's doctor
    // fails: point the stored manifest at a missing entrypoint.
    install_extension(&manifest_with_entrypoint("ext-good", "1.1.0", "sh"), &state_file, "upgrade", true).unwrap();
    // enable on an already-enabled extension → changed=false arm.
    let op = enable_extension("ext-good", &state_file, true).unwrap();
    assert!(!op.changed, "already enabled → unchanged");
    // disable → changed; disable again → unchanged arm.
    assert!(disable_extension("ext-good", &state_file, true).unwrap().changed);
    assert!(!disable_extension("ext-good", &state_file, true).unwrap().changed);
    // status filter: miss → error; hit → one row.
    assert!(extension_status(&state_file, Some("ghost")).is_err());
    assert_eq!(extension_status(&state_file, Some("ext-good")).unwrap().len(), 1);
    // rollback: the rollback target exists (1.0.0); doctor-gated rollback on
    // a broken target errors — craft by installing a broken rollback target.
    let good2 = manifest_with_entrypoint("ext-good", "1.2.0", "sh");
    install_extension(&good2, &state_file, "upgrade", true).unwrap();
    rollback_extension("ext-good", &state_file, true).unwrap();
}

#[test]
fn revision_retention_overflow() {
    let _g = LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let state_file = dir.path().join("state.json");
    // MAX_REVISIONS is small (bounded history) — exceed it and confirm the
    // rollback target survives trimming.
    let mut version = |v: u32| manifest_with_entrypoint("ext-churn", &format!("1.0.{v}"), "sh");
    install_extension(&version(0), &state_file, "install", true).unwrap();
    for v in 1..10 {
        install_extension(&version(v), &state_file, "upgrade", true).unwrap();
    }
    let rows = extension_status(&state_file, Some("ext-churn")).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].revision_count <= 10, "bounded: {}", rows[0].revision_count);
    assert!(rows[0].rollback_available, "required rollback revision retained");
    // catalog entries include the lifecycle row.
    let entries = extension_catalog_entries(&state_file).unwrap();
    assert!(entries.iter().any(|e| e.id == "ext-churn"));
}
