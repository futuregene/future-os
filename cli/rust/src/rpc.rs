//! gRPC client — port of `cli/src/rpc/grpc-client.ts` `RunClient`.
//!
//! P1 ports the methods the P1 commands use: get_agent_info, list_models,
//! get_state, list_sessions, get_session_entries, set_session_name (rename),
//! delete_session. The rest (prompt/stream/fork/...) arrive with P2 (`run`).
//!
//! Error surface: like the TS client, transport failures and `success:false`
//! responses surface as plain `String` messages; the exact bytes of
//! transport errors differ from grpc-js (network-stack dependent), which the
//! golden diff tests accept for remote commands.

use crate::generated::proto::future_agent_client::FutureAgentClient;
use crate::generated::proto::RpcCommand;
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `grpcAddr()` from agent.ts/session.ts — env override, then localhost default.
pub fn grpc_addr() -> String {
    std::env::var("FUTURE_AGENT_GRPC_ADDR").unwrap_or_else(|_| "127.0.0.1:50051".to_string())
}

/// `String(Date.now())` — millisecond epoch, used as the request correlation id.
fn now_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

/// Port of `RunClient` — fire-and-forget one-shot gRPC calls.
pub struct RunClient {
    addr: String,
}

impl RunClient {
    pub fn new(addr: &str) -> Self {
        Self {
            addr: addr.to_string(),
        }
    }

    /// Low-level `executeCommand(type, cmd, sessionId?, timeoutSecs=10)`.
    ///
    /// The TS client reuses one channel per command instance; this port
    /// connects per call — equivalent for one-shot CLI usage and keeps the
    /// client usable across `.await` points without interior mutability.
    async fn execute_command(
        &self,
        r#type: &str,
        mut cmd: RpcCommand,
        session_id: Option<&str>,
        timeout_secs: u64,
    ) -> Result<Value, String> {
        cmd.id = now_id();
        cmd.r#type = r#type.to_string();
        if let Some(sid) = session_id {
            cmd.session_id = sid.to_string();
        }

        let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{}", self.addr))
            .map_err(|e| e.to_string())?
            .timeout(Duration::from_secs(timeout_secs));
        let channel = endpoint.connect().await.map_err(|e| e.to_string())?;
        let mut client = FutureAgentClient::new(channel);
        let response = client
            .execute_command(cmd)
            .await
            .map_err(|status| {
                let msg = status.message();
                if msg.is_empty() {
                    status.to_string()
                } else {
                    msg.to_string()
                }
            })?
            .into_inner();

        if !response.success {
            return Err(if response.error.is_empty() {
                "unknown error".to_string()
            } else {
                response.error
            });
        }

        // `response.data` is a JSON string; try to parse it, else pass the
        // raw string through (mirrors the TS try/parse/catch).
        if response.data.is_empty() {
            return Ok(Value::Null);
        }
        match serde_json::from_str::<Value>(&response.data) {
            Ok(value) => Ok(value),
            Err(_) => Ok(Value::String(response.data)),
        }
    }

    /// `getAgentInfo()` — `get_agent_info` → `{version, skillsCount}`.
    pub async fn get_agent_info(&self) -> Result<Value, String> {
        self.execute_command("get_agent_info", RpcCommand::default(), None, 5)
            .await
    }

    /// `listModels()` — `list_models` → `{models, defaultModel}`.
    pub async fn list_models(&self) -> Result<Value, String> {
        self.execute_command("list_models", RpcCommand::default(), None, 5)
            .await
    }

    /// `getState(sessionId?)` — `get_state` → SessionState JSON.
    pub async fn get_state(&self, session_id: Option<&str>) -> Result<Value, String> {
        self.execute_command("get_state", RpcCommand::default(), session_id, 5)
            .await
    }

    /// `listSessions()` — `list_sessions` → `{sessions: [...]}`.
    pub async fn list_sessions(&self) -> Result<Value, String> {
        self.execute_command("list_sessions", RpcCommand::default(), None, 5)
            .await
    }

    /// `getSessionEntries(sessionId)` — `get_session_entries` → `{entries}`.
    pub async fn get_session_entries(&self, session_id: &str) -> Result<Value, String> {
        self.execute_command(
            "get_session_entries",
            RpcCommand::default(),
            Some(session_id),
            5,
        )
        .await
    }

    /// `renameSession(sessionId, name)` — `set_session_name`; errors on failure.
    pub async fn rename_session(&self, session_id: &str, name: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            name: name.to_string(),
            ..Default::default()
        };
        self.execute_command("set_session_name", cmd, Some(session_id), 5)
            .await?;
        Ok(())
    }

    /// `deleteSession(sessionId)` — `delete_session` → `{deleted: bool}`.
    pub async fn delete_session(&self, session_id: &str) -> Result<Value, String> {
        self.execute_command("delete_session", RpcCommand::default(), Some(session_id), 5)
            .await
    }
}

/// `process.env.FUTURE_AGENT_GRPC_ADDR ?? "127.0.0.1:50051"` (doctor.ts).
pub fn grpc_addr_env() -> String {
    grpc_addr()
}
