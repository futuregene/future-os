//! Coverage drive — console command group B: status / quota / scheduler /
//! store / backfill / privacy / runs / heartbeat-prompt / turn / todo-event /
//! evidence-log / diagnose / history / registry / version / canary.

mod common;

use common::{
    add_todo, cli, cli_err, cli_ok, cli_root, first_todo_id, init_goal, open_store, run_record,
};
use future_loop::state::now_epoch;
use future_loop::store::Event;

// ── status ─────────────────────────────────────────────────────────────────

#[test]
fn status_surface() {
    let cr = cli_root();
    // Empty registry.
    cli_ok(&["status"]);
    cli_ok(&["status", "--format", "json"]);
    let gid = init_goal(&cr, "status goal");
    cli_ok(&["status"]);
    cli_ok(&["status", "--goal", &gid]);
    cli_ok(&["status", "--format", "json"]);
    cli_ok(&["status", "--format", "json", "--goal", &gid]);
    assert!(cli_err(&["status", "--goal", "goal_nope"]).contains("not found"));
    assert!(cli_err(&["status", "--format", "json", "--goal", "goal_nope"]).contains("not found"));
}

// ── quota ──────────────────────────────────────────────────────────────────

#[test]
fn quota_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "quota goal");
    cli_ok(&["quota", "should-run", "--goal", &gid]);
    cli_ok(&["quota", "should-run", "--goal", &gid, "--format", "json"]);
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w1"]);
    cli_ok(&["quota", "should-run", "--goal", &gid, "--agent-id", "w1"]);
    // usage: single goal (text + json), --all (text + json), neither → err.
    cli_ok(&["quota", "usage", "--goal", &gid]);
    cli_ok(&["quota", "usage", "--goal", &gid, "--format", "json"]);
    cli_ok(&["quota", "usage", "--all"]);
    cli_ok(&["quota", "usage", "--all", "--format", "json"]);
    assert!(cli_err(&["quota", "usage"]).contains("--all"));
    assert!(cli_err(&["quota", "usage", "--goal", "goal_nope"]).contains("not found"));
    // spend.
    cli_ok(&["quota", "spend", "--goal", &gid]);
    assert!(cli_err(&["quota", "spend"]).contains("--goal required"));
    assert!(cli_err(&["quota", "spend", "--goal", "goal_nope"]).contains("not found"));
    // should-run errors.
    assert!(cli_err(&["quota", "should-run"]).contains("--goal required"));
    assert!(cli_err(&["quota", "should-run", "--goal", "goal_nope"]).contains("not found"));
    assert!(cli_err(&["quota", "bogus", "--goal", &gid]).contains("should-run"));
    assert!(cli_err(&["quota"]).contains("should-run"));
}

// ── scheduler ──────────────────────────────────────────────────────────────

#[test]
fn scheduler_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "scheduler goal");
    // show with no state → hint line.
    cli_ok(&["scheduler", "show", "--goal", &gid]);
    // bootstrap tick, then advancing ticks.
    cli_ok(&["scheduler", "tick", "--goal", &gid]);
    cli_ok(&["scheduler", "show", "--goal", &gid]);
    cli_ok(&["scheduler", "tick", "--goal", &gid]);
    // custom progression / cadence class / action on a second agent.
    cli_ok(&[
        "scheduler",
        "tick",
        "--goal",
        &gid,
        "--agent-id",
        "agent-b",
        "--cadence-class",
        "hourly",
        "--progression",
        "15,30,60",
        "--action",
        "custom_reset",
    ]);
    cli_ok(&["scheduler", "tick", "--goal", &gid, "--agent-id", "agent-b"]);
    // record-host-failure with and without existing state.
    cli_ok(&[
        "scheduler",
        "record-host-failure",
        "--goal",
        &gid,
        "--target-rrule",
        "FREQ=MINUTELY;INTERVAL=15",
        "--observed-rrule",
        "FREQ=MINUTELY;INTERVAL=30",
        "--failure-kind",
        "host_stale_rrule",
        "--failure-count",
        "2",
    ]);
    cli_ok(&[
        "scheduler",
        "record-host-failure",
        "--goal",
        &gid,
        "--agent-id",
        "agent-fresh",
        "--target-rrule",
        "FREQ=HOURLY",
        "--failure-kind",
        "host_stale_rrule",
    ]);
    // errors.
    assert!(cli_err(&[
        "scheduler",
        "record-host-failure",
        "--goal",
        &gid,
        "--failure-kind",
        "k"
    ])
    .contains("--target-rrule"));
    assert!(cli_err(&[
        "scheduler",
        "record-host-failure",
        "--goal",
        &gid,
        "--target-rrule",
        "R"
    ])
    .contains("--failure-kind"));
    assert!(cli_err(&["scheduler", "bogus", "--goal", &gid]).contains("tick"));
    assert!(cli_err(&["scheduler"]).contains("tick"));
    assert!(cli_err(&["scheduler", "tick"]).contains("--goal required"));
    assert!(cli_err(&["scheduler", "tick", "--goal", "goal_nope"]).contains("not found"));
}

