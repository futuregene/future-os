//! Attention queue (G-15) — LoopX
//! `control_plane/work_items/attention_queue.py` + `attention_item.py`,
//! natively (compact set). One attention item per goal (status, waiting_on,
//! severity, recommended action) projected into a queue with routing counts —
//! the operator/controller triage surface.

use crate::state::Goal;

/// Who an attention item waits on (LoopX waiting_on vocabulary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttentionWaitingOn {
    Codex,
    UserOrController,
    Controller,
    ExternalEvidence,
    Monitor,
}

impl AttentionWaitingOn {
    pub fn label(&self) -> &'static str {
        match self {
            AttentionWaitingOn::Codex => "codex",
            AttentionWaitingOn::UserOrController => "user_or_controller",
            AttentionWaitingOn::Controller => "controller",
            AttentionWaitingOn::ExternalEvidence => "external_evidence",
            AttentionWaitingOn::Monitor => "monitor_signal",
        }
    }
}

/// One attention item (LoopX attention_item).
#[derive(Debug, Clone, serde::Serialize)]
pub struct AttentionItem {
    pub goal_id: String,
    pub status: String,
    pub waiting_on: String,
    pub severity: String,
    pub recommended_action: String,
    pub source: String,
}

/// The projected attention queue (LoopX build_attention_queue_projection).
#[derive(Debug, Clone, serde::Serialize)]
pub struct AttentionQueue {
    pub available: bool,
    pub item_count: usize,
    pub needs_user_or_controller: usize,
    pub needs_controller: usize,
    pub needs_codex: usize,
    pub watching_monitor: usize,
    pub items: Vec<AttentionItem>,
}

/// Project a queue from items (counts by waiting_on).
pub fn build_attention_queue(items: Vec<AttentionItem>) -> AttentionQueue {
    let mut needs_user_or_controller = 0usize;
    let mut needs_controller = 0usize;
    let mut needs_codex = 0usize;
    let mut watching_monitor = 0usize;
    for item in &items {
        match item.waiting_on.as_str() {
            "user_or_controller" => needs_user_or_controller += 1,
            "controller" => needs_controller += 1,
            "codex" => needs_codex += 1,
            "monitor_signal" => watching_monitor += 1,
            _ => {}
        }
    }
    AttentionQueue {
        available: true,
        item_count: items.len(),
        needs_user_or_controller,
        needs_controller,
        needs_codex,
        watching_monitor,
        items,
    }
}

/// Project one goal's attention item (None when the goal needs nothing).
/// Routing (LoopX goal_attention):
/// - open user gate → waiting on user_or_controller;
/// - replan obligation / projection gap → waiting on codex;
/// - open monitor due → watching monitor;
/// - terminal closure → no item.
pub fn goal_attention_item(goal: &Goal) -> Option<AttentionItem> {
    if goal.terminal_closure().is_some() {
        return None;
    }
    if goal.open_gates().count() > 0 {
        let question = goal
            .open_gates()
            .next()
            .and_then(|g| g.gate_question.clone())
            .unwrap_or_else(|| "resolve the open user gate".to_string());
        return Some(AttentionItem {
            goal_id: goal.goal_id.clone(),
            status: "operator_gate".to_string(),
            waiting_on: AttentionWaitingOn::UserOrController.label().to_string(),
            severity: "high".to_string(),
            recommended_action: format!("decide: {question}"),
            source: "goal_attention".to_string(),
        });
    }
    if let Some(gap) = crate::store::projection_gap(goal) {
        return Some(AttentionItem {
            goal_id: goal.goal_id.clone(),
            status: "projection_gap".to_string(),
            waiting_on: AttentionWaitingOn::Codex.label().to_string(),
            severity: "action".to_string(),
            recommended_action: format!("self-repair projection gap: {gap}"),
            source: "goal_attention".to_string(),
        });
    }
    let open_advancement = goal.open_of(crate::state::TaskClass::Advancement).count();
    if open_advancement > 0 {
        return Some(AttentionItem {
            goal_id: goal.goal_id.clone(),
            status: "advancement_open".to_string(),
            waiting_on: AttentionWaitingOn::Codex.label().to_string(),
            severity: "action".to_string(),
            recommended_action: format!("advance {open_advancement} open advancement todo(s)"),
            source: "goal_attention".to_string(),
        });
    }
    let monitor_due = goal
        .open_of(crate::state::TaskClass::Monitor)
        .filter(|m| {
            m.resume_when
                .map(|t| t <= std::time::SystemTime::now())
                .unwrap_or(false)
        })
        .count();
    if monitor_due > 0 {
        return Some(AttentionItem {
            goal_id: goal.goal_id.clone(),
            status: "monitor_due".to_string(),
            waiting_on: AttentionWaitingOn::Monitor.label().to_string(),
            severity: "info".to_string(),
            recommended_action: format!("poll {monitor_due} due monitor(s)"),
            source: "goal_attention".to_string(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Todo;

    fn goal_with(todos: Vec<Todo>) -> Goal {
        let mut goal = Goal::new("g1", "objective", "/tmp");
        goal.todos = todos;
        goal
    }

    #[test]
    fn deferred_not_due_goal_has_no_attention_item() {
        // A deferred-not-due todo is neither terminal nor actionable: no
        // gate, no gap, no open advancement, no due monitor → None.
        let mut t = Todo::advancement("t1", "later");
        t.status = crate::state::TodoStatus::Deferred;
        t.resume_when = Some(std::time::SystemTime::now() + std::time::Duration::from_secs(3600));
        let goal = goal_with(vec![t]);
        assert!(goal_attention_item(&goal).is_none());
    }

    #[test]
    fn gate_waits_on_user() {
        let goal = goal_with(vec![Todo::user_gate("g1", "approve?", &[])]);
        let item = goal_attention_item(&goal).unwrap();
        assert_eq!(item.waiting_on, "user_or_controller");
        assert_eq!(item.severity, "high");
        assert!(item.recommended_action.contains("approve?"));
    }

    #[test]
    fn open_advancement_waits_on_codex() {
        let goal = goal_with(vec![Todo::advancement("t1", "work")]);
        let item = goal_attention_item(&goal).unwrap();
        assert_eq!(item.waiting_on, "codex");
        assert_eq!(item.severity, "action");
    }

    #[test]
    fn terminal_goal_has_no_item() {
        let mut goal = goal_with(vec![]);
        // terminal closure: all gaps satisfied + a declared next action.
        goal.next_action = Some("complete; no further action".to_string());
        assert!(goal_attention_item(&goal).is_none());
    }

    #[test]
    fn queue_counts_by_waiting_on() {
        let items = vec![
            AttentionItem {
                goal_id: "g1".into(),
                status: "gate".into(),
                waiting_on: "user_or_controller".into(),
                severity: "high".into(),
                recommended_action: "decide".into(),
                source: "test".into(),
            },
            AttentionItem {
                goal_id: "g2".into(),
                status: "advancement".into(),
                waiting_on: "codex".into(),
                severity: "action".into(),
                recommended_action: "advance".into(),
                source: "test".into(),
            },
            AttentionItem {
                goal_id: "g3".into(),
                status: "monitor".into(),
                waiting_on: "monitor_signal".into(),
                severity: "info".into(),
                recommended_action: "poll".into(),
                source: "test".into(),
            },
        ];
        let queue = build_attention_queue(items);
        assert_eq!(queue.item_count, 3);
        assert_eq!(queue.needs_user_or_controller, 1);
        assert_eq!(queue.needs_codex, 1);
        assert_eq!(queue.watching_monitor, 1);
    }
}
