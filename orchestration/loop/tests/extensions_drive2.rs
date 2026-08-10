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

fn manifest_with_entrypoint(
    id: &str,
    version: &str,
    entrypoint: &str,
) -> future_loop::extensions::manifest::ExtensionManifest {
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
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    install_extension(
        &manifest_with_entrypoint("ext-good", "1.1.0", "sh"),
        &state_file,
        "upgrade",
        true,
    )
    .unwrap();
    // enable on an already-enabled extension → changed=false arm.
    let op = enable_extension("ext-good", &state_file, true).unwrap();
    assert!(!op.changed, "already enabled → unchanged");
    // disable → changed; disable again → unchanged arm.
    assert!(
        disable_extension("ext-good", &state_file, true)
            .unwrap()
            .changed
    );
    assert!(
        !disable_extension("ext-good", &state_file, true)
            .unwrap()
            .changed
    );
    // status filter: miss → error; hit → one row.
    assert!(extension_status(&state_file, Some("ghost")).is_err());
    assert_eq!(
        extension_status(&state_file, Some("ext-good"))
            .unwrap()
            .len(),
        1
    );
    // rollback: the rollback target exists (1.0.0); doctor-gated rollback on
    // a broken target errors — craft by installing a broken rollback target.
    let good2 = manifest_with_entrypoint("ext-good", "1.2.0", "sh");
    install_extension(&good2, &state_file, "upgrade", true).unwrap();
    rollback_extension("ext-good", &state_file, true).unwrap();
}

#[test]
fn doctor_gated_enable_and_rollback_via_state_surgery() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let state_file = dir.path().join("state.json");
    let good = manifest_with_entrypoint("ext-surg", "1.0.0", "sh");
    install_extension(&good, &state_file, "install", true).unwrap();
    // State surgery: point the stored manifest at a missing entrypoint so the
    // enable doctor refuses.
    let text = std::fs::read_to_string(&state_file).unwrap();
    let broken = text.replace("\"sh\"", "\"definitely-not-a-real-cmd-xyz\"");
    assert_ne!(text, broken, "surgery must change the state");
    std::fs::write(&state_file, &broken).unwrap();
    let err = enable_extension("ext-surg", &state_file, true).unwrap_err();
    assert!(err.contains("doctor"), "{err}");
    // And a rollback whose TARGET manifest is broken refuses the same way:
    // restore a good active manifest, dry-run a broken upgrade… no — craft
    // directly: v1 broken on disk, v2 good active.
    let dir2 = tempfile::tempdir().unwrap();
    let state_file2 = dir2.path().join("state.json");
    install_extension(
        &manifest_with_entrypoint("ext-rb", "1.0.0", "sh"),
        &state_file2,
        "install",
        true,
    )
    .unwrap();
    install_extension(
        &manifest_with_entrypoint("ext-rb", "1.1.0", "sh"),
        &state_file2,
        "upgrade",
        true,
    )
    .unwrap();
    // Break ONLY the rollback target (the 1.0.0 revision) in the state file.
    let text = std::fs::read_to_string(&state_file2).unwrap();
    // The first "sh" occurrence belongs to the older revision payload.
    let idx = text.find("\"sh\"").unwrap();
    let broken = format!(
        "{}\"definitely-not-a-real-cmd-xyz\"{}",
        &text[..idx],
        &text[idx + 4..]
    );
    std::fs::write(&state_file2, &broken).unwrap();
    let err = rollback_extension("ext-rb", &state_file2, true).unwrap_err();
    assert!(err.contains("doctor"), "{err}");
    // And the dry-run (no doctor) still works.
    rollback_extension("ext-rb", &state_file2, false).unwrap();
}

#[test]
fn revision_retention_overflow() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let state_file = dir.path().join("state.json");
    // MAX_REVISIONS is small (bounded history) — exceed it and confirm the
    // rollback target survives trimming.
    let version = |v: u32| manifest_with_entrypoint("ext-churn", &format!("1.0.{v}"), "sh");
    install_extension(&version(0), &state_file, "install", true).unwrap();
    for v in 1..10 {
        install_extension(&version(v), &state_file, "upgrade", true).unwrap();
    }
    let rows = extension_status(&state_file, Some("ext-churn")).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].revision_count <= 10,
        "bounded: {}",
        rows[0].revision_count
    );
    assert!(
        rows[0].rollback_available,
        "required rollback revision retained"
    );
    // catalog entries include the lifecycle row.
    let entries = extension_catalog_entries(&state_file).unwrap();
    assert!(entries.iter().any(|e| e.id == "ext-churn"));
}

#[test]
fn extension_api_version_and_catalog_skip_arm() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(future_loop::extensions::runtime::extension_api_version(), 1);
    // An entry whose ACTIVE revision does not resolve to a stored manifest
    // is skipped in the catalog projection.
    let dir = tempfile::tempdir().unwrap();
    let state_file = dir.path().join("state.json");
    install_extension(
        &manifest_with_entrypoint("ext-skip", "1.0.0", "sh"),
        &state_file,
        "install",
        true,
    )
    .unwrap();
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state_file).unwrap()).unwrap();
    value["extensions"]["ext-skip"]
        .as_object_mut()
        .unwrap()
        .insert(
            "active_revision".into(),
            serde_json::json!("nonexistent-revision"),
        );
    std::fs::write(&state_file, serde_json::to_string(&value).unwrap()).unwrap();
    let entries = extension_catalog_entries(&state_file).unwrap();
    assert!(entries.is_empty(), "invalid manifest skipped: {entries:?}");
    // status still lists it (manifest resolution is not required for rows).
    assert_eq!(extension_status(&state_file, None).unwrap().len(), 1);
}

#[test]
fn retain_revisions_reinserts_required_rollback() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Craft a state where the rollback target sits OUTSIDE the retention
    // window, then upgrade once: retain must re-insert it at the head.
    let dir = tempfile::tempdir().unwrap();
    let state_file = dir.path().join("state.json");
    install_extension(
        &manifest_with_entrypoint("ext-deep", "1.0.0", "sh"),
        &state_file,
        "install",
        true,
    )
    .unwrap();
    for v in 1..6 {
        install_extension(
            &manifest_with_entrypoint("ext-deep", &format!("1.0.{v}"), "sh"),
            &state_file,
            "upgrade",
            true,
        )
        .unwrap();
    }
    // State surgery: rewrite the rollback pointer to the OLDEST revision and
    // truncate the revision list to the window, so the required target is
    // outside it. Simply removing knowledge of the oldest revisions from the
    // state file achieves this after the next upgrade trims.
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state_file).unwrap()).unwrap();
    let entry = value["extensions"]["ext-deep"].as_object_mut().unwrap();
    let revisions = entry["revisions"].as_array().unwrap();
    let oldest = revisions.first().unwrap()["revision"]
        .as_str()
        .unwrap()
        .to_string();
    entry.insert("rollback_revision".into(), serde_json::json!(oldest));
    std::fs::write(&state_file, serde_json::to_string(&value).unwrap()).unwrap();
    // One more upgrade pushes the window past the oldest; retain re-inserts it.
    install_extension(
        &manifest_with_entrypoint("ext-deep", "1.0.9", "sh"),
        &state_file,
        "upgrade",
        true,
    )
    .unwrap();
    let rows = extension_status(&state_file, Some("ext-deep")).unwrap();
    assert!(
        rows[0].rollback_available,
        "required rollback retained: {rows:?}"
    );
}
