//! Decision replay (G-19) — reference `control_plane/testing/decision_replay.py`
//! (268 lines), natively: record a REAL kernel decision as a public-safe
//! reduced case, then replay the decision kernel against the reconstructed
//! state and diff the outcome. This is the LLM-side complement to the
//! deterministic contract tests: a behavior regression canary that fails
//! loudly when a kernel change alters a recorded decision.
//!
//! Privacy is the point: the reduced case strips credentials, raw logs,
//! trajectories and local paths before it can be persisted; `validate` fails
//! closed on any banned key or absolute path.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::contract::ShouldRunPacket;
use crate::state::{Goal, TaskClass, Todo, TodoStatus};

pub const PUBLIC_SAFE_DECISION_REPLAY_SCHEMA_VERSION: &str = "public_safe_decision_replay_v0";
pub const PUBLIC_SAFE_DECISION_CASE_SCHEMA_VERSION: &str = "public_safe_decision_case_v0";

/// Keys that must never survive reduction into a persisted case (LoopX
/// _BANNED_KEYS).
pub const BANNED_KEYS: &[&str] = &[
    "credential",
    "credentials",
    "raw_log",
    "raw_logs",
    "raw_state",
    "trajectory",
    "trajectories",
    "verifier_output",
];

/// Compact todo fields (reference _TODO_FIELDS; `decision` is our additive field
/// so closed gate decisions replay deterministically).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompactTodo {
    pub todo_id: String,
    pub status: String,
    pub task_class: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_bound: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_gate: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_when: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_ready: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
}

/// The reduced decision fields (reference decision block).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionFields {
    pub should_run: bool,
    pub effective_action: String,
    pub normal_delivery_allowed: bool,
    pub recovery_delivery_allowed: bool,
    pub self_repair_allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractionCase {
    pub schema_version: String,
    pub mode: String,
    pub user_channel: UserChannelCase,
    pub agent_channel: AgentChannelCase,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct UserChannelCase {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AgentChannelCase {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub must_attempt: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_allowed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet_noop_allowed: Option<bool>,
}

/// Expected kernel outputs recorded at capture time (reference expected block).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpectedCase {
    pub scheduler_action: String,
    pub scheduler_cadence_class: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduler_interval_minutes: Option<u64>,
    pub decision_scope_status: String,
}

/// One public-safe decision case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionCase {
    pub schema_version: String,
    pub case_id: String,
    pub agent_id: String,
    pub decision: DecisionFields,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_todo: Option<CompactTodo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_todos: Vec<CompactTodo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_todos: Vec<CompactTodo>,
    pub interaction_contract: InteractionCase,
    pub expected: ExpectedCase,
}

impl DecisionCase {
    pub fn case_id(&self) -> &str {
        &self.case_id
    }
}

/// A replay file: schema version + cases (LoopX
/// PUBLIC_SAFE_DECISION_REPLAY_SCHEMA_VERSION).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionReplay {
    pub schema_version: String,
    pub cases: Vec<DecisionCase>,
}

impl DecisionReplay {
    pub fn new() -> Self {
        Self {
            schema_version: PUBLIC_SAFE_DECISION_REPLAY_SCHEMA_VERSION.to_string(),
            cases: Vec::new(),
        }
    }

    pub fn add(&mut self, case: DecisionCase) {
        self.cases.push(case);
    }

