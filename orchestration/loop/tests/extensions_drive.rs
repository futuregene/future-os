//! Coverage drive for `extensions/`: manifest validation error matrix,
//! protocol tokens, API-version clauses, readiness (doctor) branches, and
//! the runtime lifecycle validation arms.
//!
//! All tests serialize on one lock: readiness resolves commands against
//! PATH, and one test mutates PATH (process-global).

use future_loop::extensions::manifest::{
    load_extension_manifest, require_compatible_future_loop_api, validate_manifest_value,
    validate_protocol_token, ExtensionManifest, EXTENSION_MANIFEST_SCHEMA_VERSION,
};
use future_loop::extensions::readiness::{extension_doctor, resolve_runtime_entrypoint, DoctorStatus};
use future_loop::extensions::runtime::{
    disable_extension, enable_extension, extension_catalog_entries, extension_status,
    install_extension, manifest_revision, rollback_extension,
};

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn manifest_json(overrides: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>)) -> serde_json::Value {
    let raw = serde_json::json!({
        "schema_version": EXTENSION_MANIFEST_SCHEMA_VERSION,
        "id": "ext-x",
        "version": "1.0.0",
        "requires_future_loop_api": ">=1,<3",
        "permissions": ["shell"],
        "runtime": {
            "protocol": "command_json_v0",
            "entrypoint": "sh",
            "args": [],
            "doctor_args": [],
            "required_permissions": ["shell"],
            "timeout_seconds": 30
        },
        "provides": [{"id": "ext_x_cap", "kind": "domain_rule", "visibility": "public"}],
        "implements": [{"capability_id": "ext_x_cap", "protocol": "command_json_v0"}]
    });
    let mut obj = raw.as_object().unwrap().clone();
    overrides(&mut obj);
    serde_json::Value::Object(obj)
}

fn valid_manifest() -> ExtensionManifest {
    validate_manifest_value(&manifest_json(|_| {}), "test").unwrap()
}

// ── manifest validation matrix ─────────────────────────────────────────────

#[test]
fn manifest_top_level_errors() {
    let _g = LOCK.lock().unwrap();
    // Not an object.
    assert!(validate_manifest_value(&serde_json::json!([]), "t").is_err());
    // Bad / legacy schema.
    assert!(validate_manifest_value(&manifest_json(|m| {
        m.insert("schema_version".into(), "nope".into());
    }), "t")
    .is_err());
    validate_manifest_value(&manifest_json(|m| {
        m.insert("schema_version".into(), "loopx_extension_manifest_v0".into());
    }), "t")
    .unwrap();
    // Missing required strings.
    for key in ["schema_version", "id", "version", "requires_future_loop_api"] {
        assert!(validate_manifest_value(&manifest_json(|m| {
            m.remove(key);
        }), "t")
        .is_err(), "{key}");
    }
    // permissions not an array / non-string items.
    assert!(validate_manifest_value(&manifest_json(|m| {
        m.insert("permissions".into(), "shell".into());
    }), "t")
    .is_err());
    assert!(validate_manifest_value(&manifest_json(|m| {
        m.insert("permissions".into(), serde_json::json!(["shell", 7]));
    }), "t")
    .is_err());
    // Empty manifest (no runtime/provides/implements) → error.
    assert!(validate_manifest_value(&serde_json::json!({
        "schema_version": EXTENSION_MANIFEST_SCHEMA_VERSION,
        "id": "e", "version": "1", "requires_future_loop_api": ">=1",
    }), "t")
    .is_err());
}

#[test]
fn manifest_runtime_errors() {
    let _g = LOCK.lock().unwrap();
    // Bad protocol token.
    assert!(validate_manifest_value(&manifest_json(|m| {
        m["runtime"].as_object_mut().unwrap().insert("protocol".into(), "NOPE".into());
    }), "t")
    .is_err());
    // Both entrypoint and python_module.
    assert!(validate_manifest_value(&manifest_json(|m| {
        m["runtime"].as_object_mut().unwrap().insert("python_module".into(), "mod".into());
    }), "t")
    .is_err());
    // Neither entrypoint nor python_module.
    assert!(validate_manifest_value(&manifest_json(|m| {
        m["runtime"].as_object_mut().unwrap().remove("entrypoint");
    }), "t")
    .is_err());
    // python_module-only runtime is valid.
    validate_manifest_value(&manifest_json(|m| {
        let rt = m["runtime"].as_object_mut().unwrap();
        rt.remove("entrypoint");
        rt.insert("python_module".into(), "ext_mod".into());
    }), "t")
    .unwrap();
    // Undeclared required_permissions.
    assert!(validate_manifest_value(&manifest_json(|m| {
        m["runtime"].as_object_mut().unwrap()
            .insert("required_permissions".into(), serde_json::json!(["shell", "net"]));
    }), "t")
    .is_err());
    // timeout out of range.
    for t in [0, 121] {
        assert!(validate_manifest_value(&manifest_json(|m| {
            m["runtime"].as_object_mut().unwrap()
                .insert("timeout_seconds".into(), serde_json::json!(t));
        }), "t")
        .is_err(), "{t}");
    }
}

