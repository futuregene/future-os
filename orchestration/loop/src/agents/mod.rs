//! agents subdomain (G-16/G-24) — single-process multi-agent reservation:
//! identity-scoped frontiers (scope), lane recommendations (lane), the
//! supervisor proposal/receipt event surface (supervisor), and the
//! capability gate binding agent capabilities to todo runnability
//! (capability_gate). Cross-process A2A stays a contract-schema concern.

pub mod capability_gate;
pub mod lane;
pub mod scope;
pub mod supervisor;
