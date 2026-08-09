//! Keyboard input handling — 1:1 port of `tui/src/keys.ts`.
//!
//! Supports: legacy terminal sequences, Kitty CSI-u protocol, xterm
//! modifyOtherKeys. `parse_key` maps raw terminal input to a normalized key
//! identifier string (e.g. `"ctrl+c"`, `"shift+tab"`, `"up"`, `"enter"`, `"a"`).
//!
//! The Kitty protocol global state is an `AtomicBool` so `terminal.rs` (reader
//! thread) and callers on the main thread agree on the active mode.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use regex::Regex;

// ─── Global Kitty Protocol State ─────────────────────────────────────────

static KITTY_PROTOCOL_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn set_kitty_protocol_active(active: bool) {
    KITTY_PROTOCOL_ACTIVE.store(active, Ordering::SeqCst);
}

pub fn is_kitty_protocol_active() -> bool {
    KITTY_PROTOCOL_ACTIVE.load(Ordering::SeqCst)
}

// ─── Constants ───────────────────────────────────────────────────────────

const SYMBOL_KEYS: [char; 31] = [
    '`', '-', '=', '[', ']', '\\', ';', '\'', ',', '.', '/', '!', '@', '#', '$', '%', '^', '&',
    '*', '(', ')', '_', '+', '|', '~', '{', '}', ':', '<', '>', '?',
];

const MOD_SHIFT: u32 = 1;
const MOD_ALT: u32 = 2;
const MOD_CTRL: u32 = 4;
const MOD_SUPER: u32 = 8;
const MOD_SUPPORTED: u32 = MOD_SHIFT | MOD_CTRL | MOD_ALT | MOD_SUPER;

const LOCK_MASK: u32 = 64 + 128;

const CP_ESCAPE: i64 = 27;
const CP_TAB: i64 = 9;
const CP_ENTER: i64 = 13;
const CP_SPACE: i64 = 32;
const CP_BACKSPACE: i64 = 127;
const CP_KP_ENTER: i64 = 57414;
/// Kitty codepoint for Delete — present in the TS `CODEPOINTS` table but not
/// referenced by any code path (delete matching uses `FUNCTIONAL_CODEPOINTS`).
#[allow(dead_code)]
const CP_DELETE: i64 = 57425;

const ARROW_UP: i64 = -1;
const ARROW_DOWN: i64 = -2;
const ARROW_RIGHT: i64 = -3;
const ARROW_LEFT: i64 = -4;

const FUNC_DELETE: i64 = -10;
const FUNC_INSERT: i64 = -11;
const FUNC_PAGE_UP: i64 = -12;
const FUNC_PAGE_DOWN: i64 = -13;
const FUNC_HOME: i64 = -14;
const FUNC_END: i64 = -15;

/// Kitty functional key codepoints → the normalized codepoints the rest of the
/// codebase uses (port of the `KITTY_FUNCTIONAL_KEY_EQUIVALENTS` map).
fn kitty_functional_equivalent(codepoint: i64) -> Option<i64> {
    Some(match codepoint {
        57399 => 48,
        57400 => 49,
        57401 => 50,
        57402 => 51,
        57403 => 52,
        57404 => 53,
        57405 => 54,
        57406 => 55,
        57407 => 56,
        57408 => 57,
        57409 => 46,
        57410 => 47,
        57411 => 42,
        57412 => 45,
        57413 => 43,
        57415 => 61,
        57416 => 44,
        57417 => ARROW_LEFT,
        57418 => ARROW_RIGHT,
        57419 => ARROW_UP,
        57420 => ARROW_DOWN,
        57421 => FUNC_PAGE_UP,
        57422 => FUNC_PAGE_DOWN,
        57423 => FUNC_HOME,
        57424 => FUNC_END,
        57425 => FUNC_INSERT,
        57426 => FUNC_DELETE,
        _ => return None,
    })
}

fn normalize_kitty_functional_codepoint(codepoint: i64) -> i64 {
    kitty_functional_equivalent(codepoint).unwrap_or(codepoint)
}

fn normalize_shifted_letter_identity_codepoint(codepoint: i64, modifier: u32) -> i64 {
    let effective_modifier = modifier & !LOCK_MASK;
    if (effective_modifier & MOD_SHIFT) != 0 && (65..=90).contains(&codepoint) {
        return codepoint + 32;
    }
    codepoint
}

// ─── Regexes (compiled once) ─────────────────────────────────────────────

fn csi_u_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\x1b\[(\d+)(?::(\d*))?(?::(\d+))?(?:;(\d+))?(?::(\d+))?u$").unwrap()
    })
}

fn arrow_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\x1b\[1;(\d+)(?::(\d+))?([ABCD])$").unwrap())
}

fn home_end_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\x1b\[(\d+);(\d+)(?::(\d+))?([HF])$").unwrap())
}

fn func_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\x1b\[(\d+)(?:;(\d+))?(?::(\d+))?~$").unwrap())
}

fn modify_other_keys_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\x1b\[27;(\d+);(\d+)~$").unwrap())
}

fn release_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r":3[u~ABCDHF]").unwrap())
}

fn repeat_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r":2[u~ABCDHF]").unwrap())
}

/// `String.fromCharCode` semantics for the narrow uses in keys.ts: wraps the
/// value to 16 bits (JS truncates with `& 0xFFFF`), which is what makes
/// astral codepoints fall outside SYMBOL_KEYS exactly like in JS.
fn js_from_char_code(code: i64) -> char {
    char::from_u32((code & 0xFFFF) as u32).unwrap_or('\0')
}

// ─── Legacy Sequence Mappings ────────────────────────────────────────────

/// Port of the `LEGACY_SEQUENCE_KEY_IDS` record.
fn legacy_sequence_key_id(data: &str) -> Option<&'static str> {
    Some(match data {
        "\x1bOA" => "up",
        "\x1bOB" => "down",
        "\x1bOC" => "right",
        "\x1bOD" => "left",
        "\x1b[A" => "up",
        "\x1b[B" => "down",
        "\x1b[C" => "right",
        "\x1b[D" => "left",
        "\x1bOH" => "home",
        "\x1bOF" => "end",
        "\x1b[E" => "clear",
        "\x1bOE" => "clear",
        "\x1bOe" => "ctrl+clear",
        "\x1b[e" => "shift+clear",
        "\x1b[2~" => "insert",
        "\x1b[2$" => "shift+insert",
        "\x1b[2^" => "ctrl+insert",
        "\x1b[3~" => "delete",
        "\x1b[3$" => "shift+delete",
        "\x1b[3^" => "ctrl+delete",
        "\x1b[[5~" => "pageUp",
        "\x1b[[6~" => "pageDown",
        "\x1b[a" => "shift+up",
        "\x1b[b" => "shift+down",
        "\x1b[c" => "shift+right",
        "\x1b[d" => "shift+left",
        "\x1bOa" => "ctrl+up",
        "\x1bOb" => "ctrl+down",
        "\x1bOc" => "ctrl+right",
        "\x1bOd" => "ctrl+left",
        "\x1b[5$" => "shift+pageUp",
        "\x1b[6$" => "shift+pageDown",
        "\x1b[7$" => "shift+home",
        "\x1b[8$" => "shift+end",
        "\x1b[5^" => "ctrl+pageUp",
        "\x1b[6^" => "ctrl+pageDown",
        "\x1b[7^" => "ctrl+home",
        "\x1b[8^" => "ctrl+end",
        "\x1bOP" => "f1",
        "\x1bOQ" => "f2",
        "\x1bOR" => "f3",
        "\x1bOS" => "f4",
        "\x1b[11~" => "f1",
        "\x1b[12~" => "f2",
        "\x1b[13~" => "f3",
        "\x1b[14~" => "f4",
        "\x1b[[A" => "f1",
        "\x1b[[B" => "f2",
        "\x1b[[C" => "f3",
        "\x1b[[D" => "f4",
        "\x1b[[E" => "f5",
        "\x1b[15~" => "f5",
        "\x1b[17~" => "f6",
        "\x1b[18~" => "f7",
        "\x1b[19~" => "f8",
        "\x1b[20~" => "f9",
        "\x1b[21~" => "f10",
        "\x1b[23~" => "f11",
        "\x1b[24~" => "f12",
        "\x1bb" => "alt+left",
        "\x1bf" => "alt+right",
        "\x1bp" => "alt+up",
        "\x1bn" => "alt+down",
        _ => return None,
    })
}

// ─── Kitty CSI-u Parsing ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEventType {
    Press,
    Repeat,
    Release,
}

#[derive(Debug, Clone, Copy)]
struct ParsedKittySequence {
    codepoint: i64,
    /// Set by the CSI-u parser; consumed by `decode_kitty_printable` via its own
    /// regex, never read here (matches the TS code, which also drops it).
    #[allow(dead_code)]
    shifted_key: Option<u32>,
    base_layout_key: Option<u32>,
    modifier: u32,
    /// Parsed but unused by the match/format path (release/repeat detection is
    /// regex-based, `is_key_release`/`is_key_repeat`), mirroring TS.
    #[allow(dead_code)]
    event_type: KeyEventType,
}

#[derive(Debug, Clone, Copy)]
struct ParsedModifyOtherKeysSequence {
    codepoint: i64,
    modifier: u32,
}

fn parse_event_type(event_type_str: Option<&str>) -> KeyEventType {
    let Some(s) = event_type_str else {
        return KeyEventType::Press;
    };
    match s.parse::<u32>() {
        Ok(2) => KeyEventType::Repeat,
        Ok(3) => KeyEventType::Release,
        _ => KeyEventType::Press,
    }
}

/// JS `parseInt(s, 10)` semantics for the captures: leading digits are parsed,
/// garbage → the whole parse fails in Rust whereas JS yields NaN. Both produce
/// the same observable result (unrecognized sequence), so a plain parse is used.
fn parse_js_int(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    s.parse::<u64>().ok()
}

