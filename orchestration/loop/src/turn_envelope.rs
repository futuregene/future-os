//! Turn envelope (G-9) — the per-turn prompt assembly, expanded from the P0
//! ~30-line `compose_turn_message` into an envelope: instruction + context +
//! evidence + decision summary (reference `control_plane/quota/turn_envelope.py`,
//! 796 lines — we implement the minimal deterministic core the host needs).
//!
//! The envelope is a plain-text prompt for a policy-free agent: one bounded
//! turn, no loop policy embedded (all policy stays in the decision kernel).
//! [`compose_turn_message`] keeps the P0 signature used by the gRPC executor
//! and delegates here; the richer [`compose_turn_envelope`] carries the
//! decision summary for hosts that have the packet at hand.

use crate::contract::ShouldRunPacket;
use crate::decision::truncate;
use crate::state::{FailureKind, Goal, RunRecord, Todo, TodoStatus};

/// Envelope schema version (reference `TURN_ENVELOPE_SCHEMA_VERSION`).
pub const TURN_ENVELOPE_SCHEMA_VERSION: &str = "future_loop_turn_envelope_v0";

/// How many recent semantic events surface in the goal-memory block.
const GOAL_MEMORY_SEMANTIC_EVENTS: usize = 5;
/// Cap the failure-cause text so a runaway error never bloats the prompt.
const GOAL_MEMORY_ERROR_CHARS: usize = 200;

/// Compose the goal-memory block fed to the orchestrating agent every turn.
///
/// This closes the gap between what the kernel *collects* (failure
/// classification, semantic history) and what the agent *actually sees*:
///
/// 1. the current todo's failure context — how many prior attempts failed and
///    WHY (classified: infra-recoverable vs verify-gate vs hard error), so the
///    agent knows whether to retry, fix the `--verify` gate, or supersede;
/// 2. the goal's recent semantic history — the last few turn outcomes, so a
///    long-run goal's agent stops re-deriving context it already established.
///
/// The kernel still never decides for the agent: it only surfaces
/// observations (ARCHITECTURE-SIMPLIFICATION). Returns an empty string when
/// there is nothing to surface.
pub fn compose_goal_memory(goal: &Goal, todo: &Todo) -> String {
    let mut blocks: Vec<String> = vec![];

    // 1. Current todo failure context (most relevant to THIS turn).
    let failures: Vec<&RunRecord> = goal
        .history
        .iter()
        .filter(|r| r.todo_id == todo.id && run_failed(r))
        .collect();
    if !failures.is_empty() {
        let last = failures.last().unwrap();
        let mut line = format!(
            "prior attempts on this todo: {} failed; last = {}",
            failures.len(),
            failure_label(last)
        );
        if let Some(err) = &last.error {
            line.push_str(&format!(": {}", truncate(err, GOAL_MEMORY_ERROR_CHARS)));
        } else if let Some(v) = &last.validation {
            if !v.ok {
                line.push_str(&format!(
                    ": verify gate {} rejected (exit {})",
                    v.validator_kind,
                    v.exit_code.unwrap_or(-1)
                ));
            }
        }
        if let Some(cmd) = &todo.validator {
            line.push_str(&format!("; --verify {cmd} must exit 0 to complete"));
        }
        blocks.push(line);
    }

    // 2. Recent semantic history (goal-level, newest at the bottom).
    if !goal.semantic_history.is_empty() {
        let mut lines: Vec<String> = goal
            .semantic_history
            .iter()
            .rev()
            .take(GOAL_MEMORY_SEMANTIC_EVENTS)
            .map(|e| match &e.todo_id {
                Some(id) => format!("- {} [{}] {}", e.kind, id, e.summary),
                None => format!("- {} {}", e.kind, e.summary),
            })
            .collect();
        lines.reverse();
        blocks.push(format!("recent goal history:\n{}", lines.join("\n")));
    }

    if blocks.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str("\nPrior activity:\n");
    for b in blocks {
        out.push_str(&b);
        out.push('\n');
    }
    out
}

/// A turn counts as failed when its writeback classification is not
/// `None`; legacy records without a classification fall back to
/// `terminal_state != "completed"`.
fn run_failed(r: &RunRecord) -> bool {
    match r.failure_kind {
        Some(FailureKind::None) => false,
        Some(_) => true,
        None => r.terminal_state != "completed",
    }
}

/// Human-readable failure label for the last attempt (writeback classification
/// first, then a legacy fallback).
fn failure_label(r: &RunRecord) -> String {
    match r.failure_kind {
        Some(kind) => kind.label().to_string(),
        None => format!("legacy `{}` (unclassified)", r.terminal_state),
    }
}

