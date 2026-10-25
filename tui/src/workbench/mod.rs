//! The workbench — a full-screen terminal UI for FutureOS.
//!
//! This is **a second TUI**, not a mode of the inline one. `future-tui` renders
//! a conversation by appending rows into the terminal's own scrollback and
//! diffing the tail; that design is why it feels like a shell transcript, and it
//! is exactly what a full-screen surface cannot be built out of — a workbench
//! owns the whole screen, repaints absolute rows, and resolves mouse clicks
//! against what it painted. Wrapping the latter around the former would mean two
//! renderers fighting for one output stream.
//!
//! So the split is:
//!
//! * **This module tree** owns the new surface: its own layout, palette, hit
//!   map, mouse protocol, panels and launchpad, and its own entry point
//!   (`future-workbench`). It never calls `app::App`.
//! * **The existing crate** donates the reusable leaves: the gRPC client
//!   (`rpc`), key parsing (`keys`), width/markdown helpers (`utils`,
//!   `components::markdown`), the conversation renderer
//!   (`components::chat_area`) and the terminal backend (`terminal`).
//!
//! The contract between the two halves is [`WorkbenchData`] (see `state`): the
//! surface paints from it and returns [`WbAction`]s; whoever fills the data and
//! applies the actions is the app. Today that is the demo adapter; the agent
//! adapter is the next slice, and it can be written without touching a single
//! drawing function here.
//!
//! Rendering rule, enforced by every row going through `ansi::fit`: the frame is
//! exactly `h` rows of exactly `w` display columns — for any data, at any size,
//! including 1×1.

pub mod ansi;
pub mod demo;
pub mod launchpad;
pub mod layout;
pub mod live;
pub mod mouse;
pub mod panels;
pub mod run;
pub mod screen;
pub mod state;
pub mod theme;
pub mod widgets;

use state::{PanelId, WorkbenchData};
use theme::{Palette, PALETTES};

/// Which input row takes keys on the launchpad. `Prompt` is the default; the
/// others are the parameter chips, reachable with `tab`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LaunchField {
    #[default]
    Prompt,
    Model,
    Effort,
    Mode,
    Permission,
}

impl LaunchField {
    fn next(self) -> Self {
        match self {
            Self::Prompt => Self::Model,
            Self::Model => Self::Effort,
            Self::Effort => Self::Mode,
            Self::Mode => Self::Permission,
            Self::Permission => Self::Prompt,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Prompt => Self::Permission,
            Self::Model => Self::Prompt,
            Self::Effort => Self::Model,
            Self::Mode => Self::Effort,
            Self::Permission => Self::Mode,
        }
    }
}

/// A layer drawn over the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    Help,
    /// The `/` command palette.
    Palette,
    /// `ctrl+b` on a terminal too narrow for the sidebar: the current panel
    /// takes the body.
    PanelFull,
}

/// Which pane owns the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneId {
    Input,
    Chat,
    Sidebar,
}

/// What the app has to do about a key or click. The workbench never mutates the
/// conversation itself: it returns intents, the app applies them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WbAction {
    /// Replace the draft with this text and cursor (byte offset).
    EditInput {
        text: String,
        cursor: usize,
    },
    /// `enter` on a non-empty draft.
    Submit(String),
    /// A command entry points at the agent: `/new`, `/sessions`, `reply-detail`…
    /// The leading slash is included, exactly like a typed command.
    Command(String),
    /// Move the conversation by `delta` rows (positive = towards the tail).
    Scroll(i32),
    /// The reader reached the top: the app should fetch an older page.
    LoadOlder,
    /// Open this conversation.
    SwitchSession(String),
    SwitchPanel(PanelId),
    /// The divider was dragged to this width. The app persists it on release.
    ResizeSidebar(u16),
    /// The divider drag finished; persist the width now.
    PersistSidebar,
    FocusPane(PaneId),
    /// Leave the workbench.
    Quit,
    /// Nothing but the surface changed; repaint.
    Redraw,
}

/// One input event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WbEvent {
    /// A normalised key id (`keys::parse_key`).
    Key(String),
    Mouse(mouse::MouseEvent),
    Resize(u16, u16),
    /// A bracketed paste.
    Paste(String),
    /// One animation tick (spinner, elapsed time).
    Tick,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    start_x: u16,
    start_width: u16,
}

