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
pub mod files;
pub mod git;
pub mod i18n;
pub mod image_view;
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
    Workspace,
    Model,
    Effort,
    /// The approval mode — the desktop composer's fourth chip. The old separate
    /// `Permission` field is gone: the desktop has no such control, so the TUI
    /// does not invent one.
    Approval,
}

impl LaunchField {
    fn next(self) -> Self {
        match self {
            Self::Prompt => Self::Workspace,
            Self::Workspace => Self::Model,
            Self::Model => Self::Effort,
            Self::Effort => Self::Approval,
            Self::Approval => Self::Prompt,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Prompt => Self::Approval,
            Self::Workspace => Self::Prompt,
            Self::Model => Self::Workspace,
            Self::Effort => Self::Model,
            Self::Approval => Self::Effort,
        }
    }

    /// The chip this field stands on, as the launchpad's hit map names it.
    pub fn chip_id(self) -> Option<&'static str> {
        match self {
            Self::Prompt => None,
            Self::Workspace => Some("launchpad.workspace"),
            Self::Model => Some("launchpad.model"),
            Self::Effort => Some("launchpad.effort"),
            Self::Approval => Some("launchpad.approval"),
        }
    }

    /// The verb that cycles this chip — the same one a click on it runs, so the
    /// key and the pointer cannot drift apart.
    fn command(self) -> Option<&'static str> {
        self.chip_id().map(|id| match id {
            "launchpad.workspace" => "/launchpad.workspace",
            "launchpad.model" => "/launchpad.model",
            "launchpad.effort" => "/launchpad.effort",
            _ => "/launchpad.approval",
        })
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
    /// The file preview: a picture (zoomable) or a document's text.
    FilePreview,
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
    /// The approval-mode menu chose a tier: apply it to the session, or stage it
    /// while nothing is attached (`manual` | `sandbox` | `auto` | `off`).
    SetApproval(String),
    /// The workspace menu chose a directory — an existing workspace or one just
    /// typed into its search line. The app moves the session there, or stages it
    /// for the session the next message creates.
    SelectWorkspace(String),
    /// The 文件 panel opened or closed a directory: the app re-reads it and the
    /// next frame paints the new shape.
    ToggleFolder(String),
    /// The 文件 panel asked for a file: the app reads it (an image decoded, a
    /// document's text) and fills `preview`; the box then paints it.
    PreviewFile(String),
    /// Re-read the tree, keeping the open directories.
    RefreshFiles,
    /// Read the clipboard: an image becomes a draft attachment, text is pasted.
    PasteClipboard,
    /// Show the newest image already attached to the draft.
    PreviewAttachment,
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
    /// Move the rail's cursor `delta` conversations and open the one it lands
    /// on. Resolved by the driver *after* the whole input burst, so a held
    /// `shift+←`/`shift+→` pays for one switch instead of one per repeat.
    SessionStep(i32),
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
    /// The reader pinned (or unpinned) a workspace in the rail; the app records
    /// it and rebuilds the groups.
    SetWorkspacePinned {
        path: String,
        pinned: bool,
    },
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

/// How far the preview box will zoom in. Six steps over the fitted size is
/// already past "one terminal cell is one image pixel" on a screenshot; more
/// would only show the sampling grid.
pub const MAX_PREVIEW_ZOOM: u32 = 6;

/// The pseudo-keys the preview box answers beyond its literal chords.
///
/// The pointer's wheel and `+`/`-` are one intent ("further in") spelled three
/// ways; naming it once keeps the key handler and the wheel handler from
/// drifting, and keeps the box's own vocabulary out of the i18n table — these
/// strings are never drawn.
const ZOOM_IN: &str = "preview.zoomIn";
const ZOOM_OUT: &str = "preview.zoomOut";

