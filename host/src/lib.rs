//! Application services shared by GUI and foreground CLI hosting.
//!
//! Entry points own runtime creation and presentation. This library does not
//! depend on Desktop, Tauri or the CLI. Backend migration is incremental.
pub mod config_io;
mod error;

pub use error::Error;
