//! The new-session screen (`ctrl+n`) — the TUI's own first message box.
//!
//! A whole-surface composer, in the shape OpenCode opens with: nothing but a
//! prompt box and the three choices a first message needs — which workspace it
//! runs in, which model answers it, and how much the model should think.
//! Nothing else from the conversation screen is on it (no transcript, no
//! footer, no tool output): the first message is composed before there is a
//! session to show, so showing a session would be a lie about what exists.
//!
//! Layout is decided here and drawn by the app: [`NewSessionScreen::render_above`]
//! fills the rows the transcript would use, [`NewSessionScreen::hints`] is the
//! line under the box, and the prompt box itself is the real [`Input`]
//! component, so editing, paste folding, image attachments and the history
//! keybindings behave exactly as they do in the conversation box.
//!
//! The screen owns *no* I/O: creating the folder a new workspace names is the
//! app's job ([`NewSessionAction::CreateWorkspace`]), as is opening a session
//! with the answer.

use crate::components::input::Input;
use crate::theme::{bold, dim, fg, Theme, DARK_THEME};
use crate::tui::Component;
use crate::utils::{truncate_to_width, visible_width, TruncateOptions};

/// The reasoning levels the agent accepts, in the order the picker cycles
/// them — the same `ThinkingLevel` wire values (`off` … `xhigh`).
pub const THINKING_LEVELS: [&str; 6] = ["off", "minimal", "low", "medium", "high", "xhigh"];

/// One folder a session can be started in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceEntry {
    /// Absolute path — what `new_session` is given.
    pub path: String,
    /// What the row shows (`~/future-os`, the home-relative form).
    pub label: String,
    /// Sessions already filed here, for the row's count.
    pub sessions: usize,
}

/// Which row of the screen owns the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewSessionFocus {
    Prompt,
    Workspace,
    Model,
    Thinking,
    /// Typing the path of a workspace that does not exist yet.
    NewWorkspace,
}

impl NewSessionFocus {
    fn next(self) -> Self {
        match self {
            Self::Prompt => Self::Workspace,
            Self::Workspace => Self::Model,
            Self::Model => Self::Thinking,
            Self::Thinking => Self::Prompt,
            // The path editor is a mode of the workspace row: leaving it goes
            // back to that row, never straight to the model list.
            Self::NewWorkspace => Self::Workspace,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Prompt => Self::Thinking,
            Self::Workspace => Self::Prompt,
            Self::Model => Self::Workspace,
            Self::Thinking => Self::Model,
            Self::NewWorkspace => Self::Workspace,
        }
    }
}

/// What the caller must do about a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewSessionAction {
    /// The screen did not take the key; the app may handle it as usual.
    Pass,
    /// State changed — redraw.
    Changed,
    /// `enter`: start a session with this answer.
    Submit,
    /// `escape`: leave the screen without starting anything.
    Cancel,
    /// `enter` in the path editor: create this folder, then select it.
    CreateWorkspace { path: String },
}

pub struct NewSessionScreen {
    /// The prompt box — the same component as the conversation's composer.
    pub input: Input,
    focus: NewSessionFocus,
    workspaces: Vec<WorkspaceEntry>,
    workspace: usize,
    /// The path editor behind [`NewSessionFocus::NewWorkspace`].
    path: Input,
    models: Vec<(String, String)>,
    model: usize,
    thinking: usize,
    /// Something the screen could not do, phrased for the reader.
    notice: Option<String>,
    theme: Theme,
}

