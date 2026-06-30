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
    let Some(tool_call_id) = value_string(value, &["tool_id", "toolID", "tool_call_id"]) else {
        return;
    };
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
    let reviewer = value_string(value, &["reviewer"]);

    if let Err(error) = store::ensure_approval_request(store::EnsureApprovalRequestInput {
        approval_request_id: Some(approval_request_id),
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
        reviewer,
    }) {
        eprintln!("FutureOS approval persistence failed: {error}");
    }
    if let Err(error) = store::update_run_status(store::UpdateRunStatusInput {
        run_id: run_id.to_string(),
        status: "waiting_approval".to_string(),
        error_message: None,
        error_type: None,
    }) {
        eprintln!("FutureOS run approval status update failed: {error}");
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
    // (a `bash` call could bypass them). "上一轮变更" now comes from real
    // before/after shadow snapshots — see agent_bridge/review.rs (§14.3).
    let _ = tool_call_id;
}

fn persist_tool_end(run_id: &str, value: &serde_json::Value, sequence: i64) {
    let tool_name =
        value_string(value, &["tool_name", "toolName"]).unwrap_or_else(|| "tool".to_string());
    let tool_call_id = value_string(value, &["tool_id", "toolID", "tool_call_id"])
        .unwrap_or_else(|| format!("{run_id}_tool_{sequence}"));
    let error = value_string(value, &["error", "errorText"]);
    let output_content =
        value_string(value, &["text", "result"]).or_else(|| value.get("output").map(compact_json));
    let status = if error.as_deref().unwrap_or_default().is_empty() {
        "completed".to_string()
    } else {
        "failed".to_string()
    };
    let output_kind = if status == "completed" {
        "text".to_string()
    } else {
        "error".to_string()
    };
    let final_output = error.or(output_content);

    if status == "completed" {
        persist_written_file_artifact(run_id, &tool_name, final_output.as_deref());
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

fn persist_written_file_artifact(run_id: &str, tool_name: &str, output: Option<&str>) {
    if tool_name != "write" {
        return;
    }

    let Some(path) = output.and_then(written_path_from_tool_output) else {
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
