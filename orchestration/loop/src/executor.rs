//! gRPC executor — one bounded agent turn through FutureAgent.
//!
//! The agent stays policy-free: one prompt = one bounded turn. This module is
//! pure transport (build the packet, prompt, observe the event stream,
//! account spend) — all loop policy lives in `decision.rs` / `state.rs`.

use anyhow::Result;

use crate::agent_client::{AgentClient, RunSummary, TurnProgressTracker};
use crate::decision::{compose_goal_boundary, compose_turn_envelope, compose_turn_message};
use crate::state::{
    now_epoch, task_validation_receipt, Goal, RecoveryKind, RunRecord, TaskValidation, Todo,
    ValidationStatus,
};

/// Evidence is a summary the orchestrator reads to decide what a worker
/// actually landed — it is NOT a full transcript. Truncating head-only loses
/// the conclusion (workers state findings last), so keep a bounded head AND
/// tail with an explicit elision marker. Bounded because evidence is replayed
/// into every subsequent turn envelope.
pub fn truncate_evidence(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    // The elision marker is a 3-byte '…'. Head gets the bulk (context /
    // approach); tail keeps the conclusion. All bounds are BYTE bounds
    // (str::len), so they hold against any UTF-8 content.
    const MARK: &str = "…";
    let head_bytes = max * 3 / 4;
    let tail_bytes = max - head_bytes - MARK.len();
    let head = &text[..text.floor_char_boundary(head_bytes)];
    let tail_start = text.floor_char_boundary(text.len() - tail_bytes);
    let tail = &text[tail_start..];
    format!("{head}{MARK}{tail}")
}

/// Backoff (seconds) a turn sleeps before the run exits when it ended in an
/// HTTP 429 (engine overloaded / rate-limited). Relaunching immediately after
/// a 429 re-hits the same throttle and burns turns without progress — a short
/// sleep lets the orchestrator's next `run` land on a cooled-down engine.
/// (Measured: 10 concurrent workers all hammering one model produced 22
/// error turns in a single night, every one of them a 429.)
pub const RATE_LIMIT_BACKOFF_SECS: u64 = 45;

/// Default bound for consecutive `incomplete` turns (model stream truncated
/// mid-reply) on the same todo before `future loop run` stops. The retry
/// itself is free — an incomplete turn is an infra event that never consumes
/// the repair budget — but an endless truncation loop would spin without
/// progress, so it is bounded.
pub const DEFAULT_MAX_INCOMPLETE_RETRIES: u32 = 3;

/// Whether a turn's error is a recoverable rate-limit (HTTP 429 / overloaded).
/// These are NOT science failures — they are throttle events — so the loop
/// must back off, not count them toward a repair budget or replan.
///
/// NOTE: this is deliberately narrow (429-only) because the caller uses it
/// for two things: failure classification AND the 45s cool-down sleep before
/// relaunch. Classification of all infra causes (rate-limit OR upstream
/// transport) goes through `is_infra_recoverable_error`.
pub fn is_rate_limit_error(error: Option<&str>) -> bool {
    match error {
        None => false,
        Some(e) => {
            let lower = e.to_lowercase();
            lower.contains("429")
                || lower.contains("rate limit")
                || lower.contains("rate-limited")
                || lower.contains("overloaded")
        }
    }
}

/// Whether a turn's error is an upstream transport failure — the provider or
/// the network cut the response mid-stream. reqwest folds every body-read
/// failure (connection reset, premature EOF, content decoding) into the one
/// opaque message `error decoding response body` (`Kind::Decode`); the agent
/// surfaces its own idle / disconnect verdicts as `[UPSTREAM_DISCONNECTED] …`.
/// None of these are plan problems, so they must not count toward the repair
/// budget or page the supervisor — same policy as HTTP 429.
pub fn is_transport_error(error: Option<&str>) -> bool {
    // Lower-cased marker substrings. Deliberately conservative: science
    // failure messages (validator output, proof/derivation errors) never
    // contain these, and the agent's own verdicts are exact strings.
    const MARKERS: &[&str] = &[
        "decoding response body",
        "upstream_disconnected",
        "upstream disconnected",
        "stream was idle",
        "connection reset",
        "connection closed",
        "unexpected eof",
        "error reading a body",
        "broken pipe",
        "reset by peer",
        "event stream gap",
    ];
    match error {
        None => false,
        Some(e) => {
            let lower = e.to_lowercase();
            MARKERS.iter().any(|marker| lower.contains(marker))
        }
    }
}

