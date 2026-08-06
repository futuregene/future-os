//! future_tui — Rust port of the TypeScript future-tui.
//!
//! Goal: 1:1 behavior — UI rendering, key handling, interaction, argument
//! parsing, and help text — with a self-implemented terminal backend
//! (`terminal.rs`, libc on POSIX / windows-sys planned for Windows), replacing
//! Node's `process.stdin`/`process.stdout` machinery.
//!
//! P0 modules (pure logic, ported 1:1 from `tui/src/*.ts`):
//!   - `stdin_buffer` — escape-sequence buffering / bracketed paste
//!   - `keys`         — raw bytes → normalized key ids (Kitty CSI-u,
//!     modifyOtherKeys, legacy)
//!   - `theme`        — color constants, style helpers
//!   - `utils`        — grapheme width, ANSI tracking, wrap/slice/truncate
//!   - `terminal`     — self-implemented POSIX terminal backend
//!   - `help`         — verbatim `--help` text (index.ts `printHelp`)
//!   - `version`      — build-injected version (scripts/version.mjs)
//!
//! P1 modules (components layer, ported 1:1 from `tui/src/components/*.ts` +
//! `tui/src/tui.ts` + `tui/src/help-screen.ts` + `tui/src/keybindings.ts`):
//!   - `tui`              — Component architecture, constants, overlay layout
//!   - `components`       — input / autocomplete / select-list /
//!     scoped-models-selector / footer
//!   - `help_screen`      — `renderHelp` card
//!   - `keybindings`      — KeybindingManager
//!   - `rpc`              — RPC types (ModelInfo for the selector)

pub mod components;
pub mod help;
pub mod help_screen;
pub mod keybindings;
pub mod keys;
pub mod rpc;
pub mod stdin_buffer;
pub mod terminal;
pub mod terminal_image;
pub mod theme;
pub mod tui;
pub mod utils;
pub mod version;

/// Shared lock for tests that mutate process-global state (env vars), so
/// parallel `cargo test` threads cannot race each other. Same pattern as
/// `cli/rust/src/test_env.rs`.
#[cfg(test)]
pub mod test_env {
    use std::sync::Mutex;

    pub static ENV_LOCK: Mutex<()> = Mutex::new(());
}
