//! P1-3 contract tests — automation liveness + monitor poll executor.
//!
//! ① liveness heartbeat: `scheduler tick` lands a `SchedulerTicked`
//!    heartbeat event; `scheduler liveness` evaluates the silence against a
//!    threshold — a breach records an `AutomationLivenessAlert`
//!    (cooldown-deduped), drops an operator-inbox alert file, and the
//!    attention projection escalates to a high-severity operator item until
//!    a fresh heartbeat recovers the automation.
//! ② monitor poll executor: the tick-driven poll plan classifies
//!    due/waiting/stalled monitors with no-spend eligibility, and the
//!    cadence-aware next-due writeback is shared by the run path and replay.

use future_loop::console;
use future_loop::scheduler::liveness as lv;
use future_loop::state::{Goal, Todo};
use future_loop::store::{Event, Store};
use future_loop::work_items::attention::{goal_attention_item, latest_unrecovered_liveness_alert};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "future-loop-p13-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn with_root<F: FnOnce(&str)>(tag: &str, f: F) {
    // FUTURE_LOOP_ROOT is process-global; serialize CLI tests behind a mutex.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = format!("{}/.future/loop", tmp_root(tag));
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("FUTURE_LOOP_ROOT", &root);
    f(&root);
}

fn cli(args: &[&str]) -> Result<(), String> {
    console::run(
        "future-loop",
        args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
    )
    .map_err(|e| format!("{e:#}"))
}

fn init_goal(root: &str, goal_id: &str, cwd: &str) {
    cli(&[
        "goal",
        "init",
        "--objective",
        "p1-3 liveness",
        "--cwd",
        cwd,
        "--goal-id",
        goal_id,
    ])
    .unwrap();
    let _ = root;
}

fn replay(root: &str, goal_id: &str) -> Goal {
    Store::open(root)
        .unwrap()
        .replay(goal_id)
        .unwrap()
        .expect("goal exists")
}

// ── ① heartbeat: tick lands the event; liveness evaluates alive ──────────

#[test]
fn tick_lands_heartbeat_and_liveness_stays_alive() {
    with_root("alive", |root| {
        let proj = tmp_root("alive-proj");
        init_goal(root, "g1", &proj);

        // No heartbeat yet → no_heartbeat, and crucially NOT a breach (no
        // alert event may land for an automation that was never installed).
        cli(&["scheduler", "liveness", "--goal", "g1"]).unwrap();
        cli(&["scheduler", "liveness", "--goal", "g1", "--format", "json"]).unwrap();
        let goal = replay(root, "g1");
        assert!(goal.scheduler_heartbeats.is_empty());
        assert!(goal.liveness_alerts.is_empty());

        // Tick → heartbeat event folds into the replay projection.
        cli(&["scheduler", "tick", "--goal", "g1"]).unwrap();
        let goal = replay(root, "g1");
        let hb = goal
            .scheduler_heartbeats
            .get("codex-app")
            .copied()
            .expect("tick lands a heartbeat for the default agent");
        let now = future_loop::state::now_epoch();
        assert!(now.saturating_sub(hb) < 60, "heartbeat is fresh");

        // Fresh heartbeat → alive, no alert.
        cli(&[
            "scheduler",
            "liveness",
            "--goal",
            "g1",
            "--threshold-secs",
            "3600",
        ])
        .unwrap();
        assert!(replay(root, "g1").liveness_alerts.is_empty());
    });
}

// ── ① breach: stale heartbeat → alert + inbox + attention; fresh tick ────
// ── recovers ──────────────────────────────────────────────────────────────

