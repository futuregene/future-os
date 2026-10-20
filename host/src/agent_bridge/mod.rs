//! Shared Agent application services. The bridge migration is incremental.
pub mod config;
pub mod models;
pub mod skills;

pub use models::{list_agent_models, list_builtin_providers, AgentModelOption};
pub use skills::{
    install_skill, list_available_skills, list_installed_skills, refresh_skills, suggest_skill,
    sync_skills, uninstall_skill, AvailableSkill, InstalledSkill, SkillCandidate,
};

mod client {
    pub use crate::agent_client::{connect_agent, map_rpc_error};
    pub use crate::agent_commands::*;
}
#[cfg(test)]
pub(crate) use crate::test_support::agent as test_support;

pub mod persist;
pub mod review;

pub mod config_observer;
