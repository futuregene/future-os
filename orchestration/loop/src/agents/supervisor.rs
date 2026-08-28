//! Supervisor (G-16) — reference `control_plane/agents/supervisor_events.py` +
//! `supervisor.py`, natively (minimal set). The supervisor proposes bounded
//! decisions for target agents; hosts record execution receipts. Both land
//! as ledger events (`SupervisorProposed` / `SupervisorReceiptRecorded`) and
//! are read back through a projection — the goal state itself is untouched.
//!
//! Receipt rules (reference normalize_supervisor_receipt):
//! - a receipt must reference a recorded proposal (`decision_id`);
//! - `observe` decisions never accept host execution receipts;
//! - an `executed` receipt requires the host capabilities the decision
//!   declared AND an explicit `authority_ref` (fail closed otherwise);
//! - one decision accepts at most one executed receipt.

use crate::store::{Event, Store};

pub const SUPERVISOR_RECEIPT_SCHEMA_VERSION: &str = "supervisor_host_receipt_v1";
pub const SUPERVISOR_EVENT_PROJECTION_SCHEMA_VERSION: &str = "supervisor_event_projection_v1";

/// Supervisor decision kinds (reference supervisor decisions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorDecisionKind {
    Observe,
    Execute,
}

impl SupervisorDecisionKind {
    pub fn label(&self) -> &'static str {
        match self {
            SupervisorDecisionKind::Observe => "observe",
            SupervisorDecisionKind::Execute => "execute",
        }
    }
}

/// A normalized supervisor decision (reference normalize_supervisor_decision).
#[derive(Debug, Clone)]
pub struct SupervisorDecision {
    pub decision_id: String,
    pub kind: SupervisorDecisionKind,
    pub target_agent_id: String,
    pub required_host_capabilities: Vec<String>,
    /// Public-safe summary of the decision.
    pub summary: String,
}

impl SupervisorDecision {
    pub fn observe(decision_id: &str, target_agent_id: &str, summary: &str) -> Self {
        Self {
            decision_id: decision_id.to_string(),
            kind: SupervisorDecisionKind::Observe,
            target_agent_id: target_agent_id.to_string(),
            required_host_capabilities: vec![],
            summary: summary.to_string(),
        }
    }

    pub fn execute(
        decision_id: &str,
        target_agent_id: &str,
        required_host_capabilities: Vec<String>,
        summary: &str,
    ) -> Self {
        Self {
            decision_id: decision_id.to_string(),
            kind: SupervisorDecisionKind::Execute,
            target_agent_id: target_agent_id.to_string(),
            required_host_capabilities,
            summary: summary.to_string(),
        }
    }
}

/// Receipt outcomes (reference SupervisorReceiptOutcome).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorReceiptOutcome {
    Executed,
    Rejected,
    Failed,
}

impl SupervisorReceiptOutcome {
    pub fn label(&self) -> &'static str {
        match self {
            SupervisorReceiptOutcome::Executed => "executed",
            SupervisorReceiptOutcome::Rejected => "rejected",
            SupervisorReceiptOutcome::Failed => "failed",
        }
    }
}

/// A normalized host execution receipt.
#[derive(Debug, Clone)]
pub struct SupervisorReceipt {
    pub receipt_id: String,
    pub decision_id: String,
    pub adapter_id: String,
    pub outcome: SupervisorReceiptOutcome,
    pub authority_ref: Option<String>,
    pub rollback_ref: Option<String>,
    pub evidence_refs: Vec<String>,
    pub reason_codes: Vec<String>,
}

/// Record a supervisor proposal (idempotent by decision_id).
pub fn record_supervisor_proposal(
    store: &mut Store,
    goal_id: &str,
    supervisor_agent_id: &str,
    decision: &SupervisorDecision,
) -> anyhow::Result<String> {
    store.append(Event::SupervisorProposed {
        goal_id: goal_id.to_string(),
        supervisor_agent_id: supervisor_agent_id.to_string(),
        decision_id: decision.decision_id.clone(),
        decision_kind: decision.kind.label().to_string(),
        target_agent_id: decision.target_agent_id.clone(),
        required_host_capabilities: decision.required_host_capabilities.clone(),
        decision: decision.summary.clone(),
        ts: crate::state::now_epoch(),
    })
}

