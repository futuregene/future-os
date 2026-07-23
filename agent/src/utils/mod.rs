//! Utility functions — matching Go internal/utils

use std::io::IsTerminal;
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

    if std::fs::metadata(path).ok()?.len() > MAX_SOURCE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
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

    // Small enough already: send the original bytes, keeping the source format
    // (e.g. PNG transparency) instead of forcing a JPEG re-encode.
    if width <= MAX_DIM && height <= MAX_DIM && fits_base64(bytes.len()) {
        let mime = detect_image_mime_type(Path::new(path))
            .or_else(|| detect_image_mime_type_from_extension(Path::new(path)))
            .unwrap_or_else(|| "image/png".to_string());
        return Some(data_url(&mime, &bytes));
    }

    // Downscale to fit MAX_DIM (aspect-preserving), then JPEG-compress at
    // decreasing quality until the payload fits.
    let scaled = if width > MAX_DIM || height > MAX_DIM {
        img.resize(MAX_DIM, MAX_DIM, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let rgb = scaled.to_rgb8();
    for quality in [80u8, 70, 60, 50, 40] {
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
        if encoded && fits_base64(buf.len()) {
            return Some(data_url("image/jpeg", &buf));
        }
    }
    None
}

/// Version string — injected at build time via build.rs from FUTURE_VERSION
/// (see scripts/version.mjs). Release builds are a plain `X.Y.Z`; dev builds
/// carry a `-<hash>` suffix (`+local[.dirty]` for local builds).
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
    std::io::stdin().is_terminal()
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
}

#[cfg(test)]
mod util_tests {
    use super::*;

    #[test]
    fn generate_id_is_unique() {
        let id1 = generate_id();
        let id2 = generate_id();
        assert_ne!(id1, id2);
        assert!(id1.contains('-'));
        assert!(id1.len() > 12);
    }

    #[test]
    fn generate_entry_id_format() {
        let id = generate_entry_id();
        // Format: YYYYMMDD-HHMMSS-hex
        assert!(id.len() >= 21);
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 3);
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
            f.write_all(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0]).unwrap();
        }
        assert_eq!(
            detect_image_mime_type(&path),
            Some("image/png".to_string())
        );

        // JPEG magic
        let path2 = dir.path().join("test.jpg");
        {
            let mut f = std::fs::File::create(&path2).unwrap();
            f.write_all(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        }
        assert_eq!(
            detect_image_mime_type(&path2),
            Some("image/jpeg".to_string())
        );

        // GIF magic
        let path3 = dir.path().join("test.gif");
        {
            let mut f = std::fs::File::create(&path3).unwrap();
            f.write_all(&[0x47, 0x49, 0x46, 0x38, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        }
        assert_eq!(
            detect_image_mime_type(&path3),
            Some("image/gif".to_string())
        );

        // WEBP magic
        let path4 = dir.path().join("test.webp");
        {
            let mut f = std::fs::File::create(&path4).unwrap();
            f.write_all(&[0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, 0x57, 0x45, 0x42, 0x50]).unwrap();
        }
        assert_eq!(
            detect_image_mime_type(&path4),
            Some("image/webp".to_string())
        );

        // Unknown magic
        let path5 = dir.path().join("test.unk");
        {
            let mut f = std::fs::File::create(&path5).unwrap();
            f.write_all(&[0x00, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
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
        let result = canonical_path(Path::new("/tmp"));
        assert!(result.is_ok());
    }

    #[test]
    fn is_tty_returns_bool() {
        // Just verify it doesn't panic
        let _ = is_tty();
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
