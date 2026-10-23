//! Recovery reads the authoritative transcript, never its healed placeholders.
//! Unknown side effects fence all further mutations; a model assertion cannot
//! release the fence. Complete file writes can be verified without redoing them.
use super::prompt_helpers::prepare_session_tool_call;
use crate::session::{Manager, SessionEntry, ENTRY_TYPE_RUN_STARTED, ENTRY_TYPE_TOOL};
use crate::types::ToolCallResult;
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct RecoverySafety {
    unresolved: Vec<String>,
    completed: HashSet<String>,
    cwd: String,
    pub(super) history_changed: bool,
}

impl RecoverySafety {
    pub(super) fn load(
        manager: &Manager,
        session: &str,
        cwd: &str,
        automatic: bool,
    ) -> Result<Self> {
        if !manager.contains(session)? {
            return Ok(Self::default());
        }
        let entries = manager.load_unrepaired_entries(session)?;
        let mut calls = HashMap::new();
        let mut results = HashMap::new();
        let mut run = String::new();
        let mut execution_cwd = cwd.to_owned();
        let mut parents = HashMap::new();
        let mut terminals = HashMap::new();
        for entry in &entries {
            if entry.entry_type == crate::session::ENTRY_TYPE_RUN_TERMINAL {
                if let Some(content) = &entry.content {
                    if let (Some(run), Some(state)) =
                        (content["run_id"].as_str(), content["state"].as_str())
                    {
                        terminals.insert(run.to_owned(), state.to_owned());
                    }
                }
            }
            if entry.entry_type == crate::session::ENTRY_TYPE_SESSION_INFO {
                if let Some(path) = entry.content.as_ref().and_then(|v| v["cwd"].as_str()) {
                    execution_cwd = path.to_owned();
                }
            }
            if entry.entry_type == ENTRY_TYPE_RUN_STARTED {
                run = entry
                    .content
                    .as_ref()
                    .and_then(|v| v["run_id"].as_str())
                    .unwrap_or_default()
                    .to_owned();
                if let Some(parent) = entry
                    .content
                    .as_ref()
                    .and_then(|v| v["auto_resume_of"].as_str())
                {
                    parents.insert(run.clone(), parent.to_owned());
                }
            }
            let owner = entry
                .meta
                .as_ref()
                .and_then(|v| v["run_id"].as_str())
                .unwrap_or(&run)
                .to_owned();
            if entry.entry_type == ENTRY_TYPE_TOOL {
                results.insert(
                    (owner.clone(), entry.tool_call_id.clone()),
                    result_state(entry),
                );
            }
            for call in &entry.tool_calls {
                let call_cwd = entry
                    .meta
                    .as_ref()
                    .and_then(|m| m["execution_cwd"].as_str())
                    .unwrap_or(&execution_cwd);
                calls.insert(
                    (owner.clone(), call.id.clone()),
                    (call.clone(), call_cwd.to_owned()),
                );
            }
        }
        let mut recovered_runs = HashSet::new();
        let mut candidate = if automatic {
            manager.resumable_run_id(session)?
        } else {
            None
        };
        while let Some(run) = candidate {
            if !recovered_runs.insert(run.clone()) {
                break;
            }
            candidate = parents.get(&run).cloned();
        }
        let mut safety = Self {
            cwd: cwd.to_owned(),
            ..Self::default()
        };
        let mut verified = Vec::new();
        for ((run, id), (call, call_cwd)) in calls {
            let name = &call.function.name;
            if name == "read" {
                continue;
            }
            let args = prepare_session_tool_call(&call_cwd, name, &call.function.arguments);
            let key = fingerprint(name, &args, &call_cwd)?;
            // A timeout already handled within a settled successful task is
            // not a new recovery incident. Missing results are never excused.
            let handled = results.contains_key(&(run.clone(), id.clone()))
                && terminals
                    .get(&run)
                    .is_some_and(|state| state == crate::session::RUN_STATE_COMPLETED);
            if let Some(Some(completed)) = results.get(&(run.clone(), id.clone())) {
                if *completed && recovered_runs.contains(&run) {
                    safety.completed.insert(key);
                }
            } else if handled {
                continue;
            } else if name == "write" && write_is_applied(&args) {
                // This is an observation of the desired complete file, not a
                // claim that the old process successfully returned. Persist it
                // so later starts and clients see the same verified fact.
                let mut result = SessionEntry::new_tool(&id,
                    "Recovery verified that the file already contains the complete requested content. No write was repeated.");
                result.name = name.clone();
                result.tool_args = serde_json::to_string(&args)?;
                result.meta = Some(json!({"run_id":run,"recovery_verified":true}));
                verified.push(result);
                if recovered_runs.contains(&run) {
                    safety.completed.insert(key);
                }
            } else {
                safety.unresolved.push(format!("{name} ({id})"));
            }
        }
        if !verified.is_empty() {
            manager.append_entries_synced(session, &verified)?;
            safety.history_changed = true;
        }
        safety.unresolved.sort();
        Ok(safety)
    }

