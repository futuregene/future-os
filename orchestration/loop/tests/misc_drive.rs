//! Coverage drive — the long tail across agents/, benchmark/loop_protocol,
//! capabilities parsing, cli/registry, compat, decision/, heartbeat,
//! migration, quota/, runtime/, work_items/ and cli_projection.

mod common;

use common::{cli_root, init_goal, open_store, run_record};
use future_loop::state::{now_epoch, Goal, TaskClass, Todo, TodoStatus};
use future_loop::store::{Event, Store};

// ── agents/scope ───────────────────────────────────────────────────────────

#[test]
fn scope_frontier_arms() {
    use future_loop::agents::scope::{identity_scoped_frontier, todo_matches_agent};
    let now = now_epoch();
    let mut goal = Goal::new("g", "scope", "/tmp");
    // A user action bound to ANOTHER agent (diagnostic-only arm).
    let mut ua = Todo::user_action("ua1", "user action");
    ua.claimed_by = Some("other-agent".into());
    goal.todos.push(ua);
    // A user action bound to THIS agent (not diagnostic).
    let mut ua2 = Todo::user_action("ua2", "user action two");
    ua2.claimed_by = Some("w1".into());
    goal.todos.push(ua2);
    // Agent work claimed by another agent with a live lease.
    let mut theirs = Todo::advancement("t1", "theirs");
    theirs.claim("other-agent", 3600, now);
    goal.todos.push(theirs);
    // Agent work claimed by another agent with an EXPIRED lease (free).
    let mut lapsed = Todo::advancement("t2", "lapsed");
    lapsed.claim("other-agent", 1, now - 10);
    goal.todos.push(lapsed);
    // Unclaimed advancement + a monitor + a blocker (agent-work classes).
    goal.todos.push(Todo::advancement("t3", "free"));
    goal.todos.push(Todo::monitor("m1", "watch", std::time::Duration::from_secs(60)));
    goal.todos.push(Todo::blocker("b1", "blocker", &[]));
    // An open gate for the gates list.
    goal.todos.push(Todo::user_gate("g1", "q?", &[]));

    let f = identity_scoped_frontier(&goal, "w1", &[]);
    assert!(f.visible_agent_todo_ids.contains(&"t3".to_string()));
    assert!(f.other_agent_claimed_ids.contains(&"t1".to_string()));
    assert!(f.open_user_gate_ids.contains(&"g1".to_string()));
    assert!(f.unclaimed_advancement_count >= 1);
    // todo_matches_agent matrix.
    let mut claimed = Todo::advancement("x", "x");
    claimed.claimed_by = Some("other-agent".into());
    assert!(todo_matches_agent(&Todo::advancement("x", "x"), "w1"));
    assert!(todo_matches_agent(&claimed, "other-agent"));
    assert!(!todo_matches_agent(&claimed, "w1"));
}

// ── agents/capability_gate ─────────────────────────────────────────────────

#[test]
fn capability_gate_arms() {
    use future_loop::agents::capability_gate as cg;
    // available_capabilities_with_defaults merges declared over defaults.
    let avail = cg::available_capabilities_with_defaults(&["custom-cap".to_string()]);
    assert!(avail.contains(&"custom-cap".to_string()));
    // missing_required_capabilities on a todo with/without requirements.
    let mut t = Todo::advancement("t1", "x");
    t.required_capability = Some("shell".into());
    assert!(cg::missing_required_capabilities(&t, &avail).is_empty());
    assert_eq!(cg::missing_required_capabilities(&t, &[]), vec!["shell".to_string()]);
    let plain = Todo::advancement("t2", "y");
    assert!(cg::missing_required_capabilities(&plain, &[]).is_empty());
    // todo_is_runnable (note: the DEFAULT available set includes shell, so
    // only a custom capability blocks).
    assert!(cg::todo_is_runnable(&plain, &[]));
    assert!(cg::todo_is_runnable(&t, &[]), "shell is in the default set");
    let mut custom = Todo::advancement("t4", "z");
    custom.required_capability = Some("custom-xyz".into());
    assert!(!cg::todo_is_runnable(&custom, &[]));
    // build_capability_gate with blocked todos across owner classes.
    let mut needs_shell = Todo::advancement("t1", "needs shell");
    needs_shell.required_capability = Some("shell".into());
    let mut needs_custom = Todo::advancement("t2", "needs custom");
    needs_custom.required_capability = Some("custom-xyz".into());
    let todos = vec![needs_shell, needs_custom, Todo::advancement("t3", "free")];
    let gate = cg::build_capability_gate(&todos, &[]).expect("gate with blocked todos");
    assert!(gate.runnable_todo_ids.contains(&"t3".to_string()));
    assert!(gate.runnable_todo_ids.contains(&"t1".to_string()), "shell is default-available");
    assert!(gate.blocked_todo_ids.contains(&"t2".to_string()), "custom-xyz missing");
    // Everything runnable → None.
    let free = vec![Todo::advancement("t9", "free")];
    assert!(cg::build_capability_gate(&free, &[]).is_none());
    let _ = gate;
}

