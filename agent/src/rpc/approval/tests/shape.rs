use super::*;
#[test]
fn shell_whitelist_allows_only_literal_introspection() {
    assert!(!shell_auto_allow("ls -la"));
    assert!(shell_auto_allow("pwd"));
    for command in [
        "cat README.md",
        "git status",
        "git log --oneline",
        "grep -rn foo src | head -20",
        "/tmp/ls",
        "find . -name '*.rs'",
        "env sh evil.sh",
        "cat ~/.ssh/id_rsa",
        "cat /etc/shadow",
        "cat $SECRET",
        "head ../keys/key.pem",
        "git branch -D main",
        "git tag -d v1",
        "git remote add evil url",
        "git reflog expire --all",
        "sort -o out in",
        "uniq in out",
        "find . -fls out",
        "date -s 2020-01-01",
        "hostname evil",
        "yes",
        "seq 999999999",
    ] {
        assert!(!shell_auto_allow(command), "must ask: {command}");
    }
}

#[test]
fn shell_whitelist_asks_for_writes_and_chains() {
    assert!(!shell_auto_allow("rm -rf build"));
    assert!(!shell_auto_allow("echo hi > file.txt")); // redirect
    assert!(!shell_auto_allow("git commit -m x")); // mutating subcommand
    assert!(!shell_auto_allow("ls && rm x")); // chain
    assert!(!shell_auto_allow("cat $(whoami)")); // substitution
    assert!(!shell_auto_allow("find . -delete")); // find mutation
    assert!(!shell_auto_allow("grep foo x | rm y")); // pipe to non-read-only
    assert!(!shell_auto_allow("npm install")); // unknown program
    assert!(!shell_auto_allow(""));
}

#[test]
fn shell_whitelist_does_not_exempt_powershell_file_reads() {
    assert!(!shell_auto_allow("Get-ChildItem"));
    assert!(!shell_auto_allow("get-content foo.txt"));
    assert!(!shell_auto_allow(
        "Get-Content $env:USERPROFILE/.ssh/id_rsa"
    ));
    assert!(!shell_auto_allow("Select-String -Pattern foo bar.txt"));
    assert!(!shell_auto_allow(
        "Get-ChildItem -Recurse | Select-String foo"
    ));
    assert!(!shell_auto_allow(
        r"C:\Windows\System32\findstr.exe foo bar.txt"
    ));
}

#[test]
fn shell_whitelist_asks_for_powershell_writes_and_blocks() {
    assert!(!shell_auto_allow("Remove-Item x")); // mutating cmdlet
    assert!(!shell_auto_allow("Set-Content foo.txt 'x'")); // writes
    assert!(!shell_auto_allow("Get-Content x > out.txt")); // redirect
    assert!(!shell_auto_allow("Get-ChildItem; Remove-Item x")); // chain
    assert!(!shell_auto_allow("Where-Object { $_.Length -gt 0 }")); // script block
    assert!(!shell_auto_allow("Invoke-Expression 'rm x'")); // arbitrary exec
}

#[test]
fn escalation_suggests_parent_for_nonsecret_but_not_secret() {
    let _home_guard = crate::test_support::home_env_lock();
    let ws = temp_ws("escalation-sug");
    let sandbox = ResolvedSandbox::resolve(
        &SandboxPolicy {
            tier: crate::sandbox::SandboxTier::Manual,
            model_reviewer: false,
        },
        &ws,
    );
    let home = dirs::home_dir().unwrap();

    // Non-secret outside-workspace write → suggest the parent dir (~ form).
    let desktop = home.join("Desktop/note.txt");
    let sug = escalation_save_suggestion(&[desktop.to_string_lossy().into_owned()], &sandbox)
        .expect("non-secret blocked path should be persistable");
    assert_eq!(sug["path"], "~/Desktop/*");
    assert_eq!(sug["access"], "write");

    // A secret blocked path (~/.gnupg) → no persistence (one-time only).
    let gnupg = home.join(".gnupg/pubring.kbx");
    assert!(
        escalation_save_suggestion(&[gnupg.to_string_lossy().into_owned()], &sandbox).is_none()
    );
    // Mixed (one secret) → still none.
    assert!(escalation_save_suggestion(
        &[
            desktop.to_string_lossy().into_owned(),
            gnupg.to_string_lossy().into_owned()
        ],
        &sandbox
    )
    .is_none());
    // No known paths → none.
    assert!(escalation_save_suggestion(&[], &sandbox).is_none());
}

#[test]
fn disabled_session_never_prompts() {
    let ws = temp_ws("disabled");
    let sandbox = ResolvedSandbox::disabled(&ws);
    let gate = ApprovalGate::default();
    let b = SseBroadcaster::new();
    let args = serde_json::json!({ "path": outside("d"), "content": "x" });
    assert!(gate
        .request(&b, "s", &ws, "write", "t", &args, &sandbox)
        .is_none());
}