/// The surface's own state: where the panels and the pointers are.
///
/// Deliberately *not* the data — the conversation, the todos and the usage
/// figures live in [`WorkbenchData`], which the app owns and passes in. Keeping
/// them apart is what lets the surface be tested with fixtures and later fed by
/// a real session without either side knowing the other's shape.
pub struct Workbench {
    pub palette: Palette,
    pub panel: PanelId,
    pub sidebar_focus: bool,
    /// Is the sidebar allowed to show? `ctrl+b` toggles it; the width still
    /// decides whether it *can*.
    pub sidebar_visible: bool,
    pub sidebar_w: u16,
    pub panel_scroll: [usize; 5],
    pub overlay: Option<Overlay>,
    pub overlay_scroll: usize,
    pub palette_query: String,
    pub palette_index: usize,
    pub launch_field: LaunchField,
    pub chat_scroll: usize,
    pub tick: usize,
    /// The frame size of the last render; mouse coordinates are resolved
    /// against it.
    last_frame: (u16, u16),
    hit: layout::HitMap,
    cursor: Option<(u16, u16)>,
    drag: Option<Drag>,
}

impl Default for Workbench {
    fn default() -> Self {
        Self::new()
    }
}

impl Workbench {
    pub fn new() -> Self {
        Self {
            palette: theme::FUTURE_DARK,
            panel: PanelId::Todo,
            sidebar_focus: false,
            sidebar_visible: true,
            sidebar_w: layout::SIDEBAR_DEFAULT,
            panel_scroll: [0; 5],
            overlay: None,
            overlay_scroll: 0,
            palette_query: String::new(),
            palette_index: 0,
            launch_field: LaunchField::Prompt,
            chat_scroll: 0,
            tick: 0,
            last_frame: (0, 0),
            hit: layout::HitMap::new(),
            cursor: None,
            drag: None,
        }
    }

