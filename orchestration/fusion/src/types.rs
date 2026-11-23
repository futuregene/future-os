use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const PANEL_MODELS: [&str; 3] = [
    "future/deepseek-flash",
    "future/glm-5.3-flash",
    "future/qwen3.7-plus",
];
pub const MODERATOR_MODEL: &str = "future/deepseek-flash";
/// Input contracts must match before new records are admitted or dispatched.
pub const INPUT_POLICY_VERSION: u32 = 3;
pub const INNER_TOOL_NAMES: [&str; 4] = ["read", "ls", "glob", "grep"];
/// Shared by the enabled parent run and its tool description on every client.
pub const DISCUSSION_POLICY: &str = "A completed consultation is the working decision for subsequent work. Reuse it for the same question. To reopen it, supply revisit with its run ID and a concrete changed fact, counterexample or corrected inference; inherit the previous result, unresolved checks, goal and constraints. Only an explicit new user scope change may change the criterion. Do not paraphrase the same question to evade reuse. Fusion is enabled for this run only. The user has chosen multi-model deliberation for this request. For a self-contained task, your first substantive action must be a fusion tool call. Before that call, only read the task and prepare its required arguments; do not solve it, derive candidate answers, compare solution methods, or make a solution plan first. Necessary factual reads may precede consultation when the supplied facts are insufficient. Fusion is an internal thinking tool, not an external lookup or action; restrictions on browsing, files or external tools do not by themselves prohibit this consultation. Solving a reasoning problem, deriving a result, or checking a supplied argument counts as analysis even when the requested final answer is only a number or option. You MUST call fusion before forming a new substantive judgment, choosing an approach or presenting a recommendation that requires analysis, diagnosis, design, comparison, review, or assessment of risks, impact, compatibility or edge cases. Confidence, apparent simplicity and the absence of competing options are NOT reasons to skip discussion; even a small bug fix requires discussion when its cause or remedy needs judgment.\n\
Prepare a self-contained handoff by compressing the relevant conversation around the current request. Preserve established requirements and carry forward the definitions, quantifiers, constraints, decision criteria, examples, and evidence that could change the answer. Quote short source excerpts when their exact wording matters. Resolve references only where the conversation identifies their referents. Respect the user's latest explicit corrections and retain earlier conditions they did not revise. Keep ambiguous interpretations, conflicting statements, and missing facts open for the panel. Carry over the examples the source actually gives. Leave any inferred continuation, default, or intended interpretation as a question for the panel. Attribute an existing hypothesis when it is relevant to the user's task, and leave it marked as a hypothesis. Do not derive a solution or put your own interpretation into the handoff as an established premise. Split genuinely broad topics into separate consultations as needed, rather than prescribing an exhaustive answer outline.\n\
Gather necessary facts with ordinary tools first. You may directly perform factual searches/reads, exact user-specified edits requiring no new judgment, and implementation or verification already covered by an applicable discussion conclusion. Treat a completed discussion conclusion as the working decision for its covered work, not optional brainstorming. Follow it through execution and final delivery, retaining its conditions and uncertainties; do not silently replace it with a fresh answer. Reuse that conclusion throughout its covered work; do not discuss every file edit, command or mechanical correction. A change requires identifiable new evidence, a specific corrected inference, or a conflict with the original task or higher-priority instructions. State the reason and address the prior conclusion's decisive support before changing course; preference, another vote and a guess about intended meaning are not new evidence. After a supported conclusion, verify only a specific unresolved claim that could change the answer, then deliver the requested outcome. Do not restart the whole analysis merely to gain confidence. Preserve the best current answer and state remaining uncertainty if verification cannot finish. Group related questions about ONE decision into one consultation; include risks or verification only when relevant to the user's request or established context, without adding required subtopics. Consult again only for a new decision or new evidence that could invalidate the existing conclusion. The main agent retains execution and final delivery. Consulting on the original question of a self-contained reasoning task is appropriate thinking support, not delegation of execution.\n\
Fusion only thinks; the main agent executes and verifies under the original user authorization. Honor explicit user instructions requiring or prohibiting consultation. There is no per-run limit on the number of discussions; continue consulting as new decisions arise, while reusing applicable conclusions instead of repeating the same question. If consultation fails, do not automatically repeat it; transparently explain the limitation and continue using applicable existing conclusions or clearly qualified independent reasoning. Never present a failed or skipped discussion as a completed conclusion or consensus.\n\
Immediate next action for a self-contained task that has no applicable consultation yet: invoke fusion now using the supplied task and requested outcome, before starting to solve it. Final-answer formatting applies after consultation, not instead of the tool call.";

