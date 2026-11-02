//! RPC command/response and SSE event envelopes.
use serde::{Deserialize, Serialize};

// ─── RPC Command (stdin) ────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcCommand {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type")]
    pub cmd_type: String,

    // Prompting
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub model_context: String,
    #[serde(default)]
    pub images: Vec<crate::types::ImageContent>,
    #[serde(default)]
    pub attachments: Vec<crate::types::Attachment>,
    #[serde(default)]
    pub parent_session: String,

    // set_model
    #[serde(default)]
    pub model_id: String,

    // set_thinking_level
    #[serde(default)]
    pub level: String,

    // Generic decision/rule/fork-point mode.
    #[serde(default)]
    pub mode: String,

    // compact
    #[serde(default)]
    pub custom_instructions: String,
    #[serde(default)]
    pub fusion_settings_json: String,

    // new_session — typed provenance (see proto). Legacy clients smuggle the
    // same info as JSON in custom_instructions.
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub source_meta: String,
    #[serde(default)]
    pub creator_id: String,

    // set_auto_compaction / set_auto_retry
    #[serde(default)]
    pub enabled: bool,

    // shell
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub shell_timeout_ms: u64,

    // Session
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub session_ids: Vec<String>,
    #[serde(default)]
    pub entry_id: String,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    /// Optional forward cursor for get_session_entries. Absent preserves the
    /// released all-at-once response; remote clients already send offset=0 and
    /// therefore opt into bounded pages.
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
    /// Exclusive backward cursor for lazy history pages. `limit` counts user
    /// exchanges when this is present; `next_offset` walks toward zero.
    #[serde(default)]
    pub before: Option<i64>,
    #[serde(default)]
    pub fragment_offset: Option<i64>,
    #[serde(default)]
    pub content_request: Option<future_rpc::content_range::ContentRequest>,
    #[serde(default)]
    pub replay_until_idx: Option<i64>,
    /// Upper bound on how many sessions `search_all_session_history` scans,
    /// most recently updated first. Absent selects the server default.
    #[serde(default)]
    pub max_sessions: Option<i64>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub cwd: String,

    // set_system_prompt
    #[serde(default)]
    pub system_prompt: String,

    // set_tools / disable_tools
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    // set_ephemeral
    pub ephemeral: bool,

    // set_enabled_models
    #[serde(default)]
    pub enabled_models: Option<Vec<String>>,

    // get_events_since (P1)
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub since_idx: i64,
    /// Page size for get_events_since; 0 = unlimited (legacy behavior).
    /// See proto RpcCommand.max_events.
    #[serde(default)]
    pub max_events: i64,
    #[serde(default)]
    pub requested_run_id: String,
    #[serde(default)]
    pub client_request_id: String,
    #[serde(default)]
    pub busy_policy: String,

    // list_models: also carry a summary of the built-in provider catalog
    // (`builtinProviders`) in the response, so clients can render the full
    // catalog at runtime instead of compiling it in.
    #[serde(default)]
    pub include_builtin_providers: bool,

    // set_sandbox_policy — populated from the typed proto sub-message by the
    // gRPC layer (not part of the JSON command surface).
    #[serde(skip)]
    pub sandbox_policy: Option<crate::sandbox::SandboxPolicy>,

    // set_auth / upsert_provider / delete_provider — typed config writes
    // (audit item 2). Populated from the proto sub-messages by the gRPC
    // layer; the agent applies them to its own auth.json/models.json and
    // refreshes live sessions, replacing out-of-band file edits + reload_auth.
    #[serde(skip)]
    pub auth_update: Option<crate::config::providers::AuthMutation>,
    #[serde(skip)]
    pub provider_config: Option<crate::config::providers::ProviderUpsertSpec>,

    // probe_thinking_level — an ad-hoc provider/model probe for the
    // custom-provider editor's thinking-level detection. Populated from the
    // proto sub-message by the gRPC layer; the handler answers per level.
    #[serde(skip)]
    pub thinking_probe: Option<crate::llm::probe::ThinkingLevelProbeSpec>,

    // suggest_skill — the user's first-turn text plus the UNINSTALLED skill
    // candidates the caller offers to Jev. Populated from the proto fields by
    // the gRPC layer; the agent never sees installed skills.
    #[serde(skip)]
    pub suggest_query: String,
    #[serde(skip)]
    pub suggest_candidates: Vec<crate::skill_reco::SkillCandidate>,

    // reset_session_history — in-place history reset selectors. Populated from
    // the proto `HistoryReset` sub-message by the gRPC layer; the handler owns
    // validation, so a bad selector is a `invalid_selectors` command error
    // rather than a transport-level rejection.
    #[serde(skip)]
    pub reset_history: Option<HistoryResetCommand>,

    #[serde(default)]
    pub skill_id: String,
    #[serde(default)]
    pub skill_version: String,
}

