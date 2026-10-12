use super::*;

#[test]
fn serialize_entry_empty_content_skips_content_reinsert() {
    // Non-array/non-string content → `_ => Vec::new()` (empty blocks) and
    // the `!blocks.is_empty()` guard skips re-inserting `content`.
    let mut entry = SessionEntry::new_user("user", serde_json::json!("x"));
    entry.content = None;
    let json = Manager::serialize_entry(&entry).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(value.get("content").is_none());
    assert!(value.get("thinking").is_none());
}

#[test]
fn serialize_entry_tool_with_non_text_block_extracts_empty_text() {
    // A tool entry whose only block is a reasoning block exercises the
    // `_ => None` arm of the text-extraction filter and still emits a
    // synthetic (empty) tool_result.
    let mut tool = SessionEntry::new_tool("tc1", "result");
    tool.content = Some(serde_json::json!([serde_json::to_value(
        crate::types::ContentBlock::reasoning("reason", Default::default())
    )
    .unwrap()]));
    let json = Manager::serialize_entry(&tool).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let content = value.get("content").unwrap().as_array().unwrap();
    assert!(content
        .iter()
        .any(|b| { b.get("type").and_then(|t| t.as_str()) == Some("tool_result") }));
}
