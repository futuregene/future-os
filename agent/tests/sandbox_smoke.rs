//! Seatbelt profile smoke tests (SANDBOX_PLAN.md §5 Phase 1).
//!
//! These execute real commands under `sandbox-exec` to validate the generated
//! profile against actual tool behavior: writes land only in writable roots,
//! credential paths are unreadable, the network is blocked, and common
//! developer commands still work. macOS only; marked `#[ignore]` so they run
//! on demand (`cargo test --test sandbox_smoke -- --ignored`) rather than in
//! every CI pass.

#![cfg(target_os = "macos")]

use future_agent::sandbox::{ResolvedSandbox, SandboxPolicy};
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
    ResolvedSandbox::resolve(
        &SandboxPolicy { enabled: true },
        ws.to_string_lossy().as_ref(),
    )
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
    let out = run_sandboxed(&default_sandbox(&ws), "cat ~/.future/agent/models.json");
    assert!(!out.status.success(), "models.json read should be denied");

    // Deny a path we create ourselves, so the test distinguishes a real
    // sandbox EPERM from a plain file-not-found on machines lacking the file.
    let home = dirs::home_dir().unwrap();
    let netrc = home.join(".netrc");
    let created = if netrc.exists() {
        false
    } else {
        std::fs::write(&netrc, "machine example.com login u password p").is_ok()
    };
    let out = run_sandboxed(&default_sandbox(&ws), "cat ~/.netrc");
    if created {
        std::fs::remove_file(&netrc).ok();
    }
    assert!(
        !out.status.success()
            && String::from_utf8_lossy(&out.stderr).contains("Operation not permitted"),
        "~/.netrc read should be denied with EPERM: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[ignore = "runs real sandbox-exec; invoke with --ignored on macOS"]
fn network_is_open_in_v2() {
    // v2: network is unrestricted inside the sandbox.
    let ws = workspace("network");
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
    if !baseline {
        eprintln!("offline; skipping network smoke");
        return;
    }
    let out = run_sandboxed(
        &default_sandbox(&ws),
        "curl -sS --max-time 10 https://example.com -o /dev/null && echo net-ok",
    );
    assert!(
        out.status.success() && String::from_utf8_lossy(&out.stdout).contains("net-ok"),
        "network should be open in v2: {}",
        String::from_utf8_lossy(&out.stderr)
    );
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
fn workspace_secret_and_rule_file_writes_are_denied() {
    let ws = workspace("secrets");
    std::fs::create_dir_all(ws.join(".future")).unwrap();

    // A workspace `.env` is a built-in ask → compiled to deny in the profile.
    let out = run_sandboxed(&default_sandbox(&ws), "echo SECRET=1 > .env");
    assert!(!out.status.success(), ".env write must be denied");
    assert!(!ws.join(".env").exists());
    // But reading a `.env` is also denied (ask → deny for bash).
    std::fs::write(ws.join(".env"), "x").unwrap();
    let out = run_sandboxed(&default_sandbox(&ws), "cat .env");
    assert!(!out.status.success(), ".env read must be denied");

    // The rule file itself cannot be written — even via rename (mv), which SBPL
    // file-write* also covers — so the agent can't escalate its own rules.
    let out = run_sandboxed(
        &default_sandbox(&ws),
        "echo '{}' > .future/approval_rule.json",
    );
    assert!(!out.status.success(), "rule-file write must be denied");
    let out = run_sandboxed(
        &default_sandbox(&ws),
        "echo '{}' > /tmp/r.json && mv /tmp/r.json .future/approval_rule.json",
    );
    assert!(!out.status.success(), "rule-file rename-in must be denied");
    assert!(!ws.join(".future/approval_rule.json").exists());
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