/// The surface's own state: where the panels and the pointers are.
///
/// Deliberately *not* the data — the conversation, the todos and the usage
/// figures live in [`WorkbenchData`], which the app owns and passes in. Keeping
/// them apart is what lets the surface be tested with fixtures and later fed by
/// a real session without either side knowing the other's shape.
/// A picture placed on screen by the terminal's image protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaintedImage {
    /// The source bytes it was transmitted from (`screen::ImageRequest::source`).
    pub source: u64,
    /// The id it was transmitted under, which is how it is placed and deleted.
    pub id: u32,
}

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
    /// Whether the rail (not the composer) takes `up`/`down`/`enter`.
    pub sidebar_focus: bool,
    /// The row the rail's own cursor stands on, as an index into
    /// [`panels::rows`] for the current panel. Only a conversation row can hold
    /// it (see [`Workbench::select_row`]).
    pub sidebar_index: usize,
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
    pub panel_scroll: [usize; PanelId::ALL.len()],
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
    /// How many list rows the last frame showed. Only the renderer knows,
    /// and `pageUp`/`pageDown` need it to move the cursor by a page.
    pub list_page: usize,
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
    /// The preview box's own state: how far in a picture is zoomed, which cell
    /// of it the window starts at, and how far a document has scrolled. Kept
    /// here rather than in `WorkbenchData` because it is what the *reader* is
    /// doing, not what is on screen.
    pub preview_zoom: u32,
    pub preview_pan: (i32, i32),
    pub preview_scroll: usize,
    /// The workspace headings the reader folded shut, by path (see
    /// [`panels::Folded`]). A rail with a long conversation list is scrolled a
    /// group at a time once its workspaces are folded.
    pub folded_workspaces: panels::Folded,
    /// The picture the last frame wanted on screen, if any.
    ///
    /// A terminal picture is not text: the frame is a list of rows, and a
    /// bitmap is placed by the terminal at a cell of its own. So the preview box
    /// *asks* for one and the driver paints it after the frame it belongs to
    /// (`run::paint_image`) — the one thing a text frame cannot express.
    pub image_request: Option<screen::ImageRequest>,
    /// The picture actually placed on screen: its terminal id, and the source
    /// bytes it was transmitted from. `None` means nothing is placed.
    pub painted_image: Option<PaintedImage>,
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
            panel: PanelId::Recent,
            sidebar_focus: false,
            sidebar_index: 0,
            // Visible until the driver applies the stored preference: a first
            // start is hidden (see `run::layout_preferences`), and a bare
            // surface still paints its rail so tests can address it without a
            // settings file.
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
            panel_scroll: [0; PanelId::ALL.len()],
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
            list_page: 10,
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
            preview_zoom: 1,
            preview_pan: (0, 0),
            preview_scroll: 0,
            folded_workspaces: panels::Folded::new(),
            image_request: None,
            painted_image: None,
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
        // Each frame asks again: a frame that paints no picture asks for none,
        // and the driver takes that as "remove the one on screen".
        self.image_request = None;
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

    /// `/` palette rows that match the current query: the surface's verbs first,
    /// then the skills the agent has installed.
    ///
    /// One list, not two menus: a skill is written `/name` in the draft exactly
    /// like a command is, so a reader who opens `/` and types `web` should see
    /// both the verb and the skill that spell that way. The skills come last
    /// because a slash menu is opened *for* a command far more often than for a
    /// skill, and their rows are marked as skills so the two cannot be confused
    /// (a skill's `<name>` is a prompt token the agent expands; a command never
    /// reaches the model).
    pub fn palette_rows(&self, data: &WorkbenchData) -> Vec<PaletteRow> {
        let needle = self.palette_query.trim_start_matches('/').to_lowercase();
        let matches = |values: &[&str]| {
            needle.is_empty()
                || values
                    .iter()
                    .any(|value| value.to_lowercase().contains(&needle))
        };
        let mut rows: Vec<PaletteRow> = COMMANDS
            .iter()
            .filter(|(command, key)| matches(&[command, t(self.lang, *key)]))
            .map(|(command, key)| PaletteRow::Command {
                command,
                description: t(self.lang, *key),
            })
            .collect();
        rows.extend(
            data.skills
                .iter()
                .filter(|skill| matches(&[skill.name.as_str(), skill.description.as_str()]))
                .take(PALETTE_SKILL_LIMIT)
                .map(|skill| PaletteRow::Skill {
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                }),
        );
        rows
    }

    /// The highlighted palette row, if the palette is open.
    pub fn palette_selection(&self, data: &WorkbenchData) -> Option<PaletteRow> {
        let rows = self.palette_rows(data);
        rows.get(self.palette_index.min(rows.len().saturating_sub(1)))
            .cloned()
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
        // The preview box owns its own keys — zoom, pan, scroll — and hands
        // everything else back: `ctrl+p` still opens the palette over it, the
        // same way `esc` closes it.
        if self.overlay == Some(Overlay::FilePreview) {
            if let Some(actions) = self.preview_key(key, data) {
                return actions;
            }
        }
        // Leaving is never an overlay's to swallow. Every other key belongs to
        // whoever owns the screen, but a modal that eats `ctrl+c` leaves the
        // user with no way out except a signal — and a signalled TUI is exactly
        // the exit that cannot restore the terminal properly (alt screen and
        // keyboard protocol left behind).
        if self.keys.action(key) == Some(preferences::Action::Quit) {
            return vec![WbAction::Quit];
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
            // The four choosers reachable without leaving the keyboard. Each
            // opens its picker through the same command a typed line would run
            // (`PickerKind::from_command`), so the key, the palette and the
            // draft share one implementation — and a chooser is what these keys
            // get, not a silent cycle: a cycle has no `esc`.
            Some(preferences::Action::Model) => return vec![WbAction::Command("/model".into())],
            Some(preferences::Action::Thinking) => {
                return vec![WbAction::Command("/thinking".into())]
            }
            Some(preferences::Action::Approval) => return vec![WbAction::Command("/mode".into())],
            Some(preferences::Action::Workspaces) => {
                return vec![WbAction::Command("/workspace".into())]
            }
            // The rail's two tabs, from anywhere: the same intent as a click on
            // the tab strip, so the key and the pointer land in the same place
            // (the conversation on screen gets the cursor).
            Some(preferences::Action::PanelRecent) => {
                return self.focus_panel_action(PanelId::Recent, data)
            }
            Some(preferences::Action::PanelWorkspaces) => {
                return self.focus_panel_action(PanelId::Workspaces, data)
            }
            Some(preferences::Action::PanelFiles) => {
                return self.focus_panel_action(PanelId::Files, data)
            }
            // The clipboard: an image becomes an attachment, text is pasted.
            Some(preferences::Action::PasteImage) => return vec![WbAction::PasteClipboard],
            Some(preferences::Action::PreviewAttachment) => {
                return vec![WbAction::PreviewAttachment]
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
            // One conversation either way, with the transcript following it: a
            // rail whose highlight moved while the chat stayed put would name a
            // conversation the screen is not showing. The *switch* is deferred
            // to the driver (`WbAction::SessionStep`), which applies just the
            // net step once the whole input burst is read: a held shift+arrow
            // used to queue one round trip and one transcript load per repeat.
            Some(preferences::Action::PrevSession) => return vec![WbAction::SessionStep(-1)],
            Some(preferences::Action::NextSession) => return vec![WbAction::SessionStep(1)],
            _ => {}
        }
        match key {
            "ctrl+q" => vec![WbAction::Quit],
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
                    if self.sidebar_focus {
                        // Arriving in the rail lands on the conversation that
                        // is on screen, not on row 0 of an unrelated list.
                        let row = self.current_row(data);
                        self.select_row(row, data);
                    }
                }
                vec![WbAction::Redraw]
            }
            "shift+tab" => {
                if data.on_launchpad() {
                    self.launch_field = self.launch_field.previous();
                }
                vec![WbAction::Redraw]
            }
            // On the launchpad a focused chip answers `enter`: it cycles the
            // choice it names, which is what clicking it does. Without a chip,
            // `enter` is the submission — the same key, two intents decided by
            // where `tab` left the cursor.
            "enter" if data.on_launchpad() && self.launch_field != LaunchField::Prompt => self
                .launch_field
                .command()
                .map(|command| vec![WbAction::Command(command.into())])
                .unwrap_or_default(),
            // Back to the prompt, so a reader who tabbed into the chips is not
            // stuck in them with `enter` unable to send.
            "escape" if data.on_launchpad() && self.launch_field != LaunchField::Prompt => {
                self.launch_field = LaunchField::Prompt;
                vec![WbAction::Redraw]
            }
            "enter" if self.sidebar_focus => self.open_selected(data),
            "enter" => self.submit_draft(data),
            "up" if self.sidebar_focus => self.move_selection(-1, data),
            "down" if self.sidebar_focus => self.move_selection(1, data),
            // The rail's two tabs, from the rail itself: a focused panel answers
            // left/right with its neighbour the way a tab strip does with a
            // click. The cursor lands on the conversation on screen, exactly as
            // the click on a tab leaves it.
            //
            // 文件 is the exception: there, left/right walk the tree (close a
            // directory, open it) the way every other tree in every other
            // editor does. The tabs are still one keystroke away (`ctrl+1`…
            // `ctrl+3`, or a click on the strip).
            "left" if self.sidebar_focus && self.panel == PanelId::Files => self.file_close(data),
            "right" if self.sidebar_focus && self.panel == PanelId::Files => self.file_open(data),
            "left" if self.sidebar_focus => self.switch_panel(-1, data),
            "right" if self.sidebar_focus => self.switch_panel(1, data),
            // Pin the workspace the selected conversation belongs to, so the
            // 置顶 section is reachable with the keyboard. Only a *focused* rail
            // takes it: elsewhere `p` is a letter in the draft.
            "p" if self.sidebar_focus => self.toggle_pin_selected(data),
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
                    // Typing takes the cursor back to the prompt: a chip stays
                    // focused only while the keys are still naming choices.
                    if data.on_launchpad() {
                        self.launch_field = LaunchField::Prompt;
                    }
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

    /// Put the rail's cursor on the conversation on screen.
    ///
    /// The app calls this after a switch it did not make through the rail (the
    /// `ctrl+r` picker, a click on the launchpad's recents): the transcript moved,
    /// so the highlighted row has to move with it, or the rail names a
    /// conversation the screen is not showing.
    pub fn follow_current(&mut self, data: &WorkbenchData) {
        let row = self.current_row(data);
        self.select_row(row, data);
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

    // ─── the rail's own cursor ───────────────────────────────────────

    /// The panel's rows, as the renderer paints them.
    fn panel_rows(&self, data: &WorkbenchData) -> Vec<panels::Row> {
        panels::rows_with(self.panel, data, &self.folded_workspaces)
    }

    /// Fold a workspace's conversations away, or unfold them.
    ///
    /// The cursor stays on the heading it acted on, which is what lets `enter`
    /// be pressed twice: the second one opens it again instead of the cursor
    /// having moved to a conversation it never asked for.
    fn toggle_workspace(&mut self, at: usize, data: &WorkbenchData) -> Vec<WbAction> {
        let Some(workspace) = data.workspaces.get(at) else {
            return Vec::new();
        };
        let path = workspace.path.clone();
        if !self.folded_workspaces.remove(&path) {
            self.folded_workspaces.insert(path);
        }
        vec![WbAction::Redraw]
    }

    /// The row the conversation on screen stands on, if the panel lists it.
    fn current_row(&self, data: &WorkbenchData) -> Option<usize> {
        let rows = self.panel_rows(data);
        let current = data.sessions.iter().position(|row| row.current)?;
        rows.iter().position(|row| row.session() == Some(current))
    }

    /// Put the rail's cursor on `row`, if that row holds something the cursor
    /// can stand on (a conversation, or a file in the tree).
    fn select_row(&mut self, row: Option<usize>, data: &WorkbenchData) {
        let rows = self.panel_rows(data);
        if let Some(row) = row.filter(|row| rows.get(*row).is_some_and(|row| row.selectable())) {
            self.sidebar_index = row;
        } else if let Some(first) = panels::first_session(&rows, 0) {
            self.sidebar_index = first;
        } else {
            self.sidebar_index = 0;
        }
    }

    /// Move the cursor one row up or down, keeping the window on it.
    ///
    /// In 最近 a row is a conversation, so the cursor skips the headings
    /// (`step_session`); in 文件 every row is an entry of the tree, and in 工作区
    /// a workspace heading is a row the cursor can act on (it folds), so both
    /// walk every selectable row (`step_row`).
    fn move_selection(&mut self, delta: i32, data: &WorkbenchData) -> Vec<WbAction> {
        let rows = self.panel_rows(data);
        let next = if self.panel == PanelId::Recent {
            panels::step_session(&rows, self.sidebar_index, delta)
        } else {
            panels::step_row(&rows, self.sidebar_index, delta)
        };
        if let Some(next) = next {
            self.sidebar_index = next;
            let index = self.panel.index();
            // The renderer also scrolls to the cursor; this keeps the stored
            // offset honest for the frame that follows.
            if self.sidebar_index < self.panel_scroll[index] {
                self.panel_scroll[index] = self.sidebar_index;
            }
        }
        vec![WbAction::Redraw]
    }

    /// Open what the cursor stands on: a conversation, a file's preview, or a
    /// directory the reader wants to see inside.
    fn open_selected(&mut self, data: &WorkbenchData) -> Vec<WbAction> {
        let rows = self.panel_rows(data);
        let Some(row) = rows.get(self.sidebar_index) else {
            return Vec::new();
        };
        if let panels::Row::Workspace(at) = row {
            return self.toggle_workspace(*at, data);
        }
        if let Some(index) = row.session() {
            return data
                .sessions
                .get(index)
                .map(|session| vec![WbAction::SwitchSession(session.id.clone())])
                .unwrap_or_default();
        }
        let Some(index) = row.file() else {
            return Vec::new();
        };
        let Some(node) = data.tree.get(index) else {
            return Vec::new();
        };
        let path = node.path.to_string_lossy().to_string();
        if node.is_dir() {
            vec![WbAction::ToggleFolder(path)]
        } else {
            vec![WbAction::PreviewFile(path)]
        }
    }

    /// `→` in the tree: open the directory, or step into its first child.
    ///
    /// On an already-open directory the key does what the reader is pointing
    /// at — it moves the cursor into the directory rather than collapsing it,
    /// which is the one thing `→` means in every file tree.
    fn file_open(&mut self, data: &WorkbenchData) -> Vec<WbAction> {
        let rows = self.panel_rows(data);
        let Some(index) = rows.get(self.sidebar_index).and_then(|row| row.file()) else {
            return Vec::new();
        };
        let Some(node) = data.tree.get(index) else {
            return Vec::new();
        };
        if !node.is_dir() {
            // A file is not a folder: `→` on it is the preview, the same as `enter`.
            return vec![WbAction::PreviewFile(
                node.path.to_string_lossy().to_string(),
            )];
        }
        if !node.expanded {
            return vec![WbAction::ToggleFolder(
                node.path.to_string_lossy().to_string(),
            )];
        }
        // Open already: step onto the first child, if it is on screen.
        if data
            .tree
            .get(index + 1)
            .is_some_and(|child| child.depth > node.depth)
        {
            self.sidebar_index = index + 1;
        }
        vec![WbAction::Redraw]
    }

    /// `←` in the tree: close the directory, or jump to the one that holds it.
    fn file_close(&mut self, data: &WorkbenchData) -> Vec<WbAction> {
        let rows = self.panel_rows(data);
        let Some(index) = rows.get(self.sidebar_index).and_then(|row| row.file()) else {
            return Vec::new();
        };
        let Some(node) = data.tree.get(index) else {
            return Vec::new();
        };
        if node.is_dir() && node.expanded {
            return vec![WbAction::ToggleFolder(
                node.path.to_string_lossy().to_string(),
            )];
        }
        // Not open: up to the directory this row sits in. `depth` is the flat
        // tree's own record of nesting, so the parent is the first row above
        // with a smaller depth.
        let depth = node.depth;
        if depth == 0 {
            return Vec::new();
        }
        if let Some(parent) = (0..index)
            .rev()
            .find(|above| data.tree.get(*above).is_some_and(|row| row.depth < depth))
        {
            self.sidebar_index = parent;
        }
        vec![WbAction::Redraw]
    }

    /// The workspace the conversation under the cursor belongs to.
    fn selected_workspace<'a>(
        &'a self,
        data: &'a WorkbenchData,
    ) -> Option<&'a state::WorkspaceGroup> {
        let rows = self.panel_rows(data);
        let session = rows.get(self.sidebar_index)?.session()?;
        let id = &data.sessions.get(session)?.id;
        data.workspaces
            .iter()
            .find(|workspace| workspace.session_ids.contains(id))
    }

    /// Pin or unpin the workspace the cursor's conversation belongs to.
    fn toggle_pin_selected(&mut self, data: &WorkbenchData) -> Vec<WbAction> {
        match self.selected_workspace(data) {
            Some(workspace) => vec![WbAction::SetWorkspacePinned {
                path: workspace.path.clone(),
                pinned: !workspace.pinned,
            }],
            None => vec![WbAction::Notice(
                t(self.lang, Key::NoticePinUnavailable).to_string(),
            )],
        }
    }

    /// Switch to the previous/next conversation as the *panel* lists it.
    ///
    /// The rail's order is the reader's order, so this walks the rows rather
    /// than the raw session list: under 工作区 it follows the sections, in 最近 it
    /// follows refresh time. The cursor is left where the switch went, so a
    /// second call moves a second conversation.
    ///
    /// The driver calls this once per input burst with the *net* step a held
    /// `shift+←`/`shift+→` accumulated: every step still moves the cursor, but
    /// the conversation is switched once — a switch is two round trips and a
    /// transcript load, and paying that per key repeat is what made the rail
    /// feel stuck.
    pub(crate) fn step_session(&mut self, delta: i32, data: &WorkbenchData) -> Vec<WbAction> {
        if delta == 0 {
            return Vec::new();
        }
        let rows = self.panel_rows(data);
        let direction = if delta < 0 { -1 } else { 1 };
        // The cursor may stand on nothing yet (a fresh rail): start from the
        // conversation on screen so the first press moves *one* step.
        let mut from = match self.current_row(data) {
            Some(current)
                if rows
                    .get(self.sidebar_index)
                    .is_none_or(|row| row.session().is_none()) =>
            {
                current
            }
            _ => self.sidebar_index,
        };
        let mut moved = false;
        for _ in 0..delta.unsigned_abs() {
            let Some(next) = panels::step_session(&rows, from, direction) else {
                // The list ends here: further steps have nowhere to go, and the
                // one conversation we did move to is still the right answer.
                break;
            };
            from = next;
            moved = true;
        }
        if !moved {
            return Vec::new();
        }
        self.sidebar_index = from;
        let index = self.panel.index();
        if self.sidebar_index < self.panel_scroll[index] {
            self.panel_scroll[index] = self.sidebar_index;
        }
        self.open_selected(data)
    }

    /// The preview box's keys: zoom a picture, pan it, scroll a document.
    ///
    /// Returns `None` for a key it does not own, so the caller can fall through
    /// to the global map — `ctrl+p` still opens the palette over a preview, the
    /// same way it does over every other box.
    fn preview_key(&mut self, key: &str, data: &WorkbenchData) -> Option<Vec<WbAction>> {
        let is_image = matches!(data.preview, Some(state::PreviewView::Image { .. }));
        match key {
            "escape" | "q" => {
                self.overlay = None;
                Some(vec![WbAction::Redraw])
            }
            "+" | "=" | ZOOM_IN => {
                if is_image {
                    self.preview_zoom = (self.preview_zoom + 1).min(MAX_PREVIEW_ZOOM);
                } else {
                    // A document has nothing to zoom: "further in" is up, which
                    // is also what a wheel-up means here.
                    self.preview_scroll = self.preview_scroll.saturating_sub(3);
                }
                Some(vec![WbAction::Redraw])
            }
            "-" | "_" | ZOOM_OUT => {
                if is_image {
                    self.preview_zoom = self.preview_zoom.saturating_sub(1).max(1);
                    self.preview_pan = (0, 0);
                } else {
                    self.preview_scroll = self.preview_scroll.saturating_add(3);
                }
                Some(vec![WbAction::Redraw])
            }
            "0" => {
                self.preview_zoom = 1;
                self.preview_pan = (0, 0);
                self.preview_scroll = 0;
                Some(vec![WbAction::Redraw])
            }
            // A zoomed picture pans; a document scrolls. One key, two readings
            // of "further into the thing", decided by what is in the box.
            "left" | "shift+left" => Some(self.pan_preview(-2, 0)),
            "right" | "shift+right" => Some(self.pan_preview(2, 0)),
            "up" => Some(self.pan_preview(0, -1)),
            "down" => Some(self.pan_preview(0, 1)),
            "pageUp" => Some(self.page_preview(-1)),
            "pageDown" => Some(self.page_preview(1)),
            _ => None,
        }
    }

    /// Pan by cells, or scroll a document by rows.
    fn pan_preview(&mut self, dx: i32, dy: i32) -> Vec<WbAction> {
        if self.preview_scroll > 0 || self.preview_zoom == 1 {
            // Not zoomed in (or the box holds text): `↑`/`↓` are scroll keys.
            if self.preview_scroll > 0 {
                let next = (self.preview_scroll as i32 + dy).max(0) as usize;
                self.preview_scroll = next;
            } else if dy != 0 {
                self.preview_scroll = (self.preview_scroll as i32 + dy * 3).max(0) as usize;
            }
        }
        if dx != 0 || self.preview_zoom > 1 {
            self.preview_pan = (self.preview_pan.0 + dx, self.preview_pan.1 + dy);
        }
        vec![WbAction::Redraw]
    }

    /// A window of a document at a time.
    fn page_preview(&mut self, pages: i32) -> Vec<WbAction> {
        let step = (self.list_page.max(1) as i32) * pages;
        self.preview_scroll = (self.preview_scroll as i32 + step).max(0) as usize;
        vec![WbAction::Redraw]
    }

    /// Reset the box's view state: called whenever a new file opens, so a zoom
    /// chosen for one picture does not arrive on the next one.
    pub fn reset_preview(&mut self) {
        self.preview_zoom = 1;
        self.preview_pan = (0, 0);
        self.preview_scroll = 0;
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
                "pageUp" => {
                    self.queue_index = self.queue_index.saturating_sub(self.list_page.max(1));
                    vec![WbAction::Redraw]
                }
                "pageDown" => {
                    self.queue_index = (self.queue_index + self.list_page.max(1))
                        .min(data.queue.len().saturating_sub(1));
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
                "pageUp" => {
                    self.preferences_index =
                        self.preferences_index.saturating_sub(self.list_page.max(1));
                    vec![WbAction::Redraw]
                }
                "pageDown" => {
                    self.preferences_index = (self.preferences_index + self.list_page.max(1))
                        .min(preferences::PREFERENCE_ROWS.len() - 1);
                    vec![WbAction::Redraw]
                }
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
                if !self.awaiting_keybinding {
                    match key {
                        "pageUp" => {
                            self.keybinding_index =
                                self.keybinding_index.saturating_sub(self.list_page.max(1));
                            return vec![WbAction::Redraw];
                        }
                        "pageDown" => {
                            self.keybinding_index = (self.keybinding_index + self.list_page.max(1))
                                .min(preferences::ACTIONS.len() - 1);
                            return vec![WbAction::Redraw];
                        }
                        _ => {}
                    }
                }
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
            // Everything the preview owns was taken by `preview_key` before
            // this point; what is left is a key the box does not use.
            Overlay::FilePreview => Vec::new(),
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
                    let chosen = self.palette_selection(data);
                    self.overlay = None;
                    match chosen {
                        // A skill is a prompt token, not a verb the surface
                        // answers: running it here would be a command the agent
                        // never sees, so it fills the draft instead and the
                        // reader's next `enter` sends it.
                        Some(row) if row.is_skill() => vec![WbAction::EditInput {
                            text: row.fill(),
                            cursor: row.fill().len(),
                        }],
                        Some(row) => vec![WbAction::Command(row.command())],
                        None => vec![WbAction::Redraw],
                    }
                }
                // `tab` completes into the draft without leaving the palette's
                // meaning behind: the reader can then add arguments, exactly
                // like the desktop's composer.
                "tab" => match self.palette_selection(data) {
                    Some(row) => {
                        let fill = row.fill();
                        self.overlay = None;
                        vec![WbAction::EditInput {
                            text: fill.clone(),
                            cursor: fill.len(),
                        }]
                    }
                    None => Vec::new(),
                },
                "up" => {
                    self.palette_index = self.palette_index.saturating_sub(1);
                    vec![WbAction::Redraw]
                }
                "down" => {
                    let count = self.palette_rows(data).len();
                    if self.palette_index + 1 < count {
                        self.palette_index += 1;
                    }
                    vec![WbAction::Redraw]
                }
                "pageUp" => {
                    self.palette_index = self.palette_index.saturating_sub(self.list_page.max(1));
                    vec![WbAction::Redraw]
                }
                "pageDown" => {
                    let count = self.palette_rows(data).len();
                    self.palette_index =
                        (self.palette_index + self.list_page.max(1)).min(count.saturating_sub(1));
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
            // `space` types into history's filter, so only the two named page
            // keys page there; everywhere else it is the same verb. A detail
            // view (an approval, a patch) scrolls; every list moves its cursor
            // — the diff is both, so whichever pane has the keys decides.
            "pageUp" | "pageDown" | "space" if key != "space" || overlay != Overlay::History => {
                let page = self.list_page.max(1);
                let back = key == "pageUp";
                // A detail view (an approval, a patch) scrolls; every list moves
                // its cursor. The diff is both, so the pane with the keys decides.
                let scrolls = overlay == Overlay::Approval
                    || (overlay == Overlay::Diff && self.diff_patch_focus);
                if scrolls {
                    if back {
                        self.overlay_scroll = self.overlay_scroll.saturating_sub(page);
                    } else {
                        self.overlay_scroll += page;
                    }
                } else if back {
                    self.workflow_index = self.workflow_index.saturating_sub(page);
                } else {
                    self.workflow_index = (self.workflow_index + page).min(count.saturating_sub(1));
                }
            }
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
            // Staged while nothing is attached, applied once a session exists:
            // the agent answers both setters with "session not found" otherwise.
            (PickerKind::Thinking, Some(PickerRow::Item { key, .. })) => {
                if launchpad {
                    vec![WbAction::SetPendingThinking(key)]
                } else {
                    vec![WbAction::SetThinking(key)]
                }
            }
            (PickerKind::Approval, Some(PickerRow::Item { key, .. })) => {
                vec![WbAction::SetApproval(key)]
            }
            // The first row of the workspace menu is a directory typed into the
            // search line; an existing workspace is the same action, so "switch"
            // and "create" cannot take two different roads.
            (PickerKind::Workspace, Some(PickerRow::Item { key, .. })) => {
                vec![WbAction::SelectWorkspace(key)]
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
            // Over a picture the wheel is the zoom (a pointer has no `+`), and
            // over a document it is the scroll.
            if overlay == Overlay::FilePreview {
                let key = if event.kind == mouse::MouseKind::WheelUp {
                    ZOOM_IN
                } else {
                    ZOOM_OUT
                };
                return self
                    .preview_key(key, data)
                    .unwrap_or_else(|| vec![WbAction::Redraw]);
            }
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
                    // is what `enter` does on the highlighted one. A skill fills
                    // the draft for the same reason `enter` does.
                    self.palette_index = index;
                    let chosen = self.palette_selection(data);
                    self.overlay = None;
                    return match chosen {
                        Some(row) if row.is_skill() => {
                            let fill = row.fill();
                            vec![WbAction::EditInput {
                                text: fill.clone(),
                                cursor: fill.len(),
                            }]
                        }
                        Some(row) => vec![WbAction::Command(row.command())],
                        None => vec![WbAction::Redraw],
                    };
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
        // The rail's collapse/expand button — one control, two states: the
        // arrow inside the rail hides it, the arrow it leaves behind brings it
        // back. Unlike `ctrl+b` (which cycles show → focus → hide), a click only
        // ever means "the other state": a pointer has no third intent.
        if id == "sidebar.toggle" {
            self.sidebar_visible = !self.sidebar_visible;
            if self.sidebar_visible {
                self.linear_reading = false;
                self.sidebar_focus = true;
                let row = self.current_row(data);
                self.select_row(row, data);
            } else {
                self.sidebar_focus = false;
            }
            return vec![WbAction::SetPreferences];
        }
        if let Some(panel) = id.strip_prefix("sidebar.tab.").and_then(PanelId::parse) {
            self.panel = panel;
            // Clicking the sidebar means working in it; the same click that
            // switches the panel therefore takes the keys, and the cursor lands
            // on the conversation on screen rather than on whatever index the
            // other panel held.
            self.sidebar_focus = true;
            let row = self.current_row(data);
            self.select_row(row, data);
            return vec![WbAction::SwitchPanel(panel)];
        }
        if let Some(index) = id
            .strip_prefix("sidebar.row.")
            .and_then(|rest| rest.parse::<usize>().ok())
        {
            self.sidebar_focus = true;
            let rows = self.panel_rows(data);
            return match rows.get(index) {
                // A conversation opens on one click: the row was aimed at.
                Some(panels::Row::Session(session)) => {
                    self.sidebar_index = index;
                    data.sessions
                        .get(*session)
                        .map(|session| vec![WbAction::SwitchSession(session.id.clone())])
                        .unwrap_or_default()
                }
                // A tree row is aimed at too: a file previews, a directory opens
                // — the same two verbs `enter` runs on it.
                Some(panels::Row::File(_)) => {
                    self.sidebar_index = index;
                    self.open_selected(data)
                }
                // A workspace heading folds what is under it away, and opens it
                // again on the next click: one click on a group is the quickest
                // way to make a long rail scannable.
                Some(panels::Row::Workspace(at)) => {
                    self.sidebar_index = index;
                    self.toggle_workspace(*at, data)
                }
                // A heading is not a verb: the click takes the keys and stops.
                Some(_) => {
                    self.sidebar_index = index;
                    vec![WbAction::Redraw]
                }
                None => vec![WbAction::Redraw],
            };
        }
        // A launchpad chip is a pointer target: the click runs the same verb
        // `tab` + `enter` does on that chip. The block teaches no key of its
        // own, so there is nothing else here to aim at.
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
    fn focus_panel(&mut self, panel: PanelId, data: &WorkbenchData) {
        self.panel = panel;
        self.sidebar_focus = true;
        // The cursor is an index into *this* panel's rows: carry it to the
        // conversation on screen instead of leaving it on whatever the other
        // tab's row at that index happened to be.
        let row = self.current_row(data);
        self.select_row(row, data);
        if layout::sidebar_fits(self.last_frame.0) {
            self.sidebar_visible = true;
        } else {
            self.overlay = Some(Overlay::PanelFull);
        }
    }

    /// The same intent as a click on the rail's tab strip (and as the verb
    /// `/recent`), returned as an action so the app records which panel the
    /// reader chose.
    fn focus_panel_action(&mut self, panel: PanelId, data: &WorkbenchData) -> Vec<WbAction> {
        self.focus_panel(panel, data);
        vec![WbAction::SwitchPanel(panel)]
    }

    /// The rail's neighbouring tab: `←`/`→` while the rail has the keys, which
    /// is the keyboard spelling of a click on the tab strip.
    fn switch_panel(&mut self, delta: i32, data: &WorkbenchData) -> Vec<WbAction> {
        let panel = if delta < 0 {
            self.panel.previous()
        } else {
            self.panel.next()
        };
        self.panel = panel;
        self.sidebar_focus = true;
        // The cursor is an index into *this* panel's rows: carry it to the
        // conversation on screen rather than leaving it on whatever the other
        // tab's row at that index happened to be.
        let row = self.current_row(data);
        self.select_row(row, data);
        vec![WbAction::SwitchPanel(panel)]
    }

    /// Carry out a `/…` verb that belongs to the *surface* rather than to the
    /// agent: naming a panel, or saying where a workspace comes from.
    ///
    /// The palette advertises these ([`COMMANDS`]), so a verb it offers has to
    /// answer *something*. The panels a reader can name are the rail's two, so
    /// only those two focus something: the old `/usage`, `/skills` and `/todo`
    /// names went with the panels they opened (the usage figures are on the
    /// status bar, the skills are the palette's own).
    pub fn surface_command(&mut self, command: &str, data: &WorkbenchData) -> VerbOutcome {
        let name = command.trim_start_matches('/').trim();
        match name {
            "approval" | "diff" | "history" | "tree" | "fork" | "worktree" => VerbOutcome::Agent,
            "queue" => {
                self.overlay = Some(Overlay::Queue);
                self.queue_index = self.queue_index.min(data.queue.len().saturating_sub(1));
                VerbOutcome::Done { notice: None }
            }
            "recent" => {
                self.focus_panel(PanelId::Recent, data);
                VerbOutcome::Done { notice: None }
            }
            "workspaces" => {
                self.focus_panel(PanelId::Workspaces, data);
                VerbOutcome::Done { notice: None }
            }
            // `/workspace` never lands here: it is the workspace chooser, and
            // `PickerKind::from_command` opens it the way `/model` and `/theme`
            // are opened, so the rows, the filter and the hit regions are one
            // implementation.
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

    /// Record how many list rows this frame shows; the page keys move the
    /// cursor by exactly that many.
    pub(crate) fn set_list_page(&mut self, rows: usize) {
        self.list_page = rows.max(1);
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

/// One row of the `/` palette: a verb the surface answers, or an installed skill
/// the *agent* answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteRow {
    Command {
        command: &'static str,
        description: &'static str,
    },
    /// A skill, offered as `/<name>`; it is a prompt token, not a surface verb.
    Skill { name: String, description: String },
}

/// How many skills the palette lists. A machine with a hundred installed skills
/// must not turn `/` into a scroll through all of them; the query narrows it and
/// a typed `/name` still reaches the agent either way.
const PALETTE_SKILL_LIMIT: usize = 40;

impl PaletteRow {
    /// What pressing `enter` on this row reports.
    pub fn command(&self) -> String {
        match self {
            Self::Command { command, .. } => (*command).to_string(),
            Self::Skill { name, .. } => format!("/{name}"),
        }
    }

    /// The text `tab` puts in the draft — a *command* whole, so the reader can
    /// add arguments before running it.
    pub fn fill(&self) -> String {
        format!("{} ", self.command())
    }

    /// Does this row name a skill (a prompt token) rather than a surface verb?
    pub fn is_skill(&self) -> bool {
        matches!(self, Self::Skill { .. })
    }

    /// The row's name, without the slash.
    pub fn label(&self) -> &str {
        match self {
            Self::Command { command, .. } => command,
            Self::Skill { name, .. } => name,
        }
    }

    pub fn description(&self) -> &str {
        match self {
            Self::Command { description, .. } => description,
            Self::Skill { description, .. } => description,
        }
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
    ("/paste", Key::CmdPaste),
    ("/mouse", Key::CmdMouse),
    ("/diff", Key::CmdDiff),
    ("/approval", Key::CmdApproval),
    ("/history", Key::CmdHistory),
    ("/tree", Key::CmdTree),
    ("/fork", Key::CmdFork),
    ("/worktree", Key::CmdWorktree),
    ("/mode", Key::CmdMode),
    ("/sandbox", Key::CmdSandbox),
    ("/compact", Key::CmdCompact),
    ("/tools", Key::CmdTools),
    ("/providers", Key::CmdProviders),
    ("/sessions", Key::CmdSessions),
    ("/model", Key::CmdModel),
    ("/thinking", Key::CmdThinking),
    ("/lang", Key::CmdLanguage),
    ("/reply-detail", Key::CmdReplyDetail),
    ("/workspace", Key::CmdWorkspace),
    ("/theme", Key::CmdTheme),
    ("/preferences", Key::CmdPreferences),
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
/// The keys that are *not* configurable actions, for the help overlay: they are
/// spelled inside the composer (`/`, `?`, `@`, `#`), or they are the pointer, or
/// they belong to the focused rail (`p`). Everything with a `preferences::Action`
/// is listed from that table instead, so a rebind is shown rebind.
pub const HELP_KEYS: [(&str, Key); 11] = [
    ("tab", Key::HelpKeyFocus),
    ("↑ ↓", Key::HelpKeyScroll),
    ("pageUp / pageDown", Key::HelpKeyPage),
    ("esc", Key::HintBackToChat),
    ("/", Key::HelpKeyPalette),
    ("?", Key::HelpKeyHelp),
    ("@", Key::HelpKeyMentionFile),
    ("#", Key::HelpKeyMentionSession),
    ("p", Key::HelpKeyPin),
    ("click", Key::HelpKeyClick),
    ("drag", Key::HelpKeyDrag),
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
            .find(|(_, id)| id == "sidebar.tab.recent")
            .unwrap()
            .0;
        assert_eq!(rail.x, 0);
    }

    #[test]
    fn keybinding_editor_rebinds_and_invalid_keys_leave_the_binding_intact() {
        let data = demo::chat();
        let mut wb = workbench();
        wb.overlay = Some(Overlay::Keybindings);
        // The editor lists `ACTIONS` in table order; stand on the palette row so
        // the case names one action rather than "whatever is first today".
        wb.keybinding_index = preferences::ACTIONS
            .iter()
            .position(|action| *action == preferences::Action::Palette)
            .expect("the palette is rebindable");
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
        for data in [demo::launchpad(), demo::chat(), demo::workspaces()] {
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
        assert!(text.contains("最近"), "the sidebar's tabs: {text}");
        assert!(text.contains("命令"), "{text}");
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

    /// `/sessions` and `/workspaces` name the rail's two panels. With room for
    /// a rail they focus it (showing it if the reader had hidden it); on a
    /// terminal too narrow for a rail they take the body, which is the one
    /// surface there is.
    #[test]
    fn the_panel_verbs_focus_their_panel_and_never_reach_the_agent() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);

        assert_eq!(
            workbench.surface_command("/recent", &data),
            VerbOutcome::Done { notice: None }
        );
        assert_eq!(workbench.panel, PanelId::Recent);
        assert!(workbench.sidebar_focus, "the panel it names takes the keys");
        assert!(workbench.sidebar_visible);
        let text = plain(&workbench.render(&data, 120, 38));
        assert!(
            text.contains("最近"),
            "the recent panel is on screen: {text}"
        );

        assert_eq!(
            workbench.surface_command("/workspaces", &data),
            VerbOutcome::Done { notice: None }
        );
        assert_eq!(workbench.panel, PanelId::Workspaces);
        let text = plain(&workbench.render(&data, 120, 38));
        assert!(
            text.contains("个工作区"),
            "the workspace panel is on screen: {text}"
        );

        // A hidden rail comes back rather than swallowing the request.
        workbench.sidebar_visible = false;
        workbench.sidebar_focus = false;
        workbench.surface_command("/recent", &data);
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
            workbench.surface_command("/workspaces", &data),
            VerbOutcome::Done { notice: None }
        );
        assert_eq!(workbench.overlay, Some(Overlay::PanelFull));
        let text = plain(&workbench.render(&data, 60, 20));
        assert!(text.contains("工作区"), "the panel takes the body: {text}");
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
        // `/workspace` is the workspace chooser now: the directory the session
        // runs in is the row ticked `✓` in it, and that row is where "where am
        // I" is answered.
        assert_eq!(
            PickerKind::from_command("/workspace"),
            Some(PickerKind::Workspace)
        );
        workbench.open_picker(PickerKind::Workspace, &data);
        let view = workbench.picker.view(&data, "");
        let current: Vec<&str> = view
            .rows
            .iter()
            .filter_map(|row| match row {
                PickerRow::Item {
                    label,
                    current: true,
                    ..
                } => Some(label.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(current.len(), 1, "{:?}", view.rows);
        workbench.overlay = None;
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

    /// `tab` takes the keys into the rail and lands the cursor on the
    /// conversation that is on screen: a cursor left at row 0 of a list the
    /// reader never looked at would answer `enter` with someone else's session.
    #[test]
    fn tab_focuses_the_rail_on_the_conversation_on_screen() {
        let mut data = demo::chat();
        // Put the conversation on screen second in the list, so "row 0" and
        // "the current conversation" are different answers.
        data.sessions.swap(0, 1);
        let (mut workbench, _rows) = frame(&data, 120, 38);
        assert!(!workbench.sidebar_focus);
        workbench.handle(WbEvent::Key("tab".into()), &data);
        assert!(workbench.sidebar_focus);
        let rows = panels::rows(PanelId::Recent, &data);
        let current = data
            .sessions
            .iter()
            .position(|session| session.current)
            .expect("the fixture marks one conversation current");
        assert_eq!(current, 1, "the fixture moved it off row 0");
        assert_eq!(workbench.sidebar_index, 1, "the cursor lands on the chat");
        assert_eq!(rows[workbench.sidebar_index].session(), Some(current));
    }

    /// `up`/`down` walk the rail's rows (a section heading is stepped over, a
    /// workspace heading is not — it folds) and `enter` opens what is under the
    /// cursor, or folds a group away when the cursor is on its heading.
    #[test]
    fn up_and_down_move_the_rail_cursor_and_enter_opens_it() {
        let data = demo::workspaces();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.handle(WbEvent::Key("tab".into()), &data);
        workbench.panel = PanelId::Workspaces;
        workbench.sidebar_index = 0;

        // The first row is a section heading, which `down` steps over; the row
        // it lands on is the first workspace heading and `enter` folds it.
        let rows = panels::rows(PanelId::Workspaces, &data);
        assert!(rows[0].session().is_none());
        assert!(matches!(rows[1], panels::Row::Workspace(_)));
        workbench.handle(WbEvent::Key("down".into()), &data);
        assert_eq!(workbench.sidebar_index, 1, "the first workspace heading");
        let group = &data.workspaces[0];
        let folded_rows =
            panels::rows_with(PanelId::Workspaces, &data, &workbench.folded_workspaces).len();
        workbench.handle(WbEvent::Key("enter".into()), &data);
        assert!(
            workbench.folded_workspaces.contains(&group.path),
            "enter on a workspace heading folds it"
        );
        assert!(
            panels::rows_with(PanelId::Workspaces, &data, &workbench.folded_workspaces).len()
                < folded_rows,
            "the folded group's conversations leave the list"
        );
        // The cursor stays on the heading, so `enter` again opens it: the fold
        // is a toggle, not a one-way door.
        assert_eq!(workbench.sidebar_index, 1);
        workbench.handle(WbEvent::Key("enter".into()), &data);
        assert!(
            workbench.folded_workspaces.is_empty(),
            "enter opens it again"
        );

        // A second press moves again, onto a conversation, and `enter` opens it.
        workbench.handle(WbEvent::Key("down".into()), &data);
        let index = workbench.sidebar_index;
        let rows = panels::rows(PanelId::Workspaces, &data);
        assert!(index > 1, "the second press moves again");
        let session = rows[index].session().expect("a conversation row");
        assert_eq!(
            workbench.handle(WbEvent::Key("enter".into()), &data),
            vec![WbAction::SwitchSession(data.sessions[session].id.clone())]
        );
    }

    /// The rail's two tabs answer `←`/`→` while the rail has the keys: the
    /// keyboard spelling of a click on the tab strip, and the cursor follows
    /// the switch to the conversation on screen.
    #[test]
    fn left_and_right_move_the_rail_between_its_two_tabs() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.handle(WbEvent::Key("tab".into()), &data);
        assert!(workbench.sidebar_focus);
        assert_eq!(workbench.panel, PanelId::Recent);

        assert_eq!(
            workbench.handle(WbEvent::Key("right".into()), &data),
            vec![WbAction::SwitchPanel(PanelId::Workspaces)]
        );
        assert_eq!(workbench.panel, PanelId::Workspaces);
        assert!(workbench.sidebar_focus, "the rail keeps the keys");
        // The cursor is an index into *this* panel's rows, and it landed on the
        // conversation the transcript shows.
        let rows = panels::rows(PanelId::Workspaces, &data);
        let current = data
            .sessions
            .iter()
            .position(|session| session.current)
            .expect("the fixture marks one conversation current");
        assert_eq!(rows[workbench.sidebar_index].session(), Some(current));

        // …and back the other way, wrapping to 最近 from 最近's neighbour.
        assert_eq!(
            workbench.handle(WbEvent::Key("left".into()), &data),
            vec![WbAction::SwitchPanel(PanelId::Recent)]
        );
        assert_eq!(workbench.panel, PanelId::Recent);

        // The keys belong to the focused rail only: from the composer `left`
        // is still the caret.
        workbench.sidebar_focus = false;
        let actions = workbench.handle(WbEvent::Key("right".into()), &data);
        assert!(
            !actions.contains(&WbAction::SwitchPanel(PanelId::Workspaces)),
            "{actions:?}"
        );
    }

    /// The previous/next keys switch the conversation *and* leave the cursor on
    /// it, so the chat and the rail never name different conversations.
    #[test]
    fn the_session_keys_walk_the_rail_and_switch() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        let next = workbench
            .keys
            .key(preferences::Action::NextSession)
            .to_string();
        let previous = workbench
            .keys
            .key(preferences::Action::PrevSession)
            .to_string();
        // The key asks the driver for a *step*: the surface has the rail, the
        // app has the session list, and the step is resolved against both
        // (`run::event_loop` collapses a held key's repeats into one).
        assert_eq!(
            workbench.handle(WbEvent::Key(next.clone()), &data),
            vec![WbAction::SessionStep(1)]
        );
        assert_eq!(
            workbench.step_session(1, &data),
            vec![WbAction::SwitchSession(data.sessions[1].id.clone())]
        );
        assert_eq!(workbench.sidebar_index, 1);
        // …and back, from the conversation it just named.
        assert_eq!(
            workbench.handle(WbEvent::Key(previous.clone()), &data),
            vec![WbAction::SessionStep(-1)]
        );
        assert_eq!(
            workbench.step_session(-1, &data),
            vec![WbAction::SwitchSession(data.sessions[0].id.clone())]
        );
        assert_eq!(workbench.sidebar_index, 0);
        // At either end the rail stops: the key still asks for a step, and the
        // step finds nowhere to go, so no conversation is switched.
        assert_eq!(
            workbench.handle(WbEvent::Key(previous), &data),
            vec![WbAction::SessionStep(-1)]
        );
        assert!(workbench.step_session(-1, &data).is_empty());
    }

    /// A held `shift+→` is one switch: the driver accumulates the burst's steps
    /// and asks the surface to walk them in one go.
    #[test]
    fn a_burst_of_session_steps_is_one_switch() {
        let mut data = demo::chat();
        data.sessions.truncate(4);
        data.chats.clear();
        data.workspaces.clear();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        for _ in 0..3 {
            let actions = workbench.handle(WbEvent::Key("shift+right".into()), &data);
            assert_eq!(actions, vec![WbAction::SessionStep(1)]);
        }
        assert_eq!(
            workbench.step_session(3, &data),
            vec![WbAction::SwitchSession(data.sessions[3].id.clone())],
            "the net step lands on the fourth conversation, in one switch"
        );
        assert_eq!(workbench.sidebar_index, 3);
        // Past the end the steps find nowhere to go: no switch, and no wrap
        // around to the first conversation.
        assert!(workbench.step_session(5, &data).is_empty());
        assert_eq!(workbench.sidebar_index, 3);
    }

    /// `p` in the rail pins the workspace the cursor's conversation belongs to,
    /// and does nothing but type a `p` anywhere else.
    #[test]
    fn p_pins_the_selected_conversations_workspace() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        // Not in the rail: a letter in the draft, as always.
        let typed = workbench.handle(WbEvent::Key("p".into()), &data);
        assert!(
            matches!(typed.as_slice(), [WbAction::EditEditor(_)]),
            "{typed:?}"
        );

        workbench.handle(WbEvent::Key("tab".into()), &data);
        workbench.sidebar_index = 0;
        let workspace = &data.workspaces[0];
        assert!(workspace.pinned);
        assert_eq!(
            workbench.handle(WbEvent::Key("p".into()), &data),
            vec![WbAction::SetWorkspacePinned {
                path: workspace.path.clone(),
                pinned: false,
            }]
        );

        // A conversation that belongs to no workspace says why it cannot be
        // pinned instead of pinning something else.
        workbench.panel = PanelId::Workspaces;
        workbench.sidebar_index = 0;
        while workbench
            .panel_rows(&data)
            .get(workbench.sidebar_index)
            .is_some_and(|row| row.session().is_some())
        {
            workbench.sidebar_index += 1;
        }
        let rows = workbench.panel_rows(&data);
        if let Some(index) = rows.iter().position(|row| {
            row.session()
                .is_some_and(|session| data.chats.contains(&data.sessions[session].id))
        }) {
            workbench.sidebar_index = index;
            match workbench.handle(WbEvent::Key("p".into()), &data).as_slice() {
                [WbAction::Notice(_)] => {}
                other => panic!("a chat cannot be pinned: {other:?}"),
            }
        }
    }

    #[test]
    fn the_sidebar_tabs_switch_the_panel_by_click() {
        let data = demo::chat();
        let (mut workbench, rows) = frame(&data, 120, 38);
        let text = plain(&rows);
        assert!(text.contains("最近"), "{text}");

        // The tab's own region is registered while drawing, so it cannot drift
        // from what is on screen.
        let (rect, _) = workbench
            .hits()
            .regions()
            .iter()
            .find(|(_, id)| id == "sidebar.tab.recent")
            .cloned()
            .expect("the recent tab is a target");
        let press = mouse::MouseEvent {
            kind: mouse::MouseKind::Press,
            x: rect.x + 1,
            y: rect.y,
            shift: false,
            alt: false,
            ctrl: false,
        };
        let actions = workbench.handle(WbEvent::Mouse(press), &data);
        assert_eq!(actions, vec![WbAction::SwitchPanel(PanelId::Recent)]);
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

    /// On the launchpad `tab` walks the prompt and the parameter chips, `enter`
    /// cycles the chip under the cursor instead of submitting, and typing or
    /// `escape` hands the keys back to the prompt — the chips are reachable
    /// without a mouse, and a chip cannot swallow the send key.
    #[test]
    fn the_launchpad_chips_answer_tab_and_enter_and_give_the_keys_back() {
        let data = demo::launchpad();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        assert_eq!(workbench.launch_field, LaunchField::Prompt);

        // tab → the first chip is the workspace, then the model.
        workbench.handle(WbEvent::Key("tab".into()), &data);
        assert_eq!(workbench.launch_field, LaunchField::Workspace);
        workbench.handle(WbEvent::Key("tab".into()), &data);
        assert_eq!(workbench.launch_field, LaunchField::Model);
        workbench.handle(WbEvent::Key("shift+tab".into()), &data);
        assert_eq!(workbench.launch_field, LaunchField::Workspace);

        // …the focused chip is the one the command names, and `enter` runs it.
        let actions = workbench.handle(WbEvent::Key("enter".into()), &data);
        assert_eq!(
            actions,
            vec![WbAction::Command("/launchpad.workspace".into())]
        );

        // `escape` puts the cursor back in the prompt, so `enter` sends again.
        workbench.handle(WbEvent::Key("escape".into()), &data);
        assert_eq!(workbench.launch_field, LaunchField::Prompt);

        // Typing does the same: a letter belongs in the draft, and the chip it
        // would have been typed into stops claiming the keys.
        workbench.handle(WbEvent::Key("tab".into()), &data);
        assert_ne!(workbench.launch_field, LaunchField::Prompt);
        let actions = workbench.handle(WbEvent::Key("h".into()), &data);
        assert!(matches!(actions.as_slice(), [WbAction::EditEditor(_)]));
        assert_eq!(workbench.launch_field, LaunchField::Prompt);
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
        let rows = workbench.palette_rows(&data);
        assert!(rows.iter().any(|row| row.command() == "/model"), "{rows:?}");
        assert!(
            rows.iter().all(|row| row.label().contains("mod")
                || row.description().to_lowercase().contains("mod")),
            "only matches are listed: {rows:?}"
        );
        let chosen = workbench.palette_selection(&data).expect("a selection");
        assert_eq!(
            workbench.handle(WbEvent::Key("enter".into()), &data),
            vec![WbAction::Command(chosen.command())]
        );
        assert_eq!(workbench.overlay, None);
    }

    /// `ctrl+m` is the model picker, `ctrl+1`/`ctrl+2` the rail's two tabs — the
    /// three keys the reader asked for so the pointer is never the only way.
    #[test]
    fn the_model_and_panel_keys_reach_their_surfaces() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        assert_eq!(
            workbench.handle(WbEvent::Key("ctrl+m".into()), &data),
            vec![WbAction::Command("/model".into())]
        );
        assert_eq!(
            workbench.handle(WbEvent::Key("ctrl+1".into()), &data),
            vec![WbAction::SwitchPanel(PanelId::Recent)]
        );
        assert_eq!(
            workbench.handle(WbEvent::Key("ctrl+2".into()), &data),
            vec![WbAction::SwitchPanel(PanelId::Workspaces)]
        );
        // …and each one lands where a click on the tab strip does: the panel is
        // the tab in force, the rail takes the keys, and the cursor is on the
        // conversation on screen.
        assert_eq!(workbench.panel, PanelId::Workspaces);
        assert!(workbench.sidebar_focus && workbench.sidebar_visible);
        let rows = workbench.render(&data, 120, 38);
        assert!(plain(&rows).contains("对话会话"), "{}", plain(&rows));

        // `ctrl+3` is the file tree, and it takes the keys like the other two.
        assert_eq!(
            workbench.handle(WbEvent::Key("ctrl+3".into()), &data),
            vec![WbAction::SwitchPanel(PanelId::Files)]
        );
        assert_eq!(workbench.panel, PanelId::Files);
        let rows = workbench.render(&data, 120, 38);
        let text = plain(&rows);
        assert!(text.contains("文件"), "{text}");
        assert!(text.contains("README.md"), "{text}");
    }

    /// The tree's own keys: `↑↓` walk every row, `→`/`←` open and close a
    /// directory, `enter` previews a file or opens a directory.
    #[test]
    fn the_file_panel_navigates_the_tree_with_the_keyboard() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.handle(WbEvent::Key("ctrl+3".into()), &data);
        assert_eq!(workbench.panel, PanelId::Files);
        // 0 = `docs` (open in the fixture), 1 = its `tui` child, 2 = README.md.
        assert_eq!(workbench.sidebar_index, 0);
        workbench.handle(WbEvent::Key("down".into()), &data);
        assert_eq!(
            workbench.sidebar_index, 1,
            "one row per step, headings included"
        );
        workbench.handle(WbEvent::Key("down".into()), &data);
        assert_eq!(workbench.sidebar_index, 2);

        // `enter` on a file asks for its preview; on a directory, for its
        // children.
        assert_eq!(
            workbench.handle(WbEvent::Key("enter".into()), &data),
            vec![WbAction::PreviewFile(
                data.tree.rows()[2].path.to_string_lossy().to_string()
            )]
        );
        workbench.sidebar_index = 3; // the `tui` directory at depth 0
        assert_eq!(
            workbench.handle(WbEvent::Key("enter".into()), &data),
            vec![WbAction::ToggleFolder(
                data.tree.rows()[3].path.to_string_lossy().to_string()
            )]
        );
        // `→` opens what is closed, `←` closes what is open, and neither
        // switches the panel (that is `ctrl+1`…`ctrl+3` on this tab).
        assert!(matches!(
            workbench
                .handle(WbEvent::Key("right".into()), &data)
                .as_slice(),
            [WbAction::ToggleFolder(_)]
        ));
        workbench.sidebar_index = 0;
        assert!(matches!(
            workbench
                .handle(WbEvent::Key("left".into()), &data)
                .as_slice(),
            [WbAction::ToggleFolder(_)]
        ));
        assert_eq!(workbench.panel, PanelId::Files, "the panel did not move");
    }

    /// A preview keeps its own view state, and a new file starts fresh.
    #[test]
    fn the_preview_box_zooms_pans_and_resets() {
        let mut data = demo::chat();
        data.preview = Some(demo::preview_image());
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.overlay = Some(Overlay::FilePreview);
        assert_eq!(workbench.preview_zoom, 1);
        workbench.handle(WbEvent::Key("+".into()), &data);
        assert_eq!(workbench.preview_zoom, 2);
        workbench.handle(WbEvent::Key("=".into()), &data);
        assert_eq!(workbench.preview_zoom, 3, "`=` is `+` without the shift");
        for _ in 0..20 {
            workbench.handle(WbEvent::Key("+".into()), &data);
        }
        assert_eq!(
            workbench.preview_zoom, MAX_PREVIEW_ZOOM,
            "the zoom is bounded"
        );

        // A zoomed picture pans; the pan is clamped by the renderer, so the
        // stored value only has to move in the asked direction.
        let before = workbench.preview_pan.1;
        workbench.handle(WbEvent::Key("down".into()), &data);
        assert!(workbench.preview_pan.1 > before);

        // `esc` closes the box without touching the zoom; opening another file
        // resets it, so a zoom chosen for one picture does not arrive on the
        // next.
        let actions = workbench.handle(WbEvent::Key("escape".into()), &data);
        assert_eq!(actions, vec![WbAction::Redraw]);
        assert_eq!(workbench.overlay, None);
        workbench.reset_preview();
        assert_eq!((workbench.preview_zoom, workbench.preview_pan), (1, (0, 0)));
    }

    /// The wheel is the pointer's zoom, and text previews scroll with it.
    #[test]
    fn the_wheel_zooms_a_picture_and_scrolls_a_document() {
        let mut data = demo::chat();
        data.preview = Some(demo::preview_image());
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.overlay = Some(Overlay::FilePreview);
        let press = |kind| mouse::MouseEvent {
            kind,
            x: 10,
            y: 10,
            shift: false,
            alt: false,
            ctrl: false,
        };
        workbench.handle(WbEvent::Mouse(press(mouse::MouseKind::WheelUp)), &data);
        assert_eq!(workbench.preview_zoom, 2);
        workbench.handle(WbEvent::Mouse(press(mouse::MouseKind::WheelDown)), &data);
        assert_eq!(workbench.preview_zoom, 1);

        // A document has no zoom to give: the wheel scrolls it instead.
        data.preview = Some(state::PreviewView::Text {
            path: "README.md".into(),
            lines: (0..40).map(|n| n.to_string()).collect(),
            truncated: false,
        });
        workbench.preview_scroll = 5;
        workbench.handle(WbEvent::Mouse(press(mouse::MouseKind::WheelDown)), &data);
        assert!(workbench.preview_scroll > 5, "wheel down moves the text");
    }

    /// `ctrl+v` asks the adapter to read the clipboard; `alt+v` for the image
    /// already in the draft.
    #[test]
    fn the_clipboard_and_attachment_keys_ask_for_their_verbs() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        assert_eq!(
            workbench.handle(WbEvent::Key("ctrl+v".into()), &data),
            vec![WbAction::PasteClipboard]
        );
        assert_eq!(
            workbench.handle(WbEvent::Key("alt+v".into()), &data),
            vec![WbAction::PreviewAttachment]
        );
    }

    /// The palette is one list of two kinds of thing: a verb the surface answers
    /// and a skill the agent expands. A skill must not be *run* as a verb — the
    /// agent would never see it — so it fills the draft instead.
    #[test]
    fn the_palette_lists_skills_after_the_commands_and_fills_the_draft() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.handle(WbEvent::Key("/".into()), &data);
        let rows = workbench.palette_rows(&data);
        let commands = rows.iter().filter(|row| !row.is_skill()).count();
        let skills = rows.iter().filter(|row| row.is_skill()).count();
        assert_eq!(commands, COMMANDS.len());
        assert_eq!(
            skills,
            data.skills.len(),
            "every installed skill is offered"
        );
        assert!(
            rows[..commands].iter().all(|row| !row.is_skill()),
            "commands first, skills after"
        );
        // Filtering reaches a skill by its name.
        for key in ["f", "u", "t", "u", "r", "e", "-", "p"] {
            workbench.handle(WbEvent::Key(key.into()), &data);
        }
        let filtered = workbench.palette_rows(&data);
        assert!(
            filtered
                .iter()
                .any(|row| row.label() == "future-peer-review"),
            "{filtered:?}"
        );
        // Enter on a skill fills the draft: the skill is a prompt token the agent
        // expands, so running it as a surface command would be a lie.
        let selection = workbench.palette_selection(&data).expect("a row");
        assert!(selection.is_skill());
        let actions = workbench.handle(WbEvent::Key("enter".into()), &data);
        assert!(
            matches!(actions.as_slice(), [WbAction::EditInput { text, .. }] if *text == selection.fill()),
            "{actions:?}"
        );
        assert_eq!(workbench.overlay, None);
    }

    /// `tab` completes the row into the draft and leaves the palette: the reader
    /// can then add arguments to a command, or type the sentence a skill starts.
    #[test]
    fn tab_completes_the_highlighted_row_into_the_draft() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.handle(WbEvent::Key("/".into()), &data);
        for key in ["m", "o", "d", "e", "l"] {
            workbench.handle(WbEvent::Key(key.into()), &data);
        }
        let chosen = workbench.palette_selection(&data).expect("a row");
        let actions = workbench.handle(WbEvent::Key("tab".into()), &data);
        let [WbAction::EditInput { text, cursor }] = actions.as_slice() else {
            panic!("{actions:?}");
        };
        assert_eq!(text, &chosen.fill());
        assert_eq!(*cursor, text.len(), "the caret is after the command");
        assert_eq!(workbench.overlay, None);
    }

    /// The painted palette says which rows are which, exactly once per kind, and
    /// a click still lands on the row it was painted on with the headings there.
    #[test]
    fn the_palette_paints_a_heading_once_per_kind_and_keeps_clicks_aligned() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.handle(WbEvent::Key("/".into()), &data);
        let text = plain(&workbench.render(&data, 120, 38));
        let headings = text.matches(" 命令").count() + text.matches(" 技能").count();
        assert!(headings >= 2, "both kinds are named: {text}");
        assert!(text.contains("/future-web"), "a skill is listed: {text}");
        // The last command's target is on the line the last command was drawn on:
        // click it and the action names that command, not its neighbour.
        let last = COMMANDS.len() - 1;
        let expected = workbench.palette_rows(&data)[last].command();
        assert_eq!(
            click_region(
                &mut workbench,
                &data,
                120,
                38,
                &format!("palette.row.{last}")
            ),
            vec![WbAction::Command(expected)]
        );
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
        assert!(plain(&rows).contains("最近"));

        // open → focus
        workbench.handle(WbEvent::Key("ctrl+b".into()), &data);
        assert!(workbench.sidebar_visible && workbench.sidebar_focus);

        // focus → closed. Closing is a *preference*, not a transient view
        // state: the action asks the driver to write it, so the next start opens
        // the way the reader left it.
        let actions = workbench.handle(WbEvent::Key("ctrl+b".into()), &data);
        assert!(actions.contains(&WbAction::SetPreferences), "{actions:?}");
        assert!(!workbench.sidebar_visible && !workbench.sidebar_focus);
        assert!(
            !workbench.preferences().sidebar_visible,
            "the choice is the state"
        );
        let rows = workbench.render(&data, 120, 38);
        assert!(
            !plain(&rows).contains("待办"),
            "the rail is gone: {}",
            plain(&rows)
        );
        // …and the one thing the frame keeps is the button that brings it back:
        // the closed rail's edge, not a panel row.
        let ids: Vec<&str> = workbench
            .hits()
            .regions()
            .iter()
            .map(|(_, id)| id.as_str())
            .collect();
        assert_eq!(
            ids.iter()
                .filter(|id| id.starts_with("sidebar."))
                .copied()
                .collect::<Vec<_>>(),
            vec!["sidebar.toggle"],
            "{ids:?}"
        );
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

    /// Press the middle of a painted region: the same path a real click takes,
    /// resolved against the hit map the last frame registered. One cell is
    /// enough — a one-column target (the rail's edge) is aimed at, not rounded
    /// off.
    fn press_region(
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
            x: rect.x + rect.w.saturating_sub(1) / 2,
            y: rect.y + rect.h.saturating_sub(1) / 2,
            shift: false,
            alt: false,
            ctrl: false,
        };
        workbench.handle(WbEvent::Mouse(press), data)
    }

    #[test]
    fn the_rail_button_collapses_and_reopens_from_the_edge() {
        let data = demo::chat();
        let (mut workbench, rows) = frame(&data, 120, 38);
        // The rail is up and its own edge carries the collapse arrow, on the
        // row the tabs are painted on.
        assert!(workbench.sidebar_visible);
        assert!(plain(&rows).contains("最近"));
        let (rect, _) = workbench
            .hits()
            .regions()
            .iter()
            .find(|(_, id)| id == "sidebar.toggle")
            .expect("the collapse button");
        assert_eq!(rect.x + rect.w, 120, "it sits on the rail's outer edge");
        assert_eq!((rect.y, rect.h), (3, 1), "on the tab-strip row");
        assert!(
            crate::utils::strip_ansi_codes(&rows[3]).ends_with('›'),
            "{}",
            crate::utils::strip_ansi_codes(&rows[3])
        );

        // One click closes it, and it asks the driver to remember that.
        let actions = press_region(&mut workbench, &data, 120, 38, "sidebar.toggle");
        assert_eq!(actions, vec![WbAction::SetPreferences]);
        assert!(!workbench.sidebar_visible && !workbench.sidebar_focus);
        let rows = workbench.render(&data, 120, 38);
        assert!(!plain(&rows).contains("最近"), "{}", plain(&rows));
        // …and the closed rail leaves its edge behind: the way back is a target
        // in the same column the arrow was in.
        let (rect, _) = workbench
            .hits()
            .regions()
            .iter()
            .find(|(_, id)| id == "sidebar.toggle")
            .expect("the expand button");
        assert_eq!(rect.x + rect.w, 120);
        // The edge runs the body *and* the composer band: from under the header
        // down to the footer, so nothing beside the box is bare canvas.
        assert_eq!((rect.y, rect.h), (3, 29), "{rect:?}");
        assert!(plain(&rows).contains('‹'), "{}", plain(&rows));

        // A second click brings it back, focused, with the cursor on the
        // conversation on screen.
        press_region(&mut workbench, &data, 120, 38, "sidebar.toggle");
        assert!(workbench.sidebar_visible && workbench.sidebar_focus);
        let rows = workbench.render(&data, 120, 38);
        assert!(plain(&rows).contains("最近"), "{}", plain(&rows));

        // A left-positioned rail keeps the same bargain: the button is on the
        // rail's own outer edge (column 0) and stays there when it closes.
        workbench.sidebar_position = preferences::SidebarPosition::Left;
        let (rect, _) = {
            workbench.render(&data, 120, 38);
            workbench
                .hits()
                .regions()
                .iter()
                .find(|(_, id)| id == "sidebar.toggle")
                .cloned()
                .expect("the collapse button")
        };
        assert_eq!((rect.x, rect.w), (0, 1), "{rect:?}");
        press_region(&mut workbench, &data, 120, 38, "sidebar.toggle");
        assert!(!workbench.sidebar_visible);
        let rows = workbench.render(&data, 120, 38);
        let (rect, _) = workbench
            .hits()
            .regions()
            .iter()
            .find(|(_, id)| id == "sidebar.toggle")
            .cloned()
            .expect("the expand button");
        assert_eq!((rect.x, rect.w), (0, 1), "{rect:?}");
        assert!(
            crate::utils::strip_ansi_codes(&rows[3]).starts_with('›'),
            "{}",
            crate::utils::strip_ansi_codes(&rows[3])
        );
    }

    #[test]
    fn a_focused_rail_names_the_keys_it_answers() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        let unfocused = plain(&workbench.render(&data, 120, 38));
        assert!(!unfocused.contains("⏎ 打开"), "{unfocused}");
        workbench.sidebar_focus = true;
        let rows = workbench.render(&data, 120, 38);
        let text = plain(&rows);
        // The footer is the rail's own last row, and it is not a click target:
        // a reader aiming at a hint must not open the conversation that used to
        // be painted there.
        assert!(text.contains("↑↓ 选择"), "{text}");
        let footer = rows
            .iter()
            .position(|row| crate::utils::strip_ansi_codes(row).contains("页签"))
            .expect("the rail's footer");
        assert!(
            workbench
                .hits()
                .regions()
                .iter()
                .all(|(rect, _)| rect.y as usize != footer),
            "nothing is clickable on the hint row"
        );
    }

    #[test]
    fn a_narrow_terminal_has_no_sidebar_but_ctrl_b_still_reaches_the_panel() {
        let data = demo::workspaces();
        let (mut workbench, rows) = frame(&data, 70, 24);
        let text = plain(&rows);
        assert!(!text.contains("最近"), "no sidebar at 70 columns: {text}");
        assert!(workbench
            .hits()
            .regions()
            .iter()
            .all(|(_, id)| !id.starts_with("sidebar.")));

        let actions = workbench.handle(WbEvent::Key("ctrl+b".into()), &data);
        assert_eq!(actions, vec![WbAction::Redraw]);
        // The full-screen panel is now the surface: it carries the same two
        // tabs the rail does, starting on 最近.
        let text = plain(&workbench.render(&data, 70, 24));
        assert!(text.contains("最近"), "{text}");
        assert!(
            text.contains("future-os"),
            "a workspace in the list: {text}"
        );
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
        workbench.panel = PanelId::Recent;
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
        assert!(text.contains("future-os"), "the workspace column: {text}");
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

    /// The preview box names the picture and keeps the frame's size invariant at
    /// every terminal size. A picture is either placed by the terminal's image
    /// protocol — never drawn as half blocks, which is a mosaic of the thing the
    /// reader opened — or named plainly with the reason it cannot be shown.
    #[test]
    fn the_preview_box_paints_the_picture_and_stays_in_the_frame() {
        use crate::terminal_image::{set_capabilities, ImageProtocol, TerminalCapabilities};
        let mut data = demo::chat();
        data.preview = Some(demo::preview_image());
        for (w, h) in [(120u16, 38u16), (88, 24), (60, 16), (24, 8), (1, 1)] {
            let (mut workbench, _rows) = frame(&data, w, h);
            workbench.overlay = Some(Overlay::FilePreview);
            let rows = workbench.render(&data, w, h);
            assert_eq!(rows.len(), h as usize, "{w}x{h}");
            for row in &rows {
                assert_eq!(ansi::width(row), w as usize, "{w}x{h}: {row:?}");
            }
        }

        // A terminal with the kitty graphics protocol gets the picture: the box
        // asks for a placement over its own content area and draws no pixels of
        // its own.
        let kitty = TerminalCapabilities {
            images: ImageProtocol::Kitty,
            true_color: true,
            ..TerminalCapabilities::default()
        };
        set_capabilities(kitty);
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.overlay = Some(Overlay::FilePreview);
        let rows = workbench.render(&data, 120, 38);
        let text = plain(&rows);
        assert!(text.contains("design.png"), "{text}");
        assert!(text.contains("96×64"), "the title names the size: {text}");
        assert!(!text.contains('▀'), "no half-block mosaic: {text}");
        let request = workbench
            .image_request
            .expect("a kitty terminal is asked to place the picture");
        assert!(request.columns > 0 && request.rows > 0, "{request:?}");
        assert_eq!(
            request.crop,
            (0, 0, 96, 64),
            "at 1x the whole picture is transmitted"
        );

        // Zooming says so, so `+` is visible before the picture grows, and the
        // window is what gets transmitted.
        workbench.handle(WbEvent::Key("+".into()), &data);
        let rows = workbench.render(&data, 120, 38);
        assert!(plain(&rows).contains("2x"), "{}", plain(&rows));
        let zoomed = workbench.image_request.expect("still placed");
        assert!(
            zoomed.crop.2 - zoomed.crop.0 < 96 || zoomed.crop.3 - zoomed.crop.1 < 64,
            "a zoomed window is a crop of the picture: {zoomed:?}"
        );

        // A terminal without the protocol gets the facts and an honest
        // sentence, and still no mosaic.
        set_capabilities(TerminalCapabilities::default());
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.overlay = Some(Overlay::FilePreview);
        let text = plain(&workbench.render(&data, 120, 38));
        assert!(text.contains("design.png"), "{text}");
        assert!(text.contains("96×64"), "{text}");
        assert!(
            text.contains("cannot show pictures") || text.contains("不能显示图片"),
            "{text}"
        );
        assert!(!text.contains('▀'), "{text}");
        assert!(workbench.image_request.is_none());
        crate::terminal_image::reset_capabilities_cache();
    }

    /// A preview whose bytes could not be read says why, in the reader's
    /// language, instead of painting an empty box.
    #[test]
    fn a_preview_that_could_not_load_explains_itself() {
        let mut data = demo::chat();
        data.preview = Some(state::PreviewView::Unavailable {
            path: "/tmp/nothing.bin".into(),
            reason: "permission denied".into(),
        });
        let (mut workbench, _rows) = frame(&data, 120, 38);
        workbench.overlay = Some(Overlay::FilePreview);
        let text = plain(&workbench.render(&data, 120, 38));
        assert!(text.contains("nothing.bin"), "{text}");
        assert!(text.contains("没有可预览的内容"), "{text}");
        assert!(text.contains("permission denied"), "{text}");
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
    fn quit_is_reachable_from_the_surface_and_from_inside_an_overlay() {
        let data = demo::chat();
        let (mut workbench, _rows) = frame(&data, 120, 38);
        assert_eq!(
            workbench.handle(WbEvent::Key("ctrl+c".into()), &data),
            vec![WbAction::Quit]
        );
        // Inside the palette every other key belongs to the palette, and escape
        // is its way out — but quitting still has to work: a modal that ate
        // `ctrl+c` would leave a signal as the only exit, and a signalled TUI is
        // the exit that cannot hand the terminal back cleanly.
        workbench.handle(WbEvent::Key("/".into()), &data);
        assert!(workbench.overlay.is_some());
        assert_eq!(
            workbench.handle(WbEvent::Key("ctrl+c".into()), &data),
            vec![WbAction::Quit]
        );
        workbench.overlay = Some(Overlay::Help);
        assert_eq!(
            workbench.handle(WbEvent::Key("ctrl+c".into()), &data),
            vec![WbAction::Quit]
        );
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
        assert!(zh.contains("最近"), "{zh}");
        assert!(zh.contains("新会话"), "{zh}");

        workbench.set_language(LanguagePreference::En);
        let en = plain(&workbench.render(&data, 120, 38));
        assert!(en.contains("Recent"), "{en}");
        assert!(en.contains("New Chat"), "{en}");
        assert!(!en.contains("最近"), "no Chinese tab left: {en}");

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
            .palette_selection(&data)
            .is_some_and(|row| row.command() == "/lang"));

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
            workbench.handle(WbEvent::Key("alt+l".into()), &data),
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
            .bind(preferences::Action::LinearReading, "ctrl+y")
            .unwrap();
        workbench.handle(WbEvent::Key("ctrl+y".into()), &data);
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
        workbench.handle(WbEvent::Key("alt+a".into()), &data);
        assert!(workbench.reduced_motion && !workbench.motion_enabled());
        workbench.handle(WbEvent::Key("alt+l".into()), &data);
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

    /// A list longer than its box must page. `pageDown` used to add to
    /// `overlay_scroll`, which list overlays never read — the key was inert on
    /// every one of them while the help table advertised a page key.
    #[test]
    fn page_keys_move_any_list_cursor_by_a_page() {
        let mut data = demo::chat();
        data.sessions = (0..30)
            .map(|index| state::SessionRow {
                id: format!("page-{index:03}"),
                title: format!("第 {index} 个会话 · 标题足够长，长到必须被截断才放得下"),
                workspace: format!("D:/future-os/.worktrees/page-{index}"),
                ..SessionRow::default()
            })
            .collect();
        data.fork_points = (0..30)
            .map(|index| state::ForkPoint {
                id: format!("e{index}"),
                text: format!("第 {index} 条用户消息，同样很长，长到占不下一行"),
            })
            .collect();
        data.worktrees = (0..30)
            .map(|index| state::WorktreeView {
                path: format!("D:/future-os/.worktrees/page-branch-{index}"),
                label: format!("feat/page-{index}"),
                description: format!("第 {index} 个工作区"),
                current: index == 0,
            })
            .collect();

        for overlay in [
            Overlay::History,
            Overlay::Tree,
            Overlay::Fork,
            Overlay::Worktree,
        ] {
            let mut wb = workbench();
            wb.overlay = Some(overlay);
            wb.render(&data, 100, 30);
            let page = wb.list_page;
            assert!(page > 1, "{overlay:?}: the list shows one row");
            assert_eq!(wb.workflow_index, 0);

            wb.handle(WbEvent::Key("pageDown".into()), &data);
            let rows = wb.render(&data, 100, 30);
            assert_eq!(
                wb.workflow_index,
                page.min(29),
                "{overlay:?}: pageDown should land a page down"
            );
            // …and the cursor is still on screen, with its arrow.
            let (rect, _) = wb
                .hits()
                .regions()
                .iter()
                .find(|(_, id)| *id == format!("workflow.row.{}", wb.workflow_index))
                .cloned()
                .unwrap_or_else(|| panic!("{overlay:?}: the selection is not painted"));
            assert!(
                crate::utils::strip_ansi_codes(&rows[rect.y as usize]).contains('\u{203a}'),
                "{overlay:?}: row {} has no arrow",
                rect.y
            );

            wb.handle(WbEvent::Key("pageUp".into()), &data);
            assert_eq!(wb.workflow_index, 0, "{overlay:?}: pageUp should come back");
        }

        // The diff is a list *and* a detail view: whichever pane has the keys
        // decides whether the page key moves the cursor or the patch.
        data.diff = Some(state::DiffView {
            files: (0..30)
                .map(|index| diff_review::DiffFile {
                    path: format!("src/page_{index}.rs"),
                    status: "M".into(),
                })
                .collect(),
            ..state::DiffView::default()
        });
        let mut wb = workbench();
        wb.overlay = Some(Overlay::Diff);
        wb.diff_patch_focus = false;
        wb.render(&data, 100, 30);
        let page = wb.list_page;
        wb.handle(WbEvent::Key("pageDown".into()), &data);
        assert_eq!(
            wb.workflow_index,
            page.min(29),
            "the file navigator pages its own cursor"
        );
        assert_eq!(wb.overlay_scroll, 0, "and leaves the patch where it was");
        wb.diff_patch_focus = true;
        wb.handle(WbEvent::Key("pageDown".into()), &data);
        assert_eq!(wb.overlay_scroll, page, "the patch scrolls a page");
    }

    /// The cursor menus keep their own field, so they page too — a long list on
    /// a short screen cannot be crossed with `down` alone in reasonable time.
    #[test]
    fn page_keys_move_the_menu_cursors_too() {
        let data = demo::chat();

        let mut wb = workbench();
        wb.open_preferences();
        wb.render(&data, 100, 30);
        let page = wb.list_page;
        wb.handle(WbEvent::Key("pageDown".into()), &data);
        assert_eq!(
            wb.preferences_index,
            page.min(crate::workbench::preferences::PREFERENCE_ROWS.len() - 1)
        );
        wb.handle(WbEvent::Key("pageUp".into()), &data);
        assert_eq!(wb.preferences_index, 0);

        let mut wb = workbench();
        wb.open_preferences();
        wb.overlay = Some(Overlay::Keybindings);
        wb.render(&data, 100, 30);
        let page = wb.list_page;
        wb.handle(WbEvent::Key("pageDown".into()), &data);
        assert_eq!(
            wb.keybinding_index,
            page.min(crate::workbench::preferences::ACTIONS.len() - 1)
        );

        let mut wb = workbench();
        wb.overlay = Some(Overlay::Palette);
        wb.render(&data, 100, 30);
        let page = wb.list_page;
        wb.handle(WbEvent::Key("pageDown".into()), &data);
        assert_eq!(
            wb.palette_index,
            page.min(wb.palette_rows(&data).len().saturating_sub(1))
        );

        let mut data = demo::chat();
        data.queue = (0..30)
            .map(|index| state::QueuedPrompt {
                run_id: format!("run-{index}"),
                text: format!("第 {index} 条排队消息"),
            })
            .collect();
        let mut wb = workbench();
        wb.surface_command("/queue", &data);
        wb.render(&data, 100, 30);
        let page = wb.list_page;
        wb.handle(WbEvent::Key("pageDown".into()), &data);
        assert_eq!(wb.queue_index, page.min(29));
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
        let expected = wb.palette_rows(&data)[0].command();
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
