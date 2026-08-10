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
    md_code_block: 142,        // #b5bd68 (green)
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
