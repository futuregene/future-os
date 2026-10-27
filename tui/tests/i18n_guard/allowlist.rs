//! What the i18n guard lets through, and why.
//!
//! Every row here is a string literal that [`super::chrome_is_drawn_from_the_table`]
//! would otherwise report: a Chinese literal anywhere under `tui/src/`, or an
//! English sentence drawn by the surface. A row is one `(file, literal)` pair —
//! the literal is the text as it is *written in the source*, escapes and all,
//! so it can be copied out of the failing message — plus the reason it is not
//! "add a row to the table".
//!
//! Two reasons are not the same thing and the distinction matters when reading
//! this file:
//!
//! - [`Why::NotChrome`] — the literal is not interface text at all: a proper
//!   noun, a format skeleton, an invariant's panic message, or data that stands
//!   in for the reader's own content. It will never be translated.
//! - [`Why::KnownGap`] — the literal *is* interface text and it is not in the
//!   table: today the surface says it in English whichever language the reader
//!   picked. Moving one into `workbench/i18n.rs` is a separate change (it edits
//!   a drawing path and changes what a Chinese reader sees), so the guard
//!   freezes it here instead of fixing it. These rows are the port list for
//!   whoever picks that up, and closing one means deleting its row **and**
//!   lowering [`BUDGET`].
//!
//! [`BUDGET`] is the number of exemptions the tree is allowed to have —
//! allowlist rows, exempt files and in-source `// i18n:` markers together. It
//! exists so a new exemption cannot be added quietly: the test fails until the
//! number moves too, and the number moving is the line a reviewer reads. It
//! only ever goes down without a reviewer agreeing to the opposite.

