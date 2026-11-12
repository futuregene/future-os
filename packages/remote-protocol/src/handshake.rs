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

/// Capabilities this endpoint advertises, in wire order.
///
/// One list, one place. The secure handshake (`secure_finish`) seals it into the
/// confirmation, and the legacy `pair_handshake_confirm` answers with it too.
/// Two literals used to do that job, and the secure one — the only path a
/// current client takes — had fallen three flags behind: `segmented_content_v1`,
/// `reasoning_fetch_v1` and `terminal_v1` were advertised by the legacy reply
/// and by nothing a paired desktop actually reads. That is why a remote
/// terminal never appeared in the window, on any host build.
///
/// The `*_v1` flags are not a compatibility version: each names one negotiated
/// increment, and a client enables an increment only after finding its flag
/// here (see `remote-client`'s `DECLARED_FEATURES`). Adding one is additive for
/// every client that does not look for it.
///
/// `terminal` is the one flag a *process* decides rather than a build: only a
/// host that installed the terminal registry advertises it, because a panel
/// offered by a machine that cannot spawn a shell is a button whose every press
/// fails.
pub fn advertised_features(terminal: bool) -> Vec<&'static str> {
    let mut features = vec![
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
        // Shelve/restore a conversation or a workspace from another machine. The
        // catalogue has reported `archived` for both since before this flag; the
        // flag is what says the *commands* that change it are answered.
        "archive_v1",
        "desktop_settings_v1",
        "skill_management_v1",
        "compaction_v1",
        "fusion_v1",
        "fusion_settings_v1",
        "provider_management_v1",
        "fork_v1",
        "tasks_v1",
        "task_deps_v1",
        // A chunk's bytes may travel outside the JSON reply, behind a
        // length-prefixed header, instead of base64 inside it. Advertised rather
        // than merely honoured because the client gates its declaration on this
        // list: the two ends have to agree before either changes its encoding.
        crate::READ_BINARY_FEATURE,
        // Conversation content split into addressable segments, so a reader can
        // fetch one window of a long entry instead of the whole of it.
        crate::SEGMENTED_CONTENT_FEATURE,
        // A reasoning row's body on demand. The catalogue says a row exists; this
        // flag is what says the text behind it can be fetched.
        crate::REASONING_FETCH_FEATURE,
    ];
    if terminal {
        features.push(crate::TERMINAL_FEATURE);
    }
    features
}

/// The authenticated body sealed under the existing `handshake-confirm` context.
///
/// `features` is the caller's list, not this crate's: which capabilities a
/// process actually serves is a property of that process (the terminal being
/// the one that varies by entry point), so the endpoint that owns the host
/// services names them and this function only seals the envelope. Catalog
/// construction belongs to the owning endpoint for the same reason.
pub fn confirmation(
    pair_id: &str,
    bridge_instance_id: &str,
    features: &[&str],
    presence: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "confirmed": true, "pairId": pair_id, "bridgeInstanceId": bridge_instance_id,
        "features": features, "presence": presence,
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
    fn the_advertised_list_is_one_list_and_terminal_is_the_only_conditional_flag() {
        // Regression: two literals meant the secure handshake — the only path a
        // current client takes — advertised neither `segmented_content_v1`,
        // `reasoning_fetch_v1` nor `terminal_v1`, while the legacy reply had all
        // three. A remote terminal therefore never appeared on any host.
        let without = advertised_features(false);
        let with = advertised_features(true);
        assert!(!without.contains(&crate::TERMINAL_FEATURE));
        assert!(with.contains(&crate::TERMINAL_FEATURE));
        assert_eq!(with.len(), without.len() + 1);
        assert_eq!(&with[..without.len()], &without[..]);

        // The two flags the secure path used to miss are unconditional.
        for flag in [
            crate::SEGMENTED_CONTENT_FEATURE,
            crate::REASONING_FETCH_FEATURE,
        ] {
            assert!(without.contains(&flag), "{flag} must always be advertised");
        }

        let mut seen = std::collections::HashSet::new();
        for feature in &with {
            assert!(seen.insert(*feature), "{feature} advertised twice");
        }
    }
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
