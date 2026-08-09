//! Terminal image protocol support (Kitty, iTerm2). 1:1 port of
//! `tui/src/terminal-image.ts`.
//!
//! Module-level state (capabilities cache, cell dimensions) mirrors the TS
//! module globals; both live behind mutexes for test safety.

use std::env;
use std::sync::Mutex;

/// `ImageProtocol` from TS — `"kitty" | "iterm2" | null`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageProtocol {
    Kitty,
    Iterm2,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalCapabilities {
    pub images: ImageProtocol,
    pub true_color: bool,
    pub hyperlinks: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct CellDimensions {
    pub width_px: usize,
    pub height_px: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct ImageDimensions {
    pub width_px: usize,
    pub height_px: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ImageRenderOptions {
    pub max_width_cells: Option<usize>,
    pub max_height_cells: Option<usize>,
    pub preserve_aspect_ratio: Option<bool>,
    pub image_id: Option<u32>,
    pub move_cursor: Option<bool>,
}

/// `let cachedCapabilities: TerminalCapabilities | null = null` (TS module
/// global). The `getCapabilities` cache.
static CACHED_CAPABILITIES: Mutex<Option<TerminalCapabilities>> = Mutex::new(None);

/// `let cellDimensions: CellDimensions = { widthPx: 9, heightPx: 18 }`.
static CELL_DIMENSIONS: Mutex<CellDimensions> = Mutex::new(CellDimensions {
    width_px: 9,
    height_px: 18,
});

pub fn get_cell_dimensions() -> CellDimensions {
    *CELL_DIMENSIONS.lock().unwrap()
}

pub fn set_cell_dimensions(dims: CellDimensions) {
    *CELL_DIMENSIONS.lock().unwrap() = dims;
}

/// `!!process.env.X` semantics — Node truthiness: set AND non-empty.
fn env_nonempty(key: &str) -> bool {
    env::var_os(key).is_some_and(|v| !v.is_empty())
}

pub fn detect_capabilities() -> TerminalCapabilities {
    let term_program = env::var("TERM_PROGRAM").unwrap_or_default().to_lowercase();
    let term = env::var("TERM").unwrap_or_default().to_lowercase();
    let color_term = env::var("COLORTERM").unwrap_or_default().to_lowercase();

    let in_tmux_or_screen =
        env_nonempty("TMUX") || term.starts_with("tmux") || term.starts_with("screen");
    if in_tmux_or_screen {
        let true_color = color_term == "truecolor" || color_term == "24bit";
        return TerminalCapabilities {
            images: ImageProtocol::None,
            true_color,
            hyperlinks: false,
        };
    }

    if env_nonempty("KITTY_WINDOW_ID") || term_program == "kitty" {
        return TerminalCapabilities {
            images: ImageProtocol::Kitty,
            true_color: true,
            hyperlinks: true,
        };
    }

    if term_program == "ghostty"
        || term.contains("ghostty")
        || env_nonempty("GHOSTTY_RESOURCES_DIR")
    {
        return TerminalCapabilities {
            images: ImageProtocol::Kitty,
            true_color: true,
            hyperlinks: true,
        };
    }

    if env_nonempty("WEZTERM_PANE") || term_program == "wezterm" {
        return TerminalCapabilities {
            images: ImageProtocol::Kitty,
            true_color: true,
            hyperlinks: true,
        };
    }

    if env_nonempty("ITERM_SESSION_ID") || term_program == "iterm.app" {
        return TerminalCapabilities {
            images: ImageProtocol::Iterm2,
            true_color: true,
            hyperlinks: true,
        };
    }

    if term_program == "vscode" {
        return TerminalCapabilities {
            images: ImageProtocol::None,
            true_color: true,
            hyperlinks: true,
        };
    }

    if term_program == "alacritty" {
        return TerminalCapabilities {
            images: ImageProtocol::None,
            true_color: true,
            hyperlinks: true,
        };
    }

    let true_color = color_term == "truecolor" || color_term == "24bit";
    TerminalCapabilities {
        images: ImageProtocol::None,
        true_color,
        hyperlinks: false,
    }
}

pub fn get_capabilities() -> TerminalCapabilities {
    let mut cached = CACHED_CAPABILITIES.lock().unwrap();
    if cached.is_none() {
        *cached = Some(detect_capabilities());
    }
    cached.unwrap()
}

pub fn reset_capabilities_cache() {
    *CACHED_CAPABILITIES.lock().unwrap() = None;
}

pub fn set_capabilities(caps: TerminalCapabilities) {
    *CACHED_CAPABILITIES.lock().unwrap() = Some(caps);
}

const KITTY_PREFIX: &str = "\x1b_G";
const ITERM2_PREFIX: &str = "\x1b]1337;File=";

pub fn is_image_line(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    if line.starts_with(KITTY_PREFIX) || line.starts_with(ITERM2_PREFIX) {
        return true;
    }
    line.contains(KITTY_PREFIX) || line.contains(ITERM2_PREFIX)
}

// ─── PRNG (port of Math.random) ───────────────────────────────────────────

/// xorshift64* seeded once; `allocateImageId` only needs uniform-ish values
/// in [1, 0xfffffffe], and no test/parity corpus asserts exact ids.
fn next_random() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    let mut state = STATE.load(Ordering::Relaxed);
    if state == 0 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9e37_79b9_7f4a_7c15);
        let addr = &STATE as *const _ as u64;
        state = seed_from_time_and_addr(nanos, addr);
    }
    state ^= state >> 12;
    state ^= state << 25;
    state ^= state >> 27;
    STATE.store(state, Ordering::Relaxed);
    state.wrapping_mul(0x2545_f491_4f6c_dd1d)
}

