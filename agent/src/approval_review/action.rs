use super::fingerprint::{action_digest, denial_arguments_digest};
use super::redaction::{redact, redact_value};
use serde_json::{json, Value};
use std::time::Instant;

pub(crate) struct PreparedReviewAction {
    pub facts: Value,
    pub action_digest: String,
    pub denial_bucket_key: String,
    pub tool_call_id: String,
    pub started: Instant,
}

/// Exact executable arguments are hashed; only bounded redacted facts leave the host.
pub(crate) fn prepare_review_action(
    tool: &str,
    tool_id: &str,
    arguments: &Value,
    shape: &Value,
    boundary: &Value,
    cwd: &str,
) -> PreparedReviewAction {
    let started = Instant::now();
    let digest = action_digest(tool, tool_id, arguments, boundary, cwd);
    let targets = shape
        .get("targets")
        .cloned()
        .or_else(|| shape.get("paths").cloned())
        .or_else(|| shape.get("path").map(|p| json!([p])))
        .unwrap_or(json!([]));
    let target_access = if tool == "read" {
        "read"
    } else if tool == "shell" {
        "unknown"
    } else {
        "write"
    };
    let normalized_targets: Vec<Value> = targets
        .as_array()
        .into_iter()
        .flatten()
        .map(|target| {
            let path = target
                .as_str()
                .or_else(|| target.get("path").and_then(Value::as_str));
            if let Some(path) = path {
                json!({
                    "type":"path", "value":path, "access":target_access,
                    "inside_workspace":crate::sandbox::paths::path_within(
                        std::path::Path::new(path), std::path::Path::new(cwd)),
                    "scope":target.get("scope")
                })
            } else {
                json!({"type":"unknown", "value":target, "access":"unknown"})
            }
        })
        .collect();
    let mut bucket_targets: Vec<String> = targets
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|target| {
            target
                .as_str()
                .or_else(|| target.get("path").and_then(Value::as_str))
        })
        .map(str::to_owned)
        .collect();
    bucket_targets.sort();
    bucket_targets.dedup();
    // Diagnostic paths cannot bind an action's scope. With no confirmed
    // target, bind the executable parameters, excluding changing review prose
    // and call identity so another call ID cannot reset the same denial bucket.
    let bucket_scope = if bucket_targets.is_empty() {
        json!({"execution_digest":denial_arguments_digest(arguments)})
    } else {
        json!({"targets":bucket_targets})
    };
    let denial_bucket_key = json!({
        "tool":tool, "kind":shape["category"], "cwd":cwd,
        "scope":bucket_scope, "behavior":shape["behavior"]
    })
    .to_string();
    let mut facts = json!({
        "version":1, "tool_name":tool, "tool_call_id":tool_id, "cwd":cwd,
        "kind":shape["category"], "targets":normalized_targets,
        "sandbox_boundary":boundary, "network":if tool=="shell" {"unknown"} else {"none"},
        "rule_result":"ask", "scope":shape["scope"], "behavior":shape["behavior"],
        "diagnostic_paths":shape["diagnostic_paths"]
    });
    if tool == "shell" && normalized_targets.is_empty() {
        if let Some(scope) = facts["scope"].as_object_mut() {
            // A workspace cwd does not prove where an arbitrary command acts.
            scope.insert("inside_workspace".into(), Value::Null);
        }
    }
    if let Some(command) = arguments.get("command").and_then(Value::as_str) {
        if command.len() > super::ACTION_RAW_BYTES_LIMIT {
            facts["oversized_command_bytes"] = json!(command.len());
        } else {
            facts["command"] = json!(redact(command));
        }
    }
    // File bodies and environment values are never approval input or audit data.
    PreparedReviewAction {
        facts: redact_value(&facts),
        action_digest: digest,
        denial_bucket_key,
        tool_call_id: tool_id.to_owned(),
        started,
    }
}
