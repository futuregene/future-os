//! Autocomplete system — provider-based completion. 1:1 port of
//! `tui/src/components/autocomplete.ts`.
//!
//! The TS implementation debounces queries (20ms) and runs `getCompletions`
//! asynchronously with an `AbortSignal`. This port keeps the same method
//! surface and semantics but executes synchronously: the debounce/abort
//! machinery is an event-loop concern the app layer (P2) will drive from its
//! own render loop. Providers are also synchronous — the async fs/child
//! process calls in the TS providers are direct calls here.

use std::env;
use std::path::PathBuf;

use regex::Regex;

use crate::theme::{bold, fg};
use crate::tui::Component;
use crate::utils::{truncate_to_width, visible_width, TruncateOptions};

// ─── Types ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutocompleteItem {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutocompleteContext {
    pub text: String,
    pub cursor_pos: usize,
    /// The token being completed at cursor position.
    pub token: String,
    /// Start offset of the token being completed.
    pub token_start: usize,
}

pub trait AutocompleteProvider {
    fn name(&self) -> &str;
    /// Return non-null context if this provider should handle the input.
    fn r#match(&self, text: &str, cursor_pos: usize) -> Option<AutocompleteContext>;
    /// Return completion items for the matched context.
    fn get_completions(&self, ctx: &AutocompleteContext) -> Vec<AutocompleteItem>;
}

// ─── Autocomplete Manager ──────────────────────────────────────────────────

pub struct AutocompleteManager {
    providers: Vec<Box<dyn AutocompleteProvider>>,
    last_text: String,
    last_cursor_pos: usize,
    /// The latest matched context (for token-aware completion).
    active_context: Option<AutocompleteContext>,
    /// Callback when items are ready (or empty to hide).
    #[allow(clippy::type_complexity)]
    on_items: Option<Box<dyn FnMut(&[AutocompleteItem])>>,
}

impl Default for AutocompleteManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AutocompleteManager {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            last_text: String::new(),
            last_cursor_pos: 0,
            active_context: None,
            on_items: None,
        }
    }

    /// Register a provider; returns its index for `unregister`.
    pub fn register(&mut self, provider: Box<dyn AutocompleteProvider>) -> usize {
        self.providers.push(provider);
        self.providers.len() - 1
    }

    pub fn unregister(&mut self, index: usize) {
        if index < self.providers.len() {
            self.providers.remove(index);
        }
    }

    /// Trigger a query for the given text. (TS debounces 20ms; the sync port
    /// runs the query immediately — the debounce lives in the app loop.)
    pub fn query(&mut self, text: &str, cursor_pos: usize) {
        self.last_text = text.to_string();
        self.last_cursor_pos = cursor_pos;
        self.run_query(text, cursor_pos);
    }

    /// Run query immediately (bypasses debounce). Used for Tab trigger.
    pub fn query_immediate(&mut self, text: &str, cursor_pos: usize) {
        self.run_query(text, cursor_pos);
    }

    /// Re-run last query (for continued typing within same token).
    pub fn refresh(&mut self) {
        let text = self.last_text.clone();
        let cursor_pos = self.last_cursor_pos;
        self.query(&text, cursor_pos);
    }

    /// Install the items callback (app layer: popup show/hide + render).
    #[allow(clippy::type_complexity)]
    pub fn set_on_items(&mut self, cb: Box<dyn FnMut(&[AutocompleteItem])>) {
        self.on_items = Some(cb);
    }

    fn run_query(&mut self, text: &str, cursor_pos: usize) {
        // Try each provider in registration order
        for provider in &mut self.providers {
            let Some(ctx) = provider.r#match(text, cursor_pos) else {
                continue;
            };
            let items = provider.get_completions(&ctx);
            if !items.is_empty() {
                self.active_context = Some(ctx);
                if let Some(on_items) = self.on_items.as_mut() {
                    on_items(&items);
                }
                return;
            }
        }

        // No provider matched or returned items
        self.active_context = None;
        if let Some(on_items) = self.on_items.as_mut() {
            on_items(&[]);
        }
    }

    pub fn destroy(&mut self) {
        self.providers.clear();
    }

    pub fn active_context(&self) -> Option<&AutocompleteContext> {
        self.active_context.as_ref()
    }
}

