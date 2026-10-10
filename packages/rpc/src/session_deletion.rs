//! Bounded, idempotent session deletion shared by the Agent and its clients.

use serde::{Deserialize, Serialize};

pub const MAX_DELETE_SESSIONS: usize = 32;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSessionResult {
    pub session_id: String,
    pub deleted: bool,
    pub error: String,
    pub error_code: String,
    pub error_data: serde_json::Value,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSessionsResponse {
    pub results: Vec<DeleteSessionResult>,
}
