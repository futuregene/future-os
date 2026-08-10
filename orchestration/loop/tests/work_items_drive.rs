//! Coverage drive for `work_items/`: attention queue branches, delivery
//! outcome/scale classifiers, and the operator-inbox attention kinds.

use future_loop::state::{Goal, Todo};
use future_loop::work_items::attention::{
    build_attention_queue, goal_attention_item, AttentionItem, AttentionWaitingOn,
};
use future_loop::work_items::delivery::{
    delivery_batch_scale_for_run, delivery_outcome_for_run, is_progress_outcome,
    outcome_floor_configured, outcome_gap_streak, small_delivery_batch_scale_streak,
    DeliveryBatchScale, DeliveryOutcome,
};
use future_loop::work_items::operator_inbox::{
    operator_inbox_attention_kind, project_operator_inbox_urgency, OperatorAttentionKind,
    OperatorInboxConfig, OperatorInboxEvent,
};

use common::run_record;
mod common;

// ── attention ──────────────────────────────────────────────────────────────

#[test]
fn attention_labels_and_queue() {
    for (w, label) in [
        (AttentionWaitingOn::Codex, "codex"),
        (AttentionWaitingOn::UserOrController, "user_or_controller"),
        (AttentionWaitingOn::Controller, "controller"),
        (AttentionWaitingOn::ExternalEvidence, "external_evidence"),
        (AttentionWaitingOn::Monitor, "monitor_signal"),
    ] {
        assert_eq!(w.label(), label);
    }
    let item = |waiting_on: &str| AttentionItem {
        goal_id: "g".into(),
        status: "s".into(),
        waiting_on: waiting_on.into(),
        severity: "info".into(),
        recommended_action: "act".into(),
        source: "test".into(),
    };
    let queue = build_attention_queue(vec![
        item("user_or_controller"),
        item("controller"),
        item("codex"),
        item("monitor_signal"),
        item("something_else"),
    ]);
    assert_eq!(queue.item_count, 5);
    assert_eq!(queue.needs_user_or_controller, 1);
    assert_eq!(queue.needs_controller, 1);
    assert_eq!(queue.needs_codex, 1);
    assert_eq!(queue.watching_monitor, 1);
}

#[test]
fn goal_attention_item_branches() {
    // Gate with a question → high-severity gate item.
    let mut goal = Goal::new("g1", "attn", "/tmp");
    goal.todos
        .push(Todo::user_gate("tg", "approve the plan?", &[]));
    let item = goal_attention_item(&goal).unwrap();
    assert_eq!(item.status, "operator_gate");
    assert!(item.recommended_action.contains("approve the plan?"));
    // Gate without a question → default text arm.
    let mut goal = Goal::new("g1", "attn", "/tmp");
    let mut g = Todo::user_gate("tg", "q?", &[]);
    g.gate_question = None;
    goal.todos.push(g);
    let item = goal_attention_item(&goal).unwrap();
    assert!(item
        .recommended_action
        .contains("resolve the open user gate"));
    // Projection gap → codex self-repair item (goal must be non-terminal:
    // an open user_action keeps it open while leaving agent_open == 0).
    let mut goal = Goal::new("g1", "attn", "/tmp");
    goal.todos
        .push(Todo::user_action("ua", "pending user action"));
    goal.next_action = Some("totally stale action".into());
    let item = goal_attention_item(&goal).unwrap();
    assert_eq!(item.status, "projection_gap");
    // Open advancement → codex advancement item.
    let mut goal = Goal::new("g1", "attn", "/tmp");
    goal.todos.push(Todo::advancement("t1", "do work"));
    goal.next_action = Some("do work".into());
    let item = goal_attention_item(&goal).unwrap();
    assert_eq!(item.status, "advancement_open");
    // Due monitor → monitor item (a user_action keeps the goal non-terminal
    // without an open advancement, and a "waiting" next-action avoids a
    // projection gap).
    let mut goal = Goal::new("g1", "attn", "/tmp");
    goal.todos
        .push(Todo::user_action("ua", "pending user action"));
    goal.todos.push(Todo::monitor(
        "m1",
        "watch it",
        std::time::Duration::from_secs(0),
    ));
    goal.next_action = Some("waiting on the user".into());
    let item = goal_attention_item(&goal).unwrap();
    assert_eq!(item.status, "monitor_due");
    // Nothing actionable → None.
    let mut goal = Goal::new("g1", "attn", "/tmp");
    goal.next_action = Some("all todos complete; no further action".into());
    assert!(goal_attention_item(&goal).is_none());
}

// ── delivery ───────────────────────────────────────────────────────────────

