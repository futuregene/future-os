//! Assign canonical content identities at the single ordered broadcast point.
//! Offsets count UTF-8 bytes, independently of live event/token boundaries.
use super::SseEvent;
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Clone, Default)]
pub(super) struct ContentSequence {
    lanes: HashMap<String, (String, i64)>,
    scope: (String, i64),
    message_id: String,
    result_entries: HashMap<String, String>,
}

impl ContentSequence {
    pub(super) fn take_result_entry(&mut self, tool: &str) -> Option<String> {
        self.result_entries.remove(tool)
    }

    pub(super) fn stamp(
        &mut self,
        event: &SseEvent,
        data: &mut Value,
        initial_tool_bytes: usize,
    ) -> Result<()> {
        if self.scope != (event.run_id.clone(), event.epoch) {
            self.lanes.clear();
            self.message_id.clear();
            self.result_entries.clear();
            self.scope = (event.run_id.clone(), event.epoch);
        }
        ensure!(
            data["block_id"].as_str().is_none_or(|id| id.len() <= 512)
                && data["tool_id"].as_str().is_none_or(|id| id.len() <= 1024)
                && data["entry_id"].as_str().is_none_or(|id| id.len() <= 512)
                && data["tool_name"]
                    .as_str()
                    .is_none_or(|name| name.len() <= 512),
            "provider content identity exceeds metadata budget"
        );
        if event.event_type == "message_start" {
            self.message_id = data["entry_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("message identity is missing"))?
                .to_owned();
            self.lanes.clear();
            return Ok(());
        }
        if event.event_type == "message_end" {
            self.lanes.clear();
            return Ok(());
        }
        if event.event_type == "tool_end" {
            let tool = data["tool_id"].as_str().unwrap_or("").to_owned();
            if !tool.is_empty() {
                ensure!(
                    self.result_entries.len() < 1024,
                    "too many unconsumed tool results"
                );
                let id = crate::utils::generate_entry_id();
                self.result_entries.insert(tool, id.clone());
                data["entry_id"] = json!(id);
            }
            return Ok(());
        }
        ensure!(
            data["block_id"].as_str().is_none_or(|id| id.len() <= 512)
                && data["tool_id"].as_str().is_none_or(|id| id.len() <= 1024),
            "provider content identity exceeds metadata budget"
        );
        let lane = match event.event_type.as_str() {
            "text_start" | "text_chunk" | "text_end" => {
                format!("text:{}", data["block_id"].as_str().unwrap_or(""))
            }
            "thinking_start" | "thinking_delta" | "thinking_end" => {
                format!("thinking:{}", data["block_id"].as_str().unwrap_or(""))
            }
            "tool_start" if data["phase"] == "input" => {
                format!("tool:{}", data["tc_index"].as_u64().unwrap_or(0))
            }
            "tool_delta" | "tool_input_end" => {
                format!("tool:{}", data["tc_index"].as_u64().unwrap_or(0))
            }
            _ => return Ok(()),
        };
        let starts = matches!(event.event_type.as_str(), "text_start" | "thinking_start")
            || (event.event_type == "tool_start" && data["phase"] == "input")
            || (event.event_type == "tool_delta" && data["snapshot"] == true);
        if starts && lane.starts_with("tool:") {
            self.lanes.remove(&lane);
        }
        ensure!(
            self.lanes.len() < 1024 || self.lanes.contains_key(&lane),
            "too many simultaneous content blocks"
        );
        let key = if !self.message_id.is_empty() && !lane.starts_with("tool:") {
            format!("{}:{lane}", self.message_id)
        } else {
            format!("{}:{}:{lane}", self.message_id, event.idx)
        };
        let (key, offset) = self.lanes.entry(lane.clone()).or_insert((key, 0));
        if !self.message_id.is_empty() {
            data["entry_id"] = json!(self.message_id);
        }
        data["content_key"] = json!(key);
        data["content_offset"] = json!(offset);
        data["content_first_idx"] = json!(event.idx);
        let length = i64::try_from(if event.event_type == "tool_start" {
            initial_tool_bytes
        } else if matches!(
            event.event_type.as_str(),
            "text_chunk" | "thinking_delta" | "tool_delta"
        ) {
            data["text"].as_str().unwrap_or("").len()
        } else {
            0
        })?;
        data["content_bytes"] = json!(length);
        *offset = offset
            .checked_add(length)
            .ok_or_else(|| anyhow::anyhow!("content offset overflow"))?;
        let ends = matches!(
            event.event_type.as_str(),
            "text_end" | "thinking_end" | "tool_input_end"
        );
        if ends {
            data["content_seal"] = json!(true);
            self.lanes.remove(&lane);
        }
        Ok(())
    }
}
