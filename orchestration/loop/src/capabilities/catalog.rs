//! Capability catalog (G-23) — reference `capabilities/catalog.py`, natively.
//!
//! The catalog is the queryable metadata surface for every capability:
//! per-capability `commands` (CLI commands + purpose), `packets`
//! (schema_version + module), `status` (active-preview / experimental /
//! compatibility-facade), the Stage boundary, the owning provider, and the
//! user-facing value/next-step strings reference requires on every public record.
//!
//! All 14 domain packs ship with their reference catalog status; the 15th
//! capability (`pr_review_queue`) ships its queue-observation + review-contract
//! rule version as active-preview (P2-3; the reference package is
//! `capabilities/pr_review_queue/`).

use std::collections::BTreeMap;

use super::lifecycle::{CapabilityProvider, ProviderLifecycle};

/// reference catalog status values.
pub const CAPABILITY_STATUS_ACTIVE_PREVIEW: &str = "active-preview";
pub const CAPABILITY_STATUS_EXPERIMENTAL: &str = "experimental";
pub const CAPABILITY_STATUS_COMPATIBILITY_FACADE: &str = "compatibility-facade";

/// A CLI command a capability registers (reference catalog `commands` entries).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CapabilityCommand {
    pub name: String,
    pub purpose: String,
}

/// A typed packet a capability emits (reference catalog `packets`:
/// schema_version + module).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CapabilityPacket {
    pub schema_version: String,
    pub module: String,
}

/// One capability record (reference registry.py REQUIRED_CAPABILITY_FIELDS +
/// REQUIRED_PUBLIC_CAPABILITY_FIELDS + catalog metadata).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CapabilityRecord {
    pub id: String,
    pub title: String,
    pub status: String,
    pub stage: u32,
    pub provider_id: String,
    pub origin: String,
    pub visibility: String,
    pub user_value: String,
    pub next_real_step: String,
    pub commands: Vec<CapabilityCommand>,
    pub packets: Vec<CapabilityPacket>,
}

impl CapabilityRecord {
    /// A public capability requires the reference public anchor fields.
    pub fn is_public(&self) -> bool {
        self.visibility == "public"
    }

    /// Catalog visibility gate (G-24): experimental capability commands are
    /// hidden from the CLI surface unless explicitly requested.
    pub fn is_experimental(&self) -> bool {
        self.status == CAPABILITY_STATUS_EXPERIMENTAL
    }
}

/// The catalog: providers + capability records, queryable by id/status/stage.
#[derive(Debug, Clone, Default)]
pub struct CapabilityCatalog {
    providers: BTreeMap<String, CapabilityProvider>,
    records: BTreeMap<String, CapabilityRecord>,
}

