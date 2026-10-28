//! Utility functions — matching Go internal/utils

use std::collections::VecDeque;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::UNIX_EPOCH;

const IMAGE_CACHE_ENTRIES: usize = 32;
const IMAGE_CACHE_BYTES: usize = 64 * 1024 * 1024;
const IMAGE_CACHE_MAX_ENTRY_BYTES: usize = 8 * 1024 * 1024;

type ImageCacheKey = (PathBuf, u128, u64);

#[derive(Default)]
struct ImageUrlCache {
    entries: VecDeque<(ImageCacheKey, String)>,
    bytes: usize,
}

static IMAGE_URL_CACHE: LazyLock<parking_lot::Mutex<ImageUrlCache>> =
    LazyLock::new(Default::default);

fn cached_image_url(key: &ImageCacheKey) -> Option<String> {
    let mut cache = IMAGE_URL_CACHE.lock();
    let index = cache
        .entries
        .iter()
        .position(|(candidate, _)| candidate == key)?;
    let entry = cache.entries.remove(index)?;
    let url = entry.1.clone();
    cache.entries.push_back(entry);
    Some(url)
}

fn remember_image_url(key: ImageCacheKey, url: &str) {
    if url.len() > IMAGE_CACHE_MAX_ENTRY_BYTES {
        return;
    }
    let mut cache = IMAGE_URL_CACHE.lock();
    if let Some(index) = cache
        .entries
        .iter()
        .position(|(candidate, _)| candidate == &key)
    {
        if let Some((_, old)) = cache.entries.remove(index) {
            cache.bytes = cache.bytes.saturating_sub(old.len());
        }
    }
    while !cache.entries.is_empty()
        && (cache.entries.len() >= IMAGE_CACHE_ENTRIES
            || cache.bytes.saturating_add(url.len()) > IMAGE_CACHE_BYTES)
    {
        if let Some((_, old)) = cache.entries.pop_front() {
            cache.bytes = cache.bytes.saturating_sub(old.len());
        }
    }
    cache.bytes = cache.bytes.saturating_add(url.len());
    cache.entries.push_back((key, url.to_owned()));
}

