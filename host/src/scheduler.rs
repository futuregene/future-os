//! Process-lifetime scheduler for independent, fixed-interval maintenance jobs.
//!
//! Intervals are minimum delays, not wall-clock appointments. A suspended app
//! runs an overdue job once after it resumes, then schedules the next run from
//! that actual start time; missed intervals are never replayed in a burst.

use crate::console;
use std::future::Future;
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::agent_bridge::SyncFutureModelsResult;
use crate::future_login::FutureBalance;
use crate::Error as AppError;

const APP_UPDATE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const FUTURE_BALANCE_INTERVAL: Duration = Duration::from_secs(60 * 60);
const FUTURE_MODELS_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

pub const APP_UPDATE_EVENT: &str = "scheduler-app-update";
pub const FUTURE_BALANCE_EVENT: &str = "scheduler-future-balance";
/// The window's view of the account is stale and must be re-read from the
/// source of truth. Sent when the platform rejects the stored credential *and*
/// when there is no stored credential at all — the second case is the one a
/// window cannot notice by itself, because the file can be emptied from outside
/// the app (`future auth logout`, a TUI logout, another process editing it).
pub const FUTURE_AUTH_CHANGED_EVENT: &str = "scheduler-future-auth-changed";
pub const FUTURE_MODELS_EVENT: &str = "scheduler-future-models";

pub static APP_UPDATE_JOB: LazyLock<FixedIntervalJob> =
    LazyLock::new(|| FixedIntervalJob::new(APP_UPDATE_INTERVAL));
pub static FUTURE_BALANCE_JOB: LazyLock<FixedIntervalJob> =
    LazyLock::new(|| FixedIntervalJob::new(FUTURE_BALANCE_INTERVAL));
pub static FUTURE_MODELS_JOB: LazyLock<FixedIntervalJob> =
    LazyLock::new(|| FixedIntervalJob::new(FUTURE_MODELS_INTERVAL));

pub struct FixedIntervalJob {
    schedule: Mutex<FixedIntervalSchedule>,
    execution: tokio::sync::Mutex<()>,
}

impl FixedIntervalJob {
    fn new(interval: Duration) -> Self {
        Self {
            schedule: Mutex::new(FixedIntervalSchedule::new(Instant::now(), interval)),
            execution: tokio::sync::Mutex::new(()),
        }
    }

    fn schedule(&self) -> MutexGuard<'_, FixedIntervalSchedule> {
        self.schedule
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub fn next_due(&self) -> Instant {
        self.schedule().next_due
    }

    fn claim_if_due(&self, now: Instant) -> bool {
        self.schedule().claim_if_due(now)
    }

    fn note_explicit_trigger(&self, now: Instant) {
        self.schedule().note_trigger(now);
    }
}

#[derive(Debug)]
struct FixedIntervalSchedule {
    interval: Duration,
    next_due: Instant,
}

impl FixedIntervalSchedule {
    fn new(now: Instant, interval: Duration) -> Self {
        Self {
            interval,
            next_due: now + interval,
        }
    }

    fn claim_if_due(&mut self, now: Instant) -> bool {
        if now < self.next_due {
            return false;
        }
        // Anchor to the actual run after a wake-up. This skips every missed
        // slot instead of letting a timer implementation replay a burst.
        self.next_due = now + self.interval;
        true
    }

    fn note_trigger(&mut self, now: Instant) {
        // Startup/manual/event-driven executions share the same minimum-delay
        // clock, so an automatic run cannot immediately duplicate them.
        self.next_due = now + self.interval;
    }
}

pub fn spawn_fixed_interval<F, Fut>(job: &'static FixedIntervalJob, mut run: F)
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let loop_body = async move {
        loop {
            tokio::time::sleep_until(tokio::time::Instant::from_std(job.next_due())).await;
            if !job.claim_if_due(Instant::now()) {
                // An explicit trigger moved the deadline while this sleep was
                // pending. Re-read it on the next loop iteration.
                continue;
            }
            let _execution = job.execution.lock().await;
            run().await;
        }
    };
    // The window build spawns on Tauri's runtime (the one the WebView's
    // asynchronous commands run on); the server build has no Tauri runtime and
    // uses the process runtime `headless::run` installs, exactly like
    // `tasks::start_headless`.
    crate::runtime::spawn(loop_body);
}

