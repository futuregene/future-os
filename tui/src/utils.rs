//! Text processing utilities for terminal rendering — 1:1 port of
//! `tui/src/utils.ts`: grapheme-aware width, ANSI code tracking, word
//! wrapping, truncation, and overlay compositing.
//!
//! `Intl.Segmenter("en", { granularity: "grapheme" })` is replaced by
//! `unicode-segmentation`'s extended grapheme clusters (both implement UAX #29;
//! the tested cases — CJK/emoji/combining marks/VS15/VS16 — agree). Caches are
//! thread-local, mirroring the single-threaded JS runtime while keeping
//! parallel unit tests isolated.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use unicode_segmentation::UnicodeSegmentation;

// ─── Grapheme Width ────────────────────────────────────────────────────────

fn is_regional_indicator(c: char) -> bool {
    (0x1F1E6..=0x1F1FF).contains(&(c as u32))
}

fn is_regional_indicator_pair(s: &str) -> bool {
    let mut chars = s.chars();
    let a = chars.next().map(is_regional_indicator).unwrap_or(false);
    let b = chars.next().map(is_regional_indicator).unwrap_or(false);
    a && b
}

fn is_keycap_sequence(s: &str) -> bool {
    if s.chars().count() < 2 {
        return false;
    }
    let code = s.chars().next().unwrap() as u32;
    // Keycap: digit/#/* + U+FE0F + U+20E3
    if code == 0x23 || code == 0x2A || (0x30..=0x39).contains(&code) {
        return s.contains('\u{20E3}');
    }
    false
}

fn is_emoji_flag(s: &str) -> bool {
    // Regional indicator pairs
    if is_regional_indicator_pair(s) {
        return true;
    }
    // Flag emoji tag sequences: U+1F3F4 + tags + U+E007F
    if s.chars().next().map(|c| c as u32) == Some(0x1F3F4) {
        return true;
    }
    false
}

/// Code points with Emoji_Presentation=Yes (emoji-data.txt) — these render as
/// emoji (2 cells) by DEFAULT, without needing VS16.
///
/// IMPORTANT: code points that are Emoji=Yes but Emoji_Presentation=No
/// (e.g. ▶ U+25B6, ©, ™, ↔, ⭐, ☀) render as TEXT (1 cell) by default in
/// terminals — they only become 2 cells when followed by VS16, which is
/// handled separately via `has_emoji_presentation()`. Including them here
/// would measure them 1 column too wide and break padding/alignment.
fn has_default_emoji_presentation(code: u32) -> bool {
    (0x1F300..=0x1F64F).contains(&code) || // misc symbols, emoticons
        (0x1F680..=0x1F6FF).contains(&code) || // transport
        (0x1F900..=0x1F9FF).contains(&code) || // supplemental
        (0x1FA00..=0x1FA6F).contains(&code) || // chess, etc
        (0x1FA70..=0x1FAFF).contains(&code) || // symbols ext-a
        (0x231A..=0x231B).contains(&code) || // watch, hourglass
        (0x23E9..=0x23EC).contains(&code) || // media controls ⏩⏪⏫⏬
        code == 0x23F0 || // alarm clock ⏰
        code == 0x23F3 || // hourglass ⏳
        (0x25AA..=0x25AB).contains(&code) || // squares
        (0x25FB..=0x25FE).contains(&code) || // squares
        (0x2B1B..=0x2B1C).contains(&code) || // squares
        (0x2614..=0x2615).contains(&code) || // umbrella, hot beverage
        (0x2648..=0x2653).contains(&code) || // zodiac
        code == 0x267F || // wheelchair
        code == 0x2693 || // anchor
        code == 0x26A1 || // high voltage
        (0x26AA..=0x26AB).contains(&code) || // white/black circle
        (0x26BD..=0x26BE).contains(&code) || // soccer, baseball
        (0x26C4..=0x26C5).contains(&code) || // snowman, sun behind cloud
        code == 0x26CE || code == 0x26D4 || // ophiuchus, no entry
        code == 0x26EA || // church
        (0x26F2..=0x26F3).contains(&code) || // fountain, golf
        code == 0x26F5 || code == 0x26FA || code == 0x26FD || // sailboat, tent, fuel
        code == 0x2705 || // white heavy check mark ✅
        (0x270A..=0x270B).contains(&code) || // raised fist, raised hand
        code == 0x2728 || // sparkles ✨
        code == 0x274C || code == 0x274E || // cross mark, negative cross ❌❎
        (0x2753..=0x2755).contains(&code) || // question/exclamation marks ❓❔❕
        code == 0x2757 || // heavy exclamation ❗
        (0x2795..=0x2797).contains(&code) || // heavy plus/minus/division
        code == 0x27B0 || code == 0x27BF // curly loops
}

/// Check for variation selector-16 (U+FE0F) making it emoji.
fn has_emoji_presentation(s: &str) -> bool {
    s.contains('\u{FE0F}')
}

fn is_cjk(code: u32) -> bool {
    (0x1100..=0x115F).contains(&code) || // Hangul Jamo
        (0x2329..=0x232A).contains(&code) || // angle brackets
        (0x2E80..=0x303E).contains(&code) || // CJK radicals
        (0x3040..=0x33BF).contains(&code) || // Hiragana, Katakana, Bopomofo, Hangul, CJK compat
        (0x3400..=0x4DBF).contains(&code) || // CJK ext A
        (0x4E00..=0xA4CF).contains(&code) || // CJK unified
        (0xA960..=0xA97C).contains(&code) || // Hangul extended
        (0xAC00..=0xD7A3).contains(&code) || // Hangul syllables
        (0xF900..=0xFAFF).contains(&code) || // CJK compat ideographs
        (0xFE10..=0xFE19).contains(&code) || // vertical forms
        (0xFE30..=0xFE6F).contains(&code) || // CJK compat forms
        (0xFF01..=0xFF60).contains(&code) || // fullwidth forms
        (0xFFE0..=0xFFE6).contains(&code) || // fullwidth signs
        (0x1B000..=0x1B2FF).contains(&code) || // Kana supplement/extended
        (0x1F200..=0x1F2FF).contains(&code) || // enclosed ideographic
        (0x20000..=0x2FFFF).contains(&code) || // CJK ext B+
        (0x30000..=0x3FFFF).contains(&code) // CJK ext G+
}

fn is_zero_width(code: u32) -> bool {
    code == 0x200B || // zero-width space
        code == 0x200C || // zero-width non-joiner
        code == 0x200D || // zero-width joiner
        code == 0xFEFF || // BOM / ZWNBSP
        code == 0x200E || // left-to-right mark
        code == 0x200F || // right-to-left mark
        code == 0x061C || // ALM
        code == 0x2060 || // word joiner
        code == 0x2061 || code == 0x2062 || code == 0x2063 || code == 0x2064 || // invisible ops
        (0x0300..=0x036F).contains(&code) || // combining diacritical marks
        (0x0483..=0x0489).contains(&code) || // combining cyrillic
        (0x0591..=0x05BD).contains(&code) || // combining Hebrew
        (0x0610..=0x061A).contains(&code) || // combining Arabic
        (0x064B..=0x065F).contains(&code) || // Arabic
        code == 0x0670 || // Arabic
        (0x06D6..=0x06DC).contains(&code) || // Arabic
        (0x06DF..=0x06E4).contains(&code) || // Arabic
        (0x06E7..=0x06E8).contains(&code) || // Arabic
        (0x06EA..=0x06ED).contains(&code) || // Arabic
        code == 0x0711 || // Syriac
        (0x0730..=0x074A).contains(&code) || // Syriac
        (0x07A6..=0x07B0).contains(&code) || // Thaana
        (0x0900..=0x0902).contains(&code) || // Devanagari
        code == 0x093A || code == 0x093C || // Devanagari
        (0x0941..=0x0948).contains(&code) || // Devanagari
        code == 0x094D || code == 0x0951 || code == 0x0955 || code == 0x0962 || code == 0x0963 ||
        (0x1DC0..=0x1DFF).contains(&code) || // combining diacritical marks supplement
        (0x20D0..=0x20FF).contains(&code) || // combining diacritical marks for symbols
        (0xFE20..=0xFE2F).contains(&code) // combining half marks
}

