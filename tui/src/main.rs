//! `future-tui` — Rust port of the TypeScript TUI entry point.
//!
//! All argument parsing / print mode / list-models / interactive wiring lives
//! in `index.rs` (a 1:1 port of `tui/src/index.ts`); this binary just collects
//! `argv` and forwards it.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    future_tui::index::run(&args)
}
