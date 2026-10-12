use super::*;
use serde_json::json;

fn event(kind: &str, index: i64) -> serde_json::Value {
    json!({"event_type":kind,"data":"{\"synthetic\":true}","session_id":"s","run_id":"r",
        "epoch":1,"idx":index,"session_idx":-1,"run_sequence":1,
        "timestamp":"2026-01-01T00:00:00Z"})
}

fn one_session(store: &SqliteStore) {
    store
        .replace(
            "s",
            vec![json!({"id":"e","type":"user","role":"user",
                "timestamp":"2026-01-01T00:00:00Z","content":"question"})],
        )
        .unwrap();
}

#[test]
fn only_pricing_events_are_read_back_and_identical_appends_are_no_ops() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    one_session(&store);
    store.append_event("s", event("usage", 0)).unwrap();
    store.append_event("s", event("model_changed", 1)).unwrap();
    store.append_event("s", event("text_chunk", 2)).unwrap();
    let payloads = store.pricing_event_payloads("s").unwrap();
    assert_eq!(payloads.len(), 2);
    assert!(
        !payloads
            .iter()
            .any(|payload| payload.contains("text_chunk")),
        "{payloads:?}"
    );

    // Re-appending the identical event is accepted silently, not a conflict.
    store.append_event("s", event("usage", 0)).unwrap();
    assert_eq!(store.events("s", "r").unwrap().len(), 3);
    assert!(store.has_events("s", "r").unwrap());
    assert!(!store.has_events("s", "other-run").unwrap());
}

#[test]
fn refreshing_a_session_info_keeps_one_record_with_the_latest_content() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    let info = |name: &str, id: &str| {
        json!({"id":id,"type":"session_info","role":"system",
            "timestamp":"2026-01-01T00:00:00Z",
            "content":{"session_name":name,"model":"mock","tokens_in":0}})
    };
    store.replace("s", vec![info("first", "info-1")]).unwrap();
    store.append("s", vec![info("second", "info-2")]).unwrap();
    let entries = store.entries("s").unwrap();
    let infos: Vec<_> = entries
        .iter()
        .filter(|entry| entry["type"] == "session_info")
        .collect();
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0]["content"]["session_name"], "second");
}
