//! Desktop start input and adapters around shared controlled-endpoint status.
pub use future_remote_server::types::DeviceControl;
#[cfg(any(test, feature = "test-support"))]
pub use future_remote_server::types::RemoteFailureReason;
pub use future_remote_server::types::{ClientIdentity, RemotePhase, RemoteStatus};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStartInput {}
