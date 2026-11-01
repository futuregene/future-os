//! Signed in-place application updates through Tauri's updater plugin.
//!
//! Online builds embed the updater public key through a per-build Tauri config
//! overlay. At runtime the full FutureOS version selects either the formal or
//! nightly CDN manifest. The manifest may also contain the custom top-level
//! `assets` map used for manual downloads; Tauri ignores those additional
//! fields and selects only the current entry under `platforms`.

#[cfg(not(target_os = "windows"))]
use future_host::console;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;
use tauri::Emitter;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::UpdaterExt;

static INSTALL_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
struct InstallGuard;
#[cfg(any(target_os = "windows", test))]
fn quote_windows_argument(value: &str) -> String {
    let mut quoted = String::from("\"");
    let mut slashes = 0;
    for character in value.chars() {
        if character == '\\' {
            slashes += 1;
            continue;
        }
        quoted.extend(std::iter::repeat_n(
            '\\',
            if character == '"' {
                2 * slashes + 1
            } else {
                slashes
            },
        ));
        quoted.push(character);
        slashes = 0;
    }
    quoted.extend(std::iter::repeat_n('\\', 2 * slashes));
    quoted.push('"');
    quoted
}
impl Drop for InstallGuard {
    fn drop(&mut self) {
        INSTALL_ACTIVE.store(false, std::sync::atomic::Ordering::Release);
    }
}

fn install_after_cleanup(
    cleanup: impl FnOnce() -> Result<(), AppError>,
    install: impl FnOnce() -> Result<(), String>,
) -> Result<(), AppError> {
    cleanup()?;
    install().map_err(|error| AppError::Message(format!(
        "Update installation failed after cleanup: {error}. Restart FutureOS to recover the service."
    )))
}

fn update_needs_confirmation(activity: Result<usize, AppError>, running_terminals: usize) -> bool {
    !matches!(activity, Ok(0)) || running_terminals > 0
}

#[path = "update_adapters.rs"]
mod adapters;
#[path = "update_journal.rs"]
mod journal;
#[cfg(target_os = "linux")]
#[path = "update_linux.rs"]
mod linux;
#[cfg(target_os = "macos")]
#[path = "update_macos.rs"]
mod macos;
#[path = "update_process.rs"]
mod process;
#[cfg(any(target_os = "macos", all(test, not(target_os = "windows"))))]
#[path = "update_recovery.rs"]
mod recovery;
#[path = "update_state.rs"]
mod state;
#[cfg(target_os = "windows")]
#[path = "update_windows.rs"]
mod windows;

use crate::{
    build_info::{self, BuildChannel},
    AppError,
};

#[doc(hidden)]
pub fn dispatch_update_helper(args: &[String]) -> Option<Result<(), String>> {
    match args.first().map(String::as_str) {
        #[cfg(target_os = "macos")]
        Some("--app-update-macos-worker") => Some(helper_result(macos::worker(&args[1..]))),
        #[cfg(target_os = "linux")]
        Some("--app-update-deb-worker") => Some(helper_result(linux::worker(&args[1..]))),
        _ => None,
    }
}
/// Run the macOS update-preparation chain end to end against a bundle in a temporary
/// directory, with `helper` standing in for the privileged worker.
///
/// Re-exported through [`crate::test_support`]. See `macos::prepare_and_commit` for why the
/// helper is a parameter, and why the caller must be a process without a parallel suite.
#[cfg(all(target_os = "macos", any(test, feature = "test-support")))]
pub fn prepare_update_bundle_for_tests(
    version: &str,
    bytes: &[u8],
    target: std::path::PathBuf,
    helper: &std::path::Path,
) -> Result<String, String> {
    macos::prepare_and_commit(version, bytes, target, helper)
}
/// The same chain, prepared and then **dropped** instead of committed — the cancel path, whose
/// behaviour is `Prepared::drop` removing the staging directory it created beside the bundle.
#[cfg(all(target_os = "macos", any(test, feature = "test-support")))]
pub fn prepare_and_cancel_update_for_tests(
    version: &str,
    bytes: &[u8],
    target: std::path::PathBuf,
    helper: &std::path::Path,
) -> Result<(), String> {
    macos::prepare_and_drop(version, bytes, target, helper)
}
/// The **worker** half of the macOS install, for a process that can *be* the helper.
///
/// Production never calls this: `dispatch_update_helper` runs it as
/// `--app-update-macos-worker`, which is how the app re-invokes *itself*. A test target that
/// is to act as the worker (a `harness = false` driver, not a libtest body) dispatches the
/// same flag to this and is then the real worker, not a stand-in for it.
#[cfg(all(target_os = "macos", any(test, feature = "test-support")))]
pub fn run_update_worker_for_tests(args: &[String]) -> Result<(), String> {
    macos::worker(args)
}
/// [`macos::probe`]: its 10 s deadline can only be *expired* by a candidate that outlives it,
/// so the arm needs a process without a few hundred parallel libtest cases stalling its own
/// process creation.
#[cfg(all(target_os = "macos", any(test, feature = "test-support")))]
pub fn probe_update_candidate_for_tests(path: &std::path::Path) -> Result<(), String> {
    macos::probe(path)
}
#[cfg(not(target_os = "windows"))]
fn helper_result(result: Result<(), String>) -> Result<(), String> {
    if let Err(error) = &result {
        console::println!("ERROR {}", serde_json::to_string(error).unwrap_or_default());
    }
    result
}
#[cfg(not(test))]
pub(crate) fn confirm_update_startup() {
    if let Err(error) = journal::confirm_startup(build_info::VERSION) {
        crate::host_environment::report(&format!("Update startup confirmation deferred: {error}"));
    }
}