/// Derive the initial xorshift state from time + address entropy, with a
/// fixed nonzero fallback (xorshift degenerates at state 0).
fn seed_from_time_and_addr(nanos: u64, addr: u64) -> u64 {
    let state = nanos ^ addr.rotate_left(17) ^ 0x9e37_79b9_7f4a_7c15;
    if state == 0 {
        return 0x2545_f491_4f6c_dd1d;
    }
    state
}

/// Port of `Math.floor(Math.random() * 0xfffffffe) + 1` → [1, 0xfffffffe].
pub fn allocate_image_id() -> u32 {
    (next_random() % 0xffff_fffe) as u32 + 1
}

// ─── Encoding ─────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct KittyEncodeOptions {
    pub columns: Option<usize>,
    pub rows: Option<usize>,
    pub image_id: Option<u32>,
    pub move_cursor: Option<bool>,
}

pub fn encode_kitty(base64_data: &str, options: &KittyEncodeOptions) -> String {
    const CHUNK_SIZE: usize = 4096;

    let mut params: Vec<String> = vec!["a=T".to_string(), "f=100".to_string(), "q=2".to_string()];

    if options.move_cursor == Some(false) {
        params.push("C=1".to_string());
    }
    if let Some(columns) = options.columns {
        params.push(format!("c={columns}"));
    }
    if let Some(rows) = options.rows {
        params.push(format!("r={rows}"));
    }
    if let Some(image_id) = options.image_id {
        params.push(format!("i={image_id}"));
    }

    if base64_data.len() <= CHUNK_SIZE {
        return format!("\x1b_G{};{}\x1b\\", params.join(","), base64_data);
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut offset = 0;
    let mut is_first = true;

    while offset < base64_data.len() {
        let end = (offset + CHUNK_SIZE).min(base64_data.len());
        let chunk = &base64_data[offset..end];
        let is_last = offset + CHUNK_SIZE >= base64_data.len();

        if is_first {
            chunks.push(format!("\x1b_G{},m=1;{}\x1b\\", params.join(","), chunk));
            is_first = false;
        } else if is_last {
            chunks.push(format!("\x1b_Gm=0;{}\x1b\\", chunk));
        } else {
            chunks.push(format!("\x1b_Gm=1;{}\x1b\\", chunk));
        }

        offset += CHUNK_SIZE;
    }

    chunks.join("")
}

pub fn delete_kitty_image(image_id: u32) -> String {
    format!("\x1b_Ga=d,d=I,i={image_id},q=2\x1b\\")
}

pub fn delete_all_kitty_images() -> String {
    "\x1b_Ga=d,d=A,q=2\x1b\\".to_string()
}

/// `width?: number | string` — caller passes already-stringified values.
#[derive(Default)]
pub struct Iterm2EncodeOptions {
    pub width: Option<String>,
    pub height: Option<String>,
    pub name: Option<String>,
    pub preserve_aspect_ratio: Option<bool>,
    pub inline: Option<bool>,
}

pub fn encode_iterm2(base64_data: &str, options: &Iterm2EncodeOptions) -> String {
    use base64::Engine as _;
    let mut params: Vec<String> = vec![format!(
        "inline={}",
        if options.inline != Some(false) { 1 } else { 0 }
    )];

    if let Some(width) = &options.width {
        params.push(format!("width={width}"));
    }
    if let Some(height) = &options.height {
        params.push(format!("height={height}"));
    }
    if let Some(name) = &options.name {
        let name_base64 = base64::engine::general_purpose::STANDARD.encode(name.as_bytes());
        params.push(format!("name={name_base64}"));
    }
    if options.preserve_aspect_ratio == Some(false) {
        params.push("preserveAspectRatio=0".to_string());
    }

    format!("\x1b]1337;File={}:{}\x07", params.join(";"), base64_data)
}

pub fn calculate_image_rows(
    image_dimensions: ImageDimensions,
    target_width_cells: usize,
    cell_dims: CellDimensions,
) -> usize {
    let target_width_px = target_width_cells * cell_dims.width_px;
    let scale = target_width_px as f64 / image_dimensions.width_px as f64;
    let scaled_height_px = image_dimensions.height_px as f64 * scale;
    let rows = (scaled_height_px / cell_dims.height_px as f64).ceil();
    (rows.max(1.0)) as usize
}

// ─── Image dimension sniffing (byte-level, port of Buffer reads) ──────────

fn decode_base64(base64_data: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .ok()
}

fn read_u32be(b: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([b[offset], b[offset + 1], b[offset + 2], b[offset + 3]])
}

fn read_u16be(b: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([b[offset], b[offset + 1]])
}

fn read_u16le(b: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([b[offset], b[offset + 1]])
}

fn read_u32le(b: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([b[offset], b[offset + 1], b[offset + 2], b[offset + 3]])
}

pub fn get_png_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let buffer = decode_base64(base64_data)?;
    if buffer.len() < 24 {
        return None;
    }
    if buffer[0] != 0x89 || buffer[1] != 0x50 || buffer[2] != 0x4e || buffer[3] != 0x47 {
        return None;
    }
    let width = read_u32be(&buffer, 16);
    let height = read_u32be(&buffer, 20);
    Some(ImageDimensions {
        width_px: width as usize,
        height_px: height as usize,
    })
}

