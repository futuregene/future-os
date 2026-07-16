//! Generated model catalog — re-exports from the future-agent-models crate.
//!
//! The models live in a separate crate so we can compile them with a lower
//! opt-level (opt-level=1) while the main agent gets normal optimizations.
//! This cuts release build time by ~60%.

pub use future_agent_models::*;