/// Standard UUID v4 for new application-owned identities. Time is stored in
/// record metadata, not embedded in the identity. All 122 random bits remain.
pub fn generate_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Public journal/operation identity uses the same canonical 36-character UUID
/// as database inspection. The storage adapter owns the 16-byte representation.
pub fn generate_entry_id() -> String {
    generate_id()
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

/// Normalize a workspace cwd for storage: trim surrounding whitespace and
/// trailing path separators, but preserve the filesystem root. The root
/// directory "/" would otherwise collapse to an empty string
/// (`"/".trim_end_matches(['/', '\\'])` is empty), and an empty cwd later
/// fails workspace validation with the misleading "workspace path does not
/// exist: " — so a path that is entirely separators is kept verbatim.
pub fn normalize_cwd(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches(['/', '\\']).to_string();
    if trimmed.is_empty() && !path.trim().is_empty() {
        // All separators (e.g. "/"): the root. Keep it instead of returning
        // an empty path that workspace validation would reject.
        path.trim().to_string()
    } else {
        trimmed
    }
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

/// Read a user-attached image and return a `data:<mime>;base64,…` URL for a
/// vision model's image_url block. Oversized images are downscaled so one
/// attachment can't blow up the model request (mirrors opencode's normalize):
/// an image within `MAX_DIM`×`MAX_DIM` whose base64 is ≤ `MAX_BASE64_BYTES` is
/// used verbatim (format preserved); otherwise it's resized to fit `MAX_DIM`
/// and JPEG-re-encoded at decreasing quality until it fits. Returns `None` when
/// the file can't be read/decoded or won't fit even at the lowest quality — the
/// caller then skips the image (a path reference is useless: it's unreadable or
/// too large either way).
pub fn image_data_url_for_model(path: &str) -> Option<String> {
    use base64::Engine as _;

    const MAX_DIM: u32 = 2000;
    const MAX_BASE64_BYTES: usize = 5 * 1024 * 1024;
    const MAX_SOURCE_BYTES: u64 = 25 * 1024 * 1024;

    let data_url = |mime: &str, bytes: &[u8]| {
        format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    };
    // Projected base64 length is ~4/3 of the raw byte count.
    let fits_base64 = |len: usize| len.div_ceil(3) * 4 <= MAX_BASE64_BYTES;

    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > MAX_SOURCE_BYTES {
        return None;
    }
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let key = (PathBuf::from(path), modified, metadata.len());
    if let Some(url) = cached_image_url(&key) {
        return Some(url);
    }
    let bytes = std::fs::read(path).ok()?;
    // Read dimensions from the header first. The common case sends the original
    // bytes unchanged, so decoding the full image before this check spent tens of
    // milliseconds per screenshot for a value the header already contains.
    let dimensions_reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .ok()?;
    let format = dimensions_reader.format()?;
    let (width, height) = dimensions_reader.into_dimensions().ok()?;

    // Small enough already: send the original bytes, keeping the source format
    // (e.g. PNG transparency) instead of forcing a JPEG re-encode.
    if width <= MAX_DIM && height <= MAX_DIM && fits_base64(bytes.len()) {
        let url = data_url(format.to_mime_type(), &bytes);
        remember_image_url(key, &url);
        return Some(url);
    }

    // Cap the decoder's allocation so a decompression bomb (a tiny file that
    // decodes to a huge bitmap) can't OOM the agent. 512MB comfortably fits any
    // legitimate photo/screenshot while rejecting absurd dimensions.
    let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    let (width, height) = (img.width(), img.height());

    // Downscale to fit MAX_DIM (aspect-preserving), then JPEG-compress at
    // decreasing quality until the payload fits.
    let scaled = if width > MAX_DIM || height > MAX_DIM {
        img.resize(MAX_DIM, MAX_DIM, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let rgb = scaled.to_rgb8();
    // First quality whose JPEG fits the base64 budget wins. `None` (every
    // quality too big, or the encoder failed) means the caller skips the
    // image — with the current caps a 2000px JPEG at q40 is always far under
    // 5 MiB, so in practice this only triggers on an encoder error.
    let url = [80u8, 70, 60, 50, 40].into_iter().find_map(|quality| {
        let mut buf = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
        let encoded = image::ImageEncoder::write_image(
            encoder,
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .is_ok();
        (encoded && fits_base64(buf.len())).then(|| data_url("image/jpeg", &buf))
    })?;
    remember_image_url(key, &url);
    Some(url)
}

/// Version string — injected at build time via build.rs from FUTURE_VERSION
/// (see scripts/version.mjs). Release builds are a plain `X.Y.Z`; dev builds
/// carry a `-<hash>` suffix (`+local[.dirty]` for local builds).
pub const VERSION: &str = env!("FUTURE_VERSION");

/// Build identity of this agent binary, injected by `agent/build.rs` and
/// reported through `get_agent_info`. `VERSION` alone cannot identify the code:
/// a release tag (`1.2.3`) and a coordinated test/nightly build
/// (`0.0.2-<run>+test`) contain no commit at all, so a client checking whether
/// the *running* process is the commit it is reading needs these separately.
/// `unknown` means the build had no git checkout.
pub const GIT_COMMIT: &str = env!("FUTURE_GIT_COMMIT");
pub const GIT_COMMIT_SHORT: &str = env!("FUTURE_GIT_COMMIT_SHORT");
pub const GIT_DIRTY: &str = env!("FUTURE_GIT_DIRTY");
pub const BUILD_TARGET: &str = env!("FUTURE_BUILD_TARGET");
pub const BUILD_PROFILE: &str = env!("FUTURE_BUILD_PROFILE");

/// The build identity as JSON, ready to merge into a response object.
///
/// `gitCommit` is `null` rather than the literal `"unknown"` when the build had
/// no checkout: a caller comparing it against `git rev-parse HEAD` must not read
/// "no commit recorded" as a commit name. `gitDirty` follows the same rule —
/// with no working tree there is nothing to be dirty, which is not the same as
/// verified clean.
pub fn build_identity_json() -> serde_json::Value {
    let known =
        |value: &str| (!value.is_empty() && value != "unknown").then(|| serde_json::json!(value));
    let commit_present = known(GIT_COMMIT).is_some();
    serde_json::json!({
        "gitCommit": known(GIT_COMMIT),
        "gitCommitShort": if commit_present { known(GIT_COMMIT_SHORT) } else { None },
        "gitDirty": commit_present.then(|| GIT_DIRTY == "1"),
        "buildTarget": known(BUILD_TARGET),
        "buildProfile": known(BUILD_PROFILE),
    })
}

/// Resolve the user home directory consistently for every Agent subsystem.
///
/// `HOME`/`USERPROFILE` are honoured first so isolated home redirects
/// (integration tests, portable setups) apply on every platform. Empty and
/// relative values are ignored: neither is safe as the root of credentials,
/// sandbox policy or capability state. On Windows the `dirs` crate alone reads
/// the token profile and therefore does not observe a redirected portable home.
pub fn home_dir_opt() -> Option<PathBuf> {
    home_dir_from(
        std::env::var_os("HOME"),
        std::env::var_os("USERPROFILE"),
        dirs::home_dir(),
    )
}

fn home_dir_from(
    home: Option<std::ffi::OsString>,
    userprofile: Option<std::ffi::OsString>,
    system_home: Option<PathBuf>,
) -> Option<PathBuf> {
    home.into_iter()
        .chain(userprofile)
        .map(PathBuf::from)
        .find(|path| !path.as_os_str().is_empty() && path.is_absolute())
        .or(system_home)
}

/// User home directory, falling back to the OS temporary directory only on a
/// host where neither environment nor the platform profile API yields one.
pub fn home_dir() -> PathBuf {
    home_dir_opt().unwrap_or_else(std::env::temp_dir)
}

/// The configured FutureOS home override, when this process has one — set by
/// `FUTURE_HOME` (see [`future_home`]). `None` means the state root
/// is the default `~/.future`.
pub fn future_home_override() -> Option<PathBuf> {
    future_rpc::home::future_home_override()
}

/// FutureOS home root: the directory that owns all local Agent state
/// (`<home>/agent`, `<home>/run`, ...), normally `~/.future`.
///
/// `FUTURE_HOME` replaces the whole root, which is how a second, fully isolated
/// Agent instance runs beside the default one: its own singleton lock,
/// database, sessions, logs and local IPC endpoint. Only paths FutureOS owns
/// move — the user's real home (sandbox guards over `~/.ssh`, the shared
/// `~/.agents/skills` scope) is unaffected.
///
/// None only on a host where neither environment nor the platform profile API
/// yields a home directory (same edge case as [`home_dir_opt`]).
pub fn future_home_opt() -> Option<PathBuf> {
    future_home_from(future_home_override(), home_dir_opt())
}

fn future_home_from(override_dir: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    override_dir.or_else(|| home.map(|home| home.join(".future")))
}

/// FutureOS home root, falling back to the OS temporary directory only when no
/// home can be resolved at all (see [`future_home_opt`]).
pub fn future_home() -> PathBuf {
    future_home_opt().unwrap_or_else(|| std::env::temp_dir().join(".future"))
}

/// Default base session directory (contains per-cwd subdirectories)
pub fn default_session_dir(_cwd: &str) -> PathBuf {
    future_home().join("agent").join("sessions")
}

/// Default config directory
pub fn default_config_dir() -> PathBuf {
    future_home().join("agent")
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
    std::io::stdin().is_terminal()
}

/// True when `path` lives under the FutureOS-managed data root (default
/// `~/.future/`, or the `FUTURE_HOME` override — see [`future_home`]). These
/// directories (chat temp workspaces, the agent's default workspace) are owned
/// by FutureOS, so the agent may auto-create and repair them. A user-chosen
/// workspace directory never qualifies — it must not be silently recreated or
/// chmod'ed.
pub fn is_future_managed_dir(path: &Path) -> bool {
    future_home_opt().is_some_and(|home| path.starts_with(home))
}

/// Ensure a workspace directory exists and is writable. Creates the directory
/// (and parents) if missing. When the directory exists but is not writable and
/// `auto_repair` is true, attempts a permission repair on Unix (grant owner
/// read/write/execute) before retrying once. `auto_repair` should only be true
/// for FutureOS-managed directories (see [`is_future_managed_dir`]); a user
/// workspace that is missing or blocked returns an error so the caller can
/// surface it instead of silently rebuilding the user's project dir. Returns an
/// error when the path is not a directory or a write test still fails.
pub fn ensure_workspace_accessible(path: &Path, auto_repair: bool) -> Result<(), std::io::Error> {
    // A plain file at the workspace path is never recoverable — reject it
    // before create_dir_all (which would fail with AlreadyExists anyway).
    if path.exists() && !path.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            format!("workspace path is not a directory: {}", path.display()),
        ));
    }
    // FutureOS-managed dirs are auto-created when missing; a user workspace
    // that vanished is an error the caller surfaces (never silently rebuilt).
    if path.exists() {
        // Verify writability with a test file.
        let test = path.join(".future_write_test");
        return match std::fs::write(&test, b"") {
            Ok(()) => {
                let _ = std::fs::remove_file(&test);
                Ok(())
            }
            // Directory exists but the process can't write it. For FutureOS-
            // managed dirs, repair owner permissions and retry once (e.g. a
            // workspace chmod'ed read-only by a previous session). A user
            // workspace is left untouched — the caller surfaces the error.
            Err(_error) if auto_repair => {
                repair_dir_permissions(path)?;
                std::fs::write(&test, b"")?;
                let _ = std::fs::remove_file(&test);
                Ok(())
            }
            Err(error) => Err(error),
        };
    }
    if auto_repair {
        std::fs::create_dir_all(path)?;
        let test = path.join(".future_write_test");
        std::fs::write(&test, b"")?;
        let _ = std::fs::remove_file(&test);
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("workspace path does not exist: {}", path.display()),
        ))
    }
}