pub fn get_jpeg_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let buffer = decode_base64(base64_data)?;
    if buffer.len() < 2 {
        return None;
    }
    if buffer[0] != 0xff || buffer[1] != 0xd8 {
        return None;
    }

    let mut offset = 2;
    // TS: `offset < buffer.length - 9` — JS length-9 can be negative for
    // short buffers; equivalent to offset + 9 < length.
    while offset + 9 < buffer.len() {
        if buffer[offset] != 0xff {
            offset += 1;
            continue;
        }
        let marker = buffer[offset + 1];
        if (0xc0..=0xc2).contains(&marker) {
            let height = read_u16be(&buffer, offset + 5);
            let width = read_u16be(&buffer, offset + 7);
            return Some(ImageDimensions {
                width_px: width as usize,
                height_px: height as usize,
            });
        }
        // NOTE: the TS defensive `offset + 3 >= buffer.length` check is
        // subsumed by the loop guard (`offset + 9 < len` ⇒ `offset + 3 < len`).
        let length = read_u16be(&buffer, offset + 2);
        if length < 2 {
            return None;
        }
        offset += 2 + length as usize;
    }
    None
}

pub fn get_gif_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let buffer = decode_base64(base64_data)?;
    if buffer.len() < 10 {
        return None;
    }
    let sig = std::str::from_utf8(&buffer[0..6]).ok()?;
    if sig != "GIF87a" && sig != "GIF89a" {
        return None;
    }
    let width = read_u16le(&buffer, 6);
    let height = read_u16le(&buffer, 8);
    Some(ImageDimensions {
        width_px: width as usize,
        height_px: height as usize,
    })
}

pub fn get_webp_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let buffer = decode_base64(base64_data)?;
    if buffer.len() < 30 {
        return None;
    }
    let riff = std::str::from_utf8(&buffer[0..4]).ok()?;
    let webp = std::str::from_utf8(&buffer[8..12]).ok()?;
    if riff != "RIFF" || webp != "WEBP" {
        return None;
    }

    let chunk = std::str::from_utf8(&buffer[12..16]).ok()?;
    // All chunk arms read within 30 bytes — guaranteed by the guard above.
    if chunk == "VP8 " {
        let width = (read_u16le(&buffer, 26) & 0x3fff) as usize;
        let height = (read_u16le(&buffer, 28) & 0x3fff) as usize;
        return Some(ImageDimensions {
            width_px: width,
            height_px: height,
        });
    } else if chunk == "VP8L" {
        let bits = read_u32le(&buffer, 21);
        let width = (bits & 0x3fff) as usize + 1;
        let height = ((bits >> 14) & 0x3fff) as usize + 1;
        return Some(ImageDimensions {
            width_px: width,
            height_px: height,
        });
    } else if chunk == "VP8X" {
        let width =
            (buffer[24] as u32 | (buffer[25] as u32) << 8 | (buffer[26] as u32) << 16) as usize + 1;
        let height =
            (buffer[27] as u32 | (buffer[28] as u32) << 8 | (buffer[29] as u32) << 16) as usize + 1;
        return Some(ImageDimensions {
            width_px: width,
            height_px: height,
        });
    }
    None
}

pub fn get_image_dimensions(base64_data: &str, mime_type: &str) -> Option<ImageDimensions> {
    match mime_type {
        "image/png" => get_png_dimensions(base64_data),
        "image/jpeg" => get_jpeg_dimensions(base64_data),
        "image/gif" => get_gif_dimensions(base64_data),
        "image/webp" => get_webp_dimensions(base64_data),
        _ => None,
    }
}

pub struct RenderImageResult {
    pub sequence: String,
    pub rows: usize,
    pub image_id: Option<u32>,
}

pub fn render_image(
    base64_data: &str,
    image_dimensions: ImageDimensions,
    options: &ImageRenderOptions,
) -> Option<RenderImageResult> {
    let caps = get_capabilities();
    if caps.images == ImageProtocol::None {
        return None;
    }

    let max_width = options.max_width_cells.unwrap_or(80);
    let rows = calculate_image_rows(image_dimensions, max_width, get_cell_dimensions());

    if caps.images == ImageProtocol::Kitty {
        let sequence = encode_kitty(
            base64_data,
            &KittyEncodeOptions {
                columns: Some(max_width),
                rows: Some(rows),
                image_id: options.image_id,
                move_cursor: options.move_cursor,
            },
        );
        return Some(RenderImageResult {
            sequence,
            rows,
            image_id: options.image_id,
        });
    }

    // Iterm2 is the only remaining protocol (None returned above).
    let sequence = encode_iterm2(
        base64_data,
        &Iterm2EncodeOptions {
            width: Some(max_width.to_string()),
            height: Some("auto".to_string()),
            name: None,
            preserve_aspect_ratio: options.preserve_aspect_ratio,
            inline: None,
        },
    );
    Some(RenderImageResult {
        sequence,
        rows,
        image_id: None,
    })
}

pub fn hyperlink(text: &str, url: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\")
}

pub fn image_fallback(
    mime_type: &str,
    dimensions: Option<ImageDimensions>,
    filename: Option<&str>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(filename) = filename {
        parts.push(filename.to_string());
    }
    parts.push(format!("[{mime_type}]"));
    if let Some(dimensions) = dimensions {
        parts.push(format!("{}x{}", dimensions.width_px, dimensions.height_px));
    }
    format!("[Image: {}]", parts.join(" "))
}

