//! The TUI application — port of `tui/src/app.ts` (2669 lines).
//!
//! Architecture notes (the TS app is a single-threaded event loop; this port
//! keeps the same model):
//!
//!   - All app logic lives in sync `&mut self` methods.
//!   - Every async client operation (slash commands, refresh-on-event, model
//!     cycling, ...) is spawned as a tokio task that only talks to the
//!     `GrpcClient` and reports back through an `mpsc::UnboundedSender<UiCmd>`
//!     channel. The app loop (in `index.rs`) applies each `UiCmd` to the app.
//!     This mirrors the TS fire-and-forget `promise.then(...)` chains without
//!     holding `&mut self` across awaits.
//!   - Overlay callbacks (SelectList onSelect/onCancel, ...) likewise send
//!     `UiCmd`s instead of capturing the app (no self-referential borrows).
//!   - The render scheduler is deadline-driven: `request_render` computes a
//!     deadline (33 ms minimum interval, `process.nextTick`-equivalent for
//!     force renders) and the loop's `on_tick` fires `do_render` when due.
//!   - The terminal is abstracted behind `TerminalIo` so the diff pipeline can
//!     be driven against a fake terminal in tests.

use crate::components::autocomplete::{
    AttachmentProvider, AutocompleteItem, AutocompleteManager, AutocompletePopup, FilePathProvider,
    SlashCommand, SlashCommandProvider,
};
use crate::components::chat_area::{ChatArea, ChatMessage, ChatRole, RunState, ToolStatus};
use crate::components::footer::{Footer, FooterData};
use crate::components::input::Input;
use crate::components::scoped_models_selector::{
    ScopedModelsSelector, ScopedModelsSelectorOptions,
};
use crate::components::select_list::{SelectItem, SelectList, SelectListOptions};
use crate::keybindings::KeybindingManager;
use crate::keys::key as Key;
use crate::keys::{is_key_release, parse_key};
use crate::rpc::grpc_client::GrpcClient;
use crate::rpc::types::{AgentEvent, ModelInfo, RpcSessionState, SessionSummary, ThinkingLevel};
use crate::terminal_image::{
    collect_kitty_image_ids, delete_kitty_images, extract_kitty_image_ids, get_capabilities,
    is_image_line, set_cell_dimensions, CellDimensions, ImageProtocol,
};
use crate::theme::{bold, fg, Theme, DARK_THEME};
use crate::tui::{
    is_focusable, resolve_overlay_layout, set_component_focused, Component, OverlayOptions,
    SizeValue, SYNC_BEGIN, SYNC_END,
};
use crate::utils::{
    extract_segments, normalize_terminal_output, slice_by_column, strip_ansi_codes,
    truncate_to_width, visible_width, wrap_text_with_ansi, TruncateOptions,
};
use crate::version::VERSION;
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use uuid::Uuid;

// ─── Terminal abstraction ───────────────────────────────────────────────────

/// The subset of `Terminal` the app drives, abstracted for testability.
pub trait TerminalIo {
    fn write(&self, data: &str);
    fn columns(&self) -> u16;
    fn rows(&self) -> u16;
    fn hide_cursor(&self);
    fn show_cursor(&self);
    fn start(
        &mut self,
        on_input: Box<dyn FnMut(String) + Send + 'static>,
        on_resize: Box<dyn FnMut() + Send + 'static>,
    ) -> std::io::Result<()>;
    fn stop(&mut self);
    fn drain_input(&mut self, max_ms: u64, idle_ms: u64);
    /// Called by the reader thread on SIGINT/SIGTERM (restore happens in
    /// `stop()`; the TS equivalent is `process.on("SIGINT", ...)`).
    fn set_exit_signal_callback(&mut self, cb: Option<Box<dyn FnMut() + Send + 'static>>);
}

impl TerminalIo for crate::terminal::Terminal {
    fn write(&self, data: &str) {
        self.write(data);
    }
    fn columns(&self) -> u16 {
        self.columns()
    }
    fn rows(&self) -> u16 {
        self.rows()
    }
    fn hide_cursor(&self) {
        self.hide_cursor();
    }
    fn show_cursor(&self) {
        self.show_cursor();
    }
    fn start(
        &mut self,
        on_input: Box<dyn FnMut(String) + Send + 'static>,
        on_resize: Box<dyn FnMut() + Send + 'static>,
    ) -> std::io::Result<()> {
        self.start(on_input, on_resize)
    }
    fn stop(&mut self) {
        self.stop();
    }
    fn drain_input(&mut self, max_ms: u64, idle_ms: u64) {
        self.drain_input(max_ms, idle_ms);
    }
    fn set_exit_signal_callback(&mut self, cb: Option<Box<dyn FnMut() + Send + 'static>>) {
        self.set_exit_signal_callback(cb);
    }
}

// ─── App commands (loop ↔ app messages) ─────────────────────────────────────

/// Input events from the terminal reader thread.
pub enum UiInput {
    Input(String),
    Resize,
    /// SIGINT/SIGTERM received (restore + exit).
    ExitSignal,
}

/// One-shot timer ids (TS `setTimeout` call sites).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerId {
    InitialPrompt,
    ReconnectRefresh,
}

/// Keybinding actions — the keybinding closures send these instead of
/// capturing the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Interrupt,
    ForceClear,
    CycleModel,
    ShowSessions,
    CycleThinking,
    ToggleThinking,
    ScrollChatUpPage,
    ScrollChatDownPage,
    ScrollChatUpLine,
    ScrollChatDownLine,
}

/// Overlay kinds whose selection is routed back to the loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    Sessions,
    Tree,
    Fork,
    Model,
    Settings,
}

/// Who requested the model list (different overlays are built).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelsPurpose {
    /// `/model` selector.
    Selector,
    /// `/scoped-models` configuration.
    Scoped,
}

/// Who requested the session list (different overlays are built).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionsPurpose {
    Browse,
    Tree,
}

/// Async results + overlay events applied by the app loop.
#[derive(Debug)]
pub enum UiCmd {
    // ── async results ─────────────────────────────────────────────────
    Refreshed(Result<RpcSessionState, String>),
    ModelsLoaded {
        result: Result<Vec<ModelInfo>, String>,
        purpose: ModelsPurpose,
    },
    SessionsLoaded {
        result: Result<Vec<SessionSummary>, String>,
        purpose: SessionsPurpose,
    },
    ForkMessagesLoaded(Result<Value, String>),
    SetModelDone {
        set_result: Result<(), String>,
        state: Option<RpcSessionState>,
    },
    ModelCycled {
        result: Result<Value, String>,
        state: Option<RpcSessionState>,
    },
    ThinkingCycled(Result<Value, String>),
    CompactDone(Result<String, String>),
    ReloadDone {
        result: Result<Value, String>,
        state: Option<RpcSessionState>,
    },
    SessionNamed(Result<(), String>),
    CwdSet {
        result: Result<(), String>,
        resolved: String,
    },
    ApprovalDone {
        result: Result<(), String>,
        kind: String, // "approved" | "rejected"
        request_id: String,
    },
    StopDone(Result<(), String>),
    QueuedCancelled {
        result: Result<(), String>,
        run_id: String,
    },
    StatusLoaded {
        state: Result<RpcSessionState, String>,
        models: Result<Vec<ModelInfo>, String>,
    },
    SessionSwitched {
        result: Result<(), String>,
        state: Option<RpcSessionState>,
        messages: Result<Value, String>,
        label: String,
    },
    TreeSelected {
        item: SelectItem,
    },
    ForkSelected {
        item: SelectItem,
    },
    ForkDone {
        fork_result: Result<Value, String>,
        state: Option<RpcSessionState>,
        messages: Result<Value, String>,
        label: String,
    },
    NewSessionDone {
        result: Result<Value, String>,
        state: Option<RpcSessionState>,
    },
    CloneDone {
        result: Result<Value, String>,
        state: Option<RpcSessionState>,
        messages: Result<Value, String>,
    },
    ModelSelected(SelectItem),
    PromptAck {
        local_id: String,
        result: Result<crate::rpc::types::RunAck, String>,
    },
    InitialPromptDone(Result<crate::rpc::types::RunAck, String>),

    // ── overlay events ────────────────────────────────────────────────
    OverlaySelect {
        kind: OverlayKind,
        item: SelectItem,
    },
    OverlayCancel,
    ScopedModelsSaved(Vec<String>),

    // ── input / keybinding events ─────────────────────────────────────
    Submit(String),
    InputChanged(String),
    InputEscape,
    KeyAction(KeyAction),
    AcItems(Vec<AutocompleteItem>),
}

// ─── App state ──────────────────────────────────────────────────────────────

/// TUI-local settings persisted to `~/.future/tui/settings.json`.
#[derive(Debug, Clone, Default)]
pub struct TuiSettings {
    pub default_model: Option<String>,
    pub default_thinking_level: Option<String>,
    pub default_permission_level: Option<String>,
    pub enabled_model_ids: Option<Vec<String>>,
}

impl TuiSettings {
    fn to_json(&self) -> Value {
        // Key order mirrors the TS object literal + late `enabledModelIds`
        // assignment (JSON.stringify preserves insertion order).
        let mut obj = Map::new();
        if let Some(m) = &self.default_model {
            obj.insert("defaultModel".into(), Value::String(m.clone()));
        }
        if let Some(t) = &self.default_thinking_level {
            obj.insert("defaultThinkingLevel".into(), Value::String(t.clone()));
        }
        if let Some(p) = &self.default_permission_level {
            obj.insert("defaultPermissionLevel".into(), Value::String(p.clone()));
        }
        if let Some(ids) = &self.enabled_model_ids {
            obj.insert(
                "enabledModelIds".into(),
                Value::Array(ids.iter().map(|s| Value::String(s.clone())).collect()),
            );
        }
        Value::Object(obj)
    }

    fn from_json(v: &Value) -> Self {
        TuiSettings {
            default_model: v
                .get("defaultModel")
                .and_then(Value::as_str)
                .map(String::from),
            default_thinking_level: v
                .get("defaultThinkingLevel")
                .and_then(Value::as_str)
                .map(String::from),
            default_permission_level: v
                .get("defaultPermissionLevel")
                .and_then(Value::as_str)
                .map(String::from),
            enabled_model_ids: v.get("enabledModelIds").and_then(Value::as_array).map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect()
            }),
        }
    }
}

struct AppState {
    model: String,
    thinking: String,
    streaming: bool,
    spinner_frame: usize,
    session_id: String,
    cwd: String,
    version: String,
    skills: Vec<String>,
    context_files: Vec<String>,
    extensions: Vec<String>,
    context_tokens: i64,
    context_window: i64,
    context_percent: f64,
    tokens_in: i64,
    tokens_out: i64,
    tokens_cache_r: i64,
    tokens_cache_w: i64,
    total_cost: f64,
    auto_compaction_enabled: bool,
    tool_start_time: Option<Instant>,
    active_tool_count: usize,
    explicit_session: bool,
}

impl Default for AppState {
    fn default() -> Self {
        AppState {
            model: String::new(),
            thinking: "off".into(),
            streaming: false,
            spinner_frame: 0,
            session_id: String::new(),
            cwd: String::new(),
            version: String::new(),
            skills: Vec::new(),
            context_files: Vec::new(),
            extensions: Vec::new(),
            context_tokens: 0,
            context_window: 0,
            context_percent: 0.0,
            tokens_in: 0,
            tokens_out: 0,
            tokens_cache_r: 0,
            tokens_cache_w: 0,
            total_cost: 0.0,
            auto_compaction_enabled: true,
            tool_start_time: None,
            active_tool_count: 0,
            explicit_session: false,
        }
    }
}

/// CLI session options passed to the App (index.ts parse result subset).
#[derive(Debug, Clone, Default)]
pub struct CliOptions {
    pub session: Option<String>,
    pub r#continue: bool,
    pub resume: bool,
    pub fork: Option<String>,
    pub initial_prompt: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusTarget {
    None,
    Input,
    Overlay(u64),
}

struct OverlayEntry {
    id: u64,
    component: Box<dyn Component>,
    options: OverlayOptions,
    pre_focus: FocusTarget,
    hidden: bool,
    focus_order: u64,
}

/// Stored pending approval (the TS keeps it for reference; the visible
/// surface is the chat message + input autofill).
#[allow(dead_code)]
struct PendingApproval {
    request_id: String,
    tool_name: String,
    title: String,
    summary: String,
    risk_level: String,
    requested_action: Option<Value>,
}

/// `{ consume?: boolean; data?: string }` from the input listener pipeline.
#[derive(Debug, Clone, Default)]
pub struct InputListenerResult {
    pub consume: bool,
    pub data: Option<String>,
}

type InputListener = Box<dyn FnMut(&str) -> Option<InputListenerResult> + 'static>;

const MIN_RENDER_INTERVAL: Duration = Duration::from_millis(33);
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(150);
const SEGMENT_RESET: &str = "\x1b[0m\x1b]8;;\x07"; // SGR reset + OSC 8 close

/// `crypto.randomUUID()`.
fn random_id() -> String {
    Uuid::new_v4().to_string()
}

/// Lexically resolve `.` / `..` path components (no filesystem access, so
/// the target need not exist yet) and clamp at the root like `cd ..` does at
/// `/`. Makes `/cwd ../../` (and `/cwd /a/../b`) resolve to a clean absolute
/// path before it reaches the agent.
fn normalize_path(path: &str) -> String {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in std::path::Path::new(path).components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out.display().to_string()
}

/// Collapse all whitespace runs to a single space and trim (TS
/// `sanitizeSessionName`).
fn sanitize_session_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending_space = false;
    for c in name.chars() {
        if c.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(c);
        }
    }
    out
}

/// JS `s.split(/\s+/)` — split on whitespace runs, preserving a leading
/// empty element (e.g. `" model".split(/\s+/)` → `["", "model"]`).
fn split_ws_js(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut in_run = false;
    for (i, c) in s.char_indices() {
        if c.is_whitespace() {
            if !in_run {
                out.push(&s[start..i]);
                in_run = true;
            }
        } else if in_run {
            start = i;
            in_run = false;
        }
    }
    if in_run {
        // Trailing whitespace: JS `split(/\s+/)` yields a trailing EMPTY
        // element ("cwd ../ " → ["cwd", "../", ""]). Pushing `&s[start..]`
        // here would re-emit the last token with the trailing space
        // attached (["cwd", "../", "../ "]), which turned `/cwd ../ ` into
        // arg "../ ../ " and corrupted the resolved cwd.
        out.push("");
    } else {
        out.push(&s[start..]);
    }
    out
}

/// `new Date(b.updated_at).getTime()` — comparable timestamp for session
/// sorting (RFC3339, or `"YYYY-MM-DD HH:MM:SS"`).
fn parse_updated_at(s: &str) -> i64 {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt.timestamp_millis();
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return dt.and_utc().timestamp_millis();
    }
    0
}

/// Termux detection: skip full redraw on height changes.
fn is_termux_session() -> bool {
    std::env::var("TERMUX_VERSION").is_ok()
}

// ─── The App ────────────────────────────────────────────────────────────────

pub struct App<T: TerminalIo> {
    terminal: T,
    client: Arc<GrpcClient>,
    op_tx: mpsc::UnboundedSender<UiCmd>,
    #[allow(dead_code)] // mirrors TS `this.theme` (components own their themes)
    theme: Theme,
    input: Input,
    chat: ChatArea,
    footer: Footer,
    overlay_stack: Vec<OverlayEntry>,
    next_overlay_id: u64,
    focus_order_counter: u64,
    focused: FocusTarget,
    input_listeners: Vec<InputListener>,
    autocomplete: AutocompletePopup,
    ac_manager: AutocompleteManager,
    keybindings: KeybindingManager,
    enabled_model_ids: Option<Vec<String>>,
    connection_lost: bool,
    tui_settings: TuiSettings,
    tui_settings_path: PathBuf,
    slash_commands: Vec<SlashCommand>,
    session_input_cache: HashMap<String, String>,
    state: AppState,
    running: bool,
    cli_options: CliOptions,
    cli_initial_prompt: Option<String>,
    pending_name_arg: Option<String>,
    pub on_debug: Option<Box<dyn FnMut() + 'static>>,

    // ── Render scheduler state ────────────────────────────────────────
    previous_lines: Vec<String>,
    cursor_row: usize,
    hardware_cursor_row: usize,
    max_lines_rendered: usize,
    previous_viewport_top: usize,
    clear_on_shrink: bool,
    force_clear_next_render: bool,
    show_hardware_cursor: bool,
    render_requested: bool,
    render_now: bool,
    render_deadline: Option<Instant>,
    last_render_at: Instant,
    resize_deadline: Option<Instant>,
    ac_query_deadline: Option<Instant>,
    pending_ac_query: Option<(String, usize)>,
    timers: Vec<(Instant, TimerId)>,
    previous_width: usize,
    previous_height: usize,
    previous_kitty_image_ids: BTreeSet<u32>,
    full_redraw_count: usize,
    pending_approval: Option<PendingApproval>,
    cached_models: Vec<String>,
    cached_sessions: Vec<String>,
    #[allow(dead_code)] // mirrors TS `performance.now()` origin anchor
    start_time: Instant,
}

impl<T: TerminalIo> App<T> {
    pub fn new(
        terminal: T,
        client: Arc<GrpcClient>,
        op_tx: mpsc::UnboundedSender<UiCmd>,
        cli_options: &CliOptions,
        tui_settings_path: PathBuf,
    ) -> Self {
        let terminal_width = terminal.columns() as usize;
        let mut chat = ChatArea::new(terminal_width, None);
        let mut footer = Footer::new(terminal_width);
        let mut input = Input::new();
        input.focused = true;

        // Callbacks → UiCmd messages (no self-capture).
        let tx = op_tx.clone();
        input.onSubmit = Some(Box::new(move |v: &str| {
            let _ = tx.send(UiCmd::Submit(v.to_string()));
        }));
        let tx = op_tx.clone();
        input.onChange = Some(Box::new(move |v: &str| {
            let _ = tx.send(UiCmd::InputChanged(v.to_string()));
        }));
        let tx = op_tx.clone();
        input.onEscape = Some(Box::new(move || {
            let _ = tx.send(UiCmd::InputEscape);
        }));
        let _ = &mut chat;
        let _ = &mut footer;
        let _ = cli_options;
        let _ = &mut input;

        let mut app = App {
            terminal,
            client,
            op_tx,
            theme: DARK_THEME,
            input,
            chat,
            footer,
            overlay_stack: Vec::new(),
            next_overlay_id: 1,
            focus_order_counter: 0,
            focused: FocusTarget::Input,
            input_listeners: Vec::new(),
            autocomplete: AutocompletePopup::new(),
            ac_manager: AutocompleteManager::new(),
            keybindings: KeybindingManager::new(),
            enabled_model_ids: None,
            connection_lost: false,
            tui_settings: TuiSettings::default(),
            tui_settings_path,
            slash_commands: Vec::new(),
            session_input_cache: HashMap::new(),
            state: AppState::default(),
            running: false,
            cli_options: cli_options.clone(),
            cli_initial_prompt: cli_options.initial_prompt.clone(),
            pending_name_arg: None,
            on_debug: None,
            previous_lines: Vec::new(),
            cursor_row: 0,
            hardware_cursor_row: 0,
            max_lines_rendered: 0,
            previous_viewport_top: 0,
            clear_on_shrink: std::env::var("PI_CLEAR_ON_SHRINK").as_deref() == Ok("1"),
            force_clear_next_render: false,
            show_hardware_cursor: std::env::var("PI_HARDWARE_CURSOR").as_deref() == Ok("1"),
            render_requested: false,
            render_now: false,
            render_deadline: None,
            last_render_at: Instant::now(),
            resize_deadline: None,
            ac_query_deadline: None,
            pending_ac_query: None,
            timers: Vec::new(),
            previous_width: 0,
            previous_height: 0,
            previous_kitty_image_ids: BTreeSet::new(),
            full_redraw_count: 0,
            pending_approval: None,
            cached_models: Vec::new(),
            cached_sessions: Vec::new(),
            start_time: Instant::now(),
        };
        app.setup();
        app
    }

