//! Agent scope (G-16) — LoopX `control_plane/agents/agent_scope.py`,
//! natively. The identity-scoped frontier: every agent session holds the
//! frontier of {unclaimed work} ∪ {its own claimed work} — never another
//! agent's claimed slices (equal-peer, no central leader; the first claimant
//! wins). Excluded agents are hard-invisible even when unclaimed.
//!
//! This is the single-process multi-agent reservation from the P3 plan; the
//! cross-process A2A protocol stays a contract-schema concern.

use crate::state::{Goal, TaskClass, Todo, TodoStatus};

pub const AGENT_TASK_SCOPE: &str = "goal_all_read_claimed_run_global_read_v0";
pub const AGENT_SCOPE_SCHEMA_VERSION: &str = "agent_scope_frontier_v0";

/// The identity-scoped frontier for one agent session.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentScopeProjection {
    pub schema_version: String,
    pub agent_id: String,
    pub task_scope: String,
    /// Visible agent-owned todo ids: unclaimed + claimed by this agent
    /// (advancement / monitor / blocker), minus exclusions.
    pub visible_agent_todo_ids: Vec<String>,
    /// Todo ids claimed by THIS agent (inside the frontier).
    pub claimed_todo_ids: Vec<String>,
    /// Todo ids claimed by OTHER agents — OUTSIDE this frontier (boundary).
    pub other_agent_claimed_ids: Vec<String>,
    /// Open user gates (visible to every agent; they gate the goal).
    pub open_user_gate_ids: Vec<String>,
    /// User actions bound to another agent — diagnostic-only, not reminders.
    pub other_agent_bound_user_action_ids: Vec<String>,
    pub unclaimed_advancement_count: usize,
    pub boundary: AgentScopeBoundary,
}

/// The scope boundary summary (why the frontier looks the way it does).
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentScopeBoundary {
    pub policy: String,
    pub visible_count: usize,
    pub other_agent_claimed_count: usize,
    pub excluded_agents: Vec<String>,
}

impl AgentScopeProjection {
    /// True when a todo id is inside this agent's frontier.
    pub fn contains(&self, todo_id: &str) -> bool {
        self.visible_agent_todo_ids.iter().any(|id| id == todo_id)
            || self.open_user_gate_ids.iter().any(|id| id == todo_id)
    }
}

/// A todo is agent-owned work (advancement / monitor / blocker), not a user
/// todo.
fn is_agent_work(todo: &Todo) -> bool {
    [
        TaskClass::Advancement,
        TaskClass::Monitor,
        TaskClass::Blocker,
    ]
    .contains(&todo.class)
}

/// Is the todo currently actionable for scoping (open or deferred-with-resume)?
fn is_frontier_active(todo: &Todo) -> bool {
    matches!(
        todo.status,
        TodoStatus::Open | TodoStatus::Deferred | TodoStatus::Blocked
    )
}

