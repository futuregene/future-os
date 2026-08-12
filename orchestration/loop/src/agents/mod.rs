//! agents subdomain (G-16/G-24) — single-process multi-agent reservation:
//! identity-scoped frontiers (scope), lane recommendations (lane), the
//! supervisor proposal/receipt event surface (supervisor), the capability
//! gate binding agent capabilities to todo runnability (capability_gate),
//! and the workspace guard against shared-workspace write conflicts
//! (workspace_guard). Cross-process A2A stays a contract-schema concern.

pub mod capability_gate;
pub mod lane;
pub mod scope;
pub mod supervisor;
pub mod workspace_guard;
