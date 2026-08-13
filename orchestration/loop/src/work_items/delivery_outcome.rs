//! P0-2: post-delivery outcome closure — LoopX
//! `control_plane/work_items/delivery_outcome.py` + `outcome_followthrough.py`,
//! natively (compact set).
//!
//! A completed advancement todo is a DELIVERY, not a verified outcome
//! ("delivered ≠ succeeded" — the gap the P0-2 report item closes). This
//! module is the signal chain that closes the loop:
//!
//! ① `delivery_outcome` — every delivery starts in `delivered` (pending
//!    verification) and must be resolved to one of the three terminal
//!    outcomes — `verified` / `failed` / `rework` — by an operator or
//!    validator writeback (`delivery record`). The durable signal is the
//!    `DeliveryOutcomeRecorded` event; the read model is
//!    [`crate::state::DeliveryState`].
//! ② `outcome_followthrough` — a delivery left unverified for N turns
//!    derives a follow-up todo automatically ([`overdue_deliveries`] + the
//!    run-path / `delivery followthrough` driver), so an unverified delivery
//!    can never silently age out of the frontier. Fires exactly once per
//!    delivery cycle (the `FollowthroughCreated` event stamps the delivery).

use crate::state::{DeliveryState, Goal};

/// The pending state: work was delivered, the outcome is not yet verified.
pub const OUTCOME_DELIVERED: &str = "delivered";
/// Terminal resolution: the delivery was verified as successful.
pub const OUTCOME_VERIFIED: &str = "verified";
/// Terminal resolution: the delivery failed verification.
pub const OUTCOME_FAILED: &str = "failed";
/// Terminal resolution: the delivery needs rework.
pub const OUTCOME_REWORK: &str = "rework";

/// All outcome values (CLI `--outcome` choices).
pub const DELIVERY_OUTCOME_CHOICES: [&str; 4] = [
    OUTCOME_DELIVERED,
    OUTCOME_VERIFIED,
    OUTCOME_FAILED,
    OUTCOME_REWORK,
];

/// Default follow-through threshold: a delivery unverified for this many
/// turns auto-derives a follow-up todo (P0-2② "交付后 N turn 内未验证").
pub const DEFAULT_FOLLOWTHROUGH_TURNS: u32 = 3;

/// Normalize an outcome token (case/whitespace-insensitive) to its canonical
/// value; `None` for unknown values.
pub fn normalize_outcome(value: &str) -> Option<&'static str> {
    match value.trim().to_lowercase().as_str() {
        OUTCOME_DELIVERED => Some(OUTCOME_DELIVERED),
        OUTCOME_VERIFIED => Some(OUTCOME_VERIFIED),
        OUTCOME_FAILED => Some(OUTCOME_FAILED),
        OUTCOME_REWORK => Some(OUTCOME_REWORK),
        _ => None,
    }
}

/// Whether the outcome is one of the three terminal resolutions
/// (verified / failed / rework) — i.e. not the pending `delivered` state.
pub fn is_resolution(outcome: &str) -> bool {
    matches!(outcome, OUTCOME_VERIFIED | OUTCOME_FAILED | OUTCOME_REWORK)
}

/// Validate an outcome transition against the current read-model state
/// (checked at the command layer BEFORE the event is appended — replay folds
/// latest-wins unconditionally):
///
/// - `delivered` starts a fresh delivery cycle — legal from nothing, or
///   after `failed` / `rework` (re-delivery after repair); illegal while
///   already pending, and illegal after `verified` (a verified delivery is
///   closed — supersede/reopen the todo for new work instead).
/// - a resolution (`verified` / `failed` / `rework`) requires a pending
///   `delivered` state.
pub fn validate_transition(current: Option<&DeliveryState>, next: &str) -> Result<(), String> {
    match (current.map(|d| d.outcome.as_str()), next) {
        (None, OUTCOME_DELIVERED) => Ok(()),
        (Some(OUTCOME_FAILED), OUTCOME_DELIVERED) | (Some(OUTCOME_REWORK), OUTCOME_DELIVERED) => {
            Ok(())
        }
        (Some(OUTCOME_DELIVERED), OUTCOME_DELIVERED) => Err(
            "already delivered and pending verification — resolve it with \
             verified/failed/rework first"
                .to_string(),
        ),
        (Some(OUTCOME_VERIFIED), OUTCOME_DELIVERED) => Err(
            "already verified — the delivery is closed; open a new todo for new work".to_string(),
        ),
        (Some(OUTCOME_DELIVERED), r) if is_resolution(r) => Ok(()),
        (cur, r) if is_resolution(r) => Err(match cur {
            None => "no pending delivery for this todo (nothing was delivered yet)".to_string(),
            Some(c) => format!("delivery is already resolved as `{c}`"),
        }),
        _ => Err(format!(
            "unknown outcome `{next}` (choices: {})",
            DELIVERY_OUTCOME_CHOICES.join(", ")
        )),
    }
}

