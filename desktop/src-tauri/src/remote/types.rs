//! Desktop start input and adapters around shared controlled-endpoint status.
#[cfg(test)]
pub use future_remote_server::types::DeviceControl;
#[cfg(test)]
pub use future_remote_server::types::RemoteFailureReason;
pub use future_remote_server::types::{RemotePhase, RemoteStatus};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStartInput {}