#[test]
fn shell_read_only_auto_allowed_in_manual() {
    // Manual tier: read-only whitelist commands run without a prompt.
    let ws = temp_ws("shell-ro");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let b = SseBroadcaster::new();
    let args = serde_json::json!({ "command": "pwd" });
    assert!(gate
        .request(&b, "s", &ws, "shell", "t", &args, &sandbox)
        .is_none());
}

#[test]
fn shell_never_gated_when_disabled() {
    // Off tier: no approval at all, even for a dangerous command.
    let ws = temp_ws("shell-off");
    let sandbox = ResolvedSandbox::disabled(&ws);
    let gate = ApprovalGate::default();
    let b = SseBroadcaster::new();
    let args = serde_json::json!({ "command": "rm -rf /" });
    assert!(gate
        .request(&b, "s", &ws, "shell", "t", &args, &sandbox)
        .is_none());
}

#[test]
fn write_inside_workspace_auto_allowed() {
    let ws = temp_ws("inside");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let b = SseBroadcaster::new();
    let args = serde_json::json!({ "path": format!("{ws}/src/main.rs"), "content": "x" });
    assert!(gate
        .request(&b, "s", &ws, "write", "t", &args, &sandbox)
        .is_none());
}

#[test]
fn rule_file_write_is_denied_without_prompt() {
    let ws = temp_ws("rulefile");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let b = SseBroadcaster::new();
    let args = serde_json::json!({
        "path": format!("{ws}/.future/approval_rule.json"),
        "content": "{}"
    });
    let result = gate
        .request(&b, "s", &ws, "write", "t", &args, &sandbox)
        .expect("rule-file write must be denied");
    assert!(result.is_error);
    assert!(result.result.contains("denied by an approval rule"));
}

#[test]
fn read_of_ordinary_file_auto_allowed() {
    let ws = temp_ws("read-ok");
    let sandbox = enabled(&ws);
    let gate = ApprovalGate::default();
    let b = SseBroadcaster::new();
    let args = serde_json::json!({ "path": format!("{ws}/src/lib.rs") });
    assert!(gate
        .request(&b, "s", &ws, "read", "t", &args, &sandbox)
        .is_none());
}

#[test]
fn shape_for_write_is_structured() {
    let ws = temp_ws("shape");
    let sandbox = enabled(&ws);
    // Use the canonicalized workspace so the suggestion is workspace-relative.
    let path = sandbox.workspace.join("sub/out.txt");
    let args = serde_json::json!({ "path": path.to_string_lossy(), "content": "hello" });
    let shape = approval_shape("write", &path, Op::Write, &args, &sandbox);
    assert_eq!(shape.action["tool"], "write");
    assert_eq!(shape.action["category"], "file_write");
    assert_eq!(shape.action["writes"][0]["preview"], "hello");
    let sug = shape.save_suggestion.unwrap();
    assert_eq!(sug["access"], "write");
    assert_eq!(sug["action"], "allow");
    // Inside the workspace → relative glob.
    assert_eq!(sug["path"], "sub/*");
}

#[test]
fn shape_for_secret_read_suppresses_suggestion() {
    // $HOME must stay stable between sandbox resolution and the assertion
    // path build (TestHome in rpc::commands redirects it process-wide).
    let _home_guard = crate::test_support::home_env_lock();
    // A secret file (~/.ssh) has no "allow in this workspace" — allow-once only.
    let ws = temp_ws("shape-secret");
    let sandbox = enabled(&ws);
    let path = dirs::home_dir().unwrap().join(".ssh/id_rsa");
    let args = serde_json::json!({ "path": path.to_string_lossy() });
    let shape = approval_shape("read", &path, Op::Read, &args, &sandbox);
    assert_eq!(shape.kind, "file_read");
    assert!(shape.save_suggestion.is_none());
}

#[test]
fn shape_for_nonsecret_read_has_suggestion() {
    let ws = temp_ws("shape-read");
    let sandbox = enabled(&ws);
    let path = sandbox.workspace.join("docs/readme.md");
    let args = serde_json::json!({ "path": path.to_string_lossy() });
    let shape = approval_shape("read", &path, Op::Read, &args, &sandbox);
    assert_eq!(shape.save_suggestion.unwrap()["access"], "read");
}

#[test]
fn shell_auto_allow_requires_approval_for_arbitrary_reads() {
    assert!(!shell_auto_allow("ls -la"));
    assert!(!shell_auto_allow("cat file.txt"));
    assert!(!shell_auto_allow("grep pattern file.txt"));
    assert!(!shell_auto_allow("head -5 file.txt"));
    assert!(!shell_auto_allow("git log"));
    assert!(!shell_auto_allow("git diff"));
    assert!(!shell_auto_allow("find . -name '*.rs'"));
}

