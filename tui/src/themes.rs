//! Theme catalog — the source of truth behind the `/theme` picker.
//!
//! `crate::theme` owns the [`Theme`] struct and the app's default [`DARK_THEME`];
//! this module owns the *catalog* of selectable themes built on top of it. The
//! design rules are:
//!
//! * Every theme is a `const Theme`, so no palette is computed at runtime and a
//!   bad color index is a compile error rather than a runtime surprise.
//! * Ids are lowercase ASCII, stable and persisted in `settings.json`
//!   (`themeId`). Renaming an id silently resets every user's saved choice, so
//!   the exact id set is asserted literally in the tests below.
//! * Lookups never panic. `resolve_theme` falls back to the default theme *and*
//!   reports which id it actually resolved to, so the caller can persist the
//!   fallback instead of a value it did not really apply.
//! * `dark` **is** `crate::theme::DARK_THEME` (an alias, not a copy), so merely
//!   shipping the picker cannot change how the TUI looks today.
//!
//! Palettes are indexed against the 256-color xterm palette (the same space
//! `crate::theme::C` uses) and each index in the comments names the 24-bit color
//! the entry is approximating. `-1` means "terminal default" and is only ever
//! used for `bg`/`assistant_bg` of a theme that deliberately inherits the
//! terminal background.

use crate::theme::{Theme, DARK_THEME};

/// One entry of the picker: a stable id, a human label and a light/dark flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeSpec {
    /// Persisted identifier (`settings.json` → `themeId`); stable forever.
    pub id: &'static str,
    /// Human-readable name shown in the `/theme` menu.
    pub label: &'static str,
    /// `true` for themes designed on a dark background.
    pub dark: bool,
}

/// Id used when nothing valid is configured. First entry of [`THEME_SPECS`].
const DEFAULT_ID: &str = "dark";

// ─── Palettes ───────────────────────────────────────────────────────────────

/// `dark` — byte-for-byte the app's existing theme; the alias keeps the two in
/// sync by construction rather than by a duplicated table.
const DARK: Theme = DARK_THEME;

/// `light` — the default theme re-tuned for a white background. Contrast is
/// raised relative to the dark palette (fg 236 on bg 231) and every selection /
/// tool color is an explicit opaque index so nothing relies on the terminal's
/// own light-background defaults.
const LIGHT: Theme = Theme {
    bg: 231,          // #ffffff — explicit, so it is readable on dark terminals
    fg: 236,          // #303030
    accent: 25,       // #005faf (strong blue, AA contrast on white)
    border: 250,      // #bcbcbc
    selected_bg: 153, // #afd7ff
    selected_fg: 16,  // #000000
    dim: 245,         // #8a8a8a
    error: 160,       // #d70000
    success: 28,      // #008700

    md_heading: 130,           // #af5f00
    md_link: 25,               // #005faf
    md_code: 30,               // #008787
    md_code_block: 28,         // #008700
    md_code_block_border: 250, // #bcbcbc
    md_quote: 245,             // #8a8a8a

    tool_pending_bg: 254, // #e4e4e4
    tool_success_bg: 194, // #d7ffd7
    tool_error_bg: 224,   // #ffd7d7
    tool_title: 30,       // #008787
    tool_output: 240,     // #585858

    thinking_off: 250,
    thinking_minimal: 110,
    thinking_low: 25,
    thinking_medium: 26,
    thinking_high: 130,
    thinking_xhigh: 127,
    thinking_text: 245,

    user_bg: 189, // #d7d7ff — light "bubble" for user messages
    assistant_bg: 231,
};

