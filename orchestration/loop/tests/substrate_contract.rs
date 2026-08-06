//! P1 contract tests: state substrate completeness — backup/restore,
//! concurrent append safety (file lock), authority + approval gates, and the
//! public/private boundary scan. Deterministic.

use std::thread;

use future_loop::state::{boundary_scan_leaks, Authority, Goal, Todo};
use future_loop::store::{Event, Store};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("loopx-p1-{tag}-{}", nano()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn nano() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

fn seeded_store(root: &str, goal_id: &str) -> Store {
    let mut store = Store::open(root).unwrap();
    let g = Goal::new(goal_id, "objective", "/tmp");
    store.register(&g).unwrap();
    let ts = g.created_at;
    store
        .append(Event::GoalStarted {
            goal_id: goal_id.into(),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: goal_id.into(),
            todo: Todo::advancement("t1", "Work"),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoCompleted {
            goal_id: goal_id.into(),
            todo_id: "t1".into(),
            no_follow_up: true,
            successor_ids: vec![],
            evidence: None,
            ts,
        })
        .unwrap();
    store
}

// ── Contract: backup captures state; restore recovers it ───────────────────
#[test]
fn backup_and_restore_roundtrip() {
    let root = tmp_root("backup");
    let store = seeded_store(&root, "bg1");

    let backup = store.backup_goal("bg1").unwrap();
    assert!(backup.contains("bg1"));
    assert_eq!(store.backups("bg1").len(), 1);

    // Corrupt current state: append garbage is hard; simulate by deleting runs
    // and re-restoring from the snapshot.
    let goal_dir = store.goal_dir("bg1");
    std::fs::remove_file(goal_dir.join("events.jsonl")).unwrap();

    let store2 = Store::open(&root).unwrap();
    store2.restore_goal("bg1", &backup).unwrap();
    let rebuilt = Store::open(&root).unwrap().replay("bg1").unwrap().unwrap();
    assert_eq!(
        rebuilt.todo("t1").unwrap().status,
        future_loop::state::TodoStatus::Done
    );
}

// ── Contract: concurrent appends do not interleave lines (file lock) ───────
#[test]
fn concurrent_appends_are_line_safe() {
    let root = tmp_root("lock");
    let mut store = Store::open(&root).unwrap();
    let g = Goal::new("bg2", "objective", "/tmp");
    store.register(&g).unwrap();
    let ts = g.created_at;
    store
        .append(Event::GoalStarted {
            goal_id: "bg2".into(),
            ts,
        })
        .unwrap();

    let goal_id = "bg2".to_string();
    let root_c = root.clone();
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let root_c = root_c.clone();
            let goal_id = goal_id.clone();
            thread::spawn(move || {
                let mut s = Store::open(&root_c).unwrap();
                s.append(Event::TodoAdded {
                    goal_id: goal_id.clone(),
                    todo: Todo::advancement(&format!("t{i}"), &format!("work {i}")),
                    ts: 0,
                })
                .unwrap();
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    let rebuilt = Store::open(&root).unwrap().replay("bg2").unwrap().unwrap();
    assert_eq!(rebuilt.todos.len(), 8, "all 8 events must survive intact");
    let ids: std::collections::HashSet<_> = rebuilt.todos.iter().map(|t| t.id.clone()).collect();
    assert_eq!(ids.len(), 8, "no interleaved/corrupted lines");
}

// ── Contract: authority approval gates ─────────────────────────────────────
#[test]
fn approval_required_for_irreversible_actions() {
    let a = Authority::default();
    assert!(a.approval_required_for("publish"));
    assert!(a.approval_required_for("merge"));
    assert!(!a.approval_required_for("shell"));
}

// ── Contract: boundary scan flags private material in evidence ─────────────
#[test]
fn boundary_scan_flags_home_paths_and_secrets() {
    let home = std::env::var("HOME").unwrap_or_default();
    let leaks = boundary_scan_leaks(&format!("wrote to {home}/secrets/auth.json"));
    assert!(!leaks.is_empty(), "home path + auth.json must be flagged");
    let leak_text = leaks.join(";");
    assert!(leak_text.contains("home path") || leak_text.contains("auth.json"));

    assert!(boundary_scan_leaks("wrote hello.txt, verify passed").is_empty());
}