#[test]
fn manifest_provides_implements_errors() {
    let _g = LOCK.lock().unwrap();
    // provides not an array / item not an object / missing fields.
    assert!(validate_manifest_value(&manifest_json(|m| {
        m.insert("provides".into(), "x".into());
    }), "t")
    .is_err());
    assert!(validate_manifest_value(&manifest_json(|m| {
        m.insert("provides".into(), serde_json::json!(["x"]));
    }), "t")
    .is_err());
    assert!(validate_manifest_value(&manifest_json(|m| {
        m.insert("provides".into(), serde_json::json!([{"kind": "domain_rule"}]));
    }), "t")
    .is_err());
    // implements: not array / item not object / bad protocol / no runtime /
    // protocol mismatch.
    assert!(validate_manifest_value(&manifest_json(|m| {
        m.insert("implements".into(), "x".into());
    }), "t")
    .is_err());
    assert!(validate_manifest_value(&manifest_json(|m| {
        m.insert("implements".into(), serde_json::json!(["x"]));
    }), "t")
    .is_err());
    assert!(validate_manifest_value(&manifest_json(|m| {
        m.insert("implements".into(), serde_json::json!([{"capability_id": "c", "protocol": "BAD"}]));
    }), "t")
    .is_err());
    assert!(validate_manifest_value(&manifest_json(|m| {
        m.remove("runtime");
        m.insert("implements".into(), serde_json::json!([{"capability_id": "c", "protocol": "command_json_v0"}]));
    }), "t")
    .is_err(), "implements without runtime");
    assert!(validate_manifest_value(&manifest_json(|m| {
        m.insert("implements".into(), serde_json::json!([{"capability_id": "c", "protocol": "other_proto_v1"}]));
    }), "t")
    .is_err(), "protocol mismatch");
    // load_extension_manifest: unreadable + invalid JSON.
    assert!(load_extension_manifest(std::path::Path::new("/nonexistent.json")).is_err());
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.json");
    std::fs::write(&bad, "{nope").unwrap();
    assert!(load_extension_manifest(&bad).is_err());
}

#[test]
fn protocol_token_matrix() {
    let _g = LOCK.lock().unwrap();
    assert!(validate_protocol_token("command_json_v0"));
    assert!(validate_protocol_token("a_v1"));
    assert!(!validate_protocol_token(""));
    assert!(!validate_protocol_token("UPPER_v1"));
    assert!(!validate_protocol_token("1starts_digit_v1"));
    assert!(!validate_protocol_token("no_version"));
    assert!(!validate_protocol_token("trailing_v"));
    assert!(!validate_protocol_token("bad_vx"));
    assert!(!validate_protocol_token("dash-not-allowed_v1"));
}

#[test]
fn api_version_clause_matrix() {
    let _g = LOCK.lock().unwrap();
    // LOOPX_EXTENSION_API_VERSION == 1.
    assert!(require_compatible_future_loop_api(">=1").is_ok());
    assert!(require_compatible_future_loop_api(">=1,<3").is_ok());
    assert!(require_compatible_future_loop_api("<=1").is_ok());
    assert!(require_compatible_future_loop_api("==1").is_ok());
    assert!(require_compatible_future_loop_api("1").is_ok(), "bare number means ==");
    assert!(require_compatible_future_loop_api(">0").is_ok());
    assert!(require_compatible_future_loop_api("<2").is_ok());
    assert!(require_compatible_future_loop_api(">=2").is_err());
    assert!(require_compatible_future_loop_api("<1").is_err());
    assert!(require_compatible_future_loop_api("==2").is_err());
    assert!(require_compatible_future_loop_api("").is_err());
    assert!(require_compatible_future_loop_api(">=x").is_err());
}

// ── readiness (doctor) ─────────────────────────────────────────────────────