#[test]
fn stale_heartbeat_breaches_alerts_escalates_and_recovers() {
    with_root("breach", |root| {
        let proj = tmp_root("breach-proj");
        init_goal(root, "g1", &proj);
        let now = future_loop::state::now_epoch();

        // Fabricate a stale heartbeat (3h ago) — the automation went silent.
        Store::open(root)
            .unwrap()
            .append(Event::SchedulerTicked {
                goal_id: "g1".into(),
                agent_id: "codex-app".into(),
                action: "tick_next".into(),
                rrule: None,
                ts: now - 3 * 3600,
            })
            .unwrap();

        // Breach: alert lands (consecutive=1) + operator inbox file.
        cli(&[
            "scheduler",
            "liveness",
            "--goal",
            "g1",
            "--threshold-secs",
            "60",
        ])
        .unwrap();
        let goal = replay(root, "g1");
        assert_eq!(goal.liveness_alerts.len(), 1, "breach records one alert");
        let alert = &goal.liveness_alerts[0];
        assert_eq!(alert.agent_id, "codex-app");
        assert_eq!(alert.consecutive, 1);
        assert_eq!(alert.threshold_secs, 60);
        assert!(alert.elapsed_secs >= 3 * 3600 - 5);

        let inbox = std::path::Path::new(&proj)
            .join(".future")
            .join("loop")
            .join("inbox");
        let alerts: Vec<_> = std::fs::read_dir(&inbox)
            .expect("inbox dir created")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("liveness-g1-"))
            .collect();
        assert_eq!(alerts.len(), 1, "operator inbox alert file written");

        // Cooldown: an immediate second check does NOT append another alert.
        cli(&[
            "scheduler",
            "liveness",
            "--goal",
            "g1",
            "--threshold-secs",
            "60",
        ])
        .unwrap();
        assert_eq!(
            replay(root, "g1").liveness_alerts.len(),
            1,
            "cooldown dedups the breach alert"
        );

        // Attention escalation: high-severity operator item.
        let goal = replay(root, "g1");
        let item = goal_attention_item(&goal).expect("breach demands attention");
        assert_eq!(item.status, "automation_liveness_breach");
        assert_eq!(item.severity, "high");
        assert_eq!(item.waiting_on, "user_or_controller");
        assert!(item.recommended_action.contains("codex-app"));

        // Recovery: a fresh tick lands a heartbeat newer than the alert.
        cli(&["scheduler", "tick", "--goal", "g1"]).unwrap();
        let goal = replay(root, "g1");
        assert!(
            latest_unrecovered_liveness_alert(&goal).is_none(),
            "fresh heartbeat recovers the alert"
        );
        let item = goal_attention_item(&goal);
        assert!(
            item.is_none_or(|i| i.status != "automation_liveness_breach"),
            "attention no longer escalates liveness"
        );
    });
}

// ── ① alert ordinal: consecutive counts per (goal, agent) scope ───────────

#[test]
fn repeated_breaches_increment_the_alert_ordinal() {
    with_root("ordinal", |root| {
        let proj = tmp_root("ordinal-proj");
        init_goal(root, "g1", &proj);
        let now = future_loop::state::now_epoch();
        let mut store = Store::open(root).unwrap();
        // Stale heartbeat + an ancient prior alert (outside the cooldown).
        store
            .append(Event::SchedulerTicked {
                goal_id: "g1".into(),
                agent_id: "codex-app".into(),
                action: "tick_next".into(),
                rrule: None,
                ts: now - 3 * 3600,
            })
            .unwrap();
        store
            .append(Event::AutomationLivenessAlert {
                goal_id: "g1".into(),
                agent_id: "codex-app".into(),
                elapsed_secs: 8000,
                threshold_secs: 60,
                consecutive: 1,
                ts: now - 2 * lv::LIVENESS_ALERT_COOLDOWN_SECS,
            })
            .unwrap();
        drop(store);

        cli(&[
            "scheduler",
            "liveness",
            "--goal",
            "g1",
            "--threshold-secs",
            "60",
        ])
        .unwrap();
        let goal = replay(root, "g1");
        assert_eq!(goal.liveness_alerts.len(), 2, "cooldown expired → re-alert");
        assert_eq!(goal.liveness_alerts[1].consecutive, 2);
    });
}

// ── ① evaluation edges (pure) ─────────────────────────────────────────────

#[test]
fn liveness_evaluation_edges() {
    // Heartbeat newer than now (clock skew) clamps to alive.
    let eval = lv::evaluate_liveness("g", "a", Some(2000), 1000, 60);
    assert_eq!(eval.state, lv::LIVENESS_ALIVE);
    assert_eq!(eval.elapsed_secs, Some(0));
    // Exactly at threshold: alive; one second past: breach.
    assert_eq!(
        lv::evaluate_liveness("g", "a", Some(0), 60, 60).state,
        lv::LIVENESS_ALIVE
    );
    assert_eq!(
        lv::evaluate_liveness("g", "a", Some(0), 61, 60).state,
        lv::LIVENESS_BREACH
    );
}

