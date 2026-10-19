//! Re-pair policy belongs to Server; real persistence tests stay in Desktop.
use super::*;
pub async fn re_pair() -> Result<RemoteStatus, crate::AppError> {
    SUPERVISOR.re_pair().await.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use crate::remote::test_support::{init_store, HomeGuard};

    /// `pair_new_device` on a machine with nothing paired is the replace path:
    /// with no slot to keep, adding beside the others and replacing them are the
    /// same request, and the replace path is the one that stops a runtime left
    /// behind by a pairing book emptied from another process.
    ///
    /// The observable here is the start's own refusal: no sign-in is a categorized
    /// local refusal, and reaching it proves the start ran after the (empty)
    /// revocation.
    #[tokio::test]
    async fn pairing_a_device_on_an_unpaired_machine_takes_the_replace_path() {
        let _home = HomeGuard::new("remote-pair-new-unpaired");
        init_store();
        assert!(crate::remote::pairing::load_slots().is_empty());

        let status = crate::remote::supervisor::pair_new_device()
            .await
            .expect("an unpaired machine still answers with a status");

        assert!(!matches!(status.phase, crate::remote::RemotePhase::Ready));
        assert_eq!(
            status.reason,
            Some(crate::remote::RemoteFailureReason::AccountAuthorization),
            "{status:?}"
        );
    }

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
