use super::*;

#[cfg(target_os = "macos")]
struct FilePermissionReviewer {
    approved: bool,
    inputs: Arc<Mutex<Vec<Value>>>,
}

#[cfg(target_os = "macos")]
#[async_trait::async_trait]
impl ApprovalReviewerClient for FilePermissionReviewer {
    async fn assess(&self, state: Value, _: &str) -> Result<ProviderAssessment, &'static str> {
        self.inputs.lock().push(state);
        Ok(if self.approved {
            assessment("low", "high", "routine_bounded_action")
        } else {
            assessment("high", "low", "authorization_scope_mismatch")
        })
    }
}

#[cfg(target_os = "macos")]
#[allow(clippy::await_holding_lock)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn automatic_file_permissions_bind_receipts_and_never_create_human_requests() {
    let _home_guard = crate::test_support::home_env_lock();
    for approved in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("workspace");
        let target = directory.path().join("report.txt");
        std::fs::create_dir_all(workspace.join(".future")).unwrap();
        std::fs::write(&target, "old").unwrap();
        std::fs::write(
            workspace.join(".future/approval_rule.json"),
            json!({
                "rules":[{"path":target,"access":"write","action":"ask"}]
            })
            .to_string(),
        )
        .unwrap();
        let cwd = workspace.to_string_lossy().into_owned();
        let sandbox = Arc::new(crate::sandbox::ResolvedSandbox::resolve(
            &crate::sandbox::SandboxPolicy::from_mode("auto"),
            &cwd,
        ));
        if !sandbox.wraps_shell() {
            return;
        }
        let command = format!(
            "printf new > '{}'; echo executed >> executions",
            target.display()
        );
        let arguments = json!({"command":command, "additional_permissions":{"write":[{
            "path":target, "scope":"file", "reason":"Save the remaining report requested by the user"
        }]}});
        let inputs = Arc::new(Mutex::new(Vec::new()));
        let (mut context, _) = context(true, false);
        context.provider = Arc::new(FilePermissionReviewer {
            approved,
            inputs: inputs.clone(),
        });
        let gate = crate::rpc::ApprovalGate::default().with_model_reviewer(context);
        let broadcaster = crate::rpc::SseBroadcaster::new();
        let mut events = broadcaster.subscribe();
        crate::tools::with_tool_scope(
            crate::tools::ScopeOptions {
                workspace: cwd.clone(),
                permission_level: "all".into(),
                interrupt_flag: Arc::new(AtomicBool::new(false)),
                sandbox: sandbox.clone(),
                escalation: None,
                on_sandboxed: None,
            },
            async {
                let blocked = gate.request(
                    &broadcaster,
                    "s",
                    &cwd,
                    "shell",
                    "remaining-call",
                    &arguments,
                    &sandbox,
                );
                if approved {
                    assert!(blocked.is_none());
                    let output = crate::tools::with_tool_call_id(
                        "remaining-call".into(),
                        crate::tools::shell::execute(arguments.clone()),
                    )
                    .await
                    .unwrap();
                    assert!(output.contains("[exit: 0]"), "{output}");
                    // The same arguments cannot reuse the receipt for another call.
                    let output = crate::tools::with_tool_call_id(
                        "later-call".into(),
                        crate::tools::shell::execute(arguments.clone()),
                    )
                    .await
                    .unwrap_err();
                    assert!(output
                        .to_string()
                        .contains("missing an exact approval receipt"));
                } else {
                    assert!(blocked.unwrap().is_error);
                }
            },
        )
        .await;
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            if approved { "new" } else { "old" }
        );
        if approved {
            assert_eq!(
                std::fs::read_to_string(workspace.join("executions"))
                    .unwrap()
                    .lines()
                    .count(),
                1
            );
        } else {
            assert!(!workspace.join("executions").exists());
        }
        assert!(gate.pending_for_session("s").is_empty());
        let inputs = inputs.lock();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0]["host_facts"]["targets"][0]["access"], "write");
        assert_eq!(inputs[0]["host_facts"]["targets"][0]["scope"], "file");
        assert_eq!(
            inputs[0]["host_facts"]["sandbox_boundary"]["inside_sandbox"],
            true
        );
        assert!(inputs[0]["untrusted_context"]
            .to_string()
            .contains("Save the remaining report"));
        while let Ok(event) = events.try_recv() {
            assert_eq!(event.event_type, "approval_assessment");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn preparation_deadline_exhaustion_does_not_call_the_provider() {
    let (ctx, calls) = context(true, false);
    let mut action = prepare_review_action(
        "shell",
        "call",
        &json!({"command":"pwd"}),
        &json!({}),
        &json!({}),
        "/project",
    );
    action.started = std::time::Instant::now() - REVIEW_TIMEOUT - Duration::from_secs(1);
    let outcome = ctx.review_action(action, &Value::Null).await;
    assert_eq!(
        outcome.verdict.error_code.as_deref(),
        Some("reviewer_timeout")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn oversized_commands_keep_size_audit_without_forwarding_partial_commands() {
    let (ctx, calls) = context(true, false);
    let arguments = json!({"command":"private-fixture".repeat(10_000)});
    let action = prepare_review_action(
        "shell",
        "call",
        &arguments,
        &json!({"scope":{"inside_workspace":true}}),
        &json!({}),
        "/project",
    );
    assert!(action.facts["command"].is_null());
    assert!(action.facts["scope"]["inside_workspace"].is_null());
    let outcome = ctx.review_action(action, &arguments).await;
    assert_eq!(
        outcome.verdict.error_code.as_deref(),
        Some("input_too_large")
    );
    assert_eq!(
        outcome.event["input_context"]["raw_input_bytes"]["oversized_command"],
        150_000
    );
    assert!(!outcome.event.to_string().contains("private-fixture"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

struct RecoveringProvider(Arc<AtomicU64>);
#[async_trait::async_trait]
impl ApprovalReviewerClient for RecoveringProvider {
    async fn assess(&self, _: Value, _: &str) -> Result<ProviderAssessment, &'static str> {
        if self.0.fetch_add(1, Ordering::SeqCst) < 4 {
            Err("transport_error")
        } else {
            Ok(assessment("medium", "high", "bounded_external_side_effect"))
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn infrastructure_errors_do_not_consume_model_denial_attempts() {
    let (mut ctx, _) = context(true, false);
    let calls = Arc::new(AtomicU64::new(0));
    ctx.provider = Arc::new(RecoveringProvider(calls.clone()));
    for _ in 0..4 {
        assert_eq!(
            ctx.review(json!({}), "digest".into(), "scope".into(), "tool")["status"],
            "review_error"
        );
    }
    assert_eq!(
        ctx.review(json!({}), "digest".into(), "scope".into(), "tool")["status"],
        "approved"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 5);
}
#[tokio::test(flavor = "multi_thread")]
async fn oversized_mandatory_evidence_never_reaches_provider() {
    let (mut ctx, calls) = context(true, false);
    ctx.user_request = "\n".repeat(5_000);
    let event = ctx.review(json!({}), "digest".into(), "scope".into(), "tool");
    assert_eq!(event["error_code"], "input_too_large");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(
        event["input_context"]["budget"]["estimated_state_tokens"]
            .as_u64()
            .unwrap()
            > 8_000
    );
    assert!(event["input_context"]["source_ids"].is_array());
    assert!(!event["input_context"].to_string().contains("user_message"));
    // A corrected transport-sized input can still be assessed in the same run.
    for _ in 0..4 {
        let event = ctx.review(json!({}), "digest".into(), "scope".into(), "tool");
        assert_eq!(event["error_code"], "input_too_large");
    }
    ctx.user_request = "Continue with the specified file".into();
    assert_eq!(
        ctx.review(json!({}), "digest".into(), "scope".into(), "tool")["status"],
        "approved"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
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
    let assessment: Value = serde_json::from_str(&event.data).unwrap();
    assert_eq!(assessment["tool_call_id"], "shell-1");
    assert_eq!(assessment["policy_version"], POLICY_VERSION);
    assert_eq!(assessment["prompt_version"], PROMPT_VERSION);
    assert!(
        assessment["confidence"]["authorization_support"]
            .as_f64()
            .unwrap()
            >= THRESHOLD
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

#[test]
fn historical_automatic_policy_keeps_os_sandbox_after_logout() {
    let automatic = crate::sandbox::SandboxPolicy::from_mode("auto");
    let signed_in = account_sandbox_policy(automatic.clone(), true);
    assert_eq!(signed_in.mode(), "auto");
    let signed_out = account_sandbox_policy(automatic, false);
    assert_eq!(signed_out.mode(), "sandbox");
    assert_eq!(signed_out.tier, crate::sandbox::SandboxTier::Sandbox);
    for mode in ["off", "manual", "sandbox"] {
        assert_eq!(
            account_sandbox_policy(crate::sandbox::SandboxPolicy::from_mode(mode), false).mode(),
            mode
        );
    }
}
