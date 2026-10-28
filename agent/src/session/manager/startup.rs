//! Bounded startup diagnostics shared by the embedded and standalone Agent.
use anyhow::{Context, Result};
use std::{path::Path, time::Instant};

pub(super) fn step<T>(
    database: &Path,
    step: &'static str,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let started = Instant::now();
    tracing::info!(step, database = %database.display(), "session storage step: starting");
    let result = operation();
    let elapsed_ms = started.elapsed().as_millis() as u64;
    match &result {
        Ok(_) => tracing::info!(step, elapsed_ms, "session storage step: completed"),
        Err(error) if error.is::<crate::service::StartupCancelled>() => {
            tracing::info!(step, elapsed_ms, "session storage step: cancelled");
        }
        Err(error) => {
            // Display alone keeps only anyhow's outermost context. Alternate
            // Display preserves the SQLite/IO cause without dumping payloads.
            tracing::error!(step, database = %database.display(), elapsed_ms, error = %format!("{error:#}"), "session storage step: failed");
        }
    }
    result.with_context(|| format!("session storage step {step} ({})", database.display()))
}