/// Whether a turn's error is infrastructure-recoverable: a throttle event
/// (HTTP 429 / overloaded) or an upstream transport failure. Neither is a
/// science failure — the loop backs off / lets the orchestrator relaunch,
/// and must never burn the repair budget or replan for them.
pub fn is_infra_recoverable_error(error: Option<&str>) -> bool {
    is_rate_limit_error(error) || is_transport_error(error)
}

/// Classify a turn's failure into the writeback ledger (A). Backward-compat:
/// a legacy record without `failure_kind` is derived from `terminal_state` +
/// `validation` exactly as the writeback stamps it.
pub fn classify_failure(record: &crate::state::RunRecord) -> crate::state::FailureKind {
    use crate::state::{FailureKind, ValidationStatus};
    // A verify-gate failure is the science failure regardless of terminal_state.
    if let Some(v) = &record.validation {
        if v.status == ValidationStatus::Failed && !v.ok {
            return FailureKind::ScienceVerifyFailed;
        }
    }
    if record.terminal_state == "error" {
        return if is_infra_recoverable_error(record.error.as_deref()) {
            FailureKind::InfraRecoverable
        } else {
            FailureKind::HardError
        };
    }
    if record.terminal_state == "completed" {
        return FailureKind::None;
    }
    // cancelled / incomplete / other non-terminal: treat as infra-recoverable
    // (budget-truncated or externally stopped; not a science failure).
    FailureKind::InfraRecoverable
}

/// A turn counts as succeeded only when the agent finished AND any attached
/// independent validator passed (no validator ⇒ not required ⇒ ok).
pub fn turn_succeeded(record: &RunRecord) -> bool {
    record.terminal_state == "completed" && record.validation.as_ref().map(|v| v.ok).unwrap_or(true)
}

/// O3: evaluate turn-end progress. Returns `Some(idle_secs)` when the last
/// write-class tool (write/edit/shell) start — or the turn start when no
/// write-class tool started at all — is at least `threshold_secs` before
/// `now`; `None` otherwise. Pure so tests exercise it without wall-clock
/// waits.
pub fn no_progress_idle_secs(
    turn_start_at: u64,
    last_write_tool_at: Option<u64>,
    now: u64,
    threshold_secs: u64,
) -> Option<u64> {
    let idle_secs = now.saturating_sub(last_write_tool_at.unwrap_or(turn_start_at));
    (idle_secs >= threshold_secs).then_some(idle_secs)
}

/// Detect a statically-tautological validator: a `--verify` command whose
/// exit status is fixed regardless of the filesystem. Both directions are the
/// fake-completion vector:
/// - **always-false** (`test -n ""`): a `$(...)` substitution expanded to
///   empty before the shell ever ran it — the gate can never pass, so the
///   todo sits in an infinite repair loop (measured: one such validator cost
///   79 failed turns on a single todo).
/// - **always-true** (`test -z ""`): the gate never bites, so a placeholder
///   payload is marked done.
///
/// Refuse both at `todo add/update --verify` time rather than discovering the
/// loop minutes later. Returns a human reason for a tautological validator,
/// `None` for a plausible one.
pub fn validator_tautology(cmd: &str) -> Option<&'static str> {
    // `-n` with an EMPTY string literal → always false. This is the exact
    // accident: `test -n "$(ls ...)"` written in an outer shell with an empty
    // `ls` result collapses to `test -n ""` before the loop ever runs it.
    if cmd.contains("-n \"\"") || cmd.contains("-n ''") || cmd.contains("-n \"\"") {
        return Some(
            "`-n \"\"` is always false — a command substitution expanded to empty before the shell \
             ran it, so the verify gate can never pass; assert a concrete file path or content instead",
        );
    }
    // `-z` with an EMPTY string literal → always true (gate never bites).
    if cmd.contains("-z \"\"") || cmd.contains("-z ''") {
        return Some(
            "`-z \"\"` is always true — the verify gate passes regardless of output; use a \
             concrete `test -f <file>` / content assertion",
        );
    }
    None
}

