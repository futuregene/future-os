//! Request-scoped read cancellation. Writes retain their durable completion
//! semantics; cancelling a caller never discards an admitted mutation.
use anyhow::{bail, Result};
use rusqlite::Connection;
use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

thread_local! {
    static CURRENT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

#[derive(Clone)]
pub(crate) struct Context {
    pub command: String,
    pub session: String,
    deadline: Option<Instant>,
    cancelled: Arc<AtomicBool>,
}
impl Context {
    pub(crate) fn new(command: String, session: String, timeout: Option<Duration>) -> Self {
        Self {
            command,
            session,
            deadline: timeout.map(|timeout| Instant::now() + timeout),
            cancelled: Default::default(),
        }
    }
    pub(crate) fn current() -> Option<Self> {
        CURRENT.with(|current| current.borrow().clone())
    }
    pub(crate) fn enter(self) -> Scope {
        Scope(CURRENT.with(|current| current.replace(Some(self))))
    }
    pub(crate) fn cancel_on_drop(&self) -> CancelOnDrop {
        CancelOnDrop(self.clone())
    }
    fn expired(&self) -> bool {
        self.deadline.is_some_and(|deadline| {
            Instant::now() >= deadline || self.cancelled.load(Ordering::Acquire)
        })
    }
    pub(crate) fn check(&self) -> Result<()> {
        if self.expired() {
            bail!("storage read cancelled or deadline exceeded");
        }
        Ok(())
    }
    pub(crate) fn run<T>(
        &self,
        db: &mut Connection,
        operation: impl FnOnce(&mut Connection) -> Result<T>,
    ) -> Result<T> {
        self.check()?;
        if self.deadline.is_none() {
            return operation(db);
        }
        let context = self.clone();
        db.progress_handler(1_000, Some(move || context.expired()))?;
        let result = operation(db);
        let cleared = db.progress_handler(0, None::<fn() -> bool>);
        cleared?;
        if self.expired() {
            if !db.is_autocommit() {
                db.execute_batch("ROLLBACK")?;
            }
            self.check()?;
        }
        result
    }
    pub(crate) fn durable(mut self) -> Self {
        self.deadline = None;
        self
    }
    pub(crate) fn bounded(mut self, deadline: Instant) -> Self {
        // A mutation may use a leased read snapshot while preparing durable
        // work. Enforce the lease lifetime without adopting caller cancellation.
        if self.deadline.is_none() {
            self.cancelled = Default::default();
        }
        self.deadline = Some(
            self.deadline
                .map_or(deadline, |current| current.min(deadline)),
        );
        self
    }
}
pub(crate) struct Scope(Option<Context>);
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|current| {
            current.replace(self.0.take());
        });
    }
}
pub(crate) struct CancelOnDrop(Context);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancelled.store(true, Ordering::Release);
    }
}

/// gRPC timeout grammar: at most eight decimal digits followed by one unit.
pub(crate) fn timeout(value: &str) -> Option<Duration> {
    let (amount, unit) = value.split_at_checked(value.len().checked_sub(1)?)?;
    if amount.is_empty() || amount.len() > 8 || !amount.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let amount = amount.parse::<u64>().ok()?;
    Some(match unit {
        "H" => Duration::from_secs(amount.checked_mul(3600)?),
        "M" => Duration::from_secs(amount.checked_mul(60)?),
        "S" => Duration::from_secs(amount),
        "m" => Duration::from_millis(amount),
        "u" => Duration::from_micros(amount),
        "n" => Duration::from_nanos(amount),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_lifetime_does_not_turn_mutation_cancellation_into_read_cancellation() {
        let mutation = Context::new("new_session".into(), "s".into(), None);
        let cancelled = mutation.cancel_on_drop();
        drop(cancelled);
        assert!(mutation
            .bounded(Instant::now() + Duration::from_secs(10))
            .check()
            .is_ok());
        let read = Context::new(
            "get_session_entries".into(),
            "s".into(),
            Some(Duration::from_secs(30)),
        );
        let cancelled = read.cancel_on_drop();
        drop(cancelled);
        assert!(read
            .bounded(Instant::now() + Duration::from_secs(10))
            .check()
            .is_err());
    }

    #[test]
    fn grpc_timeout_keeps_units_and_rejects_invalid_metadata() {
        assert_eq!(timeout("50m"), Some(Duration::from_millis(50)));
        assert_eq!(timeout("2M"), Some(Duration::from_secs(120)));
        assert_eq!(timeout("3H"), Some(Duration::from_secs(10800)));
        assert_eq!(timeout("4u"), Some(Duration::from_micros(4)));
        assert_eq!(timeout("5n"), Some(Duration::from_nanos(5)));
        for invalid in ["", "S", "123456789S", "-1S", "1x", "ü"] {
            assert_eq!(timeout(invalid), None);
        }
    }
}
