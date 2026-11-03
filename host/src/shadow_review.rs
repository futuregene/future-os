//! Shadow review subsystem (see desktop/ER.md §4.10 / §6.8).
//!
//! Produces the "last-run changes" (last-run delta) for a Workspace by snapshotting
//! the work tree before/after each Run into an isolated bare git repository,
//! never touching the user's real `.git`.

mod diff;
mod last_run;
mod load;
mod maintenance;
mod policy;
mod process;
mod projection;
mod repository;
mod snapshot;

pub use diff::{materialize, MaterializedDiff};
pub use last_run::{build_last_run_review, LastRunReviewData};
pub use load::load_last_run_review;
pub(crate) use load::with_run_review_lock;
pub use maintenance::enforce_retention;
pub use policy::{evaluate_volume, is_sensitive, Limits, VolumeRedline, VolumeVerdict};
pub(crate) use projection::materialize_recorded;
pub use repository::{with_workspace_lock, ShadowRepo};
pub use snapshot::{capture, record_failure};
