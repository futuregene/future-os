//! Application services shared by GUI and foreground CLI hosting.
//!
//! Entry points own runtime creation and presentation. This library does not
//! depend on Desktop, Tauri or the CLI. Backend migration is incremental.
pub mod config_io;
mod error;

pub use error::Error;

pub mod approval_rules;
pub mod git_diff_parse;
pub mod proc;
pub mod run_error;

pub mod runtime;

pub mod events;

pub mod auth_store;
pub mod build_info;
pub mod future_platform;
mod home;
pub mod skills;
pub use home::home_dir;

pub fn install_rustls_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub static TEST_HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
