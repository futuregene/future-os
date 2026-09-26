//! Paste folding and image attachments — the two things that happen to text
//! *arriving from outside the keyboard* (a bracketed paste, a dragged-in file
//! path) before it reaches the input box.
//!
//! A paste longer than [`FOLD_THRESHOLD`] never enters the box as text: it is
//! stored here and replaced by a `[Pasted Content N chars]` placeholder, so a
//! 5 MB log costs one display unit instead of 5 MB of wrapping, cursor math
//! and re-rendering. The placeholder is display-only — submission expands it
//! back to the exact bytes the user pasted, and a placeholder the user deleted
//! is never expanded (see `Input::expanded`). `[Image #N]` works the same way:
//! it is a marker for an [`Attachment`] that travels beside the message.
//!
//! Everything in here is pure except [`resolve_image_path`], which reads the
//! bytes it sniffs — the widget layer holds no policy of its own, so the
//! naming, expansion, renumbering and sniffing rules are all unit-testable
//! without a terminal.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use base64::Engine as _;

/// A paste longer than this many **characters** folds into a placeholder.
/// Characters, not bytes: 900 CJK characters are 900, not 2700, and an emoji
/// is one.
///
/// The number is the input box's own height, in characters: the box is three
/// lines, so at the 80-column width it renders (`Input::render`) roughly 240
/// characters are visible before the text starts scrolling the draft out of
/// sight. Folding at 1000 (what the reference implementation uses) meant a
/// paste could fill and overflow the box while still being shown as text —
/// which is the thing the placeholder exists to prevent. 200 keeps a sentence
/// or a short URL inline and folds anything that would not fit.
pub const FOLD_THRESHOLD: usize = 200;

/// Hard cap on the text a draft can assemble into.
///
/// The reference implementation allows 1M characters. Measured on this widget
/// (release, aarch64, `Input::render` at 80 columns — the layout rebuild that
/// every edit triggers) 1M characters costs 3.2 s per edit, 500k costs 843 ms
/// and 200k costs 132 ms, so 1M is not a limit this input can carry honestly.
/// 100k lands at ~34 ms for the whole visible draft (`insert` itself is
/// sub-millisecond) — past the point where typing feels immediate, but far
/// short of the second-long stalls, and ~25k tokens of prompt in one message
/// is already a lot to send by accident.
pub const MAX_MESSAGE_CHARS: usize = 100_000;

// ─── Character counting ────────────────────────────────────────────────────

/// The length that matters for every rule here: characters, never bytes.
pub fn char_len(text: &str) -> usize {
    text.chars().count()
}

/// The rejection message for a draft that would exceed [`MAX_MESSAGE_CHARS`].
/// `Some` means "refuse and say why"; `None` means the insert fits.
pub fn over_limit_message(resulting_chars: usize) -> Option<String> {
    (resulting_chars > MAX_MESSAGE_CHARS).then(|| {
        format!(
            "Message exceeds the maximum length of {MAX_MESSAGE_CHARS} characters \
             ({resulting_chars} provided)."
        )
    })
}

// ─── Normalisation ─────────────────────────────────────────────────────────

/// Line endings and tabs, normalised the way the input box has always
/// normalised them (`\r\n` and `\r` → `\n`, a tab → four spaces).
pub fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "    ")
}

// ─── Folded pastes ─────────────────────────────────────────────────────────

/// The placeholder for the `index`-th paste of `len` characters (`index` is
/// 1-based; the first paste of a size carries no suffix).
pub fn placeholder_name(len: usize, index: usize) -> String {
    if index <= 1 {
        format!("[Pasted Content {len} chars]")
    } else {
        format!("[Pasted Content {len} chars #{index}]")
    }
}

/// The first placeholder name for a `len`-character paste that `draft` does
/// not already contain. Because the check runs against the *draft*, deleting
/// the `#1` placeholder frees `#1` for the next paste of that size instead of
/// growing the suffix forever.
pub fn free_placeholder_name(draft: &str, len: usize) -> String {
    let mut index = 1;
    loop {
        let name = placeholder_name(len, index);
        if !draft.contains(&name) {
            return name;
        }
        index += 1;
    }
}

/// Byte ranges of the well-formed `[Pasted Content N chars]` placeholders in
/// `value`, left to right. A truncated or mistyped fragment (`[Pasted Content
/// 2000 cha`) is not a placeholder: it stays literal text.
pub fn find_placeholders(value: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut search_from = 0;
    while let Some(offset) = value[search_from..].find('[') {
        let start = search_from + offset;
        match placeholder_end(value, start) {
            Some(end) => {
                found.push((start, end));
                search_from = end;
            }
            // `[` that opens nothing: step past it and keep looking (the byte
            // is ASCII, so `start + 1` is a char boundary).
            None => search_from = start + 1,
        }
    }
    found
}

/// `[Pasted Content <digits> chars]` / `… chars #<digits>]` starting at byte
/// `start`; the exclusive end offset when it is a complete, well-formed token.
fn placeholder_end(value: &str, start: usize) -> Option<usize> {
    let rest = value.get(start..)?;
    let rest = rest.strip_prefix("[Pasted Content ")?;
    let digits = rest.find(|c: char| !c.is_ascii_digit())?;
    // At least one digit, and no leading whitespace-only body.
    if digits == 0 {
        return None;
    }
    let rest = rest[digits..].strip_prefix(" chars")?;
    let rest = match rest.strip_prefix(" #") {
        Some(after_hash) => {
            let digits = after_hash.find(|c: char| !c.is_ascii_digit())?;
            if digits == 0 {
                return None;
            }
            &after_hash[digits..]
        }
        None => rest,
    };
    let rest = rest.strip_prefix(']')?;
    Some(value.len() - rest.len())
}

/// Replace every placeholder in `draft` that `store` knows about with the text
/// it stands for. A placeholder with no entry (a fragment restored from
/// history or a session draft cache) is left alone rather than silently
/// dropped.
pub fn expand(draft: &str, store: &HashMap<String, String>) -> String {
    if store.is_empty() || !draft.contains('[') {
        return draft.to_string();
    }
    let mut out = String::with_capacity(draft.len());
    let mut cursor = 0;
    for (start, end) in find_placeholders(draft) {
        let Some(content) = store.get(&draft[start..end]) else {
            continue;
        };
        out.push_str(&draft[cursor..start]);
        out.push_str(content);
        cursor = end;
    }
    out.push_str(&draft[cursor..]);
    out
}

// ─── Image markers ─────────────────────────────────────────────────────────

/// The marker an attachment shows up as in the draft.
pub fn image_marker(index: usize) -> String {
    format!("[Image #{index}]")
}

/// Byte ranges (plus the referenced number) of the `[Image #N]` markers in
/// `value`, left to right. Hand-typed text that merely looks like a marker is
/// indistinguishable from a real one — the number is what decides, and a
/// number with no attachment behind it is left as text.
pub fn find_image_markers(value: &str) -> Vec<(usize, usize, usize)> {
    let mut found = Vec::new();
    let mut search_from = 0;
    while let Some(offset) = value[search_from..].find('[') {
        let start = search_from + offset;
        match image_marker_end(value, start) {
            Some((end, number)) => {
                found.push((start, end, number));
                search_from = end;
            }
            // `[` that opens nothing: step past it and keep looking (the byte
            // is ASCII, so `start + 1` is a char boundary).
            None => search_from = start + 1,
        }
    }
    found
}

/// `[Image #<digits>]` starting at byte `start` → (exclusive end, number).
fn image_marker_end(value: &str, start: usize) -> Option<(usize, usize)> {
    let rest = value.get(start..)?.strip_prefix("[Image #")?;
    let digits = rest.find(|c: char| !c.is_ascii_digit())?;
    if digits == 0 {
        return None;
    }
    let number = rest[..digits].parse::<usize>().ok()?;
    let rest = rest[digits..].strip_prefix(']')?;
    Some((value.len() - rest.len(), number))
}

/// One marker that has to be rewritten so the numbering stays contiguous.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerRewrite {
    /// Byte offset of the marker in the old value.
    pub start: usize,
    /// Exclusive byte end of the marker in the old value.
    pub end: usize,
    /// What replaces it.
    pub text: String,
}

/// Re-derive the attachment order from the markers a draft still contains.
///
/// `tracked` is how many attachments the caller is holding. Returns the new
/// order (indices into that list, 1-based, in order of first appearance) and
/// the rewrites that renumber the markers to match — so deleting `[Image #1]`
/// turns `[Image #2]` into `[Image #1]`. Markers pointing past `tracked` are
/// not ours and stay literal text.
pub fn sync_image_markers(value: &str, tracked: usize) -> (Vec<usize>, Vec<MarkerRewrite>) {
    let mut order: Vec<usize> = Vec::new();
    let mut rewrites: Vec<MarkerRewrite> = Vec::new();
    for (start, end, number) in find_image_markers(value) {
        if number == 0 || number > tracked {
            continue;
        }
        if !order.contains(&number) {
            order.push(number);
        }
        let new_number = order.iter().position(|n| *n == number).unwrap_or(0) + 1;
        if new_number != number {
            rewrites.push(MarkerRewrite {
                start,
                end,
                text: image_marker(new_number),
            });
        }
    }
    (order, rewrites)
}

// ─── Pasted paths ──────────────────────────────────────────────────────────