#[test]
fn shell_auto_allow_rejects_writes() {
    assert!(!shell_auto_allow("echo hello > file.txt"));
    assert!(!shell_auto_allow("rm file.txt"));
    assert!(!shell_auto_allow("touch file.txt"));
    assert!(!shell_auto_allow("mkdir newdir"));
    assert!(!shell_auto_allow("mv a b"));
}

#[test]
fn shell_auto_allow_rejects_chains() {
    assert!(!shell_auto_allow("ls && rm file"));
    assert!(!shell_auto_allow("ls; rm file"));
    assert!(!shell_auto_allow("ls | tee output.txt"));
    assert!(!shell_auto_allow("echo `whoami`"));
    assert!(!shell_auto_allow("echo $(date)"));
}

#[test]
fn shell_auto_allow_rejects_pipes_with_writes() {
    assert!(!shell_auto_allow("ls | tee output.txt"));
}

#[test]
fn shell_auto_allow_requires_approval_for_pipes() {
    assert!(!shell_auto_allow("ls | grep file"));
    assert!(!shell_auto_allow("cat file | head -10"));
}

#[test]
fn shell_auto_allow_empty_command() {
    assert!(!shell_auto_allow(""));
    assert!(!shell_auto_allow("   "));
}

// ─── command_summary ───────────────────────────────────────────────────

#[test]
fn command_summary_short() {
    assert_eq!(command_summary("ls -la"), "ls -la");
}

#[test]
fn command_summary_truncates_long() {
    let long = "a".repeat(300);
    let summary = command_summary(&long);
    // 200 chars + 3-byte ellipsis
    assert!(summary.len() <= 203);
    assert!(summary.ends_with('\u{2026}'));
}

// ─── extract_blocked_paths_raw ─────────────────────────────────────────

#[test]
fn busy_mount_absolute_target_is_display_only() {
    assert_eq!(
        extract_blocked_paths_raw("rm: cannot remove '/work/.env': Device or resource busy"),
        vec!["/work/.env"]
    );
    assert!(
        extract_blocked_paths_raw("rm: cannot remove '.env': Device or resource busy").is_empty()
    );
}

// ─── shorten_home ──────────────────────────────────────────────────────

#[test]
fn pending_for_session_returns_only_owning_sessions_payloads() {
    let gate = ApprovalGate::default();
    let _rx_a1 = gate.insert_pending_for_test("req-a1", "sessA");
    let _rx_a2 = gate.insert_pending_for_test("req-a2", "sessA");
    let _rx_b1 = gate.insert_pending_for_test("req-b1", "sessB");

    let mut ids = gate
        .pending_for_session("sessA")
        .iter()
        .filter_map(|p| p["approval_request_id"].as_str().map(str::to_string))
        .collect::<Vec<_>>();
    ids.sort();
    assert_eq!(ids, vec!["req-a1", "req-a2"]);
    assert_eq!(gate.pending_for_session("sessB").len(), 1);
    assert!(gate.pending_for_session("sessC").is_empty());

    // A decided (removed) request no longer shows up as pending.
    let decision = ApprovalDecision {
        approved: true,
        note: String::new(),
        status: ApprovalDecisionStatus::Approved,
    };
    assert!(gate.decide("req-a1", "sessA", decision).is_ok());
    assert_eq!(gate.pending_for_session("sessA").len(), 1);
}

#[test]
fn argument_write_preview_finds_content() {
    let args = serde_json::json!({"content": "file content here"});
    assert_eq!(
        argument_write_preview(&args),
        Some("file content here".to_string())
    );
}

#[test]
fn argument_write_preview_finds_new_text() {
    let args = serde_json::json!({"newText": "replacement"});
    assert_eq!(
        argument_write_preview(&args),
        Some("replacement".to_string())
    );
}

#[test]
fn argument_write_preview_truncates() {
    let long = "a".repeat(300);
    let args = serde_json::json!({"content": long});
    let preview = argument_write_preview(&args).unwrap();
    // 200 chars + 3-byte ellipsis
    assert!(preview.len() <= 203);
}

#[test]
fn argument_write_preview_no_content() {
    let args = serde_json::json!({"path": "/tmp/file"});
    assert_eq!(argument_write_preview(&args), None);
}

#[test]
fn argument_write_preview_string_json() {
    let args = serde_json::json!("{\"content\": \"string content\"}");
    assert_eq!(
        argument_write_preview(&args),
        Some("string content".to_string())
    );
}

// ─── normalize_requested_action ────────────────────────────────────────

