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
pub fn error_code(error: &crate::AppError) -> Option<&'static str> {
    match error {
        crate::AppError::RemoteTransport(_) => Some("network"),
        crate::AppError::RemoteAuthorization(_) => Some("service_authorization"),
        crate::AppError::Remote { code, .. } => match code.as_deref() {
            Some("invalid_remote_credential") => Some("revoked"),
            _ => Some("server"),
        },
        _ => None,
    }
}
pub fn refresh_delay(creds: &PairingCreds) -> std::time::Duration {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default();
    std::time::Duration::from_secs(
        creds
            .jwt_expires_at
            .saturating_sub(now)
            .saturating_sub(60)
            .max(5) as u64,
    )
}