/// Build the identity-scoped frontier (LoopX agent_scope).
///
/// `excluded` is the caller-supplied session exclusion list (agent ids this
/// session must never see, e.g. from a supervisor routing table); it is not
/// a todo schema change.
pub fn identity_scoped_frontier(
    goal: &Goal,
    agent_id: &str,
    excluded: &[String],
) -> AgentScopeProjection {
    let mut visible_agent = vec![];
    let mut claimed = vec![];
    let mut other_claimed = vec![];
    let mut open_gates = vec![];
    let mut other_bound_user_actions = vec![];
    let mut unclaimed_advancement = 0usize;

    for todo in &goal.todos {
        if excluded.iter().any(|e| e == agent_id) {
            // Excluded session: nothing is visible.
            continue;
        }
        if todo.role == crate::state::TodoRole::User {
            match todo.class {
                TaskClass::UserGate => {
                    if todo.status == TodoStatus::Open {
                        open_gates.push(todo.id.clone());
                    }
                }
                TaskClass::UserAction => {
                    // Bound user actions (claimed_by on a user action) owned
                    // by another agent stay diagnostic-only.
                    if let Some(owner) = todo.claimed_by.as_deref() {
                        if owner != agent_id && todo.status == TodoStatus::Open {
                            other_bound_user_actions.push(todo.id.clone());
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        if !is_agent_work(todo) || !is_frontier_active(todo) {
            continue;
        }
        // Owner scope is checked BEFORE the lease claim: an `owner`-scoped
        // todo is reserved for its owning agent and is OUTSIDE every other
        // agent's frontier, even while unclaimed (no lease yet). The old code
        // only consulted `claimed_by`, so an unclaimed owner-scoped todo
        // surfaced in EVERY agent's "visible" list and inflated the unclaimed
        // count (a goal of ten `--owner solver-*` todos showed all ten to
        // every worker).
        match todo.owner.as_deref() {
            Some(o) if o != agent_id => {
                // Reserved for another agent — OUTSIDE this frontier.
                other_claimed.push(todo.id.clone());
                continue;
            }
            _ => {}
        }
        match todo.claimed_by.as_deref() {
            None => {
                visible_agent.push(todo.id.clone());
                if todo.class == TaskClass::Advancement {
                    unclaimed_advancement += 1;
                }
            }
            Some(owner) if owner == agent_id => {
                visible_agent.push(todo.id.clone());
                claimed.push(todo.id.clone());
            }
            Some(_) => {
                // Claimed by another agent — OUTSIDE this frontier.
                other_claimed.push(todo.id.clone());
            }
        }
    }

    let visible_count = visible_agent.len();
    let other_agent_claimed_count = other_claimed.len();
    AgentScopeProjection {
        schema_version: AGENT_SCOPE_SCHEMA_VERSION.to_string(),
        agent_id: agent_id.to_string(),
        task_scope: AGENT_TASK_SCOPE.to_string(),
        visible_agent_todo_ids: visible_agent,
        claimed_todo_ids: claimed,
        other_agent_claimed_ids: other_claimed,
        open_user_gate_ids: open_gates,
        other_agent_bound_user_action_ids: other_bound_user_actions,
        unclaimed_advancement_count: unclaimed_advancement,
        boundary: AgentScopeBoundary {
            policy:
                "identity-scoped: unclaimed + own claims; other agents' claims are never visible"
                    .to_string(),
            visible_count,
            other_agent_claimed_count,
            excluded_agents: excluded.to_vec(),
        },
    }
}

/// Scope a single todo against an agent (LoopX
/// `agent_scope_item_matches_agent_or_unclaimed`): visible when unclaimed,
/// claimed by the agent, or excluded-free.
pub fn todo_matches_agent(todo: &Todo, agent_id: &str) -> bool {
    match todo.claimed_by.as_deref() {
        None => true,
        Some(owner) => owner == agent_id,
    }
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
    fn unclaimed_and_other_class_user_todos_are_not_other_bound() {
        // A user action with NO owner is not diagnostic-bound to anyone.
        let mut free_action = Todo::user_gate("u1", "pick", &[]);
        free_action.class = TaskClass::UserAction;
        free_action.claimed_by = None;
        // A user-role todo whose class is not gate/action falls through.
        let mut odd = Todo::user_gate("u2", "note", &[]);
        odd.class = TaskClass::Monitor;
        let goal = goal_with(vec![free_action, odd]);
        let frontier = identity_scoped_frontier(&goal, "agent-a", &[]);
        assert!(frontier.other_agent_bound_user_action_ids.is_empty());
    }

    #[test]
    fn two_agents_hold_disjoint_frontiers() {
        let mut t1 = Todo::advancement("t1", "agent A work");
        t1.claimed_by = Some("agent-a".into());
        let mut t2 = Todo::advancement("t2", "agent B work");
        t2.claimed_by = Some("agent-b".into());
        let t3 = Todo::advancement("t3", "unclaimed work");
        let goal = goal_with(vec![t1, t2, t3]);

        let a = identity_scoped_frontier(&goal, "agent-a", &[]);
        assert!(a.contains("t1"));
        assert!(a.contains("t3"));
        assert!(!a.contains("t2"), "A must never see B's claimed todo");
        assert_eq!(a.other_agent_claimed_ids, vec!["t2"]);

        let b = identity_scoped_frontier(&goal, "agent-b", &[]);
        assert!(b.contains("t2"));
        assert!(b.contains("t3"));
        assert!(!b.contains("t1"), "B must never see A's claimed todo");
        assert_eq!(b.other_agent_claimed_ids, vec!["t1"]);
    }

    #[test]
    fn owner_scoped_todo_visible_only_to_its_owner() {
        // Regression: scope only consulted `claimed_by`, so an unclaimed
        // `--owner solver-b` todo surfaced in EVERY agent's visible list and
        // inflated the unclaimed count. Owner-scoped todos must be visible
        // only to their owner, and counted as other-agent work elsewhere.
        let mut owned = Todo::advancement("t-owned", "reserved for B");
        owned.owner = Some("agent-b".into());
        let goal = goal_with(vec![owned]);

        let a = identity_scoped_frontier(&goal, "agent-a", &[]);
        assert!(
            !a.contains("t-owned"),
            "A must not see B's owner-scoped todo"
        );
        assert_eq!(a.other_agent_claimed_ids, vec!["t-owned"]);
        assert_eq!(a.unclaimed_advancement_count, 0);

        let b = identity_scoped_frontier(&goal, "agent-b", &[]);
        assert!(b.contains("t-owned"), "B sees its own owner-scoped todo");
        assert_eq!(b.unclaimed_advancement_count, 1);
    }

    #[test]
    fn unclaimed_todo_appears_in_both_frontiers_first_claimant_wins() {
        let t = Todo::advancement("t-unclaimed", "shared");
        let goal = goal_with(vec![t]);
        let a = identity_scoped_frontier(&goal, "agent-a", &[]);
        let b = identity_scoped_frontier(&goal, "agent-b", &[]);
        assert!(a.contains("t-unclaimed"));
        assert!(b.contains("t-unclaimed"));
        assert_eq!(a.unclaimed_advancement_count, 1);
    }

    #[test]
    fn excluded_agent_sees_nothing() {
        let t = Todo::advancement("t1", "work");
        let goal = goal_with(vec![t]);
        let frontier = identity_scoped_frontier(&goal, "agent-a", &["agent-a".to_string()]);
        assert!(frontier.visible_agent_todo_ids.is_empty());
        assert_eq!(
            frontier.boundary.excluded_agents,
            vec!["agent-a".to_string()]
        );
    }

    #[test]
    fn completed_work_leaves_the_frontier() {
        let mut t = Todo::advancement("t-done", "done work");
        t.status = TodoStatus::Done;
        let goal = goal_with(vec![t]);
        let frontier = identity_scoped_frontier(&goal, "agent-a", &[]);
        assert!(frontier.visible_agent_todo_ids.is_empty());
    }

    #[test]
    fn user_gates_visible_to_all_agents() {
        let gate = Todo::user_gate("g1", "approve?", &["t1"]);
        let goal = goal_with(vec![gate]);
        let a = identity_scoped_frontier(&goal, "agent-a", &[]);
        let b = identity_scoped_frontier(&goal, "agent-b", &[]);
        assert!(a.contains("g1"));
        assert!(b.contains("g1"));
    }
}