    pub fn load(path: &std::path::Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let replay: DecisionReplay = serde_json::from_str(&text)?;
        if replay.schema_version != PUBLIC_SAFE_DECISION_REPLAY_SCHEMA_VERSION {
            bail!("decision replay schema_version mismatch");
        }
        if replay.cases.is_empty() {
            bail!("decision replay requires at least one case");
        }
        for case in &replay.cases {
            validate_public_safe_decision_case(case)?;
        }
        Ok(replay)
    }

    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

impl Default for DecisionReplay {
    fn default() -> Self {
        Self::new()
    }
}

fn todo_status_str(status: &TodoStatus) -> String {
    match status {
        TodoStatus::Open => "open".to_string(),
        TodoStatus::Done => "done".to_string(),
        TodoStatus::Superseded => "superseded".to_string(),
        TodoStatus::Deferred => "deferred".to_string(),
        TodoStatus::Blocked => "blocked".to_string(),
    }
}

fn task_class_str(class: &TaskClass) -> String {
    match class {
        TaskClass::Advancement => "advancement_task".to_string(),
        TaskClass::UserGate => "user_gate".to_string(),
        TaskClass::UserAction => "user_action".to_string(),
        TaskClass::Monitor => "monitor".to_string(),
        TaskClass::Blocker => "blocker".to_string(),
    }
}

fn compact_todo(todo: &Todo, now_epoch: u64) -> CompactTodo {
    CompactTodo {
        todo_id: todo.id.clone(),
        status: todo_status_str(&todo.status),
        task_class: task_class_str(&todo.class),
        action_kind: todo.action_kind.clone(),
        claimed_by: todo.claimed_by.clone(),
        goal_bound: if todo.goal_bound { Some(true) } else { None },
        global_gate: if todo.global_gate { Some(true) } else { None },
        resume_when: todo.resume_when_text.clone(),
        resume_ready: todo.resume_when.map(|r| {
            r.duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() <= now_epoch)
                .unwrap_or(false)
        }),
        decision: todo.decision.clone(),
    }
}

/// Reduce a live decision into a public-safe case (LoopX
/// `reduce_public_safe_decision`). Agent/user todos come from the goal's
/// todo graph (role-scoped), selected todo from the packet.
pub fn reduce_public_safe_decision(
    packet: &ShouldRunPacket,
    goal: &Goal,
    case_id: &str,
    agent_id: Option<&str>,
) -> DecisionCase {
    let now = crate::state::now_epoch();
    let interaction = &packet.interaction_contract;
    let selected = packet.selected_todo.as_ref().and_then(|v| {
        v.get("todo_id")
            .and_then(|id| id.as_str())
            .and_then(|id| goal.todo(id))
            .map(|t| compact_todo(t, now))
    });
    let agent_todos: Vec<CompactTodo> = goal
        .todos
        .iter()
        .filter(|t| t.role == crate::state::TodoRole::Agent)
        .map(|t| compact_todo(t, now))
        .collect();
    let user_todos: Vec<CompactTodo> = goal
        .todos
        .iter()
        .filter(|t| t.role == crate::state::TodoRole::User)
        .map(|t| compact_todo(t, now))
        .collect();
    DecisionCase {
        schema_version: PUBLIC_SAFE_DECISION_CASE_SCHEMA_VERSION.to_string(),
        case_id: case_id.to_string(),
        agent_id: agent_id.unwrap_or("replay-agent").to_string(),
        decision: DecisionFields {
            should_run: packet.should_run,
            effective_action: packet.effective_action.clone(),
            normal_delivery_allowed: packet.normal_delivery_allowed,
            recovery_delivery_allowed: packet.recovery_delivery_allowed,
            self_repair_allowed: packet.self_repair_allowed,
        },
        selected_todo: selected,
        agent_todos,
        user_todos,
        interaction_contract: InteractionCase {
            schema_version: interaction.schema_version.clone(),
            mode: interaction.mode.as_str().to_string(),
            user_channel: UserChannelCase {
                action_required: Some(interaction.user_channel.action_required),
                notify: Some(interaction.user_channel.notify.clone()),
                question: interaction.user_channel.question.clone(),
            },
            agent_channel: AgentChannelCase {
                must_attempt: Some(interaction.agent_channel.must_attempt),
                delivery_allowed: Some(interaction.agent_channel.delivery_allowed),
                quiet_noop_allowed: Some(interaction.agent_channel.quiet_noop_allowed),
            },
        },
        expected: ExpectedCase {
            scheduler_action: packet.scheduler_hint.action.clone(),
            scheduler_cadence_class: packet.scheduler_hint.cadence_class.clone(),
            scheduler_interval_minutes: packet.scheduler_hint.next_due_ms.map(|ms| ms / 60_000),
            decision_scope_status: "consistent".to_string(),
        },
    }
}

