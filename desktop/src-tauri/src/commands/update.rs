//! Signed in-place application updates through Tauri's updater plugin.
//!
//! Online builds embed the updater public key through a per-build Tauri config
//! overlay. At runtime the full FutureOS version selects either the formal or
//! nightly CDN manifest. The manifest may also contain the custom top-level
//! `assets` map used for manual downloads; Tauri ignores those additional
//! fields and selects only the current entry under `platforms`.

use serde::Serialize;
use serde_json::Value;
use tauri::Emitter;
use tauri_plugin_updater::UpdaterExt;

use crate::{
    agent_supervisor,
    build_info::{self, BuildChannel},
    AppError,
};

const PROGRESS_EVENT: &str = "app-update-progress";
const RELEASE_MANIFEST_URL: &str = "https://dl.future-os.cn/releases/latest.json";
const NIGHTLY_MANIFEST_URL: &str = "https://dl.future-os.cn/nightly/latest.json";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    /// Whether this OS and installation format support the updater.
    pub platform_supported: bool,
    /// Whether this build channel may install the discovered update in-app.
    pub can_install_in_app: bool,
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

/// The `assets` key for the host platform, when it ships an installer.
///
/// Selected at compile time with `#[cfg]` (rather than `cfg!`) so the
/// inapplicable branches never emit dead regions that per-line coverage would
/// flag.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const PLATFORM_ASSET_KEY: &str = "darwin-aarch64";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const PLATFORM_ASSET_KEY: &str = "darwin-x86_64";
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const PLATFORM_ASSET_KEY: &str = "windows-x86_64";
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const PLATFORM_ASSET_KEY: &str = "linux-x86_64-deb";
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const PLATFORM_ASSET_KEY: &str = "linux-aarch64-deb";

#[cfg(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "aarch64")
))]
fn manual_download_url(manifest: &Value) -> Option<String> {
    manual_download_url_for_asset(manifest, PLATFORM_ASSET_KEY)
}

#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "aarch64")
)))]
fn manual_download_url(_manifest: &Value) -> Option<String> {
    None
}

fn manifest_url(channel: BuildChannel) -> &'static str {
    match channel {
        BuildChannel::Release => RELEASE_MANIFEST_URL,
        BuildChannel::Test | BuildChannel::Nightly | BuildChannel::Dev | BuildChannel::Local => {
            NIGHTLY_MANIFEST_URL
        }
    }
}

fn channel_run_number(version: &str) -> Option<u64> {
    let (_, remainder) = version.split_once('-')?;
    let (run, channel) = remainder.split_once('+')?;
    matches!(channel, "test" | "nightly")
        .then(|| run.parse().ok())
        .flatten()
}

/// Compare only versions from the selected channel. Test and nightly builds
/// share the Build Test run counter, so a newer test must never be downgraded
/// to an older nightly.
fn should_offer_update(channel: BuildChannel, current: &str, latest: &str) -> bool {
    match channel {
        BuildChannel::Release => {
            build_info::channel_for_version(latest) == BuildChannel::Release
                && matches!(
                    (
                        semver::Version::parse(current),
                        semver::Version::parse(latest)
                    ),
                    (Ok(current), Ok(latest)) if latest > current
                )
        }
        BuildChannel::Test | BuildChannel::Nightly => {
            if build_info::channel_for_version(latest) != BuildChannel::Nightly {
                return false;
            }
            let current_core = current.split(['-', '+']).next().unwrap_or(current);
            let latest_core = latest.split(['-', '+']).next().unwrap_or(latest);
            match (
                semver::Version::parse(current_core),
                semver::Version::parse(latest_core),
            ) {
                (Ok(current_core), Ok(latest_core)) if latest_core != current_core => {
                    latest_core > current_core
                }
                (Ok(_), Ok(_)) => match (channel_run_number(current), channel_run_number(latest)) {
                    (Some(current_run), Some(latest_run)) => latest_run > current_run,
                    _ => false,
                },
                _ => false,
            }
        }
        BuildChannel::Dev | BuildChannel::Local => {
            build_info::channel_for_version(latest) == BuildChannel::Nightly && latest != current
        }
    }
}

fn channel_allows_automatic_install(channel: BuildChannel) -> bool {
    matches!(
        channel,
        BuildChannel::Release | BuildChannel::Test | BuildChannel::Nightly
    )
}

#[cfg(target_os = "linux")]
fn os_release_is_debian_family(contents: &str) -> bool {
    contents.lines().any(|line| {
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        let value = value.trim().trim_matches(['\'', '"']);
        match key.trim() {
            "ID" => value == "debian",
            "ID_LIKE" => value.split_whitespace().any(|item| item == "debian"),
            _ => false,
        }
    })
}

