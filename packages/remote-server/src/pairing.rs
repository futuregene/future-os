//! Credential scheduling and platform error policy; persistence stays in the host.
use crate::PairingCreds;
use std::time::{SystemTime, UNIX_EPOCH};
pub fn public_key(creds: &PairingCreds) -> Result<String, crate::Error> {
    nkeys::KeyPair::from_seed(&creds.nkey_seed)
        .map(|key_pair| key_pair.public_key())
        .map_err(|error| crate::Error::Message(format!("read desktop NKey: {error}")))
}
pub fn is_invalid_or_revoked_error(error: &crate::Error) -> bool {
    matches!(
        error,
        crate::Error::Remote {
            code: Some(code),
            ..
        } if code == "invalid_remote_credential"
    )
}
pub fn is_account_authorization_error(error: &crate::Error) -> bool {
    matches!(
        error,
        crate::Error::Remote {
            status: 401 | 403,
            ..
        }
    ) && !is_invalid_or_revoked_error(error)
}

pub fn error_code(error: &crate::Error) -> Option<&'static str> {
    match error {
        crate::Error::RemoteTransport(_) => Some("network"),
        crate::Error::RemoteAuthorization(_) => Some("service_authorization"),
        // No stored account credential: the platform issues pairing codes, so
        // the same "sign in again" answer as a rejected key — not the generic
        // device fault an uncategorized error would be shown as.
        crate::Error::AccountAuthorization(_) => Some("account_authorization"),
        error if is_account_authorization_error(error) => Some("account_authorization"),
        crate::Error::Remote { code, .. } => match code.as_deref() {
            Some("invalid_remote_credential") => Some("revoked"),
            _ => Some("server"),
        },
        _ => None,
    }
}
/// How long before expiry the bridge credential is renewed.
///
/// The relay issues a ~15 minute bridge JWT. Renewing inside its last minute —
/// the previous 60-second margin — left no room for what a renewal actually
/// does: an HTTPS round trip to the platform, a fresh TLS connection, and a
/// transport rebuild with its own readiness deadline. A renewal that lost that
/// race let the old credential expire in flight, and an expired credential is
/// rejected outright (`User Authentication Expired`, then `-ERR 'Authorization
/// Violation'` on every retry): the bridge cannot publish and drops events
/// until the next rotation lands, which is what the phone sees as the desktop
/// going offline. Four minutes gives the work room to fail once and still
/// finish inside the credential's lifetime.
pub const REFRESH_LEAD: std::time::Duration = std::time::Duration::from_secs(240);

/// Time until the credential should be renewed; zero once it is due.
pub fn refresh_delay(creds: &PairingCreds) -> std::time::Duration {
    std::time::Duration::from_secs(seconds_until_renewal(creds).max(0) as u64)
}

/// Signed seconds until renewal is due; negative once it is overdue.
pub fn seconds_until_renewal(creds: &PairingCreds) -> i64 {
    seconds_until_expiry(creds) - REFRESH_LEAD.as_secs() as i64
}

/// Signed seconds until the credential expires; negative once it has.
pub fn seconds_until_expiry(creds: &PairingCreds) -> i64 {
    creds.jwt_expires_at.saturating_sub(unix_seconds())
}

fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn now_secs() -> i64 {
        unix_seconds()
    }
    fn fixture_creds() -> PairingCreds {
        PairingCreds {
            handshake_version: 2,
            secure: None,
            pair_id: "pair_policy".into(),
            desktop_id: "host_policy".into(),
            nkey_seed: "fixture".into(),
            user_jwt: "fixture".into(),
            nats_url: "tls://relay.invalid:4222".into(),
            nats_ws_url: "wss://relay.invalid".into(),
            jwt_expires_at: 0,
            control_url: None,
        }
    }
    #[test]
    fn identifies_revoked_credential_error() {
        assert!(is_invalid_or_revoked_error(&crate::Error::Remote {
            status: 401,
            code: Some("invalid_remote_credential".to_string()),
            message: "Remote credential is invalid or revoked.".to_string(),
        }));
        // A human message that merely reads like a revocation must NOT match —
        // only the machine code is authoritative.
        assert!(!is_invalid_or_revoked_error(&crate::Error::Remote {
            status: 401,
            code: Some("unauthorized".to_string()),
            message: "Remote credential is invalid or revoked.".to_string(),
        }));
        assert!(!is_invalid_or_revoked_error(&crate::Error::Message(
            "Failed to refresh remote credential: timeout".to_string(),
        )));
    }
    #[test]
    fn platform_account_rejection_is_not_a_retryable_network_error() {
        for status in [401, 403] {
            let error = crate::Error::Remote {
                status,
                code: None,
                message: "rejected".into(),
            };
            assert_eq!(error_code(&error), Some("account_authorization"));
            assert!(!is_invalid_or_revoked_error(&error));
        }
    }
    #[test]
    fn classifies_error_codes_without_sniffing_messages() {
        // Offline / unreachable → network, regardless of the human message.
        assert_eq!(
            error_code(&crate::Error::RemoteTransport(
                "Failed to create pairing code: error sending request".to_string(),
            )),
            Some("network"),
        );
        // Server revocation is read from the machine code, not the message.
        assert_eq!(
            error_code(&crate::Error::Remote {
                status: 401,
                code: Some("invalid_remote_credential".to_string()),
                message: "gone".to_string(),
            }),
            Some("revoked"),
        );
        // Any other server status → generic server category.
        assert_eq!(
            error_code(&crate::Error::Remote {
                status: 500,
                code: None,
                message: "boom".to_string(),
            }),
            Some("server"),
        );
        // A missing account credential is the account's problem, and the type
        // says so — no message is consulted.
        assert_eq!(
            error_code(&crate::Error::AccountAuthorization(Box::new(
                std::io::Error::other("not_signed_in")
            ))),
            Some("account_authorization"),
        );
        // Local failures carry no remote category.
        assert_eq!(
            error_code(&crate::Error::Message("disk full".to_string())),
            None,
        );
    }
    #[test]
    fn refresh_delay_is_the_time_until_the_lead_before_expiry() {
        let mut creds = fixture_creds();
        // Renewal starts REFRESH_LEAD before expiry (minutes, not the previous
        // 60 seconds): the HTTPS round trip, TLS connect and transport rebuild
        // have to fit inside the old credential's lifetime.
        creds.jwt_expires_at = now_secs() + 3600;
        let delay = refresh_delay(&creds).as_secs();
        let expected = 3600 - REFRESH_LEAD.as_secs();
        assert!(
            delay <= expected && delay + 5 >= expected,
            "delay was {delay}"
        );
        // Already expired → due now, so the renewal loop retries at its fast
        // cadence instead of idling out a tick with the bridge unable to publish.
        creds.jwt_expires_at = now_secs() - 10;
        assert_eq!(refresh_delay(&creds), std::time::Duration::ZERO);
    }
    #[test]
    fn public_key_rejects_a_bad_seed() {
        let mut creds = fixture_creds();
        creds.nkey_seed = "not-a-seed".to_string();
        assert!(public_key(&creds).is_err());
    }
}