/// Record a host execution receipt against a recorded proposal (fail closed
/// on unknown decision / observe decision / missing authority for executed).
pub fn record_supervisor_receipt(
    store: &mut Store,
    goal_id: &str,
    receipt: &SupervisorReceipt,
    host_capabilities: &[String],
) -> anyhow::Result<String> {
    // The proposal must exist in the ledger.
    let events = store.events(goal_id)?;
    let proposal = events.iter().find_map(|stored| match &stored.event {
        Event::SupervisorProposed {
            goal_id: g,
            decision_id,
            ..
        } if g == goal_id && decision_id == &receipt.decision_id => Some(decision_id.clone()),
        _ => None,
    });
    let _proposal = proposal.ok_or_else(|| {
        anyhow::anyhow!(
            "no recorded supervisor proposal for decision_id={}",
            receipt.decision_id
        )
    })?;

    // Find the proposal kind to enforce the observe rule.
    let kind = events.iter().find_map(|stored| match &stored.event {
        Event::SupervisorProposed {
            goal_id: g,
            decision_id,
            decision_kind,
            required_host_capabilities,
            ..
        } if g == goal_id && decision_id == &receipt.decision_id => {
            Some((decision_kind.clone(), required_host_capabilities.clone()))
        }
        _ => None,
    });
    let (kind, required_capabilities) = kind.expect("proposal exists");
    if kind == SupervisorDecisionKind::Observe.label() {
        anyhow::bail!("observe decisions do not accept host execution receipts");
    }

    if receipt.outcome == SupervisorReceiptOutcome::Executed {
        let missing: Vec<String> = required_capabilities
            .iter()
            .filter(|c| !host_capabilities.contains(c))
            .cloned()
            .collect();
        if !missing.is_empty() {
            anyhow::bail!(
                "executed receipt is missing required host capabilities: {:?}",
                missing
            );
        }
        if receipt.authority_ref.is_none() {
            anyhow::bail!("executed receipt requires authority_ref");
        }
    }

    // At most one executed receipt per decision.
    let already_executed = events.iter().any(|stored| match &stored.event {
        Event::SupervisorReceiptRecorded {
            goal_id: g,
            decision_id,
            outcome,
            ..
        } => g == goal_id && decision_id == &receipt.decision_id && outcome == "executed",
        _ => false,
    });
    if already_executed && receipt.outcome == SupervisorReceiptOutcome::Executed {
        anyhow::bail!(
            "decision_id={} already has an executed receipt",
            receipt.decision_id
        );
    }

    store.append(Event::SupervisorReceiptRecorded {
        goal_id: goal_id.to_string(),
        decision_id: receipt.decision_id.clone(),
        receipt_id: receipt.receipt_id.clone(),
        adapter_id: receipt.adapter_id.clone(),
        outcome: receipt.outcome.label().to_string(),
        authority_ref: receipt.authority_ref.clone(),
        rollback_ref: receipt.rollback_ref.clone(),
        ts: crate::state::now_epoch(),
    })
}

/// One projection row: a proposal with its latest receipt.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SupervisorProjectionRow {
    pub decision_id: String,
    pub kind: String,
    pub target_agent_id: String,
    pub supervisor_agent_id: String,
    pub proposed_at: u64,
    pub execution_status: String,
    pub receipt_count: u32,
}

