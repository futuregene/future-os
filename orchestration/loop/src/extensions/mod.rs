//! Extensions (G-21/G-22) — the ecosystem injection point (LoopX
//! `extensions/`, natively). v1 is declarative only: manifests drive
//! capability registration and command metadata; no native code is loaded or
//! executed (security tradeoff documented in the P3 plan; process_runtime is
//! a P4 concern).

pub mod manifest;
pub mod readiness;
pub mod runtime;
