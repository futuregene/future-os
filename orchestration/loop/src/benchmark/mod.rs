//! Benchmark (G-18) — the minimal benchmark closed loop, LoopX
//! `benchmark_core/` + `benchmark_ledger.py` natively.
//!
//! LoopX ships a full evaluation system (benchmark_core 17 files +
//! benchmark_adapters 28 + benchmark_ledger 3.8k lines + canary/). We build
//! the minimal closed loop the plan scopes: the loop protocol contract
//! (`loop_protocol`), the run ledger (`ledger`), one adapter that reuses our
//! gRPC direct-drive channel (`adapter`), and a single qualification scenario
//! runner (`qualification`). The deterministic parts (protocol classification,
//! ledger compaction, adapter classification, round accounting) are fully
//! contract-tested without an LLM; the gRPC adapter is a thin transport over
//! the already-tested `AgentClient`.
//!
//! Design constraint (§5.4): the benchmark builds on the gRPC channel — never
//! a parallel execution path. `GrpcLoopxAdapter` is exactly that: preflight →
//! launch (one bounded prompt) → observe (run_turn readback) → classify →
//! ledger, with every policy decision in the deterministic kernel.

pub mod adapter;
pub mod ledger;
pub mod loop_protocol;
pub mod qualification;
