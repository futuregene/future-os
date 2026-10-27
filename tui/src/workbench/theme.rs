//! The workbench palette: the selectable 256-colour themes and the semantic
//! colours the screen paints with.
//!
//! Deliberately separate from [`crate::theme`]: that one is the *inline* TUI's
//! palette (its chat rows, footer and menus), tuned for text appended into the
//! terminal's own scrollback. A full-screen surface draws whole panels with
//! backgrounds, so it needs a canvas colour and a panel colour of its own —
//! sharing the inline table would either wash the panels out or repaint the
//! conversation.
//!
//! What the two *do* share is the **id space**. [`PALETTES`] carries exactly the
//! ids of [`crate::themes::THEME_SPECS`], in the same order, because
//! `settings.json` → `themeId` is one key for both surfaces: a reader who picks
//! `dracula` in the workbench's `/theme` picker and then runs the legacy inline
//! UI gets Dracula there too. The colours are still two tables (the inline themes
//! inherit the terminal background; a workbench canvas may not), so the ids are
//! asserted against the catalogue rather than derived from it.

use crate::theme::{bold, fg};
use crate::utils::apply_background_to_line;

/// One palette. The numbers are 256-colour indices; `future-dark` /
/// `future-light` are the contract's own (see `docs/tui/workbench-spec.md` §3),
/// so a colour may not drift from that table without changing the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Identifier persisted by `/theme`; the same id the inline TUI knows.
    pub id: &'static str,
    /// Canvas behind everything.
    pub bg: u8,
    /// Sidebar, menu and overlay background. Not the composer's: that box sits
    /// on the canvas like the chat around it, and its cursor is the focus
    /// signal.
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

