//! Coverage drive — subprocess-only paths: the stdio worker bridge (piped
//! stdin), the blocking HTTP status server, and the `todo update --help`
//! early process exit. These cannot run in-process (stdin ownership,
//! blocking accept loop, exit(0)), so they spawn the real binary — which is
//! instrumented, so the coverage still lands.

use std::io::Write;
use std::process::{Command, Stdio};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_future-loop")
}

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "future-loop-sub-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn run(root: &str, args: &[&str]) -> (String, String, i32) {
    let output = Command::new(bin())
        .env("FUTURE_LOOP_ROOT", root)
        .args(args)
        .output()
        .expect("binary runs");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

/// Spawn with piped stdin; write `input`, close, collect.
fn run_stdin(root: &str, args: &[&str], input: &str) -> (String, String, i32) {
    let mut child = Command::new(bin())
        .env("FUTURE_LOOP_ROOT", root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("binary spawns");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

fn init_goal(root: &str, objective: &str) -> String {
    let gid = format!("goal_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let cwd = format!("{root}/cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    let (_, _, code) = run(
        root,
        &[
            "goal",
            "init",
            "--objective",
            objective,
            "--goal-id",
            &gid,
            "--cwd",
            &cwd,
        ],
    );
    assert_eq!(code, 0);
    gid
}

// ── worker-bridge ──────────────────────────────────────────────────────────

#[test]
fn worker_bridge_worker_finishes_on_eof_and_done() {
    let root = tmp_root("bridge-eof");
    let gid = init_goal(&root, "bridge eof");
    // EOF on stdin → "worker finished".
    let (out, _, code) = run_stdin(&root, &["worker-bridge", "--goal", &gid], "");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("BRIDGE packet:"), "{out}");
    assert!(out.contains("worker finished"), "{out}");
    // Explicit "BRIDGE done" line takes the same close path.
    let (out, _, code) = run_stdin(&root, &["worker-bridge", "--goal", &gid], "BRIDGE done\n");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("worker finished"), "{out}");
}

#[test]
fn worker_bridge_invalid_result_line() {
    let root = tmp_root("bridge-bad");
    let gid = init_goal(&root, "bridge invalid");
    let (_, err, code) = run_stdin(&root, &["worker-bridge", "--goal", &gid], "not json\n");
    assert_ne!(code, 0);
    assert!(err.contains("invalid worker result"), "{err}");
}

#[test]
fn worker_bridge_completed_turn_to_terminal() {
    let root = tmp_root("bridge-done");
    let gid = init_goal(&root, "bridge completes");
    // One completed result for the onboarding todo → next decide is terminal.
    let onboarding_text = "future loop status";
    let result =
        "{\"todo_id\":\"TODO\",\"terminal_state\":\"completed\",\"evidence\":\"validated\",\"tools\":[\"shell\"]}\n".to_string();
    // The bridge selects the todo; we must answer with ITS id. Read the first
    // packet line to learn the id? Simpler: complete every todo via two
    // turns — but we only have one todo. Use a two-phase exchange.
    let mut child = Command::new(bin())
        .env("FUTURE_LOOP_ROOT", &root)
        .args(["worker-bridge", "--goal", &gid])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Read the packet line, extract todo_id, answer.
    use std::io::{BufRead, BufReader};
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("BRIDGE packet:"), "{line}");
    let json_start = line.find('{').unwrap();
    let packet: serde_json::Value = serde_json::from_str(&line[json_start..]).unwrap();
    let todo_id = packet["todo_id"].as_str().unwrap().to_string();
    assert!(packet["todo_text"]
        .as_str()
        .unwrap()
        .contains(onboarding_text));
    let answer = result.replace("TODO", &todo_id);
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(answer.as_bytes())
        .unwrap();
    // writeback line, then the next decide reports terminal.
    let mut line2 = String::new();
    reader.read_line(&mut line2).unwrap();
    assert!(line2.contains("BRIDGE writeback"), "{line2}");
    let mut line3 = String::new();
    reader.read_line(&mut line3).unwrap();
    assert!(line3.contains("BRIDGE terminal"), "{line3}");
    let status = child.wait().unwrap();
    assert!(status.success());
}

