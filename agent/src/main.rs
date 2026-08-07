//! `future-agent` — Rust agent backend (gRPC server entry point).
//!
//! All logic lives in `future_agent::cli` so the same code runs either as the
//! standalone binary or embedded in the `future` CLI (`future agent <args>`).

use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    future_agent::cli::run_from_args(&args)
}
