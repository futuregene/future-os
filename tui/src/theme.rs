//! Theme — 1:1 port of `tui/src/theme.ts` (the app's dark theme) plus the
//! legacy `DEFAULT_THEME` from `tui/src/tui.ts`.

/// CSI introducer (`\x1b[`).
pub const CSI: &str = "\x1b[";
/// ANSI reset (`\x1b[m`).
pub const RESET: &str = "\x1b[m";

// 256-color palette (approximate to hex values)
pub const C: ColorConstants = ColorConstants {
    cyan: 45,            // #00d7ff
    blue: 69,            // #5f87ff
    green: 143,          // #b5bd68
    red: 204,            // #cc6666
    yellow: 226,         // #ffff00
    gray: 244,           // #808080
    dim_gray: 241,       // #626262
    dark_gray: 240,      // #505050
    accent: 109,         // #8abeb7
    selected_bg: 237,    // #3a3a4a
    user_msg_bg: 59,     // #343541
    tool_pending_bg: 17, // #00005f
    tool_success_bg: 22, // #005f00
    tool_error_bg: 52,   // #5f0000

    // Markdown
    md_heading: 221,           // #f0c674 (gold)
    md_link: 117,              // #81a2be (light blue)
    md_link_url: 102,          // #666666
    md_code: 151,              // #8abeb7 (accent)
    md_code_block: 143,        // #b5bd68 (= `green`; the renderer's fence color)
    md_code_block_border: 244, // gray
    md_quote: 244,             // gray

    // Thinking levels
    thinking_off: 240,
    thinking_minimal: 110,
    thinking_low: 68,
    thinking_medium: 117,
    thinking_high: 182,
    thinking_xhigh: 213,

    // Text
    fg: 252,
    dim: 245,
};

pub struct ColorConstants {
    pub cyan: u8,
    pub blue: u8,
    pub green: u8,
    pub red: u8,
    pub yellow: u8,
    pub gray: u8,
    pub dim_gray: u8,
    pub dark_gray: u8,
    pub accent: u8,
    pub selected_bg: u8,
    pub user_msg_bg: u8,
    pub tool_pending_bg: u8,
    pub tool_success_bg: u8,
    pub tool_error_bg: u8,
    pub md_heading: u8,
    pub md_link: u8,
    pub md_link_url: u8,
    pub md_code: u8,
    pub md_code_block: u8,
    pub md_code_block_border: u8,
    pub md_quote: u8,
    pub thinking_off: u8,
    pub thinking_minimal: u8,
    pub thinking_low: u8,
    pub thinking_medium: u8,
    pub thinking_high: u8,
    pub thinking_xhigh: u8,
    pub fg: u8,
    pub dim: u8,
}

/// Terminal color indices; `-1` means "use terminal default".
pub type Color = i16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub bg: Color,
    pub fg: Color,
    pub accent: Color,
    pub border: Color,
    pub selected_bg: Color,
    pub selected_fg: Color,
    pub dim: Color,
    pub error: Color,
    pub success: Color,

    // Markdown
    pub md_heading: Color,
    pub md_link: Color,
    pub md_code: Color,
    pub md_code_block: Color,
    pub md_code_block_border: Color,
    pub md_quote: Color,

    // Tool
    pub tool_pending_bg: Color,
    pub tool_success_bg: Color,
    pub tool_error_bg: Color,
    pub tool_title: Color,
    pub tool_output: Color,

    // Thinking
    pub thinking_off: Color,
    pub thinking_minimal: Color,
    pub thinking_low: Color,
    pub thinking_medium: Color,
    pub thinking_high: Color,
    pub thinking_xhigh: Color,
    pub thinking_text: Color,

    // User/assistant messages
    pub user_bg: Color,
    pub assistant_bg: Color,
}