// ─── Slash Command Provider ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommand {
    pub value: String,
    pub label: String,
    pub description: String,
    /// If true, this command accepts a model name argument.
    pub takes_model_arg: bool,
    /// If true, this command accepts a session ID argument.
    pub takes_session_arg: bool,
}

pub struct SlashCommandProvider {
    commands: Vec<SlashCommand>,
    get_models: Option<Box<dyn Fn() -> Vec<String>>>,
    get_sessions: Option<Box<dyn Fn() -> Vec<String>>>,
}

impl SlashCommandProvider {
    pub fn new(
        commands: Vec<SlashCommand>,
        get_models: Option<Box<dyn Fn() -> Vec<String>>>,
        get_sessions: Option<Box<dyn Fn() -> Vec<String>>>,
    ) -> Self {
        Self {
            commands,
            get_models,
            get_sessions,
        }
    }
}

impl AutocompleteProvider for SlashCommandProvider {
    fn name(&self) -> &str {
        "slash-command"
    }

    fn r#match(&self, text: &str, cursor_pos: usize) -> Option<AutocompleteContext> {
        if !text.starts_with('/') {
            return None;
        }

        match text.find(' ') {
            None => {
                // Typing command name: /mod...
                Some(AutocompleteContext {
                    text: text.to_string(),
                    cursor_pos,
                    token: text[1..].to_string(),
                    token_start: 1,
                })
            }
            Some(space_idx) => {
                let cmd_name = text[1..space_idx].to_lowercase();
                let cmd = self
                    .commands
                    .iter()
                    .find(|c| c.value[1..].to_lowercase() == cmd_name)?;

                let arg = text[space_idx + 1..].to_string();
                if cmd.takes_model_arg || cmd.takes_session_arg {
                    Some(AutocompleteContext {
                        text: text.to_string(),
                        cursor_pos,
                        token: arg,
                        token_start: space_idx + 1,
                    })
                } else {
                    None
                }
            }
        }
    }

    fn get_completions(&self, ctx: &AutocompleteContext) -> Vec<AutocompleteItem> {
        let text = &ctx.text;
        match text.find(' ') {
            None => {
                // Complete command name, sorted alphabetically
                let prefix = ctx.token.to_lowercase();
                let mut matched: Vec<&SlashCommand> = self
                    .commands
                    .iter()
                    .filter(|c| c.label.to_lowercase().contains(&prefix))
                    .collect();
                matched.sort_by(|a, b| a.label.cmp(&b.label));
                matched
                    .iter()
                    .map(|c| AutocompleteItem {
                        value: c.value.clone(),
                        label: c.label.clone(),
                        description: Some(c.description.clone()),
                    })
                    .collect()
            }
            Some(space_idx) => {
                // Complete argument
                let cmd_name = text[1..space_idx].to_lowercase();
                let Some(cmd) = self
                    .commands
                    .iter()
                    .find(|c| c.value[1..].to_lowercase() == cmd_name)
                else {
                    return Vec::new();
                };

                let arg_prefix = ctx.token.to_lowercase();

                if cmd.takes_model_arg {
                    let Some(get_models) = &self.get_models else {
                        return Vec::new();
                    };
                    return get_models()
                        .iter()
                        .filter(|m| m.to_lowercase().contains(&arg_prefix))
                        .take(20)
                        .map(|m| AutocompleteItem {
                            value: format!("{} {m}", cmd.value),
                            label: m.clone(),
                            description: Some(String::new()),
                        })
                        .collect();
                }

                if cmd.takes_session_arg {
                    let Some(get_sessions) = &self.get_sessions else {
                        return Vec::new();
                    };
                    return get_sessions()
                        .iter()
                        .filter(|s| s.to_lowercase().contains(&arg_prefix))
                        .take(20)
                        .map(|s| AutocompleteItem {
                            value: format!("{} {s}", cmd.value),
                            label: s.clone(),
                            description: Some(String::new()),
                        })
                        .collect();
                }

                Vec::new()
            }
        }
    }
}

