//! FutureOS loop control plane — a native, LoopX-style implementation.
//!
//! Library surface exposes the state kernel, the decision compiler, the
//! durable store, and the gRPC executor so contract tests can exercise the
//! deterministic parts without touching gRPC or any LLM.

pub mod agent_client;
pub mod agents;
pub mod backfill;
pub mod benchmark;
pub mod canary;
pub mod capabilities;
pub mod cli;
pub mod cli_projection;
pub mod compat;
pub mod console;
pub mod contract;
pub mod decision;
pub mod executor;
pub mod extensions;
pub mod handoff;
pub mod heartbeat;
pub mod migration;
pub mod projection;
pub mod quota;
pub mod replay;
pub mod runtime;
pub mod scheduler;
pub mod state;
pub mod status_server;
pub mod store;
pub mod turn_envelope;
pub mod work_items;
pub mod worker_bridge;
