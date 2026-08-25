//! O3 drive: idle-turn (no-progress) detection — pure evaluation, stream
//! tracking, and the budget-truncation ledger path. Detection + bookkeeping
//! only: no auto-injection (orchestrators nudge via a `todo update`).

mod common;

use common::mock_agent::{ev, spawn_mock, MockState};
use common::{cli_ok, cli_root, init_goal, open_store};
use future_loop::agent_client::{is_write_class_tool, TurnProgressTracker};
use future_loop::executor::no_progress_idle_secs;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn mock_env(state: MockState) -> (tokio::runtime::Runtime, common::mock_agent::SharedState) {
    let rt = rt();
    let (addr, shared) = rt.block_on(spawn_mock(state));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    (rt, shared)
}

// ── pure evaluation ────────────────────────────────────────────────────────

#[test]
fn no_progress_pure_read_sequence_breaches() {
    // Turn started 100s ago, only read-class tools → idle == turn age.
    let now = 1_700_000_100;
    let idle = no_progress_idle_secs(now - 100, None, now, 60).unwrap();
    assert_eq!(idle, 100);
}

#[test]
fn no_progress_pure_recent_write_does_not_breach() {
    let now = 1_700_000_100;
    // Last write 5s ago — inside the 60s window.
    assert_eq!(
        no_progress_idle_secs(now - 100, Some(now - 5), now, 60),
        None
    );
}

#[test]
fn no_progress_pure_old_write_breaches() {
    let now = 1_700_000_100;
    // Last write 61s ago — outside the 60s window → idle measured from it.
    let idle = no_progress_idle_secs(now - 200, Some(now - 61), now, 60).unwrap();
    assert_eq!(idle, 61);
}

#[test]
fn no_progress_pure_boundary_and_skew() {
    let now = 1_700_000_100;
    // idle == threshold → breach (>=).
    assert!(no_progress_idle_secs(now - 60, None, now, 60).is_some());
    // idle just under threshold → no breach.
    assert!(no_progress_idle_secs(now - 59, None, now, 60).is_none());
    // Clock skew (tool timestamp ahead of now) → idle 0 → no breach.
    assert!(no_progress_idle_secs(now, Some(now + 5), now, 60).is_none());
}

// ── write-class classification + tracker ───────────────────────────────────

#[test]
fn write_class_tool_matrix() {
    for t in ["write", "edit", "shell"] {
        assert!(is_write_class_tool(t), "{t} is write-class");
    }
    for t in ["read", "grep", "todo_update", "list", ""] {
        assert!(!is_write_class_tool(t), "{t} is not write-class");
    }
}

#[test]
fn tracker_records_write_class_only() {
    let t = TurnProgressTracker::new(1_000);
    t.observe_tool_start("read", 1_010);
    t.observe_tool_start("write", 1_020);
    t.observe_tool_start("grep", 1_030);
    t.observe_tool_start("edit", 1_040);
    let snap = t.snapshot();
    assert_eq!(snap.tool_calls_total, 4);
    assert_eq!(snap.last_write_tool_at, Some(1_040));
    assert_eq!(snap.turn_start_at, 1_000);

    // Read-only stream → no write marker, all calls still counted.
    let t = TurnProgressTracker::new(2_000);
    t.observe_tool_start("read", 2_010);
    t.observe_tool_start("grep", 2_020);
    let snap = t.snapshot();
    assert_eq!(snap.tool_calls_total, 2);
    assert_eq!(snap.last_write_tool_at, None);
}

// ── stream tracking through run_turn ───────────────────────────────────────

#[test]
fn run_turn_folds_tool_starts_into_tracker() {
    rt().block_on(async {
        let events = vec![
            ev("mine", 0, "tool_start", "{\"tool_name\":\"read\"}"),
            ev("mine", 1, "tool_start", "{\"tool_name\":\"shell\"}"),
            ev("mine", 2, "text_chunk", "{\"text\":\"x\"}"),
            ev("mine", 3, "agent_end", "{\"state\":\"completed\"}"),
        ];
        let (addr, _) = spawn_mock(MockState {
            events,
            ..Default::default()
        })
        .await;
        let mut client = future_loop::agent_client::AgentClient::connect(&addr)
            .await
            .unwrap();
        let progress = TurnProgressTracker::new(1);
        let summary = client
            .run_turn("sess", "mine", None, Some(&progress))
            .await
            .unwrap();
        assert_eq!(summary.terminal_state, "completed");
        let snap = progress.snapshot();
        assert_eq!(snap.tool_calls_total, 2);
        assert!(snap.last_write_tool_at.is_some(), "shell is write-class");
    });
}

