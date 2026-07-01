//! 本地 agent 的 gRPC 客户端（照 channels/grpc_client.rs 精简）。
//! Bridge 只需要两件事：转发任意 RpcCommand、订阅某 session 的事件流。

use crate::proto::{
    future_agent_client::FutureAgentClient, RpcCommand, RpcResponse, StreamEvent, StreamRequest,
};
use anyhow::{anyhow, Result};

#[derive(Clone)]
pub struct AgentClient {
    inner: FutureAgentClient<tonic::transport::Channel>,
}

impl AgentClient {
    pub async fn connect(addr: &str) -> Result<Self> {
        let addr = format!(
            "http://{}",
            addr.trim_start_matches("http://")
                .trim_start_matches("https://")
        );
        let endpoint = tonic::transport::Endpoint::new(addr.clone())?
            .connect_timeout(std::time::Duration::from_secs(10));
        let channel = endpoint
            .connect()
            .await
            .map_err(|e| anyhow!("连接 agent {} 失败: {}", addr, e))?;
        Ok(Self {
            inner: FutureAgentClient::new(channel),
        })
    }

    /// 透传任意命令给 agent（prompt/get_state/abort/approval_decision/...）。
    pub async fn execute(&mut self, cmd: RpcCommand) -> Result<RpcResponse> {
        Ok(self
            .inner
            .execute_command(tonic::Request::new(cmd))
            .await
            .map_err(|e| anyhow!("execute_command 失败: {}", e))?
            .into_inner())
    }

    /// 订阅某 session 的实时事件流。
    pub async fn stream_events(
        &mut self,
        session_id: &str,
    ) -> Result<tonic::Streaming<StreamEvent>> {
        let req = tonic::Request::new(StreamRequest {
            session_id: session_id.to_string(),
            event_types: vec![],
        });
        Ok(self
            .inner
            .stream_events(req)
            .await
            .map_err(|e| anyhow!("stream_events 失败: {}", e))?
            .into_inner())
    }
}
