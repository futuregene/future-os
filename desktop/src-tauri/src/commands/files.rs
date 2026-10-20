//! GUI filesystem IPC, clipboard, asset protocol and OS opening adapters.
pub(crate) use future_host::files::ensure_path_allowed;
#[cfg(feature = "gui")]
use future_host::files::validate_image_preview_path;
pub use future_host::files::{
    AttachmentInfo, DirEntry, ResolvedPreviewLink, SavedAttachment, TextFilePreview,
};
use std::path::Path;
#[cfg(feature = "gui")]
use std::path::PathBuf;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePreviewAsset {
    path: String,
    version: String,
}

#[cfg(feature = "gui")]
fn native_clipboard_file_paths() -> Vec<String> {
    clipboard_file_paths_with(read_clipboard_file_list)
}

/// Read the OS clipboard's file list. Isolated from the projection below because
/// `arboard` needs a live desktop session: on headless CI it errors out, and on
/// a developer machine it reads *that* machine's pasteboard, so nothing about
/// its content can be asserted.
#[cfg(feature = "gui")]
fn read_clipboard_file_list() -> Vec<PathBuf> {
    // The OS clipboard formats are intentionally not decoded in the webview:
    // Finder may use a file-reference URL (`file:///.file/id=...`), Explorer
    // uses CF_HDROP, and Linux file managers commonly publish URI lists.
    // `arboard` resolves each platform format to actual filesystem paths.
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.get().file_list())
        .unwrap_or_default()
}

/// Keep only the absolute file paths the clipboard yielded, as display strings.
#[cfg(feature = "gui")]
fn clipboard_file_paths_with(read: impl FnOnce() -> Vec<PathBuf>) -> Vec<String> {
    read()
        .into_iter()
        .filter(|path| path.is_absolute())
        .map(|path| path.display().to_string())
        .collect()
}

/// Return native resource-manager paths from the current clipboard. Callers
/// still inspect each returned path before use.
#[cfg_attr(feature = "gui", tauri::command)]
#[cfg(feature = "gui")]
pub fn read_native_clipboard_file_paths() -> Vec<String> {
    native_clipboard_file_paths()
}

/// Validate one local image path and expose exactly that canonical file through
/// Tauri's asset protocol. The WebView then reads the bytes directly instead of
/// receiving a 1.33x Base64 string over JSON IPC.
#[tauri::command]
#[cfg(feature = "gui")]
pub fn prepare_image_preview(
    app: tauri::AppHandle,
    path: String,
) -> Result<ImagePreviewAsset, crate::AppError> {
    prepare_image_preview_with(&app, &path)
}

#[cfg(feature = "gui")]
fn prepare_image_preview_with<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    path: &str,
) -> Result<ImagePreviewAsset, crate::AppError> {
    use tauri::Manager;

    let (canonical, size) = validate_image_preview_path(path)?;
    app.asset_protocol_scope()
        .allow_file(&canonical)
        .map_err(|error| format!("failed to authorize image preview path: {error}"))?;
    Ok(ImagePreviewAsset {
        path: canonical.display().to_string(),
        // The asset URL is path-based. Give every preparation a fresh version so
        // reopening a changed file never reuses an old WebView cache entry.
        version: future_host::files::image_preview_version(size),
    })
}

#[cfg_attr(feature = "gui", tauri::command)]
pub fn open_path(path: String) -> Result<(), crate::AppError> {
    open_path_with(&path, open_path_with_system)
}

/// Validation + opener, with the OS layer injectable so the happy path and
/// the OS-error path are testable without launching a GUI app.
fn open_path_with(
    path: &str,
    opener: impl Fn(&str) -> Result<(), crate::AppError>,
) -> Result<(), crate::AppError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path cannot be empty.".to_string().into());
    }

    ensure_path_allowed(Path::new(trimmed))?;
    opener(trimmed)
}

/// Open an http(s) or mailto URL in the user's default handler. The scheme is
/// restricted to http/https/mailto so this can't be used to launch arbitrary
/// local handlers (`file:`, custom app schemes, …) via a crafted url.
#[cfg_attr(feature = "gui", tauri::command)]
pub fn open_external_url(url: String) -> Result<(), crate::AppError> {
    open_external_url_with(&url, open_path_with_system)
}

