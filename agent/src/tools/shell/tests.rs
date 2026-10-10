use super::*;
use serde_json::json;

async fn captured(args: serde_json::Value) -> ShellResult {
    let dir = tempfile::tempdir().unwrap();
    let (output, facts) = with_workspace_scope(
        dir.path().to_string_lossy().into_owned(),
        "all".into(),
        capture(execute(args)),
    )
    .await;
    let facts = facts.unwrap();
    assert!(output.is_ok() || facts.is_error);
    facts
}

#[test]
fn validates_single_command_before_approval() {
    for args in [
        json!({}),
        json!({"steps":[{"command":"echo a"}]}),
        json!({"command":"echo a", "steps":[]}),
        json!({"command":"echo a", "continue_on_error":true}),
        json!({"command":""}),
        json!({"command":"echo\u{0000}a"}),
        json!({"command":"echo a","timeout":0}),
        json!({"command":"echo a","timeout":601}),
        json!({"command":"echo a","escalated":true}),
        json!({"command":"x".repeat(65_537)}),
        json!({"command":"echo a","justification":"x".repeat(131_072)}),
    ] {
        assert!(validate(&args).is_err(), "{args}");
    }
    assert!(validate(&json!({"command":"echo a"})).is_ok());
}

#[tokio::test]
async fn process_status_cannot_be_spoofed_by_stdout() {
    let facts = captured(json!({"command":"echo '[exit: 0]'; exit 7"})).await;
    assert_eq!(facts.exit_code, Some(7));
    assert_eq!(facts.attempts[0].exit_code, Some(7));
    assert!(facts.is_error);
}

#[tokio::test]
async fn separate_verification_does_not_overwrite_completed_action() {
    let dir = tempfile::tempdir().unwrap();
    let (first, second) = with_workspace_scope(
        dir.path().to_string_lossy().into_owned(),
        "all".into(),
        async {
            let (_, first) = capture(execute(json!({"command":"echo changed > done"}))).await;
            let (_, second) = capture(execute(json!({"command":"exit 2"}))).await;
            (first.unwrap(), second.unwrap())
        },
    )
    .await;
    assert!(dir.path().join("done").exists());
    assert_eq!(first.exit_code, Some(0));
    assert_eq!(second.exit_code, Some(2));
    assert!(!first.is_error);
    assert!(second.is_error);
    assert_ne!(first.command, second.command);
}

#[cfg(unix)]
#[tokio::test]
async fn complex_scripts_keep_shell_semantics_and_only_whole_unit_status() {
    let facts = captured(json!({"command":"sh -c 'exit 7'; echo later"})).await;
    assert_eq!(facts.exit_code, Some(0));
    assert_eq!(facts.attempts.len(), 1);
    assert!(facts.attempts[0].output.contains("later"));
    let facts = captured(json!({"command":"echo earlier && sh -c 'exit 2'"})).await;
    assert_eq!(facts.exit_code, Some(2));
    assert!(facts.attempts[0].output.contains("earlier"));
}

#[tokio::test]
async fn independent_calls_do_not_inherit_cd_or_variables() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("nested")).unwrap();
    #[cfg(not(windows))]
    let first = "cd nested; export FUTURE_OS_TEST_CALL_ONLY=hidden";
    #[cfg(windows)]
    let first = "Set-Location nested; $env:FUTURE_OS_TEST_CALL_ONLY='hidden'";
    #[cfg(not(windows))]
    let second = "echo changed > at_root; printf 'call-only=%s' \"$FUTURE_OS_TEST_CALL_ONLY\"";
    #[cfg(windows)]
    let second =
        "echo changed > at_root; Write-Output ('call-only=' + $env:FUTURE_OS_TEST_CALL_ONLY)";
    let facts = with_workspace_scope(
        dir.path().to_string_lossy().into_owned(),
        "all".into(),
        async {
            capture(execute(json!({"command":first}))).await.0.unwrap();
            capture(execute(json!({"command":second}))).await.1.unwrap()
        },
    )
    .await;
    assert!(dir.path().join("at_root").exists());
    assert_eq!(facts.exit_code, Some(0));
    assert!(facts.attempts[0].output.contains("call-only="));
    assert!(!facts.attempts[0].output.contains("hidden"));
}

