//! Real entry-point checks with a fresh synthetic home for each subprocess.
use serde_json::json;
use std::process::{Command, Output};

fn run(home: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .output()
        .unwrap()
}

fn seed(home: &std::path::Path) -> (std::path::PathBuf, String) {
    let dir = home.join(".future/agent/sessions");
    std::fs::create_dir_all(&dir).unwrap();
    let source = format!(
        "{}\n",
        json!({"id":"entry-1","type":"user","role":"user","content":"synthetic text","timestamp":"2026-01-01T00:00:00Z"})
    );
    std::fs::write(dir.join("good.jsonl"), &source).unwrap();
    std::fs::write(dir.join("bad.jsonl"), "{invalid}\n").unwrap();
    (dir, source)
}

#[test]
fn maintenance_reports_skips_and_retries_without_touching_sources() {
    let home = tempfile::tempdir().unwrap();
    let (dir, original) = seed(home.path());
    let result = run(home.path(), &["--migrate-sessions"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report[0]["status"], "skipped");
    assert_eq!(report[1]["status"], "imported");
    assert!(!String::from_utf8_lossy(&result.stdout).contains("synthetic text"));
    assert_eq!(
        std::fs::read_to_string(dir.join("good.jsonl")).unwrap(),
        original
    );
    let failed_retry = run(home.path(), &["--retry-session-import", "bad"]);
    assert!(!failed_retry.status.success());
    std::fs::write(dir.join("bad.jsonl"), &original).unwrap();
    let result = run(home.path(), &["--retry-session-import", "bad"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report[0]["status"], "imported");
    let store = future_agent::session::sqlite_store::SqliteStore::open(
        &home.path().join(".future/agent/agent.db"),
    )
    .unwrap();
    assert_eq!(store.ids(false).unwrap(), ["bad", "good"]);
}

#[test]
fn normal_startup_imports_before_serving_and_keeps_bad_session() {
    let home = tempfile::tempdir().unwrap();
    let (dir, original) = seed(home.path());
    let result = run(
        home.path(),
        &["--grpc-addr", "127.0.0.1:0", "--profile-seconds", "0"],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let store = future_agent::session::sqlite_store::SqliteStore::open(
        &home.path().join(".future/agent/agent.db"),
    )
    .unwrap();
    assert_eq!(store.ids(false).unwrap(), ["good"]);
    assert_eq!(store.ids(true).unwrap(), ["bad", "good"]);
    assert_eq!(
        std::fs::read_to_string(dir.join("good.jsonl")).unwrap(),
        original
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("bad.jsonl")).unwrap(),
        "{invalid}\n"
    );
}

#[test]
fn database_unavailable_fails_startup_instead_of_falling_back() {
    let home = tempfile::tempdir().unwrap();
    let (dir, original) = seed(home.path());
    std::fs::write(home.path().join(".future/agent/agent.db"), "not sqlite").unwrap();
    let result = run(
        home.path(),
        &["--grpc-addr", "127.0.0.1:0", "--profile-seconds", "0"],
    );
    assert!(!result.status.success());
    assert_eq!(
        std::fs::read_to_string(dir.join("good.jsonl")).unwrap(),
        original
    );
}