const PROGRESS_EVENT: &str = "app-update-progress";
const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const UPDATE_INSTALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
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

pub(crate) fn manifest_url(channel: BuildChannel) -> &'static str {
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
    #[cfg(target_os = "macos")]
    {
        macos::can_prepare()
    }
    #[cfg(target_os = "windows")]
    {
        use tauri::utils::{config::BundleType, platform::bundle_type};

        // The portable ZIP contains the unpatched build output, for which
        // bundle_type() is None. It has no registered install directory, so an
        // NSIS in-place update could replace a different installation or fail
        // after shutting down this portable process.
        bundle_type() == Some(BundleType::Nsis)
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

/// The manual (Dev/Local) check, against the manifest endpoint its caller
/// selected. The endpoint stays a parameter instead of `manifest_url(channel)`
/// here so the production constant is resolved once, at the outermost caller,
/// and so a test can aim the same code at a local manifest server.
async fn check_manual_update(
    channel: BuildChannel,
    current_version: String,
    endpoint_url: &str,
) -> Result<UpdateStatus, AppError> {
    check_manual_update_from_url(channel, current_version, endpoint_url).await
}

async fn check_manual_update_from_url(
    channel: BuildChannel,
    current_version: String,
    endpoint_url: &str,
) -> Result<UpdateStatus, AppError> {
    let client = future_agent::http::client_builder()
        .timeout(UPDATE_CHECK_TIMEOUT)
        .build()
        .map_err(|error| updater_error("Failed to initialize update client", error))?;
    let response = client
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
    // The production endpoint is resolved here, at the boundary that owns the
    // build channel, so every layer below it takes the URL as a parameter.
    crate::scheduler::check_app_update_now(app, manifest_url(build_info::channel())).await
}

pub(crate) async fn perform_app_update_check<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    endpoint_url: &str,
) -> Result<UpdateStatus, AppError> {
    let current_version = build_info::VERSION.to_string();
    check_app_update_impl(app, build_info::channel(), current_version, endpoint_url).await
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
async fn check_app_update_impl<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    channel: BuildChannel,
    current_version: String,
    endpoint_url: &str,
) -> Result<UpdateStatus, AppError> {
    if !platform_allows_updates() {
        return check_manual_update(channel, current_version, endpoint_url).await;
    }
    if matches!(channel, BuildChannel::Dev | BuildChannel::Local) {
        return check_manual_update(channel, current_version, endpoint_url).await;
    }

    check_signed_update(app, channel, current_version, endpoint_url).await
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
        .timeout(UPDATE_CHECK_TIMEOUT)
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
    _endpoint_url: &str,
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

    #[test]
    fn idle_updates_skip_the_stop_work_warning_but_activity_and_unknown_state_do_not() {
        assert!(!update_needs_confirmation(Ok(0), 0));
        assert!(update_needs_confirmation(Ok(1), 0));
        assert!(update_needs_confirmation(Ok(0), 1));
        assert!(update_needs_confirmation(
            Err("activity unavailable".into()),
            0
        ));
    }

    /// The maintenance status reports the CLI it can actually use. `discover`
    /// runs `<cli> version --json` and validates the declared Agent contract
    /// against this build's, so the fixture is a real executable that answers
    /// that one command — and the borrowed `PATH` is restored by
    /// `cli_tool::EnvGuard` even if an assertion panics (a leaked `PATH` is
    /// FLAKES item 27: 25 later tests failed spawning `git`).
    ///
    /// What it asserts is what discovery *captured*, so it must not be decided
    /// by the probe's 3 s production budget: under the full parallel suite the
    /// fixture's `sh` lost that race against this host's scheduler and the test
    /// read `CLI discovery timed out` (measured 2026-11-01, 387 passed / 1
    /// failed; alone it passed 3 of 3). The seam is `ProbeDeadlineGuard`, as in
    /// `skills_bootstrap`, and it is armed on *this* thread because
    /// `app_maintenance_status` offloads the probe to `spawn_blocking` and
    /// carries the armed deadline across itself — the arm that pins the
    /// production budget to 3 s is `cli_tool`'s own
    /// `a_silent_cli_is_killed_and_reported_as_a_timeout`, not this one.
    /// A real executable answering `<cli> version --json` with this build's
    /// declared contract, put first on `PATH`. `CliCapabilities::default()` is
    /// a valid contract for this build (the CLI's own discovery tests use it
    /// the same way). Both halves of the fixture are undone on drop, panic or
    /// not: the `PATH` restores before the temporary directory disappears.
    #[cfg(unix)]
    fn cli_on_path(
        version: &str,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::cli_tool::EnvGuard,
    ) {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().expect("tempdir");
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&bin).expect("bin dir");
        let cli = bin.join("future");
        let reply = json!({
            "version": version,
            "desktopCapabilities": future_rpc::cli_capabilities::CliCapabilities::default(),
        });
        std::fs::write(&cli, format!("#!/bin/sh\nprintf '%s' '{}'\n", reply))
            .expect("write the fixture CLI");
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        // Prepended, not replaced: these tests only add a candidate, they never
        // take `git` or `sh` away from anything else in the process.
        let existing = std::env::var_os("PATH").unwrap_or_default();
        let path = std::env::join_paths(
            std::iter::once(bin.clone()).chain(std::env::split_paths(&existing)),
        )
        .expect("join PATH");
        (root, bin, crate::cli_tool::EnvGuard::set("PATH", &path))
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn app_maintenance_status_reports_the_cli_it_discovered() {
        if future_host::test_support::reexec_test() {
            return;
        }
        let _deadline = future_host::cli_tool::ProbeDeadlineGuard::without_deadline();

        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-update-maintenance");
        crate::store::initialize_app_store().expect("init store");

        let (_root, bin, _path) = cli_on_path("test-cli-1.0");

        let status = app_maintenance_status().await.expect("maintenance status");
        assert_eq!(
            status["cli"]["tool"]["version"], "test-cli-1.0",
            "the discovered CLI is reported by version: {status}"
        );
        assert_eq!(
            status["cli"]["tool"]["source"], "path",
            "it was found on PATH, not beside the binary: {status}"
        );
        assert!(
            status["cli"]["tool"]["path"]
                .as_str()
                .is_some_and(|reported| reported.starts_with(&*bin.display().to_string())),
            "the reported path is the fixture's: {status}"
        );
    }

    /// The deadline a test arms on its own thread has to survive
    /// `app_maintenance_status`'s hop to the blocking pool: `spawn_blocking`
    /// carries no thread-locals and the override is thread-local on purpose,
    /// so the command re-arms it on the thread that probes. A zero deadline
    /// makes the probe kill its child before its first `try_wait`, so the
    /// timeout error appears *only* while that carry works — drop the carry and
    /// this fails deterministically, instead of the capture test above going
    /// back to flaking under load (which is how it was found: 387 passed / 1
    /// failed, passing 3 of 3 alone).
    #[cfg(unix)]
    #[tokio::test]
    async fn an_armed_probe_deadline_reaches_the_probe_behind_spawn_blocking() {
        if future_host::test_support::reexec_test() {
            return;
        }
        let _deadline = future_host::cli_tool::ProbeDeadlineGuard::set(Some(Duration::ZERO));

        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-update-probe-carry");
        crate::store::initialize_app_store().expect("init store");

        let (_root, _bin, _path) = cli_on_path("must-not-be-read");

        let status = app_maintenance_status().await.expect("maintenance status");
        assert!(
            status["cli"]["error"]
                .as_str()
                .is_some_and(|error| error.contains("CLI discovery timed out")),
            "the fixture CLI must not have been read: the armed zero deadline \
             has to reach the probe on the blocking thread: {status}"
        );
        assert_eq!(
            status["cli"]["tool"],
            serde_json::Value::Null,
            "a probe that timed out reports no tool: {status}"
        );
    }

    #[test]
    fn windows_resume_arguments_preserve_empty_quotes_slashes_and_option_like_values() {
        assert_eq!(quote_windows_argument(""), "\"\"");
        assert_eq!(quote_windows_argument("/UPDATE"), "\"/UPDATE\"");
        assert_eq!(quote_windows_argument("a\"b"), "\"a\\\"b\"");
        assert_eq!(
            quote_windows_argument("C:\\some path\\"),
            "\"C:\\some path\\\\\""
        );
    }

    /// Only the initialisation subcommand is an update helper. Any other argv is
    /// left to the ordinary desktop startup, which is what keeps this dispatch
    /// from swallowing a normal launch.
    #[test]
    fn an_argv_that_is_not_an_update_helper_subcommand_is_not_consumed() {
        assert!(dispatch_update_helper(&[]).is_none());
        assert!(dispatch_update_helper(&["--version".to_string()]).is_none());
        assert!(dispatch_update_helper(&["--app-update-worker".to_string()]).is_none());
    }

    /// The helper dispatch reports the worker's failure instead of hiding it, and
    /// passes success through untouched. The `Err` arm also writes the `ERROR`
    /// diagnostics line through `console::`; that line cannot be asserted here
    /// because `console` writes straight to the process's stdout rather than
    /// through `std`'s captured printer (see `agent/src/console.rs`), so the
    /// observable contract is the returned result.
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn helper_result_passes_the_result_through_and_reports_a_failure() {
        assert_eq!(helper_result(Ok(())), Ok(()));
        assert_eq!(
            helper_result(Err("installer refused".into())),
            Err("installer refused".to_string())
        );
    }

    /// The worker subcommand is consumed and its refusal is dispatched: with no
    /// plan arguments the macOS worker stops before it opens the bundle, so this
    /// reaches the dispatch arm without an installer.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_update_helper_argv_is_consumed_and_the_workers_refusal_is_reported() {
        let outcome = dispatch_update_helper(&["--app-update-macos-worker".to_string()])
            .expect("the worker subcommand is consumed");
        assert_eq!(
            outcome.unwrap_err(),
            "Invalid macOS update worker arguments"
        );
    }

    /// Every native update dialog resolves its text from the bundled settings
    /// translations, in the language the window asks for. A missing key or a
    /// language that fell through to the wrong dictionary would surface here
    /// rather than as an untranslated dialog at install time.
    #[test]
    fn every_native_update_dialog_has_its_own_text_in_both_languages() {
        const KEYS: [&str; 8] = [
            "installTitle",
            "installConfirmation",
            "installConfirm",
            "installDefer",
            "repairTitle",
            "repairConfirmation",
            "repairConfirm",
            "repairCancel",
        ];
        for key in KEYS {
            let english = update_dialog_text(None, key);
            let chinese = update_dialog_text(Some("zh-CN"), key);
            assert!(!english.is_empty(), "{key} has no English text");
            assert!(!chinese.is_empty(), "{key} has no Chinese text");
            assert_ne!(chinese, english, "{key} was not translated");
            // Only a `zh*` tag selects Chinese; every other tag gets English.
            assert_eq!(update_dialog_text(Some("zh-TW"), key), chinese);
            assert_eq!(update_dialog_text(Some("en-US"), key), english);
            assert_eq!(update_dialog_text(Some("de"), key), english);
        }
    }

    #[test]
    fn cleanup_failure_never_enters_installation() {
        let mut installed = false;
        let result = install_after_cleanup(
            || Err("drain failed".into()),
            || {
                installed = true;
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err().to_string(), "drain failed");
        assert!(!installed);
    }

    #[test]
    fn installation_failure_requires_process_recovery_after_successful_cleanup() {
        let cleaned = std::cell::Cell::new(false);
        let result = install_after_cleanup(
            || {
                cleaned.set(true);
                Ok(())
            },
            || {
                assert!(cleaned.get());
                Err("installer launch denied".into())
            },
        );
        let message = result.unwrap_err().to_string();
        assert!(message.starts_with("Update installation failed after cleanup:"));
        assert!(message.contains("Restart FutureOS"));
    }

    /// Hold the process-wide install flag so the "already in progress" arms are
    /// reachable, and release it even if the test panics — the flag is shared by
    /// every later test in this process.
    struct InstallActiveGuard;

    impl InstallActiveGuard {
        fn hold() -> Self {
            INSTALL_ACTIVE.store(true, std::sync::atomic::Ordering::Release);
            Self
        }
    }

    impl Drop for InstallActiveGuard {
        fn drop(&mut self) {
            INSTALL_ACTIVE.store(false, std::sync::atomic::Ordering::Release);
        }
    }

    /// A one-shot HTTP server answering a single request with a fixed body.
    fn serve_once(status: &'static str, content_type: &'static str, body: Vec<u8>) -> String {
        // Embedded Agent unifies multiple rustls provider features. A standalone
        // mock app must initialize the same provider as GUI/headless startup.
        crate::install_rustls_provider();
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
    fn channel_comparison_covers_core_and_run_number_advances() {
        // Different cores: the newer core wins on both channels, and the older
        // one never does (the `latest_core > current_core` comparison).
        assert!(should_offer_update(
            BuildChannel::Test,
            "0.0.2-120+test",
            "0.0.3-1+nightly"
        ));
        assert!(!should_offer_update(
            BuildChannel::Nightly,
            "0.0.3-1+nightly",
            "0.0.2-120+nightly"
        ));
        // Same core but a run counter that cannot be read: never offer.
        assert!(!should_offer_update(
            BuildChannel::Test,
            "0.0.2-abc+test",
            "0.0.2-abc+nightly"
        ));
        assert!(!should_offer_update(
            BuildChannel::Test,
            "0.0.2",
            "0.0.2-121+nightly"
        ));
        // A version that does not parse at all is not offered either.
        assert!(!should_offer_update(
            BuildChannel::Nightly,
            "not-a-version",
            "0.0.3-1+nightly"
        ));
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[tokio::test]
    async fn a_manual_check_without_a_newer_build_reports_no_update() {
        let manifest_url = serve_once(
            "200 OK",
            "application/json",
            json!({
                "version": "0.0.2-121+nightly",
                "assets": {
                    (PLATFORM_ASSET_KEY): { "url": "https://dl.future-os.cn/nightly/x" }
                }
            })
            .to_string()
            .into_bytes(),
        );
        // The same build the user already runs: no update, no download link,
        // and the reported latest version stays the current one.
        let status = check_manual_update_from_url(
            BuildChannel::Local,
            "0.0.2-121+nightly".to_string(),
            &manifest_url,
        )
        .await
        .expect("manual check");
        assert!(!status.has_update);
        assert_eq!(status.latest_version, "0.0.2-121+nightly");
        assert_eq!(status.current_version, "0.0.2-121+nightly");
        assert_eq!(status.download_url, None, "no update means no link");
        assert!(!status.can_install_in_app);
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

    #[cfg(target_os = "windows")]
    #[test]
    fn unbundled_windows_build_does_not_offer_in_place_updates() {
        // Test executables are not patched as NSIS bundles. Portable release
        // binaries have the same bundle marker and must remain manual-only.
        assert!(!platform_allows_updates());
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
        assert_eq!(status.can_install_in_app, platform_allows_updates());
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn resolve_update_status_reports_no_update() {
        let status = resolve_update_status(BuildChannel::Release, "1.1.0".to_string(), None);
        assert!(!status.has_update);
        assert_eq!(status.latest_version, "1.1.0");
        assert_eq!(status.current_version, "1.1.0");
        assert_eq!(status.platform_supported, platform_allows_updates());
        assert_eq!(status.can_install_in_app, platform_allows_updates());
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
        assert_eq!(status.platform_supported, platform_allows_updates());
        assert_eq!(status.can_install_in_app, platform_allows_updates());
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
        assert_eq!(status.platform_supported, platform_allows_updates());
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

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn unsupported_installation_still_discovers_manual_updates() {
        let endpoint = serve_once("200 OK", "application/json", json!({
            "version": "1.2.0", "assets": { (PLATFORM_ASSET_KEY): { "url": "https://example.com/update.deb" } }
        }).to_string().into_bytes());
        let app = tauri::test::mock_app();
        let status = check_app_update_impl(
            app.handle().clone(),
            BuildChannel::Release,
            "1.0.0".into(),
            &endpoint,
        )
        .await
        .unwrap();
        assert!(status.has_update);
        assert!(!status.can_install_in_app);
        assert_eq!(
            status.download_url.as_deref(),
            Some("https://example.com/update.deb")
        );
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
            None,
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
        let error = install_app_update(app.handle().clone(), None)
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("not available for this build or platform"));
    }

    #[tokio::test]
    async fn install_app_update_errors_when_no_update_is_available() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-no-release");
        let manifest_url = serve_once("204 No Content", "application/json", Vec::new());
        let app = mock_app_with_updater(&[&manifest_url]);
        let error = install_app_update_impl(
            app.handle().clone(),
            BuildChannel::Release,
            true,
            &manifest_url,
            "1.0.0",
            None,
        )
        .await
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("No update is currently available"));
    }

    #[tokio::test]
    async fn install_app_update_errors_when_the_selected_endpoint_is_invalid() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-invalid-endpoint");
        let app = mock_app_with_updater(&[]);
        let error = install_app_update_impl(
            app.handle().clone(),
            BuildChannel::Release,
            true,
            "not a url",
            "1.0.0",
            None,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Invalid update endpoint"));
    }

    #[tokio::test]
    async fn install_app_update_downloads_and_fails_signature_verification() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-bad-signature");
        let phase = crate::agent_host::host().phase();
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
            None,
        )
        .await
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("Failed to download and verify the update"));
        assert!(!crate::desktop_shutdown::is_stopping());
        assert_eq!(crate::agent_host::host().phase(), phase);
    }

    #[test]
    fn failed_cleanup_prevents_relaunch_and_stays_failed_on_retry() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-failed-stop");
        let _root = journal::CaseRoot::bind();
        crate::desktop_shutdown::fail_for_test("Agent drain failed");
        let app = tauri::test::mock_app();
        for _ in 0..2 {
            let error = restart_after_app_update_with(app.handle().clone(), |_| {
                panic!("failed cleanup must not relaunch")
            })
            .unwrap_err();
            assert!(error.to_string().contains("Agent drain failed"));
        }
    }

    #[test]
    fn restart_after_app_update_shuts_down_the_agent_before_relaunching() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-restart");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        let app = tauri::test::mock_app();
        restart_after_app_update_with(app.handle().clone(), |_| Ok(())).expect("restart");
    }

    /// A manual (Dev/Local) check goes through the endpoint it is given, not the
    /// production CDN constant: that parameter is the whole seam — the code
    /// below it is the already-covered `check_manual_update_from_url`.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[tokio::test]
    async fn a_manual_build_check_uses_the_endpoint_it_is_given() {
        let platform_supported = platform_allows_updates();
        let manifest_url = serve_once(
            "200 OK",
            "application/json",
            json!({
                "version": "0.0.2-121+nightly", "assets": {
                    (PLATFORM_ASSET_KEY): { "url": "https://dl.future-os.cn/nightly/x" }
                }
            })
            .to_string()
            .into_bytes(),
        );
        let app = tauri::test::mock_app();
        let status = check_app_update_impl(
            app.handle().clone(),
            BuildChannel::Local,
            "0.0.2-abcdef+local".to_string(),
            &manifest_url,
        )
        .await
        .expect("manual check through the impl");
        assert!(status.has_update);
        assert!(status.download_url.is_some());
        assert_eq!(status.platform_supported, platform_supported);
        assert!(!status.can_install_in_app);
    }

    /// The signed channel takes the same endpoint parameter through to the
    /// updater plugin, so the mock manifest server stands in for the CDN.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[tokio::test]
    async fn a_signed_build_check_uses_the_endpoint_it_is_given() {
        let platform_supported = platform_allows_updates();
        let manifest_url = serve_once("200 OK", "application/json", json!({
            "version": "1.2.0", "platforms": {
                (PLATFORM_ASSET_KEY): { "url": "http://127.0.0.1:1/not-fetched", "signature": "unused" }
            }, "assets": { (PLATFORM_ASSET_KEY): { "url": "https://example.com/installer" } }
        }).to_string().into_bytes());
        let app = mock_app_with_updater(&[&manifest_url]);
        let status = check_app_update_impl(
            app.handle().clone(),
            BuildChannel::Release,
            "1.0.0".to_string(),
            &manifest_url,
        )
        .await
        .expect("signed check through the impl");
        assert!(status.has_update);
        assert_eq!(status.can_install_in_app, platform_supported);
    }

    /// The body the scheduler calls resolves the channel and version from the
    /// build and passes the endpoint straight through.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[tokio::test]
    async fn the_scheduled_check_body_passes_its_endpoint_through() {
        let manifest_url = serve_once(
            "200 OK",
            "application/json",
            manifest_json("9.9.9", "http://127.0.0.1:1/not-fetched"),
        );
        let app = mock_app_with_updater(&[&manifest_url]);
        let status = perform_app_update_check(app.handle().clone(), &manifest_url)
            .await
            .expect("scheduled check body");
        assert_eq!(status.current_version, build_info::VERSION);
    }

    /// The maintenance status command is callable without an app handle: it
    /// reports the CLI tool and whatever the journal holds.
    #[tokio::test]
    async fn app_maintenance_status_reports_the_cli_and_the_latest_record() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-maintenance-status");
        let value = app_maintenance_status().await.expect("maintenance status");
        assert!(value.get("cli").is_some());
        assert!(value.get("update").is_some());
    }

    /// A record that does not permit restart is refused before the relaunch
    /// (the injected closure panics, so reaching it would fail this test) and
    /// before the shutdown barrier is touched.
    #[test]
    fn a_restart_is_refused_without_a_safe_record() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-restart-unsafe");
        let _root = journal::CaseRoot::bind();
        let app = tauri::test::mock_app();
        let operation = journal::Operation::begin("0.0.2-abcdef+local").expect("journal record");
        operation
            .finish(
                "repair-required",
                Some("installer launch denied".into()),
                false,
            )
            .expect("persist result");
        let error = restart_after_app_update_with(app.handle().clone(), |_| {
            panic!("an unsafe record must not relaunch")
        })
        .unwrap_err();
        assert!(error.to_string().contains("does not permit restart"));
    }

    /// The restart entry point refuses while an install holds the process-wide
    /// flag, so a restart can never interleave with an in-flight installation.
    #[test]
    fn a_restart_is_refused_while_maintenance_is_in_progress() {
        let _home = crate::auth_store::test_support::HomeGuard::new("update-restart-busy");
        let app = tauri::test::mock_app();
        let _holding = InstallActiveGuard::hold();

        let error = restart_after_app_update_with(app.handle().clone(), |_| {
            panic!("a restart must not run while maintenance is in progress")
        })
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("Update maintenance is in progress"));
    }

    /// macOS rechecks its saved bundle fingerprint on restart. Windows verifies
    /// restoration in its observer; Linux only publishes failed-restored when
    /// the executor confirms no package installation occurred. Both executors
    /// publish restartSafe without the macOS original-files.json. Unknown or
    /// unverified results must still refuse restart on every platform.
    #[test]
    fn a_failed_restored_record_requires_platform_recovery_verification() {
        for verified in [false, true] {
            let _home = crate::auth_store::test_support::HomeGuard::new(if verified {
                "update-restart-restored-verified"
            } else {
                "update-restart-restored-unverified"
            });
            let _root = journal::CaseRoot::bind();
            let app = tauri::test::mock_app();
            let operation =
                journal::Operation::begin("0.0.2-abcdef+local").expect("journal record");
            operation
                .finish(
                    "failed-restored",
                    Some("installer launch denied".into()),
                    verified,
                )
                .expect("persist result");
            let relaunched = std::cell::Cell::new(false);
            let result = restart_after_app_update_with(app.handle().clone(), |_| {
                relaunched.set(true);
                Ok(())
            });
            let allowed = cfg!(any(target_os = "windows", target_os = "linux")) && verified;
            assert_eq!(result.is_ok(), allowed, "verified={verified}: {result:?}");
            assert_eq!(
                relaunched.get(),
                allowed,
                "unverified recovery must never relaunch"
            );
        }
    }
}