/// Compose the per-turn packet: todo + resolved gate decisions + prior
/// evidence (+ decision summary when available).
///
/// P0 signature (executor + worker paths) — delegates to the full envelope
/// without a decision summary so existing call sites keep working.
pub fn compose_turn_message(goal: &Goal, todo: &Todo, prev: Option<&RunRecord>) -> String {
    compose_turn_envelope(goal, todo, None, prev)
}

/// Full envelope: schema header + decision summary (mode/decision/reason) +
/// goal context + instruction + resolved gate decisions + prior evidence +
/// completion-contract footer.
pub fn compose_turn_envelope(
    goal: &Goal,
    todo: &Todo,
    decision_summary: Option<&ShouldRunPacket>,
    prev: Option<&RunRecord>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("── {TURN_ENVELOPE_SCHEMA_VERSION} ──\n"));
    if let Some(p) = decision_summary {
        out.push_str(&format!(
            "decision: {} | should_run: {} | mode: {}\n",
            p.decision,
            p.should_run,
            p.interaction_contract.mode.as_str()
        ));
        out.push_str(&format!("reason: {}\n", p.reason));
        if let Some(arb) = &p.scheduler_arbitration {
            out.push_str(&format!("arbitration: {}\n", arb.disposition.as_str()));
        }
    } else {
        out.push_str("decision: bounded_delivery\n");
    }
    out.push_str(&format!(
        "goal: {} | objective: {}\n",
        goal.goal_id, goal.objective
    ));
    out.push_str(&format!(
        "execution: cadence={} spend_rule={}\n",
        goal.execution_profile.cadence, goal.execution_profile.spend_rule
    ));
    out.push('\n');

    // Instruction.
    out.push_str(&format!("TODO {}: {}\n", todo.id, todo.text));

    // Context: resolved gate decisions flow into blocked todos' packets.
    if let Some(gate_ids) = todo.blocked_by_gate.as_deref() {
        let decisions: Vec<String> = gate_ids
            .split(',')
            .filter_map(|gid| goal.todo(gid))
            .filter(|g| g.status == TodoStatus::Done)
            .filter_map(|g| g.decision.clone().map(|d| format!("{}: {}", g.id, d)))
            .collect();
        if !decisions.is_empty() {
            out.push_str(&format!(
                "\nResolved gate decision(s): {}\n",
                decisions.join("; ")
            ));
        }
    }

    // Evidence from the previous turn.
    if let Some(p) = prev {
        out.push_str(&format!(
            "\nEvidence from the previous turn (todo {}):\n{}",
            p.todo_id,
            truncate(&p.evidence, 1_200)
        ));
    }

    // Goal memory: prior failures on this todo + recent semantic history.
    out.push_str(&compose_goal_memory(goal, todo));

    // Completion contract footer (LoopX: completion must declare closure intent).
    out.push_str("\n\nComplete the todo and report what you did and observed.");
    out.push_str("\nOn completion, declare the successor todo or --no-follow-up.");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::TurnMode;
    use crate::state::Todo;
    use std::time::Duration;

    #[test]
    fn envelope_carries_schema_and_instruction() {
        let mut g = Goal::new("g1", "Ship the thing", "/tmp");
        g.add(Todo::advancement("T1", "Do the work"));
        let msg = compose_turn_message(&g, g.todo("T1").unwrap(), None);
        assert!(msg.contains(TURN_ENVELOPE_SCHEMA_VERSION));
        assert!(msg.contains("TODO T1: Do the work"));
        assert!(msg.contains("Complete the todo and report what you did and observed."));
        assert!(
            msg.contains("--no-follow-up"),
            "completion contract footer present"
        );
        assert!(msg.contains("objective: Ship the thing"));
    }

    #[test]
    fn envelope_includes_resolved_gate_decisions() {
        let mut g = Goal::new("g1", "o", "/tmp");
        g.add(Todo::user_gate("G1", "Approve X", &["T2"]));
        g.todo_mut("G1").unwrap().decision = Some("approved".to_string());
        g.todo_mut("G1").unwrap().status = TodoStatus::Done;
        g.add(Todo::advancement("T2", "Blocked work").blocking(&["G1"]));
        let msg = compose_turn_message(&g, g.todo("T2").unwrap(), None);
        assert!(msg.contains("Resolved gate decision(s): G1: approved"));
    }

    #[test]
    fn envelope_includes_prior_evidence() {
        let mut g = Goal::new("g1", "o", "/tmp");
        g.add(Todo::advancement("T1", "Work"));
        let prev = RunRecord {
            turn: 1,
            todo_id: "T0".to_string(),
            run_id: "run-1".to_string(),
            terminal_state: "completed".to_string(),
            error: None,
            tokens_in_delta: 0,
            tokens_out_delta: 0,
            cost_delta: 0.0,
            tools: vec![],
            evidence: "previous evidence payload".to_string(),
            recorded_at: 0,
            spend_source: Some("run".to_string()),
            validation: None,
            failure_kind: None,
            truncation: None,
        };
        let msg = compose_turn_message(&g, g.todo("T1").unwrap(), Some(&prev));
        assert!(msg.contains("Evidence from the previous turn (todo T0)"));
        assert!(msg.contains("previous evidence payload"));
    }

    #[test]
    fn envelope_with_decision_summary_embeds_packet() {
        let mut g = Goal::new("g1", "o", "/tmp");
        g.add(Todo::monitor("M1", "Watch", Duration::from_secs(3600)));
        g.todo_mut("M1").unwrap().resume_when =
            Some(std::time::SystemTime::now() - Duration::from_secs(5));
        let packet = crate::decision::decide(&g, std::time::SystemTime::now());
        assert_eq!(packet.interaction_contract.mode, TurnMode::MonitorPoll);
        let msg = compose_turn_envelope(&g, g.todo("M1").unwrap(), Some(&packet), None);
        assert!(msg.contains("decision: run"), "envelope: {msg}");
        assert!(msg.contains("should_run: true"));
        assert!(msg.contains("mode: monitor_poll"));
        assert!(
            msg.contains("arbitration:"),
            "scheduler arbitration recorded"
        );
    }

    fn run_rec(
        todo_id: &str,
        terminal_state: &str,
        failure_kind: FailureKind,
        error: Option<&str>,
        validation: Option<crate::state::TaskValidation>,
    ) -> RunRecord {
        RunRecord {
            turn: 1,
            todo_id: todo_id.to_string(),
            run_id: format!("run-{todo_id}"),
            terminal_state: terminal_state.to_string(),
            error: error.map(str::to_string),
            tokens_in_delta: 0,
            tokens_out_delta: 0,
            cost_delta: 0.0,
            tools: vec![],
            evidence: String::new(),
            recorded_at: 0,
            spend_source: Some("run".to_string()),
            validation,
            failure_kind: Some(failure_kind),
            truncation: None,
        }
    }

    #[test]
    fn goal_memory_surfaces_prior_failures_with_classification() {
        let mut g = Goal::new("g1", "o", "/tmp");
        g.add(Todo::advancement("T1", "Work"));
        g.history.push(run_rec(
            "T1",
            "error",
            FailureKind::HardError,
            Some("validator exited 1"),
            None,
        ));
        let msg = compose_turn_message(&g, g.todo("T1").unwrap(), None);
        assert!(msg.contains("Prior activity:"), "envelope: {msg}");
        assert!(
            msg.contains("prior attempts on this todo: 1 failed"),
            "envelope: {msg}"
        );
        assert!(msg.contains("hard error"), "envelope: {msg}");
        assert!(msg.contains("validator exited 1"), "envelope: {msg}");
    }

    #[test]
    fn goal_memory_surfaces_verify_gate_rejection() {
        let mut g = Goal::new("g1", "o", "/tmp");
        g.add(Todo::advancement("T1", "Work"));
        g.todo_mut("T1").unwrap().validator = Some("sh -c test".to_string());
        g.history.push(run_rec(
            "T1",
            "completed",
            FailureKind::ScienceVerifyFailed,
            None,
            Some(crate::state::task_validation_receipt(
                crate::state::ValidationStatus::Failed,
                "sh -c test",
                "failed",
                None,
                Some(1),
            )),
        ));
        let msg = compose_turn_message(&g, g.todo("T1").unwrap(), None);
        assert!(
            msg.contains("verify-gate rejected the output"),
            "envelope: {msg}"
        );
        assert!(
            msg.contains("verify gate sh -c test rejected"),
            "envelope: {msg}"
        );
        assert!(
            msg.contains("--verify sh -c test must exit 0 to complete"),
            "envelope: {msg}"
        );
    }

    #[test]
    fn goal_memory_surfaces_semantic_history() {
        let mut g = Goal::new("g1", "o", "/tmp");
        g.add(Todo::advancement("T1", "Work"));
        g.record_semantic_event("todo_completed", Some("T0"), "finished the setup", 1);
        g.record_semantic_event("run_landed", Some("T1"), "started T1", 2);
        let msg = compose_turn_message(&g, g.todo("T1").unwrap(), None);
        assert!(msg.contains("recent goal history:"), "envelope: {msg}");
        assert!(
            msg.contains("todo_completed [T0] finished the setup"),
            "envelope: {msg}"
        );
        assert!(
            msg.contains("run_landed [T1] started T1"),
            "envelope: {msg}"
        );
    }

    #[test]
    fn goal_memory_empty_when_nothing_to_surface() {
        let mut g = Goal::new("g1", "o", "/tmp");
        g.add(Todo::advancement("T1", "Work"));
        let msg = compose_turn_message(&g, g.todo("T1").unwrap(), None);
        assert!(!msg.contains("Prior activity:"), "envelope: {msg}");
    }
}
