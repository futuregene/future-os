use super::*;
use crate::session::manager::display_cache::DISPLAY_PROJECTION_CACHE_CAPACITY;
use std::sync::Arc;

#[test]
fn a_cache_hit_is_promoted_and_the_oldest_entry_is_evicted() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let mut session = crate::session::Session::new("/synthetic", "mock");
    session.entries.push(crate::session::SessionEntry::new_user(
        "user",
        serde_json::json!("question"),
    ));
    manager.save(&session).unwrap();
    let revision = manager.session_revision(&session.id).unwrap();
    manager.cache_display_projection(
        &session.id,
        revision.clone(),
        Arc::new(vec![serde_json::json!({"id":"entry"})]),
    );
    assert_eq!(
        manager
            .cached_display_projection(&session.id, &revision)
            .unwrap()
            .len(),
        1
    );
    // The hit was promoted, so it is still cached on the next read.
    assert!(manager
        .cached_display_projection(&session.id, &revision)
        .is_some());

    for index in 0..=DISPLAY_PROJECTION_CACHE_CAPACITY {
        manager.cache_display_projection(
            &format!("filler-{index}"),
            revision.clone(),
            Arc::new(Vec::new()),
        );
    }
    assert!(
        manager
            .cached_display_projection(&session.id, &revision)
            .is_none(),
        "the cap evicts the oldest entry"
    );
}
