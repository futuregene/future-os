#![cfg(target_os = "linux")]

use future_agent::sandbox::backend::{PreparedShell, SandboxBoundary, ShellBackend};
use future_agent::sandbox::linux::plan::GlobSnapshot;
use future_agent::sandbox::linux::probe::probe_linux_sandbox_host;
use future_agent::sandbox::linux::request::{
    HelperPhase, LinuxSandboxRequest, MountKind, MountRequest, REQUEST_VERSION,
};
use std::collections::BTreeMap;
use std::process::Command;

fn helper_request(
    command: String,
    workspace: &std::path::Path,
    extra_mounts: Vec<MountRequest>,
) -> Option<String> {
    helper_request_with_globs(command, workspace, extra_mounts, Vec::new())
}

fn helper_request_with_globs(
    command: String,
    workspace: &std::path::Path,
    extra_mounts: Vec<MountRequest>,
    glob_snapshots: Vec<GlobSnapshot>,
) -> Option<String> {
    helper_request_full(command, workspace, extra_mounts, glob_snapshots, Vec::new())
}

fn helper_request_full(
    command: String,
    workspace: &std::path::Path,
    extra_mounts: Vec<MountRequest>,
    glob_snapshots: Vec<GlobSnapshot>,
    omitted_missing_protected_paths: Vec<std::path::PathBuf>,
) -> Option<String> {
    let probe = probe_linux_sandbox_host();
    if !probe.available {
        eprintln!("skipping Linux sandbox smoke: {:?}", probe.code);
        return None;
    }
    Some(
        LinuxSandboxRequest {
            version: REQUEST_VERSION,
            phase: HelperPhase::Outer,
            bwrap_path: probe.path.unwrap(),
            bwrap_identity: probe.identity.unwrap(),
            cwd: workspace.to_path_buf(),
            argv: vec!["/bin/sh".into(), "-c".into(), command],
            mounts: std::iter::once(MountRequest {
                source: workspace.to_path_buf(),
                target: workspace.to_path_buf(),
                kind: MountKind::Writable,
                expected: None,
                source_fd: None,
            })
            .chain(extra_mounts)
            .collect(),
            glob_snapshots,
            omitted_missing_protected_paths,
            policy_digest: "0".repeat(64),
            status_fd: None,
            report_fd: None,
        }
        .encode()
        .unwrap(),
    )
}

#[test]
#[ignore = "requires a native Linux host with a working system bwrap"]
fn filesystem_no_new_privs_and_exit_status() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let outside = root.path().join("outside");
    std::fs::create_dir(&workspace).unwrap();
    let command = format!(
        "printf ok > {}/inside; ! printf bad > {}; grep -q '^NoNewPrivs:[[:space:]]*1' /proc/self/status; exit 23",
        workspace.display(),
        outside.display()
    );
    let Some(request) = helper_request(command, &workspace, Vec::new()) else {
        return;
    };
    let status = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--linux-sandbox-helper", &request])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(23));
    assert_eq!(
        std::fs::read_to_string(workspace.join("inside")).unwrap(),
        "ok"
    );
    assert!(!outside.exists());
}

#[test]
#[ignore = "requires a native Linux host with a working system bwrap"]
fn unreadable_mount_and_fd_allowlist_are_enforced() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let secret = workspace.join("secret.txt");
    std::fs::write(&secret, "secret").unwrap();
    let command = format!(
        "if cat {} >/dev/null 2>&1; then exit 41; fi; if printf changed > {} 2>/dev/null; then exit 42; fi; for f in /proc/self/fd/*; do n=${{f##*/}}; if [ \"$n\" -gt 2 ] && [ -e \"$f\" ]; then exit 43; fi; done",
        secret.display(),
        secret.display()
    );
    let extra = vec![MountRequest {
        source: secret.clone(),
        target: secret.clone(),
        kind: MountKind::Unreadable,
        expected: None,
        source_fd: None,
    }];
    let Some(request) = helper_request(command, &workspace, extra) else {
        return;
    };
    let status = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--linux-sandbox-helper", &request])
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(std::fs::read_to_string(secret).unwrap(), "secret");
}