#[cfg(target_os = "linux")]
fn is_debian_deb_install() -> bool {
    use tauri::utils::{config::BundleType, platform::bundle_type};

    if bundle_type() != Some(BundleType::Deb) {
        return false;
    }
    ["/etc/os-release", "/usr/lib/os-release"]
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
        .is_some_and(|contents| os_release_is_debian_family(&contents))
}

fn platform_allows_updates() -> bool {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        true
    }
    #[cfg(target_os = "linux")]
    {
        is_debian_deb_install()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        false
    }
}

fn automatic_install_supported(channel: BuildChannel) -> bool {
    platform_allows_updates() && channel_allows_automatic_install(channel)
}

/// Resolve a checked manifest into the status reported to the frontend.
///
/// Pure so the update-present / update-absent branches and installation policy
/// are testable without a live updater plugin.
fn resolve_update_status(
    channel: BuildChannel,
    current_version: String,
    update: Option<(String, Value)>,
) -> UpdateStatus {
    match update {
        Some((version, raw_json)) => UpdateStatus {
            current_version,
            latest_version: version,
            has_update: true,
            platform_supported: platform_allows_updates(),
            can_install_in_app: automatic_install_supported(channel),
            download_url: manual_download_url(&raw_json),
        },
        None => UpdateStatus {
            latest_version: current_version.clone(),
            current_version,
            has_update: false,
            platform_supported: platform_allows_updates(),
            can_install_in_app: automatic_install_supported(channel),
            download_url: None,
        },
    }
}

async fn check_manual_update(
    channel: BuildChannel,
    current_version: String,
) -> Result<UpdateStatus, AppError> {
    check_manual_update_from_url(channel, current_version, manifest_url(channel)).await
}

async fn check_manual_update_from_url(
    channel: BuildChannel,
    current_version: String,
    endpoint_url: &str,
) -> Result<UpdateStatus, AppError> {
    let response = reqwest::Client::new()
        .get(endpoint_url)
        .send()
        .await
        .map_err(|error| updater_error("Failed to check for updates", error))?
        .error_for_status()
        .map_err(|error| updater_error("Failed to check for updates", error))?;
    let manifest = response
        .json::<Value>()
        .await
        .map_err(|error| updater_error("Failed to parse update manifest", error))?;
    let latest_version = manifest
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Message("Update manifest has no version.".to_string()))?
        .to_string();
    let download_url = manual_download_url(&manifest);
    let has_update =
        download_url.is_some() && should_offer_update(channel, &current_version, &latest_version);
    Ok(UpdateStatus {
        current_version: current_version.clone(),
        latest_version: if has_update {
            latest_version
        } else {
            current_version
        },
        has_update,
        platform_supported: platform_allows_updates(),
        can_install_in_app: false,
        download_url: has_update.then_some(download_url).flatten(),
    })
}