// ─── File Path Provider ────────────────────────────────────────────────────

fn path_token_re() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:^|\s)([.]?[^\s]*/[^\s]*|[.][^\s]*)$").unwrap())
}

pub struct FilePathProvider {
    cwd: String,
}

impl FilePathProvider {
    pub fn new(cwd: Option<String>) -> Self {
        Self {
            cwd: cwd.unwrap_or_else(|| {
                env::current_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            }),
        }
    }

    pub fn set_cwd(&mut self, cwd: &str) {
        self.cwd = cwd.to_string();
    }
}

impl AutocompleteProvider for FilePathProvider {
    fn name(&self) -> &str {
        "file-path"
    }

    fn r#match(&self, text: &str, cursor_pos: usize) -> Option<AutocompleteContext> {
        // Slash commands are literal command lines — don't hijack their
        // arguments with file-path completions. Without this, `/cwd ../`
        // opens a parent-directory popup and Enter selects the highlighted
        // entry instead of submitting the command, so the cwd lands on a
        // wrong path (`a/ ../`-style artifacts) instead of the clean parent.
        if text.starts_with('/') {
            return None;
        }
        // Detect file path patterns: starts with . or contains / at cursor
        let prefix = &text[..cursor_pos.min(text.len())];
        // Look for the last path-like token
        let caps = path_token_re().captures(prefix)?;
        let full = caps.get(0)?;
        let token = caps.get(1)?.as_str();
        let token_start = full.start() + full.as_str().find(token)?;
        Some(AutocompleteContext {
            text: text.to_string(),
            cursor_pos,
            token: token.to_string(),
            token_start,
        })
    }

    fn get_completions(&self, ctx: &AutocompleteContext) -> Vec<AutocompleteItem> {
        let token = &ctx.token;

        // Resolve the partial path
        let resolved: PathBuf = if let Some(rest) = token.strip_prefix('~') {
            let home = env::var("HOME").unwrap_or_else(|_| "/".to_string());
            PathBuf::from(home).join(rest)
        } else {
            PathBuf::from(&self.cwd).join(token)
        };

        let (dir_path, file_prefix) = match std::fs::metadata(&resolved) {
            Ok(meta) if meta.is_dir() => (resolved, String::new()),
            _ => {
                let dir = resolved
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| PathBuf::from("."));
                let base = resolved
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                (dir, base)
            }
        };

        let entries = match std::fs::read_dir(&dir_path) {
            Ok(entries) => entries,
            Err(_) => return Vec::new(),
        };

        let mut matches: Vec<(String, bool)> = Vec::new(); // (name, is_dir)
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.to_lowercase().starts_with(&file_prefix.to_lowercase()) {
                continue;
            }
            if name.starts_with('.') {
                continue; // skip hidden
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            matches.push((name, is_dir));
        }

        // Directories first, then alphabetical
        matches.sort_by(|a, b| {
            if a.1 && !b.1 {
                return std::cmp::Ordering::Less;
            }
            if !a.1 && b.1 {
                return std::cmp::Ordering::Greater;
            }
            a.0.cmp(&b.0)
        });

        matches.truncate(20);

        matches
            .into_iter()
            .map(|(name, is_dir)| {
                let suffix = if is_dir { "/" } else { "" };
                let full = dir_path.join(format!("{name}{suffix}"));
                // Make path relative to cwd for display
                let full_str = full.display().to_string();
                let mut display = full_str.clone();
                if full_str.starts_with(&self.cwd) {
                    display = full_str[self.cwd.len()..].to_string();
                    if display.starts_with('/') {
                        display = display[1..].to_string();
                    }
                }
                AutocompleteItem {
                    value: display.clone(),
                    label: display,
                    description: Some(if is_dir { "dir".into() } else { String::new() }),
                }
            })
            .collect()
    }
}

// ─── Attachment Provider ───────────────────────────────────────────────────