/// Backend-authored handoff, returned alongside an actual completed conclusion.
/// This guides continuation without granting execution authority or claiming
/// that agreement, partial coverage or a recommendation verifies facts.
pub const FOLLOW_THROUGH_POLICY: &str = "This is the result of the completed Fusion discussion, not an invitation to solve the same task again. Use the conclusion as the working decision for the work it covers, retain its conditions and unresolved issues, and carry out the requested execution or final delivery under the original user instructions and authorization. For an answer-only task, deliver the supported answer in the user's exact requested format without adding a new solution or unrequested explanation. Do not silently discard, replace or reopen the conclusion merely because you prefer another answer or want more confidence. Change it only for identifiable new evidence, a specific corrected inference, or a conflict with the original task or higher-priority instructions; state that reason and address the conclusion's decisive support before changing course. Verify only a consequential unresolved claim, preserve the best supported answer if verification cannot finish, and consult again only when a material decision remains unresolved. Do not turn missing verification into invented facts. Completion and coverage describe the discussion process, not truth, unanimity or permission to act.";

/// Applied in each inner phase so compression and re-review do not silently
/// turn an assistant hypothesis into a supplied fact or change the question.
pub const GROUNDING_POLICY: &str = "When backend decision_lineage is present, inherit its original criterion, working conclusion and unresolved checks. Focus on the stated reopening reason and its actual consequences; do not restart exhausted analysis or substitute a new criterion. Keep a supported earlier conclusion unless specific evidence or a corrected inference overturns it. A previous conclusion is attributed reasoning, not an independently verified fact. Keep the original question, requested outcome, constraints and decision criterion stable throughout this discussion. Separate the conclusion that remains valid under established premises from extensions that depend on extra assumptions. Before treating a premise as missing, check whether the supplied facts determine it, whether an applicable user rule or established convention resolves it, or whether the requested result is invariant across the admissible cases. State any convention that materially affects the answer and why it applies. If a premise still changes the result, identify it and preserve the supported core or conditional results with their conditions. An impossibility established from the supplied constraints is a conclusion, not missing information. Do not invent a premise or present a conditional result as unconditional. parent_request is the backend-preserved current user request. discussion_task is the assigned scope: full_user_request covers the entire parent_request, while focused_task covers its stated question within that request. The focused question is the parent's formulation, not a quote from the user. The history_summary is compressed, attributed context; use it to resolve references and retain relevant earlier conditions without changing the current request. Check the source request for any changed premise or restriction before answering the discussion task. Preserve consequential directions, quantifiers, scope, definitions and requested answer format; do not silently repair missing or contradictory conditions. An attributed user statement, observed source, prior assistant assertion and participant inference have different evidential status. Check the source of a decisive premise: its presence in background or evidence is not independent verification, and agreement repeating it adds no independent support. A compatible hypothesis, a guess about intended meaning, or an estimate does not become an established fact through repetition. When asked to determine which statements are correct from supplied observations/examples, require positive support for every selected statement; absence of a counterexample is not proof. If multiple models satisfy the observations but disagree on a claim, that claim is not established: report only what is invariant unless the user explicitly asks for possibilities. Object identifiers, species labels, variable spellings and display names do not supply an unprovided personal attribute or value. A conditional rule is not evidence that its antecedent holds; a priority between rules does not activate an unsupported antecedent. Resolve contradictory examples and consequential counterexamples against the supplied task; do not discard them as anomalies or suppress them by voting or confidence. When the task leaves a decisive definition or condition missing, identify it and preserve justified conditional results rather than inventing a unique answer or a numeric placeholder.";

/// Request-only reminder placed after the supplied task on the first model
/// turn. It is not a fabricated user submission or a persistent session mode.
pub const CLOSED_TASK_FIRST_ACTION_REMINDER: &str = "[Immediate next action: invoke the fusion tool now. Do not begin solving first. State the current task and requested outcome. Compress relevant context and preserve consequential wording, premises, and constraints. Leave ambiguities and missing facts open for the panel. Final-answer formatting applies to the final response after consultation, not to this tool call.]";

