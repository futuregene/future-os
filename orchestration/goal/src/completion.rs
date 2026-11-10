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

/// The gate judges a bounded TAIL of the completion text, not the whole reply:
/// the executor keeps `tail(text, EVIDENCE_TAIL_BYTES)` as the evidence that is
/// both validated and retained in the ledger. Shared so the executor, the
/// rejection diagnostic and the turn envelope cannot drift apart — the worker is
/// told the same window the gate actually applies.
pub const EVIDENCE_TAIL_BYTES: usize = 4_000;

/// An acceptance literal that the full completion text carries but the judged
/// tail does not, with the byte offset it was written at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutsideTail {
    pub literal: String,
    pub at: usize,
}

/// A rejection that says WHERE the unmet literal went, not only that it is
/// absent. `truncated_before_acceptance` is the structured form of the same
/// fact: the full text satisfied the contract and only the judged tail did not,
/// i.e. the worker wrote the literal and the window cut it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRejection {
    /// The refusal text that lands in `RunRecord::error`.
    pub message: String,
    /// True when the full completion text passed and only the judged tail failed.
    pub truncated_before_acceptance: bool,
    /// Literals found outside the judged window (empty when the flag is false).
    pub outside_tail: Vec<OutsideTail>,
}

/// Distinguish "the worker never wrote the literal" from "the worker wrote it,
/// outside the bytes this gate judges". Measured on a real goal: 5 of 6
/// acceptance rejections had the literal inside the completion text but before
/// the judged tail, and the bare `(missing: [...])` wording sent the worker back
/// to rewrite prose instead of moving its acceptance line.
///
/// The verdict is NOT changed here: a rejected tail stays rejected, and a full
/// text that also fails still reports `missing`. Only the words (and the
/// resubmit requirement) get precise.
pub fn truncated_acceptance_rejection(
    todo: &Todo,
    full_text: &str,
    judged: &str,
    tail_bytes: usize,
) -> Option<EvidenceRejection> {
    // The judged tail must be refused for a MISSING LITERAL (not empty
    // evidence), and the same contract must pass on the full text — otherwise
    // this is an ordinary rejection and the old wording is the right one.
    let judged_error = evidence_error(todo, judged)?;
    if !judged_error.contains("acceptance contract unmet") {
        return None;
    }
    if evidence_error(todo, full_text).is_some() {
        return None;
    }
    let acceptance = todo.acceptance.as_deref()?;
    let judged_lower = judged.to_lowercase();
    let mut outside_tail: Vec<OutsideTail> = Vec::new();
    for token in acceptance.split(',').map(str::trim) {
        if token.is_empty() || judged_lower.contains(&token.to_lowercase()) {
            continue;
        }
        if let Some(at) = find_ascii_case_insensitive(full_text, token) {
            outside_tail.push(OutsideTail {
                literal: token.to_string(),
                at,
            });
        }
    }
    if outside_tail.is_empty() {
        return None;
    }
    let of = full_text.len();
    let first = &outside_tail[0];
    let mut message = format!(
        "todo {} acceptance contract unmet: evidence truncated before acceptance — acceptance literal \"{}\" occurs at byte {} of {}, outside the {}-byte judged tail",
        todo.id, first.literal, first.at, of, tail_bytes
    );
    if outside_tail.len() > 1 {
        let rest: Vec<String> = outside_tail[1..]
            .iter()
            .map(|o| format!("\"{}\" at byte {}", o.literal, o.at))
            .collect();
        message.push_str(&format!(" (also outside: {})", rest.join(", ")));
    }
    message.push_str(&format!(
        "; resubmit with the acceptance line inside the final {tail_bytes} bytes"
    ));
    Some(EvidenceRejection {
        message,
        truncated_before_acceptance: true,
        outside_tail,
    })
}

/// Byte offset of `needle` in `haystack`, ASCII-case-insensitive like the gate's
/// own comparison, but reported on the ORIGINAL text so the offset the worker is
/// told is a real byte position in what it wrote.
fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len())
        .find(|&i| haystack[i..i + needle.len()].eq_ignore_ascii_case(needle))
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

    /// The measured shape of the real false red: 5648 bytes of completion, the
    /// acceptance literal at byte 1547, the judged tail starting at 1648. The
    /// refusal must name the truncation and the resubmit requirement instead of
    /// claiming the literal is missing — the worker wrote it.
    #[test]
    fn literal_outside_the_judged_tail_is_named_as_truncation_not_as_missing() {
        let mut todo = Todo::advancement("t", "deliver");
        todo.acceptance = Some("dispatches_per_step<500".into());
        let token = "dispatches_per_step<500";
        let mut full = "x".repeat(1547);
        full.push_str(token);
        full.push_str(&"y".repeat(5648 - full.len()));
        assert_eq!(full.len(), 5648);
        assert_eq!(full.find(token), Some(1547));

        let judged = tail(&full, EVIDENCE_TAIL_BYTES);
        // The verdict itself is unchanged: the judged tail is refused.
        assert!(evidence_error(&todo, &judged).is_some());

        let rejection = truncated_acceptance_rejection(&todo, &full, &judged, EVIDENCE_TAIL_BYTES)
            .expect("a truncation in the judged window is diagnosable");
        assert!(rejection.truncated_before_acceptance);
        assert_eq!(
            rejection.outside_tail,
            vec![OutsideTail {
                literal: token.to_string(),
                at: 1547
            }]
        );
        let message = &rejection.message;
        assert!(
            message.contains(&format!(
                "acceptance literal \"{token}\" occurs at byte 1547 of 5648, outside the 4000-byte judged tail"
            )),
            "{message}"
        );
        assert!(
            message.contains("resubmit with the acceptance line inside the final 4000 bytes"),
            "{message}"
        );
        assert!(
            !message.contains("(missing:"),
            "a literal that was written and cut is not missing: {message}"
        );
    }

    /// The other direction: a completion that never carried the literal is a
    /// genuine `missing`, and the diagnostic must not dress it up as truncation.
    #[test]
    fn literal_absent_from_the_whole_text_stays_missing() {
        let mut todo = Todo::advancement("t", "deliver");
        todo.acceptance = Some("dispatches_per_step<500,ms_per_step<43.75".into());
        let full = format!("{}the numbers look good", "x".repeat(5_000));
        let judged = tail(&full, EVIDENCE_TAIL_BYTES);
        assert_eq!(
            truncated_acceptance_rejection(&todo, &full, &judged, EVIDENCE_TAIL_BYTES),
            None
        );
        let message = evidence_error(&todo, &judged).unwrap();
        assert!(
            message.contains("(missing: [dispatches_per_step<500, ms_per_step<43.75])"),
            "{message}"
        );
        assert!(!message.contains("truncated"), "{message}");
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
            schema_version: "future_goal_task_validation_v0".into(),
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