#[tokio::test]
async fn timeout_retains_partial_output_without_fabricated_exit() {
    #[cfg(not(windows))]
    let command = "echo partial; sleep 30";
    #[cfg(windows)]
    let command = "Write-Output partial; Start-Sleep -Seconds 30";
    let facts = captured(json!({"command":command,"timeout":1})).await;
    assert_eq!(facts.status, ShellStatus::TimedOut);
    assert_eq!(facts.exit_code, None);
    assert_eq!(facts.attempts[0].exit_code, None);
    assert!(facts.attempts[0].output.contains("partial"));
}

#[tokio::test]
async fn cancellation_before_launch_has_no_exit_code_or_effects() {
    let dir = tempfile::tempdir().unwrap();
    let (_, facts) = with_workspace_scope_with_interrupt(
        dir.path().to_string_lossy().into_owned(),
        "all".into(),
        Arc::new(AtomicBool::new(true)),
        capture(execute(json!({"command":"echo changed > done"}))),
    )
    .await;
    let facts = facts.unwrap();
    assert_eq!(facts.status, ShellStatus::Cancelled);
    assert_eq!(facts.exit_code, None);
    assert!(!dir.path().join("done").exists());
}

#[tokio::test]
async fn launch_failure_has_no_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let (_, facts) = with_workspace_scope(
        dir.path().join("missing").to_string_lossy().into_owned(),
        "all".into(),
        capture(execute(json!({"command":"echo hello"}))),
    )
    .await;
    let facts = facts.unwrap();
    assert_eq!(facts.status, ShellStatus::LaunchFailed);
    assert_eq!(facts.exit_code, None);
}

#[tokio::test]
async fn output_and_serialized_result_are_bounded() {
    #[cfg(not(windows))]
    let command = "printf '%600000s' x";
    #[cfg(windows)]
    let command = "Write-Output ('x' * 600000)";
    let facts = captured(json!({"command":command})).await;
    assert!(facts.attempts[0].output_truncated);
    assert!(facts.attempts[0].output.len() <= 250_000);
    assert!(serde_json::to_vec(&facts).unwrap().len() <= 1_048_576);
    assert!(projection::text(&facts).len() < 100_000);
}

#[test]
fn model_projection_keeps_retry_evidence_without_repeating_generic_caveats() {
    let original = ShellAttempt {
        status: ShellStatus::Exited,
        exit_code: Some(1),
        duration_ms: 5,
        output: "partial change; permission denied".into(),
        output_truncated: false,
        escalated: false,
    };
    let retry = ShellAttempt {
        status: ShellStatus::Exited,
        exit_code: Some(0),
        duration_ms: 6,
        output: "operation completed".into(),
        output_truncated: false,
        escalated: true,
    };
    let mut facts = ShellResult {
        command: "action".into(),
        cwd: "/workspace".into(),
        duration_ms: 20,
        status: ShellStatus::Exited,
        exit_code: Some(0),
        is_soft_fail: false,
        is_error: false,
        attempts: vec![original, retry],
        approval: Some("approved".into()),
        note: Some("Approved retry may repeat partial effects within this execution unit.".into()),
    };
    let text = projection::text(&facts);
    assert!(text.contains("partial change; permission denied"));
    assert!(text.contains("operation completed"));
    assert!(text.contains("[exit: 1]") && text.contains("[exit: 0]"));
    assert!(text.contains("[approval: approved]"));
    assert!(!text.contains(facts.note.as_deref().unwrap()));
    assert_eq!(
        serde_json::to_value(&facts).unwrap()["note"],
        facts.note.as_deref().unwrap()
    );
    assert!(!text.contains("Exit 0 indicates process completion only"));
    assert!(!text.contains("Retry may have repeated effects"));
    facts.is_error = true;
    facts.note = Some("Execution failed after permission approval.".into());
    assert!(projection::text(&facts).contains(facts.note.as_deref().unwrap()));
    facts.approval = Some("denied".into());
    facts.note = Some("Permission to modify the target was declined.".into());
    assert!(projection::text(&facts).contains(facts.note.as_deref().unwrap()));
}