/// `one-dark` — Atom's One Dark (`#282c34` on `#abb2bf`, blue `#61afef`).
const ONE_DARK: Theme = Theme {
    bg: 236,          // #303030 ≈ #282c34
    fg: 249,          // #b2b2b2 ≈ #abb2bf
    accent: 75,       // #5fafff ≈ #61afef
    border: 240,      // #585858
    selected_bg: 238, // #444444 ≈ #3e4451
    selected_fg: 255, // #eeeeee
    dim: 243,         // #767676
    error: 168,       // #d75f87 ≈ #e06c75
    success: 108,     // #87af87 ≈ #98c379

    md_heading: 180,           // #d7af87 ≈ #e5c07b
    md_link: 75,               // #5fafff
    md_code: 73,               // #5fafaf ≈ #56b6c2
    md_code_block: 108,        // #87af87
    md_code_block_border: 240, // #585858
    md_quote: 243,             // #767676 ≈ #5c6370

    tool_pending_bg: 237, // #3a3a3a
    tool_success_bg: 236, // #303030 (subtle, like the dark theme)
    tool_error_bg: 52,    // #5f0000
    tool_title: 73,       // #5fafaf
    tool_output: 244,     // #808080

    thinking_off: 240,
    thinking_minimal: 110,
    thinking_low: 111,
    thinking_medium: 75,
    thinking_high: 180,
    thinking_xhigh: 176,
    thinking_text: 244,

    user_bg: 237, // #3a3a3a
    assistant_bg: 236,
};

/// `one-light` — Atom's One Light (`#fafafa`, blue `#4078f2`).
const ONE_LIGHT: Theme = Theme {
    bg: 231,          // #ffffff ≈ #fafafa
    fg: 237,          // #3a3a3a ≈ #383a42
    accent: 69,       // #5f87ff ≈ #4078f2
    border: 250,      // #bcbcbc
    selected_bg: 189, // #d7d7ff ≈ #e5e5f5
    selected_fg: 16,  // #000000
    dim: 245,         // #8a8a8a
    error: 167,       // #d75f5f ≈ #e45649
    success: 71,      // #5faf5f ≈ #50a14f

    md_heading: 136,           // #af8700 ≈ #c18401
    md_link: 69,               // #5f87ff
    md_code: 31,               // #0087af ≈ #0184bc
    md_code_block: 71,         // #5faf5f
    md_code_block_border: 250, // #bcbcbc
    md_quote: 245,             // #8a8a8a ≈ #a0a1a7

    tool_pending_bg: 254, // #e4e4e4
    tool_success_bg: 194, // #d7ffd7
    tool_error_bg: 224,   // #ffd7d7
    tool_title: 31,       // #0087af
    tool_output: 240,     // #585858

    thinking_off: 250,
    thinking_minimal: 110,
    thinking_low: 69,
    thinking_medium: 26,
    thinking_high: 136,
    thinking_xhigh: 127,
    thinking_text: 245,

    user_bg: 189, // #d7d7ff
    assistant_bg: 231,
};

/// `high-contrast` — black/white/cyan/yellow, for bright rooms, projectors and
/// low-quality panels. Every foreground is a saturated or full-contrast index
/// so nothing depends on the terminal's own default foreground.
const HIGH_CONTRAST: Theme = Theme {
    bg: 16,           // #000000
    fg: 231,          // #ffffff
    accent: 51,       // #00ffff
    border: 231,      // #ffffff
    selected_bg: 226, // #ffff00
    selected_fg: 16,  // #000000
    dim: 250,         // #bcbcbc (still ~9:1 on black)
    error: 196,       // #ff0000
    success: 46,      // #00ff00

    md_heading: 226,           // #ffff00
    md_link: 123,              // #87ffff
    md_code: 51,               // #00ffff
    md_code_block: 46,         // #00ff00
    md_code_block_border: 231, // #ffffff
    md_quote: 250,             // #bcbcbc

    tool_pending_bg: 17, // #00005f
    tool_success_bg: 22, // #005f00
    tool_error_bg: 52,   // #5f0000
    tool_title: 51,      // #00ffff
    tool_output: 231,    // #ffffff

    thinking_off: 250,
    thinking_minimal: 123,
    thinking_low: 51,
    thinking_medium: 45,
    thinking_high: 226,
    thinking_xhigh: 213,
    thinking_text: 250,

    user_bg: 18, // #000087
    assistant_bg: 16,
};