thread_local! {
    static GRAPHEME_WIDTH_CACHE: RefCell<HashMap<u32, u8>> = RefCell::new(HashMap::new());
}

pub fn grapheme_width(grapheme: &str) -> usize {
    if grapheme.is_empty() {
        return 0;
    }
    let code = grapheme.chars().next().unwrap() as u32;

    // The cache is keyed by the FIRST codepoint, which is only a valid key for
    // single-codepoint graphemes: multi-codepoint graphemes (VS15/VS16
    // variation selectors, ZWJ chains, keycap sequences) can render at a
    // different width for the same leading codepoint — e.g. "✅" is 2 cells but
    // "✅\uFE0E" is 1. Those bypass the cache and are computed fresh.
    let single_codepoint = grapheme.chars().count() == 1;

    if single_codepoint {
        if let Some(cached) = GRAPHEME_WIDTH_CACHE.with(|c| c.borrow().get(&code).copied()) {
            return cached as usize;
        }
    }

    let width: u8 = if is_zero_width(code) {
        0
    } else if is_emoji_flag(grapheme)
        || has_emoji_presentation(grapheme)
        || (has_default_emoji_presentation(code) && !grapheme.contains('\u{FE0E}'))
        || is_keycap_sequence(grapheme)
        || is_cjk(code)
        || code >= 0x1F000
    // High-plane characters default to 2 (the TS keeps the CJK and
    // high-plane conditions separate but both yield 2 cells).
    {
        2
    } else {
        1
    };

    if single_codepoint {
        GRAPHEME_WIDTH_CACHE.with(|c| c.borrow_mut().insert(code, width));
    }
    width as usize
}

// ─── Visible Width ─────────────────────────────────────────────────────────

const PUNCTUATION_CHARS: &[char] = &[
    '(', ')', '{', '}', '[', ']', '<', '>', '.', ',', ';', ':', '\'', '"', '!', '?', '+', '-', '=',
    '*', '/', '\\', '|', '&', '%', '^', '$', '#', '@', '~', '`',
];

pub fn is_whitespace_char(c: char) -> bool {
    c.is_whitespace()
}

pub fn is_punctuation_char(c: char) -> bool {
    PUNCTUATION_CHARS.contains(&c)
}

thread_local! {
    static VISIBLE_WIDTH_CACHE: RefCell<HashMap<String, usize>> = RefCell::new(HashMap::new());
}

const VISIBLE_WIDTH_CACHE_MAX: usize = 2000;

pub fn visible_width(s: &str) -> usize {
    if let Some(cached) = VISIBLE_WIDTH_CACHE.with(|c| c.borrow().get(s).copied()) {
        return cached;
    }

    // Strip ANSI codes before computing visible width — escape sequences
    // have zero visible width but individual bytes would be counted as width 1.
    let clean: Cow<'_, str> = if s.contains('\x1b') {
        Cow::Owned(strip_ansi_codes(s))
    } else {
        Cow::Borrowed(s)
    };

    let width: usize = clean.graphemes(true).map(grapheme_width).sum();

    VISIBLE_WIDTH_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() < VISIBLE_WIDTH_CACHE_MAX {
            c.insert(s.to_string(), width);
        }
    });
    width
}

pub fn clear_visible_width_cache() {
    VISIBLE_WIDTH_CACHE.with(|c| c.borrow_mut().clear());
    GRAPHEME_WIDTH_CACHE.with(|c| c.borrow_mut().clear());
}

// ─── Tab Handling ──────────────────────────────────────────────────────────

pub fn replace_tabs(s: &str, tab_width: usize) -> String {
    if tab_width == 0 {
        return s.replace('\t', "");
    }
    s.replace('\t', &" ".repeat(tab_width))
}

// ─── Normalize Terminal Output ─────────────────────────────────────────────

// Thai/Lao AM (above-main) combining vowels — only the standalone form
// (U+0E33, U+0EB3) needs decomposition; all other combining marks are zero-width
// and handled correctly by grapheme_width/visible_width.
pub fn normalize_terminal_output(s: &str) -> String {
    let s = replace_tabs(s, 3);
    // Decompose Thai/Lao AM vowels into base + combining marks for
    // terminal compatibility.
    if !s.contains('\u{0e33}') && !s.contains('\u{0eb3}') {
        return s;
    }
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\u{0e33}' => out.push_str("\u{0e4d}\u{0e32}"),
            '\u{0eb3}' => out.push_str("\u{0ecd}\u{0eb2}"),
            _ => out.push(ch),
        }
    }
    out
}

// ─── ANSI Code Extraction ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnsiKind {
    Sgr,
    Csi,
    Osc,
    Apc,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnsiCodeResult {
    pub length: usize,
    pub code: String,
    pub kind: AnsiKind,
}

/// Extract one ANSI escape sequence starting at byte `pos`.
///
/// JS indexes by UTF-16 unit; every meaningful byte here is ASCII (ESC and the
/// terminator bytes), so byte positions are equivalent. The one non-ASCII case
/// — the meta-key branch (ESC + single char) — consumes a whole char so the
/// returned length always lands on a char boundary.
pub fn extract_ansi_code(s: &str, pos: usize) -> Option<AnsiCodeResult> {
    if pos >= s.len() {
        return None;
    }
    if s.as_bytes()[pos] != 0x1b {
        return None;
    }

    let rest = &s[pos..];
    if rest.len() < 2 {
        return None;
    }
    let rest_bytes = rest.as_bytes();

    // OSC: ESC ]
    if rest_bytes[1] == b']' {
        // Terminated by BEL (\x07) or ESC \ (ST)
        let bel_idx = rest[2..].find('\x07').map(|i| i + 2);
        let st_idx = rest[2..].find("\x1b\\").map(|i| i + 2);
        let end_idx = match (bel_idx, st_idx) {
            (Some(b), Some(s)) => Some(b.min(s)),
            (Some(b), None) => Some(b),
            (None, Some(s)) => Some(s),
            (None, None) => None,
        }?;
        let length = end_idx + if rest_bytes[end_idx] == 0x07 { 1 } else { 2 };
        return Some(AnsiCodeResult {
            length,
            code: rest[..length].to_string(),
            kind: AnsiKind::Osc,
        });
    }

    // APC: ESC _ — terminated by BEL (\x07) or ST (\x1b\\)
    if rest_bytes[1] == b'_' {
        let bel_idx = rest[2..].find('\x07').map(|i| i + 2);
        let st_idx = rest[2..].find("\x1b\\").map(|i| i + 2);
        let end_idx = match (bel_idx, st_idx) {
            (Some(b), Some(s)) => Some(b.min(s)),
            (Some(b), None) => Some(b),
            (None, Some(s)) => Some(s),
            (None, None) => None,
        }?;
        let length = end_idx + if rest_bytes[end_idx] == 0x07 { 1 } else { 2 };
        return Some(AnsiCodeResult {
            length,
            code: rest[..length].to_string(),
            kind: AnsiKind::Apc,
        });
    }

    // CSI: ESC [
    if rest_bytes[1] == b'[' {
        let mut i = 2;
        while i < rest.len() {
            let c = rest_bytes[i];
            if (0x40..=0x7e).contains(&c) {
                let code = rest[..i + 1].to_string();
                let kind = if code.ends_with('m') {
                    AnsiKind::Sgr
                } else {
                    AnsiKind::Csi
                };
                return Some(AnsiCodeResult {
                    length: i + 1,
                    code,
                    kind,
                });
            }
            if !(0x20..=0x3f).contains(&c) {
                break; // not valid CSI parameter
            }
            i += 1;
        }
        return None; // unterminated
    }

    // SS3: ESC O
    if rest_bytes[1] == b'O' && rest.len() >= 3 {
        return Some(AnsiCodeResult {
            length: 3,
            code: rest[..3].to_string(),
            kind: AnsiKind::Csi,
        });
    }

    // Meta/Alt: ESC + single char (`rest.len() >= 2` is guaranteed by the
    // early return above).
    let ch = rest[1..].chars().next().unwrap();
    let length = 1 + ch.len_utf8();
    Some(AnsiCodeResult {
        length,
        code: rest[..length].to_string(),
        kind: AnsiKind::Other,
    })
}

