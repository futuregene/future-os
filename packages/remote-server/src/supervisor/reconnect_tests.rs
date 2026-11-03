//! The reconnect's three decisions, asserted without a child process.
//!
//! They used to be covered only end to end, through a child Desktop whose device
//! starts with an already-expired credential 鈥?the only credential that makes a
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
/// itself, so an `[RT001]` episode and a replacement generation are both wrong 鈥?/// the rebuild would spend the shutdown budget on a bridge that is dropped again
/// moments later, and the episode survives as a `reconnect_required` code for a
/// stop nobody suffered.
///
/// The serving case is asserted in the same test on purpose: it is what makes the
/// stopping assertion falsifiable, since removing the guard makes both read 1.
#[tokio::test]
async fn a_stopping_host_neither_records_nor_rebuilds_a_dead_generation() {
    let serving = Supervisor::new(Arc::new(MemoryHost::default()), Arc::new(TokioExecutor));
    serving.spawn_runtime_reconnect("pair_serving".into(), 1);
    assert_eq!(
        serving.runtime_reconnect_attempts.load(Ordering::Acquire),
        1,
        "a serving host counts the failure and rebuilds the generation"
    );

    let stopping = Supervisor::new(
        Arc::new(MemoryHost::default()),
        Arc::new(Stopping(TokioExecutor)),
    );
    stopping.spawn_runtime_reconnect("pair_stopping".into(), 1);
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

    supervisor
        .forget_slot("pair_legacy", "test")
        .expect("forget it");

    let slots = host.load_slots();
    assert_eq!(slots.len(), 1, "{slots:?}");
    assert_eq!(slots[0].pair_id, "pair_kept");
}

/// A spent burst budget buys a cooldown, not silence.
///
/// Before this, the third fast rebuild exhausted the budget and the watcher then
/// idled out the rest of the ten-minute failure window: nothing was attempted
/// again until the whole window elapsed, so a relay blip that the very next
/// attempt would have healed kept the device unreachable for minutes and it came
/// back only when the window expired. The decision is pure, so the cadence is
/// asserted here without a clock.
#[test]
fn a_spent_burst_budget_cools_down_instead_of_idling_the_window() {
    let now = 1_000_000;
    // Inside the burst: attempt at once.
    assert_eq!(reconnect_step(0, now - 1, now), ReconnectStep::Attempt);
    assert_eq!(
        reconnect_step(MAX_RUNTIME_RECONNECT_ATTEMPTS - 1, now - 1, now),
        ReconnectStep::Attempt
    );
    // Spent, still inside the window: wait one cooldown, then attempt.
    assert_eq!(
        reconnect_step(MAX_RUNTIME_RECONNECT_ATTEMPTS, now - 1, now),
        ReconnectStep::Wait(RUNTIME_COOLDOWN_BASE)
    );
    assert_eq!(
        reconnect_step(u8::MAX, now - 1, now),
        ReconnectStep::Wait(RUNTIME_COOLDOWN_MAX)
    );
    // Past the window the budget is forgiven, so the burst is available again.
    assert_eq!(
        reconnect_step(
            MAX_RUNTIME_RECONNECT_ATTEMPTS,
            now,
            now + RUNTIME_FAILURE_WINDOW_MS + 1
        ),
        ReconnectStep::Attempt
    );
}

/// The cooldown doubles per attempt and saturates at the ceiling, so a credential
/// the relay keeps refusing settles at one attempt per ten minutes instead of
/// retrying a dead grant in a tight loop.
#[test]
fn the_cooldown_doubles_per_attempt_and_saturates() {
    assert_eq!(
        cooldown_delay(MAX_RUNTIME_RECONNECT_ATTEMPTS),
        RUNTIME_COOLDOWN_BASE
    );
    assert_eq!(
        cooldown_delay(MAX_RUNTIME_RECONNECT_ATTEMPTS + 1),
        RUNTIME_COOLDOWN_BASE * 2
    );
    assert_eq!(
        cooldown_delay(MAX_RUNTIME_RECONNECT_ATTEMPTS + 5),
        RUNTIME_COOLDOWN_MAX
    );
    assert_eq!(cooldown_delay(u8::MAX), RUNTIME_COOLDOWN_MAX);
    // Never shorter than the base, whatever the count.
    assert!(cooldown_delay(0) >= RUNTIME_COOLDOWN_BASE);
}

