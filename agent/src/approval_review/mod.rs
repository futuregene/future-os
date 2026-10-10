//! Automatic review classifies Ask requests; deterministic policy owns execution.
mod action;
mod budget;
mod context;
mod evidence;
mod fingerprint;
mod policy;
mod prompt;
mod provider;
mod redaction;
mod tool_evidence;
mod types;

pub(crate) use action::prepare_review_action;
pub(crate) use context::RunApprovalReviewer;
pub use policy::decide;
pub use provider::ApprovalReviewerClient;
pub use types::{Assessment, ProviderAssessment, ReviewStatus, Verdict};

const THRESHOLD: f64 = 0.75;
const PROBABILITY_SUM_TOLERANCE: f64 = 0.01 + 1e-9;
const PROMPT_VERSION: u32 = 5;
const REASON_CATALOG_VERSION: u32 = 2;
const POLICY_VERSION: u32 = 2;
const REVIEW_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const ACTION_RAW_BYTES_LIMIT: usize = 32 * 1024;

/// Account logout removes the model reviewer while preserving OS enforcement.
/// Applied both to newly configured policies and historical/queued run snapshots.
pub(crate) fn account_sandbox_policy(
    mut policy: crate::sandbox::SandboxPolicy,
    future_signed_in: bool,
) -> crate::sandbox::SandboxPolicy {
    policy.model_reviewer &= future_signed_in;
    policy
}

#[cfg(test)]
mod tests;
