use super::*;
#[test]
fn decide_rejects_wrong_session_but_accepts_owning_session() {
    use std::sync::mpsc;
    let gate = ApprovalGate::default();
    let (tx, rx) = mpsc::channel();
    gate.pending.lock().insert(
        "req1".to_string(),
        PendingApproval {
            session_id: "sessA".to_string(),
            payload: serde_json::json!({"approval_request_id": "req1"}),
            tx,
        },
    );
    let decision = ApprovalDecision {
        approved: true,
        note: String::new(),
        status: ApprovalDecisionStatus::Approved,
    };
    // A different session cannot approve this request (auth I1 exception);
    // the pending request must remain so the owning session still can.
    assert!(gate.decide("req1", "sessB", decision.clone()).is_err());
    assert!(gate.pending.lock().contains_key("req1"));
    // The owning session's decision goes through and is delivered.
    assert!(gate.decide("req1", "sessA", decision).is_ok());
    assert!(!gate.pending.lock().contains_key("req1"));
    assert_eq!(
        rx.try_recv().unwrap().status,
        ApprovalDecisionStatus::Approved
    );
}

// ─── shell_auto_allow ──────────────────────────────────────────────────

#[test]
fn decide_unknown_request_errors() {
    let gate = ApprovalGate::default();
    let decision = ApprovalDecision {
        approved: false,
        note: String::new(),
        status: ApprovalDecisionStatus::Rejected,
    };
    assert!(gate.decide("nope", "sessA", decision).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_shell_read_only_auto_allows() {
    let ws = temp_ws("shell-ro");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let broadcaster = SseBroadcaster::new();
    let result = gate.request(
        &broadcaster,
        "s1",
        &ws,
        "shell",
        "t1",
        &serde_json::json!({"command": "pwd"}),
        &sandbox,
    );
    assert!(result.is_none(), "read-only shell bypasses the prompt");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_shell_ask_approved() {
    let ws = temp_ws("shell-ask");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let broadcaster = SseBroadcaster::new();
    let decider = spawn_decider(&gate, "s1", approved());
    let result = gate.request(
        &broadcaster,
        "s1",
        &ws,
        "shell",
        "t1",
        &serde_json::json!({"command": "rm -rf /tmp/some-build-dir"}),
        &sandbox,
    );
    assert!(decider.join().unwrap(), "approval request never appeared");
    assert!(result.is_none(), "approved shell runs");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_shell_ask_rejected_with_note() {
    let ws = temp_ws("shell-rej");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let broadcaster = SseBroadcaster::new();
    let decider = spawn_decider(
        &gate,
        "s1",
        ApprovalDecision {
            approved: false,
            note: "too dangerous".to_string(),
            status: ApprovalDecisionStatus::Rejected,
        },
    );
    let result = gate.request(
        &broadcaster,
        "s1",
        &ws,
        "shell",
        "t1",
        &serde_json::json!({"command": "rm -rf /tmp/some-build-dir"}),
        &sandbox,
    );
    assert!(decider.join().unwrap(), "approval request never appeared");
    let denial = result.expect("rejection returns a tool result");
    assert!(denial.is_error);
    assert!(denial
        .result
        .contains("rejected by the user: too dangerous"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_shell_ask_cancelled() {
    let ws = temp_ws("shell-cancel");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let broadcaster = SseBroadcaster::new();
    let decider = spawn_decider(
        &gate,
        "s1",
        ApprovalDecision {
            approved: false,
            note: String::new(),
            status: ApprovalDecisionStatus::Cancelled,
        },
    );
    let result = gate.request(
        &broadcaster,
        "s1",
        &ws,
        "shell",
        "t1",
        &serde_json::json!({"command": "rm -rf /tmp/some-build-dir"}),
        &sandbox,
    );
    assert!(decider.join().unwrap(), "approval request never appeared");
    let cancellation = result.expect("cancel returns a tool result");
    assert!(cancellation.is_error);
    assert!(cancellation.result.contains("cancelled"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_write_outside_workspace_ask_approved() {
    let _home_guard = crate::test_support::home_env_lock();
    let ws = temp_ws("write-approve");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let broadcaster = SseBroadcaster::new();
    let outside = dirs::home_dir()
        .unwrap()
        .join(format!("futureos-approval-{}.txt", std::process::id()));
    let decider = spawn_decider(&gate, "s1", approved());
    let result = gate.request(
        &broadcaster,
        "s1",
        &ws,
        "write",
        "t1",
        &serde_json::json!({"path": outside.to_string_lossy(), "content": "x"}),
        &sandbox,
    );
    assert!(decider.join().unwrap(), "approval request never appeared");
    assert!(result.is_none(), "approved write proceeds");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_write_outside_workspace_rejected() {
    let _home_guard = crate::test_support::home_env_lock();
    let ws = temp_ws("write-reject");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let broadcaster = SseBroadcaster::new();
    let outside = dirs::home_dir()
        .unwrap()
        .join(format!("futureos-approval-rej-{}.txt", std::process::id()));
    let decider = spawn_decider(
        &gate,
        "s1",
        ApprovalDecision {
            approved: false,
            note: String::new(),
            status: ApprovalDecisionStatus::Rejected,
        },
    );
    let result = gate.request(
        &broadcaster,
        "s1",
        &ws,
        "write",
        "t1",
        &serde_json::json!({"path": outside.to_string_lossy(), "content": "x"}),
        &sandbox,
    );
    assert!(decider.join().unwrap(), "approval request never appeared");
    let denial = result.expect("rejection returns a tool result");
    assert!(denial.is_error);
    assert!(denial.result.contains("rejected by the user."));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_read_of_secret_asks_and_is_rejected() {
    let _home_guard = crate::test_support::home_env_lock();
    let ws = temp_ws("read-secret");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let broadcaster = SseBroadcaster::new();
    let secret = dirs::home_dir().unwrap().join(".ssh/id_rsa");
    // Secrets are Ask (never auto-allowed); the user rejects.
    let decider = spawn_decider(
        &gate,
        "s1",
        ApprovalDecision {
            approved: false,
            note: String::new(),
            status: ApprovalDecisionStatus::Rejected,
        },
    );
    let result = gate.request(
        &broadcaster,
        "s1",
        &ws,
        "read",
        "t1",
        &serde_json::json!({"path": secret.to_string_lossy()}),
        &sandbox,
    );
    assert!(decider.join().unwrap(), "approval request never appeared");
    let denial = result.expect("rejection returns a tool result");
    assert!(denial.is_error);
    assert!(denial.result.contains("rejected"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_non_file_tool_and_missing_path_pass_through() {
    let ws = temp_ws("passthrough");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let broadcaster = SseBroadcaster::new();
    // Unknown tool names are not gated.
    assert!(gate
        .request(
            &broadcaster,
            "s1",
            &ws,
            "web_search",
            "t1",
            &serde_json::json!({"query": "x"}),
            &sandbox,
        )
        .is_none());
    // A file tool without a path argument cannot be evaluated → pass.
    assert!(gate
        .request(
            &broadcaster,
            "s1",
            &ws,
            "write",
            "t2",
            &serde_json::json!({"content": "no path"}),
            &sandbox,
        )
        .is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_escalation_approved_and_denied() {
    let ws = temp_ws("escalation");
    let sandbox = enabled(&ws);
    let broadcaster = SseBroadcaster::new();

    let gate = ApprovalGate::default();
    let decider = spawn_decider(&gate, "s1", approved());
    let decision = gate.request_escalation(
        &broadcaster,
        "s1",
        &crate::sandbox::EscalationRequest {
            trigger: crate::sandbox::EscalationTrigger::SandboxFailure,
            command: "touch /System/x".to_string(),
            justification: "need it".to_string(),
            failure_summary: "touch: /System/x: Operation not permitted".to_string(),
        },
        &sandbox,
    );
    assert!(decider.join().unwrap(), "approval request never appeared");
    assert!(matches!(
        decision,
        crate::sandbox::EscalationDecision::Approved
    ));

    let gate = ApprovalGate::default();
    let decider = spawn_decider(
        &gate,
        "s1",
        ApprovalDecision {
            approved: false,
            note: "stay sandboxed".to_string(),
            status: ApprovalDecisionStatus::Rejected,
        },
    );
    let decision = gate.request_escalation(
        &broadcaster,
        "s1",
        &crate::sandbox::EscalationRequest {
            trigger: crate::sandbox::EscalationTrigger::ModelRequest,
            command: "touch /System/x".to_string(),
            justification: "need it".to_string(),
            failure_summary: String::new(),
        },
        &sandbox,
    );
    assert!(decider.join().unwrap(), "approval request never appeared");
    match decision {
        crate::sandbox::EscalationDecision::Denied(note) => {
            assert_eq!(note, "stay sandboxed")
        }
        crate::sandbox::EscalationDecision::Approved => panic!("must be denied"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_session_ends_pending_request() {
    let ws = temp_ws("cancel-pending");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let broadcaster = SseBroadcaster::new();
    let requester_gate = gate.clone();
    let requester_ws = ws.clone();
    let requester = std::thread::spawn(move || {
        requester_gate.request(
            &broadcaster,
            "s1",
            &requester_ws,
            "shell",
            "t1",
            &serde_json::json!({"command": "rm -rf /tmp/some-build-dir"}),
            &sandbox,
        )
    });
    // Wait for the request to land, then cancel the whole session.
    for _ in 0..500 {
        if !gate.pending_for_session("s1").is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let cancelled = gate.cancel_session("s1", "session closed");
    assert_eq!(cancelled, 1);
    let result = requester.join().unwrap();
    let cancellation = result.expect("cancel produces a tool result");
    assert!(cancellation.is_error);
    assert!(cancellation.result.contains("cancelled"));
    assert!(gate.pending_for_session("s1").is_empty());
}

#[test]
fn poll_and_decide_times_out_when_no_request_appears() {
    // The give-up path: no request ever lands for the session.
    let gate = ApprovalGate::default();
    assert!(!poll_and_decide(&gate, "nobody", approved(), 2));
}

#[test]
fn decide_on_unknown_or_consumed_request_fails() {
    let gate = ApprovalGate::default();
    let _rx = gate.insert_pending_for_test("ap-once", "s1");
    gate.decide("ap-once", "s1", approved()).unwrap();
    // The entry was consumed — a second decision fails.
    let again = gate.decide("ap-once", "s1", approved());
    assert!(again.unwrap_err().contains("not pending"));
}

// ─── coverage batch 14: residual decision arms ─────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_shell_passes_through_when_sandbox_wraps_shell() {
    let ws = temp_ws("shell-wrapped");
    let mut sandbox = ResolvedSandbox::resolve(
        &SandboxPolicy {
            tier: crate::sandbox::SandboxTier::Sandbox,
            model_reviewer: false,
        },
        &ws,
    );
    // Force availability so the wrap check is platform-independent.
    sandbox.set_backend_available_for_test(true);
    let gate = ApprovalGate::default();
    let broadcaster = SseBroadcaster::new();
    // A command that would ASK under the manual tier runs ungated here:
    // the Seatbelt boundary is the enforcement point instead.
    let result = gate.request(
        &broadcaster,
        "s1",
        &ws,
        "shell",
        "t1",
        &serde_json::json!({"command": "rm -rf /tmp/some-build-dir"}),
        &sandbox,
    );
    assert!(result.is_none(), "wrapped shell never pre-asks");
    assert!(gate.pending_for_session("s1").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_shell_ask_rejected_without_note() {
    let ws = temp_ws("shell-rej-plain");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let broadcaster = SseBroadcaster::new();
    let decider = spawn_decider(
        &gate,
        "s1",
        ApprovalDecision {
            approved: false,
            note: String::new(),
            status: ApprovalDecisionStatus::Rejected,
        },
    );
    let result = gate.request(
        &broadcaster,
        "s1",
        &ws,
        "shell",
        "t1",
        &serde_json::json!({"command": "rm -rf /tmp/some-build-dir"}),
        &sandbox,
    );
    assert!(decider.join().unwrap(), "approval request never appeared");
    let denial = result.expect("rejection returns a tool result");
    assert!(denial.result.contains("rejected by the user."));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_file_tool_cancelled_and_rejected_with_note() {
    let _home_guard = crate::test_support::home_env_lock();
    let ws = temp_ws("file-cancel");
    let sandbox = enabled(&ws);
    let broadcaster = SseBroadcaster::new();
    let outside = dirs::home_dir()
        .unwrap()
        .join(format!("futureos-approval-fc-{}.txt", std::process::id()));
    let args = serde_json::json!({"path": outside.to_string_lossy(), "content": "x"});

    // Cancelled: the approval card went away without a decision.
    let gate = ApprovalGate::default();
    let decider = spawn_decider(
        &gate,
        "s1",
        ApprovalDecision {
            approved: false,
            note: String::new(),
            status: ApprovalDecisionStatus::Cancelled,
        },
    );
    let result = gate.request(&broadcaster, "s1", &ws, "write", "t1", &args, &sandbox);
    assert!(decider.join().unwrap(), "approval request never appeared");
    let cancel = result.expect("cancel returns a tool result");
    assert!(cancel.result.contains("approval request ended"));

    // Rejected with a note: the note is appended after a colon.
    let gate = ApprovalGate::default();
    let decider = spawn_decider(
        &gate,
        "s1",
        ApprovalDecision {
            approved: false,
            note: "not today".to_string(),
            status: ApprovalDecisionStatus::Rejected,
        },
    );
    let result = gate.request(&broadcaster, "s1", &ws, "write", "t2", &args, &sandbox);
    assert!(decider.join().unwrap(), "approval request never appeared");
    let denial = result.expect("rejection returns a tool result");
    assert!(denial.result.contains("rejected by the user: not today."));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_escalation_cancelled_variants() {
    let ws = temp_ws("esc-cancel");
    let sandbox = enabled(&ws);
    let broadcaster = SseBroadcaster::new();
    let request = crate::sandbox::EscalationRequest {
        trigger: crate::sandbox::EscalationTrigger::ModelRequest,
        command: "touch /System/x".to_string(),
        justification: String::new(),
        failure_summary: String::new(),
    };

    // Empty note → the generic "approval request ended" denial reason.
    let gate = ApprovalGate::default();
    let decider = spawn_decider(
        &gate,
        "s1",
        ApprovalDecision {
            approved: false,
            note: String::new(),
            status: ApprovalDecisionStatus::Cancelled,
        },
    );
    let decision = gate.request_escalation(&broadcaster, "s1", &request, &sandbox);
    assert!(decider.join().unwrap(), "approval request never appeared");
    assert!(matches!(
        decision,
        crate::sandbox::EscalationDecision::Denied(ref note) if note == "approval request ended"
    ));

    // Non-empty note → the note is the denial reason.
    let gate = ApprovalGate::default();
    let decider = spawn_decider(
        &gate,
        "s1",
        ApprovalDecision {
            approved: false,
            note: "window closed".to_string(),
            status: ApprovalDecisionStatus::Cancelled,
        },
    );
    let decision = gate.request_escalation(&broadcaster, "s1", &request, &sandbox);
    assert!(decider.join().unwrap(), "approval request never appeared");
    assert!(matches!(
        decision,
        crate::sandbox::EscalationDecision::Denied(ref note) if note == "window closed"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolve_approval_sender_drop_reads_as_session_end() {
    // The session-torn-down arm: the pending entry (and its sender) is
    // dropped without a decision, so the blocked receiver errors.
    let ws = temp_ws("ask-drop");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let broadcaster = SseBroadcaster::new();
    let requester_gate = gate.clone();
    let requester = std::thread::spawn(move || {
        requester_gate.request(
            &broadcaster,
            "s1",
            &ws,
            "shell",
            "t1",
            &serde_json::json!({"command": "rm -rf /tmp/some-build-dir"}),
            &sandbox,
        )
    });
    let mut request_id = String::new();
    for _ in 0..2000 {
        let pending = gate.pending_for_session("s1");
        if let Some(first) = pending.first() {
            request_id = first["approval_request_id"].as_str().unwrap().to_string();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!request_id.is_empty(), "approval request never appeared");
    // Dropping the entry drops the sender — the waiter observes Err.
    let removed = gate.pending.lock().remove(&request_id);
    assert!(removed.is_some());
    drop(removed);
    let result = requester.join().unwrap();
    // The dropped sender surfaced as a cancellation (the shell path uses
    // the generic cancel message; the "session ended" note is broadcast).
    let cancel = result.expect("drop produces a cancel tool result");
    assert!(cancel.result.contains("cancelled"));
    assert!(gate.pending_for_session("s1").is_empty());
}

#[test]
fn additional_permissions_fail_closed_when_windows_backend_is_inactive() {
    let ws = temp_ws("windows-capability-inactive");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let broadcaster = SseBroadcaster::new();
    let result = gate.request(
        &broadcaster,
        "session",
        &ws,
        "shell",
        "tool",
        &serde_json::json!({
            "command": "build-release",
            "additional_permissions": {
                "write": [{
                    "path": ws,
                    "scope": "subtree",
                    "reason": "build output"
                }]
            }
        }),
        &sandbox,
    );
    let result = result.expect("inactive backend must reject before execution");
    assert!(result.is_error);
    assert!(result.result.contains("backend is not active"));
}

#[test]
fn additional_permissions_empty_write_falls_through_to_manual_shell() {
    // An empty write list must NOT enter the Windows-capability path — it
    // falls through to the normal manual-tier shell approval.
    let ws = temp_ws("windows-capability-empty");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let broadcaster = SseBroadcaster::new();
    let result = gate.request(
        &broadcaster,
        "session",
        &ws,
        "shell",
        "tool",
        &serde_json::json!({
            "command": "pwd",
            "additional_permissions": {"write": []}
        }),
        &sandbox,
    );
    // pwd is a literal builtin → auto-allowed without a prompt.
    assert!(result.is_none());
    let result = gate.request(
        &broadcaster,
        "session",
        &ws,
        "shell",
        "tool",
        &serde_json::json!({"command":"pwd", "additional_permissions":null}),
        &sandbox,
    );
    assert!(
        result.is_none(),
        "null optional permissions must behave like absence"
    );
}

#[test]
fn additional_permissions_invalid_shape_is_rejected() {
    // The additional_permissions field must deserialize into the
    // AdditionalPermissions struct; a malformed payload fails fast with a
    // tool result instead of falling through to a plain command approval.
    let ws = temp_ws("windows-capability-invalid");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let broadcaster = SseBroadcaster::new();
    let result = gate.request(
        &broadcaster,
        "session",
        &ws,
        "shell",
        "tool",
        &serde_json::json!({
            "command": "build-release",
            "additional_permissions": {
                "write": "not-an-array"
            }
        }),
        &sandbox,
    );
    let result = result.expect("invalid additional_permissions is rejected");
    assert!(result.is_error);
    assert!(result.result.contains("invalid additional_permissions"));
}
