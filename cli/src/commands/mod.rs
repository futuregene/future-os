//! Command implementations — one module per CLI group.
//!
//! P0 status: the dispatch predicates (`is_*_command`) and signatures are
//! final (they are part of the argument-parsing surface ported 1:1 from
//! index.ts); command bodies are stubs filled in by P1 (local commands) and
//! P2 (remote/gRPC commands).

pub mod account;
pub mod auth;
pub mod browser_tools;
pub mod doctor;
pub mod init;
pub mod mcp;
pub mod models;
pub mod run;
pub mod session;
pub mod skills;
pub mod tools;
