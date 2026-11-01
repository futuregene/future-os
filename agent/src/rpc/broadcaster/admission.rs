//! Memory admission stays independent of SQLite. Reservations live until the
//! corresponding commit, including retries, rather than just until dequeue.
use super::journal_writer::AppendFailure;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

const SESSION_BYTES: usize = 32 * 1024 * 1024;
const GLOBAL_BYTES: usize = 128 * 1024 * 1024;
/// How long an admitted event may stay uncommitted before the writer is
/// declared stuck.
///
/// This is a liveness bound, not a durability promise: memory is already
/// bounded by the byte budgets above, so the age only decides when to stop
/// producing into a store that is not draining. It therefore has to sit well
/// above the *observed* commit tail, which is a batch window (25 ms) plus a
/// WAL+FULL commit plus whatever committed ahead of it on the one shared
/// writer. Measured on a working desktop under several concurrent runs:
/// `commit` reached 1054–1198 ms and `wal_checkpoint` 1090–2900 ms
/// (2026-10-31, `~/.future/app/logs/desktop.log`). A 2 s bound sat inside that
/// distribution and turned an ordinary slow commit into a killed run; 30 s is
/// ~10× the worst observed commit and still catches a wedged writer while the
/// user is watching the run.
pub(super) const MAX_AGE: Duration = Duration::from_secs(30);
static GLOBAL_PENDING: AtomicUsize = AtomicUsize::new(0);

pub(super) struct Admission {
    state: parking_lot::Mutex<State>,
}
struct State {
    bytes: usize,
    next: u64,
    pending: BTreeMap<u64, Instant>,
    age_limit: Duration,
}
impl Default for State {
    fn default() -> Self {
        Self {
            bytes: 0,
            next: 0,
            pending: BTreeMap::new(),
            age_limit: MAX_AGE,
        }
    }
}
impl Default for Admission {
    fn default() -> Self {
        Self {
            state: parking_lot::Mutex::new(State::default()),
        }
    }
}

pub(super) struct Reservation {
    admission: Arc<Admission>,
    id: u64,
    bytes: usize,
}
impl Admission {
    pub(super) fn snapshot(&self) -> serde_json::Value {
        let state = self.state.lock();
        serde_json::json!({"pendingBytes":state.bytes,"pendingEvents":state.pending.len(),
            "oldestPendingMicros":state.pending.first_key_value().map(|(_,time)|time.elapsed().as_micros().min(u64::MAX as u128) as u64).unwrap_or(0),
            "globalPendingBytes":GLOBAL_PENDING.load(Ordering::Acquire),
            "sessionByteLimit":SESSION_BYTES,"globalByteLimit":GLOBAL_BYTES,
            "ageLimitMs":state.age_limit.as_millis() as u64})
    }

    /// Test-only: shrink the age bound so a test can reach it by parking the
    /// writer for milliseconds instead of the production 30 s.
    #[cfg(test)]
    pub(super) fn set_age_limit_for_test(&self, limit: Duration) {
        self.state.lock().age_limit = limit;
    }

    pub(super) fn reserve(self: &Arc<Self>, bytes: usize) -> Result<Reservation, AppendFailure> {
        let mut state = self.state.lock();
        if state.bytes.saturating_add(bytes) > SESSION_BYTES
            || state
                .pending
                .first_key_value()
                .is_some_and(|(_, time)| time.elapsed() >= state.age_limit)
        {
            // Name which bound fired and by how much: "byte or age" was all an
            // operator got from the 2026-10-31 incident, and the two have very
            // different causes (one huge session vs. a stalled shared writer).
            let oldest = state
                .pending
                .first_key_value()
                .map(|(_, time)| time.elapsed())
                .unwrap_or_default();
            return Err(AppendFailure::Unavailable(format!(
                "event storage backlog exceeded its byte or age limit; generation interrupted \
                 (oldest pending {}ms of {}ms, {} of {} bytes, {} events)",
                oldest.as_millis(),
                state.age_limit.as_millis(),
                state.bytes.saturating_add(bytes),
                SESSION_BYTES,
                state.pending.len()
            )));
        }
        GLOBAL_PENDING
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                n.checked_add(bytes).filter(|n| *n <= GLOBAL_BYTES)
            })
            .map_err(|_| {
                AppendFailure::Unavailable(
                    "global event storage backlog exceeded its byte limit; generation interrupted"
                        .into(),
                )
            })?;
        let id = state.next;
        state.next = state.next.wrapping_add(1);
        state.bytes += bytes;
        state.pending.insert(id, Instant::now());
        Ok(Reservation {
            admission: self.clone(),
            id,
            bytes,
        })
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut state = self.admission.state.lock();
        state.bytes -= self.bytes;
        state.pending.remove(&self.id);
        GLOBAL_PENDING.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