    fn setup(&mut self) {
        // Slash commands for autocomplete (with model/session arg flags).
        self.slash_commands = vec![
            SlashCommand {
                value: "/cwd".into(),
                label: "/cwd".into(),
                description: "change working directory".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/approve".into(),
                label: "/approve".into(),
                description: "approve pending tool execution".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/reject".into(),
                label: "/reject".into(),
                description: "reject pending tool execution".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/stop".into(),
                label: "/stop".into(),
                description: "stop current generation".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/cancel".into(),
                label: "/cancel".into(),
                description: "cancel a queued run".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/status".into(),
                label: "/status".into(),
                description: "show session and model info".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/model".into(),
                label: "/model".into(),
                description: "select model".into(),
                takes_model_arg: true,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/sessions".into(),
                label: "/sessions".into(),
                description: "browse sessions".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/new".into(),
                label: "/new".into(),
                description: "new session".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/clone".into(),
                label: "/clone".into(),
                description: "clone session".into(),
                takes_model_arg: false,
                takes_session_arg: true,
            },
            SlashCommand {
                value: "/fork".into(),
                label: "/fork".into(),
                description: "fork session".into(),
                takes_model_arg: false,
                takes_session_arg: true,
            },
            SlashCommand {
                value: "/tree".into(),
                label: "/tree".into(),
                description: "session tree".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/name".into(),
                label: "/name".into(),
                description: "set session name".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/scoped-models".into(),
                label: "/scoped-models".into(),
                description: "configure model scope".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/compact".into(),
                label: "/compact".into(),
                description: "compress conversation context".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/reload".into(),
                label: "/reload".into(),
                description: "reload skills + context".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/help".into(),
                label: "/help".into(),
                description: "show help".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
        ];

        // Autocomplete manager → popup (via UiCmd to avoid self-capture).
        let tx = self.op_tx.clone();
        self.ac_manager
            .set_on_items(Box::new(move |items: &[AutocompleteItem]| {
                let _ = tx.send(UiCmd::AcItems(items.to_vec()));
            }));

        // Register autocomplete providers. Model/session lookups are sync
        // caches refreshed by the app loop (the TS providers await RPCs).
        let cached_models = self.cached_models.clone();
        let cached_sessions = self.cached_sessions.clone();
        let get_models =
            Some(Box::new(move || cached_models.clone()) as Box<dyn Fn() -> Vec<String>>);
        let get_sessions =
            Some(Box::new(move || cached_sessions.clone()) as Box<dyn Fn() -> Vec<String>>);
        self.ac_manager.register(Box::new(SlashCommandProvider::new(
            self.slash_commands.clone(),
            get_models,
            get_sessions,
        )));
        let cwd = if self.state.cwd.is_empty() {
            std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        } else {
            self.state.cwd.clone()
        };
        self.ac_manager
            .register(Box::new(FilePathProvider::new(Some(cwd))));
        self.ac_manager.register(Box::new(AttachmentProvider));

        // Register global keybindings (actions route through UiCmd).
        let tx = self.op_tx.clone();
        self.keybindings.add(
            Key::CTRL_C,
            Box::new(move || {
                let _ = tx.send(UiCmd::KeyAction(KeyAction::Interrupt));
                true
            }),
            "Interrupt / exit",
            None,
        );
        let tx = self.op_tx.clone();
        self.keybindings.add(
            Key::CTRL_L,
            Box::new(move || {
                let _ = tx.send(UiCmd::KeyAction(KeyAction::ForceClear));
                true
            }),
            "Clear screen / redraw",
            None,
        );
        let tx = self.op_tx.clone();
        self.keybindings.add(
            Key::CTRL_P,
            Box::new(move || {
                let _ = tx.send(UiCmd::KeyAction(KeyAction::CycleModel));
                true
            }),
            "Cycle model",
            None,
        );
        let tx = self.op_tx.clone();
        self.keybindings.add(
            Key::CTRL_R,
            Box::new(move || {
                let _ = tx.send(UiCmd::KeyAction(KeyAction::ShowSessions));
                true
            }),
            "Browse sessions",
            None,
        );
        let tx = self.op_tx.clone();
        self.keybindings.add(
            Key::CTRL_T,
            Box::new(move || {
                let _ = tx.send(UiCmd::KeyAction(KeyAction::CycleThinking));
                true
            }),
            "Cycle thinking",
            None,
        );
        let tx = self.op_tx.clone();
        self.keybindings.add(
            Key::SHIFT_TAB,
            Box::new(move || {
                let _ = tx.send(UiCmd::KeyAction(KeyAction::CycleThinking));
                true
            }),
            "Cycle thinking",
            None,
        );
        let tx = self.op_tx.clone();
        self.keybindings.add(
            Key::CTRL_O,
            Box::new(move || {
                let _ = tx.send(UiCmd::KeyAction(KeyAction::ToggleThinking));
                true
            }),
            "Expand/collapse thinking",
            None,
        );
        let tx = self.op_tx.clone();
        self.keybindings.add(
            Key::PAGE_UP,
            Box::new(move || {
                let _ = tx.send(UiCmd::KeyAction(KeyAction::ScrollChatUpPage));
                true
            }),
            "Scroll chat up",
            None,
        );
        let tx = self.op_tx.clone();
        self.keybindings.add(
            Key::PAGE_DOWN,
            Box::new(move || {
                let _ = tx.send(UiCmd::KeyAction(KeyAction::ScrollChatDownPage));
                true
            }),
            "Scroll chat down",
            None,
        );
        let tx = self.op_tx.clone();
        self.keybindings.add(
            Key::CTRL_UP,
            Box::new(move || {
                let _ = tx.send(UiCmd::KeyAction(KeyAction::ScrollChatUpLine));
                true
            }),
            "Scroll chat up (line)",
            None,
        );
        let tx = self.op_tx.clone();
        self.keybindings.add(
            Key::CTRL_DOWN,
            Box::new(move || {
                let _ = tx.send(UiCmd::KeyAction(KeyAction::ScrollChatDownLine));
                true
            }),
            "Scroll chat down (line)",
            None,
        );
    }

    // ─── Loop plumbing ─────────────────────────────────────────────────

    /// Public state accessors (used by the loop / tests).
    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn get_full_redraw_count(&self) -> usize {
        self.full_redraw_count
    }

    /// Apply an async result / overlay event (called by the app loop).
    pub fn handle_cmd(&mut self, cmd: UiCmd) {
        match cmd {
            UiCmd::Refreshed(result) => match result {
                Ok(state) => self.apply_refresh_state(state),
                Err(_) => self.apply_refresh_error(),
            },
            UiCmd::ModelsLoaded { result, purpose } => match result {
                Ok(models) => match purpose {
                    ModelsPurpose::Selector => self.show_model_selector_overlay(models),
                    ModelsPurpose::Scoped => self.show_scoped_models_overlay(models),
                },
                Err(err) => self.add_system_message(format!("Failed to load models: {err}")),
            },
            UiCmd::SessionsLoaded { result, purpose } => match result {
                Ok(sessions) => match purpose {
                    SessionsPurpose::Browse => self.show_sessions_overlay(sessions),
                    SessionsPurpose::Tree => self.show_tree_overlay(sessions),
                },
                Err(err) => self.add_system_message(format!("Failed to load sessions: {err}")),
            },
            UiCmd::ForkMessagesLoaded(result) => match result {
                Ok(value) => self.show_fork_overlay(value),
                Err(err) => self.add_system_message(format!("Failed to load fork messages: {err}")),
            },
            UiCmd::SetModelDone { set_result, state } => {
                if let Err(err) = set_result {
                    self.add_system_message(format!("Failed to set model: {err}"));
                } else {
                    if let Some(s) = state {
                        self.apply_refresh_state(s);
                    }
                    self.tui_settings.default_model = Some(self.state.model.clone());
                    self.save_tui_settings();
                    self.add_system_message(format!("Model: {}", self.state.model));
                }
                self.request_render(false);
            }
            UiCmd::ModelCycled { result, state } => {
                if result.is_ok() {
                    if let Some(s) = state {
                        self.apply_refresh_state(s);
                    }
                }
                self.request_render(false);
            }
            UiCmd::ThinkingCycled(Ok(value)) => {
                if let Some(level) = value.get("level").and_then(Value::as_str) {
                    self.state.thinking = level.to_string();
                    self.tui_settings.default_thinking_level = Some(level.to_string());
                    self.save_tui_settings();
                }
            }
            UiCmd::ThinkingCycled(Err(_)) => {}
            UiCmd::CompactDone(result) => match result {
                Ok(_) => self.add_system_message("Context compacted".into()),
                Err(err) => self.add_system_message(format!("Compact failed: {err}")),
            },
            UiCmd::ReloadDone { result, state } => match result {
                Ok(value) => {
                    if let Some(s) = state {
                        self.apply_refresh_state(s);
                    }
                    let skill_list = value
                        .get("skills")
                        .and_then(Value::as_array)
                        .map(|a| a.len())
                        .unwrap_or(0);
                    let skill_text = if skill_list > 0 {
                        format!("{skill_list} skills loaded")
                    } else {
                        "no skills found".to_string()
                    };
                    let ctx = value
                        .get("contextFiles")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    let ctx_text = if ctx.is_empty() {
                        String::new()
                    } else {
                        format!(", {ctx}")
                    };
                    self.add_system_message(format!("Reloaded: {skill_text}{ctx_text}"));
                }
                Err(err) => self.add_system_message(format!("Reload failed: {err}")),
            },
            UiCmd::SessionNamed(result) => match result {
                Ok(_) => {
                    self.add_system_message(format!(
                        "Session name set to: {}",
                        self.pending_name_arg.clone().unwrap_or_default()
                    ));
                    self.pending_name_arg = None;
                }
                Err(err) => {
                    self.add_system_message(format!("Failed to set session name: {err}"));
                    self.pending_name_arg = None;
                }
            },
            UiCmd::CwdSet { result, resolved } => match result {
                Ok(_) => {
                    self.state.cwd = resolved.clone();
                    self.add_system_message(format!("Working directory: {resolved}"));
                }
                Err(err) => self.add_system_message(format!("Failed to change directory: {err}")),
            },
            UiCmd::ApprovalDone {
                result,
                kind,
                request_id,
            } => match result {
                Ok(_) => {
                    let verb = if kind == "approved" {
                        "Approved"
                    } else {
                        "Rejected"
                    };
                    self.add_system_message(format!("{verb} request: {request_id}"));
                }
                Err(err) => {
                    let verb = if kind == "approved" {
                        "approve"
                    } else {
                        "reject"
                    };
                    self.add_system_message(format!("Failed to {verb}: {err}"));
                }
            },
            UiCmd::StopDone(result) => match result {
                Ok(_) => self.add_system_message("Stopped current generation.".into()),
                Err(err) => self.add_system_message(format!("Failed to stop: {err}")),
            },
            UiCmd::QueuedCancelled { result, run_id } => match result {
                Ok(_) => {
                    self.chat.update_run_state(&run_id, RunState::Cancelled);
                    self.add_system_message(format!("Cancelled queued run: {run_id}"));
                }
                Err(err) => self.add_system_message(format!("Failed to cancel queued run: {err}")),
            },
            UiCmd::StatusLoaded { state, models } => match (state, models) {
                (Ok(s), Ok(models)) => self.apply_status(&s, &models),
                (Ok(s), Err(_)) => self.apply_status(&s, &[]),
                (Err(err), _) => self.add_system_message(format!("Failed to get status: {err}")),
            },
            UiCmd::SessionSwitched {
                result,
                state,
                messages,
                label,
            } => {
                if let Err(err) = result {
                    self.add_system_message(format!("Failed to switch session: {err}"));
                } else {
                    if let Some(s) = state {
                        self.apply_refresh_state(s);
                    }
                    self.restore_session_input();
                    self.apply_messages(messages);
                    self.add_system_message(format!("Switched to session: {label}"));
                }
                self.hide_overlay();
            }
            UiCmd::TreeSelected { item } => {
                if item.value != self.state.session_id {
                    self.save_session_input();
                    self.spawn_switch_flow(&item.value, item.label.clone());
                }
                self.hide_overlay();
            }
            UiCmd::ForkSelected { item } => {
                self.save_session_input();
                let client = self.client.clone();
                let tx = self.op_tx.clone();
                let entry_id = item.value.clone();
                let label = item.label.clone();
                tokio::spawn(async move {
                    let fork_result = client.fork(&entry_id).await;
                    let mut state = None;
                    let mut messages = Ok(Value::Null);
                    if let Ok(ref v) = fork_result {
                        let cancelled =
                            v.get("cancelled").and_then(Value::as_bool).unwrap_or(false);
                        if !cancelled {
                            state = client.get_state().await.ok();
                            messages = client.get_messages().await;
                        }
                    }
                    let _ = tx.send(UiCmd::ForkDone {
                        fork_result,
                        state,
                        messages,
                        label,
                    });
                });
            }
            UiCmd::ForkDone {
                fork_result,
                state,
                messages,
                label,
            } => {
                match fork_result {
                    Ok(v) => {
                        let cancelled =
                            v.get("cancelled").and_then(Value::as_bool).unwrap_or(false);
                        if !cancelled {
                            if let Some(s) = state {
                                self.apply_refresh_state(s);
                            }
                            self.restore_session_input();
                            self.apply_messages(messages);
                            self.add_system_message(format!("Forked from {label}."));
                        }
                    }
                    Err(err) => self.add_system_message(format!("Failed to fork: {err}")),
                }
                self.hide_overlay();
            }
            UiCmd::NewSessionDone { result, state } => match result {
                Ok(v) => {
                    if v.get("sessionId").and_then(Value::as_str).is_some() {
                        if let Some(s) = state {
                            self.apply_refresh_state(s);
                        }
                        self.restore_session_input();
                        self.add_system_message("New session started.".into());
                    }
                }
                Err(_) => self.add_system_message("Not connected to agent.".into()),
            },
            UiCmd::CloneDone {
                result,
                state,
                messages,
            } => match result {
                Ok(v) => {
                    let cancelled = v.get("cancelled").and_then(Value::as_bool).unwrap_or(false);
                    if !cancelled {
                        if let Some(s) = state {
                            self.apply_refresh_state(s);
                        }
                        self.apply_messages(messages);
                        self.add_system_message("Session cloned — continue in new branch.".into());
                    }
                }
                Err(err) => self.add_system_message(format!("Failed to clone session: {err}")),
            },
            UiCmd::ModelSelected(item) => {
                let client = self.client.clone();
                let tx = self.op_tx.clone();
                let value = item.value.clone();
                tokio::spawn(async move {
                    let set_result = client.set_model(&value).await;
                    let state = client.get_state().await.ok();
                    let _ = tx.send(UiCmd::SetModelDone { set_result, state });
                });
                self.hide_overlay();
            }
            UiCmd::PromptAck { local_id, result } => match result {
                Ok(ack) => {
                    let state = if ack.accepted_state == "queued" {
                        RunState::Queued
                    } else {
                        RunState::Running
                    };
                    self.chat.bind_user_run(
                        &local_id,
                        &ack.run_id,
                        state,
                        ack.queue_position.map(|q| q as u32),
                    );
                    self.request_render(false);
                }
                Err(err) => {
                    self.chat.set_message_run_state(&local_id, RunState::Failed);
                    self.state.streaming = false;
                    let msg = err;
                    let is_transport = msg.contains("transport")
                        || msg.contains("14 UNAVAILABLE")
                        || msg.contains("Connect Failed")
                        || msg.contains("ECONNREFUSED");
                    if !is_transport {
                        self.add_system_message(
                            "Not connected to agent. Start the agent or check the gRPC connection."
                                .into(),
                        );
                    }
                    self.request_render(false);
                }
            },
            UiCmd::InitialPromptDone(_) => {}
            UiCmd::OverlaySelect { kind, item } => match kind {
                OverlayKind::Sessions => {
                    self.save_session_input();
                    self.spawn_switch_flow(&item.value, item.label.clone());
                }
                OverlayKind::Tree => {
                    if item.value != self.state.session_id {
                        self.save_session_input();
                        self.spawn_switch_flow(&item.value, item.label.clone());
                    } else {
                        self.hide_overlay();
                    }
                }
                OverlayKind::Fork => self.handle_cmd(UiCmd::ForkSelected { item }),
                OverlayKind::Model => self.handle_cmd(UiCmd::ModelSelected(item)),
                OverlayKind::Settings => match item.value.as_str() {
                    "sessions" => {
                        self.hide_overlay();
                        self.show_sessions();
                    }
                    "reload" => {
                        self.hide_overlay();
                        self.spawn_refresh();
                        self.add_system_message("Settings reloaded".into());
                    }
                    _ => self.hide_overlay(),
                },
            },
            UiCmd::OverlayCancel => {
                self.hide_overlay();
            }
            UiCmd::ScopedModelsSaved(enabled_ids) => {
                self.enabled_model_ids = Some(enabled_ids.clone());
                self.tui_settings.enabled_model_ids = Some(enabled_ids.clone());
                self.save_tui_settings();
                let total = self.cached_models.len();
                self.add_system_message(format!(
                    "Model scope saved ({}/{} enabled)",
                    enabled_ids.len(),
                    total
                ));
                self.hide_overlay();
            }
            UiCmd::Submit(value) => self.handle_submit(&value),
            UiCmd::InputChanged(value) => self.handle_input_changed(&value),
            UiCmd::InputEscape => {
                self.input.set_value("", None);
                self.request_render(false);
            }
            UiCmd::KeyAction(action) => self.handle_key_action(action),
            UiCmd::AcItems(items) => {
                if items.is_empty() {
                    self.autocomplete.hide();
                } else {
                    self.autocomplete.show(items);
                }
                self.request_render(false);
            }
        }
    }

    // ─── Tick / timers ─────────────────────────────────────────────────

    /// Earliest pending deadline (for the loop's sleep).
    pub fn next_deadline(&self) -> Option<Instant> {
        let mut d = [
            self.render_deadline,
            self.resize_deadline,
            self.ac_query_deadline,
        ]
        .into_iter()
        .flatten()
        .min();
        if let Some((at, _)) = self.timers.first() {
            d = Some(match d {
                Some(cur) => cur.min(*at),
                None => *at,
            });
        }
        d
    }

    /// Periodic loop tick: fire due timers, run the render scheduler.
    pub fn on_tick(&mut self) {
        let now = Instant::now();

        if let Some(d) = self.resize_deadline {
            if now >= d {
                self.resize_deadline = None;
                self.request_render(false);
            }
        }

        if let Some(d) = self.ac_query_deadline {
            if now >= d {
                self.ac_query_deadline = None;
                if let Some((text, cursor)) = self.pending_ac_query.take() {
                    self.ac_manager.query(&text, cursor);
                }
            }
        }

        let mut due = Vec::new();
        self.timers.retain(|(at, id)| {
            if *at <= now {
                due.push(*id);
                false
            } else {
                true
            }
        });
        for id in due {
            self.fire_timer(id);
        }

        if self.render_now
            || (self.render_requested && self.render_deadline.is_some_and(|d| now >= d))
        {
            self.render_now = false;
            self.render_requested = false;
            self.render_deadline = None;
            self.last_render_at = now;
            self.do_render();
            if self.state.streaming {
                self.request_render(false);
            }
        }
    }

    fn fire_timer(&mut self, id: TimerId) {
        match id {
            TimerId::InitialPrompt => {
                let message = self.cli_initial_prompt.clone();
                let client = self.client.clone();
                let tx = self.op_tx.clone();
                if let Some(message) = message {
                    tokio::spawn(async move {
                        let result = client.prompt(&message, "reject_if_busy").await;
                        let _ = tx.send(UiCmd::InitialPromptDone(result));
                    });
                }
            }
            TimerId::ReconnectRefresh => {
                self.spawn_refresh();
            }
        }
    }

    // ─── Lifecycle ─────────────────────────────────────────────────────

    /// Startup: load settings, enter raw mode, wait for the agent (polling
    /// every 1 s, showing "Connecting…" on first failure), establish the
    /// session, apply TUI defaults, show the welcome screen.
    pub async fn start(&mut self, input_tx: mpsc::UnboundedSender<UiInput>) -> std::io::Result<()> {
        self.load_tui_settings();
        self.terminal.hide_cursor();
        self.running = true;
        self.query_cell_size();

        // Terminal manages stdin, emits complete sequences via onInput callback.
        let tx = input_tx.clone();
        let tx2 = input_tx.clone();
        self.terminal.start(
            Box::new(move |data: String| {
                let _ = tx.send(UiInput::Input(data));
            }),
            Box::new(move || {
                let _ = tx2.send(UiInput::Resize);
            }),
        )?;

        self.wait_for_agent().await;
        if !self.running {
            return Ok(());
        }

        // Handle CLI session options (session / continue / fork / resume).
        self.handle_startup_session().await;

        // Connection state changes + stream events are delivered through the
        // client's channels, which the app loop (index.rs) polls — the TS
        // `onConnectionChange`/`subscribe` wiring is implicit here.

        self.apply_tui_defaults().await;
        self.show_welcome();
        self.request_render(false);
        Ok(())
    }

    async fn wait_for_agent(&mut self) {
        let mut first_attempt = true;
        while self.running {
            if self.client.try_connect().await {
                if !first_attempt {
                    self.chat.add_message(ChatMessage::new(
                        random_id(),
                        ChatRole::System,
                        "✅  Connected to agent",
                    ));
                    self.request_render(false);
                }
                return;
            }
            if first_attempt {
                self.chat.add_message(ChatMessage::new(
                    random_id(),
                    ChatRole::System,
                    "Connecting to agent… (retrying every 1s)",
                ));
                self.request_render(false);
                tokio::time::sleep(Duration::from_millis(50)).await;
                first_attempt = false;
            }
            tokio::time::sleep(Duration::from_millis(1000)).await;
        }
    }

    async fn handle_startup_session(&mut self) {
        let opts = self.cli_options.clone();
        let initial_prompt = self.cli_initial_prompt.clone();
        if let Some(session) = opts.session {
            // --session: switch to specific session.
            self.state.explicit_session = true;
            match self.client.switch_session(&session).await {
                Ok(_) => {
                    self.refresh_direct().await;
                    self.load_messages_direct().await;
                }
                Err(err) => {
                    self.add_system_message(format!(
                        "Failed to switch to session {session}: {err}"
                    ));
                }
            }
        } else if opts.r#continue {
            // --continue: find most recent session and continue.
            self.state.explicit_session = true;
            match self.client.list_sessions().await {
                Ok(sessions) => {
                    if !sessions.is_empty() {
                        let mut sessions = sessions;
                        sessions.sort_by(|a, b| {
                            parse_updated_at(&b.updated_at).cmp(&parse_updated_at(&a.updated_at))
                        });
                        match self.client.switch_session(&sessions[0].id).await {
                            Ok(_) => {
                                self.refresh_direct().await;
                                self.load_messages_direct().await;
                            }
                            Err(err) => {
                                self.add_system_message(format!(
                                    "Failed to continue session: {err}"
                                ));
                            }
                        }
                    }
                }
                Err(err) => {
                    self.add_system_message(format!("Failed to continue session: {err}"));
                }
            }
        } else if let Some(fork) = opts.fork {
            // --fork: fork from specific session.
            self.state.explicit_session = true;
            match self.client.fork(&fork).await {
                Ok(_) => {
                    self.refresh_direct().await;
                }
                Err(err) => {
                    self.add_system_message(format!("Failed to fork session {fork}: {err}"));
                }
            }
        } else if opts.resume {
            // --resume: show session picker (handled by showSessions).
            self.state.explicit_session = true;
            self.refresh_direct().await;
            self.show_sessions();
        } else {
            // No explicit session option — create a new session. Reload
            // skills first so getState returns the latest list.
            let _ = self.client.reload_config().await;
            self.refresh_direct().await;
            if !self.state.explicit_session {
                match self.client.new_session(None, None, None).await {
                    Ok(v) => {
                        if v.get("sessionId").and_then(Value::as_str).is_some() {
                            // Server created a new session — re-read state.
                            self.refresh_direct().await;
                        }
                    }
                    Err(_) => {
                        // Server may not support new_session — continue with
                        // the current session.
                    }
                }
            }
        }

        // Handle initial prompt (non-empty messages from CLI without -p flag).
        if initial_prompt.is_some() {
            let at = Instant::now() + Duration::from_millis(100);
            self.timers.push((at, TimerId::InitialPrompt));
            self.timers.sort_by_key(|(at, _)| *at);
        }
    }

    /// Awaited refresh (startup path): `await this.refresh()`.
    async fn refresh_direct(&mut self) {
        match self.client.get_state().await {
            Ok(state) => self.apply_refresh_state(state),
            Err(_) => self.apply_refresh_error(),
        }
    }

    /// Awaited session-message load (startup path):
    /// `await this.loadSessionMessages()`.
    async fn load_messages_direct(&mut self) {
        if let Ok(messages) = self.client.get_messages().await {
            self.apply_messages(Ok(messages));
        }
    }