/// One delivered-but-unverified work item that aged past the follow-through
/// threshold (P0-2②).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverdueDelivery {
    pub todo_id: String,
    /// Turn counter at delivery time.
    pub delivered_turn: u32,
    /// Turns elapsed since delivery (`current_turn - delivered_turn`).
    pub turns_overdue: u32,
    /// Source todo text (for the derived follow-up todo).
    pub todo_text: String,
}

/// Deliveries pending verification for at least `threshold` turns that have
/// no follow-through todo yet. `current_turn` is the goal's latest run-turn
/// counter; nothing is overdue before the first turn (current_turn == 0).
/// Deterministic order: oldest delivery first.
pub fn overdue_deliveries(goal: &Goal, current_turn: u32, threshold: u32) -> Vec<OverdueDelivery> {
    if current_turn == 0 {
        return vec![];
    }
    let mut out: Vec<OverdueDelivery> = goal
        .delivery_states
        .iter()
        .filter(|d| d.outcome == OUTCOME_DELIVERED && d.followthrough_todo_id.is_none())
        .filter_map(|d| {
            let elapsed = current_turn.saturating_sub(d.delivered_turn);
            if elapsed >= threshold {
                Some(OverdueDelivery {
                    todo_id: d.todo_id.clone(),
                    delivered_turn: d.delivered_turn,
                    turns_overdue: elapsed,
                    todo_text: goal
                        .todo(&d.todo_id)
                        .map(|t| t.text.clone())
                        .unwrap_or_default(),
                })
            } else {
                None
            }
        })
        .collect();
    out.sort_by_key(|d| d.delivered_turn);
    out
}

