//! What in the conversation can be opened: `tab` steps through these spans,
//! `enter` and a click open the one under the cursor.
//!
//! A transcript row is a finished string of styled text (see
//! [`crate::components::chat_area`]), so the only honest source of "this word is
//! a link" is what the renderer actually emitted: the OSC 8 hyperlink wrappers
//! around every markdown link. Reading them back is what keeps the surface from
//! growing a second markdown parser that could disagree with the first — the
//! same reason [`crate::workbench::ansi`] measures rows instead of trusting
//! their author.
//!
//! Two kinds of target are found:
//!
//! * **Links** — an OSC 8 span whose destination is a URL, another
//!   conversation (`futureos://session/<id>`) or a local file.
//! * **Pictures named in prose** — an agent writes "saved to `/tmp/shot.png`",
//!   which is not a link at all, so a bare token is accepted when it resolves to
//!   an image file that exists. Only pictures: a transcript is full of paths a
//!   reader does not want to step through on their way to the one link they
//!   meant, and every candidate costs a `stat`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use unicode_segmentation::UnicodeSegmentation;

use crate::utils::{extract_ansi_code, visible_width, AnsiKind};

/// What opening a span does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// Hand it to the reader's browser (or mail client).
    Url(String),
    /// Open another conversation: the id from a `futureos://session/<id>` link.
    Session(String),
    /// Show a local file in the preview box — a picture as a picture.
    File(String),
}

/// One openable stretch of one transcript row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpan {
    /// Index into `WorkbenchData::messages` — the row in the *transcript*, not
    /// on screen.
    pub row: usize,
    /// First display column of the span, and one past its last.
    pub start: usize,
    pub end: usize,
    /// The destination exactly as it was written, for the notice and the box.
    pub href: String,
    pub target: LinkTarget,
}

impl LinkSpan {
    /// Does this span cover `column` of its row?
    pub fn covers(&self, column: usize) -> bool {
        self.start <= column && column < self.end
    }
}

/// A row's targets, found once per (revision, width, cwd).
///
/// Re-scanning per frame would re-`stat` every candidate path on every repaint,
/// and a streaming reply repaints per token; the scan is cheap but the
/// filesystem is not, so the last answer is kept until something it was derived
/// from changes.
#[derive(Default)]
pub struct Cache {
    key: Option<Key>,
    spans: Vec<LinkSpan>,
    /// Path existence, kept across rescans. A transcript that grows by one
    /// message must not ask the filesystem about the other thousand rows again;
    /// clearing at a bound keeps a long file-hopping session from growing it
    /// without end.
    exists: HashMap<String, bool>,
}

#[derive(PartialEq, Eq)]
struct Key {
    revision: u64,
    rows: usize,
    /// The width the rows were wrapped at: a narrower terminal rewraps every
    /// row, which moves every span even when the content did not change.
    width: usize,
    cwd: String,
}

impl Cache {
    /// The spans for `rows`, rescanned when any input changed.
    pub fn spans(
        &mut self,
        rows: &[String],
        cwd: &str,
        revision: u64,
        width: usize,
    ) -> &[LinkSpan] {
        let key = Key {
            revision,
            rows: rows.len(),
            width,
            cwd: cwd.to_string(),
        };
        if self.key.as_ref() != Some(&key) {
            if self.exists.len() > 512 {
                self.exists.clear();
            }
            self.spans = scan(rows, cwd, &mut self.exists);
            self.key = Some(key);
        }
        &self.spans
    }

    /// Forget everything: the conversation changed under the spans.
    pub fn clear(&mut self) {
        self.key = None;
        self.spans.clear();
    }
}

/// Every openable span in `rows`, in reading order.
pub fn scan(rows: &[String], cwd: &str, exists: &mut HashMap<String, bool>) -> Vec<LinkSpan> {
    let cwd = Path::new(cwd);
    let mut spans = Vec::new();
    for (row, text) in rows.iter().enumerate() {
        scan_row(row, text, cwd, exists, &mut spans);
    }
    spans
}

