//! future-remote — Desktop Bridge (minimal skeleton / L0).
//!
//! Purpose: bridge the local agent (gRPC :50051) to NATS.
//!  - Subscribe to commands `p.{pairId}.cmd.>` (queue group `bridge.{pairId}`) → forward to agent → reply.
//!  - For each session, maintain a long-lived subscription to the agent event stream → publish to `p.{pairId}.evt.{session}`.
//!
//! This version is a skeleton: no run_id/idx, solo-flight, JetStream ack, or auth yet (see docs/remote-control-*.md P1/P5).

pub mod proto {
    tonic::include_proto!("proto");
}
mod agent_client;
mod config;

use agent_client::AgentClient;
use anyhow::Result;
use config::Config;
use futures::StreamExt;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Command sent by the client via NATS (camelCase JSON, only the fields the bridge needs).
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct IncomingCmd {
    id: String,
    #[serde(rename = "type")]
    cmd_type: String,
    session_id: String,
    message: String,
    mode: String,
    entry_id: String,
    model_id: String,
    level: String,
    cwd: String,
    streaming_behavior: String,
}

type Pumped = Arc<Mutex<HashSet<String>>>;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = Config::from_env();
    tracing::info!(
        "future-remote starting: nats={} agent={} pairId={}",
        cfg.nats_url,
        cfg.agent_addr,
        cfg.pair_id
    );

    let nats = async_nats::connect(&cfg.nats_url).await?;
    let agent = AgentClient::connect(&cfg.agent_addr).await?;
    tracing::info!("Connected to NATS + agent");

    let pumped: Pumped = Arc::new(Mutex::new(HashSet::new()));
    let cmd_subject = format!("p.{}.cmd.>", cfg.pair_id);
    let queue = format!("bridge.{}", cfg.pair_id);
    let mut sub = nats.queue_subscribe(cmd_subject.clone(), queue).await?;
    tracing::info!("Subscribing to commands: {}", cmd_subject);

    while let Some(msg) = sub.next().await {
        let nats = nats.clone();
        let mut agent = agent.clone();
        let pumped = pumped.clone();
        let pair = cfg.pair_id.clone();
        // Spawn per command: prevent a slow command from HOL-blocking other sessions
        tokio::spawn(async move {
            if let Err(e) = handle_cmd(&nats, &mut agent, &pumped, &pair, msg).await {
                tracing::warn!("Command processing failed: {}", e);
            }
        });
    }
    Ok(())
}

async fn handle_cmd(
    nats: &async_nats::Client,
    agent: &mut AgentClient,
    pumped: &Pumped,
    pair: &str,
    msg: async_nats::Message,
) -> Result<()> {
    let c: IncomingCmd = serde_json::from_slice(&msg.payload)
        .map_err(|e| anyhow::anyhow!("Failed to parse command JSON: {}", e))?;

    // Streaming commands: ensure the event pump is ready first (subscribe before command) to avoid missing early agent_start/text_chunk events.
    if matches!(c.cmd_type.as_str(), "prompt" | "steer" | "follow_up") && !c.session_id.is_empty() {
        ensure_pump(nats, agent, pumped, pair, &c.session_id).await?;
    }

    let pcmd = proto::RpcCommand {
        id: if c.id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            c.id.clone()
        },
        r#type: c.cmd_type.clone(),
        session_id: c.session_id.clone(),
        message: c.message,
        mode: c.mode,
        entry_id: c.entry_id,
        model_id: c.model_id,
        level: c.level,
        cwd: c.cwd,
        streaming_behavior: c.streaming_behavior,
        ..Default::default()
    };

    let resp = agent.execute(pcmd).await?;

    if let Some(reply) = msg.reply {
        let data: serde_json::Value = if resp.data.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str(&resp.data).unwrap_or(serde_json::Value::Null)
        };
        let body = serde_json::json!({
            "type": "response",
            "id": resp.id,
            "command": resp.command,
            "success": resp.success,
            "data": data,
            "error": if resp.error.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(resp.error) },
        });
        nats.publish(reply, serde_json::to_vec(&body)?.into())
            .await?;
        nats.flush().await.ok(); // Ensure timely reply delivery (request-reply)
    }
    Ok(())
}

/// Start a long-lived event pump for a session: subscribe to agent StreamEvents → publish to NATS.
async fn ensure_pump(
    nats: &async_nats::Client,
    agent: &AgentClient,
    pumped: &Pumped,
    pair: &str,
    session: &str,
) -> Result<()> {
    {
        let mut set = pumped.lock().await;
        if set.contains(session) {
            return Ok(());
        }
        set.insert(session.to_string());
    }

    let mut ac = agent.clone();
    let mut stream = ac.stream_events(session).await?; // Subscription established here (before command execution)

    let nats = nats.clone();
    let subj = format!("p.{}.evt.{}", pair, session);
    let pumped = pumped.clone();
    let session_owned = session.to_string();

    tokio::spawn(async move {
        tracing::info!("Event pump started: session={}", session_owned);
        loop {
            match stream.message().await {
                Ok(Some(ev)) => {
                    // Skeleton: forward {type, data} (data is still a JSON string). P1 will add run_id/idx.
                    let body = serde_json::json!({ "type": ev.r#type, "data": ev.data, "runId": ev.run_id, "idx": ev.idx });
                    if let Ok(bytes) = serde_json::to_vec(&body) {
                        if let Err(e) = nats.publish(subj.clone(), bytes.into()).await {
                            tracing::warn!("Failed to publish event: {}", e);
                        }
                    }
                }
                Ok(None) => {
                    tracing::info!("agent event stream ended: session={}", session_owned);
                    break;
                }
                Err(e) => {
                    tracing::warn!("agent event stream error session={}: {}", session_owned, e);
                    break;
                }
            }
        }
        // Allow the next command to restart the pump
        pumped.lock().await.remove(&session_owned);
    });

    Ok(())
}
