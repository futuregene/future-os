use super::*;
use crate::session::{ForkPoint, ForkRequest};

fn request(request_id: &str, parent: &str) -> ForkRequest {
    ForkRequest {
        request_id: request_id.to_string(),
        parent_session_id: parent.to_string(),
        point: ForkPoint::LatestSettled,
        created_by: "test".to_string(),
        creator_id: String::new(),
    }
}

#[test]
fn a_fork_request_needs_its_own_id_and_a_parent_session() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let error = manager
        .create_fork(request("   ", "parent"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("fork request id is required"), "{error}");
    let error = manager
        .create_fork(request("request", " "))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("fork parent session id is required"),
        "{error}"
    );
}