    pub async fn stop_async(&mut self) {
        self.stop();
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.render_requested = false;
        self.render_now = false;
        self.render_deadline = None;
        self.resize_deadline = None;
        self.ac_query_deadline = None;
        self.pending_ac_query = None;
        self.timers.clear();
        self.client.disconnect();

        // Drain stdin to prevent key release leaks, then clean up terminal state.
        self.terminal.drain_input(1000, 50);
        self.terminal.stop();

        // Move cursor to end of content.
        if !self.previous_lines.is_empty() {
            let target_row = self.previous_lines.len();
            let line_diff = target_row as i64 - self.hardware_cursor_row as i64;
            if line_diff > 0 {
                self.terminal.write(&format!("\x1b[{line_diff}B"));
            } else if line_diff < 0 {
                self.terminal.write(&format!("\x1b[{}A", -line_diff));
            }
            self.terminal.write("\r\n");
        }
        self.terminal.show_cursor();
    }

    // ─── Agent event handling ──────────────────────────────────────────

    pub fn handle_agent_event(&mut self, event: &AgentEvent) {
        match event.r#type.as_str() {
            "user_message" => {
                let text = event.text().to_string();
                // Dedup: the sender TUI already added this message locally
                // before sending the RPC, so its own broadcast would create
                // a duplicate. Observing TUIs (different client, same
                // session) see it for the first time.
                let last = self.chat.last_message().cloned();
                if let Some(last) = last {
                    if last.role == ChatRole::User && last.content == text {
                        self.request_render(false);
                        return;
                    }
                }
                self.chat
                    .add_message(ChatMessage::new(random_id(), ChatRole::User, &text));
            }
            "text_chunk" => {
                self.state.streaming = true;
                self.chat.append_to_last_message(event.text());
            }
            "agent_end" => {
                if let Some(run_id) = &event.run_id {
                    self.chat.update_run_state(run_id, RunState::Terminal);
                }
                self.state.streaming = self.client.has_running_run();
                self.state.active_tool_count = 0;
                self.state.tool_start_time = None;
                let text = event.text();
                if !text.is_empty() {
                    self.chat.update_last_message(text);
                }
                // Mark the assistant message as complete so pending→false and
                // the full markdown render replaces the streaming render.
                self.chat.mark_last_message_complete();
                // Refresh state to update context percentage, token totals.
                self.spawn_refresh();
            }
            "agent_start" => {
                if let Some(run_id) = &event.run_id {
                    self.chat.update_run_state(run_id, RunState::Running);
                }
                self.state.streaming = true;
                self.state.active_tool_count = 0;
                self.state.tool_start_time = None;
                self.chat
                    .add_message(ChatMessage::new(random_id(), ChatRole::Assistant, ""));
            }
            "thinking_start" => {
                self.state.streaming = true;
                self.chat.start_thinking();
            }
            "thinking_delta" => {
                self.chat.append_thinking_delta(event.text());
            }
            "thinking_end" => {
                self.chat.end_thinking();
            }
            "tool_start" => {
                let tool_id = event
                    .data
                    .get("tool_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let tool_name = event
                    .data
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let tool_args = match event.data.get("tool_args") {
                    Some(Value::String(s)) => Some(s.clone()),
                    Some(v @ Value::Object(_)) => serde_json::to_string(v).ok(),
                    _ => None,
                };
                self.chat.add_tool_start(&tool_id, &tool_name, tool_args);
                if self.state.active_tool_count == 0 {
                    self.state.tool_start_time = Some(Instant::now());
                }
                self.state.active_tool_count += 1;
            }
            "tool_delta" => {
                let tool_id = event
                    .data
                    .get("tool_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.chat.append_tool_delta(&tool_id, event.text());
            }
            "tool_end" => {
                let tool_id = event
                    .data
                    .get("tool_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let text = event.data.get("text").and_then(Value::as_str);
                self.chat.finish_tool(&tool_id, text);
                self.state.active_tool_count = self.state.active_tool_count.saturating_sub(1);
                if self.state.active_tool_count == 0 {
                    self.state.tool_start_time = None;
                }
                // Pull the latest cumulative cost/token totals so the footer
                // updates after every tool call, not just at agent_end.
                self.spawn_refresh();
            }
            "approval_request" => {
                let e = &event.data;
                self.show_approval_overlay(ApprovalEvent {
                    request_id: e
                        .get("approval_request_id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    tool_id: e
                        .get("tool_id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    tool_name: e
                        .get("tool_name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    kind: e
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    risk_level: e
                        .get("risk_level")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    title: e
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("Approve tool execution")
                        .to_string(),
                    summary: e
                        .get("summary")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    requested_action: e.get("requested_action").cloned(),
                });
            }
            "error" => {
                self.state.streaming = false;
                let msg = event
                    .data
                    .get("error")
                    .and_then(Value::as_str)
                    .or_else(|| event.data.get("error_message").and_then(Value::as_str))
                    .unwrap_or("unknown error");
                self.chat.add_message(ChatMessage::new(
                    random_id(),
                    ChatRole::System,
                    &format!("Error: {msg}"),
                ));
            }
            "usage" => {
                if let Some(usage) = event.data.get("usage") {
                    if let Some(v) = usage.get("prompt_tokens").and_then(Value::as_i64) {
                        self.state.tokens_in += v;
                    }
                    if let Some(v) = usage.get("completion_tokens").and_then(Value::as_i64) {
                        self.state.tokens_out += v;
                    }
                    if let Some(v) = usage.get("cache_read_tokens").and_then(Value::as_i64) {
                        self.state.tokens_cache_r += v;
                    }
                    if let Some(v) = usage.get("cache_write_tokens").and_then(Value::as_i64) {
                        self.state.tokens_cache_w += v;
                    }
                    let prompt = usage
                        .get("prompt_tokens")
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    let completion = usage
                        .get("completion_tokens")
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    self.state.context_tokens = prompt + completion;
                }
                // Pull latest cumulative cost/token totals from the agent.
                self.spawn_refresh();
            }
            // ── Settings-change events ────────────────────────────────
            "model_changed" => {
                if let Some(m) = event.data.get("model").and_then(Value::as_str) {
                    self.state.model = m.to_string();
                }
            }
            "thinking_level_changed" => {
                if let Some(l) = event.data.get("level").and_then(Value::as_str) {
                    self.state.thinking = l.to_string();
                }
            }
            "permission_level_changed" => {
                self.spawn_refresh();
            }
            "cwd_changed" => {
                if let Some(c) = event.data.get("cwd").and_then(Value::as_str) {
                    self.state.cwd = c.to_string();
                }
            }
            "session_name_changed" => {}
            "auto_compaction_changed" => {
                if let Some(v) = event.data.get("enabled").and_then(Value::as_bool) {
                    self.state.auto_compaction_enabled = v;
                }
            }
            "tools_changed" | "sandbox_policy_changed" | "ephemeral_changed" => {
                self.spawn_refresh();
            }
            "config_reloaded" => {
                if let Some(skills) = event.data.get("skills").and_then(Value::as_array) {
                    let mut list: Vec<String> = skills
                        .iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect();
                    list.sort();
                    self.state.skills = list;
                }
                if let Some(files) = event.data.get("contextFiles").and_then(Value::as_array) {
                    self.state.context_files = files
                        .iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect();
                }
                let skill_count = event
                    .data
                    .get("skills")
                    .and_then(Value::as_array)
                    .map(|a| a.len())
                    .unwrap_or(0);
                let ctx = event
                    .data
                    .get("contextFiles")
                    .and_then(Value::as_array)
                    .map(|a| {
                        let list: Vec<&str> = a.iter().filter_map(Value::as_str).collect();
                        list.join(", ")
                    })
                    .unwrap_or_default();
                let ctx_text = if ctx.is_empty() {
                    "no context files".to_string()
                } else {
                    ctx
                };
                self.chat.add_message(ChatMessage::new(
                    random_id(),
                    ChatRole::System,
                    &format!("Config reloaded: {skill_count} skills, {ctx_text}"),
                ));
            }
            _ => {}
        }
        self.request_render(false);
    }

    // ─── Input handling ────────────────────────────────────────────────

    /// Receives complete sequences from the terminal's StdinBuffer.
    pub fn handle_input(&mut self, data: &str) {
        // Cell size response.
        if self.consume_cell_size_response(data) {
            self.request_render(false);
            return;
        }

        // Filter key release events unless the focused component wants them.
        if is_key_release(data) {
            let wants = match self.focused {
                FocusTarget::Input => self.input.wants_key_release(),
                FocusTarget::Overlay(id) => self
                    .overlay_stack
                    .iter()
                    .find(|e| e.id == id)
                    .map(|e| e.component.wants_key_release())
                    .unwrap_or(false),
                FocusTarget::None => false,
            };
            if !wants {
                return;
            }
        }

        // Input listener pipeline.
        if !self.input_listeners.is_empty() {
            let mut d: Option<String> = Some(data.to_string());
            for listener in &mut self.input_listeners {
                let Some(cur) = d.take() else { break };
                let Some(result) = listener(&cur) else {
                    d = Some(cur);
                    continue;
                };
                if result.consume {
                    d = None;
                    break;
                }
                if result.data.is_some() {
                    d = result.data;
                } else {
                    d = Some(cur);
                }
            }
            if d.is_none() {
                return;
            }
            let owned = d.unwrap();
            // Continue with the (possibly rewritten) data — NOT recursively,
            // which would re-run the listeners and never terminate for a
            // pass-through listener.
            self.handle_input_continue(&owned);
            return;
        }

        self.handle_input_continue(data);
    }

    /// The post-listener input flow: paste, interrupt, key parse, fallback.
    fn handle_input_continue(&mut self, data: &str) {
        // Bracketed paste.
        if data.starts_with("\x1b[200~") {
            if let Some(end_idx) = data.find("\x1b[201~") {
                let content = &data[6..end_idx];
                if !self.overlay_stack.is_empty() {
                    let top = self.get_top_overlay_index();
                    if let Some(idx) = top {
                        self.overlay_stack[idx].component.handle_input(content);
                    }
                } else {
                    self.input.insert_text(content);
                }
                self.request_render(false);
            }
            return;
        }

        // Ctrl+C (interrupt) — check raw byte before parseKey for responsiveness.
        if data == "\x03" {
            self.handle_interrupt();
            return;
        }

        // Parse key through unified parser (Kitty CSI-u, modifyOtherKeys, legacy).
        if let Some(key_name) = parse_key(data) {
            // If focused component is a now-invisible overlay, redirect focus.
            let focused_overlay = self
                .overlay_stack
                .iter()
                .find(|o| Some(o.id) == self.overlay_id_of_focus())
                .map(|e| e.id);
            if let Some(id) = focused_overlay {
                if let Some(entry) = self.overlay_stack.iter().find(|e| e.id == id) {
                    if entry.hidden {
                        if let Some(top) = self.get_top_overlay_index() {
                            self.set_focus(FocusTarget::Overlay(self.overlay_stack[top].id));
                        } else {
                            self.set_focus(FocusTarget::Input);
                        }
                    }
                }
            }
            self.handle_key(&key_name);
            return;
        }

        // Fallback: printable character not covered by parseKey.
        let mut chars = data.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if (c as u32) >= 32 {
                if !self.overlay_stack.is_empty() {
                    if let Some(idx) = self.get_top_overlay_index() {
                        self.overlay_stack[idx]
                            .component
                            .handle_input(&c.to_string());
                        self.request_render(false);
                    }
                    return;
                }
                self.input.insert_text(&c.to_string());
                self.request_render(false);
            }
        }
    }

    fn overlay_id_of_focus(&self) -> Option<u64> {
        match self.focused {
            FocusTarget::Overlay(id) => Some(id),
            _ => None,
        }
    }

    fn handle_key(&mut self, key: &str) {
        // Shift+Ctrl+D — trigger debug callback.
        if key == "shift+ctrl+d" {
            if let Some(cb) = self.on_debug.as_mut() {
                cb();
            }
            return;
        }

        // Escape — close autocomplete or overlay or clear editor.
        if key == Key::ESCAPE {
            if self.autocomplete.is_visible() {
                self.autocomplete.hide();
                self.request_render(false);
            } else if !self.overlay_stack.is_empty() {
                self.hide_overlay();
                self.request_render(false);
            } else {
                self.input.set_value("", None);
                self.autocomplete.hide();
                self.request_render(false);
            }
            return;
        }

        // Overlay mode — dispatch to top overlay via handleInput.
        if !self.overlay_stack.is_empty() {
            if let Some(idx) = self.get_top_overlay_index() {
                self.overlay_stack[idx].component.handle_input(key);
            }
            self.request_render(false);
            return;
        }

        // Autocomplete navigation takes priority over chat scroll.
        if self.autocomplete.is_visible() {
            if key == Key::UP {
                self.autocomplete.select_prev();
                self.request_render(false);
                return;
            }
            if key == Key::DOWN {
                self.autocomplete.select_next();
                self.request_render(false);
                return;
            }
            if key == Key::ENTER {
                self.apply_autocomplete_selection();
                return;
            }
        }

        // Dispatch through keybinding manager (ctrl shortcuts, shift+tab, ...).
        if self.keybindings.dispatch(key, None) {
            self.request_render(false);
            return;
        }

        // Other ctrl+key combos — pass to editor.
        if key.starts_with("ctrl+") {
            if self.input.handle_key(key) {
                self.request_render(false);
            }
            return;
        }

        // Tab — autocomplete.
        if key == Key::TAB {
            if self.autocomplete.is_visible() {
                // Accept the highlighted completion into the input only —
                // do NOT submit (Tab is completion, not confirmation).
                self.apply_autocomplete_selection();
            } else {
                self.trigger_autocomplete();
            }
            return;
        }

        // Editor handles the rest.
        if self.input.handle_key(key) {
            self.request_render(false);
        }
    }

    pub fn handle_key_action(&mut self, action: KeyAction) {
        match action {
            KeyAction::Interrupt => self.handle_interrupt(),
            KeyAction::ForceClear => {
                self.force_clear_next_render = true;
                self.request_render(false);
            }
            KeyAction::CycleModel => {
                let client = self.client.clone();
                let tx = self.op_tx.clone();
                if self.state.streaming {
                    self.add_system_message("Cannot change model while agent is streaming.".into());
                    self.request_render(false);
                    return;
                }
                // If scoped models are set, cycle within them locally.
                if let Some(enabled) = self.enabled_model_ids.clone() {
                    if !enabled.is_empty() {
                        let current = self.state.model.clone();
                        let idx = enabled.iter().position(|m| *m == current);
                        let next_idx = match idx {
                            Some(i) => (i + 1) % enabled.len(),
                            None => 0,
                        };
                        let next_model = enabled[next_idx].clone();
                        let next_model_task = next_model.clone();
                        tokio::spawn(async move {
                            let set_result = client.set_model(&next_model_task).await;
                            let state = client.get_state().await.ok();
                            let _ = tx.send(UiCmd::SetModelDone { set_result, state });
                        });
                        self.state.model = next_model;
                        self.tui_settings.default_model = Some(self.state.model.clone());
                        self.save_tui_settings();
                        return;
                    }
                }
                tokio::spawn(async move {
                    let result = client.cycle_model().await;
                    let state = client.get_state().await.ok();
                    let _ = tx.send(UiCmd::ModelCycled { result, state });
                });
            }
            KeyAction::ShowSessions => {
                let client = self.client.clone();
                let tx = self.op_tx.clone();
                tokio::spawn(async move {
                    let _ = tx.send(UiCmd::SessionsLoaded {
                        result: client.list_sessions().await,
                        purpose: SessionsPurpose::Browse,
                    });
                });
            }
            KeyAction::CycleThinking => {
                if self.state.streaming {
                    self.add_system_message(
                        "Cannot change thinking level while agent is streaming.".into(),
                    );
                    self.request_render(false);
                    return;
                }
                let client = self.client.clone();
                let tx = self.op_tx.clone();
                tokio::spawn(async move {
                    let _ = tx.send(UiCmd::ThinkingCycled(client.cycle_thinking_level().await));
                });
            }
            KeyAction::ToggleThinking => {
                self.chat.toggle_thinking_hidden();
                self.request_render(false);
            }
            KeyAction::ScrollChatUpPage => {
                self.chat.scroll_up(self.terminal.rows() as usize);
                self.request_render(false);
            }
            KeyAction::ScrollChatDownPage => {
                self.chat.scroll_down(self.terminal.rows() as usize);
                self.request_render(false);
            }
            KeyAction::ScrollChatUpLine => {
                self.chat.scroll_up(3);
                self.request_render(false);
            }
            KeyAction::ScrollChatDownLine => {
                self.chat.scroll_down(3);
                self.request_render(false);
            }
        }
    }

    fn handle_interrupt(&mut self) {
        if self.state.streaming {
            let client = self.client.clone();
            tokio::spawn(async move {
                let _ = client.abort().await;
            });
            self.state.streaming = false;
            self.state.active_tool_count = 0;
            self.state.tool_start_time = None;
            // Mark the in-progress assistant message as stopped so the partial
            // content (thinking, text, tool calls) is preserved and visible.
            self.chat.mark_last_assistant_stopped();
            self.request_render(false);
            return;
        }
        // Not streaming: exit the app.
        self.running = false;
    }

    // ─── Autocomplete ─────────────────────────────────────────────────

    fn handle_input_changed(&mut self, value: &str) {
        // TS: the AutocompleteManager debounces 20 ms internally; the sync
        // port defers the debounce to the app loop.
        self.pending_ac_query = Some((value.to_string(), value.len()));
        self.ac_query_deadline = Some(Instant::now() + Duration::from_millis(20));
    }

    fn trigger_autocomplete(&mut self) {
        let text = self.input.get_value().to_string();
        let cursor = self.input.cursor();
        self.ac_manager.query_immediate(&text, cursor);
    }

    fn apply_autocomplete_selection(&mut self) {
        let item = self.autocomplete.get_selected_item().cloned();
        let Some(item) = item else { return };
        let ctx = self.ac_manager.active_context().cloned();
        if let Some(ctx) = ctx {
            let token = &ctx.token;
            if !token.is_empty() {
                // Replace only the token portion, preserving the prefix.
                let before = &ctx.text[..ctx.token_start];
                let after = &ctx.text[ctx.token_start + token.len()..];
                let mut value = item.value.clone();
                let max_overlap = before.len().min(value.len());
                for len in (1..=max_overlap).rev() {
                    if before.ends_with(&value[..len]) {
                        value = value[len..].to_string();
                        break;
                    }
                }
                let combined = format!("{before}{value}{after}");
                let cursor = before.len() + value.len();
                self.input.set_value(&combined, Some(cursor));
            } else {
                self.input.set_value(&item.value, None);
            }
        } else {
            self.input.set_value(&item.value, None);
        }
        self.autocomplete.hide();
        self.request_render(false);
    }

    // ─── Approval overlay ──────────────────────────────────────────────

    fn show_approval_overlay(&mut self, req: ApprovalEvent) {
        // Store pending approval.
        self.pending_approval = Some(PendingApproval {
            request_id: req.request_id.clone(),
            tool_name: req.tool_name.clone(),
            title: req.title.clone(),
            summary: req.summary.clone(),
            risk_level: req.risk_level.clone(),
            requested_action: req.requested_action.clone(),
        });

        // Show as a chat message with instructions.
        let action_preview = match &req.requested_action {
            Some(Value::String(s)) => s.clone(),
            Some(v) => serde_json::to_string_pretty(v).unwrap_or_default(),
            None => String::new(),
        };
        let risk = req.risk_level.to_uppercase();
        let preview_block = if action_preview.is_empty() {
            String::new()
        } else {
            format!(
                "```\n{}\n```",
                truncate_to_width(
                    &action_preview,
                    500,
                    &TruncateOptions {
                        ellipsis: false,
                        pad: false
                    }
                )
            )
        };
        let content = format!(
            "⚠️ **Approval Required** [{risk} RISK]\n**{}**\n{}\n{}\n\nType **/approve {}** to allow or **/reject {}** to deny.",
            req.title,
            req.summary,
            preview_block,
            req.request_id,
            req.request_id
        );
        self.chat
            .add_message(ChatMessage::new(random_id(), ChatRole::System, &content));

        // Auto-fill the input with the approve command.
        self.input
            .set_value(&format!("/approve {}", req.request_id), None);
        self.request_render(false);
    }
}

/// Payload of an `approval_request` stream event (app.ts local type).
pub struct ApprovalEvent {
    pub request_id: String,
    pub tool_id: String,
    pub tool_name: String,
    pub kind: String,
    pub risk_level: String,
    pub title: String,
    pub summary: String,
    pub requested_action: Option<Value>,
}

// ─── Actions (handle_submit + friends) ──────────────────────────────────────

impl<T: TerminalIo> App<T> {
    /// `lineDiff` helper — screen-relative row delta (doRender).
    fn line_diff(
        target_row: usize,
        hardware_cursor_row: usize,
        prev_viewport_top: usize,
        viewport_top: usize,
    ) -> i64 {
        let current_screen_row = hardware_cursor_row as i64 - prev_viewport_top as i64;
        let target_screen_row = target_row as i64 - viewport_top as i64;
        target_screen_row - current_screen_row
    }

    /// `App.SEGMENT_RESET` — SGR reset + OSC 8 close (prevents hyperlink leak).
    fn segment_reset() -> &'static str {
        SEGMENT_RESET
    }

    // ─── Submit / slash commands ───────────────────────────────────────

    fn handle_submit(&mut self, value: &str) {
        if value.trim().is_empty() {
            return;
        }

        self.input.set_value("", None);
        self.request_render(false);

        // Handle slash commands locally (don't send to LLM).
        if value.starts_with('/') {
            let parts = split_ws_js(value.strip_prefix('/').unwrap_or(value));
            let cmd = parts[0].to_lowercase();
            let arg = parts[1..].join(" ");

            let mut handled = true;
            match cmd.as_str() {
                "model" => {
                    if self.state.streaming {
                        self.add_system_message(
                            "Cannot change model while agent is streaming. Wait for the current run to finish.".into(),
                        );
                        self.request_render(false);
                        return;
                    }
                    if !arg.is_empty() {
                        // Set model directly — agent resolves provider/id.
                        let client = self.client.clone();
                        let tx = self.op_tx.clone();
                        let model = arg.clone();
                        tokio::spawn(async move {
                            let set_result = client.set_model(&model).await;
                            let state = client.get_state().await.ok();
                            let _ = tx.send(UiCmd::SetModelDone { set_result, state });
                        });
                    } else {
                        self.show_model_selector();
                    }
                }
                "sessions" => self.show_sessions(),
                "help" => self.show_help_overlay(),
                "reload" => {
                    let client = self.client.clone();
                    let tx = self.op_tx.clone();
                    tokio::spawn(async move {
                        let result = client.reload_config().await;
                        let state = client.get_state().await.ok();
                        let _ = tx.send(UiCmd::ReloadDone { result, state });
                    });
                }
                "compact" => {
                    let client = self.client.clone();
                    let tx = self.op_tx.clone();
                    tokio::spawn(async move {
                        let _ = tx.send(UiCmd::CompactDone(client.compact(None).await));
                    });
                }
                "export" => {
                    self.add_system_message("Session export is not available in the TUI.".into());
                }
                "import" => {
                    self.add_system_message("Session import is not available in the TUI.".into());
                }
                "clone" => {
                    let client = self.client.clone();
                    let tx = self.op_tx.clone();
                    tokio::spawn(async move {
                        let result = client.clone_session().await;
                        let mut state = None;
                        let mut messages = Ok(Value::Null);
                        if let Ok(ref v) = result {
                            let cancelled =
                                v.get("cancelled").and_then(Value::as_bool).unwrap_or(false);
                            if !cancelled {
                                state = client.get_state().await.ok();
                                messages = client.get_messages().await;
                            }
                        }
                        let _ = tx.send(UiCmd::CloneDone {
                            result,
                            state,
                            messages,
                        });
                    });
                }
                "fork" => {
                    let client = self.client.clone();
                    let tx = self.op_tx.clone();
                    tokio::spawn(async move {
                        let _ =
                            tx.send(UiCmd::ForkMessagesLoaded(client.get_fork_messages().await));
                    });
                }
                "tree" => {
                    let client = self.client.clone();
                    let tx = self.op_tx.clone();
                    tokio::spawn(async move {
                        let _ = tx.send(UiCmd::SessionsLoaded {
                            result: client.list_sessions().await,
                            purpose: SessionsPurpose::Tree,
                        });
                    });
                }
                "new" => {
                    // Inherit cwd, model, and thinking level from the current
                    // session so /new feels like a clean continuation.
                    self.save_session_input();
                    let client = self.client.clone();
                    let tx = self.op_tx.clone();
                    let cwd = if self.state.cwd.is_empty() {
                        None
                    } else {
                        Some(self.state.cwd.clone())
                    };
                    let model_id = if self.state.model.is_empty() {
                        None
                    } else {
                        Some(self.state.model.clone())
                    };
                    let level = if self.state.thinking.is_empty() {
                        None
                    } else {
                        Some(self.state.thinking.clone() as ThinkingLevel)
                    };
                    tokio::spawn(async move {
                        let result = client
                            .new_session(cwd.as_deref(), model_id.as_deref(), level.as_deref())
                            .await;
                        let mut state = None;
                        if let Ok(ref v) = result {
                            if v.get("sessionId").and_then(Value::as_str).is_some() {
                                state = client.get_state().await.ok();
                            }
                        }
                        let _ = tx.send(UiCmd::NewSessionDone { result, state });
                    });
                }
                "name" => {
                    if arg.is_empty() {
                        self.add_system_message("Usage: /name <session name>".into());
                    } else {
                        self.pending_name_arg = Some(arg.clone());
                        let client = self.client.clone();
                        let tx = self.op_tx.clone();
                        let name = arg.clone();
                        tokio::spawn(async move {
                            let _ =
                                tx.send(UiCmd::SessionNamed(client.set_session_name(&name).await));
                        });
                    }
                }
                "scoped-models" => {
                    let client = self.client.clone();
                    let tx = self.op_tx.clone();
                    tokio::spawn(async move {
                        let _ = tx.send(UiCmd::ModelsLoaded {
                            result: client.list_models().await,
                            purpose: ModelsPurpose::Scoped,
                        });
                    });
                }
                "cwd" if !arg.is_empty() => {
                    if self.state.streaming {
                        self.add_system_message(
                            "Cannot change working directory while agent is streaming.".into(),
                        );
                        self.request_render(false);
                        return;
                    }
                    // Trim the arg: `/cwd ../ ` (trailing space) would join to
                    // `a/b/../ ` and normalize to `a/ ` (the stray space
                    // becomes a path component). The agent trims its side;
                    // the TUI must too, or the footer shows the dirty path.
                    let mut resolved = arg.trim().to_string();
                    let homedir = dirs::home_dir().unwrap_or_default();
                    if resolved == "~" {
                        resolved = homedir.display().to_string();
                    } else if let Some(rest) = resolved.strip_prefix("~/") {
                        resolved = homedir.join(rest).display().to_string();
                    } else if !std::path::Path::new(&resolved).is_absolute() {
                        let base = if self.state.cwd.is_empty() {
                            homedir
                        } else {
                            PathBuf::from(&self.state.cwd)
                        };
                        resolved = base.join(&resolved).display().to_string();
                    }
                    // Resolve `.`/`..` lexically so `/cwd ../../` lands on a
                    // clean path (the agent stores it verbatim after a
                    // trailing-separator trim — no `..` handling on its side).
                    resolved = normalize_path(&resolved);
                    let client = self.client.clone();
                    let tx = self.op_tx.clone();
                    let cwd = resolved.clone();
                    tokio::spawn(async move {
                        let result = client.set_cwd(&cwd).await;
                        let _ = tx.send(UiCmd::CwdSet {
                            result,
                            resolved: cwd,
                        });
                    });
                }
                "approve" if !arg.is_empty() => {
                    let client = self.client.clone();
                    let tx = self.op_tx.clone();
                    let id = arg.clone();
                    tokio::spawn(async move {
                        let result = client.approval_decision(&id, true, "").await;
                        let _ = tx.send(UiCmd::ApprovalDone {
                            result,
                            kind: "approved".into(),
                            request_id: id,
                        });
                    });
                }
                "reject" if !arg.is_empty() => {
                    let client = self.client.clone();
                    let tx = self.op_tx.clone();
                    let id = arg.clone();
                    tokio::spawn(async move {
                        let result = client
                            .approval_decision(&id, false, "rejected by user")
                            .await;
                        let _ = tx.send(UiCmd::ApprovalDone {
                            result,
                            kind: "rejected".into(),
                            request_id: id,
                        });
                    });
                }
                "stop" => {
                    let client = self.client.clone();
                    let tx = self.op_tx.clone();
                    tokio::spawn(async move {
                        let _ = tx.send(UiCmd::StopDone(client.abort().await));
                    });
                }
                "cancel" => {
                    if arg.is_empty() {
                        self.add_system_message("Usage: /cancel <queued-run-id>".into());
                    } else {
                        let client = self.client.clone();
                        let tx = self.op_tx.clone();
                        let run_id = arg.clone();
                        tokio::spawn(async move {
                            let result = client.cancel_queued_run(&run_id).await;
                            let _ = tx.send(UiCmd::QueuedCancelled { result, run_id });
                        });
                    }
                }
                "status" => {
                    let client = self.client.clone();
                    let tx = self.op_tx.clone();
                    tokio::spawn(async move {
                        let state = client.get_state().await;
                        let models = client.list_models().await;
                        let _ = tx.send(UiCmd::StatusLoaded { state, models });
                    });
                }
                _ => handled = false,
            }
            if handled {
                return;
            }
            // Unknown slash command — falls through to the regular prompt.
        }

        // Regular prompt — send to server.
        let local_message_id = random_id();
        self.chat.add_message(ChatMessage::new(
            local_message_id.clone(),
            ChatRole::User,
            value,
        ));

        if self.state.streaming {
            // Every submission is its own run. The Agent owns the FIFO and
            // returns the canonical queued run identity.
            let client = self.client.clone();
            let tx = self.op_tx.clone();
            let message = value.to_string();
            tokio::spawn(async move {
                let result = client.prompt(&message, "enqueue_if_busy").await;
                let _ = tx.send(UiCmd::PromptAck {
                    local_id: local_message_id,
                    result,
                });
            });
            self.request_render(false);
            return;
        }

        self.state.streaming = true;
        self.request_render(false);

        let client = self.client.clone();
        let tx = self.op_tx.clone();
        let message = value.to_string();
        tokio::spawn(async move {
            let result = client.prompt(&message, "reject_if_busy").await;
            let _ = tx.send(UiCmd::PromptAck {
                local_id: local_message_id,
                result,
            });
        });
    }

    fn show_model_selector(&mut self) {
        if self.state.streaming {
            self.add_system_message("Cannot change model while agent is streaming.".into());
            self.request_render(false);
            return;
        }
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(UiCmd::ModelsLoaded {
                result: client.list_models().await,
                purpose: ModelsPurpose::Selector,
            });
        });
    }

    /// Build the `/model` SelectList overlay (TS `showModelSelector` body).
    fn show_model_selector_overlay(&mut self, all_models: Vec<ModelInfo>) {
        // /model shows all models (scoping only applies to ctrl+p cycling).
        let mut models: Vec<String> = all_models.iter().map(|m| m.full_id()).collect::<Vec<_>>();
        models.sort();

        let items: Vec<SelectItem> = models
            .iter()
            .map(|m| SelectItem {
                value: m.clone(),
                label: m.clone(),
                description: if *m == self.state.model {
                    Some("current".into())
                } else {
                    None
                },
            })
            .collect();

        self.show_select_overlay("Select Model", items, 15, OverlayKind::Model);
    }

    /// Build the `/scoped-models` ScopedModelsSelector overlay.
    fn show_scoped_models_overlay(&mut self, all_models: Vec<ModelInfo>) {
        let enabled_set: std::collections::HashSet<String> = match self.enabled_model_ids.clone() {
            Some(ids) => ids.into_iter().collect(),
            None => all_models
                .iter()
                .map(|m| m.full_id())
                .collect::<std::collections::HashSet<_>>(),
        };
        let tx = self.op_tx.clone();
        let tx2 = self.op_tx.clone();
        let selector = ScopedModelsSelector::new(ScopedModelsSelectorOptions {
            all_models,
            enabled_model_ids: enabled_set,
            on_save: Box::new(move |ids: &[String]| {
                let _ = tx.send(UiCmd::ScopedModelsSaved(ids.to_vec()));
            }),
            on_cancel: Box::new(move || {
                let _ = tx2.send(UiCmd::OverlayCancel);
            }),
            max_visible: None,
        });
        let width = (self.terminal.columns() as usize)
            .saturating_sub(4)
            .min(100);
        self.show_overlay(
            Box::new(selector),
            OverlayOptions {
                width: Some(SizeValue::Fixed(width)),
                ..Default::default()
            },
        );
    }

    // ─── Sessions ──────────────────────────────────────────────────────

    fn show_sessions(&mut self) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(UiCmd::SessionsLoaded {
                result: client.list_sessions().await,
                purpose: SessionsPurpose::Browse,
            });
        });
    }

    fn show_sessions_overlay(&mut self, sessions: Vec<SessionSummary>) {
        let items: Vec<SelectItem> = sessions
            .iter()
            .map(|s| {
                let name = s
                    .session_name
                    .clone()
                    .or_else(|| s.first_message.clone())
                    .unwrap_or_else(|| s.id.clone());
                SelectItem {
                    value: s.id.clone(),
                    label: sanitize_session_name(&name),
                    description: if s.id == self.state.session_id {
                        Some("current".into())
                    } else {
                        None
                    },
                }
            })
            .collect();
        self.show_select_overlay("Sessions", items, 15, OverlayKind::Sessions);
    }

    fn show_tree_overlay(&mut self, sessions: Vec<SessionSummary>) {
        if sessions.is_empty() {
            self.add_system_message("No sessions found.".into());
            return;
        }
        // Group sessions by cwd, build tree from parent_session_id.
        let mut grouped: HashMap<String, Vec<SessionSummary>> = HashMap::new();
        for s in &sessions {
            let cwd = if s.cwd.is_empty() { "" } else { s.cwd.as_str() };
            grouped.entry(cwd.to_string()).or_default().push(s.clone());
        }

        let mut items: Vec<SelectItem> = Vec::new();
        for (_, group) in grouped {
            // Build parent→children map.
            let mut children: HashMap<String, Vec<SessionSummary>> = HashMap::new();
            let mut roots: Vec<SessionSummary> = Vec::new();
            for s in &group {
                let parent_id = s.parent_session_id.clone().unwrap_or_default();
                if !parent_id.is_empty() && group.iter().any(|g| g.id == parent_id) {
                    children.entry(parent_id).or_default().push(s.clone());
                } else {
                    roots.push(s.clone());
                }
            }
            // Flatten recursively, tracking "last child" at each depth.
            self.flatten_tree(&children, &mut roots, 0, &[], &mut items);
        }
        self.show_select_overlay("Session Tree", items, 20, OverlayKind::Tree);
    }

    fn flatten_tree(
        &self,
        children: &HashMap<String, Vec<SessionSummary>>,
        list: &mut [SessionSummary],
        depth: usize,
        ancestors_last: &[bool],
        items: &mut Vec<SelectItem>,
    ) {
        // The TS passes a filtered copy (children of the parent); sort by
        // updated_at desc in place.
        list.sort_by_key(|a| std::cmp::Reverse(parse_updated_at(&a.updated_at)));
        let list_len = list.len();
        for (i, s) in list.iter().enumerate() {
            let is_last = i == list_len - 1;
            let has_children = children.get(&s.id).map(|c| !c.is_empty()).unwrap_or(false);
            // Build prefix: ancestor lines + connector.
            let mut prefix = String::new();
            for (d, last) in ancestors_last.iter().enumerate() {
                let _ = d;
                prefix += if *last { "  " } else { "│ " };
            }
            if depth > 0 {
                prefix += if is_last { "└─ " } else { "├─ " };
            }
            let current_marker = if s.id == self.state.session_id {
                "▶ "
            } else {
                "  "
            };
            let name = s
                .session_name
                .clone()
                .or_else(|| s.first_message.clone())
                .unwrap_or_else(|| s.id.clone());
            let label = format!("{current_marker}{prefix}{}", sanitize_session_name(&name));
            items.push(SelectItem {
                value: s.id.clone(),
                label,
                description: if s.id == self.state.session_id {
                    Some("current".into())
                } else {
                    None
                },
            });
            if has_children {
                let mut child_list = children.get(&s.id).cloned().unwrap_or_default();
                let mut next_ancestors = ancestors_last.to_vec();
                next_ancestors.push(is_last);
                self.flatten_tree(children, &mut child_list, depth + 1, &next_ancestors, items);
            }
        }
    }

    fn show_fork_overlay(&mut self, result: Value) {
        let messages: Vec<Value> = result
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if messages.is_empty() {
            self.add_system_message("No user messages to fork from.".into());
            return;
        }
        let items: Vec<SelectItem> = messages
            .iter()
            .enumerate()
            .map(|(i, m)| SelectItem {
                value: m
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                label: format!(
                    "#{}  {}",
                    i + 1,
                    m.get("timestamp").and_then(Value::as_str).unwrap_or("")
                ),
                description: Some(
                    m.get("content")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .chars()
                        .take(70)
                        .collect::<String>(),
                ),
            })
            .collect();
        self.show_select_overlay("Fork from message", items, 15, OverlayKind::Fork);
    }

    /// Generic SelectList overlay helper — wires onSelect/onCancel to UiCmd.
    fn show_select_overlay(
        &mut self,
        title: &str,
        items: Vec<SelectItem>,
        max_visible: usize,
        kind: OverlayKind,
    ) {
        let tx = self.op_tx.clone();
        let tx2 = self.op_tx.clone();
        let sl = SelectList::new(SelectListOptions {
            title: title.to_string(),
            items,
            max_visible: Some(max_visible),
            theme: None,
            on_select: Some(Box::new(move |item: &SelectItem| {
                let _ = tx.send(UiCmd::OverlaySelect {
                    kind,
                    item: item.clone(),
                });
            })),
            on_cancel: Some(Box::new(move || {
                let _ = tx2.send(UiCmd::OverlayCancel);
            })),
            on_selection_change: None,
            on_key: None,
        });
        let width = (self.terminal.columns() as usize).saturating_sub(4).min(80);
        self.show_overlay(
            Box::new(sl),
            OverlayOptions {
                width: Some(SizeValue::Fixed(width)),
                ..Default::default()
            },
        );
    }

    /// Switch-session flow (sessions/tree overlays): switch → refresh →
    /// load messages → message. The overlay hides when the flow completes.
    fn spawn_switch_flow(&mut self, session_id: &str, label: String) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        let sid = session_id.to_string();
        tokio::spawn(async move {
            let result = client.switch_session(&sid).await.map(|_| ());
            let mut state = None;
            let mut messages = Ok(Value::Null);
            if result.is_ok() {
                state = client.get_state().await.ok();
                messages = client.get_messages().await;
            }
            let _ = tx.send(UiCmd::SessionSwitched {
                result,
                state,
                messages,
                label,
            });
        });
    }

    fn apply_status(&mut self, s: &RpcSessionState, models: &[ModelInfo]) {
        let current_model = models
            .iter()
            .find(|m| m.id == s.model.as_deref().unwrap_or(""));
        let model_info: Vec<String> = match current_model {
            Some(m) => vec![
                format!("**Model:** {} (`{}`)", m.label, m.id),
                format!("**Provider:** {}", m.provider),
                format!(
                    "**Image support:** {}",
                    if m.supports_images { "yes" } else { "no" }
                ),
                format!(
                    "**Context window:** {}K",
                    (m.context_window as f64 / 1000.0).round() as u64
                ),
            ],
            None => vec![format!(
                "**Model:** {}",
                s.model.as_deref().unwrap_or("(unknown)")
            )],
        };
        let lines = vec![
            model_info.join("\n"),
            String::new(),
            format!(
                "**Session:** {}",
                if s.session_id.is_empty() {
                    "(none)".to_string()
                } else {
                    s.session_id.clone()
                }
            ),
            format!("**CWD:** {}", s.cwd.as_deref().unwrap_or("(none)")),
            format!("**Thinking:** {}", s.thinking_level),
            format!(
                "**Permission:** {}",
                s.permission_level.as_deref().unwrap_or("all")
            ),
            format!("**Queries:** {}", s.query_count),
            format!(
                "**Auto compaction:** {}",
                if s.auto_compaction_enabled {
                    "on"
                } else {
                    "off"
                }
            ),
            format!(
                "**Streaming:** {}",
                if s.is_streaming { "yes" } else { "no" }
            ),
            String::new(),
            format!(
                "**Context:** {} / {} ({:.1}%)",
                s.context_tokens.unwrap_or(0),
                s.context_window.unwrap_or(0),
                s.context_percent.unwrap_or(0.0)
            ),
            format!(
                "**Tokens:** {} in / {} out",
                s.tokens_in.unwrap_or(0),
                s.tokens_out.unwrap_or(0)
            ),
            format!("**Cost:** ¥{:.4}", s.total_cost.unwrap_or(0.0)),
        ];
        self.add_system_message(lines.join("\n"));
    }

    // ─── Welcome ───────────────────────────────────────────────────────

    fn show_welcome(&mut self) {
        let dim = |t: &str| fg(245, t);
        let section_hdr = |t: &str| fg(221, t);

        // Banner: "future-tui vX.X.X". Prefer the agent's reported version
        // (gRPC handshake); fall back to this binary's injected version.
        let version = if self.state.version.is_empty() {
            VERSION.to_string()
        } else {
            self.state.version.clone()
        };
        let banner = format!(
            "{}{}",
            fg(151, &bold("future-tui")),
            fg(245, &format!(" v{version}"))
        );
        self.chat.add_message(ChatMessage {
            id: random_id(),
            role: ChatRole::System,
            content: banner,
            welcome: true,
            ..ChatMessage::new(String::new(), ChatRole::System, "")
        });

        // Shortcuts line (truncate to fit terminal width).
        let term_w = self.terminal.columns() as usize;
        let shortcuts = truncate_to_width(
            "ctrl+c interrupt · ctrl+p model · ctrl+t thinking · ctrl+o expand/collapse · / commands",
            term_w.saturating_sub(4),
            &TruncateOptions::default(),
        );
        self.chat.add_message(ChatMessage {
            id: random_id(),
            role: ChatRole::System,
            content: dim(&shortcuts),
            welcome: true,
            ..ChatMessage::new(String::new(), ChatRole::System, "")
        });

        // Skills (wrap to fit terminal width). The wrapped lines join into a
        // SINGLE message (TS parity: `add(lines.join("\n"))`) — one message
        // per line would render blank lines between the wrapped segments.
        if !self.state.skills.is_empty() {
            let skills_list = format!("[skills] {}", self.state.skills.join(", "));
            let lines = wrap_text_with_ansi(&dim(&skills_list), term_w.saturating_sub(4));
            self.chat.add_message(ChatMessage {
                id: random_id(),
                role: ChatRole::System,
                content: lines.join("\n"),
                welcome: true,
                ..ChatMessage::new(String::new(), ChatRole::System, "")
            });
        }

        // Extensions (truncate to fit terminal width).
        if !self.state.extensions.is_empty() {
            self.chat.add_message(ChatMessage {
                id: random_id(),
                role: ChatRole::System,
                content: String::new(),
                welcome: true,
                ..ChatMessage::new(String::new(), ChatRole::System, "")
            });
            self.chat.add_message(ChatMessage {
                id: random_id(),
                role: ChatRole::System,
                content: section_hdr("[Extensions]"),
                welcome: true,
                ..ChatMessage::new(String::new(), ChatRole::System, "")
            });
            self.chat.add_message(ChatMessage {
                id: random_id(),
                role: ChatRole::System,
                content: dim(&format!(" {}", self.state.extensions.join(", "))),
                welcome: true,
                ..ChatMessage::new(String::new(), ChatRole::System, "")
            });
        }
    }

    // ─── Session messages / settings ───────────────────────────────────

    /// Reconstruct chat from `get_messages` (TS `loadSessionMessages`).
    fn apply_messages(&mut self, messages: Result<Value, String>) {
        let Ok(value) = messages else { return };
        let list = value
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        self.chat.clear_messages();

        for msg in list {
            let Some(obj) = msg.as_object() else { continue };
            let role = obj.get("role").and_then(Value::as_str).unwrap_or("");
            // Only render user, assistant, and tool messages.
            if !["user", "assistant", "tool"].contains(&role) {
                continue;
            }

            let mut content = String::new();
            match obj.get("content") {
                Some(Value::String(s)) => content = s.clone(),
                Some(Value::Array(blocks)) => {
                    for block in blocks {
                        if let Some(t) = block.get("text").and_then(Value::as_str) {
                            content.push_str(t);
                        } else if let Some(c) = block.get("content").and_then(Value::as_str) {
                            content.push_str(c);
                        }
                    }
                }
                _ => {}
            }

            let tool_calls = obj.get("tool_calls").and_then(Value::as_array);
            if content.is_empty() && tool_calls.is_none_or(|t| t.is_empty()) {
                continue;
            }

            let role_enum = match role {
                "user" => ChatRole::User,
                "assistant" => ChatRole::Assistant,
                "tool" => ChatRole::Tool,
                _ => continue,
            };
            let id = obj.get("id").and_then(Value::as_str).unwrap_or("");
            let id = if id.is_empty() {
                random_id()
            } else {
                id.to_string()
            };
            let mut cm = ChatMessage::new(id, role_enum, &content);
            cm.name = obj.get("name").and_then(Value::as_str).map(String::from);
            cm.tool = obj
                .get("tool_call_id")
                .and_then(Value::as_str)
                .map(String::from);
            cm.tool_args = obj
                .get("tool_args")
                .and_then(Value::as_str)
                .map(String::from);
            cm.thinking = obj
                .get("reasoning_content")
                .and_then(Value::as_str)
                .map(String::from);
            // Historical tool messages: check content for error prefix.
            if role == "tool" {
                cm.tool_status = Some(if content.starts_with("Error:") {
                    ToolStatus::Error
                } else {
                    ToolStatus::Complete
                });
            }
            self.chat.add_message(cm);
        }

        self.request_render(true);
    }

    fn load_tui_settings(&mut self) {
        let data = std::fs::read_to_string(&self.tui_settings_path).unwrap_or_default();
        let v: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
        self.tui_settings = TuiSettings::from_json(&v);
        if let Some(ids) = self.tui_settings.enabled_model_ids.clone() {
            self.enabled_model_ids = Some(ids);
        }
    }

    fn save_tui_settings(&mut self) {
        self.tui_settings.enabled_model_ids = self.enabled_model_ids.clone();
        if let Some(parent) = self.tui_settings_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = serde_json::to_string_pretty(&self.tui_settings.to_json()).unwrap_or_default();
        let _ = std::fs::write(&self.tui_settings_path, json);
    }

    async fn apply_tui_defaults(&mut self) {
        let s = self.tui_settings.clone();
        if let Some(model) = s.default_model {
            let _ = self.client.set_model(&model).await;
        }
        if let Some(level) = s.default_thinking_level {
            let _ = self.client.set_thinking_level(&level).await;
            self.state.thinking = level;
        }
        if let Some(perm) = s.default_permission_level {
            let _ = self.client.set_permission_level(&perm).await;
        }
        // Re-read agent state so the footer reflects changes.
        if let Ok(state) = self.client.get_state().await {
            self.apply_refresh_state(state);
        }
    }

    // ─── Connection state ──────────────────────────────────────────────

    pub fn on_connection_change(&mut self, connected: bool) {
        self.set_connection_lost(!connected);
    }

    fn set_connection_lost(&mut self, lost: bool) {
        if self.connection_lost == lost {
            return;
        }
        self.connection_lost = lost;
        if lost {
            self.state.streaming = false;
            self.state.active_tool_count = 0;
            self.state.tool_start_time = None;
            self.chat.add_message(ChatMessage::new(
                random_id(),
                ChatRole::System,
                "⚠️  Connection to agent lost — retrying every 1s...",
            ));
        } else {
            self.chat.add_message(ChatMessage::new(
                random_id(),
                ChatRole::System,
                "✅  Reconnected to agent",
            ));
            // Delay refresh — after stream reconnect the gRPC channel may
            // need a moment to become ready for unary RPCs.
            let at = Instant::now() + Duration::from_millis(500);
            self.timers.push((at, TimerId::ReconnectRefresh));
            self.timers.sort_by_key(|(at, _)| *at);
        }
        self.request_render(false);
    }

    fn spawn_refresh(&mut self) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(UiCmd::Refreshed(client.get_state().await));
        });
    }

    /// `refresh()` success path (also used by setter flows).
    fn apply_refresh_state(&mut self, s: RpcSessionState) {
        for run_id in self.client.take_lost_queued_run_ids() {
            self.chat
                .update_run_state(&run_id, RunState::LostOnAgentRestart);
        }
        for queued in &s.queued_runs {
            self.chat.upsert_queued_run(
                &queued.run_id,
                &queued.display_text,
                queued.queue_position as u32,
            );
        }
        for terminal in &s.recent_terminal_acks {
            let state = if terminal.reason == "superseded" {
                RunState::Superseded
            } else if terminal.state == "cancelled" {
                RunState::Cancelled
            } else {
                RunState::Terminal
            };
            self.chat.update_run_state(&terminal.run_id, state);
        }
        self.state.model = s.model.clone().unwrap_or_else(|| "(no model)".to_string());
        self.state.thinking = s.thinking_level.clone();
        self.state.streaming = s.is_streaming;
        if !self.state.streaming {
            self.state.active_tool_count = 0;
            self.state.tool_start_time = None;
        }
        if !s.session_id.is_empty() {
            self.state.session_id = s.session_id.clone();
        }
        self.state.cwd = s.cwd.clone().unwrap_or_default();
        self.state.version = s.version.clone().unwrap_or_default();
        let mut skills = s.skills.clone();
        skills.sort();
        self.state.skills = skills;
        self.state.context_files = s.context_files.clone();
        self.state.extensions = s.extensions.clone();
        self.state.context_tokens = s.context_tokens.unwrap_or(0);
        self.state.context_window = s.context_window.unwrap_or(0);
        self.state.context_percent = s.context_percent.unwrap_or(0.0);
        self.state.tokens_in = s.tokens_in.unwrap_or(0);
        self.state.tokens_out = s.tokens_out.unwrap_or(0);
        self.state.tokens_cache_r = s.tokens_cache_r.unwrap_or(0);
        self.state.tokens_cache_w = s.tokens_cache_w.unwrap_or(0);
        self.state.total_cost = s.total_cost.unwrap_or(0.0);
        self.state.explicit_session = s.explicit_session;
        self.state.auto_compaction_enabled = s.auto_compaction_enabled;

        // Update the client's session ID if the server returned a different
        // one (the event stream would otherwise stay stuck on the old one).
        if !s.session_id.is_empty() && s.session_id != self.client.get_current_session_id() {
            self.client.set_current_session_id(&s.session_id);
            self.client.connect_events();
        }

        // Clear connection-lost flag if we successfully reached the agent.
        if self.connection_lost {
            self.connection_lost = false;
            self.chat.add_message(ChatMessage::new(
                random_id(),
                ChatRole::System,
                "✅  Reconnected to agent",
            ));
            self.request_render(false);
        }
        self.request_render(false);
    }

    fn apply_refresh_error(&mut self) {
        // Keep last known model; footer briefly showing "(not connected)" is
        // confusing during transient reconnects.
        if self.state.model.is_empty() || self.state.model == "(no model)" {
            self.state.model = "(not connected)".into();
        }
    }

    // ─── Helpers ───────────────────────────────────────────────────────

    fn add_system_message(&mut self, content: String) {
        self.chat
            .add_message(ChatMessage::new(random_id(), ChatRole::System, &content));
        self.request_render(false);
    }

    fn save_session_input(&mut self) {
        if !self.state.session_id.is_empty() {
            self.session_input_cache.insert(
                self.state.session_id.clone(),
                self.input.get_value().to_string(),
            );
        }
    }

    fn restore_session_input(&mut self) {
        let cached = self
            .session_input_cache
            .get(&self.state.session_id)
            .cloned();
        self.input.set_value(cached.as_deref().unwrap_or(""), None);
    }

    fn invalidate(&mut self) {
        self.chat.invalidate();
        self.input.invalidate();
        self.footer.invalidate();
    }

    // ─── Overlays ──────────────────────────────────────────────────────

    fn show_help_overlay(&mut self) {
        struct HelpComponent;
        impl Component for HelpComponent {
            fn render(&mut self, width: usize) -> Vec<String> {
                crate::help_screen::render_help(width)
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
                self
            }
        }
        let width = self.terminal.columns() as usize;
        self.show_overlay(
            Box::new(HelpComponent),
            OverlayOptions {
                width: Some(SizeValue::Fixed(width)),
                ..Default::default()
            },
        );
    }

    fn show_overlay(&mut self, component: Box<dyn Component>, options: OverlayOptions) -> u64 {
        let pre_focus = self.focused;
        let id = self.next_overlay_id;
        self.next_overlay_id += 1;
        self.focus_order_counter += 1;
        let focus_order = self.focus_order_counter;
        let non_capturing = options.non_capturing;
        self.overlay_stack.push(OverlayEntry {
            id,
            component,
            options,
            pre_focus,
            hidden: false,
            focus_order,
        });

        // Auto-focus unless nonCapturing.
        if !non_capturing {
            self.set_focus(FocusTarget::Overlay(id));
        }
        self.terminal.hide_cursor();
        self.request_render(true);
        id
    }

    fn hide_overlay(&mut self) {
        let Some(entry) = self.overlay_stack.pop() else {
            return;
        };
        if self.focused == FocusTarget::Overlay(entry.id) {
            self.restore_focus(&entry);
        }
        if self.overlay_stack.is_empty() {
            self.terminal.hide_cursor();
        }
        self.request_render(true);
    }

    fn restore_focus(&mut self, entry: &OverlayEntry) {
        // Try next visible overlay, then preFocus, then editor.
        if let Some(idx) = self.get_top_overlay_index() {
            let id = self.overlay_stack[idx].id;
            self.set_focus(FocusTarget::Overlay(id));
        } else if entry.pre_focus != FocusTarget::None {
            self.set_focus(entry.pre_focus);
        } else {
            self.set_focus(FocusTarget::Input);
        }
    }

    fn get_top_overlay_index(&self) -> Option<usize> {
        self.overlay_stack.iter().rposition(|e| !e.hidden)
    }

    fn set_focus(&mut self, target: FocusTarget) {
        // Unset the previous focusable.
        match self.focused {
            FocusTarget::Input => self.input.focused = false,
            FocusTarget::Overlay(id) => {
                if let Some(idx) = self.overlay_stack.iter().position(|e| e.id == id) {
                    set_component_focused(self.overlay_stack[idx].component.as_mut(), false);
                }
            }
            FocusTarget::None => {}
        }
        self.focused = target;
        match target {
            FocusTarget::Input => self.input.focused = true,
            FocusTarget::Overlay(id) => {
                if let Some(idx) = self.overlay_stack.iter().position(|e| e.id == id) {
                    if is_focusable(self.overlay_stack[idx].component.as_ref()) {
                        set_component_focused(self.overlay_stack[idx].component.as_mut(), true);
                    }
                }
            }
            FocusTarget::None => {}
        }
    }

    /// `compositeOverlays` — two-pass render + compositing into the base lines.
    fn composite_overlays(
        &mut self,
        base: Vec<String>,
        term_w: usize,
        term_h: usize,
    ) -> Vec<String> {
        // Filter visible, sort by focusOrder (ascending = later overlays on top).
        let mut visible: Vec<usize> = self
            .overlay_stack
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.hidden)
            .map(|(i, _)| i)
            .collect();
        visible.sort_by_key(|&i| self.overlay_stack[i].focus_order);
        if visible.is_empty() {
            return base;
        }

        // Pad base to at least termH for stable screen-relative positioning.
        let mut lines = if base.len() < term_h {
            let mut l = base;
            l.resize(term_h, String::new());
            l
        } else {
            base
        };

        for &idx in &visible {
            let (layout, overlay_lines) = {
                let entry = &mut self.overlay_stack[idx];
                // Measure pass at termW, then render at layout.width.
                let measure_lines = entry.component.render(term_w);
                if measure_lines.is_empty() {
                    continue;
                }
                let layout = resolve_overlay_layout(
                    term_w,
                    term_h,
                    measure_lines.len(),
                    Some(&entry.options),
                );
                let overlay_lines = entry.component.render(layout.width);
                if overlay_lines.is_empty() {
                    continue;
                }
                (layout, overlay_lines)
            };

            let max_rows = overlay_lines
                .len()
                .min(layout.max_height)
                .min(term_h.saturating_sub(layout.row));
            for (i, overlay_line) in overlay_lines.iter().take(max_rows).enumerate() {
                let target_row = layout.row + i;
                if target_row < lines.len() {
                    lines[target_row] = Self::composite_line_at(
                        &lines[target_row],
                        overlay_line,
                        layout.col,
                        layout.width,
                        term_w,
                    );
                }
            }
        }
        lines
    }

    /// `compositeLineAt` — merge an overlay line into a base line at a column.
    pub fn composite_line_at(
        base: &str,
        overlay: &str,
        col: usize,
        overlay_width: usize,
        total_width: usize,
    ) -> String {
        if is_image_line(base) {
            return base.to_string();
        }

        let after_start = col + overlay_width;
        let base_segs = extract_segments(
            base,
            col,
            after_start,
            total_width.saturating_sub(after_start),
            true,
        );

        // Extract overlay with width tracking.
        let overlay_clean = strip_ansi_codes(overlay);
        let overlay_vis_width = visible_width(&overlay_clean);

        // Pad segments to target widths.
        let before_pad = col.saturating_sub(base_segs.before_width);
        let overlay_pad = overlay_width.saturating_sub(overlay_vis_width);
        let actual_before_width = col.max(base_segs.before_width);
        let actual_overlay_width = overlay_width.max(overlay_vis_width);
        let after_target =
            (total_width as i64 - actual_before_width as i64 - actual_overlay_width as i64).max(0)
                as usize;
        let after_pad = after_target.saturating_sub(base_segs.after_width);

        // Compose result with reset marker between segments.
        let reset = "\x1b[0m";
        let result = format!(
            "{}{}{}{}{}{}{}{}",
            base_segs.before,
            " ".repeat(before_pad),
            reset,
            overlay,
            " ".repeat(overlay_pad),
            reset,
            base_segs.after,
            " ".repeat(after_pad)
        );

        // Final safeguard: verify and truncate to terminal width.
        let result_width = visible_width(&result);
        if result_width <= total_width {
            return result;
        }
        slice_by_column(&result, 0, Some(total_width))
    }

    // ─── Rendering (differential with synchronized output) ─────────────

    pub fn request_render(&mut self, force: bool) {
        if force {
            self.previous_lines.clear();
            self.previous_width = usize::MAX; // triggers widthChanged in doRender
            self.previous_height = usize::MAX;
            self.cursor_row = 0;
            self.hardware_cursor_row = 0;
            self.max_lines_rendered = 0;
            self.previous_viewport_top = 0;
            self.render_now = true;
            self.render_deadline = None;
            self.render_requested = true;
            return;
        }
        if self.render_requested {
            return;
        }
        self.render_requested = true;
        let now = Instant::now();
        let d = self.last_render_at + MIN_RENDER_INTERVAL;
        self.render_deadline = Some(if d > now { d } else { now });
    }

    /// `requestResizeRender` — debounced resize render (public for the loop).
    pub fn request_resize_render(&mut self) {
        self.resize_deadline = Some(Instant::now() + RESIZE_DEBOUNCE);
    }

    /// Line resets — prevents ANSI style bleed between lines.
    fn apply_line_resets(&self, mut lines: Vec<String>) -> Vec<String> {
        let reset = Self::segment_reset();
        for line in &mut lines {
            if line.is_empty() {
                continue;
            }
            if !is_image_line(line) {
                *line = format!("{}{}", normalize_terminal_output(line), reset);
            }
        }
        lines
    }

    /// Extract the cursor marker (`\x1b_pi:c\x07`) from the last line that
    /// carries one, and strip it from the line.
    fn extract_cursor_position(lines: &mut [String], height: usize) -> Option<(usize, usize)> {
        let viewport_top = lines.len().saturating_sub(height);
        for row in (viewport_top..lines.len()).rev() {
            let line = &lines[row];
            if line.is_empty() {
                continue;
            }
            if let Some(marker_index) = line.find("\x1b_pi:c\x07") {
                let before_marker = &line[..marker_index];
                let col = visible_width(before_marker);
                lines[row] = format!("{}{}", &line[..marker_index], &line[marker_index + 7..]);
                return Some((row, col));
            }
        }
        None
    }

    fn position_hardware_cursor(&mut self, cursor_pos: Option<(usize, usize)>, total_lines: usize) {
        let Some((row, col)) = cursor_pos else { return };
        if total_lines == 0 {
            return;
        }
        let target_row = row.min(total_lines - 1);
        let current_row = self.hardware_cursor_row;
        if target_row > current_row {
            self.terminal
                .write(&format!("\x1b[{}B", target_row - current_row));
        } else if target_row < current_row {
            self.terminal
                .write(&format!("\x1b[{}A", current_row - target_row));
        }
        self.terminal.write(&format!("\x1b[{}G", col + 1));
        self.hardware_cursor_row = target_row;
        if self.show_hardware_cursor {
            self.terminal.write("\x1b[?25h");
        }
    }

    fn query_cell_size(&mut self) {
        if get_capabilities().images == ImageProtocol::None {
            return;
        }
        self.terminal.write("\x1b[16t");
    }

    fn consume_cell_size_response(&mut self, data: &str) -> bool {
        static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r"^\x1b\[6;(\d+);(\d+)t$").unwrap());
        let Some(caps) = re.captures(data) else {
            return false;
        };
        let height_px = caps[1].parse::<u32>().unwrap_or(0);
        let width_px = caps[2].parse::<u32>().unwrap_or(0);
        if height_px == 0 || width_px == 0 {
            return true;
        }
        set_cell_dimensions(CellDimensions {
            width_px: width_px as usize,
            height_px: height_px as usize,
        });
        self.invalidate();
        self.request_render(false);
        true
    }

    fn expand_last_changed_for_kitty_images(
        &self,
        first_changed: usize,
        last_changed: usize,
    ) -> usize {
        let mut expanded = last_changed;
        for i in first_changed..self.previous_lines.len() {
            if !extract_kitty_image_ids(&self.previous_lines[i]).is_empty() {
                expanded = expanded.max(i);
            }
        }
        expanded
    }

    fn delete_changed_kitty_images(&self, first_changed: usize, last_changed: usize) -> String {
        if last_changed < first_changed {
            return String::new();
        }
        let mut ids: BTreeSet<u32> = BTreeSet::new();
        let max_line = last_changed.min(self.previous_lines.len().saturating_sub(1));
        for i in first_changed..=max_line {
            for id in extract_kitty_image_ids(
                self.previous_lines.get(i).map(|s| s.as_str()).unwrap_or(""),
            ) {
                ids.insert(id);
            }
        }
        delete_kitty_images(&ids)
    }

    /// Full render: write all lines (optionally clearing first) and update
    /// the diff bookkeeping. Port of the TS `fullRender` closure.
    fn full_render(
        &mut self,
        new_lines: &[String],
        w: usize,
        h: usize,
        cursor_pos: Option<(usize, usize)>,
        clear: bool,
    ) {
        // PI_TUI_DEBUG: dump full render state to /tmp/tui/.
        if std::env::var("PI_TUI_DEBUG").as_deref() == Ok("1") {
            let debug_dir = std::env::temp_dir().join("tui");
            let _ = std::fs::create_dir_all(&debug_dir);
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let mut debug_lines = Vec::new();
            debug_lines.push(format!("=== RENDER {ts} ==="));
            debug_lines.push(format!(
                "reason: {}, W={w}, H={h}",
                if clear { "clear" } else { "full" }
            ));
            debug_lines.push(format!(
                "previousLines.length={}",
                self.previous_lines.len()
            ));
            debug_lines.push(format!("newLines.length={}", new_lines.len()));
            debug_lines.push(format!("overlayStack.length={}", self.overlay_stack.len()));
            debug_lines.push(format!(
                "cursorPos={}",
                cursor_pos
                    .map(|(r, c)| format!("{r}:{c}"))
                    .unwrap_or_else(|| "null".to_string())
            ));
            debug_lines.push("--- lines ---".into());
            for line in new_lines {
                debug_lines.push(line.replace('\x1b', "\\x1b"));
            }
            debug_lines.push("--- end ---".into());
            let _ = std::fs::write(
                debug_dir.join(format!("render-{ts}.log")),
                debug_lines.join("\\n"),
            );
        }

        let mut buf = SYNC_BEGIN.to_string();
        if clear {
            buf += &delete_kitty_images(&self.previous_kitty_image_ids);
            buf += "\x1b[H\x1b[2J"; // Home, clear screen (never clear scrollback)
        }
        for (i, line) in new_lines.iter().enumerate() {
            if i > 0 {
                buf += "\r\n";
            }
            buf += line;
        }
        buf += SYNC_END;
        self.terminal.write(&buf);
        self.cursor_row = new_lines.len().saturating_sub(1);
        self.hardware_cursor_row = self.cursor_row;
        if clear {
            self.max_lines_rendered = new_lines.len();
        } else {
            self.max_lines_rendered = self.max_lines_rendered.max(new_lines.len());
        }
        let buffer_length = h.max(new_lines.len());
        self.previous_viewport_top = buffer_length.saturating_sub(h);
        self.position_hardware_cursor(cursor_pos, new_lines.len());
        self.previous_lines = new_lines.to_vec();
        self.previous_kitty_image_ids = collect_kitty_image_ids(&self.previous_lines);
        self.previous_width = w;
        self.previous_height = h;
    }

    /// Debug redraw logging (PI_DEBUG_REDRAW=1 → ~/.future/tui/debug.log).
    fn log_redraw(&self, reason: &str, new_len: usize, w: usize, h: usize) {
        if std::env::var("PI_DEBUG_REDRAW").as_deref() != Ok("1") {
            return;
        }
        let log_path = dirs::home_dir()
            .unwrap_or_default()
            .join(".future")
            .join("tui")
            .join("debug.log");
        let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let msg = format!(
            "[{ts}] fullRender: {reason} (prev={}, new={new_len}, w={w}, h={h})\n",
            self.previous_lines.len()
        );
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(log_path)
        {
            use std::io::Write;
            let _ = f.write_all(msg.as_bytes());
        }
    }

    /// Main render pipeline — port of `doRender`.
    pub fn do_render(&mut self) {
        if !self.running {
            return;
        }
        if self.state.streaming {
            self.state.spinner_frame += 1;
        }
        let w = self.terminal.columns() as usize;
        let h = self.terminal.rows() as usize;
        let width_changed = self.previous_width != 0 && self.previous_width != w;
        let height_changed = self.previous_height != 0 && self.previous_height != h;
        let previous_buffer_length = if self.previous_height > 0 {
            self.previous_viewport_top + self.previous_height
        } else {
            h
        };
        let mut prev_viewport_top = if height_changed {
            previous_buffer_length.saturating_sub(h)
        } else {
            self.previous_viewport_top
        };
        let mut viewport_top = prev_viewport_top;
        let mut hardware_cursor_row = self.hardware_cursor_row;

        // Render editor first to determine its height (multi-line aware).
        let footer_data = FooterData {
            cwd: Some(self.state.cwd.clone()),
            model: Some(self.state.model.clone()),
            thinking: Some(self.state.thinking.clone()),
            streaming: self.state.streaming,
            spinner_frame: Some(self.state.spinner_frame),
            pending: None,
            context_tokens: Some(self.state.context_tokens as usize),
            context_window: Some(self.state.context_window as usize),
            context_percent: Some(self.state.context_percent as usize),
            tokens_in: Some(self.state.tokens_in as usize),
            tokens_out: Some(self.state.tokens_out as usize),
            tokens_cache_r: Some(self.state.tokens_cache_r as usize),
            tokens_cache_w: Some(self.state.tokens_cache_w as usize),
            tool_elapsed: self
                .state
                .tool_start_time
                .map(|t| (Instant::now() - t).as_secs_f64().floor()),
            total_cost: Some(self.state.total_cost),
            auto_compaction_enabled: self.state.auto_compaction_enabled,
        };
        self.footer.set_data(footer_data);

        let footer_rendered = self.footer.render(w);
        let footer_lines = footer_rendered.len();
        let editor_lines = self.input.render(w);
        let editor_height = editor_lines.len();

        // Set chat viewport based on remaining space.
        let chat_height = h.saturating_sub(editor_height + footer_lines);
        self.chat.set_viewport_height(chat_height.max(1));

        // Build render output: chat + editor + footer.
        let chat_lines = self.chat.render(w);
        let mut new_lines: Vec<String> = chat_lines
            .into_iter()
            .chain(editor_lines)
            .chain(footer_rendered)
            .collect();

        // Extract cursor position BEFORE overlay compositing — overlays may
        // cover the editor row and drop the cursor marker.
        let cursor_pos = Self::extract_cursor_position(&mut new_lines, h);

        // Composite overlays into rendered lines (before diff compare).
        if !self.overlay_stack.is_empty() {
            new_lines = self.composite_overlays(new_lines, w, h);
        }

        // Autocomplete popup (positioned above editor).
        if self.autocomplete.is_visible() {
            let ac_lines = self.autocomplete.render(w);
            if !ac_lines.is_empty() {
                // Position relative to the actual content length, NOT the
                // terminal height (see the TS comment).
                let editor_idx = new_lines.len() - footer_lines - editor_height;
                for (ac_top, line) in
                    (editor_idx as i64 - ac_lines.len() as i64..).zip(ac_lines.iter())
                {
                    if ac_top >= 0 && (ac_top as usize) < editor_idx {
                        new_lines[ac_top as usize] = line.clone();
                    }
                }
            }
        }

        // Apply line resets (prevents ANSI style bleed between lines).
        new_lines = self.apply_line_resets(new_lines);

        // First render — output without clearing (assumes clean screen).
        if self.previous_lines.is_empty() && !width_changed && !height_changed {
            self.log_redraw("first render", new_lines.len(), w, h);
            self.full_render(&new_lines, w, h, cursor_pos, false);
            return;
        }

        // Width changes always need full re-render (wrapping changes).
        if width_changed {
            self.log_redraw(
                &format!("terminal width changed ({} -> {w})", self.previous_width),
                new_lines.len(),
                w,
                h,
            );
            self.full_redraw_count += 1;
            self.full_render(&new_lines, w, h, cursor_pos, true);
            return;
        }

        // Height changes normally need full re-render, but Termux changes
        // height when the software keyboard shows/hides.
        if height_changed && !is_termux_session() {
            self.log_redraw(
                &format!("terminal height changed ({} -> {h})", self.previous_height),
                new_lines.len(),
                w,
                h,
            );
            self.full_redraw_count += 1;
            self.full_render(&new_lines, w, h, cursor_pos, true);
            return;
        }

        // Content shrunk — clear empty rows when clearOnShrink enabled.
        if self.clear_on_shrink
            && new_lines.len() < self.max_lines_rendered
            && self.overlay_stack.is_empty()
        {
            self.log_redraw(
                &format!(
                    "clearOnShrink (maxLinesRendered={})",
                    self.max_lines_rendered
                ),
                new_lines.len(),
                w,
                h,
            );
            self.full_redraw_count += 1;
            self.full_render(&new_lines, w, h, cursor_pos, true);
            return;
        }

        // Ctrl+L forced clear screen.
        if self.force_clear_next_render {
            self.force_clear_next_render = false;
            self.log_redraw("force clear (Ctrl+L)", new_lines.len(), w, h);
            self.full_redraw_count += 1;
            self.full_render(&new_lines, w, h, cursor_pos, true);
            return;
        }

        // ── Diff: find changed lines ──────────────────────────────────
        let mut first_changed: i64 = -1;
        let mut last_changed: i64 = -1;
        let max_lines = new_lines.len().max(self.previous_lines.len());
        for i in 0..max_lines {
            let old_line = if i < self.previous_lines.len() {
                self.previous_lines[i].as_str()
            } else {
                ""
            };
            let new_line = if i < new_lines.len() {
                new_lines[i].as_str()
            } else {
                ""
            };
            if old_line != new_line {
                if first_changed == -1 {
                    first_changed = i as i64;
                }
                last_changed = i as i64;
            }
        }

        // Appended lines detection (streaming optimization).
        let appended_lines = new_lines.len() > self.previous_lines.len();
        if appended_lines {
            if first_changed == -1 {
                first_changed = self.previous_lines.len() as i64;
            }
            last_changed = new_lines.len() as i64 - 1;
        }
        if first_changed != -1 {
            last_changed = self
                .expand_last_changed_for_kitty_images(first_changed as usize, last_changed as usize)
                as i64;
        }
        let append_start = appended_lines
            && first_changed as usize == self.previous_lines.len()
            && first_changed > 0;

        // No changes — but still need to update the hardware cursor position.
        if first_changed == -1 {
            self.position_hardware_cursor(cursor_pos, new_lines.len());
            self.previous_viewport_top = prev_viewport_top;
            self.previous_height = h;
            return;
        }

        // ── All changes in deleted lines (content shrunk) ─────────────
        if first_changed as usize >= new_lines.len() {
            if self.previous_lines.len() > new_lines.len() {
                let mut buf = SYNC_BEGIN.to_string();
                buf += &self
                    .delete_changed_kitty_images(first_changed as usize, last_changed as usize);
                let target_row = new_lines.len().saturating_sub(1);
                // If viewport moved up (content above viewport removed),
                // full render.
                if target_row < prev_viewport_top {
                    self.log_redraw(
                        &format!(
                            "deleted lines moved viewport up ({target_row} < {prev_viewport_top})"
                        ),
                        new_lines.len(),
                        w,
                        h,
                    );
                    self.full_render(&new_lines, w, h, cursor_pos, true);
                    return;
                }
                let ld = Self::line_diff(
                    target_row,
                    hardware_cursor_row,
                    prev_viewport_top,
                    viewport_top,
                );
                if ld > 0 {
                    buf += &format!("\x1b[{ld}B");
                } else if ld < 0 {
                    buf += &format!("\x1b[{}A", -ld);
                }
                buf += "\r";

                let extra_lines = self.previous_lines.len() - new_lines.len();
                // If too many lines to clear, full render.
                if extra_lines > h {
                    self.log_redraw(
                        &format!("too many lines to clear (extraLines={extra_lines} > H={h})"),
                        new_lines.len(),
                        w,
                        h,
                    );
                    self.full_render(&new_lines, w, h, cursor_pos, true);
                    return;
                }
                if extra_lines > 0 {
                    buf += "\x1b[1B";
                }
                for i in 0..extra_lines {
                    buf += "\r\x1b[2K";
                    if i < extra_lines - 1 {
                        buf += "\x1b[1B";
                    }
                }
                if extra_lines > 0 {
                    buf += &format!("\x1b[{extra_lines}A");
                }
                buf += SYNC_END;
                self.terminal.write(&buf);
                self.cursor_row = target_row;
                self.hardware_cursor_row = target_row;
            }
            self.position_hardware_cursor(cursor_pos, new_lines.len());
            self.previous_lines = new_lines;
            self.previous_kitty_image_ids = collect_kitty_image_ids(&self.previous_lines);
            self.previous_width = w;
            self.previous_height = h;
            self.previous_viewport_top = prev_viewport_top;
            return;
        }

        // Differential rendering can only touch what was actually visible.
        if (first_changed as usize) < prev_viewport_top {
            self.log_redraw(
                &format!(
                    "first changed line above viewport ({} < {prev_viewport_top})",
                    first_changed
                ),
                new_lines.len(),
                w,
                h,
            );
            self.full_render(&new_lines, w, h, cursor_pos, true);
            return;
        }

        // ── Differential render ────────────────────────────────────────
        let mut buf = SYNC_BEGIN.to_string();
        buf += &self.delete_changed_kitty_images(first_changed as usize, last_changed as usize);
        let prev_viewport_bottom = prev_viewport_top + h - 1;
        let move_target_row = if append_start {
            first_changed as usize - 1
        } else {
            first_changed as usize
        };
        if move_target_row > prev_viewport_bottom {
            let current_screen_row = hardware_cursor_row
                .saturating_sub(prev_viewport_top)
                .min(h - 1);
            let move_to_bottom = h - 1 - current_screen_row;
            if move_to_bottom > 0 {
                buf += &format!("\x1b[{move_to_bottom}B");
            }
            let scroll = move_target_row - prev_viewport_bottom;
            buf += &"\r\n".repeat(scroll);
            prev_viewport_top += scroll;
            viewport_top += scroll;
            hardware_cursor_row = move_target_row;
        }

        // Move cursor to first changed line.
        let ld = Self::line_diff(
            move_target_row,
            hardware_cursor_row,
            prev_viewport_top,
            viewport_top,
        );
        if ld > 0 {
            buf += &format!("\x1b[{ld}B");
        } else if ld < 0 {
            buf += &format!("\x1b[{}A", -ld);
        }

        buf += if append_start { "\r\n" } else { "\r" };

        let render_end = (last_changed as usize).min(new_lines.len() - 1);
        for (offset, line) in new_lines[first_changed as usize..=render_end]
            .iter()
            .enumerate()
        {
            if offset > 0 {
                buf += "\r\n";
            }
            buf += "\x1b[2K";
            if line.is_empty() {
                continue;
            }
            let is_image = is_image_line(line);
            if !is_image && visible_width(line) > w {
                // Truncate instead of crashing — graceful degradation.
                buf += &truncate_to_width(line, w - 1, &TruncateOptions::default());
            } else {
                buf += line;
            }
        }

        let mut final_cursor_row = render_end;

        // Clear extra lines when content shrunk.
        if self.previous_lines.len() > new_lines.len() {
            if render_end < new_lines.len() - 1 {
                let move_down = new_lines.len() - 1 - render_end;
                buf += &format!("\x1b[{move_down}B");
                final_cursor_row = new_lines.len() - 1;
            }
            let extra_lines = self.previous_lines.len() - new_lines.len();
            for _ in new_lines.len()..self.previous_lines.len() {
                buf += "\r\n\x1b[2K";
            }
            buf += &format!("\x1b[{extra_lines}A");
        }

        buf += SYNC_END;
        self.terminal.write(&buf);

        self.cursor_row = new_lines.len().saturating_sub(1);
        self.hardware_cursor_row = final_cursor_row;
        self.max_lines_rendered = self.max_lines_rendered.max(new_lines.len());
        self.previous_viewport_top = prev_viewport_top.max(final_cursor_row.saturating_sub(h - 1));

        self.position_hardware_cursor(cursor_pos, new_lines.len());

        self.previous_lines = new_lines;
        self.previous_kitty_image_ids = collect_kitty_image_ids(&self.previous_lines);
        self.previous_width = w;
        self.previous_height = h;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    // ─── Fake terminal ────────────────────────────────────────────────

    struct FakeTerminal {
        writes: Rc<RefCell<Vec<String>>>,
        cols: u16,
        rows: u16,
    }

    impl TerminalIo for FakeTerminal {
        fn write(&self, data: &str) {
            self.writes.borrow_mut().push(data.to_string());
        }
        fn columns(&self) -> u16 {
            self.cols
        }
        fn rows(&self) -> u16 {
            self.rows
        }
        fn hide_cursor(&self) {}
        fn show_cursor(&self) {}
        fn start(
            &mut self,
            _on_input: Box<dyn FnMut(String) + Send + 'static>,
            _on_resize: Box<dyn FnMut() + Send + 'static>,
        ) -> std::io::Result<()> {
            Ok(())
        }
        fn stop(&mut self) {}
        fn drain_input(&mut self, _max_ms: u64, _idle_ms: u64) {}
        fn set_exit_signal_callback(&mut self, _cb: Option<Box<dyn FnMut() + Send + 'static>>) {}
    }

    fn make_app(cols: u16, rows: u16) -> (App<FakeTerminal>, mpsc::UnboundedReceiver<UiCmd>) {
        let (op_tx, op_rx) = mpsc::unbounded_channel();
        let (client, _events, _conn) = GrpcClient::new("127.0.0.1:1");
        let app = App::new(
            FakeTerminal {
                writes: Rc::new(RefCell::new(Vec::new())),
                cols,
                rows,
            },
            Arc::new(client),
            op_tx,
            &CliOptions::default(),
            std::env::temp_dir().join("tui-test-settings.json"),
        );
        (app, op_rx)
    }

    fn terminal_writes(app: &App<FakeTerminal>) -> String {
        app.terminal.writes.borrow().join("")
    }

    // ─── Pure helpers ──────────────────────────────────────────────────

    #[test]
    fn sanitize_collapses_whitespace_runs() {
        assert_eq!(sanitize_session_name("  a   b\t\n c "), "a b c");
        assert_eq!(sanitize_session_name("single"), "single");
        assert_eq!(sanitize_session_name(""), "");
    }

    // ─── /cwd path normalization ────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn normalize_path_resolves_dotdot_and_clamps_at_root() {
        // `/cwd ../../` from a project dir → two levels up, clean path.
        assert_eq!(normalize_path("/Users/geilige/future-os/../../"), "/Users");
        assert_eq!(normalize_path("/a/b/../c"), "/a/c");
        assert_eq!(normalize_path("/a/./b"), "/a/b");
        assert_eq!(normalize_path("/a/b/.."), "/a");
        // Extra `..` clamps at the root, like `cd ..` at `/`.
        assert_eq!(normalize_path("/../a"), "/a");
        assert_eq!(normalize_path("/a/b/../../../c"), "/c");
        // Absolute paths without `.`/`..` pass through untouched.
        assert_eq!(normalize_path("/tmp/foo"), "/tmp/foo");
    }

    // ─── /cwd end-to-end (in-process mock agent) ──────────────────────

    use future_rpc::proto::future_agent_server::{FutureAgent, FutureAgentServer};
    use future_rpc::proto::{RpcCommand, RpcResponse, StreamEvent, StreamRequest};
    use futures_util::StreamExt;
    use std::net::TcpListener;
    use std::pin::Pin;
    use tokio_stream::wrappers::UnboundedReceiverStream;
    use tonic::transport::Server;

    /// Minimal mock agent: answers unary commands, and on `set_cwd` echoes a
    /// `cwd_changed` event carrying the stored (trailing-slash-trimmed) cwd —
    /// the same behavior as the real agent (agent/src/rpc/commands.rs).
    #[derive(Clone)]
    struct CwdMockAgent {
        subs: Arc<std::sync::Mutex<Vec<mpsc::UnboundedSender<StreamEvent>>>>,
    }

    #[tonic::async_trait]
    impl FutureAgent for CwdMockAgent {
        async fn execute_command(
            &self,
            request: tonic::Request<RpcCommand>,
        ) -> Result<tonic::Response<RpcResponse>, tonic::Status> {
            let cmd = request.into_inner();
            if cmd.r#type == "set_cwd" {
                let cwd = cmd.cwd.trim().trim_end_matches(['/', '\\']).to_string();
                let event = StreamEvent {
                    r#type: "cwd_changed".into(),
                    data: serde_json::json!({ "cwd": cwd }).to_string(),
                    ..Default::default()
                };
                for sub in self.subs.lock().unwrap().iter() {
                    let _ = sub.send(event.clone());
                }
            }
            Ok(tonic::Response::new(RpcResponse {
                id: cmd.id,
                r#type: "response".into(),
                command: cmd.r#type.clone(),
                success: true,
                data: "{}".into(),
                error: String::new(),
                error_code: String::new(),
                error_data: String::new(),
                payload: None,
            }))
        }

        type StreamEventsStream =
            Pin<Box<dyn tokio_stream::Stream<Item = Result<StreamEvent, tonic::Status>> + Send>>;

        async fn stream_events(
            &self,
            _request: tonic::Request<StreamRequest>,
        ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
            let (tx, rx) = mpsc::unbounded_channel::<StreamEvent>();
            self.subs.lock().unwrap().push(tx);
            Ok(tonic::Response::new(Box::pin(
                UnboundedReceiverStream::new(rx).map(Ok),
            )))
        }
    }

    async fn spawn_cwd_mock_agent() -> (tokio::task::JoinHandle<()>, String) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener); // tonic binds the same port below
        let agent = CwdMockAgent {
            subs: Arc::new(std::sync::Mutex::new(Vec::new())),
        };
        let handle = tokio::spawn(async move {
            let _ = Server::builder()
                .add_service(FutureAgentServer::new(agent))
                .serve(addr)
                .await;
        });
        // Give the server a moment to start listening.
        tokio::time::sleep(Duration::from_millis(50)).await;
        (handle, format!("127.0.0.1:{}", addr.port()))
    }

    /// `app.state.cwd` is exactly what the footer renders (`data.cwd`). A
    /// relative cwd like `a/b` with `/cwd ../` must land on a clean `a` —
    /// not `a/b/../` and not `a/ ../`. The trailing-space variant (`/cwd
    /// ../ `) must behave identically (the arg is trimmed, so the stray
    /// space never becomes a path component).
    #[tokio::test(flavor = "multi_thread")]
    async fn cwd_dotdot_from_relative_cwd_renders_clean_parent() {
        let (_server, addr) = spawn_cwd_mock_agent().await;
        let (op_tx, mut op_rx) = mpsc::unbounded_channel();
        let (client, mut events, _conn) = GrpcClient::new(&addr);
        let mut app = App::new(
            FakeTerminal {
                writes: Rc::new(RefCell::new(Vec::new())),
                cols: 80,
                rows: 24,
            },
            Arc::new(client),
            op_tx,
            &CliOptions::default(),
            std::env::temp_dir().join("tui-cwd-test-settings.json"),
        );

        for input in ["/cwd ../", "/cwd ../ "] {
            app.state.cwd = "a/b".into();

            app.handle_submit(input);

            // Apply the CwdSet UiCmd when it arrives.
            let deadline = tokio::time::timeout(Duration::from_secs(5), op_rx.recv()).await;
            if let Ok(Some(cmd)) = deadline {
                app.handle_cmd(cmd);
            }

            // The agent's `cwd_changed` echo (best-effort — the stream may
            // not be subscribed yet, but the real agent always emits it).
            if let Ok(Some(ev)) =
                tokio::time::timeout(Duration::from_millis(500), events.recv()).await
            {
                app.handle_agent_event(&ev);
            }

            assert_eq!(
                app.state.cwd, "a",
                "input {input:?}: footer cwd must be a clean parent `a`, got {:?}",
                app.state.cwd
            );
        }
    }

    // ─── Welcome screen ─────────────────────────────────────────────────

    #[tokio::test]
    async fn welcome_skills_wrap_stays_in_one_message() {
        // A long skills list wraps; the wrapped lines must join into a SINGLE
        // message (one message per line renders blank lines between the
        // wrapped segments — the pre-fix bug).
        let (mut app, _rx) = make_app(40, 24);
        app.state.skills = (0..20).map(|i| format!("future-skill-{i:02}")).collect();
        app.show_welcome();
        let skills = app.chat.last_message().expect("welcome must add messages");
        assert!(
            skills.content.contains("[skills]"),
            "last welcome message should be the skills list, got: {}",
            skills.content
        );
        let lines: Vec<&str> = skills.content.split('\n').collect();
        assert!(
            lines.len() >= 2,
            "long skills list must wrap inside one message"
        );
        assert!(
            lines.iter().all(|l| !l.is_empty()),
            "no blank lines between wrapped segments"
        );
    }

    #[test]
    fn split_ws_matches_js_regex_semantics() {
        assert_eq!(split_ws_js("model x"), vec!["model", "x"]);
        assert_eq!(split_ws_js(" model"), vec!["", "model"]);
        assert_eq!(split_ws_js("a  b"), vec!["a", "b"]);
        assert_eq!(split_ws_js(""), vec![""]);
        assert_eq!(split_ws_js("status"), vec!["status"]);
        // Trailing whitespace yields a trailing EMPTY element, exactly like
        // JS `split(/\s+/)` — NOT a re-emission of the last token with the
        // space attached (the pre-fix bug that corrupted `/cwd ../ `).
        assert_eq!(split_ws_js("cwd ../ "), vec!["cwd", "../", ""]);
        assert_eq!(split_ws_js("cwd "), vec!["cwd", ""]);
        assert_eq!(split_ws_js(" "), vec!["", ""]);
    }

    /// `/status` lines must match the TS template verbatim — in particular
    /// the JS `|| "(none)"` fallback: a present sessionId renders WITHOUT the
    /// ` or (none)` suffix (the P4 tmux harness caught the port always
    /// appending it), and the model fallback is `|| "(unknown)"` without an
    /// ` or (unknown)` suffix either.
    #[tokio::test]
    async fn apply_status_session_and_model_fallbacks_match_ts() {
        let (mut app, _rx) = make_app(120, 36);

        // Session id present → `**Session:** mock-session-1` exactly.
        let s = RpcSessionState {
            session_id: "mock-session-1".into(),
            model: Some("mock-model".into()),
            ..Default::default()
        };
        app.apply_status(&s, &[]);
        let last = app.chat.last_message().cloned().unwrap();
        assert!(last.content.contains("**Session:** mock-session-1"));
        assert!(!last.content.contains(" or (none)"));
        assert!(last.content.contains("**Model:** mock-model"));
        assert!(!last.content.contains(" or (unknown)"));

        // No session id / no model → `(none)` / `(unknown)` stand alone.
        let s2 = RpcSessionState::default();
        app.apply_status(&s2, &[]);
        let last = app.chat.last_message().cloned().unwrap();
        assert!(last.content.contains("**Session:** (none)"));
        assert!(!last.content.contains(" or (none)"));
        assert!(last.content.contains("**Model:** (unknown)"));
        assert!(!last.content.contains(" or (unknown)"));
    }

    #[test]
    fn composite_line_at_replaces_middle_segment() {
        let base = "abcdef";
        // The extracted `after` segment carries a leading SGR reset (the
        // tracker's pending reset flush — TS extractSegments parity).
        let result = App::<FakeTerminal>::composite_line_at(base, "XY", 2, 2, 6);
        assert_eq!(result, "ab\x1b[0mXY\x1b[0m\x1b[0mef");
        assert_eq!(visible_width(&result), 6);
    }

    #[test]
    fn composite_line_at_pads_shorter_overlay() {
        // Overlay is 1 char in a 3-char slot → pad to the slot width.
        let base = "abcdef";
        let result = App::<FakeTerminal>::composite_line_at(base, "X", 2, 3, 6);
        assert_eq!(result, "ab\x1b[0mX  \x1b[0m\x1b[0mf");
        assert_eq!(visible_width(&result), 6);
    }

    #[test]
    fn composite_line_at_skips_image_lines() {
        let base = "\x1b_Ga=1;m=0; ";
        let result = App::<FakeTerminal>::composite_line_at(base, "XY", 2, 2, 80);
        assert_eq!(result, base);
    }

    #[test]
    fn tui_settings_roundtrip() {
        let settings = TuiSettings {
            default_model: Some("deepseek-v4-pro".into()),
            default_thinking_level: Some("high".into()),
            default_permission_level: None,
            enabled_model_ids: Some(vec!["a".into(), "b".into()]),
        };
        let json = serde_json::to_string(&settings.to_json()).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();
        let back = TuiSettings::from_json(&parsed);
        assert_eq!(back.default_model.as_deref(), Some("deepseek-v4-pro"));
        assert_eq!(back.default_thinking_level.as_deref(), Some("high"));
        assert_eq!(
            back.enabled_model_ids,
            Some(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn tui_settings_json_key_order_matches_ts() {
        let settings = TuiSettings {
            default_model: Some("m".into()),
            default_thinking_level: None,
            default_permission_level: None,
            enabled_model_ids: Some(vec!["x".into()]),
        };
        let json = serde_json::to_string_pretty(&settings.to_json()).unwrap();
        let model_pos = json.find("defaultModel").unwrap();
        let ids_pos = json.find("enabledModelIds").unwrap();
        assert!(
            model_pos < ids_pos,
            "defaultModel must serialize before enabledModelIds"
        );
    }

    // ─── App state machine ─────────────────────────────────────────────

    fn sample_state() -> RpcSessionState {
        serde_json::from_value(json_parse(
            r#"{"model":"deepseek-v4-pro","thinkingLevel":"high","isStreaming":true,"sessionId":"s1","cwd":"/tmp","queryCount":2,"skills":["b","a"],"contextTokens":100,"contextWindow":128000,"contextPercent":0.1,"tokensIn":10,"tokensOut":20,"tokensCacheR":1,"tokensCacheW":2,"totalCost":0.01,"autoCompactionEnabled":true,"explicitSession":false}"#,
        ))
        .expect("state")
    }

    fn json_parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[tokio::test]
    async fn refreshed_applies_state_to_footer_sources() {
        let (mut app, _rx) = make_app(100, 30);
        let state = sample_state();
        app.handle_cmd(UiCmd::Refreshed(Ok(state)));
        assert_eq!(app.state.model, "deepseek-v4-pro");
        assert_eq!(app.state.thinking, "high");
        assert!(app.state.streaming);
        assert_eq!(app.state.session_id, "s1");
        assert_eq!(app.state.skills, vec!["a", "b"]); // sorted
        assert_eq!(app.state.tokens_in, 10);
    }

    #[tokio::test]
    async fn refresh_error_marks_not_connected() {
        let (mut app, _rx) = make_app(100, 30);
        app.handle_cmd(UiCmd::Refreshed(Err("transport error".into())));
        assert_eq!(app.state.model, "(not connected)");
    }

    #[tokio::test]
    async fn refresh_error_keeps_known_model() {
        let (mut app, _rx) = make_app(100, 30);
        app.state.model = "deepseek-v4-pro".into();
        app.handle_cmd(UiCmd::Refreshed(Err("transport error".into())));
        assert_eq!(app.state.model, "deepseek-v4-pro");
    }

    #[tokio::test]
    async fn queued_runs_reconstruct_bubbles_on_refresh() {
        let (mut app, _rx) = make_app(100, 30);
        let state: RpcSessionState = serde_json::from_value(json_parse(
            r#"{"thinkingLevel":"off","queuedRuns":[{"runId":"r1","runSequence":2,"clientRequestId":"c","state":"queued","queuePosition":1,"acceptedAt":"2026-08-07T00:00:00Z","displayText":"hi"}]}"#,
        ))
        .unwrap();
        app.handle_cmd(UiCmd::Refreshed(Ok(state)));
        let last = app.chat.last_message().cloned().unwrap();
        assert_eq!(last.id, "r1");
        assert_eq!(last.run_state, Some(RunState::Queued));
        assert_eq!(last.queue_position, Some(1));
    }

    #[tokio::test]
    async fn terminal_acks_map_to_run_states() {
        let (mut app, _rx) = make_app(100, 30);
        app.chat
            .add_message(ChatMessage::new("m1".into(), ChatRole::User, "x"));
        app.chat
            .bind_user_run("m1", "run-1", RunState::Running, None);
        let state: RpcSessionState = serde_json::from_value(json_parse(
            r#"{"thinkingLevel":"off","recentTerminalAcks":[{"run_id":"run-1","run_sequence":1,"client_request_id":"c","state":"terminal","reason":"superseded"}]}"#,
        ))
        .unwrap();
        app.handle_cmd(UiCmd::Refreshed(Ok(state)));
        assert_eq!(
            app.chat.last_message().unwrap().run_state,
            Some(RunState::Superseded)
        );
    }

    #[tokio::test]
    async fn submit_unknown_slash_falls_through_to_prompt() {
        let (mut app, mut rx) = make_app(100, 30);
        app.handle_cmd(UiCmd::Submit("/totally-unknown-cmd arg".into()));
        // A user message was added locally and a prompt task was spawned
        // (its UiCmd arrives asynchronously on the op channel).
        let last = app.chat.last_message().cloned().unwrap();
        assert_eq!(last.role, ChatRole::User);
        assert_eq!(last.content, "/totally-unknown-cmd arg");
        assert!(app.state.streaming);
        // The prompt task will fail to reach the dead agent and send
        // PromptAck; drain it briefly so the test doesn't leak state.
        let _ = tokio::time::timeout(Duration::from_millis(3000), rx.recv()).await;
    }

    #[tokio::test]
    async fn submit_export_shows_unavailable_message() {
        let (mut app, _rx) = make_app(100, 30);
        app.handle_cmd(UiCmd::Submit("/export".into()));
        let last = app.chat.last_message().cloned().unwrap();
        assert_eq!(last.role, ChatRole::System);
        assert_eq!(last.content, "Session export is not available in the TUI.");
    }

    #[tokio::test]
    async fn handle_interrupt_when_idle_stops_app() {
        let (mut app, _rx) = make_app(100, 30);
        app.running = true;
        app.handle_input("\x03");
        assert!(!app.running);
    }

    // ─── Render pipeline ───────────────────────────────────────────────

    #[tokio::test]
    async fn first_render_writes_sync_begin_lines_sync_end() {
        let (mut app, _rx) = make_app(100, 30);
        app.running = true;
        app.chat
            .add_message(ChatMessage::new("1".into(), ChatRole::User, "hello"));
        app.input.set_value("typed", None);
        app.do_render();
        let buf = terminal_writes(&app);
        assert!(buf.starts_with("\x1b[?2026h"), "starts with sync begin");
        // The sync buffer is the first write; cursor positioning follows it.
        assert!(
            app.terminal.writes.borrow()[0].ends_with("\x1b[?2026l"),
            "sync write ends with sync end"
        );
        assert!(buf.contains("hello"), "chat content present");
        assert!(buf.contains("typed"), "editor content present");
        assert_eq!(
            app.previous_lines.len(),
            app.terminal.writes.borrow()[0].matches("\r\n").count() + 1
        );
    }

    #[tokio::test]
    async fn second_render_with_no_changes_writes_only_cursor_move() {
        let (mut app, _rx) = make_app(100, 30);
        app.running = true;
        app.chat
            .add_message(ChatMessage::new("1".into(), ChatRole::User, "hello"));
        app.do_render();
        let writes_before = app.terminal.writes.borrow().len();
        app.do_render();
        // A no-op diff still positions the hardware cursor (TS
        // `positionHardwareCursor` writes the G move unconditionally) but
        // never emits a sync buffer.
        let new_writes: Vec<String> = app.terminal.writes.borrow()[writes_before..].to_vec();
        assert!(!new_writes.is_empty(), "cursor positioning write expected");
        for w in &new_writes {
            assert!(!w.contains("\x1b[?2026h"), "no sync buffer on no-op render");
            assert!(!w.contains("hello"), "no content rewrite on no-op render");
        }
    }

    #[tokio::test]
    async fn typing_triggers_input_changed_and_render() {
        let (mut app, mut rx) = make_app(100, 30);
        app.running = true;
        app.do_render(); // first render
        app.handle_input("h");
        // The printable char inserts into the input and requests a render.
        assert_eq!(app.input.get_value(), "h");
        // The onChange callback fired a UiCmd::InputChanged.
        match rx.try_recv() {
            Ok(UiCmd::InputChanged(v)) => assert_eq!(v, "h"),
            other => panic!(
                "expected InputChanged, got {:?}",
                other.map(|c| format!("{c:?}"))
            ),
        }
        app.on_tick();
        assert!(!app.terminal.writes.borrow().is_empty());
    }

    #[tokio::test]
    async fn ctrl_l_forces_clear_next_render() {
        let (mut app, mut rx) = make_app(100, 30);
        app.running = true;
        app.handle_input("\x0c");
        // Ctrl+L routes through the keybinding manager, which sends the
        // action as a UiCmd (the loop applies it).
        match rx.try_recv().expect("keybinding action queued") {
            UiCmd::KeyAction(KeyAction::ForceClear) => {
                app.handle_cmd(UiCmd::KeyAction(KeyAction::ForceClear))
            }
            other => panic!("expected ForceClear, got {other:?}"),
        }
        assert!(app.force_clear_next_render);
    }

    #[tokio::test]
    async fn force_render_resets_diff_state() {
        let (mut app, _rx) = make_app(100, 30);
        app.running = true;
        app.chat
            .add_message(ChatMessage::new("1".into(), ChatRole::User, "a"));
        app.do_render();
        assert!(!app.previous_lines.is_empty());
        app.request_render(true);
        assert!(app.previous_lines.is_empty());
        assert!(app.render_now);
    }

    #[tokio::test]
    async fn request_resize_render_sets_debounce_deadline() {
        let (mut app, _rx) = make_app(100, 30);
        app.request_resize_render();
        assert!(app.resize_deadline.is_some());
    }

    // ─── Coverage driving harness ─────────────────────────────────────

    /// Feed spawned-task results back into the app until quiescent.
    async fn pump(app: &mut App<FakeTerminal>, op_rx: &mut mpsc::UnboundedReceiver<UiCmd>) {
        let mut idle = 0;
        for _ in 0..80 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            let mut drained = Vec::new();
            while let Ok(cmd) = op_rx.try_recv() {
                drained.push(cmd);
            }
            if drained.is_empty() {
                idle += 1;
                if idle >= 3 {
                    break;
                }
                continue;
            }
            idle = 0;
            for cmd in drained {
                app.handle_cmd(cmd);
            }
        }
    }

    /// System message contents, plain text.
    fn system_messages(app: &App<FakeTerminal>) -> Vec<String> {
        app.chat
            .plain_messages()
            .iter()
            .filter(|(role, _)| *role == ChatRole::System)
            .map(|(_, content)| content.clone())
            .collect()
    }

    fn last_system(app: &App<FakeTerminal>) -> String {
        system_messages(app).last().cloned().unwrap_or_default()
    }

    fn sample_models() -> Vec<ModelInfo> {
        vec![
            serde_json::from_value(json_parse(
                r#"{"id":"gpt-4o","label":"GPT-4o","provider":"openai"}"#,
            ))
            .expect("model"),
            serde_json::from_value(json_parse(
                r#"{"id":"claude-sonnet-4","label":"Claude Sonnet 4","provider":"anthropic","supportsImages":true,"contextWindow":200000}"#,
            ))
            .expect("model"),
        ]
    }

    fn sample_sessions() -> Vec<SessionSummary> {
        vec![
            serde_json::from_value(json_parse(
                r#"{"id":"s1","cwd":"/tmp/a","updatedAt":"2026-01-02T00:00:00Z","model":"m1","sessionName":"first"}"#,
            ))
            .expect("session"),
            serde_json::from_value(json_parse(
                r#"{"id":"s2","cwd":"/tmp/a","updatedAt":"2026-01-01T00:00:00Z","model":"m1","parentSessionId":"s1"}"#,
            ))
            .expect("session"),
        ]
    }

    // ─── handle_cmd matrix ────────────────────────────────────────────

    fn make_event(t: &str, data: &str) -> AgentEvent {
        AgentEvent {
            r#type: t.to_string(),
            session_id: None,
            run_id: None,
            epoch: 0,
            idx: 0,
            event_id: None,
            timestamp: None,
            projection_snapshot: false,
            snapshot_cursor: 0,
            snapshot_events: Vec::new(),
            data: json_parse(data),
        }
    }

    fn make_event_with_run(t: &str, data: &str, run_id: &str) -> AgentEvent {
        let mut ev = make_event(t, data);
        ev.run_id = Some(run_id.to_string());
        ev
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn handle_cmd_async_result_variants() {
        let (mut app, mut rx) = make_app(100, 30);

        // Refreshed ok/err.
        app.handle_cmd(UiCmd::Refreshed(Ok(sample_state())));
        assert_eq!(app.state.model, "deepseek-v4-pro");
        app.handle_cmd(UiCmd::Refreshed(Err("down".into())));

        // ModelsLoaded → both overlay purposes + error.
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Selector,
        });
        assert!(!app.overlay_stack.is_empty());
        app.handle_cmd(UiCmd::OverlayCancel);
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Scoped,
        });
        assert!(!app.overlay_stack.is_empty());
        app.handle_cmd(UiCmd::OverlayCancel);
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Err("no models".into()),
            purpose: ModelsPurpose::Selector,
        });
        assert!(last_system(&app).contains("Failed to load models"));

        // SessionsLoaded → browse + tree + error.
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(sample_sessions()),
            purpose: SessionsPurpose::Browse,
        });
        assert!(!app.overlay_stack.is_empty());
        app.handle_cmd(UiCmd::OverlayCancel);
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(sample_sessions()),
            purpose: SessionsPurpose::Tree,
        });
        assert!(!app.overlay_stack.is_empty());
        app.handle_cmd(UiCmd::OverlayCancel);
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Err("no sessions".into()),
            purpose: SessionsPurpose::Browse,
        });
        assert!(last_system(&app).contains("Failed to load sessions"));

        // ForkMessagesLoaded ok/err.
        app.handle_cmd(UiCmd::ForkMessagesLoaded(Ok(json_parse(
            r#"{"messages":[{"id":"e1","text":"hello","role":"user"}]}"#,
        ))));
        assert!(!app.overlay_stack.is_empty());
        app.handle_cmd(UiCmd::OverlayCancel);
        app.handle_cmd(UiCmd::ForkMessagesLoaded(Err("nope".into())));
        assert!(last_system(&app).contains("Failed to load fork messages"));

        // SetModelDone: error, then ok.
        app.handle_cmd(UiCmd::SetModelDone {
            set_result: Err("bad model".into()),
            state: None,
        });
        assert!(last_system(&app).contains("Failed to set model"));
        app.handle_cmd(UiCmd::SetModelDone {
            set_result: Ok(()),
            state: Some(sample_state()),
        });
        assert!(last_system(&app).contains("Model:"));

        // ModelCycled ok (with state) + err.
        app.handle_cmd(UiCmd::ModelCycled {
            result: Ok(json_parse(r#"{"model":"m2"}"#)),
            state: Some(sample_state()),
        });
        app.handle_cmd(UiCmd::ModelCycled {
            result: Err("x".into()),
            state: None,
        });

        // ThinkingCycled ok/err.
        app.handle_cmd(UiCmd::ThinkingCycled(Ok(json_parse(r#"{"level":"xhigh"}"#))));
        assert_eq!(app.state.thinking, "xhigh");
        app.handle_cmd(UiCmd::ThinkingCycled(Err("x".into())));

        // CompactDone ok/err.
        app.handle_cmd(UiCmd::CompactDone(Ok("done".into())));
        assert!(last_system(&app).contains("Context compacted"));
        app.handle_cmd(UiCmd::CompactDone(Err("bad".into())));
        assert!(last_system(&app).contains("Compact failed"));

        // ReloadDone ok (with skills + contextFiles) / err.
        app.handle_cmd(UiCmd::ReloadDone {
            result: Ok(json_parse(r#"{"skills":["a","b"],"contextFiles":["AGENTS.md"]}"#)),
            state: Some(sample_state()),
        });
        assert!(last_system(&app).contains("Reloaded: 2 skills loaded"));
        app.handle_cmd(UiCmd::ReloadDone {
            result: Ok(json_parse(r#"{"skills":[],"contextFiles":[]}"#)),
            state: None,
        });
        assert!(last_system(&app).contains("no skills found"));
        app.handle_cmd(UiCmd::ReloadDone {
            result: Err("x".into()),
            state: None,
        });
        assert!(last_system(&app).contains("Reload failed"));

        // SessionNamed ok/err.
        app.pending_name_arg = Some("new name".into());
        app.handle_cmd(UiCmd::SessionNamed(Ok(())));
        assert!(last_system(&app).contains("new name"));
        app.pending_name_arg = Some("n2".into());
        app.handle_cmd(UiCmd::SessionNamed(Err("x".into())));
        assert!(last_system(&app).contains("Failed to set session name"));

        // CwdSet ok/err.
        app.handle_cmd(UiCmd::CwdSet {
            result: Ok(()),
            resolved: "/tmp/xyz".into(),
        });
        assert_eq!(app.state.cwd, "/tmp/xyz");
        assert!(last_system(&app).contains("/tmp/xyz"));
        app.handle_cmd(UiCmd::CwdSet {
            result: Err("nope".into()),
            resolved: String::new(),
        });
        assert!(last_system(&app).contains("Failed to change directory"));

        // ApprovalDone approved/rejected × ok/err.
        app.handle_cmd(UiCmd::ApprovalDone {
            result: Ok(()),
            kind: "approved".into(),
            request_id: "r1".into(),
        });
        assert!(last_system(&app).contains("Approved request: r1"));
        app.handle_cmd(UiCmd::ApprovalDone {
            result: Err("x".into()),
            kind: "rejected".into(),
            request_id: "r2".into(),
        });
        assert!(last_system(&app).contains("Failed to reject"));

        // StopDone ok/err.
        app.handle_cmd(UiCmd::StopDone(Ok(())));
        assert!(last_system(&app).contains("Stopped current generation"));
        app.handle_cmd(UiCmd::StopDone(Err("x".into())));
        assert!(last_system(&app).contains("Failed to stop"));

        // QueuedCancelled ok/err.
        app.handle_cmd(UiCmd::QueuedCancelled {
            result: Ok(()),
            run_id: "q1".into(),
        });
        assert!(last_system(&app).contains("Cancelled queued run"));
        app.handle_cmd(UiCmd::QueuedCancelled {
            result: Err("x".into()),
            run_id: "q1".into(),
        });
        assert!(last_system(&app).contains("Failed to cancel queued run"));

        // StatusLoaded: both ok / models err / state err.
        app.handle_cmd(UiCmd::StatusLoaded {
            state: Ok(sample_state()),
            models: Ok(sample_models()),
        });
        assert!(last_system(&app).contains("Queries"));
        app.handle_cmd(UiCmd::StatusLoaded {
            state: Ok(sample_state()),
            models: Err("m".into()),
        });
        app.handle_cmd(UiCmd::StatusLoaded {
            state: Err("s".into()),
            models: Ok(vec![]),
        });
        assert!(last_system(&app).contains("Failed to get status"));

        // InitialPromptDone is a no-op.
        app.handle_cmd(UiCmd::InitialPromptDone(Ok(
            crate::rpc::types::RunAck {
                run_id: "r".into(),
                run_epoch: 1,
                accepted_state: "running".into(),
                run_sequence: None,
                queue_position: None,
            },
        )));

        let _ = &mut rx;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn handle_cmd_session_flow_variants() {
        let (mut app, mut rx) = make_app(100, 30);
        app.state.session_id = "current".into();

        // SessionSwitched err.
        app.handle_cmd(UiCmd::SessionSwitched {
            result: Err("nope".into()),
            state: None,
            messages: Ok(Value::Null),
            label: "l".into(),
        });
        assert!(last_system(&app).contains("Failed to switch session"));
        // ok with state+messages.
        app.handle_cmd(UiCmd::SessionSwitched {
            result: Ok(()),
            state: Some(sample_state()),
            messages: Ok(json_parse(
                r#"{"messages":[{"id":"m1","role":"user","content":"hi"}]}"#,
            )),
            label: "target".into(),
        });
        assert!(last_system(&app).contains("Switched to session: target"));
        // The refresh above set the client's session id; clear it so later
        // dead-client calls skip the 5 s connect wait.
        app.client.set_current_session_id("");

        // TreeSelected: same session → just hides; different → switch flow.
        app.handle_cmd(UiCmd::TreeSelected {
            item: SelectItem {
                value: "current".into(),
                label: "cur".into(),
                description: None,
            },
        });
        app.handle_cmd(UiCmd::TreeSelected {
            item: SelectItem {
                value: "other".into(),
                label: "oth".into(),
                description: None,
            },
        });
        pump(&mut app, &mut rx).await; // switch flow fails against dead client
        assert!(system_messages(&app).iter().any(|m| m.contains("Failed to switch session")));

        // ForkSelected → ForkDone spawn chain (fails against dead client).
        app.handle_cmd(UiCmd::ForkSelected {
            item: SelectItem {
                value: "e1".into(),
                label: "entry".into(),
                description: None,
            },
        });
        pump(&mut app, &mut rx).await;
        assert!(system_messages(&app).iter().any(|m| m.contains("Failed to fork")));

        // ForkDone direct: cancelled, ok-not-cancelled, err.
        app.handle_cmd(UiCmd::ForkDone {
            fork_result: Ok(json_parse(r#"{"cancelled":true}"#)),
            state: None,
            messages: Ok(Value::Null),
            label: "l".into(),
        });
        app.handle_cmd(UiCmd::ForkDone {
            fork_result: Ok(json_parse(r#"{"cancelled":false}"#)),
            state: Some(sample_state()),
            messages: Ok(json_parse(r#"{"messages":[]}"#)),
            label: "l".into(),
        });
        assert!(last_system(&app).contains("Forked from l."));
        app.handle_cmd(UiCmd::ForkDone {
            fork_result: Err("x".into()),
            state: None,
            messages: Ok(Value::Null),
            label: "l".into(),
        });
        assert!(last_system(&app).contains("Failed to fork"));

        // NewSessionDone: with/without sessionId, err.
        app.handle_cmd(UiCmd::NewSessionDone {
            result: Ok(json_parse(r#"{"sessionId":"s-new"}"#)),
            state: Some(sample_state()),
        });
        assert!(last_system(&app).contains("New session started"));
        app.handle_cmd(UiCmd::NewSessionDone {
            result: Ok(json_parse(r#"{}"#)),
            state: None,
        });
        app.handle_cmd(UiCmd::NewSessionDone {
            result: Err("x".into()),
            state: None,
        });
        assert!(last_system(&app).contains("Not connected to agent"));

        // CloneDone: cancelled, ok, err.
        app.handle_cmd(UiCmd::CloneDone {
            result: Ok(json_parse(r#"{"cancelled":true}"#)),
            state: None,
            messages: Ok(Value::Null),
        });
        app.handle_cmd(UiCmd::CloneDone {
            result: Ok(json_parse(r#"{"cancelled":false}"#)),
            state: Some(sample_state()),
            messages: Ok(json_parse(r#"{"messages":[]}"#)),
        });
        assert!(last_system(&app).contains("Session cloned"));
        app.handle_cmd(UiCmd::CloneDone {
            result: Err("x".into()),
            state: None,
            messages: Ok(Value::Null),
        });
        assert!(last_system(&app).contains("Failed to clone session"));

        // ModelSelected → spawn (fails against dead client). Clear the
        // client session first so the calls skip the 5 s connect wait.
        app.client.set_current_session_id("");
        app.handle_cmd(UiCmd::ModelSelected(SelectItem {
            value: "openai/gpt-4o".into(),
            label: "gpt".into(),
            description: None,
        }));
        pump(&mut app, &mut rx).await;
        assert!(system_messages(&app).iter().any(|m| m.contains("Failed to set model")));

        // PromptAck: queued ack binds run; err (non-transport) adds message;
        // err (transport) doesn't.
        app.handle_cmd(UiCmd::PromptAck {
            local_id: "does-not-exist".into(),
            result: Ok(crate::rpc::types::RunAck {
                run_id: "r1".into(),
                run_epoch: 1,
                accepted_state: "queued".into(),
                run_sequence: None,
                queue_position: Some(2),
            }),
        });
        app.handle_cmd(UiCmd::PromptAck {
            local_id: "x".into(),
            result: Err("some random failure".into()),
        });
        assert!(last_system(&app).contains("Not connected to agent"));
        let before = system_messages(&app).len();
        app.handle_cmd(UiCmd::PromptAck {
            local_id: "x".into(),
            result: Err("transport error".into()),
        });
        assert_eq!(system_messages(&app).len(), before); // no message

        // ScopedModelsSaved.
        app.cached_models = sample_models()
            .iter()
            .map(|m| m.full_id())
            .collect();
        app.handle_cmd(UiCmd::ScopedModelsSaved(vec!["openai/gpt-4o".into()]));
        assert!(last_system(&app).contains("1/2 enabled"));
        assert_eq!(
            app.enabled_model_ids.as_deref(),
            Some(&["openai/gpt-4o".to_string()][..])
        );

        // InputEscape clears the input.
        app.input.set_value("draft", None);
        app.handle_cmd(UiCmd::InputEscape);
        assert!(app.input.get_value().is_empty());

        // AcItems show/hide.
        app.handle_cmd(UiCmd::AcItems(vec![crate::components::autocomplete::AutocompleteItem {
            value: "/model".into(),
            label: "/model".into(),
            description: None,
        }]));
        assert!(app.autocomplete.is_visible());
        app.handle_cmd(UiCmd::AcItems(vec![]));
        assert!(!app.autocomplete.is_visible());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn overlay_select_all_kinds() {
        let (mut app, mut rx) = make_app(100, 30);
        app.state.session_id = "current".into();

        // Sessions kind → switch flow (fails on dead client).
        app.handle_cmd(UiCmd::OverlaySelect {
            kind: OverlayKind::Sessions,
            item: SelectItem {
                value: "s9".into(),
                label: "nine".into(),
                description: None,
            },
        });
        pump(&mut app, &mut rx).await;

        // Tree kind: same session → hide only.
        app.handle_cmd(UiCmd::OverlaySelect {
            kind: OverlayKind::Tree,
            item: SelectItem {
                value: "current".into(),
                label: "cur".into(),
                description: None,
            },
        });
        // Tree kind: different session → switch flow.
        app.handle_cmd(UiCmd::OverlaySelect {
            kind: OverlayKind::Tree,
            item: SelectItem {
                value: "s9".into(),
                label: "nine".into(),
                description: None,
            },
        });
        pump(&mut app, &mut rx).await;

        // Fork + Model kinds delegate.
        app.handle_cmd(UiCmd::OverlaySelect {
            kind: OverlayKind::Fork,
            item: SelectItem {
                value: "e1".into(),
                label: "e".into(),
                description: None,
            },
        });
        pump(&mut app, &mut rx).await;
        app.handle_cmd(UiCmd::OverlaySelect {
            kind: OverlayKind::Model,
            item: SelectItem {
                value: "openai/gpt-4o".into(),
                label: "m".into(),
                description: None,
            },
        });
        pump(&mut app, &mut rx).await;

        // Settings kind: sessions / reload / other.
        app.handle_cmd(UiCmd::OverlaySelect {
            kind: OverlayKind::Settings,
            item: SelectItem {
                value: "sessions".into(),
                label: "s".into(),
                description: None,
            },
        });
        pump(&mut app, &mut rx).await;
        app.handle_cmd(UiCmd::OverlaySelect {
            kind: OverlayKind::Settings,
            item: SelectItem {
                value: "reload".into(),
                label: "r".into(),
                description: None,
            },
        });
        assert!(last_system(&app).contains("Settings reloaded"));
        app.handle_cmd(UiCmd::OverlaySelect {
            kind: OverlayKind::Settings,
            item: SelectItem {
                value: "other".into(),
                label: "o".into(),
                description: None,
            },
        });
    }

    // ─── Agent events ─────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn agent_event_all_types() {
        let (mut app, mut rx) = make_app(100, 30);

        // user_message dedup: same text as the last user message is skipped.
        app.handle_agent_event(&make_event("user_message", r#"{"text":"hello"}"#));
        let count = app.chat.plain_messages().len();
        app.handle_agent_event(&make_event("user_message", r#"{"text":"hello"}"#));
        assert_eq!(app.chat.plain_messages().len(), count); // deduped
        app.handle_agent_event(&make_event("user_message", r#"{"text":"different"}"#));
        assert!(app.chat.plain_messages().len() > count);

        // agent_start → assistant message + streaming; run state tracked.
        app.handle_agent_event(&make_event_with_run("agent_start", "{}", "r1"));
        assert!(app.state.streaming);
        // thinking lifecycle.
        app.handle_agent_event(&make_event("thinking_start", "{}"));
        app.handle_agent_event(&make_event("thinking_delta", r#"{"text":"pondering"}"#));
        app.handle_agent_event(&make_event("thinking_end", "{}"));
        // text chunk.
        app.handle_agent_event(&make_event("text_chunk", r#"{"text":"answer"}"#));
        // tool lifecycle: start (with object args), delta, end.
        app.handle_agent_event(&make_event(
            "tool_start",
            r#"{"tool_id":"t1","tool_name":"read","tool_args":{"path":"/x"}}"#,
        ));
        assert_eq!(app.state.active_tool_count, 1);
        app.handle_agent_event(&make_event("tool_delta", r#"{"tool_id":"t1","text":"part"}"#));
        app.handle_agent_event(&make_event("tool_end", r#"{"tool_id":"t1","text":"done"}"#));
        assert_eq!(app.state.active_tool_count, 0);
        // tool_start with string args.
        app.handle_agent_event(&make_event(
            "tool_start",
            r#"{"tool_id":"t2","tool_name":"shell","tool_args":"{\"command\":\"ls\"}"}"#,
        ));
        app.handle_agent_event(&make_event("tool_end", r#"{"tool_id":"t2"}"#));
        pump(&mut app, &mut rx).await; // tool_end's spawn_refresh fails silently

        // approval_request → chat card + prefilled /approve command.
        app.handle_agent_event(&make_event(
            "approval_request",
            r#"{"approval_request_id":"a1","tool_id":"t9","tool_name":"shell","kind":"exec","risk_level":"high","summary":"rm -rf","requested_action":"rm -rf /"}"#,
        ));
        assert!(last_system(&app).contains("Approval Required"));
        assert!(app.input.get_value().contains("/approve a1"));
        app.input.set_value("", None);

        // error event.
        app.handle_agent_event(&make_event("error", r#"{"error":"boom"}"#));
        assert!(last_system(&app).contains("Error: boom"));
        app.handle_agent_event(&make_event("error", r#"{"error_message":"other"}"#));
        assert!(last_system(&app).contains("Error: other"));
        app.handle_agent_event(&make_event("error", r#"{"x":1}"#));
        assert!(last_system(&app).contains("unknown error"));

        // usage event accumulates tokens.
        app.handle_agent_event(&make_event(
            "usage",
            r#"{"usage":{"prompt_tokens":10,"completion_tokens":5,"cache_read_tokens":2,"cache_write_tokens":3}}"#,
        ));
        assert_eq!(app.state.tokens_in, 10);
        assert_eq!(app.state.tokens_out, 5);
        assert_eq!(app.state.context_tokens, 15);
        pump(&mut app, &mut rx).await;

        // settings-change events.
        app.handle_agent_event(&make_event("model_changed", r#"{"model":"m9"}"#));
        assert_eq!(app.state.model, "m9");
        app.handle_agent_event(&make_event("thinking_level_changed", r#"{"level":"low"}"#));
        assert_eq!(app.state.thinking, "low");
        app.handle_agent_event(&make_event("cwd_changed", r#"{"cwd":"/tmp/z"}"#));
        assert_eq!(app.state.cwd, "/tmp/z");
        app.handle_agent_event(&make_event("auto_compaction_changed", r#"{"enabled":false}"#));
        assert!(!app.state.auto_compaction_enabled);
        app.handle_agent_event(&make_event("session_name_changed", r#"{"name":"n"}"#));
        app.handle_agent_event(&make_event("permission_level_changed", "{}"));
        app.handle_agent_event(&make_event("tools_changed", "{}"));
        app.handle_agent_event(&make_event("sandbox_policy_changed", "{}"));
        app.handle_agent_event(&make_event("ephemeral_changed", "{}"));
        pump(&mut app, &mut rx).await;

        // config_reloaded with skills + context files.
        app.handle_agent_event(&make_event(
            "config_reloaded",
            r#"{"skills":["b","a"],"contextFiles":["CLAUDE.md"]}"#,
        ));
        assert_eq!(app.state.skills, vec!["a", "b"]);
        assert!(last_system(&app).contains("Config reloaded: 2 skills, CLAUDE.md"));
        // …and with empty lists.
        app.handle_agent_event(&make_event("config_reloaded", r#"{"skills":[],"contextFiles":[]}"#));
        assert!(last_system(&app).contains("no context files"));

        // agent_end with terminal text.
        app.handle_agent_event(&make_event_with_run("agent_end", r#"{"text":"final answer"}"#, "r1"));
        assert!(!app.state.streaming);
        pump(&mut app, &mut rx).await;

        // Unknown event types are ignored.
        app.handle_agent_event(&make_event("some_future_event", "{}"));
    }

    // ─── Key actions ──────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn key_actions_all() {
        let (mut app, mut rx) = make_app(100, 30);

        // Scroll actions (need content to scroll).
        app.handle_key_action(KeyAction::ScrollChatUpPage);
        app.handle_key_action(KeyAction::ScrollChatDownPage);
        app.handle_key_action(KeyAction::ScrollChatUpLine);
        app.handle_key_action(KeyAction::ScrollChatDownLine);

        // ToggleThinking flips visibility.
        app.handle_key_action(KeyAction::ToggleThinking);
        app.handle_key_action(KeyAction::ToggleThinking);

        // ForceClear sets the flag.
        app.handle_key_action(KeyAction::ForceClear);
        assert!(app.force_clear_next_render);

        // CycleModel not streaming → spawns (fails on dead client).
        app.handle_key_action(KeyAction::CycleModel);
        pump(&mut app, &mut rx).await;

        // CycleModel while streaming → refused message.
        app.state.streaming = true;
        app.handle_key_action(KeyAction::CycleModel);
        assert!(last_system(&app).contains("Cannot change model while agent is streaming"));
        app.state.streaming = false;

        // CycleModel with a scoped list cycles locally.
        app.enabled_model_ids = Some(vec!["a/m1".into(), "b/m2".into()]);
        app.state.model = "a/m1".into();
        app.handle_key_action(KeyAction::CycleModel);
        assert_eq!(app.state.model, "b/m2");
        pump(&mut app, &mut rx).await;
        // …and wrapping around / unknown current model.
        app.state.model = "unlisted".into();
        app.handle_key_action(KeyAction::CycleModel);
        assert_eq!(app.state.model, "a/m1");
        pump(&mut app, &mut rx).await;
        app.enabled_model_ids = None;

        // CycleThinking refused while streaming.
        app.state.streaming = true;
        app.handle_key_action(KeyAction::CycleThinking);
        assert!(last_system(&app).contains("Cannot change thinking level"));
        app.state.streaming = false;
        app.handle_key_action(KeyAction::CycleThinking);
        pump(&mut app, &mut rx).await;

        // ShowSessions spawns a load.
        app.handle_key_action(KeyAction::ShowSessions);
        pump(&mut app, &mut rx).await;
        assert!(last_system(&app).contains("Failed to load sessions"));

        // Interrupt while streaming → abort spawn + stopped marker.
        app.state.streaming = true;
        app.handle_key_action(KeyAction::Interrupt);
        assert!(!app.state.streaming);
        pump(&mut app, &mut rx).await;
        // Interrupt while idle → app stops running.
        app.running = true;
        app.handle_key_action(KeyAction::Interrupt);
        assert!(!app.running);
    }

    // ─── handle_key / handle_input paths ──────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn handle_key_paths() {
        let (mut app, mut rx) = make_app(100, 30);

        // Escape on empty editor clears it.
        app.input.set_value("text", None);
        app.handle_key("escape");
        assert!(app.input.get_value().is_empty());

        // Plain keys go to the editor.
        app.handle_key("left");
        // ctrl+ combos pass through to the editor.
        app.handle_key("ctrl+a");
        // Tab triggers autocomplete machinery (slash prefix).
        app.input.set_value("/mo", None);
        app.handle_key("tab");
        pump(&mut app, &mut rx).await;

        // Autocomplete navigation.
        app.autocomplete.show(vec![
            crate::components::autocomplete::AutocompleteItem {
                value: "/model".into(),
                label: "/model".into(),
                description: None,
            },
            crate::components::autocomplete::AutocompleteItem {
                value: "/new".into(),
                label: "/new".into(),
                description: None,
            },
        ]);
        app.handle_key("down");
        app.handle_key("up");
        app.handle_key("escape"); // hides autocomplete
        assert!(!app.autocomplete.is_visible());

        // shift+ctrl+d with a debug callback.
        let hits = std::rc::Rc::new(std::cell::Cell::new(0));
        let cb = hits.clone();
        app.on_debug = Some(Box::new(move || cb.set(cb.get() + 1)));
        app.handle_key("shift+ctrl+d");
        assert_eq!(hits.get(), 1);
        app.on_debug = None;
        app.handle_key("shift+ctrl+d"); // no callback — no panic

        // Escape with an overlay closes it.
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Selector,
        });
        assert!(!app.overlay_stack.is_empty());
        app.handle_key("escape");
        assert!(app.overlay_stack.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn handle_input_paths() {
        let (mut app, mut rx) = make_app(100, 30);
        let _ = &mut rx;

        // Bracketed paste into the editor.
        app.handle_input("\x1b[200~pasted text\x1b[201~");
        assert_eq!(app.input.get_value(), "pasted text");

        // Key release events are dropped unless wanted.
        app.handle_input("\x1b[97;1:3u"); // kitty release for 'a'
        assert!(app.input.get_value().ends_with("pasted text"));

        // ctrl+c byte → interrupt.
        app.running = true;
        app.handle_input("\x03");
        assert!(!app.running);

        // Printable fallback char.
        app.handle_input("x");
        assert!(app.input.get_value().contains('x'));

        // Input listeners can rewrite/consume.
        app.input_listeners.push(Box::new(|d| {
            if d == "swallow" {
                Some(InputListenerResult {
                    consume: true,
                    data: None,
                })
            } else if d == "rewrite" {
                Some(InputListenerResult {
                    consume: false,
                    data: Some("z".to_string()),
                })
            } else {
                None
            }
        }));
        app.input.set_value("", None);
        app.handle_input("swallow");
        assert!(app.input.get_value().is_empty()); // consumed
        app.handle_input("rewrite");
        assert!(app.input.get_value().ends_with('z')); // rewritten path
        app.input_listeners.clear();
    }

    // ─── Slash commands ───────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn slash_commands_matrix() {
        let (mut app, mut rx) = make_app(100, 30);

        // Empty submit is a no-op.
        app.handle_cmd(UiCmd::Submit("   ".into()));

        for cmd in [
            "/help", "/sessions", "/tree", "/fork", "/clone", "/new", "/compact",
            "/reload", "/scoped-models", "/status", "/stop", "/export", "/import",
        ] {
            app.handle_cmd(UiCmd::Submit(cmd.into()));
        }
        pump(&mut app, &mut rx).await;
        assert!(system_messages(&app).iter().any(|m| m.contains("export is not available")));
        assert!(system_messages(&app).iter().any(|m| m.contains("import is not available")));

        // /model with arg (dead client → fails), and selector path.
        app.handle_cmd(UiCmd::Submit("/model sonnet".into()));
        pump(&mut app, &mut rx).await;
        assert!(system_messages(&app).iter().any(|m| m.contains("Failed to set model")));
        app.handle_cmd(UiCmd::Submit("/model".into()));
        pump(&mut app, &mut rx).await;
        assert!(system_messages(&app).iter().any(|m| m.contains("Failed to load models")));

        // /model while streaming → refused.
        app.state.streaming = true;
        app.handle_cmd(UiCmd::Submit("/model x".into()));
        assert!(last_system(&app).contains("Cannot change model while agent is streaming"));
        app.state.streaming = false;

        // /name with and without arg.
        app.handle_cmd(UiCmd::Submit("/name".into()));
        assert!(last_system(&app).contains("Usage: /name"));
        app.handle_cmd(UiCmd::Submit("/name my session".into()));
        pump(&mut app, &mut rx).await;
        assert!(system_messages(&app).iter().any(|m| m.contains("Failed to set session name")));

        // /cwd with no arg is not a command — it becomes a prompt.
        let before = app.chat.plain_messages().len();
        app.handle_cmd(UiCmd::Submit("/cwd".into()));
        assert!(app.chat.plain_messages().len() > before);
        app.state.streaming = false; // the prompt above set it
        app.handle_cmd(UiCmd::Submit("/cwd /tmp".into()));
        pump(&mut app, &mut rx).await;
        assert!(system_messages(&app).iter().any(|m| m.contains("Failed to change directory")));
        app.handle_cmd(UiCmd::Submit("/cwd ~".into()));
        pump(&mut app, &mut rx).await;
        app.handle_cmd(UiCmd::Submit("/cwd ~/sub".into()));
        pump(&mut app, &mut rx).await;
        app.handle_cmd(UiCmd::Submit("/cwd rel/path".into()));
        pump(&mut app, &mut rx).await;
        // /cwd while streaming → refused.
        app.state.streaming = true;
        app.handle_cmd(UiCmd::Submit("/cwd /tmp".into()));
        assert!(last_system(&app).contains("Cannot change working directory"));
        app.state.streaming = false;

        // /approve, /reject (with and without arg).
        app.handle_cmd(UiCmd::Submit("/approve".into()));
        app.handle_cmd(UiCmd::Submit("/reject".into()));
        app.handle_cmd(UiCmd::Submit("/approve req-1".into()));
        pump(&mut app, &mut rx).await;
        assert!(system_messages(&app).iter().any(|m| m.contains("Failed to approve")));
        app.handle_cmd(UiCmd::Submit("/reject req-2".into()));
        pump(&mut app, &mut rx).await;
        assert!(system_messages(&app).iter().any(|m| m.contains("Failed to reject")));

        // /cancel with and without arg.
        app.handle_cmd(UiCmd::Submit("/cancel".into()));
        assert!(last_system(&app).contains("Usage: /cancel"));
        app.handle_cmd(UiCmd::Submit("/cancel run-9".into()));
        pump(&mut app, &mut rx).await;
        assert!(system_messages(&app).iter().any(|m| m.contains("Failed to cancel queued run")));

        // Unknown slash command → falls through to a prompt.
        app.handle_cmd(UiCmd::Submit("/not-a-command".into()));
        pump(&mut app, &mut rx).await;

        // A regular prompt adds a user message and sets streaming.
        let before = app.chat.plain_messages().len();
        app.handle_cmd(UiCmd::Submit("tell me something".into()));
        assert!(app.chat.plain_messages().len() > before);
        assert!(app.state.streaming);
        pump(&mut app, &mut rx).await;
        // Prompt while streaming uses the enqueue policy.
        app.handle_cmd(UiCmd::Submit("another one".into()));
        pump(&mut app, &mut rx).await;
        app.state.streaming = false;
    }
}