#[test]
fn readiness_branches() {
    let _g = LOCK.lock().unwrap();
    // Entrypoint resolves on PATH → ready (no doctor args configured).
    let m = valid_manifest();
    let report = extension_doctor(&m);
    // `sh` exists on unix; on Windows it may not — both arms are valid.
    let expected = if cfg!(unix) {
        DoctorStatus::Ready.label()
    } else {
        DoctorStatus::EntrypointMissing.label()
    };
    assert_eq!(report.status, expected);
    // Missing entrypoint → entrypoint_missing (both doctor-args arms).
    let missing = validate_manifest_value(&manifest_json(|m| {
        m["runtime"].as_object_mut().unwrap()
            .insert("entrypoint".into(), "definitely-not-a-real-cmd-xyz".into());
    }), "t")
    .unwrap();
    assert_eq!(extension_doctor(&missing).status, DoctorStatus::EntrypointMissing.label());
    let missing_with_doctor_args = validate_manifest_value(&manifest_json(|m| {
        let rt = m["runtime"].as_object_mut().unwrap();
        rt.insert("entrypoint".into(), "definitely-not-a-real-cmd-xyz".into());
        rt.insert("doctor_args".into(), serde_json::json!(["--check"]));
    }), "t")
    .unwrap();
    assert_eq!(
        extension_doctor(&missing_with_doctor_args).status,
        DoctorStatus::EntrypointMissing.label()
    );
    // resolve_runtime_entrypoint: absolute-path fallback and None arm.
    let abs = validate_manifest_value(&manifest_json(|m| {
        m["runtime"].as_object_mut().unwrap().insert(
            "entrypoint".into(),
            if cfg!(unix) { "/bin/sh" } else { "sh" }.into(),
        );
    }), "t")
    .unwrap();
    let resolved = resolve_runtime_entrypoint(abs.runtime.as_ref().unwrap());
    if cfg!(unix) {
        assert!(resolved.is_some());
    }
    // No runtime at all → None.
    let no_rt = validate_manifest_value(&serde_json::json!({
        "schema_version": EXTENSION_MANIFEST_SCHEMA_VERSION,
        "id": "e", "version": "1", "requires_future_loop_api": ">=1",
        "provides": [{"id": "c", "kind": "k"}],
    }), "t")
    .unwrap();
    assert!(no_rt.runtime.is_none());
    // python_module resolution (needs python3/python on PATH — whatever the
    // host has; both arms acceptable).
    let py = validate_manifest_value(&manifest_json(|m| {
        let rt = m["runtime"].as_object_mut().unwrap();
        rt.remove("entrypoint");
        rt.insert("python_module".into(), "ext_mod".into());
    }), "t")
    .unwrap();
    let _ = resolve_runtime_entrypoint(py.runtime.as_ref().unwrap());
    // PATH removed entirely → which() returns None (process-global: locked).
    let saved = std::env::var_os("PATH");
    std::env::remove_var("PATH");
    assert!(resolve_runtime_entrypoint(abs.runtime.as_ref().unwrap()).is_none());
    if let Some(p) = saved {
        std::env::set_var("PATH", p);
    }
}

// ── runtime lifecycle validation arms ──────────────────────────────────────

#[test]
fn runtime_lifecycle_errors() {
    let _g = LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let state_file = dir.path().join("state.json");
    let m = valid_manifest();
    // upgrade before install → not installed.
    assert!(install_extension(&m, &state_file, "upgrade", false).is_err());
    // bad operation token.
    assert!(install_extension(&m, &state_file, "frobnicate", false).is_err());
    // install (execute) → installed; second install → already installed.
    install_extension(&m, &state_file, "install", true).unwrap();
    assert!(install_extension(&m, &state_file, "install", false).is_err());
    // Same-revision upgrade → already active.
    assert!(install_extension(&m, &state_file, "upgrade", true).is_err());
    // enable/disable/rollback on a missing extension.
    assert!(enable_extension("ghost", &state_file, false).is_err());
    assert!(disable_extension("ghost", &state_file, false).is_err());
    assert!(rollback_extension("ghost", &state_file, false).is_err());
    // rollback with no prior upgrade → no rollback revision.
    assert!(rollback_extension("ext-x", &state_file, false).is_err());
    // Upgrade to a new revision, then rollback works (execute + dry-run).
    let m2 = validate_manifest_value(&manifest_json(|mm| {
        mm.insert("version".into(), "1.1.0".into());
    }), "t")
    .unwrap();
    install_extension(&m2, &state_file, "upgrade", true).unwrap();
    rollback_extension("ext-x", &state_file, false).unwrap();
    rollback_extension("ext-x", &state_file, true).unwrap();
    // Corrupt state file → read_state error.
    std::fs::write(&state_file, "{corrupt").unwrap();
    assert!(extension_status(&state_file, None).is_err());
    // Bad schema token in state.
    std::fs::write(&state_file, "{\"schema_version\":\"nope\",\"extensions\":{}}").unwrap();
    assert!(extension_status(&state_file, None).is_err());
    // status/catalog on a missing state file → empty.
    let missing = dir.path().join("absent.json");
    assert!(extension_status(&missing, None).unwrap().is_empty());
    assert!(extension_catalog_entries(&missing).unwrap().is_empty());
}

#[test]
fn revision_digest_stable() {
    let _g = LOCK.lock().unwrap();
    let m = valid_manifest();
    assert_eq!(manifest_revision(&m), manifest_revision(&m));
    assert_eq!(manifest_revision(&m).len(), 16);
}