// ── agents/lane ────────────────────────────────────────────────────────────

#[test]
fn lane_recommendation_arms() {
    use future_loop::agents::lane::compact_agent_lane_recommendation;
    let mut goal = Goal::new("g", "lane", "/tmp");
    goal.register_agent("w1", vec![]);
    // A run with EMPTY evidence → no recommended action.
    let mut r = run_record("t1", "completed", now_epoch());
    r.evidence = String::new();
    goal.history.push(r);
    let rec = compact_agent_lane_recommendation(&goal, "w1").unwrap();
    assert!(rec.recommended_action.is_none());
    // With evidence → truncated action.
    goal.history.push(run_record("t1", "completed", now_epoch()));
    let rec = compact_agent_lane_recommendation(&goal, "w1").unwrap();
    assert!(rec.recommended_action.is_some());
}

// ── benchmark/loop_protocol ────────────────────────────────────────────────

#[test]
fn loop_protocol_comparison_contract() {
    use future_loop::benchmark::loop_protocol as lp;
    let c = lp::build_product_mode_main_table_comparison_contract(
        "bench",
        Some(7),
        lp::RAW_CODEX_AUTONOMOUS_MAX5_ROUTE,
        lp::LOOPX_GOAL_START_PRODUCT_MODE_ROUTE,
    );
    assert_eq!(c.max_rounds_budget, 7);
    // Default budget + non-special routes.
    let c2 = lp::build_product_mode_main_table_comparison_contract("bench", None, "r1", "r2");
    assert_eq!(c2.max_rounds_budget, lp::BLIND_LOOP_DEFAULT_MAX_ROUNDS);
    let c3 = lp::build_product_mode_main_table_comparison_contract(
        "bench",
        Some(0),
        "r1",
        lp::LOOPX_PRODUCT_MODE_ROUTE,
    );
    assert_eq!(c3.max_rounds_budget, lp::BLIND_LOOP_DEFAULT_MAX_ROUNDS, "0 → default");
    // Route classifiers.
    assert!(!lp::blind_loop_routes().is_empty());
    assert!(!lp::product_mode_routes().is_empty());
}

// ── capabilities parsing (auto_research / periodic_report) ─────────────────

#[test]
fn auto_research_parse_arms() {
    use future_loop::capabilities::CapabilityRegistry;
    let registry = CapabilityRegistry::with_builtin();
    let cap = registry.get("auto_research").unwrap();
    // Structured with all keys.
    let ps = cap.propose("question: does X beat Y on metric Z?\nhypothesis: X wins\nmethod: ablation");
    assert!(!ps.is_empty());
    // A non-question → clarify successor.
    let ps = cap.propose("question: just a statement");
    assert!(ps.iter().any(|p| p.reason.contains("not shaped as a research question") || p.reason.contains("Clarify")), "{ps:?}");
    // Empty.
    assert!(!cap.propose("").is_empty());
}