/// Validation + opener with the OS layer injectable (see [`open_path_with`]).
fn open_external_url_with(
    url: &str,
    opener: impl Fn(&str) -> Result<(), crate::AppError>,
) -> Result<(), crate::AppError> {
    let trimmed = url.trim();
    let normalized = trimmed.to_ascii_lowercase();
    if !(normalized.starts_with("http://")
        || normalized.starts_with("https://")
        || normalized.starts_with("mailto:"))
    {
        return Err("Only http(s) or mailto URLs can be opened."
            .to_string()
            .into());
    }

    opener(trimmed)
}

/// Hand the path/URL to the OS default handler via the `open` crate
/// (`open`/`xdg-open`/ShellExecuteW). Never route through `cmd /C start`:
/// cmd re-parses the argument, so `&`/`^`/`%VAR%` in an agent-produced path
/// would be interpreted — an injection vector, not just a broken open.
#[cfg(target_os = "macos")]
fn open_path_with_system(path: &str) -> Result<(), crate::AppError> {
    open::that(path).or_else(|_| fallback_open_text(path))
}

/// The `open -t` fallback: spawn + classify, split so the status classifier is
/// pure (spawn itself is an OS seam).
#[cfg(target_os = "macos")]
fn fallback_open_text(path: &str) -> Result<(), crate::AppError> {
    let status = open_text_command(path)
        .status()
        .map_err(|e| format!("Failed to open: {e}"))?;
    open_t_status_to_result(status.success(), path)
}

#[cfg(target_os = "macos")]
fn open_text_command(path: &str) -> std::process::Command {
    let mut command = std::process::Command::new("open");
    command.arg("-t").arg("--").arg(path);
    command
}

/// Pure classifier for the `open -t` fallback's exit status.
#[cfg(target_os = "macos")]
fn open_t_status_to_result(success: bool, path: &str) -> Result<(), crate::AppError> {
    if success {
        Ok(())
    } else {
        Err(format!("Failed to open {path}: open -t exited with an error").into())
    }
}

