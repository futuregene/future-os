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
const MAX_AGE: Duration = Duration::from_secs(2);
static GLOBAL_PENDING: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
pub(super) struct Admission {
    state: parking_lot::Mutex<State>,
}
#[derive(Default)]
struct State {
    bytes: usize,
    next: u64,
    pending: BTreeMap<u64, Instant>,
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
            "ageLimitMs":MAX_AGE.as_millis() as u64})
    }

    pub(super) fn reserve(self: &Arc<Self>, bytes: usize) -> Result<Reservation, AppendFailure> {
        let mut state = self.state.lock();
        if state.bytes.saturating_add(bytes) > SESSION_BYTES
            || state
                .pending
                .first_key_value()
                .is_some_and(|(_, time)| time.elapsed() >= MAX_AGE)
        {
            return Err(AppendFailure::Unavailable(
                "event storage backlog exceeded its byte or age limit; generation interrupted"
                    .into(),
            ));
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