#[test]
fn worker_bridge_failed_turns_hit_max_turns() {
    let root = tmp_root("bridge-max");
    let gid = init_goal(&root, "bridge max turns");
    let result = "{\"todo_id\":\"x\",\"terminal_state\":\"failed\",\"error\":\"boom\"}\n";
    let input = result.repeat(8);
    let (_, err, code) = run_stdin(
        &root,
        &["worker-bridge", "--goal", &gid, "--max-turns", "2"],
        &input,
    );
    assert_ne!(code, 0);
    assert!(err.contains("max-turns"), "{err}");
}

#[test]
fn worker_bridge_successor_chain_on_non_final_todo() {
    let root = tmp_root("bridge-succ");
    let gid = init_goal(&root, "bridge successors");
    // A second open todo → completing the selected one names it as successor.
    let (_, _, code) = run(
        &root,
        &["todo", "add", "--goal", &gid, "--text", "second task"],
    );
    assert_eq!(code, 0);
    let mut child = Command::new(bin())
        .env("FUTURE_LOOP_ROOT", &root)
        .args(["worker-bridge", "--goal", &gid, "--max-turns", "2"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::{BufRead, BufReader, Read};
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let json_start = line.find('{').unwrap();
    let packet: serde_json::Value = serde_json::from_str(&line[json_start..]).unwrap();
    let todo_id = packet["todo_id"].as_str().unwrap().to_string();
    let answer = format!(
        "{{\"todo_id\":\"{todo_id}\",\"terminal_state\":\"completed\",\"evidence\":\"done\",\"tools\":[\"shell\"]}}\n"
    );
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(answer.as_bytes())
        .unwrap();
    // writeback print, then EOF (stdin closed) → worker finished close.
    drop(child.stdin.take());
    let mut rest = String::new();
    Read::read_to_string(&mut reader, &mut rest).unwrap();
    assert!(rest.contains("BRIDGE writeback"), "{rest}");
    let status = child.wait().unwrap();
    assert!(status.success());
    // The completed todo carries the remaining one as its successor.
    let store = future_loop::store::Store::open(&root).unwrap();
    let g = store.replay(&gid).unwrap().unwrap();
    let done = g.todos.iter().find(|t| t.id == todo_id).unwrap();
    assert_eq!(done.successor_ids.len(), 1, "{done:?}");
}

#[test]
fn worker_bridge_ignores_unknown_flags() {
    let root = tmp_root("bridge-bogus");
    let gid = init_goal(&root, "bridge bogus flag");
    let (out, _, code) = run_stdin(&root, &["worker-bridge", "--goal", &gid, "--bogus", "x"], "");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("BRIDGE packet:"), "{out}");
}

#[test]
fn worker_bridge_empty_terminal_state_defaults_to_completed() {
    let root = tmp_root("bridge-empty-state");
    let gid = init_goal(&root, "bridge empty terminal state");
    let mut child = Command::new(bin())
        .env("FUTURE_LOOP_ROOT", &root)
        .args(["worker-bridge", "--goal", &gid])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::{BufRead, BufReader, Read};
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let json_start = line.find('{').unwrap();
    let packet: serde_json::Value = serde_json::from_str(&line[json_start..]).unwrap();
    let todo_id = packet["todo_id"].as_str().unwrap().to_string();
    // Empty terminal_state falls back to "completed" in the run record.
    let answer = format!(
        "{{\"todo_id\":\"{todo_id}\",\"terminal_state\":\"\",\"evidence\":\"done\",\"tools\":[\"shell\"]}}\n"
    );
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(answer.as_bytes())
        .unwrap();
    drop(child.stdin.take());
    let mut rest = String::new();
    Read::read_to_string(&mut reader, &mut rest).unwrap();
    assert!(rest.contains("BRIDGE writeback"), "{rest}");
    let status = child.wait().unwrap();
    assert!(status.success());
    let store = future_loop::store::Store::open(&root).unwrap();
    let g = store.replay(&gid).unwrap().unwrap();
    let rec = g.history.last().expect("run recorded");
    assert_eq!(rec.terminal_state, "completed");
}

#[test]
fn worker_bridge_stops_when_should_run_false() {
    let root = tmp_root("bridge-stop");
    let gid = init_goal(&root, "bridge stopped goal");
    // Complete the onboarding todo, then leave only a not-due monitor →
    // WaitMonitor (should_run=false, non-terminal) → BRIDGE stop.
    let store = future_loop::store::Store::open(&root).unwrap();
    let g = store.replay(&gid).unwrap().unwrap();
    let onboarding = g.todos.first().unwrap().id.clone();
    drop(g);
    drop(store);
    let (_, _, code) = run(
        &root,
        &[
            "todo",
            "complete",
            "--goal",
            &gid,
            "--todo-id",
            &onboarding,
            "--no-follow-up",
        ],
    );
    assert_eq!(code, 0);
    let mut store = future_loop::store::Store::open(&root).unwrap();
    store
        .append(future_loop::store::Event::TodoAdded {
            goal_id: gid.clone(),
            todo: future_loop::state::Todo::monitor(
                "mon_wait",
                "watch it",
                std::time::Duration::from_secs(3600),
            ),
            ts: future_loop::state::now_epoch(),
        })
        .unwrap();
    // Keep the projection honest (next action = the monitor's text).
    store.set_next_action(&gid, "watch it").unwrap();
    drop(store);
    let (out, _, code) = run_stdin(&root, &["worker-bridge", "--goal", &gid], "");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("BRIDGE stop"), "{out}");
}