#[cfg(not(target_os = "macos"))]
fn open_path_with_system(path: &str) -> Result<(), crate::AppError> {
    open::that(path).map_err(|_| format!("Failed to open: {path}").into())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub fn inspect_attachment(path: String) -> Result<AttachmentInfo, crate::AppError> {
    future_host::files::inspect_attachment(path)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub fn validate_image_attachment(path: String) -> Result<(), crate::AppError> {
    future_host::files::validate_image_attachment(path)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub fn generate_image_thumbnail(
    thread_id: String,
    source_path: String,
) -> Result<String, crate::AppError> {
    future_host::files::generate_image_thumbnail(thread_id, source_path)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub fn import_ephemeral_attachment(
    thread_id: String,
    source_path: String,
    name: String,
) -> Result<String, crate::AppError> {
    future_host::files::import_ephemeral_attachment(thread_id, source_path, name)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub fn delete_temp_attachment(path: String) -> Result<(), crate::AppError> {
    future_host::files::delete_temp_attachment(path)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub fn list_directory(path: String) -> Result<Vec<DirEntry>, crate::AppError> {
    future_host::files::list_directory(path)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub fn resolve_preview_link_path(
    base_file: String,
    target: String,
) -> Result<ResolvedPreviewLink, crate::AppError> {
    future_host::files::resolve_preview_link_path(base_file, target)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub fn read_text_file_preview(
    path: String,
    max_bytes: Option<usize>,
) -> Result<TextFilePreview, crate::AppError> {
    future_host::files::read_text_file_preview(path, max_bytes)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub fn export_artifact_file(
    destination_path: String,
    source_path: Option<String>,
    content: Option<String>,
) -> Result<(), crate::AppError> {
    future_host::files::export_artifact_file(destination_path, source_path, content)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub fn save_pasted_image(
    bytes: Vec<u8>,
    extension: Option<String>,
) -> Result<SavedAttachment, crate::AppError> {
    future_host::files::save_pasted_image(bytes, extension)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub fn save_pasted_file(bytes: Vec<u8>, name: String) -> Result<SavedAttachment, crate::AppError> {
    future_host::files::save_pasted_file(bytes, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "gui")]
    use std::fs;
    #[cfg(target_os = "macos")]
    #[test]
    fn text_opener_treats_option_like_paths_as_operands() {
        let command = open_text_command("-aCalculator");
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["-t", "--", "-aCalculator"]);
    }

    #[cfg(feature = "gui")]
    #[test]
    fn clipboard_file_paths_keep_only_absolute_entries() {
        let absolute = std::env::temp_dir().join("future-clipboard-absolute.txt");
        let relative = PathBuf::from("relative/pasted.txt");
        assert_eq!(
            clipboard_file_paths_with(|| vec![relative, absolute.clone()]),
            vec![absolute.display().to_string()]
        );
    }

    #[cfg(feature = "gui")]
    #[test]
    fn the_native_clipboard_reader_only_reports_absolute_paths() {
        // The pasteboard belongs to the machine running the test, so this pins
        // the projection's contract — every reported path is absolute — and
        // never the clipboard's content.
        let paths = read_native_clipboard_file_paths();
        assert!(paths.iter().all(|path| Path::new(path).is_absolute()));
    }

    #[cfg(feature = "gui")]
    #[test]
    fn prepare_image_preview_authorizes_only_the_canonical_file() {
        use tauri::Manager;

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = root.join("source.png");
        let other = root.join("other.png");
        fs::write(&source, b"image bytes").unwrap();
        fs::write(&other, b"other image bytes").unwrap();
        let app = tauri::test::mock_app();

        let asset =
            prepare_image_preview_with(app.handle(), &source.display().to_string()).unwrap();
        let refreshed =
            prepare_image_preview_with(app.handle(), &source.display().to_string()).unwrap();
        let canonical = source.canonicalize().unwrap();
        assert_eq!(asset.path, canonical.display().to_string());
        assert_ne!(asset.version, refreshed.version);
        assert!(app.asset_protocol_scope().is_allowed(&canonical));
        assert!(!app.asset_protocol_scope().is_allowed(other));
    }

    #[test]
    fn open_path_and_url_validate_before_reaching_the_os() {
        assert!(open_path("   ".into()).is_err());
        assert!(open_external_url("file:///etc/passwd".into()).is_err());
        assert!(open_external_url("ftp://x".into()).is_err());
    }

    #[test]
    fn open_path_with_injects_the_opener_for_both_arms() {
        let _home = crate::auth_store::test_support::HomeGuard::new("files_open_seam");
        assert!(open_path_with("   ", |_| unreachable!()).is_err());
        assert!(open_path_with("/tmp/ok", |_| Ok(())).is_ok());
        assert!(open_path_with("/tmp/err", |_| Err("os failed".to_string().into())).is_err());
        // Credential files stay blocked even with an injected opener.
        let home = std::env::var("HOME").expect("test home");
        let agent_dir = std::path::Path::new(&home).join(".future/agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("auth.json"), "{}").unwrap();
        let cred = agent_dir.join("auth.json");
        assert!(open_path_with(cred.to_str().unwrap(), |_| unreachable!()).is_err());
    }

    #[test]
    fn open_external_url_with_injects_the_opener_for_both_arms() {
        assert!(open_external_url_with("https://ok.example/", |_| Ok(())).is_ok());
        assert!(open_external_url_with("mailto:x@example.com", |_| Ok(())).is_ok());
        assert!(
            open_external_url_with("https://x", |_| Err("os failed".to_string().into())).is_err()
        );
        assert!(open_external_url_with("ftp://x", |_| unreachable!()).is_err());
        assert!(open_external_url_with("   ", |_| unreachable!()).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn open_t_status_classifier_maps_both_arms() {
        assert!(open_t_status_to_result(true, "x").is_ok());
        assert!(open_t_status_to_result(false, "x").is_err());
    }

    #[test]
    fn open_path_runs_the_real_os_fallback_chain() {
        let _home = crate::auth_store::test_support::HomeGuard::new("files_open_os");
        // A path whose PARENT does not exist: `open` and `open -t` both fail
        // fast without launching any GUI app, so the fallback chain runs
        // end-to-end and returns the OS error.
        let ghost = "/definitely-not-a-real-dir-cov100/file.txt";
        let err = open_path(ghost.to_string()).unwrap_err();
        assert!(!err.to_string().is_empty());
    }
}
