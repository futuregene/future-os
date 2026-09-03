#![cfg(target_os = "linux")]

use future_agent::sandbox::linux::probe::probe_linux_sandbox_host;
use future_agent::sandbox::linux::request::{
    HelperPhase, LinuxSandboxRequest, MountKind, MountRequest, REQUEST_VERSION,
};
use std::process::Command;

fn helper_request(
    command: String,
    workspace: &std::path::Path,
    extra_mounts: Vec<MountRequest>,
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
            policy_digest: "0".repeat(64),
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
    let pid_file = workspace.join("pid");
    let command = format!("echo $$ > {}; exec sleep 30", pid_file.display());
    let Some(request) = helper_request(command, &workspace, Vec::new()) else {
        return;
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--linux-sandbox-helper", &request])
        .spawn()
        .unwrap();
    for _ in 0..100 {
        if pid_file.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let pid: i32 = std::fs::read_to_string(&pid_file)
        .expect("sandboxed command did not start")
        .trim()
        .parse()
        .unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    for _ in 0..100 {
        if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("sandboxed descendant {pid} survived helper death");
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
