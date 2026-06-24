//! Tauri command surface, grouped by domain. Each submodule holds thin
//! `#[tauri::command]` wrappers that delegate to `store`, `agent_bridge`,
//! `git_review`, or `agent_providers`; orchestration that spans the agent and
//! the store lives in [`crate::agent_bridge`]. Re-exported flat so `lib.rs` can
//! list bare command names in `generate_handler!`.

mod agent;
mod app;
mod approvals;
mod artifacts;
mod files;
mod login;
mod messages;
mod providers;
mod references;
mod review;
mod runs;
mod settings;
mod threads;
mod workspaces;

pub use self::agent::*;
pub use self::app::*;
pub use self::approvals::*;
pub use self::artifacts::*;
pub use self::files::*;
pub use self::login::*;
pub use self::messages::*;
pub use self::providers::*;
pub use self::references::*;
pub use self::review::*;
pub use self::runs::*;
pub use self::settings::*;
pub use self::threads::*;
pub use self::workspaces::*;