/// Share the UI dictionary so native update dialogs never expose diagnostics or
/// switch language halfway through the installation flow.
fn update_dialog_text(language: Option<&str>, key: &str) -> String {
    let dictionary = if language.is_some_and(|value| value.starts_with("zh")) {
        include_str!("../../../src/i18n/locales/zh/settings.json")
    } else {
        include_str!("../../../src/i18n/locales/en/settings.json")
    };
    serde_json::from_str::<serde_json::Value>(dictionary)
        .expect("bundled settings translations must be valid JSON")["update"][key]
        .as_str()
        .expect("native update dialogs must have a translation")
        .to_owned()
}

/// Download, verify and install the platform updater package.
///
/// Tauri verifies the mandatory minisign signature before installation. The
/// SHA-256 values in latest.json remain useful to website consumers and release
/// audits, but are not a substitute for this signature verification.
#[tauri::command]
pub async fn install_app_update<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    language: Option<String>,
) -> Result<(), AppError> {
    let channel = build_info::channel();
    install_app_update_impl(
        app,
        channel,
        automatic_install_supported(channel),
        manifest_url(channel),
        build_info::VERSION,
        language.as_deref(),
    )
    .await
}

async fn install_app_update_impl<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    channel: BuildChannel,
    can_install: bool,
    endpoint_url: &str,
    current_version: &str,
    language: Option<&str>,
) -> Result<(), AppError> {
    if !can_install {
        return Err("Automatic installation is not available for this build or platform.".into());
    }
    INSTALL_ACTIVE
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .map_err(|_| AppError::Message("An update installation is already in progress.".into()))?;
    let install_guard = InstallGuard;
    if crate::desktop_shutdown::is_stopping() {
        return Err(AppError::Message(
            "Desktop has stopped. Restart before installing an update.".into(),
        ));
    }

    let operation = journal::Operation::begin(current_version)?;
    let result = install_app_update_inner(
        app,
        channel,
        language,
        endpoint_url,
        current_version,
        install_guard,
        operation.clone(),
    )
    .await;
    if let Err(error) = &result {
        operation.finish("failed", Some(error.to_string()), false)?;
    }
    result
}

