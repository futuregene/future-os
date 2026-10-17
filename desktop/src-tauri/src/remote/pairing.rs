//! Pairing service facade; the protocol core owns no filesystem/account/API access.
pub(crate) use super::protocol::PairingCreds;
#[cfg(test)]
pub(crate) use crate::remote_host::pairing::INJECT_SAVE_FAILURE;
use std::time::{SystemTime, UNIX_EPOCH};
pub fn load_creds() -> Option<PairingCreds> {
    super::host().load()
}
pub fn save_creds(c: &PairingCreds) -> Result<(), crate::AppError> {
    super::host().save(c)
}
pub fn clear_creds() -> Result<(), crate::AppError> {
    super::host().clear()
}
pub async fn create_pairing() -> Result<super::services::Invitation, crate::AppError> {
    super::host().invite().await
}
pub async fn refresh_bridge_jwt(c: PairingCreds) -> Result<PairingCreds, crate::AppError> {
    super::host().refresh(c).await
}
pub(crate) fn queue_revoke(c: &PairingCreds) -> Result<(), crate::AppError> {
    super::host().queue_revoke(c)
}
pub(crate) async fn retry_pending_revokes() -> Result<(), crate::AppError> {
    super::host().retry_revokes().await
}

pub fn public_key(creds: &PairingCreds) -> Result<String, crate::AppError> {
    nkeys::KeyPair::from_seed(&creds.nkey_seed)
        .map(|key_pair| key_pair.public_key())
        .map_err(|error| crate::AppError::Message(format!("read desktop NKey: {error}")))
}
pub fn is_invalid_or_revoked_error(error: &crate::AppError) -> bool {
    matches!(
        error,
        crate::AppError::Remote {
            code: Some(code),
            ..
        } if code == "invalid_remote_credential"
    )
}
pub fn is_account_authorization_error(error: &crate::AppError) -> bool {
    matches!(
        error,
        crate::AppError::Remote {
            status: 401 | 403,
            ..
        }
    ) && !is_invalid_or_revoked_error(error)
}

pub fn error_code(error: &crate::AppError) -> Option<&'static str> {
    match error {
        crate::AppError::RemoteTransport(_) => Some("network"),
        crate::AppError::RemoteAuthorization(_) => Some("service_authorization"),
        // No stored account credential: the platform issues pairing codes, so
        // the same "sign in again" answer as a rejected key — not the generic
        // device fault an uncategorized error would be shown as.
        crate::AppError::NotSignedIn => Some("account_authorization"),
        error if is_account_authorization_error(error) => Some("account_authorization"),
        crate::AppError::Remote { code, .. } => match code.as_deref() {
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
pub(crate) const REFRESH_LEAD: std::time::Duration = std::time::Duration::from_secs(240);

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
