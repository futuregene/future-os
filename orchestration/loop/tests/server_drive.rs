//! In-process coverage for `status_server.rs` (the blocking HTTP dashboard).
//! The server loops forever, so it runs on a leaked background thread — the
//! test process exits reaps it, and in-process execution means the coverage
//! is actually recorded (a killed subprocess writes no profraw).

use std::io::{Read, Write};

use future_loop::state::{now_epoch, Goal, Todo};
use future_loop::status_server::serve;
use future_loop::store::{Event, Store};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn http_get(port: u16, request: &str) -> String {
    let mut last_err = None;
    for _ in 0..100 {
        match std::net::TcpStream::connect(("127.0.0.1", port)) {
            Ok(mut stream) => {
                stream.write_all(request.as_bytes()).unwrap();
                let mut buf = String::new();
                stream.read_to_string(&mut buf).unwrap();
                return buf;
            }
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }
    panic!("server never accepted connections: {last_err:?}");
}

#[test]
fn status_server_serves_dashboard_and_json() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("loop-root");
    std::fs::create_dir_all(&root).unwrap();
    let root = root.to_string_lossy().into_owned();

    let port = free_port();
    let addr = format!("127.0.0.1:{port}");
    let root_for_thread = root.clone();
    std::thread::spawn(move || {
        let store = Store::open(&root_for_thread).unwrap();
        // Blocks forever on accept; the thread is abandoned at test end.
        let _ = serve(&store, &addr);
    });

    // Empty registry → the "no goals" dashboard (store reopened per request).
    let dash = http_get(port, "GET / HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(dash.contains("no goals"), "{dash}");

    // Register a goal with one todo; the next request sees it (per-request
    // store reopen).
    {
        let mut store = Store::open(&root).unwrap();
        let goal = Goal::new("goal_srv", "serve the dashboard", "/tmp");
        store.register(&goal).unwrap();
        store
            .append(Event::GoalStarted {
                goal_id: "goal_srv".to_string(),
                ts: now_epoch(),
            })
            .unwrap();
        let mut done = Todo::advancement("todo_done", "finished task");
        done.status = future_loop::state::TodoStatus::Done;
        let mut superseded = Todo::advancement("todo_sup", "old task");
        superseded.status = future_loop::state::TodoStatus::Superseded;
        let mut deferred = Todo::advancement("todo_def", "later task");
        deferred.status = future_loop::state::TodoStatus::Deferred;
        let mut blocked = Todo::advancement("todo_blk", "blocked task");
        blocked.status = future_loop::state::TodoStatus::Blocked;
        for t in [done, superseded, deferred, blocked, Todo::advancement("todo_open", "open task")] {
            store
                .append(Event::TodoAdded {
                    goal_id: "goal_srv".to_string(),
                    todo: t,
                    ts: now_epoch(),
                })
                .unwrap();
        }
    }
    let dash = http_get(port, "GET / HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(dash.contains("goal_srv"), "{dash}");
    assert!(dash.contains("todo_open=open"), "{dash}");
    assert!(dash.contains("todo_done=done"), "{dash}");
    assert!(dash.contains("todo_sup=superseded"), "{dash}");
    assert!(dash.contains("todo_def=deferred"), "{dash}");
    assert!(dash.contains("todo_blk=blocked"), "{dash}");

    let json = http_get(port, "GET /goals.json HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(json.contains("goal_srv"), "{json}");

    // A garbage request line falls back to the dashboard.
    let garbage = http_get(port, "GARBAGE\r\n\r\n");
    assert!(garbage.contains("200 OK"), "{garbage}");

    // serve() on an already-bound address errors immediately (deterministic
    // bind failure — the first server holds the port).
    let store = Store::open(&root).unwrap();
    let busy = format!("127.0.0.1:{port}");
    assert!(serve(&store, &busy).is_err());
}