// ── budget truncation → ledger event ───────────────────────────────────────

#[test]
fn budget_truncation_read_only_turn_records_turn_no_progress() {
    let cr = cli_root();
    let events = vec![ev(
        "mock-run-1",
        0,
        "tool_start",
        "{\"tool_name\":\"read\"}",
    )];
    let st = MockState {
        events,
        events_then_hang: true,
        ..Default::default()
    };
    let (_rt, _shared) = mock_env(st);
    // Shrink the idle window to 1s so the 2s budget truncation breaches it
    // (the env hook mirrors FUTURE_LOOP_AGENT_ADDR; default is 15 min).
    std::env::set_var("FUTURE_LOOP_NO_PROGRESS_SECS", "1");
    let goal = init_goal(&cr, "read-only hang");
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--anonymous",
        "--max-turn-secs",
        "2",
        "--max-turns",
        "3",
    ]);
    std::env::remove_var("FUTURE_LOOP_NO_PROGRESS_SECS");
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    assert_eq!(g.turn_no_progress.len(), 1, "{:?}", g.turn_no_progress);
    let np = &g.turn_no_progress[0];
    assert!(np.idle_secs >= 1, "idle {np:?}");
    assert_eq!(np.tool_calls_total, 1, "the read tool_start was observed");
    assert!(np.agent_id.is_none(), "anonymous run");
    // The ledger carries the real event (replay folds it, so this must hold).
    let text = std::fs::read_to_string(store.goal_dir(&goal).join("events.jsonl")).unwrap();
    assert!(text.contains("\"kind\":\"turn_no_progress\""), "{text}");
    // Status surfaces the breach (recent-last).
    cli_ok(&["status", "--goal", &goal]);
}

#[test]
fn budget_truncation_after_write_tool_no_event() {
    let cr = cli_root();
    let events = vec![ev(
        "mock-run-1",
        0,
        "tool_start",
        "{\"tool_name\":\"write\"}",
    )];
    let st = MockState {
        events,
        events_then_hang: true,
        ..Default::default()
    };
    let (_rt, _shared) = mock_env(st);
    // Window 5s: the 2s budget truncation ends well inside it → no breach.
    std::env::set_var("FUTURE_LOOP_NO_PROGRESS_SECS", "5");
    let goal = init_goal(&cr, "write then hang");
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--anonymous",
        "--max-turn-secs",
        "2",
        "--max-turns",
        "3",
    ]);
    std::env::remove_var("FUTURE_LOOP_NO_PROGRESS_SECS");
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    assert!(g.turn_no_progress.is_empty(), "{:?}", g.turn_no_progress);
    // Status projections (human + json) render the empty state cleanly.
    cli_ok(&["status", "--goal", &goal]);
    cli_ok(&["status", "--format", "json", "--goal", &goal]);
}

#[test]
fn normal_completion_inside_window_no_event() {
    let cr = cli_root();
    // completed_events starts a `shell` (write-class) right before end →
    // idle ≈ 0 even with a 1s window.
    let (_rt, _shared) = mock_env(MockState {
        events: common::mock_agent::completed_events("mock-run-1"),
        ..Default::default()
    });
    std::env::set_var("FUTURE_LOOP_NO_PROGRESS_SECS", "1");
    let goal = init_goal(&cr, "normal completion");
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--anonymous",
        "--max-turn-secs",
        "600",
        "--max-turns",
        "3",
    ]);
    std::env::remove_var("FUTURE_LOOP_NO_PROGRESS_SECS");
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    assert!(g.turn_no_progress.is_empty(), "{:?}", g.turn_no_progress);
}