impl NewSessionScreen {
    /// Build the screen for a good set of defaults: `cwd` selected, `model`
    /// and `thinking` where the session they were opened from stood.
    pub fn new(cwd: Option<&str>, model: Option<&str>, thinking: Option<&str>) -> Self {
        let mut screen = Self {
            input: Input::new(),
            focus: NewSessionFocus::Prompt,
            workspaces: Vec::new(),
            workspace: 0,
            path: Input::new(),
            models: Vec::new(),
            model: 0,
            thinking: 0,
            notice: None,
            theme: DARK_THEME,
        };
        screen.input.focused = true;
        if let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) {
            screen.set_workspaces(vec![WorkspaceEntry {
                path: cwd.to_string(),
                label: shorten_path(cwd),
                sessions: 0,
            }]);
        }
        if let Some(model) = model.filter(|model| !model.is_empty()) {
            screen.set_models(vec![(model.to_string(), String::new())]);
        }
        if let Some(thinking) = thinking {
            screen.select_thinking(thinking);
        }
        screen
    }

    /// Replace the workspace list, keeping the selection on the same folder
    /// when it is still there.
    pub fn set_workspaces(&mut self, workspaces: Vec<WorkspaceEntry>) {
        let selected = self.workspaces.get(self.workspace).map(|w| w.path.clone());
        self.workspaces = workspaces;
        self.workspace = selected
            .and_then(|path| self.workspaces.iter().position(|w| w.path == path))
            .unwrap_or(0);
    }

    /// Replace the model list (`(id, label)`), keeping the selected model.
    pub fn set_models(&mut self, models: Vec<(String, String)>) {
        let selected = self.current_model().map(str::to_string);
        self.models = models;
        self.model = 0;
        if let Some(id) = selected {
            self.select_model(&id);
        }
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.input.set_theme(&theme);
        self.path.set_theme(&theme);
    }

    pub fn focus(&self) -> NewSessionFocus {
        self.focus
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn current_workspace(&self) -> Option<&str> {
        // The path editor is mid-edit: what it holds is what will be created,
        // but it is not a folder yet — the selection stays the last real one.
        self.workspaces.get(self.workspace).map(|w| w.path.as_str())
    }

    pub fn current_model(&self) -> Option<&str> {
        self.models.get(self.model).map(|(id, _)| id.as_str())
    }

    /// The models, `(id, label)`, as the row shows them.
    pub fn models(&self) -> &[(String, String)] {
        &self.models
    }

    pub fn current_thinking(&self) -> &'static str {
        THINKING_LEVELS[self.thinking.min(THINKING_LEVELS.len() - 1)]
    }

    /// Select `level` when it is one of [`THINKING_LEVELS`]; an unknown value
    /// leaves the current choice alone (the agent may know a level this build
    /// does not, and silently moving to `off` would be a surprise).
    pub fn select_thinking(&mut self, level: &str) {
        if let Some(index) = THINKING_LEVELS.iter().position(|l| *l == level) {
            self.thinking = index;
        }
    }

    fn select_model(&mut self, id: &str) {
        if let Some(index) = self.models.iter().position(|(model, _)| model == id) {
            self.model = index;
        }
    }

    /// The path editor's text while a new workspace is being typed.
    pub fn new_workspace_path(&self) -> Option<&str> {
        (self.focus == NewSessionFocus::NewWorkspace).then(|| self.path.get_value())
    }

    /// Check that a path is usable and select it. `create` makes the folder
    /// first; a path that exists (or was just created) becomes the selection.
    pub fn accept_workspace(&mut self, path: &str, create: bool) -> bool {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            self.notice = Some("A workspace needs a folder path.".into());
            return false;
        }
        let resolved = crate::home::expand_path(trimmed, Some(&cwd_base()));
        if create {
            if let Err(err) = std::fs::create_dir_all(&resolved) {
                self.notice = Some(format!("Could not create {}: {err}", resolved.display()));
                return false;
            }
        } else if !std::path::Path::new(&resolved).is_dir() {
            self.notice = Some(format!("{} is not a folder.", resolved.display()));
            return false;
        }
        let resolved = resolved.display().to_string();
        if let Some(index) = self.workspaces.iter().position(|w| w.path == resolved) {
            self.workspace = index;
        } else {
            self.workspaces.insert(
                0,
                WorkspaceEntry {
                    path: resolved.clone(),
                    label: shorten_path(&resolved),
                    sessions: 0,
                },
            );
            self.workspace = 0;
        }
        self.notice = None;
        self.focus = NewSessionFocus::Workspace;
        self.path = Input::new();
        self.path.set_theme(&self.theme);
        true
    }

    /// Open the path editor, pre-filled with something to edit rather than an
    /// empty box (the current workspace, keeping its trailing separator).
    fn start_new_workspace(&mut self) {
        let seed = self
            .current_workspace()
            .map(|path| {
                let mut seed = path.to_string();
                if !seed.ends_with(std::path::MAIN_SEPARATOR) {
                    seed.push(std::path::MAIN_SEPARATOR);
                }
                seed
            })
            .unwrap_or_default();
        self.path = Input::new();
        self.path.set_theme(&self.theme);
        self.path.set_value(&seed, None);
        self.path.focused = true;
        self.focus = NewSessionFocus::NewWorkspace;
        self.notice = None;
    }

    /// One key press.
    pub fn handle_key(&mut self, key: &str) -> NewSessionAction {
        // The path editor is a plain text box plus `enter`/`escape`.
        if self.focus == NewSessionFocus::NewWorkspace {
            return match key {
                "enter" => NewSessionAction::CreateWorkspace {
                    path: self.path.get_value().trim().to_string(),
                },
                "escape" => {
                    self.focus = NewSessionFocus::Workspace;
                    self.notice = None;
                    NewSessionAction::Changed
                }
                _ => {
                    if self.path.handle_key(key) {
                        NewSessionAction::Changed
                    } else {
                        NewSessionAction::Pass
                    }
                }
            };
        }

        match key {
            "tab" => self.set_focus(self.focus.next()),
            "shift+tab" => self.set_focus(self.focus.previous()),
            "enter" => NewSessionAction::Submit,
            "escape" => NewSessionAction::Cancel,
            // The pickers move with the arrows; the prompt keeps them for the
            // cursor, exactly as it does in the conversation box.
            "left" | "h" if self.focus != NewSessionFocus::Prompt => self.cycle(-1),
            "right" | "l" if self.focus != NewSessionFocus::Prompt => self.cycle(1),
            "up" if self.focus != NewSessionFocus::Prompt => self.set_focus(self.focus.previous()),
            "down" if self.focus != NewSessionFocus::Prompt => self.set_focus(self.focus.next()),
            "space" if self.focus != NewSessionFocus::Prompt => self.cycle(1),
            "n" if self.focus == NewSessionFocus::Workspace => {
                self.start_new_workspace();
                NewSessionAction::Changed
            }
            _ => {
                if self.focus != NewSessionFocus::Prompt {
                    return NewSessionAction::Pass;
                }
                if self.input.handle_key(key) {
                    NewSessionAction::Changed
                } else {
                    NewSessionAction::Pass
                }
            }
        }
    }

    fn set_focus(&mut self, focus: NewSessionFocus) -> NewSessionAction {
        self.focus = focus;
        self.input.focused = focus == NewSessionFocus::Prompt;
        self.notice = None;
        NewSessionAction::Changed
    }

    /// Move the focused picker one step (`delta` = ±1), wrapping at the ends.
    fn cycle(&mut self, delta: i32) -> NewSessionAction {
        match self.focus {
            NewSessionFocus::Workspace => {
                if self.workspaces.is_empty() {
                    return NewSessionAction::Changed;
                }
                let len = self.workspaces.len() as i32;
                self.workspace = ((self.workspace as i32 + delta).rem_euclid(len)) as usize;
            }
            NewSessionFocus::Model => {
                if self.models.is_empty() {
                    return NewSessionAction::Changed;
                }
                let len = self.models.len() as i32;
                self.model = ((self.model as i32 + delta).rem_euclid(len)) as usize;
            }
            NewSessionFocus::Thinking => {
                let len = THINKING_LEVELS.len() as i32;
                self.thinking = ((self.thinking as i32 + delta).rem_euclid(len)) as usize;
            }
            NewSessionFocus::Prompt | NewSessionFocus::NewWorkspace => {}
        }
        NewSessionAction::Changed
    }

    /// The prompt as it will be sent, attachments included. `None` when the
    /// draft is empty (a session can still be started then).
    pub fn take_prompt(&mut self) -> Option<(String, crate::components::input::PendingDraft)> {
        let raw = self.input.get_value().to_string();
        if raw.trim().is_empty() {
            return None;
        }
        let pending = self.input.take_pending();
        Some((raw, pending))
    }

    /// The paste the screen could not take (too long, refused by the box).
    pub fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    pub fn set_notice(&mut self, notice: String) {
        self.notice = Some(notice);
    }

    // ─── Rendering ─────────────────────────────────────────────────────

    /// The rows above the prompt box, exactly `height` of them: the title and
    /// the three pickers sit at the bottom of this area so the whole thing
    /// reads as one block above the box whatever the terminal height.
    pub fn render_above(&mut self, width: usize, height: usize) -> Vec<String> {
        if width == 0 || height == 0 {
            return Vec::new();
        }
        let mut rows: Vec<String> = Vec::new();
        rows.push(fg(self.theme.accent as u8, &bold("New session")));
        rows.push(dim(
            "Ctrl+N opens this from anywhere.  Enter starts the session.",
        ));
        rows.push(String::new());
        rows.extend(self.picker_rows(width));
        if let Some(notice) = self.notice.clone() {
            rows.push(String::new());
            rows.push(fg(
                self.theme.error as u8,
                &truncate_to_width(&notice, width, &TruncateOptions::default()),
            ));
        }

        // Bottom-align inside the area: the blank rows go above, so the block
        // stays next to the box it belongs to. On a terminal too short for the
        // whole block the *title* is what goes — the pickers are the part the
        // reader has to reach.
        if rows.len() > height {
            rows.drain(..rows.len() - height);
        } else {
            let mut out = vec![String::new(); height - rows.len()];
            out.extend(rows);
            rows = out;
        }
        // Width-total like every other chrome row: nothing spills past the
        // last column, however narrow the terminal is.
        rows.into_iter()
            .map(|row| truncate_to_width(&row, width, &TruncateOptions::default()))
            .collect()
    }

    fn picker_rows(&mut self, width: usize) -> Vec<String> {
        let mut rows = Vec::new();
        rows.push(self.picker_row(
            "Workspace",
            NewSessionFocus::Workspace,
            self.workspace_value(),
            width,
        ));
        rows.push(self.picker_row("Model", NewSessionFocus::Model, self.model_value(), width));
        rows.push(self.picker_row(
            "Thinking",
            NewSessionFocus::Thinking,
            format!("‹ {} ›", self.current_thinking()),
            width,
        ));
        if self.focus == NewSessionFocus::NewWorkspace {
            // The field sits directly under the row it belongs to; what it
            // does is on the hint line under the prompt box, so it is not
            // said twice.
            for line in self.path.render(width.saturating_sub(4)) {
                rows.push(format!("    {line}"));
            }
        }
        rows
    }

    fn workspace_value(&self) -> String {
        match self.workspaces.get(self.workspace) {
            Some(entry) => {
                let count = if entry.sessions > 0 {
                    format!(" ({} session{})", entry.sessions, plural(entry.sessions))
                } else {
                    String::new()
                };
                format!("‹ {} ›{count}", entry.label)
            }
            None => "‹ no folder yet — press n to name one ›".to_string(),
        }
    }

    fn model_value(&self) -> String {
        match self.models.get(self.model) {
            Some((id, label)) if !label.is_empty() && label != id => {
                format!("‹ {label} ({id}) ›")
            }
            Some((id, _)) => format!("‹ {id} ›"),
            None => "‹ loading… ›".to_string(),
        }
    }

    fn picker_row(
        &self,
        title: &str,
        focus: NewSessionFocus,
        value: String,
        width: usize,
    ) -> String {
        let active = self.focus == focus;
        let marker = if active { "›" } else { " " };
        let text = format!("{marker} {}{value}", pad_end(&format!("{title:<10}"), 0));
        let text = truncate_to_width(&text, width, &TruncateOptions::default());
        if active {
            fg(self.theme.accent as u8, &bold(&text))
        } else {
            dim(&text)
        }
    }

    /// The line under the prompt box: what the box accepts, and how to move.
    pub fn hints(&self, width: usize) -> String {
        let hint = match self.focus {
            NewSessionFocus::Prompt => {
                "@ files · # sessions · / commands · Tab for workspace, model, thinking · Enter to start"
            }
            NewSessionFocus::Workspace => {
                "←/→ change workspace · n new folder · Tab for the next row"
            }
            NewSessionFocus::Model => "←/→ change model · Tab for the next row",
            NewSessionFocus::Thinking => "←/→ change thinking level · Tab for the prompt",
            NewSessionFocus::NewWorkspace => "type a folder path · enter creates it · esc cancels",
        };
        truncate_to_width(&dim(hint), width, &TruncateOptions::default())
    }
}