/// The default canvas: the workbench's dark navy-teal.
pub const FUTURE_DARK: Palette = Palette {
    id: "dark",
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

/// The light canvas the dark palette was re-tuned onto.
pub const FUTURE_LIGHT: Palette = Palette {
    id: "light",
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

/// Atom's One Dark: a neutral charcoal canvas, blue accent, cyan links.
///
/// `bg`/`panel` are its own (the inline `one-dark` inherits `#282c34`; a
/// full-screen canvas paints an explicit step instead), the rest mirrors
/// `crate::themes`' palette so the two surfaces read as one product.
pub const ONE_DARK: Palette = Palette {
    id: "one-dark",
    bg: 236,
    panel: 237,
    border: 240,
    fg: 249,
    muted: 243,
    accent: 75,
    iris: 73,
    amber: 180,
    green: 108,
    red: 168,
    purple: 176,
};

/// One Light: the same hues on white.
pub const ONE_LIGHT: Palette = Palette {
    id: "one-light",
    bg: 231,
    panel: 254,
    border: 250,
    fg: 236,
    muted: 245,
    accent: 25,
    iris: 30,
    amber: 130,
    green: 28,
    red: 160,
    purple: 91,
};

/// Black canvas, saturated foregrounds: bright rooms, projectors, bad panels.
pub const HIGH_CONTRAST: Palette = Palette {
    id: "high-contrast",
    bg: 16,
    panel: 17,
    border: 250,
    fg: 231,
    muted: 246,
    accent: 51,
    iris: 123,
    amber: 226,
    green: 46,
    red: 196,
    purple: 213,
};

/// Dracula: purple accent, cyan links, warm chroma.
pub const DRACULA: Palette = Palette {
    id: "dracula",
    bg: 236,
    panel: 237,
    border: 239,
    fg: 255,
    muted: 243,
    accent: 141,
    iris: 117,
    amber: 228,
    green: 84,
    red: 203,
    purple: 212,
};

/// Every built-in, in `/theme` picker order — the same six ids and the same
/// order as [`crate::themes::THEME_SPECS`], default first.
pub const PALETTES: [Palette; 6] = [
    FUTURE_DARK,
    FUTURE_LIGHT,
    ONE_DARK,
    ONE_LIGHT,
    HIGH_CONTRAST,
    DRACULA,
];

/// Ids written by workbench builds before the catalogue was shared: they name
/// the same two palettes under their old spelling.
const LEGACY_IDS: [(&str, &str); 2] = [("future-dark", "dark"), ("future-light", "light")];

/// Canonicalise a persisted id: trim, lower-case, `_` → `-`, then follow a
/// legacy alias. The two surfaces read one `themeId`, so a value an older build
/// (or the inline TUI's `One_Dark`) wrote has to resolve to the palette it meant.
fn canonical(id: &str) -> String {
    let id = id.trim().to_ascii_lowercase().replace('_', "-");
    LEGACY_IDS
        .iter()
        .find(|(legacy, _)| *legacy == id)
        .map(|(_, canonical)| (*canonical).to_string())
        .unwrap_or(id)
}

/// Resolve a persisted id. With no explicit choice, follow the terminal's
/// system light/dark hint; malformed or unavailable hints use the dark default.
pub fn resolve(id: Option<&str>) -> Palette {
    match id.map(canonical) {
        Some(id) => PALETTES
            .iter()
            .copied()
            .find(|palette| palette.id == id)
            .unwrap_or(FUTURE_DARK),
        None => PALETTES
            .iter()
            .copied()
            .find(|palette| {
                palette.id == system_theme_id(std::env::var("COLORFGBG").ok().as_deref())
            })
            .unwrap_or(FUTURE_DARK),
    }
}

/// `COLORFGBG` is the portable, query-free system light/dark hint supported by
/// terminals such as xterm. Unknown or malformed values keep the dark default.
fn system_theme_id(colorfgbg: Option<&str>) -> &'static str {
    let Some(background) = colorfgbg
        .and_then(|value| value.rsplit(';').next())
        .and_then(|value| value.trim().parse::<u8>().ok())
    else {
        return "dark";
    };
    // The conventional xterm default is 0 on 15. The xterm 256-colour cube's
    // entries 0..=7 are dark and 8..=15 are bright; high cube/grey entries are
    // light enough that a dark foreground is the useful system match.
    if background == 15 || background >= 231 || (8..=15).contains(&background) {
        "light"
    } else {
        "dark"
    }
}

/// Is `id` a palette this build knows (after canonicalisation)?
pub fn is_known(id: &str) -> bool {
    let id = canonical(id);
    PALETTES.iter().any(|palette| palette.id == id)
}

/// The palette after `current` in the catalogue (`/theme` with no argument).
pub fn next(current: &Palette) -> Palette {
    let index = PALETTES
        .iter()
        .position(|palette| palette.id == current.id)
        .unwrap_or(0);
    PALETTES[(index + 1) % PALETTES.len()]
}

/// A picker row's name: the shared catalogue's label (`One Dark`), else the id
/// spelled out, so a palette this build knows but the catalogue does not still
/// reads as words instead of a slug.
pub fn label(id: &str) -> String {
    crate::themes::spec_by_id(id)
        .map(|spec| spec.label.to_string())
        .unwrap_or_else(|| {
            id.split('-')
                .map(|part| {
                    let mut chars = part.chars();
                    match chars.next() {
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
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
    fn the_catalogue_is_the_six_themes_the_inline_ui_offers() {
        let ids: Vec<&str> = PALETTES.iter().map(|palette| palette.id).collect();
        let specs: Vec<&str> = crate::themes::THEME_SPECS
            .iter()
            .map(|spec| spec.id)
            .collect();
        // One `themeId` key for both surfaces: the ids and their order are the
        // catalogue's, so a choice made in one surface means the same in the
        // other.
        assert_eq!(ids, specs);
        assert_eq!(ids.len(), 6);
        assert_eq!(PALETTES[0].id, "dark", "the default is first");
    }

    #[test]
    fn every_palette_resolves_back_to_itself_under_any_spelling() {
        for (index, palette) in PALETTES.iter().enumerate() {
            assert_eq!(resolve(Some(palette.id)).id, palette.id);
            assert_eq!(resolve(Some(&palette.id.to_uppercase())).id, palette.id);
            assert_eq!(resolve(Some(&palette.id.replace('-', "_"))).id, palette.id);
            assert!(is_known(palette.id));
            // The picker's own labels never fall back to a slug.
            assert!(!label(palette.id).contains('-'), "{}", palette.id);
            assert_eq!(
                next(&PALETTES[index]).id,
                PALETTES[(index + 1) % PALETTES.len()].id
            );
        }
    }

    #[test]
    fn system_theme_uses_colorfgbg_and_has_safe_fallbacks() {
        assert_eq!(system_theme_id(Some("0;15")), "light");
        assert_eq!(system_theme_id(Some("15;0")), "dark");
        assert_eq!(system_theme_id(Some("0;255")), "light");
        assert_eq!(system_theme_id(Some("bad")), "dark");
        assert_eq!(system_theme_id(None), "dark");
    }

    #[test]
    fn the_ids_workbench_builds_already_wrote_still_resolve() {
        assert_eq!(resolve(Some("future-dark")).id, "dark");
        assert_eq!(resolve(Some("future-light")).id, "light");
        assert_eq!(resolve(Some(" Future_Light ")).id, "light");
    }

    #[test]
    fn an_unknown_id_is_the_default_and_never_a_panic() {
        // A palette another build wrote is not a half-themed screen.
        assert_eq!(resolve(Some("solarized")).id, "dark");
        assert_eq!(resolve(None).id, "dark");
        assert!(!is_known("solarized"));
        assert_eq!(label("solarized-dark"), "Solarized Dark");
    }

    #[test]
    fn cycling_wraps_and_comes_back() {
        assert_eq!(next(&FUTURE_DARK).id, "light");
        assert_eq!(next(&DRACULA).id, FUTURE_DARK.id);
    }

    #[test]
    fn every_palette_has_a_distinct_canvas_and_panel_and_semantics() {
        // A canvas equal to its panel would make the sidebar invisible; equal
        // semantic colours would make a badge and an error read as one thing.
        for palette in PALETTES {
            assert_ne!(palette.bg, palette.panel, "{}", palette.id);
            let mut semantics = vec![
                palette.bg,
                palette.panel,
                palette.border,
                palette.fg,
                palette.muted,
                palette.accent,
                palette.iris,
                palette.amber,
                palette.green,
                palette.red,
                palette.purple,
            ];
            let total = semantics.len();
            semantics.sort_unstable();
            semantics.dedup();
            assert_eq!(semantics.len(), total, "{} reuses a colour", palette.id);
        }
    }

    #[test]
    fn a_painted_row_keeps_its_width() {
        let row = FUTURE_DARK.panel_row(" 待办 ", 20);
        assert_eq!(crate::utils::visible_width(&row), 20);
    }
}
