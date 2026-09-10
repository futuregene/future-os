//! Canonical history contract shared by existing RPC methods and clients.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessageBlock {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
    /// Unknown provider blocks remain opaque and lossless.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessageUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub cost_cny: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessageRun {
    pub status: Option<String>,
    pub error: Option<String>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionEntryPayload {
    pub id: String,
    pub kind: String,
    pub role: String,
    pub created_at_ms: i64,
    pub run_id: Option<String>,
    pub blocks: Vec<MessageBlock>,
    pub metadata: Option<Value>,
    pub usage: Option<MessageUsage>,
    pub run: Option<MessageRun>,
    pub session: Option<Value>,
    pub checkpoint: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextMessage {
    pub role: String,
    pub run_id: Option<String>,
    pub blocks: Vec<MessageBlock>,
    pub metadata: Option<Value>,
}

impl MessageBlock {
    /// Internal model/import representation is converted once at the boundary;
    /// consumers never need legacy top-level thinking or tool_calls fields.
    pub fn from_model(value: &Value) -> Self {
        let kind = value["type"].as_str().unwrap_or("opaque");
        let mut block = Self {
            kind: kind.to_owned(),
            ..Self::default()
        };
        match kind {
            "text" | "reasoning" => block.text = value["text"].as_str().map(str::to_owned),
            "tool_call" => {
                block.tool_call_id = value["id"].as_str().map(str::to_owned);
                block.name = value["name"].as_str().map(str::to_owned);
                block.arguments = value.get("args").cloned();
            }
            "tool_result" => {
                block.tool_call_id = value["tool_call_id"].as_str().map(str::to_owned);
                block.text = value["content"].as_str().map(str::to_owned);
                block.is_error = Some(value["is_error"].as_bool().unwrap_or(false));
            }
            "image_url" => {
                block.kind = "image".into();
                block.image_url = value["image_url"]["url"].as_str().map(str::to_owned);
            }
            _ => block.data = Some(value.clone()),
        }
        block.provider_metadata = value.get("provider_metadata").cloned();
        block
    }
}

/// Public metadata names are independent from the legacy import vocabulary.
pub fn session_metadata(value: &Value) -> Value {
    let mut value = value.clone();
    let Some(object) = value.as_object_mut() else {
        return value;
    };
    let usage = SessionUsage {
        input_tokens: object
            .remove("tokens_in")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        output_tokens: object
            .remove("tokens_out")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        cache_read_tokens: object
            .remove("tokens_cache_r")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        cache_write_tokens: object
            .remove("tokens_cache_w")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        cost_cny: object
            .remove("total_cost")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
    };
    for (old, new) in [
        ("session_name", "sessionName"),
        ("parent_session_id", "parentSessionId"),
        ("thinking_level", "thinkingLevel"),
        ("last_prompt_tokens", "lastPromptTokens"),
        ("auto_compaction", "autoCompactionEnabled"),
        ("created_by", "createdBy"),
        ("creator_id", "creatorId"),
        ("source_meta", "sourceMeta"),
    ] {
        if let Some(value) = object.remove(old) {
            object.insert(new.into(), value);
        }
    }
    object.insert(
        "usage".into(),
        serde_json::to_value(usage).expect("session usage"),
    );
    value
}

pub fn checkpoint_metadata(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        for (old, new) in [
            ("schema_version", "schemaVersion"),
            ("checkpoint_id", "checkpointId"),
            ("cutoff_entry_id", "cutoffEntryId"),
            ("tokens_before", "tokensBefore"),
            ("tokens_after", "tokensAfter"),
            ("algorithm_version", "algorithmVersion"),
            ("covered_entry_ids", "coveredEntryIds"),
            ("covered_from_entry_id", "coveredFromEntryId"),
            ("model_state", "modelState"),
        ] {
            if let Some(value) = object.remove(old) {
                object.insert(new.into(), value);
            }
        }
    }
    value
}

pub fn run_terminal(value: &Value) -> Value {
    serde_json::json!({
        "runId":value["run_id"],
        "status": match value["state"].as_str() {Some("error")=>"failed",Some("incomplete"|"interrupted_by_restart")=>"interrupted",Some(state)=>state,None=>"unknown"},
        "durationMs":value["run_duration_ms"],
        "error":value["error"].as_str().filter(|s|!s.is_empty()),
        "usage":MessageUsage {input_tokens:value["input_tokens"].as_i64(), output_tokens:value["run_tokens"].as_i64(),cache_read_tokens:value["cache_read_tokens"].as_i64(),cache_write_tokens:value["cache_write_tokens"].as_i64()}
    })
}
