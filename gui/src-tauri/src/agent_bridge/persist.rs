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

    // `text_delta` is the agent's per-token twin of the consolidated `text_chunk`
    // stream. The GUI's run projection renders from `text_chunk` (and the final
    // answer lives in `messages.content`); nothing reads `text_delta`, so storing
    // it only bloats `run_events`. Skip it. Live remote streaming is unaffected —
    // that taps the event stream in `stream.rs`, before persistence.
    if event_type == "text_delta" {
        return;
    }

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
        || output_is_failure(output_content.as_deref(), run_id, &tool_call_id);
    let final_output = error.or(output_content);

    if !failed {
        persist_written_file_artifact(run_id, &tool_name, &tool_call_id, final_output.as_deref());
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

/// Whether a tool_end output represents a failure. A bash result is formatted
/// "[exit code: N]\n…" only when the command exited non-zero (see agent
/// `tools::run_bash`). Any non-zero code is a failure, except exit 1 from a
/// bare grep/diff/cmp/test (a normal "no match / differs / false" signal).
fn output_is_failure(output: Option<&str>, run_id: &str, tool_call_id: &str) -> bool {
    let Some(code) = nonzero_exit_code(output) else {
        return false;
    };
    if code != 1 {
        return true;
    }
    !is_soft_fail_command(bash_command_from_input(run_id, tool_call_id).as_deref())
}

/// The non-zero code from a "[exit code: N]" bash prefix, or None (exit 0 / not bash).
fn nonzero_exit_code(output: Option<&str>) -> Option<i64> {
    let rest = output?.trim_start().strip_prefix("[exit code: ")?;
    let (code, _) = rest.split_once(']')?;
    code.trim().parse::<i64>().ok().filter(|code| *code != 0)
}

/// A bare grep/diff/cmp/test command exiting 1 is a normal signal, not an error.
/// Any shell operator makes the exit code ambiguous (pipeline/list), so those
/// stay failures. `findstr` is the Windows grep (bash tool runs via `cmd /c`
/// there); `find` is deliberately absent — it means different things on Windows
/// vs Unix.
fn is_soft_fail_command(command: Option<&str>) -> bool {
    let Some(command) = command else {
        return false;
    };
    if command.contains(['|', '&', ';', '\n', '`', '<', '>']) || command.contains("$(") {
        return false;
    }
    let Some(first) = command.split_whitespace().next() else {
        return false;
    };
    // Basename of the program, tolerant of Windows paths (`\`), a `.exe` suffix,
    // and case (Windows resolves names case-insensitively).
    let base = first
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(first)
        .to_ascii_lowercase();
    let program = base.strip_suffix(".exe").unwrap_or(base.as_str());
    matches!(
        program,
        "grep" | "egrep" | "fgrep" | "rg" | "findstr" | "diff" | "cmp" | "test" | "["
    )
}

/// The `command` string persisted at tool_start for a bash tool call, if any.
fn bash_command_from_input(run_id: &str, tool_call_id: &str) -> Option<String> {
    let input = store::get_tool_call_input(run_id, tool_call_id).ok()??;
    let mut value: serde_json::Value = serde_json::from_str(&input).ok()?;
    for _ in 0..2 {
        match value {
            serde_json::Value::String(inner) => value = serde_json::from_str(&inner).ok()?,
            _ => break,
        }
    }
    value
        .get("command")?
        .as_str()
        .map(str::to_string)
        .filter(|command| !command.trim().is_empty())
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
    use super::{is_soft_fail_command, nonzero_exit_code};

    #[test]
    fn parses_nonzero_exit_prefix() {
        assert_eq!(
            nonzero_exit_code(Some("[exit code: 127]\nbash: future: command not found")),
            Some(127)
        );
        assert_eq!(nonzero_exit_code(Some("  [exit code: 2]\noops")), Some(2));
        assert_eq!(nonzero_exit_code(Some("[exit code: 1]\n")), Some(1));
    }

    #[test]
    fn no_prefix_or_zero_is_not_nonzero() {
        // exit 0 never carries the prefix (agent only prepends it when non-zero).
        assert_eq!(nonzero_exit_code(Some("[exit code: 0]\n")), None);
        assert_eq!(nonzero_exit_code(Some("hello world")), None);
        assert_eq!(nonzero_exit_code(Some("")), None);
        assert_eq!(nonzero_exit_code(None), None);
        assert_eq!(nonzero_exit_code(Some("[exit code: abc]\n")), None);
    }

    #[test]
    fn bare_soft_fail_commands_are_exempt() {
        assert!(is_soft_fail_command(Some("grep foo file.txt")));
        assert!(is_soft_fail_command(Some("rg pattern")));
        assert!(is_soft_fail_command(Some("diff a b")));
        assert!(is_soft_fail_command(Some("test -f missing")));
        assert!(is_soft_fail_command(Some("[ -f missing ]")));
        assert!(is_soft_fail_command(Some("/usr/bin/grep foo")));
    }

    #[test]
    fn windows_forms_are_exempt() {
        assert!(is_soft_fail_command(Some("findstr foo file.txt")));
        assert!(is_soft_fail_command(Some("grep.exe foo")));
        assert!(is_soft_fail_command(Some("GREP.EXE foo")));
        assert!(is_soft_fail_command(Some(r"C:\tools\grep.exe foo")));
    }

    #[test]
    fn pipelines_lists_and_other_commands_are_not_exempt() {
        assert!(!is_soft_fail_command(Some("grep foo | head")));
        assert!(!is_soft_fail_command(Some("grep foo && echo hi")));
        assert!(!is_soft_fail_command(Some("grep foo; echo hi")));
        assert!(!is_soft_fail_command(Some("python script.py")));
        assert!(!is_soft_fail_command(Some("npm run build")));
        assert!(!is_soft_fail_command(None));
    }
}