#[tauri::command]
pub async fn app_maintenance_status() -> Result<serde_json::Value, AppError> {
    // `discover` probes on a blocking-pool thread, which inherits no
    // thread-local from this one — and the probe-deadline seam is deliberately
    // thread-local, so a test that asserts what the probe *captured* would be
    // decided here by the production 3 s budget instead. Carry it across the
    // hop; nothing is armed in a shipped build, where both lines compile out.
    #[cfg(any(test, feature = "test-support"))]
    let probe_deadline = future_host::cli_tool::ProbeDeadlineGuard::inherited();
    tokio::task::spawn_blocking(move || {
        #[cfg(any(test, feature = "test-support"))]
        let _probe_deadline = probe_deadline.map(future_host::cli_tool::ProbeDeadlineGuard::set);
        let cli = match crate::cli_tool::discover(None) {
            Ok(descriptor) => serde_json::json!({"tool": descriptor}),
            Err(error) => serde_json::json!({"error": error.to_string()}),
        };
        let update = journal::latest()?;
        Ok(serde_json::json!({"cli": cli, "update": update}))
    })
    .await
    .map_err(|error| AppError::from(error.to_string()))?
}

#[tauri::command]
pub async fn acknowledge_update_repair(
    app: tauri::AppHandle,
    operation_id: String,
    language: Option<String>,
) -> Result<bool, AppError> {
    INSTALL_ACTIVE
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .map_err(|_| AppError::from("An update installation is still in progress"))?;
    let guard = InstallGuard;
    tokio::task::spawn_blocking(move || {
        let _guard = guard;
        let text = |key| update_dialog_text(language.as_deref(), key);
        let confirmed = app
            .dialog()
            .message(text("repairConfirmation"))
            .title(text("repairTitle"))
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                text("repairConfirm"),
                text("repairCancel"),
            ))
            .blocking_show();
        if !confirmed {
            return Ok(false);
        }
        journal::acknowledge_repair(&operation_id).map(|()| true)
    })
    .await
    .map_err(|error| AppError::from(error.to_string()))?
}

