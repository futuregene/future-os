//! Accumulation and assembly of one assistant turn's content blocks.
//!
//! The model stream interleaves reasoning, text and tool-call events, and their
//! order is part of the transcript. These types hold that order until the turn
//! is assembled into a single assistant message.

use crate::types::{AgentMessage, AgentToolCall, ContentBlock};
#[derive(Debug, Default)]
pub(super) struct AccumulatedReasoningBlock {
    pub(super) id: String,
    pub(super) text: String,
    pub(super) provider_metadata: crate::types::ProviderMetadata,
}

#[derive(Debug, Default)]
pub(super) struct AccumulatedTextBlock {
    pub(super) id: String,
    pub(super) text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AssistantBlockOrder {
    Reasoning(String),
    Text(String),
    Tool(usize),
}

pub(super) fn record_assistant_block(
    order: &mut Vec<AssistantBlockOrder>,
    block: AssistantBlockOrder,
) {
    if !order.contains(&block) {
        order.push(block);
    }
}

pub(super) fn reasoning_block_mut<'a>(
    blocks: &'a mut Vec<AccumulatedReasoningBlock>,
    id: &str,
) -> &'a mut AccumulatedReasoningBlock {
    if let Some(index) = blocks.iter().position(|block| block.id == id) {
        return &mut blocks[index];
    }
    blocks.push(AccumulatedReasoningBlock {
        id: id.to_string(),
        ..Default::default()
    });
    blocks
        .last_mut()
        .expect("reasoning block was just inserted")
}

pub(super) fn text_block_mut<'a>(
    blocks: &'a mut Vec<AccumulatedTextBlock>,
    id: &str,
) -> &'a mut AccumulatedTextBlock {
    if let Some(index) = blocks.iter().position(|block| block.id == id) {
        return &mut blocks[index];
    }
    blocks.push(AccumulatedTextBlock {
        id: id.to_string(),
        ..Default::default()
    });
    blocks.last_mut().expect("text block was just inserted")
}

pub(super) fn assemble_assistant_parts(
    order: &[AssistantBlockOrder],
    reasoning_blocks: &[AccumulatedReasoningBlock],
    text_blocks: &[AccumulatedTextBlock],
    tool_calls: &[AgentToolCall],
) -> (Vec<ContentBlock>, Vec<Option<String>>) {
    let mut content = Vec::new();
    let mut keys = Vec::new();
    let mut emitted_reasoning = vec![false; reasoning_blocks.len()];
    let mut emitted_text = vec![false; text_blocks.len()];
    let mut emitted_tools = vec![false; tool_calls.len()];

    for block in order {
        match block {
            AssistantBlockOrder::Reasoning(id) => {
                if let Some((index, reasoning)) = reasoning_blocks
                    .iter()
                    .enumerate()
                    .find(|(_, reasoning)| reasoning.id == *id)
                {
                    emitted_reasoning[index] = true;
                    if !reasoning.text.is_empty() || !reasoning.provider_metadata.is_empty() {
                        keys.push(Some(format!("thinking:{}", reasoning.id)));
                        content.push(ContentBlock::reasoning(
                            &reasoning.text,
                            reasoning.provider_metadata.clone(),
                        ));
                    }
                }
            }
            AssistantBlockOrder::Text(id) => {
                if let Some((index, text)) = text_blocks
                    .iter()
                    .enumerate()
                    .find(|(_, text)| text.id == *id)
                {
                    emitted_text[index] = true;
                    if !text.text.is_empty() {
                        keys.push(Some(format!("text:{}", text.id)));
                        content.push(ContentBlock::text(&text.text));
                    }
                }
            }
            AssistantBlockOrder::Tool(index) => {
                if let Some(tool) = tool_calls.get(*index) {
                    emitted_tools[*index] = true;
                    keys.push(None);
                    content.push(ContentBlock::tool_call(
                        tool.id.clone(),
                        tool.name.clone(),
                        tool.args.clone(),
                        tool.provider_metadata.clone(),
                    ));
                }
            }
        }
    }

    // Compatibility events may omit starts or ids. Keep their content even
    // when there was not enough wire identity to place it precisely.
    for (index, reasoning) in reasoning_blocks.iter().enumerate() {
        if !emitted_reasoning[index]
            && (!reasoning.text.is_empty() || !reasoning.provider_metadata.is_empty())
        {
            keys.push(Some(format!("thinking:{}", reasoning.id)));
            content.push(ContentBlock::reasoning(
                &reasoning.text,
                reasoning.provider_metadata.clone(),
            ));
        }
    }
    for (index, text) in text_blocks.iter().enumerate() {
        if !emitted_text[index] && !text.text.is_empty() {
            keys.push(Some(format!("text:{}", text.id)));
            content.push(ContentBlock::text(&text.text));
        }
    }
    for (index, tool) in tool_calls.iter().enumerate() {
        if !emitted_tools[index] {
            keys.push(None);
            content.push(ContentBlock::tool_call(
                tool.id.clone(),
                tool.name.clone(),
                tool.args.clone(),
                tool.provider_metadata.clone(),
            ));
        }
    }
    (content, keys)
}
#[cfg(test)]
pub(super) fn assemble_assistant_content(
    order: &[AssistantBlockOrder],
    reasoning: &[AccumulatedReasoningBlock],
    text: &[AccumulatedTextBlock],
    tools: &[AgentToolCall],
) -> Vec<ContentBlock> {
    assemble_assistant_parts(order, reasoning, text, tools).0
}

pub(super) fn bind_stream_message(
    message: &mut AgentMessage,
    entry_id: &str,
    keys: Vec<Option<String>>,
) {
    let metadata = message.metadata.get_or_insert_with(serde_json::Map::new);
    metadata.insert(
        AgentMessage::JOURNAL_ENTRY_ID_KEY.into(),
        serde_json::json!(entry_id),
    );
    metadata.insert(
        AgentMessage::STREAM_BLOCKS_KEY.into(),
        serde_json::json!(keys
            .into_iter()
            .map(|key| key.map(|key| format!("{entry_id}:{key}")))
            .collect::<Vec<_>>()),
    );
}
