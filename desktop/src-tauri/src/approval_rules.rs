//! Workspace approval-rule file writes (desktop/DEV_MD/SANDBOX/COMMON.md).
//!
//! "Allow in this workspace" appends an `allow` rule to
//! `${WORKSPACE}/.future/approval_rule.json`. The agent reads this file
//! directly (v2), so the GUI writing it — via a trusted Tauri path, not the
//! sandboxed agent tools — is how a decision persists. We read-modify-write
//! the whole file to preserve any existing rules and unknown fields.

use std::path::Path;

use serde_json::json;

use crate::config_io;

/// Append an `allow` rule for `rule_path` (workspace-relative, or `~`/absolute)
/// scoped to `access` ("read" | "write"). Creates the file if absent, skips
/// exact duplicates, and preserves existing content.
///
/// The file is a user-editable one the agent reads directly, so the read is
/// *strict*: a corrupt/hand-broken file is an error, never silently rebuilt from
/// scratch — otherwise a single GUI "Allow" would drop the user's existing (incl.
/// `deny`) rules on the floor. The whole read-modify-write is serialized
/// and the write is atomic.
pub fn append_workspace_allow_rule(
    workspace_dir: &str,
    rule_path: &str,
    access: &str,
) -> Result<(), crate::AppError> {
    append_workspace_allow_rules(
        workspace_dir,
        &[(rule_path.to_string(), access.to_string())],
    )
}