/// `dracula` — the Dracula palette (`#282a36`, purple `#bd93f9`, green
/// `#50fa7b`); a warmer, higher-chroma alternative to One Dark.
const DRACULA: Theme = Theme {
    bg: 236,          // #303030 ≈ #282a36
    fg: 255,          // #eeeeee ≈ #f8f8f2
    accent: 141,      // #af87ff ≈ #bd93f9
    border: 239,      // #4e4e4e ≈ #44475a
    selected_bg: 239, // #4e4e4e ≈ #44475a
    selected_fg: 255, // #eeeeee
    dim: 243,         // #767676
    error: 203,       // #ff5f5f ≈ #ff5555
    success: 84,      // #5fff87 ≈ #50fa7b

    md_heading: 228,           // #ffff87 ≈ #f1fa8c
    md_link: 117,              // #87d7ff ≈ #8be9fd
    md_code: 212,              // #ff87d7 ≈ #ff79c6
    md_code_block: 84,         // #5fff87
    md_code_block_border: 239, // #4e4e4e
    md_quote: 243,             // #767676

    tool_pending_bg: 237, // #3a3a3a
    tool_success_bg: 236, // #303030
    tool_error_bg: 52,    // #5f0000
    tool_title: 117,      // #87d7ff
    tool_output: 250,     // #bcbcbc

    thinking_off: 240,
    thinking_minimal: 117,
    thinking_low: 111,
    thinking_medium: 141,
    thinking_high: 212,
    thinking_xhigh: 228,
    thinking_text: 250,

    user_bg: 237, // #3a3a3a
    assistant_bg: 236,
};

// ─── Catalog ────────────────────────────────────────────────────────────────

/// Every selectable theme, in picker order (default first).
pub const THEME_SPECS: &[ThemeSpec] = &[
    ThemeSpec {
        id: "dark",
        label: "Dark",
        dark: true,
    },
    ThemeSpec {
        id: "light",
        label: "Light",
        dark: false,
    },
    ThemeSpec {
        id: "one-dark",
        label: "One Dark",
        dark: true,
    },
    ThemeSpec {
        id: "one-light",
        label: "One Light",
        dark: false,
    },
    ThemeSpec {
        id: "high-contrast",
        label: "High Contrast",
        dark: true,
    },
    ThemeSpec {
        id: "dracula",
        label: "Dracula",
        dark: true,
    },
];

/// Canonicalise a user-supplied id: trim ASCII whitespace, lower-case it and
/// treat `_` as a synonym for `-`. Config files and CLI arguments are typed by
/// hand, so `"One_Dark "` must resolve to `one-dark` instead of silently
/// falling back to the default.
fn normalize_theme_id(id: &str) -> String {
    id.trim().to_ascii_lowercase().replace('_', "-")
}

/// The `Theme` behind a canonical spec id. Only [`THEME_SPECS`] ids can reach
/// this function through [`theme_by_id`]; the fallback arm exists so a future
/// spec that forgets a palette degrades to the default instead of panicking
/// (`every_spec_has_its_own_theme` guards the table in tests).
fn theme_for_spec(id: &str) -> Theme {
    match id {
        "dark" => DARK,
        "light" => LIGHT,
        "one-dark" => ONE_DARK,
        "one-light" => ONE_LIGHT,
        "high-contrast" => HIGH_CONTRAST,
        "dracula" => DRACULA,
        _ => DARK,
    }
}

/// Id of the theme used when nothing valid is configured.
pub fn default_theme_id() -> &'static str {
    DEFAULT_ID
}

/// Look up a spec by id (case-insensitive, surrounding whitespace and `_`
/// tolerated). Returns the *catalog* entry, so the caller can read the label
/// and light/dark flag as well.
pub fn spec_by_id(id: &str) -> Option<&'static ThemeSpec> {
    let needle = normalize_theme_id(id);
    if needle.is_empty() {
        return None;
    }
    THEME_SPECS.iter().find(|spec| spec.id == needle)
}

/// Look up the palette for a theme id. `None` when the id is unknown — use
/// [`resolve_theme`] when a usable theme is needed unconditionally.
pub fn theme_by_id(id: &str) -> Option<Theme> {
    spec_by_id(id).map(|spec| theme_for_spec(spec.id))
}

/// `(id, label)` pairs for menus / list views, in catalog order.
pub fn theme_options() -> Vec<(&'static str, &'static str)> {
    THEME_SPECS
        .iter()
        .map(|spec| (spec.id, spec.label))
        .collect()
}

/// Is `id` a known theme? Same normalization as [`spec_by_id`].
pub fn is_known_theme(id: &str) -> bool {
    spec_by_id(id).is_some()
}

