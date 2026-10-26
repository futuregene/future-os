//! The workbench — the FutureOS terminal UI (`future tui`).
//!
//! It normally owns the whole screen, repaints absolute rows, and resolves mouse
//! clicks against what it painted. Linear reading is the explicit exception:
//! its plain transcript goes to native scrollback (see `accessibility`).
//!
//! So the split is:
//!
//! * **This module tree** owns the surface: its own layout, palette, hit map,
//!   mouse protocol, panels and launchpad. `run` is its driver — the one
//!   `future tui` starts.
//! * **The crate around it** donates the reusable leaves: the gRPC client
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

pub mod accessibility;
pub mod ansi;
pub mod demo;
pub mod diff_review;
pub mod editor;
pub mod git;
pub mod i18n;
pub mod launchpad;
pub mod layout;
pub mod live;
pub mod local;
pub mod mouse;
pub mod panels;
pub mod picker;
pub mod preferences;
pub mod run;
pub mod screen;
pub mod scroll;
pub mod state;
pub mod theme;
pub mod widgets;
pub mod workflows;

use picker::{Picker, PickerKind, PickerRow};
use state::{PanelId, WorkbenchData};
use theme::Palette;

use i18n::{t, Key, Language, LanguagePreference};

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
    /// `/model`, `/theme` or `/sessions`: the list picker.
    Picker(PickerKind),
    Queue,
    Approval,
    Diff,
    History,
    Tree,
    Fork,
    Worktree,
    /// Layout, density and the entry to the shortcut editor.
    Preferences,
    Keybindings,
}

/// Which pane owns the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneId {
    Input,
    Chat,
    Sidebar,
}

/// Which token the `@` / `#` menu completes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MentionKind {
    /// `@` — a file under the session's working directory.
    File,
    /// `#` — a conversation, inserted as a `futureos://session/<id>` link, the
    /// same form a conversation link travels as everywhere else.
    Session,
}

impl MentionKind {
    pub fn sigil(self) -> char {
        match self {
            Self::File => '@',
            Self::Session => '#',
        }
    }

    /// The box's title.
    pub fn title(self, lang: Language) -> &'static str {
        t(
            lang,
            match self {
                Self::File => Key::MentionTitleFile,
                Self::Session => Key::MentionTitleSession,
            },
        )
    }

    /// The footer: what the keys do *here*.
    pub fn hint(self, lang: Language) -> &'static str {
        t(
            lang,
            match self {
                Self::File => Key::MentionHintFile,
                Self::Session => Key::MentionHintSession,
            },
        )
    }
}

/// The `@` / `#` menu's own state: which token it completes, where that token
/// starts in the draft, the query typed after the sigil, and the highlight.
///
/// It is *derived* from the draft on every event (see
/// [`Workbench::sync_mention`]) rather than owned by the menu: the query is the
/// text after the sigil, so the menu and the draft cannot disagree.
pub struct Mention {
    pub kind: MentionKind,
    /// Byte offset of the sigil in the draft; an insert replaces `start..cursor`.
    pub start: usize,
    pub query: String,
    pub index: usize,
}

impl std::fmt::Debug for Mention {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mention")
            .field("kind", &self.kind)
            .field("start", &self.start)
            .field("query", &self.query)
            .field("index", &self.index)
            .finish()
    }
}

/// One row of the `@` / `#` menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionRow {
    /// What replaces the token — `@path ` or a conversation link.
    pub insert: String,
    pub label: String,
    pub detail: String,
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
    /// Apply the complete editor state, retaining undo/history/paste/attachment
    /// stacks that a text-only snapshot cannot represent.
    EditEditor(Box<editor::EditorState>),
    CopyLast,
    ToggleMouse,
    OpenQueue,
    OpenApproval,
    OpenDiff,
    OpenHistory,
    OpenTree,
    OpenFork,
    OpenWorktree,
    ApprovalDecision {
        approved: bool,
        rule: bool,
    },
    SelectDiff(usize),
    SelectWorktree(String),
    ForkAt(String),
    Compact(String),
    SetPermission(String),
    CycleSandbox,
    SetTools(Vec<String>),
    CancelQueued(String),
    RecallQueued(String),
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
    /// The model picker chose a model: apply it to the agent.
    SetModel(String),
    /// The reasoning slider moved: apply the level to the agent.
    SetThinking(String),
    /// The model picker chose a model while no session was attached: stage it
    /// as the default the next session starts with. There is nothing to set it
    /// on, so this must not reach the agent — the agent answers
    /// `set_model` with "session not found" when no session exists.
    SetPendingModel(String),
    /// The reasoning slider moved with no session attached, staged like
    /// [`WbAction::SetPendingModel`].
    SetPendingThinking(String),
    /// The theme picker committed a palette: adopt it and persist it.
    SetTheme(String),
    /// The language picker committed a preference: adopt it and persist it. The
    /// driver owns the resolution (a `system` choice re-reads the locale) and
    /// the settings file; the surface only knows the two bundles.
    SetLanguage(LanguagePreference),
    /// A layout preference changed; the driver persists the whole surface state.
    SetPreferences,
    /// Persist the effective shortcut map after a rebind or reset.
    SetKeybindings,
    /// Explain an action that could not be applied in the notice row.
    Notice(String),
    /// `/new`: back to the launchpad, with no session attached (the first
    /// message creates one).
    NewSession,
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

/// What became of one `/…` verb.
///
/// The split matters for the palette: an entry that names something on screen
/// is the surface's to carry out, and an entry that needs the agent goes back
/// to the app. A verb that is neither is a defect — [`COMMANDS`] and the two
/// handlers behind this enum are kept in step by `every_palette_entry_has_a_handler`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerbOutcome {
    /// The surface carried it out. `notice` is what belongs on the notice row,
    /// where `None` clears whatever was there.
    Done { notice: Option<String> },
    /// Not the surface's: the app runs it against the agent.
    Agent,
}

/// One input event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WbEvent {
    Focus(bool),
    /// A normalised key id (`keys::parse_key`).
    Key(String),
    Mouse(mouse::MouseEvent),
    Resize(u16, u16),
    /// A bracketed paste.
    Paste(String),
    /// One animation tick (spinner, elapsed time).
    Tick,
}

/// A divider drag in progress, in frame cells.
///
/// Both fields are captured at the press and never change during the drag: the
/// rail width is always *the width at the press* plus the pointer's own
/// displacement, so a long drag is one subtraction away from the press instead
/// of accumulating rounding per motion report.
#[derive(Debug, Clone, Copy)]
struct Drag {
    /// The column the pointer went down on (the press cell).
    grab_x: u16,
    /// The rail's width when it went down.
    grab_width: u16,
}

/// How many animation ticks the spinner holds one frame.
///
/// The ticker fires every 120 ms (see `run`), so a frame lasts 480 ms — slow
/// enough that the spinner reads as motion rather than flicker, and coarse
/// enough that a run whose only visible change is the spinner needs two frames
/// a second instead of eight.
pub const SPINNER_TICKS: usize = 4;

