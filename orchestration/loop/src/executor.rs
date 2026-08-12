//! gRPC executor — one bounded agent turn through FutureAgent.
//!
//! The agent stays policy-free: one prompt = one bounded turn. This module is
//! pure transport (build the packet, prompt, observe the event stream,
//! account spend) — all loop policy lives in `decision.rs` / `state.rs`.

use anyhow::Result;

use crate::agent_client::{AgentClient, RunSummary};
use crate::decision::{compose_goal_boundary, compose_turn_envelope, compose_turn_message};
use crate::state::{
    now_epoch, task_validation_receipt, Goal, RecoveryKind, RunRecord, TaskValidation, Todo,
    ValidationStatus,
};

/// A turn counts as succeeded only when the agent finished AND any attached
/// independent validator passed (no validator ⇒ not required ⇒ ok).
pub fn turn_succeeded(record: &RunRecord) -> bool {
    record.terminal_state == "completed" && record.validation.as_ref().map(|v| v.ok).unwrap_or(true)
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

/// Execute one bounded turn for `todo` and return the ledger entry (spend).
/// `decision_summary` (G-9): the `ShouldRunPacket` from the decision kernel,
/// embedded in the turn envelope so the agent sees the decision context
/// (mode / reason / arbitration) alongside the instruction.
#[allow(clippy::too_many_arguments)]
pub async fn execute_turn(
    client: &mut AgentClient,
    session_id: &str,
    goal: &Goal,
    todo: &Todo,
    turn: u32,
    prev: Option<&RunRecord>,
    boundary_injected: bool,
    decision_summary: Option<&crate::contract::ShouldRunPacket>,
    runs_dir: Option<std::path::PathBuf>,
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
    let message = format!("session: {session_id}\n{message}");
    // Idempotency key owned by the orchestrator — retry must not double-execute.
    let client_request_id = format!("turn-{}-{}", turn, todo.id);

    let before = client.session_totals(session_id).await?;
    let run_id = client
        .prompt(session_id, &message, &client_request_id)
        .await?;
    let live_path = runs_dir.map(|d| d.join(format!("{run_id}.live.jsonl")));
    let summary: RunSummary = client
        .run_turn(session_id, &run_id, live_path.as_deref())
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
        evidence: crate::decision::truncate(&summary.text, 2_000),
        recorded_at: now_epoch(),
        // G-7: stamped by the caller (main.rs writeback) with the mode-based
        // spend source before the record hits the ledger.
        spend_source: None,
        validation: None,
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
                m.resume_when = Some(
                    std::time::SystemTime::now()
                        + std::time::Duration::from_secs(
                            crate::decision::monitor::MONITOR_NO_CHANGE_BACKOFF_SECS,
                        ),
                );
            }
        }
        return;
    }
    if turn_succeeded(record) {
        let (no_follow_up, successors) = completion.unwrap_or((true, vec![]));
        if let Some(t) = goal.todo_mut(&record.todo_id) {
            t.complete(no_follow_up, successors);
        }
    } else if let Some(t) = goal.todo_mut(&record.todo_id) {
        // Failed turn or failed independent validation → one repair attempt
        // (bounded by max_validation_attempts in the run loop).
        t.failed_attempts += 1;
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
