//! Replay & model-behavior corpus (G-19) — the LLM-side extension of the
//! contract tests.
//!
//! - `decision_replay`: record a real kernel decision as a public-safe reduced
//!   case; replay the kernel against the reconstructed state; diff. A behavior
//!   regression canary that fails loudly when a kernel change alters a
//!   recorded decision.
//! - `corpus`: model-behavior corpus (state matrix / retained decisions /
//!   counterfactuals / candidate ablations) run through a model-behavior
//!   actor; equivalent or fail-closed per case. Deterministic under the stub
//!   actor; an LLM actor plugs into the same signal schema.

pub mod corpus;
pub mod decision_replay;
