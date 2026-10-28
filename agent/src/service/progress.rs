//! Startup progress stays readable before a business RPC listener exists.
use serde::Serialize;
use std::{sync::Arc, time::Instant};

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
    stage: parking_lot::Mutex<StartupStage>,
}

impl Default for Progress {
    fn default() -> Self {
        Self(Arc::new(State {
            started: Instant::now(),
            stage: parking_lot::Mutex::new(StartupStage::default()),
        }))
    }
}

impl Progress {
    pub(super) fn snapshot(&self) -> StartupProgress {
        StartupProgress {
            stage: *self.0.stage.lock(),
            elapsed_ms: self.0.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        }
    }

    pub(super) fn report(&self, stage: StartupStage) {
        *self.0.stage.lock() = stage;
        tracing::info!(?stage, "Agent startup progress");
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
}
