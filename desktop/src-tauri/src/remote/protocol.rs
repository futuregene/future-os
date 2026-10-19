use serde::{Deserialize, Serialize};

pub(crate) use future_remote_protocol::{IncomingCmd, UploadReference};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingCreds {
    #[serde(default)]
    pub handshake_version: u32,
    #[serde(default)]
    pub secure: Option<super::secure::PairingIdentity>,
    pub pair_id: String,
    pub desktop_id: String,
    pub nkey_seed: String,
    pub user_jwt: String,
    pub nats_url: String,
    pub nats_ws_url: String,
    pub jwt_expires_at: i64,
    /// The self-hosted relay that mints this pairing's credentials.
    ///
    /// `None` means the official platform, which is what every existing
    /// credential says by omission. It is per pairing on purpose: one machine
    /// can hold an official device and a self-hosted one at the same time, and
    /// each keeps asking the relay that issued it.
    #[serde(default)]
    pub control_url: Option<String>,
}