#[test]
fn periodic_report_parse_arms() {
    use future_loop::capabilities::periodic_report::parse_report_profile;
    let p = parse_report_profile("cadence: daily\nscope: team\naudience: ops\nnotes: n");
    assert_eq!(p.cadence, "daily");
    assert_eq!(p.scope, "team");
    assert_eq!(p.audience, "ops");
    // Free text becomes the cadence when no key is present.
    let p = parse_report_profile("every friday\nwith details");
    assert_eq!(p.cadence, "every friday with details");
    // Cadence tokens → (class, seconds).
    use future_loop::capabilities::periodic_report::cadence_due_secs;
    assert_eq!(cadence_due_secs("hourly"), Some(("hourly".to_string(), 3600)));
    assert_eq!(cadence_due_secs("every-6h"), Some(("every-6h".to_string(), 21600)));
    assert_eq!(cadence_due_secs("every-2d"), Some(("every-2d".to_string(), 172800)));
    assert_eq!(cadence_due_secs("every-0h"), None);
    assert_eq!(cadence_due_secs("fortnightly"), None);
    let _ = &p;
}

// ── cli/registry ───────────────────────────────────────────────────────────

#[test]
fn registry_idempotent_group_and_render_marks() {
    use future_loop::cli::registry::CommandRegistry;
    let mut r = CommandRegistry::new();
    let a = r.group("g", "first");
    let b = r.group("g", "second");
    assert_eq!(a, b, "group() is idempotent by name");
    // render_help marks experimental commands and skips empty groups (it
    // prints usage + summary, not the command name).
    let g = r.group("main", "m");
    r.command(g, "stable-cmd", "stable summary", "stable-cmd-usage");
    let _ = r.group("empty-group", "e");
    let text = r.render_help(false);
    assert!(text.contains("stable-cmd-usage"), "{text}");
    assert!(!text.contains("empty-group"), "empty groups skipped");
}

// ── compat ─────────────────────────────────────────────────────────────────

#[test]
fn compat_write_run_and_active_state() {
    let dir = tempfile::tempdir().unwrap();
    let goal_dir = dir.path();
    let mut goal = Goal::new("g", "compat", "/tmp");
    let mut t = Todo::advancement("t1", "task");
    t.evidence = Some("proof+plus".into());
    t.status = TodoStatus::Done;
    t.completed_at = Some(1_700_000_000);
    goal.todos.push(t);
    goal.todos.push(Todo::monitor("m1", "watch", std::time::Duration::from_secs(60)));
    // write_active_state with evidence/completed anchors.
    future_loop::compat::write_active_state(goal_dir, &goal).unwrap();
    let md = std::fs::read_to_string(goal_dir.join("ACTIVE_GOAL_STATE.md")).unwrap();
    assert!(md.contains("t1"), "{md}");
    // write_run twice (json + md artifacts).
    future_loop::compat::write_run(goal_dir, "g", &run_record("t1", "completed", now_epoch())).unwrap();
    future_loop::compat::write_run(goal_dir, "g", &run_record("t1", "completed", now_epoch())).unwrap();
    let runs = goal_dir.join("runs");
    assert!(runs.exists());
}

// ── decision misc ──────────────────────────────────────────────────────────

#[test]
fn decision_misc_arms() {
    // truncate: short pass-through + multibyte-safe ellipsis.
    assert_eq!(future_loop::decision::truncate("abc", 5), "abc");
    let long = "界".repeat(100);
    let t = future_loop::decision::truncate(&long, 10);
    assert!(t.ends_with('…'));
    // complete_todo on a missing todo is a no-op.
    let mut goal = Goal::new("g", "o", "/tmp");
    future_loop::decision::complete_todo(&mut goal, "ghost", true, vec![]);
    // boundary: a home-path objective leaks into the packet boundary.
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
        let goal = Goal::new("g", &format!("read {home}/secrets.txt"), "/tmp");
        let packet = future_loop::decision::decide_for(&goal, std::time::SystemTime::now(), None);
        assert!(!packet.boundary.public_safe);
    }
}

// ── heartbeat render arms ──────────────────────────────────────────────────

