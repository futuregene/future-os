use super::escalation::*;
use super::shape::*;
use super::*;
use crate::sandbox::SandboxPolicy;

mod diagnostics;
mod flow;
mod shape;
mod windows;
fn temp_ws(name: &str) -> String {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("futureos-approval-{name}-{stamp}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().to_string()
}

fn enabled(ws: &str) -> ResolvedSandbox {
    ResolvedSandbox::resolve(
        &SandboxPolicy {
            tier: crate::sandbox::SandboxTier::Manual,
            model_reviewer: false,
        },
        ws,
    )
}

/// A path outside the workspace and temp (never created).
fn outside(name: &str) -> String {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    dirs::home_dir()
        .unwrap()
        .join(format!("futureos-approval-outside-{name}-{stamp}.txt"))
        .to_string_lossy()
        .to_string()
}

fn spawn_decider(
    gate: &ApprovalGate,
    session_id: &str,
    decision: ApprovalDecision,
) -> std::thread::JoinHandle<bool> {
    let gate = gate.clone();
    let session_id = session_id.to_string();
    std::thread::spawn(move || poll_and_decide(&gate, &session_id, decision, 2000))
}

fn poll_and_decide(
    gate: &ApprovalGate,
    session_id: &str,
    decision: ApprovalDecision,
    max_polls: u32,
) -> bool {
    for _ in 0..max_polls {
        let pending = gate.pending_for_session(session_id);
        if let Some(first) = pending.first() {
            let request_id = first["approval_request_id"].as_str().unwrap().to_string();
            let _ = gate.decide(&request_id, session_id, decision);
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    false
}

fn approved() -> ApprovalDecision {
    ApprovalDecision {
        approved: true,
        note: String::new(),
        status: ApprovalDecisionStatus::Approved,
    }
}
