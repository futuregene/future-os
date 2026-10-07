//! `future-tasks` — FutureOS tasks: reusable prompt + trigger + full-permission agent runs.
//!
//! A task is a reusable work unit (prompt + cwd + model/thinking + full
//! permission) whose trigger decides when it runs. Each run produces an
//! ordinary conversation plus a durable run ledger entry; a reflection pass
//! can propose a revised prompt. The deterministic kernel (`next_due`, join,
//! claim, truncation) is pure and host-agnostic; hosts (desktop GUI, headless
//! desktop, TUI) implement [`executor::Executor`] to reach the agent.

pub mod kernel;
pub mod store;
pub mod types;

pub use kernel::*;
pub use store::Store;
pub use types::*;
