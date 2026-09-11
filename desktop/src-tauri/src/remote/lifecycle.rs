//! Host-independent cancellation and commit boundary for a remote access epoch.
use std::sync::Mutex;
use tokio::{sync::watch, task::AbortHandle};

pub(super) struct AccessEpoch {
    epoch: Mutex<u64>,
    changed: watch::Sender<u64>,
}

impl AccessEpoch {
    pub fn new() -> Self {
        Self {
            epoch: Mutex::new(0),
            changed: watch::channel(0).0,
        }
    }

    pub fn current(&self) -> u64 {
        *self.epoch.lock().unwrap()
    }

    pub fn invalidate<T>(&self, stop: impl FnOnce() -> T) -> T {
        let mut epoch = self.epoch.lock().unwrap();
        *epoch = epoch.wrapping_add(1);
        let result = stop();
        self.changed.send_replace(*epoch);
        result
    }

    /// Stop and installing an asynchronous result share this critical section.
    pub fn commit<T>(&self, expected: u64, install: impl FnOnce() -> T) -> Option<T> {
        let epoch = self.epoch.lock().unwrap();
        (*epoch == expected).then(install)
    }

    pub async fn cancelled(&self, expected: u64) {
        let mut changed = self.changed.subscribe();
        while *changed.borrow_and_update() == expected {
            if changed.changed().await.is_err() {
                return;
            }
        }
    }
}

/// Dropping a cancelled readiness future must also stop the tasks it spawned.
#[derive(Default)]
pub(super) struct CandidateTasks(Vec<AbortHandle>);

impl CandidateTasks {
    pub fn track<T>(&mut self, task: &tokio::task::JoinHandle<T>) {
        self.0.push(task.abort_handle());
    }
    pub fn installed(&mut self) {
        self.0.clear();
    }
}

impl Drop for CandidateTasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stop_invalidates_late_install_even_after_another_start() {
        let access = AccessEpoch::new();
        let old = access.current();
        access.invalidate(|| {});
        assert!(access.commit(old, || panic!("stale install")).is_none());
        assert_eq!(access.commit(access.current(), || 42), Some(42));
        access.cancelled(old).await;
    }

    #[tokio::test]
    async fn cancelled_candidate_aborts_its_subscription_tasks() {
        let task = tokio::spawn(std::future::pending::<()>());
        let mut candidate = CandidateTasks::default();
        candidate.track(&task);
        drop(candidate);
        assert!(task.await.unwrap_err().is_cancelled());
    }
}