/// One row: the hyperlinks the renderer wrote, then the pictures named in prose
/// that no link wraps.
fn scan_row(
    row: usize,
    text: &str,
    cwd: &Path,
    exists: &mut HashMap<String, bool>,
    spans: &mut Vec<LinkSpan>,
) {
    let mut column = 0usize;
    let mut index = 0usize;
    // The hyperlink the walker is inside, if any: where it started and where it
    // goes.
    let mut open: Option<(usize, String)> = None;
    // The visible glyphs, so a bare path can be measured in columns after the
    // escapes are gone: (grapheme, first byte in `plain`, column, width).
    let mut plain = String::new();
    let mut glyphs: Vec<(usize, usize, usize)> = Vec::new();
    let first = spans.len();
    while index < text.len() {
        if let Some(code) = extract_ansi_code(text, index) {
            index += code.length;
            // A terminal image is a bitmap placed out of band, not text: nothing
            // on that row can be selected, and its payload would parse as
            // garbage.
            if code.kind == AnsiKind::Apc {
                return;
            }
            if code.kind == AnsiKind::Osc {
                if let Some(uri) = osc8_uri(&code.code) {
                    if let Some((start, href)) = open.take() {
                        push_span(row, start, column, href, cwd, exists, spans);
                    }
                    if !uri.is_empty() {
                        open = Some((column, uri));
                    }
                }
            }
            continue;
        }
        let Some(grapheme) = text[index..].graphemes(true).next() else {
            break;
        };
        let width = visible_width(grapheme);
        glyphs.push((plain.len(), column, width));
        plain.push_str(grapheme);
        column += width;
        index += grapheme.len();
    }
    // A renderer that forgets the closing sequence still named a link; the row
    // ends where the text does.
    if let Some((start, href)) = open.take() {
        push_span(row, start, column, href, cwd, exists, spans);
    }
    let linked: Vec<(usize, usize)> = spans[first..]
        .iter()
        .map(|span| (span.start, span.end))
        .collect();
    bare_pictures(row, &plain, &glyphs, &linked, cwd, exists, spans);
}

/// Append the span `[start, end)` of `row` if its destination can be opened.
fn push_span(
    row: usize,
    start: usize,
    end: usize,
    href: String,
    cwd: &Path,
    exists: &mut HashMap<String, bool>,
    spans: &mut Vec<LinkSpan>,
) {
    if end <= start {
        return;
    }
    let Some(target) = classify(&href, cwd, exists) else {
        return;
    };
    spans.push(LinkSpan {
        row,
        start,
        end,
        href,
        target,
    });
}

/// What `href` opens, or `None` when there is nothing to open — a dead relative
/// path must not swallow a `tab` press.
fn classify(href: &str, cwd: &Path, exists: &mut HashMap<String, bool>) -> Option<LinkTarget> {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') {
        return None;
    }
    if let Some(rest) =
        strip_prefix_ci(href, "http://").or_else(|| strip_prefix_ci(href, "https://"))
    {
        return (!rest.is_empty()).then(|| LinkTarget::Url(href.to_string()));
    }
    if strip_prefix_ci(href, "mailto:").is_some() {
        return Some(LinkTarget::Url(href.to_string()));
    }
    if let Some(id) = strip_prefix_ci(href, "futureos://session/") {
        let id = id.trim_end_matches('/');
        return (!id.is_empty()).then(|| LinkTarget::Session(id.to_string()));
    }
    let path = local_path(href, cwd)?;
    file_exists(&path, exists).then_some(LinkTarget::File(path))
}

/// The local file `href` names, as an absolute path — `file:` URLs, `~` and
/// paths relative to the conversation's working directory.
///
/// Existence is *not* checked here; the caller does, because it caches the
/// answer.
pub fn local_path(href: &str, cwd: &Path) -> Option<String> {
    let href = href.trim();
    let href = strip_prefix_ci(href, "file:").unwrap_or(href);
    // `file:///tmp/a.png` and `file://localhost/tmp/a.png` both name a local
    // path; the authority is either empty or this machine.
    let href = strip_prefix_ci(href, "//localhost").unwrap_or(href);
    let href = href.trim_start_matches("//");
    // A fragment is a position inside the file, not part of its name.
    let href = href.split('#').next().unwrap_or(href);
    if href.is_empty() {
        return None;
    }
    let href = percent_decode(href);
    let path = if let Some(rest) = href.strip_prefix("~/").or(href.strip_prefix('~')) {
        home_dir()?.join(rest)
    } else {
        PathBuf::from(&href)
    };
    let path = if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    };
    Some(path.to_string_lossy().into_owned())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn file_exists(path: &str, exists: &mut HashMap<String, bool>) -> bool {
    if let Some(known) = exists.get(path) {
        return *known;
    }
    let found = std::fs::metadata(path)
        .map(|meta| meta.is_file())
        .unwrap_or(false);
    exists.insert(path.to_string(), found);
    found
}