#[test]
fn heartbeat_render_arms() {
    use future_loop::heartbeat::render_heartbeat_prompt;
    // Gate WITHOUT a question → "(see todo)" + gate id list + fallback todo.
    let mut goal = Goal::new("g", "hb", "/tmp");
    let mut gate = Todo::user_gate("tg", "gate text", &[]);
    gate.gate_question = None;
    goal.todos.push(gate);
    goal.todos.push(Todo::advancement("t1", "fallback work"));
    goal.history.push(run_record("t0", "completed", now_epoch()));
    let packet = future_loop::decision::decide_for(&goal, std::time::SystemTime::now(), None);
    let out = render_heartbeat_prompt(&goal, &packet);
    assert!(out.contains("USER ACTION REQUIRED"), "{out}");
    assert!(out.contains("gate todos: tg"), "{out}");
    assert!(out.contains("fallback todo"), "{out}");
    assert!(out.contains("last evidence"), "{out}");
    // Boundary leak + terminal stop condition.
    let home = std::env::var("HOME").unwrap_or_default();
    let goal2 = Goal::new("g2", &format!("touch {home}/x"), "/tmp");
    let packet = future_loop::decision::decide_for(&goal2, std::time::SystemTime::now(), None);
    let out = render_heartbeat_prompt(&goal2, &packet);
    if !packet.boundary.public_safe {
        assert!(out.contains("boundary leaks"), "{out}");
    }
    // Terminal goal (cancelled) → stop-condition arm + no selected todo.
    let mut goal3 = Goal::new("g3", "hb", "/tmp");
    goal3.status = "cancelled".into();
    let packet = future_loop::decision::decide_for(&goal3, std::time::SystemTime::now(), None);
    let out = render_heartbeat_prompt(&goal3, &packet);
    assert!(out.contains("goal validated closed"), "{out}");
}

// ── migration ──────────────────────────────────────────────────────────────

#[test]
fn migration_arms() {
    let cr = cli_root();
    let gid = init_goal(&cr, "migration drive");
    let store = open_store(&cr);
    let goal_dir = store.goal_dir(&gid);
    // apply_migrations with blank lines in the ledger (kept verbatim).
    {
        let events = goal_dir.join("events.jsonl");
        let existing = std::fs::read_to_string(&events).unwrap();
        std::fs::write(&events, format!("\n{existing}")).unwrap();
        std::fs::remove_file(goal_dir.join("schema.json")).unwrap();
    }
    let report = future_loop::migration::apply_migrations(&goal_dir, &gid).unwrap();
    assert!(report.migrated_lines >= 1);
    // Bridge status: rollback_plan_recorded flips once a backup exists.
    let store = open_store(&cr);
    let before = future_loop::migration::migration_bridge_status(&store, &gid, &store.goal_dir(&gid));
    assert!(!before.checks.rollback_plan_recorded);
    store.backup_goal(&gid).unwrap();
    let after = future_loop::migration::migration_bridge_status(&store, &gid, &store.goal_dir(&gid));
    assert!(after.checks.rollback_plan_recorded);
    // dual_read_parity: ACTIVE_GOAL_STATE.md with anchors matching the replay.
    let goal = store.replay(&gid).unwrap().unwrap();
    future_loop::compat::write_active_state(&store.goal_dir(&gid), &goal).unwrap();
    let with_state = future_loop::migration::migration_bridge_status(&store, &gid, &store.goal_dir(&gid));
    let _ = with_state.checks.dual_read_parity_clean;
    // migration_steps registry is non-empty and ordered.
    assert!(!future_loop::migration::migration_steps().is_empty());
}

// ── quota ──────────────────────────────────────────────────────────────────

#[test]
fn quota_slot_accounting_arms() {
    use future_loop::quota::slot_accounting as sa;
    for (s, expect) in [
        ("run", sa::SlotSpendSource::Run),
        ("agent", sa::SlotSpendSource::Agent),
        ("heartbeat", sa::SlotSpendSource::Heartbeat),
    ] {
        assert_eq!(sa::SlotSpendSource::parse(s), Some(expect));
        assert_eq!(expect.as_str(), s);
    }
    assert_eq!(sa::SlotSpendSource::parse("bogus"), None);
    // stall repair: kind() + is_stalled_mode arms.
    use future_loop::quota::stall_repair::{detect_stall, is_stalled_mode};
    for kind in ["outcome_floor", "repair_budget_exhausted", "monitor_stalled", "succession_obligation", "acceptance_gap"] {
        assert!(is_stalled_mode(kind), "{kind}");
    }
    assert!(!is_stalled_mode("normal_run"));
    // detect_stall on a healthy goal → None; with a stalled monitor → Some.
    let goal = Goal::new("g", "o", "/tmp");
    assert!(detect_stall(&goal).is_none());
    let mut goal = Goal::new("g", "o", "/tmp");
    let mut m = Todo::monitor("m1", "watch", std::time::Duration::from_secs(60));
    m.consecutive_no_change = 99;
    goal.todos.push(m);
    let hint = detect_stall(&goal).expect("stalled monitor");
    assert_eq!(hint.kind(), "monitor_stalled");
}