fn at_token_re() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:^|\s)@([^\s]*)$").unwrap())
}

/// Attachment provider: triggered by "@" for fuzzy file search.
/// Uses fd (when available) or falls back to find for fast fuzzy matching.
pub struct AttachmentProvider;

impl AutocompleteProvider for AttachmentProvider {
    fn name(&self) -> &str {
        "attachment"
    }

    fn r#match(&self, text: &str, cursor_pos: usize) -> Option<AutocompleteContext> {
        let prefix = &text[..cursor_pos.min(text.len())];
        // Match "@" at word boundary, possibly followed by partial filename
        let caps = at_token_re().captures(prefix)?;
        let full = caps.get(0)?;
        let token = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let at_idx = full.as_str().find('@').unwrap_or(0);
        let token_start = full.start() + at_idx + 1;
        Some(AutocompleteContext {
            text: text.to_string(),
            cursor_pos,
            token: token.to_string(),
            token_start,
        })
    }

    fn get_completions(&self, ctx: &AutocompleteContext) -> Vec<AutocompleteItem> {
        let pattern = ctx.token.to_lowercase();
        if pattern.is_empty() {
            return Vec::new();
        }

        let mut results: Vec<String> = Vec::new();

        // Try fd first (fast, respects .gitignore)
        let fd = std::process::Command::new("fd")
            .args(["--hidden", "--type", "f", "--max-results", "50", &pattern])
            .output();
        match fd {
            Ok(out) if out.status.success() => {
                results = String::from_utf8_lossy(&out.stdout)
                    .trim()
                    .split('\n')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();
            }
            _ => {
                // fd not available — fall back to native find (POSIX).
                // (TS also has a PowerShell branch for win32; the self-implemented
                // backend targets POSIX first, windows-sys later.)
                let find = std::process::Command::new("find")
                    .args([
                        ".",
                        "-name",
                        &format!("{pattern}*"),
                        "-type",
                        "f",
                        "-maxdepth",
                        "5",
                    ])
                    .output();
                match find {
                    Ok(out) if out.status.success() => {
                        results = String::from_utf8_lossy(&out.stdout)
                            .trim()
                            .split('\n')
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                            .collect();
                    }
                    _ => {
                        // Neither fd nor fallback available — no results
                    }
                }
            }
        }

        results
            .into_iter()
            .map(|file_path| AutocompleteItem {
                value: format!("@{file_path}"),
                label: file_path,
                description: Some(String::new()),
            })
            .collect()
    }
}

// ─── Autocomplete Popup ────────────────────────────────────────────────────

pub struct AutocompletePopup {
    items: Vec<AutocompleteItem>,
    selected_index: usize,
    visible: bool,
    max_visible: usize,
}

impl Default for AutocompletePopup {
    fn default() -> Self {
        Self::new()
    }
}

impl AutocompletePopup {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            selected_index: 0,
            visible: false,
            max_visible: 10,
        }
    }

    pub fn show(&mut self, items: Vec<AutocompleteItem>) {
        self.items = items;
        self.selected_index = 0;
        self.visible = true;
    }

    pub fn hide(&mut self) {
        self.visible = false;
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn get_selected_item(&self) -> Option<&AutocompleteItem> {
        if !self.visible || self.items.is_empty() {
            return None;
        }
        self.items.get(self.selected_index)
    }

    pub fn select_next(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + 1) % self.items.len();
    }

    pub fn select_prev(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.selected_index = if self.selected_index == 0 {
            self.items.len() - 1
        } else {
            self.selected_index - 1
        };
    }

    pub fn set_max_visible(&mut self, n: usize) {
        self.max_visible = n;
    }

    pub fn height(&self) -> usize {
        if !self.visible || self.items.is_empty() {
            return 0;
        }
        2 + self.items.len().min(self.max_visible)
    }
}