    pub(super) fn pause_reason(&self) -> Option<String> {
        (!self.unresolved.is_empty()).then(|| format!(
            "[RECOVERY_UNCERTAIN] The previous operation's effect could not be verified: {}. Work is preserved; no uncertain operation was repeated.",
            self.unresolved.join(", ")))
    }

    pub(super) fn before_tool(&self, name: &str, args: &Value) -> Option<ToolCallResult> {
        if name == "read" {
            return None;
        }
        let reason = self.pause_reason().or_else(|| match fingerprint(name, args, &self.cwd) {
            Ok(key) if self.completed.contains(&key) => Some(
                "[RECOVERY_RECORDED_OPERATION] This operation already has a recorded result in the interrupted task. It was not executed again. Inspect the existing result (which may be a failure); do not treat it as a fresh execution.".to_owned()),
            Ok(_) => None,
            Err(error) => Some(format!("[RECOVERY_UNCERTAIN] Cannot identify the operation safely: {error}")),
        });
        reason.map(|result| ToolCallResult {
            result,
            is_error: true,
        })
    }
}

fn fingerprint(name: &str, args: &Value, cwd: &str) -> Result<String> {
    // Timeout, justification, permissions and JSON field order do not turn the
    // same action into a new one. File paths have already been normalized.
    let effect = match name {
        "shell" => json!({"command":args["command"].as_str().map(str::trim),"cwd":cwd}),
        "write" => json!({"path":args["path"],"content":args["content"]}),
        "edit" => {
            let edits = args["edits"]
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![args.clone()]);
            let edits: Vec<_> = edits.iter().map(|edit| json!({
                "oldText":edit.get("oldText").or_else(|| edit.get("old_text")).or_else(|| edit.get("old_string")),
                "newText":edit.get("newText").or_else(|| edit.get("new_text")).or_else(|| edit.get("new_string")),
            })).collect();
            json!({"path":args["path"],"edits":edits})
        }
        _ => args.clone(),
    };
    crate::session::compaction_ops::canonical_hash(&json!({"tool":name,"effect":effect}))
}

fn write_is_applied(args: &Value) -> bool {
    let (Some(path), Some(content)) = (args["path"].as_str(), args["content"].as_str()) else {
        return false;
    };
    // Bound verification I/O to the expected output, including on corrupt or
    // externally replaced paths. No shell command is run as a "probe".
    use std::io::Read;
    std::fs::File::open(path).is_ok_and(|file| {
        if !file
            .metadata()
            .is_ok_and(|m| m.is_file() && m.len() == content.len() as u64)
        {
            return false;
        }
        let mut bytes = Vec::new();
        file.take(content.len() as u64 + 1)
            .read_to_end(&mut bytes)
            .is_ok()
            && bytes == content.as_bytes()
    })
}