// ── runtime: run_history / run_index / run_compaction / stale_latest_run ───

#[test]
fn runtime_run_history_and_index_arms() {
    let cr = cli_root();
    let gid = init_goal(&cr, "runtime drive");
    let store = open_store(&cr);
    let runs_dir = store.goal_dir(&gid).join("runs");
    std::fs::create_dir_all(&runs_dir).unwrap();
    // Index rows: malformed line skipped; empty classification → "work";
    // unparseable timestamp → row_epoch None (skipped in totals).
    let index = runs_dir.join("index.jsonl");
    std::fs::write(
        &index,
        concat!(
            "{bad json\n",
            "{\"goal_id\":\"G\",\"timestamp\":\"not-a-date\",\"path\":\"a.json\",\"classification\":\"\"}\n",
            "{\"goal_id\":\"G\",\"timestamp\":\"2026-08-10T00:00:00+00:00\",\"path\":\"b.json\",\"classification\":\"\"}\n",
            "{\"goal_id\":\"G\",\"timestamp\":\"2026-08-10T00:00:00+00:00\",\"path\":\"c.json\",\"classification\":\"monitor\"}\n",
        ),
    )
    .unwrap();
    let projection = future_loop::runtime::run_history::build_run_history(&cr.root, &gid, now_epoch())
        .unwrap()
        .expect("rows present");
    assert!(projection.sample_run_count >= 2);
    // detect_duplicates: duplicate identity rows are repairable.
    let dup_index = runs_dir.join("dup.jsonl");
    std::fs::write(
        &dup_index,
        concat!(
            "{\"goal_id\":\"G\",\"timestamp\":\"2026-08-10T00:00:00+00:00\",\"path\":\"a.json\",\"classification\":\"work\"}\n",
            "{\"goal_id\":\"G\",\"timestamp\":\"2026-08-10T00:00:00+00:00\",\"path\":\"a.json\",\"classification\":\"work\"}\n",
        ),
    )
    .unwrap();
    let report = future_loop::runtime::run_index::detect_duplicates(&dup_index).unwrap();
    assert_eq!(report.duplicate_groups.len(), 1);
    assert!(report.repairable);
    // detect on a missing index → empty report.
    let missing = runs_dir.join("absent.jsonl");
    let report = future_loop::runtime::run_index::detect_duplicates(&missing).unwrap();
    assert_eq!(report.total_rows, 0);
    // rebuild over an EXISTING index → pre-rebuild backup is written.
    future_loop::compat::write_run(&store.goal_dir(&gid), &gid, &run_record("t", "completed", now_epoch())).unwrap();
    let report = future_loop::runtime::run_index::rebuild_index(&cr.root, &gid).unwrap();
    assert!(report.rows_written >= 1);
    let report2 = future_loop::runtime::run_index::rebuild_index(&cr.root, &gid).unwrap();
    assert!(!report2.backup_path.is_empty(), "second rebuild backs up the prior index");
}

