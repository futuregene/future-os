//! G-14 task-graph contract tests: the todo dependency graph through the
//! store (todo add --blocks), predecessor/successor validation, cycle
//! detection (fail closed), and topological order.

use future_loop::state::{Goal, Todo};
use future_loop::store::{Event, Store};
use future_loop::work_items::task_graph::{build_task_graph, predecessors_of, successors_of};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "future-loop-p3-taskgraph-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn goal_with(todos: Vec<Todo>) -> Goal {
    let mut goal = Goal::new("g1", "objective", "/tmp");
    goal.todos = todos;
    goal
}

#[test]
fn gate_and_dependent_form_blocks_chain() {
    let g1 = Todo::user_gate("g1", "approve?", &["t2"]); // g1 → t2
    let t2 = Todo::advancement("t2", "work"); // (no blockers)
    let t3 = Todo::advancement("t3", "final").blocking(&["t2"]); // t2 → t3
    let goal = goal_with(vec![t3.clone(), g1, t2.clone()]);
    let graph = build_task_graph(&goal).unwrap();
    assert!(graph.cycle.is_none());
    let order = graph.topological_order.unwrap();
    let pos = |id: &str| order.iter().position(|o| o == id).unwrap();
    assert!(pos("g1") < pos("t2"));
    assert!(pos("t2") < pos("t3"));
    assert_eq!(successors_of(&goal, "g1"), vec!["t2"]);
    assert_eq!(predecessors_of(&t2), Vec::<String>::new());
    assert_eq!(predecessors_of(&t3), vec!["t2"]);
}

#[test]
fn cycle_is_detected_fail_closed() {
    let a = Todo::advancement("a", "a").blocking(&["b"]);
    let b = Todo::advancement("b", "b").blocking(&["a"]);
    let goal = goal_with(vec![a, b]);
    let graph = build_task_graph(&goal).unwrap();
    assert!(graph.topological_order.is_none());
    let path = graph.cycle.unwrap();
    assert_eq!(path.len(), 3, "cycle path a→b→a");
    assert_eq!(path[0], path[2]);
}

#[test]
fn invalid_references_fail_closed() {
    // Self reference.
    let a = Todo::advancement("a", "a").blocking(&["a"]);
    let err = build_task_graph(&goal_with(vec![a])).unwrap_err();
    assert!(err.contains("cannot reference itself"));
    // Unknown reference.
    let b = Todo::advancement("b", "b").blocking(&["missing"]);
    let err = build_task_graph(&goal_with(vec![b])).unwrap_err();
    assert!(err.contains("references unknown todo"));
}

#[test]
fn diamond_dependency_is_acyclic() {
    let b = Todo::advancement("b", "b").blocking(&["a"]); // a → b
    let c = Todo::advancement("c", "c").blocking(&["a"]); // a → c
    let d = Todo::advancement("d", "d").blocking(&["b", "c"]); // b → d, c → d
    let a = Todo::advancement("a", "a");
    let goal = goal_with(vec![d, c, b, a]);
    let graph = build_task_graph(&goal).unwrap();
    let order = graph.topological_order.unwrap();
    let pos = |id: &str| order.iter().position(|o| o == id).unwrap();
    assert!(pos("a") < pos("b") && pos("a") < pos("c"));
    assert!(pos("b") < pos("d") && pos("c") < pos("d"));
}

/// ── Store integration: `todo add --blocks` builds a valid graph ──────────
#[test]
fn store_blocks_relation_builds_graph() {
    let root = tmp_root("store");
    let mut store = Store::open(&root).unwrap();
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
            todo: Todo::user_gate("g1", "approve?", &["t2"]),
            ts: 1,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t2", "work"),
            ts: 2,
        })
        .unwrap();
    let replayed = store.replay("g1").unwrap().unwrap();
    let graph = build_task_graph(&replayed).unwrap();
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.edges[0].from, "g1");
    assert_eq!(graph.edges[0].to, "t2");
    assert_eq!(graph.edges[0].relation, "blocks");
    assert_eq!(graph.nodes.len(), 2);
}

#[test]
fn schema_version_is_stable() {
    assert_eq!(
        future_loop::work_items::task_graph::TASK_GRAPH_SCHEMA_VERSION,
        "task_graph_v0"
    );
}