// ── store ──────────────────────────────────────────────────────────────────

#[test]
fn store_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "store goal");
    cli_ok(&["store", "verify", "--goal", &gid]);
    // migrate on a current-schema goal errors; removing the schema stamp
    // makes the ledger read as LEGACY and the migration rewrites it.
    assert!(cli_err(&["store", "migrate", "--goal", &gid]).contains("already on schema"));
    {
        let store = open_store(&cr);
        let stamp = store.goal_dir(&gid).join("schema.json");
        std::fs::remove_file(&stamp).unwrap();
    }
    cli_ok(&["store", "migrate", "--goal", &gid]);
    // A goal dir with no ledger at all.
    assert!(cli_err(&["store", "migrate", "--goal", "goal_nope"]).contains("no event ledger"));
    cli_ok(&["store", "bridge", "--goal", &gid]);
    assert!(cli_err(&["store", "bogus", "--goal", &gid]).contains("migrate"));
    assert!(cli_err(&["store"]).contains("migrate"));
    assert!(cli_err(&["store", "verify"]).contains("--goal required"));
}

// ── backfill ───────────────────────────────────────────────────────────────

const BACKFILL_MD: &str = "---\nstatus: active\n---\n\n# Active Goal State\n\n\
    ## Agent Todo\n\n\
    - [ ] [P1] Run the check\n  <!-- future-loop:todo todo_id=todo_abc123 status=open action_kind=shell updated_at=2026-08-05T12:00:00+00:00 -->\n\
    - [x] Ship the artifact\n  <!-- future-loop:todo todo_id=todo_def456 status=done no_followup=true evidence=done%20well completed_at=2026-08-05T13:00:00+00:00 updated_at=2026-08-05T13:00:00+00:00 -->\n\n\
    ## User Todo / Owner Review Reading Queue\n\n\
    - [ ] Decide the scope\n  <!-- future-loop:todo todo_id=todo_ghi789 status=open task_class=user_gate updated_at=2026-08-05T12:30:00+00:00 -->\n";

#[test]
fn backfill_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "backfill goal");
    let md = std::path::Path::new(&cr.cwd).join("state.md");
    std::fs::write(&md, BACKFILL_MD).unwrap();
    // dry-run first, then the real append (idempotent producer).
    cli_ok(&[
        "backfill",
        "--goal",
        &gid,
        "--from",
        md.to_str().unwrap(),
        "--dry-run",
    ]);
    cli_ok(&["backfill", "--goal", &gid, "--from", md.to_str().unwrap()]);
    cli_ok(&[
        "backfill",
        "--goal",
        &gid,
        "--from",
        md.to_str().unwrap(),
        "--privacy",
        "public_safe",
    ]);
    // default --from: <cwd>/.future/loop/goals/<gid>/ACTIVE_GOAL_STATE.md? No —
    // the goal cwd's ACTIVE_GOAL_STATE.md; missing here → error.
    assert!(cli_err(&["backfill", "--goal", &gid]).contains(""));
    // bad privacy level / missing file / empty markdown.
    assert!(cli_err(&[
        "backfill",
        "--goal",
        &gid,
        "--from",
        md.to_str().unwrap(),
        "--privacy",
        "bogus"
    ])
    .contains(""));
    assert!(cli_err(&["backfill", "--goal", &gid, "--from", "/nonexistent.md"]).contains("read"));
    let empty = std::path::Path::new(&cr.cwd).join("empty.md");
    std::fs::write(&empty, "# nothing here\n").unwrap();
    assert!(cli_err(&[
        "backfill",
        "--goal",
        &gid,
        "--from",
        empty.to_str().unwrap()
    ])
    .contains("no todo records"));
    assert!(cli_err(&["backfill"]).contains("--goal required"));
    assert!(cli_err(&["backfill", "--goal", "goal_nope"]).contains("not found"));
}

// ── privacy ────────────────────────────────────────────────────────────────

