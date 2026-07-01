//! 远程控制运行时（内嵌 Bridge）。
//!
//! 设计见仓库根 `docs/remote-control-*.md`。已落地：
//!  - Step A：连 NATS、持有 client、报状态。
//!  - Step B：`publish_event` —— 在 `agent_bridge::stream` 消费处把事件镜像给手机。
//!  - Step C（本文件）：订阅 `p.{pairId}.cmd.>`，把手机命令路由进 GUI 的持久化路径。
//!     - `list_sessions` / `get_messages` / `new_session` → 直接读写 GUI store。
//!     - `prompt` → 复刻前端 handleSend：建 thread/run + append user → `agent_prompt`
//!       （流式→落 run_events + tap 镜像）→ append assistant → 通知前端刷新。

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Mutex;

/// 运行中的远程连接。持有 async-nats client + 命令订阅任务；stop 时 abort 任务并 drop client。
struct RemoteState {
    client: async_nats::Client,
    nats_url: String,
    pair_id: String,
    cmd_task: tokio::task::JoinHandle<()>,
}

static STATE: Mutex<Option<RemoteState>> = Mutex::new(None);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStartInput {
    /// GUI 后端连的是 NATS **客户端端口**（`nats://host:4222`），不是浏览器的 ws 端口。
    pub nats_url: String,
    pub pair_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub running: bool,
    pub connected: bool,
    pub nats_url: String,
    pub pair_id: String,
    pub error: Option<String>,
}

fn empty() -> RemoteStatus {
    RemoteStatus {
        running: false,
        connected: false,
        nats_url: String::new(),
        pair_id: String::new(),
        error: None,
    }
}

pub async fn start(input: RemoteStartInput) -> Result<RemoteStatus, crate::AppError> {
    // 先停旧连接（幂等：abort 旧订阅任务）。
    let _ = stop();

    let client = async_nats::connect(&input.nats_url)
        .await
        .map_err(|e| crate::AppError::Message(format!("连接 NATS 失败: {e}")))?;

    // 启动命令订阅任务（Step C）。
    let cmd_task = tokio::spawn(command_loop(client.clone(), input.pair_id.clone()));

    let status = RemoteStatus {
        running: true,
        connected: true,
        nats_url: input.nats_url.clone(),
        pair_id: input.pair_id.clone(),
        error: None,
    };
    *STATE.lock().unwrap() = Some(RemoteState {
        client,
        nats_url: input.nats_url,
        pair_id: input.pair_id,
        cmd_task,
    });
    Ok(status)
}

pub fn stop() -> RemoteStatus {
    if let Some(state) = STATE.lock().unwrap().take() {
        state.cmd_task.abort();
    }
    empty()
}

pub fn status() -> RemoteStatus {
    match STATE.lock().unwrap().as_ref() {
        Some(s) => RemoteStatus {
            running: true,
            connected: true,
            nats_url: s.nats_url.clone(),
            pair_id: s.pair_id.clone(),
            error: None,
        },
        None => empty(),
    }
}

/// 事件 tap（Step B）：若远程在运行，把一条 agent 事件镜像发布到
/// `p.{pairId}.evt.{session}`。无连接时直接返回，不阻塞 GUI 的事件消费。
pub async fn publish_event(session_id: &str, event_type: &str, data: &str, run_id: &str, idx: i64) {
    let target = {
        let guard = STATE.lock().unwrap();
        guard
            .as_ref()
            .map(|s| (s.client.clone(), s.pair_id.clone()))
    };
    let Some((client, pair_id)) = target else {
        return;
    };
    let subject = format!("p.{pair_id}.evt.{session_id}");
    let body = json!({ "type": event_type, "data": data, "runId": run_id, "idx": idx });
    if let Ok(payload) = serde_json::to_vec(&body) {
        let _ = client.publish(subject, payload.into()).await;
    }
}

// ─── Step C：命令订阅 + 路由 ────────────────────────────────────────────────

