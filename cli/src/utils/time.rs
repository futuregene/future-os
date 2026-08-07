//! Time helpers — port of `cli/src/utils/time.ts`.

/// `sleep(ms)` — resolve after `ms` milliseconds.
pub async fn sleep(ms: u64) {
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
}

/// Wall-clock milliseconds — `Date.now()`.
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
