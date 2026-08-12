//! Task graph (G-14) — LoopX `control_plane/work_items/task_graph.py`,
//! natively (data-structure side). The todo work graph: a todo's
//! `blocked_by_gate` ids are its predecessors (the gates/blockers that must
//! resolve first); every todo that lists it is its successor. The graph
//! validates predecessor/successor references, detects cycles FAIL-CLOSED,
//! and produces a topological order — the数据结构 multi-agent work
//! splitting (G-16) builds on. (LoopX's task_graph.py is the read-only
//! projection side; this module is the dependency-graph core.)

use std::collections::{BTreeMap, BTreeSet};

use crate::state::{Goal, TaskClass, Todo};

pub const TASK_GRAPH_SCHEMA_VERSION: &str = "task_graph_v0";

/// One graph node (a todo).
#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskGraphNode {
    pub todo_id: String,
    pub class: String,
    pub status: String,
    pub text: String,
}

/// One directed edge: `from` (predecessor) blocks `to` (successor).
#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskGraphEdge {
    pub from: String,
    pub to: String,
    pub relation: String,
}

/// The validated dependency graph.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskGraph {
    pub schema_version: String,
    pub nodes: Vec<TaskGraphNode>,
    pub edges: Vec<TaskGraphEdge>,
    /// Topological order (predecessors before successors). Only present when
    /// the graph is acyclic.
    pub topological_order: Option<Vec<String>>,
    /// Cycle path when detected (fail closed).
    pub cycle: Option<Vec<String>>,
}

