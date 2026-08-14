//! Terminal judgement (G13 ④) — the reference
//! `goal_frontier/terminal.py` (180 lines; core subset).
//!
//! The tightened terminal determination: closure is validated from complete
//! sources (the structured todo state + acceptance gaps + closure-intent
//! contract), and every remaining blocker is enumerated as an explicit
//! [`TerminalGap`] entry — including acceptance-gap semantics (each
//! unsatisfied gap appears with its id + description, `satisfied: false`).
//!
//! The judgement aligns with the existing `TerminalClosureProof`
//! (`todo_summary` output): it carries the same proof verbatim and adds the
//! gap detail on top. `terminal == gaps.is_empty()` matches
//! `Goal::is_terminal()` exactly — the judgement is the single authoritative
//! gate the decision kernel consults (step 6, validated closure).

use serde::{Deserialize, Serialize};

use crate::state::{Goal, TerminalClosureProof, TodoStatus};

pub const TERMINAL_JUDGEMENT_SCHEMA_VERSION: &str = "goal_terminal_judgement_v0";

/// Terminal-closure kind (reference `goal_terminal_state_v0`).
pub const TERMINAL_KIND_NO_FOLLOWUP: &str = "no_followup";
/// Terminal-closure source (reference: validated closure, never hand-written).
pub const TERMINAL_SOURCE_VALIDATED: &str = "validated_goal_closure";

/// Gap-kind vocabulary — each entry is a blocker that keeps the goal open.
pub const GAP_OPEN_TODO: &str = "open_todo";
pub const GAP_OPEN_MONITOR: &str = "open_monitor";
pub const GAP_PENDING_DEFERRED: &str = "pending_deferred";
pub const GAP_UNSATISFIED_ACCEPTANCE: &str = "unsatisfied_acceptance";
pub const GAP_SUCCESSION: &str = "succession_gap";

/// One blocking item on the road to terminal closure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalGap {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todo_id: Option<String>,
    /// Acceptance-gap id (kind `unsatisfied_acceptance` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_id: Option<String>,
    pub description: String,
    /// Acceptance gaps carry their satisfaction flag (always false here —
    /// a satisfied gap never blocks terminal closure).
    pub satisfied: bool,
}

/// The terminal judgement: closure proof + explicit gap detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalJudgement {
    pub schema_version: String,
    pub terminal: bool,
    /// `no_followup` when terminal; empty otherwise (reference
    /// `goal_terminal_state_v0.kind` is only present for a terminal state).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Explicit blockers; empty ⇔ `terminal`.
    pub gaps: Vec<TerminalGap>,
    /// The existing `TerminalClosureProof` verbatim (todo_summary aligned).
    pub closure_proof: TerminalClosureProof,
}

fn not_done(todo: &crate::state::Todo) -> bool {
    !matches!(todo.status, TodoStatus::Done | TodoStatus::Superseded)
}

