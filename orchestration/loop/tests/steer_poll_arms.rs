//! The steering-poll seam's failure arms.
//!
//! `steer_worker_poll_once` is `#[doc(hidden)] pub` precisely so tests can drive
//! it. The happy paths (offset advance, broadcast target) already have a test;
//! what follows are the arms that decide whether an interrupt can be LOST, which
//! is the property that matters - the function must always return the OLD offset
//! when it could not deliver, so the next poll retries the same event.

mod common;

use common::{init_goal, open_store};
use std::io::Write;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Build a ledger file with the given raw bytes, returning its path.
fn ledger_with(root: &str, goal: &str, bytes: &[u8]) -> std::path::PathBuf {
    let dir = std::path::Path::new(root).join("goals").join(goal);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("events.jsonl");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(bytes).unwrap();
    path
}

/// Three ways the poll must make NO progress, each returning the offset it was
/// given so the event is retried rather than skipped:
///   1. the path does not exist,
///   2. nothing new has been appended (`len <= offset`),
///   3. the new bytes contain no complete line yet (a partially written event).
#[test]
fn the_poll_returns_the_old_offset_when_it_cannot_make_progress() {
    use future_loop::console::steer_worker_poll_once;
    let cr = common::cli_root();
    let gid = init_goal(&cr, "steer no-progress arms");
    let rt = rt();

    // 1. Missing path.
    let missing = std::path::Path::new(&cr.root).join("no-such-ledger.jsonl");
    let mut client = None;
    let out = rt.block_on(async {
        steer_worker_poll_once(&missing, 7, Some("w1"), &mut client, "sess").await
    });
    assert_eq!(out, 7, "a missing ledger must not advance the offset");
    assert!(client.is_none(), "and must not open a client");

    // 2. Nothing new appended since the offset.
    let path = ledger_with(
        &cr.root,
        &gid,
        b"{\"kind\":\"worker_steered\",\"agent_id\":\"w1\"}\n",
    );
    let len = std::fs::metadata(&path).unwrap().len();
    let out = rt.block_on(async {
        steer_worker_poll_once(&path, len, Some("w1"), &mut client, "sess").await
    });
    assert_eq!(out, len, "no new bytes means no progress");

    // 3. A partially written line (no trailing newline) is left for next time.
    let path = ledger_with(&cr.root, &gid, b"{\"kind\":\"worker_steered\"");
    let out = rt.block_on(async {
        steer_worker_poll_once(&path, 0, Some("w1"), &mut client, "sess").await
    });
    assert_eq!(
        out, 0,
        "an incomplete line must be retried, not consumed: the event would be lost"
    );
}

/// An interrupt that CANNOT be delivered (no agent reachable) must return the old
/// offset: the comment in the implementation says it plainly - "retry undelivered
/// interrupt; never lose the event". A poll that advanced the offset here would
/// silently drop a supervisor's steering instruction.
#[test]
fn an_undeliverable_interrupt_does_not_advance_the_offset() {
    use future_loop::console::steer_worker_poll_once;
    let cr = common::cli_root();
    let gid = init_goal(&cr, "steer undeliverable");
    let rt = rt();
    // Point at a port nothing listens on so the client cannot be created.
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", "http://127.0.0.1:1");

    let path = ledger_with(
        &cr.root,
        &gid,
        b"{\"kind\":\"worker_steered\",\"agent_id\":\"w1\",\"instruction\":\"redirect\",\"ts\":1}\n",
    );
    let mut client = None;
    let out = rt.block_on(async {
        steer_worker_poll_once(&path, 0, Some("w1"), &mut client, "sess").await
    });
    assert_eq!(
        out, 0,
        "an interrupt that could not be delivered must be retried from the same offset"
    );
    assert!(
        client.is_none(),
        "a failed connect must leave the client unset for the next attempt"
    );

    // A NON-interrupting control instruction is not an abort trigger: the poll
    // skips it and advances, because ordinary guidance waits for the next turn
    // boundary rather than interrupting the running one.
    let path = ledger_with(
        &cr.root,
        &gid,
        b"{\"kind\":\"control_issued\",\"instruction\":{\"id\":\"i1\",\"agent_id\":\"w1\",\"text\":\"hold\",\"interrupt\":false},\"ts\":1}\n",
    );
    let len = std::fs::metadata(&path).unwrap().len();
    let out = rt.block_on(async {
        steer_worker_poll_once(&path, 0, Some("w1"), &mut client, "sess").await
    });
    assert_eq!(
        out, len,
        "a non-interrupting instruction must be consumed without aborting"
    );

    // A steer addressed to ANOTHER worker is skipped the same way.
    let path = ledger_with(
        &cr.root,
        &gid,
        b"{\"kind\":\"worker_steered\",\"agent_id\":\"other\",\"instruction\":\"x\",\"ts\":1}\n",
    );
    let len = std::fs::metadata(&path).unwrap().len();
    let out = rt.block_on(async {
        steer_worker_poll_once(&path, 0, Some("w1"), &mut client, "sess").await
    });
    assert_eq!(out, len, "another worker's steer must not abort this one");

    // Unparsable lines are skipped without losing the offset arithmetic.
    let path = ledger_with(&cr.root, &gid, b"not json at all\n");
    let len = std::fs::metadata(&path).unwrap().len();
    let out = rt.block_on(async {
        steer_worker_poll_once(&path, 0, Some("w1"), &mut client, "sess").await
    });
    assert_eq!(out, len, "a corrupt line is consumed, not retried forever");
    let _ = open_store(&cr);
}