// ─── Kitty image ID extraction & batch deletion ────────────────────────────

const KITTY_PREFIX_BYTE: &str = "\x1b_G";

/// Port of `Number(value)` for the `i=<id>` param: JS `Number` accepts
/// decimals/leading signs/scientific notation; we approximate with f64
/// parsing (hex `0x..` forms are not supported by Rust's f64 parser —
/// pathological, never produced by `encodeKitty`).
fn js_number(value: &str) -> Option<f64> {
    let t = value.trim();
    if t.is_empty() {
        return Some(0.0);
    }
    t.parse::<f64>().ok()
}

/// Extract Kitty image IDs from a terminal line (`i=<id>` in the escape
/// sequence parameters). Returns the FIRST match, like TS (early return).
pub fn extract_kitty_image_ids(line: &str) -> Vec<u32> {
    if line.is_empty() {
        return Vec::new();
    }
    let Some(sequence_start) = line.find(KITTY_PREFIX_BYTE) else {
        return Vec::new();
    };
    let params_start = sequence_start + KITTY_PREFIX_BYTE.len();
    let Some(rel) = line[params_start..].find(';') else {
        return Vec::new();
    };
    let params_end = rel + params_start;
    let params = &line[params_start..params_end];
    for param in params.split(',') {
        let mut it = param.splitn(2, '=');
        let key = it.next().unwrap_or("");
        let value = it.next();
        if key != "i" || value.is_none() {
            continue;
        }
        let value = value.unwrap();
        let id = js_number(value);
        match id {
            Some(id) if id.fract() == 0.0 && id > 0.0 && id <= 0xffff_ffffu64 as f64 => {
                return vec![id as u32];
            }
            _ => continue,
        }
    }
    Vec::new()
}

/// Collect all Kitty image IDs across an array of rendered lines.
pub fn collect_kitty_image_ids(lines: &[String]) -> std::collections::BTreeSet<u32> {
    let mut ids = std::collections::BTreeSet::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        for id in extract_kitty_image_ids(line) {
            ids.insert(id);
        }
    }
    ids
}

