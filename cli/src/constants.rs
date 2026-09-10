//! Constants — verbatim port of `cli/src/constants.ts`.
//!
//! `AUTH_FILE` is a function (not a const) because it depends on the home
//! directory, which Rust resolves at call time; behavior is equivalent to the
//! TS module evaluating `join(homedir(), ...)` once at import time.

use std::path::PathBuf;

pub const DEFAULT_PLATFORM_URL: &str = "https://future-os.cn";
pub const FUTURE_AUTH_PROVIDER: &str = "future";

pub const DEFAULT_LAUNCHD_LABEL: &str = "com.future.agent";
pub const DEFAULT_SYSTEMD_UNIT: &str = "future-agent.service";
pub const DEFAULT_WINDOWS_SERVICE: &str = "FutureAgent";
pub const DEFAULT_AGENT_GRPC_ADDR: &str = "127.0.0.1:50051";

pub const DEFAULT_CHANNEL_LAUNCHD_LABEL: &str = "com.future.channel";
pub const DEFAULT_CHANNEL_SYSTEMD_UNIT: &str = "future-channel.service";
pub const DEFAULT_CHANNEL_WINDOWS_SERVICE: &str = "FutureChannel";

/// `~/.future/agent/auth.json`
///
/// Resolved through the agent's `~/.future/agent` root rather than raw
/// `dirs::home_dir()`: on Windows the latter reads the token profile and ignores
/// a redirected `HOME`, so the CLI wrote to a different home than the agent
/// reads (and isolated test runs wrote the developer's real auth.json).
pub fn auth_file() -> PathBuf {
    future_agent::utils::default_config_dir().join("auth.json")
}
