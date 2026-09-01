//! Process-lifetime scheduler for independent, fixed-interval maintenance jobs.
//!
//! Intervals are minimum delays, not wall-clock appointments. A suspended app
//! runs an overdue job once after it resumes, then schedules the next run from
//! that actual start time; missed intervals are never replayed in a burst.

use std::future::Future;
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tauri::Emitter;

use crate::agent_bridge::SyncFutureModelsResult;
use crate::commands::UpdateStatus;
use crate::future_login::FutureBalance;
use crate::AppError;

const APP_UPDATE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const FUTURE_BALANCE_INTERVAL: Duration = Duration::from_secs(60 * 60);
const FUTURE_MODELS_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

pub const APP_UPDATE_EVENT: &str = "scheduler-app-update";
pub const FUTURE_BALANCE_EVENT: &str = "scheduler-future-balance";
pub const FUTURE_MODELS_EVENT: &str = "scheduler-future-models";

static APP_UPDATE_JOB: LazyLock<FixedIntervalJob> =
    LazyLock::new(|| FixedIntervalJob::new(APP_UPDATE_INTERVAL));
static FUTURE_BALANCE_JOB: LazyLock<FixedIntervalJob> =
    LazyLock::new(|| FixedIntervalJob::new(FUTURE_BALANCE_INTERVAL));
static FUTURE_MODELS_JOB: LazyLock<FixedIntervalJob> =
    LazyLock::new(|| FixedIntervalJob::new(FUTURE_MODELS_INTERVAL));

struct FixedIntervalJob {
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

    fn next_due(&self) -> Instant {
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

fn spawn_fixed_interval<F, Fut>(job: &'static FixedIntervalJob, mut run: F)
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
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
    });
}

async fn run_explicit<T, F, Fut>(job: &'static FixedIntervalJob, run: F) -> Result<T, AppError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, AppError>>,
{
    job.note_explicit_trigger(Instant::now());
    let _execution = job.execution.lock().await;
    run().await
}

fn future_signed_in() -> bool {
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

/// Start the three process-lifetime maintenance loops. Their first automatic
/// runs happen after one full interval; existing startup/login/manual paths use
/// the `*_now` functions below and reset the matching deadline.
pub fn start(app: tauri::AppHandle) {
    let update_app = app.clone();
    spawn_fixed_interval(&APP_UPDATE_JOB, move || {
        let app = update_app.clone();
        async move {
            match crate::commands::perform_app_update_check(app.clone()).await {
                Ok(status) => {
                    let _ = app.emit(APP_UPDATE_EVENT, status);
                }
                Err(error) => eprintln!("FutureOS scheduled app update check failed: {error}"),
            }
        }
    });

    let balance_app = app.clone();
    spawn_fixed_interval(&FUTURE_BALANCE_JOB, move || {
        let app = balance_app.clone();
        async move {
            if !future_signed_in() {
                return;
            }
            match crate::future_login::fetch_balance().await {
                Ok(balance) => {
                    let _ = app.emit(FUTURE_BALANCE_EVENT, balance);
                }
                Err(error) => eprintln!("FutureOS scheduled balance refresh failed: {error}"),
            }
        }
    });

    spawn_fixed_interval(&FUTURE_MODELS_JOB, move || {
        let app = app.clone();
        async move {
            if !future_signed_in() {
                return;
            }
            match crate::agent_bridge::sync_future_models().await {
                Ok(result) if result.synced => {
                    let _ = app.emit(FUTURE_MODELS_EVENT, result);
                }
                Ok(_) => {}
                Err(error) => eprintln!("FutureOS scheduled model refresh failed: {error}"),
            }
        }
    });
}

pub async fn check_app_update_now<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<UpdateStatus, AppError> {
    run_explicit(&APP_UPDATE_JOB, || {
        crate::commands::perform_app_update_check(app)
    })
    .await
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
}
