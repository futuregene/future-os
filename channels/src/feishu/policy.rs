//! Feishu's view of the shared access-policy engine.
//!
//! The engine itself lives in [`crate::policy`] so every channel applies the
//! same DM / group / mention rules. This module only keeps the historical
//! import paths (`feishu::policy::PolicyEngine`) resolving.

pub use crate::policy::{Access, AccessPolicyConfig as PolicyConfig, ChatOverride, PolicyEngine};
