use super::*;
fn assessment(risk: &str, authorization: &str, reason: &str) -> ProviderAssessment {
    ProviderAssessment {
        reported: Assessment {
            risk: risk.into(),
            authorization: authorization.into(),
            reason_code: reason.into(),
        },
        confidence: ["risk", "authorization", "reason_code"]
            .map(|key| (key.into(), 0.95))
            .into(),
        probabilities: Value::Null,
        model: "jev-fixture".into(),
        request_id: Some("provider-1".into()),
    }
}
#[test]
fn matrix_and_reason_invariants_are_deterministic() {
    for (ri, (risk, _)) in RISKS.iter().enumerate() {
        for (ai, (auth, _)) in AUTHORIZATIONS.iter().enumerate() {
            let expected = match ri {
                0 => true,
                1 => ai >= 2,
                2 => ai == 3,
                _ => false,
            };
            assert_eq!(
                decide(assessment(risk, auth, "routine_bounded_action")).approved(),
                expected,
                "{risk}/{auth}"
            );
        }
    }
    for (reason, _) in REASONS {
        let verdict = decide(assessment("low", "high", reason));
        match *reason {
            "insufficient_information" => {
                assert_eq!(verdict.status, "review_uncertain");
                assert!(verdict.effective["risk"].is_null());
            }
            "authorization_scope_mismatch" => {
                assert_eq!(verdict.effective["authorization"], "low");
                assert!(!verdict.approved());
            }
            "catastrophic_system_destruction"
            | "irreversible_critical_data_loss"
            | "loss_of_security_control" => assert!(!verdict.approved()),
            "routine_bounded_action" | "bounded_external_side_effect" => {
                assert!(verdict.approved())
            }
            _ => {
                assert_eq!(verdict.effective["risk"], "high");
                assert!(verdict.approved());
            }
        }
    }
}
#[test]
fn confidence_can_only_tighten_and_is_finite() {
    for c in [0.749, 0.75, 0.751, f64::NAN, 1.1] {
        let mut a = assessment("high", "high", "protected_secret_access");
        a.confidence.insert("risk".into(), c);
        assert_eq!(
            decide(a).approved(),
            c.is_finite() && (0.75..=1.0).contains(&c)
        );
    }
    let mut a = assessment("critical", "high", "routine_bounded_action");
    a.confidence.clear();
    assert_eq!(decide(a).status, "rejected");
}
fn response() -> Value {
    let mut answers = serde_json::Map::new();
    for (name, options) in [
        ("risk", RISKS),
        ("authorization", AUTHORIZATIONS),
        ("reason_code", REASONS),
    ] {
        answers.insert(name.into(),json!({"type":"choice","choice":options[0].0,"confidence":0.9,"probabilities":options.iter().enumerate().map(|(i,(key,_))| (key.to_string(),json!(if i==0 {1.0}else{0.0}))).collect::<serde_json::Map<String,Value>>()}));
    }
    json!({"model":"jev-fixture","answers":answers,"usage":{"input_tokens":123}})
}
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
fn digest_covers_exact_execution_but_audit_redacts_secrets() {
    let boundary = json!({"cwd":"/project","execution":"outside_sandbox_once"});
    let raw = json!({"command":"curl https://host/path?token=hidden -H 'Authorization: Bearer hidden'","env":{"API_KEY":"never-send"}});
    let (facts, digest, _) = action(
        "shell",
        "call1",
        &raw,
        &json!({"category":"process"}),
        &boundary,
        "/project",
    );
    assert!(!facts.to_string().contains("hidden"));
    assert!(!facts.to_string().contains("never-send"));
    let mut other = raw.clone();
    other["env"]["API_KEY"] = json!("changed");
    assert_ne!(
        digest,
        action("shell", "call1", &other, &json!({}), &boundary, "/project").1
    );
    let reversed = json!({"env":{"API_KEY":"never-send"},"command":raw["command"]});
    assert_eq!(
        digest,
        action(
            "shell",
            "call1",
            &reversed,
            &json!({}),
            &boundary,
            "/project"
        )
        .1
    );
}
struct Fake {
    calls: Arc<AtomicU64>,
    status: bool,
    hang: bool,
}
#[async_trait::async_trait]
impl ApprovalReviewerClient for Fake {
    async fn assess(&self, _: Value, _: &str) -> Result<ProviderAssessment, &'static str> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.hang {
            std::future::pending::<()>().await;
        }
        if self.status {
            Ok(assessment("medium", "high", "bounded_external_side_effect"))
        } else {
            Ok(assessment("high", "low", "broad_destructive_action"))
        }
    }
}
fn context(status: bool, hang: bool) -> (ReviewContext, Arc<AtomicU64>) {
    let calls = Arc::new(AtomicU64::new(0));
    let mut ctx = ReviewContext::new(
        "修改指定文件".into(),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicU64::new(0)),
        Arc::new(Mutex::new(HashMap::new())),
    );
    ctx.provider = Arc::new(Fake {
        calls: calls.clone(),
        status,
        hang,
    });
    (ctx, calls)
}
#[tokio::test(flavor = "multi_thread")]
async fn cancelled_or_stale_reviews_never_execute_and_drop_provider_future() {
    let (ctx, calls) = context(true, true);
    let cancel = ctx.cancelled.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.store(true, Ordering::SeqCst);
    });
    let v = ctx.review(json!({}), "digest".into(), "scope".into(), "tool");
    assert_eq!(v["status"], "cancelled");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let (ctx, calls) = context(true, false);
    ctx.generation.fetch_add(1, Ordering::SeqCst);
    assert_eq!(
        ctx.review(json!({}), "digest".into(), "scope".into(), "tool")["status"],
        "stale_request"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[tokio::test(flavor = "multi_thread")]
async fn denials_are_bounded_and_annotations_are_separate_from_output() {
    let (ctx, calls) = context(false, false);
    for n in 0..4 {
        let v = ctx.review(
            json!({"command":format!("rm data-{n}")}),
            "digest".into(),
            "scope".into(),
            "tool",
        );
        assert_eq!(v["status"], "rejected");
        if n == 3 {
            assert_eq!(v["error_code"], "repeated_denial");
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        ctx.annotations.lock()["tool"]["type"],
        "auto_approval_result"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn auto_gate_bypasses_allow_and_deny_and_never_parks_an_ask() {
    use crate::rpc::{ApprovalGate, SseBroadcaster};
    use crate::sandbox::{ResolvedSandbox, SandboxPolicy};
    let (ctx, calls) = context(true, false);
    let gate = ApprovalGate::default().with_model_reviewer(ctx);
    let workspace = crate::test_support::host_absolute_path("/project")
        .to_string_lossy()
        .into_owned();
    let outside = crate::test_support::host_absolute_path("/outside/file");
    let sandbox = ResolvedSandbox::resolve(&SandboxPolicy::from_mode("manual"), &workspace);
    let broadcaster = SseBroadcaster::new();
    assert!(gate
        .request(
            &broadcaster,
            "s",
            &workspace,
            "read",
            "a",
            &json!({"path":format!("{workspace}/README")}),
            &sandbox
        )
        .is_none());
    assert!(
        gate.request(
            &broadcaster,
            "s",
            &workspace,
            "write",
            "b",
            &json!({"path":format!("{workspace}/.future/approval_rule.json")}),
            &sandbox
        )
        .unwrap()
        .is_error
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(gate
        .request(
            &broadcaster,
            "s",
            &workspace,
            "write",
            "c",
            &json!({"path":outside,"content":"body"}),
            &sandbox
        )
        .is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(gate.pending_for_session("s").is_empty());
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
    let endpoint = crate::skill_reco::Endpoint {
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

#[tokio::test(flavor = "multi_thread")]
async fn shell_review_keeps_tool_identity_and_failed_audit_cannot_authorize() {
    use crate::rpc::{ApprovalGate, SseBroadcaster};
    use crate::sandbox::{
        EscalationDecision, EscalationRequest, EscalationTrigger, ResolvedSandbox, SandboxPolicy,
    };
    let (ctx, _) = context(true, false);
    let annotations = ctx.annotations.clone();
    let gate = ApprovalGate::default().with_model_reviewer(ctx);
    let workspace = crate::test_support::host_absolute_path("/project")
        .to_string_lossy()
        .into_owned();
    let sandbox = ResolvedSandbox::resolve(&SandboxPolicy::from_mode("manual"), &workspace);
    let broadcaster = SseBroadcaster::new();
    let mut receiver = broadcaster.subscribe();
    let request = EscalationRequest {
        trigger: EscalationTrigger::ModelRequest,
        command: "pwd".into(),
        justification: String::new(),
        failure_summary: String::new(),
    };
    let result = crate::tools::with_tool_call_id("shell-1".into(), async {
        gate.request_escalation(&broadcaster, "s", &request, &sandbox)
    })
    .await;
    assert!(matches!(result, EscalationDecision::Approved));
    let event = receiver.try_recv().unwrap();
    assert_eq!(event.event_type, "approval_assessment");
    assert_eq!(
        serde_json::from_str::<Value>(&event.data).unwrap()["tool_call_id"],
        "shell-1"
    );
    assert!(annotations.lock().contains_key("shell-1"));
    let directory = tempfile::tempdir().unwrap();
    broadcaster
        .configure_journal(
            "s",
            &crate::session::Manager::new(directory.path().to_path_buf()),
        )
        .unwrap();
    broadcaster.start_run("run".into(), 1);
    broadcaster.fail_next_append();
    let result = crate::tools::with_tool_call_id("shell-2".into(), async {
        gate.request_escalation(&broadcaster, "s", &request, &sandbox)
    })
    .await;
    assert!(matches!(result, EscalationDecision::Denied(_)));
    assert_eq!(
        annotations.lock()["shell-2"]["error_code"],
        "audit_unavailable"
    );
    assert!(gate.pending_for_session("s").is_empty());
}
