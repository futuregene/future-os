//! Project handoff (G-17) — LoopX `control_plane/handoff/project_handoff.py`,
//! natively (minimal set). Generates the handoff document from the durable
//! projections: the goal doc (GOAL.md), the active-state markdown
//! (ACTIVE_GOAL_STATE.md via the LoopX-compatible projection), the todo
//! frontier, and the run-history evidence. The handoff is the交接 document a
//! successor agent (or a human reviewer) consumes to continue the work.

use crate::state::Goal;

pub const PROJECT_HANDOFF_SCHEMA_VERSION: &str = "project_handoff_v0";

/// The sections of a handoff document.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectHandoff {
    pub schema_version: String,
    pub goal_id: String,
    pub generated_at: u64,
    pub objective: String,
    pub cwd: String,
    pub open_advancement_count: usize,
    pub open_gate_count: usize,
    pub run_count: usize,
    pub latest_run: Option<serde_json::Value>,
    pub delivery_contract: Option<String>,
    pub active_state_markdown: String,
}

/// Build the handoff document for one goal (from its projected state).
pub fn build_project_handoff(goal: &Goal, delivery_contract: Option<&str>) -> ProjectHandoff {
    let open_advancement = goal.open_of(crate::state::TaskClass::Advancement).count();
    let open_gates = goal.open_gates().count();
    let latest_run = goal
        .history
        .last()
        .map(|r| serde_json::to_value(r).unwrap_or_default());
    ProjectHandoff {
        schema_version: PROJECT_HANDOFF_SCHEMA_VERSION.to_string(),
        goal_id: goal.goal_id.clone(),
        generated_at: crate::state::now_epoch(),
        objective: goal.objective.clone(),
        cwd: goal.cwd.clone(),
        open_advancement_count: open_advancement,
        open_gate_count: open_gates,
        run_count: goal.history.len(),
        latest_run,
        delivery_contract: delivery_contract.map(|s| s.to_string()),
        active_state_markdown: crate::compat::render_active_state(goal),
    }
}

/// Render the handoff as markdown (the交接 document).
pub fn render_project_handoff_markdown(handoff: &ProjectHandoff) -> String {
    let mut out = String::new();
    out.push_str("# Project Handoff\n\n");
    out.push_str(&format!("- goal_id: `{}`\n", handoff.goal_id));
    out.push_str(&format!("- generated_at: `{}`\n", handoff.generated_at));
    out.push_str(&format!("- objective: {}\n", handoff.objective));
    out.push_str(&format!("- cwd: `{}`\n", handoff.cwd));
    out.push('\n');
    out.push_str("## Frontier\n\n");
    out.push_str(&format!(
        "- open advancement todos: `{}`\n",
        handoff.open_advancement_count
    ));
    out.push_str(&format!(
        "- open user gates: `{}`\n",
        handoff.open_gate_count
    ));
    out.push_str(&format!("- runs recorded: `{}`\n", handoff.run_count));
    out.push('\n');
    if let Some(contract) = &handoff.delivery_contract {
        out.push_str("## Delivery Contract\n\n");
        out.push_str(contract);
        out.push_str("\n\n");
    }
    out.push_str("## Active State\n\n");
    out.push_str(&handoff.active_state_markdown);
    out
}

/// Write the handoff document next to the goal projection
/// (`<cwd>/.future/loop/goals/<id>/HANDOFF.md`).
pub fn write_project_handoff(
    goal_dir: &std::path::Path,
    _goal: &Goal,
    handoff: &ProjectHandoff,
) -> std::io::Result<()> {
    std::fs::create_dir_all(goal_dir)?;
    std::fs::write(
        goal_dir.join("HANDOFF.md"),
        render_project_handoff_markdown(handoff),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Goal, Todo};

    #[test]
    fn handoff_reflects_frontier_and_active_state() {
        let mut goal = Goal::new("g1", "objective", "/tmp");
        goal.todos = vec![
            Todo::advancement("t1", "open work"),
            Todo::user_gate("g1", "approve?", &["t1"]),
        ];
        let handoff = build_project_handoff(&goal, Some("expand after repeated small delivery"));
        assert_eq!(handoff.open_advancement_count, 1);
        assert_eq!(handoff.open_gate_count, 1);
        assert!(handoff
            .active_state_markdown
            .contains("# Active Goal State"));
        let md = render_project_handoff_markdown(&handoff);
        assert!(md.contains("# Project Handoff"));
        assert!(md.contains("Delivery Contract"));
        assert!(md.contains("expand after repeated small delivery"));
    }
}