#[test]
fn privacy_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "privacy goal");
    cli_ok(&["privacy", "--goal", &gid]);
    cli_ok(&["privacy", "--goal", &gid, "--format", "json"]);
    cli_ok(&["privacy", "--goal", &gid, "--level", "local_private"]);
    cli_ok(&["privacy", "--goal", &gid, "--level", "private_pointer"]);
    // Run it twice: the status-cache stale flag path (digest comparison).
    cli_ok(&["privacy", "--goal", &gid]);
    assert!(cli_err(&["privacy", "--goal", &gid, "--level", "bogus"]).contains(""));
    assert!(cli_err(&["privacy"]).contains("--goal required"));
    assert!(cli_err(&["privacy", "--goal", "goal_nope"]).contains("not found"));
}

// ── runs ───────────────────────────────────────────────────────────────────

#[test]
fn runs_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "runs goal");
    // history on a goal with no runs → None path.
    cli_ok(&["runs", "history", "--goal", &gid]);
    // Seed runs: the run INDEX consumes per-run files under
    // <root>/goals/<gid>/runs/ (written by compat::write_run during a real
    // run); runs.jsonl feeds goal.history.
    {
        let store = open_store(&cr);
        let first = first_todo_id(&cr.root, &gid);
        store
            .append_run(&gid, &run_record(&first, "completed", now_epoch()))
            .unwrap();
        store
            .append_run(&gid, &run_record(&first, "completed", now_epoch()))
            .unwrap();
        future_loop::compat::write_run(
            &store.goal_dir(&gid),
            &gid,
            &run_record(&first, "completed", now_epoch()),
        )
        .unwrap();
    }
    // Build the index, then history has rows (text + json).
    cli_ok(&["runs", "index", "--goal", &gid, "--rebuild"]);
    cli_ok(&["runs", "history", "--goal", &gid]);
    cli_ok(&["runs", "history", "--goal", &gid, "--format", "json"]);
    // index without --rebuild → duplicate scan.
    cli_ok(&["runs", "index", "--goal", &gid]);
    // compact both modes.
    cli_ok(&["runs", "compact", "--goal", &gid, "--keep", "1"]);
    cli_ok(&["runs", "compact", "--goal", &gid, "--cutoff", "1"]);
    // retention.
    cli_ok(&["runs", "retention", "--goal", &gid, "--keep", "1"]);
    // stale: None on a fresh goal; Some after state outruns the run ledger.
    let gid2 = init_goal(&cr, "stale goal");
    cli_ok(&["runs", "stale", "--goal", &gid2]);
    {
        let store = open_store(&cr);
        let first = first_todo_id(&cr.root, &gid2);
        store
            .append_run(&gid2, &run_record(&first, "completed", 1))
            .unwrap();
    }
    let first = first_todo_id(&cr.root, &gid2);
    cli_ok(&[
        "todo",
        "update",
        "--goal",
        &gid2,
        "--todo-id",
        &first,
        "--note",
        "state moved",
    ]);
    cli_ok(&["runs", "stale", "--goal", &gid2]);
    // errors.
    assert!(cli_err(&["runs", "bogus", "--goal", &gid]).contains("history|compact"));
    assert!(cli_err(&["runs"]).contains("history|compact"));
    assert!(cli_err(&["runs", "history"]).contains("--goal required"));
    assert!(cli_err(&["runs", "history", "--goal", "goal_nope"]).contains("not found"));
    assert!(
        cli_err(&["runs", "compact", "--goal", &gid, "--cutoff", "notanumber"])
            .contains("epoch secs")
    );
}

// ── heartbeat-prompt ───────────────────────────────────────────────────────

