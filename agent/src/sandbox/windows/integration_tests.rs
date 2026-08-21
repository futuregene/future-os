//! Native, non-UI integration matrix for the Windows unelevated backend.
//!
//! These tests deliberately exercise the production runner end to end:
//! capability identity/state -> audited NTFS ACEs -> WRITE_RESTRICTED token ->
//! suspended PowerShell -> Job Object -> captured output/exit status. They are
//! invoked manually by `scripts/test-windows-sandbox.ps1`, not by CI.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use tokio::io::AsyncReadExt;

use super::audit::FrozenPath;
use super::runner;
use crate::sandbox::rules::Decision;
use crate::sandbox::windows_plan::{WindowsSandboxPlan, WindowsWriteCarveout};
use crate::sandbox::windows_request::{ApprovalTarget, ApprovedWriteCapability, WriteScope};

struct Fixture {
    _directory: tempfile::TempDir,
    workspace: PathBuf,
    external: PathBuf,
    sibling: PathBuf,
    state_path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("create integration fixture");
        let workspace = directory.path().join("workspace");
        let external = directory.path().join("approved-target");
        let sibling = directory.path().join("unapproved-sibling");
        std::fs::create_dir_all(&workspace).expect("create workspace");
        std::fs::create_dir_all(&external).expect("create approval target");
        std::fs::create_dir_all(&sibling).expect("create sibling");
        let state_path = directory.path().join("state/capabilities.json");
        Self {
            _directory: directory,
            workspace,
            external,
            sibling,
            state_path,
        }
    }

    fn plan(&self) -> WindowsSandboxPlan {
        WindowsSandboxPlan {
            writable_roots: vec![self.workspace.clone()],
            ..WindowsSandboxPlan::default()
        }
    }

    fn approval(
        &self,
        path: &Path,
        scope: WriteScope,
        request_id: &str,
    ) -> ApprovedWriteCapability {
        ApprovedWriteCapability {
            request_id: request_id.to_owned(),
            command_hash: "integration-command-hash".to_owned(),
            targets: vec![ApprovalTarget {
                path: path.to_string_lossy().into_owned(),
                scope,
            }],
        }
    }
}

struct CommandResult {
    exit_code: u32,
    stdout: String,
    stderr: String,
}

async fn run(
    fixture: &Fixture,
    plan: &WindowsSandboxPlan,
    command: &str,
    approval: Option<&ApprovedWriteCapability>,
    environment: &[(OsString, OsString)],
) -> CommandResult {
    let mut child = runner::spawn_with_plan_for_test(
        plan,
        command,
        &fixture.workspace,
        environment,
        approval,
        &fixture.state_path,
    )
    .expect("restricted runner must start");
    let mut stdout = tokio::fs::File::from_std(child.take_stdout().expect("stdout pipe"));
    let mut stderr = tokio::fs::File::from_std(child.take_stderr().expect("stderr pipe"));
    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.expect("read stdout");
        bytes
    });
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).await.expect("read stderr");
        bytes
    });
    let exit_code = child.wait().await.expect("wait for restricted process");
    CommandResult {
        exit_code,
        stdout: String::from_utf8_lossy(&stdout_task.await.expect("stdout task")).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_task.await.expect("stderr task")).into_owned(),
    }
}