/// The local paths a paste could be naming, most literal first.
///
/// Handles the five shapes that reach a terminal: a bare path, a
/// single- or double-quoted path, a `file://` URL (percent-decoded, so a
/// browser-copied `/tmp/a%20b.png` resolves), and the backslash-escaped space
/// a drag-and-drop produces. The literal reading is tried before the
/// unescaped one because a file whose name really contains a backslash and a
/// space exists on POSIX.
///
/// Returns empty for anything that cannot be a single path: multi-line text,
/// empty text, text with no path shape at all, and relative paths (an
/// attachment has to be absolute — that is what the agent reads).
pub fn path_candidates(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains('\n') {
        return Vec::new();
    }
    if let Some(rest) = trimmed.strip_prefix("file://") {
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        let decoded = percent_decode(rest);
        // `file:///C:/x.png` arrives as `/C:/x.png`; the extra leading slash is
        // the URL's, not the path's.
        let decoded = strip_windows_url_slash(&decoded);
        return absolute_only(vec![decoded]);
    }
    let unquoted = strip_matching_quotes(trimmed).unwrap_or(trimmed);
    let mut candidates = vec![unquoted.to_string()];
    if let Some(unescaped) = unescape_spaces(unquoted) {
        candidates.push(unescaped);
    }
    absolute_only(candidates)
}

fn absolute_only(candidates: Vec<String>) -> Vec<String> {
    candidates
        .into_iter()
        .filter(|candidate| Path::new(candidate).is_absolute())
        .collect()
}

/// `"…"` or `'…'` around the whole string (and nothing else).
fn strip_matching_quotes(text: &str) -> Option<&str> {
    for quote in ['"', '\''] {
        if let Some(inner) = text.strip_prefix(quote).and_then(|t| t.strip_suffix(quote)) {
            // A lone `"` is not a quoted path.
            if !inner.is_empty() {
                return Some(inner);
            }
        }
    }
    None
}

/// `\ ` → ` `, but only for the space escape: shells escape spaces that way
/// when a path is dragged in, and a general `\X` → `X` would turn a Windows
/// path (`C:\Users\…`) into a different, wrong path.
fn unescape_spaces(text: &str) -> Option<String> {
    if !text.contains("\\ ") {
        return None;
    }
    Some(text.replace("\\ ", " "))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(byte) = hex {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

/// Windows URLs carry `/C:/…`; POSIX paths keep their leading slash.
fn strip_windows_url_slash(text: &str) -> String {
    let bytes = text.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[2] == b':' && bytes[1].is_ascii_alphabetic() {
        return text[1..].to_string();
    }
    text.to_string()
}

/// The image formats the sniffer recognises. TUI has no image decoder and
/// will not grow one for this: the magic bytes are enough to tell a real image
/// from a `.png`-named text file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
    Bmp,
}

/// Sniff an image format from a file's leading bytes (12 are enough for every
/// format here).
pub fn sniff_image_format(head: &[u8]) -> Option<ImageFormat> {
    if head.len() < 2 {
        return None;
    }
    if head.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some(ImageFormat::Png);
    }
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(ImageFormat::Jpeg);
    }
    if head.starts_with(b"GIF8") {
        return Some(ImageFormat::Gif);
    }
    if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
        return Some(ImageFormat::Webp);
    }
    if head.starts_with(b"BM") {
        return Some(ImageFormat::Bmp);
    }
    None
}

/// A pasted path that is really an image: the file exists, is readable and its
/// bytes start like an image. Extension is never consulted — a text file named
/// `notes.png` is not an image, and the agent would not be able to show it.
///
/// Returns the absolute path and the name to display, or `None` to mean "treat
/// this paste as text".
pub fn resolve_image_path(text: &str) -> Option<(String, String)> {
    for candidate in path_candidates(text) {
        let path = PathBuf::from(&candidate);
        if !path.is_file() || !is_image_file(&path) {
            continue;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| candidate.clone());
        return Some((candidate, name));
    }
    None
}

/// Read the magic bytes of `path` and decide whether it is an image. Any I/O
/// failure — missing, a directory, no permission, a file too short to carry a
/// header — answers `false` — the paste then stays text, which is the safe
/// reading.
pub fn is_image_file(path: &Path) -> bool {
    match read_head(path) {
        Some(head) => sniff_image_format(&head).is_some(),
        None => false,
    }
}

// ─── The input box's attachment line ───────────────────────────────────────

/// The line the input box shows above the prompt while images are waiting to
/// be sent, or `None` when there is nothing to say.
///
/// `supported` is the active model's `supports_images`: `Some(false)` adds the
/// warning, because the agent degrades an image the model cannot take into a
/// file path in the prompt — the picture is not going to be seen, and the user
/// has to be told that while they can still switch models.
pub fn attachment_notice(images: usize, supported: Option<bool>) -> Option<String> {
    if images == 0 {
        return None;
    }
    let noun = if images == 1 { "image" } else { "images" };
    let mut line = format!("📎 {images} {noun} attached");
    if supported == Some(false) {
        line.push_str(" · ⚠ this model cannot view images — sent as a file path");
    }
    Some(line)
}

// ─── Clipboard capture (`ctrl+v`) ──────────────────────────────────────────

/// Directory under the system temp directory holding images captured from the
/// clipboard.
///
/// A captured file has to outlive the message that references it: the agent
/// reads the image by path *after* the prompt is sent (`agent/src/rpc/
/// prompt_helpers.rs`), so deleting it at submit would break the attachment.
/// Nothing here deletes a file it just wrote — they are swept by age, see
/// [`sweep_stale_clipboard_files`].
pub const CLIPBOARD_DIR_NAME: &str = "future-tui-clipboard";

/// Age at which a captured clipboard file is swept.
///
/// A day is long past any run in which the agent could still be reading the
/// file, and short enough that a machine where the user pastes screenshots
/// daily cannot accumulate without bound.
pub const CLIPBOARD_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1000;

/// How long one clipboard tool may run. Every tool here is local and answers in
/// milliseconds; a child still running after this is wedged (a permission
/// prompt, a dead compositor) and gets killed rather than freezing the TUI.
pub const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);

/// How an image probe hands the image over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payload {
    /// The probe writes the image to the file path given in its own arguments
    /// (`osascript`, PowerShell). Nothing is read from its stdout.
    File,
    /// The probe prints the image base64-encoded on stdout.
    ///
    /// The Linux tools (`wl-paste`, `xclip`) can only print the image, and the
    /// spawn machinery captures text — base64 is what makes those bytes survive
    /// it. The encoders on those hosts wrap their output, so decoding drops
    /// whitespace first.
    Base64OnStdout,
}

/// One external command: a program and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// An image probe: a command plus the way it hands the image over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageProbe {
    pub command: CaptureCommand,
    pub payload: Payload,
}

/// The commands one platform needs: image probes (most likely to work first)
/// and the text fallbacks tried when the clipboard holds no image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureCommands {
    pub image: Vec<ImageProbe>,
    pub text: Vec<CaptureCommand>,
}

/// What one clipboard read found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardPaste {
    /// An image, written to `path` (validated by its magic bytes) — attach it.
    Image { path: String, name: String },
    /// The clipboard holds text: insert it the way any paste is inserted.
    Text(String),
    /// Nothing usable, with a reason worth showing the user.
    Unavailable(String),
}

/// Spawns one clipboard command: `(exit code, stdout, stderr)`, or `Err` when
/// no process ran at all (a missing tool, a timeout). Injected so no test of
/// [`ClipboardCapture::capture`] ever runs a real clipboard program.
pub type CaptureRunner =
    Box<dyn Fn(&str, &[String]) -> Result<(i32, String, String), String> + Send + Sync>;

/// The commands that read this platform's clipboard.
///
/// `path` is where an image is to be written; only the probes that write the
/// file themselves ([`Payload::File`]) embed it. `None` for a platform with no
/// known clipboard tool at all.
pub fn capture_commands_for(os: &str, path: &Path) -> Option<CaptureCommands> {
    match os.trim().to_ascii_lowercase().as_str() {
        "macos" | "mac" | "darwin" | "ios" => Some(CaptureCommands {
            // `the clipboard as «class PNGf»` raises when the clipboard holds no
            // PNG (the ordinary case), and the file is only opened after that
            // read, so a text-only clipboard leaves no half-written file behind.
            image: vec![ImageProbe {
                command: CaptureCommand {
                    program: "osascript".to_string(),
                    args: osascript_args(path),
                },
                payload: Payload::File,
            }],
            text: vec![CaptureCommand {
                program: "pbpaste".to_string(),
                args: Vec::new(),
            }],
        }),
        "windows" | "win32" | "windows_nt" => Some(CaptureCommands {
            image: vec![ImageProbe {
                command: CaptureCommand {
                    program: "powershell".to_string(),
                    args: vec![
                        "-NoProfile".to_string(),
                        // `Clipboard::GetImage` needs a single-threaded
                        // apartment, which is what `-STA` gives it.
                        "-STA".to_string(),
                        "-Command".to_string(),
                        powershell_image_script(path),
                    ],
                },
                payload: Payload::File,
            }],
            text: vec![CaptureCommand {
                program: "powershell".to_string(),
                args: vec![
                    "-NoProfile".to_string(),
                    "-Command".to_string(),
                    "Get-Clipboard -Raw".to_string(),
                ],
            }],
        }),
        "linux" | "android" => Some(CaptureCommands {
            image: vec![
                base64_probe("wl-paste --type image/png"),
                base64_probe("xclip -selection clipboard -t image/png -o"),
            ],
            text: vec![
                CaptureCommand {
                    program: "wl-paste".to_string(),
                    args: Vec::new(),
                },
                CaptureCommand {
                    program: "xclip".to_string(),
                    args: vec![
                        "-selection".to_string(),
                        "clipboard".to_string(),
                        "-o".to_string(),
                    ],
                },
            ],
        }),
        _ => None,
    }
}

/// The AppleScript that writes the clipboard's PNG to `path`.
///
/// Four `-e` lines rather than one string with newlines: `osascript` joins them
/// itself, so the script never depends on how a shell would treat a multi-line
/// argument.
fn osascript_args(path: &Path) -> Vec<String> {
    let target = applescript_literal(&path.to_string_lossy());
    vec![
        "-e".to_string(),
        "set png to (the clipboard as «class PNGf»)".to_string(),
        "-e".to_string(),
        format!("set target to open for access POSIX file {target} with write permission"),
        "-e".to_string(),
        "write png to target".to_string(),
        "-e".to_string(),
        "close access target".to_string(),
    ]
}

