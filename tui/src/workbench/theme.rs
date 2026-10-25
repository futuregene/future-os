//! The workbench palette: two 256-colour themes and the semantic colours the
//! screen paints with.
//!
//! Deliberately separate from [`crate::theme`]: that one is the *inline* TUI's
//! palette (its chat rows, footer and menus), tuned for text appended into the
//! terminal's own scrollback. A full-screen surface draws whole panels with
//! backgrounds, so it needs a canvas colour and a panel colour of its own —
//! sharing the inline table would either wash the panels out or repaint the
//! conversation.

use crate::theme::{bold, fg};
use crate::utils::apply_background_to_line;

/// One palette. The numbers are 256-colour indices; the two built-ins are the
/// contract's `future-dark` / `future-light` (see `docs/tui/workbench-spec.md`
/// §3), so a colour may not drift between them without changing the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Identifier persisted by `/theme` (`future-dark` | `future-light`).
    pub id: &'static str,
    /// Canvas behind everything.
    pub bg: u8,
    /// Sidebar and input-box background.
    pub panel: u8,
    /// Borders, dividers, box rules.
    pub border: u8,
    /// Body text.
    pub fg: u8,
    /// Secondary notes, timestamps, hints.
    pub muted: u8,
    /// Brand, selection, progress.
    pub accent: u8,
    /// Links, the active tab.
    pub iris: u8,
    /// Reasoning, warnings.
    pub amber: u8,
    /// Success.
    pub green: u8,
    /// Errors, danger.
    pub red: u8,
    /// Sub-agents, skills.
    pub purple: u8,
}

pub const FUTURE_DARK: Palette = Palette {
    id: "future-dark",
    bg: 233,
    panel: 235,
    border: 238,
    fg: 252,
    muted: 245,
    accent: 80,
    iris: 111,
    amber: 179,
    green: 114,
    red: 203,
    purple: 141,
};

pub const FUTURE_LIGHT: Palette = Palette {
    id: "future-light",
    bg: 254,
    panel: 252,
    border: 246,
    fg: 236,
    muted: 243,
    accent: 30,
    iris: 62,
    amber: 130,
    green: 28,
    red: 124,
    purple: 91,
};

/// Every built-in, in the order `/theme` cycles them.
pub const PALETTES: [Palette; 2] = [FUTURE_DARK, FUTURE_LIGHT];

/// Resolve a persisted id; an unknown or absent id is the dark palette, the
/// same "fall back to the default" rule every other setting uses.
pub fn resolve(id: Option<&str>) -> Palette {
    match id {
        Some(id) => PALETTES
            .iter()
            .copied()
            .find(|palette| palette.id.eq_ignore_ascii_case(id))
            .unwrap_or(FUTURE_DARK),
        None => FUTURE_DARK,
    }
}

/// The palette after `current` in the catalogue (`/theme` with no argument).
pub fn next(current: &Palette) -> Palette {
    let index = PALETTES
        .iter()
        .position(|palette| palette.id == current.id)
        .unwrap_or(0);
    PALETTES[(index + 1) % PALETTES.len()]
}

impl Palette {
    pub fn text(&self, color: u8, text: &str) -> String {
        fg(color, text)
    }

    pub fn strong(&self, color: u8, text: &str) -> String {
        fg(color, &bold(text))
    }

    pub fn body(&self, text: &str) -> String {
        self.text(self.fg, text)
    }

    pub fn dim(&self, text: &str) -> String {
        self.text(self.muted, text)
    }

    /// Paint `text` and pad it with `color` out to `columns` — the one way a
    /// panel row gets its background.
    pub fn on(&self, color: u8, text: &str, columns: usize) -> String {
        apply_background_to_line(text, columns, color as i16)
    }

    /// A panel row on the panel background.
    pub fn panel_row(&self, text: &str, columns: usize) -> String {
        self.on(self.panel, text, columns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_built_ins_are_what_the_contract_names() {
        assert_eq!(FUTURE_DARK.id, "future-dark");
        assert_eq!(FUTURE_LIGHT.id, "future-light");
        assert_eq!(resolve(Some("future-light")).id, "future-light");
        assert_eq!(resolve(Some("FUTURE-LIGHT")).id, "future-light");
        // An unknown id — a palette another build wrote — is the default, not a
        // panic and not a half-themed screen.
        assert_eq!(resolve(Some("solarized")).id, "future-dark");
        assert_eq!(resolve(None).id, "future-dark");
    }

    #[test]
    fn cycling_wraps_and_comes_back() {
        assert_eq!(next(&FUTURE_DARK).id, "future-light");
        assert_eq!(next(&FUTURE_LIGHT).id, "future-dark");
        assert_eq!(next(&next(&FUTURE_DARK)).id, FUTURE_DARK.id);
    }

    #[test]
    fn the_two_palettes_never_share_a_canvas() {
        assert_ne!(FUTURE_DARK.bg, FUTURE_LIGHT.bg);
        assert_ne!(FUTURE_DARK.fg, FUTURE_LIGHT.fg);
        // …and every semantic colour is distinct inside one palette, or a
        // badge and an error would read as the same thing.
        let dark = [
            FUTURE_DARK.bg,
            FUTURE_DARK.panel,
            FUTURE_DARK.border,
            FUTURE_DARK.fg,
            FUTURE_DARK.muted,
            FUTURE_DARK.accent,
            FUTURE_DARK.iris,
            FUTURE_DARK.amber,
            FUTURE_DARK.green,
            FUTURE_DARK.red,
            FUTURE_DARK.purple,
        ];
        let mut sorted = dark.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), dark.len(), "a semantic colour is duplicated");
    }

    #[test]
    fn a_painted_row_keeps_its_width() {
        let row = FUTURE_DARK.panel_row(" 待办 ", 20);
        assert_eq!(crate::utils::visible_width(&row), 20);
    }
}
