//! App self-update: check the OSS release manifest and download the
//! platform-appropriate installer.
//!
//! The manifest (`releases/latest.json`) carries only `{ version, pub_date }`.
//! Installer download URLs are derived deterministically from the version and
//! the running platform — the release layout is flat:
//!   `releases/<version>/FutureOS_<version>_<arch>.<ext>`
//! (see `.github/workflows/build.yml`, "Stage publishable installers").
//!
//! Dev builds (version `0.0.0-dev.<hash>`) always report an available update —
//! any real release outranks `0.0.0`. That is expected, not a bug.

use std::io::Write;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::{build_info, AppError};

const MANIFEST_URL: &str = "https://futureos.oss-cn-hangzhou.aliyuncs.com/releases/latest.json";
const RELEASE_BASE: &str = "https://futureos.oss-cn-hangzhou.aliyuncs.com/releases";

/// Event name for streaming download progress to the frontend.
const PROGRESS_EVENT: &str = "app-update-progress";

#[derive(Deserialize)]
struct Manifest {
    version: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    /// Whether this platform has a downloadable installer in the release layout.
    pub platform_supported: bool,
    pub download_url: Option<String>,
    pub file_name: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    downloaded: u64,
    /// 0 when the server didn't send a Content-Length.
    total: u64,
}

/// The installer file name for the current OS, or `None` on a platform we don't
/// ship an installer for. Each platform ships a single arch, so the arch token
/// is fixed (see the build matrix).
fn platform_installer(version: &str) -> Option<String> {
    match std::env::consts::OS {
        "macos" => Some(format!("FutureOS_{version}_aarch64.dmg")),
        "windows" => Some(format!("FutureOS_{version}_x64-setup.exe")),
        "linux" => Some(format!("FutureOS_{version}_amd64.deb")),
        _ => None,
    }
}

/// (major, minor, patch, is_prerelease) from a version string. Missing or
/// non-numeric components read as 0 so a malformed manifest never panics.
fn parse_version(v: &str) -> (u64, u64, u64, bool) {
    let is_prerelease = v.contains('-');
    let core = v.split('-').next().unwrap_or(v);
    let mut parts = core.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        is_prerelease,
    )
}

/// Is `latest` newer than `current`? Compares the numeric core; on an equal
/// core a release outranks a prerelease/dev of that core. Dev builds carry a
/// `0.0.0` core, so any real release is "newer" — dev always sees an update.
fn is_newer(latest: &str, current: &str) -> bool {
    let (lm, ln, lp, l_pre) = parse_version(latest);
    let (cm, cn, cp, c_pre) = parse_version(current);
    if (lm, ln, lp) != (cm, cn, cp) {
        return (lm, ln, lp) > (cm, cn, cp);
    }
    // Same core: a plain release beats a prerelease/dev build.
    !l_pre && c_pre
}

fn http_client(timeout: Duration) -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|error| AppError::Message(format!("Failed to create HTTP client: {error}")))
}

/// Fetch the release manifest and report whether an update is available for the
/// current platform.
#[tauri::command]
pub async fn check_app_update() -> Result<UpdateStatus, AppError> {
    let client = http_client(Duration::from_secs(15))?;
    let response = client
        .get(MANIFEST_URL)
        .send()
        .await
        .map_err(|error| AppError::Message(format!("Failed to check for updates: {error}")))?;
    if !response.status().is_success() {
        return Err(AppError::Message(format!(
            "Update check failed: server returned {}",
            response.status()
        )));
    }
    let manifest: Manifest = response
        .json()
        .await
        .map_err(|error| AppError::Message(format!("Failed to parse version info: {error}")))?;

    let current_version = build_info::VERSION.to_string();
    let has_update = is_newer(&manifest.version, &current_version);

    let (download_url, file_name, platform_supported) = match platform_installer(&manifest.version)
    {
        Some(name) => (
            Some(format!("{RELEASE_BASE}/{}/{name}", manifest.version)),
            Some(name),
            true,
        ),
        None => (None, None, false),
    };

    Ok(UpdateStatus {
        current_version,
        latest_version: manifest.version,
        has_update,
        platform_supported,
        download_url,
        file_name,
    })
}

/// Stream the installer to the user's Downloads directory, emitting
/// `app-update-progress` events, and return the saved path.
#[tauri::command]
pub async fn download_app_update(
    app: tauri::AppHandle,
    url: String,
    file_name: String,
) -> Result<String, AppError> {
    // Reject anything not on our release host — this URL is passed from the
    // frontend, so pin the origin rather than trust it blindly.
    if !url.starts_with(RELEASE_BASE) {
        return Err(AppError::Message(
            "Download URL is not from an allowed release source.".to_string(),
        ));
    }
    // Guard against a crafted file_name escaping the Downloads directory.
    if file_name.is_empty() || file_name.contains('/') || file_name.contains('\\') {
        return Err(AppError::Message("Illegal filename.".to_string()));
    }

    let dir = app.path().download_dir().map_err(|error| {
        AppError::Message(format!("Failed to locate download directory: {error}"))
    })?;
    let dest = dir.join(&file_name);

    // No timeout: installers are large and a slow link shouldn't abort mid-file.
    let client = reqwest::Client::builder()
        .build()
        .map_err(|error| AppError::Message(format!("Failed to create HTTP client: {error}")))?;
    let mut response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| AppError::Message(format!("Download failed: {error}")))?;
    if !response.status().is_success() {
        return Err(AppError::Message(format!(
            "Download failed: server returned {}",
            response.status()
        )));
    }

    let total = response.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(&dest)?;
    let mut downloaded: u64 = 0;
    let mut last_emit: u64 = 0;

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| AppError::Message(format!("Download interrupted: {error}")))?
    {
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        // Throttle to ~1 MiB steps (plus a final tick) to avoid flooding the UI.
        if downloaded - last_emit >= 1_048_576 || (total > 0 && downloaded >= total) {
            last_emit = downloaded;
            let _ = app.emit(PROGRESS_EVENT, DownloadProgress { downloaded, total });
        }
    }
    file.flush()?;

    Ok(dest.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_outranks_dev_and_older() {
        assert!(is_newer("0.0.1", "0.0.0-dev.abc")); // dev always updates
        assert!(is_newer("1.2.0", "1.1.9"));
        assert!(is_newer("1.0.0", "1.0.0-dev.abc")); // release beats same-core dev
        assert!(!is_newer("1.0.0", "1.0.0")); // equal, both release
        assert!(!is_newer("1.0.0", "1.2.0")); // older isn't newer
        assert!(!is_newer("1.0.0-dev.x", "1.0.0")); // a dev never beats its release
    }
}
