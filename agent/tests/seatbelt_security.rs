//! Real-kernel regressions; run on macOS, using only disposable fixtures.
#![cfg(target_os = "macos")]

use future_agent::sandbox::{ResolvedSandbox, SandboxPolicy, SandboxTier};
use std::path::Path;
use std::process::{Command, Output};

fn sandbox(workspace: &Path) -> ResolvedSandbox {
    ResolvedSandbox::resolve(
        &SandboxPolicy {
            tier: SandboxTier::Sandbox,
        },
        workspace.to_str().unwrap(),
    )
}

fn run(sandbox: &ResolvedSandbox, program: &str, path: &Path) -> Output {
    Command::new("/usr/bin/sandbox-exec")
        .args([
            "-p",
            &future_agent::sandbox::seatbelt_profile(sandbox),
            program,
        ])
        .arg(path)
        .current_dir(&sandbox.workspace)
        .output()
        .unwrap()
}

#[test]
fn glob_secrets_deny_reads_and_future_writes_with_case_and_newlines() {
    let root = tempfile::tempdir().unwrap();
    let policy = sandbox(root.path());
    for name in [
        "secret.pem",
        "secret.PEM",
        ".env.production",
        ".ENV.PRODUCTION",
        "nested/private.key",
        "line\nbreak/private.pem",
    ] {
        let path = root.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let output = run(&policy, "/usr/bin/touch", &path);
        assert!(!output.status.success(), "created {name:?}");
        assert!(
            !path.exists(),
            "write must be prevented, not detected later"
        );
        std::fs::write(&path, "fixture-secret").unwrap();
        let output = run(&policy, "/bin/cat", &path);
        assert!(!output.status.success(), "read {name:?}: {output:?}");
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("Operation not permitted"),
            "{output:?}"
        );
        assert_eq!(
            policy.evaluate(&path, future_agent::sandbox::rules::Op::Read),
            future_agent::sandbox::rules::Decision::Ask
        );
        // APFS is commonly case-insensitive; remove the fixture before its
        // differently cased spelling is exercised in the next iteration.
        std::fs::remove_file(&path).unwrap();
    }
    let allowed = root.path().join("ordinary.txt");
    assert!(run(&policy, "/usr/bin/touch", &allowed).status.success());
}

#[test]
fn glob_literals_cannot_become_regex_or_sbpl_syntax() {
    let root = tempfile::tempdir().unwrap();
    for name in [
        "dot.dir",
        "bracket[dir]",
        "quote\"dir",
        "back\\slash",
        "unicode-中文",
    ] {
        let workspace = root.path().join(name);
        std::fs::create_dir(&workspace).unwrap();
        let policy = sandbox(&workspace);
        let ordinary = workspace.join("ordinary.txt");
        let output = run(&policy, "/usr/bin/touch", &ordinary);
        assert!(
            output.status.success(),
            "profile must compile for {name:?}: {output:?}"
        );
        let secret = workspace.join("private.pem");
        std::fs::write(&secret, "fixture-secret").unwrap();
        let output = run(&policy, "/bin/cat", &secret);
        assert!(!output.status.success(), "secret escaped via {name:?}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn malformed_or_partially_invalid_policy_never_starts_a_shell() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".future")).unwrap();
    for rules in [
        "{broken",
        r#"{"rules":[{"path":"private","action":"invalid"}]}"#,
    ] {
        std::fs::write(root.path().join(".future/approval_rule.json"), rules).unwrap();
        let policy = sandbox(root.path());
        let error = policy
            .prepare_shell_for_cwd("touch should-not-run", false, root.path())
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("approval rule layer could not be loaded"));
        // Direct consumers of the public profile builder must fail closed too.
        let output = run(
            &policy,
            "/usr/bin/touch",
            &root.path().join("should-not-run"),
        );
        assert!(!output.status.success());
        assert!(!root.path().join("should-not-run").exists());
        // An explicitly approved unsandboxed command remains a separate path.
        assert!(policy
            .prepare_shell_for_cwd("true", true, root.path())
            .is_ok());
    }
}
