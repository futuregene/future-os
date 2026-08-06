//! G-4 privacy-graded multi-projection contract tests: three-tier grading
//! (public_safe / local_private / private_pointer), redaction in public-safe
//! projections, the status-cache projection with ledger-digest staleness,
//! and end-to-end projection building.

use future_loop::projection::build_projections;
use future_loop::projection::privacy::{self, PrivacyLevel};
use future_loop::projection::status_cache;
use future_loop::state::{Goal, Todo};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "future-loop-p2-privacy-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn goal_with_private_todo() -> Goal {
    let mut goal = Goal::new("g1", "objective", "/tmp");
    goal.add(Todo::advancement("t1", "public work"));
    let mut private = Todo::advancement("t2", "edit /Users/geilige/secret with token=abc123");
    private.evidence = Some("path /var/folders/x wrote to".to_string());
    goal.add(private);
    goal.add(Todo::advancement("t3", ""));
    goal
}

/// ── Grading: public / local-private / private-pointer tiers ───────────────
#[test]
fn grades_todos_into_three_tiers() {
    let goal = goal_with_private_todo();
    let report = privacy::grade_goal(&goal);
    assert_eq!(report.item_count, 3);
    assert_eq!(report.public_safe_count, 1);
    assert_eq!(report.local_private_count, 1);
    assert_eq!(report.private_pointer_count, 1);
    // Conservative overall: any pointer ⇒ pointer (under-project, never leak).
    assert_eq!(report.overall, PrivacyLevel::PrivatePointer);

    let t2 = report.items.iter().find(|i| i.todo_id == "t2").unwrap();
    assert!(t2.private_fields.contains(&"text".to_string()));
    assert!(t2.private_fields.contains(&"evidence".to_string()));
    let t3 = report.items.iter().find(|i| i.todo_id == "t3").unwrap();
    assert_eq!(
        t3.level,
        PrivacyLevel::PrivatePointer,
        "unknown content → pointer"
    );
}

/// ── Public-safe projection redacts private surfaces ───────────────────────
#[test]
fn public_projection_redacts_private_surfaces() {
    let goal = goal_with_private_todo();
    let root = tmp_root("redact");
    let dir = std::path::Path::new(&root);
    let projections = build_projections(&goal, PrivacyLevel::PublicSafe, dir);
    assert!(!projections.public_markdown.contains("/Users/geilige"));
    assert!(!projections.public_markdown.contains("token=abc123"));
    assert!(!projections.public_markdown.contains("/var/folders"));
    assert!(projections
        .public_markdown
        .contains("[redacted-private-state]"));
    // The local-private lens preserves the full render.
    assert!(projections
        .local_private_markdown
        .contains("/Users/geilige"));
    assert_eq!(projections.private_pointer_count, 1);
}

/// ── Status cache: build → persist → read, staleness on ledger change ──────
#[test]
fn status_cache_roundtrip_and_staleness() {
    let root = tmp_root("cache");
    let mut store = future_loop::store::Store::open(&root).unwrap();
    let goal = Goal::new("g1", "objective", "/tmp");
    store.register(&goal).unwrap();
    let ts = goal.created_at;
    store
        .append(future_loop::store::Event::GoalStarted {
            goal_id: "g1".into(),
            ts,
        })
        .unwrap();
    store
        .append(future_loop::store::Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t1", "work"),
            ts,
        })
        .unwrap();
    store.set_next_action("g1", "do work").unwrap();

    let goal = store.replay("g1").unwrap().unwrap();
    let goal_dir = store.goal_dir("g1");
    let digest = status_cache::ledger_digest(&goal_dir);
    let cache = status_cache::build_status_cache(&goal, &digest, 1_700_000_000);
    assert_eq!(cache.todo_count, 1);
    assert_eq!(cache.open_agent_todos, 1);
    assert_eq!(cache.next_action.as_deref(), Some("do work"));

    status_cache::write_status_cache(&goal_dir, &cache).unwrap();
    let read = status_cache::read_status_cache(&goal_dir).unwrap();
    assert!(!status_cache::status_cache_stale(&read, &digest));

    // Appending an event changes the ledger head → the cache is stale.
    store
        .append(future_loop::store::Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t2", "more"),
            ts,
        })
        .unwrap();
    let new_digest = status_cache::ledger_digest(&goal_dir);
    assert_ne!(digest, new_digest);
    assert!(status_cache::status_cache_stale(&read, &new_digest));
    // Refresh rebuilds against the new head.
    let goal = store.replay("g1").unwrap().unwrap();
    let refreshed = status_cache::refresh_status_cache(&goal, &goal_dir).unwrap();
    assert_eq!(refreshed.todo_count, 2);
    assert!(!status_cache::status_cache_stale(
        &refreshed,
        &status_cache::ledger_digest(&goal_dir)
    ));
}

/// ── Projection set carries both lenses + the cache ────────────────────────
#[test]
fn projection_set_contains_all_read_models() {
    let goal = goal_with_private_todo();
    let root = tmp_root("set");
    let dir = std::path::Path::new(&root);
    let set = build_projections(&goal, PrivacyLevel::LocalPrivate, dir);
    assert_eq!(set.schema_version, "goal_projection_set_v0");
    assert_eq!(set.goal_id, "g1");
    let cache = set.status_cache.as_ref().unwrap();
    assert_eq!(cache.summary.agent_open, 3);
    assert_eq!(set.privacy_report.overall, PrivacyLevel::PrivatePointer);
}
