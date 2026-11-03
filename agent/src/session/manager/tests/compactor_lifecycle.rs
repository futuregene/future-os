//! Production session lifetime never starts a historical compression worker.
use super::*;
#[test]
fn initialize_and_shutdown_do_not_schedule_compaction() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    assert_eq!(manager.journal_compaction_activity(), Some(0));
    manager.initialize().unwrap();
    assert_eq!(manager.journal_compaction_activity(), Some(0));
    manager.shutdown().unwrap();
    assert_eq!(manager.journal_compaction_activity(), Some(0));
}