#[test]
fn heartbeat_prompt_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "heartbeat goal");
    // Base: selected todo, no gate, no history.
    cli_ok(&["heartbeat-prompt", "--goal", &gid]);
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w1"]);
    cli_ok(&["heartbeat-prompt", "--goal", &gid, "--agent-id", "w1"]);
    // Open gate → user-action-required branch (+ fallback todo line).
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "approve?",
        "--class",
        "user_gate",
    ]);
    cli_ok(&["heartbeat-prompt", "--goal", &gid]);
    // A seeded failed todo → "repair attempt N" line, plus run history.
    {
        let mut store = open_store(&cr);
        let mut t = future_loop::state::Todo::advancement("todo_failed", "failed once");
        t.failed_attempts = 1;
        store
            .append(Event::TodoAdded {
                goal_id: gid.clone(),
                todo: t,
                ts: now_epoch(),
            })
            .unwrap();
        store
            .append_run(&gid, &run_record("todo_failed", "error", now_epoch()))
            .unwrap();
        store.set_next_action(&gid, "failed once").unwrap();
    }
    // Resolve the gate so the failed todo becomes the selected frontier.
    let gate = common::todo_id_by_text(&cr.root, &gid, "approve?");
    cli_ok(&[
        "gate",
        "resolve",
        "--goal",
        &gid,
        "--todo-id",
        &gate,
        "--decision",
        "go",
    ]);
    cli_ok(&["heartbeat-prompt", "--goal", &gid]);
    // Terminal mode: cancel the goal (cancelled → terminal packet).
    let gid2 = init_goal(&cr, "terminal heartbeat");
    cli_ok(&["goal", "cancel", "--goal", &gid2]);
    cli_ok(&["heartbeat-prompt", "--goal", &gid2]);
    // errors.
    assert!(cli_err(&["heartbeat-prompt"]).contains("--goal required"));
    assert!(cli_err(&["heartbeat-prompt", "--goal", "goal_nope"]).contains("not found"));
}

// ── turn / todo-event / evidence-log / diagnose / history ──────────────────

#[test]
fn turn_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "turn goal");
    let first = first_todo_id(&cr.root, &gid);
    cli_ok(&["turn", "--goal", &gid, "--todo-id", &first]);
    cli_ok(&[
        "turn",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--agent-id",
        "w1",
    ]);
    assert!(cli_err(&["turn", "--goal", &gid]).contains("--todo-id required"));
    assert!(cli_err(&["turn", "--todo-id", &first]).contains("--goal required"));
    assert!(cli_err(&["turn", "--goal", &gid, "--todo-id", "todo_nope"]).contains("not found"));
    assert!(cli_err(&["turn", "--goal", "goal_nope", "--todo-id", &first]).contains("not found"));
}

#[test]
fn todo_event_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "todo-event goal");
    let first = first_todo_id(&cr.root, &gid);
    // Rich event trail for one todo: claim/renew/release/expire/complete…
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w1"]);
    cli_ok(&[
        "todo",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--agent-id",
        "w1",
    ]);
    cli_ok(&[
        "lease",
        "renew",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--agent-id",
        "w1",
        "--lease-secs",
        "30",
    ]);
    cli_ok(&[
        "lease",
        "release",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--agent-id",
        "w1",
    ]);
    cli_ok(&["lease", "expire", "--goal", &gid, "--todo-id", &first]);
    cli_ok(&[
        "todo",
        "update",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--note",
        "n",
    ]);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--no-follow-up",
        "--evidence",
        "done",
    ]);
    cli_ok(&["todo-event", "--goal", &gid, "--todo-id", &first]);
    // Craft the remaining todo-touching variants (supersede/archive/monitor/
    // quota/evidence/run + a gate resolve on a second todo).
    {
        let mut store = open_store(&cr);
        let t2 = future_loop::state::Todo::advancement("todo_crafted", "crafted trail");
        store
            .append(Event::TodoAdded {
                goal_id: gid.clone(),
                todo: t2,
                ts: now_epoch(),
            })
            .unwrap();
        store
            .append(Event::TodoSuperseded {
                goal_id: gid.clone(),
                todo_id: "todo_crafted".into(),
                ts: now_epoch(),
            })
            .unwrap();
        store
            .append(Event::TodoArchived {
                goal_id: gid.clone(),
                todo_id: "todo_crafted".into(),
                ts: now_epoch(),
            })
            .unwrap();
        store
            .append(Event::MonitorPolled {
                goal_id: gid.clone(),
                todo_id: "todo_crafted".into(),
                result: "no_change".into(),
                no_change_count: 1,
                ts: now_epoch(),
            })
            .unwrap();
        store
            .append(Event::QuotaSpent {
                goal_id: gid.clone(),
                run_id: "r1".into(),
                todo_id: "todo_crafted".into(),
                source: "run".into(),
                slots: 1,
                ts: now_epoch(),
            })
            .unwrap();
        store
            .append(Event::EvidenceAttached {
                goal_id: gid.clone(),
                todo_id: "todo_crafted".into(),
                evidence: "e".into(),
                ts: now_epoch(),
            })
            .unwrap();
        store
            .append(Event::GateResolved {
                goal_id: gid.clone(),
                todo_id: "todo_crafted".into(),
                decision: "d".into(),
                note: None,
                ts: now_epoch(),
            })
            .unwrap();
        store
            .append_run(&gid, &run_record("todo_crafted", "completed", now_epoch()))
            .unwrap();
        let mut s2 = open_store(&cr);
        s2.append(Event::RunRecorded {
            goal_id: gid.clone(),
            record: run_record("todo_crafted", "completed", now_epoch()),
            ts: now_epoch(),
        })
        .unwrap();
        // Non-todo events for the _ => false filter arms.
        s2.append(Event::ReplanAcked {
            goal_id: gid.clone(),
            delta_kinds: vec!["vision_patch".into()],
            ts: now_epoch(),
        })
        .unwrap();
        s2.append(Event::GoalCancelled {
            goal_id: gid.clone(),
            reason: "r".into(),
            ts: now_epoch(),
        })
        .unwrap();
    }
    cli_ok(&["todo-event", "--goal", &gid, "--todo-id", "todo_crafted"]);
    // A todo id with no events.
    cli_ok(&["todo-event", "--goal", &gid, "--todo-id", "todo_ghost"]);
    assert!(cli_err(&["todo-event", "--goal", &gid]).contains("--todo-id required"));
    assert!(cli_err(&["todo-event", "--todo-id", &first]).contains("--goal required"));
}