pub const DARK_THEME: Theme = Theme {
    bg: -1, // use terminal default background
    fg: 252,
    accent: 39,
    border: 240,
    selected_bg: 38,
    selected_fg: 255,
    dim: C.dim as i16,
    error: C.red as i16,
    success: C.green as i16,

    md_heading: C.md_heading as i16,
    md_link: C.md_link as i16,
    md_code: C.md_code as i16,
    md_code_block: C.md_code_block as i16,
    md_code_block_border: C.md_code_block_border as i16,
    md_quote: C.md_quote as i16,

    tool_pending_bg: 236, // subtle dark gray
    tool_success_bg: 236, // subtle dark gray
    tool_error_bg: C.tool_error_bg as i16,
    tool_title: C.accent as i16,
    tool_output: C.gray as i16,

    thinking_off: C.thinking_off as i16,
    thinking_minimal: C.thinking_minimal as i16,
    thinking_low: C.thinking_low as i16,
    thinking_medium: C.thinking_medium as i16,
    thinking_high: C.thinking_high as i16,
    thinking_xhigh: C.thinking_xhigh as i16,
    thinking_text: C.gray as i16,

    user_bg: C.user_msg_bg as i16, // ChatGPT-style user message bubble background
    assistant_bg: -1,              // use terminal default background
};

/// The palette the TUI has always shipped (`DARK_THEME`). Implemented by hand
/// rather than derived: every field of `Theme` needs its own value, and `-1`
/// (terminal default) is a meaningful color, not a zeroed one.
impl Default for Theme {
    fn default() -> Self {
        DARK_THEME
    }
}

// ─── Chrome palette ──────────────────────────────────────────────────────

/// Palette for the *chrome* widgets — the ones the ported TS renderers paint
/// from the wider `C` table instead of from `Theme`: footer, input, select
/// list, pager, usage panel and the scoped-model selector.
///
/// Why a second palette exists at all: `Theme` models the *conversation*
/// (markdown, tool blocks, thinking levels) in exactly 29 fields, and that set
/// is frozen — `themes.rs` asserts the field count and compares themes field by
/// field, so widening `Theme` is a deliberate cross-module change. The chrome
/// needs a handful of roles that set has no slot for (a faint annotation gray,
/// a cyan status tag, the token/cost green, the >70 % warning yellow, the
/// ✓/✗ marks, the list/pager backgrounds), so they live here.
///
/// Two sources, by design:
///
/// * [`Chrome::LEGACY`] is what the chrome paints **today**, byte for byte.
/// * [`Chrome::from_theme`] derives every role from a [`Theme`] so switching
///   palettes with `/theme` recolors the whole frame, and returns `LEGACY`
///   verbatim for the default palette so `/theme dark` cannot change a single
///   byte of output (the byte-level chrome tests depend on it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chrome {
    /// Default body text (`245`).
    pub base: u8,
    /// Bright primary text: model name, totals, list labels (`252`).
    pub text: u8,
    /// Faint annotation text ("Session", "no usage data yet") (`241`).
    pub muted: u8,
    /// Interactive accent: list titles, spinner, prompt marks (`39`).
    pub accent: u8,
    /// Secondary status accent ("streaming") (`45`).
    pub info: u8,
    /// Section heading ("By model") (`109`).
    pub heading: u8,
    /// Search-hit text in the pager (`117`).
    pub secondary: u8,
    /// Thinking-level tag in the footer (`117`).
    pub thinking: u8,
    /// Positive figure in the usage table (`143`).
    pub success: u8,
    /// "Enabled" mark in the scoped-model selector (`40`).
    pub mark_ok: u8,
    /// Error text (`204`).
    pub error: u8,
    /// "Disabled" mark in the scoped-model selector (`196`).
    pub mark_no: u8,
    /// Warning text and the >70 % context fill (`226`).
    pub warn: u8,
    /// Token/cost readout in the footer (`71`).
    pub token: u8,
    /// Border/secondary marker ("(auto)") (`240`).
    pub border: u8,
    /// Selection background of the list widgets (`38`).
    pub selected_bg: u8,
    /// Foreground on [`Chrome::selected_bg`] (`255`).
    pub selected_fg: u8,
    /// Selection background of a pager search hit (`237`).
    pub highlight_bg: u8,
    /// Background the list widgets pad their rows with (`235`).
    pub list_bg: u8,
}

impl Chrome {
    /// The exact indices the ported TS chrome emits today — the values every
    /// byte-level test was written against.
    pub const LEGACY: Chrome = Chrome {
        base: 245,
        text: 252,
        muted: 241,
        accent: 39,
        info: 45,
        heading: 109,
        secondary: 117,
        thinking: 117,
        success: 143,
        mark_ok: 40,
        error: 204,
        mark_no: 196,
        warn: 226,
        token: 71,
        border: 240,
        selected_bg: 38,
        selected_fg: 255,
        highlight_bg: 237,
        list_bg: 235,
    };