impl Component for AutocompletePopup {
    fn render(&mut self, width: usize) -> Vec<String> {
        if !self.visible || self.items.is_empty() {
            return Vec::new();
        }

        // Total width including the │ │ borders — every row (borders, selected
        // and unselected items) must come out exactly this wide or the right
        // border goes jagged.
        let total_w = 12.max(width.saturating_sub(2).min(50));
        let inner = total_w - 2;
        let mut lines: Vec<String> = Vec::new();

        lines.push(fg(244, "┌") + &fg(239, &"─".repeat(inner)) + &fg(244, "┐"));

        let start = (self.selected_index as i64 - self.max_visible as i64 + 1).max(0) as usize;
        let end = self.items.len().min(start + self.max_visible);

        for i in start..end {
            let item = &self.items[i];
            let is_selected = i == self.selected_index;
            // Truncate by DISPLAY width on plain text (slicing the styled
            // string by code units could sever ANSI sequences and miscounts
            // wide chars).
            let prefix = if is_selected { "▶ " } else { "  " };
            let label = truncate_to_width(
                &format!("{prefix}{}", item.label),
                inner,
                &TruncateOptions::default(),
            );
            let label_vis = visible_width(&label);
            let desc = match &item.description {
                // JS truthiness: empty description is falsy
                Some(d) if !d.is_empty() => truncate_to_width(
                    &format!(" {d}"),
                    inner.saturating_sub(label_vis),
                    &TruncateOptions::default(),
                ),
                _ => String::new(),
            };
            let pad = inner.saturating_sub(label_vis + visible_width(&desc));

            if is_selected {
                lines.push(
                    fg(244, "│")
                        + &fg(252, &bold(&label))
                        + &fg(245, &desc)
                        + &" ".repeat(pad)
                        + &fg(244, "│"),
                );
            } else {
                lines.push(
                    fg(244, "│")
                        + &fg(245, &label)
                        + &fg(245, &desc)
                        + &" ".repeat(pad)
                        + &fg(244, "│"),
                );
            }
        }

        lines.push(fg(244, "└") + &fg(239, &"─".repeat(inner)) + &fg(244, "┘"));

        lines
    }

    fn handle_input(&mut self, data: &str) {
        if data == "up" {
            self.select_prev();
        } else if data == "down" {
            self.select_next();
        }
    }

    fn invalidate(&mut self) {}

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::strip_ansi_codes;

    // ─── AutocompletePopup ────────────────────────────────────────────

    #[test]
    fn popup_hidden_by_default() {
        let mut pop = AutocompletePopup::new();
        assert!(!pop.is_visible());
        assert_eq!(pop.render(60), Vec::<String>::new());
        assert_eq!(pop.height(), 0);
        assert!(pop.get_selected_item().is_none());
    }

    #[test]
    fn popup_show_hide_and_selection() {
        let mut pop = AutocompletePopup::new();
        pop.show(vec![
            AutocompleteItem {
                value: "/model".into(),
                label: "/model".into(),
                description: Some("select model".into()),
            },
            AutocompleteItem {
                value: "/new".into(),
                label: "/new".into(),
                description: Some("new session".into()),
            },
        ]);
        assert!(pop.is_visible());
        assert_eq!(
            pop.get_selected_item().map(|i| i.value.as_str()),
            Some("/model")
        );
        pop.select_next();
        assert_eq!(
            pop.get_selected_item().map(|i| i.value.as_str()),
            Some("/new")
        );
        pop.select_next(); // wraps
        assert_eq!(
            pop.get_selected_item().map(|i| i.value.as_str()),
            Some("/model")
        );
        pop.select_prev(); // wraps back
        assert_eq!(
            pop.get_selected_item().map(|i| i.value.as_str()),
            Some("/new")
        );
        pop.hide();
        assert!(!pop.is_visible());
        assert!(pop.get_selected_item().is_none());
    }

    #[test]
    fn popup_handle_input_up_down() {
        let mut pop = AutocompletePopup::new();
        pop.show(vec![
            AutocompleteItem {
                value: "a".into(),
                label: "a".into(),
                description: None,
            },
            AutocompleteItem {
                value: "b".into(),
                label: "b".into(),
                description: None,
            },
        ]);
        pop.handle_input("down");
        assert_eq!(pop.get_selected_item().map(|i| i.value.as_str()), Some("b"));
        pop.handle_input("up");
        assert_eq!(pop.get_selected_item().map(|i| i.value.as_str()), Some("a"));
    }