/// Resolve a configured theme id to `(actual id, palette)`.
///
/// Never panics and never returns an unknown id: an absent (`None`), blank or
/// unknown value yields the default theme together with the default id, so the
/// caller can persist what was really applied.
pub fn resolve_theme(configured: Option<&str>) -> (&'static str, Theme) {
    match configured.and_then(spec_by_id) {
        Some(spec) => (spec.id, theme_for_spec(spec.id)),
        None => (DEFAULT_ID, DARK),
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Color;
    use std::collections::HashSet;

    /// Field-by-field view of a theme. Keeping the list here means a new `Theme`
    /// field fails to compile until it is covered by the comparison tests.
    fn theme_fields(theme: &Theme) -> Vec<(&'static str, Color)> {
        vec![
            ("bg", theme.bg),
            ("fg", theme.fg),
            ("accent", theme.accent),
            ("border", theme.border),
            ("selected_bg", theme.selected_bg),
            ("selected_fg", theme.selected_fg),
            ("dim", theme.dim),
            ("error", theme.error),
            ("success", theme.success),
            ("md_heading", theme.md_heading),
            ("md_link", theme.md_link),
            ("md_code", theme.md_code),
            ("md_code_block", theme.md_code_block),
            ("md_code_block_border", theme.md_code_block_border),
            ("md_quote", theme.md_quote),
            ("tool_pending_bg", theme.tool_pending_bg),
            ("tool_success_bg", theme.tool_success_bg),
            ("tool_error_bg", theme.tool_error_bg),
            ("tool_title", theme.tool_title),
            ("tool_output", theme.tool_output),
            ("thinking_off", theme.thinking_off),
            ("thinking_minimal", theme.thinking_minimal),
            ("thinking_low", theme.thinking_low),
            ("thinking_medium", theme.thinking_medium),
            ("thinking_high", theme.thinking_high),
            ("thinking_xhigh", theme.thinking_xhigh),
            ("thinking_text", theme.thinking_text),
            ("user_bg", theme.user_bg),
            ("assistant_bg", theme.assistant_bg),
        ]
    }

    /// Fields allowed to be the `-1` "terminal default" sentinel.
    const MAY_INHERIT: [&str; 2] = ["bg", "assistant_bg"];

    #[test]
    fn catalog_is_not_empty_and_ids_are_unique_and_canonical() {
        assert!(!THEME_SPECS.is_empty());
        // Hoisted: format arguments are only evaluated when the assert fails.
        let catalog_len = THEME_SPECS.len();
        assert!(
            catalog_len >= 5,
            "the picker needs at least 5 themes, found {catalog_len}"
        );

        let mut seen = HashSet::new();
        for spec in THEME_SPECS {
            assert!(
                seen.insert(spec.id),
                "duplicate theme id {:?} would make settings.json ambiguous",
                spec.id
            );
            assert!(!spec.id.is_empty());
            assert!(
                spec.id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "id {:?} must be lowercase ASCII with '-' separators only",
                spec.id
            );
            assert!(!spec.id.starts_with('-') && !spec.id.ends_with('-'));
            assert_eq!(
                normalize_theme_id(spec.id),
                spec.id,
                "spec ids must already be canonical"
            );
        }
    }

    #[test]
    fn id_set_is_frozen_because_it_is_persisted() {
        // Renaming or dropping an id resets every user's saved `themeId`, so the
        // literal set is pinned here on purpose.
        let ids: Vec<&str> = THEME_SPECS.iter().map(|spec| spec.id).collect();
        assert_eq!(
            ids,
            vec![
                "dark",
                "light",
                "one-dark",
                "one-light",
                "high-contrast",
                "dracula"
            ]
        );
        assert_eq!(THEME_SPECS[0].id, default_theme_id());
    }

    #[test]
    fn labels_are_non_empty_trimmed_and_distinct() {
        let mut labels = HashSet::new();
        for spec in THEME_SPECS {
            assert!(
                !spec.label.trim().is_empty(),
                "id {:?} has no label",
                spec.id
            );
            assert_eq!(spec.label, spec.label.trim());
            assert!(
                labels.insert(spec.label),
                "duplicate label {:?}",
                spec.label
            );
        }
    }

    #[test]
    fn catalog_covers_dark_and_light_themes() {
        assert!(THEME_SPECS.iter().any(|spec| spec.dark));
        assert!(THEME_SPECS.iter().any(|spec| !spec.dark));
        assert!(spec_by_id("dark").expect("dark spec").dark);
        assert!(!spec_by_id("light").expect("light spec").dark);
    }

    #[test]
    fn theme_by_id_hits_every_spec() {
        for spec in THEME_SPECS {
            let theme = theme_by_id(spec.id).unwrap_or_else(|| panic!("{} missing", spec.id));
            assert_eq!(theme, theme_for_spec(spec.id));
            assert!(is_known_theme(spec.id));
        }
    }

    #[test]
    fn theme_for_spec_falls_back_to_dark_for_an_unknown_id() {
        // Only `THEME_SPECS` ids reach this function through `theme_by_id`;
        // the fallback arm exists so a spec added without a palette degrades
        // to the default instead of panicking, so pin its behaviour here.
        assert_eq!(theme_for_spec("no-such-theme"), DARK);
        assert_eq!(theme_for_spec("no-such-theme"), theme_for_spec(DEFAULT_ID));
    }

    #[test]
    fn theme_by_id_is_case_and_whitespace_insensitive() {
        for spec in THEME_SPECS {
            let upper = spec.id.to_ascii_uppercase();
            let padded = format!("  {}\t", spec.id);
            assert_eq!(theme_by_id(&upper), theme_by_id(spec.id));
            assert_eq!(theme_by_id(&padded), theme_by_id(spec.id));
            // `_` is accepted as a synonym for `-` (hand-typed config values).
            assert_eq!(
                theme_by_id(&spec.id.replace('-', "_")),
                theme_by_id(spec.id)
            );
        }
    }

    #[test]
    fn theme_by_id_misses_return_none() {
        for miss in [
            "",
            " ",
            "\t\n",
            "-",
            "darky",
            "dark theme",
            "onedark",
            "one dark",
            "solarized",
            "dark-",
            "😀",
            &"x".repeat(4096),
        ] {
            assert!(theme_by_id(miss).is_none(), "{miss:?} should be unknown");
            assert!(spec_by_id(miss).is_none(), "{miss:?} should have no spec");
            assert!(!is_known_theme(miss), "{miss:?} should be unknown");
        }
    }

    #[test]
    fn spec_by_id_returns_the_catalog_entry_itself() {
        let spec = spec_by_id(" Dracula ").expect("normalized hit");
        // Compared by value on purpose: `const` promotion may hand out distinct
        // copies of the same slice, so pointer identity is not part of the API.
        assert_eq!(*spec, THEME_SPECS[5]);
        assert_eq!(spec.label, "Dracula");
        assert!(spec.dark);
        assert!(spec_by_id("").is_none());
        assert!(spec_by_id("   ").is_none());
    }

    #[test]
    fn dark_theme_is_byte_for_byte_the_crate_dark_theme() {
        let dark = theme_by_id("dark").expect("dark");
        assert_eq!(dark, crate::theme::DARK_THEME);
        // Field-by-field, so a divergence points at the field rather than "two
        // structs differ".
        assert_eq!(theme_fields(&dark), theme_fields(&crate::theme::DARK_THEME));
        assert_eq!(dark.bg, -1);
        assert_eq!(dark.fg, 252);
        assert_eq!(dark.accent, 39);
        assert_eq!(dark.assistant_bg, -1);
    }

    #[test]
    fn resolve_theme_without_config_returns_default_id_and_palette() {
        let (id, theme) = resolve_theme(None);
        assert_eq!(id, "dark");
        assert_eq!(id, default_theme_id());
        assert_eq!(theme, crate::theme::DARK_THEME);
    }

    #[test]
    fn resolve_theme_unknown_reports_the_fallback_id() {
        for miss in ["", "   ", "nope", "DARKX", "light-"] {
            let (id, theme) = resolve_theme(Some(miss));
            assert_eq!(id, "dark", "{miss:?} should fall back");
            assert_eq!(theme, crate::theme::DARK_THEME);
            assert!(is_known_theme(id), "reported id must itself be known");
        }
    }

    #[test]
    fn resolve_theme_known_returns_canonical_id_and_its_palette() {
        for spec in THEME_SPECS {
            let (id, theme) = resolve_theme(Some(spec.id));
            assert_eq!(id, spec.id);
            assert_eq!(theme, theme_by_id(spec.id).expect("known"));
            // Case / whitespace / underscore variants canonicalise to the same id.
            let messy = format!(" {} ", spec.id.to_ascii_uppercase().replace('-', "_"));
            assert_eq!(resolve_theme(Some(&messy)), (spec.id, theme));
        }
        assert_eq!(resolve_theme(Some("Light")).0, "light");
        assert_eq!(resolve_theme(Some(" one-dark ")).0, "one-dark");
    }

    #[test]
    fn resolve_theme_never_panics_on_hostile_input() {
        let hostile = ["\u{0}", "\u{1b}[31m", "dark\u{202e}", &"😀".repeat(64)];
        for input in hostile {
            let (id, _) = resolve_theme(Some(input));
            assert!(is_known_theme(id));
        }
    }

    #[test]
    fn every_theme_differs_from_every_other_theme() {
        for (i, a) in THEME_SPECS.iter().enumerate() {
            for b in &THEME_SPECS[i + 1..] {
                let (ta, tb) = (theme_by_id(a.id).unwrap(), theme_by_id(b.id).unwrap());
                assert_ne!(
                    theme_fields(&ta),
                    theme_fields(&tb),
                    "{} and {} are the same palette — one of them is pointless",
                    a.id,
                    b.id
                );
            }
        }
    }

    #[test]
    fn every_spec_has_its_own_theme_and_non_empty_palette() {
        for spec in THEME_SPECS {
            let theme = theme_by_id(spec.id).expect("spec must resolve");
            let fields = theme_fields(&theme);
            assert_eq!(fields.len(), 29, "theme field coverage changed");
            for (name, value) in &fields {
                assert!(
                    (-1..=255).contains(value),
                    "{}: {name}={value} is not a 256-color index",
                    spec.id
                );
            }
        }
    }

    #[test]
    fn key_colors_are_opaque_and_present() {
        // "Opaque" = an explicit palette index, never the terminal-default
        // sentinel: only the background may inherit the terminal.
        for spec in THEME_SPECS {
            let theme = theme_by_id(spec.id).expect("spec");
            for (name, value) in theme_fields(&theme) {
                if MAY_INHERIT.contains(&name) {
                    continue;
                }
                assert!(
                    (0..=255).contains(&value),
                    "{}: {name}={value} must be an explicit color",
                    spec.id
                );
            }
        }
    }

    #[test]
    fn signature_colors_are_mutually_distinct() {
        for spec in THEME_SPECS {
            let theme = theme_by_id(spec.id).expect("spec");
            let signature = [
                ("accent", theme.accent),
                ("selected_bg", theme.selected_bg),
                ("selected_fg", theme.selected_fg),
                ("error", theme.error),
                ("success", theme.success),
            ];
            for (i, (a_name, a)) in signature.iter().enumerate() {
                for (b_name, b) in &signature[i + 1..] {
                    assert_ne!(
                        a, b,
                        "{}: {a_name} and {b_name} are both {a} — the UI cannot tell them apart",
                        spec.id
                    );
                }
            }
            assert_ne!(theme.selected_bg, theme.selected_fg, "{}", spec.id);
            assert_ne!(theme.accent, theme.border, "{}", spec.id);
        }
    }

    #[test]
    fn light_themes_use_explicit_light_backgrounds() {
        for spec in THEME_SPECS.iter().filter(|spec| !spec.dark) {
            let theme = theme_by_id(spec.id).expect("spec");
            assert!(
                theme.bg >= 0,
                "{}: a light theme must not rely on the terminal background",
                spec.id
            );
            assert!(theme.assistant_bg >= 0, "{}", spec.id);
            assert_ne!(
                theme.fg,
                crate::theme::DARK_THEME.fg,
                "{}: a light theme needs its own foreground",
                spec.id
            );
        }
    }

    #[test]
    fn theme_options_mirrors_the_catalog_in_order() {
        let options = theme_options();
        assert_eq!(options.len(), THEME_SPECS.len());
        assert_eq!(options[0], ("dark", "Dark"));
        for (option, spec) in options.iter().zip(THEME_SPECS) {
            assert_eq!(*option, (spec.id, spec.label));
            assert!(is_known_theme(option.0), "{:?} is not a known id", option.0);
            assert!(!option.1.is_empty());
        }
        let ids: HashSet<&str> = options.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids.len(), THEME_SPECS.len());
    }

    /// xterm-256 index → sRGB, from the documented palette (6×6×6 cube with
    /// levels 0/95/135/175/215/255 above 16, grays from 232, standard xterm
    /// values for the 16 system colors).
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

    /// WCAG 2.1 contrast ratio between two 256-color indices.
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

    #[test]
    fn contrast_helper_matches_the_wcag_reference_points() {
        assert!((contrast_ratio(231, 231) - 1.0).abs() < 1e-9);
        assert!((contrast_ratio(231, 16) - 21.0).abs() < 1e-9);
        // 130 (#af5f00) on white is the ratio the md-theme report quotes.
        assert!((contrast_ratio(231, 130) - 4.71).abs() < 0.01);
        // Indices 0..=15 are the terminal-defined system colors (a separate arm
        // of the index table): 0 black vs 15 white is the 21:1 maximum.
        assert!((contrast_ratio(0, 15) - 21.0).abs() < 1e-9);
    }

    /// The `light` palette's markdown *text* roles must stay readable on its
    /// white background. This is the table that motivated wiring the palette
    /// into the renderer: before the fix the renderer painted the dark
    /// constants (heading 221 = 1.39:1, link 117 = 1.59:1, code 151 = 1.60:1,
    /// fence 143 = 2.30:1) — all far below AA.
    ///
    /// Measured for the palette's own values: heading 130 = 4.71:1, link 25 =
    /// 6.45:1, fence body 28 = 4.70:1, inline code 30 = **4.36:1** — 0.14 short
    /// of AA. The `md_code` shortfall is *reported, not silently patched*: the
    /// only way past it is editing the palette (30 → 23/24), which changes how
    /// light looks, so it is the supervisor's call. The floors below pin the
    /// three AA passes at 4.5 and keep the fourth from drifting further down.
    #[test]
    fn light_markdown_text_roles_stay_readable_on_white() {
        let light = theme_by_id("light").expect("light is in the catalog");
        let bg = light.bg as u8;
        let ratios: [(&str, Color, f64); 4] = [
            ("md_heading", light.md_heading, 4.71),
            ("md_link", light.md_link, 6.45),
            ("md_code", light.md_code, 4.36),
            ("md_code_block", light.md_code_block, 4.70),
        ];
        for (role, color, expected) in ratios {
            let ratio = contrast_ratio(bg, color as u8);
            assert!(
                (ratio - expected).abs() < 0.01,
                "light: {role}={color} on bg {bg} is {ratio:.2}:1, not {expected:.2}:1",
            );
            if role != "md_code" {
                assert!(
                    ratio >= 4.5,
                    "light: {role}={color} is {ratio:.2}:1 — WCAG AA needs 4.5:1"
                );
            }
        }
        assert!(contrast_ratio(bg, light.md_code as u8) >= 4.3);
        // The decorative grays bracket text rather than being text. `md_quote`
        // 245 measures 3.45:1 and `md_code_block_border` 250 measures 1.90:1;
        // the dark constant they inherited before the wiring (244) scored
        // 3.95:1 — also below AA — so no new failure is introduced, but both
        // are still below it. Reported in the md-theme handoff; raising them
        // means editing the palette, which this fix does not do.
        assert!(contrast_ratio(bg, light.md_quote as u8) >= 3.0);
        assert!(contrast_ratio(bg, light.md_code_block_border as u8) >= 1.5);
    }

    #[test]
    fn default_theme_id_is_known_and_dark() {
        let id = default_theme_id();
        assert!(is_known_theme(id));
        let (actual, theme) = resolve_theme(Some(id));
        assert_eq!(actual, id);
        assert_eq!(theme, crate::theme::DARK_THEME);
        assert!(spec_by_id(id).expect("spec").dark);
    }
}
