//! Seatbelt profile smoke tests (SANDBOX_PLAN.md §5 Phase 1).
//!
//! These execute real commands under `sandbox-exec` to validate the generated
//! profile against actual tool behavior: writes land only in writable roots,
//! credential paths are unreadable, the network is blocked, and common
//! developer commands still work. macOS only; marked `#[ignore]` so they run
//! on demand (`cargo test --test sandbox_smoke -- --ignored`) rather than in
//! every CI pass.

#![cfg(target_os = "macos")]

use future_agent::sandbox::{ResolvedSandbox, SandboxMode, SandboxPolicy};
use std::path::PathBuf;
use std::process::Output;

fn workspace(name: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("futureos-smoke-{name}-{stamp}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_sandboxed(sandbox: &ResolvedSandbox, command: &str) -> Output {
    // Mirror how spawn_bash invokes the wrapper (tokio Command → std here).
    let profile = future_agent::sandbox::seatbelt_profile(sandbox);
    std::process::Command::new("/usr/bin/sandbox-exec")
        .args(["-p", &profile, "bash", "-c", command])
        .current_dir(&sandbox.workspace)
        .output()
        .expect("sandbox-exec should spawn")
}

fn default_sandbox(ws: &std::path::Path) -> ResolvedSandbox {
    ResolvedSandbox::resolve(&SandboxPolicy::default(), ws.to_string_lossy().as_ref())
}

#[test]
#[ignore = "runs real sandbox-exec; invoke with --ignored on macOS"]
fn basic_shell_works() {
    let ws = workspace("basic");
    let out = run_sandboxed(&default_sandbox(&ws), "echo hello && pwd && ls -la");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("hello"));
}

#[test]
#[ignore = "runs real sandbox-exec; invoke with --ignored on macOS"]
fn workspace_and_tmp_writes_succeed() {
    let ws = workspace("writes");
    let out = run_sandboxed(
        &default_sandbox(&ws),
        "echo data > inside.txt && mkdir -p sub && echo x > sub/nested.txt \
         && echo t > \"$TMPDIR/futureos-smoke-tmp.txt\" && echo t > /tmp/futureos-smoke-tmp2.txt \
         && echo done",
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(ws.join("inside.txt").exists());
}

#[test]
#[ignore = "runs real sandbox-exec; invoke with --ignored on macOS"]
fn write_outside_roots_is_denied() {
    let ws = workspace("deny-write");
    let home = dirs::home_dir().unwrap();
    let target = home.join(format!("futureos-smoke-denied-{}.txt", std::process::id()));
    let out = run_sandboxed(
        &default_sandbox(&ws),
        &format!("echo nope > {}", target.to_string_lossy()),
    );
    let survived = target.exists();
    std::fs::remove_file(&target).ok();
    assert!(!out.status.success(), "home write should be denied");
    assert!(!survived, "file must not be created outside writable roots");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Operation not permitted"),
        "denial should surface as EPERM (heuristic depends on it): {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[ignore = "runs real sandbox-exec; invoke with --ignored on macOS"]
fn credential_paths_are_unreadable() {
    let ws = workspace("deny-read");
    // ls ~/.ssh must fail even if the dir exists; reading auth.json must fail.
    let out = run_sandboxed(&default_sandbox(&ws), "ls ~/.ssh");
    assert!(
        !out.status.success(),
        "~/.ssh listing should be denied: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let out = run_sandboxed(&default_sandbox(&ws), "cat ~/.future/agent/auth.json");
    assert!(!out.status.success(), "auth.json read should be denied");
}

#[test]
#[ignore = "runs real sandbox-exec; invoke with --ignored on macOS"]
fn network_is_blocked_by_default_and_open_when_enabled() {
    let ws = workspace("network");
    let out = run_sandboxed(
        &default_sandbox(&ws),
        "curl -sS --max-time 5 https://example.com -o /dev/null",
    );
    assert!(!out.status.success(), "network should be blocked");

    let open = ResolvedSandbox::resolve(
        &SandboxPolicy {
            network_access: true,
            ..Default::default()
        },
        ws.to_string_lossy().as_ref(),
    );
    let out = run_sandboxed(
        &open,
        "curl -sS --max-time 10 https://example.com -o /dev/null",
    );
    // Tolerate offline machines: only assert when the unsandboxed network works.
    let baseline = std::process::Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "10",
            "https://example.com",
            "-o",
            "/dev/null",
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if baseline {
        assert!(
            out.status.success(),
            "network_access=true should allow curl: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
#[ignore = "runs real sandbox-exec; invoke with --ignored on macOS"]
fn git_operations_work_in_workspace() {
    let ws = workspace("git");
    let out = run_sandboxed(
        &default_sandbox(&ws),
        "git init -q . && git config user.email smoke@test && git config user.name smoke \
         && echo x > f.txt && git add f.txt && git commit -q -m smoke --no-gpg-sign && git status --short && git log --oneline",
    );
    assert!(
        out.status.success(),
        "git init/add/commit/status should work in the sandbox\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[ignore = "runs real sandbox-exec; invoke with --ignored on macOS"]
fn interpreters_and_devices_work() {
    let ws = workspace("interp");
    let out = run_sandboxed(
        &default_sandbox(&ws),
        "python3 -c 'print(40+2)' 2>/dev/null || echo NO_PYTHON; \
         head -c 8 /dev/urandom > rand.bin && echo dev-ok > /dev/stdout",
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("42") || stdout.contains("NO_PYTHON"),
        "python should run (or be absent): {stdout}"
    );
    assert!(stdout.contains("dev-ok"));
    assert!(ws.join("rand.bin").exists());
}

#[test]
#[ignore = "runs real sandbox-exec; invoke with --ignored on macOS"]
fn read_only_mode_blocks_workspace_writes() {
    let ws = workspace("readonly");
    let sandbox = ResolvedSandbox::resolve(
        &SandboxPolicy {
            mode: SandboxMode::ReadOnly,
            ..Default::default()
        },
        ws.to_string_lossy().as_ref(),
    );
    let out = run_sandboxed(&sandbox, "echo nope > blocked.txt");
    assert!(!out.status.success(), "read-only mode must deny writes");
    assert!(!ws.join("blocked.txt").exists());
    // Reads still work.
    let out = run_sandboxed(&sandbox, "ls / > /dev/null && echo read-ok");
    assert!(String::from_utf8_lossy(&out.stdout).contains("read-ok"));
}

#[test]
#[ignore = "runs real sandbox-exec; invoke with --ignored on macOS"]
fn cargo_check_works_in_sandbox() {
    // Heaviest case: full toolchain (rustc forks, temp files, ~/.cargo reads).
    if std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("cargo unavailable; skipping");
        return;
    }
    let ws = workspace("cargo");
    std::fs::create_dir_all(ws.join("src")).unwrap();
    std::fs::write(
        ws.join("Cargo.toml"),
        "[package]\nname = \"smoke\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(ws.join("src/main.rs"), "fn main() { println!(\"ok\"); }\n").unwrap();
    let out = run_sandboxed(
        &default_sandbox(&ws),
        "cargo check --offline 2>&1 || cargo check 2>&1",
    );
    assert!(
        out.status.success(),
        "cargo check should work in the sandbox\noutput: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}