/// Run the todo's independent validator (if any) after a completed turn.
/// `todo add --verify "cmd"` attaches a validator; the kernel runs it in the
/// goal cwd and only completes the todo when it exits 0. No validator ⇒
/// `None` (validation not required ⇒ material results default to ok).
fn run_validator(goal: &Goal, todo: &Todo) -> Option<TaskValidation> {
    let cmd = todo.validator.as_deref()?;
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(&goal.cwd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status();
    match status {
        Ok(s) => {
            let code = s.code().unwrap_or(-1);
            if code == 0 {
                Some(task_validation_receipt(
                    ValidationStatus::Passed,
                    cmd,
                    "validator passed (exit 0)",
                    None,
                    Some(code),
                ))
            } else {
                Some(task_validation_receipt(
                    ValidationStatus::Failed,
                    cmd,
                    &format!("validator exited {code} — repair required"),
                    Some(RecoveryKind::RepairRequired),
                    Some(code),
                ))
            }
        }
        Err(e) => Some(task_validation_receipt(
            ValidationStatus::Inconclusive,
            cmd,
            &format!("validator failed to run: {e}"),
            Some(RecoveryKind::RepairRequired),
            None,
        )),
    }
}

/// Consecutive trailing `incomplete` records for `todo_id` (the just-written
/// record included). Replayed from the ledger, so an orchestrator restart
/// does not reset the bound.
pub fn incomplete_streak(history: &[RunRecord], todo_id: &str) -> u32 {
    history
        .iter()
        .rev()
        .take_while(|r| r.todo_id == todo_id && r.terminal_state == "incomplete")
        .count() as u32
}

/// The continue note injected into the next turn's envelope when the previous
/// turn ended `incomplete`, or `None` when the retry bound is exhausted and
/// the run must stop. A truncated stream is an infra event, not a science
/// failure — the agent gets an explicit "continue where you left off" instead
/// of a bare re-offer of the same todo (which invites restarting from scratch).
pub fn incomplete_continue_note(streak: u32, max_retries: u32, todo_id: &str) -> Option<String> {
    if max_retries == 0 || streak >= max_retries {
        return None;
    }
    Some(format!(
        "CONTINUE: your previous turn for todo {todo_id} ended INCOMPLETE \
         (the model stream was truncated mid-reply). Pick up exactly where \
         you left off — do not restart work that already completed. \
         (incomplete attempt {streak}/{max_retries})"
    ))
}

/// Execute one bounded turn for `todo` and return the ledger entry (spend).
/// `decision_summary` (G-9): the `ShouldRunPacket` from the decision kernel,
/// embedded in the turn envelope so the agent sees the decision context
/// (mode / reason / arbitration) alongside the instruction.
/// `continue_note`: injected at the top of the turn message when the previous
/// turn ended incomplete (bounded retry).
#[allow(clippy::too_many_arguments)]
pub async fn execute_turn(
    client: &mut AgentClient,
    session_id: &str,
    goal: &Goal,
    agent_id: Option<&str>,
    todo: &Todo,
    turn: u32,
    prev: Option<&RunRecord>,
    boundary_injected: bool,
    decision_summary: Option<&crate::contract::ShouldRunPacket>,
    runs_dir: Option<std::path::PathBuf>,
    progress: Option<&TurnProgressTracker>,
    continue_note: Option<&str>,
) -> Result<RunRecord> {
    if !boundary_injected {
        client
            .append_system_prompt(session_id, &compose_goal_boundary(goal))
            .await?;
    }
    let message = match decision_summary {
        Some(packet) => compose_turn_envelope(goal, todo, Some(packet), prev),
        None => compose_turn_message(goal, todo, prev),
    };
    // Surface the backing agent session id so in-turn tooling (e.g. trace
    // converters) can locate the real session deterministically instead of
    // guessing by file mtime.
    let message = match continue_note {
        Some(note) => format!("{note}\n\nsession: {session_id}\n{message}"),
        None => format!("session: {session_id}\n{message}"),
    };
    // Idempotency key owned by the orchestrator — retry must not double-execute.
    let client_request_id = format!("turn-{}-{}", turn, todo.id);

    let before = client.session_totals(session_id).await?;
    let run_id = client
        .prompt(session_id, &message, &client_request_id)
        .await?;
    let live_path = runs_dir.map(|d| d.join(format!("{run_id}.live.jsonl")));
    // Write a run-header line FIRST so the read-only dashboard can associate
    // this live run with its worker/todo and expose real-time token/cost.
    // (`run_turn` then appends streamed events to the same file.)
    if let Some(path) = &live_path {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let header = serde_json::json!({
            "type": "run_header",
            "idx": 0,
            "wall_ts": now_epoch(),
            "run_id": run_id,
            "session_id": session_id,
            "agent_id": agent_id,
            "todo_id": todo.id,
            "goal_id": goal.goal_id,
        });
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
        {
            use std::io::Write;
            let _ = writeln!(f, "{header}");
        }
    }
    let summary: RunSummary = client
        .run_turn(session_id, &run_id, live_path.as_deref(), progress)
        .await?;
    let after = client.session_totals(session_id).await?;
    let terminal_state = summary.terminal_state.clone();

    let mut record = RunRecord {
        turn,
        todo_id: todo.id.clone(),
        run_id: summary.run_id.clone(),
        terminal_state: summary.terminal_state,
        error: summary.error,
        tokens_in_delta: after.tokens_in.saturating_sub(before.tokens_in),
        tokens_out_delta: after.tokens_out.saturating_sub(before.tokens_out),
        cost_delta: (after.cost - before.cost).max(0.0),
        tools: summary.tools,
        evidence: truncate_evidence(&summary.text, 4_000),
        recorded_at: now_epoch(),
        // G-7: stamped by the caller (main.rs writeback) with the mode-based
        // spend source before the record hits the ledger.
        spend_source: None,
        // A: failure classification, stamped at writeback (None until then).
        failure_kind: None,
        validation: None,
        // A1: the agent's own truncation verdict (model stream cut off
        // mid-run) with turn/tool progress; `None` when the loop merely saw
        // the event stream close without a terminal event.
        truncation: summary.truncation,
    };
    // Independent validator (if any) runs after a completed turn; a failed or
    // interrupted turn never runs the validator (no material result to check).
    record.validation = if terminal_state == "completed" {
        run_validator(goal, todo)
    } else {
        None
    };
    Ok(record)
}

/// Writeback: fold a turn into goal state and the ledger.
/// - success → complete the todo (caller decides successor/no-follow-up)
/// - failure → failed_attempts + 1 (kernel decides bounded retry)
/// - monitor poll → update the monitor's own counter; a no-change poll never
///   spends (LoopX spend rules).
///
/// A: the repair budget charges ONLY science failures. An infra-recoverable
/// turn (HTTP 429 / rate-limit / connection reset / agent crash) is a throttle
/// event — it is backed off and retried by the orchestrator, but it never
/// consumes `failed_attempts` (otherwise a 429 storm would blow through the
/// bounded budget and replan the goal for a purely infrastructure cause).
pub fn writeback(
    goal: &mut Goal,
    record: &RunRecord,
    monitor_changed: Option<bool>,
    completion: Option<(bool, Vec<String>)>,
) {
    if let Some(changed) = monitor_changed {
        if let Some(m) = goal.todo_mut(&record.todo_id) {
            if changed {
                m.consecutive_no_change = 0;
                m.status = crate::state::TodoStatus::Done;
                goal.history.push(record.clone());
            } else {
                m.consecutive_no_change += 1;
                // P1-3②: cadence-aware reschedule, the same derivation
                // replay applies to the MonitorPolled event.
                let next_due = crate::scheduler::monitor_poll::next_poll_due_epoch(
                    now_epoch(),
                    m.monitor_cadence.as_deref(),
                );
                m.resume_when = Some(
                    std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(next_due),
                );
            }
        }
        return;
    }
    let failure_kind = record
        .failure_kind
        .unwrap_or_else(|| classify_failure(record));
    if turn_succeeded(record) {
        let (no_follow_up, successors) = completion.unwrap_or((true, vec![]));
        if let Some(t) = goal.todo_mut(&record.todo_id) {
            t.complete(no_follow_up, successors);
        }
    } else if let Some(t) = goal.todo_mut(&record.todo_id) {
        // A: infra-recoverable failures do NOT consume the repair budget.
        if failure_kind != crate::state::FailureKind::InfraRecoverable {
            t.failed_attempts += 1;
        }
    }
    // Outcome floor: a turn is material when it produced a validated artifact
    // (tools invoked + evidence). Surface-only turns accumulate the streak.
    let material = !record.tools.is_empty() && !record.evidence.trim().is_empty();
    if material {
        goal.outcome_streak = 0;
    } else {
        goal.outcome_streak += 1;
    }
    goal.history.push(record.clone());
}

#[cfg(test)]
mod validator_tests {
    use super::validator_tautology;
    #[test]
    fn empty_n_literal_is_always_false() {
        assert!(validator_tautology("test -d out && test -n \"\"").is_some());
        assert!(validator_tautology("test -n ''").is_some());
        assert!(validator_tautology("[ -n \"\" ]").is_some());
    }

    #[test]
    fn empty_z_literal_is_always_true() {
        assert!(validator_tautology("test -z \"\"").is_some());
        assert!(validator_tautology("[ -z '' ]").is_some());
    }

    #[test]
    fn concrete_assertions_pass() {
        assert_eq!(validator_tautology("test -f outputs/packing.json"), None);
        assert_eq!(
            validator_tautology("test -f out.json && python check.py"),
            None
        );
        assert_eq!(validator_tautology("test -n \"$(ls outputs)\""), None);
    }

    #[test]
    fn rate_limit_detection() {
        use super::is_rate_limit_error;
        assert!(is_rate_limit_error(Some(
            "Rate limited (HTTP 429): The engine is currently overloaded"
        )));
        assert!(is_rate_limit_error(Some("rate limit exceeded")));
        assert!(is_rate_limit_error(Some("server overloaded")));
        assert!(!is_rate_limit_error(Some("validator exited 1")));
        assert!(!is_rate_limit_error(Some("timeout")));
        assert!(!is_rate_limit_error(None));
    }

    #[test]
    fn transport_error_detection() {
        use super::is_transport_error;
        // reqwest folds every mid-stream body failure into this one message.
        assert!(is_transport_error(Some("error decoding response body")));
        // The agent's disconnect verdicts (post-#424 prefix included).
        assert!(is_transport_error(Some(
            "[UPSTREAM_DISCONNECTED] error decoding response body"
        )));
        assert!(is_transport_error(Some(
            "[UPSTREAM_DISCONNECTED] model response stream was idle for 45 seconds"
        )));
        // Transport-level body failures.
        assert!(is_transport_error(Some("connection reset by peer")));
        assert!(is_transport_error(Some("unexpected eof")));
        assert!(is_transport_error(Some("error reading a body")));
        assert!(is_transport_error(Some("broken pipe")));
        // Case-insensitive.
        assert!(is_transport_error(Some("ERROR DECODING RESPONSE BODY")));
        // Science failures and unrelated messages must NOT match.
        assert!(!is_transport_error(Some("validator exited 1")));
        assert!(!is_transport_error(Some("timeout")));
        assert!(!is_transport_error(Some(
            "[CTX_LIMIT] Request exceeds the model context limit"
        )));
        assert!(!is_transport_error(None));
    }

    #[test]
    fn infra_recoverable_error_detection() {
        use super::is_infra_recoverable_error;
        // Throttle events.
        assert!(is_infra_recoverable_error(Some("Rate limited (HTTP 429)")));
        assert!(is_infra_recoverable_error(Some("server overloaded")));
        // Upstream transport failures.
        assert!(is_infra_recoverable_error(Some(
            "error decoding response body"
        )));
        assert!(is_infra_recoverable_error(Some(
            "[UPSTREAM_DISCONNECTED] model response stream was idle"
        )));
        // Science failures stay hard.
        assert!(!is_infra_recoverable_error(Some("validator exited 1")));
        assert!(!is_infra_recoverable_error(None));
    }

    #[test]
    fn classify_failure_covers_all_terminal_branches() {
        use super::classify_failure;
        use crate::state::{FailureKind, RunRecord, TaskValidation, ValidationStatus};

        fn rec(
            terminal_state: &str,
            error: Option<&str>,
            validation: Option<TaskValidation>,
        ) -> RunRecord {
            RunRecord {
                turn: 1,
                todo_id: "T1".to_string(),
                run_id: "run-1".to_string(),
                terminal_state: terminal_state.to_string(),
                error: error.map(str::to_string),
                tokens_in_delta: 0,
                tokens_out_delta: 0,
                cost_delta: 0.0,
                tools: vec![],
                evidence: String::new(),
                recorded_at: 0,
                spend_source: None,
                failure_kind: None,
                truncation: None,
                validation,
            }
        }

        // A verify-gate failure is science regardless of terminal_state.
        let failed = Some(TaskValidation {
            schema_version: "v1".to_string(),
            status: ValidationStatus::Failed,
            validator_kind: "shell".to_string(),
            summary: String::new(),
            recovery_kind: None,
            exit_code: Some(1),
            ok: false,
        });
        assert_eq!(
            classify_failure(&rec("completed", None, failed)),
            FailureKind::ScienceVerifyFailed
        );

        // error + rate-limit → infra-recoverable (does NOT burn repair budget).
        assert_eq!(
            classify_failure(&rec("error", Some("Rate limited (HTTP 429)"), None)),
            FailureKind::InfraRecoverable
        );
        // error + upstream transport failure (provider cut the stream) →
        // infra-recoverable, same policy as 429.
        assert_eq!(
            classify_failure(&rec("error", Some("error decoding response body"), None)),
            FailureKind::InfraRecoverable
        );
        assert_eq!(
            classify_failure(&rec(
                "error",
                Some("[UPSTREAM_DISCONNECTED] model response stream was idle for 45 seconds"),
                None
            ),),
            FailureKind::InfraRecoverable
        );
        // error + non-infra → hard error.
        assert_eq!(
            classify_failure(&rec("error", Some("validator exited 1"), None)),
            FailureKind::HardError
        );
        // completed without validation → none.
        assert_eq!(
            classify_failure(&rec("completed", None, None)),
            FailureKind::None
        );
        // cancelled / incomplete / other → infra-recoverable.
        assert_eq!(
            classify_failure(&rec("cancelled", None, None)),
            FailureKind::InfraRecoverable
        );
        assert_eq!(
            classify_failure(&rec("incomplete", None, None)),
            FailureKind::InfraRecoverable
        );
    }

    #[test]
    fn incomplete_streak_counts_only_trailing_same_todo_records() {
        use super::{incomplete_streak, RunRecord};
        fn rec(turn: u32, todo: &str, state: &str) -> RunRecord {
            RunRecord {
                turn,
                todo_id: todo.to_string(),
                run_id: format!("run-{turn}"),
                terminal_state: state.to_string(),
                error: None,
                tokens_in_delta: 0,
                tokens_out_delta: 0,
                cost_delta: 0.0,
                tools: vec![],
                evidence: String::new(),
                recorded_at: 0,
                spend_source: Some("run".into()),
                failure_kind: None,
                truncation: None,
                validation: None,
            }
        }
        // completed, incomplete, incomplete → streak 2
        let history = vec![
            rec(1, "T1", "completed"),
            rec(2, "T1", "incomplete"),
            rec(3, "T1", "incomplete"),
        ];
        assert_eq!(incomplete_streak(&history, "T1"), 2);
        // a trailing record for ANOTHER todo breaks the streak
        let history = vec![
            rec(1, "T1", "incomplete"),
            rec(2, "T1", "incomplete"),
            rec(3, "T2", "completed"),
        ];
        assert_eq!(incomplete_streak(&history, "T1"), 0);
        // a completed record for the same todo breaks the streak
        let history = vec![
            rec(1, "T1", "incomplete"),
            rec(2, "T1", "completed"),
            rec(3, "T1", "incomplete"),
        ];
        assert_eq!(incomplete_streak(&history, "T1"), 1);
        assert_eq!(incomplete_streak(&[], "T1"), 0);
    }

    #[test]
    fn incomplete_continue_note_respects_the_bound() {
        use super::incomplete_continue_note;
        let note = incomplete_continue_note(1, 3, "T1").expect("retry allowed");
        assert!(note.contains("CONTINUE"), "note: {note}");
        assert!(note.contains("todo T1"), "note: {note}");
        assert!(note.contains("1/3"), "note: {note}");
        assert!(incomplete_continue_note(2, 3, "T1").is_some());
        // streak reaches the bound → stop
        assert!(incomplete_continue_note(3, 3, "T1").is_none());
        assert!(incomplete_continue_note(4, 3, "T1").is_none());
        // a zero bound disables retries entirely
        assert!(incomplete_continue_note(1, 0, "T1").is_none());
    }

    #[test]
    fn truncate_evidence_keeps_head_and_tail() {
        use super::truncate_evidence;
        // Short enough: unchanged.
        let short = "short evidence";
        assert_eq!(truncate_evidence(short, 100), short);

        // Long: keeps the head and the tail with an elision marker, and
        // stays within the budget.
        let body = "a".repeat(200);
        let text = format!("{body}END-MARKER");
        let out = truncate_evidence(&text, 100);
        assert!(out.len() <= 100, "len {} > 100", out.len());
        assert!(out.contains('…'), "elision marker: {out}");
        assert!(out.starts_with('a'), "keeps head: {out}");
        assert!(out.ends_with("END-MARKER"), "keeps tail: {out}");
    }
}
