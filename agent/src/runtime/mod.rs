//! Per-session execution ownership.
//!
//! `SessionRuntime` owns the lifecycle state, cancellation acknowledgement,
//! and single background task slot. Long-running model, tool, persistence, and
//! channel work never runs while its short control locks are held.

mod run_queue;
mod run_request;
mod run_state;
mod session_runtime;

pub use run_queue::{
    DurableRunQueue, DurableRunRequest, QueueError, QueuedCancellationReason, RunTerminalState,
};
pub use run_request::{BusyPolicy, RunAcceptedState, RunAck};
pub(crate) use run_state::RunControl;
pub use run_state::{RunLease, RunPhase, RunSnapshot};
pub use session_runtime::SessionRuntime;
