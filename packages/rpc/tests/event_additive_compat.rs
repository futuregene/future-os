use future_rpc::proto;
use prost::Message;

// Frozen pre-addition wire shape. New metadata must not renumber existing fields.
#[derive(Clone, PartialEq, Message)]
struct LegacyToolStart {
    #[prost(string, tag = "1")]
    tool_id: String,
    #[prost(string, tag = "2")]
    tool_name: String,
    #[prost(string, tag = "3")]
    tool_args: String,
}

#[test]
fn tool_start_old_and_new_wire_are_bidirectionally_compatible() {
    let current = proto::ToolStart {
        tool_id: "id".into(),
        tool_name: "read".into(),
        tool_args: "{}".into(),
        phase: Some("input".into()),
        tc_index: Some(2),
    };
    let old = LegacyToolStart::decode(current.encode_to_vec().as_slice()).unwrap();
    assert_eq!(old.tool_id, "id");
    assert_eq!(old.tool_name, "read");
    assert_eq!(old.tool_args, "{}");
    let restored = proto::ToolStart::decode(old.encode_to_vec().as_slice()).unwrap();
    assert_eq!(restored.tool_id, current.tool_id);
    assert_eq!(restored.phase, None);
    assert_eq!(restored.tc_index, None);
}
