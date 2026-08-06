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
pub fn auth_file() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".future")
        .join("agent")
        .join("auth.json")
}
