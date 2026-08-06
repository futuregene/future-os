//! `future` — Rust port of the TypeScript CLI.
//!
//! Thin wrapper: collect argv, run the dispatch (cli/src/index.ts port), flush,
//! and exit with the same code the TS CLI would. Returning `ExitCode` (rather
//! than calling `process::exit`) lets the standard runtime flush stdio first.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = cli_rust::Output::stdio();
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("failed to start runtime: {err}");
            return ExitCode::from(1);
        }
    };
    let code = runtime.block_on(cli_rust::dispatch(&args, &out));
    out.flush();
    ExitCode::from(code as u8)
}
