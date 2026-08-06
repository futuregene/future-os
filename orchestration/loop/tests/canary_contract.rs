//! G-20 canary smoke contract tests — the smoke-suite profiles and the
//! release gate, deterministic against a temp state root: a healthy root
//! passes every profile; a corrupt ledger fails the release gate.

use future_loop::canary::{run_release_gate, run_smoke, smoke_suite_profiles};
use future_loop::state::{Goal, Todo};
use future_loop::store::{Event, Store};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "future-loop-p4-canary-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn healthy_store(root: &str) -> Store {
    let mut store = Store::open(root).unwrap();
    let goal = Goal::new("g1", "obj", "/tmp");
    store.register(&goal).unwrap();
    store
        .append(Event::GoalStarted {
            goal_id: "g1".to_string(),
            ts: future_loop::state::now_epoch(),
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".to_string(),
            todo: Todo::advancement("T1", "work"),
            ts: future_loop::state::now_epoch(),
        })
        .unwrap();
    store
}

#[test]
fn profile_manifest_is_stable() {
    let profiles = smoke_suite_profiles();
    let ids: Vec<&str> = profiles.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["core-control-plane", "extension-runtime", "release-gate"]
    );
    assert!(future_loop::canary::resolve_smoke_profile("release-gate").is_ok());
    assert!(future_loop::canary::resolve_smoke_profile("nope").is_err());
}

#[test]
fn healthy_root_passes_release_gate() {
    let root = tmp_root("healthy");
    let store = healthy_store(&root);
    let result = run_release_gate(&store).unwrap();
    assert_eq!(result.schema_version, "canary_smoke_run_v0");
    assert_eq!(result.profile_id, "release-gate");
    assert!(result.all_passed, "{:?}", result.checks);
    assert!(result.checks.iter().any(|c| c.id == "ledger_integrity"));
    assert!(result.checks.iter().any(|c| c.id == "capability_catalog"));
    assert!(result.checks.iter().any(|c| c.id == "canary_self"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn empty_root_passes_release_gate() {
    let root = tmp_root("empty");
    let store = Store::open(&root).unwrap();
    let result = run_release_gate(&store).unwrap();
    assert!(result.all_passed, "{:?}", result.checks);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn corrupt_ledger_fails_release_gate() {
    let root = tmp_root("corrupt");
    let store = healthy_store(&root);
    let ledger = store.goal_dir("g1").join("events.jsonl");
    std::fs::write(&ledger, "garbage-line\n").unwrap();
    let result = run_release_gate(&store).unwrap();
    assert!(!result.all_passed);
    assert!(result
        .checks
        .iter()
        .any(|c| !c.passed && c.id == "ledger_integrity"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn per_module_checks_are_present() {
    let root = tmp_root("modules");
    let store = healthy_store(&root);
    let result = run_smoke(&store, "core-control-plane").unwrap();
    let ids: Vec<&str> = result.checks.iter().map(|c| c.id.as_str()).collect();
    for expected in [
        "root_writable",
        "ledger_integrity",
        "decision_determinism",
        "quota_should_run",
        "todo_frontier",
        "status_projection",
    ] {
        assert!(ids.contains(&expected), "missing check {expected}: {ids:?}");
    }
    let _ = std::fs::remove_dir_all(&root);
}
