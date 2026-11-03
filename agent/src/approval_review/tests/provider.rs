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

/// The client's **own** request timeout (`provider.rs` 179-183): a gateway that
/// accepts the connection and then never answers. `reqwest` reports that as a
/// timeout, not a connection failure, and the arm classifies it as
/// `reviewer_timeout`.
///
/// Note the trap this fixture has to avoid: `lifecycle.rs` already has a passing
/// test named for `reviewer_timeout`, but it exercises the **context** deadline
/// (`context.rs`'s `remaining = REVIEW_TIMEOUT - elapsed`), which is a different
/// line — a test asserting the same string while this arm reads 0 (F-44). So this
/// test calls `assess_at` directly, exactly as the rest of the classification
/// matrix does, leaving the client's timeout as the only deadline in play.
///
/// The 30 s wait is real in production and must not be paid in the suite: the
/// client's timeout is a `tokio` timer, so a paused runtime auto-advances it to
/// firing the moment every task is idle. That is what makes this line testable at
/// all — without it the honest fixture would cost the suite 30 s on every run.
#[tokio::test(start_paused = true)]
async fn a_gateway_that_never_answers_is_classified_as_a_reviewer_timeout() {
    use tokio::io::AsyncReadExt;
    crate::test_support::ensure_fd_headroom();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        // Consume the request, then hold the socket open and answer nothing.
        let mut buffer = [0u8; 4096];
        let _ = socket.read(&mut buffer).await;
        std::future::pending::<()>().await;
        drop(socket);
    });
    let endpoint = crate::system_one::Endpoint {
        url,
        key: "fixture-key".into(),
        model: "jev".into(),
    };

    assert_eq!(
        assess_at(&endpoint, reviewer_state(), "timeout-id")
            .await
            .err(),
        Some("reviewer_timeout"),
        "a request that outlives the client's own timeout is a reviewer timeout"
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_gateway_transport_retries_transient_errors_with_same_identity() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    crate::test_support::ensure_fd_headroom();
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

/// A scripted local gateway for the reviewer transport: answer each connection
/// with the next `(status, body)` pair. `None` means "accept and then drop the
/// connection without answering", which is how a hung/slow upstream behaves.
struct ScriptedGateway {
    url: String,
    requests: std::sync::Arc<parking_lot::Mutex<Vec<Value>>>,
}

async fn scripted_gateway(answers: Vec<(String, String)>) -> ScriptedGateway {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    crate::test_support::ensure_fd_headroom();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
    let requests = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let seen = requests.clone();
    tokio::spawn(async move {
        for (status, body) in answers {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut bytes = Vec::new();
            loop {
                let mut chunk = [0u8; 4096];
                match socket.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => bytes.extend_from_slice(&chunk[..n]),
                }
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                    let length: usize = header
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .and_then(|value| value.trim().parse().ok())
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            if let Some((_, body)) = bytes
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|end| (end, &bytes[end + 4..]))
            {
                if let Ok(value) = serde_json::from_slice::<Value>(body) {
                    seen.lock().push(value);
                }
            }
            let _ = socket
                .write_all(
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await;
            let _ = socket.shutdown().await;
        }
    });
    ScriptedGateway { url, requests }
}

fn endpoint_for(gateway: &ScriptedGateway) -> crate::system_one::Endpoint {
    crate::system_one::Endpoint {
        url: gateway.url.clone(),
        key: "fixture-key".into(),
        model: "jev".into(),
    }
}

fn reviewer_state() -> Value {
    json!({"action":{"command":"pwd"}})
}

/// Every way the reviewer gateway can fail is classified into one of the fixed
/// codes the reviewer contract exposes, so the run loop's retry policy and the
/// user-facing diagnosis never see an unclassified transport failure.
#[tokio::test(flavor = "multi_thread")]
async fn gateway_statuses_bodies_and_transport_failures_map_to_fixed_codes() {
    // 401 and 403 are authentication problems: no retry, and distinguishable
    // from a generic HTTP failure.
    for status in ["401 Unauthorized", "403 Forbidden"] {
        let gateway = scripted_gateway(vec![(status.into(), "{\"error\":\"nope\"}".into())]).await;
        assert_eq!(
            assess_at(&endpoint_for(&gateway), reviewer_state(), "id")
                .await
                .err(),
            Some("reviewer_auth"),
            "{status}"
        );
        assert_eq!(
            gateway.requests.lock().len(),
            1,
            "{status} must not be retried"
        );
    }

    // A request the gateway rejects is our fault: reported, not retried.
    let gateway = scripted_gateway(vec![("400 Bad Request".into(), "{}".into())]).await;
    assert_eq!(
        assess_at(&endpoint_for(&gateway), reviewer_state(), "id")
            .await
            .err(),
        Some("reviewer_http")
    );
    assert_eq!(gateway.requests.lock().len(), 1);

    // A server error *is* retried once; when the retry also fails the caller
    // still gets a classified error rather than the last raw status.
    let gateway = scripted_gateway(vec![
        ("500 Internal Server Error".into(), "[]".into()),
        ("502 Bad Gateway".into(), "[]".into()),
    ])
    .await;
    assert_eq!(
        assess_at(&endpoint_for(&gateway), reviewer_state(), "id")
            .await
            .err(),
        Some("reviewer_http")
    );
    assert_eq!(gateway.requests.lock().len(), 2, "a 5xx is retried once");

    // A success whose body is not the expected JSON.
    let gateway = scripted_gateway(vec![("200 OK".into(), "not json at all".into())]).await;
    assert_eq!(
        assess_at(&endpoint_for(&gateway), reviewer_state(), "id")
            .await
            .err(),
        Some("invalid_response")
    );

    // A success whose answer is not a choice answer: the shape is refused
    // before any probability is trusted.
    let mut wrong_kind = response();
    wrong_kind["answers"]["risk"]["type"] = json!("free_text");
    let gateway = scripted_gateway(vec![("200 OK".into(), wrong_kind.to_string())]).await;
    assert_eq!(
        assess_at(&endpoint_for(&gateway), reviewer_state(), "id")
            .await
            .err(),
        Some("invalid_choice")
    );

    // A success whose body exceeds the 64 KiB budget is refused while
    // streaming instead of being buffered and parsed.
    let oversized = "x".repeat(65 * 1024);
    let gateway = scripted_gateway(vec![("200 OK".into(), oversized)]).await;
    assert_eq!(
        assess_at(&endpoint_for(&gateway), reviewer_state(), "id")
            .await
            .err(),
        Some("response_too_large")
    );

    // Nothing is listening on this port: the connection error is retried once
    // and then classified, never surfaced as a panic or a hang.
    crate::test_support::ensure_fd_headroom();
    let dead = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_url = format!("http://{}/v1/systemone", dead.local_addr().unwrap());
    drop(dead);
    let endpoint = crate::system_one::Endpoint {
        url: dead_url,
        key: "fixture-key".into(),
        model: "jev".into(),
    };
    assert_eq!(
        assess_at(&endpoint, reviewer_state(), "id").await.err(),
        Some("reviewer_connection")
    );
}
