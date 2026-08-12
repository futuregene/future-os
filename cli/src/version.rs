//! Build-time version for `future --version` — injected by build.rs, mirroring
//! scripts/version.mjs so the Rust CLI prints exactly what the TS CLI prints.

/// Display version string (e.g. `0.0.2-479c8fee+local`).
pub const VERSION: &str = env!("FUTURE_CLI_VERSION");
