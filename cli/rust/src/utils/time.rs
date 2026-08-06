//! Time helpers — port of `cli/src/utils/time.ts`.

/// `sleep(ms)` — resolve after `ms` milliseconds.
pub async fn sleep(ms: u64) {
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
}