/// 客户端经 NATS 发来的命令（camelCase JSON，取 Bridge 需要的字段）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct IncomingCmd {
    id: String,
    #[serde(rename = "type")]
    cmd_type: String,
    session_id: String,
    message: String,
    // get_events_since (P1c backfill)
    run_id: String,
    since_idx: i64,
}

impl Default for IncomingCmd {
    fn default() -> Self {
        Self {
            id: String::new(),
            cmd_type: String::new(),
            session_id: String::new(),
            message: String::new(),
            run_id: String::new(),
            since_idx: -1,
        }
    }
}

async fn command_loop(client: async_nats::Client, pair_id: String) {
    let subject = format!("p.{pair_id}.cmd.>");
    let queue = format!("bridge.{pair_id}");
    let mut sub = match client.queue_subscribe(subject.clone(), queue).await {
        Ok(sub) => sub,
        Err(e) => {
            eprintln!("remote: 订阅命令失败 {subject}: {e}");
            return;
        }
    };
    eprintln!("remote: 已订阅命令 {subject}");
    while let Some(msg) = sub.next().await {
        let client = client.clone();
        let pair_id = pair_id.clone();
        // 每命令 spawn：防一个慢命令阻塞其它。
        tokio::spawn(async move {
            handle_command(&client, &pair_id, msg).await;
        });
    }
}

