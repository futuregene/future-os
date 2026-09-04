//! agents subdomain (G-16) — single-process multi-worker surface:
//! identity-scoped frontiers (scope), lane recommendations (lane), the
//! supervisor event projection (supervisor), and the workspace guard
//! against shared-workspace write conflicts (workspace_guard).

pub mod lane;
pub mod scope;
pub mod supervisor;
pub mod workspace_guard;
