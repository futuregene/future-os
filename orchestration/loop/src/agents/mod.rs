//! agents subdomain (G-16/G12) — single-process multi-agent reservation:
//! identity-scoped frontiers (scope), lane recommendations (lane), the
//! supervisor proposal/receipt event surface (supervisor), the workspace
//! guard against shared-workspace write conflicts
//! (workspace_guard), and the multi-agent contract/recipe/succession/
//! wake-roster/collective-turn-ledger surface (multi_agent). Cross-process
//! A2A stays a contract-schema concern.

pub mod lane;
pub mod multi_agent;
pub mod scope;
pub mod supervisor;
pub mod workspace_guard;
