//! Minimal status server — a zero-dependency HTTP dashboard of goal state
//! (LoopX `serve-status` / dashboard: a read-only PROJECTION; canonical state
//! stays the event ledger — the dashboard is never a second source of truth).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

use anyhow::Result;

use crate::state::{Goal, TodoStatus};
use crate::store::Store;

/// Serve GET-only status endpoints on `addr` (e.g. "127.0.0.1:8791"):
///   GET /            — compact text dashboard of all goals
///   GET /goals.json  — JSON projection
/// Blocks forever.
pub fn serve(store: &Store, addr: &str) -> Result<()> {
    // The store is backed by files; clone the ROOT and reopen per request so
    // each connection thread owns its store (no shared reference escaping).
    let root = store_root_path(store);
    let listener = TcpListener::bind(addr)?;
    println!("future-loop status server listening on http://{addr} (GET / , GET /goals.json)");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let root = root.clone();
                std::thread::spawn(move || {
                    if let Ok(store) = Store::open(&root) {
                        let _ = handle(&store, stream);
                    }
                });
            }
            Err(_) => continue,
        }
    }
    Ok(())
}

fn store_root_path(store: &Store) -> String {
    // Expose the root for reopening in handler threads.
    store.root_path()
}

fn handle(store: &Store, mut stream: TcpStream) -> Result<()> {
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf)?;
    let req = String::from_utf8_lossy(&buf[..n]).to_string();
    let path = req
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/");
    let (status, body) = match path {
        "/goals.json" => {
            let goals: Vec<Goal> = store
                .registry()
                .iter()
                .filter_map(|e| store.replay(&e.goal_id).ok().flatten())
                .collect();
            let json = serde_json::to_string_pretty(&goals)?;
            ("200 OK", json)
        }
        _ => {
            let body = render_dashboard(store);
            ("200 OK", body)
        }
    };
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(resp.as_bytes())?;
    stream.flush()?;
    Ok(())
}

fn render_dashboard(store: &Store) -> String {
    let mut out = String::new();
    out.push_str("=== FutureOS loop control plane — status ===\n\n");
    if store.registry().is_empty() {
        out.push_str("no goals\n");
        return out;
    }
    for entry in store.registry() {
        if let Ok(Some(goal)) = store.replay(&entry.goal_id) {
            out.push_str(&format!("## {}\n", goal.goal_id));
            out.push_str(&format!("objective: {}\n", goal.objective));
            let mut line = String::from("todos:");
            for t in &goal.todos {
                line.push_str(&format!(
                    " {}={}",
                    t.id,
                    match t.status {
                        TodoStatus::Open => "open",
                        TodoStatus::Done => "done",
                        TodoStatus::Superseded => "superseded",
                        TodoStatus::Deferred => "deferred",
                        TodoStatus::Blocked => "blocked",
                    }
                ));
            }
            out.push_str(&line);
            out.push('\n');
            out.push_str(&format!(
                "terminal: {} (validated closure)\n\n",
                goal.terminal_closure().is_some()
            ));
        }
    }
    out
}
