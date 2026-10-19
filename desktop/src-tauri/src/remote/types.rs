//! Desktop start input and adapters around shared controlled-endpoint status.
#[cfg(test)]
pub use future_remote_server::types::DeviceControl;
pub(super) use future_remote_server::types::{retryable_start_status, runtime_active};
pub use future_remote_server::types::{
    ClientIdentity, RecoveryProgress, RemoteFailureReason, RemotePhase, RemoteStatus,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStartInput {}

pub(super) fn empty() -> RemoteStatus {
    future_remote_server::types::empty(super::host().agent_available())
}