/// Derive the terminal judgement from goal state. Pure; deterministic.
/// `terminal` ⇔ `Goal::is_terminal()` (the same sources, enumerated).
pub fn terminal_judgement(goal: &Goal) -> TerminalJudgement {
    let now = std::time::SystemTime::now();
    let mut gaps: Vec<TerminalGap> = vec![];

    for todo in &goal.todos {
        if !not_done(todo) {
            continue;
        }
        match todo.status {
            TodoStatus::Deferred if !todo.is_due_deferred(now) => {
                gaps.push(TerminalGap {
                    kind: GAP_PENDING_DEFERRED.to_string(),
                    todo_id: Some(todo.id.clone()),
                    gap_id: None,
                    description: format!("deferred todo {} not yet due", todo.id),
                    satisfied: false,
                });
            }
            _ => {
                let kind = if todo.class == crate::state::TaskClass::Monitor {
                    GAP_OPEN_MONITOR
                } else {
                    GAP_OPEN_TODO
                };
                gaps.push(TerminalGap {
                    kind: kind.to_string(),
                    todo_id: Some(todo.id.clone()),
                    gap_id: None,
                    description: format!(
                        "todo {} still {} (class {})",
                        todo.id,
                        crate::compat::future_loop_status(todo.status),
                        crate::compat::future_loop_task_class(todo.class)
                    ),
                    satisfied: false,
                });
            }
        }
    }

    // Acceptance-gap semantics: every unsatisfied gap is an explicit
    // terminal gap carrying its id + description + satisfied=false.
    for gap in goal.unsatisfied_gaps() {
        gaps.push(TerminalGap {
            kind: GAP_UNSATISFIED_ACCEPTANCE.to_string(),
            todo_id: None,
            gap_id: Some(gap.id.clone()),
            description: gap.description.clone(),
            satisfied: false,
        });
    }

    // Succession gaps: done advancement without closure intent.
    for todo in goal.completed_without_closure_intent() {
        gaps.push(TerminalGap {
            kind: GAP_SUCCESSION.to_string(),
            todo_id: Some(todo.id.clone()),
            gap_id: None,
            description: format!(
                "todo {} completed without successor or no-follow-up",
                todo.id
            ),
            satisfied: false,
        });
    }

    let closure_proof = goal.todo_summary().terminal_closure_proof;
    TerminalJudgement {
        schema_version: TERMINAL_JUDGEMENT_SCHEMA_VERSION.to_string(),
        terminal: gaps.is_empty(),
        kind: gaps
            .is_empty()
            .then(|| TERMINAL_KIND_NO_FOLLOWUP.to_string()),
        source: gaps
            .is_empty()
            .then(|| TERMINAL_SOURCE_VALIDATED.to_string()),
        gaps,
        closure_proof,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{TaskClass, Todo};
    use std::time::Duration;

    fn closed_goal() -> Goal {
        let mut g = Goal::new("g", "o", "/tmp").with_acceptance(vec![("A1", "match")]);
        let mut todo = Todo::advancement("T1", "work");
        todo.complete(true, vec![]);
        g.add(todo);
        g.satisfy_gap("A1");
        g
    }

    #[test]
    fn closed_goal_is_terminal_no_followup() {
        let g = closed_goal();
        assert!(g.is_terminal());
        let j = terminal_judgement(&g);
        assert!(j.terminal);
        assert_eq!(j.kind.as_deref(), Some(TERMINAL_KIND_NO_FOLLOWUP));
        assert_eq!(j.source.as_deref(), Some(TERMINAL_SOURCE_VALIDATED));
        assert!(j.gaps.is_empty());
        // Aligned with the existing closure proof.
        assert!(j.closure_proof.all_todos_done);
        assert_eq!(j.closure_proof.successor_gap_count, 0);
        assert_eq!(j.closure_proof.monitor_open_count, 0);
    }

    #[test]
    fn open_todo_and_acceptance_gap_enumerate_as_gaps() {
        let mut g = Goal::new("g", "o", "/tmp").with_acceptance(vec![("A1", "match")]);
        g.add(Todo::advancement("T1", "work"));
        let j = terminal_judgement(&g);
        assert!(!j.terminal);
        assert_eq!(j.kind, None);
        assert_eq!(j.source, None);
        assert_eq!(j.gaps.len(), 2);
        let open: Vec<&TerminalGap> = j
            .gaps
            .iter()
            .filter(|gap| gap.kind == GAP_OPEN_TODO)
            .collect();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].todo_id.as_deref(), Some("T1"));
        let acceptance: Vec<&TerminalGap> = j
            .gaps
            .iter()
            .filter(|gap| gap.kind == GAP_UNSATISFIED_ACCEPTANCE)
            .collect();
        assert_eq!(acceptance.len(), 1);
        assert_eq!(acceptance[0].gap_id.as_deref(), Some("A1"));
        assert_eq!(acceptance[0].description, "match");
        assert!(!acceptance[0].satisfied);
        // The proof mirrors the gap detail.
        assert!(!j.closure_proof.all_todos_done);
        // Satisfying the gap + completing with no-follow-up closes it.
        g.satisfy_gap("A1");
        g.todo_mut("T1").unwrap().complete(true, vec![]);
        assert!(terminal_judgement(&g).terminal);
    }

    #[test]
    fn monitor_and_pending_deferred_have_distinct_gap_kinds() {
        let mut g = Goal::new("g", "o", "/tmp");
        let mut done = Todo::advancement("T1", "work");
        done.complete(true, vec![]);
        g.add(done);
        g.add(Todo::monitor("M1", "watch", Duration::from_secs(600)));
        g.add(Todo::deferred("D1", "later", Duration::from_secs(600)));
        let j = terminal_judgement(&g);
        assert!(!j.terminal);
        let kinds: std::collections::HashSet<&str> =
            j.gaps.iter().map(|gap| gap.kind.as_str()).collect();
        assert!(kinds.contains(GAP_OPEN_MONITOR));
        assert!(kinds.contains(GAP_PENDING_DEFERRED));
        assert_eq!(j.closure_proof.monitor_open_count, 1);
        // A DUE deferred returns to the frontier → open_todo, not pending.
        let mut g2 = Goal::new("g2", "o", "/tmp");
        g2.add(Todo::deferred("D1", "later", Duration::ZERO));
        let j2 = terminal_judgement(&g2);
        assert!(j2
            .gaps
            .iter()
            .any(|gap| gap.kind == GAP_OPEN_TODO && gap.todo_id.as_deref() == Some("D1")));
        assert!(!j2.gaps.iter().any(|gap| gap.kind == GAP_PENDING_DEFERRED));
    }

    #[test]
    fn succession_gap_blocks_terminal_with_gap_detail() {
        let mut g = Goal::new("g", "o", "/tmp");
        let mut todo = Todo::advancement("T1", "work");
        todo.complete(false, vec![]); // silent completion
        g.add(todo);
        assert!(!g.is_terminal());
        let j = terminal_judgement(&g);
        assert!(!j.terminal);
        assert_eq!(j.gaps.len(), 1);
        assert_eq!(j.gaps[0].kind, GAP_SUCCESSION);
        assert_eq!(j.gaps[0].todo_id.as_deref(), Some("T1"));
        assert_eq!(j.closure_proof.successor_gap_count, 1);
    }

    #[test]
    fn judgement_matches_is_terminal_on_mixed_states() {
        let mut g = Goal::new("g", "o", "/tmp").with_acceptance(vec![("A1", "x")]);
        g.add(Todo::advancement("T1", "a"));
        assert_eq!(terminal_judgement(&g).terminal, g.is_terminal());
        g.todo_mut("T1").unwrap().complete(true, vec![]);
        assert_eq!(terminal_judgement(&g).terminal, g.is_terminal());
        g.satisfy_gap("A1");
        assert_eq!(terminal_judgement(&g).terminal, g.is_terminal());
        assert!(g.is_terminal());
        // Superseded todos must not block terminal closure either.
        let mut g2 = Goal::new("g2", "o", "/tmp");
        g2.add(Todo::advancement("T1", "obsolete"));
        g2.supersede("T1");
        assert!(g2.is_terminal());
        assert!(terminal_judgement(&g2).terminal);
        assert!(!g2.todos.iter().any(|t| t.class == TaskClass::Monitor));
    }
}
