//! Goal boundary — the stable boundary appended once to the session system
//! prompt and the packet's `goal_boundary` JSON (repo + write scope).

use crate::state::Goal;

/// Compose the stable goal boundary appended once to the session system prompt.
pub fn compose_goal_boundary(goal: &Goal) -> String {
    let acceptance = if goal.acceptance.is_empty() {
        "see per-todo acceptance".to_string()
    } else {
        goal.acceptance
            .iter()
            .map(|g| format!("- {} ({})", g.description, g.id))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "You are an executor working under a deterministic control plane.\n\
         \n\
         GOAL: {}\n\
         ACCEPTANCE (what 'done' means):\n{}\n\
         \n\
         Rules:\n\
         - Work exactly one todo per turn. Do not invent work outside the goal.\n\
         - Write evidence: report concrete results (file paths, values, diffs).\n\
         - When the todo is complete, stop — do not continue into extra work.\n\
         - You cannot change the goal or its acceptance.",
        goal.objective, acceptance,
    )
}

/// Compose the packet's `goal_boundary` JSON (repo + write scope).
pub(crate) fn goal_boundary_json(goal: &Goal) -> serde_json::Value {
    serde_json::json!({
        "repo": goal.cwd,
        "write_scope": goal.authority.write_scope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Goal;

    #[test]
    fn boundary_prompt_lists_acceptance_and_rules() {
        let g =
            Goal::new("g", "ship the widget", "/tmp").with_acceptance(vec![("A1", "tests pass")]);
        let prompt = compose_goal_boundary(&g);
        assert!(prompt.contains("GOAL: ship the widget"));
        assert!(prompt.contains("- tests pass (A1)"));
        assert!(prompt.contains("Work exactly one todo per turn."));
        assert!(!prompt.contains("see per-todo acceptance"));
    }

    #[test]
    fn boundary_prompt_defaults_acceptance_note() {
        let g = Goal::new("g", "ship the widget", "/tmp");
        assert!(compose_goal_boundary(&g).contains("see per-todo acceptance"));
    }

    #[test]
    fn goal_boundary_json_carries_repo_and_write_scope() {
        let g = Goal::new("g", "o", "/repo/path");
        let v = goal_boundary_json(&g);
        assert_eq!(v["repo"].as_str(), Some("/repo/path"));
        assert_eq!(
            v["write_scope"],
            serde_json::json!(g.authority.write_scope),
            "write_scope must serialize as its Vec<String>"
        );
    }
}
