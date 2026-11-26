//! Host-independent Fusion contracts, durable queue, evidence and file queries.
mod config;
mod context;
mod files;
mod resilience;
mod store;
mod types;

pub use config::*;
pub use context::*;
pub use files::*;
pub use resilience::*;
pub use store::*;
pub use types::*;

/// The Future gateway accepts this request budget for the tested Fusion roles,
/// including GLM 5.3 Flash, whose catalog currently reports 128,000.
pub fn output_limit_override(provider: &str, model: &str) -> Option<i32> {
    (provider == "future" && matches!(model, "qwen3.8-flash" | "glm-5.3-flash" | "kimi-k2.6"))
        .then_some(131_072)
}
