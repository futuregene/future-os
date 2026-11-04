//! Paged message reads: a page must be the same slice a whole-list read would
//! have produced, and reading it must not touch the rest of the conversation.
use crate::session::projection::entries_to_agent_messages;
use crate::session::Manager;
use serde_json::{json, Value};

/// A transcript with the shape that makes the old whole-list read expensive:
/// many message rows plus the non-message kinds interleaved among them.
fn transcript(messages: usize) -> Vec<Value> {
    let mut entries = vec![json!({
        "id": "info",
        "type": "session_info",
        "role": "system",
        "timestamp": "2026-11-04T00:00:00Z",
        "content": {"cwd": "/workspace", "model": "mock", "session_name": "paged"}
    })];
    for index in 0..messages {
        let role = if index % 2 == 0 { "user" } else { "assistant" };
        entries.push(json!({
            "id": format!("m-{index}"),
            "type": role,
            "role": role,
            "timestamp": "2026-11-04T00:00:01Z",
            "content": [{"type": "text", "text": format!("message {index}")}]
        }));
        if index % 17 == 0 {
            entries.push(json!({
                "id": format!("terminal-{index}"),
                "type": "run_terminal",
                "timestamp": "2026-11-04T00:00:02Z",
                "content": {"run_id": format!("run-{index}"), "state": "completed"}
            }));
        }
    }
    entries
}

fn manager_with(id: &str, messages: usize) -> (tempfile::TempDir, Manager) {
    let root = tempfile::tempdir().unwrap();
    let manager = Manager::new(root.path().join("sessions"));
    let store = manager.storage().unwrap();
    store.reserve_segmented(id).unwrap();
    store.replace(id, transcript(messages)).unwrap();
    (root, manager)
}

fn as_json(messages: &[crate::types::AgentMessage]) -> Vec<Value> {
    messages
        .iter()
        .map(|message| serde_json::to_value(message).unwrap())
        .collect()
}

#[test]
fn a_page_is_exactly_the_slice_the_whole_list_would_contain() {
    let (_root, manager) = manager_with("paged-canonical", 120);
    let store = manager.storage().unwrap();
    let whole = as_json(&entries_to_agent_messages(
        &manager.load("paged-canonical").unwrap().entries,
        false,
    ));
    assert_eq!(whole.len(), 120);

    let mut collected = Vec::new();
    let mut offset = 0;
    loop {
        let (page, total) = manager.message_page("paged-canonical", offset, 40).unwrap();
        assert_eq!(
            total,
            whole.len(),
            "the page reports the real message count"
        );
        if page.is_empty() {
            break;
        }
        collected.extend(as_json(&entries_to_agent_messages(&page, false)));
        offset += page.len();
    }
    assert_eq!(collected.len(), whole.len());
    assert_eq!(collected, whole);
    assert_eq!(offset, whole.len());

    // A page past the end is empty, not an error, and still reports the count.
    let (empty, total) = manager.message_page("paged-canonical", 500, 40).unwrap();
    assert!(empty.is_empty());
    assert_eq!(total, whole.len());
    let _ = store;
}

#[test]
fn a_page_reads_only_the_rows_it_returns() {
    let (_root, manager) = manager_with("paged-work", 400);
    let store = manager.storage().unwrap();
    // Warm the statement cache so the measurement is the page, not the compile.
    manager.message_page("paged-work", 0, 40).unwrap();
    let before = store.db.metrics()["readerJob"]["count"].as_u64().unwrap();
    let (page, _) = manager.message_page("paged-work", 300, 40).unwrap();
    let after = store.db.metrics()["readerJob"]["count"].as_u64().unwrap();
    assert_eq!(page.len(), 40);
    assert!(
        after - before <= 4,
        "a page must not cost a job per stored entry ({} jobs for 400 entries)",
        after - before
    );
}

#[test]
fn a_deep_page_costs_the_same_as_a_first_page() {
    // The interface contract the audit asked for: page cost does not grow with
    // the offset. This cannot assert wall time in a test, so it asserts the
    // *shape* that makes it true — the page is one bounded read either way.
    let (_root, manager) = manager_with("paged-flat", 400);
    let store = manager.storage().unwrap();
    manager.message_page("paged-flat", 0, 40).unwrap();
    let before = store.db.metrics()["readerJob"]["count"].as_u64().unwrap();
    manager.message_page("paged-flat", 360, 40).unwrap();
    let deep = store.db.metrics()["readerJob"]["count"].as_u64().unwrap() - before;
    let before = store.db.metrics()["readerJob"]["count"].as_u64().unwrap();
    manager.message_page("paged-flat", 0, 40).unwrap();
    let first = store.db.metrics()["readerJob"]["count"].as_u64().unwrap() - before;
    assert_eq!(
        deep, first,
        "a deep page must cost the same reads as a first page"
    );
}
