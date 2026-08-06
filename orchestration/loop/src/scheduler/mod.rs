//! Scheduler subdomain (G-10) — the rrule recurrence state machine.
//!
//! reference `control_plane/scheduler/` (4,178 lines across 15 files) keeps a
//! persistent scheduler state per (goal, agent): rrule recurrence,
//! progression backoff, reset tokens, identity signatures, and host update
//! failures. This module implements the minimal state machine (G-10) as a
//! pure, deterministic library — `state.rs` holds the record, normalization,
//! progression, and atomic persistence. The decision kernel stays pure; the
//! CLI layer (`loopx scheduler`) drives persistence across decision cycles.

pub mod state;