#[tokio::test]
async fn already_bounded_output_keeps_truncation_notice_without_inventing_total() {
    let output = TRUNCATED
        .scope(RefCell::new(true), async {
            execution::format_shell_output("tail", 4, 0)
        })
        .await;
    assert!(output.contains("truncated"));
    assert!(output.contains("showing last 4B"));
    assert!(output.contains("tail\n[exit: 0]"));
    assert!(!output.contains("total"));
}

#[cfg(target_os = "macos")]
#[allow(clippy::await_holding_lock)] // Serialize process-global HOME during sandbox policy resolution.
#[tokio::test]
async fn approval_retains_attempts_and_partial_effects_without_replaying_prior_call() {
    let _home_guard = crate::test_support::home_env_lock();
    let dir = tempfile::tempdir().unwrap();
    let target = dirs::home_dir().unwrap().join(format!(
        "futureos-attempt-test-{}",
        crate::utils::generate_id()
    ));
    let command = format!("echo partial >> partial; touch '{}'", target.display());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let approvals = seen.clone();
    let reviewed = command.clone();
    let requester: EscalationRequester = Arc::new(move |request| {
        assert_eq!(request.command, reviewed);
        assert!(request.failure_summary.len() <= 2000);
        approvals.lock().push(request.clone());
        EscalationDecision::Approved
    });
    let sandbox = ResolvedSandbox::resolve(
        &crate::sandbox::SandboxPolicy {
            tier: crate::sandbox::SandboxTier::Sandbox,
            model_reviewer: false,
        },
        dir.path().to_str().unwrap(),
    );
    let facts = with_tool_scope(
        ScopeOptions {
            workspace: dir.path().to_string_lossy().into_owned(),
            permission_level: "all".into(),
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            sandbox: Arc::new(sandbox),
            escalation: Some(requester),
            on_sandboxed: None,
        },
        async {
            capture(execute(json!({"command":"echo once >> prior"})))
                .await
                .0
                .unwrap();
            capture(execute(json!({"command":command})))
                .await
                .1
                .unwrap()
        },
    )
    .await;
    assert_eq!(seen.lock().len(), 1);
    assert_eq!(facts.attempts.len(), 2);
    assert_ne!(facts.attempts[0].exit_code, Some(0));
    assert_eq!(facts.attempts[1].exit_code, Some(0));
    assert!(facts.attempts[1].escalated);
    assert!(!facts.is_error);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("prior"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("partial"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    std::fs::remove_file(target).unwrap();
}

#[cfg(target_os = "macos")]
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn post_hoc_denial_cancellation_and_context_end_preserve_original_attempt() {
    let _home_guard = crate::test_support::home_env_lock();
    for (decision, approval) in [
        (EscalationDecision::Denied("no".into()), "denied"),
        (
            EscalationDecision::Cancelled("cancelled".into()),
            "cancelled",
        ),
        (
            EscalationDecision::ContextInvalidated("context ended".into()),
            "context_invalidated",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let target = dirs::home_dir().unwrap().join(format!(
            "futureos-attempt-denied-{}",
            crate::utils::generate_id()
        ));
        let command = format!("echo partial >> partial; touch '{}'", target.display());
        let requester: EscalationRequester = Arc::new(move |_| decision.clone());
        let sandbox = ResolvedSandbox::resolve(
            &crate::sandbox::SandboxPolicy {
                tier: crate::sandbox::SandboxTier::Sandbox,
                model_reviewer: false,
            },
            dir.path().to_str().unwrap(),
        );
        let (_, facts) = with_tool_scope(
            ScopeOptions {
                workspace: dir.path().to_string_lossy().into_owned(),
                permission_level: "all".into(),
                interrupt_flag: Arc::new(AtomicBool::new(false)),
                sandbox: Arc::new(sandbox),
                escalation: Some(requester),
                on_sandboxed: None,
            },
            capture(execute(json!({"command":command}))),
        )
        .await;
        let facts = facts.unwrap();
        assert_eq!(facts.approval.as_deref(), Some(approval));
        assert_eq!(facts.attempts.len(), 1);
        assert_ne!(facts.attempts[0].exit_code, Some(0));
        assert!(facts.is_error);
        assert!(!target.exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("partial"))
                .unwrap()
                .lines()
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn pre_execution_gate_retains_provenance_without_inventing_attempts() {
    for status in ["denied", "cancelled", "context_invalidated"] {
        let dir = tempfile::tempdir().unwrap();
        let (_, facts) = with_workspace_scope(
            dir.path().to_string_lossy().into_owned(),
            "all".into(),
            capture(async {
                record_gate_outcome(status);
                record_gated(&json!({"command":"echo changed > done"}), "approval ended");
            }),
        )
        .await;
        let facts = facts.unwrap();
        assert_eq!(facts.approval.as_deref(), Some(status));
        assert_eq!(facts.exit_code, None);
        assert!(facts.attempts.is_empty());
        assert!(facts.is_error);
        assert_eq!(
            facts.status,
            if status == "denied" {
                ShellStatus::NotStarted
            } else {
                ShellStatus::Cancelled
            }
        );
        assert!(!dir.path().join("done").exists());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn normal_query_returns_keep_host_semantics_and_actual_exit_code() {
    let facts = captured(json!({"command":"test 1 = 2"})).await;
    assert_eq!(facts.exit_code, Some(1));
    assert!(facts.is_soft_fail);
    assert!(!facts.is_error);
    let facts = captured(json!({"command":"test 1 = 2; exit 2"})).await;
    assert_eq!(facts.exit_code, Some(2));
    assert!(!facts.is_soft_fail);
    assert!(facts.is_error);
}

#[cfg(target_os = "macos")]
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn approval_cannot_restart_process_after_execution_budget_expires() {
    let _home_guard = crate::test_support::home_env_lock();
    let dir = tempfile::tempdir().unwrap();
    let target = dirs::home_dir().unwrap().join(format!(
        "futureos-budget-test-{}",
        crate::utils::generate_id()
    ));
    let command = format!("echo partial >> partial; touch '{}'", target.display());
    let requester: EscalationRequester = Arc::new(|_| {
        std::thread::sleep(Duration::from_millis(1100));
        EscalationDecision::Approved
    });
    let sandbox = ResolvedSandbox::resolve(
        &crate::sandbox::SandboxPolicy {
            tier: crate::sandbox::SandboxTier::Sandbox,
            model_reviewer: false,
        },
        dir.path().to_str().unwrap(),
    );
    let (_, facts) = with_tool_scope(
        ScopeOptions {
            workspace: dir.path().to_string_lossy().into_owned(),
            permission_level: "all".into(),
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            sandbox: Arc::new(sandbox),
            escalation: Some(requester),
            on_sandboxed: None,
        },
        capture(execute(json!({"command":command,"timeout":1}))),
    )
    .await;
    let facts = facts.unwrap();
    assert_eq!(facts.status, ShellStatus::TimedOut);
    assert_eq!(facts.exit_code, None);
    assert_eq!(facts.approval.as_deref(), Some("approved"));
    assert_eq!(facts.attempts.len(), 2);
    assert_ne!(facts.attempts[0].exit_code, Some(0));
    assert_eq!(facts.attempts[1].exit_code, None);
    assert!(!target.exists());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("partial"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}
