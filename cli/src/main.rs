//! `future` — Rust port of the TypeScript CLI, plus the unified entry point
//! for the other Rust components.
//!
//! `future <group> <command>` keeps the CLI's own groups (auth, run, skills,
//! tools, models, session, doctor, ...). `future agent|tui|channel|loop
//! <args>` runs the corresponding component in-process — the same code as the
//! standalone `future-agent` / `future-tui` / `future-channel` /
//! `future-loop` binaries, which remain installed and usable directly.
//!
//! The embedded components build their OWN tokio runtimes, so they must be
//! dispatched from main() BEFORE the CLI's runtime is entered (nested
//! runtimes would panic). Everything else runs inside `dispatch` as before.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Embedded components (each builds its own tokio runtime) — dispatch
    // before the CLI runtime starts.
    if let Some(group) = args.first().map(String::as_str) {
        match group {
            "agent" => return run_agent(&args[1..]),
            "tui" => return run_tui(&args[1..]),
            "channel" | "channels" => return run_channel(&args[1..]),
            "loop" => return run_loop(&args[1..]),
            _ => {}
        }
    }

    let out = future_cli::Output::stdio();
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("failed to start runtime: {err}");
            return ExitCode::from(1);
        }
    };
    let code = runtime.block_on(future_cli::dispatch(&args, &out));
    out.flush();
    ExitCode::from(code as u8)
}

/// `future agent <args>` — run the agent gRPC server (same as `future-agent`).
fn run_agent(args: &[String]) -> ExitCode {
    match future_agent::cli::run_from_args(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::from(1)
        }
    }
}

/// `future tui <args>` — launch the terminal UI (same as `future-tui`).
fn run_tui(args: &[String]) -> ExitCode {
    future_tui::index::run(args)
}

/// `future channel <args>` — start the IM channel bridge (same as
/// `future-channel`).
fn run_channel(args: &[String]) -> ExitCode {
    match future_channel::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::from(1)
        }
    }
}

/// `future loop <args>` — loop control plane (same as `future-loop`).
fn run_loop(args: &[String]) -> ExitCode {
    match future_loop::console::run("future loop", args.to_vec()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::from(1)
        }
    }
}