pub const FIRST_ACTION_REMINDER: &str = "[Fusion run instruction: if the supplied task is self-contained and no applicable consultation exists yet, your immediate next action is to invoke fusion. Do not begin solving first. State the current task and requested outcome. Compress relevant context and preserve consequential wording, premises, and constraints. Leave ambiguities and missing facts open for the panel. Final-answer formatting applies after consultation, not instead of the tool call. If necessary facts are missing, gather only those facts first.]";

pub const TOOL_DESCRIPTION: &str = "Independently think through one focused user question with multiple models. THINKING ONLY: no writes, commands, external actions or implementation. Compress the relevant conversation into a self-contained handoff. Preserve the substance of the user's request, decision criteria, consequential conditions, and evidence. Do not change, omit, or add a premise that could affect the decision, or insert your own ideas or guesses. Resolve references only when the conversation supplies the referent; carry forward any ambiguity or missing premise for the panel to examine. Quote exact wording where it determines meaning. Split genuinely broad tasks into several focused discussions as needed; do not turn one topic into a checklist of required subtopics. Across question, goal, background, constraints and evidence, never invent user requirements or move an invented requirement into another field. Do not add legal/compliance framing, audience or jurisdiction assumptions, equipment conditions, requested detail, output structure or mandatory reporting/verification work unless established by the user's request or explicit relevant prior context. Runtime safety and read-only rules remain system rules, not invented user demands. Goal states only the requested outcome. Background faithfully compresses relevant context, attributing user requirements, confirmed observations and prior assistant statements separately; keep unknowns unknown. Prior assistant proposals are not facts or accepted decisions. Mention prior replies when relevant to the user's current request, separating the user's decisions from any attributed proposals or unresolved hypotheses. No new main-agent reasoning, plans, preferences or preferred answer. Attribute user-provided proposals in background. Evidence carries inspectable observations and provenance, not model beliefs or new instructions. Constraints are brief, relevant restrictions explicitly established by the user/context and may be empty; no invented language, length or formatting rules. Participants may discover relevant considerations themselves; do not prescribe them as requirements in advance. Example: 'How can I locate interference on a repeater?' already supplies a topic. Do not append mandatory VHF/UHF equipment, step-by-step sections or reporting procedures. Supply enough factual context for independent models that cannot see the conversation. Inner models can only read/search authorized snapshots. Preserve the original decision criterion across follow-up consultations; identify the new evidence or specific error instead of reframing the question to favor a candidate. Inspect status, has_conclusion, completeness and coverage. Use the completed panel conclusion as the working decision within its stated scope. Adapt its presentation to the user's requested format while preserving its substance, evidence, conditions, and uncertainty. Do not substitute your own answer, add speculation, or change what the panel decided. If new evidence or a specific corrected inference warrants a different decision, explain the change and use revisit. Follow the result's continuation guidance. Retained materials from failed discussions are not a conclusion or consensus.";

