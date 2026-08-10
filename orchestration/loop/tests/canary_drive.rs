//! Coverage drive for `canary/mod.rs` — the per-check failure arms need
//! goals in deliberately broken states (empty/corrupt/conflicting ledgers,
//! projection gaps, extension-state files).

mod common;

use common::{cli_root, init_goal, open_store, CliRoot};
use future_loop::canary::run_smoke;
use future_loop::state::Goal;
use future_loop::store::Store;

fn check<'a>(
    result: &'a future_loop::canary::SmokeRunResult,
    id: &str,
) -> &'a future_loop::canary::SmokeCheckOutcome {
    result
        .checks
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("check {id} ran"))
}

#[test]
fn smoke_on_healthy_populated_store() {
    let cr = cli_root();
    let _gid = init_goal(&cr, "healthy canary");
    let store = open_store(&cr);
    let result = run_smoke(&store, "release-gate").unwrap();
    assert!(result.all_passed, "{:?}", result.checks.iter().map(|c| (&c.id, c.passed, &c.detail)).collect::<Vec<_>>());
    // Non-vacuous detail arms (a goal exists → checks actually examined it).
    assert!(check(&result, "ledger_integrity").detail.contains("goal"));
    assert!(check(&result, "decision_determinism").detail.contains("deterministic"));
    assert!(check(&result, "quota_should_run").detail.contains("goal"));
    assert!(check(&result, "todo_frontier").detail.contains("frontier consistent"));
    assert!(check(&result, "status_projection").detail.contains("status consistent"));
    assert!(check(&result, "canary_self").passed);
}

#[test]
fn smoke_empty_and_corrupt_ledgers_fail() {
    let cr = cli_root();
    // Registered goal with NO events file → empty-ledger failure.
    {
        let mut store = open_store(&cr);
        store.register(&Goal::new("goal_empty", "no events", "/tmp")).unwrap();
    }
    let store = open_store(&cr);
    let result = run_smoke(&store, "release-gate").unwrap();
    let c = check(&result, "ledger_integrity");
    assert!(!c.passed, "{c:?}");
    assert!(c.detail.contains("empty ledger"), "{c:?}");
    // Corrupt ledger line → events() error arm.
    {
        let store = open_store(&cr);
        let dir = store.goal_dir("goal_empty");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("events.jsonl"), "{broken json\n").unwrap();
    }
    let store = open_store(&cr);
    let result = run_smoke(&store, "release-gate").unwrap();
    let c = check(&result, "ledger_integrity");
    assert!(!c.passed);
    assert!(c.detail.contains("goal_empty"), "{c:?}");
}

#[test]
fn smoke_conflicting_ledger_fails_verify_arm() {
    let cr = cli_root();
    let gid = init_goal(&cr, "conflict canary");
    {
        let store = open_store(&cr);
        let path = store.goal_dir(&gid).join("events.jsonl");
        let a = serde_json::json!({"event_id":"e-dup","kind":"goal_started","goal_id":gid,"ts":1});
        let b = serde_json::json!({"event_id":"e-dup","kind":"goal_started","goal_id":gid,"ts":2});
        std::fs::write(&path, format!("{}\n{}\n", a, b)).unwrap();
    }
    let store = open_store(&cr);
    let result = run_smoke(&store, "release-gate").unwrap();
    let c = check(&result, "ledger_integrity");
    assert!(!c.passed, "{c:?}");
    assert!(c.detail.contains("conflict"), "{c:?}");
    assert!(!result.all_passed);
}

#[test]
fn smoke_projection_gap_fails_frontier() {
    let cr = cli_root();
    let gid = init_goal(&cr, "gap canary");
    {
        let store = open_store(&cr);
        // Complete the only todo, then point next_action at phantom work.
        let g = store.replay(&gid).unwrap().unwrap();
        let first = g.todos.first().unwrap().id.clone();
        drop(g);
        let mut store = open_store(&cr);
        store
            .append(future_loop::store::Event::TodoCompleted {
                goal_id: gid.clone(),
                todo_id: first,
                no_follow_up: true,
                successor_ids: vec![],
                evidence: None,
                ts: future_loop::state::now_epoch(),
            })
            .unwrap();
        store.set_next_action(&gid, "phantom work with no todo").unwrap();
    }
    let store = open_store(&cr);
    let result = run_smoke(&store, "release-gate").unwrap();
    let c = check(&result, "todo_frontier");
    assert!(!c.passed, "{c:?}");
    assert!(c.detail.contains("no matching open agent todo"), "{c:?}");
}

#[test]
fn smoke_extension_state_arms() {
    let cr: CliRoot = cli_root();
    // A valid extension state file → "N extension(s) readable".
    {
        let dir = std::path::Path::new(&cr.root).join("extensions");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("state.json"),
            "{\"schema_version\":\"future_loop_extension_state_v0\",\"extensions\":{}}",
        )
        .unwrap();
        let store = open_store(&cr);
        let result = run_smoke(&store, "release-gate").unwrap();
        let c = check(&result, "extension_state");
        assert!(c.passed, "{c:?}");
        assert!(c.detail.contains("extension"), "{c:?}");
    }
    // A corrupt one → check fails.
    {
        std::fs::write(
            std::path::Path::new(&cr.root).join("extensions/state.json"),
            "{corrupt",
        )
        .unwrap();
        let store = open_store(&cr);
        let result = run_smoke(&store, "release-gate").unwrap();
        let c = check(&result, "extension_state");
        assert!(!c.passed, "{c:?}");
        assert!(c.detail.contains("corrupt"), "{c:?}");
    }
}

#[test]
fn smoke_backup_check_with_backup() {
    let cr = cli_root();
    let gid = init_goal(&cr, "backup canary");
    {
        let store = open_store(&cr);
        store.backup_goal(&gid).unwrap();
    }
    let store = open_store(&cr);
    let result = run_smoke(&store, "release-gate").unwrap();
    assert!(check(&result, "backup_dir").passed);
}

#[test]
fn smoke_unknown_profile_errors() {
    let cr = cli_root();
    let store = open_store(&cr);
    assert!(run_smoke(&store, "no-such-profile").is_err());
}
