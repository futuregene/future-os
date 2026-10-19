use future_remote_protocol::{invitation::parse_invitation, json::decode_reply, IncomingCmd};
use serde_json::Value;

#[test]
fn shared_protocol_fixtures() {
    let fixture: Value = serde_json::from_str(include_str!("../fixtures/protocol.json")).unwrap();
    assert_eq!(
        fixture["version"], 1,
        "unsupported protocol fixture version"
    );
    let cases = fixture["cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    let mut ids = std::collections::HashSet::new();
    for case in cases {
        let id = case["id"].as_str().unwrap();
        assert!(!id.is_empty() && ids.insert(id), "invalid fixture ID: {id}");
        println!("shared-case {id}");
        match case["kind"].as_str().unwrap() {
            "invitation" => {
                assert_eq!(
                    parse_invitation(
                        case["input"].as_str().unwrap(),
                        case["now"].as_i64().unwrap()
                    )
                    .is_ok(),
                    case["expected"]["rust"].as_bool().unwrap(),
                    "{id}"
                );
                if case["expected"]["rust"] != case["expected"]["typescript"] {
                    assert!(!case["difference"].as_str().unwrap().is_empty(), "{id}");
                }
            }
            "json" => {
                let bytes = if let Some(size) = case["repeatSpace"].as_u64() {
                    let mut bytes = vec![b' '; size as usize];
                    bytes[0] = b'0';
                    bytes
                } else {
                    case["bytes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|byte| u8::try_from(byte.as_u64().unwrap()).unwrap())
                        .collect()
                };
                let decoded = decode_reply::<Value>(&bytes);
                if case["accepted"].as_bool().unwrap() {
                    assert_eq!(decoded.unwrap(), case["expected"], "{id}");
                } else {
                    let error = decoded.unwrap_err();
                    if let Some(message) = case["error"].as_str() {
                        assert_eq!(error.to_string(), message, "{id}");
                    }
                }
            }
            "command" => {
                let command: IncomingCmd = serde_json::from_value(case["input"].clone()).unwrap();
                let normalized = serde_json::to_value(command).unwrap();
                for (name, value) in case["expected"].as_object().unwrap() {
                    assert_eq!(&normalized[name], value, "{id}: {name}");
                }
            }
            "handshake-request" => {
                let request: future_remote_protocol::handshake::HandshakeRequest =
                    serde_json::from_value(case["input"].clone()).unwrap();
                assert_eq!(
                    serde_json::json!({
                        "type": request.kind, "message": request.message,
                        "id": request.id, "pairing": request.pairing,
                    }),
                    case["expected"],
                    "{id}"
                );
            }
            "handshake-confirmation" => {
                let input = &case["input"];
                assert_eq!(
                    future_remote_protocol::handshake::confirmation(
                        input["pairId"].as_str().unwrap(),
                        input["bridgeInstanceId"].as_str().unwrap(),
                        input["presence"].clone(),
                    ),
                    case["expected"],
                    "{id}"
                );
            }
            "handshake-transcript" => {
                use future_remote_protocol::handshake::{
                    handshake_transcript, HandshakeTranscript,
                };
                let input = &case["input"];
                let transcript = HandshakeTranscript {
                    pair_id: input["pairId"].as_str().unwrap(),
                    desktop_id: input["desktopId"].as_str().unwrap(),
                    desktop_public_key: input["desktopPublicKey"].as_str().unwrap(),
                    bridge_instance_id: input["bridgeInstanceId"].as_str().unwrap(),
                    device_id: input["deviceId"].as_str().unwrap(),
                    client_public_key: input["clientPublicKey"].as_str().unwrap(),
                    client_nonce: input["clientNonce"].as_str().unwrap(),
                    desktop_nonce: input["desktopNonce"].as_str().unwrap(),
                };
                assert_eq!(
                    handshake_transcript(&transcript),
                    case["expected"].as_str().unwrap(),
                    "{id}"
                );
            }
            other => panic!("Unsupported shared case {id}: {other}"),
        }
    }
}
