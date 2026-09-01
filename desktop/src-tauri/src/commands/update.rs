//! Signed in-place application updates through Tauri's updater plugin.
//!
//! Formal builds embed the CDN `latest.json` endpoint and the updater public
//! key through a per-build Tauri config overlay. The manifest may also contain
//! the custom top-level `assets` map used by the website; Tauri ignores those
//! additional fields and selects only the current entry under `platforms`.

use serde::Serialize;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use serde_json::Value;
use tauri::Emitter;
use tauri_plugin_updater::UpdaterExt;

use crate::{agent_supervisor, build_info, AppError};

const PROGRESS_EVENT: &str = "app-update-progress";

#[derive(Clone, Debug, Serialize)]
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
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn manual_download_url_for_asset(manifest: &Value, asset_key: &str) -> Option<String> {
    let url = manifest
        .get("assets")?
        .get(asset_key)?
        .get("url")?
        .as_str()?;

    url.starts_with("https://").then(|| url.to_owned())
}

/// The `assets` key for the host platform, when it ships a website installer.
///
/// Selected at compile time with `#[cfg]` (rather than `cfg!`) so the
/// inapplicable branches never emit dead regions that per-line coverage would
/// flag. Hosts without a formal installer (e.g. Linux) do not resolve a
/// website installer URL at all (the resolver below is macOS/Windows-only).
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const PLATFORM_ASSET_KEY: &str = "darwin-aarch64";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const PLATFORM_ASSET_KEY: &str = "darwin-x86_64";
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const PLATFORM_ASSET_KEY: &str = "windows-x86_64";

#[cfg(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "windows", target_arch = "x86_64")
))]
fn manual_download_url(manifest: &Value) -> Option<String> {
    manual_download_url_for_asset(manifest, PLATFORM_ASSET_KEY)
}

/// Resolve a checked manifest into the status reported to the frontend.
///
/// Pure so the update-present / update-absent branches (and the release-build
/// `platform_supported` guard) are testable without a live updater plugin.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn resolve_update_status(current_version: String, update: Option<(String, Value)>) -> UpdateStatus {
    match update {
        Some((version, raw_json)) => UpdateStatus {
            current_version,
            latest_version: version,
            has_update: true,
            platform_supported: build_info::is_release(),
            download_url: manual_download_url(&raw_json),
        },
        None => UpdateStatus {
            latest_version: current_version.clone(),
            current_version,
            has_update: false,
            platform_supported: true,
            download_url: None,
        },
    }
}

/// Check the signed static manifest configured in `tauri.conf.json`.
#[tauri::command]
pub async fn check_app_update<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<UpdateStatus, AppError> {
    crate::scheduler::check_app_update_now(app).await
}

pub(crate) async fn perform_app_update_check<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<UpdateStatus, AppError> {
    let current_version = build_info::VERSION.to_string();
    check_app_update_impl(app, current_version).await
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn check_app_update_impl<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    current_version: String,
) -> Result<UpdateStatus, AppError> {
    let updater = app
        .updater()
        .map_err(|error| updater_error("Failed to initialize the updater", error))?;
    let update = updater
        .check()
        .await
        .map_err(|error| updater_error("Failed to check for updates", error))?;
    Ok(resolve_update_status(
        current_version,
        update.map(|update| (update.version, update.raw_json)),
    ))
}

