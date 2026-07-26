//! Signed in-place application updates through Tauri's updater plugin.
//!
//! Formal builds embed the CDN `latest.json` endpoint and the updater public
//! key through a per-build Tauri config overlay. The manifest may also contain
//! the custom top-level `assets` map used by the website; Tauri ignores those
//! additional fields and selects only the current entry under `platforms`.

use serde::Serialize;
use serde_json::Value;
use tauri::Emitter;
use tauri_plugin_updater::UpdaterExt;

use crate::{agent_supervisor, build_info, AppError};

const PROGRESS_EVENT: &str = "app-update-progress";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    pub platform_supported: bool,
    /// Website installer URL for builds that cannot use the in-place updater.
    pub download_url: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    downloaded: u64,
    /// 0 when the server did not send a Content-Length.
    total: u64,
}

fn updater_error(context: &str, error: impl std::fmt::Display) -> AppError {
    AppError::Message(format!("{context}: {error}"))
}

/// Return the website installer URL from the custom `assets` manifest field.
///
/// Tauri consumes `platforms` for its updater archive, while `assets` points
/// to the normal DMG/EXE users should download when automatic installation is
/// unavailable (for example from a local build).
fn manual_download_url_for_asset(manifest: &Value, asset_key: &str) -> Option<String> {
    let url = manifest
        .get("assets")?
        .get(asset_key)?
        .get("url")?
        .as_str()?;

    url.starts_with("https://").then(|| url.to_owned())
}

fn manual_download_url(manifest: &Value) -> Option<String> {
    let asset_key = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "darwin-aarch64"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "darwin-x86_64"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "windows-x86_64"
    } else {
        return None;
    };

    manual_download_url_for_asset(manifest, asset_key)
}

/// Check the signed static manifest configured in `tauri.conf.json`.
#[tauri::command]
pub async fn check_app_update(app: tauri::AppHandle) -> Result<UpdateStatus, AppError> {
    let current_version = build_info::VERSION.to_string();

    // Linux is intentionally absent from formal releases. Avoid asking the
    // plugin to resolve a target that latest.json deliberately does not carry.
    if !cfg!(any(target_os = "macos", target_os = "windows")) {
        return Ok(UpdateStatus {
            latest_version: current_version.clone(),
            current_version,
            has_update: false,
            platform_supported: false,
            download_url: None,
        });
    }

    let updater = app
        .updater()
        .map_err(|error| updater_error("Failed to initialize the updater", error))?;
    let update = updater
        .check()
        .await
        .map_err(|error| updater_error("Failed to check for updates", error))?;

    Ok(match update {
        Some(update) => {
            let download_url = manual_download_url(&update.raw_json);
            UpdateStatus {
                current_version,
                latest_version: update.version,
                has_update: true,
                // Only formal signed builds embed the updater public key. Local and
                // daily builds may inspect the public manifest, but must not offer
                // installation without signature verification.
                platform_supported: build_info::is_release(),
                download_url,
            }
        }
        None => UpdateStatus {
            latest_version: current_version.clone(),
            current_version,
            has_update: false,
            platform_supported: true,
            download_url: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::manual_download_url_for_asset;

    #[test]
    fn reads_the_matching_website_asset_url() {
        let manifest = json!({
            "assets": {
                "darwin-aarch64": {
                    "url": "https://downloads.example.com/FutureOS_1.0.4_aarch64-sign.dmg"
                }
            }
        });

        assert_eq!(
            manual_download_url_for_asset(&manifest, "darwin-aarch64"),
            Some("https://downloads.example.com/FutureOS_1.0.4_aarch64-sign.dmg".to_string())
        );
    }

    #[test]
    fn rejects_non_https_website_asset_urls() {
        let manifest = json!({
            "assets": {
                "windows-x86_64": { "url": "http://downloads.example.com/FutureOS.exe" }
            }
        });

        assert_eq!(
            manual_download_url_for_asset(&manifest, "windows-x86_64"),
            None
        );
    }
}

/// Download, verify and install the platform updater package.
///
/// Tauri verifies the mandatory minisign signature before installation. The
/// SHA-256 values in latest.json remain useful to website consumers and release
/// audits, but are not a substitute for this signature verification.
#[tauri::command]
pub async fn install_app_update(app: tauri::AppHandle) -> Result<(), AppError> {
    if !build_info::is_release() {
        return Err(AppError::Message(
            "Automatic installation is only available in signed release builds.".to_string(),
        ));
    }

    let updater = app
        .updater()
        .map_err(|error| updater_error("Failed to initialize the updater", error))?;
    let update = updater
        .check()
        .await
        .map_err(|error| updater_error("Failed to check for updates", error))?
        .ok_or_else(|| AppError::Message("No update is currently available.".to_string()))?;

    let progress_app = app.clone();
    let mut downloaded = 0_u64;
    update
        .download_and_install(
            move |chunk_length, content_length| {
                downloaded = downloaded.saturating_add(chunk_length as u64);
                let _ = progress_app.emit(
                    PROGRESS_EVENT,
                    DownloadProgress {
                        downloaded,
                        total: content_length.unwrap_or(0),
                    },
                );
            },
            || {},
        )
        .await
        .map_err(|error| updater_error("Failed to install the update", error))
}

/// Relaunch only after installation has completed and the user explicitly asks
/// to do so. Keeping this separate lets an active conversation finish first.
#[tauri::command]
pub fn restart_after_app_update(app: tauri::AppHandle) -> Result<(), AppError> {
    agent_supervisor::shutdown_agent();
    app.restart()
}
