//! A cheap bounded RPC probe. It observes the existing Agent and never starts one.
use std::sync::Mutex;
use std::time::{Duration, Instant};
#[derive(Default)]
struct Availability(Mutex<Option<(bool, Instant)>>);
impl Availability {
    fn observe(&self, available: bool, now: Instant) {
        *self.0.lock().unwrap() = Some((available, now));
    }
    fn available(&self, now: Instant) -> bool {
        self.0.lock().unwrap().is_some_and(|(available, checked)| {
            available && now.saturating_duration_since(checked) <= Duration::from_secs(10)
        })
    }
}
static HEALTH: Availability = Availability(Mutex::new(None));
pub(crate) fn available() -> bool {
    HEALTH.available(Instant::now())
}
pub(crate) async fn monitor() {
    let mut previous = None;
    let mut last_warning: Option<Instant> = None;
    loop {
        let result = tokio::time::timeout(Duration::from_secs(3), async {
            let mut client = crate::agent_bridge::connect_agent().await?;
            use crate::agent_bridge::RpcResponseExt;
            client
                .execute_command(crate::agent_bridge::list_streaming_sessions_command())
                .await
                .map_err(|e| crate::AppError::AgentUnavailable(e.to_string()))?
                .into_inner()
                .ok_or_rpc_error("Agent health check failed")?;
            Ok::<_, crate::AppError>(())
        })
        .await;
        let available = matches!(result, Ok(Ok(())));
        let now = Instant::now();
        if previous != Some(available) {
            if available {
                eprintln!("remote: Agent availability restored [LC003]");
            } else if last_warning
                .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(300))
            {
                eprintln!("remote: Agent availability probe failed [LC003]: {result:?}");
                last_warning = Some(now);
            }
            previous = Some(available);
        }
        HEALTH.observe(available, now);
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn availability_requires_a_recent_success_and_recovers_after_failure() {
        let health = Availability::default();
        let now = Instant::now();
        assert!(!health.available(now));
        health.observe(true, now);
        assert!(health.available(now + Duration::from_secs(9)));
        assert!(!health.available(now + Duration::from_secs(11)));
        health.observe(false, now);
        assert!(!health.available(now));
        health.observe(true, now);
        assert!(health.available(now));
    }
}
