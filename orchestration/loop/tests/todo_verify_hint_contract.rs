//! O4 contract — `todo add` prints an advisory `--verify` hint for
//! code-like todos (coding keywords / `.rs` paths) and stays silent for
//! ordinary todos or when `--verify` is already given. Runs the REAL built
//! binary against a temp FUTURE_LOOP_ROOT so the actual printed streams are
//! asserted.

use std::process::Command;

const HINT: &str = "hint: 实现类 todo 建议挂 --verify \"cargo check -p ...\"，防不编译代码被标完成";

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_future-loop")
}

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "future-loop-o4-hint-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

/// Run the binary with an isolated FUTURE_LOOP_ROOT. Returns (stdout, stderr,
/// exit code).
fn run(root: &str, args: &[&str]) -> (String, String, i32) {
    let output = Command::new(bin())
        .env("FUTURE_LOOP_ROOT", root)
        .args(args)
        .output()
        .expect("future-loop binary runs");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

fn init(root: &str, goal: &str) {
    let (_, err, code) = run(
        root,
        &[
            "goal",
            "init",
            "--objective",
            "o4 hint contract",
            "--goal-id",
            goal,
            "--cwd",
            "/tmp",
        ],
    );
    assert_eq!(code, 0, "goal init failed: {err}");
}

#[test]
fn code_like_todo_without_verify_prints_hint() {
    let root = tmp_root("feature");
    init(&root, "g1");
    for text in [
        "改写 console.rs 的 todo add",
        "cargo clippy --workspace 全绿后 commit 到 worktree",
        "写单元测试覆盖 store verify",
    ] {
        let (out, err, code) = run(
            &root,
            &[
                "todo",
                "add",
                "--goal",
                "g1",
                "--role",
                "agent",
                "--class",
                "advancement",
                "--text",
                text,
            ],
        );
        assert_eq!(code, 0, "todo add failed: {err}");
        assert!(out.contains("added to g1"), "stdout: {out}");
        assert!(
            err.contains(HINT),
            "hint missing for {text:?}\nstderr: {err}"
        );
    }
}

#[test]
fn ordinary_todo_does_not_print_hint() {
    let root = tmp_root("plain");
    init(&root, "g1");
    let (out, err, code) = run(
        &root,
        &[
            "todo",
            "add",
            "--goal",
            "g1",
            "--role",
            "agent",
            "--class",
            "advancement",
            "--text",
            "整理会议纪要",
        ],
    );
    assert_eq!(code, 0, "todo add failed: {err}");
    assert!(out.contains("added to g1"), "stdout: {out}");
    assert!(!out.contains("hint:"), "unexpected hint on stdout: {out}");
    assert!(!err.contains("hint:"), "unexpected hint on stderr: {err}");
}

#[test]
fn code_like_todo_with_verify_does_not_print_hint() {
    let root = tmp_root("verified");
    init(&root, "g1");
    let (out, err, code) = run(
        &root,
        &[
            "todo",
            "add",
            "--goal",
            "g1",
            "--role",
            "agent",
            "--class",
            "advancement",
            "--text",
            "修复 src/lib.rs 的编译错误",
            "--verify",
            "cargo check -p future-loop",
        ],
    );
    assert_eq!(code, 0, "todo add failed: {err}");
    assert!(out.contains("added to g1"), "stdout: {out}");
    assert!(!out.contains("hint:"), "unexpected hint on stdout: {out}");
    assert!(!err.contains("hint:"), "unexpected hint on stderr: {err}");
}
