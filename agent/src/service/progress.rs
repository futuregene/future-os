//! Startup progress stays readable before a business RPC listener exists.
use serde::Serialize;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupStage {
    #[default]
    Preparing,
    LoadingModels,
    ReservingEndpoint,
    OpeningStorage,
    BackingUpStorage,
    UpgradingStorage,
    RecoveringSessions,
    BindingEndpoint,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupProgress {
    pub stage: StartupStage,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug)]
pub(super) struct Progress(Arc<State>);

#[derive(Debug)]
struct State {
    started: Instant,
    current: parking_lot::Mutex<(StartupStage, Instant)>,
}

impl Default for Progress {
    fn default() -> Self {
        Self(Arc::new(State {
            started: Instant::now(),
            current: parking_lot::Mutex::new((StartupStage::default(), Instant::now())),
        }))
    }
}

impl Progress {
    pub(super) fn snapshot(&self) -> StartupProgress {
        StartupProgress {
            stage: self.0.current.lock().0,
            elapsed_ms: self.0.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        }
    }

    pub(super) fn report(&self, stage: StartupStage) {
        *self.0.current.lock() = (stage, Instant::now());
        tracing::info!(?stage, "Agent startup progress");
    }

    pub(super) fn activity(&self) {
        self.0.current.lock().1 = Instant::now();
    }

    pub(super) fn idle_for(&self) -> Duration {
        self.0.current.lock().1.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_attempts_are_isolated_and_clones_observe_the_same_progress() {
        let attempt = crate::service::StartupCancellation::default();
        let observer = attempt.clone();
        attempt.report(StartupStage::BackingUpStorage);
        assert_eq!(observer.progress().stage, StartupStage::BackingUpStorage);
        assert_eq!(
            crate::service::StartupCancellation::default()
                .progress()
                .stage,
            StartupStage::Preparing
        );
        attempt.cancel();
        assert!(observer.check().is_err());
        assert_eq!(observer.progress().stage, StartupStage::BackingUpStorage);
        assert_eq!(
            serde_json::to_value(observer.progress()).unwrap()["stage"],
            "backing_up_storage"
        );
    }

    #[test]
    fn reading_status_does_not_renew_startup_activity() {
        let cancellation = crate::service::StartupCancellation::default();
        cancellation.progress.0.current.lock().1 = Instant::now() - Duration::from_secs(10);
        let _ = cancellation.progress();
        assert!(cancellation.idle_for() >= Duration::from_secs(10));
        cancellation.record_progress();
        assert!(cancellation.idle_for() < Duration::from_secs(1));
        assert_eq!(cancellation.progress().stage, StartupStage::Preparing);
    }

    #[test]
    fn sqlite_upgrade_work_can_be_cancelled_inside_a_statement() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let cancellation = crate::service::StartupCancellation::default();
        cancellation.monitor_sqlite(&db).unwrap();
        let cancel = cancellation.clone();
        db.create_scalar_function(
            "cancel_upgrade",
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| {
                cancel.cancel();
                Ok(1i64)
            },
        )
        .unwrap();
        let error = db.query_row(
            "WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<100000) SELECT sum(n+cancel_upgrade()) FROM numbers",
            [], |row| row.get::<_, i64>(0)).unwrap_err();
        assert!(
            matches!(error, rusqlite::Error::SqliteFailure(inner, _) if inner.code == rusqlite::ErrorCode::OperationInterrupted)
        );
        assert!(cancellation
            .check()
            .unwrap_err()
            .is::<crate::service::StartupCancelled>());
    }
}