fn parse_kitty_sequence(data: &str) -> Option<ParsedKittySequence> {
    // CSI u: \x1b[<codepoint>;<mod>u or \x1b[<codepoint>:<shifted>:<base>;<mod>:<event>u
    if let Some(caps) = csi_u_re().captures(data) {
        let codepoint = parse_js_int(caps.get(1)?.as_str())? as i64;
        let shifted_key = caps
            .get(2)
            .filter(|m| !m.as_str().is_empty())
            .and_then(|m| parse_js_int(m.as_str()))
            .map(|v| v as u32);
        let base_layout_key = caps
            .get(3)
            .and_then(|m| parse_js_int(m.as_str()))
            .map(|v| v as u32);
        let mod_value = caps
            .get(4)
            .and_then(|m| parse_js_int(m.as_str()))
            .unwrap_or(1);
        let event_type = parse_event_type(caps.get(5).map(|m| m.as_str()));
        return Some(ParsedKittySequence {
            codepoint,
            shifted_key,
            base_layout_key,
            modifier: mod_value as u32 - 1,
            event_type,
        });
    }

    // Arrow keys: \x1b[1;<mod>A/B/C/D or \x1b[1;<mod>:<event>A/B/C/D
    if let Some(caps) = arrow_re().captures(data) {
        let mod_value = parse_js_int(caps.get(1)?.as_str())?;
        let event_type = parse_event_type(caps.get(2).map(|m| m.as_str()));
        let arrow_codes: i64 = match caps.get(3)?.as_str() {
            "A" => ARROW_UP,
            "B" => ARROW_DOWN,
            "C" => ARROW_RIGHT,
            // The regex capture is restricted to [ABCD].
            _ => ARROW_LEFT,
        };
        return Some(ParsedKittySequence {
            codepoint: arrow_codes,
            shifted_key: None,
            base_layout_key: None,
            modifier: mod_value as u32 - 1,
            event_type,
        });
    }

    // Home/End: \x1b[<codepoint>;<mod>H/F or \x1b[<codepoint>;<mod>:<event>H/F
    if let Some(caps) = home_end_re().captures(data) {
        let mod_value = parse_js_int(caps.get(2)?.as_str())?;
        let event_type = parse_event_type(caps.get(3).map(|m| m.as_str()));
        let normalized_codepoint = match caps.get(4)?.as_str() {
            "H" => FUNC_HOME,
            // The regex capture is restricted to [HF].
            _ => FUNC_END,
        };
        return Some(ParsedKittySequence {
            codepoint: normalized_codepoint,
            shifted_key: None,
            base_layout_key: None,
            modifier: mod_value as u32 - 1,
            event_type,
        });
    }

    // Functional keys with CSI ~: \x1b[<num>;<mod>~ or \x1b[<num>;<mod>:<event>~
    if let Some(caps) = func_re().captures(data) {
        let key_num = parse_js_int(caps.get(1)?.as_str())? as i64;
        let mod_value = caps
            .get(2)
            .and_then(|m| parse_js_int(m.as_str()))
            .unwrap_or(1);
        let event_type = parse_event_type(caps.get(3).map(|m| m.as_str()));
        let codepoint = match key_num {
            2 => FUNC_INSERT,
            3 => FUNC_DELETE,
            5 => FUNC_PAGE_UP,
            6 => FUNC_PAGE_DOWN,
            7 => FUNC_HOME,
            8 => FUNC_END,
            _ => return None,
        };
        return Some(ParsedKittySequence {
            codepoint,
            shifted_key: None,
            base_layout_key: None,
            modifier: mod_value as u32 - 1,
            event_type,
        });
    }

    None
}

// ─── modifyOtherKeys Parsing ─────────────────────────────────────────────

fn parse_modify_other_keys_sequence(data: &str) -> Option<ParsedModifyOtherKeysSequence> {
    let caps = modify_other_keys_re().captures(data)?;
    let mod_value = parse_js_int(caps.get(1)?.as_str())?;
    let codepoint = parse_js_int(caps.get(2)?.as_str())? as i64;
    Some(ParsedModifyOtherKeysSequence {
        codepoint,
        modifier: mod_value as u32 - 1,
    })
}

// ─── Event Type Detection ────────────────────────────────────────────────

pub fn is_key_release(data: &str) -> bool {
    if data.contains("\x1b[200~") {
        return false;
    }
    release_re().is_match(data)
}

pub fn is_key_repeat(data: &str) -> bool {
    if data.contains("\x1b[200~") {
        return false;
    }
    repeat_re().is_match(data)
}

// ─── Key Name Formatting ─────────────────────────────────────────────────

/// Port of `isWindowsTerminalSession()` — reads the same env vars as TS.
pub fn is_windows_terminal_session() -> bool {
    let wt_session = std::env::var("WT_SESSION").is_ok();
    let ssh_connection = std::env::var("SSH_CONNECTION").is_ok();
    let ssh_client = std::env::var("SSH_CLIENT").is_ok();
    let ssh_tty = std::env::var("SSH_TTY").is_ok();
    wt_session && !ssh_connection && !ssh_client && !ssh_tty
}

fn format_key_name_with_modifiers(key_name: &str, modifier: u32) -> Option<String> {
    let effective_mod = modifier & !LOCK_MASK;
    if (effective_mod & !MOD_SUPPORTED) != 0 {
        return None;
    }
    let mut mods: Vec<&str> = Vec::new();
    if effective_mod & MOD_SHIFT != 0 {
        mods.push("shift");
    }
    if effective_mod & MOD_CTRL != 0 {
        mods.push("ctrl");
    }
    if effective_mod & MOD_ALT != 0 {
        mods.push("alt");
    }
    if effective_mod & MOD_SUPER != 0 {
        mods.push("super");
    }
    if mods.is_empty() {
        Some(key_name.to_string())
    } else {
        Some(format!("{}+{key_name}", mods.join("+")))
    }
}

fn format_parsed_key(
    codepoint: i64,
    modifier: u32,
    base_layout_key: Option<u32>,
) -> Option<String> {
    let normalized_codepoint = normalize_kitty_functional_codepoint(codepoint);
    let identity_codepoint =
        normalize_shifted_letter_identity_codepoint(normalized_codepoint, modifier);

    let is_latin_letter = (97..=122).contains(&identity_codepoint);
    let is_digit = (48..=57).contains(&identity_codepoint);
    let is_known_symbol = SYMBOL_KEYS.contains(&js_from_char_code(identity_codepoint));
    let effective_codepoint = if is_latin_letter || is_digit || is_known_symbol {
        identity_codepoint
    } else {
        base_layout_key
            .map(|b| b as i64)
            .unwrap_or(identity_codepoint)
    };

    let key_name: Option<String> = if effective_codepoint == CP_ESCAPE {
        Some("escape".to_string())
    } else if effective_codepoint == CP_TAB {
        Some("tab".to_string())
    } else if effective_codepoint == CP_ENTER || effective_codepoint == CP_KP_ENTER {
        Some("enter".to_string())
    } else if effective_codepoint == CP_SPACE {
        Some("space".to_string())
    } else if effective_codepoint == CP_BACKSPACE {
        Some("backspace".to_string())
    } else if effective_codepoint == FUNC_DELETE {
        Some("delete".to_string())
    } else if effective_codepoint == FUNC_INSERT {
        Some("insert".to_string())
    } else if effective_codepoint == FUNC_HOME {
        Some("home".to_string())
    } else if effective_codepoint == FUNC_END {
        Some("end".to_string())
    } else if effective_codepoint == FUNC_PAGE_UP {
        Some("pageUp".to_string())
    } else if effective_codepoint == FUNC_PAGE_DOWN {
        Some("pageDown".to_string())
    } else if effective_codepoint == ARROW_UP {
        Some("up".to_string())
    } else if effective_codepoint == ARROW_DOWN {
        Some("down".to_string())
    } else if effective_codepoint == ARROW_LEFT {
        Some("left".to_string())
    } else if effective_codepoint == ARROW_RIGHT {
        Some("right".to_string())
    } else if (48..=57).contains(&effective_codepoint) {
        char::from_u32(effective_codepoint as u32).map(|c| c.to_string())
    } else if (97..=122).contains(&effective_codepoint) {
        char::from_u32(effective_codepoint as u32).map(|c| c.to_string())
    } else if SYMBOL_KEYS.contains(&char::from_u32(effective_codepoint as u32).unwrap_or('\0')) {
        char::from_u32(effective_codepoint as u32).map(|c| c.to_string())
    } else {
        None
    };
    let key_name = key_name?;
    format_key_name_with_modifiers(&key_name, modifier)
}

// ─── Main Parse Function ─────────────────────────────────────────────────

/// Parse raw terminal input into a normalized key identifier string.
/// Returns `None` for unrecognized input.
pub fn parse_key(data: &str) -> Option<String> {
    // Kitty CSI-u protocol
    if let Some(kitty) = parse_kitty_sequence(data) {
        return format_parsed_key(kitty.codepoint, kitty.modifier, kitty.base_layout_key);
    }

    // xterm modifyOtherKeys
    if let Some(mok) = parse_modify_other_keys_sequence(data) {
        return format_parsed_key(mok.codepoint, mok.modifier, None);
    }

    // Mode-aware legacy sequences
    if is_kitty_protocol_active() && (data == "\x1b\r" || data == "\n") {
        return Some("shift+enter".to_string());
    }

    if let Some(legacy) = legacy_sequence_key_id(data) {
        return Some(legacy.to_string());
    }

    // Individual legacy sequences
    if data == "\x1b" {
        return Some("escape".to_string());
    }
    if data == "\x1c" {
        return Some("ctrl+\\".to_string());
    }
    if data == "\x1d" {
        return Some("ctrl+]".to_string());
    }
    if data == "\x1f" {
        return Some("ctrl+-".to_string());
    }
    if data == "\x1b\x1b" {
        return Some("ctrl+alt+[".to_string());
    }
    if data == "\x1b\x1c" {
        return Some("ctrl+alt+\\".to_string());
    }
    if data == "\x1b\x1d" {
        return Some("ctrl+alt+]".to_string());
    }
    if data == "\x1b\x1f" {
        return Some("ctrl+alt+-".to_string());
    }
    if data == "\t" {
        return Some("tab".to_string());
    }
    if data == "\r" || data == "\x1bOM" {
        return Some("enter".to_string());
    }
    if !is_kitty_protocol_active() && data == "\n" {
        return Some("ctrl+j".to_string());
    }
    if data == "\x00" {
        return Some("ctrl+space".to_string());
    }
    if data == " " {
        return Some("space".to_string());
    }
    if data == "\x7f" {
        return Some("backspace".to_string());
    }
    if data == "\x08" {
        return Some(if is_windows_terminal_session() {
            "ctrl+backspace".to_string()
        } else {
            "backspace".to_string()
        });
    }
    if data == "\x1b[Z" {
        return Some("shift+tab".to_string());
    }
    if !is_kitty_protocol_active() && data == "\x1b\r" {
        return Some("alt+enter".to_string());
    }
    if data == "\x1b\x7f" || data == "\x1b\x08" {
        return Some("alt+backspace".to_string());
    }
    if !is_kitty_protocol_active() && data == "\x1b " {
        return Some("alt+space".to_string());
    }

    // Legacy alt+letter/digit (ESC followed by key)
    if !is_kitty_protocol_active() && data.len() == 2 && data.starts_with('\x1b') {
        let code = data.as_bytes()[1];
        if (1..=26).contains(&code) {
            return Some(format!("ctrl+alt+{}", (code + 96) as char));
        }
        if (97..=122).contains(&code) || (48..=57).contains(&code) {
            return Some(format!("alt+{}", code as char));
        }
    }

    // Raw Ctrl+letter / printable char. Every other single-byte input is
    // claimed above (control pictures, space, \x7f, ...); a len-1 &str is a
    // single byte ≤ 0x7f by UTF-8 validity, so what remains is printable.
    if data.len() == 1 {
        let code = data.as_bytes()[0];
        if (1..=26).contains(&code) {
            return Some(format!("ctrl+{}", (code + 96) as char));
        }
        return Some(data.to_string());
    }

    None
}

// ─── Printable Key Decoding ──────────────────────────────────────────────