#[test]
fn worker_bridge_errors() {
    let root = tmp_root("bridge-err");
    let (_, err, code) = run(&root, &["worker-bridge"]);
    assert_ne!(code, 0);
    assert!(err.contains("--goal required"), "{err}");
    let (_, err, code) = run(&root, &["worker-bridge", "--goal", "goal_nope"]);
    assert_ne!(code, 0);
    assert!(err.contains("not found"), "{err}");
}

// ── serve-status ───────────────────────────────────────────────────────────

/// Find a free TCP port (best-effort; released before the server binds).
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn http_get(port: u16, path: &str) -> String {
    use std::io::Read;
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
        .unwrap();
    let mut buf = String::new();
    stream.read_to_string(&mut buf).unwrap();
    buf
}

fn spawn_server(root: &str, port: u16) -> std::process::Child {
    let mut child = Command::new(bin())
        .env("FUTURE_LOOP_ROOT", root)
        .args(["serve-status", "--port", &port.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // Wait for the listen line.
    use std::io::{BufRead, BufReader};
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("listening"), "{line}");
    child
}

#[test]
fn serve_status_dashboard_and_json() {
    let root = tmp_root("serve");
    let gid = init_goal(&root, "serve goal");
    let port = free_port();
    let mut child = spawn_server(&root, port);
    let dash = http_get(port, "/");
    assert!(dash.contains("status"), "{dash}");
    assert!(dash.contains(&gid), "{dash}");
    let json = http_get(port, "/goals.json");
    assert!(json.contains(&gid), "{json}");
    // Unknown path → dashboard; garbage request line → dashboard default.
    let other = http_get(port, "/nope");
    assert!(other.contains("status"), "{other}");
    {
        use std::io::Read;
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.write_all(b"GARBAGE\r\n\r\n").unwrap();
        let mut buf = String::new();
        stream.read_to_string(&mut buf).unwrap();
        assert!(buf.contains("200 OK"), "{buf}");
    }
    child.kill().unwrap();
    child.wait().unwrap();

    // Empty registry → "no goals" dashboard.
    let root2 = tmp_root("serve-empty");
    let port2 = free_port();
    let mut child2 = spawn_server(&root2, port2);
    let dash = http_get(port2, "/");
    assert!(dash.contains("no goals"), "{dash}");
    child2.kill().unwrap();
    child2.wait().unwrap();
}

// ── todo update --help (process exit) ──────────────────────────────────────

#[test]
fn todo_update_help_exits_zero() {
    let root = tmp_root("help-exit");
    let (_, _, code) = run(&root, &["todo", "update", "--help"]);
    assert_eq!(code, 0);
}