    #[test]
    fn popup_renders_all_rows_at_same_visible_width() {
        let mut pop = AutocompletePopup::new();
        pop.show(vec![
            AutocompleteItem {
                value: "/model".into(),
                label: "/model".into(),
                description: Some("select model".into()),
            },
            AutocompleteItem {
                value: "/new".into(),
                label: "/new".into(),
                description: Some("new session".into()),
            },
            AutocompleteItem {
                value: "/long".into(),
                label: "/a-very-long-command-name-that-exceeds-the-popup-width-abcdefgh".into(),
                description: Some("d".into()),
            },
        ]);
        for width in [30usize, 60, 120] {
            let lines = pop.render(width);
            let widths: Vec<usize> = lines.iter().map(|l| visible_width(l)).collect();
            let mut unique = widths.clone();
            unique.dedup();
            assert_eq!(unique.len(), 1);
            assert!(widths[0] <= width);
        }
    }

    #[test]
    fn popup_does_not_sever_ansi_sequences_when_truncating_long_labels() {
        let mut pop = AutocompletePopup::new();
        pop.show(vec![AutocompleteItem {
            value: "x".into(),
            label: "l".repeat(200),
            description: Some("d".repeat(50)),
        }]);
        for line in pop.render(40) {
            // No dangling ESC without a terminator, and every row resets at
            // the end
            assert!(!strip_ansi_codes(&line).contains('\x1b'));
        }
    }

    #[test]
    fn popup_height_matches_visible_item_count() {
        let mut pop = AutocompletePopup::new();
        pop.show(vec![
            AutocompleteItem {
                value: "1".into(),
                label: "1".into(),
                description: None,
            },
            AutocompleteItem {
                value: "2".into(),
                label: "2".into(),
                description: None,
            },
            AutocompleteItem {
                value: "3".into(),
                label: "3".into(),
                description: None,
            },
        ]);
        pop.set_max_visible(2);
        // 2 border rows + min(3, 2) items
        assert_eq!(pop.height(), 4);
    }

    // ─── SlashCommandProvider ─────────────────────────────────────────

    fn slash_commands() -> Vec<SlashCommand> {
        vec![
            SlashCommand {
                value: "/model".into(),
                label: "/model".into(),
                description: "select model".into(),
                takes_model_arg: true,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/new".into(),
                label: "/new".into(),
                description: "start a new session".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/sessions".into(),
                label: "/sessions".into(),
                description: "browse and switch sessions".into(),
                takes_model_arg: false,
                takes_session_arg: true,
            },
        ]
    }

    #[test]
    fn slash_provider_matches_command_name_token() {
        let provider = SlashCommandProvider::new(slash_commands(), None, None);
        let ctx = provider.r#match("/mo", 3).unwrap();
        assert_eq!(ctx.token, "mo");
        assert_eq!(ctx.token_start, 1);
    }

    #[test]
    fn slash_provider_no_match_for_plain_text() {
        let provider = SlashCommandProvider::new(slash_commands(), None, None);
        assert!(provider.r#match("hello", 5).is_none());
    }

    #[test]
    fn slash_provider_matches_arg_token_for_model_command() {
        let provider = SlashCommandProvider::new(slash_commands(), None, None);
        let ctx = provider.r#match("/model gpt", 10).unwrap();
        assert_eq!(ctx.token, "gpt");
        assert_eq!(ctx.token_start, 7);
    }

    #[test]
    fn slash_provider_no_arg_token_for_non_arg_command() {
        let provider = SlashCommandProvider::new(slash_commands(), None, None);
        assert!(provider.r#match("/new foo", 8).is_none());
    }

    #[test]
    fn slash_provider_completes_command_names_sorted() {
        let provider = SlashCommandProvider::new(slash_commands(), None, None);
        let ctx = AutocompleteContext {
            text: "/".into(),
            cursor_pos: 1,
            token: String::new(),
            token_start: 1,
        };
        let items = provider.get_completions(&ctx);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["/model", "/new", "/sessions"]);
    }

    #[test]
    fn slash_provider_filters_command_names_by_prefix() {
        let provider = SlashCommandProvider::new(slash_commands(), None, None);
        let ctx = AutocompleteContext {
            text: "/s".into(),
            cursor_pos: 2,
            token: "s".into(),
            token_start: 1,
        };
        let items = provider.get_completions(&ctx);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["/sessions"]);
    }