/// The attempt counter saturates instead of wrapping: slow retries now push it
/// past the burst budget, so `fetch_add(1) + 1` on a `u8` would wrap to zero and
/// silently shorten the cooldown back to the base.
#[test]
fn the_attempt_counter_saturates_instead_of_wrapping() {
    let supervisor = Supervisor::new(Arc::new(MemoryHost::default()), Arc::new(TokioExecutor));
    // An open window, or the first call forgives the counter instead of growing it.
    supervisor
        .runtime_failure_window_started
        .store(1_000, Ordering::Release);
    supervisor
        .runtime_reconnect_attempts
        .store(u8::MAX - 1, Ordering::Release);
    assert_eq!(supervisor.record_runtime_failure(2_000), u8::MAX);
    assert_eq!(supervisor.record_runtime_failure(3_000), u8::MAX);
    assert_eq!(
        supervisor
            .runtime_reconnect_attempts
            .load(Ordering::Acquire),
        u8::MAX,
        "a saturated counter stays saturated"
    );
}

/// The present tier retries every minute at most, and there is no "give up"
/// arm for a network failure to fall into.
///
/// The old ceiling was ten minutes for every case, which is a length of silence
/// a reader holding the phone reads as a broken product rather than as a retry.
#[test]
fn a_present_device_is_retried_every_minute_and_never_abandoned() {
    for attempts in 0..=u8::MAX {
        match reconnect_step_for(attempts, 0, 1_000, true) {
            ReconnectStep::Attempt => {}
            ReconnectStep::Wait(wait) => assert!(
                wait <= RUNTIME_COOLDOWN_PRESENT_MAX,
                "attempt {attempts} waited {wait:?}, past the present ceiling"
            ),
        }
    }
    assert_eq!(
        cooldown_delay_for(MAX_RUNTIME_RECONNECT_ATTEMPTS, true),
        RUNTIME_COOLDOWN_BASE
    );
    assert_eq!(
        cooldown_delay_for(MAX_RUNTIME_RECONNECT_ATTEMPTS + 1, true),
        Duration::from_secs(60)
    );
    assert_eq!(
        cooldown_delay_for(u8::MAX, true),
        RUNTIME_COOLDOWN_PRESENT_MAX
    );
    assert_eq!(cooldown_delay_for(u8::MAX, false), RUNTIME_COOLDOWN_MAX);
}

/// The hour cap bounds the *rate*, which the cooldown alone does not.
#[test]
fn the_hourly_cap_bounds_the_rate_in_each_tier() {
    assert!(!hourly_attempts_reached(
        RUNTIME_HOURLY_ATTEMPTS_PRESENT - 1,
        true
    ));
    assert!(hourly_attempts_reached(
        RUNTIME_HOURLY_ATTEMPTS_PRESENT,
        true
    ));
    assert!(!hourly_attempts_reached(
        RUNTIME_HOURLY_ATTEMPTS_ABSENT - 1,
        false
    ));
    assert!(hourly_attempts_reached(
        RUNTIME_HOURLY_ATTEMPTS_ABSENT,
        false
    ));
    assert_eq!(
        wait_until_next_hour(RUNTIME_HOUR_MS - 1),
        Duration::from_millis(1)
    );
    assert_eq!(
        wait_until_next_hour(0),
        Duration::from_millis(RUNTIME_HOUR_MS)
    );
}