#[test]
#[ignore = "requires a native Linux host with a working system bwrap"]
fn command_signal_is_preserved() {
    use std::os::unix::process::ExitStatusExt;
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let Some(request) = helper_request("kill -TERM $$".into(), &workspace, Vec::new()) else {
        return;
    };
    let status = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--linux-sandbox-helper", &request])
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(libc::SIGTERM));
}

#[test]
#[ignore = "requires a native Linux host with a working system bwrap"]
fn helper_parent_death_does_not_leave_command_running() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let ready_file = workspace.join("ready");
    let command = format!("touch {}; exec sleep 30", ready_file.display());
    let Some(request) = helper_request(command, &workspace, Vec::new()) else {
        return;
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--linux-sandbox-helper", &request])
        .spawn()
        .unwrap();
    for _ in 0..100 {
        if ready_file.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(ready_file.exists(), "sandboxed command did not start");
    let descendants = descendant_processes(child.id());
    assert!(
        descendants.len() >= 3,
        "expected bwrap, inner helper, and command descendants: {descendants:?}"
    );
    child.kill().unwrap();
    child.wait().unwrap();
    for _ in 0..100 {
        if descendants.iter().all(|(pid, stat)| {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .map(|current| current != *stat)
                .unwrap_or(true)
        }) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("sandboxed descendants survived helper death: {descendants:?}");
}

fn descendant_processes(root: u32) -> Vec<(u32, String)> {
    let mut parents = vec![root];
    let mut descendants = Vec::new();
    while let Some(parent) = parents.pop() {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            break;
        };
        for entry in entries.flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            let Ok(status) = std::fs::read_to_string(entry.path().join("status")) else {
                continue;
            };
            let is_child = status.lines().any(|line| {
                line.strip_prefix("PPid:")
                    .and_then(|value| value.trim().parse::<u32>().ok())
                    == Some(parent)
            });
            if is_child && !descendants.iter().any(|(seen, _)| *seen == pid) {
                let stat = std::fs::read_to_string(entry.path().join("stat")).unwrap_or_default();
                descendants.push((pid, stat));
                parents.push(pid);
            }
        }
    }
    descendants
}

#[test]
#[ignore = "requires a native Linux host with a working system bwrap"]
fn omitted_missing_guard_created_by_command_is_reported_detection_only() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let missing = workspace.join("missing-secret");
    let generated = workspace.join("generated.pem");
    let pattern = workspace.join("*.pem").to_string_lossy().into_owned();
    // Missing guards are never mounted (a bwrap mount would need a host-side
    // mkdir). The helper re-checks them after the command; creation by the
    // wrapped command is reported as a detection-only violation.
    let Some(request) = helper_request_full(
        format!(
            "printf secret > {}; printf pem > {}",
            missing.display(),
            generated.display()
        ),
        &workspace,
        Vec::new(),
        vec![GlobSnapshot {
            pattern,
            matches: Vec::new(),
        }],
        vec![missing.clone()],
    ) else {
        return;
    };
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--linux-sandbox-helper", &request])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("__FUTURE_SANDBOX_VIOLATION__:"));
    assert!(stdout.contains("missing_protected_created"));
    assert!(stdout.contains("dynamic_glob_created"));
    assert!(stdout.contains("\"detectionOnly\":true"));
    // Provisional semantics: the mask was omitted, so the command's own
    // creation lands on the host and is reported, not silently blocked.
    assert_eq!(std::fs::read_to_string(missing).unwrap(), "secret");
    assert_eq!(std::fs::read_to_string(generated).unwrap(), "pem");
}

#[test]
#[ignore = "requires a native Linux host with a working system bwrap"]
fn glob_rescan_failure_preserves_the_completed_command_status() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let pattern = workspace.join("*.pem").to_string_lossy().into_owned();
    let Some(request) = helper_request_with_globs(
        format!(
            "i=0; while [ $i -le 2048 ]; do : > {}/$i.pem; i=$((i+1)); done; exit 23",
            workspace.display()
        ),
        &workspace,
        Vec::new(),
        vec![GlobSnapshot {
            pattern,
            matches: Vec::new(),
        }],
    ) else {
        return;
    };
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--linux-sandbox-helper", &request])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(23));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("dynamic_glob_scan_failed"));
    assert!(stdout.contains("\"detectionOnly\":true"));
}