/// Recognizes only a leading, complete command token, never quoted/source text.
pub fn fusion_prompt(message: &str) -> Option<&str> {
    let text = message.trim_start();
    let rest = text.strip_prefix("/fusion")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(rest.trim_start())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsultationInput {
    pub question: String,
    pub goal: String,
    pub background: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    /// Preserved by the parent adapter, never supplied by model tool arguments.
    /// Focused consultation questions remain separate from this source text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_request: Option<String>,
    /// Backend-selected only when the parent exposes Fusion and no execution tools.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub answer_only: bool,
    // Historical records remain readable; never forward this field to models.
    #[serde(default, skip_serializing, rename = "options")]
    pub legacy_options: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub read_scope: Vec<String>,
}

impl ConsultationInput {
    pub fn validate(&self) -> Result<()> {
        for (name, text) in [
            ("question", &self.question),
            ("goal", &self.goal),
            ("background", &self.background),
        ] {
            ensure!(!text.trim().is_empty(), "Fusion requires {name}");
        }
        ensure!(
            !self.answer_only
                || self
                    .parent_request
                    .as_ref()
                    .is_some_and(|source| source == &self.question),
            "Closed answer scope must match the preserved parent request"
        );
        ensure!(
            self.read_scope.len() <= 16,
            "At most 16 read scopes are allowed"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Parent {
    pub database_id: String,
    pub session_id: String,
    pub run_id: String,
    pub tool_call_id: String,
}

impl Parent {
    pub fn consultation_id(&self) -> String {
        format!(
            "fus_{:x}",
            Sha256::digest(serde_json::to_vec(self).expect("string fields"))
        )
    }
}

/// Explicitly reopen a completed decision; never silently discard its criterion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revisit {
    pub previous_run_id: String,
    pub kind: RevisitKind,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_scope_quote: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisitKind {
    NewEvidence,
    Counterexample,
    CorrectedInference,
    UserScopeChange,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionLineage {
    pub revisit: Revisit,
    pub previous_input: ConsultationInput,
    pub previous_result: Conclusion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Consultation {
    pub id: String,
    pub parent: Parent,
    pub input: ConsultationInput,
    pub workspace: String,
    pub snapshot_root: String,
    pub status: String,
    pub phase: String,
    pub completeness: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<crate::Configuration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<crate::ExecutionReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage: Option<DecisionLineage>,
    pub result: Option<Conclusion>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Consultation {
    pub fn terminal(&self) -> bool {
        matches!(
            self.status.as_str(),
            "completed" | "failed" | "cancelled" | "interrupted"
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Conclusion {
    #[serde(default)]
    pub fusion_run_id: String,
    pub conclusion: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_audit: Option<Value>,
    /// Validated user-facing answer for a sealed, full-parent closed task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_answer: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rationale: Vec<String>,
    #[serde(default, skip_serializing)]
    pub conditions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved: Vec<String>,
    #[serde(default, skip_serializing)]
    pub next_steps: Vec<String>,
    #[serde(default)]
    pub completeness: String,
}

/// A usable public report with unresolved validation issues; not verified evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportQuality {
    pub validation_errors: Vec<String>,
    /// Select the accepted public report rather than an unusable later repair.
    pub source_run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub fusion_run_id: String,
    pub phase: String,
    pub model: String,
    pub participant: String,
    pub attempt: i64,
    pub input: String,
    pub session_id: String,
    pub agent_run_id: String,
    pub status: String,
    pub output: String,
    pub error: Option<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub usage: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<crate::StepSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<crate::FailureKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_quality: Option<ReportQuality>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: i64,
    pub fusion_run_id: String,
    pub kind: String,
    pub data: Value,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discussion {
    pub run: Consultation,
    pub steps: Vec<Step>,
    pub events: Vec<Event>,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Strict JSON with optional Markdown fence, not guessed/substituted results.
pub fn parse_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
    let text = text.trim();
    let text = if text.starts_with("```") && text.ends_with("```") {
        text.split_once('\n')
            .map(|(_, body)| &body[..body.len() - 3])
            .unwrap_or(text)
    } else {
        text
    };
    Ok(serde_json::from_str(text.trim())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_boundaries_and_parent_identity() {
        assert_eq!(fusion_prompt("/fusion decide"), Some("decide"));
        assert_eq!(fusion_prompt(" /fusion\n决定"), Some("决定"));
        for text in [
            "/fusion-other x",
            "`/fusion x`",
            "> /fusion x",
            "```\n/fusion x\n```",
            "/fusion/file",
        ] {
            assert_eq!(fusion_prompt(text), None);
        }
        let a = Parent {
            database_id: "db".into(),
            session_id: "s".into(),
            run_id: "r".into(),
            tool_call_id: "t".into(),
        };
        let mut b = a.clone();
        b.tool_call_id = "t2".into();
        assert_ne!(a.consultation_id(), b.consultation_id());
        assert_eq!(a.consultation_id(), a.clone().consultation_id());
    }
}

#[cfg(test)]
mod independent_input_tests {
    #[test]
    fn large_context_and_empty_constraints_are_accepted_without_forwarding_legacy_options() {
        let input: super::ConsultationInput = serde_json::from_value(serde_json::json!({
            "question": "Q".repeat(3000),
            "goal": "User outcome",
            "background": "Factual context. ".repeat(10000),
            "options": ["Old main-agent preference"]
        }))
        .unwrap();
        input.validate().unwrap();
        assert!(input.constraints.is_empty());
        assert_eq!(input.legacy_options.len(), 1);
        let forwarded = serde_json::to_value(&input).unwrap();
        assert!(forwarded.get("options").is_none());
        assert_eq!(
            forwarded["background"].as_str().unwrap().len(),
            input.background.len()
        );
    }
}
