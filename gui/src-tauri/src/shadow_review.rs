//! Shadow review subsystem (SHADOW_REVIEW_DESIGN.md).
//!
//! Produces the "上一轮变更" (last-run delta) for a Workspace by snapshotting
//! the work tree before/after each Run into an isolated bare git repository,
//! never touching the user's real `.git`.

mod diff;
mod maintenance;
mod policy;
mod repository;
mod snapshot;

pub use diff::{materialize, MaterializedDiff};
pub use maintenance::{enforce_retention, run_startup_maintenance};
pub use policy::{evaluate_volume, Limits, VolumeRedline, VolumeVerdict};
pub use repository::{with_workspace_lock, ShadowRepo};
pub use snapshot::{capture, record_failure};
