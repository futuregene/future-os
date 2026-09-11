//! Desktop command surface. The server build retains the file/workspace
//! business helpers used by Remote, without compiling Tauri IPC wrappers.

#[cfg(feature = "gui")]
mod agent;
#[cfg(test)]
pub(crate) mod agent_mock;
#[cfg(feature = "gui")]
mod app;
#[cfg(feature = "gui")]
mod approvals;
#[cfg(feature = "gui")]
mod artifacts;
#[cfg(feature = "gui")]
mod debug;
mod files;
#[cfg(all(test, feature = "gui"))]
pub(crate) mod ipc_harness;
#[cfg(feature = "gui")]
mod login;
#[cfg(feature = "gui")]
mod providers;
#[cfg(feature = "gui")]
mod references;
#[cfg(feature = "gui")]
mod remote;
#[cfg(feature = "gui")]
mod review;
#[cfg(feature = "gui")]
mod runs;
#[cfg(feature = "gui")]
mod settings;
#[cfg(feature = "gui")]
mod skills;
#[cfg(feature = "gui")]
mod terminal;
#[cfg(feature = "gui")]
mod threads;
#[cfg(feature = "gui")]
mod update;
mod workspaces;

#[cfg(feature = "gui")]
pub use self::agent::*;
#[cfg(feature = "gui")]
pub use self::app::*;
#[cfg(feature = "gui")]
pub use self::approvals::*;
#[cfg(feature = "gui")]
pub use self::artifacts::*;
#[cfg(feature = "gui")]
pub use self::debug::*;
pub use self::files::*;
#[cfg(feature = "gui")]
pub use self::login::*;
#[cfg(feature = "gui")]
pub use self::providers::*;
#[cfg(feature = "gui")]
pub use self::references::*;
#[cfg(feature = "gui")]
pub use self::remote::*;
#[cfg(feature = "gui")]
pub use self::review::*;
#[cfg(feature = "gui")]
pub use self::runs::*;
#[cfg(feature = "gui")]
pub use self::settings::*;
#[cfg(feature = "gui")]
pub use self::skills::*;
#[cfg(feature = "gui")]
pub use self::terminal::*;
#[cfg(feature = "gui")]
pub use self::threads::*;
#[cfg(feature = "gui")]
pub use self::update::*;
pub use self::workspaces::*;