#[test]
fn run_compaction_arms() {
    let cr = cli_root();
    let gid = init_goal(&cr, "compaction drive");
    let store = open_store(&cr);
    let runs_dir = store.goal_dir(&gid).join("runs");
    std::fs::create_dir_all(&runs_dir).unwrap();
    // Two run files with real timestamp payloads (rows derive epochs from
    // file content, not filenames).
    let old_file = runs_dir.join("2020-01-01T00-00-00-00-00.json");
    std::fs::write(&old_file, "{\"timestamp\":\"2020-01-01T00:00:00+00:00\",\"turn\":1,\"terminal_state\":\"completed\"}").unwrap();
    let new_file = runs_dir.join("2030-01-01T00-00-00-00-00.json");
    std::fs::write(&new_file, "{\"timestamp\":\"2030-01-01T00:00:00+00:00\",\"turn\":2,\"terminal_state\":\"completed\"}").unwrap();
    future_loop::runtime::run_index::rebuild_index(&cr.root, &gid).unwrap();
    // archive_runs_before: cutoff between the two → old one moves to archive/.
    let cutoff = chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00+00:00")
        .unwrap()
        .timestamp() as u64;
    let report = future_loop::runtime::run_compaction::archive_runs_before(&cr.root, &gid, cutoff).unwrap();
    assert_eq!(report.archived.len(), 1, "{report:?}");
    assert!(!old_file.exists());
    // Second run: the file is gone (already archived) — row re-pointed only.
    let report = future_loop::runtime::run_compaction::archive_runs_before(&cr.root, &gid, cutoff).unwrap();
    assert!(report.archived.is_empty());
    // archive_keeping_latest on a fresh goal with 3 runs: keep=1 → the
    // cutoff is the second-newest row → the single oldest run archives.
    let gid3 = init_goal(&cr, "keeping latest");
    {
        let store = open_store(&cr);
        let runs_dir = store.goal_dir(&gid3).join("runs");
        std::fs::create_dir_all(&runs_dir).unwrap();
        for (name, ts) in [
            ("2020-01-01T00-00-00-00-00.json", "2020-01-01T00:00:00+00:00"),
            ("2025-01-01T00-00-00-00-00.json", "2025-01-01T00:00:00+00:00"),
            ("2030-01-01T00-00-00-00-00.json", "2030-01-01T00:00:00+00:00"),
        ] {
            std::fs::write(
                runs_dir.join(name),
                format!("{{\"timestamp\":\"{ts}\",\"turn\":1,\"terminal_state\":\"completed\"}}"),
            )
            .unwrap();
        }
    }
    future_loop::runtime::run_index::rebuild_index(&cr.root, &gid3).unwrap();
    let report = future_loop::runtime::run_compaction::archive_keeping_latest(&cr.root, &gid3, 1).unwrap();
    assert_eq!(report.archived.len(), 1, "{report:?}");
    // No index → error.
    let gid2 = init_goal(&cr, "no index goal");
    assert!(future_loop::runtime::run_compaction::archive_runs_before(&cr.root, &gid2, 1).is_err());
}

// ── cli_projection remaining arms ──────────────────────────────────────────

#[test]
fn cli_projection_arms() {
    // render_quota_projection with a stall that carries a blocked scope, an
    // arbitration, and a terminal closure payload.
    let mut goal = Goal::new("g", "proj", "/tmp");
    goal.todos.push(Todo::advancement("t1", "work"));
    let packet = future_loop::decision::decide_for(&goal, std::time::SystemTime::now(), None);
    let stall = future_loop::quota::stall_repair::StallRepairHint {
        kind: "monitor_no_change_stalled".into(),
        reason: "r".into(),
        replan_hint: "h".into(),
        blocked_action_scope: Some("deploy".into()),
    };
    let out = future_loop::cli_projection::render_quota_projection(&packet, None, Some(&stall));
    assert!(out.contains("blocked action scope: deploy"), "{out}");
    let out2 = future_loop::cli_projection::render_quota_projection(&packet, None, None);
    assert!(!out2.contains("stall:"), "{out2}");
    // render_cadence_plan: multi-interval progressions show the next step;
    // the last index wraps.
    let out = future_loop::cli_projection::render_cadence_plan("hourly", &[15, 30], 0);
    assert!(out.contains("next"), "{out}");
    let out = future_loop::cli_projection::render_cadence_plan("hourly", &[15, 30], 1);
    assert!(out.contains("wraps to start"), "{out}");
    // Unknown class with no progression → single-execution early return.
    let out = future_loop::cli_projection::render_cadence_plan("once", &[], 0);
    assert!(out.contains("single execution"), "{out}");
    // initial_rrule_for arms.
    assert!(future_loop::cli_projection::initial_rrule_for("hourly").is_some());
    assert!(future_loop::cli_projection::initial_rrule_for("daily").is_some());
    assert!(future_loop::cli_projection::initial_rrule_for("weekly").is_some());
    assert!(future_loop::cli_projection::initial_rrule_for("once").is_none());
    // monitor_metadata_lines with/without metadata.
    let mut goal = Goal::new("g", "mon", "/tmp");
    let mut m = Todo::monitor("m1", "watch", std::time::Duration::from_secs(60));
    m.monitor_target = Some("file:x".into());
    goal.todos.push(m);
    goal.todos.push(Todo::monitor("m2", "plain", std::time::Duration::from_secs(60)));
    let lines = future_loop::cli_projection::monitor_metadata_lines(&goal);
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("target=file:x"), "{lines:?}");
    assert!(!lines[1].contains("target="), "{lines:?}");
}