/// Decode a Kitty CSI-u sequence into a printable character.
pub fn decode_kitty_printable(data: &str) -> Option<String> {
    let caps = csi_u_re().captures(data)?;
    let codepoint = parse_js_int(caps.get(1)?.as_str())? as i64;
    let shifted_key = caps
        .get(2)
        .filter(|m| !m.as_str().is_empty())
        .and_then(|m| parse_js_int(m.as_str()))
        .map(|v| v as i64);
    let mod_value = caps
        .get(4)
        .and_then(|m| parse_js_int(m.as_str()))
        .unwrap_or(1);
    let modifier = mod_value as u32 - 1;

    // Anything beyond shift/lock (alt, ctrl, super, ...) is not printable.
    if (modifier & !(MOD_SHIFT | LOCK_MASK)) != 0 {
        return None;
    }

    let mut effective_codepoint = codepoint;
    if (modifier & MOD_SHIFT) != 0 {
        if let Some(sk) = shifted_key {
            effective_codepoint = sk;
        }
    }
    effective_codepoint = normalize_kitty_functional_codepoint(effective_codepoint);
    if effective_codepoint < 32 {
        return None;
    }
    char::from_u32(effective_codepoint as u32).map(|c| c.to_string())
}

fn decode_modify_other_keys_printable(data: &str) -> Option<String> {
    let parsed = parse_modify_other_keys_sequence(data)?;
    let modifier = parsed.modifier & !LOCK_MASK;
    if (modifier & !MOD_SHIFT) != 0 {
        return None;
    }
    if parsed.codepoint < 32 {
        return None;
    }
    char::from_u32(parsed.codepoint as u32).map(|c| c.to_string())
}

pub fn decode_printable_key(data: &str) -> Option<String> {
    decode_kitty_printable(data).or_else(|| decode_modify_other_keys_printable(data))
}

// ─── Key ID Helpers ──────────────────────────────────────────────────────

/// Key identifier constants matching the TS `Key` helper object.
pub mod key {
    // Named keys
    pub const ESCAPE: &str = "escape";
    pub const TAB: &str = "tab";
    pub const ENTER: &str = "enter";
    pub const SPACE: &str = "space";
    pub const BACKSPACE: &str = "backspace";
    pub const DELETE: &str = "delete";
    pub const INSERT: &str = "insert";
    pub const HOME: &str = "home";
    pub const END: &str = "end";
    pub const PAGE_UP: &str = "pageUp";
    pub const PAGE_DOWN: &str = "pageDown";
    pub const UP: &str = "up";
    pub const DOWN: &str = "down";
    pub const LEFT: &str = "left";
    pub const RIGHT: &str = "right";
    pub const CLEAR: &str = "clear";

    // Function keys
    pub const F1: &str = "f1";
    pub const F2: &str = "f2";
    pub const F3: &str = "f3";
    pub const F4: &str = "f4";
    pub const F5: &str = "f5";
    pub const F6: &str = "f6";
    pub const F7: &str = "f7";
    pub const F8: &str = "f8";
    pub const F9: &str = "f9";
    pub const F10: &str = "f10";
    pub const F11: &str = "f11";
    pub const F12: &str = "f12";

    // Ctrl shortcuts
    pub const CTRL_A: &str = "ctrl+a";
    pub const CTRL_B: &str = "ctrl+b";
    pub const CTRL_C: &str = "ctrl+c";
    pub const CTRL_D: &str = "ctrl+d";
    pub const CTRL_E: &str = "ctrl+e";
    pub const CTRL_F: &str = "ctrl+f";
    pub const CTRL_G: &str = "ctrl+g";
    pub const CTRL_H: &str = "ctrl+h";
    pub const CTRL_I: &str = "ctrl+i";
    pub const CTRL_J: &str = "ctrl+j";
    pub const CTRL_K: &str = "ctrl+k";
    pub const CTRL_L: &str = "ctrl+l";
    pub const CTRL_M: &str = "ctrl+m";
    pub const CTRL_N: &str = "ctrl+n";
    pub const CTRL_O: &str = "ctrl+o";
    pub const CTRL_P: &str = "ctrl+p";
    pub const CTRL_Q: &str = "ctrl+q";
    pub const CTRL_R: &str = "ctrl+r";
    pub const CTRL_S: &str = "ctrl+s";
    pub const CTRL_T: &str = "ctrl+t";
    pub const CTRL_U: &str = "ctrl+u";
    pub const CTRL_V: &str = "ctrl+v";
    pub const CTRL_W: &str = "ctrl+w";
    pub const CTRL_X: &str = "ctrl+x";
    pub const CTRL_Y: &str = "ctrl+y";
    pub const CTRL_Z: &str = "ctrl+z";

    // Ctrl + named keys
    pub const CTRL_UP: &str = "ctrl+up";
    pub const CTRL_DOWN: &str = "ctrl+down";
    pub const CTRL_LEFT: &str = "ctrl+left";
    pub const CTRL_RIGHT: &str = "ctrl+right";
    pub const CTRL_HOME: &str = "ctrl+home";
    pub const CTRL_END: &str = "ctrl+end";
    pub const CTRL_PAGE_UP: &str = "ctrl+pageUp";
    pub const CTRL_PAGE_DOWN: &str = "ctrl+pageDown";
    pub const CTRL_BACKSPACE: &str = "ctrl+backspace";
    pub const CTRL_DELETE: &str = "ctrl+delete";
    pub const CTRL_SPACE: &str = "ctrl+space";
    pub const CTRL_ENTER: &str = "ctrl+enter";

    // Shift modifiers
    pub const SHIFT_TAB: &str = "shift+tab";
    pub const SHIFT_ENTER: &str = "shift+enter";
    pub const SHIFT_UP: &str = "shift+up";
    pub const SHIFT_DOWN: &str = "shift+down";
    pub const SHIFT_LEFT: &str = "shift+left";
    pub const SHIFT_RIGHT: &str = "shift+right";

    // Alt modifiers
    pub const ALT_UP: &str = "alt+up";
    pub const ALT_DOWN: &str = "alt+down";
    pub const ALT_LEFT: &str = "alt+left";
    pub const ALT_RIGHT: &str = "alt+right";
    pub const ALT_ENTER: &str = "alt+enter";
    pub const ALT_BACKSPACE: &str = "alt+backspace";
    pub const ALT_SPACE: &str = "alt+space";
}

/// Build a modified key string: `modified_key("ctrl", "c")` → `"ctrl+c"`.
pub fn modified_key(modifier: &str, key_name: &str) -> String {
    format!("{modifier}+{key_name}")
}

/// Build a ctrl+key string: `ctrl_key("c")` → `"ctrl+c"`.
pub fn ctrl_key(key_name: &str) -> String {
    format!("ctrl+{key_name}")
}

// ─── Key ID Parsing ───────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
struct ParsedKeyId {
    key: String,
    ctrl: bool,
    shift: bool,
    alt: bool,
    super_modifier: bool,
}

fn parse_key_id(key_id: &str) -> Option<ParsedKeyId> {
    let lower = key_id.to_lowercase();
    let parts: Vec<&str> = lower.split('+').collect();
    let key = *parts.last()?;
    if key.is_empty() {
        return None;
    }
    Some(ParsedKeyId {
        key: key.to_string(),
        ctrl: parts.contains(&"ctrl"),
        shift: parts.contains(&"shift"),
        alt: parts.contains(&"alt"),
        super_modifier: parts.contains(&"super"),
    })
}

// ─── Sequence Matching Helpers ─────────────────────────────────────────────

fn matches_kitty_sequence(data: &str, expected_codepoint: i64, expected_modifier: u32) -> bool {
    let Some(parsed) = parse_kitty_sequence(data) else {
        return false;
    };
    let actual_mod = parsed.modifier & !LOCK_MASK;
    let expected_mod = expected_modifier & !LOCK_MASK;
    if actual_mod != expected_mod {
        return false;
    }
    let normalized_codepoint = normalize_shifted_letter_identity_codepoint(
        normalize_kitty_functional_codepoint(parsed.codepoint),
        parsed.modifier,
    );
    let normalized_expected_codepoint = normalize_shifted_letter_identity_codepoint(
        normalize_kitty_functional_codepoint(expected_codepoint),
        expected_modifier,
    );
    if normalized_codepoint == normalized_expected_codepoint {
        return true;
    }
    if let Some(base) = parsed.base_layout_key {
        if base as i64 == expected_codepoint {
            let cp = normalized_codepoint;
            let is_latin_letter = (97..=122).contains(&cp);
            let is_known_symbol = SYMBOL_KEYS.contains(&js_from_char_code(cp));
            if !is_latin_letter && !is_known_symbol {
                return true;
            }
        }
    }
    false
}

fn matches_modify_other_keys(data: &str, expected_keycode: i64, expected_modifier: u32) -> bool {
    let Some(parsed) = parse_modify_other_keys_sequence(data) else {
        return false;
    };
    parsed.codepoint == expected_keycode && parsed.modifier == expected_modifier
}

fn matches_printable_modify_other_keys(
    data: &str,
    expected_keycode: i64,
    expected_modifier: u32,
) -> bool {
    if expected_modifier == 0 {
        return false;
    }
    let Some(parsed) = parse_modify_other_keys_sequence(data) else {
        return false;
    };
    if parsed.modifier != expected_modifier {
        return false;
    }
    normalize_shifted_letter_identity_codepoint(parsed.codepoint, parsed.modifier)
        == normalize_shifted_letter_identity_codepoint(expected_keycode, expected_modifier)
}

fn raw_ctrl_char(key: &str) -> Option<char> {
    let char = key.to_lowercase().chars().next()?;
    let code = char as u32;
    if (97..=122).contains(&code)
        || char == '['
        || char == '\\'
        || char == ']'
        || char == '_'
        || char == '-'
    {
        return char::from_u32(code & 0x1f);
    }
    None
}

fn matches_raw_backspace(data: &str, expected_modifier: u32) -> bool {
    if data == "\x7f" {
        return expected_modifier == 0;
    }
    if data != "\x08" {
        return false;
    }
    if is_windows_terminal_session() {
        expected_modifier == MOD_CTRL
    } else {
        expected_modifier == 0
    }
}

fn matches_legacy_sequence(data: &str, seqs: &[&str]) -> bool {
    seqs.contains(&data)
}

fn legacy_key_sequences(key: &str) -> &'static [&'static str] {
    match key {
        "insert" => &["\x1b[2~"],
        "delete" => &["\x1b[3~"],
        "clear" => &["\x1b[E", "\x1bOE"],
        "home" => &["\x1bOH", "\x1b[H"],
        "end" => &["\x1bOF", "\x1b[F"],
        "pageUp" => &["\x1b[5~", "\x1b[[5~"],
        "pageDown" => &["\x1b[6~", "\x1b[[6~"],
        "up" => &["\x1b[A", "\x1bOA"],
        "down" => &["\x1b[B", "\x1bOB"],
        "left" => &["\x1b[D", "\x1bOD"],
        "right" => &["\x1b[C", "\x1bOC"],
        "f1" => &["\x1bOP", "\x1b[11~", "\x1b[[A"],
        "f2" => &["\x1bOQ", "\x1b[12~", "\x1b[[B"],
        "f3" => &["\x1bOR", "\x1b[13~", "\x1b[[C"],
        "f4" => &["\x1bOS", "\x1b[14~", "\x1b[[D"],
        "f5" => &["\x1b[15~", "\x1b[[E"],
        "f6" => &["\x1b[17~"],
        "f7" => &["\x1b[18~"],
        "f8" => &["\x1b[19~"],
        "f9" => &["\x1b[20~"],
        "f10" => &["\x1b[21~"],
        "f11" => &["\x1b[23~"],
        "f12" => &["\x1b[24~"],
        _ => &[],
    }
}