#[test]
fn delivery_labels_and_classifiers() {
    for (s, label) in [
        (DeliveryBatchScale::TestOnly, "test_only"),
        (DeliveryBatchScale::SingleSurface, "single_surface"),
        (DeliveryBatchScale::MultiSurface, "multi_surface"),
        (DeliveryBatchScale::Implementation, "implementation"),
        (DeliveryBatchScale::Unknown, "unknown"),
    ] {
        assert_eq!(s.label(), label);
    }
    for (o, label) in [
        (DeliveryOutcome::OutcomeProgress, "outcome_progress"),
        (DeliveryOutcome::SurfaceOnly, "surface_only"),
        (DeliveryOutcome::OutcomeGap, "outcome_gap"),
        (DeliveryOutcome::NotConfigured, "not_configured"),
        (DeliveryOutcome::Unknown, "unknown"),
    ] {
        assert_eq!(o.label(), label);
    }
    assert!(is_progress_outcome(DeliveryOutcome::OutcomeProgress));
    assert!(!is_progress_outcome(DeliveryOutcome::SurfaceOnly));

    // Batch scale from evidence text.
    let scale = |evidence: &str| {
        let mut r = run_record("t", "completed", 1);
        r.evidence = evidence.to_string();
        delivery_batch_scale_for_run(&r)
    };
    assert_eq!(scale("unit test passed"), DeliveryBatchScale::TestOnly);
    assert_eq!(
        scale("changes across surfaces"),
        DeliveryBatchScale::MultiSurface
    );
    assert_eq!(
        scale("implementation landed"),
        DeliveryBatchScale::Implementation
    );
    // NOTE: the Unknown arm is unreachable through delivery_batch_scale_for_run
    // (evidence_text always contains the "<state> " separator); a non-empty
    // non-matching text is a single surface.
    assert_eq!(scale(""), DeliveryBatchScale::SingleSurface);
    assert_eq!(scale("one file edited"), DeliveryBatchScale::SingleSurface);

    // Outcome classification against markers.
    let markers = vec!["shipped".to_string()];
    let surface = vec!["docs only".to_string()];
    let outcome = |evidence: &str, m: &[String], s: &[String]| {
        let mut r = run_record("t", "completed", 1);
        r.evidence = evidence.to_string();
        delivery_outcome_for_run(&r, m, s)
    };
    assert_eq!(
        outcome("anything", &[], &[]),
        DeliveryOutcome::NotConfigured
    );
    assert_eq!(
        outcome("docs only tweak", &markers, &surface),
        DeliveryOutcome::SurfaceOnly
    );
    assert_eq!(
        outcome("shipped it", &markers, &surface),
        DeliveryOutcome::OutcomeProgress
    );
    assert_eq!(
        outcome("nothing relevant", &markers, &surface),
        DeliveryOutcome::OutcomeGap
    );
    assert!(outcome_floor_configured(&markers, &[]));
    assert!(!outcome_floor_configured(&[], &[]));

    // Streaks over run history.
    let mut gap_run = || {
        let mut r = run_record("t", "completed", 1);
        r.evidence = "nothing relevant".to_string();
        r
    };
    let mut good = run_record("t", "completed", 1);
    good.evidence = "shipped it".to_string();
    let runs = vec![gap_run(), gap_run(), good.clone(), gap_run()];
    // Leading streak (callers pass newest-first runs): stops at the first
    // progress run.
    assert_eq!(outcome_gap_streak(&runs, &markers, &surface), 2);
    let all_gaps = vec![gap_run(), gap_run()];
    assert_eq!(outcome_gap_streak(&all_gaps, &markers, &surface), 2);
    // Not-configured floor → gap streak treats runs as gaps? probe both.
    let _ = outcome_gap_streak(&runs, &[], &[]);
    // Small-batch streak (SingleSurface/TestOnly count; larger scales break).
    let mut small = run_record("t", "completed", 1);
    small.evidence = "unit test ok".to_string();
    let mut big = run_record("t", "completed", 1);
    big.evidence = "changes across surfaces".to_string();
    assert_eq!(
        small_delivery_batch_scale_streak(&[small.clone(), small.clone()]),
        2
    );
    assert_eq!(small_delivery_batch_scale_streak(&[small, big]), 1);
}

// ── operator inbox ─────────────────────────────────────────────────────────

fn inbox_event(content: &str, verified: bool, reply: bool) -> OperatorInboxEvent {
    OperatorInboxEvent {
        message_id: "m".into(),
        create_time: "2026-08-10T00:00:00Z".into(),
        content: content.to_string(),
        reply_context_verified: verified,
        reply_to_operator: reply,
    }
}

