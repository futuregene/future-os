//! Pairing service facade; the protocol core owns no filesystem/account/API access.
#[cfg(test)]
pub(crate) use crate::remote_host::pairing::INJECT_SAVE_FAILURE;
pub use future_remote_server::PairingCreds;
pub fn load_creds() -> Option<PairingCreds> {
    super::host().load()
}
/// Every paired device, in pairing order.
#[cfg(any(test, feature = "test-support"))]
pub fn load_slots() -> Vec<PairingCreds> {
    super::host().load_slots()
}
/// Which device drives this machine, as persisted.
#[cfg(any(test, feature = "test-support"))]
pub fn active_slot() -> Option<String> {
    super::host().active_slot()
}
#[cfg(any(test, feature = "test-support"))]
pub fn set_active(pair_id: &str) -> Result<(), crate::Error> {
    super::host().set_active(pair_id).map_err(Into::into)
}
#[cfg(any(test, feature = "test-support"))]
pub fn remove_slot(pair_id: &str) -> Result<(), crate::Error> {
    super::host().remove_slot(pair_id).map_err(Into::into)
}
#[cfg(any(test, feature = "test-support"))]
pub fn save_creds(c: &PairingCreds) -> Result<(), crate::Error> {
    super::host().save(c).map_err(Into::into)
}
#[cfg(any(test, feature = "test-support"))]
pub fn clear_creds() -> Result<(), crate::Error> {
    super::host().clear().map_err(Into::into)
}
/// Queue a revoke through the actual application pairing store.
#[cfg(test)]
pub(crate) fn queue_revoke(c: &PairingCreds) -> Result<(), crate::Error> {
    super::host().queue_revoke(c).map_err(Into::into)
}
#[cfg(test)]
pub(crate) async fn retry_pending_revokes() -> Result<(), crate::Error> {
    super::host().retry_revokes().await.map_err(Into::into)
}

#[cfg(any(test, feature = "test-support"))]
pub fn public_key(creds: &PairingCreds) -> Result<String, crate::Error> {
    future_remote_server::pairing::public_key(creds).map_err(Into::into)
}
#[cfg(any(test, feature = "test-support"))]
fn classified_error(error: &crate::Error) -> future_remote_server::Error {
    use future_remote_server::Error;
    match error {
        crate::Error::RemoteTransport(_) => Error::RemoteTransport(String::new()),
        crate::Error::RemoteAuthorization(_) => Error::RemoteAuthorization(String::new()),
        crate::Error::NotSignedIn => {
            Error::AccountAuthorization(Box::new(std::io::Error::other("not_signed_in")))
        }
        crate::Error::Remote { status, code, .. } => Error::Remote {
            status: *status,
            code: code.clone(),
            message: String::new(),
        },
        _ => Error::Message(String::new()),
    }
}
#[cfg(any(test, feature = "test-support"))]
pub fn is_invalid_or_revoked_error(error: &crate::Error) -> bool {
    future_remote_server::pairing::is_invalid_or_revoked_error(&classified_error(error))
}
#[cfg(any(test, feature = "test-support"))]
pub fn error_code(error: &crate::Error) -> Option<&'static str> {
    future_remote_server::pairing::error_code(&classified_error(error))
}

/// Read the existing pairing book, preserving errors instead of assuming an empty machine.
pub fn load_creds_checked() -> Result<Option<PairingCreds>, crate::Error> {
    crate::remote_host::pairing::load_creds_checked()
}
