//! `future-loop` — binary entry for the FutureOS loop control plane.
//!
//! All logic lives in `future_loop::console`, shared with the embedded
//! `future loop` CLI command.

use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    future_loop::console::run("future-loop", args)
}
