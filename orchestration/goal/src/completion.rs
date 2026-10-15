//! Completion evidence shared by manual and automatic paths. These checks
//! enforce declared contracts, not the truth of a scientific claim.
use crate::state::{Goal, RunRecord, TaskClass, Todo, TodoStatus};

/// The caller owns explicit manual overrides; automatic writeback never bypasses
/// this check. Validate the evidence that will actually survive in the ledger.
pub fn evidence_error(todo: &Todo, evidence: &str) -> Option<String> {
    if todo.class != TaskClass::Advancement {
        return None;
    }
    let evidence = evidence.trim();
    if evidence.is_empty() {
        return Some(format!(
            "todo {} needs non-empty --evidence (artifacts, results and verification); --force is a manual operator override only",
            todo.id
        ));
    }
    if let Some(acceptance) = &todo.acceptance {
        let lower = evidence.to_lowercase();
        let missing: Vec<_> = acceptance
            .split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty() && !lower.contains(&token.to_lowercase()))
            .collect();
        if !missing.is_empty() {
            return Some(format!(
                "todo {} acceptance contract unmet: evidence must contain [{}] (missing: [{}]); repair the handoff, not the criteria",
                todo.id, acceptance, missing.join(", ")
            ));
        }
    }
    None
}

pub fn check_record(todo: &Todo, record: &mut RunRecord) {
    if record.terminal_state == "completed" && record.error.is_none() {
        record.error = evidence_error(todo, &record.evidence);
    }
}

/// A rolling UTF-8-safe tail. Full assistant text is retained in the live journal;
/// bounded summaries prioritize the latest outcome, not the opening plan.
pub fn tail(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    const MARK: &str = "…";
    if max_bytes < MARK.len() {
        return String::new();
    }
    let start = text.ceil_char_boundary(text.len() - (max_bytes - MARK.len()));
    format!("{MARK}{}", &text[start..])
}

/// Notification != global closure. Count all pending classes/owners, including
/// deferred/blocked work; never infer global state from the shared frontier.
pub fn pending_others(goal: &Goal, todo_id: &str) -> usize {
    goal.todos
        .iter()
        .filter(|t| {
            t.id != todo_id && !matches!(t.status, TodoStatus::Done | TodoStatus::Superseded)
        })
        .count()
}

/// A structured, bounded handoff with trusted run identity and a full-evidence
/// pointer. Artifact/score claims remain in evidence; do not invent parsed facts.
pub fn notice(
    goal: &Goal,
    record: &RunRecord,
    agent_id: Option<&str>,
    session_id: &str,
    journal: &std::path::Path,
) -> String {
    let validation = record.validation.as_ref().map(|v| {
        serde_json::json!({
            "status": v.status,
            "ok": v.ok,
            "exit_code": v.exit_code,
            "diagnostics_tail": tail(&v.summary, 400),
        })
    });
    serde_json::json!({
        "type": "task_delivery",
        "goal_id": goal.goal_id,
        "todo_id": record.todo_id,
        "agent_id": agent_id,
        "session_id": session_id,
        "run_id": record.run_id,
        "execution_state": record.terminal_state,
        "delivery": "awaiting_review",
        "pending_other_todos": pending_others(goal, &record.todo_id),
        "validation": validation,
        "evidence_tail": tail(&record.evidence, 800),
        "full_text_journal": journal,
        "next_action": "Read artifacts and current goal state; a task delivery is not global completion.",
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_evidence_gate_checks_empty_and_all_tokens() {
        let mut todo = Todo::advancement("t", "deliver");
        todo.acceptance = Some("attempt, scored".into());
        assert!(evidence_error(&todo, " ").is_some());
        assert!(evidence_error(&todo, "attempt queued").is_some());
        assert!(evidence_error(&todo, "ATTEMPT 123 SCORED 0; failed science").is_none());
        // Tokens are a contract floor, NOT a guarantee of positive results.
    }

    #[test]
    fn tail_keeps_late_multibyte_results_with_a_byte_bound() {
        for limit in 0..40 {
            let result = tail(&"原始证据".repeat(30), limit);
            assert!(result.len() <= limit);
        }
        let text = format!("{}FINAL: attempt 123 scored", "初期探索".repeat(4000));
        assert!(tail(&text, 4000).ends_with("FINAL: attempt 123 scored"));
    }

    #[test]
    fn pending_counts_owners_and_blocked_work_not_the_shared_frontier() {
        let mut goal = Goal::new("g", "work", ".");
        goal.add(Todo::advancement("a", "a").owned_by("alice"));
        goal.add(Todo::advancement("b", "b").owned_by("bob"));
        goal.add(Todo::coordination("review", "review").blocking(&["a", "b"]));
        assert_eq!(goal.runnable_advancement().count(), 0);
        assert_eq!(pending_others(&goal, "a"), 2);
    }

    /// The delivery notice is what a host hands to a human: it must carry the
    /// validation receipt (bounded) when one exists, and omitting it must not
    /// drop the rest of the handoff. Both shapes are asserted on parsed JSON, so
    /// a field rename fails here rather than in a consumer.
    #[test]
    fn notice_includes_the_validation_receipt_and_bounds_it() {
        use crate::state::{TaskValidation, ValidationStatus};
        let mut goal = Goal::new("g", "work", ".");
        goal.add(Todo::advancement("t", "deliver"));
        let mut record = crate::state::RunRecord {
            agent_id: Some("a".into()),
            turn: 1,
            todo_id: "t".into(),
            run_id: "run-1".into(),
            terminal_state: "completed".into(),
            error: None,
            tokens_in_delta: 0,
            tokens_out_delta: 0,
            cost_delta: 0.0,
            tools: vec![],
            evidence: "artifact validated".into(),
            recorded_at: 0,
            spend_source: None,
            validation: None,
            failure_kind: None,
            truncation: None,
        };
        // No receipt → `validation` is present but null (stable JSON shape).
        let without: serde_json::Value =
            serde_json::from_str(&notice(&goal, &record, None, "s", &goal_dir())).unwrap();
        assert!(without["validation"].is_null(), "{without}");
        assert_eq!(without["delivery"], "awaiting_review");
        assert_eq!(without["pending_other_todos"], 0);
        assert_eq!(without["execution_state"], "completed");

        // With a receipt, the diagnostics tail is bounded to 400 bytes.
        record.validation = Some(TaskValidation {
            schema_version: "future_loop_task_validation_v0".into(),
            status: ValidationStatus::Failed,
            validator_kind: "verify".into(),
            summary: "E".repeat(5_000),
            recovery_kind: Some(crate::state::RecoveryKind::RepairRequired),
            exit_code: Some(1),
            ok: false,
        });
        let with: serde_json::Value =
            serde_json::from_str(&notice(&goal, &record, Some("a"), "s", &goal_dir())).unwrap();
        let v = &with["validation"];
        assert_eq!(v["ok"], false);
        assert_eq!(v["exit_code"], 1);
        assert!(
            v["diagnostics_tail"].as_str().unwrap().len() <= 400,
            "diagnostics must stay bounded: {v}"
        );
        assert_eq!(with["agent_id"], "a");
    }

    fn goal_dir() -> std::path::PathBuf {
        std::path::PathBuf::from("journal.jsonl")
    }
}
