//! Projects Future Agent stream events into the local store: raw run-event log,
//! tool-call lifecycle, approval requests/decisions, review changes, and
//! artifacts. All persistence is best-effort — failures are logged, never
//! propagated, so a storage hiccup can't abort an in-flight agent response.

use std::path::Path;

use crate::{git_review, store};

pub(super) fn persist_run_event(
    run_id: Option<&str>,
    event_type: &str,
    payload: &str,
    sequence: i64,
) {
    let Some(run_id) = run_id else {
        return;
    };

    if let Err(error) = store::append_run_event(store::AppendRunEventInput {
        run_id: run_id.to_string(),
        event_type: event_type.to_string(),
        payload: if payload.is_empty() {
            None
        } else {
            Some(payload.to_string())
        },
        sequence,
    }) {
        eprintln!("FutureOS run event persistence failed: {error}");
    }

    persist_agent_tool_projection(run_id, event_type, payload, sequence);
}

fn persist_agent_tool_projection(run_id: &str, event_type: &str, payload: &str, sequence: i64) {
    let Some(value) = event_value(payload) else {
        return;
    };

    match event_type {
        "approval_request" => persist_approval_request(run_id, &value),
        "approval_decision" => persist_approval_decision(run_id, &value),
        "tool_start" | "toolcall_start" => persist_tool_start(run_id, &value, sequence),
        "tool_end" | "tool_result" => persist_tool_end(run_id, &value, sequence),
        "artifact_created" | "artifact.created" => persist_artifact(run_id, &value),
        _ => {}
    }
}

fn persist_approval_request(run_id: &str, value: &serde_json::Value) {
    let Some(approval_request_id) =
        value_string(value, &["approval_request_id", "approvalRequestId"])
    else {
        return;
    };
    // Escalation approvals carry no tool_call id (they belong to the run, not a
    // specific persisted tool_call). Store NULL rather than an empty string —
    // an empty string violates the tool_calls(id) foreign key, which would drop
    // the approval on the floor and leave the run stuck "waiting_approval" with
    // no card to act on.
    let tool_call_id =
        value_string(value, &["tool_id", "toolID", "tool_call_id"]).filter(|id| !id.is_empty());
    let tool_name =
        value_string(value, &["tool_name", "toolName"]).unwrap_or_else(|| "tool".to_string());
    let requested_action = value
        .get("requested_action")
        .or_else(|| value.get("requestedAction"))
        .or_else(|| value.get("tool_args"))
        .map(compact_json);

    // P2: structured action and sandbox boundary, persisted as JSON strings.
    let action_value = value.get("action").or_else(|| value.get("actionPayload"));
    let action_payload = action_value.map(compact_json);
    let action_category = action_value
        .and_then(|action| action.get("category"))
        .and_then(|category| category.as_str())
        .map(|category| category.to_string());
    let sandbox_boundary = value
        .get("sandbox_boundary")
        .or_else(|| value.get("sandboxBoundary"))
        .map(compact_json);
    // Only persist a real suggestion object (agent sends JSON null when none).
    let save_suggestion = value
        .get("save_suggestion")
        .or_else(|| value.get("saveSuggestion"))
        .filter(|v| v.is_object())
        .map(compact_json);
    let reviewer = value_string(value, &["reviewer"]);

    if let Err(error) = store::ensure_approval_request(store::EnsureApprovalRequestInput {
        approval_request_id: Some(approval_request_id.clone()),
        run_id: run_id.to_string(),
        tool_call_id,
        kind: value_string(value, &["kind"]).unwrap_or_else(|| "tool".to_string()),
        title: value_string(value, &["title"]).unwrap_or_else(|| format!("Approve `{tool_name}`")),
        summary: value_string(value, &["summary"]),
        risk_level: value_string(value, &["risk_level", "riskLevel"]),
        requested_action,
        action_category,
        action_payload,
        sandbox_boundary,
        save_suggestion,
        reviewer,
    }) {
        eprintln!("FutureOS approval persistence failed: {error}");
    }
    // CAS the run to waiting_approval only if it isn't already terminal. Without
    // the guard a late-arriving approval_request event (the user aborted while
    // this event was in flight) would resurrect a `cancelled` run — and since
    // the agent has already aborted, no decision event ever comes back, stranding
    // the run in `waiting_approval` forever. When the run is terminal we
    // cancel the approval we just recorded so no dangling pending card remains.
    match store::update_run_status_if_active(store::UpdateRunStatusInput {
        run_id: run_id.to_string(),
        status: "waiting_approval".to_string(),
        error_message: None,
        error_type: None,
    }) {
        Ok(false) => {
            if let Err(error) = store::decide_approval_request(store::DecideApprovalRequestInput {
                approval_request_id,
                status: "cancelled".to_string(),
                decision_note: Some("Cancelled because the run had already ended.".to_string()),
            }) {
                eprintln!("FutureOS stale approval cancellation failed: {error}");
            }
        }
        Ok(true) => {}
        Err(error) => eprintln!("FutureOS run approval status update failed: {error}"),
    }
}

