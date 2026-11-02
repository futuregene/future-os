//! NATS file framing. The host retains file capabilities and access policy.
use crate::{diagnostics::FailureEpisode, secure::Transport, services::FileHost};
use futures::{future::BoxFuture, stream::FuturesUnordered, StreamExt};
use serde_json::json;
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

/// Indexed downloads are independent reads. Four matches the clients' transfer
/// window without allowing one peer to create an unbounded blocking-read queue.
const DOWNLOAD_CONCURRENCY: usize = 4;

/// A chunk index that is not a number cannot address a chunk at all. Uploads
/// and pulls are malformed in the same way, so they report the same message;
/// clients match on it.
const INVALID_CHUNK_INDEX: &str = "Invalid chunk index.";

type DownloadResult = (
    Option<async_nats::Subject>,
    crate::secure::Reply,
    Result<serde_json::Value, String>,
);

/// One generation's transfer subscription. The owner polls/cancels this future;
/// the endpoint does not detach work or own process shutdown signals.
pub struct TransferLoop {
    host: Arc<dyn FileHost>,
    security: Transport,
    active: Arc<AtomicBool>,
    cleanup_interval: Duration,
    failures: FailureEpisode,
}
impl TransferLoop {
    /// `cleanup_interval` must be nonzero, as required by Tokio's interval.
    pub fn new(
        host: Arc<dyn FileHost>,
        security: Transport,
        active: Arc<AtomicBool>,
        cleanup_interval: Duration,
    ) -> Self {
        Self {
            host,
            security,
            active,
            cleanup_interval,
            failures: FailureEpisode::default(),
        }
    }
    pub async fn run(
        self,
        client: async_nats::Client,
        pair_id: String,
        mut ready: Option<tokio::sync::oneshot::Sender<()>>,
    ) {
        let subject = format!("p.{pair_id}.xfer.up.>");
        let queue = format!("bridge-transfer.{pair_id}");
        let mut sub = match client.queue_subscribe(subject, queue).await {
            Ok(sub) => sub,
            Err(error) => {
                if let Some(line) = self.failures.record("transfer_subscription", error) {
                    future_remote_protocol::remote_log!("{line}");
                }
                return;
            }
        };
        if let Some(line) = self.failures.recovered() {
            future_remote_protocol::remote_log!("{line}");
        }
        if let Some(sender) = ready.take() {
            let _ = sender.send(());
        }
        let mut cleanup = tokio::time::interval(self.cleanup_interval);
        let mut downloads: FuturesUnordered<BoxFuture<'static, DownloadResult>> =
            FuturesUnordered::new();
        loop {
            // Apply backpressure before accepting more messages. Upload writes
            // stay ordered on the receiving loop; only independent indexed reads
            // and their publishes overlap.
            if downloads.len() >= DOWNLOAD_CONCURRENCY {
                if let Some((reply, security, response)) = downloads.next().await {
                    reply_to(&client, reply, &security, response).await;
                }
            }
            tokio::select! {
                Some((reply, security, response)) = downloads.next(), if !downloads.is_empty() => {
                    reply_to(&client, reply, &security, response).await;
                },
                _ = cleanup.tick() => self.host.prune_transfers(),
                next = sub.next() => {
                    let Some(message) = next else { break };
                    if !self.active.load(std::sync::atomic::Ordering::Acquire) {
                        continue;
                    }
                    let Ok((plaintext, reply_security)) = self.security.open(&message.subject, &message.payload) else { continue; };
                    if !self.security.is_active(&reply_security) { continue; }
                    let suffix = message
                        .subject
                        .strip_prefix(&format!("p.{pair_id}.xfer.up."))
                        .unwrap_or_default();
                    let parts: Vec<&str> = suffix.split('.').collect();
                    if let [transfer_id, "pull", index] = parts.as_slice() {
                        if let Ok(index) = index.parse::<u64>() {
                            let client = client.clone();
                            let security = self.security.clone();
                            let host = self.host.clone();
                            let pair_id = pair_id.clone();
                            let transfer_id = transfer_id.to_string();
                            let reply = message.reply;
                            downloads.push(Box::pin(async move {
                                let result = publish_download_chunk(
                                    &client, &security, host.clone(), &pair_id, &transfer_id, index,
                                )
                                .await
                                .map(|_| json!({ "published": true, "index": index }))
                                .map_err(|error| error.to_string());
                                (reply, reply_security, result)
                            }));
                            continue;
                        }
                    }
                    let response = match parts.as_slice() {
                        [transfer_id, "chunk", index] => index
                            .parse::<u64>()
                            .map_err(|_| INVALID_CHUNK_INDEX.to_string())
                            .and_then(|index| self.host.upload_chunk(transfer_id, index, &plaintext)),
                        // Only numeric pulls are dispatched above; a non-numeric one
                        // falls through to here, exactly like an upload's bad index.
                        [_, "pull", _] => Err(INVALID_CHUNK_INDEX.to_string()),
                        _ => Err("Unsupported transfer operation.".to_string()),
                    };
                    reply_to(&client, message.reply, &reply_security, response).await;
                }
            }
        }
        while let Some((reply, security, response)) = downloads.next().await {
            reply_to(&client, reply, &security, response).await;
        }
        if let Some(line) = self
            .failures
            .record("transfer_subscription", "subscription ended unexpectedly")
        {
            future_remote_protocol::remote_log!("{line}");
        }
    }
}

async fn reply_to(
    client: &async_nats::Client,
    reply: Option<async_nats::Subject>,
    security: &crate::secure::Reply,
    response: Result<serde_json::Value, String>,
) {
    let Some(reply) = reply else { return };
    let body = match response {
        Ok(data) => json!({ "success": true, "data": data }),
        Err(error) => json!({ "success": false, "error": error }),
    };
    // A Value always serializes.
    let bytes = serde_json::to_vec(&body).expect("a transfer reply Value always serializes");
    if let Ok(wire) = security.seal(&bytes) {
        let _ = client.publish(reply, wire.into()).await;
    }
}

async fn publish_download_chunk(
    client: &async_nats::Client,
    security: &crate::secure::Transport,
    host: Arc<dyn crate::services::FileHost>,
    pair_id: &str,
    transfer_id: &str,
    index: u64,
) -> Result<(), crate::Error> {
    let id = transfer_id.to_string();
    // Native file IO belongs off the async executor. The four-future bound above
    // also bounds these jobs, including callers that pipeline many chunks.
    let bytes = tokio::task::spawn_blocking(move || host.download_chunk(&id, index))
        .await
        .map_err(|error| crate::Error::Message(format!("Download chunk read failed: {error}")))??;
    let subject = format!("p.{pair_id}.xfer.down.{transfer_id}.chunk.{index}");
    crate::secure::publish(client, security, subject, bytes).await?;
    client
        .flush()
        .await
        .map_err(|error| format!("Download chunk flush failed: {error}"))?;
    Ok(())
}
