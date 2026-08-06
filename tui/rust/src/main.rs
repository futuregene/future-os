//! `future-tui` — Rust port of the TypeScript TUI entry point.
//!
//! P0 scaffold: `--version`/`-v` and `--help`/`-h` are wired to match
//! `tui/src/index.ts` byte-for-byte (including the ordering quirk where
//! `--help` exits during argument scanning, before `--version` is checked).
//! Interactive mode is stubbed until the App port lands (P2+).

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut version = false;
    for arg in &args {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{}", future_tui::help::help_text());
                return ExitCode::SUCCESS;
            }
            "--version" | "-v" => version = true,
            _ => {
                if arg.starts_with('-') {
                    eprintln!("Unknown option: {arg}");
                    return ExitCode::from(1);
                }
                // Messages / @file args: only meaningful in interactive or
                // print mode, which land in the P0 stub below.
            }
        }
    }

    if version {
        println!("future-tui v{}", future_tui::version::VERSION);
        return ExitCode::SUCCESS;
    }

    eprintln!(
        "future-tui: interactive TUI not yet implemented (P0 scaffold; \
         the App port lands in a later phase)"
    );
    ExitCode::from(1)
}