/// Some(true): recorded outcome; Some(false): not executed; None: partial/unknown.
fn result_state(entry: &SessionEntry) -> Option<bool> {
    if entry
        .meta
        .as_ref()
        .is_some_and(|m| m["recovery_verified"] == true)
    {
        return Some(true);
    }
    let mut text = String::new();
    if let Some(content) = &entry.content {
        match content {
            Value::String(s) => text.push_str(s),
            Value::Array(blocks) => {
                for block in blocks {
                    text.push_str(
                        block["content"]
                            .as_str()
                            .or_else(|| block["text"].as_str())
                            .unwrap_or_default(),
                    );
                }
            }
            _ => return None,
        }
    } else {
        return None;
    }
    if text.starts_with("[RECOVERY_") || text.starts_with("[Tool execution cancelled —") {
        return Some(false);
    }
    if text.starts_with("[Tool execution lost") {
        return None;
    }
    if let Some(facts) = entry.meta.as_ref().and_then(|m| m.get("shell_result")) {
        return match facts["status"].as_str() {
            Some("not_started" | "launch_failed")
                if facts["attempts"].as_array().is_none_or(|attempts| {
                    attempts.iter().all(|a| {
                        matches!(a["status"].as_str(), Some("not_started" | "launch_failed"))
                    })
                }) =>
            {
                Some(false)
            }
            Some("exited") => Some(true),
            _ => None,
        };
    }
    Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;
    use crate::types::{ToolCall, ToolCallFn};

    fn fixture(
        name: &str,
        arguments: Value,
        result: Option<SessionEntry>,
    ) -> (tempfile::TempDir, Manager) {
        let root = tempfile::tempdir().unwrap();
        let manager = Manager::new(root.path().join("sessions"));
        let mut entries = vec![
            SessionEntry::new_user("user", json!("finish my work")),
            SessionEntry::run_started("run", 1),
            SessionEntry::new_assistant(
                json!("working"),
                vec![ToolCall {
                    id: "call".into(),
                    call_type: "function".into(),
                    function: ToolCallFn {
                        name: name.into(),
                        arguments,
                    },
                }],
            ),
        ];
        entries.extend(result);
        manager
            .save(&Session::snapshot(
                "session".into(),
                root.path().to_string_lossy().into_owned(),
                "mock".into(),
                String::new(),
                String::new(),
                entries,
            ))
            .unwrap();
        (root, manager)
    }

    #[test]
    fn unknown_shell_is_never_executed_and_manual_prompt_cannot_bypass_it() {
        let (root, manager) = fixture("shell", json!({"command":"append-once"}), None);
        // Loading history has already manufactured a placeholder; it cannot
        // turn uncertainty into a real tool result.
        manager.load("session").unwrap();
        for automatic in [true, false] {
            let safety = RecoverySafety::load(
                &manager,
                "session",
                root.path().to_str().unwrap(),
                automatic,
            )
            .unwrap();
            assert!(safety.pause_reason().is_some());
            assert!(safety
                .before_tool("read", &json!({"path":"result"}))
                .is_none());
            for name in ["shell", "write", "edit"] {
                assert!(safety.before_tool(name, &json!({})).unwrap().is_error);
            }
        }
    }

    #[test]
    fn already_applied_write_is_verified_and_persisted_without_writing_again() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "complete content").unwrap();
        let args = json!({"path":file.path(),"content":"complete content"});
        let (root, manager) = fixture("write", args.clone(), None);
        let safety =
            RecoverySafety::load(&manager, "session", root.path().to_str().unwrap(), true).unwrap();
        assert!(safety.pause_reason().is_none());
        assert!(safety.before_tool("write", &args).unwrap().is_error);
        let stored = manager.load_unrepaired_entries("session").unwrap();
        assert!(stored.iter().any(|e| e
            .meta
            .as_ref()
            .is_some_and(|m| m["recovery_verified"] == true)));
        assert_eq!(
            std::fs::read_to_string(file.path()).unwrap(),
            "complete content"
        );
        let second =
            RecoverySafety::load(&manager, "session", root.path().to_str().unwrap(), true).unwrap();
        assert!(second.pause_reason().is_none());
        assert_eq!(
            manager.load_unrepaired_entries("session").unwrap().len(),
            stored.len()
        );
    }

    #[test]
    fn mismatched_write_and_unverifiable_edit_remain_held() {
        for (name, args) in [
            ("write", json!({"path":"missing","content":"new"})),
            (
                "edit",
                json!({"path":"missing","oldText":"old","newText":"new"}),
            ),
        ] {
            let (root, manager) = fixture(name, args, None);
            assert!(
                RecoverySafety::load(&manager, "session", root.path().to_str().unwrap(), true)
                    .unwrap()
                    .pause_reason()
                    .is_some()
            );
        }
    }

    #[test]
    fn completed_action_is_fenced_during_automatic_recovery_only() {
        let args = json!({"command":"append-once"});
        let (root, manager) = fixture(
            "shell",
            args.clone(),
            Some(SessionEntry::new_tool("call", "done")),
        );
        let cwd = root.path().to_str().unwrap();
        let safety = RecoverySafety::load(&manager, "session", cwd, true).unwrap();
        assert!(safety.pause_reason().is_none());
        assert!(safety
            .before_tool("shell", &args)
            .unwrap()
            .result
            .contains("RECORDED_OPERATION"));
        assert!(safety
            .before_tool(
                "shell",
                &json!({"command":"append-once","timeout":60,"justification":"recover"})
            )
            .is_some());
        assert!(RecoverySafety::load(&manager, "session", cwd, false)
            .unwrap()
            .before_tool("shell", &args)
            .is_none());
    }

    #[test]
    fn cancelled_or_timed_out_process_can_have_partial_effects() {
        for status in ["cancelled", "timed_out", "execution_failed"] {
            let mut result = SessionEntry::new_tool("call", "command interrupted");
            result.meta = Some(json!({"shell_result":{"status":status,"is_error":true}}));
            let (root, manager) = fixture("shell", json!({"command":"append-once"}), Some(result));
            assert!(
                RecoverySafety::load(&manager, "session", root.path().to_str().unwrap(), true)
                    .unwrap()
                    .pause_reason()
                    .is_some()
            );
        }
    }

    #[test]
    fn call_ids_are_scoped_to_their_original_run() {
        let (root, manager) = fixture("shell", json!({"command":"append-once"}), None);
        manager
            .append_entries_synced(
                "session",
                &[
                    SessionEntry::run_started("other", 2),
                    SessionEntry::new_tool("call", "a different run's result"),
                ],
            )
            .unwrap();
        assert!(
            RecoverySafety::load(&manager, "session", root.path().to_str().unwrap(), true)
                .unwrap()
                .pause_reason()
                .is_some()
        );
    }

    #[test]
    fn queued_user_input_and_later_continuations_do_not_forget_completed_steps() {
        let args = json!({"command":"append-once"});
        let (root, manager) = fixture(
            "shell",
            args.clone(),
            Some(SessionEntry::new_tool("call", "done")),
        );
        manager
            .append_entries_synced(
                "session",
                &[
                    SessionEntry::run_terminal(
                        "run",
                        crate::session::RUN_STATE_INTERRUPTED_BY_RESTART,
                        0,
                        0,
                        None,
                    ),
                    SessionEntry::run_started_continuation("continued", 2, Some(2), "run"),
                    SessionEntry::new_user("user", json!("an unexecuted queued follow-up")),
                ],
            )
            .unwrap();
        let safety =
            RecoverySafety::load(&manager, "session", root.path().to_str().unwrap(), true).unwrap();
        assert!(safety.before_tool("shell", &args).is_some());
    }

    #[test]
    fn handled_tool_failures_do_not_quarantine_later_work() {
        for status in ["exited", "timed_out"] {
            let mut result = SessionEntry::new_tool("call", "a handled error");
            result.meta =
                Some(json!({"shell_result":{"status":status,"is_error":true,"exit_code":1}}));
            let (root, manager) = fixture("shell", json!({"command":"check"}), Some(result));
            manager
                .append_entries_synced(
                    "session",
                    &[SessionEntry::run_terminal(
                        "run",
                        crate::session::RUN_STATE_COMPLETED,
                        0,
                        0,
                        None,
                    )],
                )
                .unwrap();
            assert!(RecoverySafety::load(
                &manager,
                "session",
                root.path().to_str().unwrap(),
                false
            )
            .unwrap()
            .pause_reason()
            .is_none());
        }
    }
}
