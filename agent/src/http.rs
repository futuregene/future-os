//! HTTP clients with the process TLS provider initialized before construction.
//!
//! The implementation lives in `future-http`, because the provider choice is
//! process-wide and the TUI's update check needs it without linking the Agent.
//! Re-exported here so every existing `future_agent::http::…` caller (the
//! application host, the CLI, the Agent's own modules) keeps its entry point.

pub use future_http::*;