// ── work_items misc ────────────────────────────────────────────────────────

#[test]
fn task_graph_and_lease_arms() {
    use future_loop::work_items::task_graph as tg;
    // topological_sort skips edges referencing unknown todos.
    let todos = vec![Todo::advancement("a", "a"), Todo::advancement("b", "b")];
    let edges = vec![
        ("a".to_string(), "b".to_string()),
        ("ghost".to_string(), "b".to_string()),
    ];
    let (order, cycle) = tg::topological_sort(&todos, &edges);
    assert!(cycle.is_empty());
    assert_eq!(order, vec!["a".to_string(), "b".to_string()]);
    // successors_of: blocking source names dependents; plain todo → reverse lookup.
    let mut goal = Goal::new("g", "o", "/tmp");
    let gate = Todo::user_gate("gate1", "q?", &["x", "y"]);
    goal.todos.push(gate);
    let mut dependent = Todo::advancement("dep", "d");
    dependent.blocked_by_gate = Some("gate1".into());
    goal.todos.push(dependent);
    assert_eq!(tg::successors_of(&goal, "gate1"), vec!["x".to_string(), "y".to_string()]);
    let mut adv = Todo::advancement("plain", "p");
    adv.blocked_by_gate = None;
    goal.todos.push(adv);
    let succ = tg::successors_of(&goal, "dep");
    assert!(succ.is_empty(), "nothing lists dep as predecessor: {succ:?}");
    assert!(tg::successors_of(&goal, "ghost").is_empty());

    // task_lease: claim on a non-open todo bails; release after expiry is a
    // no-op success.
    use future_loop::work_items::task_lease as lease;
    let mut t = Todo::advancement("t", "x");
    t.status = TodoStatus::Done;
    assert!(lease::claim(&mut t, "a", 60, 100).is_err());
    let mut t = Todo::advancement("t", "x");
    lease::claim(&mut t, "a", 1, 100).unwrap();
    let op = lease::release(&mut t, "a", 10_000).unwrap();
    assert!(matches!(op, lease::LeaseOp::Released { missing: true }));
}

// ── store projection-gap + apply guard arms ────────────────────────────────

