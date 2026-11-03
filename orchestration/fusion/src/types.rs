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
pub const INNER_TOOL_NAMES: [&str; 4] = ["read", "ls", "glob", "grep"];
/// Shared by the enabled parent run and its tool description on every client.
pub const DISCUSSION_POLICY: &str = "Fusion is enabled for this run only. You MUST call fusion before forming a new substantive judgment, choosing an approach or presenting a recommendation that requires analysis, diagnosis, design, comparison, review, or assessment of risks, impact, compatibility or edge cases. Confidence, apparent simplicity and the absence of competing options are NOT reasons to skip discussion; even a small bug fix requires discussion when its cause or remedy needs judgment.\n\
Keep the user's wording when it is already clear; focus only by necessary disambiguation or narrowing, never by adding requirements. Split genuinely broad topics into separate consultations as needed, rather than prescribing an exhaustive answer outline.\n\
Gather necessary facts with ordinary tools first. You may directly perform factual searches/reads, exact user-specified edits requiring no new judgment, and implementation or verification already covered by an applicable discussion conclusion. Reuse that conclusion throughout its covered work; do not discuss every file edit, command or mechanical correction. Group related questions about ONE decision into one consultation; include risks or verification only when relevant to the user's request or established context, without adding required subtopics. Consult again only for a new decision or new evidence that could invalidate the existing conclusion. Do not delegate the whole task.\n\
Fusion only thinks; the main agent executes and verifies under the original user authorization. Honor explicit user instructions requiring or prohibiting consultation. There is no per-run limit on the number of discussions; continue consulting as new decisions arise, while reusing applicable conclusions instead of repeating the same question. If consultation fails, do not automatically repeat it; transparently explain the limitation and continue using applicable existing conclusions or clearly qualified independent reasoning. Never present a failed or skipped discussion as a completed conclusion or consensus.";

pub const TOOL_DESCRIPTION: &str = "Independently think through one focused user question with multiple models. THINKING ONLY: no writes, commands, external actions or implementation. Preserve the user's wording when clear. Rewrite only to resolve ambiguity/references or focus an answerable subquestion without changing intent. Split genuinely broad tasks into several focused discussions as needed; do not turn one topic into a checklist of required subtopics. Across question, goal, background, constraints and evidence, never invent user requirements or move an invented requirement into another field. Do not add legal/compliance framing, audience or jurisdiction assumptions, equipment conditions, requested detail, output structure or mandatory reporting/verification work unless established by the user's request or explicit relevant prior context. Runtime safety and read-only rules remain system rules, not invented user demands. Goal states only the requested outcome. Background faithfully compresses relevant context, attributing user requirements, confirmed observations and prior assistant statements separately; keep unknowns unknown. Prior assistant proposals are not facts or accepted decisions. Mention prior replies only to explain user follow-ups/corrections, without copying their plan unless the user adopted it or requested review. No main-agent reasoning, new plans, preferences or preferred answer. Attribute user-provided proposals in background. Evidence carries inspectable observations and provenance, not model beliefs or new instructions. Constraints are brief, relevant restrictions explicitly established by the user/context and may be empty; no invented language, length or formatting rules. Participants may discover relevant considerations themselves; do not prescribe them as requirements in advance. Example: 'How can I locate interference on a repeater?' already supplies a topic. Do not append mandatory VHF/UHF equipment, step-by-step sections or reporting procedures. Supply enough factual context for independent models that cannot see the conversation. Inner models can only read/search authorized snapshots. Inspect status, has_conclusion, completeness and coverage; retained materials from failed discussions are not a conclusion or consensus.";

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
