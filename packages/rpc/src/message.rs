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
    #[serde(skip_serializing_if = "is_false")]
    pub is_error: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
    /// Unknown provider blocks remain opaque and lossless.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Skip `isError` unless it is `true`.
///
/// The field is only worth sending when it says something happened. `false` is
/// not "unset" in the usual sense — the value is genuinely set, just to the one
/// that says nothing went wrong — but no consumer can tell it from an absent
/// flag: the phone reads `if (block.isError && …)`. A history page was therefore
/// spending ~16 bytes on every tool result to say "not an error".
///
/// Both uninformative spellings are skipped, so this must replace
/// `Option::is_none` rather than sit beside it: `None` would otherwise start
/// serializing as `null` again.
///
/// This pairs with recording the real outcome — once failures set `true`, a
/// success sends no flag at all instead of a `false`.
fn is_false(value: &Option<bool>) -> bool {
    !matches!(value, Some(true))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Token counts of one message. The fields carry `skip_serializing_if` for the
/// same reason as [`SessionEntryPayload`]'s optionals: an unset category is
/// `null` on every entry that has the object, and the phone reads them through
/// an optional accessor (`usage?.outputTokens`), so an omitted field is
/// indistinguishable from an explicit `null`. Deserialization still accepts
/// both spellings.
pub struct MessageUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    /// Amount this session has spent (¥): the provider's own billing when it
    /// reports one (the Future platform's `credit_cost`), else the sum of the
    /// per-category estimates below.
    pub cost_cny: f64,
    /// Per-category estimates from the resolved model's per-1M-token prices, so
    /// a client can show where the amount came from. All zero for a model with
    /// no prices on file — a client must then show tokens only, never a
    /// fabricated ¥0 breakdown, and must not assume these sum to `cost_cny` for
    /// a provider that bills itself.
    #[serde(default)]
    pub cost_input_cny: f64,
    #[serde(default)]
    pub cost_output_cny: f64,
    #[serde(default)]
    pub cost_cache_read_cny: f64,
    #[serde(default)]
    pub cost_cache_write_cny: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Outcome of the run a message belongs to. `skip_serializing_if` follows
/// [`MessageUsage`]: nearly every entry carries this object, and its unset
/// members were a `null` on each of them.
pub struct MessageRun {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// One display-projected journal entry.
///
/// The optional fields carry `skip_serializing_if`, matching [`MessageBlock`]:
/// otherwise an absent field is written as `null`, and a history page is mostly
/// absent optionals — measured on the three heaviest real sessions, those
/// `null`s are 2.4-3.0% of the page the phone downloads, for a field set that
/// carries no information when unset. Every consumer reads them through an
/// optional (`entry.usage?.outputTokens`, `obj.get("checkpoint")`), so an
/// omitted field and an explicit `null` are indistinguishable to it.
///
/// Deserialization is unchanged: a peer or cached page that still spells these
/// out as `null` decodes exactly as before.
pub struct SessionEntryPayload {
    pub id: String,
    pub kind: String,
    pub role: String,
    pub created_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub blocks: Vec<MessageBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<MessageUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<MessageRun>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
                if block.image_url.is_none() {
                    block.data = Some(value.clone());
                }
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
        // The per-category split is derived live from the resolved model's
        // prices (get_state), not journalled: a replay prices tokens at today's
        // rates rather than freezing the rate a run was billed at.
        cost_input_cny: 0.0,
        cost_output_cny: 0.0,
        cost_cache_read_cny: 0.0,
        cost_cache_write_cny: 0.0,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn assistant_entry(
        usage: Option<MessageUsage>,
        run: Option<MessageRun>,
    ) -> SessionEntryPayload {
        SessionEntryPayload {
            id: "e1".into(),
            kind: "assistant".into(),
            role: "assistant".into(),
            created_at_ms: 1_785_931_200_000,
            blocks: Vec::new(),
            usage,
            run,
            ..Default::default()
        }
    }

    /// A partially-filled `usage`/`run` must not spend a `null` on each unset
    /// subfield: a history page carries a `run` object on nearly every entry and
    /// most subfields are unset, so those `null`s are dead weight there too.
    #[test]
    fn session_entry_usage_and_run_omit_unset_subfields_instead_of_null() {
        let entry = assistant_entry(
            Some(MessageUsage {
                output_tokens: Some(12),
                ..Default::default()
            }),
            Some(MessageRun {
                status: Some("completed".into()),
                ..Default::default()
            }),
        );
        let value = serde_json::to_value(&entry).unwrap();
        assert_eq!(value["usage"], json!({"outputTokens": 12}));
        assert_eq!(value["run"], json!({"status": "completed"}));
        let usage = value["usage"].as_object().unwrap();
        for key in ["inputTokens", "cacheReadTokens", "cacheWriteTokens"] {
            assert!(
                !usage.contains_key(key),
                "unset usage.{key} must be omitted, not null"
            );
        }
        let run = value["run"].as_object().unwrap();
        for key in ["error", "durationMs"] {
            assert!(
                !run.contains_key(key),
                "unset run.{key} must be omitted, not null"
            );
        }
    }

    /// The omission is a serialization concern only: `null` subfields written by
    /// an older peer (or a cached page) must keep deserializing exactly as before.
    #[test]
    fn session_entry_usage_and_run_still_read_explicit_null_subfields() {
        let raw = json!({
            "id": "e1",
            "kind": "assistant",
            "role": "assistant",
            "createdAtMs": 1_785_931_200_000_i64,
            "blocks": [],
            "usage": {
                "inputTokens": null,
                "outputTokens": 12,
                "cacheReadTokens": null,
                "cacheWriteTokens": null,
            },
            "run": { "status": null, "error": null, "durationMs": 34 },
        });
        let entry: SessionEntryPayload = serde_json::from_value(raw).unwrap();
        let usage = entry.usage.expect("usage object");
        assert_eq!(usage.input_tokens, None);
        assert_eq!(usage.output_tokens, Some(12));
        assert_eq!(usage.cache_read_tokens, None);
        assert_eq!(usage.cache_write_tokens, None);
        let run = entry.run.expect("run object");
        assert_eq!(run.status, None);
        assert_eq!(run.error, None);
        assert_eq!(run.duration_ms, Some(34));
    }

    /// The same rule through the other producer: `run_terminal` serialises a
    /// `MessageUsage` built from a journal row, and must omit what the row
    /// does not carry rather than nulling it.
    #[test]
    fn run_terminal_usage_omits_unset_categories() {
        let value = run_terminal(&json!({
            "run_id": "r1",
            "state": "completed",
            "run_duration_ms": 10,
            "input_tokens": 5,
        }));
        assert_eq!(value["usage"], json!({"inputTokens": 5}));
        assert_eq!(value["error"], Value::Null, "the run-level error stays");
    }

    /// A `false` is the flag saying nothing happened, so it does not need to be
    /// written: every consumer already reads an absent flag as falsy. `true` is
    /// the only informative value and must survive.
    #[test]
    fn block_is_error_is_written_only_when_it_is_true() {
        let block = |is_error| MessageBlock {
            kind: "tool_result".into(),
            text: Some("output".into()),
            tool_call_id: Some("c1".into()),
            is_error,
            ..Default::default()
        };
        let false_value = serde_json::to_value(block(Some(false))).unwrap();
        assert!(
            false_value.get("isError").is_none(),
            "a false flag carries no information and must be omitted"
        );
        assert_eq!(false_value["text"], json!("output"), "the rest is intact");

        let true_value = serde_json::to_value(block(Some(true))).unwrap();
        assert_eq!(true_value["isError"], json!(true));

        let unset = serde_json::to_value(block(None)).unwrap();
        assert!(unset.get("isError").is_none());
    }

    /// Omitting `false` is a serialization choice only. A page written by an
    /// older peer (or read back from cache) still spells it out, and both
    /// spellings have to decode to the same value.
    #[test]
    fn block_is_error_still_reads_an_explicit_false_or_null() {
        let decode = |raw: Value| -> Option<bool> {
            serde_json::from_value::<MessageBlock>(raw)
                .unwrap()
                .is_error
        };
        let base = |extra: Value| {
            let mut value = json!({"kind": "tool_result", "text": "o"});
            for (key, item) in extra.as_object().unwrap() {
                value[key] = item.clone();
            }
            value
        };
        assert_eq!(decode(base(json!({"isError": false}))), Some(false));
        assert_eq!(decode(base(json!({"isError": null}))), None);
        assert_eq!(decode(base(json!({}))), None);
        // The informative value is unchanged in both directions.
        assert_eq!(decode(base(json!({"isError": true}))), Some(true));
    }
}