async fn handle_command(client: &async_nats::Client, pair_id: &str, msg: async_nats::Message) {
    let cmd: IncomingCmd = match serde_json::from_slice(&msg.payload) {
        Ok(cmd) => cmd,
        Err(e) => {
            reply(
                client,
                &msg,
                false,
                Value::Null,
                Some(&format!("命令 JSON 解析失败: {e}")),
            )
            .await;
            return;
        }
    };

    match cmd.cmd_type.as_str() {
        "list_sessions" => match crate::store::list_threads() {
            Ok(threads) => {
                let sessions: Vec<Value> = threads
                    .into_iter()
                    .filter_map(|t| {
                        t.agent_session_id.map(
                            |sid| json!({ "sessionId": sid, "title": t.title, "threadId": t.id }),
                        )
                    })
                    .collect();
                reply(client, &msg, true, json!({ "sessions": sessions }), None).await;
            }
            Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
        },
        "get_messages" => {
            let result = (|| -> Result<Value, crate::AppError> {
                match crate::store::find_thread_by_agent_session(&cmd.session_id)? {
                    Some(thread) => {
                        let messages = crate::store::list_messages(&thread.id)?;
                        Ok(json!({ "messages": messages }))
                    }
                    None => Ok(json!({ "messages": [] })),
                }
            })();
            match result {
                Ok(data) => reply(client, &msg, true, data, None).await,
                Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "get_events_since" => {
            // P1c：回放当前进行中这一轮的缓冲事件，让中途加入的客户端补齐丢失的前缀。
            match crate::agent_bridge::get_events_since(
                cmd.session_id.clone(),
                cmd.run_id.clone(),
                cmd.since_idx,
            )
            .await
            {
                Ok(data) => reply(client, &msg, true, data, None).await,
                Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "new_session" => match crate::store::create_thread(new_chat_thread_input()) {
            Ok(thread) => {
                crate::emit_remote_activity(&thread.id);
                let sid = thread.agent_session_id.unwrap_or(thread.id);
                reply(client, &msg, true, json!({ "sessionId": sid }), None).await;
            }
            Err(e) => reply(client, &msg, false, Value::Null, Some(&e.to_string())).await,
        },
        "prompt" => {
            // accept-ack 立即回；实际执行在后台（完成看事件流 agent_end）。
            let session_id = cmd.session_id.clone();
            let message = cmd.message.clone();
            let _pair = pair_id.to_string();
            tokio::spawn(async move {
                if let Err(e) = handle_remote_prompt(session_id, message).await {
                    eprintln!("remote: prompt 处理失败: {e}");
                }
            });
            reply(client, &msg, true, json!({}), None).await;
        }
        other => {
            reply(
                client,
                &msg,
                false,
                Value::Null,
                Some(&format!("暂不支持的命令: {other}")),
            )
            .await;
        }
    }
}

fn new_chat_thread_input() -> crate::store::CreateThreadInput {
    crate::store::CreateThreadInput {
        mode: "chat".to_string(),
        title: None,
        workspace_id: None,
        workspace_path: None,
        workspace_name: None,
        model_provider: None,
        model_id: None,
        thinking_level: None,
    }
}

/// 复刻前端 handleSend 的持久化序列：手机 prompt → 落 GUI SQLite + 显示 + tap 镜像。
async fn handle_remote_prompt(session_id: String, message: String) -> Result<(), crate::AppError> {
    // (a) 找/建 thread（按 agent_session_id；找不到就新建 chat thread）。
    let thread = match crate::store::find_thread_by_agent_session(&session_id)? {
        Some(thread) => thread,
        None => crate::store::create_thread(new_chat_thread_input())?,
    };
    let agent_session_id = thread
        .agent_session_id
        .clone()
        .unwrap_or_else(|| thread.id.clone());

    // (b) append user message。
    let user_msg = crate::store::append_message(crate::store::AppendMessageInput {
        thread_id: thread.id.clone(),
        run_id: None,
        role: "user".to_string(),
        content_type: Some("markdown".to_string()),
        content: message.clone(),
        status: Some("complete".to_string()),
    })?;

    // (c) 建 run。
    let run = crate::store::create_run(crate::store::CreateRunInput {
        thread_id: thread.id.clone(),
        trigger_message_id: Some(user_msg.id),
        model_provider: thread.model_provider.clone(),
        model_id: thread.model_id.clone(),
    })?;

    // 通知前端：新 thread/run 出现（列表刷新）。
    crate::emit_remote_activity(&thread.id);

    // (d) 跑 agent_prompt（流式事件由 stream.rs 落 run_events + tap 镜像给手机）。
    let result = crate::agent_bridge::agent_prompt(
        message,
        None,
        thread.id.clone(),
        Some(agent_session_id),
        Some(run.id.clone()),
        thread.model_id.clone(),
        thread.thinking_level.clone(),
    )
    .await;

    // (e) 结算 run + append assistant message（内容=返回全文），和前端一致。
    match result {
        Ok(response) => {
            let _ = crate::store::update_run_status(crate::store::UpdateRunStatusInput {
                run_id: run.id.clone(),
                status: "completed".to_string(),
                error_message: None,
                error_type: None,
            });
            let content = if response.content.trim().is_empty() {
                "Future Agent 已完成，但没有返回文本。".to_string()
            } else {
                response.content
            };
            let _ = crate::store::append_message(crate::store::AppendMessageInput {
                thread_id: thread.id.clone(),
                run_id: Some(run.id.clone()),
                role: "assistant".to_string(),
                content_type: Some("markdown".to_string()),
                content,
                status: Some("complete".to_string()),
            });
        }
        Err(e) => {
            let _ = crate::store::update_run_status(crate::store::UpdateRunStatusInput {
                run_id: run.id.clone(),
                status: "failed".to_string(),
                error_message: Some(e.to_string()),
                error_type: None,
            });
            let _ = crate::store::append_message(crate::store::AppendMessageInput {
                thread_id: thread.id.clone(),
                run_id: Some(run.id.clone()),
                role: "assistant".to_string(),
                content_type: Some("markdown".to_string()),
                content: format!("Future Agent 出错：{e}"),
                status: Some("failed".to_string()),
            });
        }
    }

    crate::emit_remote_activity(&thread.id);
    Ok(())
}

/// 统一回 request-reply 应答（`RpcResponse` 形状），并 flush 保证及时送达。
async fn reply(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    success: bool,
    data: Value,
    error: Option<&str>,
) {
    let Some(reply_subject) = msg.reply.clone() else {
        return;
    };
    let body = json!({
        "type": "response",
        "success": success,
        "data": data,
        "error": error,
    });
    if let Ok(payload) = serde_json::to_vec(&body) {
        let _ = client.publish(reply_subject, payload.into()).await;
        let _ = client.flush().await;
    }
}
