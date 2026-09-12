use serde::{Deserialize, Serialize};
/// Command sent by the client via NATS (camelCase JSON, only the fields the bridge needs).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct IncomingCmd {
    pub(crate) chunked_read: bool,
    pub(crate) reply_id: String,
    pub(crate) replay_until_idx: Option<i64>,
    pub(crate) bridge_instance_id: String,
    pub(crate) id: String,
    #[serde(rename = "type")]
    pub(crate) cmd_type: String,
    pub(crate) session_id: String,
    pub(crate) message: String,
    // approval_decision
    pub(crate) entry_id: String,
    pub(crate) mode: String,
    // get_events_since (P1c backfill)
    pub(crate) run_id: String,
    pub(crate) prompt_id: String,
    pub(crate) since_idx: i64,
    // get_messages pagination (NATS payload-limit guard)
    pub(crate) offset: i64,
    pub(crate) limit: i64,
    // get_session_entries backward cursor (mobile lazy history)
    pub(crate) before: Option<i64>,
    // set_model / set_thinking_level
    pub(crate) model_id: String,
    pub(crate) provider_id: String,
    pub(crate) level: String,
    // set_approval_tier
    pub(crate) tier: String,
    // set_session_name
    pub(crate) name: String,
    pub(crate) transfer_name: String,
    // delete_session / set_session_pinned (thread-scoped, see ThreadRecord)
    pub(crate) thread_id: String,
    pub(crate) pinned: bool,
    // prompt creation mode / existing workspace selection
    pub(crate) workspace_id: String,
    // file transfer control + prompt attachment references
    pub(crate) mime_type: String,
    pub(crate) kind: String,
    pub(crate) original_size: u64,
    pub(crate) transfer_size: u64,
    pub(crate) transfer_id: String,
    pub(crate) file_path: String,
    pub(crate) attachments: Vec<UploadReference>,
    // signed application-level pairing handshake
    pub(crate) protocol_version: u32,
    pub(crate) pair_id: String,
    pub(crate) device_id: String,
    pub(crate) client_public_key: String,
    pub(crate) client_nonce: String,
    pub(crate) desktop_nonce: String,
    pub(crate) expected_desktop_id: String,
    pub(crate) expected_desktop_public_key: String,
    pub(crate) client_signature: String,
}

impl Default for IncomingCmd {
    fn default() -> Self {
        Self {
            chunked_read: false,
            reply_id: String::new(),
            replay_until_idx: None,
            bridge_instance_id: String::new(),
            id: String::new(),
            cmd_type: String::new(),
            session_id: String::new(),
            message: String::new(),
            entry_id: String::new(),
            mode: String::new(),
            run_id: String::new(),
            prompt_id: String::new(),
            since_idx: -1,
            offset: 0,
            limit: 0,
            before: None,
            model_id: String::new(),
            provider_id: String::new(),
            level: String::new(),
            tier: String::new(),
            name: String::new(),
            transfer_name: String::new(),
            thread_id: String::new(),
            pinned: false,
            workspace_id: String::new(),
            mime_type: String::new(),
            kind: String::new(),
            original_size: 0,
            transfer_size: 0,
            transfer_id: String::new(),
            file_path: String::new(),
            attachments: Vec::new(),
            protocol_version: 0,
            pair_id: String::new(),
            device_id: String::new(),
            client_public_key: String::new(),
            client_nonce: String::new(),
            desktop_nonce: String::new(),
            expected_desktop_id: String::new(),
            expected_desktop_public_key: String::new(),
            client_signature: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingCreds {
    #[serde(default)]
    pub handshake_version: u32,
    pub pair_id: String,
    pub desktop_id: String,
    pub nkey_seed: String,
    pub user_jwt: String,
    pub nats_url: String,
    pub nats_ws_url: String,
    pub jwt_expires_at: i64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadReference {
    pub upload_id: String,
}
