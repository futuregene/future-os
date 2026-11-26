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

/// Read an image from the current clipboard and persist it as a pasted attachment.
///
/// WebKitGTK dispatches a `paste` event whose `DataTransfer` is entirely empty
/// when the clipboard holds only an image, so the WebView never sees the bytes
/// and the composer's own paste handler has nothing to attach on Linux. The
/// pasteboard itself still has them, so the WebView falls back to this command
/// when a paste carried neither text nor files.
#[cfg_attr(feature = "gui", tauri::command)]
#[cfg(feature = "gui")]
pub fn read_native_clipboard_image() -> Option<SavedAttachment> {
    clipboard_image_attachment_with(read_clipboard_image_png)
}

/// Encode the clipboard image as PNG and hand it to the shared pasted-image
/// persistence (which owns the size cap and the temp-directory layout).
#[cfg(feature = "gui")]
fn clipboard_image_attachment_with(
    read: impl FnOnce() -> Option<Vec<u8>>,
) -> Option<SavedAttachment> {
    let png = read()?;
    future_host::files::save_pasted_image(png, Some("png".to_string())).ok()
}

/// Decode the OS clipboard image into PNG bytes. `arboard` yields raw RGBA, so
/// the encode happens here rather than in the WebView, which never received it.
#[cfg(all(feature = "gui", target_os = "linux"))]
fn read_clipboard_image_png() -> Option<Vec<u8>> {
    let image = arboard::Clipboard::new().ok()?.get_image().ok()?;
    let rgba = image::RgbaImage::from_raw(
        image.width as u32,
        image.height as u32,
        image.bytes.into_owned(),
    )?;
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut png, image::ImageFormat::Png)
        .ok()?;
    Some(png.into_inner())
}

/// WKWebView and WebView2 hand a pasted clipboard image to the page as a
/// `File`, so the fallback never runs there and those graphs skip arboard's
/// image support (see the `image-data` feature in Cargo.toml).
#[cfg(all(feature = "gui", not(target_os = "linux")))]
fn read_clipboard_image_png() -> Option<Vec<u8>> {
    None
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

/// Reveal a path in the OS file manager: the containing folder opens with the
/// item selected (Windows/macOS), or the folder itself opens when it *is* a
/// folder.
///
/// This is what "Show in folder" means to a user who is looking at a file name
/// and wants to find the file, not to open it in its default app. `open_path`
/// would launch the editor/PDF viewer instead, which is a different question.
#[cfg_attr(feature = "gui", tauri::command)]
pub fn reveal_in_file_manager(path: String) -> Result<(), crate::AppError> {
    reveal_in_file_manager_with(&path, reveal_with_system, open_folder_with_system)
}

/// Validation + revealer, with the OS layer injectable so both arms are
/// testable without opening a window.
///
/// Same credential guard as [`open_path_with`]: revealing a credential file's
/// folder would show a directory the webview must not lead a user to, and a
/// window left standing on it is the one thing a later screenshot could capture.
///
/// A path that no longer exists never reaches the OS as-is: `explorer /select`
/// on a missing item answers by opening Documents, which says nothing about the
/// link and reads as the same bug the link arrived to report. The link falls
/// back to the deepest folder that still exists, and opens nothing at all when
/// even that is gone — no window beats the wrong one.
fn reveal_in_file_manager_with(
    path: &str,
    mut reveal: impl FnMut(&Path) -> Result<(), crate::AppError>,
    mut open_folder: impl FnMut(&Path) -> Result<(), crate::AppError>,
) -> Result<(), crate::AppError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path cannot be empty.".to_string().into());
    }
    let target = Path::new(trimmed);
    ensure_path_allowed(target)?;
    match nearest_existing(target) {
        // The file is there: select it, exactly as before.
        Some(existing) if existing == target => reveal(target),
        Some(folder) => {
            // The walk up leaves the requested path, so what replaces it is
            // checked on its own too (a link may aim *through* a credential
            // file, whose own name the first guard let pass).
            ensure_path_allowed(folder)?;
            open_folder(folder)
        }
        None => {
            Err(format!("Nothing to reveal for {trimmed}: no part of that path exists.").into())
        }
    }
}