/// A single-quoted PowerShell string literal: `'` is escaped by doubling it.
fn powershell_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// The PowerShell that saves the clipboard image to `path` as a PNG.
fn powershell_image_script(path: &Path) -> String {
    let target = powershell_literal(&path.to_string_lossy());
    format!(
        "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; \
         $image = [System.Windows.Forms.Clipboard]::GetImage(); \
         if ($null -eq $image) {{ exit 3 }}; \
         $image.Save({target}, [System.Drawing.Imaging.ImageFormat]::Png)"
    )
}

/// An image probe that prints the image base64-encoded, for a tool that can only
/// write to stdout.
fn base64_probe(script: &str) -> ImageProbe {
    ImageProbe {
        command: CaptureCommand {
            program: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                format!("{script} | base64"),
                // Named so the shell's own diagnostics say which tool failed.
                "sh".to_string(),
            ],
        },
        payload: Payload::Base64OnStdout,
    }
}

/// A quoted AppleScript string literal.
fn applescript_literal(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Reads the system clipboard for a `ctrl+v`.
pub struct ClipboardCapture {
    runner: CaptureRunner,
    dir: PathBuf,
    seq: AtomicU64,
}

impl ClipboardCapture {
    /// Real tools, system temp directory.
    pub fn new() -> Self {
        Self::with_runner(
            Box::new(spawn_capture),
            std::env::temp_dir().join(CLIPBOARD_DIR_NAME),
        )
    }

    /// Test seam: an injected runner and an explicit output directory.
    pub fn with_runner(runner: CaptureRunner, dir: PathBuf) -> Self {
        Self {
            runner,
            dir,
            seq: AtomicU64::new(0),
        }
    }

    /// The directory captured images are written to.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Read the clipboard: an image when it holds one, its text otherwise.
    ///
    /// `os` picks the platform's tools (a parameter so every table is testable
    /// on any host), `pid` plus the capture time name the file, and `now_ms` is
    /// the clock the sweep runs against.
    pub fn capture(&self, os: &str, pid: u32, now_ms: u64) -> ClipboardPaste {
        // One path per capture: the probe that writes the file itself is told
        // where to put it, and every later step reads the same file.
        let path = self.fresh_path(pid, now_ms);
        let Some(commands) = capture_commands_for(os, &path) else {
            return ClipboardPaste::Unavailable(format!(
                "no clipboard tool is known for this platform ({os})"
            ));
        };
        // Sweep first: the directory can only hold *our* files, and this is the
        // one moment we know the user is here (an unattended sweep would have to
        // be a timer).
        sweep_stale_clipboard_files(&self.dir, now_ms);
        for probe in &commands.image {
            match self.image_from(probe, &path) {
                Ok(Some(captured)) => {
                    return ClipboardPaste::Image {
                        name: captured.name,
                        path: captured.path,
                    }
                }
                // The tool ran and the clipboard held no image of that flavour:
                // try the next probe, then the text fallback.
                Ok(None) => {}
                // The tool could not run at all. The text fallback may still
                // work, so this is not reported yet.
                Err(_) => {}
            }
        }
        match self.text_from(&commands.text) {
            Ok(Some(text)) => ClipboardPaste::Text(text),
            // Not an image and not text: the ordinary empty clipboard.
            Ok(None) => ClipboardPaste::Unavailable(
                "the clipboard holds neither an image nor text".to_string(),
            ),
            // No text tool could be started — say which one, that is the fix.
            Err(error) => ClipboardPaste::Unavailable(error),
        }
    }

    /// Try one image probe at `path`. `Ok(None)` means "not an image" (including
    /// every way the clipboard can simply hold no image), `Err` means the tool
    /// could not be run.
    fn image_from(&self, probe: &ImageProbe, path: &Path) -> Result<Option<Captured>, String> {
        prepare_dir(&self.dir)
            .map_err(|error| format!("could not create {}: {error}", self.dir.display()))?;
        // A probe that writes the file itself must not inherit an earlier
        // probe's bytes: the tools truncate only as much as they write.
        let _ = std::fs::remove_file(path);
        let (code, stdout, _stderr) = (self.runner)(&probe.command.program, &probe.command.args)?;
        // Most clipboard tools exit non-zero when the clipboard holds nothing of
        // the requested type (osascript raises, `xclip` exits 1): the ordinary
        // "no image" answer, not a failure.
        if code != 0 {
            return Ok(None);
        }
        match probe.payload {
            Payload::File => {
                // The probe wrote the file itself; nothing there means no image.
                if !is_non_empty_file(path) {
                    return Ok(None);
                }
            }
            Payload::Base64OnStdout => {
                let Some(bytes) = decode_base64(&stdout) else {
                    return Ok(None);
                };
                if bytes.is_empty() {
                    return Ok(None);
                }
                std::fs::write(path, &bytes)
                    .map_err(|error| format!("could not write {}: {error}", path.display()))?;
            }
        }
        let Some(head) = read_head(path) else {
            return Ok(None);
        };
        let Some(format) = sniff_image_format(&head) else {
            // The tool answered, but the bytes are not an image (a tool that
            // echoes the clipboard whatever it holds). Not an attachment — and
            // not junk left in the temp directory either.
            let _ = std::fs::remove_file(path);
            return Ok(None);
        };
        let path = rename_to_format(path, format);
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Some(Captured {
            path: path.to_string_lossy().into_owned(),
            name,
        }))
    }

    /// Try the text tools in order. `Ok(None)` means every one of them ran (or
    /// was skipped) and none had anything; `Err` means none of them could be
    /// started, with the last failure's message.
    fn text_from(&self, commands: &[CaptureCommand]) -> Result<Option<String>, String> {
        let mut last_error = None;
        for command in commands {
            match (self.runner)(&command.program, &command.args) {
                Ok((0, stdout, _stderr)) => {
                    if !stdout.trim().is_empty() {
                        return Ok(Some(stdout));
                    }
                }
                // A non-zero exit is "that tool had nothing" (a text tool on an
                // image-only clipboard), so the next candidate gets its turn.
                Ok(_) => {}
                Err(error) => last_error = Some(error),
            }
        }
        match last_error {
            Some(error) => Err(error),
            None => Ok(None),
        }
    }

    /// A path no other capture can be using: the directory, the capture time,
    /// the process and a counter that makes two captures in the same
    /// millisecond distinct.
    fn fresh_path(&self, pid: u32, now_ms: u64) -> PathBuf {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        self.dir.join(clipboard_file_name(now_ms, pid, seq))
    }
}

impl Default for ClipboardCapture {
    fn default() -> Self {
        Self::new()
    }
}

/// A captured image file: where it is and what to call it.
struct Captured {
    path: String,
    name: String,
}

/// The file name one capture writes (`clipboard-<epoch_ms>-<pid>-<seq>.png`).
///
/// The capture time is part of the name because that is what
/// [`sweep_stale_clipboard_files`] reads: no filesystem timestamps, so a sweep
/// is testable by moving the clock.
pub fn clipboard_file_name(epoch_ms: u64, pid: u32, seq: u64) -> String {
    format!("clipboard-{epoch_ms}-{pid}-{seq}.png")
}

/// How long ago the file `name` was captured, or `None` when it is not one of
/// ours (or claims a time in the future).
pub fn clipboard_age_ms(name: &str, now_ms: u64) -> Option<u64> {
    let rest = name.strip_prefix("clipboard-")?;
    let stamp = rest.split('-').next()?;
    now_ms.checked_sub(stamp.parse::<u64>().ok()?)
}

/// Delete captured files older than [`CLIPBOARD_MAX_AGE_MS`]; returns how many
/// went. A missing directory sweeps nothing (nothing has been captured yet),
/// and a name that is not ours is never touched.
pub fn sweep_stale_clipboard_files(dir: &Path, now_ms: u64) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|entry| stale_capture(&entry, now_ms))
        .filter(|path| std::fs::remove_file(path).is_ok())
        .count()
}

/// The path of one directory entry that is an old capture of ours, or `None`
/// when it is too recent (or not ours at all).
///
/// The name is read lossily because a file name need not be UTF-8 while ours
/// always is: a name that cannot be read is not one we wrote, and the timestamp
/// in it is what decides.
fn stale_capture(entry: &std::fs::DirEntry, now_ms: u64) -> Option<PathBuf> {
    let name = entry.file_name().to_string_lossy().into_owned();
    let age = clipboard_age_ms(&name, now_ms)?;
    (age >= CLIPBOARD_MAX_AGE_MS).then(|| entry.path())
}

/// The file extension for a sniffed format, so the agent picks its decoder by
/// the path it is handed.
pub fn image_extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Gif => "gif",
        ImageFormat::Webp => "webp",
        ImageFormat::Bmp => "bmp",
    }
}

/// Rename a captured file to the extension its magic bytes earned. Best effort:
/// a rename that fails leaves the file where it is, which is still readable by
/// the agent (the sniffer, not the name, is what decided it is an image).
fn rename_to_format(path: &Path, format: ImageFormat) -> PathBuf {
    let target = path.with_extension(image_extension(format));
    match std::fs::rename(path, &target) {
        Ok(()) => target,
        Err(_) => path.to_path_buf(),
    }
}

/// Create the capture directory if it is not there yet.
fn prepare_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// Is `path` a file with at least one byte in it?
fn is_non_empty_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.len() > 0)
        .unwrap_or(false)
}

