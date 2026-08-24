//! P0 contract tests: oscillation detection (LoopX 对比改进项 ③) — the
//! projection-layer sliding-window signature-pair guard. A goal whose
//! delivery outcomes strictly alternate accept/reject (A→V→A→V) is burning
//! spend without converging; the kernel must convert the next delivery
//! into a replan (frontier delta), then let delivery resume once the agent
//! ACKs. Deterministic: signatures derive from the run-history projection.

use std::time::SystemTime;

use future_loop::contract::TurnMode;
use future_loop::decision::decide;
use future_loop::state::{
    task_validation_receipt, Goal, RecoveryKind, ReplanAck, RunRecord, Todo, ValidationStatus,
};

/// A completed delivery turn; `validation_ok = Some(false)` carries a failed
/// independent-validator receipt (V), otherwise the outcome is accepted (A).
fn delivery(turn: u32, ts: u64, validation_ok: Option<bool>) -> RunRecord {
    RunRecord {
        turn,
        todo_id: format!("t{turn}"),
        run_id: format!("run-{turn}"),
        terminal_state: "completed".to_string(),
        error: None,
        tokens_in_delta: 0,
        tokens_out_delta: 0,
        cost_delta: 0.0,
        tools: vec!["edit".to_string()],
        evidence: "artifact".to_string(),
        recorded_at: ts,
        spend_source: Some("run".to_string()),
        validation: validation_ok.map(|ok| {
            if ok {
                task_validation_receipt(ValidationStatus::Passed, "make test", "ok", None, Some(0))
            } else {
                task_validation_receipt(
                    ValidationStatus::Failed,
                    "make test",
                    "validator exited 1 — repair required",
                    Some(RecoveryKind::RepairRequired),
                    Some(1),
                )
            }
        }),
        failure_kind: None,
    }
}

fn accepted(turn: u32, ts: u64) -> RunRecord {
    delivery(turn, ts, None)
}

fn rejected(turn: u32, ts: u64) -> RunRecord {
    delivery(turn, ts, Some(false))
}

fn goal_with_history(history: Vec<RunRecord>) -> Goal {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("t-open", "Keep delivering"));
    goal.history = history;
    goal
}

// ── Detection fires on the A→V→A→V pattern ────────────────────────────────
#[test]
fn oscillation_surfaces_as_a_signal_not_a_replan() {
    // ARCHITECTURE-SIMPLIFICATION: oscillation is a signal the agent reads,
    // surfaced in the delivery reason — NOT a kernel-forced replan.
    let goal = goal_with_history(vec![
        accepted(1, 1),
        rejected(2, 2),
        accepted(3, 3),
        rejected(4, 4),
    ]);
    let p = decide(&goal, SystemTime::now());
    assert_eq!(
        p.interaction_contract.mode,
        TurnMode::BoundedDelivery,
        "A→V→A→V surfaces as an advisory, delivery continues"
    );
    assert!(p.reason.contains("oscillation"), "{}", p.reason);
    assert!(p.reason.contains("A→V→A→V"), "{}", p.reason);
    assert!(p.normal_delivery_allowed);
}

#[test]
fn v_first_alternation_also_surfaces() {
    let goal = goal_with_history(vec![
        rejected(1, 1),
        accepted(2, 2),
        rejected(3, 3),
        accepted(4, 4),
    ]);
    let p = decide(&goal, SystemTime::now());
    assert_eq!(p.interaction_contract.mode, TurnMode::BoundedDelivery);
    assert!(p.reason.contains("V→A→V→A"), "{}", p.reason);
}

// ── Below the pattern length the guard stays silent ───────────────────────
#[test]
fn shorter_alternation_still_delivers() {
    // One pair plus a lead-in (len 3 < OSCILLATION_PATTERN_LEN): no signal.
    let goal = goal_with_history(vec![accepted(1, 1), rejected(2, 2), accepted(3, 3)]);
    let p = decide(&goal, SystemTime::now());
    assert_eq!(p.interaction_contract.mode, TurnMode::BoundedDelivery);
    assert!(!p.reason.contains("oscillation"), "{}", p.reason);
}