/// The base a typed workspace path is relative to: the folder the TUI was
/// started in, which is what the user sees as "here".
fn cwd_base() -> String {
    std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_default()
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

/// `$HOME` shortened to `~`, and a long path tail-truncated — the same rule the
/// session rows use, so one folder is spelled the same everywhere.
pub fn shorten_path(path: &str) -> String {
    let home = crate::home::home_dir_or_default().display().to_string();
    let shown = if !home.is_empty() && path == home {
        "~".to_string()
    } else if !home.is_empty() && path.starts_with(&format!("{home}/")) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
    };
    if visible_width(&shown) > 48 {
        let tail: String = shown
            .chars()
            .skip(shown.chars().count().saturating_sub(47))
            .collect();
        format!("…{tail}")
    } else {
        shown
    }
}

/// Left-pad `text` to `width` visible columns (no-op when it is already wide).
fn pad_end(text: &str, width: usize) -> String {
    let visible = visible_width(text);
    if visible >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - visible))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::strip_ansi_codes;

    const W: usize = 80;

    fn screen() -> NewSessionScreen {
        let mut screen = NewSessionScreen::new(Some("/tmp/alpha"), Some("m/one"), Some("low"));
        screen.set_workspaces(vec![
            WorkspaceEntry {
                path: "/tmp/alpha".into(),
                label: "alpha".into(),
                sessions: 2,
            },
            WorkspaceEntry {
                path: "/tmp/beta".into(),
                label: "beta".into(),
                sessions: 0,
            },
        ]);
        screen.set_models(vec![
            ("m/one".into(), "One".into()),
            ("m/two".into(), "Two".into()),
        ]);
        screen
    }

    #[test]
    fn a_new_screen_starts_from_the_session_it_was_opened_over() {
        let screen = screen();
        assert_eq!(screen.current_workspace(), Some("/tmp/alpha"));
        assert_eq!(screen.current_model(), Some("m/one"));
        assert_eq!(screen.current_thinking(), "low");
        assert_eq!(screen.focus(), NewSessionFocus::Prompt);
    }

    #[test]
    fn tab_and_shift_tab_walk_the_rows_and_wrap() {
        let mut screen = screen();
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(screen.focus());
            screen.handle_key("tab");
        }
        assert_eq!(
            seen,
            vec![
                NewSessionFocus::Prompt,
                NewSessionFocus::Workspace,
                NewSessionFocus::Model,
                NewSessionFocus::Thinking,
            ]
        );
        assert_eq!(
            screen.focus(),
            NewSessionFocus::Prompt,
            "the fifth tab wraps"
        );
        screen.handle_key("shift+tab");
        assert_eq!(screen.focus(), NewSessionFocus::Thinking);
    }

    #[test]
    fn the_pickers_cycle_and_wrap_at_both_ends() {
        let mut screen = screen();
        screen.handle_key("tab"); // workspace
        assert_eq!(screen.current_workspace(), Some("/tmp/alpha"));
        screen.handle_key("right");
        assert_eq!(screen.current_workspace(), Some("/tmp/beta"));
        screen.handle_key("right");
        assert_eq!(screen.current_workspace(), Some("/tmp/alpha"));
        screen.handle_key("left");
        assert_eq!(screen.current_workspace(), Some("/tmp/beta"));

        screen.handle_key("tab"); // model
        assert_eq!(screen.current_model(), Some("m/one"));
        screen.handle_key("space");
        assert_eq!(screen.current_model(), Some("m/two"));

        screen.handle_key("tab"); // thinking
        screen.select_thinking("off");
        assert_eq!(screen.current_thinking(), "off");
        screen.handle_key("left");
        assert_eq!(screen.current_thinking(), "xhigh");
        // A level this build does not know leaves the choice alone.
        screen.select_thinking("turbo");
        assert_eq!(screen.current_thinking(), "xhigh");
    }

    #[test]
    fn the_prompt_keeps_the_typing_keys_and_the_pickers_do_not() {
        let mut screen = screen();
        for ch in "hi @a.rs".chars() {
            screen.handle_key(&ch.to_string());
        }
        assert_eq!(screen.input.get_value(), "hi @a.rs");
        // On a picker row the same letters are not typed into the prompt.
        screen.handle_key("tab");
        screen.handle_key("j");
        assert_eq!(screen.input.get_value(), "hi @a.rs");
    }

    #[test]
    fn enter_submits_and_escape_cancels() {
        let mut screen = screen();
        assert_eq!(screen.handle_key("enter"), NewSessionAction::Submit);
        assert_eq!(screen.handle_key("escape"), NewSessionAction::Cancel);
    }

    #[test]
    fn a_workspace_path_is_created_and_selected_or_explained() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut screen = screen();
        let target = dir.path().join("new-ws");
        assert!(screen.accept_workspace(&target.display().to_string(), true));
        assert!(target.is_dir());
        assert_eq!(
            screen.current_workspace(),
            Some(target.display().to_string().as_str())
        );
        assert_eq!(screen.focus(), NewSessionFocus::Workspace);
        assert!(screen.notice().is_none());

        // A path that cannot be created keeps the screen on the field and
        // says why; it is not silently adopted.
        let refused = if cfg!(windows) {
            "Z:/definitely/not/a/drive".to_string()
        } else {
            "/proc/definitely/not/creatable".to_string()
        };
        assert!(!screen.accept_workspace(&refused, true));
        assert!(screen.notice().is_some());
    }

    #[test]
    fn the_editor_opens_from_the_workspace_row_only() {
        let mut screen = screen();
        screen.handle_key("n");
        assert_eq!(
            screen.focus(),
            NewSessionFocus::Prompt,
            "the prompt keeps n"
        );
        screen.handle_key("tab");
        screen.handle_key("n");
        assert_eq!(screen.focus(), NewSessionFocus::NewWorkspace);
        // The field starts from the selected folder, so a sibling is a few
        // keystrokes away and an absolute path still works (it overwrites).
        assert!(screen
            .new_workspace_path()
            .unwrap()
            .starts_with("/tmp/alpha"));
        assert_eq!(
            screen.handle_key("escape"),
            NewSessionAction::Changed,
            "escape leaves the editor, not the screen"
        );
        assert_eq!(screen.focus(), NewSessionFocus::Workspace);
    }

    #[test]
    fn the_surface_is_width_total_at_every_size() {
        for width in [0usize, 1, 2, 3, 10, 40, 120] {
            for height in [0usize, 1, 4, 20] {
                let mut screen = screen();
                let above = screen.render_above(width, height);
                assert_eq!(above.len().min(height), above.len());
                assert!(above.len() <= height, "height {height}: {}", above.len());
                for row in &above {
                    let cols = crate::utils::visible_width(&strip_ansi_codes(row));
                    assert!(
                        cols <= width.max(3),
                        "width {width}: row {cols} cells: {row:?}"
                    );
                }
                let hint = screen.hints(width);
                let cols = crate::utils::visible_width(&strip_ansi_codes(&hint));
                assert!(cols <= width.max(3), "hint at width {width}: {cols}");
            }
        }
    }

    #[test]
    fn a_short_terminal_keeps_the_pickers_and_drops_the_title() {
        let mut screen = screen();
        let above = screen.render_above(W, 5);
        let text: Vec<String> = above.iter().map(|row| strip_ansi_codes(row)).collect();
        assert_eq!(text.len(), 5);
        assert!(
            text.iter().any(|row| row.contains("Thinking")),
            "the last picker survives: {text:?}"
        );
        assert!(
            text.iter().any(|row| row.contains("Workspace")),
            "and so does the first: {text:?}"
        );
        assert!(
            !text.iter().any(|row| row.contains("New session")),
            "the title is what goes: {text:?}"
        );
    }

    #[test]
    fn the_rows_name_every_choice() {
        let mut screen = screen();
        let text: String = screen
            .render_above(W, 20)
            .iter()
            .map(|row| strip_ansi_codes(row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Workspace"), "{text}");
        assert!(text.contains("alpha"), "{text}");
        assert!(text.contains("2 sessions"), "{text}");
        assert!(text.contains("One (m/one)"), "{text}");
        assert!(text.contains("low"), "{text}");
        assert!(screen.hints(W).contains("@ files"), "{}", screen.hints(W));
    }

    #[test]
    fn unknown_values_and_an_empty_list_are_reported_not_invented() {
        let mut screen = NewSessionScreen::new(None, None, None);
        assert_eq!(screen.current_workspace(), None);
        assert_eq!(screen.current_model(), None);
        let text: String = screen
            .render_above(W, 20)
            .iter()
            .map(|row| strip_ansi_codes(row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("no folder yet"), "{text}");
        assert!(text.contains("loading"), "{text}");
        // Cycling an empty list is a no-op rather than a panic.
        screen.handle_key("tab");
        screen.handle_key("right");
        assert_eq!(screen.current_workspace(), None);
    }

    #[test]
    fn a_model_list_replace_keeps_the_selected_model() {
        let mut screen = screen();
        screen.handle_key("tab");
        screen.handle_key("tab"); // model
        screen.handle_key("right");
        assert_eq!(screen.current_model(), Some("m/two"));
        screen.set_models(vec![
            ("m/zero".into(), "Zero".into()),
            ("m/two".into(), "Two".into()),
        ]);
        assert_eq!(screen.current_model(), Some("m/two"));
    }
}
