//! Run lifecycle subdomain (G-5, minimal set) — run history, compaction
//! (archive, never delete), index dedup/rebuild, context retention, and the
//! stale-latest-run projection, mirroring LoopX `control_plane/runtime/`
//! (run_history / run_compaction / run_index_* / run_context_retention /
//! stale_latest_run). The remaining runtime subdomains (trajectory_hygiene,
//! run_ingest_health, run_artifacts, …) are deferred per the refactor plan.
//!
//! These operate on the run-history FILE projection under
//! `<runtime_root>/goals/<goal_id>/runs/`; the authoritative spend ledger
//! (`runs.jsonl` in the goal dir) is NEVER touched by compaction/retention.

pub mod run_compaction;
pub mod run_context_retention;
pub mod run_history;
pub mod run_index;
pub mod stale_latest_run;

use std::path::PathBuf;

/// `<runtime_root>/goals/<goal_id>/runs` — the LoopX run-history dir.
pub fn runs_dir(runtime_root: &str, goal_id: &str) -> PathBuf {
    PathBuf::from(runtime_root)
        .join("goals")
        .join(goal_id)
        .join("runs")
}

/// The append-only run index.
pub fn index_path(runtime_root: &str, goal_id: &str) -> PathBuf {
    runs_dir(runtime_root, goal_id).join("index.jsonl")
}