#[test]
fn operator_inbox_kinds() {
    for (k, label) in [
        (OperatorAttentionKind::DirectQuestion, "direct_question"),
        (OperatorAttentionKind::DirectMention, "direct_mention"),
        (OperatorAttentionKind::ReplyToOperator, "reply_to_operator"),
    ] {
        assert_eq!(k.label(), label);
    }
    // Verified reply always counts (any scope).
    assert_eq!(
        operator_inbox_attention_kind(&inbox_event("ok", true, true), "operator", "addressed_only"),
        Some(OperatorAttentionKind::ReplyToOperator)
    );
    // Question mark (ASCII + full-width).
    assert_eq!(
        operator_inbox_attention_kind(
            &inbox_event("@operator ready?", false, false),
            "operator",
            "addressed_only"
        ),
        Some(OperatorAttentionKind::DirectQuestion)
    );
    assert_eq!(
        operator_inbox_attention_kind(
            &inbox_event("@operator ready？", false, false),
            "operator",
            "addressed_only"
        ),
        Some(OperatorAttentionKind::DirectQuestion)
    );
    // Mention without question.
    assert_eq!(
        operator_inbox_attention_kind(
            &inbox_event("@operator FYI", false, false),
            "operator",
            "addressed_only"
        ),
        Some(OperatorAttentionKind::DirectMention)
    );
    // future-loop mention.
    assert_eq!(
        operator_inbox_attention_kind(
            &inbox_event("@future-loop ping", false, false),
            "operator",
            "addressed_only"
        ),
        Some(OperatorAttentionKind::DirectMention)
    );
    // Scope semantics (as implemented, mirroring the reference): in
    // configured_chat_all a message WITHOUT any mention is dropped; in
    // addressed_only a bare question still counts (the '?' heuristic).
    assert_eq!(
        operator_inbox_attention_kind(
            &inbox_event("random chatter", false, false),
            "operator",
            "addressed_only"
        ),
        None
    );
    assert_eq!(
        operator_inbox_attention_kind(
            &inbox_event("random chatter", false, false),
            "operator",
            "configured_chat_all"
        ),
        None
    );
    assert_eq!(
        operator_inbox_attention_kind(
            &inbox_event("is this on?", false, false),
            "operator",
            "configured_chat_all"
        ),
        None
    );
    assert_eq!(
        operator_inbox_attention_kind(
            &inbox_event("is this on?", false, false),
            "operator",
            "addressed_only"
        ),
        Some(OperatorAttentionKind::DirectQuestion)
    );
    // Unverified reply flag without verification → not a reply.
    assert_eq!(
        operator_inbox_attention_kind(
            &inbox_event("no marks", false, true),
            "operator",
            "addressed_only"
        ),
        None
    );
    // Empty operator name: no explicit mention arm.
    assert_eq!(
        operator_inbox_attention_kind(
            &inbox_event("@someone hi", false, false),
            "",
            "addressed_only"
        ),
        None
    );
}

#[test]
fn operator_inbox_urgency_projection() {
    let config = |enabled: bool, scope: &str| OperatorInboxConfig {
        enabled,
        capture_scope: scope.to_string(),
        inbox_dir: "inbox".to_string(),
        operator_display_name: "operator".to_string(),
        reply_enabled: true,
    };
    // Disabled → zeroed projection.
    let u = project_operator_inbox_urgency(
        &config(false, "addressed_only"),
        &[inbox_event("@operator q?", false, false)],
    );
    assert!(!u.enabled);
    assert_eq!(u.pending_count, 0);
    // Enabled with a mix.
    let pending = vec![
        inbox_event("@operator ready?", false, false),
        inbox_event("@operator note", false, false),
        inbox_event("replied", true, true),
        inbox_event("noise", false, false),
    ];
    let u = project_operator_inbox_urgency(&config(true, "addressed_only"), &pending);
    assert_eq!(u.pending_count, 4);
    assert_eq!(u.direct_question_count, 1);
    assert_eq!(u.direct_mention_count, 1);
    assert_eq!(u.reply_to_operator_count, 1);
    assert!(u.attention_required_count >= 3);
    assert!(u.reply_due);
    // reply_due = any attention required + replies enabled.
    let u = project_operator_inbox_urgency(&config(true, "addressed_only"), &[]);
    assert!(!u.reply_due, "no pending events");
    let mut no_reply = config(true, "addressed_only");
    no_reply.reply_enabled = false;
    let u = project_operator_inbox_urgency(&no_reply, &[inbox_event("@operator q?", false, false)]);
    assert!(!u.reply_due, "replies disabled");
}
