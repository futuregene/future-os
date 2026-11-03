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
//!
//! Two rules decide the numbers, and `text_roles_clear_aa_on_their_own_canvas`
//! pins them:
//!
//! * **The canvas is ours, and it is the theme's own.** Every palette paints
//!   its own `bg` on every row, with `panel` the first 256-colour step that
//!   reads as raised. The canvas used to be whatever the reader's terminal
//!   profile painted — Ghostty ships `#282c34`, a mid slate — which made every
//!   dark theme look washed and dropped secondary text under 4.5:1. It then
//!   became one shared near-black for all twenty-two palettes, which made a
//!   catalogue of themes read as one theme with different accents. Each entry
//!   now carries the nearest 256-colour step to its own background, the same
//!   one the desktop app paints for that theme: `#121212`–`#303030` charcoal for
//!   the dark palettes, cream/tan/off-white for the light ones, near-black for
//!   `dark` itself and true black for `high-contrast`. A 256-colour terminal
//!   cannot express a *tinted* dark background — the cube's dark entries are
//!   saturated primaries (`#00005f`, `#005f5f`), not theme backgrounds — so the
//!   variation the table can carry is lightness and temperature.
//! * **Every text role clears WCAG AA (4.5:1)** against both the canvas and the
//!   panel it can sit on, and body text clears 7:1 on the canvas. `muted` is the
//!   role this bit hardest: at 2.9–3.8:1 several palettes' secondary text was
//!   decoration rather than text. Moving a canvas up the ramp costs contrast,
//!   so the palette that did so lifted the roles that no longer cleared the
//!   floor on it — the note beside each value records whose hue it is.
//!
//! The light palettes keep their hues: those numbers are shared with the desktop
//! app's palette table, so only `fg`/`muted` — the two purely neutral roles —
//! were lifted there. Where a light palette's accent colour is below AA on its
//! own canvas it is left alone and reported in `docs/tui/workbench-spec.md` §3
//! rather than silently changed here.

use crate::theme::{bold, fg};
use crate::utils::apply_background_to_line;
use crate::workbench::syntax;

/// One palette. The numbers are 256-colour indices; `future-dark` /
/// `future-light` are the contract's own (see `docs/tui/workbench-spec.md` §3),
/// so a colour may not drift from that table without changing the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Identifier persisted by `/theme`; the same id the inline TUI knows.
    pub id: &'static str,
    /// Canvas behind everything, painted on every row (`workbench::run`). Each
    /// palette carries its own — the nearest 256-colour step to the theme's real
    /// background (see the module note) — and every row is painted with it, so
    /// the reader's terminal profile cannot show through.
    pub bg: u8,
    /// Sidebar, menu and overlay background — the first 256-colour step above
    /// [`Palette::bg`] that reads as raised (> 1.1:1 against the canvas). Not
    /// the composer's: that box sits on the canvas like the chat around it, and
    /// its cursor is the focus signal.
    pub panel: u8,
    /// Borders, dividers, box rules.
    pub border: u8,
    /// Body text.
    pub fg: u8,
    /// Secondary notes, timestamps, hints. Never below 4.5:1 on the canvas or
    /// the panel: this is the role that made the palette unreadable.
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

/// The default palette: near-black canvas, teal accent, blue links.
pub const FUTURE_DARK: Palette = Palette {
    id: "dark",
    bg: 232, // #080808 — the brand canvas; the only palette left near-black
    panel: 234,
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
    bg: 254, // #e4e4e4 ≈ #f6f7f9 — a cool off-white, not pure white
    panel: 252,
    border: 246,
    fg: 236,
    muted: 240, // #585858 — 4.5:1 on the canvas (243 was 4.0:1)
    accent: 30,
    iris: 62,
    amber: 130,
    green: 28,
    red: 124,
    purple: 91,
};

/// Atom's One Dark: a neutral charcoal canvas, blue accent, cyan links.
///
/// Its hues mirror `crate::themes`' palette so the two surfaces read as one
/// product; `bg`/`panel` take the theme's own `#21252b` charcoal, because the
/// inline `one-dark` inherits the terminal and this surface paints its canvas.
pub const ONE_DARK: Palette = Palette {
    id: "one-dark",
    bg: 235, // #262626 ≈ #21252b — the theme's own charcoal
    panel: 236,
    border: 240,
    fg: 249,
    muted: 247, // #9e9e9e — 4.9:1 on the panel (245 was 3.8:1 on the raised canvas)
    accent: 75,
    iris: 73,
    amber: 180,
    green: 108,
    red: 174, // #d78787 — 4.8:1 on the panel (168 was 3.7:1)
    purple: 176,
};

