//! Utility functions — matching Go internal/utils

use std::path::{Path, PathBuf};

/// GenerateID creates a unique session ID with timestamp and random hex.
/// Format: "20260508-090513-a1b2c3" (time-6randomhex for uniqueness)
pub fn generate_id() -> String {
    use rand::RngCore;
    let now = chrono::Local::now();
    let ts = now.format("%Y%m%d-%H%M%S").to_string();
    let mut rng = rand::thread_rng();
    let mut buf = [0u8; 3];
    rng.fill_bytes(&mut buf);
    let hex: String = buf.iter().map(|b| format!("{:02x}", b)).collect();
    format!("{}-{}", ts, hex)
}

/// GenerateEntryID creates a time-sortable entry ID.
/// Format: "20260508-090513-a1b2c3" (date-time-6randomhex)
pub fn generate_entry_id() -> String {
    use rand::RngCore;
    let now = chrono::Local::now();
    let ts = now.format("%Y%m%d-%H%M%S").to_string();
    let mut rng = rand::thread_rng();
    let mut buf = [0u8; 3];
    rng.fill_bytes(&mut buf);
    let hex: String = buf.iter().map(|b| format!("{:02x}", b)).collect();
    format!("{}-{}", ts, hex)
}

/// encode_cwd converts a filesystem path into a safe directory name using base32.
/// Matches Go: `base32.StdEncoding.WithPadding(base32.NoPadding).EncodeToString([]byte(s))`
pub fn encode_cwd(cwd: &str) -> String {
    let s = cwd.strip_prefix('/').unwrap_or(cwd);
    let s = if s.is_empty() || s == "." { "root" } else { s };
    let encoded = base32::encode(base32::Alphabet::Rfc4648 { padding: false }, s.as_bytes());
    // Remove padding (Go uses NoPadding)
    encoded.trim_end_matches('=').to_lowercase()
}

/// Detect image MIME type from file extension
pub fn detect_image_mime_type_from_extension(path: &Path) -> Option<String> {
    match path.extension()?.to_str()?.to_lowercase().as_str() {
        "png" => Some("image/png".to_string()),
        "jpg" | "jpeg" => Some("image/jpeg".to_string()),
        "gif" => Some("image/gif".to_string()),
        "webp" => Some("image/webp".to_string()),
        "svg" => Some("image/svg+xml".to_string()),
        "bmp" => Some("image/bmp".to_string()),
        _ => None,
    }
}

/// Detect image MIME type by reading file header magic bytes
pub fn detect_image_mime_type(path: &Path) -> Option<String> {
    use std::fs::File;
    use std::io::Read;
    let mut file = File::open(path).ok()?;
    let mut header = [0u8; 12];
    file.read_exact(&mut header).ok()?;
    match &header[..4] {
        [0x89, 0x50, 0x4E, 0x47] => Some("image/png".to_string()),
        [0xFF, 0xD8, 0xFF, _] => Some("image/jpeg".to_string()),
        [0x47, 0x49, 0x46, _] => Some("image/gif".to_string()),
        [0x52, 0x49, 0x46, 0x46] if &header[8..12] == b"WEBP" => Some("image/webp".to_string()),
        _ => None,
    }
}

/// Version string — injected at build time via build.rs from FUTURE_VERSION
/// (see scripts/version.mjs). Release builds are a plain `X.Y.Z`; dev builds
/// carry a `-dev.<hash>` suffix.
pub const VERSION: &str = env!("FUTURE_VERSION");

/// Default base session directory (contains per-cwd subdirectories)
pub fn default_session_dir(_cwd: &str) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".future/agent").join("sessions")
}

/// Default config directory
pub fn default_config_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".future/agent")
}

/// Get default settings paths (global and project-level)
pub fn default_settings_paths() -> (PathBuf, PathBuf) {
    let home = default_config_dir();
    (
        home.join("settings.json"),
        PathBuf::from(".future/agent/settings.json"),
    )
}

/// Canonical path (resolve symlinks, absolute)
pub fn canonical_path(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

/// Detect if running in a terminal
pub fn is_tty() -> bool {
    atty::is(atty::Stream::Stdin)
}

/// ANSI color codes (matching Go constants)
pub mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const RED: &str = "\x1b[31m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const BLUE: &str = "\x1b[34m";
    pub const MAGENTA: &str = "\x1b[35m";
}
