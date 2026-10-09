//! Source-preserving input selection. Missing evidence remains a model judgment;
//! only oversized mandatory input is rejected here. No investigation tools run.
use super::tool_evidence::result_summary;
use super::{
    budget::{self, estimate, validate_request, STATE_LIMIT, STATE_TARGET},
    prompt::build_jev_request,
    redaction::redact,
};
use crate::types::{AgentMessage, ContentBlock};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;

const EVIDENCE_BUFFER_BYTES_LIMIT: usize = 32 * 1024;

#[derive(Clone, Serialize)]
pub(super) struct Evidence {
    pub source_id: String,
    source_kind: &'static str,
    pub sequence: usize,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_error: Option<bool>,
    redacted_text_bytes: usize,
    retained_range: [usize; 2],
    truncated: bool,
    #[serde(skip)]
    serialized_bytes: usize,
}

impl Evidence {
    fn new(source_id: String, source_kind: &'static str, sequence: usize, text: String) -> Self {
        let text = redact(&text);
        let bytes = text.len();
        Self {
            source_id,
            source_kind,
            sequence,
            text,
            tool_call_id: None,
            tool_name: None,
            is_error: None,
            redacted_text_bytes: bytes,
            retained_range: [0, bytes],
            truncated: false,
            serialized_bytes: 0,
        }
    }

    fn value(&self) -> Value {
        serde_json::to_value(self).expect("evidence JSON")
    }
}

#[derive(Clone)]
pub(super) struct EvidenceContext {
    users: VecDeque<Evidence>,
    user_bytes: usize,
    background: VecDeque<Evidence>,
    background_bytes: usize,
    omitted_users: usize,
    omitted_background: usize,
    omitted_checkpoints: usize,
    current_sequence: usize,
    next_sequence: usize,
    pub current_source_id: String,
}

impl Default for EvidenceContext {
    fn default() -> Self {
        Self {
            users: VecDeque::new(),
            user_bytes: 0,
            background: VecDeque::new(),
            background_bytes: 0,
            omitted_users: 0,
            omitted_background: 0,
            omitted_checkpoints: 0,
            current_sequence: 0,
            next_sequence: 1,
            current_source_id: "snapshot:0".into(),
        }
    }
}

#[derive(Debug)]
pub(super) struct PreparationError {
    pub code: &'static str,
    pub audit: Value,
}

pub(super) struct PreparedInput {
    pub state: Value,
    pub audit: Value,
}

fn is_checkpoint(message: &AgentMessage) -> bool {
    message.metadata.as_ref().is_some_and(|metadata| {
        metadata
            .get(crate::compaction::INTERNAL_CHECKPOINT_METADATA_KEY)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    })
}

