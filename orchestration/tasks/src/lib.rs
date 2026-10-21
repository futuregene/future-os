//! `future-tasks` — FutureOS tasks: reusable prompt + trigger + full-permission agent runs.
//!
//! A task is a reusable work unit (prompt + cwd + model/thinking + full
//! permission) whose trigger decides when it runs; it can always be run on
//! demand as well. Each run produces an ordinary conversation plus a durable run
//! ledger entry. The deterministic kernel (`next_due`, truncation, prompt
//! revisions) is pure and host-agnostic; hosts (desktop GUI, headless desktop,
//! TUI) drive it from their own tick loop.

pub mod kernel;
pub mod store;
pub mod timezone;
pub mod types;

pub use kernel::*;
pub use store::Store;
pub use timezone::*;
pub use types::*;
