//! FutureOS loop control plane — a native, LoopX-style implementation.
//!
//! This crate contains code derived from LoopX
//! (<https://github.com/huangruiteng/loopx>), licensed under the Apache
//! License, Version 2.0 — see `LICENSE`, `NOTICE`, and `UPSTREAM.md` in
//! this directory for upstream attribution, the derivation scope, and
//! FutureGene's modifications.
//!
//! Library surface exposes the state kernel, the decision compiler, the
//! durable store, and the gRPC executor so contract tests can exercise the
//! deterministic parts without touching gRPC or any LLM.

pub mod agent_client;
pub mod agents;
pub mod backfill;
pub mod canary;
pub mod cli;
pub mod cli_projection;
pub mod compat;
pub mod console;
pub mod contract;
pub mod decision;
pub mod executor;
pub mod heartbeat;
pub mod migration;
pub mod projection;
pub mod quota;
pub mod runtime;
pub mod scheduler;
pub mod state;
pub mod store;
pub mod turn_envelope;
pub mod webui;
pub mod work_items;
pub mod worker_bridge;
