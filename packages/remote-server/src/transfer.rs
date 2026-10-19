//! NATS file framing. The host retains file capabilities and access policy.
use crate::{diagnostics::FailureEpisode, secure::Transport, services::FileHost};
use futures::StreamExt;
use serde_json::json;
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

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
                    eprintln!("{line}");
                }
                return;
            }
        };
        if let Some(line) = self.failures.recovered() {
            eprintln!("{line}");
        }
        if let Some(sender) = ready.take() {
            let _ = sender.send(());
        }
        let mut cleanup = tokio::time::interval(self.cleanup_interval);
        loop {
            tokio::select! {
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
                    let response = match parts.as_slice() {
                        [transfer_id, "chunk", index] => index
                            .parse::<u64>()
                            .map_err(|_| "Invalid chunk index.".to_string())
                            .and_then(|index| self.host.upload_chunk(transfer_id, index, &plaintext)),
                        [transfer_id, "pull", index] => match index.parse::<u64>() {
                            Ok(index) => publish_download_chunk(
                                &client,
                                &self.security,
                                self.host.as_ref(),
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
                        if let Ok(wire) = reply_security.seal(&bytes) {
                            let _ = client.publish(reply, wire.into()).await;
                        }
                    }
                }
            }
        }
        if let Some(line) = self
            .failures
            .record("transfer_subscription", "subscription ended unexpectedly")
        {
            eprintln!("{line}");
        }
    }
}

async fn publish_download_chunk(
    client: &async_nats::Client,
    security: &crate::secure::Transport,
    host: &dyn crate::services::FileHost,
    pair_id: &str,
    transfer_id: &str,
    index: u64,
) -> Result<(), crate::Error> {
    let bytes = host.download_chunk(transfer_id, index)?;
    let subject = format!("p.{pair_id}.xfer.down.{transfer_id}.chunk.{index}");
    crate::secure::publish(client, security, subject, bytes).await?;
    client
        .flush()
        .await
        .map_err(|error| format!("Download chunk flush failed: {error}"))?;
    Ok(())
}