// ── ② cadence-aware writeback: replay + run path share the derivation ────

#[test]
fn cadence_monitor_reschedules_by_cadence_on_replay() {
    let root = tmp_root("cadence-replay");
    let store_root = format!("{root}/.future/loop");
    std::fs::create_dir_all(&store_root).unwrap();
    let mut store = Store::open(&store_root).unwrap();
    let goal = Goal::new("g1", "objective", "/tmp");
    store.register(&goal).unwrap();
    store
        .append(Event::GoalStarted {
            goal_id: "g1".into(),
            ts: goal.created_at,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::monitor_with(
                "M1",
                "Watch",
                None,
                None,
                Some("1h"),
                std::time::Duration::from_secs(60),
            ),
            ts: goal.created_at,
        })
        .unwrap();
    let poll_ts = 1_784_000_000u64;
    store
        .append(Event::MonitorPolled {
            goal_id: "g1".into(),
            todo_id: "M1".into(),
            result: "no_change".into(),
            no_change_count: 1,
            ts: poll_ts,
        })
        .unwrap();
    let goal = store.replay("g1").unwrap().unwrap();
    let due = goal
        .todo("M1")
        .unwrap()
        .resume_when
        .unwrap()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert_eq!(
        due,
        poll_ts + 3600,
        "1h cadence ⇒ next due = poll ts + 3600 (not the fixed 12s backoff)"
    );
}

#[test]
fn no_cadence_monitor_keeps_fixed_backoff_replay_parity() {
    let root = tmp_root("backoff-parity");
    let store_root = format!("{root}/.future/loop");
    std::fs::create_dir_all(&store_root).unwrap();
    let mut store = Store::open(&store_root).unwrap();
    let goal = Goal::new("g1", "objective", "/tmp");
    store.register(&goal).unwrap();
    store
        .append(Event::GoalStarted {
            goal_id: "g1".into(),
            ts: goal.created_at,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::monitor("M1", "Watch", std::time::Duration::from_secs(60)),
            ts: goal.created_at,
        })
        .unwrap();
    let poll_ts = 1_784_000_000u64;
    store
        .append(Event::MonitorPolled {
            goal_id: "g1".into(),
            todo_id: "M1".into(),
            result: "no_change".into(),
            no_change_count: 1,
            ts: poll_ts,
        })
        .unwrap();
    let goal = store.replay("g1").unwrap().unwrap();
    let due = goal
        .todo("M1")
        .unwrap()
        .resume_when
        .unwrap()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert_eq!(
        due,
        poll_ts + future_loop::decision::MONITOR_NO_CHANGE_BACKOFF_SECS,
        "no cadence ⇒ the fixed G-8 backoff (pre-P1-3 replay parity)"
    );
}

#[test]
fn run_path_writeback_uses_the_same_cadence_derivation() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::monitor_with(
        "M1",
        "Watch",
        None,
        None,
        Some("30m"),
        std::time::Duration::from_millis(1),
    ));
    let record = future_loop::state::RunRecord {
        turn: 1,
        todo_id: "M1".into(),
        run_id: "run-1".into(),
        terminal_state: "completed".into(),
        error: None,
        tokens_in_delta: 0,
        tokens_out_delta: 0,
        cost_delta: 0.0,
        tools: vec![],
        evidence: String::new(),
        recorded_at: future_loop::state::now_epoch(),
        spend_source: Some("heartbeat".into()),
        validation: None,
    };
    let before = future_loop::state::now_epoch();
    future_loop::executor::writeback(&mut goal, &record, Some(false), None);
    let due = goal
        .todo("M1")
        .unwrap()
        .resume_when
        .unwrap()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(
        (before + 1800..=before + 1805).contains(&due),
        "30m cadence ⇒ run path reschedules ~1800s out, got due={due} before={before}"
    );
}