/// Walk a JSON value collecting banned-key paths and local paths (LoopX
/// `_walk` + validation). Returns human-readable violation strings.
pub fn walk_public_safe_violations(value: &serde_json::Value) -> Vec<String> {
    let mut violations = Vec::new();
    fn walk(value: &serde_json::Value, path: &str, violations: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, child) in map {
                    let child_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    if BANNED_KEYS.iter().any(|b| key.to_lowercase() == *b) {
                        violations.push(format!("banned key: {child_path}"));
                    }
                    walk(child, &child_path, violations);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(item, path, violations);
                }
            }
            serde_json::Value::String(s) if s.starts_with('/') || s.contains("file://") => {
                violations.push(format!("local path in {path}"));
            }
            _ => {}
        }
    }
    walk(value, "", &mut violations);
    violations
}

/// Validate a case: schema, banned keys, local paths (LoopX
/// `validate_public_safe_decision_case`).
pub fn validate_public_safe_decision_case(case: &DecisionCase) -> Result<()> {
    if case.schema_version != PUBLIC_SAFE_DECISION_CASE_SCHEMA_VERSION {
        bail!("decision replay case schema_version mismatch");
    }
    let value = serde_json::to_value(case)?;
    let violations = walk_public_safe_violations(&value);
    if !violations.is_empty() {
        bail!(
            "decision replay contains public-safety violations: {}",
            violations.join("; ")
        );
    }
    Ok(())
}

fn status_from_str(s: &str) -> TodoStatus {
    match s {
        "done" => TodoStatus::Done,
        "superseded" => TodoStatus::Superseded,
        "deferred" => TodoStatus::Deferred,
        "blocked" => TodoStatus::Blocked,
        _ => TodoStatus::Open,
    }
}

fn class_from_str(s: &str) -> TaskClass {
    match s {
        "user_gate" => TaskClass::UserGate,
        "user_action" => TaskClass::UserAction,
        "monitor" => TaskClass::Monitor,
        "blocker" => TaskClass::Blocker,
        _ => TaskClass::Advancement,
    }
}

/// Reconstruct a goal from a case's compact todos (LoopX
/// `_source_todo_item` + `quota_status_payload`). The replayed kernel runs
/// against this reconstructed state — the recording must capture everything
/// the decision depends on.
pub fn goal_from_case(case: &DecisionCase) -> Goal {
    let mut goal = Goal::new(&case.case_id, "decision replay", "/tmp");
    for (index, item) in case
        .agent_todos
        .iter()
        .chain(case.user_todos.iter())
        .enumerate()
    {
        let mut todo = Todo::advancement(&item.todo_id, &format!("Replay item {}.", item.todo_id));
        todo.class = class_from_str(&item.task_class);
        todo.status = status_from_str(&item.status);
        todo.index = index as u32;
        todo.action_kind = item.action_kind.clone();
        todo.claimed_by = item.claimed_by.clone();
        todo.goal_bound = item.goal_bound.unwrap_or(false);
        todo.global_gate = item.global_gate.unwrap_or(false);
        todo.resume_when_text = item.resume_when.clone();
        todo.decision = item.decision.clone();
        if let Some(role) = case
            .agent_todos
            .iter()
            .any(|t| t.todo_id == item.todo_id)
            .then_some(crate::state::TodoRole::Agent)
            .or_else(|| {
                case.user_todos
                    .iter()
                    .any(|t| t.todo_id == item.todo_id)
                    .then_some(crate::state::TodoRole::User)
            })
        {
            todo.role = role;
        }
        goal.add(todo);
    }
    goal
}

