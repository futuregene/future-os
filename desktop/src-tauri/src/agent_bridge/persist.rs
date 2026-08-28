//! Projects Future Agent stream events into the local store: tool-call
//! lifecycle, approval requests/decisions, review changes, and
//! artifacts. All persistence is best-effort — failures are logged, never
//! propagated, so a storage hiccup can't abort an in-flight agent response.

use std::path::Path;

use crate::{git_review, store};

/// Whether an Agent event contributes to a GUI-owned projection. The Agent
/// journal remains the canonical, complete event log; events outside this set
/// need no duplicate SQLite/tool projection work in the desktop process.
pub(super) fn requires_gui_projection(event_type: &str) -> bool {
    matches!(
        event_type,
        "tool_start"
            | "toolcall_start"
            | "tool_end"
            | "tool_result"
            | "approval_request"
            | "approval_decision"
            | "artifact_created"
            | "artifact.created"
    )
}

pub(super) fn persist_run_event(
    run_id: Option<&str>,
    event_type: &str,
    payload: &str,
    sequence: i64,
) {
    let Some(run_id) = run_id else {
        return;
    };

    // Fold tool-lifecycle events into the shared tool projection as they land:
    // it is the single in-memory tool index (Runs-panel reads and tool_end
    // artifact-path extraction both draw from it), and feeding it here keeps it
    // warm in real time instead of waiting for a journal-tail poll. Journal-tail
    // pollers and fork/import synthesis advance the same cache; the projection's
    // sequence guard makes the overlap idempotent. Deltas only reach this fold
    // via projection snapshots (reattach replay) — the live path keeps
    // token-heavy events on the cheap notification path in stream.rs.
    if matches!(
        event_type,
        "tool_start"
            | "toolcall_start"
            | "tool_delta"
            | "toolcall_delta"
            | "tool_end"
            | "tool_result"
    ) {
        let record = store::RunEventRecord {
            id: store::create_id("event"),
            run_id: run_id.to_string(),
            event_type: event_type.to_string(),
            payload: Some(payload.to_string()),
            sequence,
            created_at: store::now_millis(),
        };
        store::advance_tool_projection(run_id, std::slice::from_ref(&record));
    }

    // Raw events are durable in the Agent event journal. Do not create a
    // second GUI JSONL copy: SQLite receives only independently useful
    // projections needed by sidebar approvals and tool/detail panels.
    // Tool starts have already advanced the shared in-memory tool projection
    // above and need no payload parse a second time.
    if !matches!(
        event_type,
        "approval_request"
            | "approval_decision"
            | "tool_end"
            | "tool_result"
            | "artifact_created"
            | "artifact.created"
    ) {
        return;
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
    // specific tool call). Store NULL rather than an empty string so queries
    // can treat "no tool call" uniformly with IS NULL.
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
    // A shell command that runs but exits non-zero is returned as a *successful*
    // tool result (no error field). The agent reports the conclusion structured
    // (`exit_code` / `is_soft_fail`) on the event; legacy/synthesized events
    // without those fields fall back to inspecting the output text.
    let failed = !error.as_deref().unwrap_or_default().is_empty()
        || output_is_failure(value, output_content.as_deref(), run_id, &tool_call_id);
    let final_output = error.or(output_content);

    if !failed {
        let target_path = value_string(value, &["target_path", "targetPath"]);
        persist_file_artifact(
            run_id,
            &tool_name,
            &tool_call_id,
            final_output.as_deref(),
            target_path.as_deref(),
        );
    }
}

fn persist_file_artifact(
    run_id: &str,
    tool_name: &str,
    tool_call_id: &str,
    output: Option<&str>,
    target_path: Option<&str>,
) {
    let summary = match tool_name {
        "write" => "Written by Agent.",
        "edit" => "Edited by Agent.",
        _ => return,
    };

    // Prefer the structured `tool_args` persisted at tool_start — the output
    // prose ("Written to …" / "Edited …") is display text, not a contract, and a
    // reworded agent message would otherwise silently stop artifact recording.
    // The agent's structured `target_path` on tool_end is next, and the prose
    // parse stays as a last fallback for rows without either.
    let Some(path) = file_path_from_tool_input(run_id, tool_call_id)
        .or_else(|| target_path.map(str::to_string))
        .or_else(|| output.and_then(file_path_from_tool_output))
    else {
        return;
    };
    match path_allowed_for_run(run_id, Some(&path)) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            eprintln!("FutureOS {tool_name} artifact workspace check failed: {error}");
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
        summary: Some(summary.to_string()),
    }) {
        eprintln!("FutureOS {tool_name} artifact persistence failed: {error}");
    }
}

