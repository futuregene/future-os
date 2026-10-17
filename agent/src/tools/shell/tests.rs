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
        // Over the justification ceiling but under the serialized-input ceiling,
        // so it reaches the justification bound rather than the size bound.
        json!({"command":"echo a","justification":"x".repeat(2_049)}),
        // Escalation and a file grant are mutually exclusive: an approved
        // escalation applies to the whole command, and must not be mixed with
        // a per-path grant the approval card would then have to describe.
        json!({"command":"echo a","escalated":true,"justification":"needed","additional_permissions":{"read":[]}}),
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
async fn sandbox_failure_returns_partial_effects_without_approval_or_replay() {
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
    assert!(seen.lock().is_empty());
    assert_eq!(facts.attempts.len(), 1);
    assert_ne!(facts.attempts[0].exit_code, Some(0));
    assert!(!facts.attempts[0].escalated);
    assert!(facts.is_error);
    assert!(facts.approval.is_none());
    assert!(!target.exists());
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
        1
    );
}

#[cfg(target_os = "macos")]
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn failure_does_not_consult_any_approval_outcome() {
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
        assert!(
            facts.approval.is_none(),
            "unused approval decision: {approval}"
        );
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
async fn failure_does_not_enter_a_slow_approval_hook_or_restart() {
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
    assert_eq!(facts.status, ShellStatus::Exited);
    assert_ne!(facts.exit_code, Some(0));
    assert!(facts.approval.is_none());
    assert_eq!(facts.attempts.len(), 1);
    assert!(!target.exists());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("partial"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[cfg(not(windows))]
#[tokio::test]
async fn file_permission_receipts_are_bound_to_invocation_and_consumed_once() {
    use crate::sandbox::file_request::{prepare, AdditionalPermissions};
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    let key = workspace.join(".env");
    std::fs::write(&key, "secret").unwrap();
    let rules = crate::sandbox::rules::RuleSet::resolve_isolated_with_home(&workspace, &home);
    let sandbox = ResolvedSandbox::with_rules_for_test(rules);
    let permissions: AdditionalPermissions = serde_json::from_value(
        json!({"read": [{"path": key, "scope": "file", "reason": "Read task configuration"}]}),
    )
    .unwrap();
    let prepared = prepare(&sandbox, "cat key", &permissions).unwrap();
    let receipt = prepared.approved_receipt("request-1".into(), "tool-1".into());
    with_tool_scope(
        ScopeOptions {
            workspace: workspace.to_string_lossy().into_owned(),
            permission_level: "all".into(),
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            sandbox: Arc::new(sandbox),
            escalation: None,
            on_sandboxed: None,
        },
        async {
            approve_file_permissions(receipt);
            with_tool_call_id("tool-2".into(), async {
                assert!(!consume_file_permissions(&prepared));
            })
            .await;
            with_tool_call_id("tool-1".into(), async {
                assert!(consume_file_permissions(&prepared));
                assert!(!consume_file_permissions(&prepared));
            })
            .await;
        },
    )
    .await;
}

#[cfg(target_os = "macos")]
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn recovery_executes_only_remaining_work_inside_sandbox() {
    use crate::sandbox::file_request::{prepare, AdditionalPermissions};
    let _home_guard = crate::test_support::home_env_lock();
    let dir = tempfile::tempdir().unwrap();
    let protected = dir.path().join("protected");
    std::fs::create_dir(&protected).unwrap();
    let file = protected.join("report.txt");
    std::fs::write(&file, "old").unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(workspace.join(".future")).unwrap();
    std::fs::write(
        workspace.join(".future/approval_rule.json"),
        json!({"rules": [{"path": protected, "access": "write", "action": "ask"}]}).to_string(),
    )
    .unwrap();
    let sandbox = ResolvedSandbox::resolve(
        &crate::sandbox::SandboxPolicy {
            tier: crate::sandbox::SandboxTier::Sandbox,
            model_reviewer: false,
        },
        &workspace.to_string_lossy(),
    );
    if !sandbox.wraps_shell() {
        return;
    }
    let first = format!("echo partial >> partial; printf new > '{}'", file.display());
    let remaining = format!("printf new > '{}'", file.display());
    let permissions: AdditionalPermissions = serde_json::from_value(json!({"write": [{"path": file, "scope": "file", "reason": "Save the already prepared report"}]})).unwrap();
    with_tool_scope(ScopeOptions {
        workspace: workspace.to_string_lossy().into_owned(), permission_level: "all".into(),
        interrupt_flag: Arc::new(AtomicBool::new(false)), sandbox: Arc::new(sandbox.clone()),
        escalation: Some(Arc::new(|_| panic!("failed script must not request replay approval"))), on_sandboxed: None,
    }, async {
        let (_, facts) = capture(execute(json!({"command": first}))).await;
        let facts = facts.unwrap();
        assert!(facts.is_error);
        assert_eq!(facts.attempts.len(), 1);
        let prepared = prepare(&sandbox, &remaining, &permissions).unwrap();
        approve_file_permissions(prepared.approved_receipt("request-remaining".into(), "remaining".into()));
        let (result, facts) = with_tool_call_id("remaining".into(), capture(execute(json!({"command": remaining, "additional_permissions": {"write": [{"path": file, "scope": "file", "reason": "Save the already prepared report"}]}})))).await;
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(facts.unwrap().attempts.len(), 1);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(workspace.join("partial")).unwrap().lines().count(), 1);
        let (_, later) = capture(execute(json!({"command": format!("printf later > '{}'", file.display())}))).await;
        assert!(later.unwrap().is_error, "the grant must not persist into another call");
    }).await;
}

/// A cancelled approval gate recorded in the scope must outrank the process
/// status the attempt reports: the user stopped the call, so the projected
/// verdict is `Cancelled` even though the command itself exited.
#[tokio::test]
async fn cancelled_gate_outcome_outranks_the_process_status() {
    let dir = tempfile::tempdir().unwrap();
    let (_, facts) = with_workspace_scope(
        dir.path().to_string_lossy().into_owned(),
        "all".into(),
        capture(async {
            record_gate_outcome("cancelled");
            execute(json!({"command":"echo hi"})).await
        }),
    )
    .await;
    let facts = facts.unwrap();
    assert_eq!(facts.approval.as_deref(), Some("cancelled"));
    assert_eq!(facts.status, ShellStatus::Cancelled);
}

/// JSON escaping can expand a bounded raw payload past the 1 MiB result
/// budget on its own (each C0 control byte becomes `\u0001`), so the result
/// path must re-truncate the recorded attempts rather than emit a document
/// the transport rejects. `\x01` is escaped as 6 bytes, so 200 000 raw bytes
/// serialize to about 1.2 MiB.
#[cfg(unix)]
#[tokio::test]
async fn json_escaping_expansion_is_retruncated_to_the_result_budget() {
    let facts = captured(json!({"command":"head -c 200000 /dev/zero | tr '\\0' '\\001'"})).await;
    let attempt = &facts.attempts[0];
    assert_eq!(attempt.status, ShellStatus::Exited);
    assert!(
        attempt.output_truncated,
        "the escaped payload must be halved"
    );
    assert!(attempt.output.len() <= 100_000);
    assert!(serde_json::to_vec(&facts).unwrap().len() <= 1_048_576);
}

/// The workspace string is echoed into every result as `cwd`; a workspace whose
/// JSON form exceeds the documented metadata budget is refused before any
/// process is spawned, so the model never receives a truncated workspace. The
/// path uses C0 control bytes because serde escapes each of them to six bytes,
/// which crosses the 262 144-byte budget with a 44 000-character path.
#[tokio::test]
async fn oversized_workspace_metadata_is_refused_before_execution() {
    let oversized = "\u{1}".repeat(44_000);
    assert!(serde_json::to_string(&oversized).unwrap().len() > 262_144);
    let error = with_workspace_scope(oversized, "all".into(), async {
        execute(json!({"command":"echo hi"}))
            .await
            .unwrap_err()
            .to_string()
    })
    .await;
    assert!(error.contains("result budget"), "{error}");
}

/// A call whose total budget is already spent (a shared deadline in the past)
/// must be refused before any process is launched: the model gets the typed
/// "budget exhausted" verdict instead of a command that runs with no budget.
/// The deadline is set on the call rather than through `execute`, which
/// installs its own deadline scope for the requested timeout.
#[tokio::test]
async fn an_exhausted_total_budget_refuses_before_launching() {
    let dir = tempfile::tempdir().unwrap();
    let expired = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .expect("the monotonic clock is past its first second");
    let error = with_workspace_scope(
        dir.path().to_string_lossy().into_owned(),
        "all".into(),
        DEADLINE.scope(expired, async {
            run_shell("echo changed > done", 10, false, "")
                .await
                .unwrap_err()
                .to_string()
        }),
    )
    .await;
    assert!(
        error.contains("Total execution budget exhausted"),
        "{error}"
    );
    assert!(!dir.path().join("done").exists());
}

/// A sandboxed shell run through the tool: the wrapper is announced, the command
/// executes inside the sandbox, and the process facts reach the result.
///
/// Portable by construction rather than by `#[cfg]`: the guard is a *runtime
/// measurement* (`wraps_shell()`), so the same body runs under seatbelt here and —
/// on Linux — drives the **helper-report** wiring in `spawn_shell_impl`, the branch
/// that exists only when the resolved backend is `LinuxBubblewrap`
/// (`expects_report`; see the module record's §6.2). A `#[cfg(target_os = "linux")]`
/// twin would be uncompilable on this host (the crate's build scripts need a Linux
/// `cc`), so this test is the form that can be *verified* here and still measure
/// those lines on a Linux runner.
#[allow(clippy::await_holding_lock)] // HOME is process-global during policy resolution.
#[tokio::test(flavor = "current_thread")]
async fn a_sandboxed_run_announces_the_wrapper_and_reports_process_facts() {
    let _home_guard = crate::test_support::home_env_lock();
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_string_lossy().into_owned();
    let sandbox = ResolvedSandbox::resolve(
        &crate::sandbox::SandboxPolicy {
            tier: crate::sandbox::SandboxTier::Sandbox,
            model_reviewer: false,
        },
        &workspace,
    );
    if !sandbox.wraps_shell() {
        // No OS sandbox on this host: the wiring is measured where one exists.
        eprintln!("skipping the sandboxed-run test: no OS sandbox backend on this host");
        return;
    }
    let announced = Arc::new(Mutex::new(Vec::new()));
    let seen = announced.clone();
    let command = "printf 'sandboxed-%s' ok";
    let (result, facts) = with_tool_scope(
        ScopeOptions {
            workspace,
            permission_level: "all".into(),
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            sandbox: Arc::new(sandbox),
            escalation: None,
            on_sandboxed: Some(Arc::new(move |command: &str| {
                seen.lock().push(command.to_string());
            })),
        },
        capture(execute(json!({"command": command}))),
    )
    .await;
    let facts = facts.expect("a shell run records its process facts");
    assert!(
        !facts.is_error,
        "a benign sandboxed command must not be reported as an error: {:?}",
        facts.attempts
    );
    assert_eq!(facts.exit_code, Some(0), "{:?}", facts.attempts);
    assert_eq!(facts.status, ShellStatus::Exited);
    let output = result.expect("a benign sandboxed command must succeed");
    assert!(output.contains("sandboxed-ok"), "{output}");
    assert_eq!(
        announced.lock().as_slice(),
        [command],
        "the sandboxed path must be announced exactly once, with the command it wraps"
    );
}