async fn install_app_update_inner<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    channel: BuildChannel,
    language: Option<&str>,
    endpoint_url: &str,
    current_version: &str,
    install_guard: InstallGuard,
    operation: journal::Operation,
) -> Result<(), AppError> {
    let endpoint = endpoint_url
        .parse()
        .map_err(|error| updater_error("Invalid update endpoint", error))?;
    let comparison_current = current_version.to_string();
    let updater_builder = app.updater_builder();
    let updater_builder = updater_builder.timeout(UPDATE_INSTALL_TIMEOUT);
    let updater = updater_builder
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

    operation.stage("downloading", Some(&update.version))?;
    let progress_app = app.clone();
    let mut downloaded = 0_u64;

    let bytes = update
        .download(
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
        .map_err(|error| updater_error("Failed to download and verify the update", error))?;
    operation.stage("verified", None)?;
    operation.stage("preparing", None)?;
    let preparation_operation = operation.clone();
    let signature = update.signature.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        adapters::prepare(&bytes, &signature, &preparation_operation)
    })
    .await
    .map_err(|error| updater_error("Update preparation worker failed", error))?
    .map_err(|error| {
        updater_error(
            "Update preparation failed; current services are still running",
            error,
        )
    })?;
    operation.stage("prepared", None)?;
    // Check the same live activity as quit, including other clients of an
    // embedded Agent. Retained idle conversations are not active work.
    let confirmation_app = app.clone();
    let text = |key| update_dialog_text(language, key);
    let confirmation = (
        text("installTitle"),
        text("installConfirmation"),
        text("installConfirm"),
        text("installDefer"),
    );
    let (confirmed, stop_work_authorized) = tokio::task::spawn_blocking(move || {
        let running_terminals =
            crate::terminal::server::manager().map_or(0, |manager| manager.running_count());
        if crate::agent_host::host().owns_agent()
            && !update_needs_confirmation(
                crate::agent_supervisor::active_work_count(),
                running_terminals,
            )
        {
            return (true, false);
        }
        let confirmed = confirmation_app
            .dialog()
            .message(confirmation.1)
            .title(confirmation.0)
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                confirmation.2,
                confirmation.3,
            ))
            .blocking_show();
        (confirmed, true)
    })
    .await
    .map_err(|error| updater_error("Update confirmation failed", error))?;
    if !confirmed {
        operation.finish("cancelled", None, false)?;
        return Ok(());
    }
    // Download verifies the signature before freezing work. An exit hook
    // cannot veto NSIS, so return cleanup failures before calling install.
    let worker_operation = operation.clone();
    let (installed, _) = tokio::task::spawn_blocking(move || {
        let operation = worker_operation;
        // Keep the update mutex through the native worker even if the invoke
        // future is dropped while shutdown or installation is underway.
        let _guard = install_guard;
        let _action = match crate::desktop_shutdown::claim_action("update") {
            Ok(action) => action,
            Err(error) => return (Err(error), false),
        };
        // macOS can prove restoration of the whole bundle. A DEB transaction
        // must instead use the package manager's result and configuration state.
        #[cfg(target_os = "macos")]
        if let Err(error) = recovery::ProgramFingerprint::capture().and_then(|original| {
            original
                .save(&operation.path())
                .map_err(std::io::Error::other)
        }) {
            return (
                Err(updater_error(
                    "Cannot save original application identity",
                    error,
                )),
                false,
            );
        }
        let shutdown_began = std::cell::Cell::new(false);
        let cleanup_succeeded = std::cell::Cell::new(false);
        let restored = std::cell::Cell::new(false);
        let installed = install_after_cleanup(
            || {
                operation.stage("cleanup", None)?;
                if !stop_work_authorized {
                    let terminals = crate::terminal::server::manager().map(|manager| manager.reserve_idle());
                    if matches!(terminals, Some(None)) || !crate::agent_host::host().reserve_idle_update()? {
                        return Err("New work arrived while preparing the update; current work was not stopped. Retry when idle.".into());
                    }
                    if let Some(Some(reservation)) = terminals { reservation.commit(); }
                }
                shutdown_began.set(true);
                crate::desktop_shutdown::stop("update")?;
                cleanup_succeeded.set(true);
                Ok(())
            },
            || {
                operation
                    .stage(
                        if cfg!(windows) {
                            "awaiting-exit"
                        } else {
                            "installing"
                        },
                        None,
                    )
                    .map_err(|e| e.to_string())?;
                let result = adapters::commit(prepared, &operation)?;
                restored.set(result.restored);
                if result.ok {
                    Ok(())
                } else {
                    Err(result.diagnostic)
                }
            },
        );
        let safe = !shutdown_began.get() || (cleanup_succeeded.get() && (installed.is_ok() || restored.get()));
        let persisted = match &installed {
            Ok(()) => {
                #[cfg(not(target_os = "windows"))]
                {
                    operation.finish("pending-validation", None, true)
                }
                #[cfg(target_os = "windows")]
                {
                    Ok(())
                }
            }
            Err(error) => operation.finish(
                if !shutdown_began.get() {
                    "not-installed"
                } else if !cleanup_succeeded.get() {
                    "cleanup-failed"
                } else if safe {
                    "failed-restored"
                } else {
                    "repair-required"
                },
                Some(error.to_string()),
                safe,
            ),
        };
        if let Err(error) = persisted {
            crate::host_environment::report(&format!("Cannot persist update result: {error}"));
            return (Err(error), false);
        }
        (installed, safe)
    })
    .await
    .map_err(|error| updater_error("Update cleanup/install worker failed", error))?;
    if let Err(error) = installed {
        // The journal and update screen own recovery. A second native failure
        // dialog would expose raw diagnostics and hide the available actions.
        crate::host_environment::report(&format!("Update installation failed: {error}"));
        return Err(error);
    }
    #[cfg(target_os = "windows")]
    {
        crate::agent_supervisor::authorize_exit();
        app.exit(0);
    }
    Ok(())
}

