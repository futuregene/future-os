//! Recovery reads the authoritative transcript, never its healed placeholders.
//! Unknown side effects fence all further mutations; a model assertion cannot
//! release the fence. Complete file writes can be verified without redoing them,
//! and a shell command whose text provably cannot change anything observable is
//! not an unknown effect at all. The decision is made from the command text,
//! never from what the model claimed about it.
use super::prompt_helpers::prepare_session_tool_call;
use crate::session::{Manager, SessionEntry, ENTRY_TYPE_RUN_STARTED, ENTRY_TYPE_TOOL};
use crate::types::ToolCallResult;
use anyhow::Result;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct RecoverySafety {
    unresolved: Vec<String>,
    completed: HashSet<String>,
    /// Fingerprints of calls the user aborted while they were in flight. Their
    /// effect is unknown, but only that one operation is questioned: fencing
    /// the whole session for the user's own stop is not a safety property.
    aborted: HashSet<String>,
    /// Operations already warned about in this run. A second attempt of the
    /// same command is the model acting on the warning, not a new incident.
    warned: Mutex<HashSet<String>>,
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
            let recorded = results.get(&(run.clone(), id.clone()));
            // The user ending this run is the only state that makes a missing
            // result an abort rather than a crash. A system shutdown or a kill
            // lands on `interrupted_by_restart` and keeps fencing.
            let user_stopped = terminals
                .get(&run)
                .is_some_and(|state| state == crate::session::RUN_STATE_CANCELLED);
            // A timeout already handled within a settled successful task is
            // not a new recovery incident. Missing results are never excused.
            let handled = recorded.is_some()
                && terminals
                    .get(&run)
                    .is_some_and(|state| state == crate::session::RUN_STATE_COMPLETED);
            let doubt = match recorded {
                Some(ResultState::Recorded) => {
                    if recovered_runs.contains(&run) {
                        safety.completed.insert(key.clone());
                    }
                    Doubt::Settled
                }
                Some(ResultState::NotExecuted) => Doubt::Settled,
                // The user pressed stop in this run. The effect of that one
                // operation is genuinely unknown, but the session's own
                // liveness is not in question.
                Some(ResultState::Aborted) => Doubt::Aborted,
                // No result at all in a run the user ended: the process that
                // would have written it was stopped, which is the same
                // question — not the crash this fence exists for.
                None if user_stopped => Doubt::Aborted,
                Some(ResultState::Unknown) | None if handled => Doubt::Settled,
                Some(ResultState::Unknown) | None => Doubt::Unknown,
            };
            if doubt == Doubt::Settled {
                continue;
            }
            if name == "shell" && args["command"].as_str().is_some_and(command_is_read_only) {
                // Provably read-only: no side effect can be waiting, so this
                // call does not fence the session — not even as an aborted
                // operation. Anything it cannot prove stays questioned below.
                tracing::debug!(
                    tool_call_id = %id,
                    "recovery: interrupted read-only shell is not fenced"
                );
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
            } else if doubt == Doubt::Aborted {
                // Questioned once, on its own, by `before_tool`.
                safety.aborted.insert(key);
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
            Ok(key) if self.aborted.contains(&key) => self.aborted_once(&key),
            Ok(_) => None,
            Err(error) => Some(format!("[RECOVERY_UNCERTAIN] Cannot identify the operation safely: {error}")),
        });
        reason.map(|result| ToolCallResult {
            result,
            is_error: true,
        })
    }

    /// Warns once per run about the operation the user aborted, then leaves the
    /// decision to the model: fencing the same command forever would trade a
    /// session-wide deadlock for a per-command one.
    fn aborted_once(&self, key: &str) -> Option<String> {
        if !self.warned.lock().insert(key.to_owned()) {
            return None;
        }
        Some("[RECOVERY_ABORTED_OPERATION] The previous run was stopped while this operation was in flight, so whether it took effect is unknown. Check the result with a read-only command first, and re-run it only if that check shows it did not happen.".to_owned())
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

/// A shell command whose every segment is a whitelisted read-only program.
/// Returns false on anything it cannot prove: the fence is the safe default.
fn command_is_read_only(command: &str) -> bool {
    let Some(segments) = split_command_segments(command) else {
        return false;
    };
    segments.iter().all(|segment| segment_is_read_only(segment))
}

/// Split on unquoted `;`, `&&`, `||`, `|` and newlines. `None` means the text
/// holds something this predicate cannot reason about at all: command
/// substitution, grouping, a stop-parsing token, any redirection, or an
/// unbalanced quote.
fn split_command_segments(command: &str) -> Option<Vec<String>> {
    if command
        .chars()
        .any(|c| matches!(c, '`' | '>' | '<' | '(' | ')' | '{' | '}'))
    {
        return None;
    }
    if command.contains("--%") {
        return None;
    }
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(open) = quote {
            current.push(c);
            if c == open {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                current.push(c);
            }
            // `&&` chains; a lone `&` is the call/background operator.
            '&' if chars.peek() == Some(&'&') => {
                chars.next();
                segments.push(std::mem::take(&mut current));
            }
            '&' => return None,
            '|' => {
                if chars.peek() == Some(&'|') {
                    chars.next();
                }
                segments.push(std::mem::take(&mut current));
            }
            ';' | '\n' | '\r' => segments.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    if quote.is_some() {
        return None;
    }
    segments.push(current);
    let segments: Vec<String> = segments
        .into_iter()
        .map(|segment| segment.trim().to_owned())
        .filter(|segment| !segment.is_empty())
        .collect();
    (!segments.is_empty()).then_some(segments)
}

/// Whitespace-split honouring quotes, so a quoted separator stays inside its
/// argument; `None` on an unbalanced quote or a segment with no token.
fn command_tokens(segment: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    for c in segment.chars() {
        if let Some(open) = quote {
            started = true;
            if c == open {
                quote = None;
            } else {
                current.push(c);
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                started = true;
            }
            c if c.is_whitespace() => {
                if started {
                    tokens.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            _ => {
                started = true;
                current.push(c);
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if started {
        tokens.push(current);
    }
    (!tokens.is_empty()).then_some(tokens)
}

fn segment_is_read_only(segment: &str) -> bool {
    // A leading `$` or `@` assigns shell state (or splats arguments); neither is
    // a command whose effect can be reasoned about.
    if segment.starts_with('$') || segment.starts_with('@') {
        return false;
    }
    let Some(tokens) = command_tokens(segment) else {
        return false;
    };
    let Some((program, args)) = tokens.split_first() else {
        return false;
    };
    match program.to_ascii_lowercase().as_str() {
        // `cd` only moves this process: no observable effect can be waiting,
        // and repeating it cannot apply anything twice, so it is harmless in
        // any position of the chain. Its arguments still have to be plain
        // paths, because an expansion can hide another command.
        "cd" | "set-location" => cd_is_read_only(args),
        "git" => git_is_read_only(args),
        "command" => args
            .first()
            .is_some_and(|flag| flag.eq_ignore_ascii_case("-v")),
        // Programs with no write direction at all. Anything absent from this
        // list is false, including every interpreter (`bash -c`, `node -e`,
        // `python3 -c`), which can do whatever its argument says.
        "echo" | "write-output" | "get-childitem" | "get-content" | "get-date" | "get-location"
        | "get-process" | "test-path" | "select-string" | "select-object" | "measure-object"
        | "where-object" | "pwd" | "ls" | "cat" | "head" | "tail" | "wc" | "which" | "type" => true,
        _ => false,
    }
}

/// `cd` with literal path arguments only. A `$`-expansion, a substitution or a
/// script block could name anything, so those stay fenced.
fn cd_is_read_only(args: &[String]) -> bool {
    args.iter().all(|arg| {
        !arg.starts_with(['$', '@', '-']) && !arg.contains(['$', '@', '`', '(', ')', '{', '}'])
    })
}

/// `git` subcommands that only read, with the arguments that would still write
/// or launch a program rejected so the list stays a list of effects.
fn git_is_read_only(args: &[String]) -> bool {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "--no-pager" | "--no-optional-locks" | "--literal-pathspecs" => index += 1,
            // `-C <dir>` only changes the directory the read happens in.
            "-C" => index += 2,
            _ => break,
        }
    }
    let Some(subcommand) = args.get(index).map(|arg| arg.to_ascii_lowercase()) else {
        return false;
    };
    let rest = &args[index + 1..];
    if rest.iter().any(|arg| git_arg_writes(arg)) {
        return false;
    }
    match subcommand.as_str() {
        "status" | "log" | "diff" | "show" | "rev-parse" | "ls-files" => true,
        // A positional argument would create a branch; the mutating short and
        // long flags are refused explicitly.
        "branch" => {
            rest.iter().all(|arg| arg.starts_with('-'))
                && !rest.iter().any(|arg| branch_flag_creates_or_deletes(arg))
        }
        "worktree" => match rest.first().map(|arg| arg.to_ascii_lowercase()).as_deref() {
            None => true,
            Some("list") => rest[1..].iter().all(|arg| arg.starts_with('-')),
            Some(_) => false,
        },
        "remote" => matches!(
            rest,
            [flag] if matches!(flag.as_str(), "-v" | "-vv" | "--verbose")
        ),
        _ => false,
    }
}

fn git_arg_writes(arg: &str) -> bool {
    let lower = arg.to_ascii_lowercase();
    // `--output` writes a file; the external diff/textconv helpers run a
    // program chosen by configuration, and `--open-files-in-pager` spawns one.
    lower == "--output"
        || lower.starts_with("--output=")
        || lower.starts_with("--ext-diff")
        || lower.starts_with("--textconv")
        || lower.starts_with("--open-files-in-pager")
}

fn branch_flag_creates_or_deletes(arg: &str) -> bool {
    if let Some(long) = arg.strip_prefix("--") {
        let long = long.split('=').next().unwrap_or(long).to_ascii_lowercase();
        return matches!(
            long.as_str(),
            "delete"
                | "move"
                | "copy"
                | "force"
                | "set-upstream-to"
                | "unset-upstream"
                | "edit-description"
                | "track"
                | "no-track"
                | "create-reflog"
        );
    }
    arg.chars()
        .skip(1)
        .any(|c| matches!(c, 'd' | 'D' | 'm' | 'M' | 'c' | 'C' | 'f' | 'u' | 't'))
}

/// How much of a call the authoritative transcript can vouch for.
///
/// `Settled`: the outcome is recorded, or provably never ran. `Aborted`: the
/// user ended the run this call belonged to, so only that one operation is in
/// question. `Unknown`: nobody can say — the fence applies to the session.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Doubt {
    Settled,
    Aborted,
    Unknown,
}

/// Recorded: settled outcome; NotExecuted: provably never ran; Aborted: the
/// user stopped the run it belonged to; Unknown: partial or missing outcome.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ResultState {
    Recorded,
    NotExecuted,
    Aborted,
    Unknown,
}

fn result_state(entry: &SessionEntry) -> ResultState {
    if entry
        .meta
        .as_ref()
        .is_some_and(|m| m["recovery_verified"] == true)
    {
        return ResultState::Recorded;
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
            _ => return ResultState::Unknown,
        }
    } else {
        return ResultState::Unknown;
    }
    if text.starts_with("[RECOVERY_") || text.starts_with("[Tool execution cancelled —") {
        return ResultState::NotExecuted;
    }
    if text.starts_with("[Tool execution lost") {
        return ResultState::Unknown;
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
                ResultState::NotExecuted
            }
            Some("exited") => ResultState::Recorded,
            // An abort is neither a success nor a crash: claiming it ran
            // would be a lie, and treating it as an unknown effect would lock
            // the session for a stop the user asked for.
            Some("cancelled") => ResultState::Aborted,
            _ => ResultState::Unknown,
        };
    }
    ResultState::Recorded
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

    // `cancelled` used to sit in this loop, which made the user pressing stop
    // fence the whole session. It is an abort the user asked for, not an
    // unsupervised interruption, and is covered by
    // `user_aborted_shell_does_not_fence_the_whole_session` and
    // `aborted_operation_is_warned_once_then_allowed` instead.
    #[test]
    fn timed_out_or_failed_process_can_have_partial_effects() {
        for status in ["timed_out", "execution_failed"] {
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

    // ─── read-only classification ──────────────────────────────────────────

    #[test]
    fn read_only_classification_follows_the_whitelist() {
        let proven = [
            "git status",
            "git status --short",
            "git log --oneline -5",
            "git diff --stat",
            "git show HEAD",
            "git rev-parse --abbrev-ref HEAD",
            "git worktree list",
            "git worktree list --porcelain",
            "git branch",
            "git branch -a -v",
            "git branch --list",
            "git remote -v",
            "git ls-files",
            "git -C D:/future-os status",
            "git --no-pager log -1",
            "Get-ChildItem -Path .",
            "Get-Content -Path README.md -TotalCount 5",
            "Get-Location",
            "Get-Process",
            "Get-Date",
            "Test-Path D:/future-os",
            "Select-String -Pattern 'git worktree' -Path agent/src/rpc/*.rs",
            "Get-ChildItem | Select-Object -First 5",
            "Get-Process | Where-Object ProcessName",
            "Get-ChildItem | Measure-Object",
            "echo hi",
            "Write-Output hi",
            "cd ..",
            // `cd` only moves the process, so its position in the chain is
            // irrelevant — that is tonight's third incident.
            "cd D:/future-os; git worktree list; git status --short",
            "cd x; git status",
            "cd ..; git status --short",
            "cd ..; cd ..; git status",
            "cd \"D:/a b\"; git status",
            "Set-Location ..; git status",
            "git status; cd ..",
            "pwd",
            "ls",
            "cat README.md",
            "head -5 README.md",
            "tail -5 README.md",
            "wc -l README.md",
            "which git",
            "command -v git",
            "type git",
            "git status | Select-String churn",
            "git status; git log --oneline -3",
            "git status && echo done",
            "git status\n\ngit worktree list",
        ];
        for command in proven {
            assert!(
                command_is_read_only(command),
                "expected read-only: {command}"
            );
        }

        let fenced = [
            "",
            "   ",
            // Anything not on the list, including every write direction.
            "rm -rf build",
            "Remove-Item -Recurse -Force build",
            "Set-Content -Path out.txt -Value hi",
            "Add-Content out.txt hi",
            "New-Item out.txt",
            "Move-Item a b",
            "Copy-Item a b",
            "Start-Process notepad",
            "Invoke-WebRequest http://example.com",
            "Out-File out.txt",
            "cargo build",
            "npm install",
            "git commit -m x",
            "git push",
            "git checkout main",
            "git reset --hard",
            "git merge dev",
            "git worktree add ../other",
            "git branch -D old",
            "git branch new-branch",
            "git remote add origin url",
            "git diff --output=patch.diff",
            "curl http://example.com",
            "wget http://example.com",
            // Interpreters execute whatever their argument says.
            "bash -c \"rm x\"",
            "sh -c \"rm x\"",
            "node -e \"1\"",
            "python3 -c \"print(1)\"",
            "python -c \"print(1)\"",
            // Read-only-looking disguises: redirection and writing pipes.
            "echo hi > f",
            "echo hi >> f",
            "Get-ChildItem 2> err.txt",
            "Get-ChildItem *> all.txt",
            "cat < README.md",
            "git status > status.txt",
            "git status | tee f",
            "Get-Content a.txt | Set-Content b.txt",
            // A chain is only as read-only as its worst segment.
            "git status; rm x",
            "git status && rm x",
            "git status || rm x",
            "cd ..; rm x",
            // Shell state mutation and command substitution.
            "$x = git status",
            "$env:FOO = 'bar'",
            "echo $(rm x)",
            "echo `rm x`",
            // `cd` is only harmless with a literal path: an expansion could
            // name whatever the hidden command produced.
            "cd x; git status > f",
            "cd $(echo x); git status",
            "cd `pwd`; git status",
            "cd $env:FOO; git status",
            "cd $HOME; git status",
            "cd x; rm y",
            "cd x | tee f",
            "git status \"unbalanced",
        ];
        for command in fenced {
            assert!(
                !command_is_read_only(command),
                "expected fenced: {command:?}"
            );
        }
    }

    #[test]
    fn interrupted_read_only_shell_does_not_fence_the_session() {
        // The shape of tonight's incident: the call went out, the user pressed
        // stop a second later, and no result was ever recorded.
        for command in [
            "git worktree list",
            "git status --short",
            "Get-Date",
            // Tonight's third incident verbatim: `cd` came first in the chain.
            "cd D:/future-os; git worktree list; git status --short",
        ] {
            let args = json!({"command":command});
            let (root, manager) = fixture("shell", args.clone(), None);
            for automatic in [true, false] {
                let safety = RecoverySafety::load(
                    &manager,
                    "session",
                    root.path().to_str().unwrap(),
                    automatic,
                )
                .unwrap();
                assert!(safety.pause_reason().is_none(), "{command}");
                assert!(safety.before_tool("shell", &args).is_none(), "{command}");
            }
        }
    }

    #[test]
    fn interrupted_mutating_shell_still_fences_the_whole_session() {
        let args = json!({"command":"rm -rf build"});
        let (root, manager) = fixture("shell", args.clone(), None);
        let safety =
            RecoverySafety::load(&manager, "session", root.path().to_str().unwrap(), false)
                .unwrap();
        assert!(safety.pause_reason().is_some());
        assert!(safety.before_tool("shell", &args).unwrap().is_error);
        // The fence belongs to the session, not to the command that raised it:
        // a later read-only command is refused as well.
        assert!(
            safety
                .before_tool("shell", &json!({"command":"git status"}))
                .unwrap()
                .is_error
        );
    }

    #[test]
    fn read_only_excuse_never_covers_write_or_edit() {
        // One unresolved write and one unresolved read-only shell call in the
        // same transcript: only the shell is excused.
        let (root, manager) = fixture("write", json!({"path":"missing","content":"new"}), None);
        manager
            .append_entries_synced(
                "session",
                &[SessionEntry::new_assistant(
                    json!("more"),
                    vec![ToolCall {
                        id: "call2".into(),
                        call_type: "function".into(),
                        function: ToolCallFn {
                            name: "shell".into(),
                            arguments: json!({"command":"git status"}),
                        },
                    }],
                )],
            )
            .unwrap();
        let safety =
            RecoverySafety::load(&manager, "session", root.path().to_str().unwrap(), false)
                .unwrap();
        let reason = safety.pause_reason().expect("the write is still unknown");
        assert!(reason.contains("write (call)"), "{reason}");
        assert!(!reason.contains("shell"), "{reason}");
    }

    // ─── user abort is an operation-level question ─────────────────────────

    fn interrupted_shell_fixture(status: &str, command: &str) -> (tempfile::TempDir, Manager) {
        let mut result = SessionEntry::new_tool("call", "command interrupted");
        result.meta = Some(json!({"shell_result":{
            "status":status,
            "is_error":true,
            "exit_code":null,
            "note":"Shell command interrupted by abort",
        }}));
        let (root, manager) = fixture("shell", json!({"command":command}), Some(result));
        manager
            .append_entries_synced(
                "session",
                &[SessionEntry::run_terminal(
                    "run",
                    crate::session::RUN_STATE_CANCELLED,
                    0,
                    0,
                    None,
                )],
            )
            .unwrap();
        (root, manager)
    }

    #[test]
    fn user_aborted_shell_does_not_fence_the_whole_session() {
        // The incident: the user pressed stop, the shell was recorded as
        // cancelled, and every later tool in that session was refused.
        let (root, manager) =
            interrupted_shell_fixture("cancelled", "Remove-Item -Recurse -Force build");
        for automatic in [true, false] {
            let safety = RecoverySafety::load(
                &manager,
                "session",
                root.path().to_str().unwrap(),
                automatic,
            )
            .unwrap();
            assert!(safety.pause_reason().is_none());
            assert!(safety
                .before_tool("write", &json!({"path":"missing","content":"new"}))
                .is_none());
            assert!(safety
                .before_tool("shell", &json!({"command":"Get-Date"}))
                .is_none());
        }
    }

    #[test]
    fn aborted_operation_is_warned_once_then_allowed() {
        let args = json!({"command":"Remove-Item -Recurse -Force build"});
        let (root, manager) =
            interrupted_shell_fixture("cancelled", "Remove-Item -Recurse -Force build");
        let safety =
            RecoverySafety::load(&manager, "session", root.path().to_str().unwrap(), false)
                .unwrap();
        assert!(safety.pause_reason().is_none());
        let first = safety
            .before_tool("shell", &args)
            .expect("first attempt warns");
        assert!(first.is_error);
        assert!(
            first.result.starts_with("[RECOVERY_ABORTED_OPERATION]"),
            "{}",
            first.result
        );
        assert!(!first.result.starts_with("[RECOVERY_UNCERTAIN]"));
        // Same run, same operation: the model acted on the warning, so it may
        // now re-run the command instead of deadlocking on it forever.
        assert!(safety.before_tool("shell", &args).is_none());
        // An unrelated operation never paid for that warning.
        assert!(safety
            .before_tool(
                "shell",
                &json!({"command":"Remove-Item -Recurse -Force dist"})
            )
            .is_none());
        // A fresh run starts with a fresh warning, not a silent allowance.
        let reloaded =
            RecoverySafety::load(&manager, "session", root.path().to_str().unwrap(), false)
                .unwrap();
        assert!(reloaded.before_tool("shell", &args).is_some());
    }

    // A test named `missing_result_still_fences_even_in_a_cancelled_run` lived
    // here. It built "no result at all" with a `RUN_STATE_CANCELLED` terminal and
    // asserted the session fences — that folded a user stop into the crash case,
    // which is exactly the behaviour this file no longer has. A missing result
    // after a *crash* is still fenced by
    // `missing_result_after_a_restart_still_fences_the_session`, and a missing
    // result after a user stop is covered by
    // `missing_result_in_a_user_stopped_run_is_questioned_once_not_fenced`.

    #[test]
    fn not_started_shell_is_neither_fenced_nor_completed() {
        let args = json!({"command":"Remove-Item -Recurse -Force build"});
        let mut result = SessionEntry::new_tool("call", "not started");
        result.meta = Some(json!({"shell_result":{
            "status":"not_started",
            "is_error":true,
            "attempts":[{"status":"not_started"}],
        }}));
        let (root, manager) = fixture("shell", args.clone(), Some(result));
        let safety =
            RecoverySafety::load(&manager, "session", root.path().to_str().unwrap(), true).unwrap();
        assert!(safety.pause_reason().is_none());
        // Nothing ran, so this is not a recorded operation either: repeating it
        // is a fresh execution, not a repeat of a settled one.
        assert!(safety.before_tool("shell", &args).is_none());
    }

    #[test]
    fn a_refused_call_never_becomes_a_new_incident() {
        // The fence's own refusal is stored as the tool result. Reading it
        // back must not manufacture a second unresolved call, in this design
        // or any later one.
        let (root, manager) = fixture(
            "shell",
            json!({"command":"append-once"}),
            Some(SessionEntry::new_tool(
                "call",
                "[RECOVERY_UNCERTAIN] The previous operation's effect could not be verified.",
            )),
        );
        let safety =
            RecoverySafety::load(&manager, "session", root.path().to_str().unwrap(), false)
                .unwrap();
        assert!(safety.pause_reason().is_none());
    }

    #[test]
    fn lost_tool_result_still_fences_the_session() {
        let args = json!({"command":"Remove-Item -Recurse -Force build"});
        let (root, manager) = fixture(
            "shell",
            args.clone(),
            Some(SessionEntry::new_tool(
                "call",
                "[Tool execution lost before the result was recorded]",
            )),
        );
        manager
            .append_entries_synced(
                "session",
                &[SessionEntry::run_terminal(
                    "run",
                    crate::session::RUN_STATE_CANCELLED,
                    0,
                    0,
                    None,
                )],
            )
            .unwrap();
        let safety =
            RecoverySafety::load(&manager, "session", root.path().to_str().unwrap(), false)
                .unwrap();
        assert!(safety.pause_reason().is_some());
        assert!(safety.before_tool("shell", &args).unwrap().is_error);
    }

    // ─── an unexecuted call must be on disk as unexecuted ──────────────────

    /// The fixture's run ended in `state`, which is what decides whether a
    /// missing result is a user stop or a crash.
    fn stop_state(state: &str) -> SessionEntry {
        SessionEntry::run_terminal("run", state, 0, 0, None)
    }

    #[test]
    fn persisted_skip_placeholder_does_not_fence_after_a_reload() {
        // Before the skip placeholder was persisted, this session had a call
        // with no result at all and read as a crash. With the result on disk
        // it reads as what it is: a call that never ran.
        let args = json!({"command":"Remove-Item -Recurse -Force build"});
        let (root, manager) = fixture(
            "shell",
            args.clone(),
            Some(SessionEntry::new_tool(
                "call",
                "[Tool execution cancelled — shell was skipped due to user interrupt]",
            )),
        );
        manager
            .append_entries_synced(
                "session",
                &[stop_state(crate::session::RUN_STATE_CANCELLED)],
            )
            .unwrap();
        for automatic in [true, false] {
            let safety = RecoverySafety::load(
                &manager,
                "session",
                root.path().to_str().unwrap(),
                automatic,
            )
            .unwrap();
            assert!(safety.pause_reason().is_none());
            // Nothing ran, so nothing is questioned and nothing is "recorded".
            assert!(safety.before_tool("shell", &args).is_none());
            assert!(safety
                .before_tool("write", &json!({"path":"missing","content":"new"}))
                .is_none());
        }
    }

    #[test]
    fn missing_result_in_a_user_stopped_run_is_questioned_once_not_fenced() {
        // The shape left on disk by an older build: two calls in flight, the
        // user stopped the run, only one result ever landed.
        let args = json!({"command":"Remove-Item -Recurse -Force build"});
        let (root, manager) = fixture("shell", args.clone(), None);
        manager
            .append_entries_synced(
                "session",
                &[stop_state(crate::session::RUN_STATE_CANCELLED)],
            )
            .unwrap();
        for automatic in [true, false] {
            let safety = RecoverySafety::load(
                &manager,
                "session",
                root.path().to_str().unwrap(),
                automatic,
            )
            .unwrap();
            assert!(safety.pause_reason().is_none());
            assert!(safety
                .before_tool("write", &json!({"path":"missing","content":"new"}))
                .is_none());
            let first = safety
                .before_tool("shell", &args)
                .expect("the interrupted operation itself is questioned once");
            assert!(first.result.starts_with("[RECOVERY_ABORTED_OPERATION]"));
            assert!(safety.before_tool("shell", &args).is_none());
        }
    }

    #[test]
    fn missing_result_after_a_restart_still_fences_the_session() {
        // A system shutdown or a kill is what this fence exists for: nobody
        // was present, so the session must not silently continue.
        for state in [
            crate::session::RUN_STATE_INTERRUPTED_BY_RESTART,
            crate::session::RUN_STATE_INCOMPLETE,
            crate::session::RUN_STATE_ERROR,
        ] {
            let args = json!({"command":"Remove-Item -Recurse -Force build"});
            let (root, manager) = fixture("shell", args.clone(), None);
            manager
                .append_entries_synced("session", &[stop_state(state)])
                .unwrap();
            let safety =
                RecoverySafety::load(&manager, "session", root.path().to_str().unwrap(), true)
                    .unwrap();
            assert!(safety.pause_reason().is_some(), "{state}");
            assert!(
                safety.before_tool("shell", &args).unwrap().is_error,
                "{state}"
            );
        }
    }

    #[test]
    fn timed_out_result_still_fences_even_in_a_user_stopped_run() {
        // A timeout is nobody's decision: the run's own state must not excuse
        // it just because the user also pressed stop later.
        for status in ["timed_out", "execution_failed"] {
            let mut result = SessionEntry::new_tool("call", "command interrupted");
            result.meta = Some(json!({"shell_result":{"status":status,"is_error":true}}));
            let (root, manager) = fixture(
                "shell",
                json!({"command":"Remove-Item -Recurse -Force build"}),
                Some(result),
            );
            manager
                .append_entries_synced(
                    "session",
                    &[stop_state(crate::session::RUN_STATE_CANCELLED)],
                )
                .unwrap();
            let safety =
                RecoverySafety::load(&manager, "session", root.path().to_str().unwrap(), false)
                    .unwrap();
            assert!(safety.pause_reason().is_some(), "{status}");
        }
    }

    #[test]
    fn aborted_read_only_shell_stays_silently_exempt() {
        // A read-only command cannot have changed anything, so there is
        // nothing to check and nothing to warn about.
        let (root, manager) = interrupted_shell_fixture("cancelled", "git status --short");
        let safety =
            RecoverySafety::load(&manager, "session", root.path().to_str().unwrap(), true).unwrap();
        assert!(safety.pause_reason().is_none());
        assert!(safety
            .before_tool("shell", &json!({"command":"git status --short"}))
            .is_none());
    }

    #[test]
    fn aborted_write_that_already_landed_is_verified_instead_of_questioned() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "complete content").unwrap();
        let args = json!({"path":file.path(),"content":"complete content"});
        let mut result = SessionEntry::new_tool("call", "command interrupted");
        result.meta = Some(json!({"shell_result":{"status":"cancelled","is_error":true}}));
        let (root, manager) = fixture("write", args.clone(), Some(result));
        manager
            .append_entries_synced(
                "session",
                &[stop_state(crate::session::RUN_STATE_CANCELLED)],
            )
            .unwrap();
        let safety =
            RecoverySafety::load(&manager, "session", root.path().to_str().unwrap(), true).unwrap();
        assert!(safety.pause_reason().is_none());
        // The file already holds the requested content: that answer exists, so
        // the model is not asked to go and check it itself.
        let stored = manager.load_unrepaired_entries("session").unwrap();
        assert!(stored.iter().any(|e| e
            .meta
            .as_ref()
            .is_some_and(|m| m["recovery_verified"] == true)));
        assert!(!safety
            .before_tool("write", &args)
            .is_some_and(|r| r.result.starts_with("[RECOVERY_ABORTED_OPERATION]")));
        assert_eq!(
            std::fs::read_to_string(file.path()).unwrap(),
            "complete content"
        );
    }
}