    #[test]
    fn slash_provider_completes_model_args() {
        let provider = SlashCommandProvider::new(
            slash_commands(),
            Some(Box::new(|| {
                vec!["gpt-4o".into(), "claude-sonnet-4".into(), "o3-mini".into()]
            })),
            None,
        );
        let ctx = AutocompleteContext {
            text: "/model g".into(),
            cursor_pos: 8,
            token: "g".into(),
            token_start: 7,
        };
        let items = provider.get_completions(&ctx);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].value, "/model gpt-4o");
        assert_eq!(items[0].label, "gpt-4o");
    }

    #[test]
    fn slash_provider_completes_session_args() {
        let provider = SlashCommandProvider::new(
            slash_commands(),
            None,
            Some(Box::new(|| vec!["session-1".into(), "session-2".into()])),
        );
        let ctx = AutocompleteContext {
            text: "/sessions session-".into(),
            cursor_pos: 19,
            token: "session-".into(),
            token_start: 10,
        };
        let items = provider.get_completions(&ctx);
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|i| i.value == "/sessions session-1"));
    }

    // ─── AutocompleteManager ──────────────────────────────────────────

    struct DummyProvider {
        name: &'static str,
        trigger: &'static str,
        completions: Vec<AutocompleteItem>,
    }

    impl AutocompleteProvider for DummyProvider {
        fn name(&self) -> &str {
            self.name
        }

        fn r#match(&self, text: &str, _cursor_pos: usize) -> Option<AutocompleteContext> {
            text.strip_prefix(self.trigger)
                .map(|rest| AutocompleteContext {
                    text: text.to_string(),
                    cursor_pos: text.len(),
                    token: rest.to_string(),
                    token_start: self.trigger.len(),
                })
        }

        fn get_completions(&self, _ctx: &AutocompleteContext) -> Vec<AutocompleteItem> {
            self.completions.clone()
        }
    }

    #[test]
    fn manager_runs_first_matching_provider() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let mut manager = AutocompleteManager::new();
        manager.register(Box::new(DummyProvider {
            name: "first",
            trigger: "/",
            completions: vec![AutocompleteItem {
                value: "/a".into(),
                label: "/a".into(),
                description: None,
            }],
        }));
        manager.register(Box::new(DummyProvider {
            name: "second",
            trigger: "/x",
            completions: vec![AutocompleteItem {
                value: "/x1".into(),
                label: "/x1".into(),
                description: None,
            }],
        }));

        let last_items = Rc::new(RefCell::new(Vec::<String>::new()));
        let cb = Rc::clone(&last_items);
        manager.on_items = Some(Box::new(move |items| {
            *cb.borrow_mut() = items.iter().map(|i| i.value.clone()).collect();
        }));

        // TS semantics: providers run in registration order; the FIRST one
        // that matches AND returns items wins — "first" matches "/x" via its
        // "/" trigger, so its items are returned even though "second" is more
        // specific.
        manager.query("/x", 2);
        assert_eq!(*last_items.borrow(), vec!["/a"]);
        assert!(manager.active_context().is_some());
    }

    #[test]
    fn manager_reports_empty_when_no_provider_matches() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let mut manager = AutocompleteManager::new();
        manager.register(Box::new(DummyProvider {
            name: "only",
            trigger: "/",
            completions: Vec::new(),
        }));
        let last_items = Rc::new(RefCell::new(vec!["stale".to_string()]));
        let cb = Rc::clone(&last_items);
        manager.on_items = Some(Box::new(move |items| {
            *cb.borrow_mut() = items.iter().map(|i| i.value.clone()).collect();
        }));
        manager.query("hello", 5);
        assert!(last_items.borrow().is_empty());
        assert!(manager.active_context().is_none());
    }

    #[test]
    fn manager_skips_provider_with_empty_completions() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let mut manager = AutocompleteManager::new();
        manager.register(Box::new(DummyProvider {
            name: "empty",
            trigger: "/",
            completions: Vec::new(),
        }));
        manager.register(Box::new(DummyProvider {
            name: "full",
            trigger: "/",
            completions: vec![AutocompleteItem {
                value: "/filled".into(),
                label: "/filled".into(),
                description: None,
            }],
        }));
        let last_items = Rc::new(RefCell::new(Vec::<String>::new()));
        let cb = Rc::clone(&last_items);
        manager.on_items = Some(Box::new(move |items| {
            *cb.borrow_mut() = items.iter().map(|i| i.value.clone()).collect();
        }));
        manager.query("/", 1);
        assert_eq!(*last_items.borrow(), vec!["/filled"]);
    }

    #[test]
    fn manager_refresh_reruns_last_query() {
        use std::cell::Cell;
        use std::rc::Rc;
        let mut manager = AutocompleteManager::new();
        manager.register(Box::new(DummyProvider {
            name: "p",
            trigger: "/",
            completions: vec![AutocompleteItem {
                value: "/v".into(),
                label: "/v".into(),
                description: None,
            }],
        }));
        let count = Rc::new(Cell::new(0));
        let cb = Rc::clone(&count);
        manager.on_items = Some(Box::new(move |_| cb.set(cb.get() + 1)));
        manager.query("/", 1);
        assert_eq!(count.get(), 1);
        manager.refresh();
        assert_eq!(count.get(), 2);
    }

    // ─── FilePathProvider ─────────────────────────────────────────────

    #[test]
    fn file_path_match_detects_slash_token() {
        let provider = FilePathProvider::new(Some("/tmp".into()));
        let ctx = provider.r#match("ls /usr/lo", 10).unwrap();
        assert_eq!(ctx.token, "/usr/lo");
    }

    #[test]
    fn file_path_match_detects_dot_token() {
        let provider = FilePathProvider::new(Some("/tmp".into()));
        let ctx = provider.r#match("cat .git", 8).unwrap();
        assert_eq!(ctx.token, ".git");
    }

    #[test]
    fn file_path_match_none_without_slash_or_dot() {
        let provider = FilePathProvider::new(Some("/tmp".into()));
        assert!(provider.r#match("hello world", 11).is_none());
    }

    #[test]
    fn file_path_match_refuses_slash_command_context() {
        let provider = FilePathProvider::new(Some("/tmp".into()));
        // `/cwd ../` is a slash command — path completion must NOT hijack
        // its argument (the popup would consume Enter and corrupt the cwd).
        assert!(provider.r#match("/cwd ../", 8).is_none());
        assert!(provider.r#match("/cwd /usr/lo", 13).is_none());
        // Plain messages still complete paths (slashes at position 0 are
        // reserved for slash commands).
        assert!(provider.r#match("ls /usr/lo", 10).is_some());
        assert!(provider.r#match("cat .git", 8).is_some());
    }

    #[test]
    fn file_path_completions_list_directory_entries() {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_str().unwrap().to_string();
        std::fs::write(dir.path().join("alpha.txt"), "x").unwrap();
        std::fs::write(dir.path().join("beta.txt"), "x").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let provider = FilePathProvider::new(Some(dir_path.clone()));
        let ctx = AutocompleteContext {
            text: format!("{dir_path}/"),
            cursor_pos: dir_path.len() + 1,
            token: format!("{dir_path}/"),
            token_start: 0,
        };
        let items = provider.get_completions(&ctx);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        // Directories first, then alphabetical
        assert_eq!(labels, vec!["subdir/", "alpha.txt", "beta.txt"]);
        assert_eq!(items[0].description.as_deref(), Some("dir"));
    }
}
