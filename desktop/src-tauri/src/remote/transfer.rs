//! NATS chunk framing only; Desktop owns file capabilities and access policy.

use futures::StreamExt;
use serde_json::json;
use std::{sync::LazyLock, time::Duration};
static TRANSFER_EPISODE: LazyLock<super::FailureEpisode> = LazyLock::new(Default::default);
/// Periodic expiry sweep cadence inside the transfer loop. Tests shrink it so
/// the sweep path runs without a one-minute wait.
fn cleanup_tick() -> Duration {
    #[cfg(test)]
    const TICK: Duration = Duration::from_millis(20);
    #[cfg(not(test))]
    const TICK: Duration = Duration::from_secs(60);
    TICK
}

#[cfg(test)]
pub fn spawn_transfer_loop(
    client: async_nats::Client,
    pair_id: String,
    active: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    spawn_transfer_loop_with_ready(client, pair_id, active, None)
}

pub fn spawn_transfer_loop_with_ready(
    client: async_nats::Client,
    pair_id: String,
    active: std::sync::Arc<std::sync::atomic::AtomicBool>,
    mut ready: Option<tokio::sync::oneshot::Sender<()>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let subject = format!("p.{pair_id}.xfer.up.>");
        let queue = format!("bridge-transfer.{pair_id}");
        let mut sub = match client.queue_subscribe(subject, queue).await {
            Ok(sub) => sub,
            Err(error) => {
                if let Some(line) = TRANSFER_EPISODE.record("transfer_subscription", error) {
                    eprintln!("{line}");
                }
                return;
            }
        };
        if let Some(line) = TRANSFER_EPISODE.recovered() {
            eprintln!("{line}");
        }
        if let Some(sender) = ready.take() {
            let _ = sender.send(());
        }
        let mut cleanup = tokio::time::interval(cleanup_tick());
        loop {
            tokio::select! {
                _ = cleanup.tick() => super::host().prune_transfers(),
                next = sub.next() => {
                    let Some(message) = next else { break };
                    if !active.load(std::sync::atomic::Ordering::Acquire) {
                        continue;
                    }
                    let suffix = message
                        .subject
                        .strip_prefix(&format!("p.{pair_id}.xfer.up."))
                        .unwrap_or_default();
                    let parts: Vec<&str> = suffix.split('.').collect();
                    let response = match parts.as_slice() {
                        [transfer_id, "chunk", index] => index
                            .parse::<u64>()
                            .map_err(|_| "Invalid chunk index.".to_string())
                            .and_then(|index| super::host().upload_chunk(transfer_id, index, &message.payload)),
                        [transfer_id, "pull", index] => match index.parse::<u64>() {
                            Ok(index) => publish_download_chunk(
                                &client,
                                &pair_id,
                                transfer_id,
                                index,
                            )
                            .await
                            .map(|_| json!({ "published": true, "index": index }))
                            .map_err(|error| error.to_string()),
                            Err(_) => Err("Invalid chunk index.".to_string()),
                        },
                        _ => Err("Unsupported transfer operation.".to_string()),
                    };
                    if let Some(reply) = message.reply {
                        let body = match response {
                            Ok(data) => json!({ "success": true, "data": data }),
                            Err(error) => json!({ "success": false, "error": error }),
                        };
                        // A Value always serializes.
                        let bytes = serde_json::to_vec(&body)
                            .expect("a transfer reply Value always serializes");
                        let _ = client.publish(reply, bytes.into()).await;
                    }
                }
            }
        }
        if let Some(line) =
            TRANSFER_EPISODE.record("transfer_subscription", "subscription ended unexpectedly")
        {
            eprintln!("{line}");
        }
    })
}

async fn publish_download_chunk(
    client: &async_nats::Client,
    pair_id: &str,
    transfer_id: &str,
    index: u64,
) -> Result<(), crate::AppError> {
    let bytes = super::host().download_chunk(transfer_id, index)?;
    let subject = format!("p.{pair_id}.xfer.down.{transfer_id}.chunk.{index}");
    client
        .publish(subject, bytes.into())
        .await
        .map_err(|error| format!("Download chunk publish failed: {error}"))?;
    client
        .flush()
        .await
        .map_err(|error| format!("Download chunk flush failed: {error}"))?;
    Ok(())
}

pub(crate) fn clear_transfers() {
    super::host().clear_transfers();
}
pub(crate) fn clear_preview_cache() {
    super::host().clear_preview_cache();
}