impl EvidenceContext {
    pub fn from_history(messages: &[AgentMessage]) -> Self {
        let mut context = Self {
            next_sequence: 0,
            ..Self::default()
        };
        for message in messages {
            let sequence = context.next_sequence;
            if message.role == "user" && !is_checkpoint(message) {
                let text = display_text(message);
                if !text.trim().is_empty() {
                    let source = message
                        .journal_entry_id()
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("snapshot:{sequence}"));
                    if text.len() > EVIDENCE_BUFFER_BYTES_LIMIT {
                        context.omitted_users += context.users.len() + 1;
                        context.users.clear();
                        context.user_bytes = 0;
                    } else {
                        let mut record =
                            Evidence::new(source, "user_message", sequence, text.to_owned());
                        record.serialized_bytes = estimate(&record.value());
                        context.user_bytes += record.serialized_bytes;
                        context.users.push_back(record);
                        // Never skip a missing middle restriction to retain older permission.
                        while context.user_bytes > EVIDENCE_BUFFER_BYTES_LIMIT {
                            context.user_bytes -=
                                context.users.pop_front().unwrap().serialized_bytes;
                            context.omitted_users += 1;
                        }
                    }
                }
            }
            context.observe(message);
        }
        context.current_sequence = context.next_sequence;
        context.current_source_id = format!("snapshot:{}", context.current_sequence);
        context.next_sequence += 1;
        context
    }

    /// Called by this run's save callback, including ephemeral runs. User
    /// authorization stays frozen; new assistant/tool records are only background.
    pub fn observe(&mut self, message: &AgentMessage) {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        if is_checkpoint(message) {
            self.omitted_checkpoints += 1;
            return;
        }
        if !matches!(message.role.as_str(), "assistant" | "tool") {
            return;
        }
        let source = message
            .journal_entry_id()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("snapshot:{sequence}"));
        let mut records = Vec::new();
        if message.role == "assistant" {
            let text = display_text(message);
            if text.len() > EVIDENCE_BUFFER_BYTES_LIMIT {
                self.omitted_background += 1;
            } else if !text.trim().is_empty() {
                records.push(Evidence::new(
                    source.clone(),
                    "assistant_message",
                    sequence,
                    text.to_owned(),
                ));
            }
        }
        'blocks: for (index, block) in message.content.iter().enumerate() {
            let mut record = match block {
                ContentBlock::ToolCall { id, name, args, .. } if message.role == "assistant" => {
                    // Preserve useful invocation details without sending file bodies,
                    // environment values or opaque provider protocol metadata.
                    let parsed;
                    let args = if let Some(raw) = args.as_str() {
                        parsed = if raw.len() <= EVIDENCE_BUFFER_BYTES_LIMIT {
                            serde_json::from_str(raw).unwrap_or(Value::Null)
                        } else {
                            Value::Null
                        };
                        &parsed
                    } else {
                        args
                    };
                    let mut arguments = serde_json::Map::new();
                    for key in ["command", "path", "file_path", "filename"] {
                        if let Some(value) = args.get(key).and_then(Value::as_str) {
                            if value.len() > EVIDENCE_BUFFER_BYTES_LIMIT {
                                self.omitted_background += 1;
                                continue 'blocks;
                            }
                            arguments.insert(key.into(), json!(redact(value)));
                        }
                    }
                    let mut record = Evidence::new(
                        format!("{source}:{index}"),
                        "tool_call",
                        sequence,
                        json!({"tool_name":name,"arguments":arguments,"arguments_projected":true})
                            .to_string(),
                    );
                    record.tool_call_id = Some(id.clone());
                    // Arguments stay whole. Oversized calls are omitted by storage/admission.
                    record
                }
                ContentBlock::ToolResult {
                    tool_call_id,
                    content,
                    is_error,
                } if message.role == "tool" => {
                    let text = result_summary(&message.name, *is_error, content).to_string();
                    let mut record =
                        Evidence::new(format!("{source}:{index}"), "tool_result", sequence, text);
                    record.tool_call_id = Some(tool_call_id.clone());
                    record.tool_name = Some(message.name.clone());
                    record.is_error = Some(*is_error);
                    record
                }
                _ => continue,
            };
            // Metadata is always assigned by the host, never parsed from evidence text.
            record.source_id = format!("{source}:{index}");
            records.push(record);
        }
        for mut record in records {
            if self
                .background
                .iter()
                .any(|old| old.source_id == record.source_id)
            {
                continue;
            }
            record.serialized_bytes = estimate(&record.value());
            if record.serialized_bytes > EVIDENCE_BUFFER_BYTES_LIMIT {
                self.omitted_background += 1;
                continue;
            }
            self.background_bytes += record.serialized_bytes;
            self.background.push_back(record);
            while self.background_bytes > EVIDENCE_BUFFER_BYTES_LIMIT {
                self.background_bytes -= self.background.pop_front().unwrap().serialized_bytes;
                self.omitted_background += 1;
            }
        }
    }

    pub fn prepare(
        &self,
        user_request: &str,
        action: &Value,
        requested_action: &Value,
    ) -> Result<PreparedInput, PreparationError> {
        if user_request.len() > super::ACTION_RAW_BYTES_LIMIT
            || estimate(action) > super::ACTION_RAW_BYTES_LIMIT
            || action["oversized_command_bytes"].as_u64().is_some()
        {
            return Err(PreparationError {
                code: "input_too_large",
                audit: json!({
                    "schema_version":3, "source_ids":[self.current_source_id,action["tool_call_id"]],
                    "raw_input_bytes":{
                        "user_request":user_request.len(), "action":estimate(action),
                        "oversized_command":action["oversized_command_bytes"]
                    },
                    "estimator":budget::ESTIMATOR, "raw_input_limit":super::ACTION_RAW_BYTES_LIMIT,
                    "rejected_at":"raw_limit"
                }),
            });
        }
        let mut state = self.required_state(user_request, action);
        // Mandatory action/current request/host facts are never truncated.
        self.validate_state(&state)?;
        self.select_evidence(&mut state, action, requested_action);
        let req = build_jev_request(state.clone(), "jev");
        self.validate_state(&state)?;
        let audit = input_audit(&state, &req);
        Ok(PreparedInput { state, audit })
    }
    fn required_state(&self, user_request: &str, action: &Value) -> Value {
        let current = Evidence::new(
            self.current_source_id.clone(),
            "user_message",
            self.current_sequence,
            user_request.to_owned(),
        );
        let tool_id = action["tool_call_id"].as_str().unwrap_or("unknown");
        let mut planned = action.clone();
        if let Some(fields) = planned.as_object_mut() {
            for key in [
                "sandbox_boundary",
                "targets",
                "scope",
                "behavior",
                "rule_result",
                "network",
                "diagnostic_paths",
            ] {
                fields.remove(key);
            }
            fields.insert("source_id".into(), json!(tool_id));
            fields.insert("source_kind".into(), json!("tool_request"));
        }
        json!({
            "schema_version":3,
            "action":planned,
            "trusted_context":{"user_request":current,"user_history":[]},
            "host_facts":{
                "source_id":format!("approval:{tool_id}"),"source_kind":"host_approval_facts",
                "targets":action["targets"],"sandbox_boundary":action["sandbox_boundary"],
                "scope":action["scope"],"behavior":action["behavior"],"rule_result":"ask",
                "network_enforcement":"unrestricted","network_intent":action["network"]
            },
            "untrusted_context":[],
            "coverage":{
                "user_history":{"omitted":self.omitted_users + self.users.len()},
                "background":{"omitted":self.omitted_background + self.background.len()},
                "action_context":{"omitted":0},
                "checkpoints_omitted":self.omitted_checkpoints
            }
        })
    }

    fn select_evidence(&self, state: &mut Value, action: &Value, requested_action: &Value) {
        let tool_id = action["tool_call_id"].as_str().unwrap_or("unknown");
        let limit = STATE_TARGET.max(estimate(state)).min(STATE_LIMIT);
        let (local, action_context_count) = self.action_context(tool_id, action, requested_action);
        state["coverage"]["action_context"]["omitted"] = json!(action_context_count);
        // The assistant question immediately preceding the current user reply
        // has priority over older history, but is never promoted to authorization.
        let mut selected_background = Vec::new();
        for record in self.background.iter().filter(|record| {
            record.source_kind == "assistant_message"
                && record.sequence + 1 == self.current_sequence
        }) {
            if admit_background(state, record, "background", limit) {
                selected_background.push(record.source_id.as_str());
            }
        }
        let mut user_sequences = vec![self.current_sequence];
        for record in self.users.iter().rev() {
            let mut candidate = state.clone();
            candidate["trusted_context"]["user_history"]
                .as_array_mut()
                .unwrap()
                .insert(0, record.value());
            let omitted = candidate["coverage"]["user_history"]["omitted"]
                .as_u64()
                .unwrap();
            candidate["coverage"]["user_history"]["omitted"] = json!(omitted - 1);
            if estimate(&candidate) > limit {
                break;
            }
            *state = candidate;
            user_sequences.push(record.sequence);
        }
        let mut candidates = self
            .background
            .iter()
            .filter(|record| !selected_background.contains(&record.source_id.as_str()))
            .collect::<Vec<_>>();
        candidates.sort_by_key(|record| {
            let associated = record.tool_call_id.as_deref() == Some(tool_id);
            let adjacent = record.source_kind == "assistant_message"
                && user_sequences
                    .iter()
                    .any(|sequence| record.sequence.abs_diff(*sequence) == 1);
            (
                std::cmp::Reverse(associated),
                std::cmp::Reverse(adjacent),
                std::cmp::Reverse(record.sequence),
            )
        });
        for record in &local {
            if record.source_kind != "assistant_justification" {
                admit_background(state, record, "action_context", limit);
            }
        }
        // Current call facts and fixed result summaries precede assistant prose.
        for record in candidates
            .iter()
            .copied()
            .filter(|record| record.source_kind != "assistant_message")
        {
            admit_background(state, record, "background", limit);
        }
        for record in &local {
            if record.source_kind == "assistant_justification" {
                admit_background(state, record, "action_context", limit);
            }
        }
        for record in candidates
            .into_iter()
            .filter(|record| record.source_kind == "assistant_message")
        {
            let related = record.sequence > self.current_sequence
                || user_sequences
                    .iter()
                    .any(|sequence| record.sequence.abs_diff(*sequence) == 1);
            if related {
                admit_background(state, record, "background", limit);
            }
        }
        state["untrusted_context"]
            .as_array_mut()
            .unwrap()
            .sort_by_key(|record| record["sequence"].as_u64().unwrap_or(0));
    }

    fn validate_state(&self, state: &Value) -> Result<(), PreparationError> {
        let req = build_jev_request(state.clone(), "jev");
        validate_request(&req).map_err(|code| PreparationError {
            code,
            audit: input_audit(state, &req),
        })
    }

    fn action_context(
        &self,
        tool_id: &str,
        action: &Value,
        requested_action: &Value,
    ) -> (Vec<Evidence>, usize) {
        let mut records = Vec::new();
        let mut count = 0;
        for (key, kind) in [
            ("failure_summary", "tool_failure"),
            ("justification", "assistant_justification"),
        ] {
            if let Some(text) = requested_action
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
            {
                count += 1;
                if kind == "assistant_justification" && text.len() > EVIDENCE_BUFFER_BYTES_LIMIT {
                    continue;
                }
                let text = if kind == "tool_failure" {
                    result_summary("shell", true, text).to_string()
                } else {
                    text.to_owned()
                };
                let mut record =
                    Evidence::new(format!("{tool_id}:{key}"), kind, self.next_sequence, text);
                record.tool_call_id = Some(tool_id.to_owned());
                records.push(record);
            }
        }
        if let Some(paths) = action
            .get("diagnostic_paths")
            .filter(|paths| paths.as_array().is_some_and(|paths| !paths.is_empty()))
        {
            count += 1;
            if estimate(paths) <= EVIDENCE_BUFFER_BYTES_LIMIT {
                let mut record = Evidence::new(
                    format!("{tool_id}:diagnostic_paths"),
                    "diagnostic_paths",
                    self.next_sequence,
                    json!({"mentioned_paths":paths,"complete_targets":false}).to_string(),
                );
                record.tool_call_id = Some(tool_id.to_owned());
                records.insert(0, record);
            }
        }
        (records, count)
    }
}

