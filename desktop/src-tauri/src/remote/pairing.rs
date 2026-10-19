//! Pairing service facade; the protocol core owns no filesystem/account/API access.
#[cfg(test)]
pub(crate) use crate::remote_host::pairing::INJECT_SAVE_FAILURE;
pub(crate) use future_remote_server::PairingCreds;
pub fn load_creds() -> Option<PairingCreds> {
    super::host().load()
}
/// Every paired device, in pairing order.
#[cfg(test)]
pub fn load_slots() -> Vec<PairingCreds> {
    super::host().load_slots()
}
/// Which device drives this machine, as persisted.
#[cfg(test)]
pub fn active_slot() -> Option<String> {
    super::host().active_slot()
}
#[cfg(test)]
pub fn set_active(pair_id: &str) -> Result<(), crate::AppError> {
    super::host().set_active(pair_id).map_err(Into::into)
}
#[cfg(test)]
pub fn remove_slot(pair_id: &str) -> Result<(), crate::AppError> {
    super::host().remove_slot(pair_id).map_err(Into::into)
}
#[cfg(test)]
pub fn save_creds(c: &PairingCreds) -> Result<(), crate::AppError> {
    super::host().save(c).map_err(Into::into)
}
#[cfg(test)]
pub fn clear_creds() -> Result<(), crate::AppError> {
    super::host().clear().map_err(Into::into)
}
/// Queue a revoke through the actual application pairing store.
#[cfg(test)]
pub(crate) fn queue_revoke(c: &PairingCreds) -> Result<(), crate::AppError> {
    super::host().queue_revoke(c).map_err(Into::into)
}
#[cfg(test)]
pub(crate) async fn retry_pending_revokes() -> Result<(), crate::AppError> {
    super::host().retry_revokes().await.map_err(Into::into)
}

#[cfg(test)]
pub fn public_key(creds: &PairingCreds) -> Result<String, crate::AppError> {
    future_remote_server::pairing::public_key(creds).map_err(Into::into)
}
#[cfg(test)]
fn classified_error(error: &crate::AppError) -> future_remote_server::Error {
    use future_remote_server::Error;
    match error {
        crate::AppError::RemoteTransport(_) => Error::RemoteTransport(String::new()),
        crate::AppError::RemoteAuthorization(_) => Error::RemoteAuthorization(String::new()),
        crate::AppError::NotSignedIn => {
            Error::AccountAuthorization(Box::new(std::io::Error::other("not_signed_in")))
        }
        crate::AppError::Remote { status, code, .. } => Error::Remote {
            status: *status,
            code: code.clone(),
            message: String::new(),
        },
        _ => Error::Message(String::new()),
    }
}
#[cfg(test)]
pub fn is_invalid_or_revoked_error(error: &crate::AppError) -> bool {
    future_remote_server::pairing::is_invalid_or_revoked_error(&classified_error(error))
}
#[cfg(test)]
pub fn error_code(error: &crate::AppError) -> Option<&'static str> {
    future_remote_server::pairing::error_code(&classified_error(error))
}