fn legacy_shift_sequences(key: &str) -> &'static [&'static str] {
    match key {
        "up" => &["\x1b[a"],
        "down" => &["\x1b[b"],
        "right" => &["\x1b[c"],
        "left" => &["\x1b[d"],
        "clear" => &["\x1b[e"],
        "insert" => &["\x1b[2$"],
        "delete" => &["\x1b[3$"],
        "pageUp" => &["\x1b[5$"],
        "pageDown" => &["\x1b[6$"],
        "home" => &["\x1b[7$"],
        "end" => &["\x1b[8$"],
        _ => &[],
    }
}

fn legacy_ctrl_sequences(key: &str) -> &'static [&'static str] {
    match key {
        "up" => &["\x1bOa"],
        "down" => &["\x1bOb"],
        "right" => &["\x1bOc"],
        "left" => &["\x1bOd"],
        "clear" => &["\x1bOe"],
        "insert" => &["\x1b[2^"],
        "delete" => &["\x1b[3^"],
        "pageUp" => &["\x1b[5^"],
        "pageDown" => &["\x1b[6^"],
        "home" => &["\x1b[7^"],
        "end" => &["\x1b[8^"],
        _ => &[],
    }
}

fn matches_legacy_modifier_sequence(data: &str, key: &str, modifier: u32) -> bool {
    if modifier == MOD_SHIFT {
        return matches_legacy_sequence(data, legacy_shift_sequences(key));
    }
    if modifier == MOD_CTRL {
        return matches_legacy_sequence(data, legacy_ctrl_sequences(key));
    }
    false
}

fn is_digit_key(key: &str) -> bool {
    key.len() == 1 && key.as_bytes()[0].is_ascii_digit()
}

// ─── matches_key() — Match Input Against a Key Identifier ───────────────────

/// Match raw terminal input against a key identifier (`"escape"`, `"ctrl+c"`,
/// `"shift+tab"`, `"f1"`..`"f12"`, modifier combos, ...).
///
/// The alt+arrow branches and mode-dependent checks intentionally keep the TS
/// boolean/structure verbatim (a subexpression duplicates `data == "\x1bb"`
/// inside the kitty-off clause; the raw-ctrl check nests like the JS).
#[allow(clippy::nonminimal_bool, clippy::collapsible_if)]
pub fn matches_key(data: &str, key_id: &str) -> bool {
    let Some(parsed) = parse_key_id(key_id) else {
        return false;
    };

    let ParsedKeyId {
        key,
        ctrl,
        shift,
        alt,
        super_modifier,
    } = parsed;
    let mut modifier = 0u32;
    if shift {
        modifier |= MOD_SHIFT;
    }
    if alt {
        modifier |= MOD_ALT;
    }
    if ctrl {
        modifier |= MOD_CTRL;
    }
    if super_modifier {
        modifier |= MOD_SUPER;
    }

    let key = key.as_str();
    match key {
        "escape" | "esc" => {
            if modifier != 0 {
                return false;
            }
            return data == "\x1b"
                || matches_kitty_sequence(data, CP_ESCAPE, 0)
                || matches_modify_other_keys(data, CP_ESCAPE, 0);
        }
        "space" => {
            if !is_kitty_protocol_active() {
                if modifier == MOD_CTRL && data == "\x00" {
                    return true;
                }
                if modifier == MOD_ALT && data == "\x1b " {
                    return true;
                }
            }
            if modifier == 0 {
                return data == " "
                    || matches_kitty_sequence(data, CP_SPACE, 0)
                    || matches_modify_other_keys(data, CP_SPACE, 0);
            }
            return matches_kitty_sequence(data, CP_SPACE, modifier)
                || matches_modify_other_keys(data, CP_SPACE, modifier);
        }
        "tab" => {
            if modifier == MOD_SHIFT {
                return data == "\x1b[Z"
                    || matches_kitty_sequence(data, CP_TAB, MOD_SHIFT)
                    || matches_modify_other_keys(data, CP_TAB, MOD_SHIFT);
            }
            if modifier == 0 {
                return data == "\t" || matches_kitty_sequence(data, CP_TAB, 0);
            }
            return matches_kitty_sequence(data, CP_TAB, modifier)
                || matches_modify_other_keys(data, CP_TAB, modifier);
        }
        "enter" | "return" => {
            if modifier == MOD_SHIFT {
                if matches_kitty_sequence(data, CP_ENTER, MOD_SHIFT)
                    || matches_kitty_sequence(data, CP_KP_ENTER, MOD_SHIFT)
                {
                    return true;
                }
                if matches_modify_other_keys(data, CP_ENTER, MOD_SHIFT) {
                    return true;
                }
                if is_kitty_protocol_active() {
                    return data == "\x1b\r" || data == "\n";
                }
                return false;
            }
            if modifier == MOD_ALT {
                if matches_kitty_sequence(data, CP_ENTER, MOD_ALT)
                    || matches_kitty_sequence(data, CP_KP_ENTER, MOD_ALT)
                {
                    return true;
                }
                if matches_modify_other_keys(data, CP_ENTER, MOD_ALT) {
                    return true;
                }
                if !is_kitty_protocol_active() {
                    return data == "\x1b\r";
                }
                return false;
            }
            if modifier == 0 {
                return data == "\r"
                    || data == "\x1bOM"
                    || matches_kitty_sequence(data, CP_ENTER, 0)
                    || matches_kitty_sequence(data, CP_KP_ENTER, 0);
            }
            return matches_kitty_sequence(data, CP_ENTER, modifier)
                || matches_kitty_sequence(data, CP_KP_ENTER, modifier)
                || matches_modify_other_keys(data, CP_ENTER, modifier);
        }
        "backspace" => {
            if modifier == MOD_ALT {
                if data == "\x1b\x7f" || data == "\x1b\x08" {
                    return true;
                }
                return matches_kitty_sequence(data, CP_BACKSPACE, MOD_ALT)
                    || matches_modify_other_keys(data, CP_BACKSPACE, MOD_ALT);
            }
            if modifier == MOD_CTRL {
                if matches_raw_backspace(data, MOD_CTRL) {
                    return true;
                }
                return matches_kitty_sequence(data, CP_BACKSPACE, MOD_CTRL)
                    || matches_modify_other_keys(data, CP_BACKSPACE, MOD_CTRL);
            }
            if modifier == 0 {
                return matches_raw_backspace(data, 0)
                    || matches_kitty_sequence(data, CP_BACKSPACE, 0)
                    || matches_modify_other_keys(data, CP_BACKSPACE, 0);
            }
            return matches_kitty_sequence(data, CP_BACKSPACE, modifier)
                || matches_modify_other_keys(data, CP_BACKSPACE, modifier);
        }
        "insert" => {
            if modifier == 0 {
                return matches_legacy_sequence(data, legacy_key_sequences("insert"))
                    || matches_kitty_sequence(data, FUNC_INSERT, 0);
            }
            return matches_kitty_sequence(data, FUNC_INSERT, modifier);
        }
        "delete" => {
            if modifier == 0 {
                return matches_legacy_sequence(data, legacy_key_sequences("delete"))
                    || matches_kitty_sequence(data, FUNC_DELETE, 0);
            }
            return matches_kitty_sequence(data, FUNC_DELETE, modifier);
        }
        "clear" => {
            if modifier == 0 {
                return matches_legacy_sequence(data, legacy_key_sequences("clear"));
            }
            return matches_legacy_modifier_sequence(data, "clear", modifier);
        }
        "home" => {
            if modifier == 0 {
                return matches_legacy_sequence(data, legacy_key_sequences("home"))
                    || matches_kitty_sequence(data, FUNC_HOME, 0);
            }
            return matches_kitty_sequence(data, FUNC_HOME, modifier);
        }
        "end" => {
            if modifier == 0 {
                return matches_legacy_sequence(data, legacy_key_sequences("end"))
                    || matches_kitty_sequence(data, FUNC_END, 0);
            }
            return matches_kitty_sequence(data, FUNC_END, modifier);
        }
        "pageup" => {
            if modifier == 0 {
                return matches_legacy_sequence(data, legacy_key_sequences("pageUp"))
                    || matches_kitty_sequence(data, FUNC_PAGE_UP, 0);
            }
            return matches_kitty_sequence(data, FUNC_PAGE_UP, modifier);
        }
        "pagedown" => {
            if modifier == 0 {
                return matches_legacy_sequence(data, legacy_key_sequences("pageDown"))
                    || matches_kitty_sequence(data, FUNC_PAGE_DOWN, 0);
            }
            return matches_kitty_sequence(data, FUNC_PAGE_DOWN, modifier);
        }
        "up" => {
            if modifier == MOD_ALT {
                return data == "\x1bp" || matches_kitty_sequence(data, ARROW_UP, MOD_ALT);
            }
            if modifier == 0 {
                return matches_legacy_sequence(data, legacy_key_sequences("up"))
                    || matches_kitty_sequence(data, ARROW_UP, 0);
            }
            return matches_kitty_sequence(data, ARROW_UP, modifier);
        }
        "down" => {
            if modifier == MOD_ALT {
                return data == "\x1bn" || matches_kitty_sequence(data, ARROW_DOWN, MOD_ALT);
            }
            if modifier == 0 {
                return matches_legacy_sequence(data, legacy_key_sequences("down"))
                    || matches_kitty_sequence(data, ARROW_DOWN, 0);
            }
            return matches_kitty_sequence(data, ARROW_DOWN, modifier);
        }
        "left" => {
            if modifier == MOD_ALT {
                return data == "\x1bb"
                    || data == "\x1b[1;3D"
                    || (!is_kitty_protocol_active() && (data == "\x1bB" || data == "\x1bb"))
                    || matches_kitty_sequence(data, ARROW_LEFT, MOD_ALT);
            }
            if modifier == MOD_CTRL {
                return data == "\x1b[1;5D" || matches_kitty_sequence(data, ARROW_LEFT, MOD_CTRL);
            }
            if modifier == 0 {
                return matches_legacy_sequence(data, legacy_key_sequences("left"))
                    || matches_kitty_sequence(data, ARROW_LEFT, 0);
            }
            return matches_kitty_sequence(data, ARROW_LEFT, modifier);
        }
        "right" => {
            if modifier == MOD_ALT {
                return data == "\x1bf"
                    || data == "\x1b[1;3C"
                    || (!is_kitty_protocol_active() && (data == "\x1bF" || data == "\x1bf"))
                    || matches_kitty_sequence(data, ARROW_RIGHT, MOD_ALT);
            }
            if modifier == MOD_CTRL {
                return data == "\x1b[1;5C" || matches_kitty_sequence(data, ARROW_RIGHT, MOD_CTRL);
            }
            if modifier == 0 {
                return matches_legacy_sequence(data, legacy_key_sequences("right"))
                    || matches_kitty_sequence(data, ARROW_RIGHT, 0);
            }
            return matches_kitty_sequence(data, ARROW_RIGHT, modifier);
        }
        "f1" | "f2" | "f3" | "f4" | "f5" | "f6" | "f7" | "f8" | "f9" | "f10" | "f11" | "f12" => {
            if modifier != 0 {
                return false;
            }
            return matches_legacy_sequence(data, legacy_key_sequences(key));
        }
        _ => {}
    }

    // Handle single letter/digit keys and symbols
    if key.len() == 1 && is_single_key_char(key) {
        let codepoint = key.as_bytes()[0] as i64;
        let raw_ctrl = raw_ctrl_char(key);
        let is_letter = key.as_bytes()[0].is_ascii_lowercase();

        if modifier == MOD_CTRL + MOD_ALT && !is_kitty_protocol_active() {
            if let Some(rc) = raw_ctrl {
                if data == format!("\x1b{rc}") {
                    return true;
                }
            }
        }
        if modifier == MOD_ALT && !is_kitty_protocol_active() && (is_letter || is_digit_key(key)) {
            if data == format!("\x1b{key}") {
                return true;
            }
        }
        if modifier == MOD_CTRL {
            if let Some(rc) = raw_ctrl {
                if data == rc.to_string() {
                    return true;
                }
            }
            return matches_kitty_sequence(data, codepoint, MOD_CTRL)
                || matches_printable_modify_other_keys(data, codepoint, MOD_CTRL);
        }
        if modifier == MOD_SHIFT + MOD_CTRL {
            return matches_kitty_sequence(data, codepoint, MOD_SHIFT + MOD_CTRL)
                || matches_printable_modify_other_keys(data, codepoint, MOD_SHIFT + MOD_CTRL);
        }
        if modifier == MOD_SHIFT {
            if is_letter && data == key.to_uppercase() {
                return true;
            }
            return matches_kitty_sequence(data, codepoint, MOD_SHIFT)
                || matches_printable_modify_other_keys(data, codepoint, MOD_SHIFT);
        }
        if modifier != 0 {
            return matches_kitty_sequence(data, codepoint, modifier)
                || matches_printable_modify_other_keys(data, codepoint, modifier);
        }
        return data == key || matches_kitty_sequence(data, codepoint, 0);
    }

    false
}

