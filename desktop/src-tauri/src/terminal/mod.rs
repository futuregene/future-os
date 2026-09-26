//! Embedded terminal — a PTY session registry owned by the desktop main
//! process and served to the app's own webview over a loopback-only
//! HTTP/WebSocket listener.
//!
//! Architecture (see `docs/internals/desktop/embedded-terminal.md`): this is the
//! opencode transport model, not a Tauri-IPC message pump.
//!
//! * `session`/`manager` own the PTY children and a bounded output buffer with
//!   an absolute byte cursor; a client that (re)attaches asks for the bytes
//!   after a cursor it already applied.
//! * `server` exposes control routes plus one WebSocket per attached view on
//!   `127.0.0.1:<ephemeral port>`, authenticated by a per-process secret and a
//!   one-time connect ticket (`ticket`).
//! * `pty` is the portable-pty boundary (spawn, resize, write, process-tree
//!   teardown); `shell` and `cwd` resolve what to run and where.
//!
//! Terminal output never flows through the agent, the RPC bridge, the remote
//! control plane, logs or SQLite: it lives in the session buffer and the
//! WebSocket connection, and the renderer keeps its own screen state.

pub mod cwd;
pub mod manager;
pub mod protocol;
pub mod pty;
pub mod server;
pub mod session;
pub mod shell;
pub mod ticket;

#[cfg(test)]
pub mod test_support;

pub use server::ServerInfo;
