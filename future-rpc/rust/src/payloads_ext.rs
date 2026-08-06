//! Serde carriers for the remaining Tier-1 response payloads — the ad-hoc
//! `json!` shapes the agent builds for prompt acks and the model/info/skills
//! commands, lifted into typed structs so encode and decode share one
//! definition (parity by construction). Field casing mirrors the wire JSON:
//! camelCase everywhere except the prompt ack and refresh_skills, which were
//! introduced snake_case.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ── prompt ack ───────────────────────────────────────────────────────────────

/// How an accepted prompt was admitted by the session scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunAcceptedState {
    Existing,
    Running,
    Queued,
}

impl RunAcceptedState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Existing => "existing",
            Self::Running => "running",
            Self::Queued => "queued",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "existing" => Some(Self::Existing),
            "running" => Some(Self::Running),
            "queued" => Some(Self::Queued),
            _ => None,
        }
    }
}

/// Canonical acknowledgement for every accepted prompt request.
///
/// `run_sequence` and `queue_position` remain absent until the session
/// scheduler is the allocator; callers must not invent either value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunAck {
    pub run_id: String,
    pub run_epoch: u64,
    pub accepted_state: RunAcceptedState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_sequence: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_position: Option<u64>,
}

impl RunAck {
    pub fn existing(run_id: String, run_epoch: u64) -> Self {
        Self {
            run_id,
            run_epoch,
            accepted_state: RunAcceptedState::Existing,
            run_sequence: None,
            queue_position: None,
        }
    }

    pub fn running(run_id: String, run_epoch: u64) -> Self {
        Self {
            run_id,
            run_epoch,
            accepted_state: RunAcceptedState::Running,
            run_sequence: None,
            queue_position: None,
        }
    }

    pub fn queued(run_id: String, run_sequence: u64, queue_position: u64) -> Self {
        Self {
            run_id,
            run_epoch: 0,
            accepted_state: RunAcceptedState::Queued,
            run_sequence: Some(run_sequence),
            queue_position: Some(queue_position),
        }
    }
}

// ── list_models ──────────────────────────────────────────────────────────────

/// list_models payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListModelsPayload {
    pub models: Vec<ModelEntryPayload>,
    pub default_model: String,
    pub is_scoped: bool,
    /// Built-in provider catalog summary keyed by provider id; present only
    /// when the command set include_builtin_providers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub builtin_providers: Option<BTreeMap<String, BuiltinProviderPayload>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntryPayload {
    pub id: String,
    pub label: String,
    pub provider: String,
    pub supports_images: bool,
    pub thinking_level: String,
    pub context_window: i64,
    pub is_default: bool,
    /// JSON null when the catalog carries no description (must serialize as
    /// null, not be omitted).
    pub description: Option<String>,
    pub description_en: Option<String>,
    pub recommended: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinProviderPayload {
    pub name: String,
    pub model_count: usize,
    pub base_url: String,
}

// ── get_agent_info ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfoPayload {
    pub version: String,
    pub agent_instance_id: String,
    pub skills_count: usize,
}

// ── get_commands ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandsPayload {
    pub commands: Vec<CommandPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandPayload {
    pub name: String,
    pub description: String,
    /// Localized variants; JSON null when the skill carries none (must
    /// serialize as null, not be omitted).
    pub name_zh: Option<String>,
    pub description_zh: Option<String>,
    pub source: String,
}

// ── cycle_model ──────────────────────────────────────────────────────────────

/// cycle_model payload. `is_scoped` is absent on the empty-catalog edge
/// case, matching the agent's JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CycleModelPayload {
    pub model: String,
    pub thinking_level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_scoped: Option<bool>,
}

// ── sync_future_models ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncFutureModelsPayload {
    pub synced: bool,
    pub model_count: usize,
}

// ── refresh_skills ───────────────────────────────────────────────────────────

/// refresh_skills payload — snake_case on the wire, unlike the other
/// camelCase payloads in this module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshSkillsPayload {
    pub skills_count: usize,
    pub skills: Vec<String>,
    pub refreshed: bool,
}

// ── compact ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactPayload {
    pub tokens_before: i64,
    pub tokens_after: i64,
    pub summary: String,
    pub messages_removed: i64,
}