fn persist_approval_decision(run_id: &str, value: &serde_json::Value) {
    let Some(approval_request_id) =
        value_string(value, &["approval_request_id", "approvalRequestId"])
    else {
        return;
    };
    let status = value_string(value, &["status"]).unwrap_or_else(|| "cancelled".to_string());
    let note = value_string(value, &["note"]);

    if let Err(error) = store::decide_approval_request(store::DecideApprovalRequestInput {
        approval_request_id,
        status: status.clone(),
        decision_note: note,
    }) {
        eprintln!("FutureOS approval decision persistence failed: {error}");
    }

    if status == "cancelled" {
        if let Err(error) = store::update_run_status_if_active(store::UpdateRunStatusInput {
            run_id: run_id.to_string(),
            status: "cancelled".to_string(),
            error_message: Some("Approval request was cancelled.".to_string()),
            error_type: None,
        }) {
            eprintln!("FutureOS run approval cancellation status update failed: {error}");
        }
        return;
    }

    if let Err(error) = store::update_run_status_if_active(store::UpdateRunStatusInput {
        run_id: run_id.to_string(),
        status: "running".to_string(),
        error_message: None,
        error_type: None,
    }) {
        eprintln!("FutureOS run approval decision status update failed: {error}");
    }
}

fn persist_tool_start(run_id: &str, value: &serde_json::Value, sequence: i64) {
    let tool_name =
        value_string(value, &["tool_name", "toolName"]).unwrap_or_else(|| "tool".to_string());
    let tool_call_id = value_string(value, &["tool_id", "toolID", "tool_call_id"])
        .unwrap_or_else(|| format!("{run_id}_tool_{sequence}"));
    let input = value
        .get("tool_args")
        .or_else(|| value.get("toolArgs"))
        .or_else(|| value.get("arguments"))
        .map(compact_json);

    if let Err(error) = store::upsert_tool_call(store::UpsertToolCallInput {
        run_id: run_id.to_string(),
        tool_call_id: tool_call_id.clone(),
        name: tool_name.clone(),
        kind: "agent_tool".to_string(),
        input: input.clone(),
        status: "running".to_string(),
    }) {
        eprintln!("FutureOS tool call persistence failed: {error}");
    }

    // Review changesets are no longer guessed from write/edit tool-start events
    // (a `bash` call could bypass them). "Previous turn changes" now come from real
    // before/after shadow snapshots — see agent_bridge/review.rs (§14.3).
}

fn persist_tool_end(run_id: &str, value: &serde_json::Value, sequence: i64) {
    let tool_name =
        value_string(value, &["tool_name", "toolName"]).unwrap_or_else(|| "tool".to_string());
    let tool_call_id = value_string(value, &["tool_id", "toolID", "tool_call_id"])
        .unwrap_or_else(|| format!("{run_id}_tool_{sequence}"));
    let error = value_string(value, &["error", "errorText"]);
    let output_content =
        value_string(value, &["text", "result"]).or_else(|| value.get("output").map(compact_json));
    // A bash command that runs but exits non-zero is returned as a *successful*
    // tool result (no error field) with the code baked into the output text as
    // "[exit code: N]\n…". Treat a non-zero code as a failure so the Runs panel
    // and inspector don't mark an errored command as completed.
    let failed = !error.as_deref().unwrap_or_default().is_empty()
        || output_has_nonzero_exit(output_content.as_deref());
    let status = if failed {
        "failed".to_string()
    } else {
        "completed".to_string()
    };
    let output_kind = if status == "completed" {
        "text".to_string()
    } else {
        "error".to_string()
    };
    let final_output = error.or(output_content);

    if status == "completed" {
        persist_written_file_artifact(run_id, &tool_name, &tool_call_id, final_output.as_deref());
    }

    if let Err(error) = store::complete_tool_call(store::CompleteToolCallInput {
        run_id: run_id.to_string(),
        tool_call_id,
        name: tool_name,
        status,
        output_kind,
        output_content: final_output,
    }) {
        eprintln!("FutureOS tool output persistence failed: {error}");
    }
}

fn persist_written_file_artifact(
    run_id: &str,
    tool_name: &str,
    tool_call_id: &str,
    output: Option<&str>,
) {
    if tool_name != "write" {
        return;
    }

    // Prefer the structured `tool_args` persisted at tool_start — the output
    // prose ("Written to …") is display text, not a contract, and a reworded
    // agent message would otherwise silently stop artifact recording. The
    // prose parse stays as a fallback for rows without a stored input.
    let Some(path) = written_path_from_tool_input(run_id, tool_call_id)
        .or_else(|| output.and_then(written_path_from_tool_output))
    else {
        return;
    };
    match path_allowed_for_run(run_id, Some(&path)) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            eprintln!("FutureOS write artifact workspace check failed: {error}");
            return;
        }
    }

    let path_ref = Path::new(&path);
    let title = path_ref
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Written file")
        .to_string();
    let artifact_type = crate::store::artifact_type_from_path(path_ref);

    if let Err(error) = store::ensure_artifact(store::EnsureArtifactInput {
        run_id: run_id.to_string(),
        title,
        artifact_type,
        path: Some(path),
        content: None,
        content_storage: Some("file".to_string()),
        summary: Some("Written by Agent.".to_string()),
    }) {
        eprintln!("FutureOS write artifact persistence failed: {error}");
    }
}