/// Extract the write/edit target from the tool call's stored `input` (the
/// agent's `tool_args`). The stored value may be a JSON object or a JSON-encoded
/// string of one (the agent serializes args to a string field), so unwrap up to
/// two string layers before reading `path`.
fn file_path_from_tool_input(run_id: &str, tool_call_id: &str) -> Option<String> {
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

/// Fallback parse of the agent's write/edit success prose: "Written to <path>"
/// (`tools::run_write`) or "Edited <path>" (`tools::run_edit`).
fn file_path_from_tool_output(output: &str) -> Option<String> {
    let output = output.trim();
    output
        .strip_prefix("Written to ")
        .or_else(|| output.strip_prefix("Edited "))
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

/// Whether a tool_end output represents a failure. Prefers the agent's
/// structured `exit_code` + `is_soft_fail` conclusion on the event; events
/// without those (older agents, journal-synthesized imports) fall back to
/// parsing the "[exit: N]" footer the agent's shell tool appends (see
/// `tools::run_shell`). Any non-zero code is a failure, except exit 1 from a
/// bare grep/diff/cmp/test (a normal "no match / differs / false" signal).
fn output_is_failure(
    value: &serde_json::Value,
    output: Option<&str>,
    run_id: &str,
    tool_call_id: &str,
) -> bool {
    if let Some(code) = value
        .get("exit_code")
        .or_else(|| value.get("exitCode"))
        .and_then(|code| code.as_i64())
    {
        if code == 0 {
            return false;
        }
        let soft_fail = value
            .get("is_soft_fail")
            .or_else(|| value.get("isSoftFail"))
            .and_then(|flag| flag.as_bool())
            .unwrap_or(false);
        return !soft_fail;
    }
    let Some(code) = nonzero_exit_code(output) else {
        return false;
    };
    if code != 1 {
        return true;
    }
    !is_soft_fail_command(shell_command_from_input(run_id, tool_call_id).as_deref())
}

/// The non-zero code from the "[exit: N]" footer line, or None (exit 0 / not a
/// shell result).
fn nonzero_exit_code(output: Option<&str>) -> Option<i64> {
    let line = output?.trim_end().lines().last()?;
    let code = line.strip_prefix("[exit: ")?.strip_suffix(']')?;
    code.trim().parse::<i64>().ok().filter(|code| *code != 0)
}

/// A bare grep/diff/cmp/test command exiting 1 is a normal signal, not an error.
/// Any shell operator makes the exit code ambiguous (pipeline/list), so those
/// stay failures. `findstr` is the Windows grep (the shell tool runs via PowerShell
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

/// The `command` string persisted at tool_start for a shell tool call, if any.
fn shell_command_from_input(run_id: &str, tool_call_id: &str) -> Option<String> {
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
    // serde_json::Value serialization is infallible (no custom Serialize impls).
    serde_json::to_string(value).expect("Value serialization is infallible")
}

#[cfg(test)]
mod tests {
    use super::{
        file_path_from_tool_output, is_soft_fail_command, nonzero_exit_code,
        persist_agent_tool_projection,
    };

    #[test]
    fn parses_write_and_edit_success_prose() {
        assert_eq!(
            file_path_from_tool_output("Written to /ws/report.md"),
            Some("/ws/report.md".to_string())
        );
        assert_eq!(
            file_path_from_tool_output("Edited /ws/report.md"),
            Some("/ws/report.md".to_string())
        );
        assert_eq!(
            file_path_from_tool_output(r"Edited C:\ws\report.md"),
            Some(r"C:\ws\report.md".to_string())
        );
    }

    #[test]
    fn ignores_output_without_a_path() {
        assert_eq!(file_path_from_tool_output("Written to "), None);
        assert_eq!(file_path_from_tool_output("Edited"), None);
        assert_eq!(file_path_from_tool_output("Read 40 lines"), None);
        assert_eq!(file_path_from_tool_output(""), None);
    }

    #[test]
    fn unknown_event_types_are_ignored() {
        // The caller pre-filters to the six projected types; the catch-all arm
        // is the defensive no-op for any other event type reaching this helper.
        persist_agent_tool_projection("run-1", "text_chunk", "{}", 0);
        persist_agent_tool_projection("run-1", "agent_end", "{}", 0);
    }

    #[test]
    fn parses_nonzero_exit_footer() {
        assert_eq!(
            nonzero_exit_code(Some("bash: future: command not found\n[exit: 127]")),
            Some(127)
        );
        assert_eq!(nonzero_exit_code(Some("oops\n[exit: 2]  ")), Some(2));
        assert_eq!(nonzero_exit_code(Some("[exit: 1]")), Some(1));
    }

    #[test]
    fn no_prefix_or_zero_is_not_nonzero() {
        // an exit-0 footer is present but parses to None.
        assert_eq!(nonzero_exit_code(Some("[exit: 0]")), None);
        assert_eq!(nonzero_exit_code(Some("hello world")), None);
        assert_eq!(nonzero_exit_code(Some("")), None);
        assert_eq!(nonzero_exit_code(None), None);
        assert_eq!(nonzero_exit_code(Some("[exit: abc]")), None);
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

    #[test]
    fn structured_exit_fields_beat_the_output_prose() {
        use super::output_is_failure;
        use serde_json::json;

        // Structured exit 0 wins even if prose carries a scary footer.
        let value = json!({"exit_code": 0, "text": "ok\n[exit: 127]"});
        assert!(!output_is_failure(
            &value,
            Some("ok\n[exit: 127]"),
            "r",
            "t"
        ));

        // A structured non-zero exit is a failure without any prose.
        let value = json!({"exit_code": 127});
        assert!(output_is_failure(&value, None, "r", "t"));

        // The agent's soft-fail conclusion exempts a bare grep exit 1.
        let value = json!({"exit_code": 1, "is_soft_fail": true});
        assert!(!output_is_failure(&value, None, "r", "t"));

        // camelCase aliases are accepted too.
        let value = json!({"exitCode": 1, "isSoftFail": true});
        assert!(!output_is_failure(&value, None, "r", "t"));

        // No structured fields, no prose -> not a failure.
        let value = json!({"text": "hello"});
        assert!(!output_is_failure(&value, Some("hello"), "r", "t"));
    }

    // ── store-backed persistence paths ────────────────────────────────

    use super::super::test_support::{seed_run, seed_thread, seed_workspace, TestHome};
    use super::{event_value, persist_run_event, requires_gui_projection, value_string};
    use serde_json::json;

    struct Fixture {
        _home: TestHome,
        workspace: crate::store::WorkspaceRecord,
        thread: crate::store::ThreadRecord,
        run: crate::store::RunRecord,
    }

    fn fixture(tag: &str) -> Fixture {
        let home = TestHome::new(tag);
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);
        Fixture {
            _home: home,
            workspace,
            thread,
            run,
        }
    }

    #[test]
    fn only_gui_projection_events_require_blocking_work() {
        for event_type in [
            "tool_start",
            "toolcall_start",
            "tool_end",
            "tool_result",
            "approval_request",
            "approval_decision",
            "artifact_created",
            "artifact.created",
        ] {
            assert!(
                requires_gui_projection(event_type),
                "{event_type} must retain its GUI projection"
            );
        }

        for event_type in [
            "text_chunk",
            "thinking_chunk",
            "usage",
            "agent_end",
            "error",
        ] {
            assert!(
                !requires_gui_projection(event_type),
                "{event_type} should use the journal-only fast path"
            );
        }
    }

    fn artifacts(fixture: &Fixture) -> Vec<crate::store::ArtifactRecord> {
        crate::store::list_artifacts(&fixture.thread.id).expect("artifacts")
    }

    /// Create `path` on disk and return its string form. Artifact paths must
    /// exist for `canonical_or_raw` to resolve them (macOS symlinks /var).
    fn touch(path: &std::path::Path) -> String {
        std::fs::write(path, "content").expect("touch artifact file");
        path.display().to_string()
    }

    #[test]
    fn none_run_id_and_invalid_payloads_are_dropped() {
        let fixture = fixture("persist-none");
        // No run id: nothing is projected at all.
        persist_run_event(None, "tool_start", "{}", 0);
        // Invalid JSON payload: parsed to None and dropped.
        persist_run_event(Some(&fixture.run.id), "tool_end", "not json", 1);
        assert!(event_value("not json").is_none());
        assert!(event_value("{}").is_some());
        assert!(artifacts(&fixture).is_empty());
    }

    #[test]
    fn value_string_picks_strings_objects_and_skips_nulls() {
        let value = json!({"a": "text", "b": {"k": 1}, "c": null, "d": 7});
        assert_eq!(value_string(&value, &["a"]), Some("text".to_string()));
        assert_eq!(value_string(&value, &["b"]), Some(r#"{"k":1}"#.to_string()));
        assert_eq!(
            value_string(&value, &["c"]),
            None,
            "explicit null is absent"
        );
        assert_eq!(value_string(&value, &["d"]), Some("7".to_string()));
        assert_eq!(value_string(&value, &["missing"]), None);
        assert_eq!(
            value_string(&value, &["missing", "a"]),
            Some("text".to_string()),
            "first matching key wins"
        );
    }

    #[test]
    fn approval_request_event_is_projected_with_full_field_mapping() {
        let fixture = fixture("persist-appr");
        let payload = json!({
            "approvalRequestId": "appr-1",
            "tool_id": "tc-1",
            "toolName": "shell",
            "kind": "tool",
            "title": "Approve shell",
            "summary": "run ls",
            "riskLevel": "high",
            "requestedAction": {"command": "ls"},
            "actionPayload": {"category": "process"},
            "sandboxBoundary": {"scope": "workspace"},
            "saveSuggestion": {"scope": "always"},
            "reviewer": "user"
        });
        persist_run_event(
            Some(&fixture.run.id),
            "approval_request",
            &payload.to_string(),
            0,
        );

        let record = crate::store::get_approval_request("appr-1")
            .expect("query")
            .expect("approval row");
        assert_eq!(record.tool_call_id.as_deref(), Some("tc-1"));
        assert_eq!(record.title, "Approve shell");
        assert_eq!(record.summary.as_deref(), Some("run ls"));
        assert_eq!(record.risk_level.as_deref(), Some("high"));
        assert_eq!(
            record.requested_action.as_deref(),
            Some(r#"{"command":"ls"}"#)
        );
        assert_eq!(record.action_category.as_deref(), Some("process"));
        assert_eq!(
            record.action_payload.as_deref(),
            Some(r#"{"category":"process"}"#)
        );
        assert_eq!(
            record.sandbox_boundary.as_deref(),
            Some(r#"{"scope":"workspace"}"#)
        );
        assert_eq!(
            record.save_suggestion.as_deref(),
            Some(r#"{"scope":"always"}"#)
        );
        assert_eq!(record.reviewer, "user");
        assert_eq!(
            crate::store::get_run(&fixture.run.id)
                .expect("run")
                .expect("some")
                .status,
            "waiting_approval",
            "the run parks on the approval"
        );
    }

    #[test]
    fn approval_request_defaults_and_null_suggestion() {
        let fixture = fixture("persist-appr-defaults");
        let payload = json!({
            "approval_request_id": "appr-2",
            "tool_id": "",
            "save_suggestion": null
        });
        persist_run_event(
            Some(&fixture.run.id),
            "approval_request",
            &payload.to_string(),
            0,
        );
        let record = crate::store::get_approval_request("appr-2")
            .expect("query")
            .expect("approval row");
        assert_eq!(record.tool_call_id, None, "empty tool id stores NULL");
        assert_eq!(record.title, "Approve `tool`", "default tool name in title");
        assert_eq!(record.kind, "tool");
        assert_eq!(
            record.save_suggestion, None,
            "JSON null is not a suggestion"
        );

        // Missing approval_request_id: dropped.
        persist_run_event(
            Some(&fixture.run.id),
            "approval_request",
            &json!({"tool_name": "shell"}).to_string(),
            1,
        );
    }

    #[test]
    fn approval_request_for_a_terminal_run_is_cancelled_immediately() {
        let fixture = fixture("persist-appr-terminal");
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: fixture.run.id.clone(),
            status: "completed".to_string(),
            error_message: None,
            error_type: None,
        })
        .expect("complete run");
        let payload = json!({"approval_request_id": "appr-3"});
        persist_run_event(
            Some(&fixture.run.id),
            "approval_request",
            &payload.to_string(),
            0,
        );
        let record = crate::store::get_approval_request("appr-3")
            .expect("query")
            .expect("approval row");
        assert_eq!(
            record.status, "cancelled",
            "a late approval for a terminal run is cancelled, not left pending"
        );
        assert!(
            record
                .decision_note
                .as_deref()
                .unwrap_or_default()
                .contains("already ended"),
            "note: {:?}",
            record.decision_note
        );
    }

    #[test]
    fn approval_decision_variants() {
        let fixture = fixture("persist-appr-decision");

        // Missing id: dropped.
        persist_run_event(
            Some(&fixture.run.id),
            "approval_decision",
            &json!({"status": "approved"}).to_string(),
            0,
        );

        crate::store::ensure_approval_request(crate::store::EnsureApprovalRequestInput {
            approval_request_id: Some("appr-4".to_string()),
            run_id: fixture.run.id.clone(),
            tool_call_id: None,
            kind: "tool".to_string(),
            title: "t".to_string(),
            summary: None,
            risk_level: None,
            requested_action: None,
            action_category: None,
            action_payload: None,
            sandbox_boundary: None,
            save_suggestion: None,
            reviewer: None,
        })
        .expect("seed approval");

        // Approved decision: run resumes.
        persist_run_event(
            Some(&fixture.run.id),
            "approval_decision",
            &json!({"approval_request_id": "appr-4", "status": "approved", "note": "ok"})
                .to_string(),
            1,
        );
        let record = crate::store::get_approval_request("appr-4")
            .expect("query")
            .expect("approval row");
        assert_eq!(record.status, "approved");
        assert_eq!(record.decision_note.as_deref(), Some("ok"));
        assert_eq!(
            crate::store::get_run(&fixture.run.id)
                .expect("run")
                .expect("some")
                .status,
            "running"
        );

        // Cancelled decision: run is cancelled too.
        crate::store::ensure_approval_request(crate::store::EnsureApprovalRequestInput {
            approval_request_id: Some("appr-5".to_string()),
            run_id: fixture.run.id.clone(),
            tool_call_id: None,
            kind: "tool".to_string(),
            title: "t".to_string(),
            summary: None,
            risk_level: None,
            requested_action: None,
            action_category: None,
            action_payload: None,
            sandbox_boundary: None,
            save_suggestion: None,
            reviewer: None,
        })
        .expect("seed approval");
        persist_run_event(
            Some(&fixture.run.id),
            "approval_decision",
            &json!({"approval_request_id": "appr-5", "status": "cancelled"}).to_string(),
            2,
        );
        assert_eq!(
            crate::store::get_run(&fixture.run.id)
                .expect("run")
                .expect("some")
                .status,
            "cancelled"
        );

        // Default status is "cancelled" (no status field).
        crate::store::ensure_approval_request(crate::store::EnsureApprovalRequestInput {
            approval_request_id: Some("appr-6".to_string()),
            run_id: fixture.run.id.clone(),
            tool_call_id: None,
            kind: "tool".to_string(),
            title: "t".to_string(),
            summary: None,
            risk_level: None,
            requested_action: None,
            action_category: None,
            action_payload: None,
            sandbox_boundary: None,
            save_suggestion: None,
            reviewer: None,
        })
        .expect("seed approval");
        persist_run_event(
            Some(&fixture.run.id),
            "approval_decision",
            &json!({"approval_request_id": "appr-6"}).to_string(),
            3,
        );
        assert_eq!(
            crate::store::get_approval_request("appr-6")
                .expect("query")
                .expect("approval row")
                .status,
            "cancelled"
        );
    }

    #[test]
    fn failed_tool_end_records_no_artifact() {
        let fixture = fixture("persist-tool-fail");
        // Structured error field.
        persist_run_event(
            Some(&fixture.run.id),
            "tool_end",
            &json!({"tool_name": "write", "tool_id": "tc-e", "error": "disk full"}).to_string(),
            0,
        );
        // Structured non-zero exit code.
        persist_run_event(
            Some(&fixture.run.id),
            "tool_end",
            &json!({"tool_name": "shell", "exit_code": 2, "text": "oops"}).to_string(),
            1,
        );
        assert!(artifacts(&fixture).is_empty());
    }

    #[test]
    fn write_and_edit_tool_ends_record_file_artifacts() {
        let fixture = fixture("persist-tool-artifacts");
        let inside = touch(&std::path::Path::new(&fixture.workspace.path).join("report.md"));

        // Path from the tool_start tool_args (structured, preferred).
        persist_run_event(
            Some(&fixture.run.id),
            "tool_start",
            &json!({"tool_id": "tc-w", "tool_name": "write", "tool_args": json!({"path": inside}).to_string()})
                .to_string(),
            0,
        );
        persist_run_event(
            Some(&fixture.run.id),
            "tool_end",
            &json!({"tool_name": "write", "tool_id": "tc-w", "text": "Written to placeholder"})
                .to_string(),
            1,
        );

        // Path from the structured target_path on tool_end.
        let edit_path = touch(&std::path::Path::new(&fixture.workspace.path).join("notes.txt"));
        persist_run_event(
            Some(&fixture.run.id),
            "tool_end",
            &json!({"toolName": "edit", "targetPath": edit_path, "text": "Edited notes.txt"})
                .to_string(),
            2,
        );

        // Path from the success prose (last-resort fallback), tool_call_id
        // synthesized from run + sequence.
        let prose_path = touch(&std::path::Path::new(&fixture.workspace.path).join("prose.md"));
        persist_run_event(
            Some(&fixture.run.id),
            "tool_result",
            &json!({"tool_name": "write", "text": format!("Written to {prose_path}")}).to_string(),
            3,
        );

        let rows = artifacts(&fixture);
        let paths: Vec<_> = rows.iter().filter_map(|a| a.path.clone()).collect();
        assert!(paths.contains(&inside), "paths: {paths:?}");
        assert!(paths.contains(&edit_path), "paths: {paths:?}");
        assert!(paths.contains(&prose_path), "paths: {paths:?}");
        let written = rows
            .iter()
            .find(|a| a.path.as_deref() == Some(inside.as_str()))
            .expect("write artifact");
        assert_eq!(written.summary.as_deref(), Some("Written by Agent."));
        assert_eq!(written.title, "report.md");
        let edited = rows
            .iter()
            .find(|a| a.path.as_deref() == Some(edit_path.as_str()))
            .expect("edit artifact");
        assert_eq!(edited.summary.as_deref(), Some("Edited by Agent."));
    }

    #[test]
    fn tool_end_guards_drop_unrecordable_artifacts() {
        let fixture = fixture("persist-tool-guards");

        // Non write/edit tools never record file artifacts.
        persist_run_event(
            Some(&fixture.run.id),
            "tool_end",
            &json!({"tool_name": "shell", "text": "done"}).to_string(),
            0,
        );

        // No path recoverable from input, target, or prose.
        persist_run_event(
            Some(&fixture.run.id),
            "tool_end",
            &json!({"tool_name": "write", "tool_id": "tc-none", "text": "wrote something"})
                .to_string(),
            1,
        );

        // A path outside the workspace is refused.
        persist_run_event(
            Some(&fixture.run.id),
            "tool_end",
            &json!({"tool_name": "write", "target_path": "/etc/passwd"}).to_string(),
            2,
        );

        assert!(artifacts(&fixture).is_empty());
    }

    #[test]
    fn git_workspaces_opt_out_of_artifacts() {
        let home = TestHome::new("persist-git-ws");
        let workspace = seed_workspace(home.path(), "gitws");
        // A real git repo (is_git_workspace shells out to git rev-parse).
        let status = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&workspace.path)
            .status()
            .expect("git init");
        assert!(status.success());
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);

        persist_run_event(
            Some(&run.id),
            "artifact_created",
            &json!({"title": "Doc", "path": format!("{}/a.md", workspace.path)}).to_string(),
            0,
        );
        persist_run_event(
            Some(&run.id),
            "tool_end",
            &json!({"tool_name": "write", "target_path": format!("{}/b.md", workspace.path)})
                .to_string(),
            1,
        );
        assert!(
            crate::store::list_artifacts(&thread.id)
                .expect("artifacts")
                .is_empty(),
            "git workspaces route changes through review, not artifacts"
        );
    }

    #[test]
    fn artifact_created_event_field_mapping() {
        let fixture = fixture("persist-artifact");

        // Full mapping with an inline artifact (no path → always allowed).
        persist_run_event(
            Some(&fixture.run.id),
            "artifact_created",
            &json!({
                "name": "Report",
                "artifactType": "markdown",
                "content": "body",
                "description": "a report"
            })
            .to_string(),
            0,
        );
        // Defaults + file artifact inside the workspace.
        persist_run_event(
            Some(&fixture.run.id),
            "artifact.created",
            &json!({"filePath": touch(&std::path::Path::new(&fixture.workspace.path).join("x.bin"))}).to_string(),
            1,
        );

        let rows = artifacts(&fixture);
        assert_eq!(rows.len(), 2, "rows: {rows:?}");
        let inline = rows.iter().find(|a| a.title == "Report").expect("inline");
        assert_eq!(inline.artifact_type, "markdown");
        assert_eq!(inline.content_storage.as_deref(), Some("inline"));
        assert_eq!(inline.summary.as_deref(), Some("a report"));
        let file = rows
            .iter()
            .find(|a| a.title == "Artifact")
            .expect("default title");
        assert_eq!(file.artifact_type, "document");
        assert_eq!(file.content_storage.as_deref(), Some("file"));

        // Unknown run: the workspace check fails and the artifact is dropped.
        persist_run_event(
            Some("run-missing"),
            "artifact_created",
            &json!({"title": "Orphan"}).to_string(),
            2,
        );
        assert_eq!(artifacts(&fixture).len(), 2);
    }

    #[test]
    fn soft_fail_exit_one_from_grep_is_not_a_failure() {
        let fixture = fixture("persist-soft-fail");
        let inside = touch(&std::path::Path::new(&fixture.workspace.path).join("out.txt"));
        // grep exits 1 on "no match": not a failure → write artifact recorded.
        persist_run_event(
            Some(&fixture.run.id),
            "tool_start",
            &json!({"tool_id": "tc-g", "tool_name": "write", "tool_args": json!({"path": inside, "command": "grep needle file"}).to_string()})
                .to_string(),
            0,
        );
        persist_run_event(
            Some(&fixture.run.id),
            "tool_end",
            &json!({"tool_name": "write", "tool_id": "tc-g", "text": "nothing\n[exit: 1]"})
                .to_string(),
            1,
        );
        let paths: Vec<_> = artifacts(&fixture)
            .iter()
            .filter_map(|a| a.path.clone())
            .collect();
        assert!(paths.contains(&inside), "paths: {paths:?}");

        // Same exit-1 footer from a non-exempt command IS a failure.
        persist_run_event(
            Some(&fixture.run.id),
            "tool_start",
            &json!({"tool_id": "tc-m", "tool_name": "write", "tool_args": json!({"path": inside, "command": "make build"}).to_string()})
                .to_string(),
            2,
        );
        persist_run_event(
            Some(&fixture.run.id),
            "tool_end",
            &json!({"tool_name": "write", "tool_id": "tc-m", "text": "failed\n[exit: 1]"})
                .to_string(),
            3,
        );
        let count = artifacts(&fixture)
            .iter()
            .filter(|a| a.path.as_deref() == Some(inside.as_str()))
            .count();
        assert_eq!(count, 1, "the failing tool_end added no second artifact");
    }

    #[test]
    fn tool_input_unwraps_double_encoded_json() {
        let fixture = fixture("persist-double-encoded");
        let inside = touch(&std::path::Path::new(&fixture.workspace.path).join("deep.md"));
        // tool_args stored as a string whose value is itself a JSON string.
        let inner = json!({"path": inside}).to_string();
        let double = serde_json::Value::String(inner).to_string();
        persist_run_event(
            Some(&fixture.run.id),
            "tool_start",
            &json!({"tool_id": "tc-d", "tool_name": "write", "tool_args": double}).to_string(),
            0,
        );
        persist_run_event(
            Some(&fixture.run.id),
            "tool_end",
            &json!({"tool_name": "write", "tool_id": "tc-d", "text": "Written"}).to_string(),
            1,
        );
        let paths: Vec<_> = artifacts(&fixture)
            .iter()
            .filter_map(|a| a.path.clone())
            .collect();
        assert!(paths.contains(&inside), "paths: {paths:?}");

        // Garbage stored input: parse fails, no artifact, no panic.
        persist_run_event(
            Some(&fixture.run.id),
            "tool_start",
            &json!({"tool_id": "tc-bad", "tool_name": "write", "tool_args": "{not json"})
                .to_string(),
            2,
        );
        persist_run_event(
            Some(&fixture.run.id),
            "tool_end",
            &json!({"tool_name": "write", "tool_id": "tc-bad", "text": "Written"}).to_string(),
            3,
        );
    }

    #[test]
    fn tool_input_string_layer_that_is_not_json_is_dropped() {
        let fixture = fixture("persist-string-layer");
        // tool_args is a JSON string whose content is NOT an object: the
        // double-unwrap bails on the inner parse.
        let not_an_object = serde_json::Value::String("plain text".to_string()).to_string();
        persist_run_event(
            Some(&fixture.run.id),
            "tool_start",
            &json!({"tool_id": "tc-s", "tool_name": "write", "tool_args": not_an_object})
                .to_string(),
            0,
        );
        persist_run_event(
            Some(&fixture.run.id),
            "tool_end",
            &json!({"tool_name": "write", "tool_id": "tc-s", "text": "Written"}).to_string(),
            1,
        );
        assert!(artifacts(&fixture).is_empty());
    }

    #[test]
    fn shell_command_double_encoded_is_unwrapped_for_soft_fail() {
        let fixture = fixture("persist-shell-double");
        let inside = touch(&std::path::Path::new(&fixture.workspace.path).join("out.txt"));
        // tool_args double-encoded: the shell command lives two string layers
        // deep, so `shell_command_from_input` must unwrap twice to read it.
        let inner = json!({"path": inside, "command": "grep needle file"}).to_string();
        let double = serde_json::Value::String(inner).to_string();
        persist_run_event(
            Some(&fixture.run.id),
            "tool_start",
            &json!({"tool_id": "tc-ds", "tool_name": "shell", "tool_args": double}).to_string(),
            0,
        );
        // grep exits 1 on "no match": a soft-fail, so the write artifact (from
        // the double-encoded path) is still recorded.
        persist_run_event(
            Some(&fixture.run.id),
            "tool_end",
            &json!({"tool_name": "write", "tool_id": "tc-ds", "text": "no match\n[exit: 1]"})
                .to_string(),
            1,
        );
        let paths: Vec<_> = artifacts(&fixture)
            .iter()
            .filter_map(|a| a.path.clone())
            .collect();
        assert!(paths.contains(&inside), "paths: {paths:?}");
    }

    #[test]
    fn footer_exit_codes_above_one_are_failures() {
        let fixture = fixture("persist-footer-fail");
        persist_run_event(
            Some(&fixture.run.id),
            "tool_end",
            &json!({"tool_name": "shell", "tool_id": "tc-f", "text": "boom\n[exit: 2]"})
                .to_string(),
            0,
        );
        assert!(artifacts(&fixture).is_empty());
    }

    #[test]
    fn whitespace_only_command_is_not_soft_fail() {
        assert!(!super::is_soft_fail_command(Some("   ")));
        assert!(!super::is_soft_fail_command(Some("")));
    }

    #[test]
    fn approval_persistence_failures_are_logged_not_raised() {
        let fixture = fixture("persist-appr-errors");

        // FK violation: the run does not exist, so the insert fails; the
        // run-status CAS then matches nothing and the follow-up cancellation
        // of the never-created row fails too. Everything is logged, nothing
        // propagated.
        persist_run_event(
            Some("run-missing"),
            "approval_request",
            &json!({"approval_request_id": "appr-fk"}).to_string(),
            0,
        );
        assert!(crate::store::get_approval_request("appr-fk")
            .expect("query")
            .is_none());

        // A decision for a never-persisted approval fails to record; the
        // default "cancelled" status then fails its run-status CAS as well.
        persist_run_event(
            Some("run-missing"),
            "approval_decision",
            &json!({"approval_request_id": "appr-fk"}).to_string(),
            1,
        );
        persist_run_event(
            Some("run-missing"),
            "approval_decision",
            &json!({"approval_request_id": "appr-fk", "status": "approved"}).to_string(),
            2,
        );

        // With the store fully unreadable every best-effort write logs and
        // returns: approval request/decision, artifact, and tool artifact.
        let prev = super::super::test_support::break_home();
        persist_run_event(
            Some(&fixture.run.id),
            "approval_request",
            &json!({"approval_request_id": "appr-broken"}).to_string(),
            3,
        );
        persist_run_event(
            Some(&fixture.run.id),
            "approval_decision",
            &json!({"approval_request_id": "appr-broken", "status": "cancelled"}).to_string(),
            4,
        );
        persist_run_event(
            Some(&fixture.run.id),
            "approval_decision",
            &json!({"approval_request_id": "appr-broken", "status": "approved"}).to_string(),
            5,
        );
        persist_run_event(
            Some(&fixture.run.id),
            "tool_end",
            &json!({"tool_name": "write", "target_path": "/tmp/x.md"}).to_string(),
            6,
        );
        super::super::test_support::restore_home(prev);
    }

    #[test]
    fn artifact_write_failures_are_logged_not_raised() {
        let fixture = fixture("persist-artifact-locked");
        let inside = touch(&std::path::Path::new(&fixture.workspace.path).join("locked.md"));

        // Hold the write lock from a second connection: reads (the workspace
        // allow check) still work under WAL, but the INSERT times out and is
        // only logged.
        let mut conn = rusqlite::Connection::open(fixture._home.path().join(".future/app/app.db"))
            .expect("open db");
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Exclusive)
            .expect("exclusive lock");

        persist_run_event(
            Some(&fixture.run.id),
            "tool_end",
            &json!({"tool_name": "write", "target_path": inside}).to_string(),
            0,
        );
        persist_run_event(
            Some(&fixture.run.id),
            "artifact_created",
            &json!({"title": "Inline", "content": "body"}).to_string(),
            1,
        );
        tx.rollback().expect("rollback");

        assert!(
            artifacts(&fixture).is_empty(),
            "locked-out writes record nothing"
        );
    }
}
