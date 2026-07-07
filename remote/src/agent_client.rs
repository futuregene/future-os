//! gRPC client for the local agent (stripped-down from channels/grpc_client.rs).
//! The bridge only needs two things: forward arbitrary RpcCommands, and subscribe to a session's event stream.

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
            .map_err(|e| anyhow!("Failed to connect to agent {}: {}", addr, e))?;
        Ok(Self {
            inner: FutureAgentClient::new(channel),
        })
    }

    /// Pass any command through to the agent (prompt/get_state/abort/approval_decision/...).
    pub async fn execute(&mut self, cmd: RpcCommand) -> Result<RpcResponse> {
        Ok(self
            .inner
            .execute_command(tonic::Request::new(cmd))
            .await
            .map_err(|e| anyhow!("execute_command failed: {}", e))?
            .into_inner())
    }

    /// Subscribe to a session's real-time event stream.
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
            .map_err(|e| anyhow!("stream_events failed: {}", e))?
            .into_inner())
    }
}
