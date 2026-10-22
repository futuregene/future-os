//! The reconnect's three decisions, asserted without a child process.
//!
//! They used to be covered only end to end, through a child Desktop whose device
//! starts with an already-expired credential — the only credential that makes a
//! live refusal *reconnectable*, because `health.rs` treats a rejection of a
//! still-valid, refreshed credential as terminal. That state is also exactly when
//! the device's own renewal worker is due, so the first generation could be
//! rebuilt before the child ever printed `Remote: Ready`: which arm the run
//! reached depended on host timing, and it failed on the CI runner while passing
//! on a laptop. The decision below has no timing in it, so it is asserted here,
//! and the child tests keep their host measurements.
use super::start::{recovery_plan, SlotRecovery};
use super::test_support::{MemoryHost, TokioExecutor};
use super::*;
use crate::services::PairingHost;

/// A paired device as the book stores it: v2 handshake, peer identity known.
fn paired(pair_id: &str) -> PairingCreds {
    let mut creds = unpaired(pair_id);
    creds.secure = Some(secure::PairingIdentity {
        private_key: format!("private_{pair_id}"),
        public_key: format!("public_{pair_id}"),
        peer_public_key: Some(format!("peer_{pair_id}")),
        secret: None,
        expires_at: 0,
    });
    creds
}

/// The same device before the signed mutual handshake completed.
fn unpaired(pair_id: &str) -> PairingCreds {
    PairingCreds {
        handshake_version: 2,
        secure: None,
        pair_id: pair_id.to_string(),
        desktop_id: format!("desktop_{pair_id}"),
        nkey_seed: format!("seed_{pair_id}"),
        user_jwt: "jwt".to_string(),
        nats_url: "nats://127.0.0.1:4222".to_string(),
        nats_ws_url: "ws://127.0.0.1:4222".to_string(),
        jwt_expires_at: 0,
        control_url: None,
    }
}

#[test]
fn a_recoverable_device_is_refreshed_and_a_missing_one_is_stopped() {
    let kept = paired("pair_kept");
    let slots = vec![kept.clone(), paired("pair_other")];

    match recovery_plan(&slots, "pair_kept") {
        SlotRecovery::Refresh(creds) => {
            let creds = *creds;
            assert_eq!(creds.pair_id, "pair_kept");
            assert_eq!(creds.nkey_seed, kept.nkey_seed);
        }
        other => panic!("a v2 slot with a peer identity must be refreshed, got {other:?}"),
    }
    // A device the book no longer holds, and an empty book, both stop.
    assert!(matches!(
        recovery_plan(&slots, "pair_gone"),
        SlotRecovery::Stop
    ));
    assert!(matches!(
        recovery_plan(&[], "pair_kept"),
        SlotRecovery::Stop
    ));
}

/// An executor whose host has closed admission, which is exactly what a
/// teardown creates. Every other behaviour is `TokioExecutor`'s, so the only
/// difference the test below can measure is `is_stopping`.
struct Stopping(TokioExecutor);
impl TaskExecutor for Stopping {
    fn spawn(&self, future: BoxFuture<'static, ()>) {
        self.0.spawn(future);
    }
    fn spawn_native(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        self.0.spawn_native(job);
    }
    fn is_stopping(&self) -> bool {
        true
    }
}

/// A dead generation is rebuilt only while the host is serving. Once admission
/// is closed the host is tearing down: the tasks that stopped are the teardown
/// itself, so an `[RT001]` episode and a replacement generation are both wrong —
/// the rebuild would spend the shutdown budget on a bridge that is dropped again
/// moments later, and the episode survives as a `reconnect_required` code for a
/// stop nobody suffered.
///
/// The serving case is asserted in the same test on purpose: it is what makes the
/// stopping assertion falsifiable, since removing the guard makes both read 1.
#[tokio::test]
async fn a_stopping_host_neither_records_nor_rebuilds_a_dead_generation() {
    let serving = Supervisor::new(Arc::new(MemoryHost::default()), Arc::new(TokioExecutor));
    serving.spawn_runtime_reconnect("pair_serving".into());
    assert_eq!(
        serving.runtime_reconnect_attempts.load(Ordering::Acquire),
        1,
        "a serving host counts the failure and rebuilds the generation"
    );

    let stopping = Supervisor::new(
        Arc::new(MemoryHost::default()),
        Arc::new(Stopping(TokioExecutor)),
    );
    stopping.spawn_runtime_reconnect("pair_stopping".into());
    assert_eq!(
        stopping.runtime_reconnect_attempts.load(Ordering::Acquire),
        0,
        "a stopping host must not report its own teardown as a failure"
    );
    assert!(
        stopping.last_error_code.lock().unwrap().is_none(),
        "and must not arm the reconnect-required code"
    );
}

/// A credential that cannot be upgraded in place is forgotten, not served: a
/// pre-v2 slot, and a v2 slot that never learned its peer's identity.
#[test]
fn a_credential_that_cannot_be_upgraded_is_forgotten() {
    let mut legacy = paired("pair_legacy");
    legacy.handshake_version = 1;
    assert!(matches!(
        recovery_plan(&[legacy], "pair_legacy"),
        SlotRecovery::Forget
    ));
    assert!(matches!(
        recovery_plan(&[unpaired("pair_unconfirmed")], "pair_unconfirmed"),
        SlotRecovery::Forget
    ));
}

/// The `Forget` arm's own effect: the slot leaves the book, so a device that can
/// never connect stops being retried on every start.
#[tokio::test]
async fn forgetting_a_slot_removes_it_from_the_book() {
    let host = Arc::new(MemoryHost::default());
    let mut legacy = paired("pair_legacy");
    legacy.handshake_version = 1;
    host.save(&paired("pair_kept"))
        .expect("save the kept device");
    host.save(&legacy).expect("save the legacy device");
    let supervisor = Supervisor::new(host.clone(), Arc::new(TokioExecutor));

    supervisor.forget_slot("pair_legacy").expect("forget it");

    let slots = host.load_slots();
    assert_eq!(slots.len(), 1, "{slots:?}");
    assert_eq!(slots[0].pair_id, "pair_kept");
}