/// Project the supervisor event log for one goal (LoopX
/// build_supervisor_event_projection).
pub fn build_supervisor_event_projection(
    store: &Store,
    goal_id: &str,
) -> anyhow::Result<serde_json::Value> {
    let events = store.events(goal_id)?;
    // Extract at collection time so the projection loop needs no re-match
    // (the event kind is fixed by the arm that pushed it).
    let mut proposals: Vec<(String, String, String, String, u64)> = vec![];
    let mut receipts: Vec<(&str, &str)> = vec![];
    let mut progress: Vec<(String, String, String, u64)> = vec![];
    for stored in &events {
        match &stored.event {
            Event::SupervisorProposed {
                goal_id: g,
                decision_id,
                decision_kind,
                target_agent_id,
                supervisor_agent_id,
                ts,
                ..
            } if g == goal_id => proposals.push((
                decision_id.clone(),
                decision_kind.clone(),
                target_agent_id.clone(),
                supervisor_agent_id.clone(),
                *ts,
            )),
            Event::SupervisorReceiptRecorded {
                goal_id: g,
                decision_id,
                outcome,
                ..
            } if g == goal_id => receipts.push((decision_id.as_str(), outcome.as_str())),
            // Worker mid-run progress notes — advisory, projection-only (see
            // `report` command). Collected for the supervisor's idle-loop
            // consumption; never a push.
            Event::ProgressReported {
                goal_id: g,
                agent_id,
                todo_id,
                message,
                ts,
            } if g == goal_id => {
                progress.push((agent_id.clone(), todo_id.clone(), message.clone(), *ts))
            }
            _ => {}
        }
    }
    let mut rows: Vec<SupervisorProjectionRow> = vec![];
    let mut receipt_count = 0u32;
    for (decision_id, kind, target_agent_id, supervisor_agent_id, ts) in proposals {
        let decision_receipts: Vec<&&str> = receipts
            .iter()
            .filter(|(d, _)| *d == decision_id.as_str())
            .map(|(_, outcome)| outcome)
            .collect();
        receipt_count += decision_receipts.len() as u32;
        let execution_status = decision_receipts
            .last()
            .map(|outcome| outcome.to_string())
            .unwrap_or_else(|| "proposal_only".to_string());
        rows.push(SupervisorProjectionRow {
            decision_id,
            kind,
            target_agent_id,
            supervisor_agent_id,
            proposed_at: ts,
            execution_status,
            receipt_count: decision_receipts.len() as u32,
        });
    }
    let progress_items: Vec<serde_json::Value> = progress
        .into_iter()
        .map(|(agent_id, todo_id, message, ts)| {
            serde_json::json!({
                "agent_id": agent_id,
                "todo_id": todo_id,
                "message": message,
                "ts": ts,
            })
        })
        .collect();
    Ok(serde_json::json!({
        "ok": true,
        "schema_version": SUPERVISOR_EVENT_PROJECTION_SCHEMA_VERSION,
        "goal_id": goal_id,
        "proposal_count": rows.len(),
        "receipt_count": receipt_count,
        "items": rows,
        "progress_count": progress_items.len(),
        "progress": progress_items,
        "boundary": {
            "proposal_is_execution_evidence": false,
            "executed_requires_capability_matched_receipt": true,
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Goal, Todo};
    use crate::store::Store;

    fn tmp_root(tag: &str) -> String {
        let dir = std::env::temp_dir().join(format!(
            "future-loop-p3-supervisor-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().into_owned()
    }

    fn open_goal(store: &mut Store, goal_id: &str) {
        let goal = Goal::new(goal_id, "objective", "/tmp");
        store.register(&goal).unwrap();
        store
            .append(Event::GoalStarted {
                goal_id: goal_id.into(),
                ts: goal.created_at,
            })
            .unwrap();
        let _ = Todo::advancement("t1", "work");
    }

    #[test]
    fn proposal_and_receipt_projection_closes() {
        let root = tmp_root("loop");
        let mut store = Store::open(&root).unwrap();
        open_goal(&mut store, "g1");
        let decision = SupervisorDecision::execute(
            "d-1",
            "agent-b",
            vec!["github".into()],
            "review and merge the change",
        );
        record_supervisor_proposal(&mut store, "g1", "supervisor-1", &decision).unwrap();
        let receipt = SupervisorReceipt {
            receipt_id: "r-1".into(),
            decision_id: "d-1".into(),
            adapter_id: "adapter-x".into(),
            outcome: SupervisorReceiptOutcome::Executed,
            authority_ref: Some("auth-1".into()),
            rollback_ref: Some("rb-1".into()),
            evidence_refs: vec!["ev-1".into()],
            reason_codes: vec!["merge_verified".into()],
        };
        record_supervisor_receipt(&mut store, "g1", &receipt, &["github".into()]).unwrap();
        let projection = build_supervisor_event_projection(&store, "g1").unwrap();
        assert_eq!(projection["proposal_count"], 1);
        assert_eq!(projection["receipt_count"], 1);
        assert_eq!(projection["items"][0]["execution_status"], "executed");
        assert_eq!(projection["items"][0]["kind"], "execute");
        assert_eq!(projection["items"][0]["target_agent_id"], "agent-b");
    }

    #[test]
    fn progress_reports_surface_in_projection() {
        let root = tmp_root("progress");
        let mut store = Store::open(&root).unwrap();
        open_goal(&mut store, "g1");
        store
            .append(Event::ProgressReported {
                goal_id: "g1".into(),
                agent_id: "agent-b".into(),
                todo_id: "todo-1".into(),
                message: "submitted attempt 34444, waiting on score".into(),
                ts: 100,
            })
            .unwrap();
        // A report for another goal must not leak into this projection.
        let goal2 = crate::state::Goal::new("g2", "obj2", "/tmp");
        store.register(&goal2).unwrap();
        store
            .append(Event::GoalStarted {
                goal_id: "g2".into(),
                ts: goal2.created_at,
            })
            .unwrap();
        store
            .append(Event::ProgressReported {
                goal_id: "g2".into(),
                agent_id: "agent-c".into(),
                todo_id: "".into(),
                message: "other goal".into(),
                ts: 101,
            })
            .unwrap();
        let projection = build_supervisor_event_projection(&store, "g1").unwrap();
        assert_eq!(projection["proposal_count"], 0);
        assert_eq!(projection["progress_count"], 1);
        assert_eq!(projection["progress"][0]["agent_id"], "agent-b");
        assert_eq!(projection["progress"][0]["todo_id"], "todo-1");
        assert_eq!(
            projection["progress"][0]["message"],
            "submitted attempt 34444, waiting on score"
        );
        assert_eq!(projection["progress"][0]["ts"], 100);
        // Projection-only: replay must not mutate the goal kanban.
        let goal = store.replay("g1").unwrap().unwrap();
        assert!(goal.todos.is_empty());
    }

    #[test]
    fn executed_receipt_requires_authority_and_capabilities() {
        let root = tmp_root("auth");
        let mut store = Store::open(&root).unwrap();
        open_goal(&mut store, "g1");
        let decision =
            SupervisorDecision::execute("d-2", "agent-b", vec!["github".into()], "merge");
        record_supervisor_proposal(&mut store, "g1", "supervisor-1", &decision).unwrap();

        // Missing authority → fail.
        let receipt = SupervisorReceipt {
            receipt_id: "r-2".into(),
            decision_id: "d-2".into(),
            adapter_id: "a".into(),
            outcome: SupervisorReceiptOutcome::Executed,
            authority_ref: None,
            rollback_ref: None,
            evidence_refs: vec![],
            reason_codes: vec![],
        };
        assert!(record_supervisor_receipt(&mut store, "g1", &receipt, &["github".into()]).is_err());

        // Missing host capability → fail.
        let receipt = SupervisorReceipt {
            receipt_id: "r-3".into(),
            decision_id: "d-2".into(),
            adapter_id: "a".into(),
            outcome: SupervisorReceiptOutcome::Executed,
            authority_ref: Some("auth".into()),
            rollback_ref: None,
            evidence_refs: vec![],
            reason_codes: vec![],
        };
        assert!(record_supervisor_receipt(&mut store, "g1", &receipt, &[]).is_err());

        // Correct → ok.
        assert!(record_supervisor_receipt(&mut store, "g1", &receipt, &["github".into()]).is_ok());
    }

    #[test]
    fn observe_decisions_never_accept_receipts() {
        let root = tmp_root("observe");
        let mut store = Store::open(&root).unwrap();
        open_goal(&mut store, "g1");
        let decision = SupervisorDecision::observe("d-3", "agent-b", "observe the target");
        record_supervisor_proposal(&mut store, "g1", "supervisor-1", &decision).unwrap();
        let receipt = SupervisorReceipt {
            receipt_id: "r-4".into(),
            decision_id: "d-3".into(),
            adapter_id: "a".into(),
            outcome: SupervisorReceiptOutcome::Executed,
            authority_ref: Some("auth".into()),
            rollback_ref: None,
            evidence_refs: vec![],
            reason_codes: vec![],
        };
        let err = record_supervisor_receipt(&mut store, "g1", &receipt, &[]).unwrap_err();
        assert!(err.to_string().contains("observe decisions"));
    }

    #[test]
    fn receipt_without_proposal_fails_closed() {
        let root = tmp_root("orphan");
        let mut store = Store::open(&root).unwrap();
        open_goal(&mut store, "g1");
        let receipt = SupervisorReceipt {
            receipt_id: "r-5".into(),
            decision_id: "d-missing".into(),
            adapter_id: "a".into(),
            outcome: SupervisorReceiptOutcome::Rejected,
            authority_ref: None,
            rollback_ref: None,
            evidence_refs: vec![],
            reason_codes: vec![],
        };
        assert!(record_supervisor_receipt(&mut store, "g1", &receipt, &[]).is_err());
    }

    #[test]
    fn only_one_executed_receipt_per_decision() {
        let root = tmp_root("once");
        let mut store = Store::open(&root).unwrap();
        open_goal(&mut store, "g1");
        let decision = SupervisorDecision::execute("d-4", "agent-b", vec![], "merge");
        record_supervisor_proposal(&mut store, "g1", "supervisor-1", &decision).unwrap();
        let receipt = SupervisorReceipt {
            receipt_id: "r-6".into(),
            decision_id: "d-4".into(),
            adapter_id: "a".into(),
            outcome: SupervisorReceiptOutcome::Executed,
            authority_ref: Some("auth".into()),
            rollback_ref: None,
            evidence_refs: vec![],
            reason_codes: vec![],
        };
        record_supervisor_receipt(&mut store, "g1", &receipt, &[]).unwrap();
        let second = SupervisorReceipt {
            receipt_id: "r-7".into(),
            decision_id: "d-4".into(),
            adapter_id: "a".into(),
            outcome: SupervisorReceiptOutcome::Executed,
            authority_ref: Some("auth".into()),
            rollback_ref: None,
            evidence_refs: vec![],
            reason_codes: vec![],
        };
        assert!(record_supervisor_receipt(&mut store, "g1", &second, &[]).is_err());
    }

    #[test]
    fn rejected_receipt_needs_no_authority() {
        let root = tmp_root("rejected");
        let mut store = Store::open(&root).unwrap();
        open_goal(&mut store, "g1");
        let decision =
            SupervisorDecision::execute("d-5", "agent-b", vec!["github".into()], "merge");
        record_supervisor_proposal(&mut store, "g1", "supervisor-1", &decision).unwrap();
        let receipt = SupervisorReceipt {
            receipt_id: "r-8".into(),
            decision_id: "d-5".into(),
            adapter_id: "a".into(),
            outcome: SupervisorReceiptOutcome::Rejected,
            authority_ref: None,
            rollback_ref: None,
            evidence_refs: vec![],
            reason_codes: vec!["insufficient_evidence".into()],
        };
        assert!(record_supervisor_receipt(&mut store, "g1", &receipt, &[]).is_ok());
    }
}
