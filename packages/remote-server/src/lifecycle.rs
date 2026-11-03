//! Host-independent cancellation and commit boundary for a remote access epoch.
use std::sync::Mutex;
use tokio::{sync::watch, task::AbortHandle};

/// Test-support record of who moved an [`AccessEpoch`].
///
/// A start that answers with an *empty* status (`Stopped`, no pair id, no
/// invitation) is produced by four different early returns, and the status
/// cannot tell them apart — which is why a failing fixture only ever reported
/// "the host bridge that started must show an invitation". Every one of those
/// returns is a reaction to *someone else* moving the epoch or the flags the
/// epoch protects, so the useful evidence is who moved them and when. This
/// journal is that: it costs nothing outside `test-support`, and it is what
/// turns "the suite fails this one test 4 runs in 10" into a named caller.
#[cfg(feature = "test-support")]
pub mod journal {
    use std::collections::VecDeque;
    use std::sync::{Mutex, OnceLock};

    /// Enough for every epoch event in a suite run plus the notes around it: a
    /// full `future-host --lib` run records a few hundred.
    const KEEP: usize = 256;
    /// Where a dump stops listing. The failure is always near the end, and a
    /// panic message is read by a person.
    const SHOWN: usize = 40;

    #[derive(Debug)]
    pub struct Entry {
        pub what: &'static str,
        pub epoch: u64,
        /// The thread that made the write. This is the discriminator the status
        /// itself lacks: a test thread is named after the test, so a write from
        /// `remote::control::tests::…` names the offender, while a write from a
        /// runtime worker names the *class* — a task that outlived its test.
        /// (No backtrace: capturing one here aborted a `remote_peer::` run with
        /// "has overflowed its stack" on a 2 MB tokio worker.)
        pub thread: String,
    }

    static JOURNAL: OnceLock<Mutex<VecDeque<Entry>>> = OnceLock::new();

    fn journal() -> &'static Mutex<VecDeque<Entry>> {
        JOURNAL.get_or_init(|| Mutex::new(VecDeque::new()))
    }

    pub fn record(what: &'static str, epoch: u64) {
        let mut entries = journal()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if entries.len() >= KEEP {
            entries.pop_front();
        }
        entries.push_back(Entry {
            what,
            epoch,
            thread: std::thread::current()
                .name()
                .unwrap_or("<unnamed thread>")
                .to_string(),
        });
    }

    pub fn clear() {
        journal()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clear();
    }

    /// The last [`SHOWN`] events, oldest first.
    pub fn dump() -> String {
        let entries = journal()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let skip = entries.len().saturating_sub(SHOWN);
        let mut out = String::new();
        for entry in entries.iter().skip(skip) {
            out.push_str(&format!(
                "  [epoch {}] {} — {}\n",
                entry.epoch, entry.thread, entry.what
            ));
        }
        out
    }
}

pub struct AccessEpoch {
    epoch: Mutex<u64>,
    changed: watch::Sender<u64>,
}

impl Default for AccessEpoch {
    fn default() -> Self {
        Self::new()
    }
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

    /// A non-locking observation for callbacks invoked inside `commit`.
    /// It does not replace the exclusion boundary used by admission/install.
    pub(crate) fn published(&self) -> u64 {
        *self.changed.borrow()
    }

    pub fn invalidate<T>(&self, stop: impl FnOnce() -> T) -> T {
        let mut epoch = self.epoch.lock().unwrap();
        *epoch = epoch.wrapping_add(1);
        #[cfg(feature = "test-support")]
        journal::record("invalidate", *epoch);
        let result = stop();
        self.changed.send_replace(*epoch);
        result
    }

    /// Test-support: name an event that a later failure can be read against.
    ///
    /// A no-op outside test builds, so a call site costs one argument and no
    /// `cfg` of its own. `&'static str`, never `format!`: these notes run on the
    /// same chains as `start`, and a `format!` is a dozen debug-build frames —
    /// enough to tip a 2 MB runtime worker into "has overflowed its stack"
    /// (measured on `remote_peer::runtime_tests`, whose fixture chain already
    /// sits at the limit).
    pub fn note(&self, what: &'static str) {
        #[cfg(feature = "test-support")]
        {
            let epoch = self.published();
            journal::record(what, epoch);
        }
        #[cfg(not(feature = "test-support"))]
        {
            let _ = what;
        }
    }

    /// Stop and installing an asynchronous result share this critical section.
    pub fn commit<T>(&self, expected: u64, install: impl FnOnce() -> T) -> Option<T> {
        let epoch = self.epoch.lock().unwrap();
        let admitted = (*epoch == expected).then(install);
        #[cfg(feature = "test-support")]
        if admitted.is_none() {
            journal::record("commit refused", *epoch);
        }
        admitted
    }

    pub async fn cancelled(&self, expected: u64) {
        let mut changed = self.changed.subscribe();
        while *changed.borrow_and_update() == expected {
            if changed.changed().await.is_err() {
                return;
            }
        }
        #[cfg(feature = "test-support")]
        journal::record("cancelled", *changed.borrow());
    }
}

/// Dropping a cancelled readiness future must also stop the tasks it spawned.
#[derive(Default)]
pub struct CandidateTasks {
    joined: Vec<AbortHandle>,
    owned: Vec<crate::tasks::TaskHandle>,
}

impl CandidateTasks {
    pub fn track<T>(&mut self, task: &tokio::task::JoinHandle<T>) {
        self.joined.push(task.abort_handle());
    }
    pub fn track_owned(&mut self, task: &crate::tasks::TaskHandle) {
        self.owned.push(task.clone());
    }
    pub fn installed(&mut self) {
        self.joined.clear();
        self.owned.clear();
    }
}

impl Drop for CandidateTasks {
    fn drop(&mut self) {
        for task in &self.joined {
            task.abort();
        }
        for task in &self.owned {
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