/// One Light: the same hues on a soft off-white.
pub const ONE_LIGHT: Palette = Palette {
    id: "one-light",
    bg: 255, // #eeeeee ≈ #f0f0f1 — the theme's own canvas, one step off white
    panel: 231,
    border: 250,
    fg: 236,
    muted: 241, // #626262 — 6.1:1 on the panel (245 was 3.9:1 on white)
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
    bg: 16,     // #000000 — the one palette that goes to true black
    panel: 233, // #121212 — the first step that reads as raised on true black (17 is the cube's navy)
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
    bg: 235, // #262626 ≈ #21222c — the theme's own charcoal
    panel: 236,
    border: 239,
    fg: 255,
    muted: 247, // #9e9e9e — 4.9:1 on the panel (245 was 3.8:1 on the raised canvas)
    accent: 141,
    iris: 117,
    amber: 228,
    green: 84,
    red: 204, // #ff5f87 — 4.6:1 on the panel (203 was 4.4:1)
    purple: 212,
};

/// Ayu, by Dmitri Voronianski.
pub const AYU: Palette = Palette {
    id: "ayu",
    bg: 235, // #262626 ≈ #1f2430 — the theme's own charcoal
    panel: 236,
    border: 240, // #585858 ≈ #505664
    fg: 251,     // #c6c6c6 ≈ #cbccc6
    muted: 247,  // #9e9e9e — 4.9:1 on the panel (245 was 3.8:1 on the raised canvas)
    accent: 221, // #ffd75f ≈ #ffcc66
    iris: 81,    // #5fd7ff ≈ #73d0ff
    amber: 222,  // #ffd787 ≈ #ffd580
    green: 150,  // #afd787 ≈ #bae67e
    red: 210,    // #ff8787 ≈ #f28779
    purple: 183, // #d7afff ≈ #d4bfff
};

/// Catppuccin — Mocha.
pub const CATPPUCCIN: Palette = Palette {
    id: "catppuccin",
    bg: 233, // #121212 ≈ #11111b — the theme's own near-black
    panel: 235,
    border: 239, // #4e4e4e ≈ #45475a
    fg: 189,     // #d7d7ff ≈ #cdd6f4
    muted: 246,  // #949494 — 5.0:1 on the panel (103 was 4.4:1)
    accent: 111, // #87afff ≈ #89b4fa
    iris: 117,   // #87d7ff
    amber: 223,  // #ffd7af ≈ #f9e2af
    green: 151,  // #afd7af ≈ #a6e3a1
    red: 211,    // #ff87af ≈ #f38ba8
    purple: 218, // #ffafd7 ≈ #f5c2e7
};

/// Catppuccin — Latte.
pub const CATPPUCCIN_LIGHT: Palette = Palette {
    id: "catppuccin-light",
    bg: 255,     // #eeeeee ≈ #e8ebf1
    panel: 231,  // #ffffff ≈ #ffffff
    border: 252, // #d0d0d0 ≈ #ccd0da
    fg: 239,     // #4e4e4e ≈ #474962
    muted: 242,  // #6c6c6c — 4.5:1 on the canvas (103 was 3.0:1)
    accent: 62,  // #5f5fd7 ≈ #3b5bdb
    iris: 31,    // #0087af
    amber: 94,   // #875f00 ≈ #9b6200
    green: 28,   // #008700 ≈ #2f7d1f
    red: 125,    // #af005f ≈ #c10f33
    purple: 99,  // #875fff ≈ #8839ef
};

/// Everforest, by sainnhe.
pub const EVERFOREST: Palette = Palette {
    id: "everforest",
    bg: 236, // #303030 ≈ #2b3339 — the theme's own warm charcoal
    panel: 237,
    border: 59,  // #5f5f5f ≈ #586260
    fg: 188,     // #d7d7d7 ≈ #e2dbcb
    muted: 248,  // #a8a8a8 — 4.8:1 on the panel (245 was 3.3:1 on the raised canvas)
    accent: 109, // #87afaf
    iris: 108,   // #87af87
    amber: 180,  // #d7af87 ≈ #dbbc7f
    green: 144,  // #afaf87 ≈ #a7c080
    red: 210,    // #ff8787 — 4.9:1 on the panel (174 was 4.2:1)
    purple: 176, // #d787d7 — 4.5:1 on the panel (175 was 4.3:1)
};