fn admit_background(state: &mut Value, record: &Evidence, section: &str, limit: usize) -> bool {
    let mut candidate = state.clone();
    candidate["untrusted_context"]
        .as_array_mut()
        .unwrap()
        .push(record.value());
    let omitted = candidate["coverage"][section]["omitted"].as_u64().unwrap();
    candidate["coverage"][section]["omitted"] = json!(omitted - 1);
    if estimate(&candidate) > limit {
        return false;
    }
    *state = candidate;
    true
}

fn display_text(message: &AgentMessage) -> &str {
    message
        .content
        .iter()
        .find_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .unwrap_or("")
}

fn input_audit(state: &Value, req: &Value) -> Value {
    let mut sources = vec![
        state["action"]["source_id"].clone(),
        state["host_facts"]["source_id"].clone(),
        state["trusted_context"]["user_request"]["source_id"].clone(),
    ];
    for section in [
        &state["trusted_context"]["user_history"],
        &state["untrusted_context"],
    ] {
        sources.extend(
            section
                .as_array()
                .unwrap()
                .iter()
                .map(|record| record["source_id"].clone()),
        );
    }
    let truncated_sources = state["untrusted_context"].as_array().unwrap().iter()
        .filter(|record| record["truncated"] == true)
        .map(|record| json!({
            "source_id":record["source_id"], "redacted_text_bytes":record["redacted_text_bytes"],
            "retained_range":record["retained_range"]
        })).collect::<Vec<_>>();
    json!({
        "schema_version":3,
        "context_digest":format!("sha256:{:x}",Sha256::digest(state.to_string().as_bytes())),
        "source_ids":sources, "coverage":state["coverage"], "budget":budget::audit(req),
        "section_bytes":{
            "action":estimate(&state["action"]),
            "trusted_context":estimate(&state["trusted_context"]),
            "host_facts":estimate(&state["host_facts"]),
            "untrusted_context":estimate(&state["untrusted_context"])
        },
        "truncated_sources":truncated_sources
    })
}