/// Per-field replay comparison (reference diff semantics, natively).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayComparison {
    pub case_id: String,
    pub matched: bool,
    /// (field, matched) for the decision block.
    pub decision: Vec<(String, bool)>,
    /// (field, matched) for the expected block.
    pub expected: Vec<(String, bool)>,
    /// (field, matched) for the interaction contract block.
    pub interaction: Vec<(String, bool)>,
    pub mismatches: Vec<String>,
}

/// Replay a recorded case: reconstruct the goal, run the decision kernel, and
/// diff the replayed decision against the recorded case (LoopX
/// `replay_public_safe_decision_case`).
pub fn replay_public_safe_decision_case(case: &DecisionCase) -> Result<ReplayComparison> {
    validate_public_safe_decision_case(case)?;
    let mut goal = goal_from_case(case);
    // Register the replay agent (LoopX: coordination.registered_agents =
    // [agent_id]) so the identity gate does not misfire on replay.
    if !goal.registered_agents.iter().any(|a| a == &case.agent_id) {
        goal.register_agent(&case.agent_id, vec![]);
    }
    let now = std::time::SystemTime::now();
    let packet = crate::decision::decide_for(&goal, now, Some(&case.agent_id));
    let replayed = reduce_public_safe_decision(&packet, &goal, &case.case_id, Some(&case.agent_id));

    let mut decision = Vec::new();
    let mut expected = Vec::new();
    let mut interaction = Vec::new();
    let mut mismatches = Vec::new();

    let push_bool = |field: &str,
                     a: bool,
                     b: bool,
                     decision: &mut Vec<(String, bool)>,
                     mismatches: &mut Vec<String>| {
        let matched = a == b;
        decision.push((field.to_string(), matched));
        if !matched {
            mismatches.push(format!("decision.{field}: recorded={b:?} replayed={a:?}"));
        }
    };
    push_bool(
        "should_run",
        replayed.decision.should_run,
        case.decision.should_run,
        &mut decision,
        &mut mismatches,
    );
    push_bool(
        "normal_delivery_allowed",
        replayed.decision.normal_delivery_allowed,
        case.decision.normal_delivery_allowed,
        &mut decision,
        &mut mismatches,
    );
    push_bool(
        "recovery_delivery_allowed",
        replayed.decision.recovery_delivery_allowed,
        case.decision.recovery_delivery_allowed,
        &mut decision,
        &mut mismatches,
    );
    push_bool(
        "self_repair_allowed",
        replayed.decision.self_repair_allowed,
        case.decision.self_repair_allowed,
        &mut decision,
        &mut mismatches,
    );
    {
        let (field, a, b) = (
            "effective_action",
            replayed.decision.effective_action.as_str(),
            case.decision.effective_action.as_str(),
        );
        let matched = a == b;
        decision.push((field.to_string(), matched));
        if !matched {
            mismatches.push(format!("decision.{field}: recorded={b:?} replayed={a:?}"));
        }
    }
    for (field, a, b) in [
        (
            "scheduler_action",
            replayed.expected.scheduler_action.as_str(),
            case.expected.scheduler_action.as_str(),
        ),
        (
            "scheduler_cadence_class",
            replayed.expected.scheduler_cadence_class.as_str(),
            case.expected.scheduler_cadence_class.as_str(),
        ),
    ] {
        let matched = a == b;
        expected.push((field.to_string(), matched));
        if !matched {
            mismatches.push(format!("expected.{field}: recorded={b:?} replayed={a:?}"));
        }
    }
    {
        let (field, a, b) = (
            "scheduler_interval_minutes",
            replayed.expected.scheduler_interval_minutes,
            case.expected.scheduler_interval_minutes,
        );
        let matched = a == b;
        expected.push((field.to_string(), matched));
        if !matched {
            mismatches.push(format!("expected.{field}: recorded={b:?} replayed={a:?}"));
        }
    }
    {
        let (field, a, b) = (
            "mode",
            replayed.interaction_contract.mode.as_str(),
            case.interaction_contract.mode.as_str(),
        );
        let matched = a == b;
        interaction.push((field.to_string(), matched));
        if !matched {
            mismatches.push(format!(
                "interaction.{field}: recorded={b:?} replayed={a:?}"
            ));
        }
    }
    {
        let (field, a, b) = (
            "user_channel.notify",
            replayed.interaction_contract.user_channel.notify.as_deref(),
            case.interaction_contract.user_channel.notify.as_deref(),
        );
        let matched = a == b;
        interaction.push((field.to_string(), matched));
        if !matched {
            mismatches.push(format!(
                "interaction.{field}: recorded={b:?} replayed={a:?}"
            ));
        }
    }
    let mut push_interaction_bool =
        |field: &str, a: bool, b: bool, mismatches: &mut Vec<String>| {
            let matched = a == b;
            interaction.push((field.to_string(), matched));
            if !matched {
                mismatches.push(format!(
                    "interaction.{field}: recorded={b:?} replayed={a:?}"
                ));
            }
        };
    push_interaction_bool(
        "user_channel.action_required",
        replayed
            .interaction_contract
            .user_channel
            .action_required
            .unwrap_or(false),
        case.interaction_contract
            .user_channel
            .action_required
            .unwrap_or(false),
        &mut mismatches,
    );
    push_interaction_bool(
        "agent_channel.must_attempt",
        replayed
            .interaction_contract
            .agent_channel
            .must_attempt
            .unwrap_or(false),
        case.interaction_contract
            .agent_channel
            .must_attempt
            .unwrap_or(false),
        &mut mismatches,
    );
    push_interaction_bool(
        "agent_channel.delivery_allowed",
        replayed
            .interaction_contract
            .agent_channel
            .delivery_allowed
            .unwrap_or(false),
        case.interaction_contract
            .agent_channel
            .delivery_allowed
            .unwrap_or(false),
        &mut mismatches,
    );

    Ok(ReplayComparison {
        case_id: case.case_id.clone(),
        matched: mismatches.is_empty(),
        decision,
        expected,
        interaction,
        mismatches,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Todo;

    fn sample_goal() -> Goal {
        let mut g = Goal::new("g1", "Ship the thing", "/tmp");
        g.add(Todo::advancement("T1", "Implement the feature").with_action_kind("shell"));
        g.add(Todo::user_gate("G1", "Approve the release", &["T2"]));
        g.add(Todo::advancement("T2", "Release after approval").blocking(&["G1"]));
        g
    }

    #[test]
    fn reduce_then_replay_matches_exactly() {
        let goal = sample_goal();
        let packet = crate::decision::decide(&goal, std::time::SystemTime::now());
        let case = reduce_public_safe_decision(&packet, &goal, "case-1", Some("agent-a"));
        assert_eq!(
            case.schema_version,
            PUBLIC_SAFE_DECISION_CASE_SCHEMA_VERSION
        );
        assert_eq!(case.agent_id, "agent-a");
        assert!(!case.agent_todos.is_empty());
        assert!(!case.user_todos.is_empty());
        validate_public_safe_decision_case(&case).unwrap();
        // replay must reproduce the recorded decision exactly
        let comparison = replay_public_safe_decision_case(&case).unwrap();
        assert!(
            comparison.matched,
            "replay mismatches: {:?}",
            comparison.mismatches
        );
        assert!(comparison.decision.iter().all(|(_, m)| *m));
        assert!(comparison.expected.iter().all(|(_, m)| *m));
        assert!(comparison.interaction.iter().all(|(_, m)| *m));
    }

    #[test]
    fn replay_detects_kernel_change() {
        let goal = sample_goal();
        let packet = crate::decision::decide(&goal, std::time::SystemTime::now());
        let mut case = reduce_public_safe_decision(&packet, &goal, "case-1", None);
        // Corrupt the recorded expectation: the replay must flag it.
        case.expected.scheduler_action = "different_action".to_string();
        let comparison = replay_public_safe_decision_case(&case).unwrap();
        assert!(!comparison.matched);
        assert!(comparison
            .mismatches
            .iter()
            .any(|m| m.contains("expected.scheduler_action")));
        // Corrupt a decision field too.
        let mut case2 = reduce_public_safe_decision(&packet, &goal, "case-1", None);
        case2.decision.should_run = !case2.decision.should_run;
        let comparison = replay_public_safe_decision_case(&case2).unwrap();
        assert!(!comparison.matched);
        assert!(comparison
            .mismatches
            .iter()
            .any(|m| m.contains("decision.should_run")));
    }

    #[test]
    fn banned_keys_fail_validation() {
        let goal = sample_goal();
        let packet = crate::decision::decide(&goal, std::time::SystemTime::now());
        let case = reduce_public_safe_decision(&packet, &goal, "case-1", None);
        // inject a credential-shaped KEY into the serialized case
        let mut value = serde_json::to_value(&case).unwrap();
        value["credential"] = serde_json::json!("secret");
        let violations = walk_public_safe_violations(&value);
        assert!(
            violations
                .iter()
                .any(|v| v.contains("banned key") && v.contains("credential")),
            "violations: {violations:?}"
        );
        let mut value2 = serde_json::to_value(&case).unwrap();
        value2["agent_todos"][0]["raw_log"] = serde_json::json!("...");
        assert!(walk_public_safe_violations(&value2)
            .iter()
            .any(|v| v.contains("raw_log")));
        // the clean case passes
        assert!(validate_public_safe_decision_case(&case).is_ok());
    }

    #[test]
    fn local_paths_fail_validation() {
        let goal = sample_goal();
        let packet = crate::decision::decide(&goal, std::time::SystemTime::now());
        let mut case = reduce_public_safe_decision(&packet, &goal, "case-1", None);
        case.expected.scheduler_action = "/etc/passwd".to_string();
        let err = validate_public_safe_decision_case(&case).unwrap_err();
        assert!(err.to_string().contains("local path"), "{err}");
        // file:// scheme also flagged
        let mut case2 = reduce_public_safe_decision(&packet, &goal, "case-1", None);
        case2.expected.scheduler_action = "file:///tmp/x".to_string();
        assert!(validate_public_safe_decision_case(&case2).is_err());
    }

    #[test]
    fn replay_file_round_trip() {
        let dir =
            std::env::temp_dir().join(format!("future-loop-replay-{}", crate::state::now_epoch()));
        std::fs::create_dir_all(&dir).unwrap();
        let goal = sample_goal();
        let packet = crate::decision::decide(&goal, std::time::SystemTime::now());
        let mut replay = DecisionReplay::new();
        replay.add(reduce_public_safe_decision(&packet, &goal, "case-1", None));
        let path = dir.join("replay.json");
        replay.save(&path).unwrap();
        let loaded = DecisionReplay::load(&path).unwrap();
        assert_eq!(loaded.cases.len(), 1);
        assert_eq!(loaded.cases[0].case_id, "case-1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replay_reconstructs_goal_and_keeps_gate_decisions() {
        let mut goal = sample_goal();
        goal.todo_mut("G1").unwrap().decision = Some("approved".to_string());
        goal.todo_mut("G1").unwrap().status = TodoStatus::Done;
        let packet = crate::decision::decide(&goal, std::time::SystemTime::now());
        let case = reduce_public_safe_decision(&packet, &goal, "case-g", None);
        let reconstructed = goal_from_case(&case);
        assert_eq!(
            reconstructed.todo("G1").unwrap().decision.as_deref(),
            Some("approved")
        );
        assert_eq!(reconstructed.todo("G1").unwrap().status, TodoStatus::Done);
        // replay still matches because the recorded case captured the closure
        let comparison = replay_public_safe_decision_case(&case).unwrap();
        assert!(comparison.matched, "{:?}", comparison.mismatches);
    }
}