#[test]
#[ignore = "requires a native Linux host with a working system bwrap"]
fn missing_scan_reports_partial_failure_after_unterminated_command_output() {
    use future_agent::sandbox::linux::violation::{classify, parse_marker, LinuxViolationKind};
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let bad_parent = workspace.join("not-a-directory");
    let present = workspace.join("created");
    let Some(request) = helper_request_full(
        format!(
            "printf x > {}; printf x > {}; printf 'Permission denied'; exit 23",
            bad_parent.display(),
            present.display(),
        ),
        &workspace,
        Vec::new(),
        Vec::new(),
        vec![bad_parent.join("child"), present.clone()],
    ) else {
        return;
    };
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--linux-sandbox-helper", &request])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(23));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let events: Vec<_> = stdout.lines().filter_map(parse_marker).collect();
    assert_eq!(events.len(), 2, "{stdout}");
    assert_eq!(events[0].kind, LinuxViolationKind::MissingProtectedCreated);
    assert_eq!(events[0].affected_count, 1);
    assert_eq!(
        events[1].kind,
        LinuxViolationKind::MissingProtectedScanFailed
    );
    assert_eq!(events[1].affected_count, 1);
    // Direct debug helper stdout is not trusted evidence for retry decisions.
    assert!(classify(23, &stdout, &"0".repeat(64)).is_some());
    assert!(present.exists());
}