// ─── RPC Response (stdout) ───────────────────────────────────────────────

/// In-place history reset selectors as the wire carries them (proto
/// `HistoryReset`). Raw on purpose: the handler parses the drop kinds and the
/// bounds, so an unknown kind or a negative bound is a command error the caller
/// can act on, not a transport failure with no detail.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryResetCommand {
    /// false = dry run: report the plan, change nothing.
    pub apply: bool,
    /// Drop every turn (mutually exclusive with the keep bounds).
    pub all: Option<bool>,
    pub keep_first: Option<i64>,
    pub keep_last: Option<i64>,
    /// Entry kinds to remove from the retained part.
    pub drop: Vec<String>,
}

// ─── RPC Response (stdout) ───────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct RpcResponse {
    #[serde(rename = "type")]
    pub resp_type: String,
    #[serde(default)]
    pub id: String,
    pub command: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_data: Option<serde_json::Value>,
}

impl RpcResponse {
    pub(super) fn ok(id: &str, command: &str, data: impl Into<serde_json::Value>) -> String {
        let resp = Self {
            resp_type: "response".to_string(),
            id: id.to_string(),
            command: command.to_string(),
            success: true,
            data: Some(data.into()),
            error: None,
            error_code: None,
            error_data: None,
        };
        serde_json::to_string(&resp).unwrap_or_default()
    }

    pub fn build_fail(id: &str, command: &str, err: &str) -> String {
        let resp = Self {
            resp_type: "response".to_string(),
            id: id.to_string(),
            command: command.to_string(),
            success: false,
            data: None,
            error: Some(err.to_string()),
            error_code: None,
            error_data: None,
        };
        serde_json::to_string(&resp).unwrap_or_default()
    }

    pub fn build_fail_code(
        id: &str,
        command: &str,
        code: &str,
        err: &str,
        details: impl Into<serde_json::Value>,
    ) -> String {
        let resp = Self {
            resp_type: "response".to_string(),
            id: id.to_string(),
            command: command.to_string(),
            success: false,
            data: None,
            error: Some(err.to_string()),
            error_code: Some(code.to_string()),
            error_data: Some(details.into()),
        };
        serde_json::to_string(&resp).unwrap_or_default()
    }
}

/// SSE Event structure
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SseEvent {
    pub event_type: String,
    pub data: String,
    /// P1: stamped by `SseBroadcaster::broadcast` (callers leave default).
    /// `run_id` + `epoch` + `idx` are the run-scoped identity; `session_id` is
    /// added at the gRPC wire boundary (the stream is session-scoped).
    pub run_id: String,
    pub epoch: i64,
    pub idx: i64,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub event_id: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default = "default_session_idx")]
    pub session_idx: i64,
    #[serde(default = "default_session_idx")]
    pub run_sequence: i64,
}

impl SseEvent {
    pub fn new(event_type: &str, data: serde_json::Value) -> Self {
        Self {
            event_type: event_type.to_string(),
            data: serde_json::to_string(&data).unwrap_or_default(),
            run_id: String::new(),
            epoch: 0,
            idx: 0,
            session_id: String::new(),
            event_id: String::new(),
            timestamp: String::new(),
            session_idx: -1,
            run_sequence: -1,
        }
    }
}

fn default_session_idx() -> i64 {
    -1
}

pub(super) fn is_session_scoped_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "model_changed"
            | "thinking_level_changed"
            | "session_name_changed"
            | "cwd_changed"
            | "permission_level_changed"
            | "sandbox_policy_changed"
            | "auto_compaction_changed"
            | "tools_changed"
            | "config_reloaded"
            | "skills_reloaded"
            | "provider_config_changed"
    )
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