#[test]
fn consecutive_rejects_break_the_pattern() {
    // A,V,A,V,V — the same-symbol pair at the tail ends the alternation.
    let goal = goal_with_history(vec![
        accepted(1, 1),
        rejected(2, 2),
        accepted(3, 3),
        rejected(4, 4),
        rejected(5, 5),
    ]);
    let p = decide(&goal, SystemTime::now());
    assert_eq!(p.interaction_contract.mode, TurnMode::BoundedDelivery);
    assert!(!p.reason.contains("oscillation"), "{}", p.reason);
}

#[test]
fn stabilized_tail_clears_an_earlier_pattern() {
    // A,V,A,V,A,A — oscillated, then two consecutive accepts stabilized.
    let goal = goal_with_history(vec![
        accepted(1, 1),
        rejected(2, 2),
        accepted(3, 3),
        rejected(4, 4),
        accepted(5, 5),
        accepted(6, 6),
    ]);
    let p = decide(&goal, SystemTime::now());
    assert_eq!(p.interaction_contract.mode, TurnMode::BoundedDelivery);
    assert!(!p.reason.contains("oscillation"), "{}", p.reason);
}

// ── Non-delivery records are transparent to the detector ──────────────────
#[test]
fn monitor_polls_and_failed_turns_neither_fabricate_nor_break() {
    let mut poll = accepted(2, 2);
    poll.spend_source = Some("heartbeat".to_string());
    let mut crashed = accepted(4, 4);
    crashed.terminal_state = "failed".to_string();
    crashed.spend_source = Some("run".to_string());
    let goal = goal_with_history(vec![
        accepted(1, 1),
        poll,
        rejected(3, 3),
        crashed,
        accepted(5, 5),
        rejected(6, 6),
    ]);
    let p = decide(&goal, SystemTime::now());
    assert_eq!(p.interaction_contract.mode, TurnMode::BoundedDelivery);
    assert!(
        p.reason.contains("oscillation"),
        "heartbeat polls and crashed turns must be transparent: A,_,V,_,A,V still alternates — {}",
        p.reason
    );
}

// ── The replan ACK consumes the pattern (liveness) ─────────────────────────
#[test]
fn replan_ack_consumes_the_pattern_and_signal_clears() {
    let mut goal = goal_with_history(vec![
        accepted(1, 1),
        rejected(2, 2),
        accepted(3, 3),
        rejected(4, 4),
    ]);
    assert!(decide(&goal, SystemTime::now())
        .reason
        .contains("oscillation"));
    // The agent records a frontier-changing delta (ACK at ts=10); every
    // alternating record predates the ACK, so the pattern is consumed and the
    // oscillation signal clears from the delivery reason.
    goal.replan_ack = Some(ReplanAck {
        recorded: true,
        delta_kinds: vec!["vision_patch".to_string()],
        at: 10,
    });
    let p = decide(&goal, SystemTime::now());
    assert_eq!(p.interaction_contract.mode, TurnMode::BoundedDelivery);
    assert!(!p.reason.contains("oscillation"), "{}", p.reason);
}

#[test]
fn fresh_post_ack_alternation_surfaces_again() {
    let mut goal = goal_with_history(vec![
        accepted(1, 1),
        rejected(2, 2),
        accepted(3, 3),
        rejected(4, 4),
    ]);
    goal.replan_ack = Some(ReplanAck {
        recorded: true,
        delta_kinds: vec!["vision_patch".to_string()],
        at: 10,
    });
    // One fresh pair post-ACK: not enough.
    goal.history.push(accepted(5, 11));
    goal.history.push(rejected(6, 12));
    assert!(!decide(&goal, SystemTime::now())
        .reason
        .contains("oscillation"));
    // Two full fresh pairs: the loop survived the replan — signal again.
    goal.history.push(accepted(7, 13));
    goal.history.push(rejected(8, 14));
    let p = decide(&goal, SystemTime::now());
    assert_eq!(p.interaction_contract.mode, TurnMode::BoundedDelivery);
    assert!(p.reason.contains("oscillation"), "{}", p.reason);
}

// ── The guard only blocks delivery, never other modes ─────────────────────
#[test]
fn oscillation_does_not_block_validated_closure() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("t1", "Work"));
    goal.todo_mut("t1").unwrap().complete(true, vec![]);
    goal.history = vec![
        accepted(1, 1),
        rejected(2, 2),
        accepted(3, 3),
        rejected(4, 4),
    ];
    let p = decide(&goal, SystemTime::now());
    assert_eq!(
        p.interaction_contract.mode,
        TurnMode::Terminal,
        "no runnable work → nothing to defend; closure must proceed"
    );
}