    /// The chrome palette for `theme`: [`Chrome::LEGACY`] for the default
    /// palette, otherwise every role resolved from a `Theme` field.
    ///
    /// Roles map to their nearest semantic `Theme` field (`base`/`muted` →
    /// `dim`, `text` → `fg`, `accent` → `accent`, `token`/`success`/`mark_ok` →
    /// `success`, `warn` → `thinking_high`, …). A theme that leaves `bg` at `-1`
    /// keeps the legacy list background: `-1` means "terminal default" and
    /// cannot be used as an opaque background for a padded row.
    pub fn from_theme(theme: &Theme) -> Chrome {
        if *theme == DARK_THEME {
            return Self::LEGACY;
        }
        Chrome {
            base: index(theme.dim),
            text: index(theme.fg),
            muted: index(theme.dim),
            accent: index(theme.accent),
            info: index(theme.md_link),
            heading: index(theme.tool_title),
            secondary: index(theme.md_link),
            thinking: index(theme.thinking_medium),
            success: index(theme.success),
            mark_ok: index(theme.success),
            error: index(theme.error),
            mark_no: index(theme.error),
            warn: index(theme.thinking_high),
            token: index(theme.success),
            border: index(theme.border),
            selected_bg: index(theme.selected_bg),
            selected_fg: index(theme.selected_fg),
            highlight_bg: index(theme.selected_bg),
            list_bg: if theme.bg < 0 {
                Self::LEGACY.list_bg
            } else {
                index(theme.bg)
            },
        }
    }
}

/// A `Theme` color index as the `u8` the `38;5;N`/`48;5;N` sequences need.
/// `Theme` allows `-1` ("terminal default"), which has no `38;5;` spelling;
/// such a role falls back to index 0 rather than wrapping to 255.
pub(crate) fn index(color: Color) -> u8 {
    color.clamp(0, 255) as u8
}

/// Legacy theme table from `tui/src/tui.ts` (the app uses `DARK_THEME` from
/// theme.ts; kept for completeness of the 1:1 port).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyTheme {
    pub bg: Color,
    pub fg: Color,
    pub accent: Color,
    pub border: Color,
    pub selected_bg: Color,
    pub selected_fg: Color,
    pub dim_fg: Color,
    pub error: Color,
    pub success: Color,
}

pub const DEFAULT_THEME: LegacyTheme = LegacyTheme {
    bg: -1,
    fg: 252,
    accent: 39,
    border: 240,
    selected_bg: 38,
    selected_fg: 255,
    dim_fg: 245,
    error: 160,
    success: 76,
};

// ─── Color helpers ─────────────────────────────────────────────────────

pub fn fg(c: u8, text: &str) -> String {
    format!("{CSI}38;5;{c}m{text}{RESET}")
}

pub fn bg(c: u8, text: &str) -> String {
    format!("{CSI}48;5;{c}m{text}{RESET}")
}

pub fn bold(text: &str) -> String {
    format!("{CSI}1m{text}{RESET}")
}

pub fn dim(text: &str) -> String {
    format!("{CSI}2m{text}{RESET}")
}

pub fn italic(text: &str) -> String {
    format!("{CSI}3m{text}{RESET}")
}

pub fn underline(text: &str) -> String {
    format!("{CSI}4m{text}{RESET}")
}

pub fn strikethrough(text: &str) -> String {
    format!("{CSI}9m{text}{RESET}")
}

pub fn reset(text: &str) -> String {
    format!("{RESET}{text}{RESET}")
}

// ─── Raw style primitives (no auto-RESET, for composable theme building) ──

/// Apply foreground color without trailing RESET.
pub fn fg_raw(c: u8, text: &str) -> String {
    format!("{CSI}38;5;{c}m{text}")
}

/// Apply background color without trailing RESET.
pub fn bg_raw(c: u8, text: &str) -> String {
    format!("{CSI}48;5;{c}m{text}")
}

/// Apply bold without trailing RESET.
pub fn bold_raw(text: &str) -> String {
    format!("{CSI}1m{text}")
}

/// Apply dim without trailing RESET.
pub fn dim_raw(text: &str) -> String {
    format!("{CSI}2m{text}")
}