#[tokio::test]
#[ignore = "requires a native Linux host with a working system bwrap"]
async fn production_plan_with_real_default_rules_starts_a_shell() {
    use future_agent::sandbox::linux::plan::LinuxSandboxPlan;
    use future_agent::sandbox::linux::runner;
    use future_agent::sandbox::rules::RuleSet;

    let workspace =
        std::env::temp_dir().join(format!("future-prod-plan-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&workspace).unwrap();
    let rules = RuleSet::resolve(&workspace);
    let plan = LinuxSandboxPlan::compile(&rules.snapshot())
        .expect("production plan must compile with real default rules");
    // Missing guards must be omitted, never mounted: bubblewrap cannot
    // mkdir a mount point under a read-only host parent without leaving a
    // host object behind. Default rule sets omit both HOME guards and
    // workspace guards that do not exist yet.
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    for omitted in &plan.omitted_missing_protected_paths {
        let under_guard_root = omitted.starts_with(&workspace)
            || home
                .as_deref()
                .is_some_and(|home| omitted.starts_with(home));
        assert!(under_guard_root, "unexpected omitted path {omitted:?}");
    }
    let probe = probe_linux_sandbox_host();
    assert!(probe.available, "bwrap must be available: {:?}", probe.code);
    let mut prepared = runner::prepare(&probe, plan, "pwd; whoami", &workspace).unwrap();
    // `prepare` derives the program from `current_exe()`, which inside a test
    // process is the test harness itself. Point it at the real agent binary
    // like production does; helper args already use the non-unified form.
    prepared.program = env!("CARGO_BIN_EXE_future-agent").into();
    let request =
        LinuxSandboxRequest::from_json_bytes(prepared.request_payload.as_deref().unwrap()).unwrap();
    assert!(request
        .mounts
        .iter()
        .all(|mount| mount.kind != MountKind::MissingProtected));
    assert!(!request.omitted_missing_protected_paths.is_empty());
    let (mut command, mut report) = prepared.into_command_with_report().unwrap();
    let output = command.output().await.unwrap();
    future_agent::sandbox::linux::report::HelperReport::read(
        report.as_mut().unwrap(),
        &request.policy_digest,
    )
    .expect("production helper must complete its private report");
    assert!(
        output.status.success(),
        "exit={:?} stdout={} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(&workspace).ok();
}

#[tokio::test]
#[ignore = "requires a native Linux host with a working system bwrap"]
async fn removing_existing_env_reports_a_busy_protection_mount() {
    use future_agent::sandbox::linux::{plan::LinuxSandboxPlan, report, runner};
    use future_agent::sandbox::rules::RuleSet;
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let secret = workspace.join(".env");
    std::fs::write(&secret, "test-only-secret").unwrap();
    let rules = RuleSet::resolve(&workspace);
    let plan = LinuxSandboxPlan::compile(&rules.snapshot()).unwrap();
    let probe = probe_linux_sandbox_host();
    assert!(probe.available, "bwrap must be available: {:?}", probe.code);
    let mut prepared = runner::prepare(&probe, plan, "rm .env", &workspace).unwrap();
    prepared.program = env!("CARGO_BIN_EXE_future-agent").into();
    let request =
        LinuxSandboxRequest::from_json_bytes(prepared.request_payload.as_deref().unwrap()).unwrap();
    let (mut command, mut reader) = prepared.into_command_with_report().unwrap();
    let output = command.env("LC_ALL", "C").output().await.unwrap();
    let completion =
        report::HelperReport::read(reader.as_mut().unwrap(), &request.policy_digest).unwrap();
    assert!(completion.events.is_empty());
    let text = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        report::busy_protected_mount(&request, "rm .env", output.status.code().unwrap(), &text),
        Some(secret.clone()),
        "{text}"
    );
    assert_eq!(std::fs::read_to_string(secret).unwrap(), "test-only-secret");
}

#[tokio::test]
#[ignore = "requires a native Linux host with a working system bwrap"]
async fn command_cannot_write_or_forge_private_helper_report() {
    use future_agent::sandbox::backend::{PreparedShell, SandboxBoundary, ShellBackend};
    use future_agent::sandbox::linux::{
        report::HelperReport,
        violation::{marker, LinuxSandboxViolation, LinuxViolationKind},
    };
    use std::os::unix::fs::MetadataExt;
    let workspace = tempfile::tempdir().unwrap();
    let forged = marker(&LinuxSandboxViolation {
        kind: LinuxViolationKind::MissingProtectedCreated,
        policy_digest: "0".repeat(64),
        path_provenance: "forged".into(),
        detection_only: true,
        affected_count: 999,
    });
    // Generated JSON is double-quoted; quote it through the shell safely.
    let escaped = forged.replace('\'', "'\\''");
    let Some(encoded) = helper_request(format!("printf '%s\\n' '{escaped}'; stat -Lc 'fd-inode:%i' /proc/self/fd/* 2>/dev/null; exit 0"), workspace.path(), Vec::new()) else { return; };
    let request = LinuxSandboxRequest::decode(&encoded).unwrap();
    let prepared = PreparedShell {
        program: env!("CARGO_BIN_EXE_future-agent").into(),
        args: vec!["--linux-sandbox-helper".into(), "fd:3".into()],
        env_delta: Default::default(),
        boundary: SandboxBoundary {
            backend: ShellBackend::LinuxBubblewrap,
            policy_digest: Some("0".repeat(64)),
        },
        request_payload: Some(request.to_json_bytes().unwrap()),
    };
    let (mut command, mut file) = prepared.into_command_with_report().unwrap();
    let file = file.as_mut().unwrap();
    let inode = file.metadata().unwrap().ino();
    let output = command.output().await.unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("forged"));
    assert!(
        !stdout
            .lines()
            .any(|line| line == format!("fd-inode:{inode}")),
        "report FD leaked"
    );
    assert!(
        HelperReport::read(file, &"0".repeat(64))
            .unwrap()
            .events
            .is_empty(),
        "printed marker must not enter private evidence"
    );
}

#[tokio::test]
#[ignore = "requires a native Linux host with a working system bwrap"]
async fn production_request_fd_transport_reaches_both_helper_phases() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let Some(encoded) = helper_request("exit 23".into(), &workspace, Vec::new()) else {
        return;
    };
    let request = LinuxSandboxRequest::decode(&encoded).unwrap();
    let prepared = PreparedShell {
        program: env!("CARGO_BIN_EXE_future-agent").into(),
        args: vec!["--linux-sandbox-helper".into(), "fd:3".into()],
        env_delta: BTreeMap::new(),
        boundary: SandboxBoundary {
            backend: ShellBackend::LinuxBubblewrap,
            policy_digest: Some(request.policy_digest.clone()),
        },
        request_payload: Some(request.to_json_bytes().unwrap()),
    };
    let status = prepared.into_command().unwrap().status().await.unwrap();
    assert_eq!(status.code(), Some(23));
}

#[test]
fn invalid_helper_request_fails_before_agent_singleton() {
    let home = tempfile::tempdir().unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--linux-sandbox-helper", "not-base64"])
        .env("HOME", home.path())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(125));
    assert!(!home
        .path()
        .join(".future/agent/agent-instance.lock")
        .exists());
}