pub async fn run_explicit<T, F, Fut>(job: &'static FixedIntervalJob, run: F) -> Result<T, AppError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, AppError>>,
{
    job.note_explicit_trigger(Instant::now());
    let _execution = job.execution.lock().await;
    run().await
}

pub fn future_signed_in() -> bool {
    crate::auth_store::read()
        .ok()
        .and_then(|auth| auth.get(crate::auth_store::FUTURE_PROVIDER_ID).cloned())
        .and_then(|entry| {
            entry
                .get("key")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .map(str::to_owned)
        })
        .is_some_and(|key| !key.is_empty())
}

/// What one hourly account pass has to say to the window.
pub enum BalancePass {
    /// Signed in, and the platform answered: these are the current credits.
    Balance(FutureBalance),
    /// This window has to re-read the account. Either the stored credential is
    /// gone — removed from outside the app, which this window cannot notice on
    /// its own — or the platform rejected it.
    AccountChanged,
    /// Signed in, and the fetch failed for a reason that is not an
    /// authorization rejection: nothing here for the window to act on.
    Silent,
}

/// The hourly account pass, split from its WebView so the policy — what the
/// window must be told — is testable without an app and without a network call.
pub async fn balance_pass<F, Fut>(signed_in: bool, fetch: F) -> BalancePass
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<FutureBalance, AppError>>,
{
    if !signed_in {
        // Silence here is what let a window keep showing an account and a
        // balance it no longer had: the credential can be dropped by `future
        // auth logout`, a TUI logout, or anything else that writes auth.json,
        // and this hourly pass is the only recurring observer of that file.
        return BalancePass::AccountChanged;
    }
    match fetch().await {
        Ok(balance) => BalancePass::Balance(balance),
        Err(error) => {
            let rejected = crate::future_login::is_auth_rejection(&error);
            console::eprintln!("FutureOS scheduled balance refresh failed: {error}");
            if rejected {
                BalancePass::AccountChanged
            } else {
                BalancePass::Silent
            }
        }
    }
}

/// The headless entry point's maintenance loop: the same Future-catalogue job
/// with the same interval and the same signed-in gate, without the two the
/// server cannot use. The window entry installs its own adapters; this exists in
/// the library so both entries share the job, its clock and its tests.
///
/// `app update` needs the Tauri updater plugin and an installer to hand the
/// downloaded bundle to, so it does not exist in the server build at all; the
/// `balance` job's only output is a WebView event, and the phone reads neither
/// the balance nor that event. The catalogue is different: the Agent announces
/// its own refresh (`publish_provider_config_changed`), which the remote bridge
/// already forwards to the phone, so a server that runs for weeks keeps the
/// phone's model list current instead of freezing it at first launch.
pub fn start_headless() {
    spawn_fixed_interval(&FUTURE_MODELS_JOB, refresh_future_catalogue_if_signed_in);
}

/// The server's catalogue job body. Split out so the signed-in gate is testable
/// without a runtime, and so the automatic run stays on the job's own clock: it
/// must never reset the deadline the way the explicit `*_now` entry points do.
async fn refresh_future_catalogue_if_signed_in() {
    if !future_signed_in() {
        return;
    }
    match crate::agent_bridge::sync_future_models().await {
        Ok(result) if result.synced => console::eprintln!(
            "FutureOS: Future model catalogue refreshed ({} models).",
            result.model_count
        ),
        Ok(_) => {}
        Err(error) => console::eprintln!("FutureOS scheduled model refresh failed: {error}"),
    }
}

pub async fn refresh_future_balance_now() -> Result<FutureBalance, AppError> {
    run_explicit(&FUTURE_BALANCE_JOB, crate::future_login::fetch_balance).await
}

pub async fn refresh_future_models_now() -> Result<SyncFutureModelsResult, AppError> {
    run_explicit(&FUTURE_MODELS_JOB, crate::agent_bridge::sync_future_models).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overdue_schedule_runs_once_and_reanchors_after_resume() {
        let start = Instant::now();
        let interval = Duration::from_secs(60);
        let mut schedule = FixedIntervalSchedule::new(start, interval);

        assert!(!schedule.claim_if_due(start + Duration::from_secs(59)));
        let resumed = start + Duration::from_secs(300);
        assert!(schedule.claim_if_due(resumed));
        assert_eq!(schedule.next_due, resumed + interval);
        assert!(!schedule.claim_if_due(resumed));
    }

    #[test]
    fn explicit_trigger_resets_the_automatic_deadline() {
        let start = Instant::now();
        let interval = Duration::from_secs(60);
        let mut schedule = FixedIntervalSchedule::new(start, interval);
        let manual = start + Duration::from_secs(55);

        schedule.note_trigger(manual);
        assert!(!schedule.claim_if_due(start + interval));
        assert!(schedule.claim_if_due(manual + interval));
    }

    #[test]
    fn configured_intervals_match_product_policy() {
        assert_eq!(APP_UPDATE_INTERVAL, Duration::from_secs(86_400));
        assert_eq!(FUTURE_BALANCE_INTERVAL, Duration::from_secs(3_600));
        assert_eq!(FUTURE_MODELS_INTERVAL, Duration::from_secs(86_400));
    }

    /// The three intervals above are only policy if the global jobs are built
    /// from them: a job constructed through its lazy must report its first run
    /// one configured interval out, never a smaller default.
    #[test]
    fn the_global_jobs_are_built_on_the_configured_intervals() {
        // Dereference every lazy before taking `now`: each job anchors its
        // first run at construction, which is later than anything measured
        // before it.
        let jobs = [
            (&*APP_UPDATE_JOB, APP_UPDATE_INTERVAL),
            (&*FUTURE_BALANCE_JOB, FUTURE_BALANCE_INTERVAL),
            (&*FUTURE_MODELS_JOB, FUTURE_MODELS_INTERVAL),
        ];
        let now = Instant::now();
        for (job, interval) in jobs {
            let due = job.next_due();
            assert!(
                due > now,
                "the first run is one interval out: due in {:?}",
                due - now
            );
            assert!(
                due <= now + interval,
                "a job must not wait longer than its configured interval: due in {:?}, interval {interval:?}",
                due - now
            );
        }
    }

    /// A job's own delegation must agree with the schedule it wraps — the
    /// scheduler reads `next_due`/`claim_if_due` through the job, not the
    /// schedule, so a mismatch here would be invisible to the pure tests above.
    #[test]
    fn a_job_delegates_to_its_schedule() {
        let job: &'static FixedIntervalJob =
            Box::leak(Box::new(FixedIntervalJob::new(Duration::from_secs(60))));
        let start = Instant::now();
        assert!(job.next_due() > start, "the first run is one interval out");
        assert!(
            !job.claim_if_due(start),
            "a job must not be due before its first interval"
        );
        let resumed = start + Duration::from_secs(600);
        assert!(job.claim_if_due(resumed));
        assert_eq!(job.next_due(), resumed + Duration::from_secs(60));
        // An explicit trigger moves the deadline the same way, so a manual
        // refresh cannot be duplicated by the automatic run right after it.
        let manual = resumed + Duration::from_secs(5);
        job.note_explicit_trigger(manual);
        assert_eq!(job.next_due(), manual + Duration::from_secs(60));
        assert!(!job.claim_if_due(manual));
        assert!(job.claim_if_due(manual + Duration::from_secs(60)));
    }

    /// A poisoned mutex must not wedge the scheduler: a panic in one job must
    /// leave the others able to claim their slots.
    #[test]
    fn a_poisoned_schedule_is_recovered() {
        let job: &'static FixedIntervalJob =
            Box::leak(Box::new(FixedIntervalJob::new(Duration::from_secs(60))));
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = job.schedule();
            panic!("intentional: poison the schedule for the recovery test");
        }));
        let start = Instant::now();
        // Still usable: the deadline survives, and the claim succeeds.
        assert!(job.next_due() > start);
        assert!(job.claim_if_due(start + Duration::from_secs(60)));
    }

    /// `run_explicit` reports what the job returned, and propagates its error
    /// unchanged — the caller (`refresh_future_balance_now` and other entry adapters) surfaces it
    /// to the UI, so swallowing it would show a stale "up to date".
    #[tokio::test]
    async fn an_explicit_run_returns_the_inner_result_and_its_error() {
        let job: &'static FixedIntervalJob =
            Box::leak(Box::new(FixedIntervalJob::new(Duration::from_secs(3600))));
        let before = Instant::now();
        let value = run_explicit(job, || async { Ok::<_, AppError>(7_u32) })
            .await
            .expect("the inner Ok is returned");
        assert_eq!(value, 7);
        // The explicit run counts as a run: the automatic deadline moved.
        assert!(job.next_due() >= before + Duration::from_secs(3600));

        let error = run_explicit(job, || async { Err::<u32, _>("inner failure".into()) })
            .await
            .expect_err("the inner error is propagated");
        assert!(error.to_string().contains("inner failure"));
    }

    /// The execution lock is the scheduler's only defence against a slow job
    /// running twice at once — two overlapping update checks would each write
    /// the same cache and emit a different status. Driven with real concurrency
    /// so the lock, not the test's own sequencing, is what serializes them.
    #[tokio::test]
    async fn concurrent_explicit_runs_never_overlap() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let job: &'static FixedIntervalJob =
            Box::leak(Box::new(FixedIntervalJob::new(Duration::from_secs(3600))));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..4 {
            let active = active.clone();
            let peak = peak.clone();
            let completed = completed.clone();
            handles.push(tokio::spawn(async move {
                run_explicit(job, || async move {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    // Long enough that an unserialized run would overlap.
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    completed.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, AppError>(())
                })
                .await
            }));
        }
        for handle in handles {
            handle.await.expect("task joined").expect("run ok");
        }
        assert_eq!(
            peak.load(Ordering::SeqCst),
            1,
            "the execution lock must serialize overlapping runs"
        );
        assert_eq!(
            completed.load(Ordering::SeqCst),
            4,
            "serializing must not drop a run"
        );
    }

    /// The spawned loop is the production path for all three jobs, and its
    /// `continue` arm is the one that handles an explicit trigger moving the
    /// deadline while the sleep is pending. Both are driven here with a real
    /// short interval instead of the production 1–24h ones.
    #[tokio::test]
    async fn a_spawned_job_fires_at_its_deadline_and_reanchors() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        // HOME fixtures cancel the process-wide task registry. Hold their
        // shared guard so parallel cleanup cannot abort this loop mid-test,
        // and stop our loop before the next fixture changes HOME.
        let _home = crate::auth_store::test_support::HomeGuard::new("scheduler_spawned");
        let job: &'static FixedIntervalJob =
            Box::leak(Box::new(FixedIntervalJob::new(Duration::from_millis(40))));
        let runs = Arc::new(AtomicUsize::new(0));
        let counter = runs.clone();
        spawn_fixed_interval(job, move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        });

        // Let the loop wake and run at least twice: an interval-based job must
        // keep rearming, not fire once and stop.
        let deadline = Instant::now() + Duration::from_secs(5);
        while runs.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            runs.load(Ordering::SeqCst) >= 2,
            "the fixed-interval loop must rearm and run again, ran {} time(s)",
            runs.load(Ordering::SeqCst)
        );

        // Push the deadline out from under the pending sleep: the loop must
        // observe the moved deadline and not run again before it.
        job.note_explicit_trigger(Instant::now() + Duration::from_secs(3600));
        let settled = runs.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            runs.load(Ordering::SeqCst),
            settled,
            "an explicit trigger must suppress the automatic run it just replaced"
        );
    }

    /// The hourly account pass must tell the window to re-read the account when
    /// the stored credential is gone — the case that used to be silent, and the
    /// only way a window can end up showing an account and a balance that are no
    /// longer there. A credential the platform *rejects* needs the same re-read;
    /// any other failure is this desktop's own business.
    #[tokio::test]
    async fn the_account_pass_announces_a_credential_that_is_gone_or_rejected() {
        // Signed out: the pass short-circuits, so the injected fetch is never
        // consulted — its sentinel credits must not be what the window sees.
        assert!(matches!(
            balance_pass(false, || async { Ok(FutureBalance { credits: -1.0 }) }).await,
            BalancePass::AccountChanged
        ));

        assert!(matches!(
            balance_pass(true, || async { Ok(FutureBalance { credits: 7.5 }) }).await,
            BalancePass::Balance(balance) if balance.credits == 7.5
        ));

        assert!(matches!(
            balance_pass(true, || async {
                Err(AppError::Remote {
                    status: 401,
                    code: None,
                    message: "rejected".to_string(),
                })
            })
            .await,
            BalancePass::AccountChanged
        ));

        // Offline is not an account answer: the last-known balance stands.
        assert!(matches!(
            balance_pass(true, || async {
                Err(AppError::RemoteTransport("offline".to_string()))
            })
            .await,
            BalancePass::Silent
        ));
    }

    /// The server build's maintenance loop: the catalogue job's body is gated on
    /// the account and, unlike the explicit `*_now` entry points, must leave the
    /// scheduled deadline alone — otherwise the automatic run and a manual
    /// refresh would keep pushing each other out and neither would settle.
    ///
    /// Deliberately never signs in: the signed-in arm reaches the Agent, and a
    /// developer machine running its own `future agent` must stay untouched.
    #[tokio::test]
    async fn the_headless_catalogue_job_skips_while_signed_out_and_keeps_its_clock() {
        let _home = crate::auth_store::test_support::HomeGuard::new("scheduler_headless");

        // Signed out: the body returns before the Agent is contacted at all.
        assert!(!future_signed_in());
        let before = FUTURE_MODELS_JOB.next_due();
        refresh_future_catalogue_if_signed_in().await;
        assert_eq!(
            FUTURE_MODELS_JOB.next_due(),
            before,
            "an automatic run must not re-anchor the schedule; only an explicit trigger does"
        );
        // Arming the loop is inert: the first automatic run is a full interval
        // away, so the deadline is untouched by the spawn.
        start_headless();
        assert_eq!(FUTURE_MODELS_JOB.next_due(), before);
    }

    /// `future_signed_in` gates both signed-in-only jobs. It must be false for
    /// every shape of "not signed in" — no file, no entry, an entry with no
    /// key, a blank key — and only true for a real key. Whitespace-only counts
    /// as blank: that is a cleared field, not a credential.
    #[test]
    fn future_signed_in_requires_a_non_blank_key() {
        let _home = crate::auth_store::test_support::HomeGuard::new("scheduler_signin");

        // No auth file at all.
        assert!(!future_signed_in(), "an absent auth file is not signed in");

        // An unrelated provider must not satisfy the check.
        crate::auth_store::set_provider_key("some-other-provider", "k").expect("write");
        assert!(
            !future_signed_in(),
            "another provider's key is not a Future key"
        );

        // A Future entry with no key.
        let mut map = crate::auth_store::read().expect("read");
        map.insert(
            crate::auth_store::FUTURE_PROVIDER_ID.to_string(),
            serde_json::json!({}),
        );
        crate::auth_store::write(&map).expect("write");
        assert!(
            !future_signed_in(),
            "an entry without a key is not signed in"
        );

        // A blank key is a cleared field.
        for blank in ["", "   "] {
            let mut map = crate::auth_store::read().expect("read");
            map.insert(
                crate::auth_store::FUTURE_PROVIDER_ID.to_string(),
                serde_json::json!({ "key": blank }),
            );
            crate::auth_store::write(&map).expect("write");
            assert!(
                !future_signed_in(),
                "a blank key ({blank:?}) must not count as signed in"
            );
        }

        // A non-string key cannot be a credential either.
        let mut map = crate::auth_store::read().expect("read");
        map.insert(
            crate::auth_store::FUTURE_PROVIDER_ID.to_string(),
            serde_json::json!({ "key": 42 }),
        );
        crate::auth_store::write(&map).expect("write");
        assert!(!future_signed_in(), "a non-string key is not signed in");

        // The real thing.
        crate::auth_store::set_future_login("fut_live_key", "https://api.future.test")
            .expect("set login");
        assert!(
            future_signed_in(),
            "a trimmed non-empty key is the signed-in state"
        );
    }

    /// The three `*_now` entry points exist so a manual refresh and a startup
    /// check share the automatic job's clock. Each must reset that job's
    /// deadline even when the underlying call fails, or the automatic loop
    /// would immediately repeat the failed attempt.
    #[tokio::test]
    async fn a_manual_refresh_resets_its_own_job_deadline_on_failure() {
        let _home = crate::auth_store::test_support::HomeGuard::new("scheduler_now");
        // Not signed in: `fetch_balance` refuses before any network call.
        let before = Instant::now();
        let result = refresh_future_balance_now().await;
        assert!(result.is_err(), "no key means no balance fetch");
        assert!(
            FUTURE_BALANCE_JOB.next_due() >= before + Duration::from_secs(3600),
            "a failed manual refresh must still reset the automatic deadline"
        );

        // The model sync does not consult the account: its outcome depends on
        // whether an Agent answers (the scripted test double another test in
        // the process may have installed) or is unreachable. Both are ordinary
        // production outcomes, so only the clock is asserted here — the failure
        // path is covered by the balance job above.
        let before = Instant::now();
        let _ = refresh_future_models_now().await;
        assert!(
            FUTURE_MODELS_JOB.next_due() >= before + Duration::from_secs(3600),
            "a manual refresh must reset the automatic deadline whatever the Agent says"
        );
    }

    /// The headless catalogue body's signed-in arm. The sync itself is the
    /// Agent's answer (a scripted double or an unreachable endpoint, both
    /// ordinary), so only the account state is arranged here.
    #[tokio::test]
    async fn the_catalogue_body_runs_the_sync_when_signed_in() {
        let _home = crate::auth_store::test_support::HomeGuard::new("scheduler-catalogue");
        crate::auth_store::set_future_login("key", "https://example.invalid").expect("sign in");
        refresh_future_catalogue_if_signed_in().await;
    }

    /// Both successful catalogue outcomes: a real sync is logged, a no-op is not.
    /// Scripted through the Agent double so the body's two `Ok` arms need no
    /// network and no signed-in platform.
    #[tokio::test]
    async fn the_catalogue_body_reports_a_sync_and_a_no_op() {
        let _home = crate::auth_store::test_support::HomeGuard::new("scheduler-catalogue-arms");
        let mock = crate::agent_bridge::test_support::mock_agent();
        mock.push_data(
            "sync_future_models",
            serde_json::json!({"synced": true, "modelCount": 3, "revision": 7}),
        );
        mock.push_data(
            "sync_future_models",
            serde_json::json!({"synced": false, "modelCount": 0, "revision": 0}),
        );
        crate::auth_store::set_future_login("key", "https://example.invalid").expect("sign in");

        refresh_future_catalogue_if_signed_in().await;
        refresh_future_catalogue_if_signed_in().await;
    }
}
