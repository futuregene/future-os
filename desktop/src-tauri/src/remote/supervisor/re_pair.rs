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

#[cfg(test)]
mod tests {
    use crate::remote::test_support::{init_store, HomeGuard};

    /// Settling the saved pairing's revoke is best-effort: the invitation that
    /// gets minted next replaces the old binding anyway, so a queue this machine
    /// cannot read must not cost the user the code they asked for.
    ///
    /// The corrupt queue is the whole input. `unpair` cannot be the one that
    /// fails here — with no saved device its `queue_revoke` loop does not run —
    /// so the failure seen at `retry_pending_revokes` is the *first* one, and
    /// the assertion that `re_pair` still answers with the start's own result is
    /// what proves the log arm was taken and the function continued.
    #[tokio::test]
    async fn a_revoke_queue_that_cannot_be_read_does_not_abort_the_re_pair() {
        let _home = HomeGuard::new("remote-re-pair-corrupt-queue");
        init_store();
        let queue = crate::store::future_dir()
            .expect("future dir")
            .join("remote_pending_revokes.json");
        let corrupt = b"{ this is not json";
        std::fs::write(&queue, corrupt).expect("write a corrupt queue");
        // No saved device: `unpair`'s queue_revoke loop never runs, so nothing
        // before the settlement pass can fail on this file.
        assert!(crate::remote::pairing::load_slots().is_empty());

        let status = crate::remote::supervisor::re_pair()
            .await
            .expect("a failed settlement must not abort the re-pair");

        // The answer is the start's own: no sign-in is a categorized local
        // refusal, and reaching it is the proof that `start()` ran after the
        // settlement failure.
        assert!(
            !matches!(status.phase, crate::remote::RemotePhase::Ready),
            "{status:?}"
        );
        assert_eq!(
            status.reason,
            Some(crate::remote::RemoteFailureReason::AccountAuthorization),
            "the re-pair must report the start's refusal, not the revoke failure: {status:?}"
        );
        // And nothing silently rewrote the queue it could not read.
        assert_eq!(
            std::fs::read(&queue).expect("the queue is still there"),
            corrupt,
            "an unreadable queue must be left for a later pass, not clobbered"
        );
    }
}