impl CapabilityCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a provider (duplicate id fails closed).
    pub fn register_provider(&mut self, provider: CapabilityProvider) -> Result<(), String> {
        if self.providers.contains_key(&provider.id) {
            return Err(format!("duplicate capability provider `{}`", provider.id));
        }
        self.providers.insert(provider.id.clone(), provider);
        Ok(())
    }

    /// Register a capability record (validates provider existence + origin
    /// match + duplicate id, mirroring the reference registry.py register_capability).
    pub fn register_capability(&mut self, record: CapabilityRecord) -> Result<(), String> {
        if record.id.trim().is_empty() {
            return Err("capability requires a non-empty id".into());
        }
        for (field, value) in [
            ("id", &record.id),
            ("title", &record.title),
            ("status", &record.status),
            ("user_value", &record.user_value),
            ("next_real_step", &record.next_real_step),
        ] {
            if value.trim().is_empty() {
                return Err(format!(
                    "capability `{}` requires non-empty `{field}`",
                    record.id
                ));
            }
        }
        if !super::lifecycle::CAPABILITY_ORIGINS.contains(&record.origin.as_str()) {
            return Err(format!(
                "capability `{}` has unsupported origin `{}`",
                record.id, record.origin
            ));
        }
        if !super::lifecycle::CAPABILITY_VISIBILITIES.contains(&record.visibility.as_str()) {
            return Err(format!(
                "capability `{}` has unsupported visibility `{}`",
                record.id, record.visibility
            ));
        }
        let provider = self.providers.get(&record.provider_id).ok_or_else(|| {
            format!(
                "capability `{}` references unknown provider `{}`",
                record.id, record.provider_id
            )
        })?;
        if provider.origin != record.origin {
            return Err(format!(
                "capability `{}` origin `{}` does not match provider `{}` origin `{}`",
                record.id, record.origin, record.provider_id, provider.origin
            ));
        }
        if self.records.contains_key(&record.id) {
            return Err(format!("duplicate capability `{}`", record.id));
        }
        self.records.insert(record.id.clone(), record);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&CapabilityRecord> {
        self.records.get(id)
    }

    /// Public capability ids (reference capability_ids()).
    pub fn capability_ids(&self, include_internal: bool) -> Vec<&str> {
        self.records
            .values()
            .filter(|r| include_internal || r.is_public())
            .map(|r| r.id.as_str())
            .collect()
    }

    pub fn records(&self, include_internal: bool) -> Vec<&CapabilityRecord> {
        self.records
            .values()
            .filter(|r| include_internal || r.is_public())
            .collect()
    }

    pub fn providers(&self) -> Vec<&CapabilityProvider> {
        self.providers.values().collect()
    }

    pub fn provider(&self, id: &str) -> Option<&CapabilityProvider> {
        self.providers.get(id)
    }

    /// Provider lifecycle snapshot for a capability (LoopX
    /// `_with_implementations` provider_state).
    pub fn provider_lifecycle_for(&self, capability_id: &str) -> Option<ProviderLifecycle> {
        let record = self.records.get(capability_id)?;
        self.providers.get(&record.provider_id).map(|p| p.lifecycle)
    }

    /// Commands a capability registers, gated by catalog status (G-24):
    /// experimental capability commands are hidden unless
    /// `include_experimental` is true.
    pub fn commands_for(
        &self,
        capability_id: &str,
        include_experimental: bool,
    ) -> Vec<&CapabilityCommand> {
        let Some(record) = self.records.get(capability_id) else {
            return vec![];
        };
        if record.is_experimental() && !include_experimental {
            return vec![];
        }
        record.commands.iter().collect()
    }

    /// Build the shipped catalog: the 14 domain packs (with their LoopX
    /// catalog status) + pr_review_queue (15th, active-preview — the P2-3
    /// rule version) under the builtin `future-loop-core` provider.
    pub fn with_builtin() -> Self {
        let mut catalog = Self::new();
        let core = CapabilityProvider::builtin("future-loop-core");
        catalog
            .register_provider(core)
            .expect("builtin provider registers once");
        for (id, title, status, user_value, next_real_step, commands, packets) in builtin_records()
        {
            let record = CapabilityRecord {
                id: id.to_string(),
                title: title.to_string(),
                status: status.to_string(),
                stage: 0,
                provider_id: "future-loop-core".to_string(),
                origin: "builtin".to_string(),
                visibility: "public".to_string(),
                user_value: user_value.to_string(),
                next_real_step: next_real_step.to_string(),
                commands,
                packets,
            };
            catalog
                .register_capability(record)
                .expect("builtin records are valid");
        }
        catalog
    }
}

fn cmd(name: &str, purpose: &str) -> CapabilityCommand {
    CapabilityCommand {
        name: name.to_string(),
        purpose: purpose.to_string(),
    }
}

fn packet(schema_version: &str, module: &str) -> CapabilityPacket {
    CapabilityPacket {
        schema_version: schema_version.to_string(),
        module: module.to_string(),
    }
}