/// Everforest — Light.
pub const EVERFOREST_LIGHT: Palette = Palette {
    id: "everforest-light",
    bg: 230,     // #ffffd7 ≈ #f3efdf — the theme's own cream
    panel: 254,  // #e4e4e4 — a grey step below the cream canvas
    border: 188, // #d7d7d7 ≈ #e0dccc
    fg: 239,     // #4e4e4e ≈ #414c51
    muted: 241,  // #626262 — 4.8:1 on the panel (242 was 4.1:1)
    accent: 58,  // #5f5f00 ≈ #627300
    iris: 66,    // #5f8787 ≈ #377483
    amber: 94,   // #875f00 ≈ #8e6506
    green: 64,   // #5f8700 ≈ #637400
    red: 131,    // #af5f5f ≈ #c33d32
    purple: 132, // #af5f87 ≈ #b0568c
};

/// Gruvbox, by Pavel Pertsev.
pub const GRUVBOX: Palette = Palette {
    id: "gruvbox",
    bg: 234, // #1c1c1c ≈ #1d2021 — the theme's own near-black
    panel: 235,
    border: 239, // #4e4e4e ≈ #504945
    fg: 187,     // #d7d7af ≈ #ebdbb2
    muted: 138,  // #af8787 ≈ #a89984
    accent: 108, // #87af87 ≈ #83a598
    iris: 73,    // #5fafaf ≈ #83a598, lifted to 4.5:1 (66 = 3.8:1)
    amber: 214,  // #ffaf00 ≈ #fabd2f
    green: 142,  // #afaf00 ≈ #b8bb26
    red: 209,    // #ff875f ≈ #fb7564
    purple: 174, // #d78787 ≈ #d3869b
};

/// Gruvbox — Light.
pub const GRUVBOX_LIGHT: Palette = Palette {
    id: "gruvbox-light",
    bg: 223,     // #ffd7af ≈ #f2e5bc
    panel: 230,  // #ffffd7 ≈ #fbf1c7
    border: 187, // #d7d7af ≈ #d5c4a1
    fg: 237,     // #3a3a3a ≈ #3c3836
    muted: 241,  // #626262 — 4.5:1 on the canvas (242 was 4.3:1)
    accent: 24,  // #005f87 ≈ #076678
    iris: 66,    // #5f8787
    amber: 94,   // #875f00 ≈ #945e05
    green: 58,   // #5f5f00 ≈ #4f6a12
    red: 124,    // #af0000 ≈ #9d0006
    purple: 95,  // #875f5f ≈ #8f3f71
};

/// Kanagawa, by rebelot.
pub const KANAGAWA: Palette = Palette {
    id: "kanagawa",
    bg: 234, // #1c1c1c ≈ #1f1f28 — the theme's own near-black
    panel: 235,
    border: 240, // #585858 ≈ #585653
    fg: 187,     // #d7d7af ≈ #dcd7ba
    muted: 246,  // #949494 — 5.0:1 on the panel (245 was 4.4:1)
    accent: 110, // #87afd7 ≈ #7e9cd8
    iris: 116,   // #87d7d7
    amber: 180,  // #d7af87 ≈ #e6c384
    green: 107,  // #87af5f ≈ #98bb6c
    red: 174,    // #d78787 — 5.6:1 on the panel (168 was 4.2:1)
    purple: 140, // #af87d7 — 5.2:1 on the panel (103 was 4.4:1)
};

/// Nightfox, by EdenEast.
pub const NIGHTFOX: Palette = Palette {
    id: "nightfox",
    bg: 235, // #262626 ≈ #192330 — the theme's own slate charcoal
    panel: 236,
    border: 59,  // #5f5f5f ≈ #4e5a67
    fg: 252,     // #d0d0d0 ≈ #cdcecf
    muted: 247,  // #9e9e9e — 4.9:1 on the panel (245 was 3.8:1)
    accent: 74,  // #5fafd7 ≈ #719cd6
    iris: 81,    // #5fd7ff
    amber: 180,  // #d7af87 ≈ #dbc074
    green: 108,  // #87af87 ≈ #81b29a
    red: 174,    // #d78787 ≈ #d17b8c
    purple: 140, // #af87d7 ≈ #9d79d6
};

