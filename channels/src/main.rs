//! FutureAgent Channel Bridge — unified binary for all channels.
//!
//! Reads ~/.future/channels/config.json and starts enabled channels.
//! Each channel connects to the FutureAgent via gRPC. All logic lives in
//! `future_channel::run`, shared with the embedded `future channel` CLI entry.

use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    future_channel::run(&args)
}