/// Parse a comma-separated `blocked_by_gate` value into todo ids.
fn predecessor_ids(todo: &Todo) -> Vec<String> {
    todo.blocked_by_gate
        .as_deref()
        .map(|s| {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Is this todo itself a blocking source (gate or blocker)? Its
/// `blocked_by_gate` list names the todos it BLOCKS (dependents), whereas an
/// advancement/monitor's `blocked_by_gate` names the todos that block IT.
fn is_blocking_source(todo: &Todo) -> bool {
    matches!(todo.class, TaskClass::UserGate | TaskClass::Blocker)
}

/// Build the graph from a goal's todos. Validation (fail closed):
/// - a predecessor reference to an unknown todo id is an error;
/// - a self-reference is an error;
/// - a cycle is an error (detected, reported in `cycle`, topological order
///   withheld).
///
/// Edge direction: gates/blockers list the todos they block
/// (`Todo::user_gate(.., &["T2"])` → edge gate→T2); advancement/monitor
/// todos list the gates that block them (`.blocking(&["B1"])` → edge B1→T).
/// Both declarations produce predecessor→successor edges and dedupe.
pub fn build_task_graph(goal: &Goal) -> Result<TaskGraph, String> {
    let known: BTreeSet<&str> = goal.todos.iter().map(|t| t.id.as_str()).collect();
    let mut edges: Vec<(String, String)> = vec![];
    for todo in &goal.todos {
        let refs = predecessor_ids(todo);
        for other in refs {
            if other == todo.id {
                return Err(format!("todo `{}` cannot reference itself", todo.id));
            }
            if !known.contains(other.as_str()) {
                return Err(format!(
                    "todo `{}` references unknown todo `{other}`",
                    todo.id
                ));
            }
            // Direction-aware: a blocking source gates its dependents; a
            // dependent is gated by its blocking sources. In both cases the
            // referenced todo is the predecessor.
            let (from, to) = if is_blocking_source(todo) {
                (todo.id.clone(), other)
            } else {
                (other, todo.id.clone())
            };
            edges.push((from, to));
        }
    }
    // Dedupe edges.
    edges.sort();
    edges.dedup();

    let nodes: Vec<TaskGraphNode> = goal
        .todos
        .iter()
        .map(|t| TaskGraphNode {
            todo_id: t.id.clone(),
            class: format!("{:?}", t.class).to_lowercase(),
            status: format!("{:?}", t.status).to_lowercase(),
            text: t.text.clone(),
        })
        .collect();
    let edges_out: Vec<TaskGraphEdge> = edges
        .iter()
        .map(|(from, to)| TaskGraphEdge {
            from: from.clone(),
            to: to.clone(),
            relation: "blocks".to_string(),
        })
        .collect();

    // Cycle detection (fail closed): Kahn's algorithm; leftover nodes form
    // a cycle path.
    let (order, cycle) = topological_sort(&goal.todos, &edges);
    Ok(TaskGraph {
        schema_version: TASK_GRAPH_SCHEMA_VERSION.to_string(),
        nodes,
        edges: edges_out,
        topological_order: if cycle.is_empty() { Some(order) } else { None },
        cycle: if cycle.is_empty() { None } else { Some(cycle) },
    })
}

/// Kahn topological sort. Returns (order, cycle_path) — an empty cycle path
/// means acyclic. The cycle path is a concrete cycle (from one leftover node
/// walking successors back to itself) so callers can see exactly what broke.
pub fn topological_sort(todos: &[Todo], edges: &[(String, String)]) -> (Vec<String>, Vec<String>) {
    let ids: BTreeSet<&str> = todos.iter().map(|t| t.id.as_str()).collect();
    let mut indegree: BTreeMap<String, usize> = ids.iter().map(|id| (id.to_string(), 0)).collect();
    let mut adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (from, to) in edges {
        if !ids.contains(from.as_str()) || !ids.contains(to.as_str()) {
            continue;
        }
        *indegree.entry(to.clone()).or_insert(0) += 1;
        adj.entry(from.clone()).or_default().push(to.clone());
    }
    // Stable order: process ids in sorted order for determinism.
    let mut queue: Vec<String> = indegree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(id, _)| id.clone())
        .collect();
    queue.sort();
    let mut order: Vec<String> = vec![];
    while let Some(id) = queue.first().cloned() {
        queue.remove(0);
        order.push(id.clone());
        if let Some(next) = adj.get(&id) {
            for n in next {
                // adj only holds ids that passed the ids.contains filter
                // above, so every successor has an indegree entry.
                let d = indegree
                    .get_mut(n)
                    .expect("adjacency ids all have indegree entries");
                *d -= 1;
                if *d == 0 {
                    queue.push(n.clone());
                    queue.sort();
                }
            }
        }
    }
    if order.len() == indegree.len() {
        return (order, vec![]);
    }
    // Cycle: walk from a leftover node back to itself.
    let leftover: Vec<String> = indegree
        .iter()
        .filter(|(id, _)| !order.contains(id))
        .map(|(id, _)| id.clone())
        .collect();
    let start = leftover[0].clone();
    let mut path = vec![start.clone()];
    let mut current = start.clone();
    let mut seen = BTreeSet::new();
    seen.insert(current.clone());
    loop {
        let next = adj
            .get(&current)
            .and_then(|ns| ns.iter().find(|n| leftover.contains(n)))
            .cloned();
        match next {
            Some(n) => {
                if n == start {
                    path.push(n);
                    break;
                }
                if !seen.insert(n.clone()) {
                    break;
                }
                path.push(n.clone());
                current = n;
            }
            None => break,
        }
    }
    (order, path)
}

/// Successor ids of a todo (everything it blocks / everything listing it as a
/// predecessor).
pub fn successors_of(goal: &Goal, todo_id: &str) -> Vec<String> {
    let Some(todo) = goal.todo(todo_id) else {
        return vec![];
    };
    let refs = predecessor_ids(todo);
    if is_blocking_source(todo) {
        // The todo names its dependents directly.
        return refs;
    }
    // Other todos list this todo as a blocker.
    goal.todos
        .iter()
        .filter(|t| !is_blocking_source(t) && predecessor_ids(t).iter().any(|p| p == todo_id))
        .map(|t| t.id.clone())
        .collect()
}

/// Predecessor ids of a todo (its blocking sources).
pub fn predecessors_of(todo: &Todo) -> Vec<String> {
    predecessor_ids(todo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Todo;

    fn goal_with(todos: Vec<Todo>) -> Goal {
        let mut goal = Goal::new("g1", "objective", "/tmp");
        goal.todos = todos;
        goal
    }

    #[test]
    fn cycle_walk_revisiting_a_non_start_node_breaks() {
        // a → b → c → b (b revisited before the walk returns to a).
        let todos = vec!["a", "b", "c"]
            .into_iter()
            .map(|id| Todo::advancement(id, "w"))
            .collect::<Vec<_>>();
        let edges = [
            ("a".to_string(), "b".to_string()),
            ("b".to_string(), "c".to_string()),
            ("c".to_string(), "b".to_string()),
            ("c".to_string(), "a".to_string()),
        ];
        let (order, path) = topological_sort(&todos, &edges);
        assert!(order.is_empty());
        assert_eq!(path.first().map(String::as_str), Some("a"));
        assert!(path.len() >= 3);
    }

    #[test]
    fn cycle_walk_ending_at_a_dead_end_breaks() {
        // The walk leaves the cycle into d, which has no outgoing edge back
        // into the leftover set.
        let todos = vec!["a", "b", "c", "d"]
            .into_iter()
            .map(|id| Todo::advancement(id, "w"))
            .collect::<Vec<_>>();
        let edges = [
            ("a".to_string(), "b".to_string()),
            ("b".to_string(), "c".to_string()),
            ("c".to_string(), "d".to_string()),
            ("c".to_string(), "a".to_string()),
        ];
        let (order, path) = topological_sort(&todos, &edges);
        assert!(order.is_empty());
        assert_eq!(
            path,
            vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string()
            ]
        );
    }

    #[test]
    fn blocks_chain_produces_topo_order() {
        let g1 = Todo::user_gate("g1", "approve?", &["t2"]); // g1 → t2
        let t2 = Todo::advancement("t2", "work"); // (no blockers)
        let t3 = Todo::advancement("t3", "final").blocking(&["t2"]); // t2 → t3
        let goal = goal_with(vec![t3.clone(), g1, t2.clone()]);
        let graph = build_task_graph(&goal).unwrap();
        assert!(graph.cycle.is_none());
        let order = graph.topological_order.unwrap();
        // g1 → t2 → t3 chain.
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
        assert!(graph.cycle.is_some());
        assert!(graph.topological_order.is_none());
        let path = graph.cycle.unwrap();
        assert_eq!(path.len(), 3, "a→b→a");
        assert_eq!(path[0], path[2]);
    }

    #[test]
    fn self_reference_is_an_error() {
        let a = Todo::advancement("a", "a").blocking(&["a"]);
        let goal = goal_with(vec![a]);
        let err = build_task_graph(&goal).unwrap_err();
        assert!(err.contains("cannot reference itself"));
    }

    #[test]
    fn unknown_predecessor_is_an_error() {
        let a = Todo::advancement("a", "a").blocking(&["missing"]);
        let goal = goal_with(vec![a]);
        let err = build_task_graph(&goal).unwrap_err();
        assert!(err.contains("references unknown todo"));
    }

    #[test]
    fn independent_nodes_are_ordered_deterministically() {
        let a = Todo::advancement("a", "a");
        let b = Todo::advancement("b", "b");
        let goal = goal_with(vec![b, a]);
        let graph = build_task_graph(&goal).unwrap();
        let order = graph.topological_order.unwrap();
        assert_eq!(order, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn diamond_dependency_is_acyclic() {
        let b = Todo::advancement("b", "b").blocking(&["a"]); // a → b
        let c = Todo::advancement("c", "c").blocking(&["a"]); // a → c
        let d = Todo::advancement("d", "d").blocking(&["b", "c"]); // b → d, c → d
        let a = Todo::advancement("a", "a");
        let goal = goal_with(vec![d, c, b, a]);
        let graph = build_task_graph(&goal).unwrap();
        assert!(graph.cycle.is_none());
        let order = graph.topological_order.unwrap();
        let pos = |id: &str| order.iter().position(|o| o == id).unwrap();
        assert!(pos("a") < pos("b") && pos("a") < pos("c"));
        assert!(pos("b") < pos("d") && pos("c") < pos("d"));
    }
}