/// Nord, by Arctic Ice Studio.
pub const NORD: Palette = Palette {
    id: "nord",
    bg: 236, // #303030 ≈ #2e3440 — the theme's own slate charcoal
    panel: 237,
    border: 240, // #585858 ≈ #4c566a
    fg: 255,     // #eeeeee ≈ #eceff4
    muted: 109,  // #87afaf ≈ #9aa6bb
    accent: 110, // #87afd7 ≈ #88c0d0
    iris: 146,   // #afafd7 ≈ #a3c0dd
    amber: 186,  // #d7d787 ≈ #ebcb8b
    green: 144,  // #afaf87 ≈ #a3be8c
    red: 210,    // #ff8787 — 4.9:1 on the panel (138 was 3.6:1)
    purple: 182, // #d7afd7 — 5.7:1 on the panel (139 was 3.7:1)
};

/// Rosé Pine — Main.
pub const ROSE_PINE: Palette = Palette {
    id: "rose-pine",
    bg: 234, // #1c1c1c ≈ #191724 — the theme's own near-black
    panel: 235,
    border: 238, // #444444 ≈ #393552
    fg: 189,     // #d7d7ff ≈ #e0def4
    muted: 146,  // #afafd7 — 6.9:1 on the panel (103 was 4.4:1)
    accent: 182, // #d7afd7 ≈ #c4a7e7
    iris: 74,    // #5fafd7 ≈ #6cb2cc
    amber: 216,  // #ffaf87 ≈ #f6c177
    green: 152,  // #afd7d7 ≈ #9ccfd8
    red: 174,    // #d78787 — 5.6:1 on the panel (168 was 4.2:1)
    purple: 141, // #af87ff
};

/// Rosé Pine — Dawn.
pub const ROSE_PINE_LIGHT: Palette = Palette {
    id: "rose-pine-light",
    bg: 255,     // #eeeeee ≈ #faf4ed
    panel: 231,  // #ffffff ≈ #fffaf3
    border: 253, // #dadada ≈ #e4dcd8
    fg: 239,     // #4e4e4e ≈ #4f4a6e, 7:1 on the canvas (240 was 6.6:1)
    muted: 242,  // #6c6c6c — 4.5:1 on the canvas (245 was 3.9:1)
    accent: 96,  // #875f87 ≈ #76608e
    iris: 60,    // #5f5f87 ≈ #3f7180
    amber: 94,   // #875f00 ≈ #9a5f10
    green: 24,   // #005f87 ≈ #286983
    red: 131,    // #af5f5f ≈ #a4475e
    purple: 99,  // #875fff
};

/// Solarized — Dark.
pub const SOLARIZED: Palette = Palette {
    id: "solarized",
    bg: 234, // #1c1c1c ≈ #002b36 — the theme's own near-black (the cube has no dark teal)
    panel: 235,
    border: 23,  // #005f5f ≈ #12505f
    fg: 254,     // #e4e4e4 ≈ #eee8d5
    muted: 247,  // #9e9e9e ≈ #8ba09c
    accent: 74,  // #5fafd7 ≈ #5fb3e8
    iris: 36,    // #00af87
    amber: 179,  // #d7af5f ≈ #e2c261
    green: 143,  // #afaf5f ≈ #b5bb63
    red: 210,    // #ff8787 ≈ #f08a7f
    purple: 104, // #8787d7 — 4.7:1 on the panel (103 was 4.4:1)
};

/// Solarized — Light.
pub const SOLARIZED_LIGHT: Palette = Palette {
    id: "solarized-light",
    bg: 254,     // #e4e4e4 ≈ #f4ecd8
    panel: 230,  // #ffffd7 ≈ #fdf6e3
    border: 187, // #d7d7af ≈ #ded6bf
    fg: 236,     // #303030 ≈ #073642
    muted: 241,  // #626262 — 4.5:1 on the canvas (244 was 3.9:1)
    accent: 25,  // #005faf ≈ #1e6fa8
    iris: 30,    // #008787
    amber: 94,   // #875f00 ≈ #8b6a00
    green: 58,   // #5f5f00 ≈ #5f6f00
    red: 124,    // #af0000 ≈ #b52a27
    purple: 61,  // #5f5faf
};

