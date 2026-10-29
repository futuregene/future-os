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
pub const MAX_INPUT_BYTES: usize = 48 * 1024;
pub const MAX_RESULT_BYTES: usize = 32 * 1024;
pub const MAX_CALLS_PER_PARENT_RUN: usize = 2;
pub const STEP_TIMEOUT_SECS: u64 = 180;
pub const RUN_TIMEOUT_SECS: u64 = 900;
pub const INNER_TOOL_NAMES: [&str; 4] = ["read", "ls", "glob", "grep"];
pub const TOOL_DESCRIPTION: &str = "Think through ONE focused decision with multiple models and return a final recommendation. This is THINKING ONLY: no file writes, shell commands, external actions or implementation. Investigate first. Provide the goal, relevant background, confirmed facts, unknowns, constraints and necessary evidence; include candidate options when useful. Use for substantive alternatives, important tradeoffs or independent cross-checking, not routine searches, obvious local fixes or every implementation step. Do not delegate the whole task. Inner models can read/search authorized files only. Propose actions and verification for the main agent to carry out under the original user authorization. A recommendation does not grant permission or prove verification. Reuse an existing conclusion unless new evidence changes the decision.";

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
    pub constraints: Vec<String>,
    #[serde(default)]
    pub options: Vec<String>,
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
            self.question.len() <= 2048,
            "Focus Fusion on one decision (question exceeds 2048 bytes)"
        );
        ensure!(
            self.read_scope.len() <= 16,
            "At most 16 read scopes are allowed"
        );
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_INPUT_BYTES,
            "Fusion input exceeds 48 KiB; narrow the question and evidence"
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conclusion {
    #[serde(default)]
    pub fusion_run_id: String,
    pub conclusion: String,
    pub rationale: Vec<String>,
    #[serde(default)]
    pub conditions: Vec<String>,
    #[serde(default)]
    pub unresolved: Vec<String>,
    #[serde(default)]
    pub next_steps: Vec<String>,
    #[serde(default)]
    pub completeness: String,
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
