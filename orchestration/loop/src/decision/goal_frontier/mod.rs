//! goal_frontier subdomain (G13) — the reference
//! `control_plane/goals/goal_frontier/` package (__init__.py 1960 lines +
//! outcome_continuity / replan_rules / semantic_history / terminal),
//! natively (core subset).
//!
//! Four deepening layers over the existing frontier projection:
//!   - [`outcome_continuity`]: outcome-streak segmentation — consecutive
//!     surface-only turns form one segment; a material turn or a
//!     frontier-changing event between runs resets the segment;
//!   - [`replan_rules`]: the structured replan trigger rule table
//!     (disposition → replan decision + obligation), an ordered policy
//!     table with a default rule set and a `ReplanRuleSetUpdated` event;
//!   - [`semantic_history`]: the goal-level bounded semantic event history
//!     (recent N=50 semantic summaries folded from the event ledger),
//!     consumable by the decision-context assembler;
//!   - [`terminal`]: terminal judgement — closure validation that
//!     enumerates acceptance-gap semantics and every remaining blocker as
//!     explicit gap entries, aligned with the existing
//!     `TerminalClosureProof`.
//!
//! [`frontier_show`] composes the CLI surface (`future loop frontier show`)
//! from all four layers plus the existing frontier projection.

use serde::Serialize;

use crate::contract::FrontierProjection;
use crate::state::Goal;

pub mod outcome_continuity;
pub mod replan_rules;
pub mod semantic_history;
pub mod terminal;

pub use outcome_continuity::{outcome_segments, OutcomeSegment, OUTCOME_SEGMENT_SCHEMA_VERSION};
pub use replan_rules::{
    active_rule_set, facts_for_goal, select_replan_rule, ReplanFacts, ReplanRuleDecision,
    ReplanRuleSet, DEFAULT_REPLAN_RULE_SET_VERSION, REPLAN_RULE_DECISION_SCHEMA_VERSION,
};
pub use semantic_history::{SemanticEvent, SEMANTIC_HISTORY_CAP, SEMANTIC_HISTORY_SCHEMA_VERSION};
pub use terminal::{
    terminal_judgement, TerminalGap, TerminalJudgement, TERMINAL_JUDGEMENT_SCHEMA_VERSION,
};

/// The `frontier show` read model: frontier projection + the four G13
/// deepening layers.
pub const FRONTIER_SHOW_SCHEMA_VERSION: &str = "goal_frontier_show_v0";

#[derive(Debug, Clone, Serialize)]
pub struct FrontierShow {
    pub schema_version: String,
    pub goal_id: String,
    /// The work lane (monitor vs advancement-task), same derivation as the
    /// should-run packet's `work_lane_contract.lane`.
    pub lane: String,
    pub frontier_projection: FrontierProjection,
    pub outcome_segments: Vec<OutcomeSegment>,
    pub replan_rule: ReplanRuleDecision,
    pub terminal_judgement: TerminalJudgement,
    pub semantic_history: Vec<SemanticEvent>,
}

/// The replan-pressure flag the decision kernel uses (same formula as the
/// should-run packet: open gates, unclosed completions, stalled monitors,
/// unsatisfied acceptance gaps).
fn replan_required(goal: &Goal) -> bool {
    goal.open_gates().next().is_some()
        || !goal.completed_without_closure_intent().is_empty()
        || goal.open_monitors().any(super::stall::is_monitor_stalled)
        || !goal.unsatisfied_gaps().is_empty()
}

/// Compose the `frontier show` projection for a goal.
pub fn frontier_show(goal: &Goal) -> FrontierShow {
    FrontierShow {
        schema_version: FRONTIER_SHOW_SCHEMA_VERSION.to_string(),
        goal_id: goal.goal_id.clone(),
        lane: super::frontier::lane(goal).to_string(),
        frontier_projection: super::frontier::frontier_projection(goal, replan_required(goal)),
        outcome_segments: outcome_segments(goal),
        replan_rule: select_replan_rule(goal),
        terminal_judgement: terminal_judgement(goal),
        semantic_history: goal.semantic_history.clone(),
    }
}
