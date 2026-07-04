//! Workspace approval-rule file writes (APPROVAL_PLAN.md §4).
//!
//! "Allow in this workspace" appends an `allow` rule to
//! `${WORKSPACE}/.future/approval_rule.json`. The agent reads this file
//! directly (v2), so the GUI writing it — via a trusted Tauri path, not the
//! sandboxed agent tools — is how a decision persists. We read-modify-write
//! the whole file to preserve any existing rules and unknown fields.

use std::path::Path;

use serde_json::{json, Value};

/// Append an `allow` rule for `rule_path` (workspace-relative, or `~`/absolute)
/// scoped to `access` ("read" | "write"). Creates the file if absent, skips
/// exact duplicates, and preserves existing content.
pub fn append_workspace_allow_rule(
    workspace_dir: &str,
    rule_path: &str,
    access: &str,
) -> Result<(), crate::AppError> {
    let dir = Path::new(workspace_dir).join(".future");
    let file = dir.join("approval_rule.json");

    let mut root: Value = std::fs::read_to_string(&file)
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({ "version": 1, "rules": [] }));

    let obj = root.as_object_mut().expect("filtered to object above");
    obj.entry("version").or_insert(json!(1));
    let rules = obj.entry("rules").or_insert_with(|| json!([]));
    if !rules.is_array() {
        *rules = json!([]);
    }
    let arr = rules.as_array_mut().expect("array ensured above");

    let new_rule = json!({ "path": rule_path, "access": access, "action": "allow" });
    if !arr.iter().any(|existing| existing == &new_rule) {
        arr.push(new_rule);
    }

    std::fs::create_dir_all(&dir)?;
    let pretty = serde_json::to_string_pretty(&root)
        .map_err(|error| format!("could not serialize approval rules: {error}"))?;
    std::fs::write(&file, pretty)?;
    Ok(())
}

#[cfg(test)]
mod tests {
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
        let contents =
            std::fs::read_to_string(ws.join(".future/approval_rule.json")).unwrap();
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
}