/// `https://…` → `Some("https://…")`, a closing `\x1b]8;;\x1b\\` → `Some("")`.
/// Anything that is not an OSC 8 hyperlink → `None`.
fn osc8_uri(code: &str) -> Option<String> {
    let body = code.strip_prefix("\x1b]8;")?;
    let body = body
        .strip_suffix("\x1b\\")
        .or_else(|| body.strip_suffix('\x07'))?;
    // `params;URI`, where params carries the (unused) `id=`.
    let (_params, uri) = body.split_once(';')?;
    Some(uri.to_string())
}

fn strip_prefix_ci<'a>(haystack: &'a str, prefix: &str) -> Option<&'a str> {
    let head = haystack.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &haystack[prefix.len()..])
}

/// `%20` → ` `, for the forms a browser- or editor-copied path arrives in.
fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

/// Extensions whose files a terminal can actually paint. The extension only
/// decides whether a token is *worth a stat*: whether the bytes are a picture is
/// decided by sniffing them (`files::preview_kind`), because an extension lies.
const PICTURE_EXTENSIONS: [&str; 10] = [
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "tif", "tiff", "avif",
];

/// Pictures named in plain prose: `/tmp/shot.png`, `design.png`, `file://…`.
///
/// `linked` holds the column ranges an OSC 8 span already owns — a markdown link
/// is found once, by the link pass, and not twice.
#[allow(clippy::too_many_arguments)]
fn bare_pictures(
    row: usize,
    plain: &str,
    glyphs: &[(usize, usize, usize)],
    linked: &[(usize, usize)],
    cwd: &Path,
    exists: &mut HashMap<String, bool>,
    spans: &mut Vec<LinkSpan>,
) {
    if !plain.contains('.') {
        return;
    }
    let mut start = None;
    for (offset, byte) in plain.bytes().enumerate() {
        let space = byte.is_ascii_whitespace();
        match (space, start) {
            (false, None) => start = Some(offset),
            (true, Some(from)) => {
                token(
                    row,
                    &plain[from..offset],
                    from,
                    glyphs,
                    linked,
                    cwd,
                    exists,
                    spans,
                );
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        token(
            row,
            &plain[from..],
            from,
            glyphs,
            linked,
            cwd,
            exists,
            spans,
        );
    }
}

/// Markup and prose punctuation cling to a path without being part of it: in
/// "see `/tmp/a.png`," the comma is the sentence's and the backtick is the
/// renderer's.
const CLINGING: [char; 24] = [
    '`', '\'', '"', '(', ')', '[', ']', '{', '}', '<', '>', ',', '.', ';', ':', '!', '?', '、',
    '。', '，', '；', '：', '）', '】',
];

/// One whitespace-delimited token: is it the name of a picture that exists?
#[allow(clippy::too_many_arguments)]
fn token(
    row: usize,
    raw: &str,
    offset: usize,
    glyphs: &[(usize, usize, usize)],
    linked: &[(usize, usize)],
    cwd: &Path,
    exists: &mut HashMap<String, bool>,
    spans: &mut Vec<LinkSpan>,
) {
    let trimmed = raw.trim_matches(CLINGING);
    let from = offset + (raw.len() - raw.trim_start_matches(CLINGING).len());
    if trimmed.len() < 5 || trimmed.len() > 240 {
        return;
    }
    // An `@` or `=` means an address or a query, not a path; a token with no
    // slash and no dot is a word.
    if trimmed.contains('@') || trimmed.contains('=') {
        return;
    }
    let extension = trimmed
        .rsplit_once('.')
        .map(|(_, ext)| ext.trim_end_matches(['/', ')', ']']).to_ascii_lowercase())
        .unwrap_or_default();
    if !PICTURE_EXTENSIONS.contains(&extension.as_str()) {
        return;
    }
    let Some(path) = local_path(trimmed, cwd) else {
        return;
    };
    if !file_exists(&path, exists) {
        return;
    }
    // The token's own columns, from the glyph table: the trimmed edges are
    // ASCII, so a byte offset maps back through the plain text.
    let Some(start) = column_at(glyphs, from) else {
        return;
    };
    let end = from + trimmed.len();
    let Some(last) = glyphs
        .iter()
        .rev()
        .find(|(glyph_offset, _, _)| *glyph_offset < end)
    else {
        return;
    };
    let stop = last.1 + last.2;
    if linked
        .iter()
        .any(|(link_start, link_end)| *link_start < stop && start < *link_end)
    {
        return;
    }
    spans.push(LinkSpan {
        row,
        start,
        end: stop,
        href: trimmed.to_string(),
        target: LinkTarget::File(path),
    });
}

/// The column the glyph starting at byte `offset` occupies.
fn column_at(glyphs: &[(usize, usize, usize)], offset: usize) -> Option<usize> {
    match glyphs.binary_search_by_key(&offset, |(start, _, _)| *start) {
        Ok(index) => Some(glyphs[index].1),
        Err(_) => glyphs
            .iter()
            .find(|(start, _, _)| *start == offset)
            .map(|(_, column, _)| *column),
    }
}

/// Wrap the columns `[start, end)` of `row` in reverse video.
///
/// Reverse, not another colour, because it is the one attribute the selection
/// can switch on and off *inside* a styled run: every style here closes with a
/// RESET, so the flag is re-armed after each one and dropped with `27m` at the
/// end, which leaves the glyphs' own colours alone.
pub fn highlight(row: &str, start: usize, end: usize) -> String {
    if start >= end {
        return row.to_string();
    }
    let mut out = String::with_capacity(row.len() + 16);
    let mut column = 0usize;
    let mut index = 0usize;
    let mut marked = false;
    while index < row.len() {
        if let Some(code) = extract_ansi_code(row, index) {
            out.push_str(&code.code);
            index += code.length;
            // A style run that closed inside the selection took the reverse
            // video with it; put it back, or only the first run would be marked.
            if marked && is_reset(&code.code) {
                out.push_str(REVERSE);
            }
            continue;
        }
        let Some(grapheme) = row[index..].graphemes(true).next() else {
            break;
        };
        if !marked && column >= start && column < end {
            out.push_str(REVERSE);
            marked = true;
        }
        if marked && column >= end {
            out.push_str(NO_REVERSE);
            marked = false;
        }
        out.push_str(grapheme);
        column += visible_width(grapheme);
        index += grapheme.len();
    }
    if marked {
        out.push_str(NO_REVERSE);
    }
    out
}

const REVERSE: &str = "\x1b[7m";
const NO_REVERSE: &str = "\x1b[27m";

/// `\x1b[0m` / `\x1b[m` — the two spellings of "everything off".
fn is_reset(code: &str) -> bool {
    let Some(params) = code
        .strip_prefix("\x1b[")
        .and_then(|rest| rest.strip_suffix('m'))
    else {
        return false;
    };
    params.chars().all(|c| c == '0')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "future-links-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    fn link(href: &str, text: &str) -> String {
        format!("\x1b]8;;{href}\x1b\\{text}\x1b]8;;\x1b\\")
    }

    fn found(rows: &[String], cwd: &Path) -> Vec<LinkSpan> {
        scan(rows, &cwd.to_string_lossy(), &mut HashMap::new())
    }

    /// The scan reads what the renderer wrote, not a fixture: a markdown link
    /// and a markdown picture, straight out of `components::markdown`, land on
    /// the columns their labels occupy.
    #[test]
    fn the_renderers_own_rows_are_scanned_where_the_labels_are() {
        let cwd = temp_dir("renderer");
        std::fs::write(cwd.join("shot.png"), b"png").unwrap();
        let mut md = crate::components::markdown::MarkdownRenderer::new();
        let rows = md.render_text(
            "see [the site](https://example.com) and ![a shot](shot.png) now",
            100,
        );
        let spans = found(&rows, &cwd);
        assert_eq!(spans.len(), 2, "{spans:?}");
        let visible = crate::utils::strip_ansi_codes(&rows[0]);
        let site = visible.find("the site").expect("the label is painted");
        assert_eq!((spans[0].start, spans[0].end), (site, site + 8));
        assert_eq!(
            spans[0].target,
            LinkTarget::Url("https://example.com".into())
        );
        let shot = visible.find("a shot").expect("the caption is painted");
        assert_eq!((spans[1].start, spans[1].end), (shot, shot + 6));
        assert!(matches!(&spans[1].target, LinkTarget::File(p) if p.ends_with("shot.png")));
        fs::remove_dir_all(&cwd).ok();
    }

    #[test]
    fn a_hyperlinked_url_is_a_span_over_its_label() {
        let cwd = temp_dir("url");
        let rows = vec![format!(
            "see {} here",
            link("https://example.com", "the site")
        )];
        let spans = found(&rows, &cwd);
        assert_eq!(spans.len(), 1, "{spans:?}");
        assert_eq!(
            spans[0].target,
            LinkTarget::Url("https://example.com".into())
        );
        // "see " is four columns wide, so the label starts at column 4.
        assert_eq!((spans[0].start, spans[0].end), (4, 12));
        fs::remove_dir_all(&cwd).ok();
    }

    #[test]
    fn a_conversation_link_carries_its_id() {
        let cwd = temp_dir("session");
        let rows = vec![link("futureos://session/20261016-abc", "that chat")];
        let spans = found(&rows, &cwd);
        assert_eq!(spans[0].target, LinkTarget::Session("20261016-abc".into()));
        fs::remove_dir_all(&cwd).ok();
    }

    #[test]
    fn a_relative_link_opens_the_file_beside_the_conversation() {
        let cwd = temp_dir("relative");
        fs::create_dir_all(cwd.join("docs")).unwrap();
        fs::write(cwd.join("docs/shot.png"), b"not really a picture").unwrap();
        let rows = vec![link("docs/shot.png", "the shot")];
        let spans = found(&rows, &cwd);
        assert_eq!(spans.len(), 1);
        match &spans[0].target {
            LinkTarget::File(path) => assert!(path.ends_with("docs/shot.png"), "{path}"),
            other => panic!("{other:?}"),
        }
        // A link to something that is not there is not a target at all: `tab`
        // must not stop on a dead one.
        let missing = vec![link("docs/gone.png", "gone")];
        assert!(found(&missing, &cwd).is_empty());
        fs::remove_dir_all(&cwd).ok();
    }

    #[test]
    fn a_picture_named_in_prose_is_a_target_and_a_dead_one_is_not() {
        let cwd = temp_dir("prose");
        let shot = cwd.join("screenshot.png");
        fs::write(&shot, b"png").unwrap();
        let rows = vec![format!("saved to `{}`, and then some", shot.display())];
        let spans = found(&rows, &cwd);
        assert_eq!(spans.len(), 1, "{spans:?}");
        assert_eq!(
            spans[0].target,
            LinkTarget::File(shot.to_string_lossy().into())
        );
        // The backticks are the renderer's markup, not part of the name.
        assert!(spans[0].href.ends_with("screenshot.png"));
        assert!(!spans[0].href.contains('`'));

        let nothing = vec!["saved to /tmp/definitely-not-here.png".to_string()];
        assert!(found(&nothing, &cwd).is_empty());
        fs::remove_dir_all(&cwd).ok();
    }

    #[test]
    fn a_picture_inside_a_link_is_found_once() {
        let cwd = temp_dir("once");
        let shot = cwd.join("a.png");
        fs::write(&shot, b"png").unwrap();
        let rows = vec![link(&shot.to_string_lossy(), &shot.to_string_lossy())];
        let spans = found(&rows, &cwd);
        assert_eq!(spans.len(), 1, "{spans:?}");
        fs::remove_dir_all(&cwd).ok();
    }

    #[test]
    fn text_that_is_not_a_file_is_not_a_target() {
        let cwd = temp_dir("noise");
        let rows = vec![
            "the model is future/deepseek-flash and v1.2.3 shipped".to_string(),
            "a table | a.png | row |".to_string(),
        ];
        assert!(found(&rows, &cwd).is_empty());
        fs::remove_dir_all(&cwd).ok();
    }

    #[test]
    fn a_file_url_and_an_escaped_path_resolve() {
        let cwd = temp_dir("fileurl");
        fs::write(cwd.join("a b.png"), b"png").unwrap();
        let rows = vec![
            link("file:///tmp/nowhere.png", "x"),
            link("a%20b.png", "the space one"),
        ];
        let spans = found(&rows, &cwd);
        // `/tmp/nowhere.png` does not exist; the escaped relative path does.
        assert_eq!(spans.len(), 1, "{spans:?}");
        assert!(matches!(&spans[0].target, LinkTarget::File(p) if p.ends_with("a b.png")));
        fs::remove_dir_all(&cwd).ok();
    }

    #[test]
    fn a_row_split_link_is_a_span_on_each_row() {
        // The wrapper re-opens the hyperlink on the continuation row, so what the
        // transcript shows is two spans to the same place — and `tab` stopping
        // twice is exactly what the reader sees.
        let cwd = temp_dir("split");
        let rows = vec![
            link("https://example.com/a", "first half "),
            link("https://example.com/a", "second half"),
        ];
        let spans = found(&rows, &cwd);
        assert_eq!(spans.len(), 2);
        assert_eq!((spans[0].row, spans[1].row), (0, 1));
        fs::remove_dir_all(&cwd).ok();
    }

    #[test]
    fn a_kitty_image_row_has_no_targets() {
        let cwd = temp_dir("kitty");
        let files = cwd.join("a.png");
        fs::write(&files, b"png").unwrap();
        let rows = vec![format!("\x1b_Ga=T,f=100;QUJD\x1b\\ {}", files.display())];
        assert!(found(&rows, &cwd).is_empty());
        fs::remove_dir_all(&cwd).ok();
    }

    #[test]
    fn the_cache_rescans_only_when_its_inputs_change() {
        let cwd = temp_dir("cache");
        let cwd = cwd.to_string_lossy().into_owned();
        let mut cache = Cache::default();
        let mut rows = vec![link("https://a.example", "one")];
        assert_eq!(cache.spans(&rows, &cwd, 1, 80).len(), 1);
        // The same revision, row count and width: the answer is the one from
        // last time, even though this text would scan differently.
        rows[0] = "no links at all".to_string();
        assert_eq!(cache.spans(&rows, &cwd, 1, 80).len(), 1);
        // A new revision rescans...
        assert_eq!(cache.spans(&rows, &cwd, 2, 80).len(), 0);
        // ...and so does a new width, because rewrapping moves every span.
        rows[0] = link("https://b.example", "two");
        assert_eq!(
            cache.spans(&rows, &cwd, 2, 80).len(),
            0,
            "the same key still answers from the cache"
        );
        assert_eq!(cache.spans(&rows, &cwd, 3, 80).len(), 1);
        assert_eq!(cache.spans(&rows, &cwd, 3, 40).len(), 1);
        fs::remove_dir_all(&cwd).ok();
    }

    #[test]
    fn highlight_reverses_exactly_the_span_and_survives_an_inner_reset() {
        let row = format!("\x1b[38;5;117m\x1b[4m{}\x1b[0m tail", "ab");
        let marked = highlight(&row, 0, 2);
        assert!(
            marked.starts_with("\x1b[38;5;117m\x1b[4m\x1b[7m"),
            "{marked:?}"
        );
        // The span's own reset drops reverse video; it is re-armed right after,
        // so the second glyph is still marked.
        assert!(marked.contains("\x1b[0m\x1b[7m"), "{marked:?}");
        // The glyph after the span is plain again.
        assert!(marked.ends_with("\x1b[27m tail"), "{marked:?}");
    }

    #[test]
    fn highlighting_an_empty_span_leaves_the_row_alone() {
        assert_eq!(highlight("abc", 2, 2), "abc");
    }
}