/// The deepest ancestor of `path` that exists, or `None` when not one part of
/// it does (an unmounted drive, a folder moved away with everything under it).
///
/// `Path::ancestors` starts at the path itself, so an existing file or folder
/// answers with itself — only a missing subject climbs.
fn nearest_existing(path: &Path) -> Option<&Path> {
    path.ancestors().find(|candidate| candidate.exists())
}

/// Open a folder in the OS file manager; the fallback for a revealed item that
/// is gone.
fn open_folder_with_system(folder: &Path) -> Result<(), crate::AppError> {
    let target = native_separators(folder);
    open::that(&target).map_err(|error| format!("Failed to open {target}: {error}").into())
}

/// The path in the spelling the Windows shell reads back.
///
/// Windows takes `/` in most of its path APIs, but `explorer` does not: handed
/// `C:/Users/me/pic.png` it opens Documents (or Desktop) instead — the same
/// wrong-folder symptom as a missing file, from a target that is perfectly
/// present. That spelling is not exotic: it is exactly how a model writes the
/// link for a file it just saved (`[pic.png](<C:/Users/me/pic.png>)`), so the
/// reveal has to normalize before it hands the path over. Everywhere else `/`
/// *is* the separator and the path is already what the shell expects.
#[cfg(target_os = "windows")]
fn native_separators(path: &Path) -> String {
    path.display().to_string().replace('/', "\\")
}

#[cfg(not(target_os = "windows"))]
fn native_separators(path: &Path) -> String {
    path.display().to_string()
}

/// The platform arms of [`reveal_in_file_manager`]. Built from `explorer`/`open`
/// rather than a shell so a path with `&`, `^` or `%VAR%` is passed as one
/// argument and cannot be reinterpreted.
#[cfg(target_os = "windows")]
fn reveal_with_system(path: &Path) -> Result<(), crate::AppError> {
    use std::os::windows::process::CommandExt;

    // `explorer` returns exit code 1 even on success, so its status is not
    // evidence either way; only a failure to spawn is.
    std::process::Command::new("explorer")
        .raw_arg(select_argument(path))
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Failed to reveal {}: {error}", path.display()).into())
}

/// The `/select,<path>` fragment Explorer parses, with the quotes around the
/// *path* rather than around the whole switch.
///
/// `Command::arg` quotes its argument when it contains a space, producing
/// `"/select,C:\a b.pdf"`, which Explorer does not parse: it opens Documents
/// instead — a file whose name has a space seemed to reveal nowhere near itself.
/// The documented form is `/select,"C:\a b.pdf"`, and only a raw command-line
/// fragment can express it. The path is never passed through a shell, so the
/// only escaping that matters is the closing quote: a trailing separator would
/// escape it, hence the trim (a bare `C:\` root keeps its separator).
#[cfg(target_os = "windows")]
fn select_argument(path: &Path) -> String {
    // The switch also needs the native spelling even when the link did not use
    // it — see [`native_separators`].
    let mut target = native_separators(path);
    while target.len() > 3 && (target.ends_with('\\') || target.ends_with('/')) {
        target.pop();
    }
    format!("/select,\"{target}\"")
}

/// macOS selects the item inside Finder; `-R` is a no-op "reveal" that never
/// launches the file's own application.
#[cfg(target_os = "macos")]
fn reveal_with_system(path: &Path) -> Result<(), crate::AppError> {
    std::process::Command::new("open")
        .arg("-R")
        .arg("--")
        .arg(path)
        .status()
        .map_err(|error| format!("Failed to reveal {}: {error}", path.display()))?
        .success()
        .then_some(())
        .ok_or_else(|| format!("Failed to reveal {}", path.display()).into())
}