// ─── ANSI Code Tracker ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AnsiState {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub blink: bool,
    pub inverse: bool,
    pub hidden: bool,
    pub strikethrough: bool,
    pub fg: Option<String>, // ANSI color sequence, e.g. "38;5;45"
    pub bg: Option<String>,
    pub link: Option<String>, // OSC 8 hyperlink URL
}

fn sgr_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\x1b\[([\d;]*)m$").unwrap())
}

fn osc8_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\x1b\]8;([^;\x07]*);([^\x07]*)\x07$").unwrap())
}

pub struct AnsiCodeTracker {
    state: AnsiState,
    stack: Vec<AnsiState>,
}

impl Default for AnsiCodeTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl AnsiCodeTracker {
    pub fn new() -> Self {
        Self {
            state: AnsiState::default(),
            stack: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        self.state = AnsiState::default();
        self.stack.clear();
    }

    pub fn push_state(&mut self) {
        self.stack.push(self.state.clone());
    }

    pub fn pop_state(&mut self) {
        if let Some(prev) = self.stack.pop() {
            self.state = prev;
        }
    }

    /// The standard/bright SGR color ranges are handled by the catch-all arm
    /// (the TS `feed` switch-default), which is the live path for them.
    #[allow(clippy::if_same_then_else)]
    pub fn feed(&mut self, code: &str) {
        if code == "\x1b[0m" || code == "\x1b[m" {
            self.state = AnsiState::default();
            return;
        }

        let Some(sgr_match) = sgr_re().captures(code) else {
            // OSC 8 hyperlink
            if let Some(osc_match) = osc8_re().captures(code) {
                let url = osc_match.get(2).map(|m| m.as_str().to_string());
                self.state.link = url;
            }
            return;
        };

        let params: Vec<u32> = sgr_match
            .get(1)
            .map(|m| {
                m.as_str()
                    .split(';')
                    .map(|p| {
                        if p.is_empty() {
                            0
                        } else {
                            p.parse::<u32>().unwrap_or(u32::MAX)
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut i = 0;
        while i < params.len() {
            let p = params[i];
            match p {
                0 => self.state = AnsiState::default(),
                1 => self.state.bold = true,
                2 => self.state.dim = true,
                3 => self.state.italic = true,
                4 => self.state.underline = true,
                5 | 6 => self.state.blink = true,
                7 => self.state.inverse = true,
                8 => self.state.hidden = true,
                9 => self.state.strikethrough = true,
                21 | 22 => {
                    self.state.bold = false;
                    self.state.dim = false;
                }
                23 => self.state.italic = false,
                24 => self.state.underline = false,
                25 => self.state.blink = false,
                27 => self.state.inverse = false,
                28 => self.state.hidden = false,
                29 => self.state.strikethrough = false,
                38 => {
                    if params.get(i + 1) == Some(&5) && params.get(i + 2).is_some() {
                        self.state.fg = Some(format!("38;5;{}", params[i + 2]));
                        i += 2;
                    } else if params.get(i + 1) == Some(&2) && params.get(i + 4).is_some() {
                        self.state.fg = Some(format!(
                            "38;2;{};{};{}",
                            params[i + 2],
                            params[i + 3],
                            params[i + 4]
                        ));
                        i += 4;
                    }
                }
                48 => {
                    if params.get(i + 1) == Some(&5) && params.get(i + 2).is_some() {
                        self.state.bg = Some(format!("48;5;{}", params[i + 2]));
                        i += 2;
                    } else if params.get(i + 1) == Some(&2) && params.get(i + 4).is_some() {
                        self.state.bg = Some(format!(
                            "48;2;{};{};{}",
                            params[i + 2],
                            params[i + 3],
                            params[i + 4]
                        ));
                        i += 4;
                    }
                }
                39 => self.state.fg = None,
                49 => self.state.bg = None,
                _ => {
                    // Standard + bright SGR colors. Previously these fell through
                    // silently, so a wrapped line lost e.g. \x1b[31m red on its
                    // continuation — the tracker only knew 38/48 extended forms.
                    if (30..=37).contains(&p) {
                        self.state.fg = Some(p.to_string());
                    } else if (90..=97).contains(&p) {
                        self.state.fg = Some(p.to_string());
                    } else if (40..=47).contains(&p) {
                        self.state.bg = Some(p.to_string());
                    } else if (100..=107).contains(&p) {
                        self.state.bg = Some(p.to_string());
                    }
                }
            }
            i += 1;
        }
    }

    pub fn get_ansi_code(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.state.bold {
            parts.push("1");
        }
        if self.state.dim {
            parts.push("2");
        }
        if self.state.italic {
            parts.push("3");
        }
        if self.state.underline {
            parts.push("4");
        }
        if self.state.blink {
            parts.push("5");
        }
        if self.state.inverse {
            parts.push("7");
        }
        if self.state.hidden {
            parts.push("8");
        }
        if self.state.strikethrough {
            parts.push("9");
        }
        if let Some(fg) = &self.state.fg {
            parts.push(fg);
        }
        if let Some(bg) = &self.state.bg {
            parts.push(bg);
        }
        if parts.is_empty() {
            return "\x1b[0m".to_string();
        }
        format!("\x1b[{}m", parts.join(";"))
    }

    pub fn get_state(&self) -> &AnsiState {
        &self.state
    }

    /// Get OSC 8 hyperlink open sequence if a link is active.
    pub fn get_osc8_link(&self) -> String {
        if let Some(link) = &self.state.link {
            return format!("\x1b]8;id=future_tui;{link}\x07");
        }
        String::new()
    }

    /// Get OSC 8 close sequence.
    pub fn get_osc8_close(&self) -> &'static str {
        "\x1b]8;;\x07"
    }
}

// ─── Strip ANSI ────────────────────────────────────────────────────────────

pub fn strip_ansi_codes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if let Some(code) = extract_ansi_code(s, i) {
            i += code.length;
        } else {
            let ch = s[i..].chars().next().unwrap();
            result.push(ch);
            i += ch.len_utf8();
        }
    }
    result
}

// ─── Word Wrap with ANSI ───────────────────────────────────────────────────

/// Fast path for pure-ASCII text without escape codes: every character is
/// width 1, so wrapping reduces to index arithmetic. The grapheme path below
/// re-slices the string and builds a segmenter iterator per grapheme, which
/// dominates markdown re-render cost during streaming — assistant output is
/// overwhelmingly ASCII, so this pays off disproportionately.
/// Output is byte-identical to the grapheme path (including the trailing
/// SGR reset each line gets from finalize_line).
fn is_pure_ascii_wrappable(text: &str) -> bool {
    for c in text.chars() {
        // Allow printable ASCII + newline; control chars may be zero-width in
        // the grapheme path, so they take the slow path for identical output.
        if !(c == '\n' || (32..=126).contains(&(c as u32))) {
            return false;
        }
    }
    true
}

fn wrap_ascii_fast(text: &str, width: usize) -> Vec<String> {
    const RESET: &str = "\x1b[0m";
    let mut lines: Vec<String> = Vec::new();
    let raw_lines: Vec<&str> = text.split('\n').collect();
    for (li, raw_line) in raw_lines.iter().enumerate() {
        let is_last = li == raw_lines.len() - 1;
        let mut rest = *raw_line;
        if rest.is_empty() {
            // Mirrors the grapheme path: a "\n" always flushes the (possibly
            // empty) current line; a trailing "\n" leaves nothing to flush.
            if !is_last {
                lines.push(RESET.to_string());
            }
            continue;
        }
        while rest.len() > width {
            // Prefer breaking at the last space within the window (word boundary);
            // fall back to a hard break exactly at width. `spaceIdx > 0` matches
            // the grapheme path, which hard-breaks when the only space leads.
            let space_idx = rest[..width.min(rest.len())].rfind(' ');
            if let Some(space_idx) = space_idx {
                if space_idx > 0 {
                    lines.push(rest[..space_idx].to_string() + RESET);
                    rest = &rest[space_idx + 1..];
                    continue;
                }
            }
            lines.push(rest[..width].to_string() + RESET);
            rest = &rest[width..];
        }
        if !rest.is_empty() {
            lines.push(rest.to_string() + RESET);
        }
    }
    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

pub fn wrap_text_with_ansi(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    if !text.contains('\x1b') && is_pure_ascii_wrappable(text) {
        return wrap_ascii_fast(text, width);
    }
    let mut lines: Vec<String> = Vec::new();
    let mut tracker = AnsiCodeTracker::new();
    let mut current_line = String::new();
    let mut current_width = 0usize;
    let mut i = 0usize;

    while i < text.len() {
        if text.as_bytes()[i] == b'\n' {
            lines.push(current_line.clone() + &finalize_line(&tracker));
            current_line.clear();
            current_width = 0;
            i += 1;
            continue;
        }

        // Check for ANSI code
        if let Some(ansi) = extract_ansi_code(text, i) {
            tracker.feed(&ansi.code);
            current_line.push_str(&ansi.code);
            i += ansi.length;
            continue;
        }

        // Grab one grapheme
        let grapheme = text[i..].graphemes(true).next().unwrap();
        let gw = grapheme_width(grapheme);

        if current_width + gw > width {
            // Word-boundary break: backtrack to last space
            let space_idx = current_line.rfind(' ');
            if let Some(space_idx) = space_idx {
                if space_idx > 0 && !is_all_ansi(&current_line[..space_idx]) {
                    // Check visible width up to space
                    let after_space = current_line[space_idx + 1..].to_string();
                    // Push line up to space, wrap remainder
                    lines.push(current_line[..space_idx].to_string() + &finalize_line(&tracker));
                    current_line =
                        tracker.get_ansi_code() + &tracker.get_osc8_link() + &after_space;
                    current_width = visible_width(&strip_ansi_codes(&after_space));
                    current_line.push_str(grapheme);
                    current_width += gw;
                } else {
                    // Hard break at width
                    lines.push(current_line.clone() + &finalize_line(&tracker));
                    current_line = tracker.get_ansi_code() + &tracker.get_osc8_link() + grapheme;
                    current_width = gw;
                }
            } else {
                // Hard break at width
                lines.push(current_line.clone() + &finalize_line(&tracker));
                current_line = tracker.get_ansi_code() + &tracker.get_osc8_link() + grapheme;
                current_width = gw;
            }
        } else {
            current_line.push_str(grapheme);
            current_width += gw;
        }
        i += grapheme.len();
    }

    if !current_line.is_empty() {
        lines.push(current_line + &finalize_line(&tracker));
    }

    // `lines` is never empty here: empty input takes the ASCII fast path,
    // and any grapheme/escape processed above leaves a non-empty line.
    lines
}

fn is_all_ansi(s: &str) -> bool {
    let mut i = 0;
    while i < s.len() {
        if let Some(code) = extract_ansi_code(s, i) {
            i += code.length;
        } else {
            return false;
        }
    }
    true
}

fn finalize_line(tracker: &AnsiCodeTracker) -> String {
    let osc_close = if tracker.get_state().link.is_some() {
        tracker.get_osc8_close()
    } else {
        ""
    };
    format!("{osc_close}\x1b[0m")
}

// ─── Apply Background to Line ──────────────────────────────────────────────

/// Replace every `\x1b[m` / `\x1b[0m` with `\x1b[0m` + bgCode so mid-line style
/// resets (from markdown links, code spans, etc.) don't clear the background.
/// Port of the `/\x1b\[0?m/g` replace in `applyBackgroundToLine`.
fn replace_reset_codes(line: &str, bg_code: &str) -> String {
    let mut out = String::with_capacity(line.len() + 16);
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 2 < bytes.len() && bytes[i + 1] == b'[' {
            if bytes[i + 2] == b'm' {
                out.push_str("\x1b[0m");
                out.push_str(bg_code);
                i += 3;
                continue;
            }
            if bytes[i + 2] == b'0' && i + 3 < bytes.len() && bytes[i + 3] == b'm' {
                out.push_str("\x1b[0m");
                out.push_str(bg_code);
                i += 4;
                continue;
            }
        }
        let ch = line[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

pub fn apply_background_to_line(line: &str, width: usize, bg: i16) -> String {
    let visible_len = visible_width(&strip_ansi_codes(line));
    let padding = width.saturating_sub(visible_len);

    // -1 means use terminal default background (no explicit bg color)
    if bg < 0 {
        return line.to_string() + &" ".repeat(padding);
    }

    let bg_code = format!("\x1b[48;5;{bg}m");

    // Replace every RESET with RESET + bgCode so mid-line style resets
    // (from markdown links, code spans, etc.) don't clear the background.
    let safe_line = replace_reset_codes(line, &bg_code);

    format!(
        "{bg_code}{safe_line}\x1b[0m{bg_code}{}\x1b[0m",
        " ".repeat(padding)
    )
}

// ─── Truncate to Width ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default)]
pub struct TruncateOptions {
    pub ellipsis: bool,
    pub pad: bool,
}

pub fn truncate_to_width(s: &str, width: usize, opts: &TruncateOptions) -> String {
    if width == 0 {
        return String::new();
    }
    let result = slice_with_width(s, width);
    if result.text.len() < s.len() && opts.ellipsis {
        // Replace last char with ellipsis, keeping ANSI context.
        let mut text = result.text;
        text.pop();
        return text + "…";
    }
    if opts.pad && result.width < width {
        return result.text + &" ".repeat(width - result.width);
    }
    result.text
}

// ─── Slice by Column ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceResult {
    pub text: String,
    pub width: usize,
}

pub fn slice_with_width(s: &str, max_width: usize) -> SliceResult {
    let mut result = String::new();
    let mut width = 0usize;
    let mut i = 0usize;

    while i < s.len() && width < max_width {
        if let Some(ansi) = extract_ansi_code(s, i) {
            result.push_str(&ansi.code);
            i += ansi.length;
            continue;
        }

        let grapheme = s[i..].graphemes(true).next().unwrap();
        let gw = grapheme_width(grapheme);
        if width + gw > max_width {
            break;
        }

        result.push_str(grapheme);
        width += gw;
        i += grapheme.len();
    }

    SliceResult {
        text: result,
        width,
    }
}

pub fn slice_by_column(s: &str, start: usize, end: Option<usize>) -> String {
    let mut col = 0usize;
    let mut i = 0usize;
    let mut result = String::new();

    // Skip to start
    while i < s.len() && col < start {
        if let Some(ansi) = extract_ansi_code(s, i) {
            i += ansi.length;
            continue;
        }
        let grapheme = s[i..].graphemes(true).next().unwrap();
        col += grapheme_width(grapheme);
        i += grapheme.len();
    }

    let Some(end) = end else {
        return s[i..].to_string();
    };

    // Extract [start, end)
    while i < s.len() && col < end {
        if let Some(ansi) = extract_ansi_code(s, i) {
            result.push_str(&ansi.code);
            i += ansi.length;
            continue;
        }
        let grapheme = s[i..].graphemes(true).next().unwrap();
        let gw = grapheme_width(grapheme);
        if col + gw > end {
            break;
        }
        result.push_str(grapheme);
        col += gw;
        i += grapheme.len();
    }

    result
}

// ─── Extract Segments (for overlay compositing) ────────────────────────────

thread_local! {
    // Pooled tracker instance for extract_segments (avoids allocation per call)
    static POOLED_STYLE_TRACKER: RefCell<AnsiCodeTracker> = RefCell::new(AnsiCodeTracker::new());
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segments {
    pub before: String,
    pub before_width: usize,
    pub after: String,
    pub after_width: usize,
}

/// Extract "before" and "after" segments from a line in a single pass.
/// Used for overlay compositing where we need content before and after the
/// overlay region. Preserves styling from before the overlay that should
/// affect content after it.
pub fn extract_segments(
    line: &str,
    before_end: usize,
    after_start: usize,
    after_len: usize,
    strict_after: bool,
) -> Segments {
    let mut before = String::new();
    let mut before_width = 0usize;
    let mut after = String::new();
    let mut after_width = 0usize;
    let mut current_col = 0usize;
    let mut i = 0usize;
    let mut pending_ansi_before = String::new();
    let mut after_started = false;
    let after_end = after_start + after_len;

    POOLED_STYLE_TRACKER.with(|slot| {
        let mut tracker = slot.borrow_mut();
        tracker.reset();

        while i < line.len() {
            if let Some(ansi) = extract_ansi_code(line, i) {
                tracker.feed(&ansi.code);
                if current_col < before_end {
                    pending_ansi_before.push_str(&ansi.code);
                } else if current_col >= after_start && current_col < after_end && after_started {
                    after.push_str(&ansi.code);
                }
                i += ansi.length;
                continue;
            }

            // Advance to the next ANSI sequence, staying on char boundaries.
            let mut text_end = i;
            while text_end < line.len() {
                if extract_ansi_code(line, text_end).is_some() {
                    break;
                }
                text_end += line[text_end..].chars().next().unwrap().len_utf8();
            }

            for grapheme in line[i..text_end].graphemes(true) {
                let w = grapheme_width(grapheme);

                if current_col < before_end {
                    if !pending_ansi_before.is_empty() {
                        before.push_str(&pending_ansi_before);
                        pending_ansi_before.clear();
                    }
                    before.push_str(grapheme);
                    before_width += w;
                } else if current_col >= after_start && current_col < after_end {
                    let fits = !strict_after || current_col + w <= after_end;
                    if fits {
                        if !after_started {
                            after.push_str(&tracker.get_ansi_code());
                            after_started = true;
                        }
                        after.push_str(grapheme);
                        after_width += w;
                    }
                }

                current_col += w;
                let done = if after_len == 0 {
                    current_col >= before_end
                } else {
                    current_col >= after_end
                };
                if done {
                    break;
                }
            }
            i = text_end;
            let done = if after_len == 0 {
                current_col >= before_end
            } else {
                current_col >= after_end
            };
            if done {
                break;
            }
        }
    });

    Segments {
        before,
        before_width,
        after,
        after_width,
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─── graphemeWidth ─────────────────────────────────────────────────────

    #[test]
    fn ascii_is_1_cell() {
        assert_eq!(grapheme_width("a"), 1);
        assert_eq!(grapheme_width(" "), 1);
    }

    #[test]
    fn cjk_is_2_cells() {
        assert_eq!(grapheme_width("中"), 2);
        assert_eq!(grapheme_width("あ"), 2);
        assert_eq!(grapheme_width("한"), 2);
    }

    #[test]
    fn emoji_is_2_cells() {
        assert_eq!(grapheme_width("🦀"), 2);
        assert_eq!(grapheme_width("🎉"), 2);
        assert_eq!(grapheme_width("🚀"), 2);
        assert_eq!(grapheme_width("🔧"), 2);
    }

    #[test]
    fn vs15_forces_text_presentation_narrow() {
        // U+2705 + U+FE0E renders as a 1-cell text glyph, not an emoji.
        assert_eq!(grapheme_width("✅\u{FE0E}"), 1);
        // VS16 (emoji presentation) stays wide.
        assert_eq!(grapheme_width("✅\u{FE0F}"), 2);
    }

    #[test]
    fn combining_marks_are_0_cells() {
        assert_eq!(grapheme_width("\u{0301}"), 0); // U+0301 combining acute
    }

    #[test]
    fn empty_string_is_0() {
        assert_eq!(grapheme_width(""), 0);
    }

    // ─── visibleWidth ──────────────────────────────────────────────────────

    #[test]
    fn plain_ascii() {
        assert_eq!(visible_width("hello"), 5);
    }

    #[test]
    fn cjk_counts_double() {
        assert_eq!(visible_width("你好"), 4);
        assert_eq!(visible_width("a中b"), 4);
    }

    #[test]
    fn emoji_counts_double() {
        assert_eq!(visible_width("🦀🦀"), 4);
    }

    #[test]
    fn ansi_escape_codes_are_invisible() {
        assert_eq!(visible_width("\x1b[31mred\x1b[0m"), 3);
        assert_eq!(visible_width("\x1b[1;42mbold green\x1b[m"), 10);
    }

    #[test]
    fn osc8_hyperlinks_are_invisible() {
        let link = "\x1b]8;;https://example.com\x07click\x1b]8;;\x07";
        assert_eq!(visible_width(link), 5);
    }

    // ─── stripAnsiCodes ────────────────────────────────────────────────────

    #[test]
    fn removes_csi_sequences() {
        assert_eq!(strip_ansi_codes("\x1b[31mhi\x1b[0m"), "hi");
        assert_eq!(strip_ansi_codes("\x1b[1;2;3mx"), "x");
    }

    #[test]
    fn removes_osc_sequences() {
        assert_eq!(
            strip_ansi_codes("\x1b]8;;https://x.dev\x07text\x1b]8;;\x07"),
            "text"
        );
    }

    #[test]
    fn passes_plain_text_through() {
        assert_eq!(strip_ansi_codes("plain 中文 🦀"), "plain 中文 🦀");
    }

    // ─── replaceTabs ───────────────────────────────────────────────────────

    #[test]
    fn default_tab_width_is_3() {
        assert_eq!(replace_tabs("a\tb", 3), "a   b");
    }

    #[test]
    fn custom_tab_width() {
        assert_eq!(replace_tabs("a\tb", 4), "a    b");
    }

    // ─── truncateToWidth ───────────────────────────────────────────────────

    #[test]
    fn no_op_when_shorter_than_width() {
        assert_eq!(
            truncate_to_width("abc", 5, &TruncateOptions::default()),
            "abc"
        );
    }

    #[test]
    fn truncates_ascii_to_width() {
        assert_eq!(
            truncate_to_width("abcdef", 3, &TruncateOptions::default()),
            "abc"
        );
    }

    #[test]
    fn never_splits_a_cjk_character() {
        // 3 cells requested, but the second CJK char needs cells 3-4 → dropped.
        assert_eq!(
            truncate_to_width("中文中文", 3, &TruncateOptions::default()),
            "中"
        );
        assert_eq!(
            truncate_to_width("中文", 4, &TruncateOptions::default()),
            "中文"
        );
    }

    #[test]
    fn ellipsis_replaces_last_char_when_truncated() {
        let opts = TruncateOptions {
            ellipsis: true,
            pad: false,
        };
        assert_eq!(truncate_to_width("abcdef", 3, &opts), "ab…");
        // Not truncated → no ellipsis.
        assert_eq!(truncate_to_width("abc", 3, &opts), "abc");
    }

    #[test]
    fn pad_fills_to_width() {
        let opts = TruncateOptions {
            ellipsis: false,
            pad: true,
        };
        assert_eq!(truncate_to_width("ab", 4, &opts), "ab  ");
    }

    #[test]
    fn width_zero_yields_empty() {
        assert_eq!(truncate_to_width("abc", 0, &TruncateOptions::default()), "");
    }

    // ─── sliceWithWidth / sliceByColumn ────────────────────────────────────

    #[test]
    fn reports_consumed_width() {
        let r = slice_with_width("hello", 3);
        assert_eq!((r.text.as_str(), r.width), ("hel", 3));
    }

    #[test]
    fn cjk_consumes_2_cells() {
        let r = slice_with_width("中文x", 2);
        assert_eq!((r.text.as_str(), r.width), ("中", 2));
    }

    #[test]
    fn ansi_codes_travel_with_the_slice_but_cost_no_width() {
        let r = slice_with_width("\x1b[31mabc\x1b[0m", 2);
        assert_eq!(strip_ansi_codes(&r.text), "ab");
        assert_eq!(r.width, 2);
    }

    #[test]
    fn slices_start_end_in_cells() {
        assert_eq!(slice_by_column("hello world", 0, Some(5)), "hello");
        assert_eq!(slice_by_column("hello world", 6, None), "world");
    }

    #[test]
    fn cjk_columns() {
        assert_eq!(slice_by_column("a中b", 1, Some(3)), "中");
    }

    // ─── wrapTextWithAnsi ──────────────────────────────────────────────────

    fn wrapped_plain(text: &str, width: usize) -> Vec<String> {
        wrap_text_with_ansi(text, width)
            .iter()
            .map(|l| strip_ansi_codes(l))
            .collect()
    }

    #[test]
    fn wraps_at_width_with_hard_break() {
        assert_eq!(wrapped_plain("abcdefgh", 4), vec!["abcd", "efgh"]);
    }

    #[test]
    fn prefers_word_boundary_over_hard_break() {
        assert_eq!(wrapped_plain("hello world", 6), vec!["hello", "world"]);
    }

    #[test]
    fn splits_on_newlines_first() {
        assert_eq!(wrapped_plain("ab\ncd", 10), vec!["ab", "cd"]);
    }

    #[test]
    fn cjk_wrapping_respects_double_width() {
        assert_eq!(wrapped_plain("中文中文", 4), vec!["中文", "中文"]);
    }

    #[test]
    fn carries_active_style_onto_the_next_line() {
        // Use 256-color escape which the tracker models natively.
        let lines = wrap_text_with_ansi("\x1b[38;5;1mabcdefgh\x1b[0m", 4);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("\x1b[38;5;1m"));
        assert!(lines[1].contains("efgh"));
    }

    #[test]
    fn standard_and_256_color_styles_both_survive_wrapping() {
        // SGR 31 (standard red) and 38;5;45 (256-color) must both be re-opened
        // on continuation lines — the tracker previously dropped 30-37/40-47.
        let std = wrap_text_with_ansi("\x1b[31mabcdefgh", 4);
        assert!(std[1].contains("\x1b[31m"));

        let ext = wrap_text_with_ansi("\x1b[38;5;45mabcdefgh", 4);
        assert!(ext[1].contains("38;5;45"));

        let bright = wrap_text_with_ansi("\x1b[91mabcdefgh", 4);
        assert!(bright[1].contains("\x1b[91m"));
    }

    #[test]
    fn width_zero_returns_no_lines() {
        assert_eq!(wrap_text_with_ansi("abc", 0), Vec::<String>::new());
    }

    // ─── applyBackgroundToLine ─────────────────────────────────────────────

    #[test]
    fn pads_to_width_with_bg_color() {
        let out = apply_background_to_line("ab", 5, 42);
        assert!(out.contains("\x1b[48;5;42m"));
        assert_eq!(visible_width(&out), 5);
    }

    #[test]
    fn bg_negative_pads_with_terminal_default() {
        let out = apply_background_to_line("ab", 5, -1);
        assert!(!out.contains("48;5"));
        assert!(out.ends_with("   "));
    }

    #[test]
    fn mid_line_resets_are_re_armed_with_the_bg_color() {
        let out = apply_background_to_line("\x1b[31mab\x1b[0m", 5, 42);
        // The plain \x1b[0m from the content must be upgraded to \x1b[0m + bg.
        assert!(out.contains("\x1b[0m\x1b[48;5;42m"));
    }

    // ─── wrapTextWithAnsi ASCII fast path ──────────────────────────────────

    /// Reference implementation of the original grapheme-based algorithm,
    /// specialized for plain ASCII (graphemes == chars, all width 1, no ANSI).
    /// The fast path in wrap_text_with_ansi must be byte-identical to this.
    fn ref_wrap_ascii(text: &str, width: usize) -> Vec<String> {
        const RESET: &str = "\x1b[0m";
        let mut lines: Vec<String> = Vec::new();
        let mut cur = String::new();
        for ch in text.chars() {
            if ch == '\n' {
                lines.push(cur.clone() + RESET);
                cur.clear();
                continue;
            }
            if cur.chars().count() + 1 > width {
                let space_idx = cur.rfind(' ');
                if let Some(space_idx) = space_idx {
                    if space_idx > 0 {
                        lines.push(cur[..space_idx].to_string() + RESET);
                        cur = cur[space_idx + 1..].to_string() + &ch.to_string();
                    } else {
                        lines.push(cur.clone() + RESET);
                        cur = ch.to_string();
                    }
                } else {
                    lines.push(cur.clone() + RESET);
                    cur = ch.to_string();
                }
            } else {
                cur.push(ch);
            }
        }
        if !cur.is_empty() {
            lines.push(cur + RESET);
        }
        if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        }
    }

    #[test]
    fn fuzz_fast_path_matches_grapheme_algorithm_byte_for_byte() {
        let mut seed: u32 = 0x2f6e2b1;
        let mut rand = move || {
            // xorshift32 — deterministic, no Math.random flakiness
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f64 / u32::MAX as f64
        };
        let alphabet: Vec<char> = "abcde fgh   \n\n".chars().collect(); // extra spaces/newlines bias edge cases
        for _ in 0..3000 {
            let len = (rand() * 120.0) as usize;
            let mut s = String::new();
            for _ in 0..len {
                s.push(alphabet[(rand() * alphabet.len() as f64) as usize]);
            }
            let width = 1 + (rand() * 24.0) as usize;
            assert_eq!(wrap_text_with_ansi(&s, width), ref_wrap_ascii(&s, width));
        }
    }

    #[test]
    fn long_single_word_hard_breaks_at_width() {
        let lines = wrap_text_with_ansi(&"x".repeat(1000), 80);
        assert_eq!(lines.len(), 13); // 12*80 + 40
        for l in lines.iter().take(12) {
            assert_eq!(visible_width(l), 80);
        }
    }

    // ─── extract_segments ──────────────────────────────────────────────────

    #[test]
    fn extracts_before_and_after_around_overlay() {
        // "hello world": before = cols [0,5) = "hello"; overlay region =
        // cols [6,11) = "world" (the after segment carries a style prefix).
        let seg = extract_segments("hello world", 5, 6, 5, false);
        assert_eq!(seg.before, "hello");
        assert_eq!(seg.before_width, 5);
        assert_eq!(strip_ansi_codes(&seg.after), "world");
        assert_eq!(seg.after_width, 5);
    }

    #[test]
    fn preserves_style_before_overlay_into_after() {
        // Style before the overlay must be re-opened on the after segment.
        let seg = extract_segments("\x1b[31mhello world", 5, 6, 5, false);
        assert_eq!(seg.before, "\x1b[31mhello");
        assert!(seg.after.starts_with("\x1b[31m"));
        assert_eq!(strip_ansi_codes(&seg.after), "world");
        assert_eq!(seg.after_width, 5);
    }

    #[test]
    fn strict_after_clips_partial_graphemes() {
        // afterLen 2 with CJK: "中" needs 2 cells starting at col 1 → fits at
        // [1,3) exactly; "b" at col 3 would exceed [1,3).
        let seg = extract_segments("a中b", 1, 1, 2, true);
        assert_eq!(seg.before, "a");
        assert_eq!(strip_ansi_codes(&seg.after), "中");
        assert_eq!(seg.after_width, 2);
    }

    // ─── emoji sequence classifiers ─────────────────────────────────────

    #[test]
    fn keycap_sequences_are_recognized() {
        assert!(is_keycap_sequence("1\u{FE0F}\u{20E3}"));
        assert!(is_keycap_sequence("#\u{FE0F}\u{20E3}"));
        assert!(!is_keycap_sequence("1"));
        assert!(!is_keycap_sequence("ab"));
        assert!(!is_keycap_sequence("1\u{FE0F}")); // no keycap mark
    }

    #[test]
    fn emoji_flags_are_recognized() {
        // Regional indicator pair (🇺🇸).
        assert!(is_emoji_flag("\u{1F1FA}\u{1F1F8}"));
        // Flag tag sequence (🏴 + tags).
        assert!(is_emoji_flag("\u{1F3F4}\u{E0067}\u{E0062}\u{E007F}"));
        assert!(!is_emoji_flag("a"));
    }

    #[test]
    fn clear_width_caches_empties_both() {
        let _ = visible_width("prime the cache");
        let _ = grapheme_width("x");
        clear_visible_width_cache();
        // Recompute after clearing — must still work.
        assert_eq!(visible_width("abc"), 3);
    }

    #[test]
    fn replace_tabs_width_zero_removes_tabs() {
        assert_eq!(replace_tabs("a\tb", 0), "ab");
        assert_eq!(replace_tabs("a\tb", 4), "a    b");
    }

    #[test]
    fn normalize_decomposes_thai_lao_am_vowels() {
        assert_eq!(normalize_terminal_output("plain"), "plain");
        // Thai AM (U+0E33) → NIKHAHIT + SARA AA.
        assert_eq!(normalize_terminal_output("\u{0e33}"), "\u{0e4d}\u{0e32}");
        // Lao AM (U+0EB3) decomposes too.
        assert_eq!(
            normalize_terminal_output("x\u{0eb3}y"),
            "x\u{0ecd}\u{0eb2}y"
        );
        // Tabs are replaced with 3 spaces along the way.
        assert_eq!(normalize_terminal_output("a\tb"), "a   b");
    }

    // ─── extract_ansi_code ──────────────────────────────────────────────

    #[test]
    fn extract_ansi_code_osc_termination_variants() {
        // BEL before ST → BEL wins (the earlier terminator).
        let r = extract_ansi_code("\x1b]0;t\x07rest\x1b\\", 0).unwrap();
        assert_eq!(r.code, "\x1b]0;t\x07");
        assert_eq!(r.kind, AnsiKind::Osc);
        // BEL only.
        let r = extract_ansi_code("\x1b]0;t\x07", 0).unwrap();
        assert_eq!(r.code, "\x1b]0;t\x07");
        // ST only.
        let r = extract_ansi_code("\x1b]0;t\x1b\\", 0).unwrap();
        assert_eq!(r.code, "\x1b]0;t\x1b\\");
        // Unterminated → None.
        assert!(extract_ansi_code("\x1b]0;never-closed", 0).is_none());
    }

    #[test]
    fn extract_ansi_code_apc_termination_variants() {
        let r = extract_ansi_code("\x1b_Ga=b\x07\x1b\\", 0).unwrap();
        assert_eq!(r.code, "\x1b_Ga=b\x07");
        assert_eq!(r.kind, AnsiKind::Apc);
        // BEL only.
        let r = extract_ansi_code("\x1b_Ga=b\x07", 0).unwrap();
        assert_eq!(r.code, "\x1b_Ga=b\x07");
        let r = extract_ansi_code("\x1b_Ga=b\x1b\\", 0).unwrap();
        assert_eq!(r.code, "\x1b_Ga=b\x1b\\");
        assert!(extract_ansi_code("\x1b_Gopen", 0).is_none());
    }

    #[test]
    fn extract_ansi_code_csi_ss3_meta_and_edge_cases() {
        // Lone ESC at the end → None.
        assert!(extract_ansi_code("\x1b", 0).is_none());
        // Positioning past the end / on a non-ESC byte → None.
        assert!(extract_ansi_code("ab", 5).is_none());
        assert!(extract_ansi_code("ab", 0).is_none());
        // Non-SGR CSI is classified Csi.
        let r = extract_ansi_code("\x1b[A", 0).unwrap();
        assert_eq!(r.kind, AnsiKind::Csi);
        // Invalid CSI parameter byte → unterminated → None.
        assert!(extract_ansi_code("\x1b[\x01A", 0).is_none());
        // SS3: ESC O + final char.
        let r = extract_ansi_code("\x1bOA", 0).unwrap();
        assert_eq!(r.length, 3);
        assert_eq!(r.kind, AnsiKind::Csi);
        // Meta/Alt: ESC + single char (multi-byte char stays whole).
        let r = extract_ansi_code("\x1ba", 0).unwrap();
        assert_eq!(r.code, "\x1ba");
        assert_eq!(r.kind, AnsiKind::Other);
        let r = extract_ansi_code("\x1bé", 0).unwrap();
        assert_eq!(r.code, "\x1bé");
    }

    // ─── AnsiCodeTracker ────────────────────────────────────────────────

    #[test]
    fn tracker_default_push_pop_and_reset() {
        let mut t = AnsiCodeTracker::default();
        t.feed("\x1b[1m");
        assert!(t.get_state().bold);
        t.push_state();
        t.feed("\x1b[4m");
        assert!(t.get_state().underline);
        t.pop_state();
        assert!(t.get_state().bold);
        assert!(!t.get_state().underline);
        // Pop with an empty stack is a no-op.
        t.pop_state();
        assert!(t.get_state().bold);
        t.reset();
        assert!(!t.get_state().bold);
    }

    #[test]
    fn feed_osc8_sets_and_clears_link() {
        let mut t = AnsiCodeTracker::new();
        t.feed("\x1b]8;;https://example.com\x07");
        assert_eq!(t.get_state().link.as_deref(), Some("https://example.com"));
        t.feed("\x1b]8;;\x07");
        assert_eq!(t.get_state().link.as_deref(), Some(""));
    }

    #[test]
    fn feed_sgr_attribute_switches() {
        let mut t = AnsiCodeTracker::new();
        t.feed("\x1b[5m");
        assert!(t.get_state().blink);
        t.feed("\x1b[7m");
        assert!(t.get_state().inverse);
        t.feed("\x1b[8m");
        assert!(t.get_state().hidden);
        t.feed("\x1b[9m");
        assert!(t.get_state().strikethrough);
        // Individual off-switches.
        t.feed("\x1b[23m");
        assert!(!t.get_state().italic);
        t.feed("\x1b[24m");
        assert!(!t.get_state().underline);
        t.feed("\x1b[25m");
        assert!(!t.get_state().blink);
        t.feed("\x1b[27m");
        assert!(!t.get_state().inverse);
        t.feed("\x1b[28m");
        assert!(!t.get_state().hidden);
        t.feed("\x1b[29m");
        assert!(!t.get_state().strikethrough);
        // 22 clears bold+dim; empty param defaults to 0 (full reset).
        t.feed("\x1b[1;2m");
        assert!(t.get_state().bold && t.get_state().dim);
        t.feed("\x1b[22m");
        assert!(!t.get_state().bold && !t.get_state().dim);
        t.feed("\x1b[3m");
        t.feed("\x1b[;m");
        assert!(!t.get_state().italic);
        // An explicit 0 param resets too.
        t.feed("\x1b[1m");
        t.feed("\x1b[0;4m");
        assert!(t.get_state().underline);
        assert!(!t.get_state().bold);
    }

    #[test]
    fn feed_sgr_color_forms() {
        let mut t = AnsiCodeTracker::new();
        // Standard + bright fg/bg (catch-all arm, the TS switch default).
        t.feed("\x1b[31m");
        assert_eq!(t.get_state().fg.as_deref(), Some("31"));
        t.feed("\x1b[91m");
        assert_eq!(t.get_state().fg.as_deref(), Some("91"));
        t.feed("\x1b[41m");
        assert_eq!(t.get_state().bg.as_deref(), Some("41"));
        t.feed("\x1b[101m");
        assert_eq!(t.get_state().bg.as_deref(), Some("101"));
        // 256-color and truecolor fg.
        t.feed("\x1b[38;5;45m");
        assert_eq!(t.get_state().fg.as_deref(), Some("38;5;45"));
        t.feed("\x1b[38;2;1;2;3m");
        assert_eq!(t.get_state().fg.as_deref(), Some("38;2;1;2;3"));
        // 256-color and truecolor bg.
        t.feed("\x1b[48;5;200m");
        assert_eq!(t.get_state().bg.as_deref(), Some("48;5;200"));
        t.feed("\x1b[48;2;4;5;6m");
        assert_eq!(t.get_state().bg.as_deref(), Some("48;2;4;5;6"));
        // Default fg/bg.
        t.feed("\x1b[39m");
        assert!(t.get_state().fg.is_none());
        t.feed("\x1b[49m");
        assert!(t.get_state().bg.is_none());
        // Malformed params parse to u32::MAX and hit no arm.
        t.feed("\x1b[38;5m"); // incomplete extended form — ignored
        assert!(t.get_state().fg.is_none());
    }

    #[test]
    fn get_ansi_code_reconstructs_active_state() {
        let mut t = AnsiCodeTracker::new();
        t.feed("\x1b[1;2;3;4;5;7;8;9m");
        t.feed("\x1b[31m");
        t.feed("\x1b[41m");
        let code = t.get_ansi_code();
        assert!(code.starts_with("\x1b["));
        assert!(code.ends_with('m'));
        for part in ["1", "2", "3", "4", "5", "7", "8", "9", "31", "41"] {
            assert!(code.contains(part));
        }
        // Empty state → hard reset.
        let t2 = AnsiCodeTracker::new();
        assert_eq!(t2.get_ansi_code(), "\x1b[0m");
    }

    #[test]
    fn osc8_link_open_and_close_sequences() {
        let mut t = AnsiCodeTracker::new();
        assert_eq!(t.get_osc8_link(), "");
        t.feed("\x1b]8;;https://x.example\x07");
        assert_eq!(
            t.get_osc8_link(),
            "\x1b]8;id=future_tui;https://x.example\x07"
        );
        assert_eq!(t.get_osc8_close(), "\x1b]8;;\x07");
    }

    // ─── wrap paths ─────────────────────────────────────────────────────

    #[test]
    fn wrap_grapheme_hard_breaks_when_only_space_is_line_start_or_ansi() {
        // The only space sits right after an ANSI run — not a usable word
        // boundary (the "line up to the space" would be all escape codes),
        // so the wrapper hard-breaks at width instead.
        let lines = wrap_text_with_ansi("\x1b[31m abcdef", 3);
        assert!(lines.len() > 1);
        assert_eq!(strip_ansi_codes(&lines[0]), " ab");
        // Empty input yields one empty line (grapheme path).
        assert_eq!(wrap_text_with_ansi("", 10), vec![String::new()]);
    }

    #[test]
    fn wrap_with_active_link_closes_osc8_at_line_end() {
        let text = format!(
            "\x1b]8;;https://example.com\x07{}\x1b]8;;\x07",
            "word ".repeat(10)
        );
        let lines = wrap_text_with_ansi(&text, 12);
        assert!(lines.len() > 1);
        // Each wrapped line ends with the OSC8 close + reset while the link
        // is active.
        assert!(lines[0].contains("\x1b]8;;\x07"));
        // …and re-opens the link on the continuation line.
        assert!(lines[1].contains("\x1b]8;id=future_tui;https://example.com\x07"));
    }

    #[test]
    fn is_all_ansi_directly() {
        assert!(is_all_ansi("\x1b[1m\x1b[0m"));
        assert!(!is_all_ansi("\x1b[1mx"));
        assert!(is_all_ansi(""));
    }

    // ─── slice_by_column / extract_segments ─────────────────────────────

    #[test]
    fn slice_by_column_skips_and_preserves_ansi() {
        // ANSI before `start` is skipped; ANSI inside the slice is kept.
        let s = "\x1b[31mab\x1b[0mcd";
        assert_eq!(slice_by_column(s, 1, Some(3)), "b\x1b[0mc");
        // Grapheme wider than the remaining span ends the slice.
        assert_eq!(slice_by_column("a中b", 0, Some(2)), "a");
        // No end → everything from start.
        assert_eq!(slice_by_column("\x1b[31mabc", 1, None), "bc");
    }

    #[test]
    fn extract_segments_carries_ansi_into_after_region() {
        // ANSI arriving after the after-region started is preserved in it.
        let seg = extract_segments("abx\x1b[31mcde", 1, 2, 2, false);
        assert_eq!(seg.before, "a");
        assert!(seg.after.contains("\x1b[31m"));
        assert!(strip_ansi_codes(&seg.after).starts_with("xc"));
        // after_len 0: only the before segment is gathered (ANSI before the
        // cut travels with `before`).
        let seg = extract_segments("he\x1b[31mllo", 3, 3, 0, false);
        assert_eq!(seg.before, "he\x1b[31ml");
        assert_eq!(seg.after, "");
        // strict_after clips a grapheme that would overrun after_end.
        let seg = extract_segments("ab中cd", 1, 2, 1, true);
        assert_eq!(strip_ansi_codes(&seg.after), "");
    }
}
