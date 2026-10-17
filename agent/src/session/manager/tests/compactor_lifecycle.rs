//! The optional journal compactor's lifecycle, as the host sees it.

use super::*;

/// The compactor is started by `initialize`, reports its pending work to the
/// activity snapshot, and is torn down by `shutdown`. A request after shutdown
/// must warn instead of starting a worker behind a store that is closing.
#[test]
fn the_journal_compactor_is_started_reported_requested_and_torn_down() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    assert_eq!(
        manager.journal_compaction_activity(),
        Some(0),
        "a manager that has not been initialized owns no compactor, so nothing is pending"
    );

    manager.initialize().expect("a fresh store initializes");
    assert_eq!(
        manager.journal_compaction_activity(),
        Some(0),
        "an initialized manager owns a compactor with nothing pending"
    );

    // The request reaches the compactor, which owns the pending counter.
    manager.request_journal_compaction("s", "r");
    let pending = manager
        .journal_compaction_activity()
        .expect("the compactor is still owned");
    assert!(
        pending >= 1,
        "a requested compaction stays pending until the worker completes it: {pending}"
    );

    manager.shutdown().expect("shutdown joins the compactor");
    assert_eq!(
        manager.journal_compaction_activity(),
        Some(0),
        "a stopped manager has no compactor to report"
    );

    // A request after shutdown warns and returns rather than spawning a worker.
    manager.request_journal_compaction("s", "r");
}