fn ps(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

fn write_command(path: &Path, value: &str) -> String {
    format!(
        "$ErrorActionPreference='Stop'; Set-Content -LiteralPath {} -Value '{}' -Encoding UTF8",
        ps(path),
        value.replace('\'', "''")
    )
}

fn read_fixture_text(path: &Path) -> String {
    std::fs::read_to_string(path)
        .expect("read fixture text")
        .trim_start_matches('\u{feff}')
        .to_owned()
}

#[tokio::test]
async fn runner_allows_workspace_but_denies_undeclared_external_write() {
    let fixture = Fixture::new();
    let plan = fixture.plan();
    let workspace_file = fixture.workspace.join("created.txt");
    let allowed = run(
        &fixture,
        &plan,
        &write_command(&workspace_file, "workspace-ok"),
        None,
        &[],
    )
    .await;
    assert_eq!(allowed.exit_code, 0, "stderr: {}", allowed.stderr);
    assert_eq!(read_fixture_text(&workspace_file).trim(), "workspace-ok");

    let external_file = fixture.external.join("blocked.txt");
    let denied = run(
        &fixture,
        &plan,
        &write_command(&external_file, "must-not-exist"),
        None,
        &[],
    )
    .await;
    assert_ne!(denied.exit_code, 0, "unexpected stdout: {}", denied.stdout);
    assert!(!external_file.exists());
    assert!(
        fixture.state_path.exists(),
        "capability state was not saved"
    );
}

#[tokio::test]
async fn subtree_approval_is_exact_and_old_ace_is_not_reusable() {
    let fixture = Fixture::new();
    let plan = fixture.plan();
    let approval = fixture.approval(&fixture.external, WriteScope::Subtree, "subtree-once");
    let approved_file = fixture.external.join("allowed.txt");
    let sibling_file = fixture.sibling.join("blocked.txt");
    let command = format!(
        "$ErrorActionPreference='Stop'; Set-Content -LiteralPath {} -Value 'approved'; Set-Content -LiteralPath {} -Value 'blocked'",
        ps(&approved_file),
        ps(&sibling_file)
    );
    let result = run(&fixture, &plan, &command, Some(&approval), &[]).await;
    assert_ne!(result.exit_code, 0, "sibling write unexpectedly succeeded");
    assert_eq!(read_fixture_text(&approved_file).trim(), "approved");
    assert!(!sibling_file.exists());

    // The request ACE remains on disk, but a later policy-only token does not
    // contain that request SID and therefore cannot reuse the old approval.
    let reuse = run(
        &fixture,
        &plan,
        &write_command(&approved_file, "reused"),
        None,
        &[],
    )
    .await;
    assert_ne!(reuse.exit_code, 0, "one-time SID was reusable");
    assert_eq!(read_fixture_text(&approved_file).trim(), "approved");
}

#[tokio::test]
async fn file_approval_does_not_expand_to_parent_or_delete() {
    let fixture = Fixture::new();
    let plan = fixture.plan();
    let approved_file = fixture.external.join("existing.txt");
    let sibling_file = fixture.external.join("sibling.txt");
    std::fs::write(&approved_file, "before").unwrap();
    let approval = fixture.approval(&approved_file, WriteScope::File, "file-once");

    let write = run(
        &fixture,
        &plan,
        &write_command(&approved_file, "after"),
        Some(&approval),
        &[],
    )
    .await;
    assert_eq!(write.exit_code, 0, "stderr: {}", write.stderr);
    assert_eq!(read_fixture_text(&approved_file).trim(), "after");

    let create_sibling = run(
        &fixture,
        &plan,
        &write_command(&sibling_file, "blocked"),
        Some(&approval),
        &[],
    )
    .await;
    assert_ne!(create_sibling.exit_code, 0);
    assert!(!sibling_file.exists());

    let remove = run(
        &fixture,
        &plan,
        &format!(
            "$ErrorActionPreference='Stop'; Remove-Item -LiteralPath {} -Force",
            ps(&approved_file)
        ),
        Some(&approval),
        &[],
    )
    .await;
    assert_ne!(
        remove.exit_code, 0,
        "file scope unexpectedly granted delete"
    );
    assert!(approved_file.exists());
}

#[tokio::test]
async fn existing_deny_carveout_wins_inside_writable_workspace() {
    let fixture = Fixture::new();
    let protected = fixture.workspace.join("protected");
    std::fs::create_dir_all(&protected).unwrap();
    let mut plan = fixture.plan();
    plan.write_carveouts.push(WindowsWriteCarveout {
        path: protected.clone(),
        decision: Decision::Deny,
    });

    let ordinary_file = fixture.workspace.join("ordinary.txt");
    let ordinary = run(
        &fixture,
        &plan,
        &write_command(&ordinary_file, "ok"),
        None,
        &[],
    )
    .await;
    assert_eq!(ordinary.exit_code, 0, "stderr: {}", ordinary.stderr);

    let protected_file = protected.join("blocked.txt");
    let denied = run(
        &fixture,
        &plan,
        &write_command(&protected_file, "blocked"),
        None,
        &[],
    )
    .await;
    assert_ne!(denied.exit_code, 0);
    assert!(!protected_file.exists());
}

#[tokio::test]
async fn production_runner_preserves_cwd_env_output_and_exit_code() {
    let fixture = Fixture::new();
    let plan = fixture.plan();
    let result = run(
        &fixture,
        &plan,
        "Write-Output \"$env:FUTUREOS_INTEGRATION|$pwd\"; exit 23",
        None,
        &[(
            OsString::from("FUTUREOS_INTEGRATION"),
            OsString::from("ready"),
        )],
    )
    .await;
    assert_eq!(result.exit_code, 23);
    let output = result.stdout.to_lowercase();
    assert!(output.contains("ready|"), "unexpected stdout: {output:?}");
    assert!(
        output.contains(&fixture.workspace.to_string_lossy().to_lowercase()),
        "cwd missing from stdout: {output:?}"
    );
    assert!(
        result.stderr.trim().is_empty(),
        "stderr: {:?}",
        result.stderr
    );
}

#[tokio::test]
async fn unicode_path_pipeline_redirection_and_large_output_do_not_deadlock() {
    let fixture = Fixture::new();
    let plan = fixture.plan();
    let unicode_file = fixture.workspace.join("发布-结果.txt");
    let command = format!(
        "$ErrorActionPreference='Stop'; 1..3 | ForEach-Object {{ \"项目-$_\" }} | Set-Content -LiteralPath {} -Encoding UTF8; Write-Output ('x' * 700000)",
        ps(&unicode_file)
    );
    let result = run(&fixture, &plan, &command, None, &[]).await;
    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(read_fixture_text(&unicode_file).contains("项目-3"));
    assert!(
        result.stdout.len() >= 700_000,
        "large stdout was truncated unexpectedly: {} bytes",
        result.stdout.len()
    );
}

#[test]
fn directory_reparse_point_is_rejected_by_handle_audit() {
    let fixture = Fixture::new();
    let real = fixture.workspace.join("real-directory");
    let junction = fixture.workspace.join("junction");
    std::fs::create_dir_all(&real).unwrap();
    let status = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&real)
        .status()
        .expect("start cmd.exe for junction fixture");
    assert!(status.success(), "could not create junction fixture");

    let marker = fixture.workspace.join("must-not-run.txt");
    let approval = fixture.approval(&junction, WriteScope::Subtree, "reparse-target");
    let error = match runner::spawn_with_plan_for_test(
        &fixture.plan(),
        &write_command(&marker, "started"),
        &fixture.workspace,
        &[],
        Some(&approval),
        &fixture.state_path,
    ) {
        Ok(_) => panic!("reparse-point capability target must be rejected before spawn"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(!marker.exists(), "command ran after failed reparse audit");
}

#[test]
fn unc_paths_fail_closed_before_acl_mutation() {
    let error = match FrozenPath::open_local_ntfs(Path::new(r"\\server\share\target")) {
        Ok(_) => panic!("UNC capability target must be rejected"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}
