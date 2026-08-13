//! Capability framework (LoopX: capabilities — caller-facing outcome
//! contracts that translate domain observations into a FINITE set of typed
//! proposals for the kernel to accept or reject).
//!
//! A capability never writes state itself: it proposes. The kernel decides
//! (LoopX: "capability can suggest a successor, cannot bypass todo authority
//! to assign work directly").
//!
//! All 14 domain packs from the LoopX capability catalog are shipped as
//! deterministic rule versions: agent_turn_recall, auto_research,
//! change_quality, content_ops, context_providers, decision_context,
//! explore, integration_branch, issue_fix, material_lifecycle,
//! periodic_report, reward_memory, semantic_preference, value_connectors —
//! plus the 15th (pr_review_queue, P2-3): the queue observation + review
//! contract rule version.

pub mod agent_turn_recall;
pub mod auto_research;
pub mod catalog;
pub mod change_quality;
pub mod content_ops;
pub mod context_providers;
pub mod decision_context;
pub mod explore;
pub mod integration_branch;
pub mod issue_fix;
pub mod lifecycle;
pub mod material_lifecycle;
pub mod periodic_report;
pub mod pr_review_queue;
pub mod resolver;
pub mod reward_memory;
pub mod semantic_preference;
pub mod value_connectors;

use crate::state::Todo;

/// The finite set of typed transitions a capability may propose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposalKind {
    /// Create a runnable successor todo.
    SuccessorTodo,
    /// The lane is exhausted — record an explicit no-follow-up intent.
    NoFollowUp,
    /// Something failed; reopen a bounded repair.
    Repair,
    /// A human decision is required.
    Gate,
    /// Periodic observation is required.
    Monitor,
}

#[derive(Debug, Clone)]
pub struct TypedProposal {
    pub kind: ProposalKind,
    pub todo: Option<Todo>,
    pub gate_question: Option<String>,
    pub reason: String,
}

impl TypedProposal {
    pub fn successor(todo: Todo, reason: &str) -> Self {
        Self {
            kind: ProposalKind::SuccessorTodo,
            todo: Some(todo),
            gate_question: None,
            reason: reason.to_string(),
        }
    }

    pub fn no_followup(reason: &str) -> Self {
        Self {
            kind: ProposalKind::NoFollowUp,
            todo: None,
            gate_question: None,
            reason: reason.to_string(),
        }
    }

    pub fn gate(question: &str, reason: &str) -> Self {
        Self {
            kind: ProposalKind::Gate,
            todo: None,
            gate_question: Some(question.to_string()),
            reason: reason.to_string(),
        }
    }

    pub fn monitor(todo: Todo, reason: &str) -> Self {
        Self {
            kind: ProposalKind::Monitor,
            todo: Some(todo),
            gate_question: None,
            reason: reason.to_string(),
        }
    }
}

/// A capability: a stable caller contract (input → finite typed proposals).
pub trait Capability: Send + Sync {
    fn name(&self) -> &'static str;
    fn describe(&self) -> &'static str;
    fn propose(&self, input: &str) -> Vec<TypedProposal>;
}

pub struct CapabilityRegistry {
    caps: Vec<Box<dyn Capability>>,
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self { caps: vec![] }
    }

    pub fn register(&mut self, cap: Box<dyn Capability>) {
        self.caps.push(cap);
    }

    pub fn all(&self) -> &[Box<dyn Capability>] {
        &self.caps
    }

    pub fn get(&self, name: &str) -> Option<&dyn Capability> {
        self.caps
            .iter()
            .find(|c| c.name() == name)
            .map(|b| b.as_ref())
    }

    /// Built-in registry with the shipped capabilities (all 14 domain packs
    /// from the LoopX capability catalog, deterministic rule versions).
    pub fn with_builtin() -> Self {
        let mut r = Self::new();
        r.register(Box::new(
            crate::capabilities::agent_turn_recall::AgentTurnRecallCapability,
        ));
        r.register(Box::new(
            crate::capabilities::auto_research::AutoResearchCapability,
        ));
        r.register(Box::new(
            crate::capabilities::change_quality::ChangeQualityCapability,
        ));
        r.register(Box::new(
            crate::capabilities::content_ops::ContentOpsCapability,
        ));
        r.register(Box::new(
            crate::capabilities::context_providers::ContextProvidersCapability,
        ));
        r.register(Box::new(
            crate::capabilities::decision_context::DecisionContextCapability,
        ));
        r.register(Box::new(crate::capabilities::explore::ExploreCapability));
        r.register(Box::new(
            crate::capabilities::integration_branch::IntegrationBranchCapability,
        ));
        r.register(Box::new(crate::capabilities::issue_fix::IssueFixCapability));
        r.register(Box::new(
            crate::capabilities::material_lifecycle::MaterialLifecycleCapability,
        ));
        r.register(Box::new(
            crate::capabilities::periodic_report::PeriodicReportCapability,
        ));
        r.register(Box::new(
            crate::capabilities::pr_review_queue::PrReviewQueueCapability,
        ));
        r.register(Box::new(
            crate::capabilities::reward_memory::RewardMemoryCapability,
        ));
        r.register(Box::new(
            crate::capabilities::semantic_preference::SemanticPreferenceCapability,
        ));
        r.register(Box::new(
            crate::capabilities::value_connectors::ValueConnectorsCapability,
        ));
        r
    }
}

/// Build a successor todo (helper used by capabilities).
pub fn successor_todo(prefix: &str, text: &str) -> Todo {
    Todo::advancement(&format!("{prefix}-{}", crate::state::now_epoch()), text)
}

pub fn monitor_todo(prefix: &str, text: &str, due_secs: u64) -> Todo {
    Todo::monitor(
        &format!("{prefix}-{}", crate::state::now_epoch()),
        text,
        std::time::Duration::from_secs(due_secs),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_is_empty() {
        let r = CapabilityRegistry::default();
        assert!(r.all().is_empty());
        assert!(r.get("anything").is_none());
    }
}