/// The follow-up todo text derived for an overdue delivery (P0-2②): verify
/// the delivery or write back a precise failure — mirroring the LoopX
/// obligation `advance_primary_outcome_or_write_blocker`.
pub fn followthrough_todo_text(source: &OverdueDelivery) -> String {
    format!(
        "Follow-through: verify the delivery of {} — `{}` (delivered at turn {}, unverified for {} turns; \
         resolve via `future loop delivery record --todo-id {} --outcome verified|failed|rework`)",
        source.todo_id, source.todo_text, source.delivered_turn, source.turns_overdue, source.todo_id
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Goal, Todo};

    fn goal_with_delivery(outcome: &str, delivered_turn: u32) -> Goal {
        let mut g = Goal::new("g", "obj", "/tmp");
        g.add(Todo::advancement("t1", "ship the fix"));
        g.apply_delivery_outcome("t1", outcome, None, delivered_turn, 1, 100);
        g
    }

    #[test]
    fn normalize_outcome_accepts_canonical_and_case_insensitive() {
        assert_eq!(normalize_outcome("delivered"), Some(OUTCOME_DELIVERED));
        assert_eq!(normalize_outcome(" Verified "), Some(OUTCOME_VERIFIED));
        assert_eq!(normalize_outcome("FAILED"), Some(OUTCOME_FAILED));
        assert_eq!(normalize_outcome("rework"), Some(OUTCOME_REWORK));
        assert_eq!(normalize_outcome("nope"), None);
        assert_eq!(normalize_outcome(""), None);
    }

    #[test]
    fn transition_fresh_delivery_then_resolution() {
        let g = Goal::new("g", "obj", "/tmp");
        // No state yet: delivered is legal, resolutions are not.
        assert!(validate_transition(g.delivery_state("t1"), OUTCOME_DELIVERED).is_ok());
        assert!(validate_transition(g.delivery_state("t1"), OUTCOME_VERIFIED).is_err());
        let g = goal_with_delivery(OUTCOME_DELIVERED, 1);
        assert!(validate_transition(g.delivery_state("t1"), OUTCOME_VERIFIED).is_ok());
        assert!(validate_transition(g.delivery_state("t1"), OUTCOME_FAILED).is_ok());
        assert!(validate_transition(g.delivery_state("t1"), OUTCOME_REWORK).is_ok());
        // Double delivery while pending is rejected.
        assert!(validate_transition(g.delivery_state("t1"), OUTCOME_DELIVERED).is_err());
    }

    #[test]
    fn transition_redelivery_after_failure_but_not_after_verified() {
        let g = goal_with_delivery(OUTCOME_FAILED, 1);
        assert!(validate_transition(g.delivery_state("t1"), OUTCOME_DELIVERED).is_ok());
        let g = goal_with_delivery(OUTCOME_REWORK, 1);
        assert!(validate_transition(g.delivery_state("t1"), OUTCOME_DELIVERED).is_ok());
        let g = goal_with_delivery(OUTCOME_VERIFIED, 1);
        assert!(validate_transition(g.delivery_state("t1"), OUTCOME_DELIVERED).is_err());
        assert!(validate_transition(g.delivery_state("t1"), OUTCOME_VERIFIED).is_err());
    }

    #[test]
    fn overdue_only_after_threshold_turns() {
        let g = goal_with_delivery(OUTCOME_DELIVERED, 2);
        // turn 3: 1 turn elapsed — not overdue with threshold 3.
        assert!(overdue_deliveries(&g, 3, 3).is_empty());
        // turn 5: exactly 3 turns elapsed — overdue.
        let od = overdue_deliveries(&g, 5, 3);
        assert_eq!(od.len(), 1);
        assert_eq!(od[0].todo_id, "t1");
        assert_eq!(od[0].delivered_turn, 2);
        assert_eq!(od[0].turns_overdue, 3);
        assert_eq!(od[0].todo_text, "ship the fix");
    }

    #[test]
    fn nothing_overdue_before_first_turn() {
        let g = goal_with_delivery(OUTCOME_DELIVERED, 0);
        assert!(overdue_deliveries(&g, 0, 3).is_empty());
    }

    #[test]
    fn resolved_or_followed_deliveries_are_not_overdue() {
        for outcome in [OUTCOME_VERIFIED, OUTCOME_FAILED, OUTCOME_REWORK] {
            let g = goal_with_delivery(outcome, 1);
            assert!(overdue_deliveries(&g, 99, 3).is_empty(), "{outcome}");
        }
        // A pending delivery that already fired its follow-through is skipped.
        let mut g = goal_with_delivery(OUTCOME_DELIVERED, 1);
        g.apply_followthrough("t1", "t-follow", 200);
        assert!(overdue_deliveries(&g, 99, 3).is_empty());
    }

    #[test]
    fn redelivery_resets_the_cycle() {
        let mut g = goal_with_delivery(OUTCOME_DELIVERED, 1);
        g.apply_followthrough("t1", "t-follow", 200);
        g.apply_delivery_outcome("t1", OUTCOME_FAILED, None, 0, 2, 300);
        // Re-delivery: turn stamp moves, follow-through stamp clears.
        g.apply_delivery_outcome("t1", OUTCOME_DELIVERED, None, 10, 3, 400);
        let d = g.delivery_state("t1").unwrap();
        assert_eq!(d.outcome, OUTCOME_DELIVERED);
        assert_eq!(d.delivered_turn, 10);
        assert_eq!(d.followthrough_todo_id, None);
        // Not overdue until the NEW delivery ages past the threshold.
        assert!(overdue_deliveries(&g, 12, 3).is_empty());
        assert_eq!(overdue_deliveries(&g, 13, 3).len(), 1);
    }

    #[test]
    fn oldest_delivery_fires_first() {
        let mut g = Goal::new("g", "obj", "/tmp");
        g.add(Todo::advancement("t1", "first"));
        g.add(Todo::advancement("t2", "second"));
        g.apply_delivery_outcome("t2", OUTCOME_DELIVERED, None, 1, 1, 100);
        g.apply_delivery_outcome("t1", OUTCOME_DELIVERED, None, 2, 1, 100);
        let od = overdue_deliveries(&g, 9, 3);
        assert_eq!(
            od.iter().map(|d| d.todo_id.as_str()).collect::<Vec<_>>(),
            vec!["t2", "t1"]
        );
    }

    #[test]
    fn followthrough_text_names_source_and_resolution_path() {
        let od = OverdueDelivery {
            todo_id: "t1".into(),
            delivered_turn: 2,
            turns_overdue: 4,
            todo_text: "ship the fix".into(),
        };
        let text = followthrough_todo_text(&od);
        assert!(text.contains("t1") && text.contains("ship the fix"));
        assert!(text.contains("delivery record"));
    }
}
