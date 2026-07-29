//! Per-session execution ownership.
//!
//! `SessionRuntime` owns the lifecycle state, cancellation acknowledgement,
//! and single background task slot. Long-running model, tool, persistence, and
//! channel work never runs while its short control locks are held.

mod run_state;
mod session_runtime;

pub(crate) use run_state::RunControl;
pub use run_state::{RunLease, RunPhase, RunSnapshot};
pub use session_runtime::SessionRuntime;