/// The surface's own state: where the panels and the pointers are.
///
/// Deliberately *not* the data — the conversation, the todos and the usage
/// figures live in [`WorkbenchData`], which the app owns and passes in. Keeping
/// them apart is what lets the surface be tested with fixtures and later fed by
/// a real session without either side knowing the other's shape.
pub struct Workbench {
    pub palette: Palette,
    /// The language the chrome is drawn in. A stored preference resolves to one
    /// of the two bundles before it lands here, so every renderer can read it
    /// without asking the environment again.
    pub lang: Language,
    /// What the reader picked (`system` until they pick), which is the row the
    /// language picker marks as current.
    pub language_pref: LanguagePreference,
    pub panel: PanelId,
    pub sidebar_focus: bool,
    /// Is the sidebar allowed to show? `ctrl+b` toggles it; the width still
    /// decides whether it *can*.
    pub sidebar_visible: bool,
    pub sidebar_w: u16,
    pub density: preferences::Density,
    pub sidebar_position: preferences::SidebarPosition,
    pub reduced_motion: bool,
    pub linear_reading: bool,
    pub notify: crate::notifications::NotifyConfig,
    pub keys: preferences::Keybindings,
    pub preferences_index: usize,
    pub keybinding_index: usize,
    pub awaiting_keybinding: bool,
    pub panel_scroll: [usize; 5],
    pub overlay: Option<Overlay>,
    pub overlay_scroll: usize,
    pub palette_query: String,
    pub palette_index: usize,
    /// The open picker's query, highlight and slider (`/model`, `/theme`,
    /// `/sessions`). Kept even while it is closed so reopening resumes where the
    /// reader left.
    pub picker: Picker,
    /// The palette in force when the theme picker opened: `esc` restores it, so
    /// previewing cannot change what the reader ends up with.
    picker_restore: Option<&'static str>,
    pub launch_field: LaunchField,
    pub chat_scroll: usize,
    pub scroll: scroll::ScrollState,
    pub queue_index: usize,
    pub workflow_index: usize,
    /// Diff navigator focus: files or the patch. The narrow layout uses the
    /// same distinction so file selection cannot eat the patch's scroll keys.
    pub diff_patch_focus: bool,
    pub history_query: String,
    session_key: String,
    pub mouse_capture: bool,
    pub(crate) composer_width: usize,
    pub tick: usize,
    /// The `@` / `#` completion menu, while the draft's current token carries a
    /// sigil. Derived from the draft on every event, never owned by the menu
    /// (see [`Self::sync_mention`]).
    pub mention: Option<Mention>,
    /// The token an `esc` dismissed, so the menu does not reopen on the same
    /// keystroke that closed it. Cleared as soon as the token changes.
    mention_dismissed: Option<(usize, String)>,
    /// The frame size of the last render; mouse coordinates are resolved
    /// against it.
    last_frame: (u16, u16),
    hit: layout::HitMap,
    /// The hit map's length when the overlay started painting — see
    /// [`Self::overlay_mark`].
    overlay_mark: usize,
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
            // English, on an unset preference: the driver resolves the stored
            // preference (and with it the locale) before the first frame. A
            // constructor that read the environment would make every test's
            // text depend on the machine running it.
            lang: Language::default(),
            language_pref: LanguagePreference::default(),
            panel: PanelId::Todo,
            sidebar_focus: false,
            sidebar_visible: true,
            sidebar_w: layout::SIDEBAR_DEFAULT,
            density: preferences::Density::Focused,
            sidebar_position: preferences::SidebarPosition::Right,
            reduced_motion: false,
            linear_reading: false,
            notify: crate::notifications::NotifyConfig::default(),
            keys: preferences::Keybindings::default(),
            preferences_index: 0,
            keybinding_index: 0,
            awaiting_keybinding: false,
            panel_scroll: [0; 5],
            overlay: None,
            overlay_scroll: 0,
            palette_query: String::new(),
            palette_index: 0,
            picker: Picker::new(
                PickerKind::Model,
                &WorkbenchData::default(),
                theme::FUTURE_DARK.id,
                Language::default(),
                LanguagePreference::default(),
            ),
            picker_restore: None,
            launch_field: LaunchField::Prompt,
            chat_scroll: 0,
            scroll: scroll::ScrollState::default(),
            queue_index: 0,
            workflow_index: 0,
            diff_patch_focus: true,
            history_query: String::new(),
            session_key: String::new(),
            mouse_capture: true,
            composer_width: 76,
            tick: 0,
            mention: None,
            mention_dismissed: None,
            last_frame: (0, 0),
            hit: layout::HitMap::new(),
            overlay_mark: 0,
            cursor: None,
            drag: None,
        }
    }

    /// Restore persisted surface choices before the first frame.
    pub fn set_preferences(&mut self, preferences: preferences::LayoutPreferences) {
        self.density = preferences.density;
        self.sidebar_position = preferences.sidebar_position;
        self.sidebar_w = preferences
            .sidebar_width
            .clamp(layout::SIDEBAR_MIN, layout::SIDEBAR_MAX);
        self.sidebar_visible = preferences.sidebar_visible;
        self.mouse_capture = preferences.mouse_capture;
        self.notify = preferences.notify;
        self.reduced_motion = preferences.reduced_motion;
        self.linear_reading = preferences.linear_reading;
    }

    pub fn preferences(&self) -> preferences::LayoutPreferences {
        preferences::LayoutPreferences {
            density: self.density,
            sidebar_position: self.sidebar_position,
            sidebar_width: self.sidebar_w,
            sidebar_visible: self.sidebar_visible,
            mouse_capture: self.mouse_capture,
            notify: self.notify,
            reduced_motion: self.reduced_motion,
            linear_reading: self.linear_reading,
        }
    }

    pub fn open_preferences(&mut self) {
        self.overlay = Some(Overlay::Preferences);
        self.overlay_scroll = 0;
    }

    /// `/theme` — adopt the next built-in palette and return its id.
    pub fn cycle_theme(&mut self) -> &'static str {
        self.palette = theme::next(&self.palette);
        self.palette.id
    }
    /// Adopt a palette by id; an unknown id is the default (and is reported).
    pub fn set_theme(&mut self, id: &str) -> bool {
        self.palette = theme::resolve(Some(id));
        // `is_known` accepts the catalogue's spelling *and* the ids older
        // workbench builds wrote (`future-dark`), the same way `resolve` does:
        // reporting "unknown" for a palette that was in fact applied is the
        // kind of lie that made the settings file drift.
        theme::is_known(id)
    }

    /// Adopt a language preference: `system` resolves against the locale now.
    /// Repainting is the caller's business — this changes the text of the next
    /// frame, not the frame that is up.
    pub fn set_language(&mut self, preference: LanguagePreference) {
        self.language_pref = preference;
        self.lang = preference.resolve();
    }

    /// Render one frame: exactly `h` rows, each exactly `w` display columns.
    pub fn render(&mut self, data: &WorkbenchData, w: u16, h: u16) -> Vec<String> {
        if self.session_key != data.session.id {
            self.session_key = data.session.id.clone();
            self.scroll.reset();
        }
        self.scroll.observe(data.revision);
        self.mouse_capture = data.mouse_capture;
        // The menu is a reading of the draft, and a frame is the moment the
        // draft is as fresh as it will get in this iteration: a paste inserts a
        // whole token with no keystroke after it, so painting is what opens the
        // menu for it.
        self.sync_mention(data);
        let rows = screen::compose(self, data, w, h);
        debug_assert_eq!(
            rows.len(),
            h as usize,
            "the frame must be h rows" // i18n: an invariant's panic, never drawn
        );
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

    /// Whether a divider grab is in flight. The driver needs it to know a drag
    /// is its own rather than a new press, and a pointer test needs it to tell
    /// "the press started a drag" from "the press did nothing".
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// `/` palette entries that match the current query: `(command, description)`.
    pub fn palette_entries(&self) -> Vec<(&'static str, &'static str)> {
        let needle = self.palette_query.to_lowercase();
        COMMANDS
            .iter()
            .copied()
            .map(|(command, key)| (command, t(self.lang, key)))
            .filter(|(command, description)| {
                needle.is_empty()
                    || command.to_lowercase().contains(&needle)
                    || description.to_lowercase().contains(&needle)
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

    /// One animation step: advance the clock the spinner and the elapsed-time
    /// counter read. Returns the tick it landed on.
    ///
    /// The driver's ticker calls this directly — it must not compose a frame
    /// just to advance a counter — while [`WbEvent::Tick`] is the surface's
    /// route to the same step.
    pub fn advance_tick(&mut self) -> usize {
        self.tick = self.tick.wrapping_add(1);
        self.tick
    }

    /// The spinner frame the activity row draws right now: the tick divided by
    /// [`SPINNER_TICKS`], so four ticks make one drawn frame.
    pub fn spinner_frame(&self) -> usize {
        self.tick / SPINNER_TICKS
    }

    /// Linear reading is deliberately static too: no animated activity glyph
    /// and no auxiliary rail, while streamed text still arrives immediately.
    pub fn motion_enabled(&self) -> bool {
        !self.reduced_motion && !self.linear_reading
    }

    pub fn handle(&mut self, event: WbEvent, data: &WorkbenchData) -> Vec<WbAction> {
        // The `@` / `#` menu is a reading of the draft, so it is re-derived
        // before the event is interpreted: a keystroke that just became the
        // sigil opens it, and one that made the token stop looking like a
        // mention closes it.
        self.sync_mention(data);
        match event {
            WbEvent::Focus(_) => Vec::new(),
            WbEvent::Tick => {
                self.advance_tick();
                Vec::new()
            }
            WbEvent::Resize(_, _) => vec![WbAction::Redraw],
            WbEvent::Paste(text) => {
                let mut view = data.input.editor.clone();
                view.paste(&text);
                vec![WbAction::EditEditor(Box::new(view))]
            }
            WbEvent::Mouse(event) if data.mouse_capture => self.handle_mouse(event, data),
            WbEvent::Mouse(_) => Vec::new(),
            WbEvent::Key(key) => self.handle_key(&key, data),
        }
    }

    /// Rebuild [`Self::mention`] from the draft's current token.
    ///
    /// The token is the run of non-space characters ending at the caret; it
    /// becomes a mention when it *starts* with `@` (files) or `#`
    /// (conversations). A token that merely contains the sigil (`me@example.com`,
    /// `x#y`) is not one, which is what keeps an e-mail address in a prompt from
    /// opening a menu.
    fn sync_mention(&mut self, data: &WorkbenchData) {
        if self.overlay.is_some() {
            self.mention = None;
            return;
        }
        let Some((kind, start, query)) = mention_token(&data.input) else {
            self.mention = None;
            return;
        };
        if self
            .mention_dismissed
            .as_ref()
            .is_some_and(|(at, text)| *at == start && *text == query)
        {
            self.mention = None;
            return;
        }
        match &mut self.mention {
            Some(current) if current.kind == kind && current.start == start => {
                if current.query != query {
                    current.query = query;
                    // A narrower query is a different list, so the highlight
                    // starts over at its top: keeping the old ordinal would
                    // silently move the selection to an unrelated row.
                    current.index = 0;
                }
            }
            _ => {
                self.mention = Some(Mention {
                    kind,
                    start,
                    query,
                    index: 0,
                });
            }
        }
    }

    /// The rows the open mention menu offers, filtered by what was typed after
    /// the sigil.
    pub fn mention_rows(&self, data: &WorkbenchData) -> Vec<MentionRow> {
        let Some(mention) = &self.mention else {
            return Vec::new();
        };
        let needle = mention.query.to_lowercase();
        let matches = |values: &[&str]| {
            needle.is_empty()
                || values
                    .iter()
                    .any(|value| value.to_lowercase().contains(&needle))
        };
        match mention.kind {
            MentionKind::File => data
                .files
                .iter()
                .filter(|path| matches(&[path.as_str()]))
                // A bounded window: the menu is a picker, not a file browser.
                .take(60)
                .map(|path| MentionRow {
                    insert: format!("@{path} "),
                    label: path.clone(),
                    detail: String::new(),
                })
                .collect(),
            MentionKind::Session => data
                .sessions
                .iter()
                .filter(|session| {
                    matches(&[
                        session.title.as_str(),
                        session.id.as_str(),
                        session.workspace.as_str(),
                    ])
                })
                .take(60)
                .map(|session| MentionRow {
                    insert: session_reference(&session.title, &session.id),
                    label: if session.title.is_empty() {
                        session.id.clone()
                    } else {
                        session.title.clone()
                    },
                    detail: session.workspace.clone(),
                })
                .collect(),
        }
    }

    /// Replace the mention token with `insert`, leaving the caret after it.
    fn insert_mention(&mut self, data: &WorkbenchData, insert: &str) -> Vec<WbAction> {
        let Some(mention) = self.mention.take() else {
            return Vec::new();
        };
        let text = data.input.text.as_str();
        let at = floor_char_boundary(text, mention.start.min(text.len()));
        let end = floor_char_boundary(text, data.input.cursor.min(text.len()));
        let end = end.max(at);
        let mut filled = String::with_capacity(text.len() + insert.len());
        filled.push_str(&text[..at]);
        filled.push_str(insert);
        filled.push_str(&text[end..]);
        let cursor = at + insert.len();
        self.mention_dismissed = Some((at, insert.to_string()));
        vec![WbAction::EditInput {
            text: filled,
            cursor,
        }]
    }

    fn handle_key(&mut self, key: &str, data: &WorkbenchData) -> Vec<WbAction> {
        // The mention menu answers the navigation keys while it is up, and
        // nothing else: every other key still edits the draft, which is what
        // keeps the query and the menu from disagreeing.
        if self.mention.is_some() {
            match key {
                "escape" => {
                    if let Some(mention) = self.mention.take() {
                        self.mention_dismissed = Some((mention.start, mention.query));
                    }
                    return vec![WbAction::Redraw];
                }
                "up" | "down" => {
                    let rows = self.mention_rows(data).len();
                    let index = self.mention.as_ref().map(|m| m.index).unwrap_or(0);
                    let next = if key == "up" {
                        index.saturating_sub(1)
                    } else {
                        (index + 1).min(rows.saturating_sub(1))
                    };
                    if let Some(mention) = self.mention.as_mut() {
                        mention.index = next;
                    }
                    return vec![WbAction::Redraw];
                }
                "enter" | "tab" => {
                    let rows = self.mention_rows(data);
                    let index = self.mention.as_ref().map(|m| m.index).unwrap_or(0);
                    let Some(row) = rows.get(index.min(rows.len().saturating_sub(1))) else {
                        // Nothing matches: `enter` must not eat the submission,
                        // and `tab` must not insert a token that names nothing.
                        if key == "tab" {
                            return Vec::new();
                        }
                        self.mention = None;
                        return self.submit_draft(data);
                    };
                    let insert = row.insert.clone();
                    return self.insert_mention(data, &insert);
                }
                _ => {}
            }
        }
        if let Some(overlay) = self.overlay {
            return self.handle_overlay_key(overlay, key, data);
        }

        // Action ids, not literal chords, drive global shortcuts. A rebind in
        // `/preferences` therefore changes dispatch, help and hints together.
        match self.keys.action(key) {
            Some(preferences::Action::Quit) => return vec![WbAction::Quit],
            Some(preferences::Action::Palette) => {
                self.open_palette();
                return vec![WbAction::Redraw];
            }
            Some(preferences::Action::Help) => {
                self.overlay = Some(Overlay::Help);
                self.overlay_scroll = 0;
                return vec![WbAction::Redraw];
            }
            Some(preferences::Action::Sessions) => {
                return self.open_picker(PickerKind::Sessions, data)
            }
            Some(preferences::Action::Preferences) => {
                self.open_preferences();
                return vec![WbAction::Redraw];
            }
            Some(preferences::Action::Sidebar) => {
                return self.toggle_sidebar();
            }
            Some(preferences::Action::Copy) => return vec![WbAction::CopyLast],
            Some(preferences::Action::Queue) => return vec![WbAction::OpenQueue],
            Some(preferences::Action::Mouse) => return vec![WbAction::ToggleMouse],
            Some(preferences::Action::LinearReading) => {
                self.linear_reading = !self.linear_reading;
                self.sidebar_focus = false;
                return vec![WbAction::SetPreferences];
            }
            Some(preferences::Action::ReduceMotion) => {
                self.reduced_motion = !self.reduced_motion;
                return vec![WbAction::SetPreferences];
            }
            _ => {}
        }
        match key {
            "ctrl+q" => vec![WbAction::Quit],
            "f6" => vec![WbAction::CopyLast],
            "f7" => vec![WbAction::ToggleMouse],
            "f8" => {
                self.linear_reading = !self.linear_reading;
                self.sidebar_focus = false;
                vec![WbAction::SetPreferences]
            }
            "f9" => {
                self.reduced_motion = !self.reduced_motion;
                vec![WbAction::SetPreferences]
            }
            "ctrl+g" => vec![WbAction::OpenQueue],
            "?" if data.input.text.is_empty() => {
                self.overlay = Some(Overlay::Help);
                self.overlay_scroll = 0;
                vec![WbAction::Redraw]
            }
            "/" if data.input.text.is_empty() => {
                self.open_palette();
                vec![WbAction::Redraw]
            }
            "tab" => {
                if data.on_launchpad() {
                    self.launch_field = self.launch_field.next();
                } else if self.sidebar_visible
                    && !self.linear_reading
                    && layout::sidebar_fits(self.last_frame.0)
                {
                    // A hidden rail cannot take the keys: focusing it would
                    // swallow up/down with nothing on screen to show for it.
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
            "enter" => self.submit_draft(data),
            "up" if self.sidebar_focus => self.scroll(-1, data),
            "down" if self.sidebar_focus => self.scroll(1, data),
            "ctrl+up" | "pageUp" => self.scroll(-10, data),
            "ctrl+down" | "pageDown" => self.scroll(10, data),
            "end" if self.scroll.following() && data.input.text.is_empty() => self.scroll(1, data),
            "escape" if !self.scroll.following() => {
                self.scroll.latest();
                self.chat_scroll = 0;
                vec![WbAction::Redraw]
            }
            // The shared editor owns every editing chord, including multiline
            // movement/history and undo/redo. Cloning snapshots, not rebuilding
            // an InputView, is what preserves undo and attachment state.
            "backspace" | "delete" | "home" | "end" | "left" | "right" | "up" | "down"
            | "ctrl+a" | "ctrl+e" | "ctrl+f" | "ctrl+u" | "ctrl+k" | "ctrl+w" | "ctrl+y"
            | "ctrl+z" | "ctrl+-" | "ctrl+/" | "ctrl+_" | "ctrl+shift+z" | "alt+y"
            | "alt+backspace" | "alt+d" | "alt+delete" | "ctrl+left" | "ctrl+right"
            | "alt+left" | "alt+right" | "shift+enter" | "alt+enter" | "ctrl+enter" | "ctrl+j" => {
                let mut view = data.input.editor.clone();
                view.resize(self.composer_width);
                view.key(key);
                vec![WbAction::EditEditor(Box::new(view))]
            }
            // Space is a character in the draft, not the "select" key the
            // menus use: the prompt has to be able to contain words.
            "space" => {
                let mut view = data.input.editor.clone();
                view.paste(" ");
                vec![WbAction::EditEditor(Box::new(view))]
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
                    let mut view = data.input.editor.clone();
                    view.paste(other);
                    vec![WbAction::EditEditor(Box::new(view))]
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn toggle_sidebar(&mut self) -> Vec<WbAction> {
        if self.linear_reading {
            self.linear_reading = false;
            self.sidebar_visible = true;
            self.sidebar_focus = true;
        } else if !layout::sidebar_fits(self.last_frame.0) {
            self.overlay = Some(Overlay::PanelFull);
        } else if !self.sidebar_visible {
            self.sidebar_visible = true;
            self.sidebar_focus = true;
        } else if !self.sidebar_focus {
            self.sidebar_focus = true;
        } else {
            self.sidebar_visible = false;
            self.sidebar_focus = false;
        }
        if layout::sidebar_fits(self.last_frame.0) {
            vec![WbAction::SetPreferences]
        } else {
            vec![WbAction::Redraw]
        }
    }

    fn open_palette(&mut self) {
        self.overlay = Some(Overlay::Palette);
        self.palette_query.clear();
        self.palette_index = 0;
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
        self.scroll.scroll(up, ceiling);
        self.chat_scroll = self.scroll.offset;
        if up < 0 && at_top {
            // Nothing older is loaded: the app may fetch one more page.
            return vec![WbAction::LoadOlder];
        }
        vec![WbAction::Scroll(up)]
    }

    fn handle_overlay_key(
        &mut self,
        overlay: Overlay,
        key: &str,
        data: &WorkbenchData,
    ) -> Vec<WbAction> {
        if matches!(
            overlay,
            Overlay::Approval
                | Overlay::Diff
                | Overlay::History
                | Overlay::Tree
                | Overlay::Fork
                | Overlay::Worktree
        ) {
            return self.handle_workflow_key(overlay, key, data);
        }
        match overlay {
            Overlay::Queue => match key {
                "escape" | "q" => {
                    self.overlay = None;
                    vec![WbAction::Redraw]
                }
                "up" => {
                    self.queue_index = self.queue_index.saturating_sub(1);
                    vec![WbAction::Redraw]
                }
                "down" => {
                    self.queue_index =
                        (self.queue_index + 1).min(data.queue.len().saturating_sub(1));
                    vec![WbAction::Redraw]
                }
                "x" | "d" | "delete" => data
                    .queue
                    .get(self.queue_index)
                    .map(|row| vec![WbAction::CancelQueued(row.run_id.clone())])
                    .unwrap_or_default(),
                "r" | "enter" => {
                    self.overlay = None;
                    data.queue
                        .get(self.queue_index)
                        .map(|row| vec![WbAction::RecallQueued(row.run_id.clone())])
                        .unwrap_or_default()
                }
                _ => Vec::new(),
            },
            Overlay::Preferences => match key {
                "escape" | "q" => {
                    self.overlay = None;
                    vec![WbAction::Redraw]
                }
                "up" | "k" => {
                    self.preferences_index = self.preferences_index.saturating_sub(1);
                    vec![WbAction::Redraw]
                }
                "down" | "j" => {
                    self.preferences_index =
                        (self.preferences_index + 1).min(preferences::PREFERENCE_ROWS.len() - 1);
                    vec![WbAction::Redraw]
                }
                "left" | "right" | "enter" => {
                    let row = preferences::PREFERENCE_ROWS[self.preferences_index];
                    let direction = if key == "left" { -1 } else { 1 };
                    match row {
                        preferences::PreferenceRow::Density => self.density = self.density.next(),
                        preferences::PreferenceRow::Position => {
                            self.sidebar_position = self.sidebar_position.next()
                        }
                        preferences::PreferenceRow::Width => {
                            self.sidebar_w = (self.sidebar_w as i32 + direction * 2)
                                .clamp(layout::SIDEBAR_MIN as i32, layout::SIDEBAR_MAX as i32)
                                as u16;
                        }
                        preferences::PreferenceRow::Mouse => {
                            return vec![WbAction::ToggleMouse];
                        }
                        preferences::PreferenceRow::Notifications => {
                            self.notify.enabled = !self.notify.enabled;
                        }
                        preferences::PreferenceRow::NotificationBell => {
                            self.notify.bell = !self.notify.bell;
                        }
                        preferences::PreferenceRow::Motion => {
                            self.reduced_motion = !self.reduced_motion;
                        }
                        preferences::PreferenceRow::Linear => {
                            self.linear_reading = !self.linear_reading;
                            self.sidebar_focus = false;
                        }
                        preferences::PreferenceRow::Shortcuts => {
                            if key != "enter" {
                                return Vec::new();
                            }
                            self.overlay = Some(Overlay::Keybindings);
                            self.awaiting_keybinding = false;
                            return vec![WbAction::Redraw];
                        }
                        preferences::PreferenceRow::Reset => {
                            if key != "enter" {
                                return Vec::new();
                            }
                            self.set_preferences(preferences::LayoutPreferences::default());
                        }
                    }
                    vec![WbAction::SetPreferences]
                }
                _ => Vec::new(),
            },
            Overlay::Keybindings => {
                if self.awaiting_keybinding {
                    if key == "escape" {
                        self.awaiting_keybinding = false;
                        return vec![WbAction::Redraw];
                    }
                    let action = preferences::ACTIONS[self.keybinding_index];
                    if self.keys.bind(action, key).is_err() {
                        self.awaiting_keybinding = false;
                        return vec![WbAction::Notice(
                            t(self.lang, Key::NoticePreferenceInvalid).into(),
                        )];
                    }
                    self.awaiting_keybinding = false;
                    return vec![WbAction::SetKeybindings];
                }
                match key {
                    "escape" | "q" => {
                        self.overlay = Some(Overlay::Preferences);
                        vec![WbAction::Redraw]
                    }
                    "up" | "k" => {
                        self.keybinding_index = self.keybinding_index.saturating_sub(1);
                        vec![WbAction::Redraw]
                    }
                    "down" | "j" => {
                        self.keybinding_index =
                            (self.keybinding_index + 1).min(preferences::ACTIONS.len() - 1);
                        vec![WbAction::Redraw]
                    }
                    "enter" => {
                        self.awaiting_keybinding = true;
                        vec![WbAction::Redraw]
                    }
                    "r" => {
                        self.keys.reset();
                        vec![WbAction::SetKeybindings]
                    }
                    _ => Vec::new(),
                }
            }
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
                "escape" | "q" => {
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
            Overlay::Approval
            | Overlay::Diff
            | Overlay::History
            | Overlay::Tree
            | Overlay::Fork
            | Overlay::Worktree => Vec::new(),
            Overlay::Picker(kind) => match key {
                "escape" => self.dismiss_picker(kind),
                "enter" => self.apply_picker(kind, data),
                // Only the arrows move the highlight: `j` and `k` are characters
                // in the incremental search here, unlike the help overlay.
                "up" => {
                    self.picker.move_by(-1, data);
                    self.preview_theme(data);
                    vec![WbAction::Redraw]
                }
                "down" => {
                    self.picker.move_by(1, data);
                    self.preview_theme(data);
                    vec![WbAction::Redraw]
                }
                "pageUp" => {
                    self.picker.page(-1, data);
                    self.preview_theme(data);
                    vec![WbAction::Redraw]
                }
                "pageDown" => {
                    self.picker.page(1, data);
                    self.preview_theme(data);
                    vec![WbAction::Redraw]
                }
                "left" | "right" if kind == PickerKind::Model => {
                    self.picker.slide(if key == "left" { -1 } else { 1 });
                    vec![WbAction::Redraw]
                }
                "backspace" => {
                    self.picker.backspace();
                    vec![WbAction::Redraw]
                }
                other if other.chars().count() == 1 && !other.starts_with("ctrl+") => {
                    for ch in other.chars() {
                        self.picker.push(ch);
                    }
                    self.preview_theme(data);
                    vec![WbAction::Redraw]
                }
                _ => Vec::new(),
            },
        }
    }

    fn handle_workflow_key(
        &mut self,
        overlay: Overlay,
        key: &str,
        data: &WorkbenchData,
    ) -> Vec<WbAction> {
        if matches!(key, "escape" | "q") && !(overlay == Overlay::History && key == "q") {
            self.overlay = None;
            return vec![WbAction::Redraw];
        }
        let count = match overlay {
            Overlay::History => {
                workflows::filtered_sessions(&data.sessions, &self.history_query).len()
            }
            Overlay::Tree => data.sessions.len(),
            Overlay::Fork => data.fork_points.len(),
            Overlay::Worktree => data.worktrees.len(),
            Overlay::Diff => data.diff.as_ref().map_or(0, |diff| diff.files.len()),
            Overlay::Approval => 0,
            _ => 0,
        };
        let detail = matches!(overlay, Overlay::Approval | Overlay::Diff);
        match key {
            "tab" if overlay == Overlay::Diff => {
                self.diff_patch_focus = !self.diff_patch_focus;
            }
            "up" | "k" if overlay == Overlay::Diff && !self.diff_patch_focus => {
                self.workflow_index = self.workflow_index.saturating_sub(1);
            }
            "down" | "j" if overlay == Overlay::Diff && !self.diff_patch_focus => {
                self.workflow_index = (self.workflow_index + 1).min(count.saturating_sub(1));
            }
            "up" | "k"
                if (key == "up" || overlay != Overlay::History)
                    && (overlay != Overlay::Diff || self.diff_patch_focus) =>
            {
                if detail {
                    self.overlay_scroll = self.overlay_scroll.saturating_sub(1);
                } else {
                    self.workflow_index = self.workflow_index.saturating_sub(1);
                }
            }
            "down" | "j"
                if (key == "down" || overlay != Overlay::History)
                    && (overlay != Overlay::Diff || self.diff_patch_focus) =>
            {
                if detail {
                    self.overlay_scroll += 1;
                } else {
                    self.workflow_index = (self.workflow_index + 1).min(count.saturating_sub(1));
                }
            }
            "left" if overlay == Overlay::Diff => {
                self.diff_patch_focus = false;
                self.workflow_index = self.workflow_index.saturating_sub(1);
            }
            "right" if overlay == Overlay::Diff => {
                self.diff_patch_focus = false;
                self.workflow_index = (self.workflow_index + 1).min(count.saturating_sub(1));
            }
            "pageUp" => self.overlay_scroll = self.overlay_scroll.saturating_sub(10),
            "pageDown" | "space" if overlay != Overlay::History => self.overlay_scroll += 10,
            "backspace" if overlay == Overlay::History => {
                self.history_query.pop();
                self.workflow_index = 0;
                self.overlay_scroll = 0;
            }
            "y" | "Y" if overlay == Overlay::Approval => {
                self.overlay = None;
                return vec![WbAction::ApprovalDecision {
                    approved: true,
                    rule: false,
                }];
            }
            "n" | "N" if overlay == Overlay::Approval => {
                self.overlay = None;
                return vec![WbAction::ApprovalDecision {
                    approved: false,
                    rule: false,
                }];
            }
            "a" | "A" if overlay == Overlay::Approval => {
                if data
                    .approval
                    .as_ref()
                    .is_none_or(|approval| approval.rules.is_empty())
                {
                    return Vec::new();
                }
                self.overlay = None;
                return vec![WbAction::ApprovalDecision {
                    approved: true,
                    rule: true,
                }];
            }
            "r" if matches!(overlay, Overlay::Diff | Overlay::Worktree) => {
                return vec![WbAction::Command(match overlay {
                    Overlay::Diff => "/diff".into(),
                    _ => "/worktree".into(),
                })];
            }
            "enter" => {
                if overlay != Overlay::Diff {
                    self.overlay = None;
                }
                self.overlay_scroll = 0;
                return match overlay {
                    Overlay::Diff => data
                        .diff
                        .as_ref()
                        .and_then(|diff| diff.files.get(self.workflow_index))
                        .map(|_| vec![WbAction::SelectDiff(self.workflow_index)])
                        .unwrap_or_default(),
                    Overlay::History => {
                        workflows::filtered_sessions(&data.sessions, &self.history_query)
                            .get(self.workflow_index)
                            .map(|row| vec![WbAction::SwitchSession(row.id.clone())])
                            .unwrap_or_default()
                    }
                    Overlay::Tree => workflows::tree_sessions(&data.sessions)
                        .get(self.workflow_index)
                        .map(|(row, _)| vec![WbAction::SwitchSession(row.id.clone())])
                        .unwrap_or_default(),
                    Overlay::Fork => data
                        .fork_points
                        .get(self.workflow_index)
                        .map(|point| vec![WbAction::ForkAt(point.id.clone())])
                        .unwrap_or_default(),
                    Overlay::Worktree => data
                        .worktrees
                        .get(self.workflow_index)
                        .map(|row| vec![WbAction::SelectWorktree(row.path.clone())])
                        .unwrap_or_default(),
                    _ => vec![WbAction::Redraw],
                };
            }
            other
                if overlay == Overlay::History
                    && other.chars().count() == 1
                    && !other.starts_with("ctrl+") =>
            {
                self.history_query.push_str(other);
                self.workflow_index = 0;
                self.overlay_scroll = 0;
            }
            _ => {}
        }
        if overlay == Overlay::Diff
            && matches!(key, "up" | "down" | "k" | "j" | "left" | "right")
            && !self.diff_patch_focus
        {
            return data
                .diff
                .as_ref()
                .and_then(|diff| diff.files.get(self.workflow_index))
                .map(|_| vec![WbAction::SelectDiff(self.workflow_index)])
                .unwrap_or_default();
        }
        vec![WbAction::Redraw]
    }

    /// The palette the app considers *committed*.
    ///
    /// While the theme picker is open this is the palette it opened on, not the
    /// one being previewed: the `✓` marks what the reader has, and previewing a
    /// theme must not make the menu claim it is already theirs.
    pub fn committed_theme(&self) -> &'static str {
        self.picker_restore.unwrap_or(self.palette.id)
    }

    /// Open one of the pickers on the value in force.
    pub fn open_picker(&mut self, kind: PickerKind, data: &WorkbenchData) -> Vec<WbAction> {
        self.picker = Picker::new(kind, data, self.palette.id, self.lang, self.language_pref);
        self.picker_restore = (kind == PickerKind::Theme).then_some(self.palette.id);
        self.overlay = Some(Overlay::Picker(kind));
        self.overlay_scroll = 0;
        vec![WbAction::Redraw]
    }

    /// Close the picker. `esc` and a click outside it are the same gesture, and
    /// on the theme picker both undo the preview: what was on screen when the
    /// box opened is what the reader is left with.
    fn dismiss_picker(&mut self, kind: PickerKind) -> Vec<WbAction> {
        if kind == PickerKind::Theme {
            if let Some(id) = self.picker_restore.take() {
                self.set_theme(id);
            }
        }
        self.overlay = None;
        vec![WbAction::Redraw]
    }

    /// Moving the highlight in the theme picker shows the palette it named,
    /// without committing it.
    fn preview_theme(&mut self, data: &WorkbenchData) {
        if self.picker.kind != PickerKind::Theme {
            return;
        }
        if let Some(PickerRow::Item { key, .. }) =
            self.picker.selected(data, self.committed_theme())
        {
            self.set_theme(&key);
        }
    }

    /// `enter`, and a click on a row: whichever row is highlighted, apply it.
    ///
    /// On the launchpad there is no session to apply anything *to*: the agent
    /// refuses `set_model`/`set_thinking_level` without one ("session not
    /// found"), so the same two choices are returned as `SetPending*` and the
    /// app stages them for the session the next message creates. One gesture,
    /// two roads, decided by whether anything is attached.
    fn apply_picker(&mut self, kind: PickerKind, data: &WorkbenchData) -> Vec<WbAction> {
        let selected = self.picker.selected(data, self.committed_theme());
        let launchpad = data.on_launchpad();
        self.overlay = None;
        self.picker_restore = None;
        match (kind, selected) {
            // The slider *is* the choice on its own row: `enter` there applies
            // the level and leaves the model alone.
            (PickerKind::Model, Some(PickerRow::Slider { level, .. })) => {
                if launchpad {
                    vec![WbAction::SetPendingThinking(level)]
                } else {
                    vec![WbAction::SetThinking(level)]
                }
            }
            (PickerKind::Model, Some(PickerRow::Item { key, .. })) => {
                // The model and the level it runs at are one choice, so both
                // are carried on either road.
                let level = self.picker.level().to_string();
                if launchpad {
                    vec![
                        WbAction::SetPendingModel(key),
                        WbAction::SetPendingThinking(level),
                    ]
                } else {
                    vec![WbAction::SetModel(key), WbAction::SetThinking(level)]
                }
            }
            (PickerKind::Theme, Some(PickerRow::Item { key, .. })) => {
                self.set_theme(&key);
                vec![WbAction::SetTheme(key)]
            }
            (PickerKind::Sessions, Some(PickerRow::Item { key, .. })) => {
                vec![WbAction::SwitchSession(key)]
            }
            // The row's key is the wire spelling, which is what the driver
            // persists and what `LanguagePreference` reads back.
            (PickerKind::Language, Some(PickerRow::Item { key, .. })) => {
                vec![WbAction::SetLanguage(LanguagePreference::from_wire(&key))]
            }
            _ => vec![WbAction::Redraw],
        }
    }

    fn handle_mouse(&mut self, event: mouse::MouseEvent, data: &WorkbenchData) -> Vec<WbAction> {
        if !data.mouse_capture {
            return Vec::new();
        }
        // An open picker owns the pointer: its rows are the only live targets,
        // and a click anywhere else dismisses it rather than reaching the
        // conversation painted underneath.
        if let Some(Overlay::Picker(kind)) = self.overlay {
            return self.handle_picker_mouse(kind, event, data);
        }
        let (w, h) = self.last_frame;
        let frame = layout::layout_with_preferences(
            w,
            h,
            if self.sidebar_visible && !self.linear_reading {
                self.sidebar_w
            } else {
                0
            },
            self.effective_density(),
            self.sidebar_position,
        );
        if self.overlay.is_some() && !matches!(self.overlay, Some(Overlay::Picker(_))) {
            return self.handle_surface_mouse(event, data);
        }
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
                Some(id) => self.click(&id, event, data),
                None => Vec::new(),
            },
            mouse::MouseKind::Drag => {
                if let Some(drag) = self.drag {
                    // The rail hangs off the frame's **right** edge: making it
                    // wider moves its divider *left*. So the pointer's own
                    // movement is subtracted from the width, not added to it.
                    // Adding it — this branch's first version — mirrored every
                    // drag, measured at 120 columns: press on the divider at
                    // column 84, drag to column 90, and the divider ended up at
                    // column 78 with a 42-column rail, i.e. the edge ran away
                    // from the pointer it was supposed to follow.
                    let delta = (event.x as i32 - drag.grab_x as i32)
                        * if self.sidebar_position == preferences::SidebarPosition::Left {
                            -1
                        } else {
                            1
                        };
                    let width = (drag.grab_width as i32 - delta)
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

    /// The `enter` on a picker row, expressed with a pointer: the click moves
    /// the highlight to the row it landed on and then applies it, because a
    /// mouse has no second `enter`. The wheel walks the list.
    fn handle_surface_mouse(
        &mut self,
        event: mouse::MouseEvent,
        data: &WorkbenchData,
    ) -> Vec<WbAction> {
        let Some(overlay) = self.overlay else {
            return Vec::new();
        };
        if matches!(
            event.kind,
            mouse::MouseKind::WheelUp | mouse::MouseKind::WheelDown
        ) {
            if overlay == Overlay::Diff && self.diff_patch_focus {
                self.overlay_scroll = self.overlay_scroll.saturating_sub(3);
                return vec![WbAction::Redraw];
            }
            return self.handle_overlay_key(
                overlay,
                if event.kind == mouse::MouseKind::WheelUp {
                    "up"
                } else {
                    "down"
                },
                data,
            );
        }
        if event.kind != mouse::MouseKind::Press {
            return Vec::new();
        }
        // Resolve against what the *overlay* painted, not the page under it. A
        // modal that leaves the frame beneath in the hit map would let a click
        // on its own padding reach a sidebar tab and switch the panel behind it.
        let mark = self.overlay_mark;
        let Some(id) = self
            .hit
            .hit_after(event.x, event.y, mark)
            .map(str::to_string)
        else {
            // Nothing of the overlay is here: the reader clicked the dimmed page,
            // which is the plain way out of every overlay.
            self.overlay = None;
            return vec![WbAction::Redraw];
        };
        match id.as_str() {
            // The box's own padding, its title rule, its hints: part of the
            // overlay, so a click there is absorbed rather than dismissing it.
            "overlay.surface" => Vec::new(),
            "approval.y" => self.handle_overlay_key(overlay, "y", data),
            "approval.n" => self.handle_overlay_key(overlay, "n", data),
            "approval.a" => self.handle_overlay_key(overlay, "a", data),
            "diff.patch" => {
                self.diff_patch_focus = true;
                vec![WbAction::Redraw]
            }
            "preferences.shortcuts" => {
                self.overlay = Some(Overlay::Keybindings);
                vec![WbAction::Redraw]
            }
            "preferences.reset" => self.handle_overlay_key(Overlay::Preferences, "enter", data),
            _ => {
                if let Some(index) = id
                    .strip_prefix("queue.row.")
                    .and_then(|v| v.parse::<usize>().ok())
                {
                    // A queued prompt is a real verb (`recall`, the same one
                    // `enter` runs); under the overlay the page's own click
                    // handler is never reached, so it has to be repeated here.
                    self.queue_index = index;
                    return data
                        .queue
                        .get(index)
                        .map(|row| vec![WbAction::RecallQueued(row.run_id.clone())])
                        .unwrap_or_default();
                }
                if let Some(index) = id
                    .strip_prefix("palette.row.")
                    .and_then(|v| v.parse::<usize>().ok())
                {
                    // The palette is a menu: a click on an entry runs it, which
                    // is what `enter` does on the highlighted one.
                    self.palette_index = index;
                    let chosen = self
                        .palette_entries()
                        .get(index)
                        .map(|(command, _)| command.to_string());
                    self.overlay = None;
                    return chosen
                        .map(|command| vec![WbAction::Command(command)])
                        .unwrap_or_else(|| vec![WbAction::Redraw]);
                }
                if let Some(index) = id
                    .strip_prefix("diff.file.")
                    .and_then(|v| v.parse::<usize>().ok())
                {
                    self.workflow_index = index;
                    self.diff_patch_focus = false;
                    return data
                        .diff
                        .as_ref()
                        .map(|_| vec![WbAction::SelectDiff(index)])
                        .unwrap_or_default();
                }
                if let Some(index) = id
                    .strip_prefix("preferences.row.")
                    .and_then(|v| v.parse::<usize>().ok())
                {
                    self.preferences_index = index;
                    return vec![WbAction::Redraw];
                }
                if let Some(index) = id
                    .strip_prefix("keybinding.row.")
                    .and_then(|v| v.parse::<usize>().ok())
                {
                    self.keybinding_index = index;
                    self.awaiting_keybinding = true;
                    return vec![WbAction::Redraw];
                }
                if let Some(index) = id
                    .strip_prefix("workflow.row.")
                    .and_then(|v| v.parse::<usize>().ok())
                {
                    self.workflow_index = index;
                    return self.handle_overlay_key(overlay, "enter", data);
                }
                Vec::new()
            }
        }
    }

    fn handle_picker_mouse(
        &mut self,
        kind: PickerKind,
        event: mouse::MouseEvent,
        data: &WorkbenchData,
    ) -> Vec<WbAction> {
        match event.kind {
            mouse::MouseKind::WheelUp | mouse::MouseKind::WheelDown => {
                let step = if event.kind == mouse::MouseKind::WheelUp {
                    -1
                } else {
                    1
                };
                self.picker.move_by(step, data);
                self.preview_theme(data);
                vec![WbAction::Redraw]
            }
            mouse::MouseKind::Press => {
                let mark = self.overlay_mark;
                let id = self
                    .hit
                    .hit_after(event.x, event.y, mark)
                    .map(str::to_string);
                match id.as_deref() {
                    Some(id) if id.strip_prefix("picker.row.").is_some() => {
                        if let Some(ordinal) = id
                            .strip_prefix("picker.row.")
                            .and_then(|rest| rest.parse::<usize>().ok())
                        {
                            self.picker.index = ordinal;
                            self.preview_theme(data);
                        }
                        self.apply_picker(kind, data)
                    }
                    // The box itself (its search line, its hints, its padding) is
                    // not a choice and not a dismissal.
                    Some("picker.body") => Vec::new(),
                    _ => self.dismiss_picker(kind),
                }
            }
            _ => Vec::new(),
        }
    }

    fn click(&mut self, id: &str, event: mouse::MouseEvent, data: &WorkbenchData) -> Vec<WbAction> {
        // The mention menu owns its rows before anything underneath them: a
        // click on a candidate inserts it, and a click on the box's own frame is
        // not a dismissal (the reader aimed at the list).
        if self.mention.is_some() {
            if let Some(index) = id
                .strip_prefix("mention.row.")
                .and_then(|rest| rest.parse::<usize>().ok())
            {
                if let Some(mention) = self.mention.as_mut() {
                    mention.index = index;
                }
                let rows = self.mention_rows(data);
                if let Some(row) = rows.get(index.min(rows.len().saturating_sub(1))) {
                    let insert = row.insert.clone();
                    return self.insert_mention(data, &insert);
                }
            }
            if id == "mention.body" {
                return Vec::new();
            }
            // A click anywhere else closes the menu and then takes effect
            // normally: the reader aimed past it.
            self.mention = None;
        }
        if id == "chat.latest" {
            self.scroll.latest();
            self.chat_scroll = 0;
            return vec![WbAction::Redraw];
        }
        if let Some(index) = id
            .strip_prefix("queue.row.")
            .and_then(|v| v.parse::<usize>().ok())
        {
            self.queue_index = index;
            return data
                .queue
                .get(index)
                .map(|row| vec![WbAction::RecallQueued(row.run_id.clone())])
                .unwrap_or_default();
        }
        if id == "input" {
            self.launch_field = LaunchField::Prompt;
            // A click in the conversation half takes the keys back from the
            // sidebar: the caret in the tab strip is the only thing that says
            // which half answers `up`/`down`, so it has to follow the pointer.
            self.sidebar_focus = false;
            return vec![WbAction::FocusPane(PaneId::Input)];
        }
        if id == "chat" {
            self.sidebar_focus = false;
            return vec![WbAction::FocusPane(PaneId::Chat)];
        }
        if id == "divider" {
            // The drag is measured from the cell the pointer went down on, not
            // from the divider's own column, so a grab that lands anywhere in
            // the one-cell seam keeps its offset instead of snapping the rail
            // to the pointer. Drags only start *on* the divider, which is why
            // the frame width at the press is the width the drag is relative
            // to.
            self.drag = Some(Drag {
                grab_x: event.x,
                grab_width: self.sidebar_w,
            });
            return Vec::new();
        }
        if let Some(panel) = id.strip_prefix("sidebar.tab.").and_then(PanelId::parse) {
            self.panel = panel;
            // Clicking the sidebar means working in it; the same click that
            // switches the panel therefore takes the keys.
            self.sidebar_focus = true;
            return vec![WbAction::SwitchPanel(panel)];
        }
        if let Some(index) = id
            .strip_prefix("sidebar.row.")
            .and_then(|rest| rest.parse::<usize>().ok())
        {
            self.sidebar_focus = true;
            return match self.panel {
                PanelId::Sessions => data
                    .sessions
                    .get(index)
                    .map(|session: &state::SessionRow| {
                        vec![WbAction::SwitchSession(session.id.clone())]
                    })
                    .unwrap_or_default(),
                // A row of a read-only panel (a todo, a skill, a job) is not a
                // verb yet: it takes the keys and nothing else. It used to
                // return `panel.row.<n>` as a command, which the app had no
                // handler for and answered `Unknown command /panel.row.3` — the
                // same lie as the five dead palette entries, one click away.
                _ => vec![WbAction::Redraw],
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

    pub fn effective_density(&self) -> preferences::Density {
        if self.linear_reading {
            preferences::Density::Focused
        } else {
            self.density
        }
    }

    /// The frame size of the last render — mouse coordinates are resolved
    /// against it, so a click that arrives between resize and repaint lands
    /// somewhere defined instead of against a stale layout.
    pub(crate) fn set_last_frame(&mut self, w: u16, h: u16) {
        self.last_frame = (w, h);
    }

    /// The frame size the last render was composed for.
    ///
    /// The surface answers "is there room for a rail?" from this, so the app's
    /// first-frame refresh (see `run::event_loop`) is also what settles the
    /// width-dependent verbs: a `/usage` asked before any frame would otherwise
    /// decide on a 0-column terminal.
    pub fn frame_size(&self) -> (u16, u16) {
        self.last_frame
    }

    /// Show `panel` and give it the keys.
    ///
    /// Two surfaces, one intent: with room for a rail the panel is the rail's
    /// current tab (and the rail is shown even if the reader had hidden it —
    /// the verb names a panel, so showing it is the whole request); without
    /// room the panel takes the body, which is the same full-screen surface
    /// `ctrl+b` opens there. Focusing a panel a narrow terminal cannot paint
    /// would be a verb that answers with nothing.
    fn focus_panel(&mut self, panel: PanelId) {
        self.panel = panel;
        self.sidebar_focus = true;
        if layout::sidebar_fits(self.last_frame.0) {
            self.sidebar_visible = true;
        } else {
            self.overlay = Some(Overlay::PanelFull);
        }
    }

    /// Carry out a `/…` verb that belongs to the *surface* rather than to the
    /// agent: naming a panel, or saying where a workspace comes from.
    ///
    /// The palette advertises these ([`COMMANDS`]), so a verb it offers has to
    /// answer *something*. Five of them used to reach the agent, which has no
    /// such commands and said so: `/usage`, `/skills`, `/workspace`, `/export`
    /// and `/transcript` all printed `Unknown command`. The ones that are about
    /// what is on screen are answered here; `/export` needs the session and
    /// stays with the app (`VerbOutcome::Agent`).
    pub fn surface_command(&mut self, command: &str, data: &WorkbenchData) -> VerbOutcome {
        let name = command.trim_start_matches('/').trim();
        match name {
            "approval" | "diff" | "history" | "tree" | "fork" | "worktree" => VerbOutcome::Agent,
            "queue" => {
                self.overlay = Some(Overlay::Queue);
                self.queue_index = self.queue_index.min(data.queue.len().saturating_sub(1));
                VerbOutcome::Done { notice: None }
            }
            "usage" => {
                self.focus_panel(PanelId::Usage);
                VerbOutcome::Done { notice: None }
            }
            "skills" => {
                self.focus_panel(PanelId::Skills);
                VerbOutcome::Done { notice: None }
            }
            // There is no workspace picker to open, and pretending there is
            // would be the same lie as `Unknown command`. What is true: the
            // directory is the session's own, chosen where the session was
            // started, and the two verbs that move a reader between workspaces
            // are named.
            "workspace" => {
                let cwd = if data.session.cwd.is_empty() {
                    t(self.lang, Key::Unspecified).to_string()
                } else {
                    data.session.cwd.clone()
                };
                VerbOutcome::Done {
                    notice: Some(i18n::render(self.lang, Key::NoticeWorkspace, &[&cwd])),
                }
            }
            // `future tui` has no transcript dump of its own: the two real
            // doors to the whole record are named instead.
            "transcript" => VerbOutcome::Done {
                notice: Some(t(self.lang, Key::NoticeTranscript).into()),
            },
            // `/lang` never lands here: the driver opens the picker for it
            // (`PickerKind::from_command`), the same way `/model` and `/theme`
            // work, so the rows, the filter and the hit regions are one
            // implementation.
            // The same box `?` opens. It used to be a no-op arm in the app, so
            // the palette's own help entry resolved to nothing visible — a verb
            // that resolves is not the same as one that does something.
            "help" => {
                self.overlay = Some(Overlay::Help);
                self.overlay_scroll = 0;
                VerbOutcome::Done { notice: None }
            }
            "preferences" | "settings" => {
                self.open_preferences();
                VerbOutcome::Done { notice: None }
            }
            _ => VerbOutcome::Agent,
        }
    }

    // Fields the screen composer fills in while drawing.
    pub(crate) fn set_cursor(&mut self, cursor: Option<(u16, u16)>) {
        self.cursor = cursor;
    }

    pub(crate) fn hit_map(&mut self) -> &mut layout::HitMap {
        &mut self.hit
    }

    /// The first region this frame's overlay painted — set by the compositor so
    /// a pointer is resolved against the overlay and not the page under it.
    pub(crate) fn set_overlay_mark(&mut self, mark: usize) {
        self.overlay_mark = mark;
    }

    pub(crate) fn clear_hits(&mut self) {
        self.hit.clear();
    }

    pub(crate) fn set_chat_scroll(&mut self, scroll: usize) {
        self.chat_scroll = scroll;
        self.scroll.offset = scroll;
    }

    pub(crate) fn chat_scroll_offset(&self) -> usize {
        self.chat_scroll
    }

    pub fn following_latest(&self) -> bool {
        self.scroll.following()
    }
    pub fn has_new_output(&self) -> bool {
        self.scroll.unread()
    }

    pub(crate) fn set_overlay_scroll(&mut self, scroll: usize) {
        self.overlay_scroll = scroll;
    }

    pub(crate) fn overlay_scroll_offset(&self) -> usize {
        self.overlay_scroll
    }

    /// `enter` on the draft: a surface verb when the line names one, a prompt
    /// otherwise.
    fn submit_draft(&mut self, data: &WorkbenchData) -> Vec<WbAction> {
        let text = data.input.text.trim().to_string();
        if text.is_empty() {
            return Vec::new();
        }
        // A typed command is a command, never a prompt: `/model` in the draft
        // used to be sent to the agent as text, which is why no picker ever
        // opened from the input box. Only the surface's own verbs are claimed —
        // `/something else` (a skill, a path) still reaches the agent, and a
        // verb with arguments keeps them (`/model <id>`).
        if let Some(command) = known_command(&text) {
            let mut actions = vec![WbAction::EditInput {
                text: String::new(),
                cursor: 0,
            }];
            match (command.arg.is_empty())
                .then(|| PickerKind::from_command(&command.name))
                .flatten()
            {
                Some(kind) => actions.extend(self.open_picker(kind, data)),
                None => actions.push(WbAction::Command(command.raw)),
            }
            return actions;
        }
        vec![WbAction::Submit(text)]
    }
}

/// The token the caret is at the end of, when it is a mention: `(kind, byte
/// offset of the sigil, text after the sigil)`.
///
/// A mention starts at a **word boundary**: the sigil must be the first
/// character of the token, so `me@example.com` and a URL fragment are text, not
/// a request for a menu.
pub fn mention_token(input: &state::InputView) -> Option<(MentionKind, usize, String)> {
    let text = &input.text;
    let cursor = floor_char_boundary(text, input.cursor);
    let head = &text[..cursor];
    let start = head
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0);
    let token = &text[start..cursor];
    let kind = match token.chars().next()? {
        '@' => MentionKind::File,
        '#' => MentionKind::Session,
        _ => return None,
    };
    Some((kind, start, token[1..].to_string()))
}

/// How a picked conversation is written into the draft: the same
/// `futureos://session/<id>` link the desktop, the phone and this very
/// conversation use, so any client (and this TUI itself) can follow it back to
/// the conversation it names.
pub fn session_reference(title: &str, id: &str) -> String {
    // Brackets would close the label early and push the id out of the link.
    let label: String = title
        .chars()
        .filter(|ch| *ch != '[' && *ch != ']')
        .collect();
    let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
    let label = if label.is_empty() {
        id.to_string()
    } else {
        label
    };
    format!("[{label}](futureos://session/{id}) ")
}

/// The largest byte offset at or below `at` that is a character boundary.
fn floor_char_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The command palette's entries: `(command, description)`. Kept here rather
/// than in the app so the palette, the help overlay and the tests share one
/// list.
pub const COMMANDS: [(&str, Key); 28] = [
    ("/new", Key::CmdNew),
    ("/queue", Key::CmdQueue),
    ("/copy", Key::CmdCopy),
    ("/mouse", Key::CmdMouse),
    ("/diff", Key::CmdDiff),
    ("/approval", Key::CmdApproval),
    ("/history", Key::CmdHistory),
    ("/tree", Key::CmdTree),
    ("/fork", Key::CmdFork),
    ("/worktree", Key::CmdWorktree),
    ("/sandbox", Key::CmdSandbox),
    ("/permission", Key::CmdPermission),
    ("/compact", Key::CmdCompact),
    ("/tools", Key::CmdTools),
    ("/providers", Key::CmdProviders),
    ("/sessions", Key::CmdSessions),
    ("/model", Key::CmdModel),
    ("/lang", Key::CmdLanguage),
    ("/reply-detail", Key::CmdReplyDetail),
    ("/workspace", Key::CmdWorkspace),
    ("/theme", Key::CmdTheme),
    ("/preferences", Key::CmdPreferences),
    ("/usage", Key::CmdUsage),
    ("/skills", Key::CmdSkills),
    ("/export", Key::CmdExport),
    ("/transcript", Key::CmdTranscript),
    ("/help", Key::CmdHelp),
    ("/quit", Key::CmdQuit),
];

/// A typed draft that names one of the surface's own verbs.
struct TypedCommand {
    /// The verb with its slash (`/model`).
    name: String,
    /// Everything after the first space, trimmed.
    arg: String,
    /// The whole line, shaped like a `WbAction::Command`.
    raw: String,
}

/// Split a submitted draft into a known command, or `None` when it is a prompt.
///
/// The list is [`COMMANDS`]: the palette's own entries, so a verb the palette
/// offers can also be typed. A draft that merely *starts* with a slash
/// (`/etc/hosts is missing`) stays a prompt.
fn known_command(text: &str) -> Option<TypedCommand> {
    let text = text.trim();
    let mut parts = text.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("");
    if !COMMANDS
        .iter()
        .any(|(command, _)| command.eq_ignore_ascii_case(name))
    {
        return None;
    }
    Some(TypedCommand {
        name: name.to_string(),
        arg: parts.next().unwrap_or("").trim().to_string(),
        raw: text.to_string(),
    })
}

/// The help overlay's key table.
pub const HELP_KEYS: [(&str, Key); 10] = [
    ("tab", Key::HelpKeyFocus),
    ("↑↓", Key::HelpKeyScroll),
    ("pageUp / pageDown", Key::HelpKeyPage),
    ("/", Key::HelpKeyPalette),
    ("@", Key::HelpKeyMentionFile),
    ("#", Key::HelpKeyMentionSession),
    ("?", Key::HelpKeyHelp),
    ("click", Key::HelpKeyClick),
    ("drag", Key::HelpKeyDrag),
    ("ctrl+c", Key::HelpKeyQuit),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workbench::state::{ActivityKind, SessionRow};
    use crate::workbench::theme::{FUTURE_DARK, PALETTES};

    fn plain(rows: &[String]) -> String {
        rows.iter()
            .map(|row| crate::utils::strip_ansi_codes(row))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A surface pinned to Chinese, which is the language every assertion about
    /// the copy below is written in. The English spellings have their own cases.
    fn workbench() -> Workbench {
        let mut workbench = Workbench::new();
        workbench.set_language(LanguagePreference::Zh);
        workbench
    }

    fn frame(data: &WorkbenchData, w: u16, h: u16) -> (Workbench, Vec<String>) {
        let mut workbench = workbench();
        let rows = workbench.render(data, w, h);
        (workbench, rows)
    }

    /// Press the middle of a painted region: the same path a real click takes,
    /// resolved against the hit map the last frame registered.
    fn click_region(
        workbench: &mut Workbench,
        data: &WorkbenchData,
        w: u16,
        h: u16,
        id: &str,
    ) -> Vec<WbAction> {
        workbench.render(data, w, h);
        let (rect, _) = workbench
            .hits()
            .regions()
            .iter()
            .find(|(_, region)| region == id)
            .cloned()
            .unwrap_or_else(|| panic!("no region {id}"));
        let press = mouse::MouseEvent {
            kind: mouse::MouseKind::Press,
            x: rect.x + 1,
            y: rect.y,
            shift: false,
            alt: false,
            ctrl: false,
        };
        workbench.handle(WbEvent::Mouse(press), data)
    }

    #[test]
    fn workflow_keys_search_history_choose_forks_and_never_deny_on_escape() {
        let mut data = demo::chat();
        data.fork_points = vec![state::ForkPoint {
            id: "entry-1".into(),
            text: "start".into(),
        }];
        let mut wb = workbench();
        wb.overlay = Some(Overlay::History);
        wb.handle(WbEvent::Key("q".into()), &data);
        assert_eq!(wb.history_query, "q");
        assert_eq!(wb.overlay, Some(Overlay::History));
        wb.handle(WbEvent::Key("backspace".into()), &data);
        wb.handle(WbEvent::Key("down".into()), &data);
        assert_eq!(wb.workflow_index, 1);
        wb.handle(WbEvent::Key("escape".into()), &data);
        assert_eq!(wb.overlay, None);

        wb.overlay = Some(Overlay::Fork);
        wb.workflow_index = 0;
        assert_eq!(
            wb.handle(WbEvent::Key("enter".into()), &data),
            vec![WbAction::ForkAt("entry-1".into())]
        );
        wb.overlay = Some(Overlay::Approval);
        assert_eq!(
            wb.handle(WbEvent::Key("escape".into()), &data),
            vec![WbAction::Redraw]
        );
        assert_eq!(wb.overlay, None);
    }

    #[test]
    fn diff_file_navigation_is_separate_from_patch_scrolling() {
        let mut data = demo::chat();
        data.diff = Some(state::DiffView {
            files: vec![
                diff_review::DiffFile {
                    path: "a".into(),
                    status: "M".into(),
                },
                diff_review::DiffFile {
                    path: "b".into(),
                    status: "M".into(),
                },
            ],
            ..state::DiffView::default()
        });
        let mut wb = workbench();
        wb.overlay = Some(Overlay::Diff);
        wb.handle(WbEvent::Key("down".into()), &data);
        assert_eq!(wb.overlay_scroll, 1);
        assert_eq!(wb.workflow_index, 0);
        assert_eq!(
            wb.handle(WbEvent::Key("right".into()), &data),
            vec![WbAction::SelectDiff(1)]
        );
    }

    #[test]
    fn diff_mouse_selects_a_file_and_approval_buttons_do_not_hide_in_scroll() {
        let mut data = demo::chat();
        data.diff = Some(state::DiffView {
            files: (0..4)
                .map(|index| diff_review::DiffFile {
                    path: format!("src/{index}.rs"),
                    status: "M".into(),
                })
                .collect(),
            patch: (0..40)
                .map(|index| format!("+line {index}"))
                .collect::<Vec<_>>()
                .join("\n"),
            ..state::DiffView::default()
        });
        data.approval = Some(state::ApprovalView {
            request_id: "a".into(),
            ..state::ApprovalView::default()
        });
        let mut wb = workbench();
        wb.overlay = Some(Overlay::Diff);
        wb.render(&data, 120, 38);
        let region = wb
            .hits()
            .regions()
            .iter()
            .find(|(_, id)| id == "diff.file.1")
            .unwrap()
            .0;
        let event = mouse::MouseEvent {
            kind: mouse::MouseKind::Press,
            x: region.x + 1,
            y: region.y,
            shift: false,
            alt: false,
            ctrl: false,
        };
        assert_eq!(
            wb.handle(WbEvent::Mouse(event), &data),
            vec![WbAction::SelectDiff(1)]
        );
        wb.workflow_index = 1;
        data.diff.as_mut().unwrap().selected = 1;
        wb.diff_patch_focus = false;
        wb.handle(WbEvent::Key("tab".into()), &data);
        assert!(wb.diff_patch_focus);
        wb.handle(WbEvent::Key("up".into()), &data);
        assert_eq!(wb.workflow_index, 1);
        assert_eq!(wb.overlay_scroll, 0);
        wb.handle(WbEvent::Key("tab".into()), &data);
        assert!(!wb.diff_patch_focus);
        wb.handle(WbEvent::Key("up".into()), &data);
        assert_eq!(wb.workflow_index, 0);
        wb.overlay = Some(Overlay::Approval);
        let text = plain(&wb.render(&data, 80, 12));
        assert!(text.contains("[y]") && text.contains("[n]"), "{text}");
        assert_eq!(
            click_region(&mut wb, &data, 120, 38, "approval.n"),
            vec![WbAction::ApprovalDecision {
                approved: false,
                rule: false,
            }]
        );
    }

    #[test]
    fn centred_preference_and_narrow_diff_clicks_use_the_painted_geometry() {
        let mut data = demo::chat();
        let mut wb = workbench();
        wb.open_preferences();
        assert_eq!(
            click_region(&mut wb, &data, 120, 38, "preferences.shortcuts"),
            vec![WbAction::Redraw]
        );
        assert_eq!(wb.overlay, Some(Overlay::Keybindings));
        wb.keybinding_index = 4;
        wb.awaiting_keybinding = false;
        assert_eq!(
            click_region(&mut wb, &data, 120, 38, "keybinding.row.4"),
            vec![WbAction::Redraw]
        );
        assert!(wb.awaiting_keybinding);

        data.diff = Some(state::DiffView {
            files: (0..3)
                .map(|index| diff_review::DiffFile {
                    path: format!("src/{index}.rs"),
                    status: "M".into(),
                })
                .collect(),
            ..state::DiffView::default()
        });
        wb.overlay = Some(Overlay::Diff);
        wb.diff_patch_focus = true;
        let text = plain(&wb.render(&data, 80, 24));
        assert!(!text.contains("src/0.rs"), "{text}");
        wb.diff_patch_focus = false;
        let text = plain(&wb.render(&data, 80, 24));
        assert!(text.contains("src/0.rs"), "{text}");
        assert_eq!(
            click_region(&mut wb, &data, 80, 24, "diff.file.1"),
            vec![WbAction::SelectDiff(1)]
        );
        wb.workflow_index = 1;
        data.diff.as_mut().unwrap().selected = 1;
        wb.diff_patch_focus = true;
        assert_eq!(
            wb.handle(WbEvent::Key("down".into()), &data),
            vec![WbAction::Redraw]
        );
        assert_eq!(wb.overlay_scroll, 1);
    }

    #[test]
    fn multiline_editing_and_queue_actions_use_shared_state() {
        let mut data = demo::chat();
        data.input.editor.paste("one");
        data.input.text = "one".into();
        let mut wb = workbench();
        let actions = wb.handle(WbEvent::Key("alt+enter".into()), &data);
        let [WbAction::EditEditor(editor)] = actions.as_slice() else {
            panic!("{actions:?}")
        };
        data.input.editor = editor.as_ref().clone();
        data.input.text = editor.text().into();
        assert_eq!(data.input.text, "one\n");
        let actions = wb.handle(WbEvent::Paste("two".into()), &data);
        let [WbAction::EditEditor(editor)] = actions.as_slice() else {
            panic!("{actions:?}")
        };
        data.input.editor = editor.as_ref().clone();
        data.input.text = editor.text().into();
        assert_eq!(data.input.text, "one\ntwo");
        let rows = wb.render(&data, 90, 24);
        assert!(plain(&rows).contains("one"));
        assert!(plain(&rows).contains("two"));

        data.queue = vec![state::QueuedPrompt {
            run_id: "q1".into(),
            text: "later".into(),
        }];
        assert!(matches!(
            wb.surface_command("/queue", &data),
            VerbOutcome::Done { .. }
        ));
        assert_eq!(
            wb.handle(WbEvent::Key("x".into()), &data),
            vec![WbAction::CancelQueued("q1".into())]
        );
        assert_eq!(
            wb.handle(WbEvent::Key("r".into()), &data),
            vec![WbAction::RecallQueued("q1".into())]
        );
    }

    #[test]
    fn new_output_never_steals_a_manual_scroll_position() {
        let mut data = demo::chat();
        data.messages = (0..100).map(|n| format!("line {n}")).collect();
        let mut wb = workbench();
        wb.render(&data, 120, 38);
        wb.handle(WbEvent::Key("pageUp".into()), &data);
        let offset = wb.chat_scroll_offset();
        assert!(offset > 0 && !wb.following_latest());
        data.revision += 1;
        let text = plain(&wb.render(&data, 120, 38));
        assert_eq!(wb.chat_scroll_offset(), offset);
        assert!(wb.has_new_output());
        assert!(text.contains("新输出"));
        wb.handle(WbEvent::Key("escape".into()), &data);
        assert!(wb.following_latest() && !wb.has_new_output());
        assert_eq!(wb.chat_scroll_offset(), 0);
    }

    #[test]
    fn preferences_edit_the_frame_and_configurable_keys_drive_actions() {
        let data = demo::chat();
        let mut wb = workbench();
        assert_eq!(
            wb.handle(WbEvent::Key("ctrl+p".into()), &data),
            vec![WbAction::Redraw]
        );
        assert_eq!(wb.overlay, Some(Overlay::Palette));
        wb.overlay = None;
        assert!(wb
            .handle(WbEvent::Key("ctrl+r".into()), &data)
            .contains(&WbAction::Redraw));
        assert_eq!(wb.overlay, Some(Overlay::Picker(PickerKind::Sessions)));
        wb.overlay = None;
        wb.keys
            .bind(preferences::Action::Palette, "ctrl+j")
            .unwrap();
        wb.handle(WbEvent::Key("ctrl+j".into()), &data);
        assert_eq!(wb.overlay, Some(Overlay::Palette));
        wb.overlay = None;
        wb.open_preferences();
        wb.handle(WbEvent::Key("right".into()), &data);
        assert_eq!(wb.density, preferences::Density::Comfortable);
        wb.handle(WbEvent::Key("down".into()), &data);
        wb.handle(WbEvent::Key("right".into()), &data);
        assert_eq!(wb.sidebar_position, preferences::SidebarPosition::Left);
        let _ = wb.render(&data, 120, 38);
        assert!(wb
            .hits()
            .regions()
            .iter()
            .any(|(rect, id)| id == "divider" && rect.x == wb.sidebar_w));
        wb.overlay = None;
        wb.density = preferences::Density::Focused;
        let _ = wb.render(&data, 120, 38);
        let rail = wb
            .hits()
            .regions()
            .iter()
            .find(|(_, id)| id == "sidebar.tab.todo")
            .unwrap()
            .0;
        assert_eq!(rail.x, 0);
    }

    #[test]
    fn keybinding_editor_rebinds_and_invalid_keys_leave_the_binding_intact() {
        let data = demo::chat();
        let mut wb = workbench();
        wb.overlay = Some(Overlay::Keybindings);
        wb.handle(WbEvent::Key("enter".into()), &data);
        assert!(wb.awaiting_keybinding);
        assert_eq!(
            wb.handle(WbEvent::Key("ctrl+j".into()), &data),
            vec![WbAction::SetKeybindings]
        );
        assert_eq!(wb.keys.key(preferences::Action::Palette), "ctrl+j");
        wb.handle(WbEvent::Key("enter".into()), &data);
        let actions = wb.handle(WbEvent::Key("x".into()), &data);
        assert_eq!(actions.len(), 1);
        assert_eq!(wb.keys.key(preferences::Action::Palette), "ctrl+j");
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
        // Focused density gives the conversation the space the old branding
        // block consumed, while keeping identity and context on the first row.
        assert!(
            text.contains("FUTURE OS"),
            "the text brand is drawn: {text}"
        );
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

    /// Every entry the palette advertises has exactly one handler: a picker, a
    /// verb the surface carries out, or a verb the app runs against the agent.
    ///
    /// A table test is the only way this does not rot: five entries
    /// (`/usage`, `/skills`, `/workspace`, `/export`, `/transcript`) used to be
    /// listed, selectable, and then answered `Unknown command` by the agent,
    /// which has no such commands. Adding a sixth entry without a handler now
    /// fails here instead of in front of the reader.
    #[test]
    fn every_palette_entry_has_a_handler() {
        let data = demo::chat();
        let mut workbench = workbench();
        workbench.render(&data, 120, 38);
        for (command, key) in COMMANDS {
            let description = t(workbench.lang, key);
            let picker = PickerKind::from_command(command).is_some();
            let surface = matches!(
                workbench.surface_command(command, &data),
                VerbOutcome::Done { .. }
            );
            let app = crate::workbench::live::APP_COMMANDS.contains(&command);
            let handlers = [picker, surface, app]
                .iter()
                .filter(|handled| **handled)
                .count();
            assert_eq!(
                handlers, 1,
                "{command} ({description}) resolves to {handlers} handlers: \
                 picker={picker} surface={surface} app={app}"
            );
        }
    }

    /// `/usage` and `/skills` are names for two panels. With room for a rail
    /// they focus it (showing it if the reader had hidden it); on a terminal too
    /// narrow for a rail they take the body, which is the one surface there is.
    #[test]
    fn usage_and_skills_focus_their_panel_and_never_reach_the_agent() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);

        assert_eq!(
            workbench.surface_command("/usage", &data),
            VerbOutcome::Done { notice: None }
        );
        assert_eq!(workbench.panel, PanelId::Usage);
        assert!(workbench.sidebar_focus, "the panel it names takes the keys");
        assert!(workbench.sidebar_visible);
        let text = plain(&workbench.render(&data, 120, 38));
        assert!(
            text.contains("上下文"),
            "the usage panel is on screen: {text}"
        );

        assert_eq!(
            workbench.surface_command("/skills", &data),
            VerbOutcome::Done { notice: None }
        );
        assert_eq!(workbench.panel, PanelId::Skills);
        let text = plain(&workbench.render(&data, 120, 38));
        assert!(
            text.contains("个技能"),
            "the skills panel is on screen: {text}"
        );

        // A hidden rail comes back rather than swallowing the request.
        workbench.sidebar_visible = false;
        workbench.sidebar_focus = false;
        workbench.surface_command("/usage", &data);
        assert!(workbench.sidebar_visible && workbench.sidebar_focus);
    }

    /// Below the rail's breakpoint the same verbs open the full-screen panel:
    /// focusing a rail that cannot be painted would answer with nothing.
    #[test]
    fn a_narrow_terminal_gets_the_panel_full_screen() {
        let data = demo::chat();
        let (mut workbench, rows) = frame(&data, 60, 20);
        // No rail at this width: the panel's tab strip is not on screen to
        // click, and `ctrl+b` is the only way in.
        assert!(!layout::sidebar_fits(60));
        assert!(!plain(&rows).contains("用量"));

        assert_eq!(
            workbench.surface_command("/usage", &data),
            VerbOutcome::Done { notice: None }
        );
        assert_eq!(workbench.overlay, Some(Overlay::PanelFull));
        let text = plain(&workbench.render(&data, 60, 20));
        assert!(text.contains("上下文"), "the panel takes the body: {text}");
        // The rail's own key still leaves it, which is what the hint row already
        // names at this width (`ctrl+b 侧栏`), so the overlay is not a dead end
        // even though its title line carries no footer of its own.
        assert!(text.contains("ctrl+b"));
    }

    /// The verbs that cannot be carried out here answer with what *is* true
    /// instead of leaving the reader at a dead end.
    #[test]
    fn workspace_and_transcript_explain_themselves() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        match workbench.surface_command("/workspace", &data) {
            VerbOutcome::Done {
                notice: Some(notice),
            } => {
                assert!(
                    notice.contains(&data.session.cwd),
                    "names the workspace: {notice}"
                );
                assert!(
                    notice.contains("/sessions"),
                    "names a verb that moves: {notice}"
                );
            }
            other => panic!("/workspace must explain itself: {other:?}"),
        }
        match workbench.surface_command("/transcript", &data) {
            VerbOutcome::Done {
                notice: Some(notice),
            } => {
                assert!(notice.contains("/export"), "points at the export: {notice}");
                assert!(
                    notice.contains("/reply-detail"),
                    "and at the detail toggle: {notice}"
                );
            }
            other => panic!("/transcript must explain itself: {other:?}"),
        }
        // The ones that need the agent stay the agent's.
        assert_eq!(
            workbench.surface_command("/export", &data),
            VerbOutcome::Agent
        );
        assert_eq!(
            workbench.surface_command("/model", &data),
            VerbOutcome::Agent
        );
    }

    /// `/help` used to be a no-op arm in the app: the palette advertised it and
    /// selecting it changed nothing at all. It opens the same box `?` does.
    #[test]
    fn help_opens_the_key_table() {
        let data = demo::chat();
        let (mut workbench, rows) = frame(&data, 120, 38);
        assert!(!plain(&rows).contains("ctrl+q"));
        assert_eq!(
            workbench.surface_command("/help", &data),
            VerbOutcome::Done { notice: None }
        );
        assert_eq!(workbench.overlay, Some(Overlay::Help));
        let text = plain(&workbench.render(&data, 120, 38));
        assert!(
            text.contains("这份帮助"),
            "the key table is on screen: {text}"
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
        // Clicking the sidebar is working *in* the sidebar: one click both
        // switches the panel and takes the keys, so `up`/`down` and the wheel
        // act on what the reader just chose.
        assert!(workbench.sidebar_focus);
        // The panel repaints with the sessions it now shows.
        let text = plain(&workbench.render(&data, 120, 38));
        assert!(text.contains("2m"), "a session's age: {text}");
        // …and a click back in the conversation hands the keys over again.
        let (chat, _) = workbench
            .hits()
            .regions()
            .iter()
            .find(|(_, id)| id == "chat")
            .cloned()
            .expect("the conversation is a target");
        let back = mouse::MouseEvent {
            kind: mouse::MouseKind::Press,
            x: chat.x + 2,
            y: chat.y + 2,
            shift: false,
            alt: false,
            ctrl: false,
        };
        assert_eq!(
            workbench.handle(WbEvent::Mouse(back), &data),
            vec![WbAction::FocusPane(PaneId::Chat)]
        );
        assert!(!workbench.sidebar_focus);
    }

    /// Where the divider is painted, resolved from the hit map of the last frame
    /// — the same coordinate a click arrives in.
    fn divider_x(workbench: &Workbench) -> u16 {
        workbench
            .hits()
            .regions()
            .iter()
            .find(|(_, id)| id == "divider")
            .map(|(rect, _)| rect.x)
            .expect("the divider is a target")
    }

    fn mouse_at(kind: mouse::MouseKind, x: u16) -> mouse::MouseEvent {
        mouse::MouseEvent {
            kind,
            x,
            y: 12,
            shift: false,
            alt: false,
            ctrl: false,
        }
    }

    /// The divider follows the pointer: a rightward drag *narrows* the rail (its
    /// seam sits on the frame's right edge), a leftward drag widens it.
    ///
    /// Asserted in **columns**, not widths, because columns are what the reader
    /// sees. The bug this pins — the first version *added* the pointer's
    /// movement instead of subtracting it — left a rightward drag with the seam
    /// six columns to the *left* of where it started: press col 84, drag to col
    /// 90, divider at col 78, rail 42.
    #[test]
    fn a_dragged_divider_lands_under_the_pointer() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        let seam = divider_x(&workbench);
        // 120 columns, the default 36-wide rail: the seam is 0-based column 83,
        // i.e. the 84th column.
        assert_eq!((seam, workbench.sidebar_w), (83, 36));

        // Rightward by 6: the rail gives up six columns, and the seam ends on
        // the pointer's own column (89).
        assert!(workbench
            .handle(
                WbEvent::Mouse(mouse_at(mouse::MouseKind::Press, seam)),
                &data
            )
            .is_empty());
        assert_eq!(
            workbench.handle(
                WbEvent::Mouse(mouse_at(mouse::MouseKind::Drag, seam + 6)),
                &data
            ),
            vec![WbAction::ResizeSidebar(30)]
        );
        workbench.handle(
            WbEvent::Mouse(mouse_at(mouse::MouseKind::Release, seam + 6)),
            &data,
        );
        workbench.render(&data, 120, 38);
        assert_eq!(divider_x(&workbench), seam + 6);

        // Leftward from there: six columns back, seam back at 83, rail at 36.
        let seam = divider_x(&workbench);
        workbench.handle(
            WbEvent::Mouse(mouse_at(mouse::MouseKind::Press, seam)),
            &data,
        );
        assert_eq!(
            workbench.handle(
                WbEvent::Mouse(mouse_at(mouse::MouseKind::Drag, seam - 6)),
                &data
            ),
            vec![WbAction::ResizeSidebar(36)]
        );
        workbench.handle(
            WbEvent::Mouse(mouse_at(mouse::MouseKind::Release, seam - 6)),
            &data,
        );
        workbench.render(&data, 120, 38);
        assert_eq!(divider_x(&workbench), seam - 6);
    }

    /// Both ends clamp instead of running off: a rail is never under 30 columns
    /// or over 48, however far the pointer travels in one drag.
    #[test]
    fn the_drag_clamps_and_never_runs_off_the_frame() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        let seam = divider_x(&workbench);
        workbench.handle(
            WbEvent::Mouse(mouse_at(mouse::MouseKind::Press, seam)),
            &data,
        );
        // Far right, then the last column: 30 is the floor, and once it is
        // reached a further drag is a no-op instead of a second report.
        assert_eq!(
            workbench.handle(WbEvent::Mouse(mouse_at(mouse::MouseKind::Drag, 200)), &data),
            vec![WbAction::ResizeSidebar(layout::SIDEBAR_MIN)]
        );
        assert!(workbench
            .handle(WbEvent::Mouse(mouse_at(mouse::MouseKind::Drag, 119)), &data)
            .is_empty());
        assert_eq!(workbench.sidebar_w, layout::SIDEBAR_MIN);
        // The floor holds however far past the frame the pointer goes, and a
        // drag back left still works from there (the delta is measured from the
        // press, not accumulated per report).
        assert!(workbench
            .handle(WbEvent::Mouse(mouse_at(mouse::MouseKind::Drag, 255)), &data)
            .is_empty());
        assert_eq!(workbench.sidebar_w, layout::SIDEBAR_MIN);
        // Far left: 48 is the ceiling, and the drag does not go past it.
        assert_eq!(
            workbench.handle(WbEvent::Mouse(mouse_at(mouse::MouseKind::Drag, 0)), &data),
            vec![WbAction::ResizeSidebar(layout::SIDEBAR_MAX)]
        );
        assert_eq!(workbench.sidebar_w, layout::SIDEBAR_MAX);
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
        // Six columns to the right of the seam: six columns off the rail, and
        // the divider moves to the column the pointer is on.
        let actions = workbench.handle(WbEvent::Mouse(drag), &data);
        assert_eq!(actions, vec![WbAction::ResizeSidebar(30)]);
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
            let [WbAction::EditEditor(editor)] = actions.as_slice() else {
                panic!("{key}: {actions:?}")
            };
            assert_eq!((editor.text(), editor.cursor()), (key, 1), "{key}");
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

    /// A copy of `data` whose draft holds `typed`, caret at the end.
    fn with_input(data: &WorkbenchData, typed: &str) -> WorkbenchData {
        let mut data = data.clone();
        let mut editor = data.input.editor.clone();
        editor.set(typed, typed.len());
        data.input = state::InputView::from_editor(editor, String::new());
        data
    }

    /// The `@` menu is *derived* from the draft, so the assertions here are
    /// about one token: what opens it, what it offers, and what an insert
    /// replaces.
    #[test]
    fn at_opens_a_file_menu_from_the_token_the_caret_is_in() {
        let mut data = demo::chat();
        data.files = vec![
            "tui/src/workbench/mod.rs".into(),
            "tui/src/workbench/screen.rs".into(),
            "docs/tui/workbench-spec.md".into(),
        ];
        let (mut workbench, _rows) = frame(&data, 120, 38);
        data = with_input(&data, "看 @scr");
        // One frame is enough: the menu is a reading of the draft, taken when
        // the frame is composed.
        workbench.render(&data, 120, 38);
        let mention = workbench.mention.as_ref().expect("@ opens the menu");
        assert_eq!(mention.kind, MentionKind::File);
        assert_eq!(
            mention.start,
            "看 ".len(),
            "the sigil is where the token starts"
        );
        assert_eq!(mention.query, "scr");
        let rows = workbench.mention_rows(&data);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].label, "tui/src/workbench/screen.rs");
        // Enter replaces the token and nothing else.
        let actions = workbench.handle(WbEvent::Key("enter".into()), &data);
        let [WbAction::EditInput { text, cursor }] = actions.as_slice() else {
            panic!("{actions:?}");
        };
        assert_eq!(text, "看 @tui/src/workbench/screen.rs ");
        assert_eq!(*cursor, text.len());
        assert!(workbench.mention.is_none(), "inserting closes the menu");
    }

    #[test]
    fn a_sigil_inside_a_word_is_not_a_mention() {
        // An e-mail address and a channel name are text, not requests for a
        // menu — the menu is for a token that *starts* with the sigil.
        for typed in ["mail me@example.com", "x#y", "a/b"] {
            let data = with_input(&demo::chat(), typed);
            let (mut workbench, _rows) = frame(&data, 120, 38);
            workbench.render(&data, 120, 38);
            assert!(workbench.mention.is_none(), "{typed}");
        }
        // A bare `#` is one, with an empty query: the reader has typed the
        // sigil and nothing else yet.
        let data = with_input(&demo::chat(), "#");
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.render(&data, 120, 38);
        assert_eq!(
            workbench.mention.as_ref().map(|m| m.kind),
            Some(MentionKind::Session)
        );
    }

    #[test]
    fn hash_offers_conversations_and_inserts_the_link_the_rest_of_the_app_uses() {
        let data = with_input(&demo::chat(), "#");
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.render(&data, 120, 38);
        let rows = workbench.mention_rows(&data);
        assert!(!rows.is_empty(), "the demo has conversations");
        assert!(rows[0].insert.contains("futureos://session/"), "{rows:?}");
        let actions = workbench.handle(WbEvent::Key("tab".into()), &data);
        let [WbAction::EditInput { text, .. }] = actions.as_slice() else {
            panic!("{actions:?}");
        };
        assert!(
            text.starts_with(&format!(
                "[{}](futureos://session/{})",
                demo::chat().sessions[0].title,
                demo::chat().sessions[0].id
            )),
            "{text}"
        );
        // Brackets in a title would close the label early and push the id out of
        // the link, so they are dropped rather than escaped.
        assert_eq!(
            session_reference("[a] b", "s1"),
            "[a b](futureos://session/s1) "
        );
        assert_eq!(
            session_reference("   ", "s2"),
            "[s2](futureos://session/s2) "
        );
    }

    #[test]
    fn escape_closes_the_mention_menu_without_touching_the_draft() {
        let data = with_input(&demo::chat(), "看 @t");
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.render(&data, 120, 38);
        assert!(workbench.mention.is_some());
        let actions = workbench.handle(WbEvent::Key("escape".into()), &data);
        assert_eq!(actions, vec![WbAction::Redraw]);
        assert!(workbench.mention.is_none());
        // The same token does not reopen the menu on the next frame…
        workbench.render(&data, 120, 38);
        assert!(workbench.mention.is_none());
        // …but typing on does.
        let typed = with_input(&demo::chat(), "看 @tu");
        workbench.render(&typed, 120, 38);
        assert!(workbench.mention.is_some());
    }

    #[test]
    fn a_draft_with_no_mention_sends_its_at_sign_as_text() {
        // `@` with nothing after it is a mention with an empty query, so `enter`
        // on an unmatched list must still submit rather than swallow the key.
        let mut data = with_input(&demo::chat(), "@");
        data.files.clear();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.render(&data, 120, 38);
        let actions = workbench.handle(WbEvent::Key("enter".into()), &data);
        assert!(
            matches!(actions.as_slice(), [WbAction::Submit(text)] if text == "@"),
            "{actions:?}"
        );
    }

    /// The menu is drawn, not just computed: the box is on screen with the
    /// query and the highlighted candidate, and the draft under it still shows.
    #[test]
    fn the_mention_menu_is_painted_above_the_composer() {
        let mut data = demo::chat();
        data.files = vec!["src/main.rs".into(), "src/lib.rs".into()];
        data = with_input(&data, "read @src");
        let (mut workbench, _rows) = frame(&data, 120, 38);
        let text = plain(&workbench.render(&data, 120, 38));
        assert!(text.contains("src/main.rs"), "{text}");
        assert!(
            text.contains("read @src"),
            "the draft stays visible: {text}"
        );
        // The box is above the composer, never over it.
        let rows = workbench
            .hits()
            .regions()
            .iter()
            .find(|(_, id)| id == "mention.row.0")
            .map(|(rect, _)| rect.y)
            .expect("the first candidate is a target");
        let input_top = workbench
            .hits()
            .regions()
            .iter()
            .find(|(_, id)| id == "input")
            .map(|(rect, _)| rect.y)
            .expect("the composer is a target");
        assert!(rows < input_top, "{rows} < {input_top}");
        // A click on a candidate inserts it, the way `enter` does.
        let actions = click_region(&mut workbench, &data, 120, 38, "mention.row.1");
        let [WbAction::EditInput { text, .. }] = actions.as_slice() else {
            panic!("{actions:?}");
        };
        assert_eq!(text, "read @src/lib.rs ");
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
    fn ctrl_b_cycles_open_focus_closed_on_a_wide_terminal() {
        let data = demo::chat();
        let (mut workbench, rows) = frame(&data, 120, 38);
        assert!(workbench.sidebar_visible && !workbench.sidebar_focus);
        assert!(plain(&rows).contains("待办"));

        // open → focus
        workbench.handle(WbEvent::Key("ctrl+b".into()), &data);
        assert!(workbench.sidebar_visible && workbench.sidebar_focus);

        // focus → closed
        workbench.handle(WbEvent::Key("ctrl+b".into()), &data);
        assert!(!workbench.sidebar_visible && !workbench.sidebar_focus);
        let rows = workbench.render(&data, 120, 38);
        assert!(
            !plain(&rows).contains("待办"),
            "the rail is gone: {}",
            plain(&rows)
        );
        assert!(workbench
            .hits()
            .regions()
            .iter()
            .all(|(_, id)| !id.starts_with("sidebar.")));
        assert!(
            workbench.overlay.is_none(),
            "hiding the rail is not the full-screen panel"
        );

        // closed → open (and focused). The width decides whether a rail can
        // exist — asking the *layout* with a hidden rail would answer "none" and
        // open the overlay on a 120-column terminal.
        workbench.handle(WbEvent::Key("ctrl+b".into()), &data);
        assert!(workbench.sidebar_visible && workbench.sidebar_focus);
        assert!(
            workbench.overlay.is_none(),
            "a hidden sidebar on a wide terminal is not a narrow one"
        );

        // A hidden rail cannot take the keys: focusing it would swallow
        // up/down with nothing on screen to show for it.
        workbench.handle(WbEvent::Key("ctrl+b".into()), &data); // hide
        assert!(!workbench.sidebar_visible);
        workbench.handle(WbEvent::Key("tab".into()), &data);
        assert!(!workbench.sidebar_focus);
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
            parent_id: None,
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

    /// Submit a draft through the surface, the way the driver does: the app
    /// applies the edit, then the surface sees the new text.
    fn submit_draft(
        workbench: &mut Workbench,
        data: &mut WorkbenchData,
        text: &str,
    ) -> Vec<WbAction> {
        data.input.text = text.into();
        data.input.cursor = text.len();
        workbench.handle(WbEvent::Key("enter".into()), data)
    }

    #[test]
    fn typing_a_menu_command_opens_it_instead_of_prompting_the_agent() {
        let mut data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        // `/model` used to be sent to the agent as the prompt text, which is
        // why no menu ever opened from the input box.
        let actions = submit_draft(&mut workbench, &mut data, "/model");
        assert_eq!(workbench.overlay, Some(Overlay::Picker(PickerKind::Model)));
        // The draft is cleared and nothing is submitted.
        assert_eq!(
            actions,
            vec![
                WbAction::EditInput {
                    text: String::new(),
                    cursor: 0
                },
                WbAction::Redraw,
            ]
        );
        for (command, kind) in [
            ("/theme", PickerKind::Theme),
            ("/sessions", PickerKind::Sessions),
        ] {
            let mut data = demo::chat();
            let (mut workbench, _rows) = frame(&data, 120, 38);
            submit_draft(&mut workbench, &mut data, command);
            assert_eq!(workbench.overlay, Some(Overlay::Picker(kind)), "{command}");
        }
        // A markdown-ish prompt that starts with a slash is still a prompt, and
        // a verb with an argument keeps going to the agent.
        let mut data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        assert_eq!(
            submit_draft(&mut workbench, &mut data, "/etc/hosts 里少了一行"),
            vec![WbAction::Submit("/etc/hosts 里少了一行".into())]
        );
        assert_eq!(
            submit_draft(&mut workbench, &mut data, "/model Qwen"),
            vec![
                WbAction::EditInput {
                    text: String::new(),
                    cursor: 0
                },
                WbAction::Command("/model Qwen".into()),
            ]
        );
        assert_eq!(workbench.overlay, None);
    }

    #[test]
    fn the_model_picker_shows_the_models_and_applies_the_row_with_the_slider() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.open_picker(PickerKind::Model, &data);
        let text = plain(&workbench.render(&data, 120, 38));
        // The title, the two lists and the slider are all on screen.
        assert!(text.contains("模型"), "{text}");
        assert!(text.contains("westlakehpc"), "{text}");
        assert!(text.contains("azure"), "{text}");
        assert!(text.contains("推理强度"), "{text}");
        assert!(text.contains("medium"), "the current level: {text}");
        // It opens on the model in force (the demo runs `future/deepseek-flash`).
        assert_eq!(workbench.picker.index, 1);
        assert!(
            text.contains("✓"),
            "the tick marks the row in force: {text}"
        );

        // ↓ lands on Qwen, → raises the reasoning one level, and enter applies
        // both — the model and the effort it will run at.
        workbench.handle(WbEvent::Key("down".into()), &data);
        workbench.handle(WbEvent::Key("right".into()), &data);
        assert_eq!(
            workbench.handle(WbEvent::Key("enter".into()), &data),
            vec![
                WbAction::SetModel("Qwen".into()),
                WbAction::SetThinking("high".into()),
            ]
        );
        assert_eq!(workbench.overlay, None, "applying closes the box");
    }

    #[test]
    fn the_slider_row_applies_the_effort_alone() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.open_picker(PickerKind::Model, &data);
        workbench.handle(WbEvent::Key("up".into()), &data);
        assert_eq!(workbench.picker.index, 0, "the slider is the first row");
        workbench.handle(WbEvent::Key("left".into()), &data);
        assert_eq!(
            workbench.handle(WbEvent::Key("enter".into()), &data),
            vec![WbAction::SetThinking("low".into())]
        );
    }

    /// No session, no request. The agent answers `set_model` and
    /// `set_thinking_level` with "session not found — pass a valid session_id"
    /// when nothing is attached, so a picker opened on the launchpad has to
    /// stage its choice for the session the first message creates instead. The
    /// same two gestures with a session attached keep going over the wire.
    #[test]
    fn a_model_picked_before_any_session_is_staged_not_requested() {
        let data = demo::launchpad();
        assert!(data.session.id.is_empty(), "the launchpad has no session");
        let (mut workbench, _rows) = frame(&data, 120, 38);
        // It still opens on the value in force, exactly like a session's.
        workbench.open_picker(PickerKind::Model, &data);
        assert_eq!(workbench.picker.index, 1);
        workbench.handle(WbEvent::Key("down".into()), &data);
        let actions = workbench.handle(WbEvent::Key("enter".into()), &data);
        assert_eq!(
            actions,
            vec![
                WbAction::SetPendingModel("Qwen".into()),
                WbAction::SetPendingThinking("medium".into()),
            ],
            "the row and the level it will run at, staged"
        );
        assert_eq!(workbench.overlay, None, "applying still closes the box");

        // The slider alone is staged too — that is the row that produced
        // "Could not set thinking: session not found" before.
        workbench.open_picker(PickerKind::Model, &data);
        workbench.handle(WbEvent::Key("up".into()), &data);
        workbench.handle(WbEvent::Key("left".into()), &data);
        assert_eq!(
            workbench.handle(WbEvent::Key("enter".into()), &data),
            vec![WbAction::SetPendingThinking("low".into())]
        );

        // Nothing on either road may reach the agent: no request can be built
        // from what a launchpad picker returns.
        workbench.open_picker(PickerKind::Model, &data);
        workbench.handle(WbEvent::Key("down".into()), &data);
        let actions = workbench.handle(WbEvent::Key("enter".into()), &data);
        for action in &actions {
            assert!(
                !matches!(
                    action,
                    WbAction::SetModel(_) | WbAction::SetThinking(_) | WbAction::Command(_)
                ),
                "a launchpad pick must not send anything: {action:?}"
            );
        }

        // …and with a session attached the same gestures are the wire calls
        // they always were.
        let chat = demo::chat();
        let (mut workbench, _rows) = frame(&chat, 120, 38);
        workbench.open_picker(PickerKind::Model, &chat);
        workbench.handle(WbEvent::Key("down".into()), &chat);
        assert_eq!(
            workbench.handle(WbEvent::Key("enter".into()), &chat),
            vec![
                WbAction::SetModel("Qwen".into()),
                WbAction::SetThinking("medium".into()),
            ]
        );
    }

    #[test]
    fn escape_cancels_the_model_picker_without_changing_anything() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.open_picker(PickerKind::Model, &data);
        workbench.handle(WbEvent::Key("down".into()), &data);
        workbench.handle(WbEvent::Key("right".into()), &data);
        assert_eq!(
            workbench.handle(WbEvent::Key("escape".into()), &data),
            vec![WbAction::Redraw],
            "nothing is applied: no SetModel and no SetThinking"
        );
        assert_eq!(workbench.overlay, None);
        // …and the surface still paints the model that is actually in force.
        let text = plain(&workbench.render(&data, 120, 38));
        assert!(text.contains("deepseek-flash"), "{text}");
    }

    #[test]
    fn the_theme_picker_previews_on_move_and_escape_restores_what_was_there() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        assert_eq!(workbench.palette.id, FUTURE_DARK.id);
        workbench.open_picker(PickerKind::Theme, &data);
        let text = plain(&workbench.render(&data, 120, 38));
        // All six names, and the one in force is ticked.
        for name in [
            "Dark",
            "Light",
            "One Dark",
            "One Light",
            "High Contrast",
            "Dracula",
        ] {
            assert!(text.contains(name), "{name} missing: {text}");
        }
        assert!(text.contains("✓"), "{text}");
        assert_eq!(workbench.picker.index, 0, "opens on the current theme");

        // Moving the highlight previews the palette the row names…
        workbench.handle(WbEvent::Key("down".into()), &data);
        assert_eq!(workbench.palette.id, "light");
        workbench.handle(WbEvent::Key("down".into()), &data);
        assert_eq!(workbench.palette.id, "one-dark");
        // …while the `✓` stays on what the reader actually has, not on the
        // preview: a menu that ticks its own highlight would claim a choice the
        // reader has not made.
        let view = workbench.picker.view(&data, workbench.committed_theme());
        let ticked: Vec<&str> = view
            .rows
            .iter()
            .filter_map(|row| match row {
                crate::workbench::picker::PickerRow::Item {
                    key, current: true, ..
                } => Some(key.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(ticked, vec!["dark"]);
        // …and escape puts back what was there when the box opened.
        assert_eq!(
            workbench.handle(WbEvent::Key("escape".into()), &data),
            vec![WbAction::Redraw]
        );
        assert_eq!(workbench.palette.id, FUTURE_DARK.id);
        assert_eq!(workbench.overlay, None);

        // Enter commits: the palette stays and the app is told to persist it.
        workbench.open_picker(PickerKind::Theme, &data);
        workbench.handle(WbEvent::Key("down".into()), &data);
        assert_eq!(
            workbench.handle(WbEvent::Key("enter".into()), &data),
            vec![WbAction::SetTheme("light".into())]
        );
        assert_eq!(workbench.palette.id, "light");
        assert_eq!(workbench.overlay, None);
    }

    #[test]
    fn the_sessions_picker_switches_on_enter_and_on_a_click() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.open_picker(PickerKind::Sessions, &data);
        let text = plain(&workbench.render(&data, 120, 38));
        assert!(text.contains("修复 parser 的 off-by-one"), "{text}");
        assert!(text.contains("~/future-os"), "the workspace column: {text}");
        assert!(text.contains("2m"), "the relative age: {text}");
        assert_eq!(workbench.picker.index, 0, "opens on the current session");
        workbench.handle(WbEvent::Key("down".into()), &data);
        assert_eq!(
            workbench.handle(WbEvent::Key("enter".into()), &data),
            vec![WbAction::SwitchSession("s2".into())]
        );
        assert_eq!(workbench.overlay, None);

        // A pointer has no second enter: the click moves and applies in one go.
        workbench.open_picker(PickerKind::Sessions, &data);
        let actions = click_region(&mut workbench, &data, 120, 38, "picker.row.2");
        assert_eq!(actions, vec![WbAction::SwitchSession("s3".into())]);
        assert_eq!(workbench.overlay, None);
    }

    #[test]
    fn a_click_on_a_model_row_completes_the_choice() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.open_picker(PickerKind::Model, &data);
        // Row 2 in the picker's own numbering: the slider, deepseek-flash, Qwen.
        let actions = click_region(&mut workbench, &data, 120, 38, "picker.row.2");
        assert_eq!(
            actions,
            vec![
                WbAction::SetModel("Qwen".into()),
                WbAction::SetThinking("medium".into()),
            ]
        );
        assert_eq!(workbench.overlay, None);
    }

    #[test]
    fn a_click_outside_the_picker_dismisses_it_and_keeps_the_previous_theme() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.open_picker(PickerKind::Theme, &data);
        workbench.handle(WbEvent::Key("down".into()), &data);
        assert_eq!(workbench.palette.id, "light");
        let press = mouse::MouseEvent {
            kind: mouse::MouseKind::Press,
            x: 1,
            y: 1,
            shift: false,
            alt: false,
            ctrl: false,
        };
        assert_eq!(
            workbench.handle(WbEvent::Mouse(press), &data),
            vec![WbAction::Redraw]
        );
        assert_eq!(workbench.overlay, None);
        assert_eq!(
            workbench.palette.id, FUTURE_DARK.id,
            "the preview was undone"
        );
    }

    #[test]
    fn every_picker_keeps_the_frame_the_terminal_size() {
        for kind in PickerKind::ALL {
            for (w, h) in [(120u16, 38u16), (88, 24), (60, 12), (24, 8), (1, 1)] {
                let data = demo::chat();
                let (mut workbench, _rows) = frame(&data, w, h);
                workbench.open_picker(kind, &data);
                let rows = workbench.render(&data, w, h);
                assert_eq!(rows.len(), h as usize, "{kind:?} {w}x{h}");
                for row in &rows {
                    assert_eq!(ansi::width(row), w as usize, "{kind:?} {w}x{h}: {row:?}");
                }
            }
        }
    }

    #[test]
    fn the_picker_filters_as_it_is_typed_and_enter_applies_the_match() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.open_picker(PickerKind::Sessions, &data);
        for ch in "mobile".chars() {
            workbench.handle(WbEvent::Key(ch.to_string()), &data);
        }
        let text = plain(&workbench.render(&data, 120, 38));
        assert!(text.contains("mobile 恢复会话"), "{text}");
        assert!(
            !text.contains("修复 parser 的 off-by-one"),
            "filtered out: {text}"
        );
        assert_eq!(
            workbench.handle(WbEvent::Key("enter".into()), &data),
            vec![WbAction::SwitchSession("s3".into())]
        );
    }

    #[test]
    fn the_theme_cycles_through_the_whole_catalogue() {
        let mut workbench = workbench();
        assert_eq!(workbench.palette.id, FUTURE_DARK.id);
        // Every built-in in turn, wrapping back to the default.
        let mut seen = vec![workbench.palette.id];
        for _ in 1..PALETTES.len() {
            seen.push(workbench.cycle_theme());
        }
        assert_eq!(seen.len(), 6);
        assert_eq!(seen[1], "light");
        assert_eq!(workbench.cycle_theme(), FUTURE_DARK.id);
        // Setting by id accepts the shared catalogue's spelling and the one
        // workbench builds wrote before it.
        assert!(workbench.set_theme("dracula"));
        assert_eq!(workbench.palette.id, "dracula");
        assert!(workbench.set_theme("future-light"));
        assert_eq!(workbench.palette.id, "light");
        assert!(!workbench.set_theme("nope"), "an unknown id is refused");
        assert_eq!(workbench.palette.id, FUTURE_DARK.id);
    }

    #[test]
    fn a_paste_is_an_edit_and_never_a_key() {
        let data = demo::launchpad();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        let actions = workbench.handle(WbEvent::Paste("粘贴的文本".into()), &data);
        let [WbAction::EditEditor(editor)] = actions.as_slice() else {
            panic!("{actions:?}")
        };
        assert_eq!(editor.text(), "粘贴的文本");
        assert_eq!(editor.cursor(), "粘贴的文本".len());
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

    // ─── language ──────────────────────────────────────────────────────────

    /// The whole surface is the promise: the same data, the same size, one
    /// language and then the other. What must not change is under the chrome —
    /// the conversation, the ids, the numbers.
    #[test]
    fn the_chrome_switches_language_and_the_content_does_not() {
        let data = demo::chat();
        let mut workbench = Workbench::new();
        workbench.set_language(LanguagePreference::Zh);
        let zh = plain(&workbench.render(&data, 120, 38));
        assert!(zh.contains("待办"), "{zh}");
        assert!(zh.contains("新会话"), "{zh}");

        workbench.set_language(LanguagePreference::En);
        let en = plain(&workbench.render(&data, 120, 38));
        assert!(en.contains("To-do"), "{en}");
        assert!(en.contains("New Chat"), "{en}");
        assert!(!en.contains("待办"), "no Chinese tab left: {en}");

        // The demo's own session title is content, not chrome: it stays.
        assert!(zh.contains("修复 parser 的 off-by-one"));
        assert!(en.contains("修复 parser 的 off-by-one"));
    }

    /// `LanguagePreference::System` is not a third bundle: it resolves to one of
    /// the two, so the frame shows a language rather than a placeholder.
    #[test]
    fn the_system_preference_resolves_to_a_bundle() {
        let mut workbench = Workbench::new();
        workbench.set_language(LanguagePreference::System);
        assert_eq!(workbench.lang, LanguagePreference::System.resolve());
        assert_eq!(workbench.lang.code(), workbench.lang.code());
        // …and an explicit choice overrides whatever the locale said.
        workbench.set_language(LanguagePreference::En);
        assert_eq!(workbench.lang, Language::En);
    }

    /// `/lang` opens a picker whose rows are the wire values, and picking one
    /// hands the driver a `SetLanguage` to resolve and persist.
    #[test]
    fn the_language_picker_applies_a_preference() {
        let data = demo::chat();
        let mut workbench = Workbench::new();
        // Pinned, so the box's own words are the ones asserted below rather than
        // whatever this machine's locale resolves `system` to.
        workbench.set_language(LanguagePreference::Zh);

        // The palette advertises it, and a typed `/lang` opens the same box the
        // palette does.
        assert!(COMMANDS.iter().any(|(command, _)| *command == "/lang"));
        let actions = workbench.handle(WbEvent::Key("/".into()), &data);
        assert_eq!(actions, vec![WbAction::Redraw]);
        for ch in "lang".chars() {
            workbench.handle(WbEvent::Key(ch.to_string()), &data);
        }
        assert!(workbench
            .palette_selection()
            .is_some_and(|(command, _)| command == "/lang"));

        workbench.open_picker(PickerKind::Language, &data);
        let text = plain(&workbench.render(&data, 120, 38));
        assert!(text.contains("语言"), "the title is in the box: {text}");
        assert!(text.contains("跟随系统"), "{text}");
        assert!(text.contains("中文") && text.contains("English"), "{text}");

        // It opens on the row in force (中文, the second row); step to English
        // and apply it.
        assert_eq!(workbench.picker.index, 1);
        workbench.handle(WbEvent::Key("down".into()), &data);
        assert_eq!(
            workbench.handle(WbEvent::Key("enter".into()), &data),
            vec![WbAction::SetLanguage(LanguagePreference::En)]
        );
        assert_eq!(workbench.overlay, None, "applying closes the box");
    }

    #[test]
    fn accessibility_preferences_change_layout_motion_and_context_keys() {
        let mut data = demo::chat();
        let mut workbench = Workbench::new();
        workbench.render(&data, 120, 38);
        assert!(workbench
            .hits()
            .regions()
            .iter()
            .any(|(_, id)| id.starts_with("sidebar.tab.")));

        assert_eq!(
            workbench.handle(WbEvent::Key("f8".into()), &data),
            vec![WbAction::SetPreferences]
        );
        assert!(workbench.linear_reading);
        assert!(!workbench.motion_enabled());
        assert_eq!(workbench.effective_density(), preferences::Density::Focused);
        workbench.render(&data, 120, 38);
        assert!(
            !workbench
                .hits()
                .regions()
                .iter()
                .any(|(_, id)| id.starts_with("sidebar.tab.")),
            "linear mode paints no rail"
        );

        workbench
            .keys
            .bind(preferences::Action::LinearReading, "alt+l")
            .unwrap();
        workbench.handle(WbEvent::Key("alt+l".into()), &data);
        assert!(!workbench.linear_reading);
        workbench
            .keys
            .bind(preferences::Action::Queue, "alt+q")
            .unwrap();
        assert_eq!(
            workbench.handle(WbEvent::Key("alt+q".into()), &data),
            vec![WbAction::OpenQueue]
        );

        data.mouse_capture = false;
        let before = workbench.scroll.offset;
        assert!(workbench
            .handle(
                WbEvent::Mouse(mouse_at(mouse::MouseKind::WheelUp, 20)),
                &data
            )
            .is_empty());
        assert_eq!(workbench.scroll.offset, before);
    }

    #[test]
    fn reduced_motion_is_independent_and_linear_mode_restores_the_sidebar() {
        let mut workbench = Workbench::new();
        let data = demo::chat();
        workbench.render(&data, 120, 38);
        workbench.handle(WbEvent::Key("f9".into()), &data);
        assert!(workbench.reduced_motion && !workbench.motion_enabled());
        workbench.handle(WbEvent::Key("f8".into()), &data);
        workbench.handle(WbEvent::Key("ctrl+b".into()), &data);
        assert!(!workbench.linear_reading);
        assert!(workbench.sidebar_visible && workbench.sidebar_focus);
        assert!(
            workbench.reduced_motion,
            "sidebar navigation does not reset accessibility choices"
        );
    }

    /// The picker marks the preference in force, not the bundle it resolved to:
    /// a reader on `system` must not be told they chose Chinese.
    #[test]
    fn the_language_picker_ticks_the_preference_not_the_bundle() {
        let data = demo::chat();
        let mut workbench = Workbench::new();
        workbench.set_language(LanguagePreference::Zh);
        workbench.open_picker(PickerKind::Language, &data);
        let view = workbench.picker.view(&data, workbench.committed_theme());
        let current: Vec<&str> = view
            .rows
            .iter()
            .filter_map(|row| match row {
                PickerRow::Item {
                    key, current: true, ..
                } => Some(key.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(current, vec!["zh"]);
        // …and it opens on that row rather than on the first one.
        assert_eq!(workbench.picker.index, 1);
    }

    /// A point that no region claims, so a click there is the page itself.
    fn empty_point(workbench: &Workbench, w: u16, h: u16) -> Option<(u16, u16)> {
        (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .find(|&(x, y)| workbench.hits().hit(x, y).is_none())
    }

    /// An overlay is a modal surface, not a painting over live controls: a
    /// click on the box is the box's, and a click on the page beyond it closes
    /// it. Before this, the overlay registered nothing of its own, so a click
    /// on its padding reached the sidebar tab beneath (and, because the
    /// overlay's own handler did not know that id, silently did nothing at all).
    #[test]
    fn an_overlay_absorbs_clicks_on_its_own_surface_and_closes_on_the_page() {
        for overlay in [
            Overlay::Help,
            Overlay::Palette,
            Overlay::History,
            Overlay::Tree,
            Overlay::Fork,
            Overlay::Worktree,
            Overlay::Preferences,
        ] {
            let data = demo::chat();
            let mut wb = workbench();
            wb.overlay = Some(overlay);
            wb.render(&data, 120, 38);
            let Some((x, y)) = empty_point(&wb, 120, 38) else {
                continue;
            };
            wb.handle(
                WbEvent::Mouse(mouse::MouseEvent {
                    kind: mouse::MouseKind::Press,
                    x,
                    y,
                    shift: false,
                    alt: false,
                    ctrl: false,
                }),
                &data,
            );
            assert_eq!(
                wb.overlay, None,
                "{overlay:?}: clicking the page at ({x},{y}) must close it"
            );
        }

        let data = demo::chat();
        let mut wb = workbench();
        wb.overlay = Some(Overlay::Help);
        wb.render(&data, 120, 38);
        let surface = workbench_surface(&wb);
        wb.handle(
            WbEvent::Mouse(mouse::MouseEvent {
                kind: mouse::MouseKind::Press,
                x: surface.x + surface.w / 2,
                y: surface.y + surface.h / 2,
                shift: false,
                alt: false,
                ctrl: false,
            }),
            &data,
        );
        assert_eq!(
            wb.overlay,
            Some(Overlay::Help),
            "a click on the box itself must not dismiss it"
        );
    }

    /// `down` moves the highlight one *row down the screen*, not one record.
    ///
    /// A record is one line of data, but the box wraps by width: a real
    /// session title (title · workspace · id) does not fit on one row, so one
    /// index step moved the marker two rows and the entries looked skipped.
    /// The same shape had two more traps: the worktree list marked the
    /// *current* checkout with the arrow (so it had no visible selection at
    /// all), and the diff navigator drew its arrow from the file whose patch
    /// had loaded rather than from the cursor, so `down` looked dead until
    /// the reload came back.
    #[test]
    fn an_overlay_moves_the_highlight_one_visible_row_per_step() {
        fn row_of(wb: &Workbench, id: &str) -> Option<u16> {
            wb.hits()
                .regions()
                .iter()
                .find(|(_, known)| known == id)
                .map(|(rect, _)| rect.y)
        }

        // A realistic session list: long titles, long worktrees, long ids.
        let mut data = demo::chat();
        data.sessions = (0..8)
            .map(|index| state::SessionRow {
                id: format!("20261026-{index:06}-ebfc7c89507d4867b57705bd7789cf70"),
                title: format!("第 {index} 个会话 · 重构整理 TUI 的浮层布局与鼠标命中区域"),
                workspace: format!("D:/future-os/.worktrees/tui-overlay-layout-{index}"),
                ..SessionRow::default()
            })
            .collect();
        data.fork_points = (0..6)
            .map(|index| state::ForkPoint {
                id: format!("e{index}"),
                text: format!(
                    "第 {index} 条用户消息：把浮层的上下键改成一次一行，并把列表裁成一行"
                ),
            })
            .collect();
        data.worktrees = (0..6)
            .map(|index| state::WorktreeView {
                path: format!("D:/future-os/.worktrees/a-rather-long-worktree-name-{index}"),
                label: format!("feat/a-rather-long-branch-name-{index}"),
                description: format!("第 {index} 个工作区，描述也很长，长到需要换行"),
                current: index == 0,
            })
            .collect();
        data.diff = Some(state::DiffView {
            files: (0..6)
                .map(|index| diff_review::DiffFile {
                    path: format!("tui/src/workbench/a/deeply/nested/module_name_{index}.rs"),
                    status: "M".into(),
                })
                .collect(),
            ..state::DiffView::default()
        });

        for overlay in [
            Overlay::History,
            Overlay::Tree,
            Overlay::Fork,
            Overlay::Worktree,
            Overlay::Diff,
        ] {
            let mut wb = workbench();
            wb.overlay = Some(overlay);
            // Files first in the diff, so the marker is the file navigator.
            wb.diff_patch_focus = false;
            let mut painted = Vec::new();
            for _ in 0..4 {
                wb.handle(WbEvent::Key("down".into()), &data);
                let index = wb.workflow_index;
                let rows = wb.render(&data, 100, 30);
                let y = row_of(&wb, &format!("workflow.row.{index}"))
                    .or_else(|| row_of(&wb, &format!("diff.file.{index}")))
                    .unwrap_or_else(|| panic!("{overlay:?}: no row for record {index}"));
                let marked = crate::utils::strip_ansi_codes(&rows[y as usize]).contains('›');
                assert!(
                    marked,
                    "{overlay:?}: record {index} sits on row {y} without the marker:\n{}",
                    plain(&rows)
                );
                painted.push(y);
            }
            for pair in painted.windows(2) {
                assert_eq!(
                    pair[1] - pair[0],
                    1,
                    "{overlay:?}: one `down` moved the highlight {} rows (rows {painted:?})",
                    pair[1] - pair[0]
                );
            }
        }
    }

    fn workbench_surface(wb: &Workbench) -> crate::workbench::layout::Rect {
        wb.hits()
            .regions()
            .iter()
            .find(|(_, id)| id == "overlay.surface")
            .map(|(rect, _)| *rect)
            .expect("the overlay claims a surface")
    }

    /// The palette is a menu: an entry a pointer cannot press leaves the whole
    /// `ctrl+p` road open to the keyboard alone.
    #[test]
    fn the_command_palette_runs_the_entry_under_the_pointer() {
        let data = demo::chat();
        let mut wb = workbench();
        wb.overlay = Some(Overlay::Palette);
        for ch in "diff".chars() {
            wb.handle(WbEvent::Key(ch.to_string()), &data);
        }
        wb.render(&data, 120, 38);
        let expected = wb.palette_entries()[0].0.to_string();
        assert_eq!(
            click_region(&mut wb, &data, 120, 38, "palette.row.0"),
            vec![WbAction::Command(expected)]
        );
        assert_eq!(wb.overlay, None);
    }

    /// A queued prompt is a verb under the overlay too: the page's own click
    /// handler is never reached while a card is up.
    #[test]
    fn a_queued_prompt_is_clickable_inside_its_overlay() {
        let mut data = demo::chat();
        data.queue = vec![state::QueuedPrompt {
            run_id: "run-1".into(),
            text: "first".into(),
        }];
        let mut wb = workbench();
        wb.surface_command("/queue", &data);
        assert_eq!(
            click_region(&mut wb, &data, 120, 38, "queue.row.0"),
            vec![WbAction::RecallQueued("run-1".into())]
        );
    }

    /// A box is drawn where its hit regions say it is. The workflow overlays are
    /// painted at the left edge when the terminal is narrower than the box, and
    /// A box is drawn where its hit regions say it is. The workflow overlays are
    /// painted at the left edge when the terminal is narrower than the box, and
    /// centred when it is wider; a rect that assumed one of those on a frame
    /// that got the other was a whole margin out — invisible at 120 columns, and
    /// at 160 a click on a file name closed the card instead of opening it.
    #[test]
    fn a_wide_terminal_paints_the_review_box_where_its_hit_regions_are() {
        let mut data = demo::chat();
        data.diff = Some(state::DiffView {
            files: (0..4)
                .map(|index| diff_review::DiffFile {
                    path: format!("src/{index}.rs"),
                    status: "M".into(),
                })
                .collect(),
            ..state::DiffView::default()
        });
        for width in [120u16, 160, 200] {
            let mut wb = workbench();
            wb.overlay = Some(Overlay::Diff);
            let rows = wb.render(&data, width, 44);
            // Where the file name is actually drawn, not where a rect claims it
            // is: the painted cell is what a reader points at.
            let (y, x) = rows
                .iter()
                .enumerate()
                .find_map(|(y, row)| {
                    let painted = crate::utils::strip_ansi_codes(row);
                    painted
                        .find("src/1.rs")
                        .map(|byte| (y, painted[..byte].chars().count().saturating_sub(2)))
                })
                .unwrap_or_else(|| panic!("{width}: the file row is not painted"));
            assert_eq!(
                wb.handle(
                    WbEvent::Mouse(mouse::MouseEvent {
                        kind: mouse::MouseKind::Press,
                        x: x as u16,
                        y: y as u16,
                        shift: false,
                        alt: false,
                        ctrl: false,
                    }),
                    &data
                ),
                vec![WbAction::SelectDiff(1)],
                "{width}: clicking the painted file name at ({x},{y}) must open it"
            );
            assert_eq!(wb.overlay, Some(Overlay::Diff), "{width}: the card closed");
        }
    }

    /// A button the box had to clip is not clickable: a target for text nobody
    /// can see is a trap, and at 12 columns the approval row does not fit.
    #[test]
    fn clipped_approval_buttons_leave_no_target_outside_the_frame() {
        let mut data = demo::chat();
        data.approval = Some(state::ApprovalView {
            request_id: "req".into(),
            title: "run the tests".into(),
            rules: vec![state::AllowRule {
                path: "target/**".into(),
                access: "write".into(),
            }],
            ..state::ApprovalView::default()
        });
        for (w, h) in [(12u16, 6u16), (16, 8), (40, 12)] {
            let mut wb = workbench();
            wb.overlay = Some(Overlay::Approval);
            wb.render(&data, w, h);
            for (rect, id) in wb.hits().regions() {
                assert!(
                    rect.right() <= w && rect.bottom() <= h,
                    "{w}x{h}: {id} at ({},{}) {}x{} runs past the frame",
                    rect.x,
                    rect.y,
                    rect.w,
                    rect.h
                );
            }
        }
    }
}