// ── shell ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellPayload {
    pub output: String,
    pub exit_code: i32,
}

// ── get_session_stats ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatsPayload {
    pub session_file: String,
    pub session_id: String,
    pub user_messages: usize,
    pub assistant_messages: usize,
    pub tool_calls: usize,
    pub tool_results: usize,
    pub total_messages: usize,
    pub tokens: StatsTokensPayload,
    /// Carried as a raw JSON number: the agent emits an integer literal and
    /// parity must preserve the int-vs-float representation exactly.
    pub cost: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsTokensPayload {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub total: i64,
}

// ── get_runtime_metrics ──────────────────────────────────────────────────────

/// get_runtime_metrics payload. Note: `active_run_id` and
/// `event_journal_error` serialize as JSON null when absent (the agent
/// builds this payload with `json!` over Options), so they must NOT use
/// skip_serializing_if — parity depends on null, not omission.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeMetricsPayload {
    pub session_id: String,
    pub active_run_gauge: usize,
    pub stale_epoch_drops: u64,
    pub persistence_degraded: u64,
    pub broadcast_lag: u64,
    pub ring_truncations: u64,
    /// JSON null when no run snapshot exists.
    pub active_run_id: Option<String>,
    pub queued_runs: usize,
    pub queued_bytes: usize,
    pub event_journal_healthy: bool,
    /// JSON null when the journal is healthy.
    pub event_journal_error: Option<String>,
}

// ── get_session_events_since ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEventsSincePayload {
    pub events: Vec<SessionEventRecordPayload>,
}

/// One session-scoped event. Field names mirror the StreamEvent envelope
/// (camelCase on the wire; `type` is the event type).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEventRecordPayload {
    #[serde(rename = "type")]
    pub event_type: String,
    pub data: String,
    pub session_id: String,
    pub session_idx: i64,
    pub event_id: String,
    pub timestamp: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn run_ack_omits_unallocated_queue_identity() {
        let value = serde_json::to_value(RunAck::running("run-a".into(), 7)).unwrap();
        assert_eq!(value["run_id"], "run-a");
        assert_eq!(value["run_epoch"], 7);
        assert_eq!(value["accepted_state"], "running");
        assert!(value.get("run_sequence").is_none());
        assert!(value.get("queue_position").is_none());
    }

    #[test]
    fn run_ack_queued_carries_queue_identity() {
        let value = serde_json::to_value(RunAck::queued("run-b".into(), 3, 0)).unwrap();
        assert_eq!(value["accepted_state"], "queued");
        assert_eq!(value["run_sequence"], 3);
        assert_eq!(value["queue_position"], 0);
        assert_eq!(value["run_epoch"], 0);
    }

    #[test]
    fn list_models_payload_uses_camel_case() {
        let payload = ListModelsPayload {
            models: vec![ModelEntryPayload {
                id: "m1".to_string(),
                label: "Model 1".to_string(),
                provider: "future".to_string(),
                supports_images: true,
                thinking_level: "high".to_string(),
                context_window: 200_000,
                is_default: true,
                description: Some("描述".to_string()),
                description_en: Some("desc".to_string()),
                recommended: true,
            }],
            default_model: "m1".to_string(),
            is_scoped: false,
            builtin_providers: None,
        };
        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["models"][0]["supportsImages"], json!(true));
        assert_eq!(value["models"][0]["contextWindow"], json!(200_000));
        assert_eq!(value["defaultModel"], json!("m1"));
        assert!(value.get("builtinProviders").is_none());
    }

    #[test]
    fn refresh_skills_payload_keeps_snake_case() {
        let payload = RefreshSkillsPayload {
            skills_count: 2,
            skills: vec!["a".to_string(), "b".to_string()],
            refreshed: true,
        };
        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["skills_count"], json!(2));
        assert_eq!(value["refreshed"], json!(true));
    }

    #[test]
    fn cycle_model_payload_omits_is_scoped_when_absent() {
        let payload = CycleModelPayload {
            model: String::new(),
            thinking_level: String::new(),
            is_scoped: None,
        };
        let value = serde_json::to_value(&payload).unwrap();
        assert!(value.get("isScoped").is_none());
    }
}