/// Why one literal is exempt.
pub enum Why {
    /// Not interface text, and never will be.
    NotChrome(&'static str),
    /// Interface text that is not in the table yet — a pre-existing gap.
    KnownGap(&'static str),
}

/// One exempt `(file, literal)` pair.
pub struct Allow {
    /// Path relative to `tui/src/`, forward slashes.
    pub path: &'static str,
    /// The literal exactly as the source writes it.
    pub literal: &'static str,
    pub why: Why,
}

impl Allow {
    const fn not_chrome(path: &'static str, literal: &'static str, why: &'static str) -> Self {
        Self {
            path,
            literal,
            why: Why::NotChrome(why),
        }
    }

    const fn known_gap(path: &'static str, literal: &'static str, why: &'static str) -> Self {
        Self {
            path,
            literal,
            why: Why::KnownGap(why),
        }
    }
}

/// The exemptions. One row per literal; a row that no longer matches anything
/// fails the test, so a stale entry cannot hide a gap that was already closed.
pub const ALLOW: &[Allow] = &[
    // A theme's name is a proper noun: `One Dark` is not "一个深色" to a Chinese
    // reader, and the desktop does not translate it either.
    Allow::not_chrome(
        "themes.rs",
        "One Dark",
        "a theme's name — a proper noun, spelled the same in both languages",
    ),
    Allow::not_chrome(
        "themes.rs",
        "One Light",
        "a theme's name — a proper noun, spelled the same in both languages",
    ),
    Allow::not_chrome(
        "themes.rs",
        "High Contrast",
        "a theme's name — a proper noun, spelled the same in both languages",
    ),
    Allow::not_chrome(
        "themes.rs",
        "Catppuccin Light",
        "a theme's name — a proper noun, spelled the same in both languages",
    ),
    Allow::not_chrome(
        "themes.rs",
        "Everforest Light",
        "a theme's name — a proper noun, spelled the same in both languages",
    ),
    Allow::not_chrome(
        "themes.rs",
        "Gruvbox Light",
        "a theme's name — a proper noun, spelled the same in both languages",
    ),
    Allow::not_chrome(
        "themes.rs",
        "Rose Pine",
        "a theme's name — a proper noun, spelled the same in both languages",
    ),
    Allow::not_chrome(
        "themes.rs",
        "Rose Pine Light",
        "a theme's name — a proper noun, spelled the same in both languages",
    ),
    Allow::not_chrome(
        "themes.rs",
        "Solarized Light",
        "a theme's name — a proper noun, spelled the same in both languages",
    ),
    Allow::not_chrome(
        "themes.rs",
        "Tokyo Night",
        "a theme's name — a proper noun, spelled the same in both languages",
    ),
    // The approval-mode chip's four rows come from the desktop's
    // `approvalTier.*` vocabulary, which is English in both languages there
    // too. `Manual` and `Sandboxed` are one word each, so the sentence rule
    // does not reach them; they are the same gap and belong in the same change
    // as these two.
    Allow::known_gap(
        "approval_mode.rs",
        "Automatic review",
        "the approval-mode chip's row",
    ),
    Allow::known_gap(
        "approval_mode.rs",
        "Full access",
        "the approval-mode chip's row",
    ),
    // The compaction marker the TUI writes into the history it folds.
    Allow::known_gap(
        "session_rows.rs",
        "Context compacted",
        "the history's compaction marker",
    ),
    Allow::known_gap(
        "session_rows.rs",
        "Context compacted: {before} → {after} tokens (estimated)",
        "the same marker, carrying the token counts",
    ),
    // The conversation area's own words. Pluralisation happens inline in
    // English (`tool call` / `tool calls`), which a bilingual table cannot
    // express — the Chinese spelling does not inflect.
    Allow::known_gap(
        "components/chat_area.rs",
        "merged into the previous run",
        "a run state's description in the conversation",
    ),
    Allow::known_gap(
        "components/chat_area.rs",
        "tool call",
        "the reply summary counts tools in English, pluralised inline",
    ),
    Allow::known_gap(
        "components/chat_area.rs",
        "tool calls",
        "the plural half of the same count",
    ),
    Allow::known_gap(
        "components/chat_area.rs",
        "  … {hidden} more line{plural} · {hint}",
        "the folded-output footer: pluralised inline and cut to width",
    ),
    Allow::known_gap(
        "components/chat_area.rs",
        "ctrl+g to expand",
        "the footer's hint half (the other half is `truncated`, one word, so the rule \
         does not reach it)",
    ),
    // The notice row. One row per message rather than one per call site: the
    // same sentence is drawn from two places in `live.rs`.
    Allow::known_gap(
        "workbench/live.rs",
        "Could not load the transcript: {error}",
        "notice: the transcript failed to load",
    ),
    Allow::known_gap(
        "workbench/live.rs",
        "Could not load older rows: {error}",
        "notice: the page above the transcript failed to load",
    ),
    Allow::known_gap(
        "workbench/live.rs",
        "Approve tool execution",
        "notice: the approval card's title",
    ),
    Allow::known_gap(
        "workbench/live.rs",
        "Failed to send: {error}",
        "notice: the prompt failed to send",
    ),
    Allow::known_gap(
        "workbench/live.rs",
        "Could not set the model: {error}",
        "notice: the model change failed",
    ),
    Allow::known_gap(
        "workbench/live.rs",
        "Could not export: {error}",
        "notice: /export failed",
    ),
    Allow::known_gap(
        "workbench/live.rs",
        "Could not switch session: {error}",
        "notice: switching session failed",
    ),
    Allow::known_gap(
        "workbench/live.rs",
        "Could not start a session: {error}",
        "notice: a new chat failed to start",
    ),
    Allow::known_gap(
        "workbench/live.rs",
        "Could not set thinking: {error}",
        "notice: the effort change failed",
    ),
    Allow::known_gap(
        "workbench/live.rs",
        "Could not cycle the model: {error}",
        "notice: the model cycle failed",
    ),
    Allow::known_gap(
        "workbench/live.rs",
        "Could not cycle thinking: {error}",
        "notice: cycling the effort failed",
    ),
    Allow::known_gap(
        "workbench/live.rs",
        "No models to choose from yet",
        "notice: the model picker has nothing to list",
    ),
    Allow::known_gap(
        "workbench/live.rs",
        "Could not set the approval mode: {error}",
        "notice: the mode chip failed",
    ),
    Allow::known_gap(
        "workbench/live.rs",
        "Could not answer the approval: {error}",
        "notice: the approval answer failed to send",
    ),
    Allow::known_gap(
        "workbench/live.rs",
        "Could not stop the run: {error}",
        "notice: the stop request failed",
    ),
    // A title, not chrome: it stands exactly where the reader's own session
    // title would stand, and titles are never translated.
    Allow::not_chrome(
        "workbench/live.rs",
        "new session",
        "the title a nameless session falls back to — a title is content",
    ),
];

/// A file whose literals are all content rather than chrome.
pub struct FileExempt {
    /// Path relative to `tui/src/`.
    pub path: &'static str,
    pub why: &'static str,
}

/// Whole-file exemptions. A file listed here still has to *contain* something
/// the guard would have reported, or the entry is stale and the test fails.
pub const FILES: &[FileExempt] = &[FileExempt {
    path: "workbench/demo.rs",
    why: "the demo dataset: task titles, conversation lines, session names and \
          branch labels. Every string in this module is the *content* a real \
          session would carry, built to look like one for screenshots and the \
          frame tests — the chrome around it already comes from the table.",
}];

/// How many exemptions the tree is allowed: [`ALLOW`] rows, [`FILES`] entries
/// and in-source `// i18n:` markers, counted together.
///
/// Raise it only when a reviewer has agreed the new exemption is not chrome or
/// is a gap worth freezing; lower it whenever one is closed. The test fails on
/// a mismatch in either direction, so the number cannot drift away from what
/// the tree actually skips.
///
/// 35 → 37: the two `@path` inlining notes in `workbench/live.rs`. They are the
/// prompt text the model reads in place of a file it was not given — never
/// drawn — so they stay in the language the prompt is written in.
///
/// 37 → 35 → 34: the folded-thinking placeholder's exemption closed, then the
/// header's `no workspace yet` — the five-row identity header it belonged to
/// went away, leaving the two-row context line as the only header.
/// 37 → 35 → 34: the folded-thinking placeholder's exemption closed, then the
/// header's `no workspace yet` — the five-row identity header it belonged to
/// went away, leaving the two-row context line as the only header.
///
/// 34 → 41: the `/theme` picker, the files panel's entry and the rest of the
/// work that landed beside this change.
///
/// 41 → 42: the `/git` panel's `does not have any commits`, matched against
/// git's own stderr to turn an empty repository into an empty log. It is a
/// comparison, never drawn; the panel's other copy (the silent failure, the
/// ahead/behind counts, the tip template) went into the table instead.
pub const BUDGET: usize = 42;