fn is_single_key_char(key: &str) -> bool {
    if key.len() != 1 {
        return false;
    }
    let b = key.as_bytes()[0];
    b.is_ascii_lowercase() || b.is_ascii_digit() || SYMBOL_KEYS.contains(&(b as char))
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    // `KITTY_PROTOCOL_ACTIVE` is a process-wide global (the real app's reader
    // thread and main thread share it), so parallel test threads would race on
    // it. Every test that reads or writes kitty state holds this lock.
    static KITTY_STATE_LOCK: Mutex<()> = Mutex::new(());

    // Tests run with Kitty protocol OFF unless a test enables it explicitly.
    fn reset_kitty() -> parking_lot::MutexGuard<'static, ()> {
        let guard = KITTY_STATE_LOCK.lock();
        set_kitty_protocol_active(false);
        guard
    }

    /// Restore an environment variable to a saved value (None = absent).
    fn restore_env(key: &str, old: Option<std::ffi::OsString>) {
        match old {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    /// Save + clear the terminal-identity env vars; restore with the
    /// returned values via `restore_env`.
    fn clear_terminal_env() -> Vec<(&'static str, Option<std::ffi::OsString>)> {
        let keys = ["WT_SESSION", "SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"];
        let saved: Vec<(&str, Option<std::ffi::OsString>)> =
            keys.iter().map(|k| (*k, std::env::var_os(k))).collect();
        for k in keys {
            std::env::remove_var(k);
        }
        saved
    }

    #[test]
    fn restore_env_handles_set_and_unset() {
        let _guard = crate::test_env::lock();
        let old = std::env::var_os("FUTURE_TUI_KEYS_PROBE");
        restore_env("FUTURE_TUI_KEYS_PROBE", Some("x".into()));
        assert_eq!(std::env::var("FUTURE_TUI_KEYS_PROBE").as_deref(), Ok("x"));
        restore_env("FUTURE_TUI_KEYS_PROBE", None);
        assert!(std::env::var_os("FUTURE_TUI_KEYS_PROBE").is_none());
        restore_env("FUTURE_TUI_KEYS_PROBE", old);
    }

    // ─── Legacy sequences ──────────────────────────────────────────────────

    #[test]
    fn legacy_single_printable_chars_pass_through() {
        let _g = reset_kitty();
        assert_eq!(parse_key("a").as_deref(), Some("a"));
        assert_eq!(parse_key("Z").as_deref(), Some("Z"));
        assert_eq!(parse_key("5").as_deref(), Some("5"));
    }

    #[test]
    fn legacy_control_bytes_map_to_ctrl_letter() {
        let _g = reset_kitty();
        assert_eq!(parse_key("\x01").as_deref(), Some("ctrl+a"));
        assert_eq!(parse_key("\x03").as_deref(), Some("ctrl+c"));
        assert_eq!(parse_key("\x1a").as_deref(), Some("ctrl+z"));
    }

    #[test]
    fn legacy_named_keys() {
        let _g = reset_kitty();
        assert_eq!(parse_key("\x1b").as_deref(), Some("escape"));
        assert_eq!(parse_key("\t").as_deref(), Some("tab"));
        assert_eq!(parse_key("\r").as_deref(), Some("enter"));
        assert_eq!(parse_key(" ").as_deref(), Some("space"));
        assert_eq!(parse_key("\x7f").as_deref(), Some("backspace"));
    }

    #[test]
    fn legacy_arrow_and_function_keys_via_csi_ss3() {
        let _g = reset_kitty();
        assert_eq!(parse_key("\x1b[A").as_deref(), Some("up"));
        assert_eq!(parse_key("\x1b[B").as_deref(), Some("down"));
        assert_eq!(parse_key("\x1b[C").as_deref(), Some("right"));
        assert_eq!(parse_key("\x1b[D").as_deref(), Some("left"));
        assert_eq!(parse_key("\x1bOA").as_deref(), Some("up"));
        assert_eq!(parse_key("\x1bOP").as_deref(), Some("f1"));
        assert_eq!(parse_key("\x1b[15~").as_deref(), Some("f5"));
    }

    #[test]
    fn legacy_modified_sequences() {
        let _g = reset_kitty();
        assert_eq!(parse_key("\x1b[Z").as_deref(), Some("shift+tab"));
        assert_eq!(parse_key("\x1b[3~").as_deref(), Some("delete"));
        assert_eq!(parse_key("\x1b[3^").as_deref(), Some("ctrl+delete"));
        assert_eq!(parse_key("\x1b[2~").as_deref(), Some("insert"));
    }

    #[test]
    fn legacy_alt_letter_is_esc_letter_when_kitty_off() {
        let _g = reset_kitty();
        assert_eq!(parse_key("\x1ba").as_deref(), Some("alt+a"));
        assert_eq!(parse_key("\x1b5").as_deref(), Some("alt+5"));
    }

    #[test]
    fn unrecognized_input_returns_none() {
        let _g = reset_kitty();
        assert_eq!(parse_key("\x1b[999~"), None);
    }

    // ─── Kitty CSI-u ───────────────────────────────────────────────────────

    #[test]
    fn kitty_plain_keypress() {
        let _g = reset_kitty();
        assert_eq!(parse_key("\x1b[97u").as_deref(), Some("a"));
        assert_eq!(parse_key("\x1b[13u").as_deref(), Some("enter"));
        assert_eq!(parse_key("\x1b[27u").as_deref(), Some("escape"));
    }

    #[test]
    fn kitty_modifiers() {
        let _g = reset_kitty();
        assert_eq!(parse_key("\x1b[97;5u").as_deref(), Some("ctrl+a"));
        assert_eq!(parse_key("\x1b[97;2u").as_deref(), Some("shift+a"));
        assert_eq!(parse_key("\x1b[97;3u").as_deref(), Some("alt+a"));
        assert_eq!(parse_key("\x1b[99;5u").as_deref(), Some("ctrl+c"));
    }

    #[test]
    fn kitty_shift_ctrl_combines_in_order() {
        let _g = reset_kitty();
        assert_eq!(parse_key("\x1b[97;6u").as_deref(), Some("shift+ctrl+a"));
    }

    #[test]
    fn kitty_functional_arrows_and_navigation() {
        let _g = reset_kitty();
        assert_eq!(parse_key("\x1b[57419u").as_deref(), Some("up"));
        assert_eq!(parse_key("\x1b[57420u").as_deref(), Some("down"));
        assert_eq!(parse_key("\x1b[1;5A").as_deref(), Some("ctrl+up"));
        assert_eq!(parse_key("\x1b[1;2H").as_deref(), Some("shift+home"));
    }

    #[test]
    fn kitty_keypad_equivalents_normalize_to_base_keys() {
        let _g = reset_kitty();
        assert_eq!(parse_key("\x1b[57400u").as_deref(), Some("1"));
        assert_eq!(parse_key("\x1b[57414u").as_deref(), Some("enter"));
    }

    // ─── Event type detection ──────────────────────────────────────────────

    #[test]
    fn release_event_detection() {
        let _g = reset_kitty();
        assert!(is_key_release("\x1b[97;1:3u"));
        assert!(is_key_release("\x1b[1;1:3A"));
        assert!(!is_key_release("\x1b[97u"));
    }

    #[test]
    fn repeat_event_detection() {
        let _g = reset_kitty();
        assert!(is_key_repeat("\x1b[97;1:2u"));
        assert!(!is_key_repeat("\x1b[97u"));
    }

    #[test]
    fn bracketed_paste_markers_are_never_release_repeat() {
        let _g = reset_kitty();
        assert!(!is_key_release("\x1b[200~some;1:3u\x1b[201~"));
        assert!(!is_key_repeat("\x1b[200~some;1:2u\x1b[201~"));
    }

    // ─── decodeKittyPrintable ──────────────────────────────────────────────

    #[test]
    fn decodes_plain_keypress_to_character() {
        let _g = reset_kitty();
        assert_eq!(decode_kitty_printable("\x1b[97u").as_deref(), Some("a"));
    }

    #[test]
    fn rejects_non_kitty_input() {
        let _g = reset_kitty();
        assert_eq!(decode_kitty_printable("a"), None);
        assert_eq!(decode_kitty_printable("\x1b[A"), None);
    }

    // ─── Mode-dependent sequences ──────────────────────────────────────────

    #[test]
    fn esc_cr_is_shift_enter_under_kitty_alt_enter_in_legacy() {
        let _g = KITTY_STATE_LOCK.lock();
        set_kitty_protocol_active(true);
        assert_eq!(parse_key("\x1b\r").as_deref(), Some("shift+enter"));

        set_kitty_protocol_active(false);
        assert_eq!(parse_key("\x1b\r").as_deref(), Some("alt+enter"));
    }

    #[test]
    fn alt_letter_only_exists_in_legacy_mode() {
        let _g = KITTY_STATE_LOCK.lock();
        set_kitty_protocol_active(false);
        assert_eq!(parse_key("\x1bx").as_deref(), Some("alt+x"));

        // Under kitty, bare ESC+letter is not parsed as alt (kitty sends CSI-u).
        set_kitty_protocol_active(true);
        assert_eq!(parse_key("\x1bx"), None);
    }

    // ─── matches_key ───────────────────────────────────────────────────────

    #[test]
    fn matches_key_basic() {
        let _g = reset_kitty();
        assert!(matches_key("\x1b[A", "up"));
        assert!(!matches_key("\x1b[B", "up"));
        assert!(matches_key("\x03", "ctrl+c"));
        assert!(matches_key("\x1b", "escape"));
        assert!(matches_key("\t", "tab"));
        assert!(matches_key("\r", "enter"));
        assert!(matches_key(" ", "space"));
        assert!(matches_key("\x7f", "backspace"));
        assert!(matches_key("\x1b[Z", "shift+tab"));
        assert!(matches_key("a", "a"));
        assert!(!matches_key("a", "b"));
    }

    #[test]
    fn matches_key_kitty_and_modify_other_keys() {
        let _g = reset_kitty();
        assert!(matches_key("\x1b[97;5u", "ctrl+a"));
        assert!(matches_key("\x1b[27;5;97~", "ctrl+a"));
        assert!(matches_key("\x1b[57419u", "up"));
        assert!(matches_key("\x1b[1;2H", "shift+home"));
    }

    #[test]
    fn matches_key_legacy_alt_and_ctrl_arrows() {
        let _g = reset_kitty();
        assert!(matches_key("\x1bb", "alt+left"));
        assert!(matches_key("\x1b[1;5D", "ctrl+left"));
        assert!(matches_key("\x1b[1;3C", "alt+right"));
        // matchesKey for ctrl+up only accepts the kitty/modifyOtherKeys form
        // (TS parity) — the legacy SS3 "\x1bOa" maps via parseKey but not here.
        assert!(matches_key("\x1b[1;5A", "ctrl+up"));
        assert!(!matches_key("\x1bOa", "ctrl+up"));
    }

    #[test]
    fn matches_key_function_keys() {
        let _g = reset_kitty();
        assert!(matches_key("\x1bOP", "f1"));
        assert!(matches_key("\x1b[11~", "f1"));
        assert!(matches_key("\x1b[15~", "f5"));
        assert!(!matches_key("\x1b[15~", "f1"));
    }

    #[test]
    fn matches_key_unknown_key_id_returns_false() {
        let _g = reset_kitty();
        assert!(!matches_key("a", ""));
        assert!(!matches_key("a", "unknownkey"));
    }

    // ─── Codepoint tables ─────────────────────────────────────────────

    #[test]
    fn kitty_functional_equivalents_cover_the_whole_map() {
        let expected: [(i64, i64); 27] = [
            (57399, 48), (57400, 49), (57401, 50), (57402, 51), (57403, 52),
            (57404, 53), (57405, 54), (57406, 55), (57407, 56), (57408, 57),
            (57409, 46), (57410, 47), (57411, 42), (57412, 45), (57413, 43),
            (57415, 61), (57416, 44), (57417, ARROW_LEFT), (57418, ARROW_RIGHT),
            (57419, ARROW_UP), (57420, ARROW_DOWN), (57421, FUNC_PAGE_UP),
            (57422, FUNC_PAGE_DOWN), (57423, FUNC_HOME), (57424, FUNC_END),
            (57425, FUNC_INSERT), (57426, FUNC_DELETE),
        ];
        for (from, to) in expected {
            assert_eq!(kitty_functional_equivalent(from), Some(to));
            assert_eq!(normalize_kitty_functional_codepoint(from), to);
        }
        assert_eq!(kitty_functional_equivalent(57414), None);
        assert_eq!(normalize_kitty_functional_codepoint(97), 97);
    }

    #[test]
    fn shifted_letter_identity_drops_shift_for_uppercase() {
        assert_eq!(normalize_shifted_letter_identity_codepoint(65, MOD_SHIFT), 97);
        assert_eq!(normalize_shifted_letter_identity_codepoint(65, 0), 65);
        // Lock modifiers are masked out before the shift check.
        assert_eq!(
            normalize_shifted_letter_identity_codepoint(65, MOD_SHIFT | 64),
            97
        );
    }

    #[test]
    fn parse_event_type_and_js_int_variants() {
        assert_eq!(parse_event_type(None), KeyEventType::Press);
        assert_eq!(parse_event_type(Some("1")), KeyEventType::Press);
        assert_eq!(parse_event_type(Some("2")), KeyEventType::Repeat);
        assert_eq!(parse_event_type(Some("3")), KeyEventType::Release);
        assert_eq!(parse_event_type(Some("x")), KeyEventType::Press);
        assert_eq!(parse_js_int(""), None);
        assert_eq!(parse_js_int("12"), Some(12));
        assert_eq!(parse_js_int("x"), None);
    }

    // ─── Kitty parsing ────────────────────────────────────────────────

    #[test]
    fn parse_kitty_arrow_home_end_and_func_forms() {
        // Arrows with modifiers and event types.
        let p = parse_kitty_sequence("\x1b[1;3B").unwrap();
        assert_eq!(p.codepoint, ARROW_DOWN);
        assert_eq!(p.modifier, MOD_ALT);
        let p = parse_kitty_sequence("\x1b[1;5C").unwrap();
        assert_eq!(p.codepoint, ARROW_RIGHT);
        let p = parse_kitty_sequence("\x1b[1;2:3D").unwrap();
        assert_eq!(p.codepoint, ARROW_LEFT);
        assert_eq!(p.event_type, KeyEventType::Release);
        // Home/End.
        let p = parse_kitty_sequence("\x1b[1;1H").unwrap();
        assert_eq!(p.codepoint, FUNC_HOME);
        let p = parse_kitty_sequence("\x1b[1;1F").unwrap();
        assert_eq!(p.codepoint, FUNC_END);
        // Functional keys with ~.
        for (seq, cp) in [
            ("\x1b[2~", FUNC_INSERT),
            ("\x1b[3~", FUNC_DELETE),
            ("\x1b[5~", FUNC_PAGE_UP),
            ("\x1b[6~", FUNC_PAGE_DOWN),
            ("\x1b[7~", FUNC_HOME),
            ("\x1b[8~", FUNC_END),
        ] {
            assert_eq!(parse_kitty_sequence(seq).unwrap().codepoint, cp);
        }
        // Unknown functional number / non-matching input → None.
        assert!(parse_kitty_sequence("\x1b[4~").is_none());
        assert!(parse_kitty_sequence("hello").is_none());
        // Shifted/base key fields parse.
        let p = parse_kitty_sequence("\x1b[97:65:98;2u").unwrap();
        assert_eq!(p.shifted_key, Some(65));
        assert_eq!(p.base_layout_key, Some(98));
    }

    #[test]
    fn key_release_repeat_detection() {
        assert!(is_key_release("\x1b[97;1:3u"));
        assert!(is_key_release("\x1b[1;1:3A"));
        assert!(!is_key_release("\x1b[97u"));
        // Bracketed paste markers are never key events.
        assert!(!is_key_release("\x1b[200~x:3u"));
        assert!(is_key_repeat("\x1b[97;1:2u"));
        assert!(!is_key_repeat("\x1b[200~x:2u"));
        assert!(!is_key_repeat("\x1b[97u"));
    }

    // ─── is_windows_terminal_session ──────────────────────────────────

    #[test]
    fn windows_terminal_session_env_matrix() {
        let _guard = crate::test_env::lock();
        let saved = clear_terminal_env();
        assert!(!is_windows_terminal_session()); // no WT_SESSION
        std::env::set_var("WT_SESSION", "1");
        assert!(is_windows_terminal_session());
        std::env::set_var("SSH_CONNECTION", "x");
        assert!(!is_windows_terminal_session());
        std::env::remove_var("SSH_CONNECTION");
        std::env::set_var("SSH_CLIENT", "x");
        assert!(!is_windows_terminal_session());
        std::env::remove_var("SSH_CLIENT");
        std::env::set_var("SSH_TTY", "x");
        assert!(!is_windows_terminal_session());
        for (k, v) in saved {
            restore_env(k, v);
        }
    }

    // ─── format_parsed_key ────────────────────────────────────────────

    #[test]
    fn format_parsed_key_names_all_special_keys() {
        let cases: [(i64, &str); 16] = [
            (CP_ESCAPE, "escape"),
            (CP_TAB, "tab"),
            (CP_ENTER, "enter"),
            (CP_KP_ENTER, "enter"),
            (CP_SPACE, "space"),
            (CP_BACKSPACE, "backspace"),
            (FUNC_DELETE, "delete"),
            (FUNC_INSERT, "insert"),
            (FUNC_HOME, "home"),
            (FUNC_END, "end"),
            (FUNC_PAGE_UP, "pageUp"),
            (FUNC_PAGE_DOWN, "pageDown"),
            (ARROW_UP, "up"),
            (ARROW_DOWN, "down"),
            (ARROW_LEFT, "left"),
            (ARROW_RIGHT, "right"),
        ];
        for (cp, name) in cases {
            assert_eq!(format_parsed_key(cp, 0, None).as_deref(), Some(name));
        }
        // Digits, letters, symbols.
        assert_eq!(format_parsed_key(53, 0, None).as_deref(), Some("5"));
        assert_eq!(format_parsed_key(122, 0, None).as_deref(), Some("z"));
        assert_eq!(format_parsed_key(33, 0, None).as_deref(), Some("!"));
        // Unknown codepoint without a base layout key → None.
        assert_eq!(format_parsed_key(0x2603, 0, None), None);
        // …but a base layout key rescues it.
        assert_eq!(format_parsed_key(0x2603, 0, Some(99)).as_deref(), Some("c"));
        // Modifiers: super joins the list; unsupported bits reject.
        assert_eq!(
            format_parsed_key(97, MOD_SHIFT | MOD_SUPER, None).as_deref(),
            Some("shift+super+a")
        );
        assert_eq!(format_parsed_key(97, 16, None), None);
    }

    // ─── parse_key legacy singles ─────────────────────────────────────

    #[test]
    fn parse_key_legacy_control_and_alt_forms() {
        let _g = reset_kitty();
        assert_eq!(parse_key("\x1c").as_deref(), Some("ctrl+\\"));
        assert_eq!(parse_key("\x1d").as_deref(), Some("ctrl+]"));
        assert_eq!(parse_key("\x1f").as_deref(), Some("ctrl+-"));
        assert_eq!(parse_key("\x1b\x1b").as_deref(), Some("ctrl+alt+["));
        assert_eq!(parse_key("\x1b\x1c").as_deref(), Some("ctrl+alt+\\"));
        assert_eq!(parse_key("\x1b\x1d").as_deref(), Some("ctrl+alt+]"));
        assert_eq!(parse_key("\x1b\x1f").as_deref(), Some("ctrl+alt+-"));
        assert_eq!(parse_key("\x1bOM").as_deref(), Some("enter"));
        assert_eq!(parse_key("\n").as_deref(), Some("ctrl+j"));
        assert_eq!(parse_key("\x00").as_deref(), Some("ctrl+space"));
        assert_eq!(parse_key("\x08").as_deref(), Some("backspace"));
        assert_eq!(parse_key("\x1b\x7f").as_deref(), Some("alt+backspace"));
        assert_eq!(parse_key("\x1b\x08").as_deref(), Some("alt+backspace"));
        assert_eq!(parse_key("\x1b ").as_deref(), Some("alt+space"));
        // ESC + control letter → ctrl+alt+<letter>.
        assert_eq!(parse_key("\x1b\x01").as_deref(), Some("ctrl+alt+a"));
        // ESC + digit → alt+<digit>.
        assert_eq!(parse_key("\x1b5").as_deref(), Some("alt+5"));
        // Unrecognized → None.
        assert_eq!(parse_key("\x1b[").as_deref(), None);
    }

    #[test]
    fn parse_key_kitty_mode_changes_legacy_meanings() {
        let _g = reset_kitty();
        set_kitty_protocol_active(true);
        // In kitty mode these take their protocol meanings instead.
        assert_eq!(parse_key("\x1b\r").as_deref(), Some("shift+enter"));
        assert_eq!(parse_key("\n").as_deref(), Some("shift+enter"));
        // \x1b and space keep working.
        assert_eq!(parse_key(" ").as_deref(), Some("space"));
        set_kitty_protocol_active(false);
        assert_eq!(parse_key("\x1b\r").as_deref(), Some("alt+enter"));
    }

    #[test]
    fn parse_key_ctrl_backspace_on_windows_terminal() {
        let _g = reset_kitty();
        let _guard = crate::test_env::lock();
        let saved = clear_terminal_env();
        std::env::set_var("WT_SESSION", "1");
        assert_eq!(parse_key("\x08").as_deref(), Some("ctrl+backspace"));
        for (k, v) in saved {
            restore_env(k, v);
        }
    }

    // ─── printable decoding ───────────────────────────────────────────

    #[test]
    fn decode_kitty_printable_variants() {
        let _g = reset_kitty();
        // Plain printable.
        assert_eq!(decode_kitty_printable("\x1b[97u").as_deref(), Some("a"));
        // Shift with an explicit shifted key uses it.
        assert_eq!(decode_kitty_printable("\x1b[97:65;2u").as_deref(), Some("A"));
        // Shift without a shifted key keeps the base codepoint.
        assert_eq!(decode_kitty_printable("\x1b[97;2u").as_deref(), Some("a"));
        // Ctrl/Alt modifiers are not printable.
        assert_eq!(decode_kitty_printable("\x1b[97;5u"), None);
        assert_eq!(decode_kitty_printable("\x1b[97;3u"), None);
        // Control codepoints are not printable.
        assert_eq!(decode_kitty_printable("\x1b[13u"), None);
        // Functional codepoints normalize before the range check.
        assert_eq!(decode_kitty_printable("\x1b[57399u").as_deref(), Some("0"));
        // Non-kitty input → None.
        assert_eq!(decode_kitty_printable("a"), None);
    }

    #[test]
    fn decode_modify_other_keys_and_combined_printable() {
        let _g = reset_kitty();
        assert_eq!(
            decode_modify_other_keys_printable("\x1b[27;2;97~").as_deref(),
            Some("a")
        );
        // Beyond shift → not printable.
        assert_eq!(decode_modify_other_keys_printable("\x1b[27;5;97~"), None);
        // Control codepoint → None.
        assert_eq!(decode_modify_other_keys_printable("\x1b[27;2;13~"), None);
        // Unparseable → None.
        assert!(decode_modify_other_keys_printable("x").is_none());
        // The combined decoder tries kitty first, then modifyOtherKeys.
        assert_eq!(decode_printable_key("\x1b[97u").as_deref(), Some("a"));
        assert_eq!(decode_printable_key("\x1b[27;2;98~").as_deref(), Some("b"));
        assert_eq!(decode_printable_key("\x1b[A"), None);
    }

    // ─── key-id helpers ───────────────────────────────────────────────

    #[test]
    fn key_id_builders_and_parsers() {
        assert_eq!(modified_key("ctrl", "c"), "ctrl+c");
        assert_eq!(ctrl_key("c"), "ctrl+c");
        // parse_key_id: trailing empty key → None.
        assert!(parse_key_id("ctrl+").is_none());
        let p = parse_key_id("Ctrl+Shift+A").unwrap();
        assert!(p.ctrl && p.shift && !p.alt && !p.super_modifier);
        assert_eq!(p.key, "a");
        // is_single_key_char.
        assert!(is_single_key_char("a"));
        assert!(is_single_key_char("5"));
        assert!(is_single_key_char("!"));
        assert!(!is_single_key_char("ab"));
        assert!(!is_single_key_char("A"));
        // is_digit_key.
        assert!(is_digit_key("7"));
        assert!(!is_digit_key("a"));
        assert!(!is_digit_key("77"));
    }

    #[test]
    fn raw_ctrl_char_and_backspace_matching() {
        let _guard = crate::test_env::lock();
        // Letters and the five symbol controls map; others don't.
        assert_eq!(raw_ctrl_char("a"), Some('\x01'));
        assert_eq!(raw_ctrl_char("["), Some('\x1b'));
        assert_eq!(raw_ctrl_char("\\"), Some('\x1c'));
        assert_eq!(raw_ctrl_char("]"), Some('\x1d'));
        assert_eq!(raw_ctrl_char("_"), Some('\x1f'));
        assert_eq!(raw_ctrl_char("-"), Some('\r')); // '-' & 0x1f = 0x0d
        assert_eq!(raw_ctrl_char("!"), None);
        assert_eq!(raw_ctrl_char(""), None);

        // matches_raw_backspace matrix.
        assert!(matches_raw_backspace("\x7f", 0));
        assert!(!matches_raw_backspace("\x7f", MOD_CTRL));
        assert!(!matches_raw_backspace("x", 0));
        let saved = clear_terminal_env();
        assert!(matches_raw_backspace("\x08", 0));
        std::env::set_var("WT_SESSION", "1");
        assert!(matches_raw_backspace("\x08", MOD_CTRL));
        assert!(!matches_raw_backspace("\x08", 0));
        for (k, v) in saved {
            restore_env(k, v);
        }
    }

    #[test]
    fn legacy_sequence_tables_are_complete() {
        for (key, first) in [
            ("insert", "\x1b[2~"),
            ("delete", "\x1b[3~"),
            ("clear", "\x1b[E"),
            ("home", "\x1bOH"),
            ("end", "\x1bOF"),
            ("pageUp", "\x1b[5~"),
            ("pageDown", "\x1b[6~"),
            ("up", "\x1b[A"),
            ("down", "\x1b[B"),
            ("left", "\x1b[D"),
            ("right", "\x1b[C"),
            ("f1", "\x1bOP"),
            ("f2", "\x1bOQ"),
            ("f3", "\x1bOR"),
            ("f4", "\x1bOS"),
            ("f5", "\x1b[15~"),
            ("f6", "\x1b[17~"),
            ("f7", "\x1b[18~"),
            ("f8", "\x1b[19~"),
            ("f9", "\x1b[20~"),
            ("f10", "\x1b[21~"),
            ("f11", "\x1b[23~"),
            ("f12", "\x1b[24~"),
        ] {
            assert!(legacy_key_sequences(key).contains(&first));
            assert!(matches_legacy_sequence(first, legacy_key_sequences(key)));
        }
        assert!(legacy_key_sequences("nope").is_empty());
        // Shift variants.
        for (key, seq) in [
            ("up", "\x1b[a"),
            ("down", "\x1b[b"),
            ("right", "\x1b[c"),
            ("left", "\x1b[d"),
            ("clear", "\x1b[e"),
            ("insert", "\x1b[2$"),
            ("delete", "\x1b[3$"),
            ("pageUp", "\x1b[5$"),
            ("pageDown", "\x1b[6$"),
            ("home", "\x1b[7$"),
            ("end", "\x1b[8$"),
        ] {
            assert!(legacy_shift_sequences(key).contains(&seq));
            assert!(matches_legacy_modifier_sequence(seq, key, MOD_SHIFT));
        }
        assert!(legacy_shift_sequences("f1").is_empty());
        // Ctrl variants.
        for (key, seq) in [
            ("up", "\x1bOa"),
            ("down", "\x1bOb"),
            ("right", "\x1bOc"),
            ("left", "\x1bOd"),
            ("clear", "\x1bOe"),
            ("insert", "\x1b[2^"),
            ("delete", "\x1b[3^"),
            ("pageUp", "\x1b[5^"),
            ("pageDown", "\x1b[6^"),
            ("home", "\x1b[7^"),
            ("end", "\x1b[8^"),
        ] {
            assert!(legacy_ctrl_sequences(key).contains(&seq));
            assert!(matches_legacy_modifier_sequence(seq, key, MOD_CTRL));
        }
        assert!(legacy_ctrl_sequences("f1").is_empty());
        // Other modifiers match nothing.
        assert!(!matches_legacy_modifier_sequence("\x1b[a", "up", MOD_ALT));
    }

    // ─── matches_key ──────────────────────────────────────────────────

    #[test]
    fn matches_key_escape_space_tab() {
        let _g = reset_kitty();
        // escape: plain, kitty, mok forms; modified never matches.
        assert!(matches_key("\x1b", "escape"));
        assert!(matches_key("\x1b[27u", "escape"));
        assert!(matches_key("\x1b[27;1;27~", "escape"));
        assert!(!matches_key("\x1b", "ctrl+escape"));
        assert!(matches_key("\x1b", "esc"));
        // space: ctrl/alt legacy, kitty and mok forms.
        assert!(matches_key(" ", "space"));
        assert!(matches_key("\x1b[32u", "space"));
        assert!(matches_key("\x1b[27;1;32~", "space"));
        assert!(matches_key("\x00", "ctrl+space"));
        assert!(matches_key("\x1b ", "alt+space"));
        assert!(matches_key("\x1b[32;5u", "ctrl+space"));
        assert!(matches_key("\x1b[27;5;32~", "ctrl+space"));
        // tab: plain, shift, kitty/mok forms.
        assert!(matches_key("\t", "tab"));
        assert!(matches_key("\x1b[Z", "shift+tab"));
        assert!(matches_key("\x1b[9;2u", "shift+tab"));
        assert!(matches_key("\x1b[27;2;9~", "shift+tab"));
        assert!(matches_key("\x1b[9u", "tab"));
        assert!(matches_key("\x1b[9;5u", "ctrl+tab"));
        assert!(matches_key("\x1b[27;5;9~", "ctrl+tab"));
    }

    #[test]
    fn matches_key_enter_forms() {
        let _g = reset_kitty();
        assert!(matches_key("\r", "enter"));
        assert!(matches_key("\x1bOM", "enter"));
        assert!(matches_key("\x1b[13u", "enter"));
        assert!(matches_key("\x1b[57414u", "enter")); // keypad enter
        assert!(matches_key("\r", "return"));
        // shift+enter: kitty (+ keypad), mok, and the kitty-mode legacy.
        assert!(matches_key("\x1b[13;2u", "shift+enter"));
        assert!(matches_key("\x1b[57414;2u", "shift+enter"));
        assert!(matches_key("\x1b[27;2;13~", "shift+enter"));
        assert!(!matches_key("\x1b\r", "shift+enter")); // kitty off
        set_kitty_protocol_active(true);
        assert!(matches_key("\x1b\r", "shift+enter"));
        assert!(matches_key("\n", "shift+enter"));
        set_kitty_protocol_active(false);
        // alt+enter: kitty (+ keypad), mok, legacy when kitty is off.
        assert!(matches_key("\x1b[13;3u", "alt+enter"));
        assert!(matches_key("\x1b[57414;3u", "alt+enter"));
        assert!(matches_key("\x1b[27;3;13~", "alt+enter"));
        assert!(matches_key("\x1b\r", "alt+enter"));
        set_kitty_protocol_active(true);
        assert!(!matches_key("\x1b\r", "alt+enter"));
        set_kitty_protocol_active(false);
        // Other modifier combos go through kitty/mok matching.
        assert!(matches_key("\x1b[13;5u", "ctrl+enter"));
        assert!(matches_key("\x1b[57414;5u", "ctrl+enter"));
        assert!(matches_key("\x1b[27;5;13~", "ctrl+enter"));
        assert!(!matches_key("\r", "ctrl+enter"));
    }

    #[test]
    fn matches_key_backspace_forms() {
        let _g = reset_kitty();
        assert!(matches_key("\x7f", "backspace"));
        assert!(matches_key("\x08", "backspace"));
        assert!(matches_key("\x1b[127u", "backspace"));
        assert!(matches_key("\x1b[27;1;127~", "backspace"));
        assert!(matches_key("\x1b\x7f", "alt+backspace"));
        assert!(matches_key("\x1b\x08", "alt+backspace"));
        assert!(matches_key("\x1b[127;3u", "alt+backspace"));
        assert!(matches_key("\x1b[27;3;127~", "alt+backspace"));
        assert!(matches_key("\x1b[127;5u", "ctrl+backspace"));
        assert!(matches_key("\x1b[27;5;127~", "ctrl+backspace"));
        // shift+backspace falls to the generic kitty/mok path.
        assert!(matches_key("\x1b[127;2u", "shift+backspace"));
        assert!(matches_key("\x1b[27;2;127~", "shift+backspace"));
        assert!(!matches_key("x", "backspace"));
    }

    #[test]
    fn matches_key_editing_and_navigation_keys() {
        let _g = reset_kitty();
        assert!(matches_key("\x1b[2~", "insert"));
        assert!(matches_key("\x1b[57425u", "insert"));
        assert!(matches_key("\x1b[57425;5u", "ctrl+insert"));
        assert!(matches_key("\x1b[3~", "delete"));
        assert!(matches_key("\x1b[57426u", "delete"));
        assert!(matches_key("\x1b[57426;5u", "ctrl+delete"));
        assert!(matches_key("\x1b[E", "clear"));
        assert!(matches_key("\x1b[e", "shift+clear"));
        assert!(matches_key("\x1bOe", "ctrl+clear"));
        assert!(matches_key("\x1bOH", "home"));
        assert!(matches_key("\x1b[1;1H", "home"));
        assert!(matches_key("\x1b[7~", "home"));
        assert!(matches_key("\x1b[1;5H", "ctrl+home"));
        assert!(matches_key("\x1bOF", "end"));
        assert!(matches_key("\x1b[1;1F", "end"));
        assert!(matches_key("\x1b[8~", "end"));
        assert!(matches_key("\x1b[1;5F", "ctrl+end"));
        assert!(matches_key("\x1b[5~", "pageup"));
        assert!(matches_key("\x1b[5;5~", "ctrl+pageup"));
        assert!(matches_key("\x1b[6~", "pagedown"));
        assert!(matches_key("\x1b[6;5~", "ctrl+pagedown"));
    }

    #[test]
    fn matches_key_arrow_forms() {
        let _g = reset_kitty();
        assert!(matches_key("\x1b[A", "up"));
        assert!(matches_key("\x1b[1;1A", "up"));
        assert!(matches_key("\x1bp", "alt+up"));
        assert!(matches_key("\x1b[1;3A", "alt+up"));
        assert!(matches_key("\x1b[1;2A", "shift+up"));
        assert!(matches_key("\x1b[B", "down"));
        assert!(matches_key("\x1bn", "alt+down"));
        assert!(matches_key("\x1b[1;3B", "alt+down"));
        assert!(matches_key("\x1b[1;5B", "ctrl+down"));
        assert!(matches_key("\x1b[D", "left"));
        assert!(matches_key("\x1bb", "alt+left"));
        assert!(matches_key("\x1bB", "alt+left")); // kitty off
        assert!(matches_key("\x1b[1;3D", "alt+left"));
        assert!(matches_key("\x1b[1;5D", "ctrl+left"));
        assert!(matches_key("\x1b[1;2D", "shift+left"));
        assert!(matches_key("\x1b[C", "right"));
        assert!(matches_key("\x1bf", "alt+right"));
        assert!(matches_key("\x1bF", "alt+right")); // kitty off
        assert!(matches_key("\x1b[1;3C", "alt+right"));
        assert!(matches_key("\x1b[1;5C", "ctrl+right"));
        assert!(matches_key("\x1b[1;2C", "shift+right"));
        // With kitty on, the bare alt+letter forms no longer match arrows.
        set_kitty_protocol_active(true);
        assert!(!matches_key("\x1bB", "alt+left"));
        assert!(!matches_key("\x1bF", "alt+right"));
        set_kitty_protocol_active(false);
    }

    #[test]
    fn matches_key_function_key_matrix() {
        let _g = reset_kitty();
        for (key, seq) in [
            ("f1", "\x1bOP"),
            ("f2", "\x1bOQ"),
            ("f3", "\x1bOR"),
            ("f4", "\x1bOS"),
            ("f5", "\x1b[15~"),
            ("f6", "\x1b[17~"),
            ("f7", "\x1b[18~"),
            ("f8", "\x1b[19~"),
            ("f9", "\x1b[20~"),
            ("f10", "\x1b[21~"),
            ("f11", "\x1b[23~"),
            ("f12", "\x1b[24~"),
        ] {
            assert!(matches_key(seq, key));
            assert!(!matches_key(seq, &format!("ctrl+{key}")));
        }
    }

    #[test]
    fn matches_key_single_char_forms() {
        let _g = reset_kitty();
        // Plain.
        assert!(matches_key("a", "a"));
        assert!(matches_key("\x1b[97u", "a"));
        assert!(matches_key("5", "5"));
        // Raw ctrl byte.
        assert!(matches_key("\x03", "ctrl+c"));
        assert!(matches_key("\x1b[99;5u", "ctrl+c"));
        assert!(matches_key("\x1b[27;5;99~", "ctrl+c"));
        // ctrl+alt raw byte (kitty off).
        assert!(matches_key("\x1b\x03", "ctrl+alt+c"));
        set_kitty_protocol_active(true);
        assert!(!matches_key("\x1b\x03", "ctrl+alt+c"));
        set_kitty_protocol_active(false);
        // alt+letter / alt+digit raw.
        assert!(matches_key("\x1bx", "alt+x"));
        assert!(matches_key("\x1b7", "alt+7"));
        // shift+letter matches the uppercase char directly.
        assert!(matches_key("A", "shift+a"));
        assert!(matches_key("\x1b[97;2u", "shift+a"));
        assert!(matches_key("\x1b[27;2;97~", "shift+a"));
        // shift+ctrl.
        assert!(matches_key("\x1b[99;6u", "shift+ctrl+c"));
        assert!(matches_key("\x1b[27;6;99~", "shift+ctrl+c"));
        // Super / other modifier via the generic path.
        assert!(matches_key("\x1b[97;9u", "super+a"));
        assert!(matches_key("\x1b[27;9;97~", "super+a"));
        // Symbol keys.
        assert!(matches_key("!", "!"));
        assert!(matches_key("\x1b[33u", "!"));
        // A key that isn't a single char can't match char input.
        assert!(!matches_key("a", "ab"));
    }

    #[test]
    fn parse_key_accepts_modify_other_keys_input() {
        let _g = reset_kitty();
        assert_eq!(parse_key("\x1b[27;5;99~").as_deref(), Some("ctrl+c"));
        assert_eq!(parse_key("\x1b[27;1;97~").as_deref(), Some("a"));
        assert_eq!(parse_key("Q").as_deref(), Some("Q"));
        assert_eq!(parse_key("~").as_deref(), Some("~"));
    }

    #[test]
    fn matches_key_space_with_kitty_on() {
        let _g = reset_kitty();
        set_kitty_protocol_active(true);
        // Plain space still matches; the legacy ctrl/alt forms don't.
        assert!(matches_key(" ", "space"));
        assert!(!matches_key("\x00", "ctrl+space"));
        assert!(!matches_key("\x1b ", "alt+space"));
        assert!(matches_key("\x1b[32;5u", "ctrl+space"));
        set_kitty_protocol_active(false);
    }

    #[test]
    fn matches_key_ctrl_backspace_windows_terminal_raw() {
        let _g = reset_kitty();
        let _guard = crate::test_env::lock();
        let saved = clear_terminal_env();
        std::env::set_var("WT_SESSION", "1");
        // On Windows Terminal, \x08 IS ctrl+backspace.
        assert!(matches_key("\x08", "ctrl+backspace"));
        assert!(!matches_key("\x08", "backspace"));
        for (k, v) in saved {
            restore_env(k, v);
        }
    }

    #[test]
    fn matches_key_kitty_mod_zero_forms() {
        let _g = reset_kitty();
        assert!(matches_key("\x1b[5;1~", "pageup"));
        assert!(matches_key("\x1b[6;1~", "pagedown"));
        assert!(matches_key("\x1b[1;1B", "down"));
        assert!(matches_key("\x1b[1;1D", "left"));
        assert!(matches_key("\x1b[1;1C", "right"));
    }

    #[test]
    fn matches_key_raw_combo_negative_paths() {
        let _g = reset_kitty();
        // ctrl+alt raw byte for a different letter doesn't match.
        assert!(!matches_key("\x1b\x04", "ctrl+alt+c"));
        // ctrl+alt+<digit> has no raw byte form at all.
        assert!(!matches_key("\x1b5", "ctrl+alt+5"));
        // alt raw for a different letter doesn't match.
        assert!(!matches_key("\x1by", "alt+x"));
        // ctrl+<symbol> has no raw byte form.
        assert!(!matches_key("\x01", "ctrl+!"));
        // …but its kitty form matches.
        assert!(matches_key("\x1b[33;5u", "ctrl+!"));
    }

    #[test]
    fn matches_key_base_layout_and_mismatch_paths() {
        let _g = reset_kitty();
        // Modifier mismatch → no match.
        assert!(!matches_key("\x1b[97;5u", "a"));
        // Base layout fallback: non-latin codepoint with a latin base key.
        assert!(matches_key("\x1b[8364::99;5u", "ctrl+c"));
        // …but a latin letter codepoint does not fall back.
        assert!(!matches_key("\x1b[109::99;5u", "ctrl+c"));
        // …and a mismatched base key doesn't either.
        assert!(!matches_key("\x1b[8364::100;5u", "ctrl+c"));
        // modifyOtherKeys parse failure → false.
        assert!(!matches_key("junk", "ctrl+c"));
        // Printable mok requires a nonzero expected modifier path.
        assert!(!matches_printable_modify_other_keys("\x1b[27;1;97~", 97, 0));
        assert!(!matches_printable_modify_other_keys("junk", 97, MOD_CTRL));
        assert!(!matches_printable_modify_other_keys(
            "\x1b[27;2;97~",
            97,
            MOD_CTRL
        ));
    }
}
