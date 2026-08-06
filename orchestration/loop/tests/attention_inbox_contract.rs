//! G-15 attention / operator-inbox / delivery-signal contract tests: the
//! attention queue projection, the operator inbox urgency read model (content
//! never returned), and the delivery signals (batch scale / outcome / streaks)
//! that feed the handoff delivery contract.

use future_loop::state::{Goal, RunRecord, Todo};
use future_loop::work_items::attention::{
    build_attention_queue, goal_attention_item, AttentionItem,
};
use future_loop::work_items::delivery::{
    delivery_batch_scale_for_run, delivery_outcome_for_run, outcome_gap_streak,
    small_delivery_batch_scale_streak, DeliveryBatchScale, DeliveryOutcome,
};
use future_loop::work_items::operator_inbox::{
    load_pending_inbox_events, operator_inbox_attention_kind, project_operator_inbox_urgency,
    OperatorAttentionKind, OperatorInboxConfig, OperatorInboxEvent,
};

fn run(evidence: &str) -> RunRecord {
    RunRecord {
        turn: 1,
        todo_id: "t1".into(),
        run_id: "r".into(),
        terminal_state: "completed".into(),
        error: None,
        tokens_in_delta: 0,
        tokens_out_delta: 0,
        cost_delta: 0.0,
        tools: vec![],
        evidence: evidence.into(),
        recorded_at: 0,
        spend_source: None,
        validation: None,
    }
}

/// ── Attention queue ───────────────────────────────────────────────────────
#[test]
fn attention_queue_counts_by_routing() {
    let items = vec![
        AttentionItem {
            goal_id: "g1".into(),
            status: "operator_gate".into(),
            waiting_on: "user_or_controller".into(),
            severity: "high".into(),
            recommended_action: "decide".into(),
            source: "goal_attention".into(),
        },
        AttentionItem {
            goal_id: "g2".into(),
            status: "advancement_open".into(),
            waiting_on: "codex".into(),
            severity: "action".into(),
            recommended_action: "advance".into(),
            source: "goal_attention".into(),
        },
        AttentionItem {
            goal_id: "g3".into(),
            status: "monitor_due".into(),
            waiting_on: "monitor_signal".into(),
            severity: "info".into(),
            recommended_action: "poll".into(),
            source: "goal_attention".into(),
        },
    ];
    let queue = build_attention_queue(items);
    assert_eq!(queue.item_count, 3);
    assert_eq!(queue.needs_user_or_controller, 1);
    assert_eq!(queue.needs_codex, 1);
    assert_eq!(queue.watching_monitor, 1);
    assert_eq!(queue.needs_controller, 0);
}

#[test]
fn goal_attention_routing() {
    let mut gate_goal = Goal::new("g1", "obj", "/tmp");
    gate_goal.todos = vec![Todo::user_gate("g1", "approve?", &[])];
    let item = goal_attention_item(&gate_goal).unwrap();
    assert_eq!(item.waiting_on, "user_or_controller");
    assert_eq!(item.severity, "high");

    let mut work_goal = Goal::new("g2", "obj", "/tmp");
    work_goal.todos = vec![Todo::advancement("t1", "work")];
    let item = goal_attention_item(&work_goal).unwrap();
    assert_eq!(item.waiting_on, "codex");

    // Terminal goal → no attention item.
    let mut done_goal = Goal::new("g3", "obj", "/tmp");
    done_goal.next_action = Some("complete; no further action".to_string());
    assert!(goal_attention_item(&done_goal).is_none());
}

/// ── Operator inbox urgency (content-free) ─────────────────────────────────
#[test]
fn operator_inbox_urgency_projection() {
    let config = OperatorInboxConfig {
        enabled: true,
        capture_scope: "addressed_only".into(),
        inbox_dir: "inbox".into(),
        operator_display_name: "operator".into(),
        reply_enabled: true,
    };
    let pending = vec![
        OperatorInboxEvent {
            message_id: "m1".into(),
            create_time: "2026-01-01T00:00:00Z".into(),
            content: "@operator 这个结论成立吗？".into(),
            reply_context_verified: false,
            reply_to_operator: false,
        },
        OperatorInboxEvent {
            message_id: "m2".into(),
            create_time: "2026-01-01T00:00:01Z".into(),
            content: "@operator ping".into(),
            reply_context_verified: false,
            reply_to_operator: false,
        },
        OperatorInboxEvent {
            message_id: "m3".into(),
            create_time: "2026-01-01T00:00:02Z".into(),
            content: "done".into(),
            reply_context_verified: true,
            reply_to_operator: true,
        },
    ];
    let urgency = project_operator_inbox_urgency(&config, &pending);
    assert_eq!(urgency.pending_count, 3);
    assert_eq!(urgency.direct_question_count, 1);
    assert_eq!(urgency.direct_mention_count, 1);
    assert_eq!(urgency.reply_to_operator_count, 1);
    assert_eq!(urgency.attention_required_count, 3);
    assert!(urgency.reply_due);
    assert!(
        !urgency.local_private_content_returned,
        "content must never be projected"
    );
}