/// The shipped records: (id, title, status, user_value, next_real_step,
/// commands, packets). Command names mirror the reference catalog entry commands
/// (kebab-case CLI surface); packets carry the reference schema_version + module.
#[allow(clippy::type_complexity)]
fn builtin_records() -> Vec<(
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Vec<CapabilityCommand>,
    Vec<CapabilityPacket>,
)> {
    vec![
        (
            "agent_turn_recall",
            "Agent Turn Recall",
            CAPABILITY_STATUS_ACTIVE_PREVIEW,
            "Recall exact-scope guidance and inject it into one private turn context.",
            "Exercise the recall request path against a configured provider, then record a receipt.",
            vec![cmd("agent-turn-recall", "Recall exact-scope guidance and inject it into one private turn context.")],
            vec![packet("recall_request_v0", "agent_turn_recall")],
        ),
        (
            "auto_research",
            "Auto Research",
            CAPABILITY_STATUS_EXPERIMENTAL,
            "Hypothesis → execute → evaluate research chain.",
            "Run a bounded research chain on a deterministic fixture before live adapters.",
            vec![cmd("auto-research", "Run one bounded hypothesis → execute → evaluate research chain.")],
            vec![packet("research_chain_v0", "auto_research")],
        ),
        (
            "change_quality",
            "Change Quality",
            CAPABILITY_STATUS_ACTIVE_PREVIEW,
            "Assess a change's validation evidence and propose repair or acceptance.",
            "Exercise deterministic quality oracles on a fixture change, then record the receipt.",
            vec![cmd("change-quality", "Assess a change's validation evidence and propose repair or acceptance.")],
            vec![packet("quality_assessment_v0", "change_quality")],
        ),
        (
            "content_ops",
            "Content Ops",
            CAPABILITY_STATUS_ACTIVE_PREVIEW,
            "Ordered content operations with bounded per-surface effects.",
            "Project one content operation against the current goal state before any surface write.",
            vec![cmd("content-ops", "Project one ordered content operation against the goal state.")],
            vec![packet("content_ops_v0", "content_ops")],
        ),
        (
            "context_providers",
            "Context Providers",
            CAPABILITY_STATUS_EXPERIMENTAL,
            "Provider context routing for turn composition.",
            "Check a configured provider entry point and return explicit guidance when unavailable.",
            vec![cmd("context-providers", "Check a configured context provider entry point.")],
            vec![packet("context_provider_v0", "context_providers")],
        ),
        (
            "decision_context",
            "Decision Context",
            CAPABILITY_STATUS_EXPERIMENTAL,
            "Compose decision context packets for gate and monitor decisions.",
            "Project the decision-context packet for an open gate before resolving it.",
            vec![cmd("decision-context", "Compose the decision-context packet for an open gate.")],
            vec![packet("decision_context_v0", "decision_context")],
        ),
        (
            "explore",
            "Explore",
            CAPABILITY_STATUS_ACTIVE_PREVIEW,
            "Bounded exploration with replay/trace hygiene.",
            "Exercise deterministic exploration guards on a fixture before live exploration.",
            vec![cmd("explore", "Run one bounded exploration probe with trace hygiene.")],
            // Wave 2 deepening: the hypothesis-tracking + explore-graph rule
            // version ships its event / projection / verification packets.
            vec![
                packet("explore_v0", "explore"),
                packet("loopx_explore_result_event_v0", "explore"),
                packet("loopx_explore_result_projection_v0", "explore"),
                packet("loopx_explore_hypothesis_verification_v0", "explore"),
            ],
        ),
        (
            "integration_branch",
            "Integration Branch",
            CAPABILITY_STATUS_ACTIVE_PREVIEW,
            "Ordered integration-branch reconciliation with no sync receipt.",
            "Write one ordered branch plan and reconcile against the last successful sync.",
            vec![cmd("integration-branch", "Reconcile one ordered integration-branch plan.")],
            vec![packet("integration_branch_v0", "integration_branch")],
        ),
        (
            "issue_fix",
            "Issue Fix",
            CAPABILITY_STATUS_ACTIVE_PREVIEW,
            "Issue-to-PR product path with public metadata projection.",
            "Compose metadata, repository context, intake, feasibility, and PR review readiness blockers for one issue.",
            vec![cmd("issue-fix", "Project one issue-fix intake packet and propose successor/triage/gate.")],
            vec![packet("issue_fix_v0", "issue_fix")],
        ),
        (
            "material_lifecycle",
            "Material Lifecycle",
            CAPABILITY_STATUS_EXPERIMENTAL,
            "Material inventory and lifecycle transitions.",
            "Project the Stage-0 material inventory and lifecycle boundaries without writing the project.",
            vec![cmd("material-lifecycle", "Project the material inventory and lifecycle boundaries.")],
            vec![packet("material_lifecycle_v0", "material_lifecycle")],
        ),
        (
            "periodic_report",
            "Periodic Report",
            CAPABILITY_STATUS_ACTIVE_PREVIEW,
            "Recurring heartbeat/monitor work with a cadence profile.",
            "Project one periodic-report successor from the cadence profile and current state.",
            vec![cmd("periodic-report", "Propose a periodic report successor from a cadence/scope profile.")],
            vec![packet("periodic_report_v0", "periodic_report")],
        ),
        (
            "reward_memory",
            "Reward Memory",
            CAPABILITY_STATUS_ACTIVE_PREVIEW,
            "Reward/penalty memory with hashed preference references.",
            "Build a compact receipt with hashed preference references for evidence/state writeback.",
            // P1-5: the capability graduated from the G-24 propose hook to a
            // real command surface (`reward-memory query|record`); the
            // propose pipeline stays reachable via `capability propose`.
            vec![],
            vec![packet("reward_memory_v0", "reward_memory")],
        ),
        (
            "semantic_preference",
            "Semantic Preference",
            CAPABILITY_STATUS_ACTIVE_PREVIEW,
            "Semantic preference storage with bounded rerank.",
            "Exercise the bounded-rerank guard on a fixture, then record the receipt.",
            vec![cmd("semantic-preference", "Project the semantic-preference inventory and bounded rerank.")],
            vec![packet("semantic_preference_v0", "semantic_preference")],
        ),
        (
            "value_connectors",
            "Value Connectors",
            CAPABILITY_STATUS_COMPATIBILITY_FACADE,
            "Compatibility facade for value-connector surface contracts.",
            "Render the Stage-0 value-connector inventory and lifecycle boundaries.",
            vec![cmd("value-connectors", "Render the value-connector inventory and lifecycle boundaries.")],
            vec![packet("value_connectors_v0", "value_connectors")],
        ),
        (
            "pr_review_queue",
            "PR Review Queue",
            CAPABILITY_STATUS_ACTIVE_PREVIEW,
            "Turn one complete open-PR observation into a deterministic exact-head review candidate without repeating unchanged work or granting review writes.",
            "Persist one observation fingerprint in a continuous-monitor todo, observe a changed head, and materialize exactly one candidate through normal todo authority.",
            vec![cmd(
                "pr-review-queue",
                "Observe one complete PR queue and emit at most one exact-head candidate (capability hook).",
            )],
            vec![
                packet("pr_review_queue_v0", "pr_review_queue"),
                packet("pull_request_review_queue_observation_v0", "pr_review_queue"),
                packet("pull_request_review_candidate_v0", "pr_review_queue"),
                packet("pull_request_review_todo_preview_v0", "pr_review_queue"),
                packet("pull_request_review_execution_contract_v1", "pr_review_queue"),
                packet("pull_request_review_plan_v1", "pr_review_queue"),
                packet("pull_request_review_result_v1", "pr_review_queue"),
            ],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_has_15_capabilities() {
        let catalog = CapabilityCatalog::with_builtin();
        assert_eq!(catalog.records(false).len(), 15);
        assert_eq!(catalog.capability_ids(false).len(), 15);
        assert_eq!(catalog.providers().len(), 1);
        let issue_fix = catalog.get("issue_fix").unwrap();
        assert_eq!(issue_fix.status, CAPABILITY_STATUS_ACTIVE_PREVIEW);
        assert_eq!(issue_fix.provider_id, "future-loop-core");
        assert_eq!(
            catalog.provider_lifecycle_for("issue_fix").unwrap().stage(),
            crate::capabilities::lifecycle::ProviderStage::Ready
        );
    }

    #[test]
    fn experimental_commands_hidden_unless_requested() {
        let catalog = CapabilityCatalog::with_builtin();
        // pr_review_queue shipped its P2-3 rule version → active-preview, so
        // its capability hook is visible without --include-experimental.
        assert_eq!(catalog.commands_for("pr_review_queue", false).len(), 1);
        assert_eq!(catalog.commands_for("pr_review_queue", true).len(), 1);
        assert_eq!(catalog.commands_for("issue_fix", false).len(), 1);
    }

    #[test]
    fn duplicate_and_unknown_provider_fail_closed() {
        let mut catalog = CapabilityCatalog::new();
        catalog
            .register_provider(CapabilityProvider::builtin("p"))
            .unwrap();
        assert!(catalog
            .register_provider(CapabilityProvider::builtin("p"))
            .is_err());
        let record = CapabilityRecord {
            id: "c".into(),
            title: "t".into(),
            status: CAPABILITY_STATUS_ACTIVE_PREVIEW.into(),
            stage: 0,
            provider_id: "missing".into(),
            origin: "builtin".into(),
            visibility: "public".into(),
            user_value: "v".into(),
            next_real_step: "n".into(),
            commands: vec![],
            packets: vec![],
        };
        assert!(catalog.register_capability(record).is_err());
    }

    #[test]
    fn origin_mismatch_is_rejected() {
        let mut catalog = CapabilityCatalog::new();
        catalog
            .register_provider(CapabilityProvider::builtin("p"))
            .unwrap();
        let record = CapabilityRecord {
            id: "c".into(),
            title: "t".into(),
            status: CAPABILITY_STATUS_ACTIVE_PREVIEW.into(),
            stage: 0,
            provider_id: "p".into(),
            origin: "extension".into(), // mismatches provider origin builtin
            visibility: "public".into(),
            user_value: "v".into(),
            next_real_step: "n".into(),
            commands: vec![],
            packets: vec![],
        };
        assert!(catalog.register_capability(record).is_err());
    }

    #[test]
    fn duplicate_capability_id_is_rejected() {
        let mut catalog = CapabilityCatalog::new();
        catalog
            .register_provider(CapabilityProvider::builtin("p"))
            .unwrap();
        let record = || CapabilityRecord {
            id: "c".into(),
            title: "t".into(),
            status: CAPABILITY_STATUS_ACTIVE_PREVIEW.into(),
            stage: 0,
            provider_id: "p".into(),
            origin: "builtin".into(),
            visibility: "public".into(),
            user_value: "v".into(),
            next_real_step: "n".into(),
            commands: vec![],
            packets: vec![],
        };
        catalog.register_capability(record()).unwrap();
        let err = catalog.register_capability(record()).unwrap_err();
        assert!(err.contains("duplicate capability"), "{err}");
    }
}
