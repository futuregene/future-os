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

pub const HANDSHAKE_PROTOCOL_VERSION: u32 = 1;

/// Inputs bound into the pairing handshake transcript. Every field is an
/// `&str`, so a positional call could silently transpose two of them and sign
/// a different transcript — named fields make that a compile error instead.
pub struct HandshakeTranscript<'a> {
    pub pair_id: &'a str,
    pub desktop_id: &'a str,
    pub desktop_public_key: &'a str,
    pub bridge_instance_id: &'a str,
    pub device_id: &'a str,
    pub client_public_key: &'a str,
    pub client_nonce: &'a str,
    pub desktop_nonce: &'a str,
}

pub fn handshake_transcript(parts: &HandshakeTranscript<'_>) -> String {
    [
        "futureos-remote-handshake-v1",
        parts.pair_id,
        parts.desktop_id,
        parts.desktop_public_key,
        parts.bridge_instance_id,
        parts.device_id,
        parts.client_public_key,
        parts.client_nonce,
        parts.desktop_nonce,
    ]
    .join("\n")
}

#[cfg(test)]
mod legacy_tests {
    use super::*;
    #[test]
    fn handshake_transcript_binds_both_device_identities_and_nonces() {
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: "pair_1",
            desktop_id: "desktop_1",
            desktop_public_key: "UDESKTOP",
            bridge_instance_id: "bridge_1",
            device_id: "dev_1",
            client_public_key: "UCLIENT",
            client_nonce: "client_nonce",
            desktop_nonce: "desktop_nonce",
        });
        assert_eq!(
            transcript,
            "futureos-remote-handshake-v1\npair_1\ndesktop_1\nUDESKTOP\nbridge_1\ndev_1\nUCLIENT\nclient_nonce\ndesktop_nonce"
        );
        assert_ne!(
            transcript,
            handshake_transcript(&HandshakeTranscript {
                pair_id: "pair_1",
                desktop_id: "desktop_other",
                desktop_public_key: "UDESKTOP",
                bridge_instance_id: "bridge_1",
                device_id: "dev_1",
                client_public_key: "UCLIENT",
                client_nonce: "client_nonce",
                desktop_nonce: "desktop_nonce",
            })
        );
    }
    #[test]
    fn handshake_signature_rejects_tampered_transcript() {
        let desktop = nkeys::KeyPair::new_user();
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: "pair_1",
            desktop_id: "desktop_1",
            desktop_public_key: &desktop.public_key(),
            bridge_instance_id: "bridge_1",
            device_id: "dev_1",
            client_public_key: "UCLIENT",
            client_nonce: "client_nonce",
            desktop_nonce: "desktop_nonce",
        });
        let signature = desktop.sign(transcript.as_bytes()).unwrap();
        let verifier = nkeys::KeyPair::from_public_key(&desktop.public_key()).unwrap();
        assert!(verifier.verify(transcript.as_bytes(), &signature).is_ok());
        assert!(verifier
            .verify(format!("{transcript}_tampered").as_bytes(), &signature)
            .is_err());
    }
}