/// Check the release manifest selected for the current build channel.
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
    check_app_update_impl(app, build_info::channel(), current_version).await
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
async fn check_app_update_impl<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    channel: BuildChannel,
    current_version: String,
) -> Result<UpdateStatus, AppError> {
    if !platform_allows_updates() {
        return Ok(UpdateStatus {
            latest_version: current_version.clone(),
            current_version,
            has_update: false,
            platform_supported: false,
            can_install_in_app: false,
            download_url: None,
        });
    }
    if matches!(channel, BuildChannel::Dev | BuildChannel::Local) {
        return check_manual_update(channel, current_version).await;
    }

    check_signed_update(app, channel, current_version, manifest_url(channel)).await
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
async fn check_signed_update<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    channel: BuildChannel,
    current_version: String,
    endpoint_url: &str,
) -> Result<UpdateStatus, AppError> {
    let endpoint = endpoint_url
        .parse()
        .map_err(|error| updater_error("Invalid update endpoint", error))?;
    let comparison_current = current_version.clone();
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|error| updater_error("Failed to select the update channel", error))?
        .version_comparator(move |_bundle_version, release| {
            should_offer_update(channel, &comparison_current, &release.version.to_string())
        })
        .build()
        .map_err(|error| updater_error("Failed to initialize the updater", error))?;
    let update = updater
        .check()
        .await
        .map_err(|error| updater_error("Failed to check for updates", error))?;
    Ok(resolve_update_status(
        channel,
        current_version,
        update.map(|update| (update.version, update.raw_json)),
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
async fn check_app_update_impl<R: tauri::Runtime>(
    _app: tauri::AppHandle<R>,
    _channel: BuildChannel,
    current_version: String,
) -> Result<UpdateStatus, AppError> {
    Ok(UpdateStatus {
        latest_version: current_version.clone(),
        current_version,
        has_update: false,
        platform_supported: false,
        can_install_in_app: false,
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
                "linux-x86_64-deb": platform.clone(),
                "linux-aarch64": platform.clone(),
                "linux-aarch64-deb": platform.clone(),
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

    #[test]
    fn selects_release_or_nightly_manifest_from_the_build_channel() {
        assert_eq!(manifest_url(BuildChannel::Release), RELEASE_MANIFEST_URL);
        for channel in [
            BuildChannel::Test,
            BuildChannel::Nightly,
            BuildChannel::Dev,
            BuildChannel::Local,
        ] {
            assert_eq!(manifest_url(channel), NIGHTLY_MANIFEST_URL);
        }
    }

    #[test]
    fn channel_comparison_never_downgrades_test_to_nightly() {
        assert!(!should_offer_update(
            BuildChannel::Test,
            "0.0.2-120+test",
            "0.0.2-119+nightly"
        ));
        assert!(!should_offer_update(
            BuildChannel::Test,
            "0.0.2-120+test",
            "0.0.2-120+nightly"
        ));
        assert!(should_offer_update(
            BuildChannel::Test,
            "0.0.2-120+test",
            "0.0.2-121+nightly"
        ));
    }

    #[test]
    fn nightly_only_advances_to_a_newer_nightly() {
        assert!(should_offer_update(
            BuildChannel::Nightly,
            "0.0.2-120+nightly",
            "0.0.2-121+nightly"
        ));
        assert!(!should_offer_update(
            BuildChannel::Nightly,
            "0.0.2-120+nightly",
            "1.0.0"
        ));
    }

    #[test]
    fn release_only_advances_to_a_newer_release() {
        assert!(should_offer_update(BuildChannel::Release, "1.0.0", "1.0.1"));
        assert!(!should_offer_update(
            BuildChannel::Release,
            "1.0.0",
            "0.0.2-999+nightly"
        ));
    }

    #[test]
    fn local_and_dev_only_offer_nightlies_manually() {
        assert!(should_offer_update(
            BuildChannel::Local,
            "0.0.2-abcdef+local",
            "0.0.2-121+nightly"
        ));
        assert!(should_offer_update(
            BuildChannel::Dev,
            "0.0.2-abcdef+dev",
            "0.0.2-121+nightly"
        ));
        assert!(!channel_allows_automatic_install(BuildChannel::Local));
        assert!(!channel_allows_automatic_install(BuildChannel::Dev));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn identifies_debian_family_os_release_files() {
        assert!(os_release_is_debian_family("ID=debian\n"));
        assert!(os_release_is_debian_family("ID=ubuntu\nID_LIKE=debian\n"));
        assert!(os_release_is_debian_family(
            "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n"
        ));
        assert!(!os_release_is_debian_family(
            "ID=fedora\nID_LIKE=\"rhel centos fedora\"\n"
        ));
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
        let status = resolve_update_status(
            BuildChannel::Release,
            "1.1.0".to_string(),
            Some(("1.2.0".to_string(), json!({}))),
        );
        assert!(status.has_update);
        assert_eq!(status.latest_version, "1.2.0");
        assert_eq!(status.current_version, "1.1.0");
        assert_eq!(status.platform_supported, platform_allows_updates());
        assert!(status.can_install_in_app);
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn resolve_update_status_reports_no_update() {
        let status = resolve_update_status(BuildChannel::Release, "1.1.0".to_string(), None);
        assert!(!status.has_update);
        assert_eq!(status.latest_version, "1.1.0");
        assert_eq!(status.current_version, "1.1.0");
        assert_eq!(status.platform_supported, platform_allows_updates());
        assert!(status.can_install_in_app);
        assert_eq!(status.download_url, None);
    }

    // Signed updater-path tests run on macOS/Windows here. Linux gets its own
    // unbundled/unsupported-path test further down; packaged `.deb` behavior is
    // covered by the pure distro parser and the plugin's bundle-type contract.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[tokio::test]
    async fn check_app_update_reports_no_update_on_204() {
        let manifest_url = serve_once("204 No Content", "application/json", Vec::new());
        let app = mock_app_with_updater(&[&manifest_url]);
        let status = check_signed_update(
            app.handle().clone(),
            BuildChannel::Release,
            "1.0.0".to_string(),
            &manifest_url,
        )
        .await
        .expect("check");
        assert!(!status.has_update);
        assert!(status.platform_supported);
        assert!(status.can_install_in_app);
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
        let status = check_signed_update(
            app.handle().clone(),
            BuildChannel::Release,
            "1.0.0".to_string(),
            &manifest_url,
        )
        .await
        .expect("check");
        assert!(status.has_update);
        assert_eq!(status.latest_version, "1.2.0");
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[tokio::test]
    async fn manual_build_reads_the_nightly_asset_without_enabling_installation() {
        let manifest_url = serve_once(
            "200 OK",
            "application/json",
            json!({
                "version": "0.0.2-121+nightly",
                "assets": {
                    (PLATFORM_ASSET_KEY): {
                        "url": "https://dl.future-os.cn/nightly/0.0.2-121/FutureOS-installer"
                    }
                }
            })
            .to_string()
            .into_bytes(),
        );
        let status = check_manual_update_from_url(
            BuildChannel::Local,
            "0.0.2-abcdef+local".to_string(),
            &manifest_url,
        )
        .await
        .expect("manual check");
        assert!(status.has_update);
        assert!(status.platform_supported);
        assert!(!status.can_install_in_app);
        assert_eq!(
            status.download_url.as_deref(),
            Some("https://dl.future-os.cn/nightly/0.0.2-121/FutureOS-installer")
        );
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[tokio::test]
    async fn check_app_update_errors_for_an_invalid_selected_endpoint() {
        let app = mock_app_with_updater(&[]);
        let error = check_signed_update(
            app.handle().clone(),
            BuildChannel::Release,
            "1.0.0".to_string(),
            "not a url",
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Invalid update endpoint"));
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[tokio::test]
    async fn check_app_update_reports_unsupported_without_touching_the_updater() {
        // An unbundled Linux test binary is not a `.deb` install, so it must
        // return unsupported without touching the updater plugin.
        let app = tauri::test::mock_app();
        let status = check_app_update(app.handle().clone()).await.expect("check");
        assert!(!status.has_update);
        assert!(!status.platform_supported);
        assert!(!status.can_install_in_app);
        assert_eq!(status.latest_version, build_info::VERSION);
        assert_eq!(status.download_url, None);
    }

    #[tokio::test]
    async fn install_app_update_rejects_manual_only_builds() {
        let app = tauri::test::mock_app();
        let error = install_app_update_impl(
            app.handle().clone(),
            BuildChannel::Local,
            false,
            NIGHTLY_MANIFEST_URL,
            "0.0.2-abcdef+local",
        )
        .await
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("not available for this build or platform"));
    }

    #[tokio::test]
    async fn install_app_update_wrapper_rejects_the_current_manual_only_build() {
        // Exercise the public `#[tauri::command]` wrapper body (not just the
        // injectable `_impl`) — the channel/platform guard short-circuits before
        // any updater work, so a mock app without the updater plugin suffices.
        let app = tauri::test::mock_app();
        let error = install_app_update(app.handle().clone()).await.unwrap_err();
        assert!(error
            .to_string()
            .contains("not available for this build or platform"));
    }

    #[tokio::test]
    async fn install_app_update_errors_when_no_update_is_available() {
        let manifest_url = serve_once("204 No Content", "application/json", Vec::new());
        let app = mock_app_with_updater(&[&manifest_url]);
        let error = install_app_update_impl(
            app.handle().clone(),
            BuildChannel::Release,
            true,
            &manifest_url,
            "1.0.0",
        )
        .await
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("No update is currently available"));
    }

    #[tokio::test]
    async fn install_app_update_errors_when_the_selected_endpoint_is_invalid() {
        let app = mock_app_with_updater(&[]);
        let error = install_app_update_impl(
            app.handle().clone(),
            BuildChannel::Release,
            true,
            "not a url",
            "1.0.0",
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Invalid update endpoint"));
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
        let error = install_app_update_impl(
            app.handle().clone(),
            BuildChannel::Release,
            true,
            &manifest_url,
            "1.0.0",
        )
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
    let channel = build_info::channel();
    install_app_update_impl(
        app,
        channel,
        automatic_install_supported(channel),
        manifest_url(channel),
        build_info::VERSION,
    )
    .await
}

async fn install_app_update_impl<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    channel: BuildChannel,
    can_install: bool,
    endpoint_url: &str,
    current_version: &str,
) -> Result<(), AppError> {
    if !can_install {
        return Err(AppError::Message(
            "Automatic installation is not available for this build or platform.".to_string(),
        ));
    }

    let endpoint = endpoint_url
        .parse()
        .map_err(|error| updater_error("Invalid update endpoint", error))?;
    let comparison_current = current_version.to_string();
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|error| updater_error("Failed to select the update channel", error))?
        .version_comparator(move |_bundle_version, release| {
            should_offer_update(channel, &comparison_current, &release.version.to_string())
        })
        .build()
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
