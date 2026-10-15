//! `future-goal` — binary entry for the FutureOS goal control plane.
//!
//! All logic lives in `future_goal::console`, shared with the embedded
//! `future goal` CLI command.

use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    future_goal::console::run("future-goal", args)
}