/// Jitter keeps a fifth either side and never leaves the tier's ceiling.
#[test]
fn jitter_stays_within_one_fifth() {
    let base = Duration::from_secs(60);
    assert_eq!(with_jitter(base, 0.0), Duration::from_millis(48_000));
    assert_eq!(with_jitter(base, 1.0), Duration::from_millis(72_000));
    // A caller handing in nonsense still gets a usable wait.
    assert_eq!(with_jitter(base, 2.0), Duration::from_millis(72_000));
}

/// A renewal names itself once it is stuck, and says nothing while it is fine.
#[test]
fn a_renewal_names_when_it_is_stuck() {
    use super::status::refreshing_label;
    assert_eq!(refreshing_label(0, false), None);
    assert_eq!(refreshing_label(0, true), Some("pending".to_string()));
    assert_eq!(refreshing_label(2, true), Some("pending".to_string()));
    assert_eq!(
        refreshing_label(crate::health::MINT_FAILURES_BEFORE_STUCK, true),
        Some("stuck(3)".to_string())
    );
    assert_eq!(refreshing_label(9, false), Some("stuck(9)".to_string()));

    let health = crate::health::NatsHealth::default();
    assert_eq!(health.mint_failures(), 0);
    assert_eq!(health.note_mint_failure(), 1);
    assert_eq!(health.note_mint_failure(), 2);
    assert_eq!(health.note_mint_failure(), 3);
    assert_eq!(health.mint_failures(), 3);
    health.clear_mint_failures();
    assert_eq!(health.mint_failures(), 0);
}

/// A command lane that dies is re-subscribed five times inside eight seconds
/// before the generation gives up 鈥?not left deaf, and not retried forever.
#[test]
fn a_dead_command_lane_retries_five_times_within_eight_seconds() {
    assert_eq!(crate::commands::RESUBSCRIBE_ATTEMPTS, 5);
    for attempt in 1..=crate::commands::RESUBSCRIBE_ATTEMPTS {
        let delay = crate::commands::resubscribe_delay(attempt);
        assert!(
            delay >= Duration::from_millis(400),
            "attempt {attempt}: {delay:?} is below the jittered floor"
        );
        assert!(
            delay <= Duration::from_millis(9_600),
            "attempt {attempt}: {delay:?} is past the 8 s ceiling plus jitter"
        );
    }
}

/// The lane is health: a bridge with no subscriber is not "connected", and the
/// timestamp says how long it has been deaf.
#[test]
fn a_deaf_command_lane_reports_itself_down_since_it_lost_the_subscription() {
    let health = crate::health::NatsHealth::default();
    assert!(!health.command_lane_is_down());
    assert_eq!(health.command_lane_down_since(), None);
    health.command_lane_down();
    health.command_lane_down();
    assert!(health.command_lane_is_down());
    let since = health
        .command_lane_down_since()
        .expect("the first moment it went down");
    assert!(since > 0);
    health.command_lane_up();
    assert!(!health.command_lane_is_down());
    assert_eq!(health.command_lane_down_since(), None);
}

/// Both directions of the give-up rule, at the classification that decides it.
///
/// A revoked pairing is a verdict: the refresh path forgets the device instead of
/// retrying. A network failure is *not* a verdict: it keeps its ladder, which is
/// what stops an ordinary outage from becoming "re-pair to continue".
#[test]
fn a_revoked_pairing_ends_the_retry_while_a_network_failure_keeps_its_ladder() {
    let revoked = crate::Error::Remote {
        status: 401,
        code: Some("invalid_remote_credential".to_string()),
        message: "revoked".to_string(),
    };
    assert!(crate::pairing::is_invalid_or_revoked_error(&revoked));
    assert!(!crate::pairing::is_account_authorization_error(&revoked));

    let network = crate::Error::RemoteTransport("relay refused the socket".to_string());
    assert!(!crate::pairing::is_invalid_or_revoked_error(&network));
    assert!(super::start::add_retryable(1, &network));
    assert!(!super::start::add_retryable(1, &revoked));

    // And the plan for a pairing that is gone is to stop, never to keep minting.
    assert!(matches!(
        recovery_plan(&[], "pair_gone"),
        SlotRecovery::Stop
    ));
}