/// Atomically append a complete approval target set. Validation happens before
/// the config lock/write, so a malformed item cannot leave a partially saved
/// multi-target approval.
pub fn append_workspace_allow_rules(
    workspace_dir: &str,
    rules_to_add: &[(String, String)],
) -> Result<(), crate::AppError> {
    if rules_to_add.is_empty() || rules_to_add.len() > 8 {
        return Err("approval rule batch must contain 1 to 8 items".into());
    }
    for (rule_path, access) in rules_to_add {
        if rule_path.trim().is_empty() {
            return Err("approval rule path must not be empty".into());
        }
        // Guard the access scope before it lands in a persisted rule.
        if access != "read" && access != "write" {
            return Err(
                format!("approval access must be \"read\" or \"write\", got {access:?}").into(),
            );
        }
    }

    let dir = Path::new(workspace_dir).join(".future");
    let file = dir.join("approval_rule.json");

    config_io::with_config_lock(&file, || {
        let mut root = config_io::read_json_object(&file)?;
        let obj = root
            .as_object_mut()
            .expect("read_json_object always returns an object");
        obj.entry("version").or_insert(json!(1));
        let rules = obj.entry("rules").or_insert_with(|| json!([]));
        if !rules.is_array() {
            *rules = json!([]);
        }
        let arr = rules.as_array_mut().expect("array ensured above");

        for (rule_path, access) in rules_to_add {
            let new_rule = json!({ "path": rule_path, "access": access, "action": "allow" });
            if !arr.iter().any(|existing| existing == &new_rule) {
                arr.push(new_rule);
            }
        }

        config_io::write_json_atomic(&file, &root, false)
    })
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn temp_ws(name: &str) -> std::path::PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("futureos-rulefile-{name}-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(ws: &Path) -> Value {
        let contents = std::fs::read_to_string(ws.join(".future/approval_rule.json")).unwrap();
        serde_json::from_str(&contents).unwrap()
    }

    #[test]
    fn creates_file_with_rule() {
        let ws = temp_ws("create");
        append_workspace_allow_rule(ws.to_string_lossy().as_ref(), "dist/*", "write").unwrap();
        let v = read(&ws);
        assert_eq!(v["version"], 1);
        assert_eq!(v["rules"][0]["path"], "dist/*");
        assert_eq!(v["rules"][0]["access"], "write");
        assert_eq!(v["rules"][0]["action"], "allow");
    }

    #[test]
    fn appends_and_dedupes() {
        let ws = temp_ws("append");
        let dir = ws.to_string_lossy().to_string();
        append_workspace_allow_rule(&dir, "a/*", "read").unwrap();
        append_workspace_allow_rule(&dir, "b/*", "write").unwrap();
        append_workspace_allow_rule(&dir, "a/*", "read").unwrap(); // dup
        let v = read(&ws);
        assert_eq!(v["rules"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn appends_multi_target_batch_atomically() {
        let ws = temp_ws("batch");
        append_workspace_allow_rules(
            ws.to_string_lossy().as_ref(),
            &[
                ("D:\\release".to_string(), "write".to_string()),
                ("D:\\symbols".to_string(), "write".to_string()),
            ],
        )
        .unwrap();
        let value = read(&ws);
        assert_eq!(value["rules"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn invalid_multi_target_batch_writes_nothing() {
        let ws = temp_ws("batch-invalid");
        let result = append_workspace_allow_rules(
            ws.to_string_lossy().as_ref(),
            &[
                ("D:\\release".to_string(), "write".to_string()),
                ("D:\\bad".to_string(), "execute".to_string()),
            ],
        );
        assert!(result.is_err());
        assert!(!ws.join(".future/approval_rule.json").exists());
    }

    #[test]
    fn preserves_existing_rules_and_unknown_fields() {
        let ws = temp_ws("preserve");
        std::fs::create_dir_all(ws.join(".future")).unwrap();
        std::fs::write(
            ws.join(".future/approval_rule.json"),
            r#"{"version":1,"note":"hand-edited","rules":[{"path":"secrets","action":"deny"}]}"#,
        )
        .unwrap();
        append_workspace_allow_rule(ws.to_string_lossy().as_ref(), "out/*", "write").unwrap();
        let v = read(&ws);
        assert_eq!(v["note"], "hand-edited");
        assert_eq!(v["rules"][0]["action"], "deny"); // existing kept
        assert_eq!(v["rules"][1]["path"], "out/*"); // new appended
    }

    #[test]
    fn rejects_invalid_access_scope() {
        let ws = temp_ws("badaccess");
        let err = append_workspace_allow_rule(ws.to_string_lossy().as_ref(), "x/*", "execute")
            .unwrap_err();
        assert!(err.to_string().contains("approval access"));
        // Nothing was written for an invalid scope.
        assert!(!ws.join(".future/approval_rule.json").exists());
    }

    #[test]
    fn rejects_an_empty_or_oversized_batch() {
        let ws = temp_ws("batch-size");
        let dir = ws.to_string_lossy().to_string();
        let empty: &[(String, String)] = &[];
        let err = append_workspace_allow_rules(&dir, empty).unwrap_err();
        assert!(err.to_string().contains("1 to 8"));

        let oversized: Vec<(String, String)> = (0..9)
            .map(|i| (format!("path/{i}"), "read".to_string()))
            .collect();
        let err = append_workspace_allow_rules(&dir, &oversized).unwrap_err();
        assert!(err.to_string().contains("1 to 8"));
        // Nothing was written for either rejected batch.
        assert!(!ws.join(".future/approval_rule.json").exists());
    }

    #[test]
    fn rejects_an_empty_rule_path() {
        let ws = temp_ws("empty-path");
        let err = append_workspace_allow_rules(
            ws.to_string_lossy().as_ref(),
            &[("   ".to_string(), "read".to_string())],
        )
        .unwrap_err();
        assert!(err.to_string().contains("not be empty"));
        assert!(!ws.join(".future/approval_rule.json").exists());
    }

    #[test]
    fn non_array_rules_field_is_rebuilt() {
        let ws = temp_ws("nonarray");
        std::fs::create_dir_all(ws.join(".future")).unwrap();
        std::fs::write(
            ws.join(".future/approval_rule.json"),
            r#"{"version":1,"rules":"not-an-array"}"#,
        )
        .unwrap();
        append_workspace_allow_rule(ws.to_string_lossy().as_ref(), "out/*", "write").unwrap();
        let v = read(&ws);
        assert!(v["rules"].is_array());
        assert_eq!(v["rules"][0]["path"], "out/*");
    }
}
