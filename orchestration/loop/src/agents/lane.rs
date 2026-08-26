//! Agent lane recommendation (G-16) — LoopX
//! `control_plane/agents/agent_lane_recommendation.py`, natively (compact
//! set). The latest run executed on an agent's lane becomes the compact
//! lane recommendation: what the agent should do next, attributed to the
//! agent that owns the run's todo slice.

use crate::state::{Goal, RunRecord};

pub const AGENT_LANE_NEXT_ACTION_SCHEMA_VERSION: &str = "agent_lane_next_action_v0";
pub const AGENT_LANE_PROGRESS_SCOPE: &str = "agent_lane";

/// The compact lane recommendation (LoopX compact_agent_lane_recommendation).
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentLaneRecommendation {
    pub schema_version: String,
    pub progress_scope: String,
    pub agent_id: String,
    pub agent_lane: String,
    pub recommended_action: Option<String>,
    pub classification: String,
    pub generated_at: u64,
    pub run_id: String,
}

/// The latest run on an agent's lane: the newest history record whose todo
/// is claimed by `agent_id` (runs on unclaimed/other-claimed todos are not
/// this agent's lane). Falls back to the newest run overall when the goal has
/// exactly one registered agent (single-agent goals attribute all runs).
pub fn latest_agent_lane_run<'a>(goal: &'a Goal, agent_id: &str) -> Option<&'a RunRecord> {
    let own: Vec<&RunRecord> = goal
        .history
        .iter()
        .filter(|r| {
            goal.todo(&r.todo_id)
                .map(|t| t.claimed_by.as_deref() == Some(agent_id))
                .unwrap_or(false)
        })
        .collect();
    if let Some(run) = own.last() {
        return Some(run);
    }
    if goal.registered_agents.len() == 1 && goal.registered_agents[0] == agent_id {
        return goal.history.last();
    }
    None
}

/// Compact lane recommendation from the latest agent lane run (None when the
/// agent has no lane run yet).
pub fn compact_agent_lane_recommendation(
    goal: &Goal,
    agent_id: &str,
) -> Option<AgentLaneRecommendation> {
    let run = latest_agent_lane_run(goal, agent_id)?;
    let recommended_action = if run.evidence.trim().is_empty() {
        None
    } else {
        Some(crate::decision::truncate(&run.evidence, 220))
    };
    Some(AgentLaneRecommendation {
        schema_version: AGENT_LANE_NEXT_ACTION_SCHEMA_VERSION.to_string(),
        progress_scope: AGENT_LANE_PROGRESS_SCOPE.to_string(),
        agent_id: agent_id.to_string(),
        agent_lane: AGENT_LANE_PROGRESS_SCOPE.to_string(),
        recommended_action,
        classification: run.terminal_state.clone(),
        generated_at: run.recorded_at,
        run_id: run.run_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Goal, RunRecord, Todo};

    fn run(todo_id: &str, run_id: &str, recorded_at: u64) -> RunRecord {
        RunRecord {
            turn: 1,
            todo_id: todo_id.to_string(),
            run_id: run_id.to_string(),
            terminal_state: "completed".to_string(),
            error: None,
            tokens_in_delta: 0,
            tokens_out_delta: 0,
            cost_delta: 0.0,
            tools: vec![],
            evidence: format!("evidence for {todo_id}"),
            recorded_at,
            spend_source: None,
            validation: None,
            failure_kind: None,
            truncation: None,
        }
    }

    fn goal_with_claim(todo_id: &str, agent_id: &str, runs: Vec<RunRecord>) -> Goal {
        let mut todo = Todo::advancement(todo_id, "work");
        todo.claimed_by = Some(agent_id.to_string());
        let mut goal = Goal::new("g1", "objective", "/tmp");
        goal.todos = vec![todo];
        goal.history = runs;
        goal
    }

    #[test]
    fn lane_run_is_attributed_by_todo_claim() {
        let goal = goal_with_claim("t1", "agent-a", vec![run("t1", "r1", 100)]);
        let rec = latest_agent_lane_run(&goal, "agent-a").unwrap();
        assert_eq!(rec.run_id, "r1");
        assert!(latest_agent_lane_run(&goal, "agent-b").is_none());
    }

    #[test]
    fn compact_recommendation_carries_lane_fields() {
        let goal = goal_with_claim("t1", "agent-a", vec![run("t1", "r1", 100)]);
        let rec = compact_agent_lane_recommendation(&goal, "agent-a").unwrap();
        assert_eq!(rec.agent_id, "agent-a");
        assert_eq!(rec.classification, "completed");
        assert_eq!(rec.generated_at, 100);
        assert!(rec
            .recommended_action
            .as_deref()
            .unwrap()
            .contains("evidence for t1"));
    }

    #[test]
    fn single_agent_goal_attributes_all_runs() {
        let mut goal = Goal::new("g1", "objective", "/tmp");
        goal.registered_agents = vec!["agent-a".to_string()];
        goal.history = vec![run("t1", "r1", 100)];
        assert!(latest_agent_lane_run(&goal, "agent-a").is_some());
    }
}
