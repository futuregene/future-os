//! Reading a reasoning block back while its run is still writing it.
//!
//! A lean lane sends a reasoning row's boundaries and not its body, and a
//! reasoning block reaches the session store only with the entry that carries
//! it — when the run ends. A reader who opens the row mid-reasoning therefore
//! has no stored block to read, although the text is already in the run's live
//! projection: the same fold a streaming reply is drawn from. This folds that
//! projection the way the reply is rendered, so the on-demand read answers with
//! what has been written so far.
use super::SseEvent;
use serde_json::Value;
use std::collections::HashMap;

/// One reasoning block, in the order the reply draws it.
struct Block {
    id: Option<String>,
    text: String,
    /// A fragment of this block rode the wire as a content reference instead of
    /// inline text (a provider that emits one oversized chunk). The fold cannot
    /// see those bytes, so the block is not readable whole and must not be
    /// handed out as if it were.
    partial: bool,
}

impl Block {
    fn open(id: Option<String>) -> Self {
        Self {
            id,
            text: String::new(),
            partial: false,
        }
    }

    /// The block's text, or `None` when it cannot be read whole or holds nothing
    /// yet — both are "no body to show", never an error.
    fn body(self) -> Option<String> {
        (!self.partial && !self.text.trim().is_empty()).then_some(self.text)
    }
}

fn payload(event: &SseEvent) -> Value {
    serde_json::from_str(&event.data).unwrap_or(Value::Null)
}

fn block_id(data: &Value) -> Option<String> {
    data["block_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

/// The run's reasoning blocks in render order, `None` where a block cannot be
/// read whole.
///
/// The fold mirrors `packages/thread-projection`'s live apply, because the
/// index a client asks with is the position of its row in that rendering: a
/// start opens a block, a delta appends to the block its id names — or to the
/// open one, opening a block lazily when the provider sends a delta with no
/// start — and an end closes it.
pub(super) fn reasoning_blocks(events: &[SseEvent]) -> Vec<Option<String>> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut active: HashMap<String, usize> = HashMap::new();
    let mut open: Option<usize> = None;
    for event in events {
        match event.event_type.as_str() {
            "thinking_start" => {
                let id = block_id(&payload(event));
                if let Some(id) = id.clone() {
                    active.insert(id, blocks.len());
                }
                blocks.push(Block::open(id));
                open = blocks.len().checked_sub(1);
            }
            "thinking_delta" | "thinking_chunk" => {
                let data = payload(event);
                let text = data["text"].as_str().unwrap_or_default();
                let index = match block_id(&data).and_then(|id| active.get(&id).copied()) {
                    Some(index) => index,
                    None => match open {
                        Some(index) => index,
                        None => {
                            let id = block_id(&data);
                            let index = blocks.len();
                            if let Some(id) = id.clone() {
                                active.insert(id, index);
                            }
                            blocks.push(Block::open(id));
                            index
                        }
                    },
                };
                let block = &mut blocks[index];
                // A fragment moved to a content reference keeps its declared
                // length but carries no text, which is what makes it visible
                // here as a hole rather than as a short block.
                let declared = data["content_bytes"].as_u64().unwrap_or(text.len() as u64);
                block.partial |= declared > text.len() as u64;
                block.text.push_str(text);
                open = Some(index);
            }
            "thinking_end" => {
                let id = block_id(&payload(event));
                match &id {
                    Some(id) => {
                        active.remove(id);
                    }
                    None => active.clear(),
                }
                let open_is_this = match (&id, open) {
                    (None, _) => true,
                    (Some(id), Some(index)) => blocks[index].id.as_deref() == Some(id.as_str()),
                    (Some(_), None) => false,
                };
                if open_is_this {
                    open = None;
                }
            }
            _ => {}
        }
    }
    blocks.into_iter().map(Block::body).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(kind: &str, data: Value) -> SseEvent {
        SseEvent::new(kind, data)
    }

    /// Two blocks in one run, the first one closed before the second opens —
    /// the shape a reply between tool calls has. Coalesced segments (the
    /// projection's own folding) read the same as token-by-token deltas.
    #[test]
    fn folds_blocks_in_render_order_across_the_runs_turns() {
        let events = vec![
            event("agent_start", json!({})),
            event("thinking_start", json!({"block_id": "a"})),
            event("thinking_delta", json!({"block_id": "a", "text": "first "})),
            event(
                "thinking_delta",
                json!({"block_id": "a", "text": "thought"}),
            ),
            event("thinking_end", json!({"block_id": "a"})),
            event("text_chunk", json!({"text": "an answer"})),
            event("thinking_start", json!({"block_id": "b"})),
            event("thinking_delta", json!({"block_id": "b", "text": "second"})),
            event("agent_end", json!({})),
        ];
        assert_eq!(
            reasoning_blocks(&events),
            vec![Some("first thought".into()), Some("second".into())]
        );
    }

    /// A provider that opens a block without naming it, and one that sends a
    /// delta before any start: the same block count the reply renders.
    #[test]
    fn an_unnamed_or_unopened_block_still_counts_as_one() {
        let events = vec![
            event("thinking_start", json!({})),
            event("thinking_delta", json!({"text": "unnamed"})),
            event("thinking_end", json!({})),
            event(
                "thinking_delta",
                json!({"block_id": "late", "text": "lazy"}),
            ),
        ];
        assert_eq!(
            reasoning_blocks(&events),
            vec![Some("unnamed".into()), Some("lazy".into())]
        );
    }

    /// A block whose fragment was carried as a content reference is a hole, not
    /// a short thought: it is reported unreadable instead of as its visible
    /// prefix.
    #[test]
    fn a_block_with_a_reference_carried_fragment_is_not_readable() {
        let events = vec![
            event("thinking_start", json!({"block_id": "a"})),
            event("thinking_delta", json!({"block_id": "a", "text": "seen"})),
            event(
                "thinking_delta",
                json!({"block_id": "a", "text": "", "content_bytes": 40_000}),
            ),
        ];
        assert_eq!(reasoning_blocks(&events), vec![None]);
    }

    /// A block that has only just started has nothing to show yet, and an empty
    /// run has no blocks at all.
    #[test]
    fn an_empty_block_has_no_body() {
        assert!(reasoning_blocks(&[]).is_empty());
        let events = vec![event("thinking_start", json!({"block_id": "a"}))];
        assert_eq!(reasoning_blocks(&events), vec![None]);
    }
}
