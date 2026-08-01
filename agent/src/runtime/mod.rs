//! Per-session execution ownership.
//!
//! `SessionRuntime` owns the lifecycle state, cancellation acknowledgement,
//! and single background task slot. Long-running model, tool, persistence, and
//! channel work never runs while its short control locks are held.

mod run_request;
mod run_state;
mod scheduler_queue;
mod session_runtime;

pub use run_request::{BusyPolicy, RunAcceptedState, RunAck};
pub(crate) use run_state::RunControl;
pub use run_state::{RunLease, RunPhase, RunSnapshot};
pub use scheduler_queue::{
    GlobalQueueBudget, InMemoryRunQueue, QueuedCancellationReason, RunQueueError,
    ScheduledRunRequest, DEFAULT_GLOBAL_QUEUE_BYTES, DEFAULT_GLOBAL_QUEUE_CAPACITY,
    DEFAULT_REQUEST_BYTES, DEFAULT_SESSION_QUEUE_BYTES, DEFAULT_SESSION_QUEUE_CAPACITY,
};
pub use session_runtime::SessionRuntime;
