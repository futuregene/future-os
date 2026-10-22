//! Desktop start input and adapters around shared controlled-endpoint status.
pub use future_remote_server::types::DeviceControl;
pub use future_remote_server::types::{
    ClientIdentity, RemoteFailureReason, RemotePhase, RemoteStatus,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStartInput {}
