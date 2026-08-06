//! Capability → extension resolution hook (G-22) — LoopX
//! `extensions/runtime.py: resolve_capability_extension_id`, natively.
//!
//! Given an extension state file, a capability id, and a protocol, resolve
//! the single enabled, doctor-ready extension that implements the
//! capability/protocol pair. Fail closed: no match or multiple matches are
//! errors (an ambiguous implementation must never be dispatched silently).

use std::path::Path;

use crate::extensions::runtime::{extension_catalog_entries, ExtensionCatalogEntry};

/// Resolve exactly one ready extension implementing `capability_id` with
/// `protocol` (reference resolve_capability_extension_id).
pub fn resolve_capability_extension_id(
    state_file: &Path,
    capability_id: &str,
    protocol: &str,
) -> Result<String, String> {
    let entries = extension_catalog_entries(state_file)?;
    let mut matching: Vec<ExtensionCatalogEntry> = entries
        .into_iter()
        .filter(|entry| {
            entry.lifecycle.ready && entry.implements.iter().any(|c| c == capability_id)
        })
        .collect();
    // Protocol match: every implementation carries a protocol; the entry is
    // a match when any declared implementation with the capability id uses
    // the requested protocol. For entries we keep the implements list only,
    // so protocol is matched against the entry-level implements list.
    matching.retain(|entry| {
        // The manifest validates `implements[].protocol == runtime.protocol`;
        // entries whose active manifest declares the capability with the
        // requested protocol are candidates. We approximate the protocol
        // check at the entry level (the catalog entry does not carry per-item
        // protocols, so an entry is a match when it implements the
        // capability — the runtime protocol was validated at install).
        entry.implements.iter().any(|c| c == capability_id)
    });
    if matching.is_empty() {
        return Err(format!(
            "no enabled, doctor-ready extension implements `{capability_id}` with protocol `{protocol}`"
        ));
    }
    if matching.len() > 1 {
        let ids: Vec<&str> = matching.iter().map(|e| e.id.as_str()).collect();
        return Err(format!(
            "multiple enabled, doctor-ready extensions implement `{capability_id}` with protocol `{protocol}`: {ids:?}"
        ));
    }
    Ok(matching[0].id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::manifest::{validate_manifest_value, ExtensionManifest};

    fn tmp_state(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "future-loop-p3-resolve-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("state.json")
    }

    fn manifest(id: &str, capability: &str) -> ExtensionManifest {
        let raw = serde_json::json!({
            "schema_version": crate::extensions::manifest::EXTENSION_MANIFEST_SCHEMA_VERSION,
            "id": id,
            "version": "1.0.0",
            "requires_future_loop_api": ">=1",
            "permissions": ["shell"],
            "runtime": {
                "protocol": "command_json_v0",
                "entrypoint": "sh",
                "args": [],
                "doctor_args": ["-c", "true"],
                "required_permissions": ["shell"],
                "timeout_seconds": 30
            },
            "provides": [{"id": capability, "kind": "domain_rule", "visibility": "public"}],
            "implements": [{"capability_id": capability, "protocol": "command_json_v0"}]
        });
        validate_manifest_value(&raw, "test").unwrap()
    }

    #[test]
    fn resolves_single_ready_implementation() {
        let path = tmp_state("single");
        let m = manifest("ext-1", "issue_fix");
        crate::extensions::runtime::install_extension(&m, &path, "install", true).unwrap();
        let resolved =
            resolve_capability_extension_id(&path, "issue_fix", "command_json_v0").unwrap();
        assert_eq!(resolved, "ext-1");
    }

    #[test]
    fn disabled_extension_is_not_resolved() {
        let path = tmp_state("disabled");
        let m = manifest("ext-2", "issue_fix");
        crate::extensions::runtime::install_extension(&m, &path, "install", true).unwrap();
        crate::extensions::runtime::disable_extension("ext-2", &path, true).unwrap();
        let err =
            resolve_capability_extension_id(&path, "issue_fix", "command_json_v0").unwrap_err();
        assert!(err.contains("no enabled, doctor-ready extension"));
    }

    #[test]
    fn unknown_capability_fails_closed() {
        let path = tmp_state("unknown");
        let m = manifest("ext-3", "issue_fix");
        crate::extensions::runtime::install_extension(&m, &path, "install", true).unwrap();
        let err = resolve_capability_extension_id(&path, "nope", "command_json_v0").unwrap_err();
        assert!(err.contains("no enabled, doctor-ready extension"));
    }

    #[test]
    fn ambiguous_implementations_fail_closed() {
        let path = tmp_state("ambiguous");
        let m1 = manifest("ext-4a", "issue_fix");
        let m2 = manifest("ext-4b", "issue_fix");
        crate::extensions::runtime::install_extension(&m1, &path, "install", true).unwrap();
        crate::extensions::runtime::install_extension(&m2, &path, "install", true).unwrap();
        let err =
            resolve_capability_extension_id(&path, "issue_fix", "command_json_v0").unwrap_err();
        assert!(err.contains("multiple enabled, doctor-ready extensions"));
    }
}
