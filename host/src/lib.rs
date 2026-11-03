//! Application services shared by GUI and foreground CLI hosting.
//!
//! Entry points own runtime creation and presentation. This library does not
//! depend on Desktop, Tauri or the CLI. Backend migration is incremental.
pub mod config_io;
mod error;

pub use error::Error;

// Terminal writes that drop a failed write instead of panicking (see
// `future_agent::console`). Re-exported so a host or Desktop file can take
// `crate::console::eprintln` over the prelude's and keep its call sites.
pub use future_agent::console;

pub mod approval_rules;
mod git_diff_parse;
mod proc;
pub use proc::NoWindow;
pub mod run_error;

pub mod runtime;

pub mod events;

pub mod auth_store;
pub mod build_info;
pub mod future_platform;
mod home;
pub mod skills;
pub use home::{future_home_root, home_dir};

/// Install the process TLS provider before an entry point builds an HTTP client.
/// The provider choice lives in `future-http`; this alias keeps host call sites
/// (and the Desktop) reading as the host's own concern.
pub fn install_rustls_provider() {
    future_http::install_tls_provider();
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub static TEST_HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub mod agent_events;
pub mod store;
pub(crate) use events::{
    emit_approvals_updated, emit_control_claimed, emit_remote_activity, emit_review_updated,
    emit_tasks_updated, emit_thread_activity, emit_thread_runtime_updated, emit_threads_updated,
};

pub mod device_identity;
pub mod git_review;
pub mod instance;
pub mod shadow_review;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub mod agent_commands;
pub mod request_scope;

pub mod agent_client;

pub mod files;

pub mod agent_host;
mod crash_log;
pub mod environment;

pub mod agent_bridge;
pub mod agent_providers;

pub mod conversations;
pub mod tasks;

pub mod future_login;

pub mod scheduler;

pub mod workspaces;

pub mod remote;
mod remote_host;
pub mod remote_peer;
mod remote_platform;

pub mod shutdown;

/// Launch steps and the stall watchdog shared by every host entry point.
pub mod startup;

pub mod cli_tool;

pub mod application;

pub mod fusion;