/// Extract the write target from the tool call's stored `input` (the agent's
/// `tool_args`). The stored value may be a JSON object or a JSON-encoded string
/// of one (the agent serializes args to a string field), so unwrap up to two
/// string layers before reading `path`.
fn written_path_from_tool_input(run_id: &str, tool_call_id: &str) -> Option<String> {
    let input = store::get_tool_call_input(run_id, tool_call_id).ok()??;
    let mut value: serde_json::Value = serde_json::from_str(&input).ok()?;
    for _ in 0..2 {
        match value {
            serde_json::Value::String(inner) => value = serde_json::from_str(&inner).ok()?,
            _ => break,
        }
    }
    value
        .get("path")?
        .as_str()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
}

fn written_path_from_tool_output(output: &str) -> Option<String> {
    output
        .trim()
        .strip_prefix("Written to ")
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
}

fn persist_artifact(run_id: &str, value: &serde_json::Value) {
    let title = value_string(value, &["title", "name"]).unwrap_or_else(|| "Artifact".to_string());
    let artifact_type = value_string(value, &["type", "artifact_type", "artifactType"])
        .unwrap_or_else(|| "document".to_string());
    let path = value_string(value, &["path", "file_path", "filePath"]);
    match path_allowed_for_run(run_id, path.as_deref()) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            eprintln!("FutureOS artifact workspace check failed: {error}");
            return;
        }
    }
    let content = value_string(value, &["content", "text"]);
    let content_storage = value_string(value, &["content_storage", "contentStorage"])
        .or_else(|| path.as_ref().map(|_| "file".to_string()))
        .or_else(|| content.as_ref().map(|_| "inline".to_string()));
    let summary = value_string(value, &["summary", "description"]);
    if let Err(error) = store::ensure_artifact(store::EnsureArtifactInput {
        run_id: run_id.to_string(),
        title,
        artifact_type,
        path,
        content,
        content_storage,
        summary,
    }) {
        eprintln!("FutureOS artifact persistence failed: {error}");
    }
}

/// True when `path` (a write target / artifact path) is inside the Run's
/// workspace, so it's safe to persist as an artifact. Git workspaces opt out
/// entirely (their changes flow through the review pipeline, not artifacts); a
/// `None` path (inline artifact) is always allowed.
fn path_allowed_for_run(run_id: &str, path: Option<&str>) -> Result<bool, crate::AppError> {
    let run = store::get_run(run_id)?.ok_or_else(|| "Run could not be loaded.".to_string())?;
    let thread = store::get_thread(&run.thread_id)?
        .ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let workspace = store::get_workspace(&thread.workspace_id)?
        .ok_or_else(|| "Workspace could not be loaded.".to_string())?;
    if git_review::is_git_workspace(Path::new(&workspace.path)) {
        return Ok(false);
    }

    let Some(path) = path else {
        return Ok(true);
    };
    let workspace_path = git_review::canonical_or_raw(&workspace.path);
    let candidate_path = git_review::canonical_or_raw(path);
    Ok(candidate_path.starts_with(workspace_path))
}

fn event_value(payload: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(payload).ok()
}

/// A bash result is formatted "[exit code: N]\n…" only when the command exited
/// non-zero (see agent `tools::run_bash`). Detect that prefix so a command that
/// ran but failed isn't recorded as completed.
fn output_has_nonzero_exit(output: Option<&str>) -> bool {
    let Some(rest) = output.and_then(|text| text.trim_start().strip_prefix("[exit code: ")) else {
        return false;
    };
    let Some((code, _)) = rest.split_once(']') else {
        return false;
    };
    code.trim().parse::<i64>().is_ok_and(|code| code != 0)
}

fn value_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|field| {
            field
                .as_str()
                .map(str::to_string)
                .or_else(|| (!field.is_null()).then(|| compact_json(field)))
        })
    })
}

fn compact_json(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::output_has_nonzero_exit;

    #[test]
    fn nonzero_exit_prefix_is_a_failure() {
        assert!(output_has_nonzero_exit(Some(
            "[exit code: 127]\nbash: future: command not found"
        )));
        assert!(output_has_nonzero_exit(Some("[exit code: 1]\n")));
        assert!(output_has_nonzero_exit(Some("  [exit code: 2]\noops")));
    }

    #[test]
    fn success_or_plain_output_is_not_a_failure() {
        // exit 0 never carries the prefix (agent only prepends it when non-zero),
        // but guard the literal case anyway.
        assert!(!output_has_nonzero_exit(Some("[exit code: 0]\n")));
        assert!(!output_has_nonzero_exit(Some("hello world")));
        assert!(!output_has_nonzero_exit(Some("")));
        assert!(!output_has_nonzero_exit(None));
        assert!(!output_has_nonzero_exit(Some("[exit code: abc]\n")));
    }
}