/// The first bytes of `path`, or `None` when it cannot be read that far.
fn read_head(path: &Path) -> Option<[u8; 12]> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 12];
    file.read_exact(&mut head).ok()?;
    Some(head)
}

/// Decode base64 straight off stdout: encoders on these platforms wrap at 76
/// columns, so whitespace is dropped first. `None` for anything that is not
/// base64 — that is "no image", not a failure to report.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let compact: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(compact)
        .ok()
}

/// Real [`CaptureRunner`]: the shared capped spawner.
///
/// Reusing `skills_cli::spawn_with_timeout` is deliberate: it drains both pipes
/// while the child runs (so a large clipboard cannot deadlock the child on a
/// full pipe), kills and reaps a child that outlives the deadline, and never
/// hands the child the TUI's raw-mode stdin. A second copy of that machinery is
/// how its defects would come back.
fn spawn_capture(program: &str, args: &[String]) -> Result<(i32, String, String), String> {
    crate::skills_cli::spawn_with_timeout(Path::new(program), args, CAPTURE_TIMEOUT)
        .map_err(humanize_capture_error)
}

/// Re-word the shared spawner's timeout marker, which names the skill CLI: a
/// `ctrl+v` that gave up must not read as "future skills: timed out".
fn humanize_capture_error(error: String) -> String {
    match error.strip_prefix(crate::skills_cli::TIMEOUT_MARKER) {
        Some(rest) => format!("the clipboard tool timed out{rest}"),
        None => error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    // ─── thresholds ────────────────────────────────────────────────────

    #[test]
    fn threshold_counts_characters_not_bytes() {
        // The boundary is inclusive of the threshold itself: a paste of exactly
        // `FOLD_THRESHOLD` characters is still text, one more folds. Written
        // relative to the constant so moving it does not need this test edited.
        assert_eq!(
            char_len(&"a".repeat(FOLD_THRESHOLD - 1)),
            FOLD_THRESHOLD - 1
        );
        assert!(!folds(&"a".repeat(FOLD_THRESHOLD - 1)));
        assert!(!folds(&"a".repeat(FOLD_THRESHOLD)));
        assert!(folds(&"a".repeat(FOLD_THRESHOLD + 1)));
        // CJK: the count is characters, not bytes. One character under the
        // threshold is three times that many bytes and must still be text.
        let cjk = "汉".repeat(FOLD_THRESHOLD - 1);
        assert_eq!(cjk.len(), (FOLD_THRESHOLD - 1) * 3);
        assert_eq!(char_len(&cjk), FOLD_THRESHOLD - 1);
        assert!(!folds(&cjk));
        assert!(folds(&"汉".repeat(FOLD_THRESHOLD + 1)));
        // An emoji is one character, even though it is 4 bytes.
        let emoji = "🚀".repeat(FOLD_THRESHOLD + 1);
        assert_eq!(char_len(&emoji), FOLD_THRESHOLD + 1);
        assert!(folds(&emoji));
    }

    /// The input's own rule: a paste folds when it is longer than the
    /// threshold (whitespace-only and ANSI-carrying pastes included — the rule
    /// is about length, and reading content would make it surprising).
    fn folds(text: &str) -> bool {
        char_len(text) > FOLD_THRESHOLD
    }

    #[test]
    fn whitespace_only_and_ansi_pastes_follow_the_length_rule() {
        assert!(folds(&" ".repeat(FOLD_THRESHOLD + 1)));
        assert!(!folds(&" ".repeat(FOLD_THRESHOLD)));
        // The rule is about length alone: the text is never inspected for
        // content, so escape sequences count as the characters they are. A
        // paste of exactly the threshold plus its escapes is therefore over.
        let ansi = format!("\x1b[31m{}\x1b[0m", "x".repeat(FOLD_THRESHOLD + 1));
        assert!(folds(&ansi));
        let ansi_exact = format!("\x1b[31m{}\x1b[0m", "x".repeat(FOLD_THRESHOLD));
        let escapes = char_len(&ansi_exact) - FOLD_THRESHOLD;
        assert_eq!(escapes, 9, "\x1b[31m is 5 characters, \x1b[0m is 4");
        assert!(folds(&ansi_exact), "the escapes push it past the line");
        // Drop the escapes and the same text is at the line, not over it.
        assert!(!folds(&"x".repeat(FOLD_THRESHOLD)));
    }

    #[test]
    fn normalize_folds_line_endings_and_tabs() {
        assert_eq!(normalize("a\r\nb\rc\td"), "a\nb\nc    d");
    }

    // ─── placeholder naming ────────────────────────────────────────────

    #[test]
    fn placeholder_names_suffix_repeats_and_reuse_a_freed_number() {
        assert_eq!(placeholder_name(2000, 1), "[Pasted Content 2000 chars]");
        assert_eq!(placeholder_name(2000, 2), "[Pasted Content 2000 chars #2]");

        // First paste of this size, then a second one beside it.
        let mut draft = String::new();
        let first = free_placeholder_name(&draft, 2000);
        assert_eq!(first, "[Pasted Content 2000 chars]");
        draft.push_str(&first);
        let second = free_placeholder_name(&draft, 2000);
        assert_eq!(second, "[Pasted Content 2000 chars #2]");
        draft.push_str(&second);

        // Deleting the first frees `#1`: the next paste reuses it instead of
        // growing the suffix.
        let after_delete = draft.replacen(&first, "", 1);
        assert_eq!(
            free_placeholder_name(&after_delete, 2000),
            "[Pasted Content 2000 chars]"
        );
        // A different size is independent of those numbers.
        assert_eq!(
            free_placeholder_name(&after_delete, 1500),
            "[Pasted Content 1500 chars]"
        );
    }

    // ─── expansion ─────────────────────────────────────────────────────

    #[test]
    fn expand_replaces_every_known_placeholder_and_keeps_the_rest() {
        let a = placeholder_name(2000, 1);
        let b = placeholder_name(2000, 2);
        let text = format!("before {a} middle {b} after");
        let expanded = expand(&text, &store(&[(a.as_str(), "AAA"), (b.as_str(), "BBB")]));
        assert_eq!(expanded, "before AAA middle BBB after");

        // No placeholders, nothing to do (and the store may be empty).
        assert_eq!(expand("plain text", &HashMap::new()), "plain text");
        assert_eq!(expand(&a, &store(&[(a.as_str(), "AAA")])), "AAA");
    }

    #[test]
    fn expand_leaves_an_unknown_or_truncated_placeholder_as_text() {
        let known = placeholder_name(2000, 1);
        // A placeholder whose text was edited by hand is not a placeholder.
        let truncated = &known[..known.len() - 3];
        let text = format!("{truncated} tail");
        assert_eq!(
            expand(&text, &store(&[(known.as_str(), "AAA")])),
            text,
            "a partial placeholder must not expand"
        );
        // A well-formed placeholder the store has never seen (restored from a
        // session draft) likewise stays as the user sees it.
        let unknown = placeholder_name(3000, 1);
        assert_eq!(
            expand(&unknown, &store(&[(known.as_str(), "AAA")])),
            unknown
        );
        // Malformed shapes are not placeholders at all — not even when the
        // store would answer the nearest well-formed reading of them.
        let greedy = store(&[
            ("[Pasted Content  chars]", "X"),
            ("[Pasted Content 12 chars]", "Y"),
        ]);
        for bad in [
            "[Pasted Content chars]",
            "[Pasted Content  chars]",
            "[Pasted Content 12 chars",
            "[Pasted Content 12 chars #]",
            "[Pasted Content 12 chars #x]",
            "[Pasted Content 12 chars#2]",
        ] {
            assert_eq!(expand(bad, &greedy), bad, "{bad} must not expand");
        }
    }

    #[test]
    fn expand_repeats_a_duplicated_placeholder() {
        let a = placeholder_name(2000, 1);
        let text = format!("{a} and {a}");
        assert_eq!(
            expand(&text, &store(&[(a.as_str(), "AAA")])),
            "AAA and AAA",
            "a copied placeholder names the same stored text twice"
        );
    }

    #[test]
    fn expand_handles_placeholders_around_wide_characters() {
        let a = placeholder_name(1200, 1);
        let text = format!("中文{a}🚀尾巴");
        assert_eq!(expand(&text, &store(&[(a.as_str(), "X")])), "中文X🚀尾巴");
        assert_eq!(
            find_placeholders(&text),
            vec![("中文".len(), "中文".len() + a.len())]
        );
    }

    // ─── image markers ─────────────────────────────────────────────────

    #[test]
    fn image_markers_find_only_well_formed_numbers() {
        let value = format!("{}x{}", image_marker(1), image_marker(12));
        assert_eq!(
            find_image_markers(&value),
            vec![
                (0, image_marker(1).len(), 1),
                (image_marker(1).len() + 1, value.len(), 12),
            ]
        );
        for bad in [
            "[Image #]",
            "[Image #x]",
            "[Image 1]",
            "[Image #1",
            "[image #1]",
        ] {
            assert!(find_image_markers(bad).is_empty(), "{bad}");
        }
    }

    #[test]
    fn sync_renumbers_after_a_deletion_and_drops_the_rest() {
        // Two images attached: #1 #2, both markers present → nothing to do.
        let full = format!("{} {}", image_marker(1), image_marker(2));
        let (order, rewrites) = sync_image_markers(&full, 2);
        assert_eq!(order, vec![1, 2]);
        assert!(rewrites.is_empty());

        // The user deletes the first marker: #2 becomes #1.
        let after = format!("  {}", image_marker(2));
        let (order, rewrites) = sync_image_markers(&after, 2);
        assert_eq!(order, vec![2]);
        assert_eq!(
            rewrites,
            vec![MarkerRewrite {
                start: 2,
                end: 2 + image_marker(2).len(),
                text: image_marker(1),
            }]
        );

        // Both gone → no attachments at all.
        let (order, rewrites) = sync_image_markers("nothing here", 2);
        assert!(order.is_empty());
        assert!(rewrites.is_empty());
    }

    #[test]
    fn sync_treats_a_number_with_no_attachment_as_text() {
        // A hand-typed marker past the tracked range is not ours: it stays put
        // and does not consume a slot.
        let value = format!("{} {}", image_marker(1), image_marker(7));
        let (order, rewrites) = sync_image_markers(&value, 1);
        assert_eq!(order, vec![1]);
        assert!(rewrites.is_empty());
        // A tracking-consistent draft with an out-of-range marker at the front
        // keeps the real one numbered where the user put it.
        let value = format!("{} {}", image_marker(3), image_marker(1));
        let (order, rewrites) = sync_image_markers(&value, 1);
        assert_eq!(order, vec![1]);
        assert!(rewrites.is_empty());
    }

    #[test]
    fn sync_keeps_a_duplicated_marker_on_the_same_attachment() {
        let value = format!("{} {}", image_marker(1), image_marker(2));
        // Copy-paste of the second marker, then the deletion of the first:
        // both copies refer to attachment 2 and both become #1.
        let value = format!("{value} {}", image_marker(2));
        let value = value.replacen(&format!("{} ", image_marker(1)), "", 1);
        let (order, rewrites) = sync_image_markers(&value, 2);
        assert_eq!(order, vec![2]);
        assert_eq!(rewrites.len(), 2);
        assert!(rewrites.iter().all(|r| r.text == image_marker(1)));
    }

    // ─── path shapes ───────────────────────────────────────────────────

    #[test]
    fn path_candidates_cover_the_five_paste_shapes() {
        // The *shapes* are what this test is about; whether `/tmp/a.png`
        // counts as absolute is the host's business (`Path::is_absolute`
        // requires a drive on Windows), so the fixtures are spelled for the
        // host instead of assuming POSIX.
        let root = if cfg!(windows) { "C:/tmp" } else { "/tmp" };
        // A `file://` URL spells a Windows drive as `file:///C:/…`: the slash
        // the URL adds before the drive letter is not part of the path.
        let drive_slash = if cfg!(windows) { "/" } else { "" };
        let png = format!("{root}/a.png");
        let spaced = format!("{root}/a b.png");
        let escaped = format!("{root}/a\\ b.png");

        assert_eq!(path_candidates(&png), vec![png.clone()]);
        assert_eq!(
            path_candidates(&format!("\"{spaced}\"")),
            vec![spaced.clone()]
        );
        assert_eq!(
            path_candidates(&format!("'{spaced}'")),
            vec![spaced.clone()]
        );
        assert_eq!(
            path_candidates(&format!("file://{drive_slash}{png}")),
            vec![png.clone()]
        );
        assert_eq!(
            path_candidates(&format!(
                "file://{drive_slash}{}",
                spaced.replace(' ', "%20")
            )),
            vec![spaced.clone()]
        );
        assert_eq!(
            path_candidates(&format!("file://localhost{drive_slash}{png}")),
            vec![png.clone()]
        );
        // Backslash-escaped spaces: the literal reading first, then the
        // unescaped one.
        assert_eq!(
            path_candidates(&escaped),
            vec![escaped.clone(), spaced.clone()]
        );
        // Surrounding whitespace (a copy that picked up a newline) is trimmed.
        assert_eq!(path_candidates(&format!("  {png}\n")), vec![png.clone()]);
    }

    #[test]
    fn path_candidates_reject_text_that_cannot_be_a_single_path() {
        // Multi-line text is never one path.
        assert!(path_candidates("/tmp/a.png\n/tmp/b.png").is_empty());
        assert!(path_candidates("/tmp/a\n.png").is_empty());
        assert!(path_candidates("").is_empty());
        assert!(path_candidates("   \n\t").is_empty());
        // Relative paths are not attachments — the agent reads absolute ones.
        assert!(path_candidates("a.png").is_empty());
        assert!(path_candidates("./a.png").is_empty());
        assert!(path_candidates("~/a.png").is_empty());
        // A quote on one side only is not a quoted path.
        assert!(path_candidates("\"/tmp/a.png").is_empty());
        assert!(path_candidates("''").is_empty());
        // A file URL that is not absolute after decoding.
        assert!(path_candidates("file://relative.png").is_empty());
        // A bare word with a space that is not absolute.
        assert!(path_candidates("hello world").is_empty());
    }

    #[test]
    fn is_image_file_needs_the_header_it_actually_reads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let png = write(&dir, "real.png", b"\x89PNG\r\n\x1a\nbody bytes");
        assert!(is_image_file(&png));
        // A file that stops inside the header is not an image, however its
        // name reads — the sniffer never gets enough bytes to say yes.
        let stub = write(&dir, "stub.png", b"\x89PNG");
        assert!(!is_image_file(&stub));
        assert!(
            !is_image_file(dir.path()),
            "a directory is not readable as one"
        );
        assert!(!is_image_file(Path::new("/definitely/not/here.png")));
    }

    #[test]
    fn windows_style_file_urls_lose_the_url_slash() {
        // The URL form of `C:\shots\a.png`: the slash the URL adds in front of
        // the drive letter is not part of the path.
        assert_eq!(strip_windows_url_slash("/C:/shots/a.png"), "C:/shots/a.png");
        assert_eq!(strip_windows_url_slash("/tmp/a.png"), "/tmp/a.png");
        // Whether the decoded path is absolute depends on the host, and
        // `path_candidates` only ever hands back paths the host can resolve —
        // which is what the caller goes on to read. Both readings are built
        // unconditionally so this test says the same thing on either platform.
        let decoded = vec!["C:/shots/a.png".to_string()];
        let expected = if cfg!(windows) { decoded } else { Vec::new() };
        assert_eq!(path_candidates("file:///C:/shots/a.png"), expected);
    }

    #[test]
    fn percent_decode_leaves_broken_escapes_alone() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%2"), "%2");
        // Invalid UTF-8 after decoding falls back to the raw text.
        assert_eq!(percent_decode("%ff"), "%ff");
    }

    // ─── magic bytes ───────────────────────────────────────────────────

    #[test]
    fn sniff_image_format_recognises_each_format() {
        assert_eq!(
            sniff_image_format(b"\x89PNG\r\n\x1a\n"),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            sniff_image_format(b"\xff\xd8\xff\xe0"),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(sniff_image_format(b"GIF89a"), Some(ImageFormat::Gif));
        assert_eq!(sniff_image_format(b"GIF87a"), Some(ImageFormat::Gif));
        assert_eq!(
            sniff_image_format(b"RIFF\x24\x00\x00\x00WEBP"),
            Some(ImageFormat::Webp)
        );
        assert_eq!(sniff_image_format(b"BM\x36\x00"), Some(ImageFormat::Bmp));
        // Not images.
        assert_eq!(sniff_image_format(b"plain text\n"), None);
        assert_eq!(sniff_image_format(b"RIFF\x24\x00\x00\x00WAVE"), None);
        assert_eq!(sniff_image_format(b"B"), None);
        assert_eq!(sniff_image_format(b""), None);
    }

    // ─── filesystem: what counts as an image ───────────────────────────

    fn write(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).expect("write fixture");
        path
    }

    #[test]
    fn resolve_image_path_accepts_a_real_image_and_rejects_a_lookalike() {
        let dir = tempfile::tempdir().expect("tempdir");
        let png = write(&dir, "shot.png", b"\x89PNG\r\n\x1a\nrest of the file");
        let liar = write(&dir, "notes.png", b"this is text, not a png\n");
        let txt = write(&dir, "notes.txt", b"\x89PNG\r\n\x1a\nactually an image");

        let resolved = resolve_image_path(&png.to_string_lossy()).expect("a real PNG");
        assert_eq!(resolved.0, png.to_string_lossy());
        assert_eq!(resolved.1, "shot.png");

        // Extension says `.png`, bytes say text → text, not an attachment. The
        // agent cannot render it and the user must not believe otherwise.
        assert!(resolve_image_path(&liar.to_string_lossy()).is_none());
        // …and the reverse: a real image under a `.txt` name is still an image.
        assert!(resolve_image_path(&txt.to_string_lossy()).is_some());

        // Missing, a directory, and a non-path all fall back to text.
        assert!(resolve_image_path(&dir.path().join("nope.png").to_string_lossy()).is_none());
        assert!(resolve_image_path(&dir.path().to_string_lossy()).is_none());
        assert!(resolve_image_path("hello world").is_none());
        assert!(resolve_image_path("").is_none());
    }

    #[test]
    fn resolve_image_path_walks_the_quoted_and_escaped_shapes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let spaced = write(&dir, "a b.png", b"\x89PNG\r\n\x1a\nbody");
        let path = spaced.to_string_lossy().to_string();
        let quoted = format!("\"{path}\"");
        let single = format!("'{path}'");
        let escaped = path.replace(' ', "\\ ");
        let url = format!("file://{}", path.replace(' ', "%20"));
        for shape in [path.clone(), quoted, single, escaped, url] {
            let resolved = resolve_image_path(&shape)
                .unwrap_or_else(|| panic!("{shape} should resolve to an image"));
            assert_eq!(resolved.0, path, "{shape}");
            assert_eq!(resolved.1, "a b.png");
        }
    }

    // ─── the attachment line ───────────────────────────────────────────

    #[test]
    fn attachment_notice_counts_images_and_warns_only_when_it_knows() {
        assert_eq!(attachment_notice(0, Some(true)), None);
        assert_eq!(
            attachment_notice(1, Some(true)).unwrap(),
            "📎 1 image attached"
        );
        assert_eq!(attachment_notice(2, None).unwrap(), "📎 2 images attached");
        let warned = attachment_notice(1, Some(false)).unwrap();
        assert!(warned.contains("this model cannot view images"), "{warned}");
        assert!(warned.contains("1 image"), "{warned}");
    }

    // ─── the input cap ─────────────────────────────────────────────────

    #[test]
    fn over_limit_message_states_the_cap_and_the_attempted_length() {
        assert_eq!(over_limit_message(MAX_MESSAGE_CHARS), None);
        assert_eq!(over_limit_message(1), None);
        let message = over_limit_message(MAX_MESSAGE_CHARS + 1).unwrap();
        assert!(message.contains("100000 characters"), "{message}");
        assert!(message.contains("100001 provided"), "{message}");
    }

    // ─── clipboard capture: the platform tables ────────────────────────

    /// The calls an injected capture runner saw: `(program, args)`.
    type CaptureCalls = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<String>)>>>;

    /// The bytes of a tiny but real PNG (magic plus filler).
    fn png_bytes() -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"rest of the file");
        bytes
    }

    /// A capture runner that records every call and answers with `answer`.
    fn capture_runner<F>(calls: &CaptureCalls, answer: F) -> CaptureRunner
    where
        F: Fn(&str, &[String]) -> Result<(i32, String, String), String> + Send + Sync + 'static,
    {
        let sink = std::sync::Arc::clone(calls);
        Box::new(move |program: &str, args: &[String]| {
            sink.lock()
                .unwrap()
                .push((program.to_string(), args.to_vec()));
            answer(program, args)
        })
    }

    /// Every call recorded so far.
    fn capture_calls(calls: &CaptureCalls) -> Vec<(String, Vec<String>)> {
        calls.lock().unwrap().clone()
    }

    /// The path a macOS probe was told to write (`POSIX file "<path>"`).
    fn osascript_target(args: &[String]) -> PathBuf {
        let joined = args.join("\n");
        let marker = "POSIX file \"";
        let start = joined
            .find(marker)
            .unwrap_or_else(|| panic!("no target in {joined}"))
            + marker.len();
        let rest = &joined[start..];
        let end = rest.find('"').expect("the literal is closed");
        PathBuf::from(&rest[..end])
    }

    /// The path a PowerShell probe was told to write (`$image.Save('<path>',`).
    fn powershell_target(args: &[String]) -> PathBuf {
        let joined = args.join(" ");
        let marker = "$image.Save('";
        let start = joined
            .find(marker)
            .unwrap_or_else(|| panic!("no target in {joined}"))
            + marker.len();
        let rest = &joined[start..];
        let end = rest.find('\'').expect("the literal is closed");
        PathBuf::from(&rest[..end])
    }

    /// A capture whose runner is injected and whose files land in a temp dir.
    fn capture_with<F>(answer: F) -> (ClipboardCapture, CaptureCalls, tempfile::TempDir)
    where
        F: Fn(&str, &[String]) -> Result<(i32, String, String), String> + Send + Sync + 'static,
    {
        let calls: CaptureCalls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let dir = tempfile::tempdir().expect("tempdir");
        let capture = ClipboardCapture::with_runner(
            capture_runner(&calls, answer),
            dir.path().join("clipboard"),
        );
        (capture, calls, dir)
    }

    /// The `(path, name)` an outcome carries when it is an image, `None`
    /// otherwise. Total on purpose: the tests assert on it, so a wrong outcome
    /// fails an assertion (with the outcome printed) rather than a `panic!` arm
    /// that only ever exists to be compiled.
    fn image_parts(outcome: &ClipboardPaste) -> Option<(String, String)> {
        match outcome {
            ClipboardPaste::Image { path, name } => Some((path.clone(), name.clone())),
            ClipboardPaste::Text(_) | ClipboardPaste::Unavailable(_) => None,
        }
    }

    /// Why an outcome is unavailable. Total for the same reason as
    /// [`image_parts`].
    fn unavailable_reason(outcome: &ClipboardPaste) -> String {
        match outcome {
            ClipboardPaste::Unavailable(reason) => reason.clone(),
            ClipboardPaste::Image { path, .. } => format!("an image at {path}"),
            ClipboardPaste::Text(text) => format!("text: {text}"),
        }
    }

    #[test]
    fn the_platform_tables_name_the_right_tools() {
        let path = PathBuf::from("/tmp/shot.png");

        // macOS: osascript writes the PNG itself, pbpaste is the text fallback.
        let macos = capture_commands_for("macOS", &path).expect("macos");
        assert_eq!(macos.image.len(), 1);
        assert_eq!(macos.image[0].command.program, "osascript");
        assert_eq!(macos.image[0].payload, Payload::File);
        assert_eq!(osascript_target(&macos.image[0].command.args), path);
        let script = macos.image[0].command.args.join("\n");
        assert!(script.contains("the clipboard as «class PNGf»"), "{script}");
        assert!(script.contains("write png to target"), "{script}");
        assert!(script.contains("close access target"), "{script}");
        assert_eq!(
            macos.text,
            vec![CaptureCommand {
                program: "pbpaste".to_string(),
                args: Vec::new()
            }]
        );
        // …and the aliases behave like macOS.
        for os in ["mac", "darwin", "ios"] {
            assert_eq!(capture_commands_for(os, &path), Some(macos.clone()), "{os}");
        }

        // Windows: PowerShell saves the image, `Get-Clipboard -Raw` is the text.
        let windows = capture_commands_for("windows", &path).expect("windows");
        assert_eq!(windows.image[0].command.program, "powershell");
        assert_eq!(windows.image[0].payload, Payload::File);
        let args = &windows.image[0].command.args;
        assert!(args.contains(&"-NoProfile".to_string()), "{args:?}");
        // `Clipboard::GetImage` throws without a single-threaded apartment.
        assert!(args.contains(&"-STA".to_string()), "{args:?}");
        assert_eq!(powershell_target(args), path);
        let command = args.last().expect("a script");
        assert!(command.contains("[System.Windows.Forms.Clipboard]::GetImage()"));
        assert!(command.contains("[System.Drawing.Imaging.ImageFormat]::Png"));
        assert_eq!(windows.text[0].program, "powershell");
        assert_eq!(windows.text[0].args.last().unwrap(), "Get-Clipboard -Raw");
        assert_eq!(
            capture_commands_for("Windows", &path),
            Some(windows.clone())
        );

        // Linux: both display servers are tried, and the image can only be
        // printed, so it comes back base64-encoded.
        let linux = capture_commands_for("linux", &path).expect("linux");
        assert_eq!(linux.image.len(), 2);
        assert_eq!(linux.image[0].payload, Payload::Base64OnStdout);
        assert_eq!(linux.image[0].command.program, "sh");
        let probe = linux.image[0].command.args.join(" ");
        assert!(
            probe.contains("wl-paste --type image/png | base64"),
            "{probe}"
        );
        assert_eq!(linux.image[0].command.args[2], "sh");
        let x11 = linux.image[1].command.args.join(" ");
        assert!(
            x11.contains("xclip -selection clipboard -t image/png -o | base64"),
            "{x11}"
        );
        assert_eq!(linux.text[0].program, "wl-paste");
        assert_eq!(linux.text[1].program, "xclip");
        assert_eq!(linux.text[1].args, ["-selection", "clipboard", "-o"]);

        // Any other platform has no clipboard tools at all.
        for os in ["freebsd", "plan9", ""] {
            assert_eq!(capture_commands_for(os, &path), None, "{os}");
        }
    }

    #[test]
    fn script_literals_escape_what_the_platform_escapes() {
        // AppleScript: a backslash and a quote inside the path stay inside it.
        assert_eq!(applescript_literal("plain"), "\"plain\"");
        assert_eq!(applescript_literal("a\"b"), "\"a\\\"b\"");
        assert_eq!(applescript_literal("C:\\tmp"), "\"C:\\\\tmp\"");
        let quoted = osascript_args(Path::new("/tmp/a\"b.png"));
        assert!(quoted.join("\n").contains("/tmp/a\\\"b.png"), "{quoted:?}");

        // PowerShell single quotes are escaped by doubling.
        assert_eq!(powershell_literal("plain"), "'plain'");
        assert_eq!(powershell_literal("it's"), "'it''s'");
    }

    // ─── clipboard capture: reading it ─────────────────────────────────

    #[test]
    fn a_png_on_the_clipboard_becomes_an_attachment_path() {
        let (capture, calls, _dir) = capture_with(|_program, args| {
            std::fs::write(osascript_target(args), png_bytes()).expect("write the capture");
            Ok((0, String::new(), String::new()))
        });
        let outcome = capture.capture("macos", 4321, 1_700_000_000_000);
        let parts = image_parts(&outcome);
        assert!(parts.is_some(), "expected an image, got {outcome:?}");
        let (path, name) = parts.unwrap_or_default();
        assert!(path.ends_with(".png"), "{path}");
        assert_eq!(
            name,
            Path::new(&path)
                .file_name()
                .and_then(|file| file.to_str())
                .expect("the captured path has a file name")
        );
        assert!(name.starts_with("clipboard-1700000000000-4321-0"), "{name}");
        assert!(is_image_file(Path::new(&path)), "{path} must be the image");
        assert!(
            unavailable_reason(&outcome).starts_with("an image at"),
            "{outcome:?}"
        );
        // The text fallback was never asked: the image answered the question.
        assert_eq!(capture_calls(&calls).len(), 1);
        assert_eq!(capture_calls(&calls)[0].0, "osascript");
    }

    #[test]
    fn a_jpeg_behind_a_png_name_is_renamed_to_its_real_format() {
        let (capture, _calls, _dir) = capture_with(|_program, args| {
            std::fs::write(
                osascript_target(args),
                b"\xff\xd8\xff\xe0rest of the jpeg file",
            )
            .expect("write the capture");
            Ok((0, String::new(), String::new()))
        });
        let outcome = capture.capture("macos", 1, 10);
        let parts = image_parts(&outcome);
        assert!(parts.is_some(), "expected an image, got {outcome:?}");
        let (path, name) = parts.unwrap_or_default();
        assert!(path.ends_with(".jpg"), "{path}");
        assert!(name.ends_with(".jpg"), "{name}");
        assert_eq!(
            sniff_image_format(&std::fs::read(&path).expect("read")),
            Some(ImageFormat::Jpeg)
        );
    }

    #[test]
    fn a_base64_probe_writes_the_decoded_bytes_and_ignores_the_wrapping() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(png_bytes());
        // Encoders wrap at 76 columns; the decoder has to cope with that.
        let wrapped = encoded
            .as_bytes()
            .chunks(40)
            .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        let (capture, calls, _dir) =
            capture_with(move |_program, _args| Ok((0, format!("{wrapped}\n"), String::new())));
        let outcome = capture.capture("linux", 7, 20);
        let parts = image_parts(&outcome);
        assert!(parts.is_some(), "expected an image, got {outcome:?}");
        let (path, _name) = parts.unwrap_or_default();
        assert_eq!(std::fs::read(&path).expect("read"), png_bytes());
        // Only the first Linux probe ran: it had the image.
        assert_eq!(capture_calls(&calls).len(), 1);
        assert!(capture_calls(&calls)[0].1.join(" ").contains("wl-paste"));
    }

    #[test]
    fn a_base64_probe_that_prints_junk_is_not_an_image() {
        // A tool that answers with something that is not base64 at all: no
        // image (and no error either) — the text fallback gets its turn.
        let (capture, _calls, _dir) = capture_with(|program, _args| {
            if program == "sh" {
                Ok((0, "clipboard text, not base64!!".to_string(), String::new()))
            } else {
                Ok((0, "the text answer".to_string(), String::new()))
            }
        });
        let outcome = capture.capture("linux", 1, 21);
        assert!(image_parts(&outcome).is_none(), "{outcome:?}");
        assert_eq!(outcome, ClipboardPaste::Text("the text answer".to_string()));

        // …and the same for a probe whose stdout is empty.
        let (capture, _calls, _dir) = capture_with(|program, _args| {
            if program == "sh" {
                Ok((0, "\n".to_string(), String::new()))
            } else {
                Ok((0, "text again".to_string(), String::new()))
            }
        });
        let outcome = capture.capture("linux", 1, 22);
        assert!(image_parts(&outcome).is_none(), "{outcome:?}");
        assert_eq!(outcome, ClipboardPaste::Text("text again".to_string()));
    }

    #[test]
    fn a_capture_file_too_short_to_sniff_is_not_an_image() {
        // The probe wrote *something*, but not enough bytes for a header: the
        // sniffer never gets to say yes, so the paste stays text.
        let (capture, _calls, _dir) = capture_with(|program, args| {
            if program == "osascript" {
                std::fs::write(osascript_target(args), b"\x89PNG").expect("write");
                Ok((0, String::new(), String::new()))
            } else {
                Ok((0, "text instead".to_string(), String::new()))
            }
        });
        let outcome = capture.capture("macos", 1, 23);
        assert!(image_parts(&outcome).is_none(), "{outcome:?}");
        assert_eq!(outcome, ClipboardPaste::Text("text instead".to_string()));
    }

    #[test]
    fn a_text_tool_that_exits_non_zero_gives_the_next_one_its_turn() {
        // The X11 fallbacks exit non-zero when they have nothing: that is the
        // next candidate's cue, not the end of the read.
        let (capture, calls, _dir) = capture_with(|program, _args| match program {
            "sh" => Ok((0, String::new(), String::new())),
            "wl-paste" => Ok((1, String::new(), "no selection".to_string())),
            _ => Ok((0, "xclip had it".to_string(), String::new())),
        });
        let outcome = capture.capture("linux", 1, 41);
        assert!(image_parts(&outcome).is_none(), "{outcome:?}");
        assert_eq!(outcome, ClipboardPaste::Text("xclip had it".to_string()));
        let programs: Vec<String> = capture_calls(&calls)
            .into_iter()
            .map(|(program, _)| program)
            .collect();
        assert_eq!(programs, vec!["sh", "sh", "wl-paste", "xclip"]);
    }

    #[test]
    fn the_default_capture_uses_the_system_temp_directory() {
        // `new()` is a constructor only — it spawns nothing — so both of these
        // are safe to build in a test.
        let default = ClipboardCapture::default();
        assert_eq!(default.dir(), ClipboardCapture::new().dir());
        let system_dir = default.dir().to_path_buf();
        assert!(system_dir.ends_with(CLIPBOARD_DIR_NAME), "{system_dir:?}");
    }

    #[test]
    fn a_probe_that_answers_without_an_image_falls_back_to_the_text() {
        // The tool exited 0 but wrote text: not an image, and the file it left
        // behind is cleaned up rather than attached.
        let (capture, calls, _dir) = capture_with(|program, args| {
            if program == "osascript" {
                std::fs::write(osascript_target(args), b"this is text, not a png\n")
                    .expect("write");
                Ok((0, String::new(), String::new()))
            } else {
                Ok((0, "clipboard text\n".to_string(), String::new()))
            }
        });
        let outcome = capture.capture("macos", 1, 30);
        assert!(image_parts(&outcome).is_none(), "{outcome:?}");
        assert_eq!(
            outcome,
            ClipboardPaste::Text("clipboard text\n".to_string())
        );
        // …and the other direction of the same helper: a text outcome is not a
        // failure reason either.
        assert!(
            unavailable_reason(&outcome).starts_with("text: "),
            "{outcome:?}"
        );
        let recorded = capture_calls(&calls);
        assert_eq!(recorded.len(), 2, "the text tool was asked: {recorded:?}");
        assert_eq!(recorded[1].0, "pbpaste");
        let left: Vec<_> = std::fs::read_dir(capture.dir())
            .expect("the directory exists")
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        assert!(left.is_empty(), "no half-captured file is left: {left:?}");
    }

    #[test]
    fn a_clipboard_with_no_image_of_the_requested_flavour_is_text() {
        // osascript raises when the clipboard holds no PNG — exit code 1, which
        // is the ordinary case and must not be reported as a failure.
        let (capture, calls, _dir) = capture_with(|program, _args| {
            if program == "osascript" {
                Ok((
                    1,
                    String::new(),
                    "execution error: the clipboard doesn’t contain a PNG image.".to_string(),
                ))
            } else {
                Ok((0, "/tmp/shot.png".to_string(), String::new()))
            }
        });
        let outcome = capture.capture("macos", 1, 40);
        assert!(image_parts(&outcome).is_none(), "{outcome:?}");
        assert_eq!(outcome, ClipboardPaste::Text("/tmp/shot.png".to_string()));
        assert_eq!(capture_calls(&calls).len(), 2);
    }

    #[test]
    fn a_clipboard_with_neither_image_nor_text_says_so() {
        let (capture, _calls, _dir) = capture_with(|_program, _args| {
            // Both tools ran and had nothing.
            Ok((0, String::new(), String::new()))
        });
        let outcome = capture.capture("macos", 1, 50);
        assert_eq!(
            unavailable_reason(&outcome),
            "the clipboard holds neither an image nor text"
        );
    }

    #[test]
    fn a_clipboard_tool_that_cannot_be_started_names_itself() {
        let (capture, _calls, _dir) = capture_with(|program, _args| {
            Err(format!(
                "failed to spawn {program}: No such file or directory"
            ))
        });
        let reason = unavailable_reason(&capture.capture("macos", 1, 60));
        // The message names the tool the user has to install, and the call did
        // not panic on the way.
        assert!(reason.contains("failed to spawn pbpaste"), "{reason}");
    }

    #[test]
    fn the_text_tools_are_tried_in_order_until_one_answers() {
        let calls_seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&calls_seen);
        let capture = ClipboardCapture::with_runner(
            Box::new(move |program: &str, _args: &[String]| {
                sink.lock().unwrap().push(program.to_string());
                match program {
                    "wl-paste" => Err("wl-paste: not found".to_string()),
                    "xclip" => Ok((0, "from xclip".to_string(), String::new())),
                    _ => Ok((1, String::new(), String::new())),
                }
            }),
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .join("clipboard"),
        );
        assert_eq!(
            capture.capture("linux", 1, 70),
            ClipboardPaste::Text("from xclip".to_string())
        );
        let seen = calls_seen.lock().unwrap().clone();
        assert_eq!(
            seen,
            vec![
                "sh".to_string(),
                "sh".to_string(),
                "wl-paste".to_string(),
                "xclip".to_string()
            ],
            "both image probes, then the text tools in order"
        );
    }

    #[test]
    fn a_platform_with_no_clipboard_tool_says_which_platform() {
        let (capture, calls, _dir) =
            capture_with(|_program, _args| Ok((0, String::new(), String::new())));
        let reason = unavailable_reason(&capture.capture("plan9", 1, 80));
        assert!(reason.contains("plan9"), "{reason}");
        // The command table is what refused: no tool was even asked.
        assert!(capture_calls(&calls).is_empty());
    }

    #[test]
    fn an_unwritable_capture_directory_is_reported_not_fatal() {
        // The directory cannot be created because a *file* is in the way: the
        // image probe fails, and the text fallback still answers.
        let dir = tempfile::tempdir().expect("tempdir");
        let blocked = dir.path().join("clipboard");
        std::fs::write(&blocked, b"not a directory").expect("write");
        let capture = ClipboardCapture::with_runner(
            // The image probe never reaches a runner here (the directory cannot
            // be made), so the only command that matters is the text fallback.
            Box::new(|_program: &str, _args: &[String]| {
                Ok((0, "text anyway".to_string(), String::new()))
            }),
            blocked,
        );
        assert_eq!(
            capture.capture("macos", 1, 90),
            ClipboardPaste::Text("text anyway".to_string())
        );
    }

    #[test]
    fn two_captures_never_share_a_path() {
        let (capture, calls, _dir) = capture_with(|_program, args| {
            let target = osascript_target(args);
            // Both captures report the same instant; the paths must still differ.
            std::fs::write(&target, png_bytes()).expect("write");
            Ok((0, String::new(), String::new()))
        });
        // Same millisecond, same process id: the counter is what separates them
        // (two pastes cannot overwrite each other's image).
        let first = capture.capture("macos", 99, 1_700_000_000_000);
        let second = capture.capture("macos", 99, 1_700_000_000_000);
        let paths: Vec<String> = capture_calls(&calls)
            .iter()
            .map(|(_, args)| osascript_target(args).to_string_lossy().into_owned())
            .collect();
        assert_eq!(paths.len(), 2);
        assert_ne!(paths[0], paths[1], "{paths:?}");
        assert_ne!(first, second, "the outcomes name different files");
        for path in paths {
            assert!(
                Path::new(&path).is_file(),
                "{path} must exist for the agent"
            );
        }
    }

    // ─── clipboard capture: the temp files ─────────────────────────────

    #[test]
    fn stale_captures_are_swept_and_recent_ones_are_kept() {
        let dir = tempfile::tempdir().expect("tempdir");
        let now = 1_700_000_000_000;
        let fresh = clipboard_file_name(now - 1000, 1, 0);
        let just_under = clipboard_file_name(now - CLIPBOARD_MAX_AGE_MS + 1, 1, 1);
        let at_the_edge = clipboard_file_name(now - CLIPBOARD_MAX_AGE_MS, 1, 2);
        let stale = clipboard_file_name(now - CLIPBOARD_MAX_AGE_MS - 1, 1, 3);
        for name in [&fresh, &just_under, &at_the_edge, &stale] {
            std::fs::write(dir.path().join(name), png_bytes()).expect("write");
        }
        // A file that is not one of ours is never touched, whatever its age.
        std::fs::write(dir.path().join("notes.txt"), b"keep me").expect("write");

        assert_eq!(sweep_stale_clipboard_files(dir.path(), now), 2);
        assert!(dir.path().join(&fresh).is_file());
        assert!(dir.path().join(&just_under).is_file());
        assert!(!dir.path().join(&at_the_edge).exists());
        assert!(!dir.path().join(&stale).exists());
        assert!(dir.path().join("notes.txt").is_file());

        // A directory that does not exist sweeps nothing (nothing captured yet).
        assert_eq!(
            sweep_stale_clipboard_files(&dir.path().join("nope"), now),
            0
        );
    }

    #[test]
    fn a_capture_sweeps_the_directory_it_writes_into() {
        let (capture, _calls, _dir) = capture_with(|_program, args| {
            std::fs::write(osascript_target(args), png_bytes()).expect("write");
            Ok((0, String::new(), String::new()))
        });
        let now = 1_700_000_000_000;
        std::fs::create_dir_all(capture.dir()).expect("mkdir");
        let ancient = clipboard_file_name(now - CLIPBOARD_MAX_AGE_MS - 1, 1, 0);
        std::fs::write(capture.dir().join(&ancient), png_bytes()).expect("write");
        assert!(matches!(
            capture.capture("macos", 1, now),
            ClipboardPaste::Image { .. }
        ));
        assert!(!capture.dir().join(ancient).exists());
    }

    #[test]
    fn clipboard_age_reads_the_stamp_in_the_name() {
        assert_eq!(clipboard_age_ms("clipboard-100-1-0.png", 250), Some(150));
        assert_eq!(clipboard_age_ms("clipboard-250-1-0.jpg", 250), Some(0));
        // Not ours, or not a time at all.
        assert_eq!(clipboard_age_ms("notes.txt", 250), None);
        assert_eq!(clipboard_age_ms("clipboard-.png", 250), None);
        assert_eq!(clipboard_age_ms("clipboard-abc-1-0.png", 250), None);
        assert_eq!(clipboard_age_ms("clipboard", 250), None);
        // A clock that went backwards (or a file from a faster machine) is not
        // "infinitely old": it is left alone.
        assert_eq!(clipboard_age_ms("clipboard-300-1-0.png", 250), None);
    }

    #[test]
    fn the_capture_file_name_carries_time_pid_and_sequence() {
        assert_eq!(
            clipboard_file_name(1700000000000, 42, 7),
            "clipboard-1700000000000-42-7.png"
        );
        assert_ne!(
            clipboard_file_name(1, 1, 1),
            clipboard_file_name(1, 1, 2),
            "two captures in one millisecond still differ"
        );
    }

    #[test]
    fn image_extensions_follow_the_sniffed_format() {
        assert_eq!(image_extension(ImageFormat::Png), "png");
        assert_eq!(image_extension(ImageFormat::Jpeg), "jpg");
        assert_eq!(image_extension(ImageFormat::Gif), "gif");
        assert_eq!(image_extension(ImageFormat::Webp), "webp");
        assert_eq!(image_extension(ImageFormat::Bmp), "bmp");
    }

    #[test]
    fn a_rename_that_cannot_happen_keeps_the_capture_readable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("clipboard-1-1-0.png");
        std::fs::write(&path, png_bytes()).expect("write");
        // A directory in the target's place: the rename fails, and the file has
        // to stay where the agent can read it.
        std::fs::create_dir(dir.path().join("clipboard-1-1-0.jpg")).expect("mkdir");
        std::fs::write(
            dir.path().join("clipboard-1-1-0.jpg").join("occupied"),
            b"x",
        )
        .expect("write");
        assert_eq!(rename_to_format(&path, ImageFormat::Jpeg), path);
        assert!(is_image_file(&path));
    }

    #[test]
    fn base64_decoding_tolerates_wrapping_and_rejects_junk() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(png_bytes());
        assert_eq!(decode_base64(&encoded), Some(png_bytes()));
        assert_eq!(decode_base64(&format!("{encoded}\n")), Some(png_bytes()));
        assert_eq!(
            decode_base64(&format!("{}\n  {}", &encoded[..8], &encoded[8..])),
            Some(png_bytes())
        );
        assert_eq!(decode_base64(""), Some(Vec::new()));
        assert_eq!(decode_base64("not base64!!"), None);
        assert_eq!(decode_base64("text from a clipboard\n"), None);
    }

    #[test]
    fn reading_a_missing_or_short_file_reports_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(read_head(&dir.path().join("nope.png")), None);
        assert!(!is_non_empty_file(&dir.path().join("nope.png")));
        let empty = write(&dir, "empty.png", b"");
        assert!(!is_non_empty_file(&empty));
        assert_eq!(read_head(&empty), None);
        // A short file is not readable *as a header*, even though it exists.
        let short = write(&dir, "short.png", b"\x89PNG");
        assert_eq!(read_head(&short), None);
        assert!(is_non_empty_file(&short));
        assert!(!is_non_empty_file(dir.path()));
    }

    // ─── clipboard capture: the real spawner ───────────────────────────

    #[test]
    fn the_timeout_marker_is_reworded_for_the_clipboard() {
        let error =
            humanize_capture_error(format!("{} after 10s", crate::skills_cli::TIMEOUT_MARKER));
        assert_eq!(error, "the clipboard tool timed out after 10s");
        assert!(!error.contains("skills"), "{error}");
        // Anything that is not the marker is passed through untouched.
        assert_eq!(
            humanize_capture_error("failed to spawn pbpaste: ENOENT".to_string()),
            "failed to spawn pbpaste: ENOENT"
        );
    }

    #[test]
    #[cfg(unix)]
    fn the_real_spawner_captures_a_tool_that_prints_text() {
        // A real child, but a harmless one: `cat` is not a clipboard tool.
        let (code, stdout, stderr) = spawn_capture("cat", &[]).expect("cat runs");
        assert_eq!((code, stdout.as_str(), stderr.as_str()), (0, "", ""));
        let (code, stdout, _) =
            spawn_capture("sh", &["-c".to_string(), "printf 'clip text'".to_string()])
                .expect("sh runs");
        assert_eq!((code, stdout.as_str()), (0, "clip text"));
    }

    #[test]
    #[cfg(unix)]
    fn the_real_spawner_reports_a_tool_that_is_not_installed() {
        let error =
            spawn_capture("future-tui-no-such-clipboard-tool", &[]).expect_err("spawn must fail");
        assert!(error.contains("failed to spawn"), "{error}");
    }

    /// The Windows half of the pair above. `spawn_capture` is the **real**
    /// capture path — every other test injects a capture function — so without
    /// this the production path would only ever run on POSIX. It must report
    /// the exit status together with both captured streams.
    #[test]
    #[cfg(windows)]
    fn the_real_spawner_runs_a_program_and_reports_a_missing_one() {
        let args: Vec<String> = vec!["/c".into(), "exit 0".into()];
        let (code, stdout, stderr) = spawn_capture("cmd", &args).expect("cmd /c exit 0 runs");
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
        assert_eq!(stderr, "");

        // Output on both streams comes back separated, so a caller can tell a
        // tool's answer from its diagnostics.
        let args: Vec<String> = vec!["/c".into(), "echo out & echo err 1>&2".into()];
        let (code, stdout, stderr) = spawn_capture("cmd", &args).expect("cmd runs");
        assert_eq!(code, 0);
        assert!(stdout.contains("out"), "{stdout:?}");
        assert!(stderr.contains("err"), "{stderr:?}");

        let error =
            spawn_capture("future-tui-no-such-clipboard-tool", &[]).expect_err("spawn must fail");
        assert!(
            error.contains("no-such-clipboard-tool") || error.contains("failed to spawn"),
            "the failure must name the tool or the spawn: {error}"
        );
    }
}