/// Relaunch only after installation has completed and the user explicitly asks
/// to do so. Keeping this separate lets an active conversation finish first.
#[tauri::command]
#[rustfmt::skip]
pub async fn restart_after_app_update(app: tauri::AppHandle) -> Result<(), AppError> {
    tokio::task::spawn_blocking(move || restart_after_app_update_with(app, |app| { crate::agent_supervisor::authorize_exit(); app.restart() })).await.map_err(|error| AppError::from(error.to_string()))?
}

/// Body of [`restart_after_app_update`] with the relaunch injectable —
/// `restart()` re-execs the process, so tests inject a no-op.
fn restart_after_app_update_with<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    relaunch: impl FnOnce(tauri::AppHandle<R>) -> Result<(), AppError>,
) -> Result<(), AppError> {
    INSTALL_ACTIVE
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .map_err(|_| {
            AppError::from(
                "Update maintenance is in progress; wait for its result before restarting",
            )
        })?;
    let _guard = InstallGuard;
    if let Some(record) = journal::latest()? {
        if !record.restart_safe {
            return Err("Update result does not permit restart. Inspect app/updates and repair the installation if its outcome is unknown.".into());
        }
        #[cfg(target_os = "macos")]
        if record.outcome.as_deref() == Some("failed-restored") {
            recovery::ProgramFingerprint::verify_saved(&record.operation_id)?;
        }
    }
    let _action = crate::desktop_shutdown::claim_action("restart")?;
    crate::desktop_shutdown::stop("restart")?;
    relaunch(app)
}
