//! The projection is the same run read once instead of token by token, so it
//! must not carry what the token lane already drops.
use super::*;
use serde_json::json;

/// A bootstrap must not carry what the stream it replaces already drops.
#[test]
fn a_lean_projection_drops_tool_output_text_and_keeps_the_outcome() {
    let mut page = json!({
        "projection": { "events": [
            {"type": "tool_end", "idx": 1,
             "data": "{\"tool_name\":\"shell\",\"text\":\"lots of output\",\"exit_code\":0}"},
            {"type": "tool_start", "idx": 2,
             "data": "{\"tool_name\":\"shell\",\"phase\":\"execution\",\"tool_args\":{\"command\":\"git status\"}}"},
            {"type": "thinking_delta", "idx": 3, "data": "{\"text\":\"reasoning\"}"}
        ]}
    });
    lean_replay_page(&mut page, true);
    let events = page["projection"]["events"].as_array().unwrap();
    assert_eq!(events.len(), 3, "the array keeps its length and idx");
    let tool_end = events[0]["data"].as_str().unwrap();
    assert!(!tool_end.contains("lots of output"), "{tool_end}");
    assert!(tool_end.contains("exit_code"), "{tool_end}");
    let tool_start = events[1]["data"].as_str().unwrap();
    assert!(!tool_start.contains("git status"), "{tool_start}");
    assert_eq!(
        events[2]["data"].as_str().unwrap(),
        "{\"text\":\"\"}",
        "reasoning text is blanked, its event kept"
    );

    // A page from a connection that declared nothing is untouched.
    let mut plain = json!({"projection": {"events": [
        {"type": "tool_end", "idx": 1,
         "data": "{\"tool_name\":\"shell\",\"text\":\"lots of output\",\"exit_code\":0}"}
    ]}});
    lean_replay_page(&mut plain, false);
    assert_eq!(
        plain["projection"]["events"][0]["data"].as_str().unwrap(),
        "{\"tool_name\":\"shell\",\"text\":\"lots of output\",\"exit_code\":0}"
    );
}

/// The legacy shape has no outcome field, so its text is the only signal a
/// reader can derive pass/fail from: it survives the trim.
#[test]
fn a_legacy_tool_result_keeps_its_text_in_the_projection() {
    let mut page = json!({"projection": {"events": [
        {"type": "tool_end", "idx": 1,
         "data": "{\"tool_name\":\"shell\",\"text\":\"boom\\n[exit: 1]\"}"}
    ]}});
    lean_replay_page(&mut page, true);
    assert!(
        page["projection"]["events"][0]["data"]
            .as_str()
            .unwrap()
            .contains("[exit: 1]"),
        "a legacy result keeps the text its outcome is derived from"
    );
}

/// An `input`-phase `tool_start` is dropped from the live lane; in a projection
/// the event has to stay (its `idx` is part of the sequence the client checks),
/// so its text is blanked instead.
#[test]
fn an_input_phase_tool_start_stays_in_place_but_carries_nothing() {
    let mut page = json!({"projection": {"events": [
        {"type": "tool_start", "idx": 5,
         "data": "{\"tool_name\":\"read\",\"phase\":\"input\",\"tool_args\":{\"text\":\"partial\"}}"}
    ]}});
    lean_replay_page(&mut page, true);
    let events = page["projection"]["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["idx"], json!(5));
}