#[test]
fn store_projection_gap_and_guard_arms() {
    // projection_gap: "[P1]" prefix and "waiting" markers suppress the gap;
    // "decide" without gates fires the user-side gap.
    let mut goal = Goal::new("g", "o", "/tmp");
    goal.next_action = Some("[P1] prioritized".into());
    assert!(future_loop::store::projection_gap(&goal).is_none());
    goal.next_action = Some("waiting for the host".into());
    assert!(future_loop::store::projection_gap(&goal).is_none());
    goal.next_action = Some("please decide the scope".into());
    assert!(future_loop::store::projection_gap(&goal).is_some(), "decide without gate");
    goal.next_action = Some(String::new());
    assert!(future_loop::store::projection_gap(&goal).is_none(), "empty → no gap");
    goal.next_action = None;
    assert!(future_loop::store::projection_gap(&goal).is_none());

    // Apply guards: release by a non-owner and expiry without a claim are
    // no-ops on replay.
    let cr = cli_root();
    let gid = init_goal(&cr, "guard arms");
    {
        let mut store: Store = open_store(&cr);
        let g = store.replay(&gid).unwrap().unwrap();
        let first = g.todos.first().unwrap().id.clone();
        drop(g);
        // Renew sets a claim when none existed (claim-fill arm).
        store
            .append(Event::TodoRenewed {
                goal_id: gid.clone(),
                todo_id: first.clone(),
                agent_id: "a".into(),
                lease_expires_at: 42,
                ts: now_epoch(),
            })
            .unwrap();
        // Release by a DIFFERENT agent must not clear it.
        store
            .append(Event::TodoReleased {
                goal_id: gid.clone(),
                todo_id: first.clone(),
                agent_id: "b".into(),
                ts: now_epoch(),
            })
            .unwrap();
        let g = store.replay(&gid).unwrap().unwrap();
        let t = g.todo(&first).unwrap();
        assert_eq!(t.claimed_by.as_deref(), Some("a"));
        assert_eq!(t.lease_expires_at, Some(42));
        // Expiry on a todo with no claim → no-op arm.
        store
            .append(Event::TodoExpired {
                goal_id: gid.clone(),
                todo_id: "todo_never_claimed".into(),
                ts: now_epoch(),
            })
            .unwrap();
        // TodoUpdated priority P2 arm.
        store
            .append(Event::TodoUpdated {
                goal_id: gid.clone(),
                todo_id: first.clone(),
                text: None,
                status: None,
                evidence: None,
                note: None,
                priority: Some("p2".into()),
                resume_when: None,
                blocks: None,
                ts: now_epoch(),
            })
            .unwrap();
        let g = store.replay(&gid).unwrap().unwrap();
        assert_eq!(g.todo(&first).unwrap().priority, future_loop::state::Priority::P2);
    }
}

// ── replan obligation arms ─────────────────────────────────────────────────

#[test]
fn replan_obligation_cleared_arms() {
    use future_loop::work_items::replan_obligation as ro;
    let mut goal = Goal::new("g", "o", "/tmp");
    // Monitor over the no-change threshold → obligation raised (uncleared).
    let mut m = Todo::monitor("m1", "watch", std::time::Duration::from_secs(60));
    m.consecutive_no_change = 99;
    m.updated_at = 1_000;
    goal.todos.push(m);
    let obligations = ro::unfulfilled_obligations(&goal);
    assert!(obligations.iter().any(|o| o.kind == "monitor_no_change_streak"));
    assert!(ro::has_unfulfilled_obligation(&goal));
    // A ReplanAck with a frontier-changing delta recorded AFTER raised_at
    // clears the obligation (has_frontier_delta accepts only these kinds).
    goal.replan_ack = Some(future_loop::state::ReplanAck {
        recorded: true,
        delta_kinds: vec!["vision_patch".into()],
        at: 2_000,
    });
    let obligations = ro::unfulfilled_obligations(&goal);
    assert!(obligations.iter().all(|o| o.kind != "monitor_no_change_streak"), "{obligations:?}");
}

// ── operator inbox load arms ───────────────────────────────────────────────

#[test]
fn operator_inbox_load_arms() {
    use future_loop::work_items::operator_inbox::load_pending_inbox_events;
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().to_string_lossy().into_owned();
    let inbox = dir.path().join(".future/loop/inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    // Events with missing optional fields get defaults; entries without a
    // message_id or content are skipped; non-objects are skipped.
    std::fs::write(inbox.join("a.json"), "{\"message_id\":\"m1\",\"content\":\"hi\"}").unwrap();
    std::fs::write(inbox.join("b.json"), "\"just a string\"").unwrap();
    std::fs::write(inbox.join("c.json"), "[{\"message_id\":\"m2\"}]").unwrap();
    std::fs::write(inbox.join("d.json"), "{\"message_id\":\"m3\"}").unwrap();
    let events = load_pending_inbox_events(&project, "inbox").unwrap();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].message_id, "m1");
    assert!(load_pending_inbox_events(&project, "../escape").is_err());
    assert!(load_pending_inbox_events(&project, "/abs").is_err());
    // Missing inbox dir → empty.
    let empty = tempfile::tempdir().unwrap();
    assert!(load_pending_inbox_events(&empty.path().to_string_lossy(), "inbox").unwrap().is_empty());
}
