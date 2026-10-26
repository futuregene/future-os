//! Canonical UTF-8 range overlap handling, shared by stream consumers.
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Default, Clone)]
pub struct ContentCursors {
    offsets: HashMap<(String, i64, String), u64>,
}

/// A coalesced event covers every source cursor in this inclusive range.
/// Legacy events keep their existing one-cursor contract.
pub fn covers_next(data: &Value, expected: i64, last: i64) -> bool {
    if last == expected {
        return true;
    }
    data.get("content_key").and_then(Value::as_str).is_some()
        && data
            .get("content_first_idx")
            .and_then(Value::as_i64)
            .is_some_and(|first| first >= 0 && first <= expected && expected <= last)
}

impl ContentCursors {
    pub fn clear(&mut self) {
        self.offsets.clear();
    }

    /// Trim only an already applied prefix. Gaps and offsets inside a UTF-8
    /// scalar fail explicitly, rather than producing plausible corrupted text.
    pub fn apply(
        &mut self,
        run: &str,
        epoch: i64,
        kind: &str,
        data: &mut Value,
    ) -> Result<(), String> {
        let Some(key) = data.get("content_key").and_then(Value::as_str) else {
            return Ok(());
        };
        let identity = (run.to_owned(), epoch, key.to_owned());
        let offset = data
            .get("content_offset")
            .and_then(Value::as_u64)
            .ok_or("invalid canonical content offset")?;
        let length = data
            .get("content_bytes")
            .and_then(Value::as_u64)
            .ok_or("invalid canonical content length")?;
        let end = offset
            .checked_add(length)
            .ok_or("canonical content range overflow")?;
        let seen = self.offsets.get(&identity).copied().unwrap_or(0);
        if offset > seen && data.get("content_seal") != Some(&Value::Bool(true)) {
            return Err("canonical content has a missing prefix".into());
        }
        if matches!(kind, "text_chunk" | "thinking_delta" | "tool_delta") {
            let text = data.get("text").and_then(Value::as_str).unwrap_or("");
            if u64::try_from(text.len()).ok() != Some(length) {
                return Err("canonical content byte length mismatch".into());
            }
            let overlap = usize::try_from(seen.saturating_sub(offset).min(length))
                .map_err(|_| "canonical overlap overflow")?;
            let tail = text
                .get(overlap..)
                .ok_or("canonical overlap splits UTF-8")?
                .to_owned();
            data["text"] = json!(tail);
            data["content_offset"] = json!(offset + overlap as u64);
            data["content_bytes"] = json!(length - overlap as u64);
        }
        if data.get("content_seal") == Some(&Value::Bool(true)) {
            self.offsets.remove(&identity);
        } else {
            self.offsets.insert(identity, end.max(seen));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replay_overlaps_live_fragments_at_utf8_boundaries() {
        let mut cursors = ContentCursors::default();
        let mut live = json!({"content_key":"b","content_offset":0,"content_bytes":3,"text":"你","content_first_idx":4});
        cursors.apply("r", 1, "text_chunk", &mut live).unwrap();
        let mut replay = json!({"content_key":"b","content_offset":0,"content_bytes":6,"text":"你好","content_first_idx":4});
        assert!(covers_next(&replay, 5, 8));
        cursors.apply("r", 1, "text_chunk", &mut replay).unwrap();
        assert_eq!(replay["text"], "好");
        assert_eq!(replay["content_offset"], 3);
        assert_eq!(replay["content_bytes"], 3);
        let mut gap = json!({"content_key":"b","content_offset":7,"content_bytes":1,"text":"x"});
        assert!(cursors.apply("r", 1, "text_chunk", &mut gap).is_err());
    }
}

/// An event field, scoped by session/run on the command and by database restore
/// identity here. Content object IDs are never an RPC authority.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContentRequest {
    pub database_id: String,
    pub restore_generation: String,
    pub epoch: i64,
    pub cursor: i64,
    pub field: String,
    #[serde(default)]
    pub offset: i64,
    #[serde(default = "default_page_bytes")]
    pub max_bytes: i64,
}
fn default_page_bytes() -> i64 {
    64 * 1024
}
impl From<ContentRequest> for crate::proto::ContentRangeRequest {
    fn from(value: ContentRequest) -> Self {
        Self {
            database_id: value.database_id,
            restore_generation: value.restore_generation,
            epoch: value.epoch,
            cursor: value.cursor,
            field: value.field,
            offset: value.offset,
            max_bytes: value.max_bytes,
        }
    }
}
impl From<crate::proto::ContentRangeRequest> for ContentRequest {
    fn from(value: crate::proto::ContentRangeRequest) -> Self {
        Self {
            database_id: value.database_id,
            restore_generation: value.restore_generation,
            epoch: value.epoch,
            cursor: value.cursor,
            field: value.field,
            offset: value.offset,
            max_bytes: value.max_bytes,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StorageIdentity {
    pub database_id: String,
    pub restore_generation: String,
}