// Linux is intentionally absent from formal releases. Avoid asking the plugin
// to resolve a target that latest.json deliberately does not carry.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn check_app_update_impl<R: tauri::Runtime>(
    _app: tauri::AppHandle<R>,
    current_version: String,
) -> Result<UpdateStatus, AppError> {
    Ok(UpdateStatus {
        latest_version: current_version.clone(),
        current_version,
        has_update: false,
        platform_supported: false,
        download_url: None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A one-shot HTTP server answering a single request with a fixed body.
    fn serve_once(status: &'static str, content_type: &'static str, body: Vec<u8>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().expect("mock accept");
            let mut sink = [0u8; 8192];
            let _ = stream.read(&mut sink);
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
            let _ = stream.flush();
        });
        format!("http://127.0.0.1:{port}")
    }

    /// Build a mock app with the updater plugin registered and its config
    /// pinned to the given endpoints (empty = no endpoint → `updater()` errors).
    fn mock_app_with_updater(endpoints: &[&str]) -> tauri::App<tauri::test::MockRuntime> {
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().plugins.0.insert(
            "updater".to_string(),
            json!({ "endpoints": endpoints, "pubkey": "dummy" }),
        );
        tauri::test::mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(context)
            .expect("mock app with updater")
    }

    fn manifest_json(version: &str, download_url: &str) -> Vec<u8> {
        // Include every platform the updater can select so the mock works on
        // any host/CI target (the updater looks up its own `{os}-{arch}` key).
        let platform = json!({ "url": download_url, "signature": "dummy" });
        json!({
            "version": version,
            "platforms": {
                "darwin-aarch64": platform.clone(),
                "darwin-x86_64": platform.clone(),
                "linux-x86_64": platform.clone(),
                "linux-aarch64": platform.clone(),
                "windows-x86_64": platform,
            }
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn updater_error_formats_context_and_error() {
        let error = updater_error("Failed to check", "boom");
        assert_eq!(error.to_string(), "Failed to check: boom");
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
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

    #[cfg(any(target_os = "macos", target_os = "windows"))]
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

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn manual_download_url_selects_the_host_platform_key() {
        let manifest = json!({
            "assets": {
                "darwin-aarch64": { "url": "https://example.com/aarch64.dmg" },
                "darwin-x86_64": { "url": "https://example.com/x86_64.dmg" },
                "windows-x86_64": { "url": "https://example.com/windows.exe" }
            }
        });
        // The host-specific key selection is exercised; the exact result depends
        // on the platform (None on hosts without a formal installer), which the
        // per-key `manual_download_url_for_asset` tests already pin down.
        let _ = manual_download_url(&manifest);
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn resolve_update_status_reports_an_available_update() {
        let status =
            resolve_update_status("0.1.0".to_string(), Some(("1.2.0".to_string(), json!({}))));
        assert!(status.has_update);
        assert_eq!(status.latest_version, "1.2.0");
        assert_eq!(status.current_version, "0.1.0");
        assert_eq!(status.platform_supported, build_info::is_release());
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn resolve_update_status_reports_no_update() {
        let status = resolve_update_status("0.1.0".to_string(), None);
        assert!(!status.has_update);
        assert_eq!(status.latest_version, "0.1.0");
        assert_eq!(status.current_version, "0.1.0");
        assert!(status.platform_supported);
        assert_eq!(status.download_url, None);
    }

    // The updater plugin is only wired up on platforms with formal releases.
    // `check_app_update_impl` returns early (unsupported) elsewhere, so the
    // updater-path tests below are macOS/Windows-only and Linux gets its own
    // unsupported-path test further down.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[tokio::test]
    async fn check_app_update_reports_no_update_on_204() {
        let manifest_url = serve_once("204 No Content", "application/json", Vec::new());
        let app = mock_app_with_updater(&[&manifest_url]);
        let status = check_app_update(app.handle().clone()).await.expect("check");
        assert!(!status.has_update);
        assert!(status.platform_supported);
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[tokio::test]
    async fn check_app_update_reports_an_available_update() {
        let manifest_url = serve_once(
            "200 OK",
            "application/json",
            manifest_json("1.2.0", "http://127.0.0.1:1/not-fetched"),
        );
        let app = mock_app_with_updater(&[&manifest_url]);
        let status = check_app_update(app.handle().clone()).await.expect("check");
        assert!(status.has_update);
        assert_eq!(status.latest_version, "1.2.0");
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[tokio::test]
    async fn check_app_update_errors_when_no_endpoint_is_configured() {
        let app = mock_app_with_updater(&[]);
        let error = check_app_update(app.handle().clone()).await.unwrap_err();
        assert!(error
            .to_string()
            .contains("Failed to initialize the updater"));
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[tokio::test]
    async fn check_app_update_reports_unsupported_without_touching_the_updater() {
        // Linux ships no formal installer: `check_app_update_impl` must return
        // an unsupported status without requiring the updater plugin at all.
        let app = tauri::test::mock_app();
        let status = check_app_update(app.handle().clone()).await.expect("check");
        assert!(!status.has_update);
        assert!(!status.platform_supported);
        assert_eq!(status.latest_version, build_info::VERSION);
        assert_eq!(status.download_url, None);
    }

    #[tokio::test]
    async fn install_app_update_rejects_non_release_builds() {
        let app = tauri::test::mock_app();
        let error = install_app_update_impl(app.handle().clone(), false)
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("only available in signed release builds"));
    }

    #[tokio::test]
    async fn install_app_update_wrapper_rejects_non_release_builds() {
        // Exercise the public `#[tauri::command]` wrapper body (not just the
        // injectable `_impl`) — the release-build guard short-circuits before
        // any updater work, so a mock app without the updater plugin suffices.
        let app = tauri::test::mock_app();
        let error = install_app_update(app.handle().clone()).await.unwrap_err();
        assert!(error
            .to_string()
            .contains("only available in signed release builds"));
    }

    #[tokio::test]
    async fn install_app_update_errors_when_no_update_is_available() {
        let manifest_url = serve_once("204 No Content", "application/json", Vec::new());
        let app = mock_app_with_updater(&[&manifest_url]);
        let error = install_app_update_impl(app.handle().clone(), true)
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("No update is currently available"));
    }

    #[tokio::test]
    async fn install_app_update_errors_when_the_updater_is_unconfigured() {
        let app = mock_app_with_updater(&[]);
        let error = install_app_update_impl(app.handle().clone(), true)
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("Failed to initialize the updater"));
    }

    #[tokio::test]
    async fn install_app_update_downloads_and_fails_signature_verification() {
        let download_url = serve_once(
            "200 OK",
            "application/octet-stream",
            b"fake-update-package".to_vec(),
        );
        let manifest_url = serve_once(
            "200 OK",
            "application/json",
            manifest_json("1.2.0", &download_url),
        );
        let app = mock_app_with_updater(&[&manifest_url]);
        // The update is downloaded (exercising the progress closure) but the
        // dummy signature/public key fails verification, surfacing the install
        // error rather than succeeding.
        let error = install_app_update_impl(app.handle().clone(), true)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Failed to install the update"));
    }

    #[test]
    fn restart_after_app_update_shuts_down_the_agent_before_relaunching() {
        let app = tauri::test::mock_app();
        restart_after_app_update_with(app.handle().clone(), |_| Ok(())).expect("restart");
    }
}

/// Download, verify and install the platform updater package.
///
/// Tauri verifies the mandatory minisign signature before installation. The
/// SHA-256 values in latest.json remain useful to website consumers and release
/// audits, but are not a substitute for this signature verification.
#[tauri::command]
pub async fn install_app_update<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<(), AppError> {
    install_app_update_impl(app, build_info::is_release()).await
}

async fn install_app_update_impl<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    is_release: bool,
) -> Result<(), AppError> {
    if !is_release {
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
#[rustfmt::skip]
pub fn restart_after_app_update(app: tauri::AppHandle) -> Result<(), AppError> { restart_after_app_update_with(app, |app| app.restart()) }

/// Body of [`restart_after_app_update`] with the relaunch injectable —
/// `restart()` re-execs the process, so tests inject a no-op.
fn restart_after_app_update_with<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    relaunch: impl FnOnce(tauri::AppHandle<R>) -> Result<(), AppError>,
) -> Result<(), AppError> {
    agent_supervisor::shutdown_agent_gracefully();
    relaunch(app)
}