/// No portable "select this item" on Linux desktops, so open the containing
/// folder — the same thing the tree's own Open Folder button does.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn reveal_with_system(path: &Path) -> Result<(), crate::AppError> {
    let folder = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    open::that(folder.unwrap_or(path))
        .map_err(|error| format!("Failed to reveal {}: {error}", path.display()).into())
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
    fn a_clipboard_image_becomes_a_pasted_png_attachment() {
        let rgba = image::RgbaImage::from_raw(2, 2, vec![0u8; 16]).expect("rgba buffer");
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(rgba)
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("encode png");
        let saved = clipboard_image_attachment_with(|| Some(png.into_inner())).expect("saved");
        // `SavedAttachment` keeps its fields private to the host crate; the JSON
        // it serializes to is the same shape the WebView receives.
        let value = serde_json::to_value(&saved).expect("serialized attachment");
        assert!(value["name"].as_str().unwrap_or_default().ends_with(".png"));
        assert!(Path::new(value["path"].as_str().unwrap_or_default()).is_file());
        // No clipboard image (text-only or headless desktop) attaches nothing.
        assert!(clipboard_image_attachment_with(|| None).is_none());
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

    /// Explorer parses `/select,"<path>"` and not `"/select,<path>"`: with the
    /// switch inside the quotes it falls back to Documents, so "show in folder"
    /// on a file whose name has a space opened the wrong folder entirely.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_reveal_quotes_the_path_rather_than_the_switch() {
        assert_eq!(
            select_argument(Path::new(r"C:\a b\c.pdf")),
            "/select,\"C:\\a b\\c.pdf\""
        );
        // A trailing separator would escape the closing quote.
        assert_eq!(
            select_argument(Path::new(r"C:\a b\")),
            "/select,\"C:\\a b\""
        );
    }

    /// Explorer answers a forward-slash target by opening Documents (verified
    /// against a real file: `"/select,C:/…/pic.png"` lands on Documents, the
    /// backslash spelling of the same path selects the file). A model writes
    /// the link for a file it just saved the POSIX way, so the spelling has to
    /// be normalized before the switch is built.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_reveal_normalizes_forward_slashes() {
        assert_eq!(
            select_argument(Path::new("C:/Users/me/pic.png")),
            "/select,\"C:\\Users\\me\\pic.png\""
        );
        // Mixed spellings and a trailing forward slash normalize the same way,
        // and the fallback folder is opened with native separators too.
        assert_eq!(
            native_separators(Path::new("C:/Users/me\\pics/")),
            "C:\\Users\\me\\pics\\"
        );
    }

    #[test]
    fn reveal_path_validates_before_reaching_the_os() {
        let _home = crate::auth_store::test_support::HomeGuard::new("files_reveal_seam");
        assert!(
            reveal_in_file_manager_with("   ", |_| unreachable!(), |_| unreachable!()).is_err()
        );

        // An item that exists is still selected in place: the fallback must not
        // hijack the ordinary case.
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("present.txt");
        fs::write(&file, b"x").unwrap();
        let mut revealed = None;
        reveal_in_file_manager_with(
            file.to_str().unwrap(),
            |path| {
                revealed = Some(path.to_path_buf());
                Ok(())
            },
            |_| panic!("an existing file must not open its folder"),
        )
        .expect("reveal");
        assert_eq!(revealed.as_deref(), Some(file.as_path()));

        assert!(reveal_in_file_manager_with(
            file.to_str().unwrap(),
            |_| Err("os failed".to_string().into()),
            |_| unreachable!()
        )
        .is_err());

        // The credential guard holds here too: a window standing on the folder
        // of `auth.json` is exposure the raw-bytes guard exists to prevent.
        let home = std::env::var("HOME").expect("test home");
        let agent_dir = std::path::Path::new(&home).join(".future/agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("auth.json"), "{}").unwrap();
        let cred = agent_dir.join("auth.json");
        assert!(reveal_in_file_manager_with(
            cred.to_str().unwrap(),
            |_| unreachable!(),
            |_| unreachable!()
        )
        .is_err());
    }

    /// `explorer /select` on a path that no longer exists answers by opening
    /// Documents, so a link to a moved file looked like a bug in the link. The
    /// fallback lands on the deepest folder that still exists instead.
    #[test]
    fn reveal_falls_back_to_the_nearest_existing_folder() {
        let _home = crate::auth_store::test_support::HomeGuard::new("files_reveal_gone");
        let directory = tempfile::tempdir().unwrap();
        let kept = directory.path().join("kept");
        fs::create_dir(&kept).unwrap();
        let gone = kept.join("moved-away").join("report.pdf");

        let mut opened = None;
        reveal_in_file_manager_with(
            gone.to_str().unwrap(),
            |_| panic!("a missing file must not be revealed as-is"),
            |folder| {
                opened = Some(folder.to_path_buf());
                Ok(())
            },
        )
        .expect("fallback");
        assert_eq!(opened.as_deref(), Some(kept.as_path()));

        // A missing file whose own folder still exists lands one level up from
        // the path, which is the ordinary case for a file that was deleted.
        let deleted = kept.join("deleted.pdf");
        let mut opened = None;
        reveal_in_file_manager_with(
            deleted.to_str().unwrap(),
            |_| panic!("a missing file must not be revealed as-is"),
            |folder| {
                opened = Some(folder.to_path_buf());
                Ok(())
            },
        )
        .expect("fallback");
        assert_eq!(opened.as_deref(), Some(kept.as_path()));
    }

    /// Nothing left to open means no window: `explorer` would answer a missing
    /// path with Documents, which is the failure this whole fallback replaces.
    #[test]
    fn reveal_opens_nothing_when_no_part_of_the_path_exists() {
        let _home = crate::auth_store::test_support::HomeGuard::new("files_reveal_none");
        // Relative and absent all the way up, so `ancestors` yields no existing
        // candidate on either platform — a name no fixture directory carries.
        let ghost = std::path::Path::new("futureos-no-such-root-cov100/gone/report.pdf");
        assert_eq!(nearest_existing(ghost), None);

        let err = reveal_in_file_manager_with(
            ghost.to_str().unwrap(),
            |_| panic!("nothing exists to reveal"),
            |_| panic!("nothing exists to open"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("Nothing to reveal"), "{err}");
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

    /// A tiny real PNG: the thumbnail and image-validation commands refuse to
    /// guess from a file extension, so the fixture has to be a decodable image.
    #[cfg(feature = "gui")]
    fn png_bytes() -> Vec<u8> {
        let rgba = image::RgbaImage::from_raw(2, 2, vec![0u8; 16]).expect("rgba buffer");
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(rgba)
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("encode png");
        png.into_inner()
    }

    /// The attachment commands the composer and the transcript actually call,
    /// each against its own fixture. Every assertion reads a property of the
    /// file the caller named, so a wrapper that lost an argument answers about a
    /// different file (or refuses) instead of passing.
    #[cfg(feature = "gui")]
    #[test]
    fn the_attachment_commands_forward_the_paths_they_are_given() {
        let _home = crate::auth_store::test_support::HomeGuard::new("files_attachment_commands");
        crate::store::initialize_app_store().expect("init store");
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".into(),
            title: Some("Attachments".into()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .expect("create thread");
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let note = root.join("note.txt");
        fs::write(&note, b"hello attachment").unwrap();
        let note_path = note.display().to_string();

        let inspected =
            serde_json::to_value(inspect_attachment(note_path.clone()).expect("inspect"))
                .expect("serialize");
        assert_eq!(inspected["isDir"], serde_json::json!(false));
        assert_eq!(inspected["size"], serde_json::json!(16));
        assert_eq!(inspected["isBinary"], serde_json::json!(false));
        // A directory is the *other* classification, so the flag is read from
        // the path rather than fixed.
        let as_dir = serde_json::to_value(
            inspect_attachment(root.display().to_string()).expect("inspect dir"),
        )
        .expect("serialize");
        assert_eq!(as_dir["isDir"], serde_json::json!(true));
        assert_eq!(as_dir["size"], serde_json::json!(0));

        let preview = serde_json::to_value(
            read_text_file_preview(note_path.clone(), Some(5)).expect("preview"),
        )
        .expect("serialize");
        assert_eq!(preview["content"], serde_json::json!("hello"));
        assert_eq!(preview["truncated"], serde_json::json!(true));
        assert_eq!(preview["size"], serde_json::json!(16));

        let entries = list_directory(root.display().to_string()).expect("list directory");
        assert_eq!(
            entries.len(),
            1,
            "the directory holds exactly the note file"
        );
        let entry = serde_json::to_value(&entries[0]).expect("serialize entry");
        assert_eq!(entry["name"], serde_json::json!("note.txt"));
        assert_eq!(entry["isDir"], serde_json::json!(false));

        // A relative target is resolved against the base file the transcript
        // handed over, not against the process cwd.
        let resolved =
            resolve_preview_link_path(note_path.clone(), "linked.png".into()).expect("resolve");
        assert_eq!(
            serde_json::to_value(&resolved).expect("serialize link"),
            serde_json::json!({"name": "linked.png", "path": root.join("linked.png").display().to_string()})
        );

        // The image commands refuse a text file and accept a real PNG.
        let rejected = validate_image_attachment(note_path.clone())
            .expect_err("a text file is not an attachment image");
        assert!(!rejected.to_string().is_empty());
        let image = root.join("dot.png");
        fs::write(&image, png_bytes()).unwrap();
        let image_path = image.display().to_string();
        validate_image_attachment(image_path.clone()).expect("a PNG is an attachment image");

        let thumbnail =
            generate_image_thumbnail(thread.id.clone(), image_path.clone()).expect("thumbnail");
        assert!(Path::new(&thumbnail).is_file(), "{thumbnail}");
        let imported = import_ephemeral_attachment(thread.id.clone(), image_path, "dot.png".into())
            .expect("import");
        assert!(
            imported.ends_with("dot.png"),
            "the copy keeps the requested file name: {imported}"
        );
        assert!(Path::new(&imported).is_file());

        // Inline content and a source-file copy are the two export arms.
        let inline = root.join("inline.txt");
        export_artifact_file(inline.display().to_string(), None, Some("inline!".into()))
            .expect("export inline content");
        assert_eq!(fs::read_to_string(&inline).unwrap(), "inline!");
        let copied = root.join("copied.txt");
        export_artifact_file(copied.display().to_string(), Some(note_path.clone()), None)
            .expect("export a source file");
        assert_eq!(fs::read_to_string(&copied).unwrap(), "hello attachment");

        // Pasted bytes land in the temp attachment directory, and only a file
        // inside it can be deleted again.
        let pasted_image = serde_json::to_value(
            save_pasted_image(png_bytes(), Some("png".into())).expect("save pasted image"),
        )
        .expect("serialize");
        let pasted_image_path = pasted_image["path"].as_str().expect("path").to_string();
        assert!(pasted_image["name"]
            .as_str()
            .expect("name")
            .ends_with(".png"));
        assert!(Path::new(&pasted_image_path).is_file());
        delete_temp_attachment(pasted_image_path.clone()).expect("delete the temp image");
        assert!(!Path::new(&pasted_image_path).exists());
        assert!(
            delete_temp_attachment(pasted_image_path).is_err(),
            "the deletion is guarded to FutureOS's own temp attachments"
        );

        let pasted_file = serde_json::to_value(
            save_pasted_file(b"pasted bytes".to_vec(), "notes.txt".into())
                .expect("save pasted file"),
        )
        .expect("serialize");
        assert_eq!(pasted_file["name"], serde_json::json!("notes.txt"));
        delete_temp_attachment(pasted_file["path"].as_str().expect("path").to_string())
            .expect("delete the pasted file");

        // On the platforms that hand a clipboard image to the page as a `File`
        // this fallback has nothing to read, and the stub says so; Linux is the
        // one that decodes arboard's RGBA. The command must answer on every
        // platform, so it is called unconditionally here.
        let clipboard_image = read_native_clipboard_image();
        #[cfg(not(target_os = "linux"))]
        assert!(
            clipboard_image.is_none(),
            "this platform delivers a clipboard image as a File, not through arboard"
        );
        #[cfg(target_os = "linux")]
        let _ = clipboard_image;
    }
}
