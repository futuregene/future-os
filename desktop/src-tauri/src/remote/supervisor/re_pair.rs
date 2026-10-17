//! `--re-pair`: replace this machine's pairing from the command line.
//!
//! The two steps are the ones the Remote page's buttons perform, in the only
//! order that works: the old binding is dropped first, because a fresh invitation
//! cannot be issued while this machine still holds the previous one.
//!
//! What the operator is choosing is worth stating plainly, since it is the whole
//! reason the flag exists: pairing replaces. The platform revokes the previous
//! pairing as soon as the new code is created, so the device that was connected
//! is cut off when this runs — whether or not the new code is ever scanned.

use super::*;

/// Revoke the saved pairing, then start a fresh one.
pub async fn re_pair() -> Result<RemoteStatus, crate::AppError> {
    unpair().await?;
    // `unpair` only queues the revoke: it is also the Remote page's button and
    // must not wait on HTTP. Here the queue cannot be left for the compensation
    // loop, because the platform gives a desktop it already knows the *same*
    // `pair_id` for the replacement invitation — a revoke delivered after it
    // would cancel the very code this command is about to print. Deliver it now.
    // A failure here is not fatal: creating the invitation replaces the old
    // binding anyway, and `create_pairing` retires whatever revoke is left.
    if let Err(error) = pairing::retry_pending_revokes().await {
        eprintln!("remote: re-pair could not settle the saved pairing's revoke: {error}");
    }
    start(RemoteStartInput {}).await
}
