//! The materialized-page cache: what it may answer, and what it must never
//! answer.
//!
//! A page is read again on every switch (the client re-reads the tail to learn
//! whether the conversation moved on while it was away), so it has to be cheap
//! the second time; and the revision it was built from is the one fact that
//! decides whether the answer is still true.

use super::*;
use crate::session::Manager;

fn exchange(index: usize) -> Vec<Value> {
    vec![
        json!({"id": format!("u{index}"), "type": "user", "role": "user",
            "timestamp": "2026-10-30T00:00:00Z", "content": format!("question {index}")}),
        json!({"id": format!("a{index}"), "type": "assistant", "role": "assistant",
            "timestamp": "2026-10-30T00:00:01Z",
            "content": [{"type": "text", "text": format!("answer {index}")}]}),
    ]
}

fn manager_with(exchanges: usize) -> (tempfile::TempDir, Manager) {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let store = manager.storage().unwrap();
    store.reserve_segmented("s").unwrap();
    let entries: Vec<Value> = (0..exchanges).flat_map(exchange).collect();
    store.replace("s", entries).unwrap();
    (dir, manager)
}

#[test]
fn a_page_read_twice_is_the_same_answer_and_one_cache_slot() {
    let (_dir, manager) = manager_with(3);
    let first = manager.history_page("s", None, Some(0), Some(10)).unwrap();
    assert_eq!(manager.history_pages.lock().len(), 1, "the page is kept");
    let second = manager.history_page("s", None, Some(0), Some(10)).unwrap();
    assert_eq!(second, first, "a cache hit answers with the same page");
    assert_eq!(manager.history_pages.lock().len(), 1, "one page, one slot");
}

/// The revision is bumped by every write to the conversation, so a page read
/// beside it can never be the one served afterwards.
#[test]
fn a_write_to_the_conversation_is_never_served_from_the_cache() {
    let (_dir, manager) = manager_with(2);
    let first = manager.history_page("s", None, Some(0), Some(10)).unwrap();
    assert_eq!(first["entries"].as_array().unwrap().len(), 4);

    let store = manager.storage().unwrap();
    let entries: Vec<Value> = (0..3).flat_map(exchange).collect();
    store.replace("s", entries).unwrap();

    let after = manager.history_page("s", None, Some(0), Some(10)).unwrap();
    assert_eq!(
        after["entries"].as_array().unwrap().len(),
        6,
        "the third exchange is on the page"
    );
    assert_ne!(after, first);
}

/// Pages of one conversation are separate answers: two windows of the same
/// conversation share a session and a revision, and must not stand in for each
/// other.
#[test]
fn the_pages_of_one_conversation_are_cached_apart() {
    let (_dir, manager) = manager_with(4);
    let first = manager.history_page("s", None, Some(0), Some(1)).unwrap();
    let second = manager.history_page("s", None, Some(2), Some(1)).unwrap();
    let tail = manager
        .history_page("s", Some(i64::MAX), Some(0), Some(1))
        .unwrap();
    assert_ne!(first, second, "different windows, different pages");
    assert_ne!(first, tail, "a tail page is its own window");
    assert_eq!(manager.history_pages.lock().len(), 3);
}

#[test]
fn the_cache_forgets_the_oldest_page_before_it_grows_past_its_bound() {
    let (_dir, manager) = manager_with(4);
    for offset in 0..(PAGE_CACHE_CAPACITY as i64 + 4) {
        manager
            .history_page("s", None, Some(offset), Some(1))
            .unwrap();
    }
    let cache = manager.history_pages.lock();
    assert!(
        cache.len() <= PAGE_CACHE_CAPACITY,
        "held {} pages",
        cache.len()
    );
    assert!(
        cache.bytes() <= PAGE_CACHE_BYTES,
        "held {} bytes",
        cache.bytes()
    );
}
