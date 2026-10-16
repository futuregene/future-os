//! Multi-projection layer (G-4) — the same canonical goal state projected
//! through multiple privacy-graded lenses plus a status cache, mirroring
//! LoopX `state_projection` / `status_projection_cache` / `public_safety`:
//! the ledger is the single source of truth; every projection here is a
//! rebuildable read model. Unknown content grades `private_pointer`
//! (conservative — under-project rather than leak).

pub mod privacy;
pub mod status_cache;

use serde::Serialize;

use crate::state::{now_epoch, Goal};

pub const PROJECTION_SET_SCHEMA_VERSION: &str = "goal_projection_set_v0";

/// One privacy-graded projection of a goal.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectionSet {
    pub schema_version: String,
    pub goal_id: String,
    pub privacy: privacy::PrivacyLevel,
    /// Public-safe markdown: private surfaces redacted to
    /// `[redacted-private-state]`; unknown content kept behind pointers.
    pub public_markdown: String,
    /// Local-private markdown: the full ACTIVE_GOAL_STATE render.
    pub local_private_markdown: String,
    /// Count of private-pointer items (content not projected).
    pub private_pointer_count: usize,
    pub privacy_report: privacy::GoalPrivacyReport,
    /// The status-cache projection (freshly built, not yet persisted).
    pub status_cache: Option<status_cache::StatusCache>,
}

/// Build the full projection set for a goal. `goal_dir` provides the ledger
/// digest for the status cache. `privacy` selects the grading lens for the
/// public markdown (public_safe redacts; local_private/private_pointer pass
/// content through — the caller decides what to emit).
pub fn build_projections(
    goal: &Goal,
    privacy: privacy::PrivacyLevel,
    goal_dir: &std::path::Path,
) -> ProjectionSet {
    let full_md = crate::compat::render_active_state(goal);
    let report = privacy::grade_goal(goal);
    ProjectionSet {
        schema_version: PROJECTION_SET_SCHEMA_VERSION.to_string(),
        goal_id: goal.goal_id.clone(),
        privacy,
        public_markdown: privacy::redact(&full_md, privacy),
        local_private_markdown: full_md,
        private_pointer_count: report.private_pointer_count,
        privacy_report: report,
        status_cache: Some(status_cache::build_status_cache(
            goal,
            &status_cache::ledger_digest(goal_dir),
            now_epoch(),
        )),
    }
}
