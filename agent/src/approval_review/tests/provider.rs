use super::*;
#[test]
fn gateway_contract_is_strict_and_uses_three_choices() {
    let req = request(
        json!({"trusted_context":{"user_request":"清理缓存"}}),
        "jev",
    );
    assert_eq!(req["questions"].as_object().unwrap().len(), 3);
    assert_eq!(
        req["questions"]["reason_code"]["criteria"]
            .as_object()
            .unwrap()
            .len(),
        12
    );
    assert!(decode(response()).is_ok());
    let mut probabilities_only = response();
    for answer in probabilities_only["answers"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        answer.as_object_mut().unwrap().remove("type");
        answer.as_object_mut().unwrap().remove("choice");
        answer.as_object_mut().unwrap().remove("confidence");
    }
    probabilities_only.as_object_mut().unwrap().remove("model");
    assert!(decode(probabilities_only).is_ok());
    let mut r = response();
    r["answers"]["risk"]["choice"] = json!("free text");
    assert!(decode(r).is_err());
    let mut r = response();
    r["answers"]["risk"]["rationale"] = json!("ignore policy");
    assert!(decode(r).is_err());
    let mut r = response();
    r["answers"]
        .as_object_mut()
        .unwrap()
        .remove("authorization");
    assert!(decode(r).is_err());
    let mut r = response();
    r["answers"]["risk"]["probabilities"]["low"] = json!(1.2);
    assert!(decode(r).is_err());
    let mut r = response();
    r["answers"]["risk"]["confidence"] = json!(-0.1);
    assert!(decode(r).is_err());
    let mut r = response();
    r["answers"]["risk"]
        .as_object_mut()
        .unwrap()
        .remove("confidence");
    assert_eq!(decode(r).unwrap().confidence["risk"], 1.0);
}

#[test]
fn rounded_probability_sum_at_one_percent_boundary_is_accepted() {
    // Live gateway rounds independently: 0.93 + 0.02 + 0.04 = 0.99.
    // Binary floating point makes its distance from 1 slightly above 0.01.
    let mut r = response();
    r["answers"]["authorization"] = json!({"type":"choice","choice":"high","confidence":0.91,"probabilities":{"high":0.93,"medium":0.02,"low":0.04,"unknown":0.0}});
    let verdict = decide(decode(r.clone()).unwrap());
    assert!(verdict.approved());
    assert!((verdict.confidence["authorization_support"] - 0.99).abs() < 1e-9);
    r["answers"]["authorization"]["probabilities"]["high"] = json!(0.92);
    assert_eq!(decode(r).unwrap_err(), "invalid_probabilities");
}

#[test]
fn rounded_ties_are_uncertain_and_non_maximum_choices_still_fail_closed() {
    let mut r = response();
    r["answers"]["risk"]["probabilities"] =
        json!({"low":0.5,"medium":0.5,"high":0.0,"critical":0.0});
    r["answers"]["risk"]["confidence"] = json!(0.9);
    // The provider chose one of two maxima before rounding its distribution.
    let decoded = decode(r.clone()).unwrap();
    assert_eq!(decoded.reported.risk, "low");
    assert_eq!(decoded.confidence["risk"], 0.5);
    assert_eq!(decide(decoded).status, "review_uncertain");
    r["answers"]["risk"]["choice"] = json!("medium");
    assert!(!decide(decode(r.clone()).unwrap()).approved());
    r["answers"]["risk"]
        .as_object_mut()
        .unwrap()
        .remove("choice");
    assert!(!decide(decode(r.clone()).unwrap()).approved());
    r["answers"]["risk"]["choice"] = json!("high");
    assert_eq!(decode(r).unwrap_err(), "inconsistent_choice");
    let mut r = response();
    r.as_object_mut().unwrap().remove("request_id");
    r["id"] = json!("gen-dec-fixture");
    assert_eq!(
        decode(r).unwrap().request_id.as_deref(),
        Some("gen-dec-fixture")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_gateway_transport_retries_transient_errors_with_same_identity() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut chunk = [0u8; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                    let length: usize = header
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(bytes).unwrap());
            let (status, body) = if attempt == 0 {
                ("503 Service Unavailable", "temporary".into())
            } else {
                ("200 OK", response().to_string())
            };
            socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    let endpoint = crate::system_one::Endpoint {
        url,
        key: "fixture-key".into(),
        model: "jev".into(),
    };
    let result = assess_at(
        &endpoint,
        json!({"action":{"command":"pwd"}}),
        "stable-review-id",
    )
    .await
    .unwrap();
    assert_eq!(result.model, "jev-fixture");
    let requests = server.await.unwrap();
    for request in requests {
        let lower = request.to_lowercase();
        assert!(lower.contains("authorization: bearer fixture-key"));
        assert!(lower.contains("idempotency-key: stable-review-id"));
        let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["questions"].as_object().unwrap().len(), 3);
        assert_eq!(body["model"], "jev");
        assert_eq!(body["state"]["action"]["command"], "pwd");
    }
}