/// Tokyo Night, by enkia.
pub const TOKYO_NIGHT: Palette = Palette {
    id: "tokyo-night",
    bg: 234, // #1c1c1c ≈ #1a1b26 — the theme's own near-black
    panel: 235,
    border: 59,  // #5f5f5f ≈ #4f546f
    fg: 153,     // #afd7ff ≈ #c0caf5
    muted: 146,  // #afafd7 — 6.9:1 on the panel (103 was 4.4:1)
    accent: 111, // #87afff ≈ #88acf8
    iris: 117,   // #87d7ff
    amber: 179,  // #d7af5f ≈ #e0af68
    green: 149,  // #afd75f ≈ #9ece6a
    red: 210,    // #ff8787 ≈ #f8849a
    purple: 141, // #af87ff ≈ #bb9af7
};

/// Vesper, by Rauno Freiberg.
pub const VESPER: Palette = Palette {
    id: "vesper",
    bg: 233, // #121212 ≈ #101010 — the theme's own near-black
    panel: 235,
    border: 239, // #4e4e4e ≈ #4e4e4e
    fg: 188,     // #d7d7d7 ≈ #d7d7d7
    muted: 246,  // #949494 — 5.0:1 on the panel (245 was 4.4:1)
    accent: 222, // #ffd787 ≈ #ffc799
    iris: 116,   // #87d7d7 ≈ #8fc8e0
    amber: 215,  // #ffaf5f
    green: 150,  // #afd787 ≈ #a8d582
    red: 210,    // #ff8787 ≈ #ff8080
    purple: 182, // #d7afd7 ≈ #d4a9e8
};

/// Every built-in, in `/theme` picker order — the same ids and the same order
/// as [`crate::themes::THEME_SPECS`], default first.
pub const PALETTES: [Palette; 22] = [
    FUTURE_DARK,
    FUTURE_LIGHT,
    ONE_DARK,
    ONE_LIGHT,
    HIGH_CONTRAST,
    DRACULA,
    AYU,
    CATPPUCCIN,
    CATPPUCCIN_LIGHT,
    EVERFOREST,
    EVERFOREST_LIGHT,
    GRUVBOX,
    GRUVBOX_LIGHT,
    KANAGAWA,
    NIGHTFOX,
    NORD,
    ROSE_PINE,
    ROSE_PINE_LIGHT,
    SOLARIZED,
    SOLARIZED_LIGHT,
    TOKYO_NIGHT,
    VESPER,
];

/// Ids written by workbench builds before the catalogue was shared: they name
/// the same two palettes under their old spelling.
const LEGACY_IDS: [(&str, &str); 2] = [("future-dark", "dark"), ("future-light", "light")];

/// The RGB a 256-colour index paints.
///
/// The table is not a number line: `0..16` are the named colours, `16..232` a
/// 6×6×6 cube whose levels jump by forty, and `232..256` a grey ramp of ten-RGB
/// steps. Stepping from one index to the next therefore changes brightness by
/// anything from nothing to eighty, which is why anything that has to reason
/// about *lightness* (a selection band, a shadow) converts through here first.
pub fn rgb(index: u8) -> (u8, u8, u8) {
    /// The sixteen named colours, in the xterm order.
    const NAMED: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (128, 0, 0),
        (0, 128, 0),
        (128, 128, 0),
        (0, 0, 128),
        (128, 0, 128),
        (0, 128, 128),
        (192, 192, 192),
        (128, 128, 128),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (0, 0, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    /// The cube's six levels.
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match index {
        0..=15 => NAMED[index as usize],
        232..=255 => {
            let grey = 8 + 10 * (index - 232);
            (grey, grey, grey)
        }
        cube => {
            let cube = cube - 16;
            (
                LEVELS[(cube / 36) as usize],
                LEVELS[((cube % 36) / 6) as usize],
                LEVELS[(cube % 6) as usize],
            )
        }
    }
}

/// Perceived brightness of a colour, 0–255 (Rec. 601 weights).
pub fn luma(index: u8) -> u8 {
    let (r, g, b) = rgb(index);
    ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000) as u8
}

