//! Cross-crate producer type -> wire encoder -> JSON/bridge consumers.
use future_rpc::{decode, encode, events, proto};
use serde_json::json;

#[test]
fn actual_agent_usage_type_preserves_provider_fields_and_stop_reason() {
    let usage: future_agent::types::Usage = serde_json::from_value(json!({
        "prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14,
        "reasoning_tokens": 3, "credit_cost": "0.125",
        "provider_metadata": {"vendor": {"trace": "opaque"}}
    }))
    .unwrap();
    // Same serializer and wrapper as RunEvent::Model(Finish) production projection.
    let wire = json!({"type":"usage", "stopReason":"tool_calls", "usage":usage});
    let payload = encode::event_payload("usage", &wire.to_string()).unwrap();
    let event = proto::StreamEvent {
        r#type: "usage".into(),
        payload: Some(payload),
        ..Default::default()
    };
    let mut expected = wire;
    expected.as_object_mut().unwrap().remove("type");
    assert_eq!(decode::event_data(&event), expected);
}

#[test]
fn bridge_parser_keeps_existing_tool_semantics_with_additive_metadata() {
    let data = json!({"type":"tool_start","phase":"input","tc_index":2,"tool_id":"call","tool_name":"read","tool_args":"{\"path\":\"x\"}"});
    let mut event = proto::StreamEvent {
        r#type: "tool_start".into(),
        run_id: "run".into(),
        data: data.to_string(),
        ..Default::default()
    };
    let legacy = events::parse_agent_event(&event);
    event.payload = encode::event_payload("tool_start", &event.data);
    event.data.clear();
    for parsed in [legacy, events::parse_agent_event(&event)] {
        let Some((
            run_id,
            events::AgentEvent::ToolStart {
                tool_id,
                tool_name,
                tool_args,
            },
        )) = parsed
        else {
            panic!("expected tool start")
        };
        assert_eq!(run_id, "run");
        assert_eq!(tool_id, "call");
        assert_eq!(tool_name, "read");
        assert_eq!(tool_args.as_deref(), Some("{\"path\":\"x\"}"));
    }
    assert_eq!(decode::event_data(&event)["phase"], "input");
    assert_eq!(decode::event_data(&event)["tc_index"], 2);
}
