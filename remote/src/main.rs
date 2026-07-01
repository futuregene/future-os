//! future-remote —— 桌面 Bridge（最小骨架 / L0）。
//!
//! 职责：把本机 agent（gRPC :50051）桥接到 NATS。
//!  - 订阅命令 `p.{pairId}.cmd.>`（queue group `bridge.{pairId}`）→ 透传给 agent → reply。
//!  - 对每个 session 长期订阅 agent 事件流 → 发布到 `p.{pairId}.evt.{session}`。
//!
//! 本版是骨架：先不做 run_id/idx、单飞、JetStream ack、鉴权（见 docs/remote-control-*.md 的 P1/P5）。

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

/// 客户端经 NATS 发来的命令（camelCase JSON，取 Bridge 需要的字段）。
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
        "future-remote 启动：nats={} agent={} pairId={}",
        cfg.nats_url,
        cfg.agent_addr,
        cfg.pair_id
    );

    let nats = async_nats::connect(&cfg.nats_url).await?;
    let agent = AgentClient::connect(&cfg.agent_addr).await?;
    tracing::info!("已连接 NATS + agent");

    let pumped: Pumped = Arc::new(Mutex::new(HashSet::new()));
    let cmd_subject = format!("p.{}.cmd.>", cfg.pair_id);
    let queue = format!("bridge.{}", cfg.pair_id);
    let mut sub = nats.queue_subscribe(cmd_subject.clone(), queue).await?;
    tracing::info!("订阅命令：{}", cmd_subject);

    while let Some(msg) = sub.next().await {
        let nats = nats.clone();
        let mut agent = agent.clone();
        let pumped = pumped.clone();
        let pair = cfg.pair_id.clone();
        // 每命令 spawn：防一个慢命令 HOL 阻塞其它 session
        tokio::spawn(async move {
            if let Err(e) = handle_cmd(&nats, &mut agent, &pumped, &pair, msg).await {
                tracing::warn!("命令处理失败: {}", e);
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
        .map_err(|e| anyhow::anyhow!("命令 JSON 解析失败: {}", e))?;

    // 流式命令：先确保事件泵就绪（订阅先于命令），避免漏掉早期 agent_start/text_chunk。
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
        nats.flush().await.ok(); // 确保 reply 及时发出（request-reply）
    }
    Ok(())
}

/// 为某 session 启动一次长期事件泵：订阅 agent StreamEvents → 发布到 NATS。
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
    let mut stream = ac.stream_events(session).await?; // 订阅在此建立（先于命令执行）

    let nats = nats.clone();
    let subj = format!("p.{}.evt.{}", pair, session);
    let pumped = pumped.clone();
    let session_owned = session.to_string();

    tokio::spawn(async move {
        tracing::info!("事件泵启动：session={}", session_owned);
        loop {
            match stream.message().await {
                Ok(Some(ev)) => {
                    // 骨架：转发 {type, data}（data 仍是 JSON 字符串）。P1 会补 run_id/idx。
                    let body = serde_json::json!({ "type": ev.r#type, "data": ev.data });
                    if let Ok(bytes) = serde_json::to_vec(&body) {
                        if let Err(e) = nats.publish(subj.clone(), bytes.into()).await {
                            tracing::warn!("发布事件失败: {}", e);
                        }
                    }
                }
                Ok(None) => {
                    tracing::info!("agent 事件流结束：session={}", session_owned);
                    break;
                }
                Err(e) => {
                    tracing::warn!("agent 事件流错误 session={}: {}", session_owned, e);
                    break;
                }
            }
        }
        // 允许下次命令重新启动 pump
        pumped.lock().await.remove(&session_owned);
    });

    Ok(())
}