/// Build the escape sequences to delete a set of Kitty images.
pub fn delete_kitty_images(ids: &std::collections::BTreeSet<u32>) -> String {
    let mut buffer = String::new();
    for id in ids {
        buffer.push_str(&delete_kitty_image(*id));
    }
    buffer
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::lock as env_lock;
    use base64::Engine as _;

    /// Deterministic env snapshot/restore under ENV_LOCK. Mirrors the TS
    /// tests which set process.env directly.
    fn with_env(caps_env: &[(&str, &str)], f: impl FnOnce()) {
        let _guard = env_lock();
        let keys = [
            "TERM_PROGRAM",
            "TERM",
            "COLORTERM",
            "TMUX",
            "KITTY_WINDOW_ID",
            "GHOSTTY_RESOURCES_DIR",
            "WEZTERM_PANE",
            "ITERM_SESSION_ID",
        ];
        let saved: Vec<(&str, Option<std::ffi::OsString>)> =
            keys.iter().map(|k| (*k, env::var_os(k))).collect();
        for (k, _) in &saved {
            env::remove_var(k);
        }
        for (k, v) in caps_env {
            env::set_var(k, v);
        }
        f();
        for (k, v) in saved {
            match v {
                Some(v) => env::set_var(k, v),
                None => env::remove_var(k),
            }
        }
    }

    #[test]
    fn detects_kitty_via_term_program() {
        with_env(&[("TERM_PROGRAM", "kitty")], || {
            let caps = detect_capabilities();
            assert_eq!(caps.images, ImageProtocol::Kitty);
            assert!(caps.true_color);
            assert!(caps.hyperlinks);
        });
    }

    #[test]
    fn detects_alacritty_caps() {
        // Pre-set a managed variable so with_env's restore path (which
        // re-installs saved values) is exercised too.
        {
            let _guard = env_lock();
            env::set_var("TERM_PROGRAM", "outer");
        }
        with_env(&[("TERM_PROGRAM", "alacritty")], || {
            let caps = detect_capabilities();
            assert_eq!(caps.images, ImageProtocol::None);
            assert!(caps.true_color);
            assert!(caps.hyperlinks);
        });
        let _guard = env_lock();
        env::remove_var("TERM_PROGRAM");
    }

    #[test]
    fn detects_kitty_via_window_id_env() {
        with_env(&[("KITTY_WINDOW_ID", "1")], || {
            let caps = detect_capabilities();
            assert_eq!(caps.images, ImageProtocol::Kitty);
        });
    }

    #[test]
    fn detects_ghostty() {
        with_env(&[("TERM_PROGRAM", "ghostty")], || {
            let caps = detect_capabilities();
            assert_eq!(caps.images, ImageProtocol::Kitty);
        });
    }

    #[test]
    fn detects_wezterm() {
        with_env(&[("WEZTERM_PANE", "0")], || {
            let caps = detect_capabilities();
            assert_eq!(caps.images, ImageProtocol::Kitty);
        });
    }

    #[test]
    fn detects_iterm2() {
        with_env(&[("ITERM_SESSION_ID", "x")], || {
            let caps = detect_capabilities();
            assert_eq!(caps.images, ImageProtocol::Iterm2);
        });
    }

    #[test]
    fn tmux_disables_images_and_hyperlinks() {
        with_env(&[("TERM", "tmux-256color")], || {
            let caps = detect_capabilities();
            assert_eq!(caps.images, ImageProtocol::None);
            assert!(!caps.hyperlinks);
        });
    }

    #[test]
    fn tmux_with_truecolor_colorterm() {
        with_env(&[("TERM", "screen"), ("COLORTERM", "truecolor")], || {
            let caps = detect_capabilities();
            assert_eq!(caps.images, ImageProtocol::None);
            assert!(caps.true_color);
            assert!(!caps.hyperlinks);
        });
    }

    #[test]
    fn vscode_has_hyperlinks_no_images() {
        with_env(&[("TERM_PROGRAM", "vscode")], || {
            let caps = detect_capabilities();
            assert_eq!(caps.images, ImageProtocol::None);
            assert!(caps.hyperlinks);
            assert!(caps.true_color);
        });
    }

    #[test]
    fn default_environment_no_images() {
        with_env(&[], || {
            let caps = detect_capabilities();
            assert_eq!(caps.images, ImageProtocol::None);
            assert!(!caps.hyperlinks);
            assert!(!caps.true_color);
        });
    }

    #[test]
    fn default_colorterm_truecolor() {
        with_env(&[("COLORTERM", "24bit")], || {
            let caps = detect_capabilities();
            assert_eq!(caps.images, ImageProtocol::None);
            assert!(caps.true_color);
        });
    }

    #[test]
    fn capabilities_cache_and_reset() {
        let _guard = env_lock();
        env::remove_var("TERM_PROGRAM");
        env::remove_var("TERM");
        env::remove_var("COLORTERM");
        env::remove_var("TMUX");
        env::remove_var("KITTY_WINDOW_ID");
        env::remove_var("GHOSTTY_RESOURCES_DIR");
        env::remove_var("WEZTERM_PANE");
        env::remove_var("ITERM_SESSION_ID");
        reset_capabilities_cache();
        let caps = get_capabilities();
        assert_eq!(caps.images, ImageProtocol::None);
        // setCapabilities overrides the cache
        set_capabilities(TerminalCapabilities {
            images: ImageProtocol::Kitty,
            true_color: true,
            hyperlinks: true,
        });
        assert_eq!(get_capabilities().images, ImageProtocol::Kitty);
        reset_capabilities_cache();
        let caps = get_capabilities();
        assert_eq!(caps.images, ImageProtocol::None);
    }

    #[test]
    fn is_image_line_detects_prefixes() {
        assert!(is_image_line("\x1b_Ga=T,f=100;AAAA\x1b\\"));
        assert!(is_image_line("\x1b]1337;File=inline=1:AAAA\x07"));
        assert!(!is_image_line("plain text"));
        assert!(!is_image_line(""));
        // embedded (not at start)
        assert!(is_image_line("text \x1b_Ga=T;AA\x1b\\ more"));
    }

    #[test]
    fn allocate_image_id_in_range() {
        for _ in 0..1000 {
            let id = allocate_image_id();
            assert!(id >= 1);
            assert!(id <= 0xffff_fffe);
        }
    }

    #[test]
    fn encode_kitty_single_chunk() {
        let seq = encode_kitty(
            "QUJD",
            &KittyEncodeOptions {
                columns: Some(80),
                rows: Some(20),
                image_id: Some(7),
                move_cursor: None,
            },
        );
        assert_eq!(seq, "\x1b_Ga=T,f=100,q=2,c=80,r=20,i=7;QUJD\x1b\\");
    }

    #[test]
    fn encode_kitty_no_options() {
        let seq = encode_kitty("QUJD", &KittyEncodeOptions::default());
        assert_eq!(seq, "\x1b_Ga=T,f=100,q=2;QUJD\x1b\\");
    }

    #[test]
    fn encode_kitty_move_cursor_false_adds_c1() {
        let seq = encode_kitty(
            "QUJD",
            &KittyEncodeOptions {
                columns: None,
                rows: None,
                image_id: None,
                move_cursor: Some(false),
            },
        );
        assert_eq!(seq, "\x1b_Ga=T,f=100,q=2,C=1;QUJD\x1b\\");
    }

    #[test]
    fn encode_kitty_three_chunks_cover_middle_branch() {
        // > 2 chunks: first carries params + m=1, middle chunks bare m=1,
        // the last m=0.
        let payload = "A".repeat(4096 * 2 + 10);
        let seq = encode_kitty(&payload, &KittyEncodeOptions::default());
        assert_eq!(seq.matches("\x1b_G").count(), 3);
        assert!(seq.contains("m=1"));
        assert!(seq.contains("m=0;"));
        // Reassembling the chunk payloads reproduces the original payload.
        let mut reassembled = String::new();
        for chunk in seq.split("\x1b_G").skip(1) {
            let data = chunk.split(';').nth(1).unwrap().strip_suffix("\x1b\\").unwrap();
            reassembled.push_str(data);
        }
        assert_eq!(reassembled, payload);
    }

    #[test]
    fn encode_kitty_chunked() {
        // 5000-char base64 payload → 2 chunks (4096 + 904)
        let payload = "A".repeat(5000);
        let seq = encode_kitty(&payload, &KittyEncodeOptions::default());
        assert!(seq.starts_with("\x1b_Ga=T,f=100,q=2,m=1;"));
        assert!(seq.contains("\x1b_Gm=0;"));
        assert!(seq.ends_with("\x1b\\"));
        // First chunk carries 4096 payload chars, last chunk the remainder
        let first_end = seq.find("\x1b_Gm=0;").unwrap();
        assert!(seq[..first_end].ends_with(&format!("{}\x1b\\", "A".repeat(4096))));
        assert!(seq[first_end..].starts_with("\x1b_Gm=0;"));
        assert!(seq.ends_with(&format!("{}\x1b\\", "A".repeat(904))));
    }

    #[test]
    fn encode_kitty_chunked_exactly_4096() {
        // Exactly 4096 chars → single chunk (length <= CHUNK_SIZE)
        let payload = "A".repeat(4096);
        let seq = encode_kitty(&payload, &KittyEncodeOptions::default());
        assert_eq!(
            seq,
            format!("\x1b_Ga=T,f=100,q=2;{}\x1b\\", "A".repeat(4096))
        );
    }

    #[test]
    fn delete_kitty_sequences() {
        assert_eq!(delete_kitty_image(42), "\x1b_Ga=d,d=I,i=42,q=2\x1b\\");
        assert_eq!(delete_all_kitty_images(), "\x1b_Ga=d,d=A,q=2\x1b\\");
    }

    #[test]
    fn encode_iterm2_defaults_inline_1() {
        let seq = encode_iterm2("QUJD", &Iterm2EncodeOptions::default());
        assert_eq!(seq, "\x1b]1337;File=inline=1:QUJD\x07");
    }

    #[test]
    fn encode_iterm2_with_options() {
        let seq = encode_iterm2(
            "QUJD",
            &Iterm2EncodeOptions {
                width: Some("80".into()),
                height: Some("auto".into()),
                name: Some("pic.png".into()),
                preserve_aspect_ratio: Some(false),
                inline: Some(false),
            },
        );
        // name is base64 of "pic.png"
        let name_b64 = base64::engine::general_purpose::STANDARD.encode("pic.png".as_bytes());
        assert_eq!(
            seq,
            format!(
                "\x1b]1337;File=inline=0;width=80;height=auto;name={name_b64};preserveAspectRatio=0:QUJD\x07"
            )
        );
    }

    #[test]
    fn calculate_image_rows_scales_to_width() {
        let dims = ImageDimensions {
            width_px: 100,
            height_px: 100,
        };
        let cells = CellDimensions {
            width_px: 10,
            height_px: 20,
        };
        // target 200px wide → scale 2 → 200px tall → 10 rows
        assert_eq!(calculate_image_rows(dims, 20, cells), 10);
        // tiny image → min 1 row
        assert_eq!(calculate_image_rows(dims, 1, cells), 1);
    }

    // ─── Dimension sniffing (real minimal byte fixtures) ────────────────

    fn png_bytes() -> Vec<u8> {
        // 1x1 PNG: signature + IHDR (width=1, height=1) + minimal tail
        let mut b = vec![
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, // signature
            0x00, 0x00, 0x00, 0x0d, // IHDR length
            b'I', b'H', b'D', b'R',
        ];
        b.extend_from_slice(&1u32.to_be_bytes()); // width
        b.extend_from_slice(&1u32.to_be_bytes()); // height
        b.extend_from_slice(&[8, 6, 0, 0, 0]); // bit depth, color type, etc.
        b
    }

    fn jpeg_bytes() -> Vec<u8> {
        // SOI + APP0 + SOF0 (width=640, height=480) + EOI
        let mut b = vec![0xff, 0xd8]; // SOI
        b.extend_from_slice(&[0xff, 0xe0, 0x00, 0x10]); // APP0 len 16
        b.extend_from_slice(b"JFIF\x00"); // identifier
        b.extend_from_slice(&[0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]); // JFIF data
        b.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08]); // SOF0 len 17 precision 8
        b.extend_from_slice(&480u16.to_be_bytes()); // height
        b.extend_from_slice(&640u16.to_be_bytes()); // width
        b.extend_from_slice(&[0x03, 0x01, 0x11, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01]); // components
        b.extend_from_slice(&[0xff, 0xd9]); // EOI
        b
    }

    fn gif_bytes() -> Vec<u8> {
        let mut b = b"GIF89a".to_vec();
        b.extend_from_slice(&10u16.to_le_bytes()); // width
        b.extend_from_slice(&8u16.to_le_bytes()); // height
        b.extend_from_slice(&[0, 0, 0]); // flags, bg, aspect
        b
    }

    fn webp_bytes() -> Vec<u8> {
        // RIFF + WEBP + VP8X. The TS parser reads width-1 from bytes
        // 24-26 and height-1 from 27-29 (its VP8X layout assumption) —
        // place the fields exactly there.
        let mut b = b"RIFF".to_vec();
        b.extend_from_slice(&[0, 0, 0, 0]); // RIFF size (unused)
        b.extend_from_slice(b"WEBP");
        b.extend_from_slice(b"VP8X");
        b.extend_from_slice(&[0, 0, 0, 0]); // chunk size (unused)
        b.extend_from_slice(&[0, 0, 0, 0]); // flags + filler to byte 24
        b.extend_from_slice(&[99, 0, 0]); // width-1 = 99 at bytes 24-26
        b.extend_from_slice(&[49, 0, 0]); // height-1 = 49 at bytes 27-29
        b
    }

    fn b64(b: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(b)
    }

    #[test]
    fn png_dimensions() {
        let dims = get_png_dimensions(&b64(&png_bytes())).unwrap();
        assert_eq!((dims.width_px, dims.height_px), (1, 1));
        assert!(get_png_dimensions("not base64!!").is_none());
        assert!(get_png_dimensions(&b64(&[1, 2, 3])).is_none());
        // wrong signature
        let mut bad = png_bytes();
        bad[0] = 0x00;
        assert!(get_png_dimensions(&b64(&bad)).is_none());
    }

    #[test]
    fn jpeg_dimensions() {
        let dims = get_jpeg_dimensions(&b64(&jpeg_bytes())).unwrap();
        assert_eq!((dims.width_px, dims.height_px), (640, 480));
        assert!(get_jpeg_dimensions(&b64(&[0xff, 0xd8, 0xff, 0xd9])).is_none());
    }

    #[test]
    fn gif_dimensions() {
        let dims = get_gif_dimensions(&b64(&gif_bytes())).unwrap();
        assert_eq!((dims.width_px, dims.height_px), (10, 8));
        // GIF87a accepted too
        let mut b = gif_bytes();
        b[4] = b'7';
        let dims = get_gif_dimensions(&b64(&b)).unwrap();
        assert_eq!((dims.width_px, dims.height_px), (10, 8));
    }

    #[test]
    fn webp_dimensions() {
        let dims = get_webp_dimensions(&b64(&webp_bytes())).unwrap();
        assert_eq!((dims.width_px, dims.height_px), (100, 50));
    }

    #[test]
    fn jpeg_dimension_edge_cases() {
        // Too short / bad magic.
        assert!(get_jpeg_dimensions(&b64(&[0x00])).is_none());
        assert!(get_jpeg_dimensions(&b64(&[0x00, 0x00])).is_none());
        // No SOF marker: non-0xff bytes are skipped until the window closes.
        let mut buf = vec![0xff, 0xd8];
        buf.extend_from_slice(&[0x00; 20]);
        assert!(get_jpeg_dimensions(&b64(&buf)).is_none());
        // A segment whose declared length is < 2 is invalid.
        let buf = [
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert!(get_jpeg_dimensions(&b64(&buf)).is_none());
        // A valid-length non-SOF segment is skipped, then the buffer ends.
        let buf = [0xff, 0xd8, 0xff, 0xe0, 0x00, 0x02];
        assert!(get_jpeg_dimensions(&b64(&buf)).is_none());
    }

    #[test]
    fn gif_dimension_edge_cases() {
        // Too short / bad signature.
        assert!(get_gif_dimensions(&b64(&[0u8; 5])).is_none());
        assert!(get_gif_dimensions(&b64(&[0u8; 10])).is_none());
    }

    fn webp_buf(chunk: &[u8; 4]) -> Vec<u8> {
        let mut buf = vec![0u8; 30];
        buf[0..4].copy_from_slice(b"RIFF");
        buf[8..12].copy_from_slice(b"WEBP");
        buf[12..16].copy_from_slice(chunk);
        buf
    }

    #[test]
    fn webp_dimension_edge_cases() {
        // Too short / bad RIFF / bad WEBP.
        assert!(get_webp_dimensions(&b64(&[0u8; 10])).is_none());
        assert!(get_webp_dimensions(&b64(&[0u8; 30])).is_none());
        let mut bad = webp_buf(b"VP8 ");
        bad[8] = b'X';
        assert!(get_webp_dimensions(&b64(&bad)).is_none());

        // VP8 (lossy): LE width/height at 26/28, masked to 14 bits.
        let mut buf = webp_buf(b"VP8 ");
        buf[26] = 0x40;
        buf[27] = 0x01; // 0x140 = 320
        buf[28] = 0x80;
        buf[29] = 0x02; // 0x280 = 640
        let dims = get_webp_dimensions(&b64(&buf)).unwrap();
        assert_eq!((dims.width_px, dims.height_px), (320, 640));

        // VP8L: 14-bit (width-1)/(height-1) packed in the LE u32 at 21.
        let mut buf = webp_buf(b"VP8L");
        let bits: u32 = 9 | (4 << 14); // width 10, height 5
        buf[21] = (bits & 0xff) as u8;
        buf[22] = ((bits >> 8) & 0xff) as u8;
        buf[23] = ((bits >> 16) & 0xff) as u8;
        buf[24] = ((bits >> 24) & 0xff) as u8;
        let dims = get_webp_dimensions(&b64(&buf)).unwrap();
        assert_eq!((dims.width_px, dims.height_px), (10, 5));

        // VP8X: 24-bit (width-1) at 24..27 and (height-1) at 27..30.
        let mut buf = webp_buf(b"VP8X");
        buf[24] = 0x2f;
        buf[25] = 0x01; // 0x12f + 1 = 304
        buf[27] = 0x63; // 0x63 + 1 = 100
        let dims = get_webp_dimensions(&b64(&buf)).unwrap();
        assert_eq!((dims.width_px, dims.height_px), (304, 100));

        // Unknown chunk type → None.
        assert!(get_webp_dimensions(&b64(&webp_buf(b"VP8?"))).is_none());
    }

    #[test]
    fn js_number_mirrors_js_semantics() {
        assert_eq!(js_number(""), Some(0.0));
        assert_eq!(js_number("   "), Some(0.0));
        assert_eq!(js_number("42"), Some(42.0));
        assert_eq!(js_number("abc"), None);
    }

    #[test]
    fn extract_kitty_image_ids_early_returns() {
        assert!(extract_kitty_image_ids("").is_empty());
        // Kitty prefix with no terminating ';'.
        assert!(extract_kitty_image_ids("\x1b_Ga=T").is_empty());
    }

    #[test]
    fn seed_from_time_and_addr_has_nonzero_fallback() {
        // Craft inputs that mix to zero → fixed fallback kicks in.
        let nanos = 0x1234_5678_9abc_def0u64;
        let addr = (nanos ^ 0x9e37_79b9_7f4a_7c15u64).rotate_right(17);
        assert_eq!(seed_from_time_and_addr(nanos, addr), 0x2545_f491_4f6c_dd1d);
        // Ordinary input passes through the mix.
        assert_eq!(
            seed_from_time_and_addr(1, 0),
            1u64 ^ 0x9e37_79b9_7f4a_7c15u64
        );
    }

    #[test]
    fn image_dimensions_by_mime() {
        let p = b64(&png_bytes());
        assert!(get_image_dimensions(&p, "image/png").is_some());
        assert!(get_image_dimensions(&p, "image/jpeg").is_none());
        assert!(get_image_dimensions(&p, "image/unknown").is_none());
    }

    #[test]
    fn render_image_kitty() {
        let _guard = env_lock();
        set_capabilities(TerminalCapabilities {
            images: ImageProtocol::Kitty,
            true_color: true,
            hyperlinks: true,
        });
        let dims = ImageDimensions {
            width_px: 100,
            height_px: 100,
        };
        let result = render_image("QUJD", dims, &ImageRenderOptions::default()).unwrap();
        // 100x100 @ maxWidth 80 cells, cell 9x18 → scale 7.2 → 40 rows
        assert_eq!(result.rows, 40);
        assert!(result
            .sequence
            .starts_with("\x1b_Ga=T,f=100,q=2,c=80,r=40;"));
        assert!(result.image_id.is_none());
    }

    #[test]
    fn render_image_iterm2() {
        let _guard = env_lock();
        set_capabilities(TerminalCapabilities {
            images: ImageProtocol::Iterm2,
            true_color: true,
            hyperlinks: true,
        });
        let dims = ImageDimensions {
            width_px: 100,
            height_px: 100,
        };
        let result = render_image("QUJD", dims, &ImageRenderOptions::default()).unwrap();
        assert_eq!(result.rows, 40);
        assert!(result
            .sequence
            .starts_with("\x1b]1337;File=inline=1;width=80;height=auto:"));
        assert!(result.image_id.is_none());
    }

    #[test]
    fn render_image_none_capabilities_returns_null() {
        let _guard = env_lock();
        set_capabilities(TerminalCapabilities {
            images: ImageProtocol::None,
            true_color: true,
            hyperlinks: false,
        });
        let dims = ImageDimensions {
            width_px: 100,
            height_px: 100,
        };
        assert!(render_image("QUJD", dims, &ImageRenderOptions::default()).is_none());
    }

    #[test]
    fn hyperlink_wraps_text() {
        assert_eq!(
            hyperlink("text", "https://x.com"),
            "\x1b]8;;https://x.com\x1b\\text\x1b]8;;\x1b\\"
        );
    }

    #[test]
    fn image_fallback_formats() {
        assert_eq!(
            image_fallback("image/png", None, None),
            "[Image: [image/png]]"
        );
        let dims = ImageDimensions {
            width_px: 640,
            height_px: 480,
        };
        assert_eq!(
            image_fallback("image/png", Some(dims), Some("pic.png")),
            "[Image: pic.png [image/png] 640x480]"
        );
    }

    #[test]
    fn extract_kitty_ids_from_line() {
        let line = "\x1b_Ga=T,f=100,c=80,r=5,i=42;QUJD\x1b\\";
        assert_eq!(extract_kitty_image_ids(line), vec![42u32]);
        // no i= param
        assert_eq!(
            extract_kitty_image_ids("\x1b_Ga=T,f=100;QUJD\x1b\\"),
            Vec::<u32>::new()
        );
        // no kitty prefix
        assert_eq!(extract_kitty_image_ids("plain"), Vec::<u32>::new());
        // invalid id
        assert_eq!(
            extract_kitty_image_ids("\x1b_Ga=T,i=abc;QUJD\x1b\\"),
            Vec::<u32>::new()
        );
        assert_eq!(
            extract_kitty_image_ids("\x1b_Ga=T,i=-5;QUJD\x1b\\"),
            Vec::<u32>::new()
        );
    }

    #[test]
    fn collect_and_delete_kitty_ids() {
        let lines = vec![
            "\x1b_Ga=T,i=1;A\x1b\\".to_string(),
            "\x1b_Ga=T,i=2;A\x1b\\".to_string(),
            "plain".to_string(),
            "".to_string(),
        ];
        let ids = collect_kitty_image_ids(&lines);
        assert_eq!(ids.len(), 2);
        let seq = delete_kitty_images(&ids);
        assert_eq!(
            seq,
            "\x1b_Ga=d,d=I,i=1,q=2\x1b\\\x1b_Ga=d,d=I,i=2,q=2\x1b\\"
        );
    }

    #[test]
    fn cell_dimensions_global() {
        let _guard = env_lock();
        let saved = get_cell_dimensions();
        set_cell_dimensions(CellDimensions {
            width_px: 10,
            height_px: 20,
        });
        assert_eq!(get_cell_dimensions().width_px, 10);
        set_cell_dimensions(saved);
    }
}