#[test]
fn evidence_log_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "evidence goal");
    // Empty trail.
    cli_ok(&["evidence-log", "--goal", &gid]);
    cli_ok(&["evidence-log", "--goal", &gid, "--todo-id", "todo_ghost"]);
    let first = first_todo_id(&cr.root, &gid);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--no-follow-up",
        "--evidence",
        "shipped",
    ]);
    {
        let mut store = open_store(&cr);
        store
            .append(Event::EvidenceAttached {
                goal_id: gid.clone(),
                todo_id: "todo_x".into(),
                evidence: "attached proof".into(),
                ts: now_epoch(),
            })
            .unwrap();
        store
            .append_run(&gid, &run_record("todo_x", "completed", now_epoch()))
            .unwrap();
        store
            .append(Event::RunRecorded {
                goal_id: gid.clone(),
                record: run_record("todo_x", "completed", now_epoch()),
                ts: now_epoch(),
            })
            .unwrap();
        // A run with EMPTY evidence is skipped by the run-evidence arm.
        let mut r = run_record("todo_y", "completed", now_epoch());
        r.evidence = String::new();
        store
            .append(Event::RunRecorded {
                goal_id: gid.clone(),
                record: r,
                ts: now_epoch(),
            })
            .unwrap();
    }
    cli_ok(&["evidence-log", "--goal", &gid]);
    cli_ok(&["evidence-log", "--goal", &gid, "--todo-id", "todo_x"]);
    cli_ok(&["evidence-log", "--goal", &gid, "--todo-id", &first]);
    assert!(cli_err(&["evidence-log"]).contains("--goal required"));
}

#[test]
fn diagnose_and_history_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "diag goal");
    // No runs yet.
    cli_ok(&["history", "--goal", &gid]);
    cli_ok(&["diagnose", "--goal", &gid]);
    cli_ok(&["diagnose", "--goal", &gid, "--format", "json"]);
    // Seed runs (one with empty evidence, one with tools+evidence).
    {
        let store = open_store(&cr);
        let first = first_todo_id(&cr.root, &gid);
        store
            .append_run(&gid, &run_record(&first, "completed", now_epoch()))
            .unwrap();
        let mut bare = run_record(&first, "error", now_epoch());
        bare.evidence = String::new();
        bare.tools = vec![];
        store.append_run(&gid, &bare).unwrap();
    }
    cli_ok(&["history", "--goal", &gid]);
    cli_ok(&["diagnose", "--goal", &gid]);
    cli_ok(&["diagnose", "--goal", &gid, "--format", "json"]);
    assert!(cli_err(&["history"]).contains("--goal required"));
    assert!(cli_err(&["history", "--goal", "goal_nope"]).contains("not found"));
    assert!(cli_err(&["diagnose"]).contains("--goal required"));
    assert!(cli_err(&["diagnose", "--goal", "goal_nope"]).contains("not found"));
}

// ── registry / version / canary ────────────────────────────────────────────

#[test]
fn registry_version_canary_surface() {
    let _cr = cli_root();
    cli_ok(&["registry"]);
    cli_ok(&["registry", "--json"]);
    cli_ok(&["registry", "--include-experimental"]);
    cli_ok(&["version"]);
    cli_ok(&["canary", "smoke"]);
    cli_ok(&["canary", "smoke", "--json"]);
    assert!(cli_err(&["canary", "smoke", "--profile", "no-such-profile"]).contains(""));
}
