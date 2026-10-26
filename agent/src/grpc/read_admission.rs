//! Async admission for expensive read commands, separate from control and live
//! event delivery. Waiting requests do not consume blocking-pool threads. The
//! running permit follows the blocking job if the client cancels its future.
use std::sync::{Arc, LazyLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

struct Lane {
    waiting: Arc<Semaphore>,
    running: Arc<Semaphore>,
}
impl Lane {
    fn new(running: usize) -> Self {
        Self {
            waiting: Arc::new(Semaphore::new(32)),
            running: Arc::new(Semaphore::new(running)),
        }
    }
    async fn enter(&self) -> Result<OwnedSemaphorePermit, Box<tonic::Status>> {
        let waiting = self.waiting.clone().try_acquire_owned().map_err(|_| {
            tonic::Status::resource_exhausted("history read queue is full; retry this read")
        })?;
        let running = self
            .running
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| tonic::Status::unavailable("history reads are closed"))?;
        drop(waiting);
        Ok(running)
    }
}
static HEAVY: LazyLock<Lane> = LazyLock::new(|| Lane::new(2));
static PAGES: LazyLock<Lane> = LazyLock::new(|| Lane::new(4));
static REPLAY: LazyLock<Lane> = LazyLock::new(|| Lane::new(2));

pub(super) async fn command(
    command: &str,
) -> Result<Option<OwnedSemaphorePermit>, Box<tonic::Status>> {
    let lane = match command {
        "list_sessions"
        | "search_session_history"
        | "search_all_session_history"
        | "get_messages" => Some(&*HEAVY),
        "get_session_entries"
        | "get_session_history_entry"
        | "list_tool_calls"
        | "get_tool_output"
        | "get_tool_call_args"
        | "get_run_snapshot"
        | "get_events_since"
        | "get_session_events_since"
        | "get_event_content_range" => Some(&*PAGES),
        _ => None,
    };
    match lane {
        Some(lane) => Ok(Some(lane.enter().await?)),
        None => Ok(None),
    }
}

pub(super) async fn replay() -> Result<OwnedSemaphorePermit, Box<tonic::Status>> {
    REPLAY.enter().await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancelling_a_waiter_or_running_job_does_not_lose_the_admission_permit() {
        let lane = Arc::new(Lane::new(1));
        let active = lane.enter().await.unwrap();
        let waiter = {
            let lane = lane.clone();
            tokio::spawn(async move { lane.enter().await })
        };
        tokio::task::yield_now().await;
        waiter.abort();
        let _ = waiter.await;
        assert_eq!(lane.running.available_permits(), 0);
        assert_eq!(lane.waiting.available_permits(), 32);
        let (entered, entered_rx) = tokio::sync::oneshot::channel();
        let (release, release_rx) = std::sync::mpsc::sync_channel(1);
        let job = tokio::task::spawn_blocking(move || {
            let _active = active;
            entered.send(()).unwrap();
            release_rx.recv().unwrap();
        });
        entered_rx.await.unwrap();
        job.abort();
        assert_eq!(
            lane.running.available_permits(),
            0,
            "blocking work must retain ownership after cancellation"
        );
        release.send(()).unwrap();
        let _ = job.await;
        let permit = lane.enter().await.unwrap();
        drop(permit);
        assert_eq!(lane.running.available_permits(), 1);
    }
}
