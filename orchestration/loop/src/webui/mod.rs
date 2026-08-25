//! `future loop ui` — local read-mostly web dashboard for the loop control
//! plane. Zero-dependency HTTP server (raw tokio TCP, loopback only) with an
//! embedded single-page UI; state is projected live from the event ledger
//! via [`crate::store::Store::replay`] on every request, so the dashboard
//! never drifts from CLI state.
//!
//! Surface:
//!   GET  /                              embedded dashboard (static)
//!   GET  /api/overview                  registry + per-goal cards + attention + 24h/7d totals
//!   GET  /api/goals/{id}                full goal detail projection
//!   GET  /api/goals/{id}/runs?limit=N   run ledger (newest first)
//!   GET  /api/goals/{id}/events?limit=N raw event ledger (newest first)
//!   GET  /api/stream                    SSE: `overview` + `goals` pushed on change / interval
//!   POST /api/goals/{id}/gate           {"todo_id","decision","note"?} — resolve a user gate
//!   POST /api/goals/{id}/lifecycle      {"action":"cancel"|"resume","reason"?}

mod api;
mod page;
mod server;

pub use server::run_server;