/// Is this a light canvas — text dark on light?
///
/// Read off the canvas's brightness rather than its id, so a palette added
/// later is classified without being named here.
pub fn is_light(palette: &Palette) -> bool {
    luma(palette.bg) > 127
}

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

    /// The colour a syntax token is painted in.
    ///
    /// The palette's semantic colours *are* the syntax palette, which is what
    /// keeps highlighted code inside the theme the reader chose: a light theme
    /// gets its own hues instead of a dark highlighter's. Two of the eight kinds
    /// borrow a colour from one they never share a language with — a key wears
    /// the function colour (keys belong to formats that have no functions), and
    /// an attribute is a keyword in bold.
    pub fn token(&self, token: syntax::Token, text: &str) -> String {
        match token {
            syntax::Token::Keyword => self.text(self.purple, text),
            syntax::Token::Type => self.text(self.iris, text),
            syntax::Token::Function => self.text(self.accent, text),
            syntax::Token::String => self.text(self.green, text),
            syntax::Token::Number => self.text(self.amber, text),
            syntax::Token::Comment => self.dim(text),
            syntax::Token::Attribute => self.strong(self.purple, text),
            syntax::Token::Key => self.text(self.accent, text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 256-colour table's three stretches: the named colours, a cube whose
    /// levels jump by forty, and a grey ramp of tens. Reading a palette's
    /// *brightness* off its indices is only possible through this conversion.
    #[test]
    fn the_256_colour_table_converts_back_to_rgb() {
        assert_eq!(rgb(0), (0, 0, 0));
        assert_eq!(rgb(15), (255, 255, 255));
        assert_eq!(rgb(232), (8, 8, 8), "the ramp starts just above black");
        assert_eq!(rgb(255), (238, 238, 238), "twenty-three ten-step greys");
        assert_eq!(rgb(231), (255, 255, 255), "the cube's white corner");
        assert_eq!(rgb(196), (255, 0, 0), "the cube's saturated red");
        assert_eq!(rgb(59), (95, 95, 95), "a cube grey");
        assert_eq!(luma(15), 255);
        assert_eq!(luma(0), 0);
    }

    /// A canvas is light or dark by its own brightness, not by its name: the
    /// near-white palettes sit on the cube (231) as well as on the ramp (254),
    /// and index arithmetic would misclassify one of the two.
    #[test]
    fn every_palette_is_classified_by_its_canvas() {
        for palette in PALETTES {
            assert_eq!(
                is_light(&palette),
                palette.id.contains("light"),
                "{} ({:?}, luma {})",
                palette.id,
                rgb(palette.bg),
                luma(palette.bg)
            );
        }
        assert!(!is_light(&FUTURE_DARK));
        assert!(!is_light(&HIGH_CONTRAST));
    }

    #[test]
    fn the_catalogue_is_exactly_the_themes_the_inline_ui_offers() {
        let ids: Vec<&str> = PALETTES.iter().map(|palette| palette.id).collect();
        let specs: Vec<&str> = crate::themes::THEME_SPECS
            .iter()
            .map(|spec| spec.id)
            .collect();
        // One `themeId` key for both surfaces: the ids and their order are the
        // catalogue's, so a choice made in one surface means the same in the
        // other.
        assert_eq!(ids, specs);
        assert_eq!(ids.len(), 22);
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
        assert_eq!(resolve(Some("solarized-dark")).id, "dark");
        assert_eq!(resolve(None).id, "dark");
        assert!(!is_known("solarized-dark"));
        assert_eq!(label("solarized-dark"), "Solarized Dark");
    }

    #[test]
    fn cycling_wraps_and_comes_back() {
        assert_eq!(next(&FUTURE_DARK).id, "light");
        assert_eq!(next(&DRACULA).id, AYU.id);
        assert_eq!(next(&VESPER).id, FUTURE_DARK.id);
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

    // ─── Contrast ───────────────────────────────────────────────────────────

    /// xterm-256 index → sRGB, from the documented palette (6×6×6 cube with
    /// levels 0/95/135/175/215/255 above 16, grays from 232, standard xterm
    /// values for the 16 system colours).
    fn xterm_rgb(index: u8) -> (f64, f64, f64) {
        const CUBE: [f64; 6] = [0.0, 95.0, 135.0, 175.0, 215.0, 255.0];
        const SYSTEM: [(u8, u8, u8); 16] = [
            (0, 0, 0),
            (128, 0, 0),
            (0, 128, 0),
            (128, 128, 0),
            (0, 0, 128),
            (128, 0, 128),
            (0, 128, 128),
            (192, 192, 192),
            (128, 128, 128),
            (255, 0, 0),
            (0, 255, 0),
            (255, 255, 0),
            (0, 0, 255),
            (255, 0, 255),
            (0, 255, 255),
            (255, 255, 255),
        ];
        let (r, g, b) = match index {
            0..=15 => {
                let (r, g, b) = SYSTEM[index as usize];
                (r as f64, g as f64, b as f64)
            }
            16..=231 => {
                let i = index - 16;
                (
                    CUBE[(i / 36) as usize],
                    CUBE[((i % 36) / 6) as usize],
                    CUBE[(i % 6) as usize],
                )
            }
            _ => {
                let v = (8 + 10 * (index as i32 - 232)) as f64;
                (v, v, v)
            }
        };
        (r / 255.0, g / 255.0, b / 255.0)
    }

    /// WCAG 2.1 contrast ratio between two 256-colour indices.
    fn contrast_ratio(a: u8, b: u8) -> f64 {
        fn channel(v: f64) -> f64 {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        }
        fn luminance(index: u8) -> f64 {
            let (r, g, b) = xterm_rgb(index);
            0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
        }
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    /// Each palette paints its own canvas, and the catalogue is not flattened
    /// onto one near-black.
    ///
    /// The canvas used to be a single `232` for every dark palette — the fix for
    /// "every theme's background is not black enough", since before that it was
    /// the terminal's (`#282c34` on Ghostty), which no palette value could make
    /// black. Twenty-two palettes on one canvas then made the catalogue read as
    /// one theme with different accents, so each entry now carries the nearest
    /// 256-colour step to its own background (module note). This pins the
    /// spread: a palette added later brings its own canvas or fails here.
    #[test]
    fn every_palette_paints_its_own_canvas() {
        let mut dark: Vec<u8> = PALETTES
            .iter()
            .filter(|palette| !is_light(palette))
            .map(|palette| palette.bg)
            .collect();
        dark.sort_unstable();
        dark.dedup();
        assert!(
            dark.len() >= 5,
            "the dark palettes share one canvas again: {dark:?}"
        );
        let spread = i32::from(luma(*dark.last().unwrap())) - i32::from(luma(dark[0]));
        assert!(
            spread >= 32,
            "the dark canvases span only {spread} luma steps: {dark:?}"
        );
        for palette in PALETTES.iter() {
            if palette.id != FUTURE_DARK.id {
                assert_ne!(
                    palette.bg, FUTURE_DARK.bg,
                    "{} borrows the default palette's canvas",
                    palette.id
                );
            }
            assert!(
                contrast_ratio(palette.bg, palette.panel) > 1.1,
                "{}: the panel must be a visible step above the canvas",
                palette.id
            );
        }
        // The one palette that is deliberately true black, on a neutral panel:
        // `17` is the cube's navy, not a dark grey.
        assert_eq!(HIGH_CONTRAST.bg, 16);
        assert!(contrast_ratio(HIGH_CONTRAST.bg, HIGH_CONTRAST.panel) > 1.1);
    }

    /// Every text role has to be readable on both backgrounds it can sit on.
    /// Before this, `muted` measured 2.9:1 on `one-dark`'s and `dracula`'s own
    /// panel — decoration, not text.
    #[test]
    fn text_roles_clear_aa_on_their_own_canvas() {
        for palette in PALETTES {
            // The light palettes keep the desktop app's shared hues; where one of
            // those is below AA it is reported in the spec rather than changed
            // here, so only the two neutral roles are pinned for them.
            let light = palette.id == "light" || palette.id.ends_with("-light");
            let mut roles: Vec<(&str, u8, f64)> =
                vec![("fg", palette.fg, 7.0), ("muted", palette.muted, 4.5)];
            if !light {
                roles.extend([
                    ("accent", palette.accent, 4.5),
                    ("iris", palette.iris, 4.5),
                    ("amber", palette.amber, 4.5),
                    ("green", palette.green, 4.5),
                    ("red", palette.red, 4.5),
                    ("purple", palette.purple, 4.5),
                ]);
            }
            for (role, color, floor) in roles {
                let on_canvas = contrast_ratio(color, palette.bg);
                let on_panel = contrast_ratio(color, palette.panel);
                assert!(
                    on_canvas >= floor,
                    "{}: {role}={color} on the canvas {} is {on_canvas:.2} : 1, needs {floor}",
                    palette.id,
                    palette.bg
                );
                if role != "fg" {
                    assert!(
                        on_panel >= floor,
                        "{}: {role}={color} on the panel {} is {on_panel:.2} : 1, needs {floor}",
                        palette.id,
                        palette.panel
                    );
                }
            }
        }
    }
}
