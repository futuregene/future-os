//! Embedded terminal, Desktop half: the loopback transport the app's own
//! webview talks to.
//!
//! The registry itself lives in `future_host::terminal` — a paired desktop
//! drives the same shells, and `future host` serves them with no GUI at all
//! (`docs/internals/desktop/embedded-terminal.md`). What stays here is the
//! transport that only exists where there is a webview: a loopback-only HTTP
//! and WebSocket listener, authenticated by a per-process secret and a
//! one-time connect ticket (`server`, `ticket`), which speaks to the
//! process-wide registry.
//!
//! Terminal output never flows through the agent, the RPC bridge, logs or
//! SQLite: it lives in the session buffer and the WebSocket connection, and the
//! renderer keeps its own screen state.

pub mod server;
pub mod ticket;

pub use server::ServerInfo;