/// Grant the owner read/write/execute on `dir` (Unix only; a no-op elsewhere).
fn repair_dir_permissions(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let current = std::fs::metadata(dir)?.permissions();
        let mode = current.mode();
        // Add owner rwx — never strip any existing bits.
        let repaired = mode | 0o700;
        if repaired != mode {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(repaired))?;
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
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

#[cfg(test)]
mod image_prep_tests {
    use super::image_data_url_for_model;

    fn write_png(tag: &str, w: u32, h: u32) -> std::path::PathBuf {
        let img = image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        let path = std::env::temp_dir().join(format!(
            "futureos-imgtest-{}-{}.png",
            std::process::id(),
            tag
        ));
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn bmp_with_nonstandard_extension_keeps_true_mime() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attachment.bin");
        image::RgbImage::new(2, 2)
            .save_with_format(&path, image::ImageFormat::Bmp)
            .unwrap();
        let url = image_data_url_for_model(path.to_str().unwrap()).unwrap();
        assert!(url.starts_with("data:image/bmp;base64,"));
    }

    #[test]
    fn small_image_keeps_format() {
        let p = write_png("small", 64, 64);
        let url = image_data_url_for_model(p.to_str().unwrap());
        std::fs::remove_file(&p).ok();
        let url = url.expect("data url");
        // Within limits → original PNG bytes, format preserved.
        assert!(url.starts_with("data:image/png;base64,"), "{url:.40}");
    }

    #[test]
    fn oversized_image_is_downscaled_to_jpeg() {
        let p = write_png("big", 4000, 3000);
        let url = image_data_url_for_model(p.to_str().unwrap());
        std::fs::remove_file(&p).ok();
        let url = url.expect("data url");
        // Exceeds the 2000px cap → resized + JPEG re-encoded.
        assert!(url.starts_with("data:image/jpeg;base64,"), "{url:.40}");
    }

    #[test]
    fn missing_or_undecodable_returns_none() {
        assert!(image_data_url_for_model("/no/such/file-xyz.png").is_none());
    }

    #[test]
    fn over_25mb_source_returns_none() {
        // Sparse file past MAX_SOURCE_BYTES: rejected before any decode work.
        let path =
            std::env::temp_dir().join(format!("futureos-imgtest-{}-huge.bin", std::process::id()));
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(26 * 1024 * 1024).unwrap();
        drop(f);
        assert!(image_data_url_for_model(path.to_str().unwrap()).is_none());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn small_dims_but_fat_bytes_reencode_without_resize() {
        // Dimensions under MAX_DIM but a payload over the base64 budget
        // (incompressible noise): takes the no-resize arm and JPEG-encodes.
        let mut state: u32 = 0x12345678;
        let img = image::RgbImage::from_fn(1200, 1200, |_, _| {
            // xorshift32 — deterministic high-entropy pixels.
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            image::Rgb([state as u8, (state >> 8) as u8, (state >> 16) as u8])
        });
        let path =
            std::env::temp_dir().join(format!("futureos-imgtest-{}-noise.png", std::process::id()));
        img.save(&path).unwrap();
        // Sanity: the fixture really is over the base64 budget, or the test
        // stops exercising the re-encode path.
        let raw_len = std::fs::metadata(&path).unwrap().len() as usize;
        assert!(
            raw_len.div_ceil(3) * 4 > 5 * 1024 * 1024,
            "fixture too small"
        );

        let url = image_data_url_for_model(path.to_str().unwrap());
        std::fs::remove_file(&path).ok();
        let url = url.expect("noise image should still fit after JPEG q80");
        assert!(url.starts_with("data:image/jpeg;base64,"), "{url:.40}");
    }
}

#[cfg(test)]
mod util_tests {
    use super::*;

    #[test]
    fn home_resolution_prefers_valid_environment_paths() {
        let home = std::env::temp_dir().join("futureos-home-env");
        let profile = std::env::temp_dir().join("futureos-profile-env");
        let system = std::env::temp_dir().join("futureos-system-home");

        assert_eq!(
            home_dir_from(
                Some(home.clone().into_os_string()),
                Some(profile.into_os_string()),
                Some(system),
            ),
            Some(home)
        );
    }

    #[test]
    fn home_resolution_ignores_empty_and_relative_environment_paths() {
        let profile = std::env::temp_dir().join("futureos-profile-env");
        let system = std::env::temp_dir().join("futureos-system-home");

        assert_eq!(
            home_dir_from(
                Some(std::ffi::OsString::new()),
                Some(profile.clone().into_os_string()),
                Some(system.clone()),
            ),
            Some(profile)
        );
        assert_eq!(
            home_dir_from(
                Some(std::ffi::OsString::from("relative-home")),
                Some(std::ffi::OsString::from("relative-profile")),
                Some(system.clone()),
            ),
            Some(system)
        );
        assert_eq!(home_dir_from(None, None, None), None);
    }

    #[test]
    fn future_home_defaults_to_dot_future_under_the_user_home() {
        let home = std::env::temp_dir().join("futureos-user-home");
        assert_eq!(
            future_home_from(None, Some(home.clone())),
            Some(home.join(".future"))
        );
        assert_eq!(future_home_from(None, None), None);
    }

    #[test]
    fn future_home_override_replaces_the_whole_root() {
        let home = std::env::temp_dir().join("futureos-user-home");
        let isolated = std::env::temp_dir().join("futureos-isolated-home");
        assert_eq!(
            future_home_from(Some(isolated.clone()), Some(home)),
            Some(isolated)
        );
    }

    #[test]
    fn generate_id_is_unique() {
        let id1 = generate_id();
        let id2 = generate_id();
        assert_ne!(id1, id2);
        assert_eq!(id1.len(), 36);
        assert_eq!(uuid::Uuid::parse_str(&id1).unwrap().to_string(), id1);
    }

    #[test]
    fn ensure_workspace_accessible_creates_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nested").join("workspace");
        ensure_workspace_accessible(&missing, true).unwrap();
        assert!(missing.is_dir());
    }

    #[test]
    fn ensure_workspace_accessible_does_not_create_without_repair() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nested").join("workspace");
        let err = ensure_workspace_accessible(&missing, false).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        assert!(!missing.exists(), "user workspace must not be auto-created");
    }

    #[test]
    fn ensure_workspace_accessible_accepts_writable_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(ensure_workspace_accessible(dir.path(), true).is_ok());
        assert!(ensure_workspace_accessible(dir.path(), false).is_ok());
    }

    /// The repair path is entered for any blocked managed directory: the write
    /// test fails because `.future_write_test` is an existing directory, the
    /// repair runs (a no-op when the owner bits are already intact), and the
    /// retry fails the same way — so the caller gets the write error instead of
    /// a false "the workspace is fine".
    #[test]
    fn ensure_workspace_accessible_repair_retry_reports_the_same_blockage() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".future_write_test")).unwrap();
        let error = ensure_workspace_accessible(dir.path(), true).unwrap_err();
        assert_ne!(
            error.kind(),
            std::io::ErrorKind::NotFound,
            "a blocked existing directory is not a missing one: {error:?}"
        );
        assert!(dir.path().join(".future_write_test").is_dir());
    }

    /// Without `auto_repair` the blocked directory is returned as-is: a user
    /// workspace must never be chmod'ed, rebuilt or removed.
    #[test]
    fn ensure_workspace_accessible_reports_a_blocked_dir_without_repair() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".future_write_test")).unwrap();
        let error = ensure_workspace_accessible(dir.path(), false).unwrap_err();
        assert_ne!(error.kind(), std::io::ErrorKind::NotFound, "{error:?}");
        assert!(dir.path().is_dir(), "the caller's directory is left alone");
        assert!(dir.path().join(".future_write_test").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn ensure_workspace_accessible_repairs_readonly_dir() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        // chmod 000 — directory still exists but the test file write must fail.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o000)).unwrap();
        // The test created the tempdir, so the repair always succeeds.
        let result = ensure_workspace_accessible(dir.path(), true);
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        result.expect("repair of an owned dir must succeed");
        let mode = std::fs::metadata(dir.path()).unwrap().permissions().mode();
        assert_ne!(mode & 0o700, 0, "owner bits should have been restored");
    }

    #[cfg(unix)]
    #[test]
    fn repair_dir_permissions_noop_when_owner_bits_present() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        // Force the repair path with permissions already intact: the write
        // test fails because `.future_write_test` is an existing DIRECTORY,
        // not because of mode bits — repaired == mode, so no chmod happens.
        std::fs::create_dir(dir.path().join(".future_write_test")).unwrap();
        let mode_before = std::fs::metadata(dir.path()).unwrap().permissions().mode();
        assert_eq!(mode_before & 0o700, 0o700, "tempdir starts with owner rwx");
        // The repair runs (no-op chmod arm) and the retry still fails on the
        // directory collision → overall error.
        assert!(ensure_workspace_accessible(dir.path(), true).is_err());
        let mode_after = std::fs::metadata(dir.path()).unwrap().permissions().mode();
        assert_eq!(mode_before, mode_after);
    }

    #[cfg(unix)]
    #[test]
    fn ensure_workspace_accessible_does_not_repair_without_flag() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o000)).unwrap();
        let result = ensure_workspace_accessible(dir.path(), false);
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        // auto_repair=false must surface the write failure, not repair.
        assert!(result.is_err());
    }

    #[test]
    fn ensure_workspace_accessible_rejects_plain_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-dir");
        std::fs::write(&file, b"x").unwrap();
        let err = ensure_workspace_accessible(&file, false).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotADirectory);
    }

    #[test]
    fn is_future_managed_dir_detects_future_root() {
        let home = home_dir();
        assert!(is_future_managed_dir(
            &home
                .join(".future")
                .join("workspaces")
                .join("chat")
                .join("x")
        ));
        assert!(is_future_managed_dir(&home.join(".future/agent/workspace")));
        // A sibling dir like ~/.futureworks or a user project is NOT managed.
        assert!(!is_future_managed_dir(&home.join(".futureworks")));
        assert!(!is_future_managed_dir(Path::new(
            "/home/user/projects/my-app"
        )));
    }

    #[test]
    fn generate_entry_id_format() {
        let id = generate_entry_id();
        assert_eq!(id.len(), 36);
        let uuid = uuid::Uuid::parse_str(&id).unwrap();
        assert_eq!(uuid.get_version_num(), 4);
        assert_eq!(uuid.get_variant(), uuid::Variant::RFC4122);
        assert_eq!(uuid.to_string(), id);
    }

    #[test]
    fn encode_cwd_basic() {
        let encoded = encode_cwd("/home/user/projects/my-app");
        assert!(!encoded.is_empty());
        assert_eq!(encoded, encoded.to_lowercase());
    }

    #[test]
    fn encode_cwd_root_fallback() {
        let encoded = encode_cwd("");
        assert!(!encoded.is_empty());
    }

    #[test]
    fn encode_cwd_dot_fallback() {
        let encoded = encode_cwd(".");
        assert!(!encoded.is_empty());
    }

    #[test]
    fn normalize_cwd_preserves_the_filesystem_root() {
        // The root "/" must survive the trailing-separator trim: collapsing it
        // to an empty string later fails workspace validation.
        assert_eq!(normalize_cwd("/"), "/");
        assert_eq!(normalize_cwd("//"), "//");
    }

    #[test]
    fn normalize_cwd_trims_separators_from_ordinary_paths() {
        assert_eq!(normalize_cwd("/tmp/project/"), "/tmp/project");
        assert_eq!(normalize_cwd("/tmp/project\\"), "/tmp/project");
        assert_eq!(normalize_cwd("  /tmp/project  "), "/tmp/project");
    }

    #[test]
    fn normalize_cwd_keeps_an_already_clean_path_untouched() {
        assert_eq!(normalize_cwd("/tmp/project"), "/tmp/project");
    }

    #[test]
    fn normalize_cwd_empty_stays_empty() {
        assert_eq!(normalize_cwd(""), "");
        assert_eq!(normalize_cwd("   "), "");
    }

    #[test]
    fn detect_image_mime_type_from_extension_all_formats() {
        use std::path::Path;
        assert_eq!(
            detect_image_mime_type_from_extension(Path::new("test.png")),
            Some("image/png".to_string())
        );
        assert_eq!(
            detect_image_mime_type_from_extension(Path::new("test.jpg")),
            Some("image/jpeg".to_string())
        );
        assert_eq!(
            detect_image_mime_type_from_extension(Path::new("test.jpeg")),
            Some("image/jpeg".to_string())
        );
        assert_eq!(
            detect_image_mime_type_from_extension(Path::new("test.gif")),
            Some("image/gif".to_string())
        );
        assert_eq!(
            detect_image_mime_type_from_extension(Path::new("test.webp")),
            Some("image/webp".to_string())
        );
        assert_eq!(
            detect_image_mime_type_from_extension(Path::new("test.svg")),
            Some("image/svg+xml".to_string())
        );
        assert_eq!(
            detect_image_mime_type_from_extension(Path::new("test.bmp")),
            Some("image/bmp".to_string())
        );
        assert_eq!(
            detect_image_mime_type_from_extension(Path::new("test.txt")),
            None
        );
        assert_eq!(
            detect_image_mime_type_from_extension(Path::new("noext")),
            None
        );
    }

    #[test]
    fn detect_image_mime_type_by_magic() {
        use std::io::Write;
        // Create a real PNG file and test magic detection
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.png");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0])
                .unwrap();
        }
        assert_eq!(detect_image_mime_type(&path), Some("image/png".to_string()));

        // JPEG magic
        let path2 = dir.path().join("test.jpg");
        {
            let mut f = std::fs::File::create(&path2).unwrap();
            f.write_all(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0])
                .unwrap();
        }
        assert_eq!(
            detect_image_mime_type(&path2),
            Some("image/jpeg".to_string())
        );

        // GIF magic
        let path3 = dir.path().join("test.gif");
        {
            let mut f = std::fs::File::create(&path3).unwrap();
            f.write_all(&[0x47, 0x49, 0x46, 0x38, 0, 0, 0, 0, 0, 0, 0, 0])
                .unwrap();
        }
        assert_eq!(
            detect_image_mime_type(&path3),
            Some("image/gif".to_string())
        );

        // WEBP magic
        let path4 = dir.path().join("test.webp");
        {
            let mut f = std::fs::File::create(&path4).unwrap();
            f.write_all(&[0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, 0x57, 0x45, 0x42, 0x50])
                .unwrap();
        }
        assert_eq!(
            detect_image_mime_type(&path4),
            Some("image/webp".to_string())
        );

        // Unknown magic
        let path5 = dir.path().join("test.unk");
        {
            let mut f = std::fs::File::create(&path5).unwrap();
            f.write_all(&[0x00, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0])
                .unwrap();
        }
        assert_eq!(detect_image_mime_type(&path5), None);
    }

    #[test]
    fn default_session_dir_exists() {
        let dir = default_session_dir("/any/path");
        assert!(dir.to_string_lossy().contains("sessions"));
    }

    #[test]
    fn default_config_dir_exists() {
        // This is the one accessor here whose value depends on `FUTURE_HOME`
        // (`default_session_dir`'s assertion only checks its leaf, and
        // `default_settings_paths_are_valid` checks a filename): a concurrent
        // test that redirects the root under `test_support::home_env_lock()`
        // makes `$HOME/.future` become its temp directory, and `.contains(".future")`
        // then fails for a reason that has nothing to do with this function.
        // Every other reader of a home-derived path in this crate takes this
        // lock; this test was the one that did not (observed failing once in a
        // gate run under parallel load).
        let _guard = crate::test_support::home_env_lock();
        let dir = default_config_dir();
        assert!(dir.to_string_lossy().contains(".future"));
    }

    #[test]
    fn default_settings_paths_are_valid() {
        let (global, project) = default_settings_paths();
        assert!(global.to_string_lossy().contains("settings.json"));
        assert!(project.to_string_lossy().contains("settings.json"));
        assert_ne!(global, project);
    }

    #[test]
    fn canonical_path_resolves() {
        // The crate directory exists on every platform; a hard-coded `/tmp`
        // would make this a Unix-only test.
        let result = canonical_path(Path::new(env!("CARGO_MANIFEST_DIR")));
        assert!(result.is_ok());
    }

    #[test]
    fn is_tty_inspects_standard_input() {
        use std::io::IsTerminal;
        // Pins *which* stream the helper reads: a redirect of stdout (how cargo
        // test runs) must not change the answer, so this cannot be replaced by
        // `stdout().is_terminal()`. A discriminating test needs a pty.
        assert_eq!(is_tty(), std::io::stdin().is_terminal());
    }

    #[test]
    fn ansi_constants_are_non_empty() {
        assert!(!ansi::RESET.is_empty());
        assert!(!ansi::BOLD.is_empty());
        assert!(!ansi::RED.is_empty());
        assert!(!ansi::GREEN.is_empty());
        assert!(!ansi::YELLOW.is_empty());
        assert!(!ansi::BLUE.is_empty());
        assert!(!ansi::MAGENTA.is_empty());
    }
}