/// Apply italic without trailing RESET.
pub fn italic_raw(text: &str) -> String {
    format!("{CSI}3m{text}")
}

/// Apply underline without trailing RESET.
pub fn underline_raw(text: &str) -> String {
    format!("{CSI}4m{text}")
}

/// Apply strikethrough without trailing RESET.
pub fn strikethrough_raw(text: &str) -> String {
    format!("{CSI}9m{text}")
}

/// Reverse video without trailing RESET.
pub fn reverse_raw(text: &str) -> String {
    format!("{CSI}7m{text}")
}

/// Compose multiple style functions into one. Each fn receives text and
/// returns styled text WITHOUT reset codes — the caller appends the final
/// reset. Example: `style("hello", |t| fg_raw(151, t), |t| bold_raw(t))`.
pub fn style(text: &str, fns: &[&dyn Fn(&str) -> String]) -> String {
    let mut result = text.to_string();
    for f in fns {
        result = f(&result);
    }
    result + RESET
}

// ─── Thinking ────────────────────────────────────────────────────────────

pub fn thinking_color(level: &str) -> u8 {
    match level {
        "minimal" => C.thinking_minimal,
        "low" => C.thinking_low,
        "medium" => C.thinking_medium,
        "high" => C.thinking_high,
        "xhigh" => C.thinking_xhigh,
        _ => C.thinking_off,
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fg_wraps_with_256_color_and_reset() {
        assert_eq!(fg(151, "hi"), "\x1b[38;5;151mhi\x1b[m");
    }

    #[test]
    fn bg_wraps_with_256_color_and_reset() {
        assert_eq!(bg(59, "hi"), "\x1b[48;5;59mhi\x1b[m");
    }

    #[test]
    fn raw_variants_omit_reset() {
        assert_eq!(fg_raw(151, "hi"), "\x1b[38;5;151mhi");
        assert_eq!(bold_raw("hi"), "\x1b[1mhi");
        assert_eq!(reverse_raw("hi"), "\x1b[7mhi");
    }

    #[test]
    fn style_composes_without_double_reset() {
        let fns: [&dyn Fn(&str) -> String; 2] = [&|t| fg_raw(151, t), &|t| bold_raw(t)];
        // fns apply in order, each wrapping the previous result (TS `fn(result)`
        // semantics) — so bold lands outside the fg code.
        assert_eq!(style("hi", &fns), "\x1b[1m\x1b[38;5;151mhi\x1b[m");
    }

    #[test]
    fn thinking_color_maps_levels() {
        assert_eq!(thinking_color("minimal"), C.thinking_minimal);
        assert_eq!(thinking_color("high"), C.thinking_high);
        assert_eq!(thinking_color("off"), C.thinking_off);
        assert_eq!(thinking_color("bogus"), C.thinking_off);
    }

    #[test]
    fn dark_theme_uses_terminal_default_for_bg() {
        assert_eq!(DARK_THEME.bg, -1);
        assert_eq!(DARK_THEME.assistant_bg, -1);
        assert_eq!(DARK_THEME.fg, 252);
        assert_eq!(DARK_THEME.user_bg, 59);
    }

    #[test]
    fn legacy_default_theme_matches_ts() {
        assert_eq!(DEFAULT_THEME.error, 160);
        assert_eq!(DEFAULT_THEME.success, 76);
        assert_eq!(DEFAULT_THEME.dim_fg, 245);
    }

    #[test]
    fn wrapped_styles_emit_exact_sequences() {
        assert_eq!(italic("x"), "\x1b[3mx\x1b[m");
        assert_eq!(underline("x"), "\x1b[4mx\x1b[m");
        assert_eq!(strikethrough("x"), "\x1b[9mx\x1b[m");
        assert_eq!(reset("x"), "\x1b[mx\x1b[m");
    }

    #[test]
    fn theme_default_is_the_dark_palette() {
        assert_eq!(Theme::default(), DARK_THEME);
    }

    #[test]
    fn chrome_legacy_freezes_the_ported_chrome_indices() {
        // The roles `Theme` has no slot for — the ones that must survive a
        // refactor untouched, because the byte-level tests were written to them.
        assert_eq!(Chrome::LEGACY.base, 245);
        assert_eq!(Chrome::LEGACY.muted, 241);
        assert_eq!(Chrome::LEGACY.info, 45);
        assert_eq!(Chrome::LEGACY.heading, 109);
        assert_eq!(Chrome::LEGACY.token, 71);
        assert_eq!(Chrome::LEGACY.warn, 226);
        assert_eq!(Chrome::LEGACY.highlight_bg, 237);
        assert_eq!(Chrome::LEGACY.mark_ok, 40);
        assert_eq!(Chrome::LEGACY.mark_no, 196);
        assert_eq!(Chrome::LEGACY.list_bg, 235);
    }

    #[test]
    fn chrome_from_the_default_palette_is_the_legacy_table() {
        assert_eq!(Chrome::from_theme(&DARK_THEME), Chrome::LEGACY);
        assert_eq!(Chrome::from_theme(&Theme::default()), Chrome::LEGACY);
    }

    #[test]
    fn chrome_from_a_theme_resolves_every_role() {
        let custom = Theme {
            bg: 231,
            dim: 111,
            fg: 112,
            accent: 113,
            md_link: 114,
            tool_title: 115,
            thinking_medium: 116,
            success: 117,
            error: 118,
            thinking_high: 119,
            border: 120,
            selected_bg: 121,
            selected_fg: 122,
            ..DARK_THEME
        };
        let chrome = Chrome::from_theme(&custom);
        assert_eq!(chrome.base, 111);
        assert_eq!(chrome.muted, 111);
        assert_eq!(chrome.text, 112);
        assert_eq!(chrome.accent, 113);
        assert_eq!(chrome.info, 114);
        assert_eq!(chrome.secondary, 114);
        assert_eq!(chrome.heading, 115);
        assert_eq!(chrome.thinking, 116);
        assert_eq!(chrome.success, 117);
        assert_eq!(chrome.token, 117);
        assert_eq!(chrome.mark_ok, 117);
        assert_eq!(chrome.error, 118);
        assert_eq!(chrome.mark_no, 118);
        assert_eq!(chrome.warn, 119);
        assert_eq!(chrome.border, 120);
        assert_eq!(chrome.selected_bg, 121);
        assert_eq!(chrome.highlight_bg, 121);
        assert_eq!(chrome.selected_fg, 122);
        assert_eq!(chrome.list_bg, 231);
    }

    /// A non-default palette that still inherits the terminal background, and
    /// a role left at "terminal default" — both have no `38;5;` spelling.
    #[test]
    fn chrome_keeps_the_list_background_and_clamps_terminal_default_roles() {
        let inherited = Theme {
            bg: -1,
            md_link: -1,
            dim: 111,
            ..DARK_THEME
        };
        let chrome = Chrome::from_theme(&inherited);
        assert_eq!(chrome.list_bg, Chrome::LEGACY.list_bg);
        assert_eq!(chrome.info, 0);
        assert_eq!(chrome.secondary, 0);
    }

    #[test]
    fn chrome_from_the_light_catalog_theme_differs_from_legacy() {
        let light = crate::themes::theme_by_id("light").expect("light is in the catalog");
        let chrome = Chrome::from_theme(&light);
        assert_ne!(chrome, Chrome::LEGACY);
        assert_eq!(chrome.text, 236); // light fg
        assert_eq!(chrome.selected_bg, 153); // light selected_bg
        assert_eq!(chrome.error, 160); // light error
    }

    #[test]
    fn index_clamps_colors_into_the_256_color_range() {
        assert_eq!(index(-1), 0);
        assert_eq!(index(0), 0);
        assert_eq!(index(255), 255);
        assert_eq!(index(4096), 255);
    }

    #[test]
    fn raw_styles_omit_trailing_reset() {
        assert_eq!(fg_raw(42, "x"), "\x1b[38;5;42mx");
        assert_eq!(bg_raw(42, "x"), "\x1b[48;5;42mx");
        assert_eq!(bold_raw("x"), "\x1b[1mx");
        assert_eq!(dim_raw("x"), "\x1b[2mx");
        assert_eq!(italic_raw("x"), "\x1b[3mx");
        assert_eq!(underline_raw("x"), "\x1b[4mx");
        assert_eq!(strikethrough_raw("x"), "\x1b[9mx");
        assert_eq!(reverse_raw("x"), "\x1b[7mx");
    }
}