    /// `/theme` — adopt the next built-in palette and return its id.
    pub fn cycle_theme(&mut self) -> &'static str {
        self.palette = theme::next(&self.palette);
        self.palette.id
    }

    /// Adopt a palette by id; an unknown id is the default (and is reported).
    pub fn set_theme(&mut self, id: &str) -> bool {
        let resolved = theme::resolve(Some(id));
        self.palette = resolved;
        PALETTES
            .iter()
            .any(|palette| palette.id.eq_ignore_ascii_case(id))
    }

    /// Render one frame: exactly `h` rows, each exactly `w` display columns.
    pub fn render(&mut self, data: &WorkbenchData, w: u16, h: u16) -> Vec<String> {
        let rows = screen::compose(self, data, w, h);
        debug_assert_eq!(rows.len(), h as usize, "the frame must be h rows");
        rows
    }

    /// Where the hardware cursor belongs, in frame coordinates.
    pub fn cursor(&self) -> Option<(u16, u16)> {
        self.cursor
    }

    /// What was painted at `(x, y)` in the last frame.
    pub fn hit_at(&self, x: u16, y: u16) -> Option<&str> {
        self.hit.hit(x, y)
    }

    /// The hit map of the last frame (tests, diagnostics).
    pub fn hits(&self) -> &layout::HitMap {
        &self.hit
    }

    /// `/` palette entries that match the current query: `(command, description)`.
    pub fn palette_entries(&self) -> Vec<(&'static str, &'static str)> {
        let needle = self.palette_query.to_lowercase();
        COMMANDS
            .iter()
            .copied()
            .filter(|(command, description)| {
                needle.is_empty()
                    || command.to_lowercase().contains(&needle)
                    || description.contains(&needle)
            })
            .collect()
    }

    /// The highlighted palette entry, if the palette is open.
    pub fn palette_selection(&self) -> Option<(&'static str, &'static str)> {
        let entries = self.palette_entries();
        entries
            .get(self.palette_index.min(entries.len().saturating_sub(1)))
            .copied()
    }

    pub fn handle(&mut self, event: WbEvent, data: &WorkbenchData) -> Vec<WbAction> {
        match event {
            WbEvent::Tick => {
                self.tick = self.tick.wrapping_add(1);
                Vec::new()
            }
            WbEvent::Resize(_, _) => vec![WbAction::Redraw],
            WbEvent::Paste(text) => {
                let mut view = data.input.clone();
                view.insert(&text);
                vec![WbAction::EditInput {
                    text: view.text,
                    cursor: view.cursor,
                }]
            }
            WbEvent::Mouse(event) => self.handle_mouse(event, data),
            WbEvent::Key(key) => self.handle_key(&key, data),
        }
    }

    fn handle_key(&mut self, key: &str, data: &WorkbenchData) -> Vec<WbAction> {
        if let Some(overlay) = self.overlay {
            return self.handle_overlay_key(overlay, key);
        }

        match key {
            "ctrl+c" | "ctrl+q" => vec![WbAction::Quit],
            "ctrl+b" => {
                // Hidden → shown and focused → shown and unfocused → hidden.
                // On a terminal too narrow for a sidebar the same key opens the
                // panel full-screen instead, which is the only way to reach it
                // there.
                let frame = layout::layout(
                    self.last_frame.0,
                    self.last_frame.1,
                    if self.sidebar_visible {
                        self.sidebar_w
                    } else {
                        0
                    },
                );
                if frame.sidebar.is_none() {
                    self.overlay = match self.overlay {
                        Some(Overlay::PanelFull) => None,
                        _ => Some(Overlay::PanelFull),
                    };
                } else if !self.sidebar_visible {
                    self.sidebar_visible = true;
                    self.sidebar_focus = true;
                } else if !self.sidebar_focus {
                    self.sidebar_focus = true;
                } else {
                    self.sidebar_visible = false;
                    self.sidebar_focus = false;
                }
                vec![WbAction::Redraw]
            }
            "?" => {
                self.overlay = Some(Overlay::Help);
                self.overlay_scroll = 0;
                vec![WbAction::Redraw]
            }
            "/" if data.input.text.is_empty() => {
                self.overlay = Some(Overlay::Palette);
                self.palette_query.clear();
                self.palette_index = 0;
                vec![WbAction::Redraw]
            }
            "tab" => {
                if data.on_launchpad() {
                    self.launch_field = self.launch_field.next();
                } else {
                    self.sidebar_focus = !self.sidebar_focus;
                }
                vec![WbAction::Redraw]
            }
            "shift+tab" => {
                if data.on_launchpad() {
                    self.launch_field = self.launch_field.previous();
                }
                vec![WbAction::Redraw]
            }
            "enter" => {
                let text = data.input.text.trim().to_string();
                if text.is_empty() {
                    Vec::new()
                } else {
                    vec![WbAction::Submit(text)]
                }
            }
            "backspace" => {
                let mut view = data.input.clone();
                view.backspace();
                vec![WbAction::EditInput {
                    text: view.text,
                    cursor: view.cursor,
                }]
            }
            "left" => {
                let mut view = data.input.clone();
                view.move_left();
                vec![WbAction::EditInput {
                    text: view.text,
                    cursor: view.cursor,
                }]
            }
            "right" => {
                let mut view = data.input.clone();
                view.move_right();
                vec![WbAction::EditInput {
                    text: view.text,
                    cursor: view.cursor,
                }]
            }
            "ctrl+u" => vec![WbAction::EditInput {
                text: String::new(),
                cursor: 0,
            }],
            "up" => self.scroll(-1, data),
            "down" => self.scroll(1, data),
            "pageUp" => self.scroll(-10, data),
            "pageDown" => self.scroll(10, data),
            // Space is a character in the draft, not the "select" key the
            // menus use: the prompt has to be able to contain words.
            "space" => {
                let mut view = data.input.clone();
                view.insert(" ");
                vec![WbAction::EditInput {
                    text: view.text,
                    cursor: view.cursor,
                }]
            }
            "escape" => {
                self.chat_scroll = 0;
                vec![WbAction::Redraw]
            }
            other => {
                // Every other printable key is text. Control chords are not: a
                // key the surface does not know must not silently become a
                // character in the draft.
                if other.chars().count() == 1
                    && !other.starts_with("ctrl+")
                    && !other.starts_with("alt+")
                {
                    let mut view = data.input.clone();
                    view.insert(other);
                    vec![WbAction::EditInput {
                        text: view.text,
                        cursor: view.cursor,
                    }]
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// Move the conversation (`up` negative = towards the tail) or the focused
    /// panel's list, and ask for an older page when the top is reached.
    ///
    /// One method for the keys and the wheel: a scroll offset the surface keeps
    /// in two places would disagree with itself the first time a key and the
    /// wheel alternated.
    fn scroll(&mut self, up: i32, data: &WorkbenchData) -> Vec<WbAction> {
        if self.sidebar_focus {
            let index = self.panel.index();
            let next = self.panel_scroll[index] as i32 - up;
            self.panel_scroll[index] = next.max(0) as usize;
            return vec![WbAction::Redraw];
        }
        let total = data.messages.len();
        // `chat_scroll` counts rows up from the tail: 0 is pinned to the newest
        // row, and the ceiling is "everything loaded is above the viewport".
        let ceiling = total.saturating_sub(1);
        let at_top = self.chat_scroll >= ceiling && total > 0;
        let next = (self.chat_scroll as i32 - up).clamp(0, ceiling as i32);
        self.chat_scroll = next as usize;
        if up < 0 && at_top {
            // Nothing older is loaded: the app may fetch one more page.
            return vec![WbAction::LoadOlder];
        }
        vec![WbAction::Scroll(up)]
    }

    fn handle_overlay_key(&mut self, overlay: Overlay, key: &str) -> Vec<WbAction> {
        match overlay {
            Overlay::Help => match key {
                "escape" | "q" | "?" | "enter" => {
                    self.overlay = None;
                    vec![WbAction::Redraw]
                }
                "down" | "j" => {
                    self.overlay_scroll += 1;
                    vec![WbAction::Redraw]
                }
                "up" | "k" => {
                    self.overlay_scroll = self.overlay_scroll.saturating_sub(1);
                    vec![WbAction::Redraw]
                }
                "pageDown" | "space" => {
                    self.overlay_scroll += 10;
                    vec![WbAction::Redraw]
                }
                "pageUp" => {
                    self.overlay_scroll = self.overlay_scroll.saturating_sub(10);
                    vec![WbAction::Redraw]
                }
                _ => Vec::new(),
            },
            Overlay::PanelFull => match key {
                "escape" | "q" | "ctrl+b" => {
                    self.overlay = None;
                    vec![WbAction::Redraw]
                }
                "down" | "j" => {
                    let index = self.panel.index();
                    self.panel_scroll[index] += 1;
                    vec![WbAction::Redraw]
                }
                "up" | "k" => {
                    let index = self.panel.index();
                    self.panel_scroll[index] = self.panel_scroll[index].saturating_sub(1);
                    vec![WbAction::Redraw]
                }
                _ => Vec::new(),
            },
            Overlay::Palette => match key {
                "escape" => {
                    self.overlay = None;
                    vec![WbAction::Redraw]
                }
                "enter" => {
                    let chosen = self
                        .palette_selection()
                        .map(|(command, _)| command.to_string());
                    self.overlay = None;
                    match chosen {
                        Some(command) => vec![WbAction::Command(command)],
                        None => vec![WbAction::Redraw],
                    }
                }
                "up" => {
                    self.palette_index = self.palette_index.saturating_sub(1);
                    vec![WbAction::Redraw]
                }
                "down" => {
                    let count = self.palette_entries().len();
                    if self.palette_index + 1 < count {
                        self.palette_index += 1;
                    }
                    vec![WbAction::Redraw]
                }
                "backspace" => {
                    self.palette_query.pop();
                    self.palette_index = 0;
                    vec![WbAction::Redraw]
                }
                other if other.chars().count() == 1 && !other.starts_with("ctrl+") => {
                    self.palette_query.push_str(other);
                    self.palette_index = 0;
                    vec![WbAction::Redraw]
                }
                _ => Vec::new(),
            },
        }
    }

    fn handle_mouse(&mut self, event: mouse::MouseEvent, data: &WorkbenchData) -> Vec<WbAction> {
        let (w, h) = self.last_frame;
        let frame = layout::layout(
            w,
            h,
            if self.sidebar_visible {
                self.sidebar_w
            } else {
                0
            },
        );
        match event.kind {
            mouse::MouseKind::WheelUp | mouse::MouseKind::WheelDown => {
                let delta = if event.kind == mouse::MouseKind::WheelUp {
                    -3
                } else {
                    3
                };
                if frame
                    .sidebar
                    .is_some_and(|rect| rect.contains(event.x, event.y))
                {
                    let index = self.panel.index();
                    let next = self.panel_scroll[index] as i32 + delta;
                    self.panel_scroll[index] = next.max(0) as usize;
                    return vec![WbAction::Redraw];
                }
                // The wheel's sign is "up = negative", the same as the keys.
                self.scroll(delta, data)
            }
            mouse::MouseKind::Press => match self.hit.hit(event.x, event.y).map(str::to_string) {
                Some(id) => self.click(&id, data),
                None => Vec::new(),
            },
            mouse::MouseKind::Drag => {
                if let Some(drag) = self.drag {
                    let delta = event.x as i32 - drag.start_x as i32;
                    let width = (drag.start_width as i32 + delta)
                        .clamp(layout::SIDEBAR_MIN as i32, layout::SIDEBAR_MAX as i32)
                        as u16;
                    if width != self.sidebar_w {
                        self.sidebar_w = width;
                        return vec![WbAction::ResizeSidebar(width)];
                    }
                }
                Vec::new()
            }
            mouse::MouseKind::Release => {
                if self.drag.take().is_some() {
                    return vec![WbAction::PersistSidebar];
                }
                Vec::new()
            }
            mouse::MouseKind::Move => Vec::new(),
        }
    }

    fn click(&mut self, id: &str, data: &WorkbenchData) -> Vec<WbAction> {
        if id == "input" {
            self.launch_field = LaunchField::Prompt;
            return vec![WbAction::FocusPane(PaneId::Input)];
        }
        if id == "chat" {
            return vec![WbAction::FocusPane(PaneId::Chat)];
        }
        if id == "divider" {
            // The drag starts from where the divider *is*, not where the click
            // landed, so a grab two cells off-centre does not jump the panel.
            self.drag = Some(Drag {
                start_x: self.last_frame.0.saturating_sub(self.sidebar_w + 1),
                start_width: self.sidebar_w,
            });
            return Vec::new();
        }
        if let Some(panel) = id.strip_prefix("sidebar.tab.").and_then(PanelId::parse) {
            self.panel = panel;
            return vec![WbAction::SwitchPanel(panel)];
        }
        if let Some(index) = id
            .strip_prefix("sidebar.row.")
            .and_then(|rest| rest.parse::<usize>().ok())
        {
            return match self.panel {
                PanelId::Sessions => data
                    .sessions
                    .get(index)
                    .map(|session: &state::SessionRow| {
                        vec![WbAction::SwitchSession(session.id.clone())]
                    })
                    .unwrap_or_default(),
                _ => vec![WbAction::Command(format!(
                    "panel.{}",
                    id.trim_start_matches("sidebar.")
                ))],
            };
        }
        if let Some(index) = id
            .strip_prefix("launchpad.session.")
            .and_then(|rest| rest.parse::<usize>().ok())
        {
            return data
                .sessions
                .get(index)
                .map(|session: &state::SessionRow| {
                    vec![WbAction::SwitchSession(session.id.clone())]
                })
                .unwrap_or_default();
        }
        if id.starts_with("launchpad.") {
            return vec![WbAction::Command(format!("/{id}"))];
        }
        Vec::new()
    }

    /// The frame size of the last render — mouse coordinates are resolved
    /// against it, so a click that arrives between resize and repaint lands
    /// somewhere defined instead of against a stale layout.
    pub(crate) fn set_last_frame(&mut self, w: u16, h: u16) {
        self.last_frame = (w, h);
    }

    // Fields the screen composer fills in while drawing.
    pub(crate) fn set_cursor(&mut self, cursor: Option<(u16, u16)>) {
        self.cursor = cursor;
    }

    pub(crate) fn hit_map(&mut self) -> &mut layout::HitMap {
        &mut self.hit
    }

    pub(crate) fn clear_hits(&mut self) {
        self.hit.clear();
    }

    pub(crate) fn set_chat_scroll(&mut self, scroll: usize) {
        self.chat_scroll = scroll;
    }

    pub(crate) fn chat_scroll_offset(&self) -> usize {
        self.chat_scroll
    }

    pub(crate) fn set_overlay_scroll(&mut self, scroll: usize) {
        self.overlay_scroll = scroll;
    }

    pub(crate) fn overlay_scroll_offset(&self) -> usize {
        self.overlay_scroll
    }
}

/// The command palette's entries: `(command, description)`. Kept here rather
/// than in the app so the palette, the help overlay and the tests share one
/// list.
pub const COMMANDS: [(&str, &str); 12] = [
    ("/new", "新建会话"),
    ("/sessions", "浏览并切换会话"),
    ("/model", "选择模型"),
    ("/reply-detail", "最终输出 / 完整过程"),
    ("/workspace", "workspace 目录"),
    ("/theme", "切换主题"),
    ("/usage", "用量与成本"),
    ("/skills", "技能列表"),
    ("/export", "导出会话"),
    ("/transcript", "完整记录"),
    ("/help", "帮助与快捷键"),
    ("/quit", "退出 workbench"),
];

/// The help overlay's key table.
pub const HELP_KEYS: [(&str, &str); 12] = [
    ("ctrl+n", "新会话（任意位置）"),
    ("ctrl+b", "侧栏：打开 / 聚焦 / 收起"),
    ("tab", "在输入框与侧栏之间移动"),
    ("↑↓", "滚动聊天，或侧栏列表"),
    ("pageUp / pageDown", "整页滚动聊天"),
    ("/", "命令面板"),
    ("?", "这份帮助"),
    ("ctrl+e", "最终输出 / 完整过程"),
    ("/reply-detail", "同上，直接设置"),
    ("鼠标点击", "切页签、点行、点参数"),
    ("鼠标拖动", "侧栏分隔条调宽"),
    ("ctrl+c", "退出"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workbench::state::{ActivityKind, SessionRow};
    use crate::workbench::theme::FUTURE_DARK;

    fn plain(rows: &[String]) -> String {
        rows.iter()
            .map(|row| crate::utils::strip_ansi_codes(row))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn frame(data: &WorkbenchData, w: u16, h: u16) -> (Workbench, Vec<String>) {
        let mut workbench = Workbench::new();
        let rows = workbench.render(data, w, h);
        (workbench, rows)
    }

    #[test]
    fn a_frame_is_always_the_terminal_size() {
        for data in [demo::launchpad(), demo::chat(), demo::todo()] {
            for (w, h) in [
                (120u16, 38u16),
                (96, 30),
                (88, 24),
                (87, 23),
                (60, 12),
                (20, 6),
                (1, 1),
            ] {
                let (_, rows) = frame(&data, w, h);
                assert_eq!(rows.len(), h as usize, "{}x{h}", w);
                for row in &rows {
                    assert_eq!(ansi::width(row), w as usize, "{w}x{h}: {row:?}");
                }
            }
        }
    }

    #[test]
    fn the_chat_surface_shows_the_header_the_conversation_and_the_status() {
        let data = demo::chat();
        let (workbench, rows) = frame(&data, 120, 38);
        let text = plain(&rows);
        // At 120 columns the header spells its name in the block font, so the
        // literal string is not on screen — the logo and the wordmark are.
        assert!(text.contains('▄'), "the pixel logo is drawn: {text}");
        assert!(text.contains("██"), "the wordmark is drawn: {text}");
        assert!(text.contains("future-tui v0.11.0"), "{text}");
        assert!(text.contains("修复 parser 的 off-by-one"), "{text}");
        assert!(text.contains("dev-huichen"), "{text}");
        assert!(text.contains("2 tool calls"), "{text}");
        assert!(text.contains("deepseek-flash"), "{text}");
        assert!(text.contains("ctx"), "{text}");
        assert!(text.contains("待办"), "the sidebar's tabs: {text}");
        assert!(text.contains("/ 命令"), "{text}");
        assert!(workbench.cursor().is_some(), "the prompt owns the cursor");
    }

    #[test]
    fn the_launchpad_replaces_the_conversation_rather_than_sitting_on_it() {
        let data = demo::launchpad();
        let (workbench, rows) = frame(&data, 120, 38);
        let text = plain(&rows);
        assert!(text.contains("新会话"), "{text}");
        assert!(text.contains("模型"), "{text}");
        // None of the conversation surface's rows are behind it.
        assert!(!text.contains("2 tool calls"), "{text}");
        assert!(
            !text.contains("输入 · enter 发送"),
            "the conversation's input band is not drawn under it: {text}"
        );
        // …and its own prompt row owns the cursor: the row under the box's
        // title, wherever the band starts.
        let box_row = text
            .lines()
            .position(|line| line.contains("新会话"))
            .expect("the box title");
        assert_eq!(
            workbench.cursor().map(|(_, row)| row as usize),
            Some(box_row + 1)
        );
    }

    #[test]
    fn the_sidebar_tabs_switch_the_panel_by_click() {
        let data = demo::chat();
        let (mut workbench, rows) = frame(&data, 120, 38);
        let text = plain(&rows);
        assert!(text.contains("用量"), "{text}");

        // The tab's own region is registered while drawing, so it cannot drift
        // from what is on screen.
        let (rect, _) = workbench
            .hits()
            .regions()
            .iter()
            .find(|(_, id)| id == "sidebar.tab.sessions")
            .cloned()
            .expect("the sessions tab is a target");
        let press = mouse::MouseEvent {
            kind: mouse::MouseKind::Press,
            x: rect.x + 1,
            y: rect.y,
            shift: false,
            alt: false,
            ctrl: false,
        };
        let actions = workbench.handle(WbEvent::Mouse(press), &data);
        assert_eq!(actions, vec![WbAction::SwitchPanel(PanelId::Sessions)]);
        // The panel repaints with the sessions it now shows.
        let text = plain(&workbench.render(&data, 120, 38));
        assert!(text.contains("2m"), "a session's age: {text}");
    }

    #[test]
    fn the_divider_drag_resizes_and_persists() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        let (rect, _) = workbench
            .hits()
            .regions()
            .iter()
            .find(|(_, id)| id == "divider")
            .cloned()
            .expect("the divider is a target");
        let press = mouse::MouseEvent {
            kind: mouse::MouseKind::Press,
            x: rect.x,
            y: rect.y + 2,
            shift: false,
            alt: false,
            ctrl: false,
        };
        assert!(workbench.handle(WbEvent::Mouse(press), &data).is_empty());
        let drag = mouse::MouseEvent {
            kind: mouse::MouseKind::Drag,
            x: rect.x + 6,
            y: rect.y + 2,
            shift: false,
            alt: false,
            ctrl: false,
        };
        let actions = workbench.handle(WbEvent::Mouse(drag), &data);
        assert_eq!(actions, vec![WbAction::ResizeSidebar(42)]);
        let release = mouse::MouseEvent {
            kind: mouse::MouseKind::Release,
            x: rect.x + 6,
            y: rect.y + 2,
            shift: false,
            alt: false,
            ctrl: false,
        };
        assert_eq!(
            workbench.handle(WbEvent::Mouse(release), &data),
            vec![WbAction::PersistSidebar]
        );
        // The drag is over: another move changes nothing.
        assert!(workbench.handle(WbEvent::Mouse(drag), &data).is_empty());
    }

    #[test]
    fn typing_edits_the_draft_and_enter_submits_it() {
        let data = demo::launchpad();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        for key in ["h", "i"] {
            let actions = workbench.handle(WbEvent::Key(key.into()), &data);
            assert_eq!(
                actions,
                vec![WbAction::EditInput {
                    text: key.to_string(),
                    cursor: 1
                }],
                "{key}"
            );
        }
        // The app applies the edit; the next frame sees it. Emulate that.
        let mut data = data;
        data.input.text = "hi".into();
        data.input.cursor = 2;
        assert_eq!(
            workbench.handle(WbEvent::Key("enter".into()), &data),
            vec![WbAction::Submit("hi".into())]
        );
        // An empty draft does not submit.
        let empty = demo::launchpad();
        assert!(workbench
            .handle(WbEvent::Key("enter".into()), &empty)
            .is_empty());
    }

    #[test]
    fn the_palette_filters_and_runs_a_command() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        // `/` on an empty draft opens the palette; it is a command key there,
        // not the character.
        assert_eq!(
            workbench.handle(WbEvent::Key("/".into()), &data),
            vec![WbAction::Redraw]
        );
        assert_eq!(workbench.overlay, Some(Overlay::Palette));
        for key in ["m", "o", "d"] {
            workbench.handle(WbEvent::Key(key.into()), &data);
        }
        let entries = workbench.palette_entries();
        assert!(
            entries.iter().any(|(command, _)| *command == "/model"),
            "{entries:?}"
        );
        assert!(
            entries.iter().all(|(command, _)| command.contains("mod")),
            "only matches are listed: {entries:?}"
        );
        let chosen = workbench.palette_selection().expect("a selection");
        assert_eq!(
            workbench.handle(WbEvent::Key("enter".into()), &data),
            vec![WbAction::Command(chosen.0.to_string())]
        );
        assert_eq!(workbench.overlay, None);
    }

    #[test]
    fn the_help_overlay_scrolls_and_closes_on_escape() {
        let data = demo::chat();
        let (mut workbench, rows) = frame(&data, 120, 38);
        workbench.handle(WbEvent::Key("?".into()), &data);
        let text = plain(&workbench.render(&data, 120, 38));
        assert!(text.contains("帮助"), "{text}");
        assert!(text.contains("ctrl+b"), "{text}");
        // The frame behind it is still the frame: same size, no leftovers.
        assert_eq!(rows.len(), 38);
        workbench.handle(WbEvent::Key("down".into()), &data);
        assert!(workbench.overlay_scroll_offset() > 0);
        workbench.handle(WbEvent::Key("escape".into()), &data);
        assert_eq!(workbench.overlay, None);
    }

    #[test]
    fn a_narrow_terminal_has_no_sidebar_but_ctrl_b_still_reaches_the_panel() {
        let data = demo::todo();
        let (mut workbench, rows) = frame(&data, 70, 24);
        let text = plain(&rows);
        assert!(!text.contains("待办"), "no sidebar at 70 columns: {text}");
        assert!(workbench
            .hits()
            .regions()
            .iter()
            .all(|(_, id)| !id.starts_with("sidebar.")));

        let actions = workbench.handle(WbEvent::Key("ctrl+b".into()), &data);
        assert_eq!(actions, vec![WbAction::Redraw]);
        // The full-screen panel is now the surface: it carries the todos.
        let text = plain(&workbench.render(&data, 70, 24));
        assert!(text.contains("第 1 项"), "{text}");
    }

    #[test]
    fn the_wheel_scrolls_and_reaching_the_top_asks_for_older_rows() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        let wheel = |kind| mouse::MouseEvent {
            kind,
            x: 10,
            y: 12,
            shift: false,
            alt: false,
            ctrl: false,
        };
        // The wheel moves three rows and reports the gesture; the app uses it
        // to know the reader is scrolling, not to move anything itself.
        assert_eq!(
            workbench.handle(WbEvent::Mouse(wheel(mouse::MouseKind::WheelDown)), &data),
            vec![WbAction::Scroll(3)]
        );
        assert_eq!(
            workbench.handle(WbEvent::Mouse(wheel(mouse::MouseKind::WheelUp)), &data),
            vec![WbAction::Scroll(-3)]
        );
        // Enough wheel-up gestures reach the top of what is loaded, and the
        // next one asks for a page above it.
        for _ in 0..8 {
            workbench.handle(WbEvent::Mouse(wheel(mouse::MouseKind::WheelUp)), &data);
        }
        assert_eq!(
            workbench.handle(WbEvent::Mouse(wheel(mouse::MouseKind::WheelUp)), &data),
            vec![WbAction::LoadOlder]
        );
    }

    #[test]
    fn the_tick_drives_the_spinner_without_touching_the_data() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        let before = workbench.render(&data, 120, 38);
        assert!(workbench.handle(WbEvent::Tick, &data).is_empty());
        let after = workbench.render(&data, 120, 38);
        assert_eq!(before.len(), after.len());
        // The spinner glyph is one of the frames, at the activity row.
        let activity = plain(&after)
            .lines()
            .nth(35)
            .unwrap_or_default()
            .to_string();
        assert!(
            widgets::SPINNER
                .iter()
                .any(|glyph| activity.contains(glyph)),
            "{activity}"
        );
        assert_eq!(data.activity.kind, ActivityKind::Thinking);
    }

    #[test]
    fn a_click_on_a_session_row_opens_it() {
        let mut data = demo::chat();
        data.sessions.push(SessionRow {
            id: "s9".into(),
            title: "another conversation".into(),
            workspace: "~/x".into(),
            age: "4d".into(),
            current: false,
        });
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.panel = PanelId::Sessions;
        workbench.render(&data, 120, 38);
        let (rect, _) = workbench
            .hits()
            .regions()
            .iter()
            .find(|(_, id)| id == "sidebar.row.0")
            .cloned()
            .expect("a session row");
        let press = mouse::MouseEvent {
            kind: mouse::MouseKind::Press,
            x: rect.x + 1,
            y: rect.y,
            shift: false,
            alt: false,
            ctrl: false,
        };
        let actions = workbench.handle(WbEvent::Mouse(press), &data);
        assert_eq!(actions, vec![WbAction::SwitchSession("s1".into())]);
    }

    #[test]
    fn the_theme_cycles_between_the_two_built_ins() {
        let mut workbench = Workbench::new();
        assert_eq!(workbench.palette.id, FUTURE_DARK.id);
        assert_eq!(workbench.cycle_theme(), "future-light");
        assert_eq!(workbench.cycle_theme(), "future-dark");
        assert!(workbench.set_theme("future-light"));
        assert_eq!(workbench.palette.id, "future-light");
        assert!(!workbench.set_theme("nope"), "an unknown id is refused");
        assert_eq!(workbench.palette.id, FUTURE_DARK.id);
    }

    #[test]
    fn a_paste_is_an_edit_and_never_a_key() {
        let data = demo::launchpad();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        assert_eq!(
            workbench.handle(WbEvent::Paste("粘贴的文本".into()), &data),
            vec![WbAction::EditInput {
                text: "粘贴的文本".into(),
                cursor: "粘贴的文本".len()
            }]
        );
    }

    #[test]
    fn quit_is_reachable_and_only_from_the_surface() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        assert_eq!(
            workbench.handle(WbEvent::Key("ctrl+c".into()), &data),
            vec![WbAction::Quit]
        );
        // With the palette open, ctrl+c is not text and does not quit either —
        // the palette owns the keys, and escape is its way out.
        workbench.handle(WbEvent::Key("/".into()), &data);
        assert!(workbench
            .handle(WbEvent::Key("ctrl+c".into()), &data)
            .is_empty());
    }
}
