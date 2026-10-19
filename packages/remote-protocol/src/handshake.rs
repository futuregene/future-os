//! Existing Remote v2 handshake wire envelope. Unknown fields remain additive.
use serde::Deserialize;

pub const REFUSED: &str = "remote_secure_channel_invalid";

#[derive(Deserialize)]
pub struct HandshakeRequest {
    #[serde(rename = "type")]
    pub kind: String,
    pub message: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub pairing: bool,
}

/// Capabilities advertised by the existing v2 controlled endpoint, in wire order.
pub const SERVER_FEATURES: &[&str] = &[
    "e2ee_v2",
    "file_transfer_v1",
    "file_download_v2",
    "approval_tier_v1",
    "auto_approval_v1",
    "continue_run_v1",
    "prompt_receipt_v1",
    "session_files_v1",
    "skills_v1",
    "selective_events_v1",
    "workspace_pinning_v1",
    "workspace_create_v1",
    "desktop_settings_v1",
    "skill_management_v1",
    "compaction_v1",
    "provider_management_v1",
    "fork_v1",
    "tasks_v1",
    "task_deps_v1",
];

/// The authenticated body sealed under the existing `handshake-confirm` context.
/// Catalog construction belongs to the owning endpoint and its host services.
pub fn confirmation(
    pair_id: &str,
    bridge_instance_id: &str,
    presence: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "confirmed": true, "pairId": pair_id, "bridgeInstanceId": bridge_instance_id,
        "features": SERVER_FEATURES, "presence": presence,
    })
}
