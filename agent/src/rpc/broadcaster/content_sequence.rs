//! Assign canonical content identities at the single ordered broadcast point.
//! Offsets count UTF-8 bytes, independently of live event/token boundaries.
use super::SseEvent;
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Default)]
pub(super) struct ContentSequence {
    lanes: HashMap<String, (String, i64)>,
    /// Keys this message already sealed. A block's identity is
    /// `{message}:{lane}`, so a repeated close for the same block names the
    /// same body again; knowing which bodies are closed lets `stamp` drop that
    /// duplicate instead of opening an empty block over durable bytes.
    sealed: HashSet<String>,
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
            self.sealed.clear();
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
            self.sealed.clear();
            return Ok(());
        }
        if event.event_type == "message_end" {
            self.lanes.clear();
            self.sealed.clear();
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
        let ends = matches!(
            event.event_type.as_str(),
            "text_end" | "thinking_end" | "tool_input_end"
        );
        // A repeated close for a block this message already sealed. Its body is
        // durable under this identity, so opening a lane here would seal an
        // empty range over bytes that exist — which the journal refuses as a
        // contradiction, failing this batch and every later one. Closing an
        // already-closed display is not new content: let the event through
        // without a content identity.
        if ends
            && !lane.starts_with("tool:")
            && !self.lanes.contains_key(&lane)
            && self.sealed.contains(&key)
        {
            return Ok(());
        }
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
        if ends {
            data["content_seal"] = json!(true);
            let sealed = key.clone();
            self.lanes.remove(&lane);
            // Only a lane whose identity outlives the block (text, thinking)
            // can be closed twice. A tool lane's identity carries its cursor,
            // so remembering it would only grow the set with every tool call.
            if !lane.starts_with("tool:") {
                self.sealed.insert(sealed);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(idx: i64, event_type: &str, data: Value) -> SseEvent {
        SseEvent {
            run_id: "run".into(),
            epoch: 1,
            idx,
            ..SseEvent::new(event_type, data)
        }
    }

    fn stamp(sequence: &mut ContentSequence, event: &SseEvent) -> Value {
        let mut data = serde_json::from_str::<Value>(&event.data).expect("event data");
        sequence
            .stamp(event, &mut data, 0)
            .expect("content sequenced");
        data
    }

    fn open_block() -> ContentSequence {
        let mut sequence = ContentSequence::default();
        stamp(
            &mut sequence,
            &event(0, "message_start", json!({"entry_id": "m"})),
        );
        stamp(
            &mut sequence,
            &event(1, "thinking_start", json!({"block_id": "reasoning"})),
        );
        sequence
    }

    #[test]
    fn a_second_close_for_a_sealed_block_carries_no_content() {
        let mut sequence = open_block();
        stamp(
            &mut sequence,
            &event(
                2,
                "thinking_delta",
                json!({"block_id": "reasoning", "text": "abc"}),
            ),
        );
        let first = stamp(
            &mut sequence,
            &event(
                3,
                "thinking_end",
                json!({"block_id": "reasoning", "text": ""}),
            ),
        );
        assert_eq!(first["content_offset"], json!(3));
        assert_eq!(first["content_seal"], json!(true));

        // The provider layer closed this display and the retry closes it again.
        // Sealing here would present an empty body for a block that already
        // holds three bytes, which the journal refuses.
        let second = stamp(
            &mut sequence,
            &event(
                4,
                "thinking_end",
                json!({"block_id": "reasoning", "text": ""}),
            ),
        );
        assert!(second.get("content_key").is_none(), "{second}");
        assert!(second.get("content_seal").is_none(), "{second}");
    }

    #[test]
    fn the_next_message_may_reuse_a_block_identity() {
        let mut sequence = open_block();
        stamp(
            &mut sequence,
            &event(
                3,
                "thinking_end",
                json!({"block_id": "reasoning", "text": ""}),
            ),
        );
        stamp(
            &mut sequence,
            &event(4, "message_end", json!({"entry_id": "m"})),
        );
        stamp(
            &mut sequence,
            &event(5, "message_start", json!({"entry_id": "m2"})),
        );
        stamp(
            &mut sequence,
            &event(6, "thinking_start", json!({"block_id": "reasoning"})),
        );
        let end = stamp(
            &mut sequence,
            &event(
                7,
                "thinking_end",
                json!({"block_id": "reasoning", "text": ""}),
            ),
        );
        assert_eq!(end["content_key"], json!("m2:thinking:reasoning"), "{end}");
        assert_eq!(end["content_seal"], json!(true));
    }
}