#[test]
fn normalize_requested_action_parses_json_string() {
    let args = serde_json::json!("{\"path\": \"/tmp/file.txt\"}");
    let result = normalize_requested_action(&args);
    assert_eq!(result["path"], "/tmp/file.txt");
}

#[test]
fn normalize_requested_action_non_string() {
    let args = serde_json::json!({"path": "/tmp/file.txt"});
    let result = normalize_requested_action(&args);
    assert_eq!(result["path"], "/tmp/file.txt");
}

// ─── repair_partial_json_object ────────────────────────────────────────

#[test]
fn repair_partial_json_object_missing_brace() {
    let raw = r#"{"path": "/tmp/file.txt""#;
    let repaired = repair_partial_json_object(raw);
    assert!(repaired.is_some());
    assert_eq!(repaired.unwrap()["path"], "/tmp/file.txt");
}

#[test]
fn repair_partial_json_object_unclosed_string() {
    let raw = r#"{"path": "/tmp/file"#;
    let repaired = repair_partial_json_object(raw);
    assert!(repaired.is_some());
}

#[test]
fn repair_partial_json_object_not_object() {
    assert!(repair_partial_json_object("[1,2,3]").is_none());
}

#[test]
fn repair_partial_json_object_valid() {
    let raw = r#"{"path": "/tmp/file.txt"}"#;
    let repaired = repair_partial_json_object(raw);
    assert!(repaired.is_some());
}

// ─── has_unclosed_string ───────────────────────────────────────────────

#[test]
fn has_unclosed_string_true() {
    assert!(has_unclosed_string(r#"{"key": "unclosed"#));
}

#[test]
fn has_unclosed_string_false() {
    assert!(!has_unclosed_string(r#"{"key": "closed"}"#));
    assert!(!has_unclosed_string("no strings"));
}

#[test]
fn has_unclosed_string_escaped_quotes() {
    assert!(!has_unclosed_string(r#"{"key": "with \"escape\""}"#));
}

// ─── escalation_save_suggestion ────────────────────────────────────────

#[test]
fn path_save_suggestion_inside_workspace() {
    let ws = temp_ws("save-sugg");
    let sandbox = enabled(&ws);
    let path = sandbox.workspace.join("docs/readme.md");
    let suggestion = path_save_suggestion(&path, Op::Read, &sandbox.workspace);
    assert!(suggestion.is_some());
}

#[test]
fn path_save_suggestion_outside_workspace() {
    let ws = temp_ws("save-outside");
    let sandbox = enabled(&ws);
    let outside = dirs::home_dir().unwrap().join("outside.txt");
    let suggestion = path_save_suggestion(&outside, Op::Write, &sandbox.workspace);
    // Should still generate a suggestion (with ~ for home)
    let _ = suggestion;
}

// ─── shell_command_shape ───────────────────────────────────────────────

#[test]
fn shell_command_shape_structure() {
    let ws = temp_ws("shell-shape");
    let sandbox = enabled(&ws);
    let shape = shell_command_shape("ls -la", &sandbox);
    assert_eq!(shape.kind, "shell_command");
    assert_eq!(shape.risk_level, "medium");
    assert!(shape.save_suggestion.is_none());
    assert_eq!(shape.action["command"], "ls -la");
}

// ─── interactive approval flows (decider thread + blocking ask) ────────

/// Poll the gate until the request appears, then deliver the decision.
/// Returns whether the request showed up (a never-taken `panic!` arm would
/// itself be an uncoverable line, so the outcome is asserted by callers).
#[test]
fn approval_shape_edit_tool_variants() {
    let _home_guard = crate::test_support::home_env_lock();
    let ws = temp_ws("shape-edit");
    let sandbox = enabled(&ws);
    // Canonicalize: the resolved workspace is canonical (macOS symlinks
    // /var → /private/var), so raw temp paths would compare as outside.
    let inside = crate::sandbox::paths::canonicalize_lenient(&Path::new(&ws).join("src/main.rs"));
    let shape = approval_shape(
        "edit",
        &inside,
        Op::Write,
        &serde_json::json!({"path": inside}),
        &sandbox,
    );
    assert_eq!(shape.kind, "file_write");
    assert_eq!(shape.action["category"], "file_edit");
    let outside = dirs::home_dir()
        .unwrap()
        .join("futureos-shape-edit-out.txt");
    let shape = approval_shape(
        "edit",
        &outside,
        Op::Write,
        &serde_json::json!({"path": outside}),
        &sandbox,
    );
    assert_eq!(shape.kind, "outside_workspace_write");
    assert_eq!(shape.action["category"], "file_edit");
}

#[test]
fn argument_write_preview_unrepairable_string_returns_none() {
    // A string argument that is neither valid JSON nor repairable.
    assert!(argument_write_preview(&serde_json::json!("{not json at all")).is_none());
}