#[test]
fn operator_inbox_attention_kind_classification() {
    let e = OperatorInboxEvent {
        message_id: "m".into(),
        create_time: "".into(),
        content: "@operator 怎么办？".into(),
        reply_context_verified: false,
        reply_to_operator: false,
    };
    assert_eq!(
        operator_inbox_attention_kind(&e, "operator", "addressed_only"),
        Some(OperatorAttentionKind::DirectQuestion)
    );
    // Unaddressed content is ignored in addressed_only scope.
    let e2 = OperatorInboxEvent {
        message_id: "m".into(),
        create_time: "".into(),
        content: "hello world".into(),
        reply_context_verified: false,
        reply_to_operator: false,
    };
    assert_eq!(
        operator_inbox_attention_kind(&e2, "operator", "addressed_only"),
        None
    );
}

#[test]
fn inbox_path_cannot_escape_project() {
    assert!(load_pending_inbox_events("/tmp", "../../etc").is_err());
    assert!(load_pending_inbox_events("/tmp", "/absolute/path").is_err());
}

/// ── Delivery signals ──────────────────────────────────────────────────────
#[test]
fn delivery_batch_scale_classification() {
    assert_eq!(
        delivery_batch_scale_for_run(&run("added a unit test only")),
        DeliveryBatchScale::TestOnly
    );
    assert_eq!(
        delivery_batch_scale_for_run(&run("changed docs and code across surfaces")),
        DeliveryBatchScale::MultiSurface
    );
    assert_eq!(
        delivery_batch_scale_for_run(&run("implemented the fix")),
        DeliveryBatchScale::Implementation
    );
    assert_eq!(
        delivery_batch_scale_for_run(&run("small tweak")),
        DeliveryBatchScale::SingleSurface
    );
}

#[test]
fn delivery_outcome_prefers_surface_hint_over_marker() {
    let markers = vec!["merged".to_string()];
    let surfaces = vec!["docs-only".to_string()];
    assert_eq!(
        delivery_outcome_for_run(&run("docs-only change, merged"), &markers, &surfaces),
        DeliveryOutcome::SurfaceOnly
    );
    assert_eq!(
        delivery_outcome_for_run(&run("real fix merged"), &markers, &surfaces),
        DeliveryOutcome::OutcomeProgress
    );
    assert_eq!(
        delivery_outcome_for_run(&run("nothing"), &markers, &surfaces),
        DeliveryOutcome::OutcomeGap
    );
    assert_eq!(
        delivery_outcome_for_run(&run("x"), &[], &[]),
        DeliveryOutcome::NotConfigured
    );
}

#[test]
fn outcome_gap_streak_breaks_on_progress() {
    let markers = vec!["merged".to_string()];
    let runs = vec![run("gap"), run("gap"), run("merged"), run("gap")];
    assert_eq!(outcome_gap_streak(&runs, &markers, &[]), 2);
    // Surface-only is a gap, not progress.
    let surfaces = vec!["docs-only".to_string()];
    let runs = vec![run("docs-only change"), run("docs-only again")];
    assert_eq!(outcome_gap_streak(&runs, &markers, &surfaces), 2);
    // No floor configured → no streak.
    assert_eq!(outcome_gap_streak(&runs, &[], &[]), 0);
}

#[test]
fn small_scale_streak_counts_consecutive_small_runs() {
    let runs = vec![
        run("small tweak"),
        run("unit test only"),
        run("implemented"),
    ];
    assert_eq!(small_delivery_batch_scale_streak(&runs), 2);
}
