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
use crate::components::input::{Input, PendingDraft};
use crate::components::keymap_view::{KeymapAction, KeymapModel, KeymapOverlay, KeymapView};
use crate::components::menu::{
    MenuAction, MenuItem, MenuOptions, MenuOverlay, MenuSection, MenuState,
};
use crate::components::pager::{Pager, PagerAction, PagerOverlay};
use crate::components::provider_dialogs::{
    ProviderForm, ProviderFormAction, ProviderFormOverlay, ProviderListAction, ProviderListOverlay,
    ProviderListState,
};
use crate::components::sandbox_view::{
    platform_default_tier, PermissionKind, ProbeState, SandboxAction, SandboxPlatform,
    SandboxProbe, SandboxStatus, SandboxTier, SandboxView,
};
use crate::components::select_list::{SelectItem, SelectList, SelectListOptions};
use crate::components::skills_view::{parse_skills, SkillsAction, SkillsView};
use crate::components::usage_view::usage_from_state;
use crate::components::worktree_view::{
    WorktreeAction, WorktreeOverlay, WorktreeView, NEW_WORKTREE_COMMAND,
};
use crate::insert_history::{insert_history, write_history, HistoryScreen, HistoryWriter};
use crate::keybindings::KeybindingManager;
use crate::keys::key as Key;
use crate::keys::{is_key_release, parse_key};
use crate::notifications::{
    self, sequences_for, set_title_sequence, terminal_title, NotifyConfig, NotifyEvent, NotifyKind,
};
use crate::rpc::grpc_client::GrpcClient;
use crate::rpc::provider_types::{validate_provider_input, ProviderInfo, ProviderInput};
use crate::rpc::types::{
    AgentEvent, ModelInfo, RpcSessionState, SessionEntriesPage, SessionSummary, ThinkingLevel,
};
use crate::skills_cli::{
    summarize_outcome, SkillCatalogue, SkillOp, SkillOpOutcome, SkillsCli, SKILL_OP_TIMEOUT,
    TIMEOUT_MARKER,
};
use crate::terminal_image::{
    collect_kitty_image_ids, delete_kitty_images, extract_kitty_image_ids, get_capabilities,
    is_image_line, set_cell_dimensions, CellDimensions, ImageProtocol,
};
use crate::theme::{bold, fg, Chrome, Theme, DARK_THEME};
use crate::tui::{
    apply_theme_to_component, is_focusable, resolve_overlay_layout, set_component_focused,
    Component, OverlayOptions, SizeValue, SYNC_BEGIN, SYNC_END,
};
// `utils::extract_segments` is deliberately not imported any more: the overlay
// row belongs to the overlay (see `App::composite_line_at`), so compositing no
// longer copies the base's left/right insets.
use crate::utils::{
    highlight_matches, normalize_terminal_output, slice_by_column, strip_ansi_codes,
    truncate_to_width, visible_width, wrap_text_with_ansi, TruncateOptions,
};
use crate::version::VERSION;
use crate::worktree::{
    create_worktree, probe_repo, GitCli, WorktreeCreated, WorktreeInfo, WORKTREE_USAGE,
};
// Only `git_for_host`'s test build names the runner directly.
#[cfg(test)]
use crate::worktree::GitRunner;
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

impl<T: TerminalIo> HistoryScreen for App<T> {
    fn leave_alternate_screen(&mut self) {
        self.suspend_terminal();
    }

    fn enter_alternate_screen(&mut self) {
        self.reenter_alternate_screen();
    }

    fn write_scrollback(&mut self, data: &str) {
        self.terminal.write(data);
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
    /// `ctrl+g` — expand/collapse tool-output bodies.
    ToggleToolOutput,
    /// `ctrl+d` — fold runs of tool calls and thinking into compact rows.
    ToggleCompactActivity,
    /// `ctrl+x` — copy the most recent assistant message to the clipboard.
    CopyLastMessage,
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
    Autocomplete,
    /// `/model` selector.
    Selector,
    /// `/scoped-models` / `/models` — the enable-scope editor.
    Scoped,
    /// `/models default` — pick the agent-side default model.
    Default,
}

/// Which new-style popup menu a [`UiCmd::MenuSelected`]/[`UiCmd::MenuCancelled`]
/// came from. `/model`, `/sessions` and `/scoped-models` keep their own
/// `OverlayKind` routing below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuPurpose {
    /// `/theme` — pick and persist a palette.
    Theme,
    /// `/tools` — enable/disable the built-in tool set.
    Tools,
    /// `/sessions` — pick a session to switch to.
    Sessions,
    /// `/tree` — pick a session from the fork/clone hierarchy.
    SessionTree,
}

/// Who requested the session list (different overlays are built).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionsPurpose {
    Autocomplete,
    Browse,
    Tree,
}

/// Which `/…` command asked for a [`UiCmd::DiagnosticsLoaded`] payload — the
/// three commands share the "fetch a JSON document, show it in the pager" shape
/// and differ only in the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticsKind {
    /// `/metrics` — `get_runtime_metrics`.
    Metrics,
    /// `/snapshot` — `get_run_snapshot`.
    Snapshot,
    /// `/shell <cmd>` — the command's captured output.
    Shell,
}

impl DiagnosticsKind {
    /// System-message prefix used when the RPC failed.
    fn error_prefix(self) -> &'static str {
        match self {
            DiagnosticsKind::Metrics => "Failed to load runtime metrics",
            DiagnosticsKind::Snapshot => "Failed to load the run snapshot",
            DiagnosticsKind::Shell => "Shell command failed",
        }
    }
}

/// Async results + overlay events applied by the app loop.
#[derive(Debug)]
pub enum UiCmd {
    /// A local-only startup notice; never sent to the agent or saved in history.
    UpdateAvailable(String),
    // ── async results ─────────────────────────────────────────────────
    Refreshed(Result<RpcSessionState, String>),
    RefreshCompleted {
        result: Result<RpcSessionState, String>,
        session_id: String,
        compaction_revision: u64,
    },
    ModelsLoaded {
        result: Result<Vec<ModelInfo>, String>,
        purpose: ModelsPurpose,
    },
    SessionsLoaded {
        result: Result<Vec<SessionSummary>, String>,
        purpose: SessionsPurpose,
    },
    ForkMessagesLoaded(Result<Value, String>),
    /// The agent answered the recommendation question for `draft`: `Some` is the
    /// skill to offer, `None` means send the draft unchanged.
    SkillRecoSuggested {
        draft: String,
        suggestion: Option<(String, String)>,
    },
    /// A recommended skill finished installing: `true` appends `/skill` to the
    /// held draft and sends it, `false` reports the failure and keeps the draft.
    SkillRecoInstalled {
        draft: String,
        skill: String,
        installed: bool,
    },
    SetModelDone {
        set_result: Result<(), String>,
        state: Option<RpcSessionState>,
    },
    ModelCycled {
        result: Result<Value, String>,
        state: Option<RpcSessionState>,
    },
    ThinkingCycled(Result<Value, String>),
    CompactDone {
        session_id: String,
        result: Result<String, String>,
    },
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
        /// The usage ledger (`get_session_stats`). `/stats` was folded into
        /// `/status`, so one command answers "what is this session doing" and
        /// "what has it cost" instead of splitting them across two panels.
        stats: Result<Value, String>,
    },
    /// Something the input box refused to do, phrased for the user (a paste
    /// past the message-length cap).
    InputNotice(String),
    SessionSwitched {
        /// The session the flow asked for. A result whose target is not the
        /// newest request was superseded by another pick (see
        /// `App::latest_session_switch`) and must not be applied.
        target: String,
        /// False when the agent declined the switch (it never does today, but
        /// the wire contract has the field): nothing changed on the wire, so
        /// the transcript on screen is still the current session's.
        switched: bool,
        result: Result<(), String>,
        state: Option<RpcSessionState>,
        /// The loaded tail page of the target session's history (see
        /// [`load_history_tail`]).
        history: Result<Value, String>,
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
        history: Result<Value, String>,
        label: String,
    },
    NewSessionDone {
        result: Result<Value, String>,
        state: Option<RpcSessionState>,
    },
    CloneDone {
        result: Result<Value, String>,
        state: Option<RpcSessionState>,
        history: Result<Value, String>,
    },
    /// An older page of the displayed session's history, requested by scrolling
    /// up past the top of the transcript.
    HistoryPageLoaded {
        session_id: String,
        /// The cursor the request asked from — the guard against a page that
        /// would move the cursor backwards or stall it.
        before: i64,
        result: Result<Value, String>,
    },
    ModelSelected(SelectItem),
    /// The provider list (`/providers`) or a mutation's follow-up refresh.
    ProvidersLoaded(Result<Vec<ProviderInfo>, String>),
    /// A provider mutation finished (`action` is the user-visible label).
    ProviderActionDone {
        action: String,
        result: Result<(), String>,
    },
    /// `/usage` — the state snapshot the usage panel renders.
    UsageLoaded(Result<Box<RpcSessionState>, String>),
    /// `/agent` — the `get_agent_info` payload.
    AgentInfoLoaded(Result<Value, String>),
    /// `/history <query>` — the `search_session_history` payload.
    HistorySearched {
        query: String,
        result: Result<Value, String>,
    },
    /// `/tool-output` — the `list_tool_calls` page.
    ToolCallsLoaded(Result<Value, String>),
    /// `/tool-output <call-id>` — the stored output of one tool call.
    ToolOutputLoaded {
        tool_call_id: String,
        result: Result<Value, String>,
    },
    /// `/autocompact [on|off]`.
    AutoCompactionSet {
        enabled: bool,
        result: Result<(), String>,
    },
    /// `/autoretry [on|off]`.
    AutoRetrySet {
        enabled: bool,
        result: Result<(), String>,
    },
    /// A one-shot session setting write (`/append-prompt`).
    SessionSettingWritten {
        success_message: String,
        error_prefix: &'static str,
        result: Result<(), String>,
    },
    /// `/export` — the `export_html` payload (`{path}`).
    SessionExported(Result<Value, String>),
    /// `/tools` — the tool set the agent accepted.
    ToolsChanged(Result<Vec<String>, String>),
    /// A popup-menu selection (see [`MenuPurpose`]).
    MenuSelected {
        purpose: MenuPurpose,
        values: Vec<String>,
    },
    /// A popup menu closed itself (`escape`).
    MenuCancelled(MenuPurpose),
    /// Copy `text` (`/copy`, `ctrl+x`, pager `y`).
    CopyRequest(String),
    /// A model was picked for the agent-side default (`/models default`).
    SetDefaultModel(Vec<String>),
    /// `/providers` list actions (see `provider_dialogs::ProviderListAction`).
    ProviderEdit(ProviderInfo),
    ProviderDelete(String),
    ProviderSetKey(String),
    ProviderAdd,
    ProviderSync(&'static str),
    /// An add/edit form submitted (`create_only` is already baked into the
    /// `ProviderInput` by `ProviderForm::to_input`).
    ProviderSubmit(Box<ProviderInput>),
    ProviderFormCancelled,
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

    // ── sandbox / permission ──────────────────────────────────────────
    /// `/sandbox` — a `probe_sandbox` / `probe_windows_sandbox` answer.
    SandboxProbeLoaded {
        result: Result<Value, String>,
    },
    /// `/sandbox` — a `set_sandbox_policy` answer. The payload carries the
    /// agent's *actual* tier, so a downgrade is visible rather than assumed
    /// away.
    SandboxPolicySet {
        result: Result<Value, String>,
    },
    /// `/sandbox` — apply this tier (the panel's `enter`).
    SandboxTierRequested(SandboxTier),
    /// `/sandbox` / `/permission` — apply a permission level.
    PermissionLevelRequested(PermissionKind),
    /// `/sandbox` `r` — re-probe the host instead of reusing the cache.
    SandboxProbeRequested,
    /// `/permission <level>` — the agent's answer to `set_permission_level`.
    PermissionLevelSet {
        level: PermissionKind,
        result: Result<(), String>,
    },

    // ── skills ────────────────────────────────────────────────────────
    /// `/skills` — the `get_commands` catalogue (the installed set).
    SkillsLoaded(Result<Value, String>),
    /// `/skills` — a `future skills list --json` answer: the platform
    /// catalogue (`SkillsCli::list`), which carries the versions an install
    /// pins and the ones an upgrade compares against.
    SkillsCatalogueLoaded(Result<SkillCatalogue, String>),
    /// The recommender's own `future skills list --json` answer — the same
    /// catalogue, asked for before the user sent anything (see
    /// [`App::prefetch_skill_catalogue`]). Its own variant because nothing on
    /// screen asked for it: it fills the cache and is *silent* on failure (a
    /// panel error or a transcript line here would report a call the user never
    /// made).
    SkillsCataloguePrefetched(Result<SkillCatalogue, String>),
    /// `/skills` `r` — a `refresh_skills` re-scan finished.
    SkillsRefreshed(Result<Value, String>),
    /// `/skills` `ctrl+o` — show the highlighted skill's preview in the pager.
    SkillsDetailRequested,
    /// `/skills` — insert this skill's name into the prompt input (`enter`).
    SkillChosen(String),
    /// `/skills` `r` — re-scan the skill directories.
    SkillsRefreshRequested,
    /// `/skills` `i`/`u`/`U` — one mutating operation the panel asked for,
    /// before the app knows the version to pin.
    SkillsMutationRequested(SkillsMutation),
    /// A `future skills …` child finished (run on the blocking pool: it is a
    /// process, and the render loop must never wait for one).
    SkillOpDone {
        outcome: SkillOpOutcome,
        /// `true` when the child completed, and therefore may have changed
        /// what the panel lists: only then does the app re-pull both sources.
        /// A failed or killed child leaves the installed set as it was.
        catalogue_dirty: bool,
    },

    // ── worktree ──────────────────────────────────────────────────────
    /// `/worktree` — a `git worktree list` answer (plus one `git status` per
    /// entry), fetched on the blocking pool.
    WorktreesLoaded {
        result: Result<Vec<WorktreeInfo>, String>,
    },
    /// `/worktree new <name>` — the `git worktree add` answer. `Ok` carries the
    /// plan so the transcript can name the branch that was created.
    WorktreeAdded {
        result: Result<WorktreeCreated, String>,
    },
    /// `/worktree` — a worktree row was confirmed: switch the session's cwd to
    /// that path (the same RPC `/cwd` sends).
    WorktreeSwitchRequested(String),
    /// `/worktree` — the create row was confirmed: a name still has to be
    /// typed, so the app puts `/worktree new ` in the prompt.
    WorktreeNewRequested,
    /// `/worktree` `ctrl+r` — re-read git's list.
    WorktreeRefreshRequested,

    // ── session lifecycle / diagnostics ───────────────────────────────
    /// `/title` — the `generate_session_title` suggestion.
    SessionTitleGenerated(Result<Value, String>),
    /// `/title` — the follow-up `set_session_name` that applies it.
    SessionTitleApplied {
        title: String,
        result: Result<(), String>,
    },
    /// `/delete --yes` — the `delete_session` outcome for a named session.
    SessionDeleted {
        session_id: String,
        result: Result<Value, String>,
    },
    /// `/metrics`, `/snapshot`, `/shell` — a JSON payload for the pager.
    DiagnosticsLoaded {
        kind: DiagnosticsKind,
        result: Result<Value, String>,
    },
    /// `/context on|off`.
    ContextFilesSet {
        enabled: bool,
        result: Result<(), String>,
    },
    // ── keymap ────────────────────────────────────────────────────────
    /// `/keymap` — a key was captured for an action (`bind`), or the user
    /// restored it (`reset`, `description` = the one action, `None` = all).
    /// Both are applied to the live manager and written back to
    /// `~/.future/tui/keybindings.json`.
    KeymapBind {
        description: String,
        key: String,
    },
    KeymapReset {
        description: Option<String>,
    },

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
    /// Terminal bell (BEL) when a run of ours completes or errors. On by default.
    pub bell_on_complete: Option<bool>,
    /// Palette id persisted by `/theme` (`themeId`).
    pub theme_id: Option<String>,
    /// Notification channels (`notify`, camelCase keys). Absent ⇒ defaults.
    pub notify: Option<NotifyConfig>,
    /// Offer a skill recommendation before sending a message (`skillRecommend`).
    /// On by default, like the desktop toggle (PRD v1.6 §3); `/skill-recommend
    /// off` opts out.
    pub skill_recommend: Option<bool>,
}

impl TuiSettings {
    /// `bellOnComplete` with the default of `true` when absent.
    pub fn bell_enabled(&self) -> bool {
        self.bell_on_complete.unwrap_or(true)
    }

    /// `skillRecommend` with the default of `true` when absent.
    pub fn skill_recommend_enabled(&self) -> bool {
        self.skill_recommend.unwrap_or(true)
    }

    /// The effective notification config for the app loop.
    ///
    /// `bellOnComplete` is the legacy spelling of `notify.bell` and still wins
    /// when it is explicitly `false`; the run-state window title is on for a
    /// settings file that predates `notify` (nothing else ever wrote a title,
    /// unlike the upstream assume-title-owner case).
    pub fn notify_config(&self) -> NotifyConfig {
        let mut config = self.notify.unwrap_or_else(notifications::default_config);
        config.bell = config.bell && self.bell_enabled();
        if self.notify.is_none() {
            config.title = true;
        }
        config
    }

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
        if let Some(bell) = self.bell_on_complete {
            obj.insert("bellOnComplete".into(), Value::Bool(bell));
        }
        if let Some(theme) = &self.theme_id {
            obj.insert("themeId".into(), Value::String(theme.clone()));
        }
        if let Some(config) = &self.notify {
            if let Ok(value) = serde_json::to_value(config) {
                obj.insert("notify".into(), value);
            }
        }
        if let Some(recommend) = self.skill_recommend {
            obj.insert("skillRecommend".into(), Value::Bool(recommend));
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
            bell_on_complete: v.get("bellOnComplete").and_then(Value::as_bool),
            theme_id: v.get("themeId").and_then(Value::as_str).map(String::from),
            // A missing or malformed `notify` object falls back to `None`
            // (= defaults) instead of failing the whole settings load.
            notify: v
                .get("notify")
                .and_then(|value| serde_json::from_value::<NotifyConfig>(value.clone()).ok()),
            skill_recommend: v.get("skillRecommend").and_then(Value::as_bool),
        }
    }
}

/// True when the draft already names a skill (`/name`), i.e. the user picked
/// one themselves — the same rule as the desktop composer.
fn draft_picks_skill(draft: &str) -> bool {
    draft
        .split_whitespace()
        .any(|token| token.len() > 1 && token.starts_with('/'))
}

/// Skill-recommendation state for the composer.
///
/// The client owns the trigger rules (PRD v1.6 §3, §7), so the TUI keeps its
/// own small state machine: a draft that is being asked about, or a
/// recommendation waiting for the user to decide. While either is active the
/// draft is held in the input box un-sent, and `a` / `Esc` decide (PRD §6.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillRecoState {
    /// Nothing in flight; submissions behave normally.
    Idle,
    /// The agent is being asked about `draft`.
    Pending { draft: String },
    /// A recommendation is on screen; the draft waits for the user.
    Suggested {
        draft: String,
        skill: String,
        summary: String,
    },
}

impl SkillRecoState {
    /// The line shown above the input, or `None` when there is nothing to show.
    ///
    /// The pending line carries the frame's spinner glyph, so the wait looks
    /// like work in progress rather than a stuck box (the input is locked for
    /// the same window — see [`App::editor_locked_by_reco`]).
    fn prompt_line(&self, spinner_frame: usize) -> Option<String> {
        match self {
            SkillRecoState::Idle => None,
            SkillRecoState::Pending { .. } => {
                let glyph = crate::components::footer::SPINNER_FRAMES
                    [spinner_frame % crate::components::footer::SPINNER_FRAMES.len()];
                Some(format!("{glyph} Looking for a skill that fits…"))
            }
            SkillRecoState::Suggested { skill, summary, .. } => {
                let detail = if summary.trim().is_empty() {
                    String::new()
                } else {
                    format!("  {}", summary.trim())
                };
                Some(format!(
                    "Recommended skill  /{skill}{detail}    [a] install & use    [Esc] send without it"
                ))
            }
        }
    }
}

struct AppState {
    model: String,
    thinking: String,
    streaming: bool,
    compacting: bool,
    compaction_requested: bool,
    compaction_revision: u64,
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
    session_name: Option<String>,
    tokens_in: i64,
    tokens_out: i64,
    tokens_cache_r: i64,
    tokens_cache_w: i64,
    total_cost: f64,
    auto_compaction_enabled: bool,
    /// Auto-retry is agent-side session state that `get_state` does not report
    /// (agent `ServerSession::auto_retry`, default on), so the TUI mirrors it
    /// locally for the `/autoretry` toggle.
    auto_retry_enabled: bool,
    /// `get_state.permissionLevel` — the level the `/permission` picker starts
    /// on. The agent's own default is `all`.
    permission_level: PermissionKind,
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
            compacting: false,
            compaction_requested: false,
            compaction_revision: 0,
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
            session_name: None,
            tokens_in: 0,
            tokens_out: 0,
            tokens_cache_r: 0,
            tokens_cache_w: 0,
            total_cost: 0.0,
            auto_compaction_enabled: true,
            auto_retry_enabled: true,
            permission_level: PermissionKind::All,
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

/// A `switch_session` request: the session it started from and the one it
/// asked for (see `App::latest_session_switch`).
struct SessionSwitchRequest {
    from: String,
    target: String,
}

/// Where the transcript on screen sits in its session's history.
///
/// The TUI reads display history through the agent's indexed pager
/// (`get_session_entries` + `before`), never `get_messages`: that one has no
/// cursor at all, so a long session costs a single unbounded response — the
/// reason a transcript could not be loaded at all before the message cap was
/// raised, and still the reason a 10 MB session felt like a stall.
///
/// The cursor belongs to one session. A switch replaces it wholesale, so the
/// struct is keyed by session id rather than reset at every call site: a page
/// whose request was already in flight when the user moved on must be dropped,
/// not prepended into another session's transcript.
#[derive(Debug, Clone, Default)]
struct HistoryPaging {
    /// The session `has_more` / `next_before` describe (empty = no transcript).
    session_id: String,
    /// Older rows exist above the top of the transcript.
    has_more: bool,
    /// Exclusive backward cursor for the next older page.
    next_before: i64,
    /// A page request is in flight, so a scroll cannot start a second one.
    loading: bool,
}

impl HistoryPaging {
    /// Forget the cursor: the transcript it described is gone (a switch, a
    /// fresh load, `/new`) or its session is known to have no history.
    fn reset(&mut self, session_id: &str) {
        *self = Self {
            session_id: session_id.to_string(),
            ..Self::default()
        };
    }

    /// Adopt a loaded page of `session_id`: the older-history cursor moves to
    /// the page's `hasMore`/`nextOffset`.
    ///
    /// A response that claims more history without advancing the cursor is
    /// treated as the end: the alternative is a scroll-up that fetches the same
    /// page forever. That is the same guard the desktop's page loop applies to
    /// a non-advancing `nextOffset`.
    fn adopt(&mut self, session_id: &str, page: &SessionEntriesPage, requested_before: i64) {
        self.session_id = session_id.to_string();
        self.loading = false;
        self.has_more =
            page.has_more && !page.entries.is_empty() && page.next_offset < requested_before;
        self.next_before = page.next_offset;
    }

    /// [`Self::adopt`] for a request that failed: nothing was prepended, so the
    /// cursor keeps its position and only the in-flight flag clears (the user
    /// can scroll again — a retry is one more PageUp).
    fn failed(&mut self) {
        self.loading = false;
    }
}

/// Load the tail of `session_id`'s display history.
///
/// `get_session_entries` is the only read here with a cursor, so it is the one
/// that keeps a long session from arriving as a single unbounded response. Two
/// cases still need the older, uncapped `get_messages`:
///
/// - an agent that predates backward paging answers with an error (or ignores
///   `before` and returns no page at all), and
/// - a session with no persisted history (the agent never wrote it, e.g. an
///   ephemeral one) still has a live in-memory context.
///
/// In both, "the pager came up empty" must not read as "the session is empty":
/// falling back shows the conversation instead of a blank transcript. Both
/// responses are accepted by the same parser ([`SessionEntriesPage`]), so the
/// caller cannot tell them apart — except that the fallback carries no cursor
/// and paging is off. When the fallback also fails, the pager's error is the
/// reported one: it names the storage the read actually wanted.
async fn load_history_tail(client: &GrpcClient, session_id: &str) -> Result<Value, String> {
    match client.session_tail_page(session_id).await {
        Ok(page) if !page.entries.is_empty() => Ok(page.to_value()),
        paged => {
            let primary = paged.err();
            client
                .get_messages()
                .await
                .map_err(|fallback| primary.unwrap_or(fallback))
        }
    }
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
/// Cadence for the DSR cursor-position recheck while streaming. A tmux
/// client attach can reset the terminal's cursor without any signal when the
/// pane size is unchanged and focus-events are off; periodically re-reading
/// the real cursor position is the last-resort net that catches that case.
const CURSOR_RECHECK_INTERVAL: Duration = Duration::from_millis(1000);
const SEGMENT_RESET: &str = "\x1b[0m\x1b]8;;\x07"; // SGR reset + OSC 8 close
/// `/history` asks the agent for this many matches (the agent caps its
/// `search_session_history` limit at `HISTORY_MAX_MATCHES` = 20).
const HISTORY_SEARCH_LIMIT: usize = 20;

fn compaction_completed_text(before: Option<&Value>, after: Option<&Value>) -> String {
    match (
        before.and_then(Value::as_u64),
        after.and_then(Value::as_u64),
    ) {
        (Some(before), Some(after)) => {
            format!("Context compacted: {before} → {after} tokens (estimated)")
        }
        _ => "Context compacted".into(),
    }
}

/// `crypto.randomUUID()`.
fn random_id() -> String {
    Uuid::new_v4().to_string()
}

/// Built-in tools the agent exposes (`agent/src/tools/mod.rs::all_tools`).
/// Used by `/tools`, which drives `set_tools`/`disable_tools`.
pub const BUILTIN_TOOLS: [&str; 4] = ["read", "write", "edit", "shell"];

/// Translate a [`ProviderListAction`] into a `UiCmd` (overlay callback body).
fn provider_list_sink(tx: mpsc::UnboundedSender<UiCmd>) -> Box<dyn FnMut(ProviderListAction)> {
    let tx = tx;
    Box::new(move |action: ProviderListAction| {
        let cmd = match action {
            ProviderListAction::Edit(info) => UiCmd::ProviderEdit(info),
            ProviderListAction::Delete(info) => UiCmd::ProviderDelete(info.id),
            ProviderListAction::SetKey(info) => UiCmd::ProviderSetKey(info.id),
            ProviderListAction::Add => UiCmd::ProviderAdd,
            ProviderListAction::SyncModels => UiCmd::ProviderSync("sync_future_models"),
            ProviderListAction::ReloadAuth => UiCmd::ProviderSync("reload_auth"),
            ProviderListAction::Cancelled => UiCmd::OverlayCancel,
            ProviderListAction::None | ProviderListAction::Moved => return,
        };
        let _ = tx.send(cmd);
    })
}

/// Translate a [`ProviderFormAction`] into a `UiCmd`.
fn provider_form_sink(tx: mpsc::UnboundedSender<UiCmd>) -> Box<dyn FnMut(ProviderFormAction)> {
    let tx = tx;
    Box::new(move |action: ProviderFormAction| {
        let cmd = match action {
            ProviderFormAction::Submit(input) => UiCmd::ProviderSubmit(Box::new(input)),
            ProviderFormAction::Cancel => UiCmd::ProviderFormCancelled,
            ProviderFormAction::None | ProviderFormAction::Changed => return,
        };
        let _ = tx.send(cmd);
    })
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
/// The name shown for a session row: its name, else its first message, else
/// its id (the same fallback chain the old `SelectList` picker used).
fn session_display_name(s: &SessionSummary) -> String {
    s.session_name
        .clone()
        .or_else(|| s.first_message.clone())
        .unwrap_or_else(|| s.id.clone())
}

/// Home-relative, width-capped cwd for a session row's description.
///
/// `home` is a parameter rather than an environment read so the rule is a pure
/// function: a host without `HOME` matches nothing and needs no special case.
fn shorten_cwd(cwd: &str, home: &str) -> String {
    let shown = if !home.is_empty() && cwd.starts_with(home) {
        format!("~{}", &cwd[home.len()..])
    } else {
        cwd.to_string()
    };
    if shown.chars().count() > 40 {
        let tail: String = shown.chars().skip(shown.chars().count() - 39).collect();
        format!("…{tail}")
    } else {
        shown
    }
}

/// A JSON object field as a string — empty when the key is absent or holds a
/// non-string (the agent's payloads omit fields rather than nulling them).
fn json_str<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// `on`/`off` label shared by the `/autocompact` and `/autoretry` readbacks.
fn enabled_label(enabled: bool) -> &'static str {
    if enabled {
        "enabled"
    } else {
        "disabled"
    }
}

/// `/agent` panel — the fields `get_agent_info` returns (the TUI appends its
/// own session/model/cwd lines when it renders).
fn agent_info_lines(info: &Value) -> Vec<String> {
    let skills = info
        .get("skillsCount")
        .and_then(Value::as_i64)
        .map(|count| count.to_string())
        .unwrap_or_else(|| "-".to_string());
    vec![
        "Agent information:".to_string(),
        format!("  version: {}", json_str(info, "version")),
        format!("  instance id: {}", json_str(info, "agentInstanceId")),
        format!("  skills discovered: {skills}"),
    ]
}

/// `/stats` panel — `get_session_stats` (message/tool/token counters and the
/// cumulative cost). Counters render as `-` when the payload omits them.
/// The usage ledger, as the tail of `/status`.
///
/// This used to be the body of a `/stats` panel. `/status` is now the single
/// place to read session state, so the ledger lives here and `/stats` is gone:
/// one command, one screen, no row printed twice. The `session` line the old
/// panel carried is dropped (it is `/status`'s `Session:` row) and so is its
/// header — the section is introduced by its own first label.
fn session_ledger_lines(stats: &Value) -> Vec<String> {
    let tokens = stats.get("tokens");
    let counter = |key: &str| {
        stats
            .get(key)
            .and_then(Value::as_i64)
            .map(|n| n.to_string())
            .unwrap_or_else(|| "-".to_string())
    };
    let token = |key: &str| {
        tokens
            .and_then(|t| t.get(key))
            .and_then(Value::as_i64)
            .map(|n| n.to_string())
            .unwrap_or_else(|| "-".to_string())
    };
    vec![
        format!("  file: {}", json_str(stats, "sessionFile")),
        format!("  messages: {}", counter("totalMessages")),
        format!(
            "  user/assistant: {}/{}",
            counter("userMessages"),
            counter("assistantMessages")
        ),
        format!(
            "  tool calls/results: {}/{}",
            counter("toolCalls"),
            counter("toolResults")
        ),
        format!("  tokens in/out: {}/{}", token("input"), token("output")),
        format!(
            "  tokens cache read/write: {}/{}",
            token("cacheRead"),
            token("cacheWrite")
        ),
        format!("  tokens total: {}", token("total")),
        format!(
            "  cost: {}",
            stats
                .get("cost")
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".to_string())
        ),
    ]
}

/// `/history <query>` results — one block per match, ready for the pager
/// (whose `/` search and `n`/`N` jumps work on these lines).
///
/// Each block is three rows, humans first:
///
/// ```text
/// 15:06:40 · assistant · edit
///   {"path":"src/greeting.rs"}      <- "greeting" marked, not the whole row
///   #entry_mock_assistant_1 · tool_call · call_mock_1
/// ```
///
/// 1. when and who (`HH:MM:SS` + role + tool name),
/// 2. the snippet, with **every** occurrence of the query highlighted by
///    [`crate::utils::highlight_matches`] so the reason a row matched is
///    visible instead of implied,
/// 3. the machine identifiers — the (48-character) entry id, the block kind and
///    the tool-call id — dimmed on one row, so they stay available without
///    competing with the text a human reads.
///
/// `highlight_bg` is the caller's `Chrome::from_theme(theme).highlight_bg`, the
/// same background the pager paints its own search hit with.
fn history_match_lines(query: &str, payload: &Value, highlight_bg: u8) -> Vec<String> {
    let matches: &[Value] = payload
        .get("matches")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    // No match at all returns no lines, so the caller reports the miss in a
    // system message instead of opening a pager with only a header.
    if matches.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!("History matches for '{query}': {}", matches.len())];
    for entry in matches {
        // A blank row before each block: three rows per match read as one unit
        // only if the next match does not start flush against the ids above it.
        lines.push(String::new());
        lines.extend(history_match_block(query, entry, highlight_bg));
    }
    if payload
        .get("hasMore")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        lines.push(String::new());
        lines.push("… more matches exist; narrow the query.".to_string());
    }
    lines
}

/// One match block of [`history_match_lines`]: the human row, the highlighted
/// snippet, the dimmed identifiers.
fn history_match_block(query: &str, entry: &Value, highlight_bg: u8) -> [String; 3] {
    let mut head = format!(
        "{} · {}",
        crate::theme::dim(&history_match_time(entry)),
        bold(json_str(entry, "role"))
    );
    let tool = json_str(entry, "toolName");
    if !tool.is_empty() {
        head.push_str(" · ");
        head.push_str(tool);
    }
    let snippet = json_str(entry, "snippet").replace(['\n', '\r'], " ");
    [
        head,
        format!("  {}", highlight_matches(&snippet, query, highlight_bg)),
        format!("  {}", crate::theme::dim(&history_match_ids(entry))),
    ]
}

/// `HH:MM:SS` in the terminal's local zone for a match's `timestampMs`.
///
/// The field is `Option<i64>` upstream (entries without a recorded time), so a
/// missing one renders `--:--:--` rather than the epoch a bare `0` would pass
/// off as a time the user could look for.
fn history_match_time(entry: &Value) -> String {
    match entry
        .get("timestampMs")
        .and_then(Value::as_i64)
        .and_then(chrono::DateTime::from_timestamp_millis)
    {
        Some(time) => time
            .with_timezone(&chrono::Local)
            .format("%H:%M:%S")
            .to_string(),
        None => "--:--:--".to_string(),
    }
}

/// The machine row of one match: `#<entryId> · <kind>`, plus `· call <id>` when
/// the block carries one (a text block has no tool call). `entryId`/`kind` come
/// with every match, so they need no missing-field placeholder.
fn history_match_ids(entry: &Value) -> String {
    let call_id = json_str(entry, "toolCallId");
    let mut parts = vec![
        format!("#{}", json_str(entry, "entryId")),
        json_str(entry, "kind").to_string(),
    ];
    if !call_id.is_empty() {
        parts.push(format!("call {call_id}"));
    }
    parts.join(" · ")
}

/// `/tool-output` with no argument — the stored tool calls of the current (or
/// last) run, from `list_tool_calls`.
fn tool_call_lines(page: &Value) -> Vec<String> {
    let tools: &[Value] = page
        .get("tools")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut lines = vec!["Tool calls in this run:".to_string()];
    for tool in tools {
        lines.push(format!(
            "{}  {}  {}",
            json_str(tool, "toolCallId"),
            json_str(tool, "name"),
            json_str(tool, "status")
        ));
    }
    if tools.is_empty() {
        lines.push("  (none recorded yet)".to_string());
    }
    if page
        .get("hasMore")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        lines.push("… more calls than fit in this page.".to_string());
    }
    lines.push("Run /tool-output <call-id> for the full stored output.".to_string());
    lines
}

/// Did a `tool_end` event report a failure?
///
/// The agent sends the structured result alongside the text (`error`, and for a
/// shell command `exit_code` with `is_soft_fail` already resolved), so the TUI
/// does not have to read an exit code back out of the output — and a soft fail
/// (bare `grep`, `diff`, … exiting 1) stays a completed call, exactly as the
/// desktop's projection treats it.
pub fn tool_end_failed(data: &Value) -> bool {
    if data
        .get("error")
        .and_then(Value::as_str)
        .is_some_and(|error| !error.trim().is_empty())
    {
        return true;
    }
    let exit_code = data.get("exit_code").and_then(Value::as_i64);
    let soft = data
        .get("is_soft_fail")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    exit_code.is_some_and(|code| code != 0) && !soft
}

/// `/tool-output <call-id>` — the untruncated stored output of one tool call.
fn tool_output_lines(tool_call_id: &str, payload: &Value) -> Vec<String> {
    let output = payload.get("output");
    let text = match output.and_then(|o| o.get("text")).and_then(Value::as_str) {
        Some(text) => text,
        // No stored result for that call: no lines, so the caller reports the
        // miss in a system message.
        None => return Vec::new(),
    };
    let is_error = output
        .and_then(|o| o.get("isError"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut lines = vec![format!(
        "Output of {tool_call_id} ({}):",
        if is_error { "error" } else { "ok" }
    )];
    lines.extend(text.lines().map(str::to_string));
    lines
}

/// `/export` readback: the written file plus its size (the size read is
/// best-effort — the agent and the TUI need not share a filesystem).
fn export_result_message(value: &Value) -> String {
    if let Some(path) = value.get("path").and_then(Value::as_str) {
        return match std::fs::metadata(path) {
            Ok(meta) => format!("Exported session to {path} ({} bytes).", meta.len()),
            Err(_) => format!("Exported session to {path}."),
        };
    }
    if value.get("html").and_then(Value::as_str).is_some() {
        return "The agent returned the export as HTML text and wrote no file.".to_string();
    }
    "Session export finished (the agent reported no file path).".to_string()
}

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
#[cfg(test)]
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
    /// Current chrome palette; fanned out to every widget by `apply_theme`.
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
    /// `~/.future/tui/keybindings.json` — the user's key overrides, read at
    /// startup and written by `/keymap`. It sits beside `settings.json`
    /// (derived from its path) and is deliberately a separate file: the two
    /// never shared a schema and both are hand-editable.
    keybindings_path: PathBuf,
    /// Why the last `keybindings.json` read did not apply — unknown actions,
    /// keys no terminal sends, a file that is not JSON. Shown by `/keymap`
    /// rather than swallowed: a silently ignored config is worse than a loud
    /// one.
    keybinding_problems: Vec<String>,
    /// Entries of `keybindings.json` that name an action this build does not
    /// have. Kept and written back on save, so running an older build once
    /// (or a typo in one action) cannot quietly delete the rest.
    keybinding_unknown: Vec<(String, String)>,
    enabled_model_ids: Option<Vec<String>>,
    connection_lost: bool,
    tui_settings: TuiSettings,
    tui_settings_path: PathBuf,
    slash_commands: Vec<SlashCommand>,
    /// Session id → the draft the user left there, plus the pastes and images
    /// that draft still refers to. Caching the text alone would turn a restored
    /// `[Image #1]` into a marker with nothing behind it.
    session_input_cache: HashMap<String, (String, PendingDraft)>,
    /// The newest `switch_session` request the app has issued — kept after its
    /// result is applied, so a *late* result from an older request is still
    /// recognisable. Two picks can overlap (the sessions menu stays open until
    /// the first result lands) and the agent has no notion of a "current"
    /// session: whichever RPC lands last wins on the wire, so results are
    /// matched against this instead of being applied in arrival order.
    latest_session_switch: Option<SessionSwitchRequest>,
    state: AppState,
    running: bool,
    cli_options: CliOptions,
    cli_initial_prompt: Option<String>,
    pending_name_arg: Option<String>,
    pending_update_notice: Option<String>,
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
    cursor_recheck_at: Instant,
    cursor_recheck_row: Option<usize>,
    timers: Vec<(Instant, TimerId)>,
    previous_width: usize,
    previous_height: usize,
    previous_kitty_image_ids: BTreeSet<u32>,
    full_redraw_count: usize,
    pending_approval: Option<PendingApproval>,
    cached_models: Vec<String>,
    /// Model id → `ModelInfo.supports_images`, from the `list_models` the TUI
    /// has seen (startup, `/model`, the autocomplete fetch). The paste path
    /// needs it *before* an image is attached to say whether the model will
    /// actually see the picture: the agent degrades an image a text-only model
    /// cannot take into a file path, so "attached" alone would be a lie.
    /// Deliberately not folded into `cached_models`, whose emptiness is what
    /// drives the `/model` autocomplete fetch.
    model_image_support: HashMap<String, bool>,
    cached_sessions: Vec<String>,
    /// Session id → display label, captured when the sessions/tree menu is
    /// built. `MenuState` hands back values (ids) only, so the label used in
    /// the "Switched to …" notice is looked up here.
    session_labels: HashMap<String, String>,
    /// Clipboard backend for `/copy`, `ctrl+x` and the pager's `y`.
    clipboard: crate::clipboard::Clipboard,
    /// Clipboard *reader* for `ctrl+v`: an image the clipboard holds (written
    /// to a temp file the agent can read afterwards), or its text. Injectable,
    /// so no unit test reads the real clipboard.
    clipboard_capture: crate::paste::ClipboardCapture,
    /// `/tools` selection the TUI last pushed to the agent. The agent has no
    /// `get_tools` command, so this is the only source for the menu's initial
    /// state (all built-ins when `None`).
    enabled_tools: Option<Vec<String>>,
    /// Set while the input line is capturing a secret for `/provider-key`:
    /// the next submission is sent as that provider's API key instead of a
    /// prompt, and an empty submission clears the stored key.
    pending_secret: Option<String>,
    /// Last rejected `/providers` action explanation.
    provider_notice: Option<String>,
    /// `input_tx` handed to `start`, kept so the terminal can be suspended for
    /// the external editor and resumed with the same wiring (`/editor`).
    input_tx: Option<mpsc::UnboundedSender<UiInput>>,
    /// Last known sandbox probe + tier, cached across `/sandbox` opens (the
    /// desktop probes once per webview process too). `r` inside the panel
    /// forces a fresh probe.
    sandbox: Option<SandboxStatus>,
    /// `/skills` — the platform catalogue as `future skills list --json` last
    /// reported it: the source of the version an install pins and the one an
    /// upgrade compares against. Kept here rather than only in the panel, so a
    /// mutation whose action carries no version stays resolvable.
    skills_catalogue: Option<SkillCatalogue>,
    /// `/skills` — the `future skills …` plumbing, or `None` when this host has
    /// no `future` executable to run (see [`skills_cli_for_host`]).
    skills_cli: Option<Arc<SkillsCli>>,
    /// The recommender's one quiet `future skills list --json` attempt has been
    /// made — see [`App::prefetch_skill_catalogue`]. One attempt per session is
    /// deliberate: it is a background call nobody asked for, so a failure is not
    /// retried on every keystroke (the `/skills` panel is where a catalogue
    /// failure is *reported*, and opening it — or `r` — retries).
    skills_catalogue_prefetch_started: bool,
    /// `/skills` — the set `U` named on its first press. The second press runs
    /// the upgrade only while the set is unchanged, so a catalogue that moved
    /// under the confirmation cannot upgrade more than the prompt listed.
    skills_upgrade_armed: Option<Vec<String>>,
    /// `/skills` — a `future skills …` child is running. `i`/`u` mark their own
    /// row in the panel, but `U` has no row, and two installers writing the
    /// same package directory at once is not something to discover by accident.
    skills_op_running: bool,
    /// Skill recommendation for the draft being submitted. Whether the feature
    /// is on comes from `self.tui_settings.skill_recommend_enabled()`.
    skill_reco: SkillRecoState,
    /// Set while a held draft is being re-submitted from the recommendation
    /// flow, so the intercept stands down for exactly one pass.
    skill_reco_send_through: bool,
    /// The TUI's own daily budget (`~/.future/tui/skill_reco.json`).
    skill_reco_path: PathBuf,
    /// `/worktree` — the `git` plumbing. Every call spawns a process, so the
    /// app runs all of them on the blocking pool; the runner is injectable
    /// ([`git_for_host`]) so no unit test needs the real git.
    git_cli: Arc<GitCli>,
    /// What has already been handed to the terminal's scrollback
    /// ([`crate::insert_history`]) — the watermark that keeps a row from being
    /// inserted twice.
    history: HistoryWriter,
    /// Backward paging cursor for the transcript on screen: which session it
    /// describes, whether older rows exist above it, and where to resume.
    history_paging: HistoryPaging,
    /// A run finished, so the transcript is final: insert its new tail into the
    /// scrollback at the next `do_render` (which owns the screen repaint the
    /// insert would otherwise have to trigger itself).
    scrollback_pending: bool,
    /// The alternate screen has been entered at least once. The exit path only
    /// drives the scrollback when this is set: a startup failure never reached
    /// the alternate screen, and dumping the transcript there would put chat
    /// rows on stdout in front of a CLI error message.
    screen_entered: bool,
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
        // Derived before the path is moved into the struct below.
        let skill_reco_path = crate::skill_reco::path_for(&tui_settings_path);
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
        let tx = op_tx.clone();
        input.onNotice = Some(Box::new(move |message: &str| {
            let _ = tx.send(UiCmd::InputNotice(message.to_string()));
        }));
        let _ = &mut chat;
        let _ = &mut footer;
        let _ = cli_options;
        let _ = &mut input;

        let keybindings_path = keybindings_path_for(&tui_settings_path);
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
            keybindings_path,
            keybinding_problems: Vec::new(),
            keybinding_unknown: Vec::new(),
            enabled_model_ids: None,
            connection_lost: false,
            tui_settings: TuiSettings::default(),
            tui_settings_path,
            skill_reco_path,
            slash_commands: Vec::new(),
            session_input_cache: HashMap::new(),
            latest_session_switch: None,
            state: AppState::default(),
            running: false,
            cli_options: cli_options.clone(),
            cli_initial_prompt: cli_options.initial_prompt.clone(),
            pending_name_arg: None,
            pending_update_notice: None,
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
            cursor_recheck_at: Instant::now(),
            cursor_recheck_row: None,
            timers: Vec::new(),
            previous_width: 0,
            previous_height: 0,
            previous_kitty_image_ids: BTreeSet::new(),
            full_redraw_count: 0,
            pending_approval: None,
            cached_models: Vec::new(),
            model_image_support: HashMap::new(),
            cached_sessions: Vec::new(),
            session_labels: HashMap::new(),
            clipboard: crate::clipboard::Clipboard::new(),
            clipboard_capture: crate::paste::ClipboardCapture::new(),
            enabled_tools: None,
            pending_secret: None,
            provider_notice: None,
            input_tx: None,
            sandbox: None,
            skills_catalogue: None,
            skills_cli: skills_cli_for_host(),
            skills_catalogue_prefetch_started: false,
            skills_upgrade_armed: None,
            skills_op_running: false,
            skill_reco: SkillRecoState::Idle,
            skill_reco_send_through: false,
            git_cli: git_for_host(),
            history: HistoryWriter::new(),
            history_paging: HistoryPaging::default(),
            scrollback_pending: false,
            screen_entered: false,
            start_time: Instant::now(),
        };
        app.setup();
        app
    }

    fn setup(&mut self) {
        // Slash commands for autocomplete (with model/session arg flags).
        self.slash_commands = vec![
            SlashCommand {
                value: "/skill-recommend".into(),
                label: "/skill-recommend".into(),
                description: "offer a fitting skill before sending (on|off)".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
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
            SlashCommand {
                value: "/theme".into(),
                label: "/theme".into(),
                description: "pick a color theme".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/providers".into(),
                label: "/providers".into(),
                description: "configure providers and API keys".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/models".into(),
                label: "/models".into(),
                description: "configure the model scope".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/skills".into(),
                label: "/skills".into(),
                description: "browse, insert and install skills".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/tools".into(),
                label: "/tools".into(),
                description: "enable or disable tools".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/usage".into(),
                label: "/usage".into(),
                description: "token, cost and context usage".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/transcript".into(),
                label: "/transcript".into(),
                description: "full transcript with search".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/copy".into(),
                label: "/copy".into(),
                description: "copy the last assistant message".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/editor".into(),
                label: "/editor".into(),
                description: "edit the draft in $EDITOR".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/export".into(),
                label: "/export".into(),
                description: "export the session to HTML".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/agent".into(),
                label: "/agent".into(),
                description: "agent version and instance info".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/history".into(),
                label: "/history".into(),
                description: "search the session history".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/autocompact".into(),
                label: "/autocompact".into(),
                description: "toggle automatic compaction".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/autoretry".into(),
                label: "/autoretry".into(),
                description: "toggle automatic retry".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/tool-output".into(),
                label: "/tool-output".into(),
                description: "list tool calls or show one output".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/provider-key".into(),
                label: "/provider-key".into(),
                description: "set or clear a provider API key".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/sandbox".into(),
                label: "/sandbox".into(),
                description: "sandbox tier and tool permissions".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/permission".into(),
                label: "/permission".into(),
                description: "tool permission level".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/shell".into(),
                label: "/shell".into(),
                description: "run a shell command via the agent".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/title".into(),
                label: "/title".into(),
                description: "generate a session title".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/metrics".into(),
                label: "/metrics".into(),
                description: "runtime metrics of the session".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/snapshot".into(),
                label: "/snapshot".into(),
                description: "diagnostic snapshot of the current run".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/context".into(),
                label: "/context".into(),
                description: "list or toggle context files".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/keymap".into(),
                label: "/keymap".into(),
                description: "view and rebind keyboard shortcuts".into(),
                takes_model_arg: false,
                takes_session_arg: false,
            },
            SlashCommand {
                value: "/delete".into(),
                label: "/delete".into(),
                description: "delete this session (needs --yes)".into(),
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
        self.ac_manager
            .register(Box::new(AttachmentProvider::default()));

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
            Key::CTRL_G,
            Box::new(move || {
                let _ = tx.send(UiCmd::KeyAction(KeyAction::ToggleToolOutput));
                true
            }),
            "Expand/collapse tool output",
            None,
        );
        let tx = self.op_tx.clone();
        self.keybindings.add(
            Key::CTRL_D,
            Box::new(move || {
                let _ = tx.send(UiCmd::KeyAction(KeyAction::ToggleCompactActivity));
                true
            }),
            "Compact view (fold tool runs and thinking)",
            None,
        );
        let tx = self.op_tx.clone();
        self.keybindings.add(
            Key::CTRL_X,
            Box::new(move || {
                let _ = tx.send(UiCmd::KeyAction(KeyAction::CopyLastMessage));
                true
            }),
            "Copy the last assistant message",
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

        // The user's own bindings land last, on top of the registrations above
        // (`/keymap` writes the same file). Reading it here — after every
        // `add` — is what makes the overrides able to move a binding that only
        // exists once the app is built.
        self.load_keybindings();
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
            UiCmd::UpdateAvailable(notice) => {
                if self.state.streaming {
                    // A system message between a tool result and the next text
                    // chunk would change the chat's assistant-bubble routing.
                    self.pending_update_notice = Some(notice);
                } else {
                    self.add_system_message(notice);
                }
            }
            UiCmd::RefreshCompleted {
                mut result,
                session_id,
                compaction_revision,
            } => {
                if session_id != self.state.session_id {
                    return;
                }
                if compaction_revision != self.state.compaction_revision {
                    if let Ok(state) = &mut result {
                        state.is_compacting = self.state.compacting;
                    }
                }
                self.handle_cmd(UiCmd::Refreshed(result));
            }
            UiCmd::Refreshed(result) => match result {
                Ok(state) => self.apply_refresh_state(state),
                Err(_) => self.apply_refresh_error(),
            },
            UiCmd::ModelsLoaded { result, purpose } => match result {
                Ok(models) => {
                    self.remember_model_image_support(&models);
                    self.cached_models = models.iter().map(|m| m.full_id()).collect();
                    match purpose {
                        ModelsPurpose::Selector => {
                            // The `/model` menu replaced the old SelectList
                            // overlay: search, footer hints, scroll window.
                            let menu = Self::build_model_menu(models, &self.state.model);
                            let tx = self.op_tx.clone();
                            self.show_menu_overlay(
                                menu,
                                80,
                                Box::new(move |action: MenuAction| match action {
                                    MenuAction::Confirmed(mut values) => {
                                        if let Some(value) = values.pop() {
                                            let _ = tx.send(UiCmd::ModelSelected(SelectItem {
                                                value: value.clone(),
                                                label: value,
                                                description: None,
                                            }));
                                        }
                                    }
                                    MenuAction::Cancelled => {
                                        let _ = tx.send(UiCmd::OverlayCancel);
                                    }
                                    _ => {}
                                }),
                            );
                        }
                        ModelsPurpose::Scoped => {
                            let menu = Self::build_scope_menu(
                                models,
                                self.enabled_model_ids.as_deref(),
                                &self.state.model,
                            );
                            let tx = self.op_tx.clone();
                            self.show_menu_overlay(
                                menu,
                                80,
                                Box::new(move |action: MenuAction| match action {
                                    MenuAction::Confirmed(values) => {
                                        let _ = tx.send(UiCmd::ScopedModelsSaved(values));
                                    }
                                    MenuAction::Cancelled => {
                                        let _ = tx.send(UiCmd::OverlayCancel);
                                    }
                                    _ => {}
                                }),
                            );
                        }
                        ModelsPurpose::Default => {
                            let menu = Self::build_default_model_menu(models);
                            let tx = self.op_tx.clone();
                            self.show_menu_overlay(
                                menu,
                                80,
                                Box::new(move |action: MenuAction| match action {
                                    MenuAction::Confirmed(values) => {
                                        let _ = tx.send(UiCmd::SetDefaultModel(values));
                                    }
                                    MenuAction::Cancelled => {
                                        let _ = tx.send(UiCmd::OverlayCancel);
                                    }
                                    _ => {}
                                }),
                            );
                        }
                        ModelsPurpose::Autocomplete => self.query_autocomplete_cached(),
                    }
                }
                Err(err) => self.add_system_message(format!("Failed to load models: {err}")),
            },
            UiCmd::SessionsLoaded { result, purpose } => match result {
                Ok(sessions) => {
                    self.cached_sessions = sessions.iter().map(|s| s.id.clone()).collect();
                    match purpose {
                        SessionsPurpose::Browse => self.show_sessions_overlay(sessions),
                        SessionsPurpose::Tree => self.show_tree_overlay(sessions),
                        SessionsPurpose::Autocomplete => self.query_autocomplete_cached(),
                    }
                }
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
            UiCmd::CompactDone { session_id, result } => {
                if session_id != self.state.session_id {
                    return;
                }
                match result {
                    // Admission is not completion. A terminal event may already
                    // have cleared the pending request; do not re-lock it then.
                    Ok(_) => {
                        if self.state.compaction_requested {
                            self.state.compaction_revision += 1;
                            self.state.compaction_requested = false;
                            self.state.compacting = true;
                            self.add_system_message("Context compaction request accepted".into());
                        }
                    }
                    Err(err) => {
                        self.state.compaction_requested = false;
                        self.add_system_message(format!("Compact failed: {err}"));
                        self.spawn_refresh();
                    }
                }
            }
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
            UiCmd::InputNotice(message) => self.add_system_message(message),
            UiCmd::StatusLoaded {
                state,
                models,
                stats,
            } => match (state, models) {
                (Ok(s), Ok(models)) => self.apply_status(&s, &models, stats.as_ref().ok()),
                (Ok(s), Err(_)) => self.apply_status(&s, &[], stats.as_ref().ok()),
                (Err(err), _) => self.add_system_message(format!("Failed to get status: {err}")),
            },
            UiCmd::SessionSwitched {
                target,
                switched,
                result,
                state,
                history,
                label,
            } => {
                match &self.latest_session_switch {
                    Some(latest) if latest.target != target => {
                        // Superseded by a newer pick: this transcript belongs
                        // to a session the app is not showing any more. Its
                        // RPC may still have re-pointed the client here (the
                        // last switch to land wins), so put the client back on
                        // the target that won — but leave the menu alone: the
                        // user is still picking in it.
                        self.realign_client_to_latest_switch();
                        return;
                    }
                    Some(latest)
                        if self.state.session_id != latest.from
                            && self.state.session_id != target =>
                    {
                        // Another session lifecycle flow (`/new`, a fork) took
                        // over while this one was in flight. The user is
                        // somewhere else now, and applying this transcript
                        // would drag the client back to a session they left.
                        return;
                    }
                    _ => {}
                }
                if let Err(err) = result {
                    self.add_system_message(format!("Failed to switch session: {err}"));
                } else if !switched {
                    // The agent declined: the client still addresses the old
                    // session, so what is on screen is still the truth.
                    self.add_system_message(format!("Session switch declined: {label}"));
                } else {
                    if let Some(s) = state {
                        self.apply_refresh_state(s);
                    }
                    // `get_state` is a separate call and can fail on its own;
                    // the switch itself already happened, so the app identity
                    // has to follow the target either way — otherwise the
                    // drafts, refreshes and event filtering below stay keyed
                    // to the session we just left.
                    self.adopt_session_identity(&target);
                    self.restore_session_input();
                    match history {
                        Ok(page) => {
                            self.apply_history_page(&target, Ok(page));
                            self.add_system_message(format!("Switched to session: {label}"));
                        }
                        Err(err) => {
                            // The switch did happen, so the previous session's
                            // conversation must come off the screen even
                            // though the new one could not be loaded — it
                            // would otherwise read as this session's history
                            // while a prompt typed here goes to `target`.
                            self.clear_transcript(&target);
                            self.add_system_message(format!(
                                "Switched to session: {label} — its transcript could not be \
                                 loaded ({err})."
                            ));
                        }
                    }
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
                    let mut history = Ok(Value::Null);
                    if let Ok(ref v) = fork_result {
                        let cancelled =
                            v.get("cancelled").and_then(Value::as_bool).unwrap_or(false);
                        if !cancelled {
                            state = client.get_state().await.ok();
                            history =
                                load_history_tail(&client, &client.get_current_session_id()).await;
                        }
                    }
                    let _ = tx.send(UiCmd::ForkDone {
                        fork_result,
                        state,
                        history,
                        label,
                    });
                });
            }
            UiCmd::ForkDone {
                fork_result,
                state,
                history,
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
                            let session_id = self.state.session_id.clone();
                            self.apply_history_page(&session_id, history);
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
                        // The new session has no history — drop the previous
                        // transcript, or the old conversation stays on screen
                        // and /new looks like it did nothing.
                        let session_id = self.state.session_id.clone();
                        self.clear_transcript(&session_id);
                        self.restore_session_input();
                        self.add_system_message("New session started.".into());
                    }
                }
                Err(_) => self.add_system_message("Not connected to agent.".into()),
            },
            UiCmd::CloneDone {
                result,
                state,
                history,
            } => match result {
                Ok(v) => {
                    let cancelled = v.get("cancelled").and_then(Value::as_bool).unwrap_or(false);
                    if !cancelled {
                        if let Some(s) = state {
                            self.apply_refresh_state(s);
                        }
                        let session_id = self.state.session_id.clone();
                        self.apply_history_page(&session_id, history);
                        self.add_system_message("Session cloned — continue in new branch.".into());
                    }
                }
                Err(err) => self.add_system_message(format!("Failed to clone session: {err}")),
            },
            UiCmd::HistoryPageLoaded {
                session_id,
                before,
                result,
            } => self.prepend_history_page(&session_id, before, result),
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
            // ── popup menus ─────────────────────────────────────────────────
            UiCmd::MenuSelected { purpose, values } => match purpose {
                MenuPurpose::Sessions => {
                    self.hide_overlay();
                    self.switch_to_session(values.first());
                }
                MenuPurpose::SessionTree => {
                    self.hide_overlay();
                    self.switch_to_session(values.first());
                }
                MenuPurpose::Theme => {
                    self.hide_overlay();
                    if let Some(id) = values.first() {
                        self.select_theme(id);
                    } else {
                        self.request_render(false);
                    }
                }
                MenuPurpose::Tools => {
                    self.hide_overlay();
                    self.apply_tools(values);
                }
            },
            UiCmd::MenuCancelled(_purpose) => self.hide_overlay(),
            UiCmd::CopyRequest(text) => {
                self.hide_overlay();
                self.copy_to_clipboard(&text);
            }
            UiCmd::SetDefaultModel(values) => {
                self.hide_overlay();
                let Some(model) = values.first().cloned() else {
                    self.request_render(false);
                    return;
                };
                let client = self.client.clone();
                let tx = self.op_tx.clone();
                tokio::spawn(async move {
                    let result = client.set_default_model(&model).await;
                    let _ = tx.send(UiCmd::SetModelDone {
                        set_result: result,
                        state: None,
                    });
                });
            }
            UiCmd::UsageLoaded(result) => match result {
                Ok(state) => {
                    let value = serde_json::to_value(&*state).unwrap_or(Value::Null);
                    let view = usage_from_state(&value);
                    let width = (self.terminal.columns() as usize).saturating_sub(4).min(80);
                    self.show_overlay(
                        Box::new(crate::components::usage_view::UsageOverlay::new(view)),
                        OverlayOptions {
                            width: Some(SizeValue::Fixed(width)),
                            ..Default::default()
                        },
                    );
                }
                Err(err) => self.add_system_message(format!("Failed to load usage: {err}")),
            },
            UiCmd::AgentInfoLoaded(result) => match result {
                Ok(info) => {
                    let mut lines = agent_info_lines(&info);
                    lines.push(format!("  session: {}", self.state.session_id));
                    lines.push(format!("  model: {}", self.state.model));
                    lines.push(format!("  cwd: {}", self.state.cwd));
                    lines.push(format!("  skills loaded: {}", self.state.skills.len()));
                    self.show_pager_text(lines, "No agent info available.");
                }
                Err(err) => self.add_system_message(format!("Failed to load agent info: {err}")),
            },
            UiCmd::HistorySearched { query, result } => match result {
                Ok(payload) => {
                    let empty = format!("No history matches for '{query}'.");
                    // The match highlight reuses the pager's own search-hit
                    // colour, so `/history` and `/`-in-the-pager agree.
                    let highlight_bg = Chrome::from_theme(&self.theme).highlight_bg;
                    let lines = history_match_lines(&query, &payload, highlight_bg);
                    self.show_pager_text(lines, &empty);
                }
                Err(err) => self.add_system_message(format!("History search failed: {err}")),
            },
            UiCmd::ToolCallsLoaded(result) => match result {
                Ok(page) => {
                    self.show_pager_text(tool_call_lines(&page), "No tool calls recorded.");
                }
                Err(err) => self.add_system_message(format!("Failed to list tool calls: {err}")),
            },
            UiCmd::ToolOutputLoaded {
                tool_call_id,
                result,
            } => match result {
                Ok(payload) => {
                    let empty = format!("No stored output for tool call {tool_call_id}.");
                    self.show_pager_text(tool_output_lines(&tool_call_id, &payload), &empty);
                }
                Err(err) => self.add_system_message(format!("Failed to read tool output: {err}")),
            },
            UiCmd::AutoCompactionSet { enabled, result } => match result {
                Ok(()) => {
                    self.state.auto_compaction_enabled = enabled;
                    self.add_system_message(format!("Auto compaction {}.", enabled_label(enabled)));
                    self.request_render(false);
                }
                Err(err) => {
                    self.add_system_message(format!("Failed to set auto compaction: {err}"))
                }
            },
            UiCmd::AutoRetrySet { enabled, result } => match result {
                Ok(()) => {
                    self.state.auto_retry_enabled = enabled;
                    self.add_system_message(format!("Auto retry {}.", enabled_label(enabled)));
                    self.request_render(false);
                }
                Err(err) => self.add_system_message(format!("Failed to set auto retry: {err}")),
            },
            UiCmd::SessionSettingWritten {
                success_message,
                error_prefix,
                result,
            } => match result {
                Ok(()) => self.add_system_message(success_message),
                Err(err) => self.add_system_message(format!("{error_prefix}: {err}")),
            },
            UiCmd::SessionExported(result) => match result {
                Ok(value) => {
                    let message = export_result_message(&value);
                    self.add_system_message(message);
                }
                Err(err) => self.add_system_message(format!("Failed to export session: {err}")),
            },
            // ── sandbox / permissions ───────────────────────────────────────
            UiCmd::SandboxProbeRequested => {
                self.request_sandbox_probe(Self::host_sandbox_platform())
            }
            UiCmd::SandboxProbeLoaded { result } => match result {
                Ok(payload) => {
                    let probe = SandboxProbe::from_probe_response(&payload);
                    self.update_sandbox_status(|status| status.probe = probe);
                }
                Err(err) => {
                    // A transport failure is not "unavailable": keep the probe
                    // unresolved so the panel keeps saying "checking" instead of
                    // claiming the sandbox is missing.
                    self.add_system_message(format!("Sandbox probe failed: {err}"));
                }
            },
            UiCmd::SandboxTierRequested(tier) => {
                let client = self.client.clone();
                let tx = self.op_tx.clone();
                tokio::spawn(async move {
                    let result = client.set_sandbox_policy(tier.to_wire()).await;
                    let _ = tx.send(UiCmd::SandboxPolicySet { result });
                });
            }
            UiCmd::SandboxPolicySet { result } => match result {
                Ok(payload) => {
                    let status = SandboxStatus::from_policy_response(
                        Self::host_sandbox_platform(),
                        &payload,
                    );
                    self.update_sandbox_status(|cached| *cached = status);
                }
                Err(err) => {
                    self.add_system_message(format!("Failed to set the sandbox tier: {err}"))
                }
            },
            UiCmd::PermissionLevelRequested(level) => self.set_permission_level(level, false),
            UiCmd::PermissionLevelSet { level, result } => match result {
                Ok(()) => {
                    self.state.permission_level = level;
                    self.sync_sandbox_panel();
                    self.add_system_message(format!(
                        "Tool permission level: {}. {}",
                        level.label(),
                        level.description()
                    ));
                }
                Err(err) => {
                    self.add_system_message(format!("Failed to set the permission level: {err}"))
                }
            },
            // ── /skills ─────────────────────────────────────────────────────
            UiCmd::SkillsLoaded(result) => match result {
                Ok(payload) => {
                    let rows = parse_skills(&payload);
                    if let Some(view) = self.top_skills_view() {
                        view.set_skills(rows);
                    }
                    // Same reason as the catalogue arm below: the panel's rows
                    // changed and nothing wrote to the transcript.
                    self.request_render(false);
                }
                Err(err) => {
                    // The panel keeps the names `get_state` gave it and the
                    // catalogue the other fetch brought; only this source is
                    // missing. It is told anyway — a browser that silently
                    // shows half the picture is worse than one that names the
                    // half it could not reach.
                    let text = format!("Failed to load the skill catalogue: {err}");
                    if let Some(view) = self.top_skills_view() {
                        view.set_error(Some(text.clone()));
                    }
                    self.add_system_message(text);
                }
            },
            UiCmd::SkillsCatalogueLoaded(result) => match result {
                Ok(catalogue) => {
                    // The panel owns the merged display list; the cache is what
                    // `i` resolves the version to pin from later on.
                    let entries = catalogue.entries.clone();
                    self.skills_catalogue = Some(catalogue);
                    self.set_skills_loading(false);
                    if let Some(view) = self.top_skills_view() {
                        view.set_catalogue(entries);
                    }
                    // Nothing here writes to the transcript, so nothing else
                    // would repaint: without this the panel keeps the frame it
                    // had while the child was running — the loading row stays
                    // on screen until the next key press.
                    self.request_render(false);
                }
                Err(err) => {
                    self.set_skills_loading(false);
                    let text = skills_catalogue_error(&err);
                    // The panel keeps the names `get_state` gave it and the
                    // catalogue the other fetch brought; only this source is
                    // missing. It is told anyway — a browser that silently
                    // shows half the picture is worse than one that names the
                    // half it could not reach.
                    //
                    // The failure is reported in exactly ONE place. Writing the
                    // transcript as well made the same problem appear twice —
                    // red in the panel and again as a system message — which
                    // reads as "the skills are broken" rather than "one of the
                    // two sources is unreachable" (round-3 acceptance saw
                    // exactly that). The panel is preferred because it carries
                    // the retry hint next to the list it is about, but it is
                    // only available while it is on screen: the load outlives
                    // the panel if the user closes it or moves on after `r`, and
                    // then the transcript is the only place the failure can
                    // surface. Silence is not an option there, so the fallback
                    // keeps it.
                    match self.top_skills_view() {
                        Some(view) => view.set_error(Some(text)),
                        None => self.add_system_message(text),
                    }
                    self.request_render(false);
                }
            },
            UiCmd::SkillsCataloguePrefetched(result) => {
                // The recommender's own quiet load (see
                // [`App::prefetch_skill_catalogue`]). Success fills the same
                // cache the panel fills. A failure is dropped in silence — the
                // user never asked for this call, and the panel is where a
                // catalogue problem is reported (opening it, or `r`, retries):
                // `skills_catalogue_prefetch_started` keeps it from being
                // retried behind their back.
                if let Ok(catalogue) = result {
                    self.skills_catalogue = Some(catalogue);
                }
            }
            UiCmd::SkillsRefreshRequested => self.request_skills_rescan(),
            UiCmd::SkillsRefreshed(result) => match result {
                Ok(_) => {
                    // The answer carries no rows: a re-scan only becomes visible
                    // by re-reading both sources — and only *after* it landed,
                    // which is why this is not done next to the install.
                    self.load_skill_catalogue();
                    self.load_cli_catalogue();
                }
                Err(err) => self.add_system_message(format!("Failed to refresh skills: {err}")),
            },
            UiCmd::SkillsMutationRequested(mutation) => self.handle_skills_mutation(mutation),
            UiCmd::SkillOpDone {
                outcome,
                catalogue_dirty,
            } => {
                self.skills_op_running = false;
                if let Some(view) = self.top_skills_view() {
                    view.clear_pending();
                    view.set_error(skill_op_error(&outcome));
                }
                self.add_system_message(describe_outcome(&outcome));
                if catalogue_dirty {
                    self.request_skills_rescan();
                }
                self.request_render(false);
            }
            UiCmd::SkillRecoSuggested { draft, suggestion } => {
                self.apply_skill_reco_suggestion(draft, suggestion);
            }
            UiCmd::SkillRecoInstalled {
                draft,
                skill,
                installed,
            } => {
                if !installed {
                    // The card stays up so the user can retry or send without
                    // it; the draft is untouched (PRD v1.6 §6.2).
                    self.add_system_message(SKILLS_RECO_INSTALL_FAILED.to_string());
                    self.request_render(false);
                } else {
                    self.use_recommended_skill(&draft, &skill);
                }
            }
            UiCmd::SkillsDetailRequested => {
                let width = (self.terminal.columns() as usize).max(1);
                let lines = self
                    .top_skills_view()
                    .map(|view| view.detail_lines(width.saturating_sub(4)))
                    .unwrap_or_default();
                self.show_pager_text(lines, "No skill is highlighted.");
            }
            UiCmd::SkillChosen(name) => {
                self.hide_overlay();
                self.input.insert_text(&name);
                self.add_system_message(format!("Inserted skill: {name}"));
                self.request_render(false);
            }
            // ── worktree ────────────────────────────────────────────────────
            UiCmd::WorktreesLoaded { result } => match result {
                Ok(list) => {
                    // The listing needs no transcript line — it is a read. A
                    // panel that was closed while git ran keeps it that way
                    // rather than repainting under whatever the user opened
                    // instead.
                    if let Some(overlay) = self.top_worktree_overlay() {
                        overlay.view_mut().set_notice(None);
                        overlay.view_mut().set_list(list);
                    }
                }
                Err(err) => {
                    self.set_worktree_notice(&format!("Failed to read git worktrees: {err}"))
                }
            },
            UiCmd::WorktreeAdded { result } => match result {
                Ok(created) => {
                    self.add_system_message(created.summary());
                    self.hide_overlay();
                    self.switch_cwd(created.plan.path.display().to_string());
                }
                Err(err) => {
                    self.set_worktree_notice(&format!("Failed to create the worktree: {err}"))
                }
            },
            UiCmd::WorktreeSwitchRequested(path) => {
                self.hide_overlay();
                self.switch_cwd(path);
            }
            UiCmd::WorktreeNewRequested => {
                self.hide_overlay();
                self.input.insert_text(NEW_WORKTREE_COMMAND);
                self.add_system_message(
                    "Type the branch to create, e.g. /worktree new feat/tui-worktree.".into(),
                );
                self.request_render(false);
            }
            UiCmd::WorktreeRefreshRequested => self.load_worktrees(),
            UiCmd::KeymapBind { description, key } => self.apply_keymap_binding(&description, &key),
            UiCmd::KeymapReset { description } => self.reset_keymap_binding(description.as_deref()),
            // ── session lifecycle / diagnostics ─────────────────────────────
            UiCmd::SessionTitleGenerated(result) => match result {
                Ok(payload) => match payload.get("title").and_then(Value::as_str) {
                    Some(title) => self.apply_session_title(title),
                    None => self.add_system_message("The agent returned no title.".to_string()),
                },
                Err(err) => self.add_system_message(format!("Failed to generate a title: {err}")),
            },
            UiCmd::SessionTitleApplied { title, result } => match result {
                Ok(()) => {
                    self.state.session_name = Some(title.clone());
                    self.add_system_message(format!("Session renamed to {title}."));
                }
                Err(err) => self.add_system_message(format!("Failed to apply the title: {err}")),
            },
            UiCmd::SessionDeleted { session_id, result } => match result {
                Ok(_) => {
                    self.add_system_message(format!(
                        "Deleted session {session_id}. Starting a new one."
                    ));
                    self.start_new_session();
                }
                Err(err) => self.add_system_message(format!("Failed to delete the session: {err}")),
            },
            UiCmd::DiagnosticsLoaded { kind, result } => match result {
                Ok(payload) => {
                    let lines = Self::diagnostics_lines(kind, &payload);
                    self.show_pager_text(lines, "The agent reported nothing for this command.");
                }
                Err(err) => self.add_system_message(format!("{}: {err}", kind.error_prefix())),
            },
            UiCmd::ContextFilesSet { enabled, result } => match result {
                Ok(()) => {
                    self.add_system_message(format!("Context files {}.", enabled_label(enabled)))
                }
                Err(err) => self.add_system_message(format!("Failed to set context files: {err}")),
            },
            UiCmd::ToolsChanged(result) => match result {
                Ok(tools) => {
                    self.enabled_tools = Some(tools.clone());
                    let text = if tools.is_empty() {
                        "no tools".to_string()
                    } else {
                        tools.join(", ")
                    };
                    self.add_system_message(format!("Tools enabled: {text}"));
                }
                Err(err) => self.add_system_message(format!("Failed to set tools: {err}")),
            },
            // ── /providers ──────────────────────────────────────────────────
            UiCmd::ProvidersLoaded(result) => match result {
                Ok(providers) => {
                    // Replace the list in place when it is already open (a
                    // mutation's follow-up refresh); otherwise open it.
                    if self.top_provider_list().is_some() {
                        self.refresh_provider_list(providers);
                    } else {
                        self.show_providers_overlay(providers);
                    }
                }
                Err(err) => self.add_system_message(format!("Failed to load providers: {err}")),
            },
            UiCmd::ProviderActionDone { action, result } => match result {
                Ok(()) => {
                    self.provider_notice = None;
                    self.close_provider_form();
                    self.add_system_message(format!("{action}."));
                    self.spawn_refresh();
                    let client = self.client.clone();
                    let tx = self.op_tx.clone();
                    tokio::spawn(async move {
                        let _ = tx.send(UiCmd::ProvidersLoaded(client.list_providers().await));
                    });
                }
                Err(err) => {
                    self.provider_notice = Some(err.clone());
                    self.add_system_message(format!("Failed: {err}"));
                    self.set_provider_form_error(&err);
                    let notice = err.clone();
                    if let Some(list) = self.top_provider_list() {
                        list.set_notice(Some(&notice));
                    }
                    self.request_render(false);
                }
            },
            UiCmd::ProviderAdd => {
                self.provider_notice = None;
                self.show_provider_form(ProviderForm::add());
            }
            UiCmd::ProviderEdit(info) => {
                self.provider_notice = None;
                self.show_provider_form(ProviderForm::edit(&info));
            }
            UiCmd::ProviderFormCancelled => {
                // Return to the provider list instead of dropping every
                // overlay: `/providers` owns the stack below the form.
                self.hide_overlay();
                self.show_providers();
            }
            UiCmd::ProviderDelete(id) => self.delete_provider(&id),
            UiCmd::ProviderSetKey(id) => self.prompt_provider_key(&id),
            UiCmd::ProviderSync(action) => self.provider_sync(action),
            UiCmd::ProviderSubmit(input) => match validate_provider_input(&input) {
                Ok(()) => self.submit_provider(*input),
                Err(err) => {
                    self.provider_notice = Some(err.clone());
                    self.set_provider_form_error(&err);
                    self.add_system_message(format!("Invalid provider: {err}"));
                    self.request_render(false);
                }
            },
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
                // Never open the popup while the user is browsing history:
                // a recalled `/…` entry fires InputChanged like a typed one,
                // and the popup would then swallow further up/down/enter
                // presses meant for history navigation.
                if items.is_empty() || self.input.is_browsing_history() {
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
        if !self.state.streaming {
            if let Some(notice) = self.pending_update_notice.take() {
                self.add_system_message(notice);
            }
        }

        if let Some(d) = self.resize_deadline {
            if now >= d {
                self.resize_deadline = None;
                // A terminal resize is our only reliable in-band signal that
                // the terminal was externally reset (e.g. a tmux client
                // attach that changed the pane size). Force a full redraw:
                // the differential renderer moves the cursor relative to the
                // last-tracked row, so after an external reset it would
                // otherwise keep writing to the wrong rows ("A / AB / ABC"
                // scrolling). A full redraw re-anchors the cursor.
                self.request_render(true);
            }
        }

        if let Some(d) = self.ac_query_deadline {
            if now >= d {
                self.ac_query_deadline = None;
                if self.pending_ac_query.take().is_some() {
                    self.trigger_autocomplete();
                }
            }
        }

        // While streaming, periodically re-read the terminal's real cursor
        // position (DSR \x1b[6n). If it diverged from our tracked row — a
        // tmux client attach that reset the cursor with no SIGWINCH and no
        // focus event — the response forces a full redraw to re-anchor.
        if self.state.streaming && now >= self.cursor_recheck_at {
            self.cursor_recheck_at = now + CURSOR_RECHECK_INTERVAL;
            self.query_cursor_position();
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
            if self.state.streaming
                || self.state.compacting
                || self.state.compaction_requested
                // Keep repainting while a recommendation is pending: the prompt
                // line's spinner is what shows the box is busy, not stuck.
                || matches!(self.skill_reco, SkillRecoState::Pending { .. })
            {
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
                        // The `--prompt` given on the command line has no draft
                        // behind it, so there is nothing to attach.
                        let result = client.prompt(&message, "enqueue_if_busy", Vec::new()).await;
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
        self.input_tx = Some(input_tx.clone());
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
        // From here on the terminal owns an alternate screen, and the exit path
        // may append the transcript to the scrollback underneath it.
        self.screen_entered = true;

        self.wait_for_agent().await;
        // (No is-running check: nothing can flip `running` during startup —
        // input events are only consumed by the caller once start returns.)

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
                        sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at_ms));
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

    /// Awaited history load (startup path): `await this.loadSessionMessages()`.
    async fn load_messages_direct(&mut self) {
        let session_id = self.state.session_id.clone();
        let page = load_history_tail(&self.client, &session_id).await;
        self.apply_history_page(&session_id, page);
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

        // Last chance for the transcript: whatever the scrollback does not have
        // yet is appended now, while the terminal is already off the alternate
        // screen — so it survives the exit and can be scrolled back to and
        // selected with the terminal's own copy mode. A run that ended normally
        // inserted its own tail already; this covers a session that was resumed
        // (and thus never ran) or a run cut short by the exit itself, and it is
        // what makes the feature visible without prompting first.
        if self.screen_entered {
            self.flush_scrollback(false);
        }
        self.terminal.show_cursor();
    }

    // ─── Agent event handling ──────────────────────────────────────────

    pub fn handle_agent_event(&mut self, event: &AgentEvent) {
        // Every broadcast event is stamped with the session that produced it.
        // The stream resubscribes when the session changes, but a frame already
        // in flight from the session just left can still land here — a
        // `text_chunk` from it would append to the last assistant bubble of the
        // *new* transcript and an `agent_end` would overwrite it, which reads
        // as the previous conversation bleeding into this one. The client's
        // current session id is what the next prompt addresses, so it is the
        // reference (the app's own `state.session_id` can lag a switch by a
        // refresh). Unstamped events have no session to compare.
        if event
            .session_id
            .as_deref()
            .is_some_and(|id| id != self.client.get_current_session_id())
        {
            return;
        }
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
                // Terminal bell / OSC 9 desktop notification when one of OUR
                // runs finishes cleanly (or errors out) — the bytes go straight
                // to the terminal emulator, so they also reach a user who
                // unfocused the window. Foreign runs (another client on the same
                // session) never get a `bind_user_run`, so `has_run` gates them
                // out.
                let state = event.data.get("state").and_then(Value::as_str);
                let is_our_run = event
                    .run_id
                    .as_deref()
                    .map(|id| self.chat.has_run(id))
                    .unwrap_or(false);
                if is_our_run && matches!(state, Some("completed") | Some("error")) {
                    let kind = if state == Some("error") {
                        NotifyKind::Failed
                    } else {
                        NotifyKind::Completed
                    };
                    let body = if self.state.model.is_empty() {
                        kind.default_body().to_string()
                    } else {
                        format!("{} ({})", kind.default_body(), self.state.model)
                    };
                    self.notify_event(kind, &body);
                }
                // The session is single-active-run: `agent_end` means the one
                // live run just finished, so streaming is now false. Don't ask
                // `has_running_run()` — a stale `get_state` snapshot answered
                // in the agent's "finalizing" window can already have rebuilt
                // the client's run table and re-marked this run Running, and
                // there is no later refresh to correct it (the footer spinner
                // would stay up forever). The next run, if any, re-asserts
                // streaming via its own `agent_start`.
                self.state.streaming = false;
                self.state.active_tool_count = 0;
                self.state.tool_start_time = None;
                self.update_terminal_title();
                let text = event.text();
                if !text.is_empty() {
                    self.chat.update_last_message(text);
                }
                // Mark the assistant message as complete so pending→false and
                // the full markdown render replaces the streaming render.
                self.chat.mark_last_message_complete();
                // Nothing in the transcript can change again until the next
                // prompt: hand it to the scrollback at the next render.
                self.scrollback_pending = true;
                self.request_render(false);
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
                self.update_terminal_title();
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
                self.chat
                    .finish_tool(&tool_id, text, tool_end_failed(&event.data));
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
            "compaction_started" => {
                self.state.compaction_revision += 1;
                self.state.compacting = true;
                self.add_system_message("Context compaction started".into());
            }
            "compaction_committed" | "compaction_failed" | "compaction_unchanged" => {
                self.state.compaction_revision += 1;
                self.state.compacting = false;
                self.state.compaction_requested = false;
                let text = match event.r#type.as_str() {
                    "compaction_committed" => compaction_completed_text(
                        event.data.get("tokens_before"),
                        event.data.get("tokens_after"),
                    ),
                    "compaction_failed" => format!(
                        "Compact failed: {}",
                        event
                            .data
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown error")
                    ),
                    _ if event.data.get("already_compacted").and_then(Value::as_bool)
                        == Some(true)
                        || event.data.get("reused").and_then(Value::as_bool) == Some(true) =>
                    {
                        "Context already compacted; no new content to compact".into()
                    }
                    _ => "Context compaction not needed".into(),
                };
                self.add_system_message(text);
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
            "tools_changed" => {
                self.spawn_refresh();
            }
            "sandbox_policy_changed" => {
                // The event carries the tier the agent now runs under — the
                // only read-back of the policy that exists (`get_state` does
                // not report it). Recording it keeps an already-open panel
                // honest when another client changes the tier.
                if let Some(tier) = event
                    .data
                    .get("tier")
                    .and_then(Value::as_str)
                    .and_then(SandboxTier::from_wire)
                {
                    self.update_sandbox_status(|status| {
                        status.tier = tier;
                        status.requested_tier = Some(tier);
                    });
                }
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
        // Terminal focus events. A tmux client attach (or window focus
        // regain) can reset the terminal's cursor position out from under the
        // differential renderer — its relative cursor moves are keyed to the
        // last-tracked row, so the next frame would otherwise write each
        // growing stream line on a fresh row ("A / AB / ABC / ABCD"
        // scrolling). Focus-in is the standard signal to force a full redraw
        // and re-anchor the cursor.
        if data == "\x1b[I" {
            self.request_render(true);
            return;
        }
        if data == "\x1b[O" {
            return;
        }

        // Cell size response.
        if self.consume_cell_size_response(data) {
            self.request_render(false);
            return;
        }

        // DSR cursor-position response (polling net for cursor desync).
        if self.consume_cursor_position_response(data) {
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
            // If the focused overlay is now hidden, redirect focus. Overlay
            // ids are unique, so one find suffices (the TS double-lookup's
            // miss arm is unreachable).
            let focused_hidden = self
                .overlay_stack
                .iter()
                .find(|o| Some(o.id) == self.overlay_id_of_focus())
                .is_some_and(|e| e.hidden);
            if focused_hidden {
                if let Some(top) = self.get_top_overlay_index() {
                    self.set_focus(FocusTarget::Overlay(self.overlay_stack[top].id));
                } else {
                    self.set_focus(FocusTarget::Input);
                }
            }
            self.handle_key(&key_name);
            return;
        }

        // Fallback: printable character not covered by parseKey. parse_key
        // claims every single-byte char (control or printable), so only
        // multi-byte characters (all ≥ 0x80, i.e. printable) reach this.
        let mut chars = data.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
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

    /// Give the top overlay the first escape when it reports an active
    /// incremental search (`escape` clears the query, a second one closes the
    /// panel). Mirrors the `wants_key_release` gate in [`Self::handle_data`].
    fn top_overlay_wants_escape(&self) -> bool {
        match self.get_top_overlay_index() {
            Some(idx) => self.overlay_stack[idx].component.wants_escape(),
            None => false,
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
            // A shown recommendation owns the first escape: it means "send the
            // draft without the skill", not "clear the draft" (which would
            // discard the very thing the card is holding).
            if matches!(self.skill_reco, SkillRecoState::Suggested { .. }) {
                self.send_held_draft();
                return;
            }
            // While the agent is being asked, escape must not clear either: the
            // held draft is what the answer belongs to, and dropping it would
            // leave a card pointing at nothing.
            if self.editor_locked_by_reco() {
                return;
            }
            if self.autocomplete.is_visible() {
                self.autocomplete.hide();
                self.request_render(false);
            } else if !self.overlay_stack.is_empty() {
                // A component with a live incremental search owns the first
                // escape (it clears the query); the app-level close handles
                // every other case. The component decides via `wants_escape`
                // so this stays one rule for every panel.
                if self.top_overlay_wants_escape() {
                    if let Some(idx) = self.get_top_overlay_index() {
                        self.overlay_stack[idx].component.handle_input(Key::ESCAPE);
                    }
                    self.request_render(false);
                    return;
                }
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

        // A shown recommendation takes `a` (install and use). Checked before
        // the keybinding manager so it cannot be shadowed by a binding, and
        // after overlays/escape so a panel in front still wins.
        if matches!(&self.skill_reco, SkillRecoState::Suggested { .. })
            && key.eq_ignore_ascii_case("a")
            && !self.autocomplete.is_visible()
            && self.overlay_stack.is_empty()
        {
            self.accept_skill_recommendation();
            return;
        }

        // Dispatch through keybinding manager (ctrl shortcuts, shift+tab, ...).
        if self.keybindings.dispatch(key, None) {
            self.request_render(false);
            return;
        }

        // ctrl+v / cmd+v — paste from the system clipboard: the image it holds
        // when it holds one (a screenshot), otherwise its text. After the
        // keybinding dispatch so a user binding still wins, before the generic
        // ctrl+ routing because no widget claims this key.
        //
        // Both spellings, because which one arrives depends on the terminal:
        // upstream of Kitty-protocol terminals (`\x1b[>7u`, which this TUI asks
        // for) a Command press is reported as `super+v`; elsewhere the terminal
        // keeps Command for its own Paste menu and only Ctrl reaches us. A
        // binding the terminal swallows would be a promise we cannot keep.
        if key == Key::CTRL_V || key == "super+v" {
            self.paste_clipboard();
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

        // Editor handles the rest — except while the recommendation flow owns
        // the box (see `editor_locked_by_reco`): typing into a draft that is
        // already being evaluated would make the sent text differ from the text
        // the answer was computed for.
        if self.editor_locked_by_reco() {
            return;
        }
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
            KeyAction::ToggleToolOutput => {
                // Silent on purpose: ctrl+g is a view toggle, and a system
                // message per press would push the transcript around.
                self.chat.toggle_tool_output_expanded();
                self.request_render(false);
            }
            KeyAction::ToggleCompactActivity => {
                // Silent for the same reason as ctrl+g: it is a view toggle,
                // and the folded rows themselves are the feedback.
                self.chat.toggle_compact_activity();
                self.request_render(false);
            }
            KeyAction::CopyLastMessage => self.copy_last_assistant_message(),
            KeyAction::ScrollChatUpPage => {
                self.chat.scroll_up(self.terminal.rows() as usize);
                self.maybe_load_older_history();
                self.request_render(false);
            }
            KeyAction::ScrollChatDownPage => {
                self.chat.scroll_down(self.terminal.rows() as usize);
                self.request_render(false);
            }
            KeyAction::ScrollChatUpLine => {
                self.chat.scroll_up(3);
                self.maybe_load_older_history();
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
            self.update_terminal_title();
            // Mark the in-progress assistant message as stopped so the partial
            // content (thinking, text, tool calls) is preserved and visible.
            self.chat.mark_last_assistant_stopped();
            // The partial reply is final now (only the "interrupted" marker can
            // still appear, which is part of that same row) — same hand-off as
            // `agent_end`.
            self.scrollback_pending = true;
            self.request_render(false);
            return;
        }
        // Not streaming: exit the app.
        self.running = false;
    }

    // ─── Autocomplete ─────────────────────────────────────────────────

    fn handle_input_changed(&mut self, value: &str) {
        // The recommender's catalogue is the one thing it cannot fetch when the
        // message is submitted (it picks from the platform catalogue, which the
        // panel alone used to load) — so it is fetched while the draft is still
        // being typed. Cheap and idempotent: it returns immediately once the
        // cache is filled, and at most once per session otherwise.
        self.prefetch_skill_catalogue(value);
        // TS: the AutocompleteManager debounces 20 ms internally; the sync
        // port defers the debounce to the app loop.
        // History browsing skips autocomplete entirely: recalling a `/…`
        // command via up-arrow must not pop the completion menu (its
        // up/down/enter handling would hijack further history navigation).
        if self.input.is_browsing_history() {
            self.pending_ac_query = None;
            self.ac_query_deadline = None;
            self.autocomplete.hide();
            return;
        }
        self.pending_ac_query = Some((value.to_string(), self.input.cursor_byte()));
        self.ac_query_deadline = Some(Instant::now() + Duration::from_millis(20));
    }

    fn trigger_autocomplete(&mut self) {
        let text = self.input.get_value();
        if text.starts_with("/model ") && self.cached_models.is_empty() {
            let client = self.client.clone();
            let tx = self.op_tx.clone();
            tokio::spawn(async move {
                let _ = tx.send(UiCmd::ModelsLoaded {
                    result: client.list_models().await,
                    purpose: ModelsPurpose::Autocomplete,
                });
            });
        } else if ["/fork ", "/clone ", "/sessions "]
            .iter()
            .any(|p| text.starts_with(p))
            && self.cached_sessions.is_empty()
        {
            let client = self.client.clone();
            let tx = self.op_tx.clone();
            tokio::spawn(async move {
                let _ = tx.send(UiCmd::SessionsLoaded {
                    result: client.list_sessions().await,
                    purpose: SessionsPurpose::Autocomplete,
                });
            });
        }
        self.query_autocomplete_cached();
    }

    fn query_autocomplete_cached(&mut self) {
        self.ac_manager
            .update_state(&self.state.cwd, &self.cached_models, &self.cached_sessions);
        let text = self.input.get_value().to_string();
        let cursor = self.input.cursor_byte();
        self.ac_manager.query_immediate(&text, cursor);
    }

    fn apply_autocomplete_selection(&mut self) {
        let item = self.autocomplete.get_selected_item().cloned();
        let Some(item) = item else { return };
        let ctx = self.ac_manager.active_context().cloned();
        if let Some(ctx) = ctx {
            let token = &ctx.token;
            {
                // Replace only the token portion, preserving the prefix.
                let before = &ctx.text[..ctx.token_start];
                let after = &ctx.text[ctx.token_start + token.len()..];
                let mut value = item.value.clone();
                let max_overlap = before.len().min(value.len());
                for len in (1..=max_overlap).rev() {
                    if value.is_char_boundary(len) && before.ends_with(&value[..len]) {
                        value = value[len..].to_string();
                        break;
                    }
                }
                let combined = format!("{before}{value}{after}");
                let cursor = before.encode_utf16().count() + value.encode_utf16().count();
                self.input.set_value(&combined, Some(cursor));
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

        // Tell the user even if the window is not focused: an approval request
        // parks the whole run until they answer, which is exactly the case the
        // desktop notifications exist for.
        let body = format!(
            "{} — /approve {} or /reject {}",
            req.title, req.request_id, req.request_id
        );
        self.notify_event(NotifyKind::ApprovalNeeded, &body);

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

// ─── /sandbox and /skills overlay wrappers ──────────────────────────────────

/// Overlay wrapper for `SandboxView`.
///
/// The panel itself is I/O-free (it reports intent as [`SandboxAction`]); this
/// wrapper is the only place that turns those into `UiCmd`s, following the same
/// rule as the menu/provider overlays: the callback sends a command instead of
/// capturing the app, so no `&mut self` is held while the component handles a
/// key. It also owns the one row the panel cannot render itself — the
/// "the agent cannot tell us the current tier" caveat.
struct SandboxOverlay {
    view: SandboxView,
    /// Row budget handed to the panel (the terminal height minus chrome).
    max_rows: usize,
    /// Caveat row(s), wrapped and drawn under the panel while present.
    note: Option<String>,
    on_action: Box<dyn FnMut(SandboxAction)>,
}

impl SandboxOverlay {
    fn new(
        view: SandboxView,
        max_rows: usize,
        note: Option<String>,
        on_action: Box<dyn FnMut(SandboxAction)>,
    ) -> Self {
        Self {
            view,
            max_rows: max_rows.max(1),
            note,
            on_action,
        }
    }

    fn view_mut(&mut self) -> &mut SandboxView {
        &mut self.view
    }

    fn set_status(&mut self, status: SandboxStatus) {
        self.note = sandbox_tier_caveat(&status);
        self.view.set_status(status);
    }

    fn set_theme(&mut self, theme: Theme) {
        self.view.set_theme(&theme);
    }
}

impl Component for SandboxOverlay {
    fn render(&mut self, width: usize) -> Vec<String> {
        let Some(note) = self.note.clone() else {
            return self.view.render(width, self.max_rows);
        };
        let note: Vec<String> = wrap_text_with_ansi(&note, width.max(1))
            .into_iter()
            .map(|row| fit_overlay_row(&row, width))
            .collect();
        let budget = self.max_rows.saturating_sub(note.len()).max(1);
        let mut rows = self.view.render(width, budget);
        rows.extend(note);
        rows
    }

    fn handle_input(&mut self, data: &str) {
        let action = self.view.handle_key(data);
        if action != SandboxAction::None {
            (self.on_action)(action);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Overlay wrapper for the stateless help card.
///
/// [`crate::help_screen::render_help`] is a pure function of the *width*, so
/// the port dumped the card at its full 65 rows and let the overlay
/// compositor clip the tail: an 80×36 pane showed the first 36 rows — no
/// bottom border, no hint that anything was missing — and the 29 rows below
/// the fold, which carry every command this branch added, could not be reached
/// by any key. This wrapper gives the card a viewport: the frame stays pinned,
/// the body scrolls, and a hint row says how much body is left on either side.
///
/// Keys mirror [`crate::components::pager::Pager`]'s (`↑`/`↓`, the page keys,
/// `home`/`end`) so the two scrolling overlays in the TUI behave the same way;
/// `escape` is the app's — it closes any overlay before the component ever sees
/// the key. The page keys are matched under both spellings on purpose:
/// `keys::parse_key` names the sequences a terminal actually sends `pageUp` /
/// `pageDown`, while the pager documents (and accepts) `pageup`/`pagedown`.
/// Matching only the lowercase form is how a "working" overlay ignores every
/// real `PageDown` — the pager does exactly that (out of this task's scope).
struct HelpOverlay {
    /// Row budget: the terminal's height, which the window may not exceed.
    max_rows: usize,
    /// Body rows scrolled off the top, clamped against the last render's
    /// metrics (a resize between keystrokes can shrink the window).
    scroll: usize,
    /// Card metrics from the last render: `(max_scroll, page)`.
    metrics: (usize, usize),
}

impl HelpOverlay {
    fn new(max_rows: usize) -> Self {
        Self {
            max_rows: max_rows.max(1),
            scroll: 0,
            metrics: (0, 0),
        }
    }
}

impl Component for HelpOverlay {
    fn render(&mut self, width: usize) -> Vec<String> {
        let window = crate::help_screen::render_help_window(width, self.max_rows, self.scroll);
        self.scroll = window.scroll;
        self.metrics = (window.max_scroll, window.page);
        window.lines
    }

    fn handle_input(&mut self, data: &str) {
        let (max_scroll, page) = self.metrics;
        // A page step is at least one row: on a viewport too short for a body
        // row `page` is 0, and a zero step would be a dead key.
        let page = page.max(1) as i64;
        let next = match data {
            "up" | "k" => self.scroll.saturating_sub(1),
            "down" | "j" => self.scroll.saturating_add(1),
            "pageup" | "pageUp" | "page-up" | "b" => self.scroll.saturating_sub(page as usize),
            "pagedown" | "pageDown" | "page-down" | "space" => {
                self.scroll.saturating_add(page as usize)
            }
            "home" | "g" => 0,
            "end" | "G" => max_scroll,
            _ => return,
        };
        // Every arm lands here, so a stale `scroll` (terminal resized since the
        // last render) can never point past the card.
        self.scroll = next.min(max_scroll);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Overlay wrapper for `SkillsView` (same contract as [`SandboxOverlay`]).
struct SkillsOverlay {
    view: SkillsView,
    max_rows: usize,
    /// A `future skills list --json` call is in flight: the panel shows a
    /// loading row above itself, because the catalogue is the one source that
    /// takes long enough for the panel to look hung.
    loading: bool,
    theme: Theme,
    on_action: Box<dyn FnMut(SkillsAction)>,
}

impl SkillsOverlay {
    fn new(view: SkillsView, max_rows: usize, on_action: Box<dyn FnMut(SkillsAction)>) -> Self {
        Self {
            view,
            max_rows: max_rows.max(1),
            loading: false,
            theme: DARK_THEME,
            on_action,
        }
    }

    fn view_mut(&mut self) -> &mut SkillsView {
        &mut self.view
    }

    /// Show or clear the loading row (see [`SkillsOverlay::loading`]).
    fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.view.set_theme(&theme);
    }
}

impl Component for SkillsOverlay {
    fn render(&mut self, width: usize) -> Vec<String> {
        if !self.loading {
            return self.view.render(width, self.max_rows);
        }
        // The row comes out of the panel's row budget, never on top of it: the
        // compositor clips an overlay taller than its `max_height`, and the row
        // it would clip here is the one this row exists to show.
        let mut rows = self.view.render(width, self.max_rows.saturating_sub(1));
        rows.push(fg(
            self.theme.dim as u8,
            &fit_overlay_row(SKILLS_LOADING, width),
        ));
        rows
    }

    fn handle_input(&mut self, data: &str) {
        let action = self.view.handle_key(data);
        if action != SkillsAction::None {
            (self.on_action)(action);
        }
    }

    /// An active (or non-empty) search row owns the first escape: it clears the
    /// query instead of closing the panel. Without this the app layer's
    /// close-the-overlay fallback swallowed the key and the panel vanished on
    /// the first `escape` (the view's own `escape()` never ran — see
    /// `SkillsView::escape`).
    fn wants_escape(&self) -> bool {
        self.view.is_filtering() || !self.view.filter().is_empty()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Where `/keymap` persists its overrides: `keybindings.json` beside
/// `settings.json` (the two are separate files with separate schemas — the
/// keymap is not a field of the settings object).
fn keybindings_path_for(settings_path: &std::path::Path) -> PathBuf {
    settings_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(crate::keybindings::KEYBINDINGS_FILE)
}

/// Pad or truncate one overlay row to exactly `width` visible columns (the
/// compositor requires every row to measure the same).
fn fit_overlay_row(row: &str, width: usize) -> String {
    let clipped = truncate_to_width(row, width, &TruncateOptions::default());
    let visible = visible_width(&clipped);
    format!("{clipped}{}", " ".repeat(width.saturating_sub(visible)))
}

/// Hand `theme` to the two panels this round added.
///
/// `tui.rs::apply_theme_to_component` is the shared palette fan-out, but it
/// cannot know types that live here (and `tui.rs` is not this round's file), so
/// the `App` calls both — from `apply_theme` (panels already on the stack) and
/// from `show_overlay` (panels built after the switch). Everything else keeps
/// its single call site.
fn apply_theme_to_new_overlay(component: &mut dyn Component, theme: Theme) {
    if let Some(overlay) = component.as_any_mut().downcast_mut::<SandboxOverlay>() {
        overlay.set_theme(theme);
    } else if let Some(overlay) = component.as_any_mut().downcast_mut::<SkillsOverlay>() {
        overlay.set_theme(theme);
    } else if let Some(overlay) = component
        .as_any_mut()
        .downcast_mut::<crate::components::worktree_view::WorktreeOverlay>(
    ) {
        overlay.set_theme(theme);
    } else if let Some(overlay) = component
        .as_any_mut()
        .downcast_mut::<crate::components::keymap_view::KeymapOverlay>()
    {
        overlay.set_theme(theme);
    }
}

/// The extra panel row shown while the session's real tier is unknown: the
/// agent has no "read the sandbox policy" command, so before the user applies a
/// tier here the panel can only show the platform default. Presenting that
/// guess as the session's policy is exactly the kind of silent lie the
/// downgrade banner exists to prevent.
fn sandbox_tier_caveat(status: &SandboxStatus) -> Option<String> {
    (status.requested_tier.is_none() && status.tier == platform_default_tier(status.platform)).then(
        || {
            "Tier shown is this platform's default — apply one here to set the session's."
                .to_string()
        },
    )
}

/// Which half of the shared sandbox card a command opens it on.
///
/// The card holds two settings — the approval tier and the tool-permission
/// level — and `/sandbox` and `/permission` both open it, so the initial
/// highlight is the only thing that can tell the two commands apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SandboxFocus {
    /// `/sandbox`: the tier list owns the highlight.
    Tiers,
    /// `/permission`: the tool-permission rows do.
    Permissions,
}

/// Move the card's highlight from the tier list into the permission block.
///
/// The card is one flat list of six rows — `↑`/`↓` walk the three tiers and
/// then the three permission levels — so its own `↓` is what does this, and
/// walking it is exactly what a user pressing `/permission` would do next.
/// (`PermissionView` is reachable from the app read-only; the walk is the only
/// way in.) `below` counts the *selectable* tiers under the current one, so
/// the walk lands on the block's first row instead of skipping into it.
fn focus_permission_block(view: &mut SandboxView) {
    let options = view.tier_options();
    let current = view.highlighted_tier();
    let below = options
        .iter()
        .skip_while(|(tier, _)| *tier != current)
        .skip(1)
        .filter(|(_, enabled)| *enabled)
        .count();
    for _ in 0..=below {
        let _ = view.handle_key("down");
    }
}

/// Does this host want Chinese chrome? The TUI has no language setting (its
/// chrome is English), so `/title` — whose RPC *requires* one locale or the
/// other — follows the locale the user is running under, the only existing
/// signal. (`/skills` needs no guess: the panel falls back per field.)
fn prefers_chinese() -> bool {
    ["LC_ALL", "LC_MESSAGES", "LANG"].iter().any(|key| {
        std::env::var(key)
            .map(|value| value.to_ascii_lowercase())
            .is_ok_and(|value| value.contains("zh") || value.contains("cn"))
    })
}

/// The skills `get_state` already told us about, as browser rows. Descriptions
/// arrive later from `get_commands`; the names are what `enter` inserts.
fn local_skill_rows(names: &[String]) -> Vec<crate::components::skills_view::SkillRow> {
    names
        .iter()
        .map(|name| crate::components::skills_view::SkillRow {
            id: name.clone(),
            name: name.clone(),
            source: "skill".to_string(),
            ..Default::default()
        })
        .collect()
}

// ─── `future skills …` plumbing (the `/skills` panel's mutations) ────────────

/// The name of the unified binary ([`SkillsCli`] spawns `future skills …`).
const FUTURE_PROGRAM: &str = "future";

/// Panel / transcript notice while this host has no `future` executable to
/// spawn (`future-tui` installed on its own). The spawn would otherwise fail
/// with an opaque OS error, minutes later, mid-operation.
const SKILLS_NO_BINARY: &str =
    "The `future` executable was not found — installing or removing skills is unavailable.";

/// Panel notice when a second mutation arrives while one is still running.
const SKILLS_BUSY: &str = "A skill operation is already running — wait for it to finish.";

/// Row the `/skills` panel shows while `future skills list --json` is in
/// flight. The catalogue is the one source that is a *process* (the panel's
/// other source, `get_commands`, is an RPC that answers at once), so without
/// this row an open panel looks frozen for as long as the child takes.
const SKILLS_LOADING: &str = "Loading installable skills…";

/// Panel notice when `U` asks to upgrade while nothing is upgradeable. The
/// panel refuses that key itself; this covers a mutation that arrives without
/// the panel in front of it (its own confirmation could not be shown either).
const SKILLS_NO_UPGRADES: &str = "No installed skill has a newer version to upgrade.";

/// Shown when installing a recommended skill fails. The card stays up so the
/// user can retry or send without the skill (PRD v1.6 §6.2).
const SKILLS_RECO_INSTALL_FAILED: &str =
    "Could not install the recommended skill — press Esc to send without it, or a to retry.";

/// Row the `/worktree` panel shows while `git worktree list` is in flight (and
/// again on a reload). Both git calls are processes, so the panel is open — and
/// actionable through its create row — before its list exists.
const WORKTREE_LOADING: &str = "Reading git worktrees…";

/// Refusal for a working-directory change while a turn is running. Shared by
/// `/cwd` and `/worktree`: both move the session's cwd, and the agent's tools
/// read that directory.
const CWD_STREAMING_ERROR: &str = "Cannot change working directory while agent is streaming.";

/// One mutating skill operation the `/skills` panel asked for.
///
/// The panel's actions carry the id at most: the version an install pins comes
/// from the catalogue the app cached, and `U` names no skill at all — the app
/// resolves the set from the panel it is confirming against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillsMutation {
    /// `i` — install the id, or upgrade it when the catalogue is ahead (the
    /// same key, exactly as [`SkillsAction::Install`] documents).
    Install(String),
    /// `u`, pressed twice — remove the installation.
    Uninstall(String),
    /// `U` — `future skills update`: every installed skill the catalogue is
    /// ahead on.
    UpgradeAll,
}

/// The row the panel marks as busy while `op` runs: the id it acts on. `None`
/// for `UpdateAll`, which has no row of its own.
fn skill_op_pending_id(op: &SkillOp) -> Option<String> {
    match op {
        SkillOp::Install { id, .. } | SkillOp::Uninstall { id } => Some(id.clone()),
        SkillOp::UpdateAll => None,
    }
}

/// The transcript line a *started* operation writes, so a child that never
/// reports back still leaves a trace of what was run.
fn skill_op_start_message(op: &SkillOp) -> String {
    match op {
        SkillOp::Install { id, version } => match version {
            Some(version) => format!("Installing skill {id} v{version}…"),
            None => format!("Installing skill {id}…"),
        },
        SkillOp::Uninstall { id } => format!("Removing skill {id}…"),
        SkillOp::UpdateAll => "Upgrading every installed skill…".to_string(),
    }
}

/// The prompt `U` shows before upgrading everything: the exact set the second
/// press would touch.
fn upgrade_confirmation(ids: &[String]) -> String {
    format!(
        "This will upgrade {} installed skill{} ({}). Press U again to confirm.",
        ids.len(),
        if ids.len() == 1 { "" } else { "s" },
        ids.join(", ")
    )
}

/// The panel's status row for a finished operation, or `None` when it
/// succeeded (the row then goes back to the panel's own hints).
fn skill_op_error(outcome: &SkillOpOutcome) -> Option<String> {
    (!outcome.ok).then(|| describe_outcome(outcome))
}

/// The line a failed `future skills list --json` writes to the panel and to the
/// transcript.
///
/// Browsing the catalogue is *retryable* — the panel's `r` asks again — and it
/// is the one call with a short budget ([`crate::skills_cli::SKILL_LIST_TIMEOUT`]),
/// so a timeout is worth naming as retryable instead of leaving the user to
/// guess whether the panel is dead. Every other failure already carries its
/// cause and none of them is fixed by pressing a key.
fn skills_catalogue_error(err: &str) -> String {
    let text = format!("Failed to list installable skills: {err}");
    if err.contains(TIMEOUT_MARKER) {
        format!("{text} — press r to retry")
    } else {
        text
    }
}

/// One line describing a finished operation.
///
/// [`summarize_outcome`] already carries the exit code and a flattened, capped
/// stderr excerpt; the one thing it cannot know is *why* nothing came back,
/// which is what the timeout note adds.
fn describe_outcome(outcome: &SkillOpOutcome) -> String {
    let summary = summarize_outcome(outcome);
    if !outcome.timed_out {
        return summary;
    }
    format!("{summary} (killed after {}s)", SKILL_OP_TIMEOUT.as_secs())
}

/// Is `program` the bare fallback name rather than a located file?
///
/// [`SkillsCli::new`] only falls back to the bare name when *nothing* in
/// `future_binary_candidates` exists as a file — and that list already carries
/// one entry per `PATH` directory — so the fallback means "no `future` on this
/// host".
fn skills_binary_missing(program: &std::path::Path) -> bool {
    program == std::path::Path::new(FUTURE_PROGRAM)
}

/// The skill installer for this host, or `None` when there is no `future`
/// executable to run (see [`skills_binary_missing`]). `None` is what turns
/// "press `i`" into a sentence instead of a failed spawn.
///
/// Under `cfg(test)` the answer is always a CLI that *refuses to spawn
/// anything*: the resolution above would otherwise hand a test app whatever
/// `future` this host has on `PATH`, and a test that reaches `i`/`u`/`U`
/// without injecting its own [`SkillsCli::with_runner`] must fail loudly rather
/// than run a real installer. "Always" is deliberate — keying the test build
/// off the located binary made the whole suite depend on the host (no `future`
/// next to the test executable and none on `PATH`, as on a CI runner, and
/// every skill test saw the no-binary panel instead of the behaviour under
/// test). A test that wants the missing-binary state sets
/// [`App::skills_cli`] to `None`.
fn skills_cli_for_host() -> Option<Arc<SkillsCli>> {
    let cli = SkillsCli::new();
    let located = !skills_binary_missing(cli.program());
    let resolved = located.then(|| Arc::new(cli));
    #[cfg(test)]
    let resolved = test_skills_cli(resolved);
    resolved
}

/// [`skills_cli_for_host`] for the test build: a runner that reports a spawn
/// attempt instead of performing one, offered whether or not this host has a
/// real `future` binary.
#[cfg(test)]
fn test_skills_cli(_resolved: Option<Arc<SkillsCli>>) -> Option<Arc<SkillsCli>> {
    let runner: crate::skills_cli::SkillRunner =
        Box::new(|program: &std::path::Path, args: &[String]| {
            Err(format!(
                "a test tried to spawn {} {}",
                program.display(),
                args.join(" ")
            ))
        });
    Some(Arc::new(SkillsCli::with_runner(
        runner,
        PathBuf::from(FUTURE_PROGRAM),
    )))
}

/// The `git` plumbing `/worktree` uses on this host.
///
/// Under `cfg(test)` the answer is a CLI that *refuses to spawn anything*,
/// mirroring [`skills_cli_for_host`]: without it a test that reaches
/// `/worktree` would run the real `git` of whatever checkout the test happens
/// to sit in, so the answer would depend on the machine rather than on the
/// test. A test that wants the real git injects its own
/// [`GitCli::with_runner`] (or [`GitCli::new`]) into [`App::git_cli`].
fn git_for_host() -> Arc<GitCli> {
    // The real CLI is built first and handed to the test override below, the
    // same shape as [`skills_cli_for_host`]: no line exists that only one of
    // the two builds can run, and `GitCli::new` spawns nothing by itself.
    let cli = Arc::new(GitCli::new());
    #[cfg(test)]
    let cli = test_git_cli(cli);
    cli
}

/// [`git_for_host`] for the test build: the same CLI with a runner that
/// refuses to spawn (the program name is carried over from the real one).
#[cfg(test)]
fn test_git_cli(real: Arc<GitCli>) -> Arc<GitCli> {
    Arc::new(GitCli::with_runner(
        test_git_runner(),
        real.program().to_path_buf(),
    ))
}

/// The refusing runner [`test_git_cli`] installs: it reports the invocation
/// instead of performing it.
#[cfg(test)]
fn test_git_runner() -> GitRunner {
    Box::new(|args: &[String], _timeout: std::time::Duration| {
        Err(format!("a test tried to run git {}", args.join(" ")))
    })
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

    /// Whether `draft` is a message the recommender should be asked about.
    ///
    /// Every gate mirrors the desktop client's (PRD v1.6 §3): the feature on,
    /// a real message rather than a slash command, the length window, no skill
    /// already picked in the draft, the day's budget unspent, and this message
    /// not already asked about. The candidate set must be loaded, because
    /// asking with no candidates would recommend from nothing.
    fn recommendation_gates_pass(&self, draft: &str) -> bool {
        if !self.draft_could_be_recommended(draft) {
            return false;
        }
        let trimmed = draft.trim();
        let day = crate::skill_reco::load_at(&self.skill_reco_path);
        if day.exhausted() || day.already_evaluated(&crate::skill_reco::message_hash(trimmed)) {
            return false;
        }
        !self.skill_reco_candidates().is_empty()
    }

    /// The gates that depend on the draft alone: the feature is on, this is a
    /// message rather than a slash command, its length is in the window, and it
    /// does not already pick a skill.
    ///
    /// Split out from [`App::recommendation_gates_pass`] because the catalogue
    /// prefetch has to ask exactly the same question — "could this draft ever
    /// produce a card?" — and a second copy of these four rules would drift
    /// from the first.
    fn draft_could_be_recommended(&self, draft: &str) -> bool {
        if !self.tui_settings.skill_recommend_enabled() {
            return false;
        }
        let trimmed = draft.trim();
        // A slash command is a local action, not a message to recommend for.
        if trimmed.starts_with('/') {
            return false;
        }
        // Length window, in the units the PRD states them: 30 UTF-8 bytes is
        // 10 汉字 or about 30 ASCII characters.
        if trimmed.len() < crate::skill_reco::MIN_QUERY_BYTES
            || trimmed.chars().count() > crate::skill_reco::MAX_QUERY_CHARS
        {
            return false;
        }
        // The user already chose a skill for this message.
        !draft_picks_skill(trimmed)
    }

    /// Load the platform catalogue quietly, once, as soon as a draft could
    /// actually be recommended.
    ///
    /// The recommender chooses from `catalogue − installed`, and the catalogue
    /// came only from the `/skills` panel — so in a fresh session the candidate
    /// set was empty, `recommendation_gates_pass` refused every message, and the
    /// feature silently did nothing until the panel had been opened once. The
    /// desktop client has no such gap: it loads the catalogue when it mounts,
    /// i.e. long before a 30-byte draft exists.
    ///
    /// The same effect here, but keyed off the *draft* rather than the startup:
    /// a session that never writes a recommendable message never pays for a
    /// `future skills list` child (which reaches the platform over HTTP), and
    /// by the time such a draft is submitted the answer has almost always
    /// arrived (the call is ~0.15 s against a live platform). Called from
    /// [`App::handle_input_changed`], so the load runs while the user is still
    /// typing.
    ///
    /// One attempt per session: a failure leaves the cache empty and is not
    /// retried (`skills_catalogue_prefetch_started`), because nobody asked for
    /// this call — the panel is where a catalogue failure is reported, and
    /// opening it (or `r`) retries.
    fn prefetch_skill_catalogue(&mut self, draft: &str) {
        if self.skills_catalogue.is_some() || self.skills_catalogue_prefetch_started {
            return;
        }
        if !self.draft_could_be_recommended(draft) {
            return;
        }
        // Marked before the spawn: the flag is what stops a keystroke burst from
        // starting a second child, and it is never cleared.
        self.skills_catalogue_prefetch_started = true;
        // No `future` on this host: nothing to run, and nothing to say about it
        // here (the panel reports that separately, and `i`/`u` refuse with a
        // sentence rather than a failed spawn).
        let Some(cli) = self.skills_cli.clone() else {
            return;
        };
        let tx = self.op_tx.clone();
        tokio::task::spawn_blocking(move || {
            let _ = tx.send(UiCmd::SkillsCataloguePrefetched(cli.list()));
        });
    }

    /// The uninstalled skills offered to the recommender: the catalogue minus
    /// what the agent already loads. Empty when the catalogue has not been
    /// fetched yet — the panel fetches it, and the recommender prefetches it as
    /// soon as a draft could be recommended for (see
    /// [`App::prefetch_skill_catalogue`]) — in which case there is nothing to
    /// recommend from and the message is sent normally.
    fn skill_reco_candidates(&self) -> Vec<(String, String)> {
        let Some(catalogue) = self.skills_catalogue.as_ref() else {
            return Vec::new();
        };
        let installed: Vec<&String> = self.state.skills.iter().collect();
        catalogue
            .entries
            .iter()
            .filter(|entry| !installed.iter().any(|name| **name == entry.id))
            .map(|entry| {
                let summary = entry
                    .summary
                    .clone()
                    .or_else(|| entry.summary_zh.clone())
                    .unwrap_or_default();
                (entry.id.clone(), summary)
            })
            .take(crate::skill_reco::MAX_CANDIDATES)
            .collect()
    }

    /// Ask the agent about `draft`, holding it in the input box.
    ///
    /// Returns true when the submission is held (the answer arrives as
    /// [`UiCmd::SkillRecoSuggested`]); false lets `handle_submit` continue.
    fn maybe_recommend_skill(&mut self, draft: &str) -> bool {
        // The catalogue may still be missing (the prefetch has not answered yet,
        // or this draft never went through the input's change callback): start
        // it here too, so the *next* message can be recommended for even when
        // this one cannot. Never awaited — a send is not held for a catalogue.
        self.prefetch_skill_catalogue(draft);
        // The held draft is re-submitted through this same path; stand down for
        // that one pass so the send is not intercepted again.
        if self.skill_reco_send_through {
            self.skill_reco_send_through = false;
            return false;
        }
        if !matches!(self.skill_reco, SkillRecoState::Idle) {
            return false;
        }
        if !self.recommendation_gates_pass(draft) {
            return false;
        }
        let candidates = self.skill_reco_candidates();
        self.skill_reco = SkillRecoState::Pending {
            draft: draft.to_string(),
        };
        self.request_render(false);
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        let query = draft.to_string();
        tokio::spawn(async move {
            // Bounded here rather than by the gRPC client's 30 s deadline: an
            // answer that arrives after the user has given up reading is a
            // card out of nowhere, and the input is locked for this window.
            let suggestion = tokio::time::timeout(
                std::time::Duration::from_millis(crate::skill_reco::RECOMMEND_TIMEOUT_MS),
                client.suggest_skill(&query, &candidates),
            )
            .await
            .unwrap_or(None);
            let _ = tx.send(UiCmd::SkillRecoSuggested {
                draft: query,
                suggestion,
            });
        });
        true
    }

    /// True while the recommendation flow owns the input box.
    ///
    /// `Pending` (the agent is being asked) locks it, which is the TUI's
    /// equivalent of the desktop's disabled send button: the message that goes
    /// out has to be the one that was evaluated. `Suggested` does not — there
    /// the card is up and `a` / `Esc` decide.
    fn editor_locked_by_reco(&self) -> bool {
        matches!(self.skill_reco, SkillRecoState::Pending { .. })
    }

    /// Send the held draft, unchanged, through the normal submission path.
    ///
    /// `handle_submit` is re-entered so the send goes through exactly one code
    /// path (paste expansion, the size cap, attachment handling); the
    /// send-through flag keeps the intercept out of the way for that pass.
    fn send_held_draft(&mut self) {
        let draft = match &self.skill_reco {
            SkillRecoState::Pending { draft } | SkillRecoState::Suggested { draft, .. } => {
                draft.clone()
            }
            SkillRecoState::Idle => return,
        };
        self.skill_reco = SkillRecoState::Idle;
        self.skill_reco_send_through = true;
        self.request_render(false);
        self.handle_submit(&draft);
    }

    /// `a` on a shown recommendation: install it, then append `/skill` to the
    /// draft and send that.
    fn accept_skill_recommendation(&mut self) {
        let SkillRecoState::Suggested { draft, skill, .. } = self.skill_reco.clone() else {
            return;
        };
        let Some(cli) = self.skills_cli.clone() else {
            self.add_system_message(SKILLS_NO_BINARY.to_string());
            return;
        };
        let version = self.install_version_for(&skill);
        self.add_system_message(skill_op_start_message(&SkillOp::Install {
            id: skill.clone(),
            version: version.clone(),
        }));
        self.request_render(false);
        let tx = self.op_tx.clone();
        let skill_for_task = skill.clone();
        tokio::task::spawn_blocking(move || {
            let outcome = cli.run(&SkillOp::Install {
                id: skill_for_task.clone(),
                version,
            });
            let _ = tx.send(UiCmd::SkillRecoInstalled {
                draft,
                skill: skill_for_task,
                installed: outcome.ok,
            });
        });
    }

    /// Apply the agent's answer to a held draft.
    fn apply_skill_reco_suggestion(&mut self, draft: String, suggestion: Option<(String, String)>) {
        // Only the draft that asked may consume the answer.
        if !matches!(&self.skill_reco, SkillRecoState::Pending { draft: held } if held == &draft) {
            return;
        }
        let Some((skill, summary)) = suggestion else {
            // No recommendation: send it. This is the common path, and the
            // reason the call is best-effort.
            self.send_held_draft();
            return;
        };
        // Re-read the day: the budget may have moved while the call was in
        // flight, and a skill shown in the meantime must not be shown twice.
        let day = crate::skill_reco::load_at(&self.skill_reco_path);
        if day.exhausted() || day.already_recommended(&skill) {
            self.send_held_draft();
            return;
        }
        // Showing the card spends the budget: it counts when displayed,
        // whatever the user then does with it (PRD v1.6 §7).
        crate::skill_reco::record_at(
            &self.skill_reco_path,
            &skill,
            &crate::skill_reco::message_hash(&draft),
        );
        self.skill_reco = SkillRecoState::Suggested {
            draft,
            skill,
            summary,
        };
        self.request_render(false);
    }

    /// Append `/skill` to the held draft and send it, after a successful
    /// install.
    fn use_recommended_skill(&mut self, draft: &str, skill: &str) {
        let separator = if draft.ends_with(' ') || draft.is_empty() {
            ""
        } else {
            " "
        };
        let composed = format!("{draft}{separator}/{skill} ");
        self.skill_reco = SkillRecoState::Idle;
        self.skill_reco_send_through = true;
        self.input.set_value(&composed, None);
        self.handle_submit(&composed);
    }

    fn handle_submit(&mut self, value: &str) {
        // A pending `/provider-key` capture owns the next submission: the text
        // is the API key (a blank submission clears the stored key), never a
        // prompt. This block must run BEFORE the empty-input guard below —
        // otherwise a blank submission is swallowed and the "empty input
        // clears the stored key" promise in `prompt_provider_key` is a lie.
        if let Some(provider) = self.pending_secret.take() {
            self.input.set_value("", None);
            if value.trim() == "/cancel-input" {
                self.add_system_message(format!("Cancelled the key prompt for {provider}."));
                self.request_render(false);
                return;
            }
            let key = value.trim().to_string();
            let client = self.client.clone();
            let tx = self.op_tx.clone();
            tokio::spawn(async move {
                let result = if key.is_empty() {
                    client.set_auth_key(&provider, None).await
                } else {
                    client.set_auth_key(&provider, Some(&key)).await
                };
                let _ = tx.send(UiCmd::ProviderActionDone {
                    action: if key.is_empty() {
                        format!("provider {provider} key cleared")
                    } else {
                        format!("provider {provider} key updated")
                    },
                    result,
                });
            });
            self.request_render(false);
            return;
        }

        // Any other blank submission is a no-op (it must not reach the prompt
        // path). This guard used to sit at the very top of the function, which
        // is exactly why the key-clear path above was unreachable.
        if value.trim().is_empty() {
            return;
        }

        // `/editor` owns the *current draft*, so it has to run before the
        // input is cleared for a normal submission.
        if value.trim() == "/editor" {
            self.open_external_editor();
            return;
        }

        // The draft is about to be consumed: take the pastes and images it
        // still refers to *before* the box is emptied (an edit drops the
        // attachments whose markers left the draft), and the pending state goes
        // back with the draft on any guard that puts it back.
        // Skill recommendation (PRD v1.6 §6.1): hold the draft while the agent
        // is asked whether a skill fits it. Placed before the draft is consumed
        // below, because holding it means leaving it in the input box.
        if self.maybe_recommend_skill(value) {
            return;
        }
        // With a card on screen Enter means what the card's second action means:
        // send the draft without the skill (desktop/mobile keep their send
        // button live for this and send too). Doing nothing here read as a dead
        // send key. The card's own `a` / `Esc` re-enter this same path with the
        // state already back to Idle.
        if matches!(self.skill_reco, SkillRecoState::Suggested { .. }) {
            self.send_held_draft();
            return;
        }
        // While the agent is being asked, a second Enter is the TUI's "send
        // button while it is disabled": it does nothing rather than racing the
        // answer that the held draft belongs to.
        if !matches!(self.skill_reco, SkillRecoState::Idle) {
            return;
        }

        let pending = self.input.take_pending();
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
                    if !arg.is_empty() {
                        self.set_session_model(&arg);
                    } else {
                        self.show_model_selector();
                    }
                }
                "models" if arg.trim() == "default" => {
                    let client = self.client.clone();
                    let tx = self.op_tx.clone();
                    tokio::spawn(async move {
                        let _ = tx.send(UiCmd::ModelsLoaded {
                            result: client.list_models().await,
                            purpose: ModelsPurpose::Default,
                        });
                    });
                }
                "models" | "scoped-models" => {
                    let client = self.client.clone();
                    let tx = self.op_tx.clone();
                    tokio::spawn(async move {
                        let _ = tx.send(UiCmd::ModelsLoaded {
                            result: client.list_models().await,
                            purpose: ModelsPurpose::Scoped,
                        });
                    });
                }
                "theme" if !arg.trim().is_empty() => {
                    self.select_theme(arg.trim());
                }
                "theme" => self.show_theme_menu(),
                "skill-recommend" => self.set_skill_recommend(&arg),
                "providers" => self.show_providers(),
                "skills" => self.show_skills(),
                "tools" => {
                    // `none` / `all` cover the two selection states the
                    // multi-select menu cannot express: `enter` on an empty
                    // selection falls back to the highlighted row (see
                    // `MenuState::confirm`), so "disable everything" needs its
                    // own argument form.
                    match arg.trim() {
                        "none" | "off" => self.apply_tools(Vec::new()),
                        "all" => self.apply_tools(
                            BUILTIN_TOOLS
                                .iter()
                                .map(|name| (*name).to_string())
                                .collect(),
                        ),
                        _ => self.show_tools_menu(),
                    }
                }
                "usage" => self.show_usage(),
                "transcript" => self.show_transcript(),
                "copy" => self.copy_last_assistant_message(),
                "provider-key" => {
                    if arg.is_empty() {
                        self.add_system_message("Usage: /provider-key <provider-id>".into());
                    } else {
                        self.prompt_provider_key(&arg);
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
                    if self.state.streaming
                        || self.state.compacting
                        || self.state.compaction_requested
                    {
                        self.add_system_message(
                            "Cannot compact while a run or compaction is in progress".into(),
                        );
                        return;
                    }
                    self.state.compaction_revision += 1;
                    self.state.compaction_requested = true;
                    self.add_system_message("Requesting context compaction…".into());
                    let client = self.client.clone();
                    let tx = self.op_tx.clone();
                    let session_id = self.state.session_id.clone();
                    tokio::spawn(async move {
                        let _ = tx.send(UiCmd::CompactDone {
                            session_id,
                            result: client.compact(None).await,
                        });
                    });
                }
                "export" => {
                    self.export_session();
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
                        let mut history = Ok(Value::Null);
                        if let Ok(ref v) = result {
                            let cancelled =
                                v.get("cancelled").and_then(Value::as_bool).unwrap_or(false);
                            if !cancelled {
                                state = client.get_state().await.ok();
                                history =
                                    load_history_tail(&client, &client.get_current_session_id())
                                        .await;
                            }
                        }
                        let _ = tx.send(UiCmd::CloneDone {
                            result,
                            state,
                            history,
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
                    self.start_new_session();
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
                "cwd" if !arg.is_empty() => {
                    if self.cwd_change_blocked() {
                        return;
                    }
                    // Trim the arg: `/cwd ../ ` (trailing space) would join to
                    // `a/b/../ ` and normalize to `a/ ` (the stray space
                    // becomes a path component). The agent trims its side;
                    // the TUI must too, or the footer shows the dirty path.
                    let mut resolved = arg.trim().to_string();
                    let homedir = crate::home::home_dir_or_default();
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
                    self.switch_cwd(normalize_path(&resolved));
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
                        let stats = client.get_session_stats().await;
                        let _ = tx.send(UiCmd::StatusLoaded {
                            state,
                            models,
                            stats,
                        });
                    });
                }
                // ── Agent/session inspection and settings ───────────────
                "agent" => self.show_agent_info(),
                "history" if !arg.trim().is_empty() => self.search_history(arg.trim()),
                "history" => self.add_system_message("Usage: /history <query>".into()),
                "autocompact" => match Self::parse_toggle_arg(&arg) {
                    Ok(want) => self
                        .set_auto_compaction(want.unwrap_or(!self.state.auto_compaction_enabled)),
                    Err(bad) => self
                        .add_system_message(format!("Usage: /autocompact [on|off] (got '{bad}')")),
                },
                "autoretry" => match Self::parse_toggle_arg(&arg) {
                    Ok(want) => self.set_auto_retry(want.unwrap_or(!self.state.auto_retry_enabled)),
                    Err(bad) => {
                        self.add_system_message(format!("Usage: /autoretry [on|off] (got '{bad}')"))
                    }
                },
                "tool-output" if !arg.trim().is_empty() => self.show_tool_output(arg.trim()),
                "tool-output" => self.show_tool_calls(),
                // ── Sandbox, permissions and the skill browser ──────────
                "sandbox" => self.show_sandbox(),
                "permission" => self.show_permission(&arg),
                "worktree" => self.worktree_command(&arg),
                // ── Key bindings ────────────────────────────────────────
                "keymap" if arg.trim().is_empty() => self.show_keymap(),
                "keymap" => self.add_system_message("Usage: /keymap (opens the key editor)".into()),
                // ── One-shot agent actions ──────────────────────────────
                "shell" => match Self::shell_argument(value) {
                    Some(command) => self.run_shell(command),
                    None => self.add_system_message("Usage: /shell <command>".into()),
                },
                "title" => self.generate_session_title(&arg),
                "metrics" => self.load_metrics(),
                "snapshot" => self.load_run_snapshot(),
                "context" => self.context_files(&arg),
                "delete" => self.delete_current_session(&arg),
                _ => handled = false,
            }
            if handled {
                return;
            }
            // Unknown slash command — falls through to the regular prompt.
        }

        // Keep the draft intact: compaction rejects prompts rather than
        // enqueueing them, even when ordinary running turns allow a queue.
        if self.state.compacting || self.state.compaction_requested {
            self.input.set_value(value, None);
            self.input.restore_pending(pending);
            self.add_system_message(
                "Context compaction is in progress. Wait before sending; your draft is preserved."
                    .into(),
            );
            return;
        }

        // The message the model sees: the draft the box shows, with every
        // surviving paste put back. A placeholder the user deleted is gone from
        // the draft, so its text is gone from the message.
        let message = pending.expand(value);
        // Insert-time checks make this unreachable by typing or pasting; a
        // hand-copied placeholder (the same name twice) is the one way a draft
        // can assemble past the cap, and the agent's bill is the last place to
        // find out.
        if let Some(notice) = crate::paste::over_limit_message(message.chars().count()) {
            self.input.set_value(value, None);
            self.input.restore_pending(pending);
            self.add_system_message(notice);
            return;
        }
        let attachments = pending.attachments().to_vec();
        let images = attachments.iter().filter(|a| a.kind == "image").count();
        if images > 0 && self.current_model_image_support() == Some(false) {
            self.add_system_message(format!(
                "⚠ {images} image(s) attached, but {} cannot view images: the agent sends \
                 their file paths in the prompt instead. Switch to a vision model to send \
                 the picture itself.",
                self.state.model
            ));
        }

        // Regular prompt — send to server.
        let local_message_id = random_id();
        self.chat.add_message(ChatMessage::new(
            local_message_id.clone(),
            ChatRole::User,
            &message,
        ));

        if self.state.streaming {
            // Every submission is its own run. The Agent owns the FIFO and
            // returns the canonical queued run identity.
            let client = self.client.clone();
            let tx = self.op_tx.clone();
            let outgoing = message.clone();
            tokio::spawn(async move {
                let result = client
                    .prompt(&outgoing, "enqueue_if_busy", attachments)
                    .await;
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
        tokio::spawn(async move {
            let result = client
                .prompt(&message, "enqueue_if_busy", attachments)
                .await;
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

    // ─── Sessions ──────────────────────────────────────────────────────

    /// `/new` — create a fresh session, inheriting cwd, model and thinking
    /// level so it feels like a clean continuation. Shared with `/delete`,
    /// which has to leave the TUI on a live session once it removes one.
    fn start_new_session(&mut self) {
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

    /// `/sessions` — pick a session. Built on [`MenuState`] so the picker gets
    /// incremental search, the scroll window and key hints; the session id is
    /// the item value and the cwd the description.
    fn show_sessions_overlay(&mut self, sessions: Vec<SessionSummary>) {
        let home = std::env::var("HOME").unwrap_or_default();
        let items: Vec<MenuItem> = sessions
            .iter()
            .map(|s| {
                let name = session_display_name(s);
                let label = sanitize_session_name(&name);
                let mut item = MenuItem::new(s.id.clone(), label.clone());
                if !s.cwd.is_empty() {
                    item = item.with_description(shorten_cwd(&s.cwd, &home));
                }
                if let Some(true) = s.is_streaming {
                    item = item.with_badges(["streaming"]);
                }
                if s.id == self.state.session_id {
                    item = item.with_badges(["current"]);
                }
                self.session_labels.insert(s.id.clone(), label);
                item
            })
            .collect();
        let options = MenuOptions::new("Sessions", vec![MenuSection::flat(items)])
            .searchable(true)
            .max_visible(15)
            .with_footer_hints(Self::select_hints());
        self.show_purpose_menu(MenuPurpose::Sessions, MenuState::new(options));
    }

    /// `/tree` — the fork/clone hierarchy, on the same menu framework. The
    /// hierarchy is drawn with `MenuItem::indent` rather than ASCII art so the
    /// label stays readable and searchable.
    fn show_tree_overlay(&mut self, sessions: Vec<SessionSummary>) {
        if sessions.is_empty() {
            self.add_system_message("No sessions found.".into());
            return;
        }
        // Group sessions by cwd, build tree from parent_session_id.
        let mut grouped: std::collections::BTreeMap<String, Vec<SessionSummary>> =
            std::collections::BTreeMap::new();
        for s in &sessions {
            let cwd = if s.cwd.is_empty() { "" } else { s.cwd.as_str() };
            grouped.entry(cwd.to_string()).or_default().push(s.clone());
        }

        let mut items: Vec<MenuItem> = Vec::new();
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
            roots.sort_by_key(|s| std::cmp::Reverse(s.updated_at_ms));
            self.flatten_tree(&children, &mut roots, 0, &mut items);
        }
        let options = MenuOptions::new("Session Tree", vec![MenuSection::flat(items)])
            .searchable(true)
            .max_visible(20)
            .with_footer_hints(Self::select_hints());
        self.show_purpose_menu(MenuPurpose::SessionTree, MenuState::new(options));
    }

    /// Depth-first flatten of the session tree into menu rows, deepest-last
    /// children first sorted by recency at every level.
    fn flatten_tree(
        &mut self,
        children: &HashMap<String, Vec<SessionSummary>>,
        list: &mut [SessionSummary],
        depth: usize,
        items: &mut Vec<MenuItem>,
    ) {
        list.sort_by_key(|a| std::cmp::Reverse(a.updated_at_ms));
        for s in list.iter() {
            let name = session_display_name(s);
            let label = sanitize_session_name(&name);
            let mut item = MenuItem::new(s.id.clone(), label.clone()).with_indent(depth);
            if s.id == self.state.session_id {
                item = item.with_badges(["current"]);
            }
            self.session_labels.insert(s.id.clone(), label);
            items.push(item);
            if let Some(child_list) = children.get(&s.id) {
                let mut child_list = child_list.clone();
                self.flatten_tree(children, &mut child_list, depth + 1, items);
            }
        }
    }

    /// Switch to a session picked from the sessions/tree menu. The label is
    /// recovered from `session_labels` because the menu returns ids only.
    fn switch_to_session(&mut self, session_id: Option<&String>) {
        let Some(id) = session_id else {
            self.request_render(false);
            return;
        };
        if id == &self.state.session_id {
            self.request_render(false);
            return;
        }
        let label = self
            .session_labels
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.clone());
        self.save_session_input();
        self.spawn_switch_flow(id, label);
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
                    m.get("createdAtMs")
                        .and_then(Value::as_i64)
                        .and_then(chrono::DateTime::from_timestamp_millis)
                        .map(|time| time.to_rfc3339())
                        .unwrap_or_default()
                ),
                description: Some(
                    m.get("blocks")
                        .and_then(Value::as_array)
                        .and_then(|blocks| blocks.iter().find(|b| b["kind"] == "text"))
                        .and_then(|b| b["text"].as_str())
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

    // ─── Popup menus (MenuState overlays) ─────────────────────────────

    /// Push a popup menu over the current screen.
    ///
    /// The callback follows the same rule as the `SelectList` overlays: it
    /// sends a `UiCmd` instead of capturing the app, so no `&mut self` is held
    /// across the overlay's key handling.
    fn show_menu_overlay(
        &mut self,
        state: MenuState,
        max_width: usize,
        on_action: Box<dyn FnMut(MenuAction)>,
    ) {
        // The palette is applied by `show_overlay` (the one place every
        // overlay passes through).
        let overlay = MenuOverlay::new(state, on_action);
        let width = (self.terminal.columns() as usize)
            .saturating_sub(4)
            .min(max_width);
        self.show_overlay(
            Box::new(overlay),
            OverlayOptions {
                width: Some(SizeValue::Fixed(width)),
                ..Default::default()
            },
        );
    }

    /// Push a popup menu whose value-carrying actions are routed back to the
    /// app loop as `MenuSelected`/`MenuCancelled` for `purpose`.
    fn show_purpose_menu(&mut self, purpose: MenuPurpose, state: MenuState) {
        let tx = self.op_tx.clone();
        self.show_menu_overlay(state, 80, Self::menu_sink(tx, purpose));
    }

    /// Footer hints shared by the single-select menus.
    fn select_hints() -> Vec<(String, String)> {
        vec![
            ("↑↓".into(), "navigate".into()),
            ("enter".into(), "select".into()),
            ("/".into(), "search".into()),
            ("esc".into(), "close".into()),
        ]
    }

    /// The menu callback body for `purpose`: every value-carrying action
    /// becomes a `UiCmd`.
    fn menu_sink(
        tx: mpsc::UnboundedSender<UiCmd>,
        purpose: MenuPurpose,
    ) -> Box<dyn FnMut(MenuAction)> {
        let tx = tx;
        Box::new(move |action: MenuAction| {
            let cmd = match action {
                MenuAction::Confirmed(values) => UiCmd::MenuSelected { purpose, values },
                MenuAction::Toggled(values) => UiCmd::MenuSelected { purpose, values },
                MenuAction::Cancelled => UiCmd::MenuCancelled(purpose),
                MenuAction::None | MenuAction::Moved | MenuAction::TabChanged => return,
            };
            let _ = tx.send(cmd);
        })
    }

    /// Sorted model ids plus their display name, the shared shape of the three
    /// model menus.
    fn model_rows(all_models: Vec<ModelInfo>) -> Vec<(String, String, bool)> {
        let mut models: Vec<ModelInfo> = all_models;
        models.sort_by_key(|model| model.full_id());
        models
            .into_iter()
            .map(|model| {
                let id = model.full_id();
                let label = if model.label.is_empty() || model.label == id {
                    String::new()
                } else {
                    model.label
                };
                (id, label, model.is_default)
            })
            .collect()
    }

    /// `/model` — single-select over every model (sets the session model).
    fn build_model_menu(all_models: Vec<ModelInfo>, current: &str) -> MenuState {
        let items: Vec<MenuItem> = Self::model_rows(all_models)
            .into_iter()
            .map(|(id, label, _)| {
                let mut item = MenuItem::new(id.clone(), id.clone());
                if !label.is_empty() {
                    item = item.with_description(label);
                }
                if id == current {
                    item = item.with_badges(["current"]);
                }
                item
            })
            .collect();
        let options = MenuOptions::new("Select Model", vec![MenuSection::flat(items)])
            .searchable(true)
            .max_visible(15)
            .with_footer_hints(Self::select_hints());
        MenuState::new(options)
    }

    /// `/scoped-models` and `/models` — multi-select over the model scope.
    fn build_scope_menu(
        all_models: Vec<ModelInfo>,
        enabled: Option<&[String]>,
        default_model: &str,
    ) -> MenuState {
        let enabled: Option<std::collections::HashSet<&str>> =
            enabled.map(|ids| ids.iter().map(String::as_str).collect());
        let items: Vec<MenuItem> = Self::model_rows(all_models)
            .into_iter()
            .map(|(id, label, _)| {
                let mut item = MenuItem::new(id.clone(), id.clone());
                if !label.is_empty() {
                    item = item.with_description(label);
                }
                if id == default_model {
                    item = item.with_badges(["default"]);
                }
                // No scope configured ⇒ every model is enabled.
                if enabled.as_ref().is_none_or(|set| set.contains(id.as_str())) {
                    item = item.selected();
                }
                item
            })
            .collect();
        let options = MenuOptions::new("Model Scope", vec![MenuSection::flat(items)])
            .searchable(true)
            .multi_select(true)
            .max_visible(15)
            .with_footer_hints(vec![
                ("↑↓".to_string(), "navigate".to_string()),
                ("space".to_string(), "toggle".to_string()),
                ("enter".to_string(), "save".to_string()),
                ("/".to_string(), "search".to_string()),
                ("esc".to_string(), "close".to_string()),
            ]);
        MenuState::new(options)
    }

    /// `/models default` — single-select, persisted as the agent-side default.
    fn build_default_model_menu(all_models: Vec<ModelInfo>) -> MenuState {
        let items: Vec<MenuItem> = Self::model_rows(all_models)
            .into_iter()
            .map(|(id, label, is_default)| {
                let mut item = MenuItem::new(id.clone(), id.clone());
                if !label.is_empty() {
                    item = item.with_description(label);
                }
                if is_default {
                    item = item.with_badges(["default"]);
                }
                item
            })
            .collect();
        let options = MenuOptions::new("Default Model", vec![MenuSection::flat(items)])
            .searchable(true)
            .max_visible(15)
            .with_footer_hints(Self::select_hints());
        MenuState::new(options)
    }

    /// Set the session model (the `/model <id>` path, shared with the menu).
    fn set_session_model(&mut self, model: &str) {
        if self.state.streaming {
            self.add_system_message(
                "Cannot change model while agent is streaming. Wait for the current run to finish."
                    .into(),
            );
            self.request_render(false);
            return;
        }
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        let model = model.to_string();
        // Optimistically reflect the pick so the menu/footer do not lag the
        // round trip (`SetModelDone` re-asserts it from `get_state`).
        self.state.model = model.clone();
        self.tui_settings.default_model = Some(model.clone());
        self.save_tui_settings();
        tokio::spawn(async move {
            let set_result = client.set_model(&model).await;
            let state = client.get_state().await.ok();
            let _ = tx.send(UiCmd::SetModelDone { set_result, state });
        });
        self.request_render(false);
    }

    /// Push a tool selection to the agent (`/tools`).
    fn apply_tools(&mut self, tools: Vec<String>) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        // Keep the list in canonical order so the menu re-opens the same way.
        let mut tools: Vec<String> = tools;
        tools.sort();
        tokio::spawn(async move {
            let result = if tools.is_empty() {
                client.disable_tools().await.map(|_| Vec::new())
            } else {
                client.set_tools(&tools).await.map(|_| tools.clone())
            };
            let _ = tx.send(UiCmd::ToolsChanged(result));
        });
        self.request_render(false);
    }

    /// `/theme` — single-select over the catalog; applying is immediate.
    fn show_theme_menu(&mut self) {
        let (current, _theme) = crate::themes::resolve_theme(self.tui_settings.theme_id.as_deref());
        let items: Vec<MenuItem> = crate::themes::theme_options()
            .into_iter()
            .map(|(id, label)| {
                let mut item = MenuItem::new(id, label);
                if id == current {
                    item = item.with_badges(["current"]);
                }
                item
            })
            .collect();
        let options = MenuOptions::new("Theme", vec![MenuSection::flat(items)])
            .searchable(true)
            .max_visible(10)
            .with_footer_hints(Self::select_hints());
        let tx = self.op_tx.clone();
        self.show_menu_overlay(
            MenuState::new(options),
            60,
            Self::menu_sink(tx, MenuPurpose::Theme),
        );
    }

    // ─── /skills (browser + use), /sandbox, /permission ───────────────

    // ─── /worktree ────────────────────────────────────────────────────

    /// `/worktree` — the git worktree picker.
    ///
    /// The panel opens with the create row and a "reading git…" notice, then
    /// fills in when the listing lands: `git worktree list` is a process, so it
    /// runs on the blocking pool and the render loop never waits for it.
    fn show_worktree(&mut self) {
        let view = WorktreeView::new(
            Vec::new(),
            &self.state.cwd,
            Some(WORKTREE_LOADING.to_string()),
        );
        let rows = (self.terminal.rows() as usize).max(3);
        let overlay = WorktreeOverlay::new(
            view,
            rows.saturating_sub(2),
            Self::worktree_sink(self.op_tx.clone()),
        );
        let width = (self.terminal.columns() as usize).saturating_sub(4).min(80);
        self.show_overlay(
            Box::new(overlay),
            OverlayOptions {
                width: Some(SizeValue::Fixed(width)),
                max_height: Some(SizeValue::Fixed(rows)),
                ..Default::default()
            },
        );
        self.load_worktrees();
    }

    /// The `/worktree` panel's callback: the panel reports an intent, the app
    /// turns it into a command (`None`/`Moved` stay silent — they only mean
    /// "redraw").
    fn worktree_sink(tx: mpsc::UnboundedSender<UiCmd>) -> Box<dyn FnMut(WorktreeAction)> {
        Box::new(move |action: WorktreeAction| {
            let cmd = match action {
                WorktreeAction::Select(path) => UiCmd::WorktreeSwitchRequested(path),
                WorktreeAction::New => UiCmd::WorktreeNewRequested,
                WorktreeAction::Refresh => UiCmd::WorktreeRefreshRequested,
                WorktreeAction::Cancelled => UiCmd::OverlayCancel,
                WorktreeAction::None => return,
            };
            let _ = tx.send(cmd);
        })
    }

    /// `git worktree list --porcelain` (+ one `git status` per entry) on the
    /// blocking pool → [`UiCmd::WorktreesLoaded`].
    ///
    /// The panel keeps whatever it already shows and only raises the notice, so
    /// a reload never blanks the list the user is reading.
    fn load_worktrees(&mut self) {
        let git = self.git_cli.clone();
        let tx = self.op_tx.clone();
        let session_cwd = self.state.cwd.clone();
        // The TUI's own directory is the fallback for a session whose cwd is
        // not inside a repository (`/mock`, `~`, a scratch dir): that is where
        // the user actually started this TUI.
        let fallback = std::env::current_dir().ok();
        if let Some(overlay) = self.top_worktree_overlay() {
            overlay
                .view_mut()
                .set_notice(Some(WORKTREE_LOADING.to_string()));
        }
        self.request_render(false);
        tokio::task::spawn_blocking(move || {
            let result = probe_repo(&git, &session_cwd, fallback.as_deref());
            let _ = tx.send(UiCmd::WorktreesLoaded { result });
        });
    }

    /// The `/worktree` panel of the top overlay, when that is what is open.
    fn top_worktree_overlay(&mut self) -> Option<&mut WorktreeOverlay> {
        let idx = self.get_top_overlay_index()?;
        self.overlay_stack[idx]
            .component
            .as_any_mut()
            .downcast_mut::<WorktreeOverlay>()
    }

    /// Write `text` into the panel's notice row, or into the transcript when no
    /// panel is open (a git answer can outlive the panel it was asked for).
    fn set_worktree_notice(&mut self, text: &str) {
        match self.top_worktree_overlay() {
            Some(overlay) => {
                overlay.view_mut().set_notice(Some(text.to_string()));
                self.request_render(false);
            }
            None => self.add_system_message(text.to_string()),
        }
    }

    /// `/worktree [new <branch>]` — the panel, or a create.
    ///
    /// The panel is the only way to *switch* (it lists what git reported, with
    /// the branch and the dirty state); a name can only be typed, which is what
    /// `/worktree new` is for.
    fn worktree_command(&mut self, arg: &str) {
        let arg = arg.trim();
        if arg.is_empty() {
            self.show_worktree();
            return;
        }
        match arg.strip_prefix("new ") {
            Some(name) => self.create_named_worktree(name),
            None if arg == "new" => self.add_system_message(
                "Usage: /worktree new <branch>, e.g. /worktree new feat/my-change.".into(),
            ),
            None => self.add_system_message(WORKTREE_USAGE.to_string()),
        }
    }

    /// `/worktree new <branch>` — create the worktree and move the session into
    /// it.
    ///
    /// The name is validated *here*, before the pool task, so a bad one costs a
    /// message rather than a process; the create itself (list → plan → add)
    /// runs on the blocking pool because it spawns git.
    fn create_named_worktree(&mut self, name: &str) {
        if self.cwd_change_blocked() {
            return;
        }
        if let Err(err) = crate::worktree::validate_worktree_name(name) {
            self.add_system_message(err);
            return;
        }
        let git = self.git_cli.clone();
        let tx = self.op_tx.clone();
        let session_cwd = self.state.cwd.clone();
        let fallback = std::env::current_dir().ok();
        let name = name.to_string();
        self.add_system_message(format!("Creating worktree {name}…"));
        self.request_render(false);
        // The child process is git's, and it is bounded by
        // `GIT_ADD_TIMEOUT` inside the CLI (a slow remote is killed, not
        // waited for); this task only carries the answer back.
        tokio::task::spawn_blocking(move || {
            let result = create_worktree(&git, &session_cwd, fallback.as_deref(), &name);
            let _ = tx.send(UiCmd::WorktreeAdded { result });
        });
    }

    /// Move the session's working directory, the way `/cwd` does: one code path
    /// for both commands, so the footer, the `cwd_changed` event and the
    /// transcript line cannot disagree.
    fn switch_cwd(&mut self, resolved: String) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let result = client.set_cwd(&resolved).await;
            let _ = tx.send(UiCmd::CwdSet { result, resolved });
        });
    }

    /// Is a working-directory change refused right now?
    ///
    /// `/cwd` and `/worktree` both move the session's cwd, and moving it under
    /// a running turn would leave the agent's tools reading one directory while
    /// the footer shows another. Returns `true` (and says so) when refused.
    fn cwd_change_blocked(&mut self) -> bool {
        if !self.state.streaming {
            return false;
        }
        self.add_system_message(CWD_STREAMING_ERROR.into());
        self.request_render(false);
        true
    }

    // ─── /keymap ──────────────────────────────────────────────────────

    /// `/keymap` — the key-binding editor.
    ///
    /// The panel lists every action the manager registered (see
    /// [`crate::components::keymap_view`]); this method only builds it and hands
    /// it the two things it cannot know: the live bindings and the problems the
    /// last `keybindings.json` read reported.
    fn show_keymap(&mut self) {
        let view = KeymapView::new(self.keymap_model(), self.keybinding_problems.clone());
        let rows = (self.terminal.rows() as usize).max(3);
        let overlay = KeymapOverlay::new(
            view,
            rows.saturating_sub(2),
            Self::keymap_sink(self.op_tx.clone()),
        );
        let width = (self.terminal.columns() as usize).saturating_sub(4).min(80);
        self.show_overlay(
            Box::new(overlay),
            OverlayOptions {
                width: Some(SizeValue::Fixed(width)),
                max_height: Some(SizeValue::Fixed(rows)),
                ..Default::default()
            },
        );
    }

    /// The `/keymap` panel's callback: a captured key or a reset becomes a
    /// `UiCmd`, so no `&mut App` is held while the component handles the key.
    fn keymap_sink(tx: mpsc::UnboundedSender<UiCmd>) -> Box<dyn FnMut(KeymapAction)> {
        Box::new(move |action: KeymapAction| {
            let cmd = match action {
                KeymapAction::Bind { description, key } => UiCmd::KeymapBind { description, key },
                KeymapAction::ResetAction(description) => UiCmd::KeymapReset {
                    description: Some(description),
                },
                KeymapAction::ResetAll => UiCmd::KeymapReset { description: None },
                KeymapAction::Cancelled => UiCmd::OverlayCancel,
                KeymapAction::None => return,
            };
            let _ = tx.send(cmd);
        })
    }

    /// The `/keymap` panel of the top overlay, when that is what is open.
    fn top_keymap_overlay(&mut self) -> Option<&mut KeymapOverlay> {
        let idx = self.get_top_overlay_index()?;
        self.overlay_stack[idx]
            .component
            .as_any_mut()
            .downcast_mut::<KeymapOverlay>()
    }

    /// The manager's state, in the shape the panel renders.
    fn keymap_model(&self) -> KeymapModel {
        KeymapModel::new(
            self.keybindings.action_bindings(),
            self.keybindings.key_owners(),
        )
    }

    /// Write `text` into the panel's status row, or into the transcript when no
    /// panel is open (a save can outlive the panel it was asked for).
    fn set_keymap_notice(&mut self, text: String) {
        match self.top_keymap_overlay() {
            Some(overlay) => {
                overlay.view_mut().set_notice(Some(text));
                self.request_render(false);
            }
            None => self.add_system_message(text),
        }
    }

    /// Re-read the manager into the panel that is already open (after a bind or
    /// a reset landed), so the rows never show the state from before the
    /// change.
    fn refresh_keymap_panel(&mut self) {
        let model = self.keymap_model();
        if let Some(overlay) = self.top_keymap_overlay() {
            overlay.view_mut().set_model(model);
        }
        self.request_render(false);
    }

    /// Read `~/.future/tui/keybindings.json` into the live manager.
    ///
    /// Never fails, and never half-applies: a missing file is the normal case
    /// (the built-in keys), a broken one leaves them alone, and every entry is
    /// validated with the same rule the panel's capture uses. An entry that only
    /// restates the built-in key is a no-op rather than a problem — that is what
    /// makes `"Interrupt / exit": "ctrl+c"` in a hand-written file harmless.
    fn load_keybindings(&mut self) {
        let file = crate::keybindings::load_keybindings_file(&self.keybindings_path);
        let mut problems = file.problems;
        let mut unknown: Vec<(String, String)> = Vec::new();
        for (description, key) in file.assignments {
            let Some(current) = self
                .keybindings
                .action_bindings()
                .into_iter()
                .find(|action| action.description == description)
            else {
                problems.push(format!(
                    "\"{description}\" is not an action in this build — kept, not applied"
                ));
                unknown.push((description, key));
                continue;
            };
            if current.is_bound_to(&key) {
                continue;
            }
            if let Err(message) = crate::keybindings::validate_binding_key(&key) {
                problems.push(format!("\"{description}\": {message}"));
                continue;
            }
            self.keybindings.set_action_key(&description, &key);
        }
        // A config file the user edited is worth a word in the transcript: the
        // panel only exists once `/keymap` is opened, and silently running on
        // the built-in keys until then is how a typo goes unnoticed for weeks.
        if !problems.is_empty() {
            let more = match problems.len() {
                1 => String::new(),
                other => format!(" (+{} more)", other - 1),
            };
            self.add_system_message(format!(
                "{} did not apply cleanly: {}{more} — run /keymap to see and fix it.",
                crate::keybindings::KEYBINDINGS_FILE,
                problems[0]
            ));
        }
        self.keybinding_problems = problems;
        self.keybinding_unknown = unknown;
    }

    /// Write the non-default bindings back to `keybindings.json`, together with
    /// any entry naming an action this build does not have (a stale file is not
    /// trimmed behind the user's back).
    fn save_keybindings(&self) -> std::io::Result<()> {
        let mut assignments = self.keybindings.assignment_overrides();
        assignments.extend(self.keybinding_unknown.iter().cloned());
        assignments.sort();
        crate::keybindings::save_keybindings_file(&self.keybindings_path, &assignments)
    }

    /// `/keymap` — bind one action to a captured key: apply it to the live
    /// dispatcher, persist it, then re-render the panel with the new state.
    fn apply_keymap_binding(&mut self, description: &str, key: &str) {
        // The panel validates before it emits; the same check runs here so no
        // programmatic caller can park a binding no terminal can send.
        if let Err(message) = crate::keybindings::validate_binding_key(key) {
            self.set_keymap_notice(message);
            return;
        }
        if !self.keybindings.set_action_key(description, key) {
            self.set_keymap_notice(format!("\"{description}\" is not an action in this build"));
            return;
        }
        // The key may have been taken. The move shadows that action (this is
        // the "bind anyway" path), so the message names it: a conflict the user
        // resolved should not need a second press to discover.
        let shadowed = self
            .keybindings
            .actions_on_key(key)
            .into_iter()
            .find(|action| action != description);
        let applied = match shadowed {
            Some(holder) => {
                crate::components::keymap_view::overridden_notice(description, key, &holder)
            }
            None => crate::components::keymap_view::bound_notice(description, key),
        };
        self.refresh_keymap_panel();
        let notice = self.with_save_outcome(applied);
        self.set_keymap_notice(notice);
    }

    /// `/keymap` — restore one action (or every action) to its built-in key.
    fn reset_keymap_binding(&mut self, description: Option<&str>) {
        let applied = match description {
            Some(description) => {
                if !self.keybindings.reset_action(description) {
                    self.set_keymap_notice(format!(
                        "\"{description}\" is not an action in this build"
                    ));
                    return;
                }
                format!("Restored «{description}» to its built-in key")
            }
            None => {
                self.keybindings.reset_all();
                "Restored every built-in key".to_string()
            }
        };
        self.refresh_keymap_panel();
        let notice = self.with_save_outcome(applied);
        self.set_keymap_notice(notice);
    }

    /// `applied`, plus what happened when it was persisted.
    ///
    /// A binding that only lives in this process is a lie the user would only
    /// discover after a restart, so a failed save is appended to the message
    /// instead of being dropped.
    fn with_save_outcome(&self, applied: String) -> String {
        match self.save_keybindings() {
            Ok(()) => applied,
            Err(err) => format!(
                "{applied} — could not write {} ({err})",
                crate::keybindings::KEYBINDINGS_FILE
            ),
        }
    }

    /// The `/sandbox` panel's callback: every value-carrying action becomes a
    /// `UiCmd` (`Moved`/`None` stay silent — they only mean "redraw").
    fn sandbox_sink(tx: mpsc::UnboundedSender<UiCmd>) -> Box<dyn FnMut(SandboxAction)> {
        Box::new(move |action: SandboxAction| {
            let cmd = match action {
                SandboxAction::TierSelected(tier) => UiCmd::SandboxTierRequested(tier),
                SandboxAction::PermissionSelected(kind) => UiCmd::PermissionLevelRequested(kind),
                SandboxAction::RefreshProbe => UiCmd::SandboxProbeRequested,
                SandboxAction::Cancelled => UiCmd::OverlayCancel,
                SandboxAction::None | SandboxAction::Moved => return,
            };
            let _ = tx.send(cmd);
        })
    }

    /// The `/skills` panel's callback.
    ///
    /// A mutation becomes [`UiCmd::SkillsMutationRequested`]: the panel's
    /// action carries the id at most, while the version an install pins and the
    /// set an upgrade confirms against are the app's own state.
    fn skills_sink(tx: mpsc::UnboundedSender<UiCmd>) -> Box<dyn FnMut(SkillsAction)> {
        Box::new(move |action: SkillsAction| {
            let cmd = match action {
                SkillsAction::Use(name) => UiCmd::SkillChosen(name),
                SkillsAction::Detail => UiCmd::SkillsDetailRequested,
                SkillsAction::Refresh => UiCmd::SkillsRefreshRequested,
                SkillsAction::Cancelled => UiCmd::OverlayCancel,
                SkillsAction::Install(id) => {
                    UiCmd::SkillsMutationRequested(SkillsMutation::Install(id))
                }
                SkillsAction::Uninstall(id) => {
                    UiCmd::SkillsMutationRequested(SkillsMutation::Uninstall(id))
                }
                SkillsAction::UpgradeAll => {
                    UiCmd::SkillsMutationRequested(SkillsMutation::UpgradeAll)
                }
                SkillsAction::None | SkillsAction::Moved | SkillsAction::TabChanged => return,
            };
            let _ = tx.send(cmd);
        })
    }

    /// `/skills` — the skill browser.
    ///
    /// The panel opens synchronously with the skills the session already
    /// reported (`get_state.skills`), so it works offline, and then pulls both
    /// sources in parallel: `get_commands` (what is installed) and
    /// `future skills list --json` (what could be). Either may fail without
    /// taking the other down — `set_skills` / `set_catalogue` merge. Rows come
    /// from the agent and the CLI, never from disk.
    fn show_skills(&mut self) {
        let mut view = SkillsView::new();
        // The TUI has no language setting of its own, and the panel already
        // falls back per field: a skill that ships a Chinese name/description
        // is shown in Chinese, one that ships none stays English. That is
        // exactly the "show Chinese when the row has it" rule, without
        // inventing a setting nobody can change.
        view.set_language(true);
        view.set_skills(local_skill_rows(&self.state.skills));
        let rows = (self.terminal.rows() as usize).max(3);
        let overlay = SkillsOverlay::new(
            view,
            rows.saturating_sub(2),
            Self::skills_sink(self.op_tx.clone()),
        );
        let width = (self.terminal.columns() as usize).saturating_sub(4).min(80);
        self.show_overlay(
            Box::new(overlay),
            OverlayOptions {
                width: Some(SizeValue::Fixed(width)),
                max_height: Some(SizeValue::Fixed(rows)),
                ..Default::default()
            },
        );
        self.load_skill_catalogue();
        self.load_cli_catalogue();
    }

    /// `get_commands` → the open `/skills` panel (a no-op when it is not on
    /// top: a stale answer must not repaint a panel the user has left).
    fn load_skill_catalogue(&mut self) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(UiCmd::SkillsLoaded(client.get_commands().await));
        });
    }

    /// `future skills list --json` → the open `/skills` panel and the version
    /// cache (`SkillsCatalogueLoaded` keeps both in step).
    ///
    /// The child is a process, so it runs on the blocking pool: the render loop
    /// never waits for it. A `future` executable this host does not have is
    /// reported here instead — that path is why the panel opens without a
    /// catalogue instead of failing.
    fn load_cli_catalogue(&mut self) {
        let Some(cli) = self.skills_cli.clone() else {
            self.set_skills_notice(SKILLS_NO_BINARY);
            return;
        };
        // The panel says it is working before the child is even spawned: the
        // answer comes back on the command channel, and until it does the
        // panel would otherwise sit unchanged (the CLI call is the slow half of
        // the two fetches this panel makes).
        self.set_skills_loading(true);
        let tx = self.op_tx.clone();
        tokio::task::spawn_blocking(move || {
            let _ = tx.send(UiCmd::SkillsCatalogueLoaded(cli.list()));
        });
    }

    /// `refresh_skills` — ask the agent to re-scan the skill directories.
    ///
    /// Both the panel's `r` and a successful install/uninstall go through here.
    /// The agent caches the skills it discovered, so the re-scan has to land
    /// *before* anything claims the new state: the answer is what re-pulls
    /// `get_commands` and the catalogue (see the `SkillsRefreshed` arm), and the
    /// desktop does the same thing for the same reason.
    fn request_skills_rescan(&mut self) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(UiCmd::SkillsRefreshed(client.refresh_skills().await));
        });
    }

    /// `/skills` — turn one panel mutation into a [`SkillOp`] and run it.
    fn handle_skills_mutation(&mut self, mutation: SkillsMutation) {
        match mutation {
            SkillsMutation::Install(id) => {
                // The panel's `Install` carries the id only; the version the
                // CLI should pin is the one the catalogue reported for it.
                let version = self.install_version_for(&id);
                self.run_skill_op(SkillOp::Install { id, version });
            }
            SkillsMutation::Uninstall(id) => self.run_skill_op(SkillOp::Uninstall { id }),
            SkillsMutation::UpgradeAll => self.confirm_or_run_upgrade_all(),
        }
    }

    /// `U` — upgrade everything, on the second press.
    ///
    /// `future skills update` rewrites every installed skill the catalogue is
    /// ahead on, so the first press only names the set (`upgrade_confirmation`)
    /// and the second one runs it. The arm is compared against the set as it is
    /// *now*: a catalogue refresh between the two presses asks for a fresh
    /// confirmation rather than upgrading something the prompt never listed.
    fn confirm_or_run_upgrade_all(&mut self) {
        let ids = self.upgradable_skill_ids();
        match self.skills_upgrade_armed.take() {
            Some(armed) if armed == ids => self.run_skill_op(SkillOp::UpdateAll),
            _ if ids.is_empty() => self.set_skills_notice(SKILLS_NO_UPGRADES),
            _ => {
                self.skills_upgrade_armed = Some(ids.clone());
                self.set_skills_notice(&upgrade_confirmation(&ids));
                self.request_render(false);
            }
        }
    }

    /// The set `U` would upgrade: the panel's own upgradable rows, which is
    /// exactly the set the key is enabled by. Empty while the panel is gone —
    /// a confirmation that outlived its panel refuses instead of guessing.
    fn upgradable_skill_ids(&mut self) -> Vec<String> {
        match self.top_skills_view() {
            Some(view) => view
                .upgradable_rows()
                .into_iter()
                .map(|row| row.id)
                .collect(),
            None => Vec::new(),
        }
    }

    /// The version `future skills install <id>` should pin: whatever the last
    /// catalogue said is the latest for that id.
    ///
    /// `None` (no catalogue answer yet, or an id it does not list) installs
    /// whatever the platform serves as current — the CLI's own default.
    fn install_version_for(&self, id: &str) -> Option<String> {
        self.skills_catalogue
            .as_ref()?
            .entries
            .iter()
            .find(|entry| entry.id == id)?
            .latest_version
            .clone()
    }

    /// Start one `future skills …` run on the blocking pool, or explain why it
    /// cannot start.
    ///
    /// [`SkillsCli::run`] spawns a child and waits for it: that must not happen
    /// on the render thread's runtime, so the whole call lives in
    /// `spawn_blocking` and only the answer comes back, as a `UiCmd` — which is
    /// also what keeps `&mut self` out of the spawned task.
    fn run_skill_op(&mut self, op: SkillOp) {
        if self.skills_op_running {
            self.set_skills_notice(SKILLS_BUSY);
            return;
        }
        let Some(cli) = self.skills_cli.clone() else {
            self.set_skills_notice(SKILLS_NO_BINARY);
            return;
        };
        self.skills_op_running = true;
        // The panel marks the row in flight (if the operation has one); a
        // failure replaces that mark with its own message once it reports back.
        if let Some(view) = self.top_skills_view() {
            view.set_error(None);
            if let Some(id) = skill_op_pending_id(&op) {
                view.set_pending(Some(&id));
            }
        }
        self.add_system_message(skill_op_start_message(&op));
        self.request_render(false);
        let tx = self.op_tx.clone();
        tokio::task::spawn_blocking(move || {
            let outcome = cli.run(&op);
            let catalogue_dirty = outcome.ok;
            let _ = tx.send(UiCmd::SkillOpDone {
                outcome,
                catalogue_dirty,
            });
        });
    }

    /// Write `text` into the panel's status row, or into the transcript when no
    /// panel is open — a mutation request can outlive the panel it came from.
    fn set_skills_notice(&mut self, text: &str) {
        match self.top_skills_view() {
            Some(view) => view.set_error(Some(text.to_string())),
            None => self.add_system_message(text.to_string()),
        }
    }

    /// `/sandbox` — open the tier + tool-permission panel.
    ///
    /// The probe answer is cached across opens (the desktop probes once per
    /// webview process); `r` inside the panel forces a fresh one. Only Linux
    /// and Windows need an RPC to learn the answer — macOS has Seatbelt by
    /// construction and an unsupported platform is definitive too.
    fn show_sandbox(&mut self) {
        self.open_sandbox_panel(Self::host_sandbox_platform());
    }

    /// The platform the sandbox panel assumes this process runs on. One place
    /// to override: `show_sandbox`, `show_permission`, the probe request and
    /// the status reconstruction all read it, so a test pinning the platform
    /// stays consistent across the whole panel lifecycle. Pinned to macOS in
    /// the test build — the platform whose copy most panel tests assert — so
    /// the assertions hold on a Linux CI host the same way they do on a
    /// developer's Mac.
    #[cfg(test)]
    fn host_sandbox_platform() -> SandboxPlatform {
        SandboxPlatform::Macos
    }

    /// The real platform outside tests.
    #[cfg(not(test))]
    fn host_sandbox_platform() -> SandboxPlatform {
        SandboxPlatform::current()
    }

    /// [`Self::show_sandbox`] for an explicit platform: the parameter is what
    /// makes the Linux/Windows probe decision reachable from a test on any
    /// host.
    fn open_sandbox_panel(&mut self, platform: SandboxPlatform) {
        self.open_sandbox_card(platform, SandboxFocus::Tiers);
    }

    /// The card `/sandbox` and `/permission` share: the approval tier and the
    /// tool-permission picker are one screen (the agent owns both halves of the
    /// policy), so the two commands differ in *where they start*, not in what
    /// they open.
    fn open_sandbox_card(&mut self, platform: SandboxPlatform, focus: SandboxFocus) {
        let status = self
            .sandbox
            .clone()
            .unwrap_or_else(|| SandboxStatus::new(platform));
        // Cache the status we are about to show: the probe answer comes back
        // later and has to update *this* platform's status, not a fresh one.
        self.sandbox = Some(status.clone());
        let mut view = SandboxView::new(platform);
        view.set_status(status.clone());
        view.set_permission_level(self.state.permission_level);
        if focus == SandboxFocus::Permissions {
            focus_permission_block(&mut view);
        }
        let rows = (self.terminal.rows() as usize).max(3);
        let overlay = SandboxOverlay::new(
            view,
            rows.saturating_sub(2),
            sandbox_tier_caveat(&status),
            Self::sandbox_sink(self.op_tx.clone()),
        );
        let width = (self.terminal.columns() as usize).saturating_sub(4).min(90);
        self.show_overlay(
            Box::new(overlay),
            OverlayOptions {
                width: Some(SizeValue::Fixed(width)),
                max_height: Some(SizeValue::Fixed(rows)),
                ..Default::default()
            },
        );
        if status.probe_state() == ProbeState::Checking {
            self.request_sandbox_probe(platform);
        }
    }

    /// `probe_sandbox` (macOS/Linux/other) or `probe_windows_sandbox`
    /// (Windows). The platform is a parameter so both arms are reachable from a
    /// test on any host.
    fn request_sandbox_probe(&mut self, platform: SandboxPlatform) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let result = match platform {
                SandboxPlatform::Windows => client.probe_windows_sandbox().await,
                SandboxPlatform::Macos | SandboxPlatform::Linux | SandboxPlatform::Other => {
                    client.probe_sandbox().await
                }
            };
            let _ = tx.send(UiCmd::SandboxProbeLoaded { result });
        });
    }

    /// `/permission [all|workspace|none]` — without an argument the panel that
    /// owns the permission picker opens; with one the level is applied.
    fn show_permission(&mut self, arg: &str) {
        let arg = arg.trim();
        if arg.is_empty() {
            // The same card `/sandbox` opens, but with the tool-permission
            // block under the highlight: two commands that open a
            // byte-identical panel read as a bug, and this is the difference
            // worth showing — nothing is hidden either way, `↑` still walks
            // into the tier list.
            self.open_sandbox_card(Self::host_sandbox_platform(), SandboxFocus::Permissions);
            return;
        }
        match PermissionKind::from_wire(arg) {
            Some(level) => self.set_permission_level(level, true),
            None => self.add_system_message(format!(
                "Usage: /permission [all|workspace|none] (got '{arg}')"
            )),
        }
    }

    /// `set_permission_level`, optionally persisting the choice as the TUI's
    /// startup default.
    fn set_permission_level(&mut self, level: PermissionKind, persist: bool) {
        if persist {
            self.tui_settings.default_permission_level = Some(level.to_wire().to_string());
            self.save_tui_settings();
        }
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let result = client.set_permission_level(level.to_wire()).await;
            let _ = tx.send(UiCmd::PermissionLevelSet { level, result });
        });
    }

    /// Push a new permission level into the open panel (a no-op otherwise).
    fn sync_sandbox_panel(&mut self) {
        let level = self.state.permission_level;
        let status = self.sandbox.clone();
        if let Some(overlay) = self.top_sandbox_overlay() {
            overlay.view_mut().set_permission_level(level);
            if let Some(status) = status {
                overlay.set_status(status);
            }
        }
    }

    /// The `/sandbox` panel of the top overlay, when that is what is open.
    fn top_sandbox_overlay(&mut self) -> Option<&mut SandboxOverlay> {
        let idx = self.get_top_overlay_index()?;
        self.overlay_stack[idx]
            .component
            .as_any_mut()
            .downcast_mut::<SandboxOverlay>()
    }

    /// `/tools` — multi-select over the built-in tool set.
    fn show_tools_menu(&mut self) {
        let selected: Vec<String> = self.enabled_tools.clone().unwrap_or_else(|| {
            BUILTIN_TOOLS
                .iter()
                .map(|name| (*name).to_string())
                .collect()
        });
        let items: Vec<MenuItem> = BUILTIN_TOOLS
            .iter()
            .map(|name| {
                let item = MenuItem::new(*name, *name);
                if selected.iter().any(|chosen| chosen == name) {
                    item.selected()
                } else {
                    item
                }
            })
            .collect();
        let options = MenuOptions::new("Tools", vec![MenuSection::flat(items)])
            .searchable(false)
            .multi_select(true)
            .max_visible(8)
            .with_footer_hints(vec![
                ("↑↓".to_string(), "navigate".to_string()),
                ("space".to_string(), "toggle".to_string()),
                ("enter".to_string(), "apply".to_string()),
                ("esc".to_string(), "close".to_string()),
            ]);
        let tx = self.op_tx.clone();
        self.show_menu_overlay(
            MenuState::new(options),
            70,
            Self::menu_sink(tx, MenuPurpose::Tools),
        );
    }

    // ─── /providers ───────────────────────────────────────────────────

    fn show_providers(&mut self) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(UiCmd::ProvidersLoaded(client.list_providers().await));
        });
    }

    /// Build the `/providers` list overlay.
    fn show_providers_overlay(&mut self, providers: Vec<ProviderInfo>) {
        let mut list = ProviderListState::new(providers);
        if let Some(notice) = self.provider_notice.clone() {
            list.set_notice(Some(&notice));
        }
        // Palette applied by `show_overlay`.
        let overlay = ProviderListOverlay::new(list, provider_list_sink(self.op_tx.clone()));
        let width = (self.terminal.columns() as usize).saturating_sub(4).min(80);
        self.show_overlay(
            Box::new(overlay),
            OverlayOptions {
                width: Some(SizeValue::Fixed(width)),
                ..Default::default()
            },
        );
    }

    /// Replace the provider list behind an open `/providers` overlay (after a
    /// mutation or a refresh) — a no-op when that overlay is not on top.
    fn refresh_provider_list(&mut self, providers: Vec<ProviderInfo>) {
        let notice = self.provider_notice.clone();
        let model = self.state.model.clone();
        if let Some(idx) = self.get_top_overlay_index() {
            if let Some(overlay) = self.overlay_stack[idx]
                .component
                .as_any_mut()
                .downcast_mut::<ProviderListOverlay>()
            {
                overlay.state_mut().set_providers(providers);
                overlay.state_mut().set_default_model(&model);
                overlay.state_mut().set_notice(notice.as_deref());
            }
        }
    }

    /// The provider list of the top overlay, when that is what is open.
    fn top_provider_list(&mut self) -> Option<&mut ProviderListState> {
        let idx = self.get_top_overlay_index()?;
        self.overlay_stack[idx]
            .component
            .as_any_mut()
            .downcast_mut::<ProviderListOverlay>()
            .map(|overlay| overlay.state_mut())
    }

    /// The `/skills` browser of the top overlay, when that is what is open.
    /// Every skill RPC answers into this, so a stale answer cannot repaint a
    /// panel the user has already left.
    fn top_skills_view(&mut self) -> Option<&mut SkillsView> {
        self.top_skills_overlay().map(|overlay| overlay.view_mut())
    }

    /// The open `/skills` panel itself, when it is on top: the wrapper owns the
    /// loading row and the panel's row budget, which its view cannot see.
    fn top_skills_overlay(&mut self) -> Option<&mut SkillsOverlay> {
        let idx = self.get_top_overlay_index()?;
        self.overlay_stack[idx]
            .component
            .as_any_mut()
            .downcast_mut::<SkillsOverlay>()
    }

    /// Mark the open `/skills` panel as waiting for (or done with) the
    /// installable-skills catalogue.
    fn set_skills_loading(&mut self, loading: bool) {
        if let Some(overlay) = self.top_skills_overlay() {
            overlay.set_loading(loading);
        }
    }

    /// Replace the cached sandbox status and push it into the open panel.
    ///
    /// The panel is updated through `set_status`, which also re-clamps a
    /// highlight left on a tier the new status disables — feeding the view
    /// directly would leave the cursor on a disabled row.
    fn update_sandbox_status<F: FnOnce(&mut SandboxStatus)>(&mut self, update: F) {
        let mut status = self
            .sandbox
            .clone()
            .unwrap_or_else(|| SandboxStatus::new(Self::host_sandbox_platform()));
        update(&mut status);
        self.sandbox = Some(status.clone());
        if let Some(overlay) = self.top_sandbox_overlay() {
            overlay.set_status(status);
        }
        self.request_render(false);
    }

    /// Pop the add/edit form when it is the top overlay (after a successful
    /// mutation, so the refresh opens the list again instead of stacking).
    fn close_provider_form(&mut self) {
        let is_form = self.get_top_overlay_index().is_some_and(|idx| {
            self.overlay_stack[idx]
                .component
                .as_any()
                .is::<ProviderFormOverlay>()
        });
        if is_form {
            self.hide_overlay();
        }
    }

    /// Surface a rejected mutation on the open form (a no-op otherwise).
    fn set_provider_form_error(&mut self, message: &str) {
        if let Some(idx) = self.get_top_overlay_index() {
            if let Some(form) = self.overlay_stack[idx]
                .component
                .as_any_mut()
                .downcast_mut::<ProviderFormOverlay>()
            {
                form.form_mut().error = Some(message.to_string());
            }
        }
    }

    /// Open the add/edit provider form, replacing the list underneath it.
    fn show_provider_form(&mut self, form: ProviderForm) {
        let overlay = ProviderFormOverlay::new(form, provider_form_sink(self.op_tx.clone()));
        let width = (self.terminal.columns() as usize).saturating_sub(4).min(84);
        self.hide_overlay();
        self.show_overlay(
            Box::new(overlay),
            OverlayOptions {
                width: Some(SizeValue::Fixed(width)),
                ..Default::default()
            },
        );
    }

    /// Spawn `upsert_provider` and report the outcome.
    /// `/skill-recommend [on|off]` — the TUI's own toggle, the counterpart of
    /// the desktop Settings switch. Bare form reports the current state.
    fn set_skill_recommend(&mut self, arg: &str) {
        match arg.trim().to_lowercase().as_str() {
            "on" | "true" => {
                self.tui_settings.skill_recommend = Some(true);
                self.save_tui_settings();
                self.add_system_message(
                    "Skill recommendation on: a fitting skill may be offered before a message is sent."
                        .to_string(),
                );
            }
            "off" | "false" => {
                self.tui_settings.skill_recommend = Some(false);
                self.save_tui_settings();
                // Anything held belongs to the feature being switched off.
                self.skill_reco = SkillRecoState::Idle;
                self.add_system_message("Skill recommendation off.".to_string());
            }
            "" => {
                let state = if self.tui_settings.skill_recommend_enabled() {
                    "on"
                } else {
                    "off"
                };
                self.add_system_message(format!(
                    "Skill recommendation is {state}. Use /skill-recommend on|off."
                ));
            }
            other => {
                self.add_system_message(format!(
                    "Unknown /skill-recommend argument '{other}' — use on or off."
                ));
            }
        }
        self.request_render(false);
    }

    fn submit_provider(&mut self, input: ProviderInput) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let result = client.upsert_provider(&input).await;
            let _ = tx.send(UiCmd::ProviderActionDone {
                action: format!("provider {} saved", input.id),
                result,
            });
        });
    }

    /// Spawn `delete_provider` and report the outcome.
    fn delete_provider(&mut self, id: &str) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let result = client.delete_provider(&id).await;
            let _ = tx.send(UiCmd::ProviderActionDone {
                action: format!("provider {id} deleted"),
                result,
            });
        });
    }

    /// Spawn `sync_future_models` / `reload_auth` and report the outcome.
    fn provider_sync(&mut self, action: &'static str) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let result = match action {
                "reload_auth" => client.reload_auth().await,
                _ => client.sync_future_models().await.map(|_| ()),
            };
            let _ = tx.send(UiCmd::ProviderActionDone {
                action: action.replace('_', " "),
                result,
            });
        });
    }

    /// `/provider-key <id>` — capture the next submission as that provider's
    /// key (`set_auth`). An empty submission clears the stored key.
    fn prompt_provider_key(&mut self, provider: &str) {
        self.pending_secret = Some(provider.to_string());
        self.add_system_message(format!(
            "Enter the API key for {provider} and press enter (empty input clears the stored key; /cancel-input aborts)."
        ));
        self.request_render(false);
    }

    // ─── /usage, /transcript, /copy, /editor ──────────────────────────

    /// Ask the agent for the state snapshot `/usage` renders.
    fn show_usage(&mut self) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(UiCmd::UsageLoaded(client.get_state().await.map(Box::new)));
        });
    }

    /// `/agent` — the agent's identity panel (`get_agent_info`).
    fn show_agent_info(&mut self) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(UiCmd::AgentInfoLoaded(client.get_agent_info().await));
        });
    }

    /// `/history <query>` — search the persisted session history.
    fn search_history(&mut self, query: &str) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        let query = query.to_string();
        tokio::spawn(async move {
            let result = client
                .search_session_history(&query, HISTORY_SEARCH_LIMIT)
                .await;
            let _ = tx.send(UiCmd::HistorySearched { query, result });
        });
    }

    /// `/tool-output` — the stored tool calls of the current (or last) run.
    fn show_tool_calls(&mut self) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(UiCmd::ToolCallsLoaded(client.list_tool_calls().await));
        });
    }

    /// `/tool-output <call-id>` — one call's full stored output.
    fn show_tool_output(&mut self, tool_call_id: &str) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        let tool_call_id = tool_call_id.to_string();
        tokio::spawn(async move {
            let result = client.get_tool_output(&tool_call_id).await;
            let _ = tx.send(UiCmd::ToolOutputLoaded {
                tool_call_id,
                result,
            });
        });
    }

    /// `/autocompact [on|off]`.
    fn set_auto_compaction(&mut self, enabled: bool) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let result = client.set_auto_compaction(enabled).await;
            let _ = tx.send(UiCmd::AutoCompactionSet { enabled, result });
        });
    }

    /// `/autoretry [on|off]`.
    fn set_auto_retry(&mut self, enabled: bool) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let result = client.set_auto_retry(enabled).await;
            let _ = tx.send(UiCmd::AutoRetrySet { enabled, result });
        });
    }

    /// `/export` — write the session to an HTML file (`export_html`).
    fn export_session(&mut self) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(UiCmd::SessionExported(client.export_html().await));
        });
    }

    /// The optional `on|off` argument of `/autocompact` and `/autoretry`:
    /// `Ok(None)` means "no argument given" so the caller toggles instead.
    fn parse_toggle_arg(arg: &str) -> Result<Option<bool>, String> {
        match arg.trim().to_ascii_lowercase().as_str() {
            "" => Ok(None),
            "on" | "true" | "enable" | "enabled" => Ok(Some(true)),
            "off" | "false" | "disable" | "disabled" => Ok(Some(false)),
            other => Err(other.to_string()),
        }
    }

    /// The raw argument of `/shell`: everything after the command word.
    ///
    /// The dispatcher's generic argument split (`split_ws_js(...).join(" ")`)
    /// collapses runs of whitespace, which would silently rewrite a command
    /// (`printf '%s  %s'` → `printf '%s %s'`) before the agent ever ran it — so
    /// `/shell` takes the rest of the line verbatim. `None` when there is
    /// nothing after the command word.
    fn shell_argument(value: &str) -> Option<&str> {
        value
            .split_once(char::is_whitespace)
            .map(|(_, rest)| rest.trim_start())
            .filter(|rest| !rest.is_empty())
    }

    // ─── One-shot agent actions (/shell, /title, /metrics, …) ──────────

    /// `/shell <cmd>` — run one command through the agent (`shell`). The agent
    /// runs it in the session cwd with the same shell contract as its shell
    /// tool and returns `{output, exitCode}`; there is no PTY, so this is a
    /// one-shot capture rather than the desktop's terminal panel.
    fn run_shell(&mut self, command: &str) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        let command = command.to_string();
        tokio::spawn(async move {
            // 0 = the agent's own default timeout (120 s).
            let result = client.shell(&command, 0).await;
            let _ = tx.send(UiCmd::DiagnosticsLoaded {
                kind: DiagnosticsKind::Shell,
                result,
            });
        });
    }

    /// `/metrics` — the session's live runtime counters.
    fn load_metrics(&mut self) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(UiCmd::DiagnosticsLoaded {
                kind: DiagnosticsKind::Metrics,
                result: client.get_runtime_metrics().await,
            });
        });
    }

    /// `/snapshot` — the current (or last) run's projection snapshot. The
    /// run-scoped reads need a run id the client learned from the event
    /// stream, so this fails client-side on a session that has not run yet.
    fn load_run_snapshot(&mut self) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let result = match client.snapshot_run_id() {
                Ok(run_id) => client.get_run_snapshot(&run_id).await,
                Err(error) => Err(error),
            };
            let _ = tx.send(UiCmd::DiagnosticsLoaded {
                kind: DiagnosticsKind::Snapshot,
                result,
            });
        });
    }

    /// The locale `/title` asks the agent for: an explicit argument wins and
    /// the host locale is only the default (the agent rejects anything else,
    /// so an unknown argument is the caller's usage error).
    fn title_mode(arg: &str) -> Result<&'static str, String> {
        match arg.trim().to_ascii_lowercase().as_str() {
            "zh" => Ok("zh"),
            "en" => Ok("en"),
            "" => Ok(if prefers_chinese() { "zh" } else { "en" }),
            other => Err(other.to_string()),
        }
    }

    /// `/title [zh|en]` — ask the model for a session title and apply it.
    ///
    /// The suggestion is never persisted agent-side: this TUI applies it with
    /// `set_session_name`, exactly like the desktop's rename flow.
    fn generate_session_title(&mut self, arg: &str) {
        let mode = match Self::title_mode(arg) {
            Ok(mode) => mode,
            Err(bad) => {
                self.add_system_message(format!("Usage: /title [zh|en] (got '{bad}')"));
                return;
            }
        };
        self.add_system_message("Generating a session title…".to_string());
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(UiCmd::SessionTitleGenerated(
                client.generate_session_title(mode).await,
            ));
        });
    }

    /// Apply a generated title (`set_session_name`). A blank suggestion is
    /// refused here instead of being written as an empty name.
    fn apply_session_title(&mut self, title: &str) {
        let title = title.trim();
        if title.is_empty() {
            self.add_system_message(
                "The agent returned an empty title; nothing was changed.".into(),
            );
            return;
        }
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        let title = title.to_string();
        tokio::spawn(async move {
            let result = client.set_session_name(&title).await;
            let _ = tx.send(UiCmd::SessionTitleApplied { title, result });
        });
    }

    /// `/context [on|off]` — list the workspace context files, or turn loading
    /// them on/off.
    ///
    /// `set_context_files` only writes: the agent has no command that reads the
    /// switch back (`get_state.contextFiles` is empty both when nothing is
    /// configured and when loading is off), so the listing states what it
    /// actually knows rather than guessing.
    fn context_files(&mut self, arg: &str) {
        match Self::parse_toggle_arg(arg) {
            Ok(None) => {
                let mut lines = vec!["Context files".to_string(), String::new()];
                if self.state.context_files.is_empty() {
                    lines.push("The agent reports no context files for this workspace.".into());
                    lines.push("They may be absent, or loading may be off.".into());
                } else {
                    lines.extend(
                        self.state
                            .context_files
                            .iter()
                            .map(|path| format!("- {path}")),
                    );
                }
                lines.push(String::new());
                lines.push("Turn loading on or off with /context on|off.".into());
                self.show_pager_text(lines, "No context files are loaded from this workspace.");
            }
            Ok(Some(enabled)) => {
                let client = self.client.clone();
                let tx = self.op_tx.clone();
                tokio::spawn(async move {
                    let result = client.set_context_files(enabled).await;
                    let _ = tx.send(UiCmd::ContextFilesSet { enabled, result });
                });
            }
            Err(bad) => {
                self.add_system_message(format!("Usage: /context [on|off] (got '{bad}')"));
            }
        }
    }

    /// `/delete [--yes]` — delete the current session. Unconfirmed calls only
    /// explain what the confirmation would do.
    fn delete_current_session(&mut self, arg: &str) {
        let session_id = self.state.session_id.clone();
        if arg.trim() != "--yes" {
            let which = if session_id.is_empty() {
                "(no session yet)"
            } else {
                session_id.as_str()
            };
            self.add_system_message(format!(
                "Deleting session {which} cannot be undone. Re-run as /delete --yes to confirm."
            ));
            return;
        }
        if session_id.is_empty() {
            self.add_system_message("No session to delete.".into());
            return;
        }
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let result = client.delete_session(&session_id).await;
            let _ = tx.send(UiCmd::SessionDeleted { session_id, result });
        });
    }

    /// A JSON scalar as one line of text: strings verbatim, everything else
    /// rendered compactly, `null` spelled out (a blank cell would read as
    /// "missing" rather than "reported as nothing").
    fn scalar_text(value: &Value) -> String {
        match value {
            Value::String(text) => text.clone(),
            Value::Null => "(none)".to_string(),
            other => other.to_string(),
        }
    }

    /// `/metrics` — the agent's counters, in the order the agent reported them.
    /// A non-object payload is rendered as-is instead of being flattened into a
    /// fabricated table.
    ///
    /// The title is plain text: the pager paints its content verbatim where
    /// the chat renders markdown, so a literal `**` reaches the screen as two
    /// asterisks.
    fn metrics_lines(value: &Value) -> Vec<String> {
        let mut lines = vec!["Runtime metrics".to_string(), String::new()];
        match value.as_object() {
            Some(map) if map.is_empty() => {
                lines.push("The agent reported no counters.".to_string());
            }
            Some(map) => {
                for (key, value) in map {
                    lines.push(format!("{key}: {}", Self::scalar_text(value)));
                }
            }
            None => lines.push(Self::scalar_text(value)),
        }
        lines
    }

    /// `/snapshot` — the run projection snapshot: the envelope's facts plus one
    /// row per projected event. Plain-text title, like [`Self::metrics_lines`].
    fn snapshot_lines(value: &Value) -> Vec<String> {
        let mut lines = vec!["Run snapshot".to_string(), String::new()];
        for key in ["runSnapshot", "watermark", "nextSinceIdx", "hasMore"] {
            if let Some(field) = value.get(key) {
                lines.push(format!("{key}: {}", Self::scalar_text(field)));
            }
        }
        let projection = value.get("projection");
        for (label, key) in [("projection run", "runId"), ("projection cursor", "cursor")] {
            if let Some(field) = projection.and_then(|projection| projection.get(key)) {
                lines.push(format!("{label}: {}", Self::scalar_text(field)));
            }
        }
        match projection
            .and_then(|projection| projection.get("events"))
            .and_then(Value::as_array)
        {
            Some(events) => {
                lines.push(String::new());
                lines.push(format!("events: {}", events.len()));
                for event in events {
                    let idx = event
                        .get("idx")
                        .map(Self::scalar_text)
                        .unwrap_or_else(|| "-".to_string());
                    let kind = event
                        .get("type")
                        .map(Self::scalar_text)
                        .unwrap_or_else(|| "-".to_string());
                    lines.push(format!("#{idx} {kind}"));
                }
            }
            None => lines.push("events: (not reported)".to_string()),
        }
        lines
    }

    /// `/shell` — the captured output and the exit code the agent reported.
    fn shell_lines(value: &Value) -> Vec<String> {
        let mut lines: Vec<String> = value
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect();
        lines.push(String::new());
        lines.push(match value.get("exitCode").and_then(Value::as_i64) {
            Some(code) => format!("exit code: {code}"),
            None => "exit code: (not reported)".to_string(),
        });
        lines
    }

    /// The renderer for a [`DiagnosticsKind`] payload.
    fn diagnostics_lines(kind: DiagnosticsKind, value: &Value) -> Vec<String> {
        match kind {
            DiagnosticsKind::Metrics => Self::metrics_lines(value),
            DiagnosticsKind::Snapshot => Self::snapshot_lines(value),
            DiagnosticsKind::Shell => Self::shell_lines(value),
        }
    }

    /// The whole transcript in a searchable pager overlay.
    fn show_transcript(&mut self) {
        let width = (self.terminal.columns() as usize).max(1);
        let mut lines = self.chat.render_all(width);
        while lines.last().is_some_and(|line| line.trim().is_empty()) {
            lines.pop();
        }
        self.show_pager_text(lines, "Nothing to show yet.");
    }

    /// Show pre-rendered `lines` in the searchable pager overlay (`/` search,
    /// `n`/`N` jumps, `y` copy). No content at all becomes a system message
    /// rather than an empty pager.
    fn show_pager_text(&mut self, lines: Vec<String>, empty_message: &str) {
        if lines.is_empty() {
            self.add_system_message(empty_message.to_string());
            return;
        }
        let width = (self.terminal.columns() as usize).max(1);
        let rows = (self.terminal.rows() as usize).max(3);
        let mut pager = Pager::new();
        pager.set_content(lines);
        let tx = self.op_tx.clone();
        // The pager is a full-screen view: it asks for exactly `rows` rows —
        // the terminal's whole height, because `Pager::render` spends one of
        // them on its status bar. A short budget (`rows - 2`) used to leave the
        // compositor to centre it, which landed the body on screen row 1 and
        // left row 0 showing the chat's first line, so `/transcript` printed
        // `New session started.` twice and every pager panel appeared to start
        // under a stray chat line. Asking for the full height makes the centred
        // layout resolve to row 0, i.e. the pager's first row is screen row 1.
        // Every line it renders is painted across the full width (see
        // `App::composite_line_at`), so no chat text survives beside it.
        let overlay = PagerOverlay::new(
            pager,
            rows,
            Box::new(move |action: PagerAction| match action {
                PagerAction::Copied(text) => {
                    let _ = tx.send(UiCmd::CopyRequest(text));
                }
                PagerAction::Closed => {
                    let _ = tx.send(UiCmd::OverlayCancel);
                }
                PagerAction::None | PagerAction::Moved | PagerAction::SearchChanged => {}
            }),
        );
        self.show_overlay(
            Box::new(overlay),
            OverlayOptions {
                width: Some(SizeValue::Fixed(width)),
                max_height: Some(SizeValue::Fixed(rows)),
                ..Default::default()
            },
        );
    }

    /// Copy `text` through the clipboard backend, reporting the outcome.
    fn copy_to_clipboard(&mut self, text: &str) {
        let in_tmux = crate::clipboard::is_tmux_from_env(std::env::var("TMUX").ok().as_deref());
        // The app only runs with a TTY (`Terminal::start` refuses otherwise).
        let outcome = self.clipboard.copy(text, in_tmux, true);
        match outcome {
            crate::clipboard::CopyOutcome::Copied => {
                self.add_system_message(format!("Copied {} characters.", text.chars().count()))
            }
            crate::clipboard::CopyOutcome::Requested => {
                if let Some(sequence) = self.clipboard.osc52_sequence(text) {
                    self.terminal.write(&sequence);
                }
                self.add_system_message(
                    "Sent an OSC 52 copy request — the terminal has to apply it (unconfirmed)."
                        .into(),
                );
            }
            crate::clipboard::CopyOutcome::Failed(err) => {
                self.add_system_message(format!("Copy failed: {err}"))
            }
        }
        self.request_render(false);
    }

    /// `/copy` and `ctrl+x` — copy the most recent assistant message.
    fn copy_last_assistant_message(&mut self) {
        match self.chat.last_assistant_text() {
            Some(text) if !text.trim().is_empty() => self.copy_to_clipboard(&text),
            _ => {
                self.add_system_message("Nothing to copy yet.".into());
                self.request_render(false);
            }
        }
    }

    /// `ctrl+v` — paste from the system clipboard.
    ///
    /// An image on the clipboard is written to a temp file by the platform's
    /// clipboard tool and attached through the same path a pasted image *path*
    /// takes (`Input::insert_text` → an `[Image #N]` marker plus the
    /// attachment), so numbering, deletion and submission are the P1 rules and
    /// not a second implementation of them. The file is deliberately **not**
    /// deleted at submit: the agent reads the image by path after the prompt is
    /// sent, and captured files are swept by age instead
    /// (`paste::sweep_stale_clipboard_files`).
    ///
    /// A clipboard that holds no image — the common case — falls back to its
    /// text, which `insert_text` treats like any other paste (folding a long
    /// one, attaching a path that names an image). Only a clipboard with
    /// neither, or a clipboard tool that cannot be started, produces a notice.
    fn paste_clipboard(&mut self) {
        self.paste_clipboard_for_os(std::env::consts::OS);
    }

    /// [`Self::paste_clipboard`] against an explicit OS: the parameter lets a
    /// test pin the platform (and therefore the probe command names) regardless
    /// of the host it runs on.
    fn paste_clipboard_for_os(&mut self, os: &str) {
        // Paste writes into the editor, so it is blocked wherever typing is.
        if self.editor_locked_by_reco() {
            return;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0);
        let outcome = self
            .clipboard_capture
            .capture(os, std::process::id(), now_ms);
        match outcome {
            crate::paste::ClipboardPaste::Image { path, .. } => self.input.insert_text(&path),
            crate::paste::ClipboardPaste::Text(text) => self.input.insert_text(&text),
            crate::paste::ClipboardPaste::Unavailable(reason) => {
                self.add_system_message(format!("Clipboard paste failed: {reason}"))
            }
        }
        self.request_render(false);
    }

    /// Suspend the terminal, run `$VISUAL`/`$EDITOR` on the current draft, then
    /// resume and read the result back. `spawn` performs the (blocking) launch
    /// and is injected so tests never start a process.
    fn edit_draft_with(
        &mut self,
        spawn: &mut dyn FnMut(
            &mut std::process::Command,
        ) -> Result<(), crate::external_editor::EditorError>,
    ) -> Result<String, crate::external_editor::EditorError> {
        use crate::external_editor::{
            build_command, resolve_editor_command, strip_trailing_newline, EditorDraft,
        };
        let parts = resolve_editor_command(
            std::env::var("VISUAL").ok().as_deref(),
            std::env::var("EDITOR").ok().as_deref(),
        )?;
        let dir = self
            .tui_settings_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("editor");
        let draft = EditorDraft::new(&dir, self.input.get_value())?;
        let mut command = build_command(&parts, draft.path());
        self.suspend_terminal();
        let spawned = spawn(&mut command);
        self.resume_terminal();
        if let Err(err) = spawned {
            let _ = draft.cleanup();
            return Err(err);
        }
        let text = draft.contents()?;
        let _ = draft.cleanup();
        Ok(strip_trailing_newline(&text))
    }

    /// `/editor` — open the draft in the external editor.
    fn open_external_editor(&mut self) {
        use crate::external_editor::EditorError;
        let result = self.edit_draft_with(&mut |command: &mut std::process::Command| {
            command
                .status()
                .map(|_| ())
                .map_err(|err| EditorError::Io(err.to_string()))
        });
        match result {
            Ok(text) => {
                self.input.set_value(&text, None);
                self.add_system_message("Draft updated from the external editor.".into());
            }
            Err(EditorError::MissingEditor | EditorError::EmptyCommand) => self.add_system_message(
                "Set $VISUAL or $EDITOR to use /editor (e.g. EDITOR=vim future tui).".into(),
            ),
            Err(err) => self.add_system_message(format!("Editor failed: {err}")),
        }
        self.request_render(true);
    }

    /// Leave the alternate screen + raw mode so a child process can own the
    /// terminal (external editor) — or, for the scrollback insert, so a write
    /// lands on the normal screen the terminal keeps.
    fn suspend_terminal(&mut self) {
        self.terminal.stop();
    }

    /// Re-enter the terminal after [`Self::suspend_terminal`], reusing the
    /// `input_tx` handed to `start`. Deliberately does not request a render: the
    /// scrollback insert repaints from inside `do_render`, which is already
    /// running and would otherwise flash an empty alternate screen until the
    /// next tick.
    fn reenter_alternate_screen(&mut self) {
        if let Some(input_tx) = self.input_tx.clone() {
            let tx = input_tx.clone();
            let tx2 = input_tx;
            let _ = self.terminal.start(
                Box::new(move |data: String| {
                    let _ = tx.send(UiInput::Input(data));
                }),
                Box::new(move || {
                    let _ = tx2.send(UiInput::Resize);
                }),
            );
        }
        self.terminal.hide_cursor();
    }

    /// Re-enter the terminal after [`Self::suspend_terminal`], always forcing a
    /// full redraw: the screen was handed to another program.
    fn resume_terminal(&mut self) {
        self.reenter_alternate_screen();
        self.request_render(true);
    }

    /// Forget the diff baseline so the next render clears the screen and repaints
    /// every row — what any path that hands the screen away and takes it back has
    /// to do (`request_render(true)` does it through here too).
    fn reset_diff_baseline(&mut self) {
        self.previous_lines.clear();
        self.previous_width = usize::MAX; // triggers widthChanged in doRender
        self.previous_height = usize::MAX;
        self.cursor_row = 0;
        self.hardware_cursor_row = 0;
        self.max_lines_rendered = 0;
        self.previous_viewport_top = 0;
    }

    // ─── Scrollback (see `crate::insert_history`) ──────────────────────

    /// Move the finished tail of the transcript into the terminal's scrollback.
    ///
    /// `switch_screens` leaves the alternate screen for the duration of the write
    /// (`true` while the TUI is running); the exit path passes `false`, because
    /// `Terminal::stop` has already left it and there is nothing to come back to.
    /// Returns the number of screen rows written.
    ///
    /// Runs the whole flow in one synchronous step: the alternate screen is gone
    /// between `leave` and `enter`, so nothing may yield in between.
    fn flush_scrollback(&mut self, switch_screens: bool) -> usize {
        let width = self.terminal.columns() as usize;
        let rows = self.chat.render_all(width);
        let pending = self.history.pending(&rows);
        if pending.is_empty() {
            return 0;
        }
        let lead_line = self.history.first_insert();
        let written = if switch_screens {
            insert_history(self, pending, width, lead_line)
        } else {
            write_history(self, pending, width, lead_line)
        };
        // The watermark records what was handed out even when the batch had no
        // visible row, so a blank transcript is not re-considered every render.
        self.history.commit(pending, written);
        written
    }

    /// Emit the configured `BEL`/`OSC 9`/title sequences for one event.
    ///
    /// The app only ever runs on a TTY (`Terminal::start` refuses otherwise),
    /// so the non-TTY gate is covered by the `notifications` unit tests.
    fn notify_event(&mut self, kind: NotifyKind, body: &str) {
        let config = self.tui_settings.notify_config();
        let event = NotifyEvent::new(kind, kind.default_title(), body);
        for sequence in sequences_for(&config, &event, true) {
            self.terminal.write(&sequence);
        }
    }

    /// Keep the window title in step with the run state (`[>] project | model`
    /// while a run is streaming).
    fn update_terminal_title(&mut self) {
        let config = self.tui_settings.notify_config();
        if !config.enabled || !config.title {
            return;
        }
        let title = terminal_title(
            &self.state.cwd,
            &self.state.model,
            self.state.session_name.as_deref(),
            self.state.streaming,
        );
        self.terminal.write(&set_title_sequence(&title));
    }

    /// Switch-session flow (sessions/tree overlays): switch → refresh →
    /// load history → message. The overlay hides when the flow completes.
    fn spawn_switch_flow(&mut self, session_id: &str, label: String) {
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        let sid = session_id.to_string();
        // Record the request *before* the RPC: from here on an older request
        // still in flight is superseded, and its result must not be applied.
        self.latest_session_switch = Some(SessionSwitchRequest {
            from: self.state.session_id.clone(),
            target: sid.clone(),
        });
        tokio::spawn(async move {
            let outcome = client.switch_session(&sid).await;
            // `switch_session` binds the client to the target unless the agent
            // declines (`cancelled`); the client is the authority on what a
            // later prompt will address.
            let switched = outcome.as_ref().is_ok_and(|v| {
                !v.get("cancelled").and_then(Value::as_bool).unwrap_or(false)
                    && client.get_current_session_id() == sid
            });
            let result = outcome.map(|_| ());
            let mut state = None;
            let mut history = Ok(Value::Null);
            if switched {
                state = client.get_state().await.ok();
                history = load_history_tail(&client, &sid).await;
            }
            let _ = tx.send(UiCmd::SessionSwitched {
                target: sid,
                switched,
                result,
                state,
                history,
                label,
            });
        });
    }

    /// Drop the transcript and its paging/scrollback anchors together.
    ///
    /// A transcript that is replaced wholesale has three pieces of state that
    /// only mean something relative to it: the older-history cursor, the
    /// scrollback watermark's alignment, and the chat itself. Clearing them in
    /// one place is what keeps a stale cursor from being applied to the next
    /// session — or from paging `get_session_entries` for a session nobody is
    /// looking at.
    fn clear_transcript(&mut self, session_id: &str) {
        self.chat.clear_messages();
        self.history.rebase();
        self.history_paging.reset(session_id);
    }

    /// A scroll reached the top of the transcript: fetch the next older page.
    ///
    /// Driven by the upward scroll *keys* rather than by the scroll result: a
    /// tail page shorter than the viewport cannot scroll at all (`scroll_up`
    /// reports `false` because it is already at the top), and that is exactly
    /// when a user has nothing else to press. Only the app's own scroll keys
    /// come through here, so this never fires while reading the middle of the
    /// transcript.
    fn maybe_load_older_history(&mut self) {
        if !self.chat.is_at_top() || !self.history_paging.has_more || self.history_paging.loading {
            return;
        }
        let session_id = self.state.session_id.clone();
        if self.history_paging.session_id != session_id {
            return;
        }
        let before = self.history_paging.next_before;
        self.history_paging.loading = true;
        let client = self.client.clone();
        let tx = self.op_tx.clone();
        tokio::spawn(async move {
            let result = client
                .session_older_page(&session_id, before)
                .await
                .map(|page| page.to_value());
            let _ = tx.send(UiCmd::HistoryPageLoaded {
                session_id,
                before,
                result,
            });
        });
    }

    fn apply_status(&mut self, s: &RpcSessionState, models: &[ModelInfo], stats: Option<&Value>) {
        self.remember_model_image_support(models);
        let current_model = models
            .iter()
            .find(|m| m.id == s.model.as_deref().unwrap_or(""));
        let model_info: Vec<String> = match current_model {
            Some(m) => vec![
                format!("Model: {} (`{}`)", m.label, m.id),
                format!("Provider: {}", m.provider),
                format!(
                    "Image support: {}",
                    if m.supports_images { "yes" } else { "no" }
                ),
                format!(
                    "Context window: {}K",
                    (m.context_window as f64 / 1000.0).round() as u64
                ),
            ],
            None => vec![format!(
                "Model: {}",
                s.model.as_deref().unwrap_or("(unknown)")
            )],
        };
        // Plain labels, no `**`: this is a system message, and the chat renders
        // system messages verbatim (only user/assistant text goes through the
        // markdown renderer), so the asterisks used to reach the screen.
        let mut lines = vec![
            model_info.join("\n"),
            String::new(),
            format!(
                "Session: {}",
                if s.session_id.is_empty() {
                    "(none)".to_string()
                } else {
                    s.session_id.clone()
                }
            ),
            format!("CWD: {}", s.cwd.as_deref().unwrap_or("(none)")),
            format!("Thinking: {}", s.thinking_level),
            format!(
                "Permission: {}",
                s.permission_level.as_deref().unwrap_or("all")
            ),
            format!("Queries: {}", s.query_count),
            format!(
                "Auto compaction: {}",
                if s.auto_compaction_enabled {
                    "on"
                } else {
                    "off"
                }
            ),
            format!("Streaming: {}", if s.is_streaming { "yes" } else { "no" }),
            String::new(),
            format!(
                "Context: {} / {} ({:.1}%)",
                s.context_tokens.unwrap_or(0),
                s.context_window.unwrap_or(0),
                s.context_percent.unwrap_or(0.0)
            ),
        ];
        // `/stats` used to be a second panel for the same counters. It is gone;
        // the ledger is a section of this one. When `get_session_stats` fails the
        // rest of the report is still worth printing, so the failure is named in
        // place rather than swallowing the whole command.
        match stats {
            Some(stats) => {
                lines.push(String::new());
                lines.extend(session_ledger_lines(stats));
            }
            None => {
                lines.push(String::new());
                lines.push("Usage ledger unavailable (get_session_stats failed).".to_string());
            }
        }
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

    // ─── Session history / settings ────────────────────────────────────

    /// Adopt a freshly loaded page of a session's history: the transcript is
    /// replaced and the older-history cursor is set from the page.
    ///
    /// `page` is the raw `get_session_entries` payload
    /// (`{entries, hasMore, nextOffset}`); an error leaves the transcript
    /// alone (the caller reports it) because a failed load is not an empty
    /// session.
    fn apply_history_page(&mut self, session_id: &str, page: Result<Value, String>) {
        let Ok(page) = page else { return };
        let parsed = SessionEntriesPage::from_value(&page);
        self.chat.clear_messages();
        // The transcript is rebuilt from scratch, so the scrollback watermark
        // anchors to the new render's own first row again.
        self.history.rebase();
        self.push_history_rows(&parsed.entries);
        self.history_paging.adopt(session_id, &parsed, i64::MAX);
        self.request_render(true);
    }

    /// Prepend an older page above the transcript, keeping the user's place.
    fn prepend_history_page(&mut self, session_id: &str, before: i64, page: Result<Value, String>) {
        if session_id != self.state.session_id || session_id != self.history_paging.session_id {
            // The user moved on while this page was in flight; its rows belong
            // to a transcript that is no longer on screen.
            return;
        }
        let parsed = match page {
            Ok(page) => SessionEntriesPage::from_value(&page),
            Err(err) => {
                self.history_paging.failed();
                self.add_system_message(format!("Failed to load older history: {err}"));
                return;
            }
        };
        let messages = Self::history_rows_to_messages(&parsed.entries);
        // A load is triggered by scrolling up at the top, so the reader is
        // normally still there when the page lands: anchoring the viewport (what
        // `prepend_messages` does) would leave the screen unchanged and the
        // loaded rows invisible above it. Show them instead — one viewport of
        // older content, the way the terminal's own scrollback pages. A reader
        // who moved in the meantime is anchored, not yanked.
        let was_at_top = self.chat.is_at_top();
        let added = self.chat.prepend_messages(messages);
        // Those rows sit above everything already handed to the scrollback,
        // which is append-only — realign the watermark so the next flush does
        // not take the transcript for new content and write it twice.
        self.history.shift(added);
        if was_at_top {
            // Reading back is not following the tail any more: without this the
            // next streamed row would drag the view away from the page the
            // reader just asked for.
            self.chat.set_auto_scroll(false);
            self.chat.scroll_up(self.chat.viewport_height());
        }
        self.history_paging.adopt(session_id, &parsed, before);
        self.request_render(true);
    }

    /// Append history rows to the transcript (first load / whole-session
    /// rebuild), in order.
    fn push_history_rows(&mut self, rows: &[Value]) {
        for message in Self::history_rows_to_messages(rows) {
            self.chat.add_message(message);
        }
    }

    /// Map journal/history rows to chat messages.
    ///
    /// The rows are the agent's display projection in either shape it serves:
    /// `get_session_entries` rows (`kind` + `blocks`) or the legacy
    /// `get_messages` LLM rows (`role` + `blocks`). Both carry the same block
    /// vocabulary, so one mapper renders history and today's live/replayed
    /// conversation identically. Rows the transcript has no place for
    /// (`session_info`, run markers, empty bodies) are dropped.
    fn history_rows_to_messages(rows: &[Value]) -> Vec<ChatMessage> {
        // A `tool_result` block carries only the call id — the display name and
        // arguments live on the matching `tool_call` block. Index those first so
        // replayed tool messages render like live ones instead of falling back
        // to the raw call id (`call_00_...`).
        let mut tool_calls: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
        for msg in rows {
            let Some(blocks) = msg.get("blocks").and_then(Value::as_array) else {
                continue;
            };
            for b in blocks {
                if b["kind"].as_str() != Some("tool_call") {
                    continue;
                }
                let Some(call_id) = b["toolCallId"].as_str().filter(|s| !s.is_empty()) else {
                    continue;
                };
                tool_calls.insert(
                    call_id.to_string(),
                    (
                        b["name"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned),
                        b.get("arguments").map(|v| match v {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        }),
                    ),
                );
            }
        }

        let mut messages = Vec::new();
        for msg in rows {
            let Some(obj) = msg.as_object() else { continue };
            if obj.get("kind").and_then(Value::as_str) == Some("compaction") {
                let checkpoint = &obj["checkpoint"];
                messages.push(ChatMessage::new(
                    obj.get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("compaction")
                        .to_string(),
                    ChatRole::System,
                    &compaction_completed_text(
                        checkpoint.get("tokensBefore"),
                        checkpoint.get("tokensAfter"),
                    ),
                ));
                continue;
            }
            let role = obj.get("role").and_then(Value::as_str).unwrap_or("");
            // Only render user, assistant, and tool messages.
            if !["user", "assistant", "tool"].contains(&role) {
                continue;
            }

            let blocks = obj
                .get("blocks")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let content = blocks
                .iter()
                .filter(|b| matches!(b["kind"].as_str(), Some("text" | "tool_result")))
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("");
            let thinking = blocks
                .iter()
                .filter(|b| b["kind"] == "reasoning")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("");
            // A row the transcript would render as nothing is dropped.
            //
            // The agent stores a tool-using turn as one assistant entry per call
            // (the `tool_call` step, which carries no text) followed by the
            // call's own `tool` entry, so those steps are structural, not
            // content: kept as messages they render no row of their own while
            // still taking a separator (a second blank line between calls the
            // live view does not have) and breaking every run of consecutive
            // calls, so a loaded session never folded. A call row is never
            // dropped on that ground — it has a row of its own to render, and
            // the live view shows it too even when the call returned nothing.
            let renders_something = role == "tool" || !content.is_empty() || !thinking.is_empty();
            if !renders_something {
                continue;
            }
            // (Pre-filtered above to user/assistant/tool.)
            let role_enum = match role {
                "user" => ChatRole::User,
                "assistant" => ChatRole::Assistant,
                _ => ChatRole::Tool,
            };
            let id = obj.get("id").and_then(Value::as_str).unwrap_or("");
            let id = if id.is_empty() {
                random_id()
            } else {
                id.to_string()
            };
            let mut cm = ChatMessage::new(id, role_enum, &content);
            let tool = blocks
                .iter()
                .find(|b| matches!(b["kind"].as_str(), Some("tool_call" | "tool_result")));
            let call_id = tool
                .and_then(|b| b["toolCallId"].as_str())
                .filter(|s| !s.is_empty());
            let known_call = call_id.and_then(|id| tool_calls.get(id));
            cm.name = tool
                .and_then(|b| b["name"].as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .or_else(|| known_call.and_then(|(name, _)| name.clone()));
            cm.tool = call_id.map(str::to_owned);
            cm.tool_args = match tool.and_then(|b| b.get("arguments")) {
                Some(Value::String(s)) => Some(s.clone()),
                Some(other) => Some(other.to_string()),
                None => known_call.and_then(|(_, args)| args.clone()),
            };
            cm.thinking = (!thinking.is_empty()).then_some(thinking);
            if role == "tool" {
                cm.tool_status = Some(
                    if tool.and_then(|b| b["isError"].as_bool()).unwrap_or(false) {
                        ToolStatus::Error
                    } else {
                        ToolStatus::Complete
                    },
                );
            }
            messages.push(cm);
        }
        messages
    }

    fn load_tui_settings(&mut self) {
        let data = std::fs::read_to_string(&self.tui_settings_path).unwrap_or_default();
        let v: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
        self.tui_settings = TuiSettings::from_json(&v);
        if let Some(ids) = self.tui_settings.enabled_model_ids.clone() {
            self.enabled_model_ids = Some(ids);
        }
        self.apply_theme_setting();
    }

    /// Adopt the palette persisted by `/theme`. An unknown or absent id falls
    /// back to the catalog default.
    fn apply_theme_setting(&mut self) {
        let (_id, theme) = crate::themes::resolve_theme(self.tui_settings.theme_id.as_deref());
        self.apply_theme(theme);
    }

    /// Hand `theme` to every live chrome widget (`chat`, `footer`, `input`) and
    /// to the overlays already on the stack (reachable while a menu is open:
    /// `/theme` applies immediately). Overlays built *later* are themed by
    /// [`Self::show_overlay`], so a widget created after the switch starts in
    /// the current palette instead of the default one.
    fn apply_theme(&mut self, theme: Theme) {
        let changed = self.theme != theme;
        self.theme = theme;
        self.chat.set_theme(theme);
        self.footer.set_theme(&theme);
        self.input.set_theme(&theme);
        for entry in self.overlay_stack.iter_mut() {
            apply_theme_to_component(entry.component.as_mut(), theme);
            apply_theme_to_new_overlay(entry.component.as_mut(), theme);
        }
        if changed {
            self.request_render(true);
        }
    }

    /// Persist + apply a `/theme` pick; returns the canonical id.
    fn select_theme(&mut self, id: &str) -> String {
        let (canonical, _theme) = crate::themes::resolve_theme(Some(id));
        self.tui_settings.theme_id = Some(canonical.to_string());
        self.save_tui_settings();
        // Persisted first, then fan out through the single palette path.
        self.apply_theme_setting();
        self.add_system_message(format!("Theme set to {canonical}"));
        self.request_render(true);
        canonical.to_string()
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
        // Nothing else in the TUI fetches the model list until `/model` or
        // `/status` is opened, and attaching an image has to know whether the
        // active model can see it. One `list_models` here (the desktop does the
        // same) — it feeds only `model_image_support`, so the `/model`
        // autocomplete keeps fetching on first use exactly as before.
        if let Ok(models) = self.client.list_models().await {
            self.remember_model_image_support(&models);
        }
    }

    /// Record which models accept image input, from a `list_models` reply.
    fn remember_model_image_support(&mut self, models: &[ModelInfo]) {
        for model in models {
            // `state.model` arrives both bare and provider-qualified depending
            // on where it came from, so both spellings answer.
            self.model_image_support
                .insert(model.id.clone(), model.supports_images);
            self.model_image_support
                .insert(model.full_id(), model.supports_images);
        }
    }

    /// Whether the active model accepts image input: `Some(false)` is what the
    /// attachment line warns about, `None` means the TUI has not been told.
    fn current_model_image_support(&self) -> Option<bool> {
        self.model_image_support.get(&self.state.model).copied()
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
        let session_id = self.state.session_id.clone();
        let compaction_revision = self.state.compaction_revision;
        tokio::spawn(async move {
            let _ = tx.send(UiCmd::RefreshCompleted {
                result: client.get_state().await,
                session_id,
                compaction_revision,
            });
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
        // Guard against a stale `get_state` snapshot racing `agent_end`: the
        // agent broadcasts `agent_end` from inside the run task but only
        // clears `is_streaming` later, when the completion monitor calls
        // `RunControl::finish`. A refresh answered inside that window returns
        // `isStreaming: true` plus an `activeRun` that may still report
        // "finalizing" — or, because `get_state` reads `active_run.state`
        // before it reads `is_streaming`, an even older "running". Don't gate
        // on the snapshot's phase: if our local event bookkeeping already
        // marked that exact run terminal, the event stream is fresher and the
        // snapshot must not re-assert the spinner (there is no later refresh
        // to correct it).
        let stale_snapshot = match &s.active_run {
            Some(active) if s.is_streaming => self
                .chat
                .run_state(&active.run_id)
                .is_some_and(|rs| rs != RunState::Running && rs != RunState::Queued),
            _ => false,
        };
        self.state.streaming = s.is_streaming && !stale_snapshot;
        self.state.compacting = s.is_compacting;
        if !self.state.streaming {
            self.state.active_tool_count = 0;
            self.state.tool_start_time = None;
        }
        if !s.session_id.is_empty() {
            if self.state.session_id != s.session_id {
                self.state.compaction_requested = false;
                self.state.compaction_revision += 1;
            }
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
        self.state.tokens_in = s.usage.input_tokens;
        self.state.tokens_out = s.usage.output_tokens;
        self.state.tokens_cache_r = s.usage.cache_read_tokens;
        self.state.tokens_cache_w = s.usage.cache_write_tokens;
        self.state.total_cost = s.usage.cost_cny;
        self.state.explicit_session = s.explicit_session;
        self.state.auto_compaction_enabled = s.auto_compaction_enabled;
        // The name is part of the identity shown in the window title, so it
        // follows the snapshot: without this a switch would leave the previous
        // session's name in the title (and a rename made elsewhere would never
        // show up here).
        if self.state.session_name != s.session_name {
            self.state.session_name = s.session_name.clone();
            self.update_terminal_title();
        }
        // The agent's own level wins (an unknown/absent value keeps the last
        // known one, which the picker mirrors).
        if let Some(level) = s
            .permission_level
            .as_deref()
            .and_then(PermissionKind::from_wire)
        {
            self.state.permission_level = level;
        }

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

    /// Move the app's session identity to `target`, without a `get_state`
    /// snapshot to carry it: everything keyed to `state.session_id` (drafts,
    /// the refresh reply guard, event filtering) must name the session the
    /// client addresses, and a failed `get_state` must not leave it behind.
    /// A real change also bumps the compaction revision, exactly as
    /// [`Self::apply_refresh_state`] does for a session change — a compaction
    /// fence from the session we left must not keep gating sends here.
    fn adopt_session_identity(&mut self, target: &str) {
        if self.state.session_id != target {
            self.state.compaction_requested = false;
            self.state.compaction_revision += 1;
            self.state.session_id = target.to_string();
        }
        if self.client.get_current_session_id() != target {
            self.client.set_current_session_id(target);
            self.client.connect_events();
        }
    }

    /// A superseded switch's RPC may have left the client addressing the older
    /// target (the last `switch_session` to land wins on the wire). Point it
    /// back at the switch the app is actually showing, so a prompt cannot be
    /// sent to a session the user is not looking at. Only once that newer
    /// switch has been applied: until then its own RPC is what binds the client,
    /// and pointing it at a target whose switch may still fail would address a
    /// session the app never showed.
    fn realign_client_to_latest_switch(&mut self) {
        let Some(latest) = self.latest_session_switch.as_ref() else {
            return;
        };
        if self.state.session_id != latest.target {
            return;
        }
        if self.client.get_current_session_id() != latest.target {
            self.client.set_current_session_id(&latest.target);
            self.client.connect_events();
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
                (
                    self.input.get_value().to_string(),
                    self.input.take_pending(),
                ),
            );
        }
    }

    fn restore_session_input(&mut self) {
        let cached = self
            .session_input_cache
            .get(&self.state.session_id)
            .cloned();
        let (draft, pending) = cached.unwrap_or_default();
        // The order matters: setting the text re-derives the attachment list
        // from the markers (the outgoing draft's images are not this one's),
        // and the draft's own pending state goes back afterwards.
        self.input.set_value(&draft, None);
        self.input.restore_pending(pending);
    }

    fn invalidate(&mut self) {
        self.chat.invalidate();
        self.input.invalidate();
        self.footer.invalidate();
    }

    // ─── Overlays ──────────────────────────────────────────────────────

    fn show_help_overlay(&mut self) {
        // The card is taller than most panes, so it is given a viewport sized
        // to this terminal rather than being dumped whole and clipped
        // (see `HelpOverlay`).
        let rows = (self.terminal.rows() as usize).max(1);
        let width = self.terminal.columns() as usize;
        self.show_overlay(
            Box::new(HelpOverlay::new(rows)),
            OverlayOptions {
                width: Some(SizeValue::Fixed(width)),
                max_height: Some(SizeValue::Fixed(rows)),
                ..Default::default()
            },
        );
    }

    fn show_overlay(&mut self, mut component: Box<dyn Component>, options: OverlayOptions) -> u64 {
        // Overlays are built at runtime, so a widget created *after* `/theme`
        // has to be handed the current palette here; the loop in
        // [`Self::apply_theme`] only reaches instances that already exist.
        apply_theme_to_component(component.as_mut(), self.theme);
        apply_theme_to_new_overlay(component.as_mut(), self.theme);
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

    /// `compositeLineAt` — paint one overlay line across the whole screen row
    /// it occupies.
    ///
    /// The row belongs to the overlay: its left and right insets (a centred
    /// card is narrower than the terminal) are painted as blanks, never copied
    /// from `base`. Copying them is what glued stray chat/footer characters
    /// onto the card's edges — a centred 76-column panel used to keep columns
    /// 0..1 and 78..79 of whatever the chat had painted on that same row
    /// (` HInput (uncached)`, `/m…k`), so half a word from the transcript sat
    /// inside the panel and, on the panel's last row, the command editor's
    /// `> ` prompt and the footer ran into the card's hint line.
    ///
    /// `overlay_width` is therefore unused: the overlay's own visible width
    /// decides where the right-hand blank run starts (`col + overlay_width`
    /// would leave the columns between the card and the terminal edge showing
    /// the base again). `base` still matters for image lines, which are
    /// emitted verbatim — an overlay cannot be painted over a sixel/kitty
    /// escape sequence.
    pub fn composite_line_at(
        base: &str,
        overlay: &str,
        col: usize,
        _overlay_width: usize,
        total_width: usize,
    ) -> String {
        if is_image_line(base) {
            return base.to_string();
        }

        // Extract overlay with width tracking.
        let overlay_clean = strip_ansi_codes(overlay);
        let overlay_vis_width = visible_width(&overlay_clean);

        // `col` blank columns of inset, the overlay, then blanks to the
        // terminal edge — the row carries no character of `base`.
        let before_pad = col.min(total_width);
        let after_pad = total_width
            .saturating_sub(before_pad)
            .saturating_sub(overlay_vis_width);

        // Compose result with reset marker between segments.
        let reset = "\x1b[0m";
        let result = format!(
            "{}{}{}{}{}",
            " ".repeat(before_pad),
            reset,
            overlay,
            " ".repeat(after_pad),
            reset
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
            self.reset_diff_baseline();
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

    /// Ask the terminal for its real cursor position (DSR / CPR). The
    /// expected row is snapshotted now: the terminal answers with the cursor
    /// position at the moment it processes this query (before any render that
    /// runs later in the same tick), so we must compare the answer against
    /// the row we tracked when the query was issued — not the row after any
    /// intervening render moved the cursor.
    fn query_cursor_position(&mut self) {
        self.cursor_recheck_row = Some(self.hardware_cursor_row);
        self.terminal.write("\x1b[6n");
    }

    /// Parse a DSR cursor-position report (`\x1b[{row};{col}R`, 1-based) and
    /// force a full redraw if the terminal's real row diverged from the row
    /// snapshotted when the query was sent. This is the polling net for a
    /// tmux attach that reset the cursor without a SIGWINCH or focus event.
    fn consume_cursor_position_response(&mut self, data: &str) -> bool {
        static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r"^\x1b\[(\d+);(\d+)R$").unwrap());
        let Some(caps) = re.captures(data) else {
            return false;
        };
        let reported_row = caps[1].parse::<usize>().unwrap_or(0);
        if reported_row == 0 {
            return true; // malformed row — consume, don't act
        }
        let real_row = reported_row - 1; // DSR is 1-based; we track 0-based
        if let Some(expected) = self.cursor_recheck_row.take() {
            if real_row != expected {
                self.request_render(true);
            }
        }
        true
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
        // Layout already wraps lines. Terminal width tables (especially tmux's
        // emoji widths) can disagree with ours: an implicit wrap would shift
        // every subsequent row and scroll the screen on each streaming frame.
        buf += "\x1b[?7l";
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
        buf += "\x1b[?7h";
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
        let log_path = crate::home::home_dir_or_default()
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
        // A finished reply now lives in the scrollback too (see
        // `flush_scrollback`). It has to happen here rather than from the event
        // handler so the repaint follows the insert in the same synchronous pass:
        // re-entering the alternate screen clears it, and a frame painted a tick
        // later would show that blank screen.
        if self.scrollback_pending {
            self.scrollback_pending = false;
            if self.flush_scrollback(true) > 0 {
                self.reset_diff_baseline();
            }
        }
        if self.state.streaming
            || self.state.compacting
            || self.state.compaction_requested
            || matches!(self.skill_reco, SkillRecoState::Pending { .. })
        {
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
        let prev_viewport_top = if height_changed {
            previous_buffer_length.saturating_sub(h)
        } else {
            self.previous_viewport_top
        };
        let viewport_top = prev_viewport_top;
        let hardware_cursor_row = self.hardware_cursor_row;

        // Render editor first to determine its height (multi-line aware).
        let footer_data = FooterData {
            cwd: Some(self.state.cwd.clone()),
            model: Some(self.state.model.clone()),
            thinking: Some(self.state.thinking.clone()),
            streaming: self.state.streaming,
            compacting: self.state.compacting || self.state.compaction_requested,
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
        // The box draws the attachment line from this, so it has to be set
        // before the input renders (same frame, no flicker).
        self.input
            .set_image_support(self.current_model_image_support());
        let mut editor_lines = self.input.render(w);
        // The recommendation prompt sits directly above the input box and is
        // counted as editor height, so the chat viewport shrinks by exactly the
        // row it takes instead of being overdrawn by it (PRD v1.6 §6.1).
        if let Some(line) = self.skill_reco.prompt_line(self.state.spinner_frame) {
            editor_lines.insert(
                0,
                crate::theme::fg(self.theme.accent as u8, &fit_overlay_row(&line, w)),
            );
        }
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
            // previous_lines is strictly longer here: a new frame at least
            // as long would place first_changed inside it.
            debug_assert!(self.previous_lines.len() > new_lines.len());
            {
                let mut buf = SYNC_BEGIN.to_string();
                buf += "\x1b[?7l";
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
                buf += "\x1b[?7h";
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
        // As in full_render, allow only our explicit CRLFs to advance rows.
        buf += "\x1b[?7l";
        buf += &self.delete_changed_kitty_images(first_changed as usize, last_changed as usize);
        let prev_viewport_bottom = prev_viewport_top + h - 1;
        let move_target_row = if append_start {
            first_changed as usize - 1
        } else {
            first_changed as usize
        };
        // (No "scroll down to target" arm: move_target_row never exceeds
        // prev_viewport_bottom here. The viewport bottom tracks
        // max(h, len)-1 after full renders and only moves within that range
        // on diff renders, while move_target_row is always an existing or
        // appended row ≤ previous_lines.len()-1 ≤ bottom. Kept as an assert
        // so tests exercise the invariant on every render.)
        debug_assert!(move_target_row <= prev_viewport_bottom);

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

        let final_cursor_row = render_end;

        // Clear extra lines when content shrunk. (render_end always equals
        // new_lines.len()-1 here: shrinking sets last_changed at the old
        // tail, so the JS move-down arm can't trigger.)
        if self.previous_lines.len() > new_lines.len() {
            let extra_lines = self.previous_lines.len() - new_lines.len();
            for _ in new_lines.len()..self.previous_lines.len() {
                buf += "\r\n\x1b[2K";
            }
            buf += &format!("\x1b[{extra_lines}A");
        }

        buf += "\x1b[?7h";
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
    use std::sync::Mutex;

    // ─── Fake terminal ────────────────────────────────────────────────

    struct FakeTerminal {
        writes: Rc<RefCell<Vec<String>>>,
        cols: u16,
        rows: u16,
        on_input: Option<Box<dyn FnMut(String) + Send + 'static>>,
        on_resize: Option<Box<dyn FnMut() + Send + 'static>>,
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
            on_input: Box<dyn FnMut(String) + Send + 'static>,
            on_resize: Box<dyn FnMut() + Send + 'static>,
        ) -> std::io::Result<()> {
            self.on_input = Some(on_input);
            self.on_resize = Some(on_resize);
            Ok(())
        }
        fn stop(&mut self) {}
        fn drain_input(&mut self, _max_ms: u64, _idle_ms: u64) {}
        fn set_exit_signal_callback(&mut self, _cb: Option<Box<dyn FnMut() + Send + 'static>>) {}
    }

    /// A settings path in its own temp directory, so the app's derived state
    /// files (the skill-recommendation budget lives beside settings) are private
    /// to one test. A shared path would couple tests through the daily budget.
    ///
    /// The directory is created here because several tests seed the settings
    /// file by hand before the app loads it.
    fn test_settings_path() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tui-test-{}", random_id()));
        let _ = std::fs::create_dir_all(&dir);
        dir.join("settings.json")
    }

    fn make_app(cols: u16, rows: u16) -> (App<FakeTerminal>, mpsc::UnboundedReceiver<UiCmd>) {
        let (op_tx, op_rx) = mpsc::unbounded_channel();
        let (client, _events, _conn) = GrpcClient::new("127.0.0.1:1");
        let app = App::new(
            FakeTerminal {
                writes: Rc::new(RefCell::new(Vec::new())),
                cols,
                rows,
                on_input: None,
                on_resize: None,
            },
            Arc::new(client),
            op_tx,
            &CliOptions::default(),
            test_settings_path(),
        );
        (app, op_rx)
    }

    #[tokio::test]
    async fn update_notice_is_local_and_preserves_input() {
        let (mut app, mut commands) = make_app(100, 30);
        app.input.set_value("unfinished prompt", None);
        let notice = "New FutureOS version available: v1.2.3 → v1.2.4.";
        app.handle_cmd(UiCmd::UpdateAvailable(notice.into()));
        let message = app.chat.last_message().unwrap();
        assert_eq!(message.role, ChatRole::System);
        assert_eq!(message.content, notice);
        assert_eq!(app.input.get_value(), "unfinished prompt");
        assert!(commands.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_notice_waits_for_stream_to_finish_and_shows_once() {
        let (mut app, _) = make_app(100, 30);
        app.state.streaming = true;
        app.chat.add_tool_start("tool-1", "read", None);
        app.handle_cmd(UiCmd::UpdateAvailable("Update available".into()));
        app.on_tick();
        assert_eq!(app.chat.last_message().unwrap().role, ChatRole::Tool);
        app.chat.append_to_last_message("Answer after tool");
        assert_eq!(app.chat.last_message().unwrap().role, ChatRole::Assistant);
        assert_eq!(
            app.chat.last_message().unwrap().content,
            "Answer after tool"
        );
        app.state.streaming = false;
        app.on_tick();
        assert_eq!(app.chat.last_message().unwrap().content, "Update available");
        assert!(app.pending_update_notice.is_none());
        let count = app.chat.plain_messages().len();
        app.on_tick();
        assert_eq!(app.chat.plain_messages().len(), count);
    }

    fn terminal_writes(app: &App<FakeTerminal>) -> String {
        app.terminal.writes.borrow().join("")
    }

    /// True when the app wrote a bare BEL — the bell byte itself, as opposed to
    /// the BEL that terminates a window-title (`OSC 0`) or `OSC 9` sequence.
    /// The `OSC` writers landed with the notification/title work, so the bell
    /// assertions must look for the standalone byte.
    fn wrote_bell(app: &App<FakeTerminal>) -> bool {
        app.terminal
            .writes
            .borrow()
            .iter()
            .any(|chunk| chunk.contains('\u{7}') && !chunk.contains("\u{1b}]"))
    }

    // ─── Cursor-tracking terminal (tmux-attach desync repro) ───────────

    /// Track the terminal's cursor *row* by replaying the ANSI the app writes.
    /// Columns and SGR/OSC are ignored; only row-changing sequences matter
    /// (LF, CUU/CUD, CUP/home). This mirrors what a real terminal does — and
    /// what tmux's screen buffer does — so we can simulate an external cursor
    /// reset (a client attach) and observe the differential renderer diverge.
    fn track_row(mut row: usize, chunk: &str) -> usize {
        let bytes = chunk.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\n' => {
                    row += 1;
                    i += 1;
                }
                b'\r' => i += 1, // CR: no row change
                0x1b => {
                    if bytes.get(i + 1) == Some(&b'[') {
                        let start = i + 2;
                        let mut j = start;
                        while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                            j += 1;
                        }
                        if j < bytes.len() {
                            let params = std::str::from_utf8(&bytes[start..j]).unwrap_or("");
                            match bytes[j] as char {
                                'A' => {
                                    let n: usize = params.parse().unwrap_or(1);
                                    row = row.saturating_sub(n);
                                }
                                'B' => {
                                    let n: usize = params.parse().unwrap_or(1);
                                    row += n;
                                }
                                'H' | 'f' => {
                                    if params.is_empty() {
                                        row = 0;
                                    } else if let Some((r, _)) = params.split_once(';') {
                                        row = r.parse::<usize>().unwrap_or(1).saturating_sub(1);
                                    }
                                }
                                _ => {} // J/K/G/m/h/l/... — no row change
                            }
                            i = j + 1;
                            continue;
                        }
                    }
                    // OSC / unknown escape: skip the ESC byte; the rest falls
                    // through as ordinary (row-neutral) bytes.
                    i += 1;
                }
                _ => i += 1,
            }
        }
        row
    }

    struct TrackingTerminal {
        writes: Rc<RefCell<Vec<String>>>,
        cursor_row: Rc<RefCell<usize>>,
        cols: u16,
        rows: u16,
        on_input: Option<Box<dyn FnMut(String) + Send + 'static>>,
        on_resize: Option<Box<dyn FnMut() + Send + 'static>>,
    }

    impl TerminalIo for TrackingTerminal {
        fn write(&self, data: &str) {
            self.writes.borrow_mut().push(data.to_string());
            let mut row = *self.cursor_row.borrow();
            row = track_row(row, data);
            *self.cursor_row.borrow_mut() = row;
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
            on_input: Box<dyn FnMut(String) + Send + 'static>,
            on_resize: Box<dyn FnMut() + Send + 'static>,
        ) -> std::io::Result<()> {
            self.on_input = Some(on_input);
            self.on_resize = Some(on_resize);
            Ok(())
        }
        fn stop(&mut self) {}
        fn drain_input(&mut self, _max_ms: u64, _idle_ms: u64) {}
        fn set_exit_signal_callback(&mut self, _cb: Option<Box<dyn FnMut() + Send + 'static>>) {}
    }

    fn make_tracking_app(
        cols: u16,
        rows: u16,
    ) -> (
        App<TrackingTerminal>,
        mpsc::UnboundedReceiver<UiCmd>,
        Rc<RefCell<usize>>,
    ) {
        let (op_tx, op_rx) = mpsc::unbounded_channel();
        let (client, _events, _conn) = GrpcClient::new("127.0.0.1:1");
        let cursor_row = Rc::new(RefCell::new(0usize));
        let app = App::new(
            TrackingTerminal {
                writes: Rc::new(RefCell::new(Vec::new())),
                cursor_row: Rc::clone(&cursor_row),
                cols,
                rows,
                on_input: None,
                on_resize: None,
            },
            Arc::new(client),
            op_tx,
            &CliOptions::default(),
            std::env::temp_dir().join("tui-test-settings.json"),
        );
        (app, op_rx, cursor_row)
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
    use std::pin::Pin;
    use tokio_stream::wrappers::UnboundedReceiverStream;
    use tonic::transport::server::TcpIncoming;
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
            // A first frame so the client's connected edge fires.
            let first = StreamEvent {
                r#type: "ping".into(),
                data: String::new(),
                ..Default::default()
            };
            Ok(tonic::Response::new(Box::pin(
                futures_util::stream::once(async move { Ok(first) })
                    .chain(UnboundedReceiverStream::new(rx).map(Ok)),
            )))
        }
    }

    async fn spawn_cwd_mock_agent() -> (
        tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
        String,
    ) {
        // Hand the *already bound* listener to tonic: with `serve(addr)` the
        // port was probe-bound, dropped and re-bound, so a concurrent mock
        // could steal it in that window and the client saw a transport error.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let incoming = TcpIncoming::from_listener(listener, true, None).unwrap();
        let agent = CwdMockAgent {
            subs: Arc::new(std::sync::Mutex::new(Vec::new())),
        };
        // Spawn the serve future directly (no never-completing task tail).
        let handle = tokio::spawn(
            Server::builder()
                .add_service(FutureAgentServer::new(agent))
                .serve_with_incoming(incoming),
        );
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
                on_input: None,
                on_resize: None,
            },
            Arc::new(client),
            op_tx,
            &CliOptions::default(),
            std::env::temp_dir().join("tui-cwd-test-settings.json"),
        );
        // Subscribe to the event stream so the agent's echo arrives.
        app.client.set_current_session_id("s1");
        app.client.connect_events();
        tokio::time::sleep(Duration::from_millis(100)).await;

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

    /// `/status` lines must match the TS template's *values* exactly — in
    /// particular the JS `|| "(none)"` fallback: a present sessionId renders
    /// WITHOUT the ` or (none)` suffix (the P4 tmux harness caught the port
    /// always appending it), and the model fallback is `|| "(unknown)"`
    /// without an ` or (unknown)` suffix either. Only the `**` emphasis is
    /// dropped: the chat paints system messages verbatim (markdown is for
    /// user/assistant text), so the TS template's bold markers reached the
    /// screen as asterisks nobody could read past.
    #[tokio::test]
    async fn apply_status_session_and_model_fallbacks_match_ts() {
        let (mut app, _rx) = make_app(120, 36);

        // Session id present → `Session: mock-session-1` exactly.
        let s = RpcSessionState {
            session_id: "mock-session-1".into(),
            model: Some("mock-model".into()),
            ..Default::default()
        };
        app.apply_status(&s, &[], None);
        let last = app.chat.last_message().cloned().unwrap();
        assert!(last.content.contains("Session: mock-session-1"));
        assert!(!last.content.contains(" or (none)"));
        assert!(last.content.contains("Model: mock-model"));
        assert!(!last.content.contains(" or (unknown)"));
        assert!(
            !last.content.contains("**"),
            "a system message is painted verbatim: {}",
            last.content
        );

        // No session id / no model → `(none)` / `(unknown)` stand alone.
        let s2 = RpcSessionState::default();
        app.apply_status(&s2, &[], None);
        let last = app.chat.last_message().cloned().unwrap();
        assert!(last.content.contains("Session: (none)"));
        assert!(!last.content.contains(" or (none)"));
        assert!(last.content.contains("Model: (unknown)"));
        assert!(!last.content.contains(" or (unknown)"));
    }

    #[test]
    fn composite_line_at_paints_the_overlay_across_the_whole_row() {
        // The overlay owns the row: the base's columns 0..1 (`ab`) and 4..5
        // (`ef`) are blanked instead of copied. Copying them is what glued
        // stray chat characters (and the editor's `> ` prompt) to the edges of
        // a centred panel.
        let base = "abcdef";
        let result = App::<FakeTerminal>::composite_line_at(base, "XY", 2, 2, 6);
        assert_eq!(result, "  \x1b[0mXY  \x1b[0m");
        assert_eq!(visible_width(&result), 6);
    }

    #[test]
    fn composite_line_at_pads_shorter_overlay() {
        // Overlay is 1 char in a 3-char slot → the row is still filled to its
        // full width, and the base's `ab`/`ef` are gone.
        let base = "abcdef";
        let result = App::<FakeTerminal>::composite_line_at(base, "X", 2, 3, 6);
        assert_eq!(result, "  \x1b[0mX   \x1b[0m");
        assert_eq!(visible_width(&result), 6);
    }

    /// The D1 shape: an 80-column screen, a 76-column card at column 2, and a
    /// chat line underneath whose first and last characters used to show at
    /// columns 0..1 and 78..79 (`H` from ` Hello …`, `!` at the right edge).
    #[test]
    fn composite_line_at_clears_both_insets_of_a_centred_panel() {
        let base = " Hello from the mock agent!";
        let overlay = "  Sandbox: available — backend macos_seatbelt";
        let result = App::<FakeTerminal>::composite_line_at(base, overlay, 2, 76, 80);
        let text = strip_ansi_codes(&result);
        assert_eq!(
            text,
            format!("  {overlay}{}", " ".repeat(80 - 2 - visible_width(overlay)))
        );
        assert!(!text.contains('H'), "the chat's first column: {text:?}");
        assert!(!text.contains('!'), "the chat's last column: {text:?}");
        assert_eq!(visible_width(&result), 80);
        // The 80-column version of the same row: full-width overlay, no inset
        // and nothing of the base left over.
        let full = App::<FakeTerminal>::composite_line_at(base, overlay, 0, 80, 80);
        assert_eq!(
            strip_ansi_codes(&full),
            format!("{overlay}{}", " ".repeat(80 - visible_width(overlay)))
        );
    }

    #[test]
    fn composite_line_at_skips_image_lines() {
        let base = "\x1b_Ga=1;m=0; ";
        let result = App::<FakeTerminal>::composite_line_at(base, "XY", 2, 2, 80);
        assert_eq!(result, base);
    }

    /// Every row the overlay covers is repainted from the overlay alone; rows
    /// outside its rectangle are handed back byte-for-byte. Before the D1 fix
    /// the covered rows kept the base's first two columns (`chat 0` → `ch`),
    /// which is the ` HInput (uncached)` / `> Total` family of defects.
    #[tokio::test]
    async fn composite_overlays_clears_every_row_it_covers() {
        let (mut app, _rx) = make_app(80, 20);
        app.show_overlay(
            Box::new(ProbeComponent {
                lines: 3,
                wants_release: false,
                render_only_at: None,
            }),
            OverlayOptions {
                width: Some(SizeValue::Fixed(76)),
                ..Default::default()
            },
        );
        // Distinctive base rows: any surviving character is detectable.
        let base: Vec<String> = (0..20).map(|i| format!("chat {i}")).collect();
        let out = app.composite_overlays(base.clone(), 80, 20);
        assert_eq!(out.len(), 20);

        let covered: Vec<usize> = out
            .iter()
            .enumerate()
            .filter(|(_, row)| strip_ansi_codes(row).contains("probe"))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(covered.len(), 3, "all three probe rows are on screen");
        for (offset, row) in covered.iter().enumerate() {
            let text = strip_ansi_codes(&out[*row]);
            assert_eq!(
                text.trim_end(),
                format!("  probe {offset}"),
                "row {row} is the overlay's own row (col 2, 76 wide)"
            );
            assert!(
                !text.contains("chat"),
                "row {row} keeps base text: {text:?}"
            );
        }

        for (row, line) in out.iter().enumerate() {
            if !covered.contains(&row) {
                assert_eq!(line, &base[row], "row {row} must be untouched");
            }
        }
    }

    /// D2: the pager is a full-screen view — its first row is screen row 1 and
    /// its status bar is the last row, so the chat line underneath never shows
    /// above the pager's body (`/transcript` used to print `New session
    /// started.` twice).
    #[tokio::test]
    async fn pager_overlay_owns_every_screen_row() {
        let (mut app, _rx) = make_app(80, 36);
        app.show_pager_text(vec!["alpha".into(), "beta".into()], "nothing");
        let text = {
            let rows = app.overlay_stack[0].component.render(80);
            assert_eq!(rows.len(), 36, "the pager renders the whole height");
            rows.iter()
                .map(|row| strip_ansi_codes(row))
                .collect::<Vec<_>>()
        };
        assert_eq!(text[0].trim_end(), "alpha", "row 1 is the pager's body");
        assert_eq!(text[1].trim_end(), "beta");
        assert!(text[35].contains("100%"), "row 36 is the status bar");

        // Composited over a chat-like base, no base character survives.
        let base: Vec<String> = (0..36).map(|i| format!("chat {i}")).collect();
        let out = app.composite_overlays(base, 80, 36);
        assert_eq!(out.len(), 36);
        for (row, line) in out.iter().enumerate() {
            let line = strip_ansi_codes(line);
            assert!(
                !line.contains("chat"),
                "row {row} keeps the base line: {line:?}"
            );
        }
        assert_eq!(strip_ansi_codes(&out[0]).trim_end(), "alpha");
        assert!(strip_ansi_codes(&out[35]).contains("100%"));
    }

    #[test]
    fn tui_settings_roundtrip() {
        let settings = TuiSettings {
            default_model: Some("deepseek-v4-pro".into()),
            default_thinking_level: Some("high".into()),
            default_permission_level: None,
            enabled_model_ids: Some(vec!["a".into(), "b".into()]),
            bell_on_complete: None,
            theme_id: None,
            notify: None,
            skill_recommend: None,
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
            bell_on_complete: None,
            theme_id: None,
            notify: None,
            skill_recommend: None,
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
            r#"{"model":"deepseek-v4-pro","thinkingLevel":"high","isStreaming":true,"sessionId":"s1","cwd":"/tmp","queryCount":2,"skills":["b","a"],"contextTokens":100,"contextWindow":128000,"contextPercent":0.1,"usage":{"inputTokens":10,"outputTokens":20,"cacheReadTokens":1,"cacheWriteTokens":2,"costCny":0.01},"autoCompactionEnabled":true,"explicitSession":false}"#,
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

    /// Regression test for the "spinner keeps spinning after the reply
    /// finishes" bug: a stale `get_state` snapshot answered in the agent's
    /// "finalizing" window (is_streaming still true, activeRun still present)
    /// must not re-assert streaming after `agent_end` already cleared it.
    #[tokio::test]
    async fn stale_get_state_after_agent_end_must_not_reassert_streaming() {
        let (mut app, _rx) = make_app(100, 30);

        // Bind a user message to run-1 and mark the run active.
        app.chat
            .add_message(ChatMessage::new("m1".into(), ChatRole::User, "hi"));
        app.chat
            .bind_user_run("m1", "run-1", RunState::Running, None);
        app.state.streaming = true;

        // agent_end marks the run terminal and clears streaming.
        app.handle_agent_event(&make_event_with_run("agent_end", "{}", "run-1"));
        assert!(!app.state.streaming, "agent_end must clear streaming");

        // A stale get_state snapshot still reports the run active + finalizing.
        let stale: RpcSessionState = serde_json::from_value(json_parse(
            r#"{"sessionId":"s1","model":"openai/gpt-4o","thinkingLevel":"high","isStreaming":true,"activeRun":{"runId":"run-1","epoch":1,"state":"finalizing","lastEventIdx":9}}"#,
        ))
        .unwrap();
        app.handle_cmd(UiCmd::Refreshed(Ok(stale)));

        assert!(
            !app.state.streaming,
            "stale finalizing snapshot must not re-assert streaming (spinner stuck)"
        );
    }

    /// Same race as above, but for a run this client never saw end (a foreign
    /// run owned by another client on the same session): local bookkeeping
    /// has no terminal record, so the snapshot's streaming must be trusted.
    #[tokio::test]
    async fn finalizing_snapshot_for_unknown_run_keeps_streaming() {
        let (mut app, _rx) = make_app(100, 30);

        // A foreign run streams: agent_start arrives over the event stream,
        // but this client has no message bound to the run.
        app.handle_agent_event(&make_event_with_run("agent_start", "{}", "foreign-run"));
        assert!(app.state.streaming);

        // The agent_end event is missed (raced past the subscription), and
        // the periodic refresh is what tells the TUI the run is still live:
        // a finalizing snapshot for the unknown run must be trusted.
        let state: RpcSessionState = serde_json::from_value(json_parse(
            r#"{"sessionId":"s1","model":"openai/gpt-4o","thinkingLevel":"high","isStreaming":true,"activeRun":{"runId":"foreign-run","epoch":1,"state":"finalizing","lastEventIdx":9}}"#,
        ))
        .unwrap();
        app.handle_cmd(UiCmd::Refreshed(Ok(state)));

        assert!(
            app.state.streaming,
            "finalizing snapshot for an unknown (foreign) run must keep streaming"
        );
    }

    /// The `agent_end` handler clears streaming, but a `get_state` answered
    /// just before `RunControl::finish` returns a stale snapshot that can
    /// report the active run as "running" (not only "finalizing"): the agent
    /// reads `active_run.state` *before* it reads `is_streaming`, so the
    /// phase can lag the terminal transition. `apply_refresh_state` must not
    /// let any such stale snapshot re-assert the spinner.
    #[tokio::test]
    async fn stale_running_snapshot_after_agent_end_must_not_reassert_streaming() {
        let (mut app, _rx) = make_app(100, 30);

        // Bind a user message to run-1 and mark the run active.
        app.chat
            .add_message(ChatMessage::new("m1".into(), ChatRole::User, "hi"));
        app.chat
            .bind_user_run("m1", "run-1", RunState::Running, None);
        app.state.streaming = true;

        // agent_end marks the run terminal and clears streaming.
        app.handle_agent_event(&make_event_with_run("agent_end", "{}", "run-1"));
        assert!(!app.state.streaming, "agent_end must clear streaming");
        assert_eq!(app.chat.run_state("run-1"), Some(RunState::Terminal));

        // A stale get_state snapshot still reports the run active with the
        // older "running" phase (not "finalizing"), is_streaming still true.
        let stale: RpcSessionState = serde_json::from_value(json_parse(
            r#"{"sessionId":"s1","model":"openai/gpt-4o","thinkingLevel":"high","isStreaming":true,"activeRun":{"runId":"run-1","epoch":1,"state":"running","lastEventIdx":9}}"#,
        ))
        .unwrap();
        app.handle_cmd(UiCmd::Refreshed(Ok(stale)));

        assert!(
            !app.state.streaming,
            "stale 'running' snapshot must not re-assert streaming (spinner stuck)"
        );
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
    async fn thinking_deltas_continue_after_queued_message() {
        // Regression: a user message queued mid-stream (enqueue_if_busy) is
        // pushed after the streaming assistant. Thinking deltas must keep
        // landing on that assistant, not silently drop or target the queued
        // user message.
        let (mut app, _rx) = make_app(100, 30);
        app.handle_agent_event(&make_event("agent_start", "{}"));
        app.handle_agent_event(&make_event("thinking_start", "{}"));
        app.handle_agent_event(&make_event("thinking_delta", "{\"text\":\"reason one\"}"));

        // Queue a message while thinking is streaming.
        app.handle_agent_event(&make_event(
            "user_message",
            "{\"text\":\"queued while streaming\"}",
        ));

        app.handle_agent_event(&make_event("thinking_delta", "{\"text\":\" reason two\"}"));
        app.handle_agent_event(&make_event("thinking_end", "{}"));

        assert_eq!(
            app.chat.last_assistant_thinking(),
            Some("reason one reason two")
        );
        // The queued user message is still the literal last message.
        assert_eq!(app.chat.last_message().unwrap().role, ChatRole::User);
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

    // ─── Done bell (agent_end terminal BEL) ────────────────────────────

    fn bind_our_run(app: &mut App<FakeTerminal>, run_id: &str) {
        app.chat
            .add_message(ChatMessage::new("m1".into(), ChatRole::User, "hello"));
        app.chat
            .bind_user_run("m1", run_id, RunState::Running, None);
    }

    #[tokio::test]
    async fn bell_rings_when_our_run_completes() {
        let (mut app, _rx) = make_app(100, 30);
        bind_our_run(&mut app, "run-1");
        app.handle_agent_event(&make_event_with_run(
            "agent_end",
            r#"{"state":"completed"}"#,
            "run-1",
        ));
        assert!(
            wrote_bell(&app),
            "a clean completion of our run must ring the terminal bell, wrote: {:?}",
            terminal_writes(&app)
        );
    }

    #[tokio::test]
    async fn bell_rings_when_our_run_errors() {
        let (mut app, _rx) = make_app(100, 30);
        bind_our_run(&mut app, "run-1");
        app.handle_agent_event(&make_event_with_run(
            "agent_end",
            r#"{"state":"error"}"#,
            "run-1",
        ));
        assert!(
            wrote_bell(&app),
            "an errored run needs the user's attention too"
        );
    }

    #[tokio::test]
    async fn bell_stays_silent_for_foreign_run() {
        let (mut app, _rx) = make_app(100, 30);
        bind_our_run(&mut app, "run-1");
        // A completed run this client never submitted (another TUI on the
        // same session) must not ring our bell.
        app.handle_agent_event(&make_event_with_run(
            "agent_end",
            r#"{"state":"completed"}"#,
            "run-foreign",
        ));
        assert!(!wrote_bell(&app), "foreign runs must not ring the bell");
    }

    #[tokio::test]
    async fn bell_stays_silent_for_cancelled_and_incomplete_runs() {
        let (mut app, _rx) = make_app(100, 30);
        bind_our_run(&mut app, "run-1");
        app.handle_agent_event(&make_event_with_run(
            "agent_end",
            r#"{"state":"cancelled"}"#,
            "run-1",
        ));
        app.handle_agent_event(&make_event_with_run(
            "agent_end",
            r#"{"state":"incomplete"}"#,
            "run-1",
        ));
        assert!(
            !wrote_bell(&app),
            "user-initiated cancels and incomplete streams must not ring"
        );
    }

    #[tokio::test]
    async fn bell_respects_bell_on_complete_setting() {
        let (mut app, _rx) = make_app(100, 30);
        bind_our_run(&mut app, "run-1");
        app.tui_settings.bell_on_complete = Some(false);
        app.handle_agent_event(&make_event_with_run(
            "agent_end",
            r#"{"state":"completed"}"#,
            "run-1",
        ));
        assert!(
            !wrote_bell(&app),
            "bellOnComplete=false must silence the bell"
        );
    }

    #[test]
    fn tui_settings_bell_on_complete_roundtrip() {
        // Absent → default on.
        let v: Value = json_parse(r#"{}"#);
        assert!(TuiSettings::from_json(&v).bell_enabled());
        // Explicit false → off.
        let v: Value = json_parse(r#"{"bellOnComplete":false}"#);
        assert!(!TuiSettings::from_json(&v).bell_enabled());
        // Explicit true → on, and serializes back to bellOnComplete.
        let v: Value = json_parse(r#"{"bellOnComplete":true}"#);
        let settings = TuiSettings::from_json(&v);
        assert!(settings.bell_enabled());
        assert_eq!(settings.to_json()["bellOnComplete"], Value::Bool(true));
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

    #[tokio::test(flavor = "multi_thread")]
    async fn submit_export_reports_the_agent_result() {
        let (mut app, mut rx) = make_app(100, 30);
        app.handle_submit("/export");
        // The client points at a dead port, so the export fails visibly — the
        // old "not available in the TUI" stub is gone (the success path is
        // covered by `inspection_commands_round_trip_through_a_live_agent`).
        pump_until_msg(&mut app, &mut rx, "Failed to export session").await;
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

    // ─── Scrollback (see `crate::insert_history`) ──────────────────────

    /// Terminal double for the scrollback tests: it records the screen lifecycle
    /// (`stop` = leave the alternate screen, `start` = re-enter), so a test can
    /// say not just *what* was written but *where in the round trip* it landed.
    struct ScrollbackTerminal {
        log: Rc<RefCell<Vec<String>>>,
        cols: u16,
        rows: u16,
        on_input: Option<Box<dyn FnMut(String) + Send + 'static>>,
        on_resize: Option<Box<dyn FnMut() + Send + 'static>>,
    }

    impl TerminalIo for ScrollbackTerminal {
        fn write(&self, data: &str) {
            self.log.borrow_mut().push(format!("write:{data}"));
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
            on_input: Box<dyn FnMut(String) + Send + 'static>,
            on_resize: Box<dyn FnMut() + Send + 'static>,
        ) -> std::io::Result<()> {
            self.log.borrow_mut().push("enter".into());
            self.on_input = Some(on_input);
            self.on_resize = Some(on_resize);
            Ok(())
        }
        fn stop(&mut self) {
            self.log.borrow_mut().push("leave".into());
        }
        fn drain_input(&mut self, _max_ms: u64, _idle_ms: u64) {}
        fn set_exit_signal_callback(&mut self, _cb: Option<Box<dyn FnMut() + Send + 'static>>) {}
    }

    fn make_scrollback_app(
        cols: u16,
        rows: u16,
    ) -> (App<ScrollbackTerminal>, Rc<RefCell<Vec<String>>>) {
        let (op_tx, _op_rx) = mpsc::unbounded_channel();
        let (client, _events, _conn) = GrpcClient::new("127.0.0.1:1");
        let (input_tx, _input_rx) = mpsc::unbounded_channel();
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut app = App::new(
            ScrollbackTerminal {
                log: Rc::clone(&log),
                cols,
                rows,
                on_input: None,
                on_resize: None,
            },
            Arc::new(client),
            op_tx,
            &CliOptions::default(),
            std::env::temp_dir().join("tui-test-scrollback-settings.json"),
        );
        // `App::start` always sets this before the terminal is entered, and the
        // re-entry path reuses it; tests that stop short of `start` have to
        // stand it up by hand.
        app.input_tx = Some(input_tx);
        app.screen_entered = true;
        (app, log)
    }

    /// The write the app made between leaving and re-entering the alternate
    /// screen — the scrollback batch, if there is one.
    fn batch_between(log: &[String], leave: usize, enter: usize) -> Option<String> {
        let batch: Vec<&String> = log[leave + 1..enter]
            .iter()
            .filter(|entry| entry.starts_with("write:"))
            .collect();
        assert_eq!(batch.len(), 1, "one batch per screen round trip: {log:?}");
        Some(batch[0].to_string())
    }

    /// The batch's screen rows, trimmed — a chat row is padded to the full
    /// terminal width, so a substring match would depend on the test's width.
    fn batch_rows(batch: &str) -> Vec<String> {
        strip_ansi_codes(batch)
            .lines()
            .map(|line| line.trim_end().to_string())
            .collect()
    }

    #[tokio::test]
    async fn a_finished_reply_is_appended_to_the_scrollback_between_leave_and_enter() {
        let (mut app, log) = make_scrollback_app(40, 12);
        app.running = true;
        app.chat.add_message(ChatMessage::new(
            "u1".into(),
            ChatRole::User,
            "scrollback me",
        ));
        app.handle_agent_event(&make_event_with_run("agent_start", "{}", "run-1"));
        app.handle_agent_event(&make_event_with_run(
            "text_chunk",
            r#"{"text":"final reply"}"#,
            "run-1",
        ));
        app.handle_agent_event(&make_event_with_run(
            "agent_end",
            r#"{"state":"completed"}"#,
            "run-1",
        ));
        assert!(
            app.scrollback_pending,
            "a finished reply asks for a scrollback insert"
        );

        app.do_render();

        let events = log.borrow().clone();
        let leave = events
            .iter()
            .position(|e| e == "leave")
            .expect("the alternate screen has to be left for a scrollback write");
        let enter = events
            .iter()
            .position(|e| e == "enter")
            .expect("and entered again right after it");
        assert!(leave < enter);

        let batch = batch_between(&events, leave, enter).unwrap();
        let rows = batch_rows(&batch);
        assert!(
            rows.contains(&" scrollback me".to_string()),
            "the user row is in the scrollback: {rows:?}"
        );
        assert!(
            rows.contains(&" final reply".to_string()),
            "so is the finished reply: {rows:?}"
        );
        // Everything after the re-entry is the repaint: the alternate screen came
        // back blank, so it has to be cleared before it is painted again.
        let after_re_entry = events[enter..].to_vec();
        assert!(
            after_re_entry.iter().any(|e| e.contains("\x1b[2J")),
            "the screen must be repainted from scratch: {after_re_entry:?}"
        );
        assert_eq!(app.history.inserts(), 1);
    }

    #[tokio::test]
    async fn the_scrollback_is_not_written_twice_for_the_same_transcript() {
        let (mut app, log) = make_scrollback_app(40, 12);
        app.running = true;
        app.chat
            .add_message(ChatMessage::new("u1".into(), ChatRole::User, "once only"));
        app.handle_agent_event(&make_event_with_run("agent_start", "{}", "run-1"));
        app.handle_agent_event(&make_event_with_run(
            "text_chunk",
            r#"{"text":"first reply"}"#,
            "run-1",
        ));
        app.handle_agent_event(&make_event_with_run(
            "agent_end",
            r#"{"state":"completed"}"#,
            "run-1",
        ));
        app.do_render();
        assert_eq!(app.history.inserts(), 1);

        // Renders that follow (a footer tick, a keystroke) must not repeat it.
        app.do_render();
        app.request_render(true);
        app.do_render();
        let events = log.borrow().clone();
        assert_eq!(
            events.iter().filter(|e| *e == "leave").count(),
            1,
            "one insert for one reply: {events:?}"
        );
        assert_eq!(app.history.inserts(), 1);

        // A second turn appends only its own rows — the first reply stays where
        // it was, it is not re-emitted under the new one.
        app.chat
            .add_message(ChatMessage::new("u2".into(), ChatRole::User, "again"));
        app.handle_agent_event(&make_event_with_run("agent_start", "{}", "run-2"));
        app.handle_agent_event(&make_event_with_run(
            "text_chunk",
            r#"{"text":"second reply"}"#,
            "run-2",
        ));
        app.handle_agent_event(&make_event_with_run(
            "agent_end",
            r#"{"state":"completed"}"#,
            "run-2",
        ));
        app.do_render();

        let events = log.borrow().clone();
        let leaves: Vec<usize> = events
            .iter()
            .enumerate()
            .filter(|(_, e)| *e == "leave")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(leaves.len(), 2, "one insert per finished turn");
        let enter = events[leaves[1]..]
            .iter()
            .position(|e| e == "enter")
            .map(|i| i + leaves[1])
            .unwrap();
        let rows = batch_rows(&batch_between(&events, leaves[1], enter).unwrap());
        assert!(
            rows.contains(&" second reply".to_string()),
            "the new reply is inserted: {rows:?}"
        );
        assert!(
            !rows.iter().any(|row| row.contains("first reply")),
            "the first reply is not inserted again: {rows:?}"
        );
    }

    #[tokio::test]
    async fn exiting_appends_the_rest_of_the_transcript_to_the_scrollback() {
        // A resumed session never ran, so nothing was inserted while the TUI was
        // up — the exit path is the only thing that puts it in the scrollback.
        let (mut app, log) = make_scrollback_app(40, 12);
        app.running = true;
        app.chat.add_message(ChatMessage::new(
            "u1".into(),
            ChatRole::User,
            "resumed session row",
        ));

        app.stop();

        let events = log.borrow().clone();
        let leave = events
            .iter()
            .position(|e| e == "leave")
            .expect("`Terminal::stop` leaves the alternate screen");
        assert!(
            !events[leave + 1..].iter().any(|e| e == "enter"),
            "nothing to re-enter on the exit path: {events:?}"
        );
        let rows: Vec<String> = events[leave + 1..]
            .iter()
            .filter(|e| e.starts_with("write:"))
            .flat_map(|entry| batch_rows(entry))
            .collect();
        assert!(
            rows.contains(&" resumed session row".to_string()),
            "the transcript lands in the scrollback on the way out: {rows:?}"
        );
    }

    #[tokio::test]
    async fn the_exit_hand_off_is_a_no_op_when_the_scrollback_is_current() {
        // The ordinary case: a run that ended cleanly already inserted its own
        // tail, so the exit path finds nothing new to hand over. Writing the
        // batch again would print the conversation twice, and a terminal cannot
        // be asked to unwrite the copy above it.
        let (mut app, log) = make_scrollback_app(40, 12);
        app.running = true;
        app.chat.add_message(ChatMessage::new(
            "u1".into(),
            ChatRole::User,
            "already handed over",
        ));
        app.handle_agent_event(&make_event_with_run("agent_start", "{}", "run-1"));
        app.handle_agent_event(&make_event_with_run(
            "text_chunk",
            r#"{"text":"final reply"}"#,
            "run-1",
        ));
        app.handle_agent_event(&make_event_with_run(
            "agent_end",
            r#"{"state":"completed"}"#,
            "run-1",
        ));
        app.do_render();
        assert_eq!(
            app.history.inserts(),
            1,
            "the render hands the rows over once"
        );
        let before = log.borrow().len();

        app.stop();

        assert_eq!(
            app.history.inserts(),
            1,
            "the exit must not insert the same transcript again"
        );
        let written_at_exit = log.borrow()[before..].to_vec();
        assert!(
            !written_at_exit
                .iter()
                .any(|e| e.contains("already handed over")),
            "the exit wrote a second copy: {written_at_exit:?}"
        );
    }

    #[tokio::test]
    async fn a_failed_startup_does_not_write_the_transcript_to_stdout() {
        // `App::start` fails before the terminal is entered, and `index.rs` still
        // calls `stop()`. There is no scrollback to fill at that point — the rows
        // would land on stdout, in front of the CLI's error message.
        let (mut app, log) = make_scrollback_app(40, 12);
        app.screen_entered = false;
        app.chat.add_message(ChatMessage::new(
            "u1".into(),
            ChatRole::User,
            "never printed",
        ));

        app.stop();

        let transitions = log.borrow().to_vec();
        assert!(
            !transitions.iter().any(|e| e.contains("never printed")),
            "no transcript on stdout: {transitions:?}"
        );
        assert_eq!(app.history.watermark_len(), 0);
    }

    #[tokio::test]
    async fn scrollback_terminal_exit_callback_setter() {
        // The double has to implement the whole `TerminalIo` surface; this pins
        // the no-op arm the same way `fake_terminal_exit_callback_setter` pins
        // the other double's. `index.rs` is what calls it on the real terminal
        // (to restore the screen from a signal handler).
        let (mut app, _log) = make_scrollback_app(40, 12);
        app.terminal.set_exit_signal_callback(None);
        app.terminal.set_exit_signal_callback(Some(Box::new(|| {})));
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
        let ok = matches!(rx.try_recv(), Ok(UiCmd::InputChanged(ref v)) if v == "h");
        assert!(ok);
        app.on_tick();
        assert!(!app.terminal.writes.borrow().is_empty());
    }

    #[tokio::test]
    async fn autocomplete_stays_down_while_browsing_history() {
        let (mut app, _rx) = make_app(100, 30);
        // Submit a slash command so up-arrow can recall it.
        app.input.set_value("/model deepseek", None);
        app.handle_key("enter");
        app.input.set_value("", None);
        app.handle_key("up"); // recall → browsing history
        assert!(app.input.is_browsing_history());

        // The loop delivers the InputChanged the recall fired: the debounced
        // autocomplete query must be suppressed (a `/…` popup would swallow
        // further up/down/enter presses).
        app.handle_cmd(UiCmd::InputChanged("/model deepseek".into()));
        assert!(app.pending_ac_query.is_none());
        assert!(app.ac_query_deadline.is_none());

        // Even a late AcItems result must not open the popup mid-browse.
        app.handle_cmd(UiCmd::AcItems(vec![
            crate::components::autocomplete::AutocompleteItem {
                value: "/model".into(),
                label: "/model".into(),
                description: None,
            },
        ]));
        assert!(!app.autocomplete.is_visible());

        // Second up press: history advances (popup never intercepted it).
        app.handle_key("up");
        assert!(app.input.is_browsing_history());

        // Down past the draft exits browsing; autocomplete resumes normally.
        app.handle_key("down");
        assert!(!app.input.is_browsing_history());
        app.handle_cmd(UiCmd::InputChanged("/mod".into()));
        assert!(app.pending_ac_query.is_some());
    }

    #[tokio::test]
    async fn ctrl_l_forces_clear_next_render() {
        let (mut app, mut rx) = make_app(100, 30);
        app.running = true;
        app.handle_input("\x0c");
        // Ctrl+L routes through the keybinding manager, which sends the
        // action as a UiCmd (the loop applies it).
        let cmd = rx.try_recv().expect("keybinding action queued");
        assert!(matches!(cmd, UiCmd::KeyAction(KeyAction::ForceClear)));
        app.handle_cmd(cmd);
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
    async fn resize_forces_full_redraw_even_when_size_unchanged() {
        // A resize (SIGWINCH, e.g. a size-changing tmux client attach) is the
        // only reliable in-band signal that the terminal was externally reset.
        // It must force a full redraw (clear screen) so the differential
        // renderer re-anchors its cursor even when the new size is identical
        // (spurious SIGWINCH) — otherwise the relative cursor moves would
        // keep writing growing stream lines onto fresh rows.
        let (mut app, _rx) = running_app(100, 30);
        app.do_render();
        app.terminal.writes.borrow_mut().clear();

        app.request_resize_render();
        // Make the debounce deadline due, then tick once.
        app.resize_deadline = Some(Instant::now() - Duration::from_millis(1));
        app.on_tick();

        let out = render_writes(&app);
        assert!(
            out.contains("\x1b[2J"),
            "resize must force a full redraw with clear, got: {out:?}"
        );
    }

    #[tokio::test]
    async fn focus_in_forces_full_redraw() {
        // Focus-in (\x1b[I) is the standard signal for "the terminal was just
        // re-shown" (tmux attach with focus-events on). It must force a full
        // redraw so a desynced cursor is re-anchored.
        let (mut app, _rx) = running_app(100, 30);
        app.do_render();
        app.terminal.writes.borrow_mut().clear();

        app.handle_input("\x1b[I");
        app.on_tick(); // request_render(true) set render_now; flush it

        let out = render_writes(&app);
        assert!(
            out.contains("\x1b[2J"),
            "focus-in must force a full redraw with clear, got: {out:?}"
        );
    }

    #[tokio::test]
    async fn focus_out_is_ignored() {
        // Focus-out (\x1b[O) carries no redraw obligation; it must be swallowed
        // before key parsing (which would otherwise treat it as an unknown key).
        let (mut app, _rx) = running_app(100, 30);
        app.do_render();
        app.terminal.writes.borrow_mut().clear();
        app.handle_input("\x1b[O");
        assert!(!app.render_now);
        assert!(!app.render_requested);
    }

    #[tokio::test]
    async fn cursor_position_report_forces_redraw_on_divergence() {
        // DSR is the polling net for an attach that reset the cursor with no
        // SIGWINCH and no focus event. A report that matches the row
        // snapshotted at query time is consumed silently; a diverged row
        // forces a full redraw.
        let (mut app, _rx) = running_app(100, 10);
        app.do_render();
        let synced_row = app.hardware_cursor_row;
        app.terminal.writes.borrow_mut().clear();

        // In sync: query snapshots the expected row, the matching report is
        // consumed without forcing a redraw.
        app.query_cursor_position();
        app.handle_input(&format!("\x1b[{};1R", synced_row + 1));
        assert!(!app.render_now);
        assert!(!app.previous_lines.is_empty());

        // Diverged (external reset): the report forces a full redraw.
        app.query_cursor_position();
        app.handle_input(&format!("\x1b[{};1R", synced_row + 2));
        assert!(app.render_now);
        assert!(app.previous_lines.is_empty());

        // A stray report with no pending query is consumed without acting.
        app.render_now = false;
        app.render_requested = false;
        app.handle_input("\x1b[0;1R"); // malformed row too
        assert!(!app.render_now);
    }

    #[tokio::test]
    async fn cursor_report_compares_against_query_snapshot_not_current_row() {
        // The terminal answers the DSR query with the cursor position at the
        // moment it processes the query. A render can run between issuing the
        // query and receiving the answer, moving the cursor; comparing the
        // answer against the *current* row would be a false positive and
        // cause a spurious full redraw every time content grows during
        // streaming. The query-time snapshot must be the comparison basis.
        let (mut app, _rx) = running_app(100, 10);
        app.do_render();
        let snapshot_row = app.hardware_cursor_row;
        app.query_cursor_position(); // snapshot = snapshot_row

        // A render then grows the content and moves the cursor.
        app.chat
            .add_message(ChatMessage::new("x".into(), ChatRole::User, "more"));
        app.do_render();
        assert_ne!(app.hardware_cursor_row, snapshot_row);

        // The answer matches the query-time snapshot → silent no-op.
        app.handle_input(&format!("\x1b[{};1R", snapshot_row + 1));
        assert!(!app.render_now);
    }

    #[tokio::test]
    async fn cursor_report_without_pending_query_is_consumed_silently() {
        // A well-formed DSR report arriving with NO pending query (the
        // cursor_recheck_row snapshot was already taken or never issued) must
        // be consumed without forcing a redraw — exercises the None arm of
        // the `if let Some(expected) = self.cursor_recheck_row.take()` guard.
        let (mut app, _rx) = running_app(100, 10);
        app.do_render();
        app.render_now = false;
        assert!(app.cursor_recheck_row.is_none());
        app.handle_input("\x1b[5;1R"); // well-formed, no pending query
        assert!(!app.render_now, "no pending query → no forced redraw");
    }

    #[tokio::test]
    async fn streaming_tick_rechecks_cursor_position() {
        // While streaming, on_tick issues a DSR cursor query once per
        // CURSOR_RECHECK_INTERVAL and then waits until the next window.
        let (mut app, _rx) = running_app(100, 10);
        app.do_render();
        app.terminal.writes.borrow_mut().clear();
        app.state.streaming = true;
        app.cursor_recheck_at = Instant::now() - Duration::from_millis(1);
        app.on_tick();
        assert!(render_writes(&app).contains("\x1b[6n"));

        // Not queried again immediately (deadline pushed forward).
        app.terminal.writes.borrow_mut().clear();
        app.on_tick();
        assert!(!render_writes(&app).contains("\x1b[6n"));

        // Not queried when not streaming, even past the deadline.
        app.state.streaming = false;
        app.cursor_recheck_at = Instant::now() - Duration::from_millis(1);
        app.terminal.writes.borrow_mut().clear();
        app.on_tick();
        assert!(!render_writes(&app).contains("\x1b[6n"));
    }

    #[tokio::test]
    async fn attach_desync_scrolls_stream_and_full_redraw_reanchors() {
        // End-to-end reproduction of the tmux-attach bug: the differential
        // renderer moves the cursor relative to `hardware_cursor_row`. When a
        // tmux client attach resets the terminal's real cursor without a
        // SIGWINCH (same size), the next diff frame writes its growing stream
        // line on the wrong row — the "A / AB / ABC / ABCD" scrolling symptom.
        // A forced full redraw (focus-in) must re-anchor the real cursor to the
        // app's belief.
        let (mut app, _rx, cursor) = make_tracking_app(100, 10);
        app.running = true;
        app.chat
            .add_message(ChatMessage::new("u".into(), ChatRole::User, "prompt"));
        app.chat
            .add_message(ChatMessage::new("a".into(), ChatRole::Assistant, ""));
        app.do_render();
        // Sanity: after a clean render, the terminal's real cursor matches the
        // app's tracked cursor (the escape sequences landed where the app
        // believed they would).
        assert_eq!(*cursor.borrow(), app.hardware_cursor_row);

        // External reset (attach): the terminal's real cursor moves without the
        // app observing it.
        *cursor.borrow_mut() = 0;

        // Stream a delta through the differential renderer.
        app.chat.append_to_last_message("hello");
        app.do_render();
        // The relative move was applied from the wrong base row, so the real
        // cursor no longer matches the app's belief.
        assert_ne!(
            *cursor.borrow(),
            app.hardware_cursor_row,
            "an externally-reset cursor must desync the differential renderer"
        );

        // The fix: a focus-in (tmux attach with focus-events on) forces a full
        // redraw, re-anchoring the real cursor to the app's belief.
        app.handle_input("\x1b[I");
        app.on_tick();
        assert_eq!(
            *cursor.borrow(),
            app.hardware_cursor_row,
            "a full redraw must re-anchor the real cursor"
        );
    }

    #[tokio::test]
    async fn request_resize_render_sets_debounce_deadline() {
        let (mut app, _rx) = make_app(100, 30);
        app.request_resize_render();
        assert!(app.resize_deadline.is_some());
    }

    // ─── Coverage driving harness ─────────────────────────────────────

    /// How long a bounded wait below may take before it gives up (iterations ×
    /// [`PUMP_INTERVAL_MS`]).
    ///
    /// Every one of these waits settles in well under a second on an idle
    /// machine, but none of them is a fixed cost: they cover a real TCP +
    /// HTTP/2 round trip (`GrpcClient::call` builds a fresh channel per RPC —
    /// see the follow-up in the handoff) against an in-process mock whose
    /// server task needs a runtime worker of its own to answer. On an
    /// oversubscribed host that stretches enormously: with 16 copies of one
    /// live-agent test running under CPU load,
    /// `spawn_paths_succeed_against_live_agent` took 66–77 s where it takes
    /// 1.1 s alone, so the old 30 s budget reported a failure for a round trip
    /// that was merely slow. The margin is deliberate: the suite is a gate that
    /// runs next to whatever else the machine is doing, and a wait that fails
    /// early is worse than a wait that is patient — the assertion it guards is
    /// unchanged either way.
    const PUMP_BUDGET_ITERS: usize = 4_800; // 120 s at 25 ms per iteration
    /// Sleep between polls of a bounded wait.
    const PUMP_INTERVAL_MS: u64 = 25;
    /// Upper bound for [`pump`], which stops early once the command channel has
    /// been quiet for [`PUMP_QUIESCENT_POLLS`] polls (48 s at 25 ms).
    const PUMP_QUIESCE_ITERS: usize = 1_920;
    /// Consecutive empty polls [`pump`] treats as "quiescent".
    const PUMP_QUIESCENT_POLLS: usize = 15;

    /// Feed spawned-task results back into the app until quiescent.
    async fn pump(app: &mut App<FakeTerminal>, op_rx: &mut mpsc::UnboundedReceiver<UiCmd>) {
        let mut idle = 0;
        for _ in 0..PUMP_QUIESCE_ITERS {
            tokio::time::sleep(Duration::from_millis(PUMP_INTERVAL_MS)).await;
            let mut drained = Vec::new();
            while let Ok(cmd) = op_rx.try_recv() {
                drained.push(cmd);
            }
            if drained.is_empty() {
                idle += 1;
                if idle >= PUMP_QUIESCENT_POLLS {
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

    /// Pump until a triggered history page has settled (its RPC answered), so a
    /// paging test waits for the page instead of for a wall-clock window — a
    /// loaded host can take far longer than `pump`'s quiesce window to answer.
    ///
    /// The request is started synchronously by the scroll key (it sets
    /// `loading`), so "not loading" means the answer arrived; when the key was
    /// not supposed to fetch anything, this returns at once and the assertion
    /// after it explains why.
    async fn pump_until_history_settled(
        app: &mut App<FakeTerminal>,
        op_rx: &mut mpsc::UnboundedReceiver<UiCmd>,
    ) {
        for _ in 0..PUMP_BUDGET_ITERS {
            while let Ok(cmd) = op_rx.try_recv() {
                app.handle_cmd(cmd);
            }
            if !app.history_paging.loading {
                return;
            }
            tokio::time::sleep(Duration::from_millis(PUMP_INTERVAL_MS)).await;
        }
        panic!("the history page never settled");
    }

    /// Pump until a system message containing `needle` appears (bounded).
    /// Deterministic alternative to fixed-window pumping for live-agent
    /// flows under parallel load.
    async fn pump_until_msg(
        app: &mut App<FakeTerminal>,
        op_rx: &mut mpsc::UnboundedReceiver<UiCmd>,
        needle: &str,
    ) {
        let mut found = false;
        // [`PUMP_BUDGET_ITERS`] — a loaded host can take far longer than a
        // local machine for the gRPC round trips behind these steps.
        for _ in 0..PUMP_BUDGET_ITERS {
            while let Ok(cmd) = op_rx.try_recv() {
                app.handle_cmd(cmd);
            }
            if system_messages(app).iter().any(|m| m.contains(needle)) {
                found = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(PUMP_INTERVAL_MS)).await;
        }
        assert!(
            found,
            "timed out waiting for system message containing {needle:?}"
        );
    }

    /// Pump until *every* needle is present, then return — the same bounded
    /// patience as [`pump_until_msg`], for calls that submit a burst of
    /// commands back to back.
    ///
    /// [`pump`] returns after 375 ms of channel silence, and that is not the
    /// same thing as "the round trips landed": under load (an instrumented
    /// run, a busy host) the spawned RPC tasks push their `UiCmd` later than
    /// that, and an assertion read straight after `pump` then judges a
    /// transcript that is merely unfinished. Waiting on the condition keeps
    /// the assertion exactly as strong while removing the guess about how long
    /// a round trip takes.
    async fn pump_until_all(
        app: &mut App<FakeTerminal>,
        op_rx: &mut mpsc::UnboundedReceiver<UiCmd>,
        needles: &[&str],
    ) {
        let mut missing = needles.to_vec();
        for _ in 0..PUMP_BUDGET_ITERS {
            while let Ok(cmd) = op_rx.try_recv() {
                app.handle_cmd(cmd);
            }
            missing.retain(|needle| !system_messages(app).iter().any(|m| m.contains(needle)));
            if missing.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(PUMP_INTERVAL_MS)).await;
        }
        assert!(missing.is_empty(), "timed out waiting for {missing:?}");
    }

    /// Pump until the top overlay's rendered text contains `needle` (bounded).
    /// The panel-based commands report into their panel rather than the chat,
    /// so `pump_until_msg` cannot see their result.
    async fn pump_until_overlay_text(
        app: &mut App<FakeTerminal>,
        op_rx: &mut mpsc::UnboundedReceiver<UiCmd>,
        needle: &str,
    ) {
        let mut found = false;
        for _ in 0..PUMP_BUDGET_ITERS {
            while let Ok(cmd) = op_rx.try_recv() {
                app.handle_cmd(cmd);
            }
            if app.get_top_overlay_index().is_some() && top_overlay_text(app, 90).contains(needle) {
                found = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(PUMP_INTERVAL_MS)).await;
        }
        assert!(found, "timed out waiting for {needle:?} in the top overlay");
    }

    /// Pump until the skills panel's status row contains `needle`.
    ///
    /// The row is one stored string that the renderer truncates at the pane
    /// width, so a hint past the cut (the retry advice sits there) is invisible
    /// to `pump_until_overlay_text`; and while the panel is open the failure is
    /// reported *only* in the panel, so `pump_until_msg` would wait for a
    /// transcript line that is deliberately not written.
    async fn pump_until_skills_row(
        app: &mut App<FakeTerminal>,
        op_rx: &mut mpsc::UnboundedReceiver<UiCmd>,
        needle: &str,
    ) {
        let mut found = false;
        for _ in 0..PUMP_BUDGET_ITERS {
            while let Ok(cmd) = op_rx.try_recv() {
                app.handle_cmd(cmd);
            }
            if skills_row(app).is_some_and(|row| row.contains(needle)) {
                found = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(PUMP_INTERVAL_MS)).await;
        }
        assert!(
            found,
            "timed out waiting for {needle:?} in the skills status row"
        );
    }

    /// Pump until the overlay stack is empty (same budget as pump_until_msg):
    /// a closing overlay is delivered through the command channel.
    async fn pump_until_no_overlay(
        app: &mut App<FakeTerminal>,
        op_rx: &mut mpsc::UnboundedReceiver<UiCmd>,
    ) {
        let mut cleared = false;
        for _ in 0..PUMP_BUDGET_ITERS {
            while let Ok(cmd) = op_rx.try_recv() {
                app.handle_cmd(cmd);
            }
            if app.overlay_stack.is_empty() {
                cleared = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(PUMP_INTERVAL_MS)).await;
        }
        assert!(cleared, "timed out waiting for the overlay to close");
    }

    /// Pump until an overlay is on the stack (same budget as pump_until_msg).
    async fn pump_until_overlay(
        app: &mut App<FakeTerminal>,
        op_rx: &mut mpsc::UnboundedReceiver<UiCmd>,
    ) {
        let mut found = false;
        for _ in 0..PUMP_BUDGET_ITERS {
            while let Ok(cmd) = op_rx.try_recv() {
                app.handle_cmd(cmd);
            }
            if !app.overlay_stack.is_empty() {
                found = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(PUMP_INTERVAL_MS)).await;
        }
        assert!(found, "timed out waiting for an overlay");
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

    /// The whole transcript as plain text, for substring assertions.
    fn plain_text(app: &App<FakeTerminal>) -> String {
        app.chat
            .plain_messages()
            .into_iter()
            .map(|(_, content)| content)
            .collect::<Vec<_>>()
            .join("\n")
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
                r#"{"id":"s1","cwd":"/tmp/a","updatedAtMs": 20455000,"model":"m1","sessionName":"first"}"#,
            ))
            .expect("session"),
            serde_json::from_value(json_parse(
                r#"{"id":"s2","cwd":"/tmp/a","updatedAtMs": 20454000,"model":"m1","parentSessionId":"s1"}"#,
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

    #[tokio::test]
    async fn compaction_blocks_prompts_preserves_drafts_and_reports_all_outcomes() {
        for (terminal, payload, expected) in [
            (
                "compaction_committed",
                r#"{"tokens_before":33064,"tokens_after":11900}"#,
                "33064 → 11900",
            ),
            (
                "compaction_failed",
                r#"{"error":"summary failed"}"#,
                "Compact failed: summary failed",
            ),
            ("compaction_unchanged", r#"{}"#, "not needed"),
            (
                "compaction_unchanged",
                r#"{"already_compacted":true}"#,
                "no new content",
            ),
        ] {
            let (mut app, _rx) = make_app(100, 30);
            app.state.compaction_requested = true;
            app.handle_submit("before acknowledgement");
            assert_eq!(app.input.get_value(), "before acknowledgement");
            assert!(!app.state.streaming);
            app.handle_agent_event(&make_event("compaction_started", "{}"));
            app.handle_submit("my next draft");
            assert_eq!(app.input.get_value(), "my next draft");
            assert!(!app.state.streaming);
            app.handle_agent_event(&make_event(terminal, payload));
            assert!(!app.state.compacting);
            assert!(!app.state.compaction_requested);
            assert!(last_system(&app).contains(expected));
            // A fast worker can finish before its RPC acknowledgement arrives.
            app.handle_cmd(UiCmd::CompactDone {
                session_id: app.state.session_id.clone(),
                result: Ok("accepted".into()),
            });
            assert!(last_system(&app).contains(expected));
            assert_eq!(app.input.get_value(), "my next draft");
            app.handle_submit("send after completion");
            assert!(app.state.streaming);
        }
    }

    #[tokio::test]
    async fn compaction_admission_locks_immediately_and_recovers_a_missed_terminal() {
        let (mut app, _rx) = make_app(100, 30);
        let mut idle = sample_state();
        idle.is_streaming = false;
        app.apply_refresh_state(idle.clone());
        app.handle_submit("/compact");
        assert!(app.state.compaction_requested);
        app.handle_submit("keep my draft");
        assert_eq!(app.input.get_value(), "keep my draft");
        assert!(!app.state.streaming);
        app.handle_cmd(UiCmd::CompactDone {
            session_id: app.state.session_id.clone(),
            result: Ok("accepted".into()),
        });
        assert!(!app.state.compaction_requested);
        assert!(app.state.compacting);
        app.apply_refresh_state(idle);
        assert!(
            !app.state.compacting,
            "reattach must release a missed terminal fence"
        );
        let mut foreign = make_event("compaction_started", "{}");
        foreign.session_id = Some("other-session".into());
        app.handle_agent_event(&foreign);
        assert!(!app.state.compacting);
    }

    #[tokio::test]
    async fn compaction_refresh_ignores_stale_snapshots_and_recovers_on_reattach() {
        let (mut app, _rx) = make_app(100, 30);
        let mut state = sample_state();
        state.is_compacting = true;
        app.apply_refresh_state(state.clone());
        assert!(app.state.compacting);
        let revision = app.state.compaction_revision;
        app.handle_agent_event(&make_event("compaction_committed", "{}"));
        app.handle_cmd(UiCmd::RefreshCompleted {
            result: Ok(state),
            session_id: app.state.session_id.clone(),
            compaction_revision: revision,
        });
        assert!(
            !app.state.compacting,
            "old get_state must not re-lock sending after completion"
        );
        app.state.compaction_requested = true;
        let mut other = sample_state();
        other.session_id = "other-session".into();
        app.apply_refresh_state(other);
        assert!(!app.state.compaction_requested);
    }

    #[tokio::test]
    async fn compaction_checkpoints_are_visible_in_reloaded_history() {
        let (mut app, _rx) = make_app(100, 30);
        app.apply_history_page(
            "s1",
            Ok(json_parse(
                r#"{"entries":[{"id":"cp","kind":"compaction","role":"system","checkpoint":{"tokensBefore":33064,"tokensAfter":11900}}]}"#,
            )),
        );
        assert!(last_system(&app).contains("33064 → 11900"));
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
        app.handle_cmd(UiCmd::ThinkingCycled(Ok(json_parse(
            r#"{"level":"xhigh"}"#,
        ))));
        assert_eq!(app.state.thinking, "xhigh");
        app.handle_cmd(UiCmd::ThinkingCycled(Err("x".into())));

        // CompactDone ok/err.
        app.state.compaction_requested = true;
        app.handle_cmd(UiCmd::CompactDone {
            session_id: app.state.session_id.clone(),
            result: Ok("done".into()),
        });
        assert!(last_system(&app).contains("Context compaction request accepted"));
        app.handle_cmd(UiCmd::CompactDone {
            session_id: app.state.session_id.clone(),
            result: Err("bad".into()),
        });
        assert!(last_system(&app).contains("Compact failed"));

        // ReloadDone ok (with skills + contextFiles) / err.
        app.handle_cmd(UiCmd::ReloadDone {
            result: Ok(json_parse(
                r#"{"skills":["a","b"],"contextFiles":["AGENTS.md"]}"#,
            )),
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

        // StatusLoaded: both ok / models err / state err, with the usage ledger
        // that `/stats` used to own folded into the report.
        app.handle_cmd(UiCmd::StatusLoaded {
            state: Ok(sample_state()),
            models: Ok(sample_models()),
            stats: Ok(json_parse(
                r#"{"sessionFile":"/tmp/s.jsonl","totalMessages":3,"userMessages":2,
                     "assistantMessages":1,"toolCalls":1,"toolResults":1,
                     "tokens":{"input":10,"output":5,"cacheRead":3,"cacheWrite":1,"total":15},
                     "cost":1.5}"#,
            )),
        });
        let report = last_system(&app);
        assert!(report.contains("Queries"), "{report}");
        // The counters that used to need `/stats` are now in this one report.
        assert!(report.contains("Context: "), "{report}");
        assert!(report.contains("file: /tmp/s.jsonl"), "{report}");
        assert!(report.contains("messages: 3"), "{report}");
        assert!(report.contains("user/assistant: 2/1"), "{report}");
        assert!(report.contains("tool calls/results: 1/1"), "{report}");
        assert!(report.contains("tokens cache read/write: 3/1"), "{report}");
        assert!(report.contains("tokens total: 15"), "{report}");
        assert!(report.contains("cost: 1.5"), "{report}");

        app.handle_cmd(UiCmd::StatusLoaded {
            state: Ok(sample_state()),
            models: Err("m".into()),
            stats: Ok(json_parse(r#"{"cost":0}"#)),
        });
        app.handle_cmd(UiCmd::StatusLoaded {
            state: Err("s".into()),
            models: Ok(vec![]),
            stats: Err("t".into()),
        });
        assert!(last_system(&app).contains("Failed to get status"));

        // A dead ledger must not swallow the rest of the report: the state is
        // still useful, so the section is replaced by a named failure.
        app.handle_cmd(UiCmd::StatusLoaded {
            state: Ok(sample_state()),
            models: Ok(sample_models()),
            stats: Err("ledger down".into()),
        });
        let report = last_system(&app);
        assert!(report.contains("Queries"), "{report}");
        assert!(
            report.contains("Usage ledger unavailable (get_session_stats failed)."),
            "{report}"
        );

        // InitialPromptDone is a no-op.
        app.handle_cmd(UiCmd::InitialPromptDone(Ok(crate::rpc::types::RunAck {
            run_id: "r".into(),
            run_epoch: 1,
            accepted_state: "running".into(),
            run_sequence: None,
            queue_position: None,
        })));

        let _ = &mut rx;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn handle_cmd_session_flow_variants() {
        let (mut app, mut rx) = make_app(100, 30);
        app.state.session_id = "current".into();

        // SessionSwitched err.
        app.handle_cmd(UiCmd::SessionSwitched {
            target: "target".into(),
            switched: false,
            result: Err("nope".into()),
            state: None,
            history: Ok(Value::Null),
            label: "l".into(),
        });
        assert!(last_system(&app).contains("Failed to switch session"));
        // ok with state+messages.
        app.handle_cmd(UiCmd::SessionSwitched {
            target: "target".into(),
            switched: true,
            result: Ok(()),
            state: Some(sample_state()),
            history: Ok(json_parse(
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
        // Switch flow fails against the dead client; the failure is async, so
        // wait for the message instead of a fixed pump window.
        pump_until_msg(&mut app, &mut rx, "Failed to switch session").await;

        // ForkSelected → ForkDone spawn chain (fails against dead client).
        app.handle_cmd(UiCmd::ForkSelected {
            item: SelectItem {
                value: "e1".into(),
                label: "entry".into(),
                description: None,
            },
        });
        pump_until_msg(&mut app, &mut rx, "Failed to fork").await;

        // ForkDone direct: cancelled, ok-not-cancelled, err.
        app.handle_cmd(UiCmd::ForkDone {
            fork_result: Ok(json_parse(r#"{"cancelled":true}"#)),
            state: None,
            history: Ok(Value::Null),
            label: "l".into(),
        });
        app.handle_cmd(UiCmd::ForkDone {
            fork_result: Ok(json_parse(r#"{"cancelled":false}"#)),
            state: Some(sample_state()),
            history: Ok(json_parse(r#"{"messages":[]}"#)),
            label: "l".into(),
        });
        assert!(last_system(&app).contains("Forked from l."));
        app.handle_cmd(UiCmd::ForkDone {
            fork_result: Err("x".into()),
            state: None,
            history: Ok(Value::Null),
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
            history: Ok(Value::Null),
        });
        app.handle_cmd(UiCmd::CloneDone {
            result: Ok(json_parse(r#"{"cancelled":false}"#)),
            state: Some(sample_state()),
            history: Ok(json_parse(r#"{"messages":[]}"#)),
        });
        assert!(last_system(&app).contains("Session cloned"));
        app.handle_cmd(UiCmd::CloneDone {
            result: Err("x".into()),
            state: None,
            history: Ok(Value::Null),
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
        pump_until_msg(&mut app, &mut rx, "Failed to set model").await;

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
        app.cached_models = sample_models().iter().map(|m| m.full_id()).collect();
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
        app.handle_cmd(UiCmd::AcItems(vec![
            crate::components::autocomplete::AutocompleteItem {
                value: "/model".into(),
                label: "/model".into(),
                description: None,
            },
        ]));
        assert!(app.autocomplete.is_visible());
        app.handle_cmd(UiCmd::AcItems(vec![]));
        assert!(!app.autocomplete.is_visible());
    }

    /// A switch whose transcript fetch failed must not leave the previous
    /// session's conversation on screen: the agent-side switch already
    /// happened, so those messages would read as this session's history while
    /// a prompt typed into the box goes to the new one.
    #[tokio::test(flavor = "multi_thread")]
    async fn session_switch_replaces_the_transcript_even_when_loading_it_fails() {
        let (mut app, _rx) = make_app(100, 30);
        app.state.session_id = "old".into();
        app.chat.add_message(ChatMessage::new(
            "old-1".into(),
            ChatRole::User,
            "previous-session-question",
        ));
        app.handle_cmd(UiCmd::SessionSwitched {
            target: "target".into(),
            switched: true,
            result: Ok(()),
            state: Some(sample_state()),
            history: Err("boom".into()),
            label: "target".into(),
        });
        let texts: Vec<String> = app
            .chat
            .plain_messages()
            .iter()
            .map(|(_, c)| c.clone())
            .collect();
        assert!(
            !texts
                .iter()
                .any(|t| t.contains("previous-session-question")),
            "old transcript survived a failed load: {texts:?}"
        );
        // The failure is named instead of silently showing an empty session.
        let report = last_system(&app);
        assert!(report.contains("Switched to session: target"), "{report}");
        assert!(report.contains("could not be loaded"), "{report}");
    }

    /// `get_state` is a second call and can fail on its own. The session
    /// identity still has to move with the switch — otherwise the drafts,
    /// refreshes and event filtering stay keyed to the session just left (and
    /// its saved draft gets restored into the new session's input box).
    #[tokio::test(flavor = "multi_thread")]
    async fn session_switch_without_state_still_adopts_the_target() {
        let (mut app, _rx) = make_app(100, 30);
        app.state.session_id = "old".into();
        app.input.set_value("old draft", None);
        app.save_session_input();
        app.state.session_id = "target".into();
        app.input.set_value("target draft", None);
        app.save_session_input();
        app.state.session_id = "old".into();
        app.input.set_value("", None);

        app.handle_cmd(UiCmd::SessionSwitched {
            target: "target".into(),
            switched: true,
            result: Ok(()),
            state: None,
            history: Ok(json_parse(r#"{"messages":[]}"#)),
            label: "target".into(),
        });
        assert_eq!(app.state.session_id, "target");
        assert_eq!(app.client.get_current_session_id(), "target");
        assert_eq!(app.input.get_value(), "target draft");
    }

    /// The new transcript opens at its tail: keeping the previous session's
    /// scroll offset would show the middle of the conversation the user just
    /// switched to (and, with `auto_scroll` off, never follow its output).
    #[tokio::test(flavor = "multi_thread")]
    async fn session_switch_re_anchors_the_chat_view() {
        let (mut app, _rx) = make_app(100, 12);
        app.state.session_id = "old".into();
        for i in 0..80 {
            app.chat.add_message(ChatMessage::new(
                format!("old-{i}"),
                ChatRole::User,
                &format!("old line {i}"),
            ));
        }
        app.chat.set_viewport_height(8);
        let _ = app.chat.render(100);
        app.chat.scroll_up(30);
        assert!(!app.chat.is_at_bottom());

        let new_messages: Vec<String> = (0..60)
            .map(|i| format!(r#"{{"id":"n{i}","role":"user","content":"new line {i}"}}"#))
            .collect();
        app.handle_cmd(UiCmd::SessionSwitched {
            target: "target".into(),
            switched: true,
            result: Ok(()),
            state: Some(sample_state()),
            history: Ok(json_parse(&format!(
                r#"{{"messages":[{}]}}"#,
                new_messages.join(",")
            ))),
            label: "target".into(),
        });
        let _ = app.chat.render(100);
        assert!(
            app.chat.is_at_bottom(),
            "the new session kept the previous scroll offset"
        );
    }

    /// Two picks can overlap (the menu stays open until the first result
    /// lands). The older flow's transcript must not replace the one the user
    /// actually asked for last, and the client — which the older RPC may have
    /// re-pointed at its own target — has to follow the winning switch.
    #[tokio::test(flavor = "multi_thread")]
    async fn superseded_session_switch_result_is_dropped() {
        let (mut app, _rx) = make_app(100, 30);
        app.state.session_id = "old".into();
        // Both picks are in flight; the newest one is "second".
        app.latest_session_switch = Some(SessionSwitchRequest {
            from: "old".into(),
            target: "second".into(),
        });
        // The first flow's RPC landed last and left the client on "first".
        app.client.set_current_session_id("first");

        app.handle_cmd(UiCmd::SessionSwitched {
            target: "first".into(),
            switched: true,
            result: Ok(()),
            state: Some(sample_state()),
            history: Ok(json_parse(
                r#"{"messages":[{"id":"f1","role":"user","blocks":[{"kind":"text","text":"first-session-question"}]}]}"#,
            )),
            label: "first".into(),
        });
        let texts: Vec<String> = app
            .chat
            .plain_messages()
            .iter()
            .map(|(_, c)| c.clone())
            .collect();
        assert!(
            !texts.iter().any(|t| t.contains("first-session-question")),
            "a superseded switch replaced the transcript: {texts:?}"
        );
        assert!(texts.iter().all(|t| !t.contains("Switched to session")));
        assert_eq!(
            app.state.session_id, "old",
            "identity moved on a stale pick"
        );

        // The winning result then lands and is applied — which is also what
        // puts the client back on the target the user asked for last.
        app.handle_cmd(UiCmd::SessionSwitched {
            target: "second".into(),
            switched: true,
            result: Ok(()),
            state: Some(sample_state()),
            history: Ok(json_parse(
                r#"{"messages":[{"id":"s1","role":"user","blocks":[{"kind":"text","text":"second-session-question"}]}]}"#,
            )),
            label: "second".into(),
        });
        let texts: Vec<String> = app
            .chat
            .plain_messages()
            .iter()
            .map(|(_, c)| c.clone())
            .collect();
        assert!(texts.iter().any(|t| t.contains("second-session-question")));
        assert!(texts.iter().all(|t| !t.contains("first-session-question")));
        assert_eq!(app.state.session_id, "second");
        assert_eq!(app.client.get_current_session_id(), "second");
        assert_eq!(
            app.latest_session_switch
                .as_ref()
                .map(|s| s.target.as_str()),
            Some("second")
        );
    }

    /// The mirror ordering: the winner's RPC landed first, so its result is
    /// applied before the older request's RPC lands and re-points the client.
    /// The late result must neither replace the transcript nor leave the client
    /// on a session the app is not showing.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_late_older_switch_result_cannot_take_over() {
        let (mut app, _rx) = make_app(100, 30);
        app.state.session_id = "second".into();
        app.client.set_current_session_id("second");
        app.chat.add_message(ChatMessage::new(
            "s1".into(),
            ChatRole::User,
            "second-session-question",
        ));
        app.latest_session_switch = Some(SessionSwitchRequest {
            from: "old".into(),
            target: "second".into(),
        });
        // The older request's RPC landed last.
        app.client.set_current_session_id("first");

        app.handle_cmd(UiCmd::SessionSwitched {
            target: "first".into(),
            switched: true,
            result: Ok(()),
            state: Some(sample_state()),
            history: Ok(json_parse(
                r#"{"messages":[{"id":"f1","role":"user","blocks":[{"kind":"text","text":"first-session-question"}]}]}"#,
            )),
            label: "first".into(),
        });
        let texts: Vec<String> = app
            .chat
            .plain_messages()
            .iter()
            .map(|(_, c)| c.clone())
            .collect();
        assert!(texts.iter().any(|t| t.contains("second-session-question")));
        assert!(!texts.iter().any(|t| t.contains("first-session-question")));
        assert_eq!(app.state.session_id, "second");
        assert_eq!(
            app.client.get_current_session_id(),
            "second",
            "the late result left the client on the wrong session"
        );
    }

    /// A declined switch (`cancelled`) changes nothing on the wire: the
    /// transcript on screen is still the current session's and must stay.
    #[tokio::test(flavor = "multi_thread")]
    async fn declined_session_switch_keeps_the_transcript() {
        let (mut app, _rx) = make_app(100, 30);
        app.state.session_id = "old".into();
        app.chat.add_message(ChatMessage::new(
            "old-1".into(),
            ChatRole::User,
            "current-session-question",
        ));
        app.handle_cmd(UiCmd::SessionSwitched {
            target: "target".into(),
            switched: false,
            result: Ok(()),
            state: None,
            history: Ok(Value::Null),
            label: "target".into(),
        });
        let texts: Vec<String> = app
            .chat
            .plain_messages()
            .iter()
            .map(|(_, c)| c.clone())
            .collect();
        assert!(texts.iter().any(|t| t.contains("current-session-question")));
        assert!(last_system(&app).contains("Session switch declined: target"));
        assert_eq!(app.state.session_id, "old");
    }

    /// A frame already in flight from the session we just left must not touch
    /// the new transcript: its `text_chunk` would append to the last assistant
    /// bubble of this one, and `agent_end` would overwrite it.
    #[tokio::test(flavor = "multi_thread")]
    async fn events_from_the_previous_session_are_ignored_after_a_switch() {
        let (mut app, _rx) = make_app(100, 30);
        app.state.session_id = "target".into();
        app.client.set_current_session_id("target");
        app.chat.add_message(ChatMessage::new(
            "a1".into(),
            ChatRole::Assistant,
            "new session answer",
        ));

        let mut stale = make_event("text_chunk", r#"{"text":" old session text"}"#);
        stale.session_id = Some("previous".into());
        app.handle_agent_event(&stale);
        assert!(!app
            .chat
            .plain_messages()
            .iter()
            .any(|(_, c)| c.contains("old session text")));

        // `agent_end` carries the whole reply and would replace the bubble.
        let mut stale_end = make_event(
            "agent_end",
            r#"{"text":"old session reply","state":"completed"}"#,
        );
        stale_end.session_id = Some("previous".into());
        app.handle_agent_event(&stale_end);
        let texts: Vec<String> = app
            .chat
            .plain_messages()
            .iter()
            .map(|(_, c)| c.clone())
            .collect();
        assert!(texts.iter().any(|t| t.contains("new session answer")));
        assert!(!texts.iter().any(|t| t.contains("old session reply")));

        // The current session's own events still land.
        let mut own = make_event("text_chunk", r#"{"text":" more"}"#);
        own.session_id = Some("target".into());
        app.handle_agent_event(&own);
        assert!(app
            .chat
            .plain_messages()
            .iter()
            .any(|(_, c)| c.contains("new session answer more")));
    }

    /// `/new` (or a fork) while a pick is in flight: the user is in a fresh
    /// session now, so the late transcript must not drag the app back to the
    /// session they left — the client would follow it and the next prompt
    /// would land there.
    #[tokio::test(flavor = "multi_thread")]
    async fn switch_result_is_dropped_when_another_session_took_over() {
        let (mut app, _rx) = make_app(100, 30);
        app.state.session_id = "old".into();
        app.latest_session_switch = Some(SessionSwitchRequest {
            from: "old".into(),
            target: "picked".into(),
        });
        // `/new` completed first: identity and client moved to the new session.
        let mut new_state = sample_state();
        new_state.session_id = "fresh".into();
        app.handle_cmd(UiCmd::NewSessionDone {
            result: Ok(json_parse(r#"{"sessionId":"fresh"}"#)),
            state: Some(new_state),
        });
        assert_eq!(app.state.session_id, "fresh");

        app.handle_cmd(UiCmd::SessionSwitched {
            target: "picked".into(),
            switched: true,
            result: Ok(()),
            state: Some(sample_state()),
            history: Ok(json_parse(
                r#"{"messages":[{"id":"p1","role":"user","blocks":[{"kind":"text","text":"picked-session-question"}]}]}"#,
            )),
            label: "picked".into(),
        });
        assert_eq!(
            app.state.session_id, "fresh",
            "a late pick hijacked the new session"
        );
        assert_eq!(
            app.client.get_current_session_id(),
            "fresh",
            "the client was dragged back to the abandoned pick"
        );
        let texts: Vec<String> = app
            .chat
            .plain_messages()
            .iter()
            .map(|(_, c)| c.clone())
            .collect();
        assert!(!texts.iter().any(|t| t.contains("picked-session-question")));
    }

    /// The window title names the session: switching must not leave the
    /// previous session's name there.
    #[tokio::test(flavor = "multi_thread")]
    async fn session_switch_replaces_the_name_in_the_window_title() {
        let (mut app, _rx) = make_app(100, 30);
        app.state.session_id = "old".into();
        app.state.session_name = Some("previous name".into());
        app.state.cwd = "/tmp/project".into();
        app.state.model = "openai/gpt-4o".into();

        let mut state = sample_state();
        state.session_name = Some("target name".into());
        app.handle_cmd(UiCmd::SessionSwitched {
            target: "target".into(),
            switched: true,
            result: Ok(()),
            state: Some(state),
            history: Ok(json_parse(r#"{"messages":[]}"#)),
            label: "target".into(),
        });
        assert_eq!(app.state.session_name.as_deref(), Some("target name"));
        let writes = terminal_writes(&app);
        assert!(writes.contains("target name"), "{writes:?}");
        assert!(!writes.contains("previous name"), "{writes:?}");
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
        app.handle_agent_event(&make_event(
            "tool_delta",
            r#"{"tool_id":"t1","text":"part"}"#,
        ));
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
        app.handle_agent_event(&make_event(
            "auto_compaction_changed",
            r#"{"enabled":false}"#,
        ));
        assert!(!app.state.auto_compaction_enabled);
        app.handle_agent_event(&make_event("session_name_changed", r#"{"name":"n"}"#));
        app.handle_agent_event(&make_event("permission_level_changed", "{}"));
        app.handle_agent_event(&make_event("tools_changed", "{}"));
        app.handle_agent_event(&make_event("sandbox_policy_changed", "{}"));
        pump(&mut app, &mut rx).await;

        // config_reloaded with skills + context files.
        app.handle_agent_event(&make_event(
            "config_reloaded",
            r#"{"skills":["b","a"],"contextFiles":["CLAUDE.md"]}"#,
        ));
        assert_eq!(app.state.skills, vec!["a", "b"]);
        assert!(last_system(&app).contains("Config reloaded: 2 skills, CLAUDE.md"));
        // …and with empty lists.
        app.handle_agent_event(&make_event(
            "config_reloaded",
            r#"{"skills":[],"contextFiles":[]}"#,
        ));
        assert!(last_system(&app).contains("no context files"));

        // agent_end with terminal text.
        app.handle_agent_event(&make_event_with_run(
            "agent_end",
            r#"{"text":"final answer"}"#,
            "r1",
        ));
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

        // ShowSessions spawns a load (async: wait for the failure message).
        app.handle_key_action(KeyAction::ShowSessions);
        pump_until_msg(&mut app, &mut rx, "Failed to load sessions").await;

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
                                                       // A listener returning None passes input through untouched.
        app.handle_input("other");
        app.input_listeners.clear();
    }

    // ─── Slash commands ───────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn slash_commands_matrix() {
        let (mut app, mut rx) = make_app(100, 30);

        // Empty submit is a no-op.
        app.handle_cmd(UiCmd::Submit("   ".into()));

        for cmd in [
            "/help",
            "/sessions",
            "/tree",
            "/fork",
            "/clone",
            "/new",
            "/compact",
            "/reload",
            "/scoped-models",
            "/status",
            "/stop",
            "/export",
            "/import",
        ] {
            app.handle_cmd(UiCmd::Submit(cmd.into()));
        }
        pump(&mut app, &mut rx).await;
        // `/export` now really calls the agent (which is unreachable here),
        // while `/import` stays a documented stub.
        assert!(system_messages(&app)
            .iter()
            .any(|m| m.contains("Failed to export session")));
        assert!(system_messages(&app)
            .iter()
            .any(|m| m.contains("import is not available")));

        // /model with arg (dead client → fails), and selector path.
        app.handle_cmd(UiCmd::Submit("/model sonnet".into()));
        pump_until_msg(&mut app, &mut rx, "Failed to set model").await;
        app.handle_cmd(UiCmd::Submit("/model".into()));
        pump_until_msg(&mut app, &mut rx, "Failed to load models").await;

        // /model while streaming → refused.
        app.state.streaming = true;
        app.handle_cmd(UiCmd::Submit("/model x".into()));
        assert!(last_system(&app).contains("Cannot change model while agent is streaming"));
        app.state.streaming = false;

        // /name with and without arg.
        app.handle_cmd(UiCmd::Submit("/name".into()));
        assert!(last_system(&app).contains("Usage: /name"));
        app.handle_cmd(UiCmd::Submit("/name my session".into()));
        pump_until_msg(&mut app, &mut rx, "Failed to set session name").await;

        // /cwd with no arg is not a command — it becomes a prompt.
        let before = app.chat.plain_messages().len();
        app.handle_cmd(UiCmd::Submit("/cwd".into()));
        assert!(app.chat.plain_messages().len() > before);
        app.state.streaming = false; // the prompt above set it
        app.handle_cmd(UiCmd::Submit("/cwd /tmp".into()));
        pump_until_msg(&mut app, &mut rx, "Failed to change directory").await;
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
        pump_until_msg(&mut app, &mut rx, "Failed to approve").await;
        app.handle_cmd(UiCmd::Submit("/reject req-2".into()));
        pump_until_msg(&mut app, &mut rx, "Failed to reject").await;

        // /cancel with and without arg.
        app.handle_cmd(UiCmd::Submit("/cancel".into()));
        assert!(last_system(&app).contains("Usage: /cancel"));
        app.handle_cmd(UiCmd::Submit("/cancel run-9".into()));
        pump_until_msg(&mut app, &mut rx, "Failed to cancel queued run").await;

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

    // ─── Input handling round 2 ───────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn handle_input_overlays_and_release_filtering() {
        // Serializes with terminal_image's cell-dimension tests.
        let _guard = crate::test_env::lock();
        let (mut app, mut rx) = make_app(100, 30);
        let _ = &mut rx;

        // Cell-size response is consumed (save/restore the global dims —
        // they feed the image renderer's row math).
        let saved_dims = crate::terminal_image::get_cell_dimensions();
        app.handle_input("\x1b[6;36;119t");
        app.handle_input("\x1b[6;0;0t"); // zero dims — consumed, no set
        app.handle_input("not-a-response"); // passes through to a key parse
        crate::terminal_image::set_cell_dimensions(saved_dims);

        // Paste with an overlay open goes to the overlay.
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Selector,
        });
        app.handle_input("\x1b[200~x\x1b[201~");
        // Unterminated paste is swallowed without effect.
        app.handle_input("\x1b[200~never closed");
        app.handle_cmd(UiCmd::OverlayCancel);

        // Key release with an overlay focused: the overlay doesn't want
        // release events → dropped.
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Selector,
        });
        app.handle_input("\x1b[97;1:3u");
        app.handle_cmd(UiCmd::OverlayCancel);

        // Printable char with an overlay goes to the overlay, not the input.
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Selector,
        });
        app.input.set_value("", None);
        app.handle_input("z");
        assert!(app.input.get_value().is_empty());
        app.handle_cmd(UiCmd::OverlayCancel);
        app.handle_input("z");
        assert_eq!(app.input.get_value(), "z");

        // Escape while autocomplete is visible hides it (editor untouched).
        app.input.set_value("/m", None);
        app.autocomplete
            .show(vec![crate::components::autocomplete::AutocompleteItem {
                value: "/model".into(),
                label: "/model".into(),
                description: None,
            }]);
        app.handle_key("escape");
        assert!(!app.autocomplete.is_visible());
        assert_eq!(app.input.get_value(), "/m");

        // Autocomplete navigation + enter applies the selection.
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
        app.handle_key("enter"); // applies "/new" into the input
        assert!(app.input.get_value().contains("new"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn autocomplete_selection_variants() {
        let (mut app, mut rx) = make_app(100, 30);

        // InputChanged drives the debounced query state.
        app.handle_cmd(UiCmd::InputChanged("/mo".into()));
        assert!(app.pending_ac_query.is_some());

        // trigger_autocomplete against the (empty) manager.
        app.trigger_autocomplete();

        // apply_autocomplete_selection with nothing shown → no-op.
        app.apply_autocomplete_selection();

        // With an item but no active context → the value replaces input.
        app.autocomplete
            .show(vec![crate::components::autocomplete::AutocompleteItem {
                value: "/model".into(),
                label: "/model".into(),
                description: None,
            }]);
        app.apply_autocomplete_selection();
        assert_eq!(app.input.get_value(), "/model");
        assert!(!app.autocomplete.is_visible());

        // With an active context through the slash provider: /mo + Tab
        // completes the token and preserves overlap.
        app.input.set_value("/mo", None);
        app.trigger_autocomplete();
        pump(&mut app, &mut rx).await; // deliver AcItems
                                       // The slash provider matched → items shown.
        assert!(app.autocomplete.is_visible());
        // Move to /model and accept.
        app.apply_autocomplete_selection();
        let v = app.input.get_value().to_string();
        assert!(v.starts_with('/'));
    }

    // ─── Overlays plumbing ────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn overlay_stack_lifecycle() {
        let (mut app, mut rx) = make_app(100, 30);

        // show_select_overlay via the sessions overlay.
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(sample_sessions()),
            purpose: SessionsPurpose::Browse,
        });
        assert!(!app.overlay_stack.is_empty());
        let top = app.get_top_overlay_index();
        assert!(top.is_some());

        // Focus transitions to the overlay and back.
        app.hide_overlay();
        assert!(app.overlay_stack.is_empty());
        assert!(app.get_top_overlay_index().is_none());

        // The tree overlay on empty sessions shows a message instead.
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(vec![]),
            purpose: SessionsPurpose::Tree,
        });
        assert!(last_system(&app).contains("No sessions found"));

        // Scoped models overlay opens.
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Scoped,
        });
        assert!(!app.overlay_stack.is_empty());
        app.hide_overlay();

        // Fork overlay via ForkMessagesLoaded.
        app.handle_cmd(UiCmd::ForkMessagesLoaded(Ok(json_parse(
            r#"{"messages":[{"id":"e1","text":"fork point","role":"user"},{"id":"e2","text":"reply","role":"assistant"}]}"#,
        ))));
        assert!(!app.overlay_stack.is_empty());
        app.hide_overlay();

        // Help overlay opens and closes.
        app.show_help_overlay();
        assert!(!app.overlay_stack.is_empty());
        app.hide_overlay();

        // composite_line_at merges an overlay segment into a base line.
        let merged = App::<FakeTerminal>::composite_line_at("abcdef", "XY", 2, 2, 6);
        assert!(merged.contains("XY"));
        let _ = &mut rx;
    }

    // ─── Welcome / messages / settings / connection ───────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn welcome_variants_and_messages() {
        let (mut app, mut rx) = make_app(100, 30);
        let _ = &mut rx;

        // Welcome with everything populated.
        app.state.version = "9.9.9-test".into();
        app.state.skills = vec!["alpha".into(), "beta".into()];
        app.state.extensions = vec!["ext1".into()];
        app.show_welcome();
        let plain: Vec<String> = app
            .chat
            .plain_messages()
            .into_iter()
            .map(|(_, c)| c)
            .collect();
        let joined = plain.join("\n");
        assert!(joined.contains("9.9.9-test"));
        assert!(joined.contains("[skills] alpha, beta"));
        assert!(joined.contains("[Extensions]"));

        // The history mapper rebuilds user/assistant/tool + skips the rest.
        // Rows arrive entries-shaped (`kind`) or as the legacy `messages`
        // shape; both go through the same mapper.
        app.apply_history_page(
            "s1",
            Ok(json_parse(
                r#"{"entries":[
              {"id":"m1","kind":"user","role":"user","blocks":[{"kind":"text","text":"q"}]},
              {"id":"m2","kind":"assistant","role":"assistant","blocks":[{"kind":"text","text":"a1"},{"kind":"text","text":"a2"},{"kind":"tool_call","toolCallId":"call","name":"read","arguments":{"path":"/tmp/notes.txt"}}]},
              {"id":"m3","kind":"tool","role":"tool","blocks":[{"kind":"tool_result","text":"tool out","toolCallId":"call","isError":false}]},
              {"id":"m4","kind":"session_info","role":"system","blocks":[{"kind":"text","text":"skipped"}]},
              {"id":"m5","kind":"assistant","role":"assistant"},
              {"kind":"user","role":"user","blocks":[{"kind":"text","text":"no id"}]},
              {"id":"m6","kind":"user","role":"user","blocks":[]}
            ]}"#,
            )),
        );
        let texts: Vec<String> = app
            .chat
            .plain_messages()
            .into_iter()
            .map(|(_, c)| c)
            .collect();
        let joined = texts.join("\n");
        assert!(joined.contains("a1a2"));
        assert!(joined.contains("tool out"));
        assert!(!joined.contains("skipped"));
        // The replayed tool message resolves name/args from its `tool_call`
        // block — it must show `read /tmp/notes.txt`, never the raw call id.
        let rendered = crate::utils::strip_ansi_codes(&app.chat.render_all(100).join("\n"));
        assert!(rendered.contains("read /tmp/notes.txt"), "{rendered}");
        assert!(!rendered.contains(" call"), "{rendered}");
        // apply_history_page with an error is a no-op; an empty page clears.
        let before = app.chat.plain_messages().len();
        app.apply_history_page("s1", Err("x".into()));
        assert_eq!(app.chat.plain_messages().len(), before);
        app.apply_history_page("s1", Ok(json_parse(r#"{"entries":[]}"#)));
        assert!(app.chat.plain_messages().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn settings_persistence_roundtrip() {
        let (mut app, mut rx) = make_app(100, 30);
        let _ = &mut rx;
        // make_app points at a temp settings path; write through and reload.
        app.tui_settings.default_model = Some("m/x".into());
        app.tui_settings.default_thinking_level = Some("high".into());
        app.tui_settings.default_permission_level = Some("auto".into());
        app.tui_settings.enabled_model_ids = Some(vec!["a".into()]);
        app.save_tui_settings();
        // Corrupt-then-load paths.
        app.load_tui_settings();
        assert_eq!(app.tui_settings.default_model.as_deref(), Some("m/x"));
        // Corrupt the file: load keeps defaults.
        std::fs::write(&app.tui_settings_path, "not json").unwrap();
        app.tui_settings = TuiSettings::default();
        app.load_tui_settings();
        assert!(app.tui_settings.default_model.is_none());
        // Missing file: defaults.
        std::fs::remove_file(&app.tui_settings_path).unwrap();
        app.load_tui_settings();
        assert!(app.tui_settings.default_model.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn connection_change_paths() {
        let (mut app, mut rx) = make_app(100, 30);
        // Lost → message + reconnect timer.
        app.on_connection_change(false);
        assert!(app.connection_lost);
        assert!(last_system(&app).contains("lost"));
        // Same state → no-op.
        let before = app.chat.plain_messages().len();
        app.on_connection_change(false);
        assert_eq!(app.chat.plain_messages().len(), before);
        // Back online → reconnect message + refresh spawn.
        app.on_connection_change(true);
        assert!(!app.connection_lost);
        assert!(last_system(&app).contains("Reconnected"));
        pump(&mut app, &mut rx).await;
        // Online when already online → no-op.
        let before = app.chat.plain_messages().len();
        app.on_connection_change(true);
        assert_eq!(app.chat.plain_messages().len(), before);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn timers_and_tick_paths() {
        let (mut app, mut rx) = make_app(100, 30);
        // InitialPrompt timer with a prompt spawns the send.
        app.cli_initial_prompt = Some("boot prompt".into());
        app.timers.push((Instant::now(), TimerId::InitialPrompt));
        app.on_tick();
        pump(&mut app, &mut rx).await;
        // InitialPrompt timer with no prompt → nothing.
        app.cli_initial_prompt = None;
        app.timers.push((Instant::now(), TimerId::InitialPrompt));
        app.on_tick();
        // ReconnectRefresh timer → spawn_refresh.
        app.timers.push((Instant::now(), TimerId::ReconnectRefresh));
        app.on_tick();
        pump(&mut app, &mut rx).await;
        // next_deadline reflects pending timers.
        app.timers.push((
            Instant::now() + Duration::from_secs(60),
            TimerId::ReconnectRefresh,
        ));
        assert!(app.next_deadline().is_some());
        app.timers.clear();
        // ac query deadline fires the pending query.
        app.pending_ac_query = Some(("/m".into(), 2));
        app.ac_query_deadline = Some(Instant::now());
        app.on_tick();
        assert!(app.pending_ac_query.is_none());
        // …and with no pending query the deadline just clears.
        app.ac_query_deadline = Some(Instant::now());
        app.on_tick();
        assert!(app.ac_query_deadline.is_none());
        // resize deadline fires a render request.
        app.resize_deadline = Some(Instant::now());
        app.on_tick();
        assert!(app.resize_deadline.is_none());
    }

    // ─── Keybinding dispatches (closures registered in setup) ─────────

    #[tokio::test(flavor = "multi_thread")]
    async fn keybinding_closures_fire() {
        let (mut app, mut rx) = make_app(100, 30);
        for key in [
            "ctrl+c",
            "ctrl+p",
            "ctrl+r",
            "ctrl+t",
            "shift+tab",
            "ctrl+o",
            "ctrl+g",
            "ctrl+d",
            "pageup",
            "pagedown",
            "ctrl+up",
            "ctrl+down",
        ] {
            app.handle_key(key);
            pump(&mut app, &mut rx).await;
        }
        // ctrl+c interrupted (not streaming) → app stopped.
        assert!(!app.running);
    }

    /// `ctrl+d` flips the compact view through the same path a real key press
    /// takes (keybinding closure → `KeyAction` → the chat), and it is a toggle:
    /// the transcript comes back exactly as it was.
    #[tokio::test(flavor = "multi_thread")]
    async fn ctrl_d_toggles_the_compact_view() {
        let (mut app, mut rx) = make_app(100, 30);
        app.chat.render(100);
        for path in ["/a.rs", "/b.rs"] {
            let mut msg = ChatMessage::new(format!("t{path}"), ChatRole::Tool, "body\n");
            msg.name = Some("read".into());
            msg.tool = Some(format!("call{path}"));
            msg.tool_args = Some(format!(r#"{{"path":"{path}"}}"#));
            msg.tool_status = Some(ToolStatus::Complete);
            app.chat.add_message(msg);
        }
        let plain = |app: &mut App<FakeTerminal>| {
            crate::utils::strip_ansi_codes(&app.chat.render_all(100).join("\n"))
        };
        assert!(!app.chat.compact_activity(), "off by default");
        let expanded = plain(&mut app);
        assert!(expanded.contains("read /a.rs"), "{expanded}");

        app.handle_key("ctrl+d");
        pump(&mut app, &mut rx).await;
        assert!(app.chat.compact_activity());
        let folded = plain(&mut app);
        assert!(folded.contains("▸ read 2 files"), "{folded}");
        assert!(!folded.contains("/a.rs"), "the calls folded away: {folded}");

        app.handle_key("ctrl+d");
        pump(&mut app, &mut rx).await;
        assert!(!app.chat.compact_activity());
        assert_eq!(
            plain(&mut app),
            expanded,
            "toggling back restores the transcript byte for byte"
        );
    }

    // ─── Startup against a live mock agent ────────────────────────────

    /// Minimal agent: rich state, models, sessions (two, for the continue
    /// sort), messages; all mutations succeed. Records command types.
    #[derive(Clone, Default)]
    struct AppMockAgent {
        seen: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
        /// Every command received, verbatim — lets a test assert the
        /// arguments a slash command actually put on the wire (`enabled`,
        /// `message`, `toolCallId`, `limit`).
        requests: std::sync::Arc<std::sync::Mutex<Vec<RpcCommand>>>,
        /// Per-type response data overrides (checked first).
        overrides: std::collections::HashMap<String, String>,
        /// Per-type failure (success=false, error "nope").
        fail: std::collections::HashSet<String>,
        /// Scripted stream frames, emitted before the stream goes silent.
        /// Empty (the default) emits the single `ping` the app needs to
        /// consider the connection live.
        events: Vec<future_rpc::proto::StreamEvent>,
        /// Scripted get_state responses, popped front-to-back (agent-restart
        /// scenarios); the built-in default replies once it drains.
        state_script: Option<std::sync::Arc<std::sync::Mutex<Vec<String>>>>,
        /// While this is `false` the mock answers `list_models` with a
        /// failure — a deterministic "agent not up yet" for the connect-retry
        /// test. (TCP refusal timing is not portable: Windows can hold the
        /// SYNs sent to a just-closed port until the next listener binds,
        /// which made the retry setup connect on its very first attempt.)
        not_ready: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
        /// Page script for `get_session_entries`, keyed by the `before` cursor
        /// the client sent (`i64::MAX` = the tail read). A cursor the script
        /// does not mention answers an empty final page; no script at all is an
        /// agent with no indexed history for the session, which is also how the
        /// `get_messages` fallback is reached.
        history_pages:
            Option<std::sync::Arc<std::sync::Mutex<std::collections::HashMap<i64, String>>>>,
    }

    /// `get_session_entries` page payload: `entries` of `(id, role, text)` plus
    /// the continuation cursor.
    fn entries_page(rows: &[(&str, &str, &str)], has_more: bool, next_offset: i64) -> String {
        let entries: Vec<Value> = rows
            .iter()
            .map(|(id, role, text)| {
                serde_json::json!({
                    "id": id,
                    "kind": role,
                    "role": role,
                    "blocks": [{"kind": "text", "text": text}],
                })
            })
            .collect();
        serde_json::json!({
            "entries": entries,
            "hasMore": has_more,
            "nextOffset": next_offset,
        })
        .to_string()
    }

    #[tonic::async_trait]
    impl FutureAgent for AppMockAgent {
        async fn execute_command(
            &self,
            request: tonic::Request<future_rpc::proto::RpcCommand>,
        ) -> Result<tonic::Response<future_rpc::proto::RpcResponse>, tonic::Status> {
            let cmd = request.into_inner();
            self.requests.lock().unwrap().push(cmd.clone());
            self.seen.lock().unwrap().push((
                cmd.r#type.clone(),
                format!("{}|{}|{}", cmd.level, cmd.model_id, cmd.session_id),
            ));
            if let Some(data) = self.overrides.get(&cmd.r#type) {
                return Ok(tonic::Response::new(future_rpc::proto::RpcResponse {
                    id: cmd.id,
                    r#type: "response".into(),
                    command: cmd.r#type.clone(),
                    success: true,
                    data: data.clone(),
                    error: String::new(),
                    error_code: String::new(),
                    error_data: String::new(),
                    payload: None,
                }));
            }
            if cmd.r#type == "get_state" {
                if let Some(script) = &self.state_script {
                    let next = {
                        let mut g = script.lock().unwrap();
                        if g.is_empty() {
                            None
                        } else {
                            Some(g.remove(0))
                        }
                    };
                    if let Some(data) = next {
                        return Ok(tonic::Response::new(future_rpc::proto::RpcResponse {
                            id: cmd.id,
                            r#type: "response".into(),
                            command: cmd.r#type.clone(),
                            success: true,
                            data,
                            error: String::new(),
                            error_code: String::new(),
                            error_data: String::new(),
                            payload: None,
                        }));
                    }
                }
            }
            if cmd.r#type == "list_models"
                && self
                    .not_ready
                    .as_ref()
                    .is_some_and(|ready| !ready.load(std::sync::atomic::Ordering::SeqCst))
            {
                return Ok(tonic::Response::new(future_rpc::proto::RpcResponse {
                    id: cmd.id,
                    r#type: "response".into(),
                    command: cmd.r#type.clone(),
                    success: false,
                    error: "agent starting".into(),
                    ..Default::default()
                }));
            }
            // Paged history: the cursor the client sent picks the page (see
            // `history_pages`). A cursor the script does not mention is an agent
            // with no indexed history for this session — empty, not an error,
            // which is also how the `get_messages` fallback is reached.
            if cmd.r#type == "get_session_entries" {
                let fail = self.fail.contains(&cmd.r#type);
                let data = self
                    .history_pages
                    .as_ref()
                    .and_then(|pages| {
                        pages
                            .lock()
                            .unwrap()
                            .get(&cmd.before.unwrap_or(i64::MAX))
                            .cloned()
                    })
                    .unwrap_or_else(|| r#"{"entries":[]}"#.to_string());
                return Ok(tonic::Response::new(future_rpc::proto::RpcResponse {
                    id: cmd.id,
                    r#type: "response".into(),
                    command: cmd.r#type.clone(),
                    success: !fail,
                    data,
                    error: if fail { "nope".into() } else { String::new() },
                    error_code: String::new(),
                    error_data: String::new(),
                    payload: None,
                }));
            }
            let fail = self.fail.contains(&cmd.r#type);
            let data = match cmd.r#type.as_str() {
                "get_state" => {
                    r#"{"sessionId":"s1","model":"openai/gpt-4o","thinkingLevel":"high","cwd":"/tmp","version":"9.9.9-mock","skills":["alpha"],"contextFiles":["CLAUDE.md"],"extensions":["ext1"],"isStreaming":false}"#
                }
                "list_models" => {
                    r#"{"models":[{"id":"gpt-4o","label":"GPT-4o","provider":"openai"},{"id":"claude-sonnet-4","label":"Claude","provider":"anthropic"}]}"#
                }
                "list_sessions" => {
                    r#"{"sessions":[{"id":"s1","cwd":"/tmp","updatedAtMs": 20454000,"model":"m","sessionName":"main"},{"id":"s0","cwd":"/tmp","updatedAtMs": 20453000,"model":"m","sessionName":"older"}]}"#
                }
                "new_session" => r#"{"sessionId":"s-new"}"#,
                "switch_session" | "fork" => r#"{"cancelled":false}"#,
                "get_messages" => {
                    r#"{"messages":[{"id":"m1","role":"user","content":"earlier question"},{"id":"m2","role":"assistant","content":"earlier answer"}]}"#
                }
                "get_fork_messages" => {
                    r#"{"messages":[{"id":"e1","text":"fork point one","role":"user"},{"id":"e2","text":"reply","role":"assistant"}]}"#
                }
                _ => "{}",
            };
            Ok(tonic::Response::new(future_rpc::proto::RpcResponse {
                id: cmd.id,
                r#type: "response".into(),
                command: cmd.r#type.clone(),
                success: !fail,
                data: data.to_string(),
                error: if fail { "nope".into() } else { String::new() },
                error_code: String::new(),
                error_data: String::new(),
                payload: None,
            }))
        }

        type StreamEventsStream = Pin<
            Box<
                dyn tokio_stream::Stream<
                        Item = Result<future_rpc::proto::StreamEvent, tonic::Status>,
                    > + Send,
            >,
        >;

        async fn stream_events(
            &self,
            _request: tonic::Request<future_rpc::proto::StreamRequest>,
        ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
            let events = if self.events.is_empty() {
                vec![future_rpc::proto::StreamEvent {
                    r#type: "ping".into(),
                    ..Default::default()
                }]
            } else {
                self.events.clone()
            };
            Ok(tonic::Response::new(Box::pin(
                futures_util::stream::iter(events.into_iter().map(Ok)).chain(
                    futures_util::stream::pending::<
                        Result<future_rpc::proto::StreamEvent, tonic::Status>,
                    >(),
                ),
            )))
        }
    }

    async fn spawn_app_mock() -> (
        String,
        std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
    ) {
        spawn_app_mock_with(AppMockAgent::default()).await
    }

    async fn spawn_app_mock_with(
        mock: AppMockAgent,
    ) -> (
        String,
        std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
    ) {
        // Serve on the listener we bound ourselves — never re-bind a port that
        // was released in between (a concurrent mock could win the race).
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let incoming = TcpIncoming::from_listener(listener, true, None).unwrap();
        let seen = mock.seen.clone();
        tokio::spawn(
            Server::builder()
                .add_service(FutureAgentServer::new(mock))
                .serve_with_incoming(incoming),
        );
        (format!("127.0.0.1:{}", addr.port()), seen)
    }

    fn make_app_at(
        addr: &str,
        cli_options: &CliOptions,
    ) -> (App<FakeTerminal>, mpsc::UnboundedReceiver<UiCmd>) {
        let (op_tx, op_rx) = mpsc::unbounded_channel();
        let (client, _events, _conn) = GrpcClient::new(addr);
        let app = App::new(
            FakeTerminal {
                writes: Rc::new(RefCell::new(Vec::new())),
                cols: 100,
                rows: 30,
                on_input: None,
                on_resize: None,
            },
            Arc::new(client),
            op_tx,
            cli_options,
            test_settings_path(),
        );
        (app, op_rx)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn startup_default_flow_with_live_agent() {
        let (addr, _seen) = spawn_app_mock().await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        assert!(app.is_running());
        assert_eq!(app.state.session_id, "s1"); // refreshed after new_session
        pump(&mut app, &mut rx).await;
        // The welcome screen rendered.
        let all = app.chat.plain_messages();
        let joined = all
            .iter()
            .map(|(_, c)| c.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("9.9.9-mock"));
        assert!(joined.contains("[skills] alpha"));
        assert!(joined.contains("[Extensions]"));
        app.stop();
        assert!(!app.is_running());
    }

    // ─── Paged history ───────────────────────────────────────────────

    /// One page in the shape the agent actually stores a tool-using turn in:
    /// an assistant entry carrying the `tool_call` step, then the `tool` entry
    /// with its result — repeated, with a reasoning-only assistant entry first.
    fn tool_run_page() -> Value {
        let mut rows = vec![serde_json::json!({
            "id": "e0",
            "kind": "assistant",
            "role": "assistant",
            "blocks": [{"kind": "reasoning", "text": "I should read the parser first."}],
        })];
        for (index, path) in ["/a.rs", "/b.rs", "/c.rs"].iter().enumerate() {
            let call_id = format!("call_{index}");
            rows.push(serde_json::json!({
                "id": format!("e{index}a"),
                "kind": "assistant",
                "role": "assistant",
                "blocks": [{
                    "kind": "tool_call",
                    "toolCallId": call_id,
                    "name": "read",
                    "arguments": {"path": path},
                }],
            }));
            rows.push(serde_json::json!({
                "id": format!("e{index}t"),
                "kind": "tool",
                "role": "tool",
                "blocks": [{
                    "kind": "tool_result",
                    "toolCallId": call_id,
                    "text": format!("body of {path}\n"),
                    "isError": false,
                }],
            }));
        }
        serde_json::json!({"entries": rows})
    }

    /// A loaded run folds exactly like a live one.
    ///
    /// The agent stores a tool-using turn as an assistant entry per call (the
    /// `tool_call` step, no text) followed by the call's `tool` entry. Those
    /// assistant steps used to become empty chat messages: they rendered no row
    /// of their own but still separated the calls, so a loaded session never
    /// folded and every pair of calls carried an extra blank line the live view
    /// does not have.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_loaded_run_folds_like_a_live_one() {
        let (mut app, _rx) = make_app(100, 30);
        app.chat.render(100);
        app.chat.set_compact_activity(true);
        app.apply_history_page("s1", Ok(tool_run_page()));

        let lines: Vec<String> = app
            .chat
            .render_all(100)
            .iter()
            .map(|line| crate::utils::strip_ansi_codes(line).trim().to_string())
            .filter(|line| !line.is_empty())
            .collect();
        assert_eq!(lines, vec!["▸ thinking", "▸ read 3 files"], "{lines:?}");

        // The same page without the compact view: three call rows, each a single
        // blank line apart — the spacing the live view has.
        app.chat.set_compact_activity(false);
        let raw = app.chat.render_all(100);
        let plain: Vec<String> = raw
            .iter()
            .map(|l| crate::utils::strip_ansi_codes(l))
            .collect();
        let text = plain.join("\n");
        assert!(text.contains("I should read the parser first."), "{text}");
        for path in ["/a.rs", "/b.rs", "/c.rs"] {
            assert!(text.contains(path), "{text}");
        }
        let calls = plain
            .iter()
            .position(|line| line.contains("read /a.rs"))
            .expect("the first call row");
        let blank = plain[calls + 1].trim().is_empty();
        assert!(blank, "a blank line separates the calls");
        assert!(
            !plain[calls + 2].trim().is_empty(),
            "exactly one blank line: {plain:?}"
        );
        assert!(plain[calls + 2].contains("read /b.rs"), "{plain:?}");
    }

    /// A call that returned nothing is still a call: the live view shows the
    /// row while it runs, so a reload must not make it disappear.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_loaded_call_with_no_output_keeps_its_row() {
        let (mut app, _rx) = make_app(100, 30);
        app.chat.render(100);
        app.apply_history_page(
            "s1",
            Ok(serde_json::json!({"entries": [
                {"id": "a1", "kind": "assistant", "role": "assistant",
                 "blocks": [{"kind": "tool_call", "toolCallId": "c1", "name": "shell",
                              "arguments": {"command": "true"}}]},
                {"id": "t1", "kind": "tool", "role": "tool",
                 "blocks": [{"kind": "tool_result", "toolCallId": "c1", "text": "", "isError": false}]}
            ]})),
        );
        let rendered = crate::utils::strip_ansi_codes(&app.chat.render_all(100).join("\n"));
        assert!(
            rendered.contains("$ true"),
            "the call row survived: {rendered}"
        );
    }

    /// A `--session` startup reads the session's history as one backward page:
    /// `get_session_entries` with the tail cursor, never `get_messages` (whose
    /// response has no cursor and has to carry the whole session).
    #[tokio::test(flavor = "multi_thread")]
    async fn startup_loads_the_history_tail_through_the_pager() {
        let mock = AppMockAgent {
            history_pages: Some(std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::from([(
                    i64::MAX,
                    entries_page(&[("e1", "user", "old question")], true, 3),
                )]),
            ))),
            ..Default::default()
        };
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                session: Some("s1".into()),
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;

        let history: Vec<_> = requests
            .lock()
            .unwrap()
            .iter()
            .filter(|cmd| cmd.r#type == "get_session_entries")
            .map(|cmd| (cmd.before, cmd.limit, cmd.session_id.clone()))
            .collect();
        assert_eq!(history, vec![(Some(i64::MAX), Some(10), "s1".to_string())]);
        assert!(
            !requests
                .lock()
                .unwrap()
                .iter()
                .any(|cmd| cmd.r#type == "get_messages"),
            "a page was served, so the uncapped read is never asked for"
        );
        let joined = plain_text(&app);
        assert!(joined.contains("old question"), "{joined}");
        // The cursor is live: there is more above.
        assert!(app.history_paging.has_more);
        assert_eq!(app.history_paging.next_before, 3);
        app.stop();
    }

    /// Scrolling up at the top of the transcript fetches the next older page and
    /// shows it — the pages are what makes a long session scrollable instead of
    /// one unbounded response.
    #[tokio::test(flavor = "multi_thread")]
    async fn scrolling_up_at_the_top_prepends_the_older_page() {
        // Enough rows that the tail page cannot fit the app's viewport, so the
        // reader has somewhere to scroll to before the load can trigger.
        let tail_rows: Vec<(String, String, String)> = (0..20)
            .map(|i| {
                (
                    format!("t{i}"),
                    if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
                    format!("tail-{i}"),
                )
            })
            .collect();
        let tail_refs: Vec<(&str, &str, &str)> = tail_rows
            .iter()
            .map(|(id, role, text)| (id.as_str(), role.as_str(), text.as_str()))
            .collect();
        let older_refs: Vec<(&str, &str, &str)> = vec![
            ("o0", "user", "old-0"),
            ("o1", "assistant", "old-1"),
            ("o2", "user", "old-2"),
            ("o3", "assistant", "old-3"),
        ];
        let mock = AppMockAgent {
            history_pages: Some(std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::from([
                    (i64::MAX, entries_page(&tail_refs, true, 2)),
                    (2, entries_page(&older_refs, false, 0)),
                ]),
            ))),
            ..Default::default()
        };
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                session: Some("s1".into()),
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;
        app.chat.render(100);
        app.chat.scroll_to_bottom();
        assert!(!app.chat.is_at_top(), "the tail page exceeds the viewport");
        // Walk to the very top (the number of page-ups depends on the terminal
        // height, which is not what this test is about).
        while app.chat.scroll_up(1_000) {}
        assert!(app.chat.is_at_top());

        app.handle_key_action(KeyAction::ScrollChatUpPage);
        pump_until_history_settled(&mut app, &mut rx).await;

        let cursors: Vec<_> = requests
            .lock()
            .unwrap()
            .iter()
            .filter(|cmd| cmd.r#type == "get_session_entries")
            .map(|cmd| cmd.before)
            .collect();
        assert_eq!(cursors, vec![Some(i64::MAX), Some(2)]);
        let transcript = plain_text(&app);
        assert!(
            transcript.contains("old-0"),
            "the older page arrived: {transcript}"
        );
        let old = transcript.find("old-0").unwrap();
        let tail = transcript.find("tail-0").unwrap();
        assert!(old < tail, "the older rows go above the tail page");
        // The reader is looking at history, not at the tail.
        assert!(
            !app.chat.auto_scroll(),
            "the view is not following the tail"
        );
        // The final page carries no continuation, so paging stops — and another
        // scroll-up past the top must not ask again.
        assert!(!app.history_paging.has_more);
        assert!(!app.history_paging.loading);
        app.handle_key_action(KeyAction::ScrollChatUpPage);
        pump_until_history_settled(&mut app, &mut rx).await;
        assert_eq!(
            requests
                .lock()
                .unwrap()
                .iter()
                .filter(|cmd| cmd.r#type == "get_session_entries")
                .count(),
            2
        );
        app.stop();
    }

    /// The loaded page must be *visible*: a reader pinned at the top who gets
    /// their transcript anchored would see no change at all.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_loaded_page_is_revealed_not_hidden_above_the_viewport() {
        let mock = AppMockAgent {
            history_pages: Some(std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::from([
                    (i64::MAX, entries_page(&[("t0", "user", "tail-0")], true, 1)),
                    (
                        1,
                        entries_page(
                            &[("o0", "user", "old-0"), ("o1", "assistant", "old-1")],
                            false,
                            0,
                        ),
                    ),
                ]),
            ))),
            ..Default::default()
        };
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                session: Some("s1".into()),
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;
        app.chat.render(100);
        assert!(
            app.chat.is_at_top(),
            "the short tail page fits the viewport"
        );

        app.handle_key_action(KeyAction::ScrollChatUpPage);
        pump(&mut app, &mut rx).await;

        let visible = crate::utils::strip_ansi_codes(&app.chat.render(100).join("\n"));
        assert!(
            visible.contains("old-0") && visible.contains("old-1"),
            "the loaded page is on screen: {visible}"
        );
        // Reading back stops following the tail, or the next streamed row would
        // drag the reader away from the page they just loaded.
        assert!(
            !app.chat.auto_scroll(),
            "the reader is not following the tail"
        );
        app.chat.add_message(ChatMessage::new(
            "n1".into(),
            ChatRole::Assistant,
            "streamed",
        ));
        assert!(
            !app.chat.auto_scroll(),
            "a new row does not re-arm the follow"
        );
        app.stop();
    }

    /// A transcript shorter than the viewport cannot scroll, and `scroll_up`
    /// then reports `false` — the page request is driven by the key, not by that
    /// result, or the rest of the history would be unreachable.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_short_transcript_still_pages_from_the_top() {
        let mock = AppMockAgent {
            history_pages: Some(std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::from([
                    (
                        i64::MAX,
                        entries_page(&[("e2", "user", "newest question")], true, 1),
                    ),
                    (
                        1,
                        entries_page(&[("e1", "user", "oldest question")], false, 0),
                    ),
                ]),
            ))),
            ..Default::default()
        };
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                session: Some("s1".into()),
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;
        app.chat.render(100);
        assert!(
            app.chat.is_at_top(),
            "everything fits, so there is nothing to scroll"
        );
        assert!(!app.chat.scroll_up(1), "and the scroll reports it");

        app.handle_key_action(KeyAction::ScrollChatUpPage);
        pump_until_history_settled(&mut app, &mut rx).await;
        let visible = crate::utils::strip_ansi_codes(&app.chat.render(100).join("\n"));
        assert!(visible.contains("oldest question"), "{visible}");
        assert!(!app.history_paging.has_more);
        app.stop();
    }

    /// A scroll that has not reached the top must not fetch: the middle of a
    /// transcript is not a page boundary.
    #[tokio::test(flavor = "multi_thread")]
    async fn scrolling_up_in_the_middle_does_not_page() {
        // Tall enough that scrolling a few lines from the tail stays inside the
        // transcript instead of reaching its top.
        let rows: Vec<(String, String, String)> = (0..20)
            .map(|i| {
                (
                    format!("t{i}"),
                    if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
                    format!("tail-{i}"),
                )
            })
            .collect();
        let refs: Vec<(&str, &str, &str)> = rows
            .iter()
            .map(|(id, role, text)| (id.as_str(), role.as_str(), text.as_str()))
            .collect();
        let mock = AppMockAgent {
            history_pages: Some(std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::from([(
                    i64::MAX,
                    // A page that *would* continue, so only the scroll position
                    // can explain a missing request.
                    entries_page(&refs, true, 9),
                )]),
            ))),
            ..Default::default()
        };
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                session: Some("s1".into()),
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;
        app.chat.render(100);
        app.chat.scroll_to_bottom();
        assert!(app.chat.scroll_up(3), "three lines above the tail");
        assert!(
            !app.chat.is_at_top() && !app.chat.is_at_bottom(),
            "the reader is inside the transcript"
        );

        app.handle_key_action(KeyAction::ScrollChatUpLine);
        pump(&mut app, &mut rx).await;
        assert_eq!(
            requests
                .lock()
                .unwrap()
                .iter()
                .filter(|cmd| cmd.r#type == "get_session_entries")
                .count(),
            1,
            "no page request while the reader is inside the transcript"
        );
        assert!(app.history_paging.has_more, "the cursor is untouched");
        app.stop();
    }

    /// An agent that answers the pager with nothing still has the conversation
    /// in its live context: the fallback read is what keeps a session from
    /// rendering as an empty transcript.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_page_falls_back_to_the_context_messages() {
        // No `history_pages` script: the pager answers `{"entries": []}`.
        // (`get_messages` without `blocks` renders nothing, see `AppMockAgent`.)
        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "get_messages".to_string(),
            r#"{"messages":[{"role":"user","blocks":[{"kind":"text","text":"from the context"}]}]}"#
                .to_string(),
        );
        let mock = AppMockAgent {
            overrides,
            ..Default::default()
        };
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                session: Some("s1".into()),
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;

        let seen: Vec<_> = requests
            .lock()
            .unwrap()
            .iter()
            .map(|cmd| cmd.r#type.clone())
            .collect();
        assert!(
            seen.iter().any(|t| t == "get_messages"),
            "the fallback ran: {seen:?}"
        );
        let joined = plain_text(&app);
        assert!(joined.contains("from the context"), "{joined}");
        // A single whole-history response carries no cursor: paging is off.
        assert!(!app.history_paging.has_more);
        app.stop();
    }

    /// An agent that predates the pager (or cannot read its index) fails the
    /// command — the same fallback applies, so version skew cannot blank a
    /// transcript.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_failing_pager_falls_back_to_the_context_messages() {
        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "get_messages".to_string(),
            r#"{"messages":[{"role":"user","blocks":[{"kind":"text","text":"legacy context"}]}]}"#
                .to_string(),
        );
        let mock = AppMockAgent {
            overrides,
            fail: std::collections::HashSet::from(["get_session_entries".to_string()]),
            ..Default::default()
        };
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                session: Some("s1".into()),
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;
        let joined = plain_text(&app);
        assert!(joined.contains("legacy context"), "{joined}");
        assert!(!app.history_paging.has_more);
        app.stop();
    }

    /// Both reads failing is a real failure: the switch reports it (keeping the
    /// pager's error, which names the storage it wanted) and the previous
    /// session's transcript comes off the screen — a stale conversation under a
    /// prompt that now addresses another session is worse than a blank one.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_history_load_is_reported_and_leaves_no_stale_transcript() {
        let mut fail = std::collections::HashSet::new();
        fail.insert("get_session_entries".to_string());
        fail.insert("get_messages".to_string());
        let mock = AppMockAgent {
            fail,
            ..Default::default()
        };
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                session: Some("s1".into()),
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;
        // The pager failed and its fallback failed too, so there is no history
        // and, crucially, no cursor pretending there is.
        assert!(!app.history_paging.has_more);
        assert!(!app.history_paging.loading);
        assert!(!plain_text(&app).contains("question"));

        // A transcript was on screen (the reader switched away from it): the
        // switch failure must clear it rather than label it as the new session's.
        app.apply_history_page(
            "old-session",
            Ok(json_parse(&entries_page(
                &[("e1", "user", "stale question")],
                false,
                0,
            ))),
        );
        assert!(plain_text(&app).contains("stale question"));

        app.handle_cmd(UiCmd::SessionSwitched {
            target: "s1".into(),
            switched: true,
            result: Ok(()),
            state: None,
            history: Err("get_session_entries failed: nope".into()),
            label: "other".into(),
        });
        assert!(!plain_text(&app).contains("stale question"));
        let notice = last_system(&app);
        assert!(notice.contains("could not be loaded"), "{notice}");
        assert!(
            notice.contains("get_session_entries failed: nope"),
            "{notice}"
        );
        // And the transcript does not keep offering to page the old session.
        assert!(!app.history_paging.has_more);
        app.stop();
    }

    /// A page that claims more history without moving the cursor is the end of
    /// the line: paging stops instead of re-fetching the same page on every
    /// scroll.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_non_advancing_page_stops_paging() {
        let mock = AppMockAgent {
            history_pages: Some(std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::from([(
                    i64::MAX,
                    // `hasMore` but the cursor stays where the request started.
                    entries_page(&[("e9", "user", "last question")], true, i64::MAX),
                )]),
            ))),
            ..Default::default()
        };
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                session: Some("s1".into()),
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;
        assert!(
            !app.history_paging.has_more,
            "a stalled cursor is not a continuation"
        );

        app.handle_key_action(KeyAction::ScrollChatUpPage);
        pump(&mut app, &mut rx).await;
        assert_eq!(
            requests
                .lock()
                .unwrap()
                .iter()
                .filter(|cmd| cmd.r#type == "get_session_entries")
                .count(),
            1,
            "the stalled cursor is never re-requested"
        );
        app.stop();
    }

    /// A page whose session is no longer on screen is dropped: its rows belong
    /// to a transcript the user has left.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_page_for_a_left_session_is_dropped() {
        let (mut app, _rx) = make_app(100, 30);
        app.state.session_id = "current".into();
        app.chat
            .add_message(ChatMessage::new("e1".into(), ChatRole::User, "on screen"));
        app.history_paging.session_id = "current".into();
        app.history_paging.has_more = true;
        app.history_paging.next_before = 4;
        app.history_paging.loading = true;

        app.handle_cmd(UiCmd::HistoryPageLoaded {
            session_id: "left-behind".into(),
            before: 4,
            result: Ok(json_parse(&entries_page(
                &[("old", "user", "older")],
                false,
                0,
            ))),
        });
        assert_eq!(app.chat.plain_messages().len(), 1);
        assert!(!last_system(&app).contains("older"));
        // An in-flight request that failed is the only thing that clears the
        // loading flag for its own session.
        assert!(app.history_paging.loading, "a foreign page changes nothing");

        app.handle_cmd(UiCmd::HistoryPageLoaded {
            session_id: "current".into(),
            before: 4,
            result: Err("pager down".into()),
        });
        assert!(!app.history_paging.loading, "the retry is unblocked");
        assert!(app.history_paging.has_more, "the cursor keeps its position");
        assert!(last_system(&app).contains("Failed to load older history: pager down"));
    }

    /// The scrollback is append-only, so rows loaded above it can never be
    /// written there — but they must not make the next flush re-emit the whole
    /// transcript either.
    #[tokio::test(flavor = "multi_thread")]
    async fn prepending_history_does_not_duplicate_the_scrollback() {
        let mock = AppMockAgent {
            history_pages: Some(std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::from([
                    (
                        i64::MAX,
                        entries_page(&[("e2", "user", "second question")], true, 1),
                    ),
                    (
                        1,
                        entries_page(&[("e1", "user", "first question")], false, 0),
                    ),
                ]),
            ))),
            ..Default::default()
        };
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                session: Some("s1".into()),
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;
        app.flush_scrollback(false);
        let first = app.history.watermark_len();
        assert!(first > 0, "the tail page reached the scrollback");

        app.chat.set_viewport_height(40);
        app.chat.render(100);
        app.handle_key_action(KeyAction::ScrollChatUpPage);
        pump_until_history_settled(&mut app, &mut rx).await;
        assert!(plain_text(&app).contains("first question"));

        app.flush_scrollback(false);
        assert_eq!(
            app.history.watermark_len(),
            first,
            "the older rows are above the scrollback, and nothing was re-emitted"
        );
        app.stop();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn startup_session_option_variants() {
        let (addr, seen) = spawn_app_mock().await;

        // --session.
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                session: Some("s1".into()),
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        assert!(app.is_running());
        pump(&mut app, &mut rx).await;
        app.stop();

        // --continue (most recent session → switch_session happens).
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                r#continue: true,
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|(t, _)| t == "switch_session"));
        app.stop();

        // --fork.
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                fork: Some("entry-1".into()),
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;
        app.stop();

        // --resume (opens the session picker).
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                resume: true,
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;
        app.stop();

        // initial prompt → timer fires in the first ticks.
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                initial_prompt: Some("boot message".into()),
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        app.on_tick();
        pump(&mut app, &mut rx).await;
        app.stop();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tui_defaults_applied_at_startup() {
        let (addr, seen) = spawn_app_mock().await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        // Pre-seed the settings file; startup loads + applies it.
        let settings = r#"{"defaultModel":"openai/gpt-4o","defaultThinkingLevel":"low","defaultPermissionLevel":"auto","enabledModelIds":["openai/gpt-4o"]}"#;
        std::fs::write(&app.tui_settings_path, settings).unwrap();
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;
        // The defaults were pushed to the agent during startup.
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|(t, payload)| t == "set_thinking_level" && payload.starts_with("low")));
        app.stop();
    }

    /// The event's structured result decides a failure, so a live failed call is
    /// marked (and can keep its body) the same way a replayed one is.
    #[test]
    fn tool_end_failure_comes_from_the_structured_result() {
        assert!(!tool_end_failed(&json_parse(r#"{"tool_id":"t"}"#)));
        assert!(!tool_end_failed(&json_parse(
            r#"{"tool_id":"t","text":"ok","exit_code":0}"#
        )));
        // The agent's error field is the primary signal.
        assert!(tool_end_failed(&json_parse(
            r#"{"tool_id":"t","error":"permission denied"}"#
        )));
        // An empty error is not a failure.
        assert!(!tool_end_failed(&json_parse(
            r#"{"tool_id":"t","error":"  "}"#
        )));
        // A non-zero exit code is one — unless the agent already resolved it as
        // a soft fail (bare grep/diff/… exiting 1).
        assert!(tool_end_failed(&json_parse(
            r#"{"tool_id":"t","exit_code":2}"#
        )));
        assert!(!tool_end_failed(&json_parse(
            r#"{"tool_id":"t","exit_code":1,"is_soft_fail":true}"#
        )));
        assert!(tool_end_failed(&json_parse(
            r#"{"tool_id":"t","exit_code":1,"is_soft_fail":false}"#
        )));
    }

    /// A live failure reaches the chat as a failed call, and the compact view
    /// leaves it (and its body) alone.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_live_tool_failure_is_marked_and_never_folded() {
        let (mut app, mut rx) = make_app(100, 30);
        app.chat.set_compact_activity(true);
        for (id, path) in [("t1", "/a.rs"), ("t2", "/b.rs")] {
            app.handle_agent_event(&make_event(
                "tool_start",
                &format!(
                    r#"{{"tool_id":"{id}","tool_name":"read","tool_args":{{"path":"{path}"}}}}"#
                ),
            ));
            app.handle_agent_event(&make_event(
                "tool_end",
                &format!(r#"{{"tool_id":"{id}","text":"out"}}"#),
            ));
        }
        app.handle_agent_event(&make_event(
            "tool_start",
            r#"{"tool_id":"t3","tool_name":"read","tool_args":{"path":"/c.rs"}}"#,
        ));
        app.handle_agent_event(&make_event(
            "tool_delta",
            r#"{"tool_id":"t3","text":"permission denied"}"#,
        ));
        app.handle_agent_event(&make_event(
            "tool_end",
            r#"{"tool_id":"t3","error":"permission denied"}"#,
        ));
        pump(&mut app, &mut rx).await; // tool_end's refresh fails silently

        let text = crate::utils::strip_ansi_codes(&app.chat.render_all(100).join("\n"));
        assert!(text.contains("▸ read 2 files"), "{text}");
        assert!(
            text.contains("read /c.rs"),
            "the failure is its own row: {text}"
        );
        assert!(text.contains("permission denied"), "{text}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn spawn_paths_succeed_against_live_agent() {
        let (addr, _seen) = spawn_app_mock().await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();

        // ForkSelected with a successful fork → get_state + messages.
        app.handle_cmd(UiCmd::ForkSelected {
            item: SelectItem {
                value: "entry-1".into(),
                label: "entry one".into(),
                description: None,
            },
        });
        pump_until_msg(&mut app, &mut rx, "Forked from entry one.").await;
        assert!(last_system(&app).contains("Forked from entry one."));

        // /model set directly succeeds.
        app.handle_cmd(UiCmd::Submit("/model claude-sonnet-4".into()));
        pump_until_msg(&mut app, &mut rx, "Model:").await;

        // /new succeeds.
        app.handle_cmd(UiCmd::Submit("/new".into()));
        pump_until_msg(&mut app, &mut rx, "New session started").await;

        // /status prints the model table and the session's settings — it ends
        // at `Streaming:` because usage moved out of it (that is `/stats`'s and
        // the footer's job).
        app.handle_cmd(UiCmd::Submit("/status".into()));
        pump_until_msg(&mut app, &mut rx, "Streaming:").await;

        // /tree with sessions → tree overlay.
        app.handle_cmd(UiCmd::Submit("/tree".into()));
        pump_until_overlay(&mut app, &mut rx).await;
        app.handle_cmd(UiCmd::OverlayCancel);

        // /fork loads messages → fork overlay.
        app.handle_cmd(UiCmd::Submit("/fork".into()));
        pump_until_overlay(&mut app, &mut rx).await;
        app.handle_cmd(UiCmd::OverlayCancel);

        // /sessions overlay.
        app.handle_cmd(UiCmd::Submit("/sessions".into()));
        pump_until_overlay(&mut app, &mut rx).await;
        app.handle_cmd(UiCmd::OverlayCancel);

        // /reload succeeds.
        app.handle_cmd(UiCmd::Submit("/reload".into()));
        pump_until_msg(&mut app, &mut rx, "Reloaded:").await;

        // /compact + /stop + /name + /cancel succeed.
        app.handle_cmd(UiCmd::Submit("/compact".into()));
        app.handle_cmd(UiCmd::Submit("/stop".into()));
        app.handle_cmd(UiCmd::Submit("/name fancy".into()));
        app.handle_cmd(UiCmd::Submit("/cancel q9".into()));
        pump_until_all(
            &mut app,
            &mut rx,
            &[
                "Context compaction request accepted",
                "Stopped current generation",
                "fancy",
                "Cancelled queued run",
            ],
        )
        .await;
        assert!(system_messages(&app)
            .iter()
            .any(|m| m.contains("Context compaction request accepted")));
        assert!(system_messages(&app)
            .iter()
            .any(|m| m.contains("Stopped current generation")));
        assert!(system_messages(&app).iter().any(|m| m.contains("fancy")));
        assert!(system_messages(&app)
            .iter()
            .any(|m| m.contains("Cancelled queued run")));

        // /approve + /reject succeed.
        app.handle_cmd(UiCmd::Submit("/approve r1".into()));
        app.handle_cmd(UiCmd::Submit("/reject r2".into()));
        pump_until_all(
            &mut app,
            &mut rx,
            &["Approved request: r1", "Rejected request: r2"],
        )
        .await;
        assert!(system_messages(&app)
            .iter()
            .any(|m| m.contains("Approved request: r1")));
        assert!(system_messages(&app)
            .iter()
            .any(|m| m.contains("Rejected request: r2")));

        app.stop();
    }

    // ─── Render pipeline ──────────────────────────────────────────────

    fn running_app(cols: u16, rows: u16) -> (App<FakeTerminal>, mpsc::UnboundedReceiver<UiCmd>) {
        let (mut app, rx) = make_app(cols, rows);
        app.running = true;
        app.chat
            .add_message(ChatMessage::new("u1".into(), ChatRole::User, "hello world"));
        (app, rx)
    }

    fn render_writes(app: &App<FakeTerminal>) -> String {
        terminal_writes(app)
    }

    #[tokio::test]
    async fn render_frames_disable_implicit_wrapping() {
        // Our grapheme table counts ⭐ as one cell; tmux counts it as two.
        // A logically fitting line can therefore wrap unexpectedly. Every
        // text write must happen with DECAWM off, including full redraws.
        let (mut app, _rx) = running_app(40, 10);
        app.chat.add_message(ChatMessage::new(
            "a".into(),
            ChatRole::Assistant,
            &format!("{}{}bbbb", "a".repeat(26), "⭐".repeat(5)),
        ));
        for frame in 0..5 {
            app.terminal.writes.borrow_mut().clear();
            match frame {
                1 => app.chat.append_to_last_message("c"),    // diff
                2 => app.request_render(true),                // full redraw
                3 => app.chat.clear_messages(),               // shrink
                4 => app.previous_lines.push("stale".into()), // deleted tail only
                _ => {}
            }
            app.do_render();
            let output = render_writes(&app);
            let mut autowrap = true;
            let mut offset = 0;
            while offset < output.len() {
                if let Some(code) = crate::utils::extract_ansi_code(&output, offset) {
                    match code.code.as_str() {
                        "\x1b[?7l" => autowrap = false,
                        "\x1b[?7h" => autowrap = true,
                        _ => {}
                    }
                    offset += code.length;
                } else {
                    let ch = output[offset..].chars().next().unwrap();
                    if !ch.is_control() {
                        assert!(!autowrap, "frame {frame}: unguarded text at {offset}");
                    }
                    offset += ch.len_utf8();
                }
            }
            assert!(autowrap, "frame {frame}: restore before input handling");
            if frame == 0 || frame == 2 {
                assert!(output.contains("\r\n"), "keep explicit line advances");
            }
            assert!(output.contains("\x1b[?7h\x1b[?2026l"));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn render_first_diff_and_noop() {
        let (mut app, _rx) = running_app(100, 30);
        // First render: full, no clear.
        app.do_render();
        let out = render_writes(&app);
        assert!(out.contains("\x1b[?2026h")); // SYNC_BEGIN
        assert!(out.contains("hello world"));
        assert!(!out.contains("\x1b[2J"));

        // No-change render: only cursor positioning (no SYNC block).
        app.terminal.writes.borrow_mut().clear();
        app.do_render();
        let out = render_writes(&app);
        assert!(!out.contains("\x1b[?2026h"));

        // A change → differential render with SYNC + clear-line.
        app.chat
            .add_message(ChatMessage::new("u2".into(), ChatRole::User, "second"));
        app.terminal.writes.borrow_mut().clear();
        app.do_render();
        let out = render_writes(&app);
        assert!(out.contains("\x1b[?2026h"));
        assert!(out.contains("second"));

        // Streaming render bumps the spinner and re-requests.
        app.state.streaming = true;
        let frame = app.state.spinner_frame;
        app.do_render();
        assert_eq!(app.state.spinner_frame, frame + 1);
        app.state.streaming = false;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn render_full_redraw_triggers() {
        let (mut app, _rx) = running_app(100, 30);
        app.do_render();

        // Width change → full redraw with clear.
        app.terminal.cols = 80;
        app.terminal.writes.borrow_mut().clear();
        app.do_render();
        let out = render_writes(&app);
        assert!(out.contains("\x1b[2J"));
        assert_eq!(app.get_full_redraw_count(), 1);

        // Height change (non-Termux) → full redraw.
        app.terminal.rows = 40;
        app.terminal.writes.borrow_mut().clear();
        app.do_render();
        assert_eq!(app.get_full_redraw_count(), 2);

        // force_clear → full redraw.
        app.force_clear_next_render = true;
        app.do_render();
        assert_eq!(app.get_full_redraw_count(), 3);

        // clear_on_shrink with shrinking content.
        app.clear_on_shrink = true;
        for i in 0..20 {
            app.chat.add_message(ChatMessage::new(
                format!("m{i}"),
                ChatRole::User,
                "line with some content here",
            ));
        }
        app.do_render();
        app.chat.clear_messages();
        app.chat
            .add_message(ChatMessage::new("only".into(), ChatRole::User, "tiny"));
        app.do_render();
        assert_eq!(app.get_full_redraw_count(), 4);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn render_shrink_and_scroll_paths() {
        let (mut app, _rx) = running_app(100, 30);
        // Tall content.
        for i in 0..40 {
            app.chat.add_message(ChatMessage::new(
                format!("m{i}"),
                ChatRole::User,
                &format!("content line {i}"),
            ));
        }
        app.do_render();

        // Shrink within limits (clear_on_shrink off) → deleted-lines path.
        app.chat.clear_messages();
        app.chat
            .add_message(ChatMessage::new("u".into(), ChatRole::User, "short"));
        app.terminal.writes.borrow_mut().clear();
        app.do_render();
        let out = render_writes(&app);
        assert!(out.contains("\x1b[2K"));

        // Scroll the viewport up, then render a change above the viewport
        // → full redraw fallback.
        for i in 0..40 {
            app.chat.add_message(ChatMessage::new(
                format!("n{i}"),
                ChatRole::User,
                &format!("more content {i}"),
            ));
        }
        app.do_render();
        app.chat.scroll_up(20);
        app.chat
            .add_message(ChatMessage::new("x".into(), ChatRole::User, "tail"));
        app.do_render();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn render_with_overlay_and_autocomplete() {
        let (mut app, mut rx) = running_app(100, 30);
        // Overlay visible → composited render.
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(sample_sessions()),
            purpose: SessionsPurpose::Browse,
        });
        app.do_render();
        let out = render_writes(&app);
        assert!(out.contains("Sessions"));

        // Autocomplete popup visible → composited above the editor.
        app.autocomplete
            .show(vec![crate::components::autocomplete::AutocompleteItem {
                value: "/model".into(),
                label: "/model".into(),
                description: Some("select model".into()),
            }]);
        app.do_render();
        let out = render_writes(&app);
        assert!(out.contains("/model"));
        app.handle_cmd(UiCmd::OverlayCancel);
        app.autocomplete.hide();
        let _ = &mut rx;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn render_debug_env_paths_and_termux() {
        let _guard = crate::test_env::lock();
        let (mut app, _rx) = running_app(100, 30);

        // PI_TUI_DEBUG dumps render state.
        let old_debug = std::env::var_os("PI_TUI_DEBUG");
        std::env::set_var("PI_TUI_DEBUG", "1");
        app.do_render();
        let dir = std::env::temp_dir().join("tui");
        assert!(std::fs::read_dir(&dir).unwrap().count() > 0);

        // PI_DEBUG_REDRAW logs to ~/.future/tui/debug.log.
        let home = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        let old_redraw = std::env::var_os("PI_DEBUG_REDRAW");
        std::env::set_var("HOME", home.path());
        std::fs::create_dir_all(home.path().join(".future/tui")).unwrap();
        std::env::set_var("PI_DEBUG_REDRAW", "1");
        app.terminal.cols = 90; // force a full redraw reason
        app.do_render();
        let log = std::fs::read_to_string(home.path().join(".future/tui/debug.log")).unwrap();
        assert!(log.contains("width changed"));
        // Unwritable log path (a directory) — the open failure is swallowed.
        // A fresh HOME whose debug.log is already a directory exercises the
        // swallow path without racing a remove_file/create_dir (instrumented
        // runs slow the suite down and the previous file can be re-created
        // between the two calls).
        let blocked = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(blocked.path().join(".future/tui/debug.log")).unwrap();
        std::env::set_var("HOME", blocked.path());
        app.terminal.cols = 80;
        app.do_render();
        std::env::set_var("HOME", home.path());
        restore_env2("PI_TUI_DEBUG", old_debug);
        restore_env2("PI_DEBUG_REDRAW", old_redraw);
        restore_env2("HOME", old_home);

        // Termux: height change does NOT full-redraw.
        let old_termux = std::env::var_os("TERMUX_VERSION");
        std::env::set_var("TERMUX_VERSION", "0.118");
        app.terminal.rows = 35;
        let before = app.get_full_redraw_count();
        app.do_render();
        assert_eq!(app.get_full_redraw_count(), before);
        restore_env2("TERMUX_VERSION", old_termux);

        // Debug dump with an overlay focused: the editor has no cursor
        // marker → the dump records cursorPos=null.
        let old_debug2 = std::env::var_os("PI_TUI_DEBUG");
        std::env::set_var("PI_TUI_DEBUG", "1");
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(sample_sessions()),
            purpose: SessionsPurpose::Browse,
        });
        app.terminal.cols = 75; // width change → full render → dump
        app.do_render();
        app.hide_overlay();
        restore_env2("PI_TUI_DEBUG", old_debug2);
    }

    fn restore_env2(key: &str, old: Option<std::ffi::OsString>) {
        match old {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn restore_env2_both_arms() {
        let _guard = crate::test_env::lock();
        let old = std::env::var_os("FUTURE_TUI_APP_PROBE");
        restore_env2("FUTURE_TUI_APP_PROBE", Some("1".into()));
        assert_eq!(std::env::var("FUTURE_TUI_APP_PROBE").as_deref(), Ok("1"));
        restore_env2("FUTURE_TUI_APP_PROBE", None);
        assert!(std::env::var_os("FUTURE_TUI_APP_PROBE").is_none());
        restore_env2("FUTURE_TUI_APP_PROBE", old);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn kitty_image_bookkeeping() {
        let (mut app, _rx) = running_app(100, 30);
        // Render content carrying a kitty image id.
        app.chat.add_message(ChatMessage::new(
            "a1".into(),
            ChatRole::Assistant,
            "\x1b_Gi=42,f=100;AAAA\x1b\\",
        ));
        app.do_render();
        assert!(app.previous_kitty_image_ids.contains(&42));
        // Change the line → the image deletion path runs.
        app.chat
            .add_message(ChatMessage::new("a2".into(), ChatRole::Assistant, "text"));
        app.do_render();
    }

    // ─── Final app coverage push ──────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn wait_for_agent_retries_until_agent_appears() {
        // The mock rejects `list_models` until 1.3 s in: the first
        // try_connects fail (showing the retry message) before the agent
        // answers. The readiness flip is mock-driven rather than a late TCP
        // bind — a just-closed Windows port can hold connects until the next
        // listener binds, which made the first attempt succeed and the retry
        // path never run.
        let ready = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mock = AppMockAgent {
            not_ready: Some(ready.clone()),
            ..Default::default()
        };
        let (addr_str, _seen) = spawn_app_mock_with(mock).await;
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1300)).await;
            ready.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let (mut app, mut rx) = make_app_at(&addr_str, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        assert!(app.is_running());
        let joined = system_messages(&app).join("\n");
        assert!(joined.contains("retrying every 1s"), "{joined}");
        assert!(joined.contains("Connected to agent"), "{joined}");
        pump(&mut app, &mut rx).await;
        app.stop();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fake_terminal_exit_callback_setter() {
        let (mut app, _rx) = make_app(100, 30);
        app.terminal.set_exit_signal_callback(None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn startup_mock_override_variants() {
        async fn spawn_variant(
            overrides: std::collections::HashMap<String, String>,
            fail: std::collections::HashSet<String>,
        ) -> String {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let incoming = TcpIncoming::from_listener(listener, true, None).unwrap();
            let mock = AppMockAgent {
                overrides,
                fail,
                ..Default::default()
            };
            tokio::spawn(
                Server::builder()
                    .add_service(FutureAgentServer::new(mock))
                    .serve_with_incoming(incoming),
            );
            format!("127.0.0.1:{}", addr.port())
        }

        // new_session returns no sessionId → tolerated silently.
        let addr = spawn_variant(
            std::collections::HashMap::from([("new_session".to_string(), "{}".to_string())]),
            Default::default(),
        )
        .await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;
        app.stop();

        // --continue with an empty session list → no switch attempted.
        let addr = spawn_variant(
            std::collections::HashMap::from([(
                "list_sessions".to_string(),
                "{\"sessions\":[]}".to_string(),
            )]),
            Default::default(),
        )
        .await;
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                r#continue: true,
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;
        app.stop();

        // --continue where the switch itself fails.
        let addr = spawn_variant(
            Default::default(),
            std::collections::HashSet::from(["switch_session".to_string()]),
        )
        .await;
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                r#continue: true,
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        assert!(system_messages(&app)
            .iter()
            .any(|m| m.contains("Failed to continue session")));
        pump(&mut app, &mut rx).await;
        app.stop();

        // A cancelled fork at startup.
        let addr = spawn_variant(
            std::collections::HashMap::from([(
                "fork".to_string(),
                "{\"cancelled\":true}".to_string(),
            )]),
            Default::default(),
        )
        .await;
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                fork: Some("e1".into()),
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;
        app.stop();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_cancelled_via_live_mock() {
        let addr = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let incoming = TcpIncoming::from_listener(listener, true, None).unwrap();
            let mock = AppMockAgent {
                overrides: std::collections::HashMap::from([(
                    "fork".to_string(),
                    "{\"cancelled\":true}".to_string(),
                )]),
                ..Default::default()
            };
            tokio::spawn(
                Server::builder()
                    .add_service(FutureAgentServer::new(mock))
                    .serve_with_incoming(incoming),
            );
            format!("127.0.0.1:{}", addr.port())
        };
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        // ForkSelected with a cancelled fork → no state/message fetch.
        app.handle_cmd(UiCmd::ForkSelected {
            item: SelectItem {
                value: "e1".into(),
                label: "entry".into(),
                description: None,
            },
        });
        pump(&mut app, &mut rx).await;
        assert!(!last_system(&app).contains("Forked"));
        app.stop();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn startup_session_option_failures() {
        // A mock that fails the session-management commands.
        #[derive(Clone, Default)]
        struct FailAgent {
            fail: std::collections::HashSet<String>,
        }
        #[tonic::async_trait]
        impl FutureAgent for FailAgent {
            async fn execute_command(
                &self,
                request: tonic::Request<future_rpc::proto::RpcCommand>,
            ) -> Result<tonic::Response<future_rpc::proto::RpcResponse>, tonic::Status>
            {
                let cmd = request.into_inner();
                let fail = self.fail.contains(&cmd.r#type);
                // new_session reports a fresh id so the client subscribes.
                let data = if cmd.r#type == "new_session" {
                    "{\"sessionId\":\"s-new\"}"
                } else {
                    "{}"
                };
                Ok(tonic::Response::new(future_rpc::proto::RpcResponse {
                    id: cmd.id,
                    r#type: "response".into(),
                    command: cmd.r#type.clone(),
                    success: !fail,
                    data: data.into(),
                    error: if fail { "nope".into() } else { String::new() },
                    error_code: String::new(),
                    error_data: String::new(),
                    payload: None,
                }))
            }
            type StreamEventsStream = Pin<
                Box<
                    dyn tokio_stream::Stream<
                            Item = Result<future_rpc::proto::StreamEvent, tonic::Status>,
                        > + Send,
                >,
            >;
            async fn stream_events(
                &self,
                _request: tonic::Request<future_rpc::proto::StreamRequest>,
            ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
                Ok(tonic::Response::new(Box::pin(
                    futures_util::stream::pending(),
                )))
            }
        }
        async fn spawn_fail_agent_with(fail: std::collections::HashSet<String>) -> String {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let incoming = TcpIncoming::from_listener(listener, true, None).unwrap();
            tokio::spawn(
                Server::builder()
                    .add_service(FutureAgentServer::new(FailAgent { fail }))
                    .serve_with_incoming(incoming),
            );
            format!("127.0.0.1:{}", addr.port())
        }
        async fn spawn_fail_agent() -> String {
            spawn_fail_agent_with(
                [
                    "switch_session",
                    "list_sessions",
                    "fork",
                    "new_session",
                    "get_state",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            )
            .await
        }

        // --session failure.
        let addr = spawn_fail_agent().await;
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                session: Some("s1".into()),
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        assert!(system_messages(&app)
            .iter()
            .any(|m| m.contains("Failed to switch to session s1")));
        pump(&mut app, &mut rx).await;
        app.stop();

        // --continue failure (list_sessions fails).
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                r#continue: true,
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        assert!(system_messages(&app)
            .iter()
            .any(|m| m.contains("Failed to continue session")));
        pump(&mut app, &mut rx).await;
        app.stop();

        // --fork failure.
        let (mut app, mut rx) = make_app_at(
            &addr,
            &CliOptions {
                fork: Some("e1".into()),
                ..Default::default()
            },
        );
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        assert!(system_messages(&app)
            .iter()
            .any(|m| m.contains("Failed to fork session e1")));
        pump(&mut app, &mut rx).await;
        app.stop();

        // Default flow with only get_state failing: new_session succeeds
        // (client subscribes to the stream) and the refresh error path runs.
        let addr2 =
            spawn_fail_agent_with(["get_state"].iter().map(|s| s.to_string()).collect()).await;
        let (mut app, mut rx) = make_app_at(&addr2, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        pump(&mut app, &mut rx).await;
        app.stop();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn misc_small_paths() {
        let (mut app, mut rx) = make_app(100, 30);

        // stop_async delegates to stop.
        app.running = true;
        app.stop_async().await;
        assert!(!app.running);

        // parse_updated_at: rfc3339, naive, invalid.
        assert!(parse_updated_at("2026-01-01T00:00:00Z") > 0);
        assert!(parse_updated_at("2026-01-01 00:00:00") > 0);
        assert_eq!(parse_updated_at("garbage"), 0);

        // normalize_path with a "." component (platform-spelled path).
        let root = if cfg!(windows) { r"C:\tmp" } else { "/tmp" };
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            normalize_path(&format!("{root}{sep}.{sep}x")),
            format!("{root}{sep}x")
        );

        // FocusTarget::None drops key releases.
        app.focused = FocusTarget::None;
        app.handle_input("\x1b[97;1:3u");
        app.focused = FocusTarget::Input;

        // Input listeners: pass-through (None) and no-data rewrite.
        app.input_listeners.push(Box::new(|d| {
            if d == "quiet" {
                Some(InputListenerResult {
                    consume: false,
                    data: None,
                })
            } else {
                None
            }
        }));
        app.handle_input("quiet"); // result with no data → original continues
        app.handle_input("q"); // listener None arm (single char inserts)
        assert!(app.input.get_value().contains('q'));
        app.input_listeners.clear();

        // Hidden overlay: focus redirects to the editor on key input.
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(sample_sessions()),
            purpose: SessionsPurpose::Browse,
        });
        if let Some(entry) = app.overlay_stack.first_mut() {
            entry.hidden = true;
        }
        app.handle_input("\x1b[B"); // down arrow: focus redirects to editor
        assert_eq!(app.focused, FocusTarget::Input);
        app.hide_overlay(); // close the leftover hidden entry
        assert!(app.overlay_stack.is_empty());

        // Autocomplete selection: no active context → value replaces input.
        app.input.set_value("/model", None);
        app.autocomplete
            .show(vec![crate::components::autocomplete::AutocompleteItem {
                value: "/model x".into(),
                label: "/model x".into(),
                description: None,
            }]);
        app.apply_autocomplete_selection();
        assert_eq!(app.input.get_value(), "/model x");

        // Approval with an object requested_action (pretty-printed) and a
        // missing one (no preview block).
        app.handle_agent_event(&make_event(
            "approval_request",
            r#"{"approval_request_id":"a2","requested_action":{"cmd":"ls"}}"#,
        ));
        assert!(last_system(&app).contains("Approval Required"));
        app.handle_agent_event(&make_event(
            "approval_request",
            r#"{"approval_request_id":"a3"}"#,
        ));
        assert!(last_system(&app).contains("Approval Required"));

        // /model selector refused while streaming.
        app.state.streaming = true;
        app.handle_cmd(UiCmd::Submit("/model".into()));
        assert!(last_system(&app).contains("Cannot change model"));
        app.state.streaming = false;

        // Scoped overlay with an existing enabled set.
        app.enabled_model_ids = Some(vec!["openai/gpt-4o".into()]);
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Scoped,
        });
        assert!(!app.overlay_stack.is_empty());
        app.handle_cmd(UiCmd::OverlayCancel);

        // Fork overlay with no user messages → info message.
        app.handle_cmd(UiCmd::ForkMessagesLoaded(Ok(json_parse(
            r#"{"messages":[]}"#,
        ))));
        assert!(last_system(&app).contains("No user messages to fork from"));

        // Select overlay key dispatch (on_select/on_cancel closures).
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(sample_sessions()),
            purpose: SessionsPurpose::Browse,
        });
        app.handle_key("enter"); // selects the highlighted session → switch flow
        pump(&mut app, &mut rx).await;
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(sample_sessions()),
            purpose: SessionsPurpose::Browse,
        });
        app.handle_key("escape"); // → OverlayCancel through the channel
        pump_until_no_overlay(&mut app, &mut rx).await;

        // A replayed tool message with an Error prefix.
        app.apply_history_page(
            "s1",
            Ok(json_parse(
                r#"{"entries":[{"id":"t1","kind":"tool","role":"tool","blocks":[{"kind":"tool_result","text":"Error: failed","toolCallId":"call","isError":true}]}]}"#,
            )),
        );
        let last = app.chat.plain_messages().last().unwrap().clone();
        assert!(last.1.contains("Error: failed"));

        // apply_refresh_state with queued runs + terminal acks.
        let mut state = sample_state();
        state.agent_instance_id = None;
        app.apply_refresh_state(state);
        let state2: RpcSessionState = serde_json::from_value(json_parse(
            r#"{"sessionId":"s1","agentInstanceId":"agent-2","queuedRuns":[{"runId":"q1","runSequence":1,"clientRequestId":"c1","queuePosition":1,"acceptedAt":"2026-01-01","displayText":"queued work"}],"recentTerminalAcks":[{"run_id":"r-old","run_sequence":1,"client_request_id":"c2","state":"cancelled","reason":"user"},{"run_id":"r-sup","run_sequence":2,"client_request_id":"c3","state":"terminal","reason":"superseded"}]}"#,
        ))
        .unwrap();
        app.apply_refresh_state(state2);
        app.client.set_current_session_id("");

        // setup() again with a cwd (FilePathProvider cwd branch).
        app.state.cwd = "/tmp/sub".into();
        app.setup();

        let _ = &mut rx;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn live_spawn_successes() {
        let (addr, _seen) = spawn_app_mock().await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();

        // /clone success (state + messages fetched).
        app.handle_cmd(UiCmd::Submit("/clone".into()));
        pump_until_msg(&mut app, &mut rx, "Session cloned").await;

        // /new with model+thinking set (inheritance path).
        app.state.model = "openai/gpt-4o".into();
        app.state.thinking = "high".into();
        app.state.cwd = "/tmp".into();
        app.handle_cmd(UiCmd::Submit("/new".into()));
        pump_until_msg(&mut app, &mut rx, "New session started").await;

        // /model selector with models loaded → overlay with "current".
        app.handle_cmd(UiCmd::Submit("/model".into()));
        pump_until_overlay(&mut app, &mut rx).await;
        app.handle_cmd(UiCmd::OverlayCancel);

        // Selecting a session in the overlay drives the full switch flow.
        // Re-selecting the *current* session is a deliberate no-op (covered by
        // `session_menu_confirmation_switches_or_ignores_the_current`), so make
        // the current id distinct from the listed ones.
        app.state.session_id = "other-session".into();
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(sample_sessions()),
            purpose: SessionsPurpose::Browse,
        });
        app.handle_key("enter");
        pump_until_msg(&mut app, &mut rx, "Switched to session").await;

        // A clone-cancelled response.
        let _ = &mut app;

        // PromptAck running state binding.
        app.handle_cmd(UiCmd::PromptAck {
            local_id: "x".into(),
            result: Ok(crate::rpc::types::RunAck {
                run_id: "r1".into(),
                run_epoch: 1,
                accepted_state: "running".into(),
                run_sequence: None,
                queue_position: None,
            }),
        });
        app.stop();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn render_pipeline_leftovers() {
        let (mut app, _rx) = running_app(100, 30);

        // apply_line_resets skips empty lines.
        let lines = app.apply_line_resets(vec![String::new(), "text".into()]);
        assert!(lines[0].is_empty());
        assert!(lines[1].ends_with(SEGMENT_RESET));

        // position_hardware_cursor: move up / down / show-hardware-cursor.
        app.show_hardware_cursor = true;
        app.position_hardware_cursor(Some((5, 3)), 10);
        assert_eq!(app.hardware_cursor_row, 5);
        app.position_hardware_cursor(Some((2, 1)), 10); // up
        assert_eq!(app.hardware_cursor_row, 2);
        app.position_hardware_cursor(None, 10); // no-op
        app.position_hardware_cursor(Some((0, 0)), 0); // zero lines → no-op

        // Kitty expand/delete: previous lines with images get re-deleted.
        app.previous_lines = vec![
            "plain".to_string(),
            "\x1b_Gi=7,f=100;AAAA\x1b\\".to_string(),
        ];
        app.previous_kitty_image_ids = [7].into_iter().collect();
        let expanded = app.expand_last_changed_for_kitty_images(0, 0);
        assert_eq!(expanded, 1);
        let del = app.delete_changed_kitty_images(0, 1);
        assert!(del.contains("i=7"));

        // full_render with clear deletes kitty images + clears the screen.
        app.terminal.writes.borrow_mut().clear();
        app.full_render(&["line".to_string()], 100, 30, Some((0, 2)), true);
        let out = render_writes(&app);
        assert!(out.contains("\x1b[2J"));

        // do_render when not running is a no-op.
        app.running = false;
        app.terminal.writes.borrow_mut().clear();
        app.do_render();
        assert!(render_writes(&app).is_empty());
        app.running = true;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn final_small_paths() {
        let (mut app, mut rx) = make_app(100, 30);

        // Input component callbacks → UiCmd messages.
        app.input.set_value("hello", None);
        app.input.handle_key("enter"); // onSubmit
        app.input.handle_key("escape"); // onEscape
        app.input.handle_key("a"); // onChange (insert fires it)
        pump(&mut app, &mut rx).await;

        // TerminalIo wrapper used by run_interactive; a host with no console to
        // build one from (redirected Windows runners) simply has none to wrap.
        if let Some(mut real) = crate::terminal::terminal_or_skip() {
            crate::app::TerminalIo::set_exit_signal_callback(&mut real, None);
        }

        // tool_start without args; usage without the usage key.
        app.handle_agent_event(&make_event(
            "tool_start",
            r#"{"tool_id":"t1","tool_name":"read"}"#,
        ));
        app.handle_agent_event(&make_event("tool_end", r#"{"tool_id":"t1"}"#));
        pump(&mut app, &mut rx).await;
        let toks = app.state.tokens_in;
        app.handle_agent_event(&make_event("usage", r#"{"nope":1}"#));
        assert_eq!(app.state.tokens_in, toks);
        pump(&mut app, &mut rx).await;

        // Two overlays: hiding the focused top one redirects to the next.
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(sample_sessions()),
            purpose: SessionsPurpose::Browse,
        });
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Selector,
        });
        // Hide the top (model) overlay while it's focused, then a key press
        // redirects focus to the sessions overlay beneath.
        if let Some(top) = app.get_top_overlay_index() {
            app.overlay_stack[top].hidden = true;
        }
        app.handle_input("\x1b[B");
        assert!(matches!(app.focused, FocusTarget::Overlay(_)));
        app.hide_overlay();
        app.hide_overlay();

        // CycleModel with an EMPTY scoped list falls through to the RPC.
        app.enabled_model_ids = Some(vec![]);
        app.handle_key_action(KeyAction::CycleModel);
        pump(&mut app, &mut rx).await;
        app.enabled_model_ids = None;

        // Scoped selector's on_save/on_cancel closures.
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Scoped,
        });
        app.handle_key("enter"); // saves the scope
        pump(&mut app, &mut rx).await;
        assert!(last_system(&app).contains("enabled"));
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Scoped,
        });
        app.handle_key("escape"); // cancels
        pump(&mut app, &mut rx).await;
        assert!(app.overlay_stack.is_empty());

        // Help component downcasts (as_any/as_any_mut callable).
        app.show_help_overlay();
        {
            let entry = &mut app.overlay_stack[0];
            assert!(entry
                .component
                .as_any()
                .downcast_ref::<crate::app::tests::HelpProbe>()
                .is_none());
            let _ = entry.component.as_any_mut();
        }
        app.hide_overlay();

        // restore_focus to a lower overlay when the top closes.
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(sample_sessions()),
            purpose: SessionsPurpose::Browse,
        });
        app.show_help_overlay();
        app.hide_overlay(); // closes help → focus back to sessions overlay
        assert!(matches!(app.focused, FocusTarget::Overlay(_)));
        app.hide_overlay();

        // set_focus(None) from an overlay focus.
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(sample_sessions()),
            purpose: SessionsPurpose::Browse,
        });
        app.set_focus(FocusTarget::None);
        assert_eq!(app.focused, FocusTarget::None);
        app.hide_overlay();

        // Refresh clears connection_lost with a message.
        app.connection_lost = true;
        app.apply_refresh_state(sample_state());
        assert!(!app.connection_lost);
        assert!(system_messages(&app)
            .iter()
            .any(|m| m.contains("Reconnected")));
        app.client.set_current_session_id("");

        // on_tick: render due while streaming re-requests a render.
        app.state.streaming = true;
        app.request_render(true);
        app.on_tick();
        app.state.streaming = false;

        // composite_overlays with every overlay hidden → base unchanged.
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(sample_sessions()),
            purpose: SessionsPurpose::Browse,
        });
        if let Some(top) = app.get_top_overlay_index() {
            app.overlay_stack[top].hidden = true;
        }
        let base = vec!["row".to_string(); 30];
        let out = app.composite_overlays(base.clone(), 100, 30);
        assert_eq!(out, base);
        app.hide_overlay();

        let _ = &mut rx;
    }

    pub(crate) struct HelpProbe; // downcast probe (never matches)

    /// Test double: renders nothing / wants key releases, as configured.
    struct ProbeComponent {
        lines: usize,
        wants_release: bool,
        render_only_at: Option<usize>,
    }

    impl Component for ProbeComponent {
        fn render(&mut self, width: usize) -> Vec<String> {
            // `render_only_at`: produce lines only at one width (drives the
            // measure-vs-layout empty branches in composite_overlays).
            if let Some(w) = self.render_only_at {
                if width != w {
                    return Vec::new();
                }
            }
            (0..self.lines).map(|i| format!("probe {i}")).collect()
        }
        fn wants_key_release(&self) -> bool {
            self.wants_release
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn final_final_paths() {
        let (mut app, mut rx) = make_app(100, 30);

        // Keybinding closures with exact key ids.
        for key in ["pageUp", "pageDown"] {
            app.handle_key(key);
            pump(&mut app, &mut rx).await;
        }

        // Release events reach a component that wants them.
        app.show_overlay(
            Box::new(ProbeComponent {
                lines: 1,
                wants_release: true,
                render_only_at: None,
            }),
            OverlayOptions::default(),
        );
        app.handle_input("\x1b[97;1:3u"); // passes the filter
        app.hide_overlay();

        // A component rendering zero lines is skipped in compositing.
        app.show_overlay(
            Box::new(ProbeComponent {
                lines: 0,
                wants_release: false,
                render_only_at: None,
            }),
            OverlayOptions::default(),
        );
        {
            let entry = &mut app.overlay_stack[0];
            let _ = entry.component.as_any_mut();
        }
        let base = vec!["row".to_string(); 30];
        let out = app.composite_overlays(base.clone(), 100, 30);
        assert_eq!(out, base);
        app.hide_overlay();

        // Renders at the measure width but empty at the layout width.
        app.show_overlay(
            Box::new(ProbeComponent {
                lines: 2,
                wants_release: false,
                render_only_at: Some(100),
            }),
            OverlayOptions::default(),
        );
        let base = vec!["row".to_string(); 30];
        let out = app.composite_overlays(base.clone(), 100, 30);
        assert_eq!(out, base);
        app.hide_overlay();

        // set_focus on a missing overlay id just records the target (the
        // component lookups are no-ops).
        app.set_focus(FocusTarget::Overlay(999));
        assert_eq!(app.focused, FocusTarget::Overlay(999));
        app.set_focus(FocusTarget::Input);

        // Non-ASCII single char takes the printable fallback.
        app.input.set_value("", None);
        app.handle_input("é");
        assert_eq!(app.input.get_value(), "é");

        // Autocomplete visible + a non-navigation key falls through.
        app.autocomplete
            .show(vec![crate::components::autocomplete::AutocompleteItem {
                value: "/model".into(),
                label: "/model".into(),
                description: None,
            }]);
        app.handle_key("left"); // editor key — ac stays open
        assert!(app.autocomplete.is_visible());
        // Tab with a visible popup accepts the completion (no submit).
        app.handle_key("tab");
        assert!(!app.autocomplete.is_visible());
        assert_eq!(app.input.get_value(), "/model");

        // Empty-token context: replace wholesale.
        app.input.set_value("/", None);
        app.trigger_autocomplete();
        pump(&mut app, &mut rx).await;
        if app.autocomplete.is_visible() {
            app.apply_autocomplete_selection();
        }
        assert!(app.input.get_value().starts_with('/'));

        // delete_changed_kitty_images with an inverted range is empty.
        assert!(app.delete_changed_kitty_images(5, 2).is_empty());

        // stop() cursor moves (up and down).
        app.previous_lines = vec!["a".into(), "b".into(), "c".into()];
        app.hardware_cursor_row = 0;
        app.stop(); // line_diff > 0 → move down write
        let (mut app2, _rx2) = make_app(100, 30);
        app2.previous_lines = vec!["a".into(), "b".into()];
        app2.hardware_cursor_row = 5;
        app2.stop(); // line_diff < 0 → move up write
        let _ = &mut rx;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn do_render_deleted_line_variants() {
        let (mut app, _rx) = running_app(100, 30);
        // Big content, then moderate shrink (≤ h) → the clear-lines path.
        for i in 0..20 {
            app.chat.add_message(ChatMessage::new(
                format!("m{i}"),
                ChatRole::User,
                &format!("content {i}"),
            ));
        }
        app.do_render();
        app.chat.clear_messages();
        for i in 0..12 {
            app.chat.add_message(ChatMessage::new(
                format!("n{i}"),
                ChatRole::User,
                &format!("smaller {i}"),
            ));
        }
        app.terminal.writes.borrow_mut().clear();
        app.do_render();
        let out = render_writes(&app);
        assert!(out.contains("\x1b[2K"));

        // Viewport moved up while content shrinks → full redraw.
        let (mut app, _rx) = running_app(100, 10);
        for i in 0..30 {
            app.chat.add_message(ChatMessage::new(
                format!("m{i}"),
                ChatRole::User,
                &format!("line {i}"),
            ));
        }
        app.do_render();
        app.chat.scroll_up(25);
        app.chat.clear_messages();
        app.chat
            .add_message(ChatMessage::new("x".into(), ChatRole::User, "one"));
        app.do_render(); // viewport above content → full render

        // Overlong line in a diff render is truncated, not crashed.
        let (mut app, _rx) = running_app(20, 10);
        app.do_render();
        app.chat.add_message(ChatMessage::new(
            "w".into(),
            ChatRole::User,
            "this line is definitely much wider than twenty columns",
        ));
        app.do_render();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn do_render_append_start_and_scroll() {
        let (mut app, _rx) = running_app(100, 10);
        app.do_render();
        // White-box: drop the tail of previous_lines → the next identical
        // frame looks like a pure append → append_start path.
        let keep = app.previous_lines.len() - 2;
        app.previous_lines.truncate(keep);
        app.do_render();

        // Diff change below the visible viewport → scroll-to-row path.
        let (mut app, _rx) = running_app(100, 6);
        for i in 0..12 {
            app.chat.add_message(ChatMessage::new(
                format!("s{i}"),
                ChatRole::User,
                &format!("scroll target row {i}"),
            ));
        }
        app.do_render();
        app.chat.add_message(ChatMessage::new(
            "s12".into(),
            ChatRole::User,
            "tail change",
        ));
        app.do_render();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn do_render_deleted_lines_move_up() {
        let (mut app, _rx) = running_app(100, 30);
        app.do_render();
        // Cursor parked low, then a prefix-shrink render → move-up write.
        app.hardware_cursor_row = 20;
        app.previous_lines
            .extend((0..8).map(|i| format!("stale {i}")));
        app.terminal.writes.borrow_mut().clear();
        app.do_render();
        assert!(render_writes(&app).contains("\x1b["));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn do_render_deleted_lines_prefix_shrink() {
        let (mut app, _rx) = running_app(100, 30);
        app.do_render();
        // White-box: pretend the last render had 8 more tail lines → the
        // new frame is a strict prefix → the deleted-lines diff path.
        app.previous_lines
            .extend((0..8).map(|i| format!("stale {i}")));
        app.terminal.writes.borrow_mut().clear();
        app.do_render();
        let out = render_writes(&app);
        assert!(out.contains("\x1b[2K")); // cleared in place

        // Too many deleted lines (> height) → full redraw (clears screen).
        app.previous_lines
            .extend((0..40).map(|i| format!("stale {i}")));
        app.terminal.writes.borrow_mut().clear();
        app.do_render();
        assert!(render_writes(&app).contains("\x1b[2J"));

        // Viewport above the shrunk content → full redraw.
        app.previous_lines
            .extend((0..3).map(|i| format!("stale {i}")));
        app.previous_viewport_top = 500;
        app.terminal.writes.borrow_mut().clear();
        app.do_render();
        assert!(render_writes(&app).contains("\x1b[2J"));

        // A change above the viewport → full redraw.
        app.previous_viewport_top = 500;
        app.chat.clear_messages();
        app.chat
            .add_message(ChatMessage::new("z".into(), ChatRole::User, "fresh"));
        app.terminal.writes.borrow_mut().clear();
        app.do_render();
        assert!(render_writes(&app).contains("\x1b[2J"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn surgical_leftovers() {
        let (mut app, mut rx) = make_app(100, 30);

        // normalize_path with a leading ./ (CurDir at the start).
        assert_eq!(normalize_path("./x"), "x");

        // overlay_id_of_focus outside overlay focus → None.
        app.focused = FocusTarget::Input;
        assert!(app.overlay_id_of_focus().is_none());

        // Printable char routed to the open overlay's component.
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(sample_sessions()),
            purpose: SessionsPurpose::Browse,
        });
        app.input.set_value("", None);
        app.handle_input("é");
        assert!(app.input.get_value().is_empty()); // overlay got it
        app.hide_overlay();

        // Autocomplete overlap completion through the real manager.
        app.input.set_value("/mo", None);
        app.trigger_autocomplete();
        pump(&mut app, &mut rx).await;
        if app.autocomplete.is_visible() {
            app.apply_autocomplete_selection();
        }
        assert!(app.input.get_value().starts_with("/m"));

        // do_render with the help overlay open renders the help component.
        app.running = true;
        app.show_help_overlay();
        app.do_render();
        let out = render_writes(&app);
        assert!(out.contains("future-tui")); // help card content
        app.hide_overlay();

        // Line resets skip kitty image lines.
        let lines = app.apply_line_resets(vec!["\x1b_Gi=9;AAAA\x1b\\".into()]);
        assert!(!lines[0].ends_with(SEGMENT_RESET));

        // query_cell_size writes when image capability exists.
        let _guard = crate::test_env::lock();
        crate::terminal_image::set_capabilities(crate::terminal_image::TerminalCapabilities {
            images: crate::terminal_image::ImageProtocol::Kitty,
            true_color: true,
            hyperlinks: true,
        });
        app.terminal.writes.borrow_mut().clear();
        app.query_cell_size();
        assert!(render_writes(&app).contains("\x1b[16t"));
        crate::terminal_image::set_capabilities(crate::terminal_image::TerminalCapabilities {
            images: crate::terminal_image::ImageProtocol::None,
            true_color: false,
            hyperlinks: false,
        });
        drop(_guard);

        // A dead-region: apply_status with a model found in the list.
        let mut s = sample_state();
        s.model = None;
        app.apply_status(&s, &sample_models(), None);
        let mut s2 = sample_state();
        s2.model = Some("gpt-4o".into()); // bare id matches the model list
        app.apply_status(&s2, &sample_models(), None);
        assert!(last_system(&app).contains("Provider: openai"));

        // show_model_selector refuses while streaming (direct call — the
        // slash arm pre-checks it).
        app.state.streaming = true;
        app.show_model_selector();
        assert!(last_system(&app).contains("Cannot change model"));
        app.state.streaming = false;

        // Dangling overlay focus + a key press (redirect block no-ops).
        app.set_focus(FocusTarget::Overlay(999));
        app.handle_input("\x1b[B");
        app.set_focus(FocusTarget::Input);

        // Lost queued runs on agent restart.
        app.state.session_id = "s1".into();
        app.apply_refresh_state(sample_state()); // registers agent instance? (sample has none)
        let with_agent: RpcSessionState = serde_json::from_value(json_parse(
            r#"{"sessionId":"s1","agentInstanceId":"agent-1"}"#,
        ))
        .unwrap();
        app.apply_refresh_state(with_agent);
        // Track a queued run client-side, then the agent restarts.
        app.handle_cmd(UiCmd::PromptAck {
            local_id: "u1".into(),
            result: Ok(crate::rpc::types::RunAck {
                run_id: "q1".into(),
                run_epoch: 1,
                accepted_state: "queued".into(),
                run_sequence: None,
                queue_position: Some(1),
            }),
        });
        let restarted: RpcSessionState = serde_json::from_value(json_parse(
            r#"{"sessionId":"s1","agentInstanceId":"agent-2","recentTerminalAcks":[{"run_id":"r-f","run_sequence":1,"client_request_id":"c","state":"failed","reason":"error"}]}"#,
        ))
        .unwrap();
        app.apply_refresh_state(restarted);
        app.client.set_current_session_id("");

        let _ = &mut rx;
    }

    // ─── Final uncovered-line push ────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn on_tick_fires_pending_ac_query_after_deadline() {
        let (mut app, mut rx) = make_app(100, 30);

        // Deadline set but not yet due → query stays pending.
        app.pending_ac_query = Some(("/m".into(), 2));
        app.ac_query_deadline = Some(Instant::now() + Duration::from_secs(60));
        app.on_tick();
        assert!(app.pending_ac_query.is_some());

        // Deadline elapsed → the pending query fires.
        app.ac_query_deadline = Some(Instant::now() - Duration::from_millis(1));
        app.on_tick();
        assert!(app.pending_ac_query.is_none());
        // The sync slash query produced items → AcItems queued.
        let mut saw_ac = false;
        while let Ok(cmd) = rx.try_recv() {
            saw_ac |= matches!(cmd, UiCmd::AcItems(_));
            app.handle_cmd(cmd);
        }
        assert!(saw_ac);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn startup_new_session_unsupported_continues() {
        // Agent rejects new_session → the startup Err arm is a silent
        // continue with the current (refreshed) session.
        let mock = AppMockAgent {
            fail: ["new_session".to_string()].into_iter().collect(),
            ..Default::default()
        };
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        assert!(app.is_running());
        assert_eq!(app.state.session_id, "s1");
        pump(&mut app, &mut rx).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn terminal_callbacks_forward_into_input_channel() {
        // The callbacks App::start hands to TerminalIo::start forward input
        // and resize events into the app's input channel.
        let (addr, _seen) = spawn_app_mock().await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        let (in_tx, mut in_rx) = mpsc::unbounded_channel();
        app.start(in_tx).await.unwrap();
        assert!(app.is_running());
        let term = &mut app.terminal;
        (term.on_input.as_mut().unwrap())("abc".to_string());
        (term.on_resize.as_mut().unwrap())();
        assert!(matches!(in_rx.try_recv(), Ok(UiInput::Input(_))));
        assert!(matches!(in_rx.try_recv(), Ok(UiInput::Resize)));
        pump(&mut app, &mut rx).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn startup_explicit_session_state_skips_new_session() {
        // get_state reports an explicit session → startup does not call
        // new_session at all.
        let mock = AppMockAgent {
            overrides: [(
                "get_state".to_string(),
                r#"{"sessionId":"s9","explicitSession":true}"#.to_string(),
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let (addr, seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        assert!(app.is_running());
        assert_eq!(app.state.session_id, "s9");
        assert!(!seen.lock().unwrap().iter().any(|(t, _)| t == "new_session"));
        pump(&mut app, &mut rx).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn slash_clone_cancelled_skips_state_refresh() {
        let mock = AppMockAgent {
            overrides: [("clone".to_string(), r#"{"cancelled":true}"#.to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.handle_submit("/clone");
        pump(&mut app, &mut rx).await;
        // Cancelled clone: no session switch, no state/messages reload.
        assert_ne!(app.state.session_id, "s-new");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn slash_new_session_response_variants() {
        // Empty thinking level → the None arm (synchronous, pre-RPC).
        let (mut app, mut rx) = make_app(100, 30);
        app.state.thinking.clear();
        app.handle_submit("/new");
        pump(&mut app, &mut rx).await; // RPC fails (no agent) — harmless

        // Ok response without a sessionId → no follow-up get_state.
        let mock = AppMockAgent {
            overrides: [("new_session".to_string(), "{}".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.handle_submit("/new");
        pump(&mut app, &mut rx).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn slash_new_clears_the_previous_transcript() {
        let (addr, _seen) = spawn_app_mock().await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.chat.add_message(ChatMessage::new(
            "old".into(),
            ChatRole::User,
            "previous session question",
        ));
        app.handle_submit("/new");
        pump_until_msg(&mut app, &mut rx, "New session started").await;
        // Only the confirmation remains — the old conversation is gone.
        let texts: Vec<String> = app
            .chat
            .plain_messages()
            .iter()
            .map(|(_, content)| content.clone())
            .collect();
        assert!(
            !texts.iter().any(|t| t.contains("previous session")),
            "old transcript survived /new: {texts:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn overlay_component_escape_fires_on_cancel_closures() {
        let (mut app, mut rx) = make_app(100, 30);

        // Generic select overlay still backed by `SelectList` (the fork
        // picker; sessions/tree moved to the popup-menu framework). App-level
        // escape hides overlays directly, so feed the component its own escape.
        app.handle_cmd(UiCmd::ForkMessagesLoaded(Ok(serde_json::json!({
            "messages": [
                {"id": "m1", "text": "first question"},
                {"id": "m2", "text": "second question"},
            ]
        }))));
        let idx = app.get_top_overlay_index().unwrap();
        app.overlay_stack[idx].component.handle_input("escape");
        let mut saw_cancel = false;
        while let Ok(cmd) = rx.try_recv() {
            saw_cancel |= matches!(cmd, UiCmd::OverlayCancel);
            app.handle_cmd(cmd);
        }
        assert!(saw_cancel);
        assert!(app.overlay_stack.is_empty());

        // Scoped models selector.
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Scoped,
        });
        let idx = app.get_top_overlay_index().unwrap();
        app.overlay_stack[idx].component.handle_input("escape");
        let mut saw_cancel = false;
        while let Ok(cmd) = rx.try_recv() {
            saw_cancel |= matches!(cmd, UiCmd::OverlayCancel);
            app.handle_cmd(cmd);
        }
        assert!(saw_cancel);
        assert!(app.overlay_stack.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lost_queued_runs_marked_on_agent_restart() {
        // get_state script: instance A with q1 queued, then instance B.
        let state_a = r#"{"sessionId":"s1","agentInstanceId":"agent-a","queuedRuns":[{"runId":"q1","runSequence":1,"clientRequestId":"c","state":"queued","queuePosition":1,"acceptedAt":"2026-08-07T00:00:00Z","displayText":"hi"}]}"#.to_string();
        let state_b = r#"{"sessionId":"s1","agentInstanceId":"agent-b"}"#.to_string();
        let mock = AppMockAgent {
            state_script: Some(std::sync::Arc::new(std::sync::Mutex::new(vec![
                state_a, state_b,
            ]))),
            ..Default::default()
        };
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        let _ = &mut rx;
        // Pre-set the client's session: apply_refresh_state syncs a changed
        // session id via set_current_session_id, which CLEARS the run
        // bookkeeping — that would wipe q1 before the restart refresh.
        app.client.set_current_session_id("s1");
        app.refresh_direct().await; // instance A; q1 queued in the chat
        app.refresh_direct().await; // instance B → restart → q1 marked lost
        assert_eq!(app.state.session_id, "s1");
    }

    #[test]
    fn composite_line_at_fits_within_width() {
        // Result fits → early return, no slice_by_column safeguard.
        let merged = App::<FakeTerminal>::composite_line_at("abcdef", "XY", 2, 2, 20);
        assert!(merged.contains("XY"));

        // Overlay spilling past the terminal width → the slice safeguard.
        let truncated = App::<FakeTerminal>::composite_line_at("abcdef", "XY", 5, 5, 6);
        assert!(visible_width(&truncated) <= 6);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn autocomplete_unicode_middle_selection_preserves_suffix_and_cursor() {
        use crate::components::autocomplete::{AutocompleteContext, AutocompleteProvider};
        struct FixedProvider;
        impl AutocompleteProvider for FixedProvider {
            fn name(&self) -> &str {
                "fixed"
            }
            fn r#match(&self, text: &str, cursor_pos: usize) -> Option<AutocompleteContext> {
                Some(AutocompleteContext {
                    text: text.into(),
                    cursor_pos,
                    token: "x".into(),
                    token_start: 2,
                })
            }
            fn get_completions(&self, _: &AutocompleteContext) -> Vec<AutocompleteItem> {
                vec![AutocompleteItem {
                    value: "中文.md".into(),
                    label: "中文.md".into(),
                    description: None,
                }]
            }
        }
        // The provider names itself; nothing else reads it, so assert it here.
        assert_eq!(FixedProvider.name(), "fixed");
        let (mut app, mut rx) = make_app(100, 30);
        app.ac_manager.destroy();
        app.ac_manager.register(Box::new(FixedProvider));
        app.input.set_value("a x tail", Some(3));
        app.trigger_autocomplete();
        pump(&mut app, &mut rx).await;
        app.apply_autocomplete_selection();
        assert_eq!(app.input.get_value(), "a 中文.md tail");
        assert_eq!(app.input.cursor(), 7);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn autocomplete_cache_result_reaches_the_registered_provider() {
        let (mut app, mut rx) = make_app(100, 30);
        app.input.set_value("/model ", None);
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Autocomplete,
        });
        pump(&mut app, &mut rx).await;
        assert!(app.autocomplete.is_visible());
        assert!(app.autocomplete.get_selected_item().is_some());
        app.apply_autocomplete_selection();
        assert!(app.input.get_value().starts_with("/model "));
        assert!(!app.input.get_value().contains("/model /model"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn autocomplete_overlap_mismatch_keeps_full_value() {
        use crate::components::autocomplete::{AutocompleteContext, AutocompleteProvider};

        // Provider whose item value shares no prefix with the text before
        // the token — the overlap check never matches.
        struct FixedProvider;
        impl AutocompleteProvider for FixedProvider {
            fn name(&self) -> &str {
                "fixed"
            }
            fn r#match(&self, text: &str, cursor_pos: usize) -> Option<AutocompleteContext> {
                Some(AutocompleteContext {
                    text: text.to_string(),
                    cursor_pos,
                    token: "y".to_string(),
                    token_start: text.len() - 1,
                })
            }
            fn get_completions(&self, _ctx: &AutocompleteContext) -> Vec<AutocompleteItem> {
                vec![AutocompleteItem {
                    value: "zzz".into(),
                    label: "zzz".into(),
                    description: None,
                }]
            }
        }

        let (mut app, mut rx) = make_app(100, 30);
        let provider = FixedProvider;
        assert_eq!(provider.name(), "fixed");
        app.ac_manager.destroy();
        app.ac_manager.register(Box::new(provider));
        app.input.set_value("ay", None);
        app.trigger_autocomplete();
        pump(&mut app, &mut rx).await; // deliver AcItems
        assert!(app.autocomplete.is_visible());
        app.apply_autocomplete_selection();
        // No overlap: before + full value + after.
        assert_eq!(app.input.get_value(), "azzz");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn render_with_visible_but_empty_autocomplete() {
        let (mut app, _rx) = running_app(100, 30);
        // Visible popup with zero items renders no lines.
        app.autocomplete.show(vec![]);
        assert!(app.autocomplete.is_visible());
        app.do_render();
        app.autocomplete.hide();
        app.do_render();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pure_append_render_via_empty_overlay() {
        let (mut app, _rx) = running_app(100, 30);
        app.do_render(); // frame 1: chat + editor + footer (< 30 lines)
        let base = app.previous_lines.len();
        assert!(base < 30);
        // An overlay whose component renders nothing: compositing pads the
        // frame to the terminal height with blank lines — a pure tail
        // append with no in-place change. Pushed directly: show_overlay's
        // request_render(true) would clear the diff baseline.
        app.overlay_stack.push(OverlayEntry {
            id: 999,
            component: Box::new(ProbeComponent {
                lines: 0,
                wants_release: false,
                render_only_at: None,
            }),
            options: OverlayOptions::default(),
            pre_focus: FocusTarget::Input,
            hidden: false,
            focus_order: 0,
        });
        app.do_render();
        assert_eq!(app.previous_lines.len(), 30);
        app.overlay_stack.clear();
        app.do_render();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn final_closure_arms() {
        let (mut app, mut rx) = running_app(100, 30);
        let _ = &mut rx;

        // Model/session argument completions invoke the cache closures.
        app.input.set_value("/model g", None);
        app.trigger_autocomplete();
        app.input.set_value("/clone s", None);
        app.trigger_autocomplete();

        // Footer tool_elapsed Some arm.
        app.state.tool_start_time = Some(Instant::now());
        app.do_render();
        app.state.tool_start_time = None;

        // Two visible overlays → the focus-order sort closure runs.
        for (id, focus_order) in [(991, 2), (992, 1)] {
            app.overlay_stack.push(OverlayEntry {
                id,
                component: Box::new(ProbeComponent {
                    lines: 1,
                    wants_release: false,
                    render_only_at: None,
                }),
                options: OverlayOptions::default(),
                pre_focus: FocusTarget::Input,
                hidden: false,
                focus_order,
            });
        }
        let base = vec!["row".to_string(); 30];
        let _ = app.composite_overlays(base, 100, 30);
        app.overlay_stack.clear();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn overwide_line_truncated_in_diff_render() {
        // The autocomplete popup enforces a minimum width of 12 — wider
        // than a w=10 terminal — and its lines are composited into the
        // frame raw. The diff render's truncate arm keeps that graceful.
        let (mut app, _rx) = running_app(10, 12);
        app.do_render(); // frame 1 (full)
        app.autocomplete
            .show(vec![crate::components::autocomplete::AutocompleteItem {
                value: "/model".into(),
                label: "/model".into(),
                description: None,
            }]);
        app.do_render(); // diff render composites the over-wide popup line
        assert!(app
            .previous_lines
            .iter()
            .any(|l| visible_width(l) > 10 || !l.is_empty()));
        app.autocomplete.hide();
        app.do_render();
    }

    // ─── popup menus, /copy, /transcript, /editor, /providers, /usage ──

    /// The components of the top overlay, as an expected concrete type.
    fn top_overlay_is<T: 'static>(app: &App<FakeTerminal>) -> bool {
        app.get_top_overlay_index()
            .is_some_and(|idx| app.overlay_stack[idx].component.as_any().is::<T>())
    }

    /// Press `key` on the top overlay and apply everything it emitted.
    fn press_on_overlay(
        app: &mut App<FakeTerminal>,
        rx: &mut mpsc::UnboundedReceiver<UiCmd>,
        key: &str,
    ) {
        if let Some(idx) = app.get_top_overlay_index() {
            app.overlay_stack[idx].component.handle_input(key);
        }
        while let Ok(cmd) = rx.try_recv() {
            app.handle_cmd(cmd);
        }
    }

    /// A clipboard whose writes are recorded instead of spawning a real
    /// `pbcopy`/`xclip`. The candidate list is a single synthetic program, not
    /// the host's own: a real candidate list is host-dependent (a Linux CI
    /// runner without X11/Wayland has none, and `copy()` would then take the
    /// OSC 52 fallback and record nothing), while an empty one would skip the
    /// native path everywhere. The OSC 52 fallback has its own test
    /// ([`copy_without_a_native_backend_requests_osc52`]).
    fn recording_clipboard(
        log: &std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    ) -> crate::clipboard::Clipboard {
        let sink = std::sync::Arc::clone(log);
        crate::clipboard::Clipboard::with_parts(
            Box::new(move |program: &str, _args: &[String], input: &str| {
                sink.lock().unwrap().push(format!("{program}:{input}"));
                Ok(())
            }),
            vec![("recorder".to_string(), Vec::new())],
            false,
        )
    }

    /// Snapshot of the recorded clipboard writes.
    fn clipboard_log(log: &std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> Vec<String> {
        log.lock().unwrap().clone()
    }

    // ─── `future skills …` fakes (no test may spawn a real process) ────

    /// One invocation a fake [`SkillsCli`] runner saw.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct SkillCall {
        program: String,
        args: Vec<String>,
    }

    /// A `SkillsCli` stand-in whose runner records every call and answers from
    /// `answer` — the injected-spawner seam the CLI module exposes, used here so
    /// that no test can run a real `future skills …`.
    fn fake_skills_cli<F>(
        calls: &std::sync::Arc<std::sync::Mutex<Vec<SkillCall>>>,
        answer: F,
    ) -> Arc<SkillsCli>
    where
        F: Fn(&[String]) -> Result<(i32, String, String), String> + Send + Sync + 'static,
    {
        let sink = std::sync::Arc::clone(calls);
        let runner: crate::skills_cli::SkillRunner =
            Box::new(move |program: &std::path::Path, args: &[String]| {
                sink.lock().unwrap().push(SkillCall {
                    program: program.display().to_string(),
                    args: args.to_vec(),
                });
                answer(args)
            });
        Arc::new(SkillsCli::with_runner(
            runner,
            PathBuf::from("/fake/bin/future"),
        ))
    }

    /// The recorded invocations, as argument vectors.
    fn skill_args(calls: &std::sync::Arc<std::sync::Mutex<Vec<SkillCall>>>) -> Vec<Vec<String>> {
        calls
            .lock()
            .unwrap()
            .iter()
            .map(|call| call.args.clone())
            .collect()
    }

    /// The recorded *mutating* invocations: everything that is not the
    /// catalogue read the panel does on open, on refresh and after an
    /// operation. What `i`/`u`/`U` must not do is reach the process runner,
    /// so assertions about "nothing ran" are made against this list.
    fn skill_op_args(calls: &std::sync::Arc<std::sync::Mutex<Vec<SkillCall>>>) -> Vec<Vec<String>> {
        skill_args(calls)
            .into_iter()
            .filter(|args| *args != crate::skills_cli::list_args())
            .collect()
    }

    /// How many recorded invocations were mutating operations.
    fn skill_op_count(calls: &[Vec<String>]) -> usize {
        calls
            .iter()
            .filter(|args| **args != crate::skills_cli::list_args())
            .count()
    }

    /// How many recorded invocations were catalogue reads.
    fn skill_list_count(calls: &[Vec<String>]) -> usize {
        calls
            .iter()
            .filter(|args| **args == crate::skills_cli::list_args())
            .count()
    }

    /// A `future skills list --json` document with one upgradable skill (alpha,
    /// 1.0.0 → 1.2.0) and one that is current (beta) — the shape the CLI emits.
    const SKILLS_CATALOGUE_JSON: &str = r#"{"skills":[
        {"id":"alpha","name":"Alpha","description":"does alpha things","latestVersion":"1.2.0","installedVersion":"1.0.0"},
        {"id":"beta","name":"Beta","description":"does beta things","latestVersion":"2.0.0","installedVersion":"2.0.0"}
    ],"count":2}"#;

    /// [`SKILLS_CATALOGUE_JSON`], parsed.
    fn skills_catalogue_fixture() -> SkillCatalogue {
        crate::skills_cli::parse_catalogue(SKILLS_CATALOGUE_JSON).expect("the fixture parses")
    }

    // ─── Skill recommendation (PRD "技能推荐") ───────────────────────────────

    /// A draft long enough to pass the minimum-length gate (well over 30 bytes).
    const RECO_DRAFT: &str = "帮我查一下这个基因在人群里的频率并找出引用来源";

    /// An app with a catalogue but a dead client, so the *gates* can be tested
    /// without a network. `make_app_at` derives a per-test budget path, so these
    /// tests cannot spend each other's daily budget.
    fn reco_app() -> App<FakeTerminal> {
        let (mut app, _rx) = make_app_at("127.0.0.1:1", &CliOptions::default());
        app.skills_catalogue = Some(skills_catalogue_fixture());
        // `App::new` derives the budget from the settings file's *directory*,
        // which every test shares (they all sit in the temp dir). Give each test
        // its own file so one test's spent budget cannot leak into another's.
        app.skill_reco_path =
            std::env::temp_dir().join(format!("tui-test-skill-reco-{}.json", random_id()));
        app
    }

    /// A *fresh session's* app: no `/skills` panel has ever been opened, so no
    /// catalogue has been fetched, and `future skills list --json` is an
    /// injected fake answering from `answer`. Recommendation used to be dead in
    /// exactly this state (see [`App::prefetch_skill_catalogue`]), which is what
    /// these tests pin.
    fn reco_app_without_catalogue<F>(
        answer: F,
    ) -> (
        App<FakeTerminal>,
        mpsc::UnboundedReceiver<UiCmd>,
        std::sync::Arc<std::sync::Mutex<Vec<SkillCall>>>,
    )
    where
        F: Fn(&[String]) -> Result<(i32, String, String), String> + Send + Sync + 'static,
    {
        let (mut app, rx) = make_app_at("127.0.0.1:1", &CliOptions::default());
        app.skill_reco_path =
            std::env::temp_dir().join(format!("tui-test-skill-reco-{}.json", random_id()));
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        app.skills_cli = Some(fake_skills_cli(&calls, answer));
        assert!(app.skills_catalogue.is_none(), "a fresh session has none");
        (app, rx, calls)
    }

    /// The catalogue the recommender picks from used to come only from the
    /// `/skills` panel, so a fresh session had an empty candidate set: every
    /// message was refused, silently, until the panel had been opened once.
    /// Now the draft itself starts the fetch — while it is still being typed.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_catalogue_is_prefetched_once_a_draft_could_be_recommended() {
        let (mut app, mut rx, calls) = reco_app_without_catalogue(|_| {
            Ok((0, SKILLS_CATALOGUE_JSON.to_string(), String::new()))
        });
        // The reported state: no catalogue, no card, whatever the user writes.
        assert!(!app.recommendation_gates_pass(RECO_DRAFT));

        // One keystroke that reaches the length window is enough.
        app.handle_cmd(UiCmd::InputChanged(RECO_DRAFT.to_string()));
        pump(&mut app, &mut rx).await;

        assert_eq!(skill_list_count(&skill_args(&calls)), 1);
        assert!(app.skills_catalogue.is_some(), "the answer fills the cache");
        assert!(
            app.recommendation_gates_pass(RECO_DRAFT),
            "the message that started the prefetch can be recommended for"
        );
        assert!(
            app.chat.last_message().is_none(),
            "the user is typing, not asking: the prefetch says nothing"
        );

        // Every later keystroke (and a whole second draft) reuses the cache
        // instead of starting another child.
        app.handle_cmd(UiCmd::InputChanged(format!("{RECO_DRAFT}，顺便找一下")));
        app.handle_cmd(UiCmd::InputChanged(format!(
            "{RECO_DRAFT}，顺便找一下更多资料"
        )));
        pump(&mut app, &mut rx).await;
        assert_eq!(skill_list_count(&skill_args(&calls)), 1);
    }

    /// The prefetch is a background call the user never asked for, so a failure
    /// is dropped in silence and *not* retried on every keystroke. The panel is
    /// where a catalogue failure is reported, and it retries.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_prefetch_is_silent_and_not_retried() {
        let (mut app, mut rx, calls) =
            reco_app_without_catalogue(|_| Err("the platform is unreachable".to_string()));

        app.handle_cmd(UiCmd::InputChanged(RECO_DRAFT.to_string()));
        pump(&mut app, &mut rx).await;

        assert!(app.skills_catalogue.is_none());
        assert!(
            app.chat.last_message().is_none(),
            "nothing was asked for, so nothing is reported: the panel reports it"
        );
        assert_eq!(skill_list_count(&skill_args(&calls)), 1);

        // Typing on must not turn a failed background call into a child per
        // keystroke.
        app.handle_cmd(UiCmd::InputChanged(
            "再换一句更长的问话看看效果如何".to_string(),
        ));
        pump(&mut app, &mut rx).await;
        assert_eq!(skill_list_count(&skill_args(&calls)), 1);
    }

    /// Only a draft that could actually produce a card is worth the fetch: the
    /// prefetch asks the same question as the gate, not a looser one.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_prefetch_waits_for_a_draft_that_could_be_recommended() {
        let (mut app, mut rx, calls) = reco_app_without_catalogue(|_| {
            Ok((0, SKILLS_CATALOGUE_JSON.to_string(), String::new()))
        });

        // Too short to ask about (9 汉字), a local slash command, and a draft
        // that already picks its own skill.
        app.handle_cmd(UiCmd::InputChanged("单细胞测序如何分析".to_string()));
        app.handle_cmd(UiCmd::InputChanged("/skills".to_string()));
        app.handle_cmd(UiCmd::InputChanged(
            "帮我查一下 /alpha 这个基因".to_string(),
        ));
        pump(&mut app, &mut rx).await;
        assert_eq!(skill_list_count(&skill_args(&calls)), 0);

        // A real message does fetch it.
        app.handle_cmd(UiCmd::InputChanged(RECO_DRAFT.to_string()));
        pump(&mut app, &mut rx).await;
        assert_eq!(skill_list_count(&skill_args(&calls)), 1);

        // The toggle is the user's opt-out: no fetch, no call.
        let (mut off, mut off_rx, off_calls) = reco_app_without_catalogue(|_| {
            Ok((0, SKILLS_CATALOGUE_JSON.to_string(), String::new()))
        });
        off.tui_settings.skill_recommend = Some(false);
        off.handle_cmd(UiCmd::InputChanged(RECO_DRAFT.to_string()));
        off.handle_submit(RECO_DRAFT);
        pump(&mut off, &mut off_rx).await;
        assert_eq!(skill_list_count(&skill_args(&off_calls)), 0);
    }

    /// A send whose draft never went through the input's change callback (`-p`,
    /// a scripted submit, a paste that arrives with the Enter) still starts the
    /// prefetch — for the *next* message. It must never hold this one: a send is
    /// not delayed for a catalogue.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_submitted_draft_prefetches_without_holding_the_send() {
        let (mut app, mut rx, calls) = reco_app_without_catalogue(|_| {
            Ok((0, SKILLS_CATALOGUE_JSON.to_string(), String::new()))
        });

        assert!(
            !app.maybe_recommend_skill(RECO_DRAFT),
            "nothing can be recommended for yet, so the message goes out"
        );
        pump(&mut app, &mut rx).await;
        assert_eq!(skill_list_count(&skill_args(&calls)), 1);
        assert!(
            app.recommendation_gates_pass(RECO_DRAFT),
            "the message after it can be recommended for"
        );
    }

    #[tokio::test]
    async fn recommendation_gates_reject_everything_but_a_real_message() {
        // The happy path: toggle on (default), a long message, a catalogue, no
        // skill picked, budget unspent.
        let app = reco_app();
        assert!(app.recommendation_gates_pass(RECO_DRAFT));

        // The toggle is the user's opt-out.
        let mut off = reco_app();
        off.tui_settings.skill_recommend = Some(false);
        assert!(!off.recommendation_gates_pass(RECO_DRAFT));

        // Slash commands are local actions, not messages.
        assert!(!app.recommendation_gates_pass("/skills"));

        // The length window, in UTF-8 bytes: 9 汉字 is 27 bytes (too short),
        // 10 is exactly 30 (allowed).
        let short = reco_app();
        assert_eq!("单细胞测序如何分析".len(), 27, "9 汉字 is 27 bytes");
        assert!(!short.recommendation_gates_pass("单细胞测序如何分析"));
        assert_eq!("单细胞测序如何分析流".len(), 30, "10 汉字 is 30 bytes");
        assert!(short.recommendation_gates_pass("单细胞测序如何分析流"));

        // Over-long drafts are sent unrecommended rather than truncated.
        let long = "字".repeat(crate::skill_reco::MAX_QUERY_CHARS + 1);
        assert!(!app.recommendation_gates_pass(&long));

        // The user already picked a skill for this message.
        assert!(!app.recommendation_gates_pass("帮我查一下 /alpha 这个基因"));

        // Nothing to recommend from: the catalogue has not been fetched.
        let mut empty = reco_app();
        empty.skills_catalogue = None;
        assert!(!empty.recommendation_gates_pass(RECO_DRAFT));

        // Every catalogue entry is installed, so the candidate set is empty.
        let mut all_installed = reco_app();
        all_installed.state.skills = vec!["alpha".to_string(), "beta".to_string()];
        assert!(!all_installed.recommendation_gates_pass(RECO_DRAFT));
    }

    #[tokio::test]
    async fn installed_skills_are_excluded_from_the_candidates() {
        let mut app = reco_app();
        let all = app.skill_reco_candidates();
        assert_eq!(
            all.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            vec!["alpha", "beta"],
            "an empty installed set offers the whole catalogue"
        );
        // `summary` carries the entry's description, which is what the agent
        // shows the model.
        assert_eq!(all[0].1, "does alpha things");

        app.state.skills = vec!["alpha".to_string()];
        let remaining = app.skill_reco_candidates();
        assert_eq!(
            remaining
                .iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>(),
            vec!["beta"],
            "an installed skill is never offered"
        );
    }

    #[tokio::test]
    async fn the_daily_budget_stops_the_question_and_the_message_still_sends() {
        let app = reco_app();
        // Three recommendations already shown today.
        for skill in ["alpha", "beta", "gamma"] {
            crate::skill_reco::record_at(&app.skill_reco_path, skill, "some-other-message");
        }
        assert!(!app.recommendation_gates_pass(RECO_DRAFT));

        // And a message already asked about is not asked about twice, even with
        // budget left.
        let app = reco_app();
        let hash = crate::skill_reco::message_hash(RECO_DRAFT);
        crate::skill_reco::record_at(&app.skill_reco_path, "alpha", &hash);
        assert!(!app.recommendation_gates_pass(RECO_DRAFT));
        assert!(
            app.recommendation_gates_pass("a completely different question about something else"),
            "only the message that was already asked about is skipped"
        );
    }

    #[test]
    fn the_prompt_line_names_the_skill_and_both_keys() {
        assert_eq!(SkillRecoState::Idle.prompt_line(0), None);
        let pending = SkillRecoState::Pending {
            draft: RECO_DRAFT.to_string(),
        }
        .prompt_line(0)
        .unwrap();
        assert!(
            pending.contains("Looking"),
            "the wait must be described: {pending}"
        );
        assert!(
            pending.starts_with('⠋'),
            "the wait must spin like the footer does: {pending}"
        );

        let line = SkillRecoState::Suggested {
            draft: RECO_DRAFT.to_string(),
            skill: "alpha".to_string(),
            summary: "does alpha things".to_string(),
        }
        .prompt_line(0)
        .unwrap();
        assert!(
            line.contains("/alpha"),
            "the command must be visible: {line}"
        );
        assert!(line.contains("does alpha things"));
        assert!(
            line.contains("[a]"),
            "the accept key must be offered: {line}"
        );
        assert!(
            line.contains("Esc"),
            "the dismiss key must be offered: {line}"
        );
    }

    /// While the agent is being asked, the input box is the TUI's disabled send
    /// button: Enter, typing, paste and Escape all leave the held draft alone.
    ///
    /// Enter matters most: a second submit used to fall through to the normal
    /// send path, so the message the answer belonged to went out while the
    /// answer was still in flight (and a card for it could land afterwards).
    #[tokio::test]
    async fn a_pending_recommendation_locks_the_input_box() {
        let mut app = reco_app();
        app.state.session_id = "s1".to_string();
        app.input.set_value(RECO_DRAFT, None);
        app.skill_reco = SkillRecoState::Pending {
            draft: RECO_DRAFT.to_string(),
        };
        assert!(app.editor_locked_by_reco());

        // Enter: the draft must not race the answer.
        app.handle_submit(RECO_DRAFT);
        assert!(
            matches!(app.skill_reco, SkillRecoState::Pending { .. }),
            "the draft stays held"
        );
        assert!(
            app.chat.last_message().is_none(),
            "nothing may be sent while the answer is in flight"
        );

        // Typing, pasting and Escape all leave the held draft as it is.
        app.handle_key("x");
        assert_eq!(app.input.get_value(), RECO_DRAFT);
        let dir = tempfile::tempdir().expect("tempdir");
        let (capture, programs) = scripted_clipboard(dir.path(), &[], "pasted text");
        app.clipboard_capture = capture;
        app.handle_key(Key::CTRL_V);
        assert_eq!(app.input.get_value(), RECO_DRAFT);
        assert!(
            clipboard_programs(&programs).is_empty(),
            "a locked box must not even read the clipboard"
        );
        app.handle_key(Key::ESCAPE);
        assert_eq!(
            app.input.get_value(),
            RECO_DRAFT,
            "Escape must not clear the draft the answer belongs to"
        );
    }

    /// The lock belongs to the wait only: once the flow releases the draft the
    /// box works normally again.
    #[tokio::test]
    async fn the_input_unlocks_once_the_recommendation_flow_ends() {
        let mut app = reco_app();
        app.skill_reco = SkillRecoState::Pending {
            draft: RECO_DRAFT.to_string(),
        };
        app.apply_skill_reco_suggestion(RECO_DRAFT.to_string(), None);
        assert!(!app.editor_locked_by_reco());
        app.handle_key("x");
        assert_eq!(app.input.get_value(), "x", "typing works again");
    }

    /// The agent declining is the common case and must send the draft, not hold
    /// it. A dead client answers nothing, which exercises the same path.
    #[tokio::test]
    async fn a_declined_recommendation_sends_the_held_draft() {
        let mut app = reco_app();
        app.state.session_id = "s1".to_string();
        app.skill_reco = SkillRecoState::Pending {
            draft: RECO_DRAFT.to_string(),
        };
        app.apply_skill_reco_suggestion(RECO_DRAFT.to_string(), None);
        assert_eq!(
            app.skill_reco,
            SkillRecoState::Idle,
            "the hold must be released so the draft can go out"
        );
    }

    /// A recommendation spends the budget when it is *shown*, so the card is
    /// recorded before the user does anything with it.
    #[tokio::test]
    async fn showing_a_recommendation_spends_the_budget_once() {
        let mut app = reco_app();
        app.skill_reco = SkillRecoState::Pending {
            draft: RECO_DRAFT.to_string(),
        };
        app.apply_skill_reco_suggestion(
            RECO_DRAFT.to_string(),
            Some(("alpha".to_string(), "does alpha things".to_string())),
        );
        assert!(matches!(
            app.skill_reco,
            SkillRecoState::Suggested { ref skill, .. } if skill == "alpha"
        ));
        let day = crate::skill_reco::load_at(&app.skill_reco_path);
        assert_eq!(day.count(), 1);
        assert!(day.already_recommended("alpha"));
        assert!(day.already_evaluated(&crate::skill_reco::message_hash(RECO_DRAFT)));

        // The same skill twice in a day is skipped rather than re-shown (and
        // never swapped for a different skill).
        let mut again = reco_app();
        crate::skill_reco::record_at(&again.skill_reco_path, "alpha", "unrelated");
        again.skill_reco = SkillRecoState::Pending {
            draft: RECO_DRAFT.to_string(),
        };
        again.apply_skill_reco_suggestion(
            RECO_DRAFT.to_string(),
            Some(("alpha".to_string(), "does alpha things".to_string())),
        );
        assert_eq!(again.skill_reco, SkillRecoState::Idle);
        assert_eq!(
            crate::skill_reco::load_at(&again.skill_reco_path).count(),
            1,
            "a skipped duplicate must not spend a second slot"
        );
    }

    /// An answer for a draft that is no longer held must not resurrect a card:
    /// the user may have sent it, or typed something else, while the call was in
    /// flight.
    #[tokio::test]
    async fn a_late_answer_for_a_stale_draft_is_ignored() {
        let mut app = reco_app();
        app.skill_reco = SkillRecoState::Pending {
            draft: "the draft that asked".to_string(),
        };
        app.apply_skill_reco_suggestion(
            "a different draft".to_string(),
            Some(("alpha".to_string(), "does alpha things".to_string())),
        );
        assert!(matches!(app.skill_reco, SkillRecoState::Pending { .. }));
        assert_eq!(crate::skill_reco::load_at(&app.skill_reco_path).count(), 0);
    }

    /// Escape on a card sends the original draft, never the skill.
    #[tokio::test]
    async fn dismissing_sends_the_draft_without_the_skill() {
        let mut app = reco_app();
        app.state.session_id = "s1".to_string();
        app.skill_reco = SkillRecoState::Suggested {
            draft: RECO_DRAFT.to_string(),
            skill: "alpha".to_string(),
            summary: "does alpha things".to_string(),
        };
        app.send_held_draft();
        assert_eq!(app.skill_reco, SkillRecoState::Idle);
        let last = app.chat.last_message().expect("the draft was sent");
        assert_eq!(last.content, RECO_DRAFT);
        assert!(
            !last.content.contains("/alpha"),
            "dismissing must not add the skill: {}",
            last.content
        );
    }

    /// Enter with the card up is the same decision as its `Esc`: send the draft
    /// without the skill. A silent no-op read as a dead send key (the desktop
    /// and mobile send buttons stay live here and send).
    #[tokio::test]
    async fn enter_on_a_shown_card_sends_the_draft_without_the_skill() {
        let mut app = reco_app();
        app.state.session_id = "s1".to_string();
        app.input.set_value(RECO_DRAFT, None);
        app.skill_reco = SkillRecoState::Suggested {
            draft: RECO_DRAFT.to_string(),
            skill: "alpha".to_string(),
            summary: "does alpha things".to_string(),
        };
        app.handle_submit(RECO_DRAFT);
        assert_eq!(app.skill_reco, SkillRecoState::Idle);
        let last = app.chat.last_message().expect("the draft was sent");
        assert_eq!(last.content, RECO_DRAFT);
        assert!(
            !last.content.contains("/alpha"),
            "sending without the skill must not append it: {}",
            last.content
        );
    }

    /// Accepting appends the slash command (after a single separating space) and
    /// sends that.
    #[tokio::test]
    async fn accepting_appends_the_slash_command_and_sends() {
        let mut app = reco_app();
        app.state.session_id = "s1".to_string();
        app.use_recommended_skill(RECO_DRAFT, "alpha");
        assert_eq!(app.skill_reco, SkillRecoState::Idle);
        let last = app
            .chat
            .last_message()
            .expect("the composed draft was sent");
        assert_eq!(last.content, format!("{RECO_DRAFT} /alpha "));
    }

    #[tokio::test]
    async fn the_slash_command_toggles_and_persists() {
        let mut app = reco_app();
        assert!(
            app.tui_settings.skill_recommend_enabled(),
            "recommendation is on by default (PRD v1.6 §3)"
        );
        app.set_skill_recommend("off");
        assert!(!app.tui_settings.skill_recommend_enabled());
        assert!(!app.recommendation_gates_pass(RECO_DRAFT));

        // Persisted: a settings round-trip keeps the opt-out.
        let json = app.tui_settings.to_json();
        assert_eq!(
            json.get("skillRecommend").and_then(Value::as_bool),
            Some(false)
        );
        assert!(!TuiSettings::from_json(&json).skill_recommend_enabled());

        app.set_skill_recommend("on");
        assert!(app.tui_settings.skill_recommend_enabled());
        // A bare form reports rather than changes.
        app.set_skill_recommend("");
        assert!(app.tui_settings.skill_recommend_enabled());
    }

    /// `draft_picks_skill` decides whether the user already chose a skill. A
    /// bare `/` (or a lone slash-word) must not be mistaken for one.
    #[test]
    fn picks_skill_detects_a_slash_token_anywhere_in_the_draft() {
        assert!(draft_picks_skill("/alpha"));
        assert!(draft_picks_skill("帮我查一下 /alpha"));
        assert!(draft_picks_skill("leading text /alpha trailing"));
        assert!(!draft_picks_skill("帮我查一下这个基因"));
        assert!(!draft_picks_skill("/"), "a bare slash is not a skill");
        assert!(!draft_picks_skill("a / b"));
    }

    /// A test app talking to a live mock agent, plus that agent's request log —
    /// the shape the skill-operation tests need (a *dead* client's failing
    /// `get_commands` would race the panel's status row assertions).
    async fn app_with_live_agent(
        overrides: std::collections::HashMap<String, String>,
    ) -> (
        App<FakeTerminal>,
        mpsc::UnboundedReceiver<UiCmd>,
        std::sync::Arc<std::sync::Mutex<Vec<RpcCommand>>>,
    ) {
        let mock = AppMockAgent {
            overrides,
            ..Default::default()
        };
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        (app, rx, requests)
    }

    /// The open skills panel's status row (what `set_error` wrote), or `None`
    /// when no panel is open.
    fn skills_row(app: &mut App<FakeTerminal>) -> Option<String> {
        app.top_skills_view()
            .and_then(|view| view.error())
            .map(str::to_string)
    }

    /// Pump until the fake runner's recorded calls satisfy `matched` (bounded).
    /// Uses the same flag + assert shape as the other bounded waiters so the
    /// failing path is the assertion, not a panic line nobody covers.
    async fn pump_until_skill_call<F>(
        app: &mut App<FakeTerminal>,
        op_rx: &mut mpsc::UnboundedReceiver<UiCmd>,
        calls: &std::sync::Arc<std::sync::Mutex<Vec<SkillCall>>>,
        matched: F,
    ) where
        F: Fn(&[Vec<String>]) -> bool,
    {
        let mut found = false;
        for _ in 0..PUMP_BUDGET_ITERS {
            while let Ok(cmd) = op_rx.try_recv() {
                app.handle_cmd(cmd);
            }
            if matched(&skill_args(calls)) {
                found = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(PUMP_INTERVAL_MS)).await;
        }
        assert!(found, "the fake runner never saw the expected calls");
    }

    /// Pump until the mock agent has seen `count` commands of one type, driving
    /// the app over their answers (the follow-up RPC of a reply is triggered by
    /// that reply, so the channel has to be drained inside the wait).
    async fn pump_until_agent_commands(
        app: &mut App<FakeTerminal>,
        op_rx: &mut mpsc::UnboundedReceiver<UiCmd>,
        requests: &std::sync::Arc<std::sync::Mutex<Vec<RpcCommand>>>,
        r#type: &str,
        count: usize,
    ) {
        let mut found = false;
        for _ in 0..PUMP_BUDGET_ITERS {
            while let Ok(cmd) = op_rx.try_recv() {
                app.handle_cmd(cmd);
            }
            if requests
                .lock()
                .unwrap()
                .iter()
                .filter(|cmd| cmd.r#type == r#type)
                .count()
                >= count
            {
                found = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(PUMP_INTERVAL_MS)).await;
        }
        assert!(found, "the agent never saw the expected commands");
    }

    /// How many commands of one type the mock agent has seen.
    fn agent_command_count(
        requests: &std::sync::Arc<std::sync::Mutex<Vec<RpcCommand>>>,
        r#type: &str,
    ) -> usize {
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|cmd| cmd.r#type == r#type)
            .count()
    }

    #[tokio::test]
    async fn model_menu_replaces_the_select_list_and_selects() {
        let (mut app, mut rx) = make_app(100, 30);
        app.state.model = "openai/gpt-4o".into();
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Selector,
        });
        assert!(
            top_overlay_is::<MenuOverlay>(&app),
            "the /model picker is a MenuState overlay now"
        );
        let rows = app.overlay_stack[0].component.render(80);
        let text = rows
            .iter()
            .map(|row| strip_ansi_codes(row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Select Model"), "{text}");
        assert!(
            text.contains("current"),
            "the active model is badged: {text}"
        );

        // Search narrows, then enter picks.
        for key in ["c", "l", "a", "u", "d", "e"] {
            press_on_overlay(&mut app, &mut rx, key);
        }
        let menu = app.overlay_stack[0]
            .component
            .as_any()
            .downcast_ref::<MenuOverlay>()
            .unwrap();
        assert_eq!(menu.state().filter(), "claude");
        assert_eq!(menu.state().visible_len(), 1);
        let _ = rx;
    }

    #[tokio::test]
    async fn model_menu_confirmation_emits_model_selected() {
        let (mut app, mut rx) = make_app(100, 30);
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Selector,
        });
        let idx = app.get_top_overlay_index().unwrap();
        app.overlay_stack[idx].component.handle_input("enter");
        let mut selected: Option<String> = None;
        while let Ok(cmd) = rx.try_recv() {
            if let UiCmd::ModelSelected(item) = &cmd {
                selected = Some(item.value.clone());
            }
            app.handle_cmd(cmd);
        }
        assert!(selected.is_some(), "enter must confirm the highlighted row");
        assert!(app.overlay_stack.is_empty(), "confirming closes the menu");
    }

    #[tokio::test]
    async fn model_menu_escape_and_page_keys_route_through_the_menu() {
        let (mut app, mut rx) = make_app(100, 30);
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Selector,
        });
        // PageDown/PageUp/Home/End are menu keys, not app scroll keys.
        for key in ["pageDown", "pageUp", "home", "end", "down", "up"] {
            press_on_overlay(&mut app, &mut rx, key);
            assert!(!app.overlay_stack.is_empty(), "{key} closed the menu");
        }
        // A search query, then escape clears it (first escape), then closes.
        press_on_overlay(&mut app, &mut rx, "g");
        assert_eq!(
            app.overlay_stack[0]
                .component
                .as_any()
                .downcast_ref::<MenuOverlay>()
                .unwrap()
                .state()
                .filter(),
            "g"
        );
        press_on_overlay(&mut app, &mut rx, "escape");
        assert_eq!(app.overlay_stack.len(), 1, "first escape clears the query");
        press_on_overlay(&mut app, &mut rx, "escape");
        assert!(app.overlay_stack.is_empty(), "second escape closes");
    }

    #[tokio::test]
    async fn escape_clears_a_menu_search_then_closes_the_popup() {
        let (mut app, mut rx) = make_app(100, 30);
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Selector,
        });
        // Type a query through the real input path.
        app.handle_key("c");
        app.handle_key("l");
        let menu_filter = |app: &App<FakeTerminal>| {
            app.overlay_stack[0]
                .component
                .as_any()
                .downcast_ref::<MenuOverlay>()
                .unwrap()
                .state()
                .filter()
                .to_string()
        };
        assert_eq!(menu_filter(&app), "cl");
        app.handle_key("escape");
        assert!(app.overlay_stack.len() == 1, "the first escape clears");
        assert_eq!(menu_filter(&app), "");
        app.handle_key("escape");
        assert!(app.overlay_stack.is_empty(), "the second escape closes");
        // Escape routing is local: nothing is emitted for the app to apply.
        let pending: Vec<UiCmd> = rx.try_recv().into_iter().collect();
        assert!(pending.is_empty(), "escape emits no command");
    }

    /// `/skills` + `/` + a query: the first `escape` clears the filter and
    /// keeps the panel, the second closes it. Same sequence as the tmux repro
    /// `.future/tui-parity/repro-skills-escape.sh`; before this route existed
    /// the app's close-the-overlay fallback ate the escape and the panel
    /// vanished while `SkillsView::escape` was never called (its own unit test
    /// passed the whole time).
    #[tokio::test]
    async fn escape_clears_the_skills_filter_then_closes_the_panel() {
        let (mut app, _rx) = make_app(100, 30);
        app.state.skills = vec!["alpha".into(), "zeta".into()];
        app.handle_submit("/skills");
        assert!(top_overlay_is::<SkillsOverlay>(&app));

        let filter = |app: &mut App<FakeTerminal>| {
            app.overlay_stack[0]
                .component
                .as_any_mut()
                .downcast_mut::<SkillsOverlay>()
                .unwrap()
                .view_mut()
                .filter()
                .to_string()
        };

        // Drive the real key route: `/` opens the search row, then query chars.
        app.handle_key("/");
        for ch in ["z", "e", "t"] {
            app.handle_key(ch);
        }
        assert_eq!(filter(&mut app), "zet");

        app.handle_key("escape");
        assert!(
            top_overlay_is::<SkillsOverlay>(&app),
            "the first escape clears the filter, it does not close the panel"
        );
        assert_eq!(filter(&mut app), "");

        app.handle_key("escape");
        assert!(app.overlay_stack.is_empty(), "the second escape closes");
    }

    /// `/skills` + `escape` with no query: nothing to clear, so one escape
    /// closes the panel (the same rule the menu follows).
    #[tokio::test]
    async fn escape_closes_the_skills_panel_when_no_filter_is_active() {
        let (mut app, _rx) = make_app(100, 30);
        app.state.skills = vec!["alpha".into()];
        app.handle_submit("/skills");
        assert!(top_overlay_is::<SkillsOverlay>(&app));
        app.handle_key("escape");
        assert!(app.overlay_stack.is_empty());
    }

    /// `/transcript` + `/` + a query: the first `escape` closes the search
    /// editor (restoring the pre-edit query), the second closes the pager.
    /// The pager's own `escape` branch was unreachable from the real UI before
    /// the `wants_escape` gate, so the first escape closed the whole panel.
    #[tokio::test]
    async fn escape_closes_the_pager_search_editor_then_the_pager() {
        let (mut app, _rx) = make_app(100, 30);
        app.show_pager_text(vec!["alpha".into(), "beta".into()], "nothing");
        assert!(top_overlay_is::<PagerOverlay>(&app));

        let search_state = |app: &App<FakeTerminal>| {
            let pager = app.overlay_stack[0]
                .component
                .as_any()
                .downcast_ref::<PagerOverlay>()
                .unwrap()
                .pager();
            (pager.is_search_editing(), pager.search_query().to_string())
        };

        // `/` opens the editor, the query chars go into it.
        app.handle_key("/");
        app.handle_key("a");
        assert_eq!(search_state(&app), (true, "a".to_string()));

        app.handle_key("escape");
        assert!(
            top_overlay_is::<PagerOverlay>(&app),
            "the first escape closes the editor, it does not close the pager"
        );
        assert_eq!(search_state(&app), (false, String::new()));

        app.handle_key("escape");
        assert!(app.overlay_stack.is_empty(), "the second escape closes");
    }

    /// `/providers` + a typed query: the first `escape` clears the search and
    /// keeps the list, the second closes it.
    #[tokio::test]
    async fn escape_clears_the_provider_search_then_closes_the_list() {
        let (mut app, _rx) = make_app(100, 30);
        app.handle_cmd(UiCmd::ProvidersLoaded(Ok(vec![
            provider_info("acme"),
            provider_info("zeta"),
        ])));
        assert!(top_overlay_is::<ProviderListOverlay>(&app));

        let search = |app: &App<FakeTerminal>| {
            let state = app.overlay_stack[0]
                .component
                .as_any()
                .downcast_ref::<ProviderListOverlay>()
                .unwrap()
                .state();
            (state.is_searching(), state.filter().to_string())
        };

        // `z` is not one of the list's single-letter actions, so it starts a
        // search (the same path `/` takes).
        app.handle_key("z");
        assert_eq!(search(&app), (true, "z".to_string()));

        app.handle_key("escape");
        assert!(
            top_overlay_is::<ProviderListOverlay>(&app),
            "the first escape clears the search, it does not close the list"
        );
        assert_eq!(search(&app), (false, String::new()));

        app.handle_key("escape");
        assert!(app.overlay_stack.is_empty(), "the second escape closes");
    }

    #[tokio::test]
    async fn sessions_menu_selects_a_session() {
        let (mut app, mut rx) = make_app(100, 30);
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(sample_sessions()),
            purpose: SessionsPurpose::Browse,
        });
        // Sessions are a `MenuState` picker now, so confirmation arrives as
        // `MenuSelected` (see the dedicated sessions menu tests).
        assert!(top_overlay_is::<MenuOverlay>(&app));
        let idx = app.get_top_overlay_index().unwrap();
        app.overlay_stack[idx].component.handle_input("enter");
        let mut saw_select = false;
        while let Ok(cmd) = rx.try_recv() {
            saw_select |= matches!(cmd, UiCmd::MenuSelected { .. });
            app.handle_cmd(cmd);
        }
        assert!(saw_select);
    }

    #[tokio::test]
    async fn scope_menu_saves_the_enabled_models_and_persists_them() {
        let dir = std::env::temp_dir().join(format!("tui-scope-{}", random_id()));
        let path = dir.join("settings.json");
        let (mut app, mut rx) = make_app(100, 30);
        app.tui_settings_path = path.clone();
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Scoped,
        });
        assert!(top_overlay_is::<MenuOverlay>(&app));
        let state = app.overlay_stack[0]
            .component
            .as_any()
            .downcast_ref::<MenuOverlay>()
            .unwrap()
            .state();
        assert!(
            state.options().multi_select,
            "the scope editor is multi-select"
        );
        assert_eq!(state.selected_values().len(), 2, "all models enabled");
        assert_eq!(state.tab_index(), 0);

        // Toggle the highlighted row off, then confirm.
        press_on_overlay(&mut app, &mut rx, "space");
        press_on_overlay(&mut app, &mut rx, "enter");
        assert!(app.overlay_stack.is_empty());
        assert_eq!(app.enabled_model_ids.as_ref().map(Vec::len), Some(1));
        let written = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(written.contains("enabledModelIds"), "{written}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn theme_command_accepts_an_explicit_id() {
        let dir = std::env::temp_dir().join(format!("tui-theme-arg-{}", random_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let (mut app, _rx) = make_app(100, 30);
        app.tui_settings_path = path.clone();
        app.handle_submit("/theme One_Dark");
        assert!(
            app.overlay_stack.is_empty(),
            "the id form does not open a menu"
        );
        assert_eq!(app.tui_settings.theme_id.as_deref(), Some("one-dark"));
        let message = last_system(&app);
        assert!(message.contains("one-dark"), "{message}");
        // An unknown id resolves to the default palette.
        app.handle_submit("/theme nope");
        assert_eq!(app.tui_settings.theme_id.as_deref(), Some("dark"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn theme_menu_applies_and_persists_the_canonical_id() {
        let dir = std::env::temp_dir().join(format!("tui-theme-{}", random_id()));
        let path = dir.join("settings.json");
        let (mut app, mut rx) = make_app(100, 30);
        app.tui_settings_path = path.clone();
        app.handle_submit("/theme");
        assert!(top_overlay_is::<MenuOverlay>(&app));
        let rows = app.overlay_stack[0].component.render(60);
        let text = rows
            .iter()
            .map(|row| strip_ansi_codes(row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Theme"), "{text}");
        assert!(text.contains("Dark"), "{text}");
        assert!(text.contains("current"), "the active theme is badged");

        // Walk to the second entry (Light) and confirm.
        press_on_overlay(&mut app, &mut rx, "down");
        press_on_overlay(&mut app, &mut rx, "enter");
        assert!(app.overlay_stack.is_empty());
        assert_eq!(app.tui_settings.theme_id.as_deref(), Some("light"));
        assert_eq!(app.theme, crate::themes::theme_by_id("light").unwrap());
        assert_eq!(app.chat.theme(), app.theme, "the chat repainted");
        let written = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(written.contains("\"themeId\": \"light\""), "{written}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn theme_menu_marks_the_current_theme_and_applies_it_to_open_overlays() {
        let (mut app, rx) = make_app(100, 30);
        // Open a menu first, then apply a theme: the open overlay follows.
        app.handle_cmd(UiCmd::ModelsLoaded {
            result: Ok(sample_models()),
            purpose: ModelsPurpose::Selector,
        });
        assert!(app.overlay_stack[0].component.as_any().is::<MenuOverlay>());
        let canonical = app.select_theme("one-dark");
        assert_eq!(canonical, "one-dark");
        let menu_theme = app.overlay_stack[0]
            .component
            .as_any()
            .downcast_ref::<MenuOverlay>()
            .unwrap()
            .state()
            .theme();
        assert_eq!(menu_theme, app.theme);
        // An unknown id falls back to the catalog default and reports it.
        assert_eq!(app.select_theme("nope"), "dark");
        let _ = rx;
    }

    /// The status bar and the prompt bar are fields, not overlays: `/theme`
    /// has to fan the palette out to them explicitly, and `/theme dark` has to
    /// leave the ported chrome table byte-identical.
    #[tokio::test]
    async fn theme_switch_repaints_the_footer_and_the_input() {
        let (mut app, _rx) = make_app(100, 30);
        let dark = crate::theme::DARK_THEME;
        let light = crate::themes::theme_by_id("light").unwrap();
        // A fresh app starts on the catalog default.
        assert_eq!(app.theme, dark);
        assert_eq!(app.footer.theme(), dark);
        assert_eq!(app.input.theme(), dark);

        app.handle_submit("/theme light");
        assert_eq!(app.theme, light);
        assert_eq!(app.chat.theme(), light, "chat repainted");
        assert_eq!(app.footer.theme(), light, "the status bar repainted");
        assert_eq!(app.input.theme(), light, "the prompt bar repainted");

        // Back to the default palette: the derived chrome table must collapse
        // to `LEGACY` again, which is the byte-stability guarantee for the
        // ported renderers.
        app.handle_submit("/theme dark");
        assert_eq!(app.footer.theme(), dark);
        assert_eq!(app.input.theme(), dark);
        assert_eq!(app.chat.theme(), dark);
        assert_eq!(
            crate::theme::Chrome::from_theme(&app.footer.theme()),
            crate::theme::Chrome::LEGACY
        );
    }

    /// Overlays are constructed at runtime, so the `/theme` handler cannot be
    /// the only place that hands out the palette: `show_overlay` must theme a
    /// widget that is created *after* the switch.
    #[tokio::test]
    async fn overlays_built_after_a_theme_switch_inherit_the_palette() {
        let (mut app, _rx) = make_app(100, 30);
        app.handle_submit("/theme light");
        let light = app.theme;
        assert_ne!(light, crate::theme::DARK_THEME);

        // A pager built after the switch (the `/history`-style panel).
        app.show_pager_text(vec!["a line".into()], "nothing");
        let pager = app
            .overlay_stack
            .last()
            .unwrap()
            .component
            .as_any()
            .downcast_ref::<PagerOverlay>()
            .expect("pager overlay");
        assert_eq!(pager.theme(), light, "the new pager took the palette");

        // ...and so is a menu pushed on top of it.
        app.handle_submit("/theme");
        let menu = app
            .overlay_stack
            .last()
            .unwrap()
            .component
            .as_any()
            .downcast_ref::<MenuOverlay>()
            .expect("theme menu");
        assert_eq!(menu.state().theme(), light, "the new menu took the palette");

        // ...and the `/keymap` editor, whose palette is fanned out by
        // `apply_theme_to_new_overlay` rather than by the shared one (it is one
        // of the panels that live in this file).
        app.handle_submit("/keymap");
        let keymap = app
            .overlay_stack
            .last()
            .unwrap()
            .component
            .as_any()
            .downcast_ref::<KeymapOverlay>()
            .expect("keymap panel");
        assert_eq!(keymap.theme(), light, "the new /keymap took the palette");
    }

    /// `/skills` opens the browser seeded with the names `get_state` already
    /// reported (so it is useful before — and without — the catalogue RPC),
    /// `enter` inserts the highlighted name, and an empty list explains itself.
    #[tokio::test]
    async fn skills_menu_lists_the_session_skills() {
        let (mut app, mut rx) = make_app(100, 30);
        app.state.skills = vec!["alpha".into(), "zeta".into()];
        app.handle_submit("/skills");
        assert!(top_overlay_is::<SkillsOverlay>(&app));
        let text = top_overlay_text(&mut app, 80);
        assert!(text.contains("alpha") && text.contains("zeta"), "{text}");
        // Enter inserts the highlighted skill name into the input and closes
        // the panel.
        press_on_overlay(&mut app, &mut rx, "enter");
        assert_eq!(app.input.get_value(), "alpha");
        assert!(app.overlay_stack.is_empty());

        // With no skills the panel says so instead of opening a blank screen.
        app.input.set_value("", None);
        app.state.skills.clear();
        app.handle_submit("/skills");
        let text = top_overlay_text(&mut app, 80);
        assert!(
            text.contains("~/.future/agent/skills"),
            "the empty state points at the skill directory: {text}"
        );
        // The dead client's failed `get_commands` is reported rather than
        // swallowed.
        let failed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                while let Ok(cmd) = rx.try_recv() {
                    app.handle_cmd(cmd);
                }
                if last_system(&app).contains("Failed to load the skill catalogue") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await;
        assert!(failed.is_ok(), "the catalogue failure was never reported");
    }

    #[tokio::test]
    async fn tools_menu_applies_the_selection_through_set_tools() {
        let (addr, seen) = spawn_app_mock().await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.handle_submit("/tools");
        assert!(top_overlay_is::<MenuOverlay>(&app));
        let state = app.overlay_stack[0]
            .component
            .as_any()
            .downcast_ref::<MenuOverlay>()
            .unwrap()
            .state();
        assert!(state.options().multi_select);
        assert_eq!(state.selected_values().len(), BUILTIN_TOOLS.len());
        // Toggle "read" off and apply.
        press_on_overlay(&mut app, &mut rx, "space");
        press_on_overlay(&mut app, &mut rx, "enter");
        assert!(app.overlay_stack.is_empty());
        pump_until_msg(&mut app, &mut rx, "Tools enabled").await;
        assert_eq!(app.enabled_tools.as_ref().map(Vec::len), Some(3));
        let sent = seen.lock().unwrap().clone();
        assert!(
            sent.iter().any(|(t, _)| t == "set_tools"),
            "set_tools must reach the agent: {sent:?}"
        );
        let message = last_system(&app);
        assert!(
            message.contains("read") || message.contains("Tools enabled"),
            "{message}"
        );

        // A fully empty selection is not expressible in the multi-select menu
        // (`enter` falls back to the highlighted row), so `/tools none` is the
        // documented way to disable everything.
        app.enabled_tools = None;
        app.handle_submit("/tools none");
        pump_until_msg(&mut app, &mut rx, "Tools enabled: no tools").await;
        assert_eq!(app.enabled_tools.as_deref(), Some(&[][..]));
        let sent = seen.lock().unwrap().clone();
        assert!(
            sent.iter().any(|(t, _)| t == "disable_tools"),
            "clearing the set uses disable_tools: {sent:?}"
        );

        // `/tools all` re-enables the built-in set through `set_tools`.
        app.handle_submit("/tools all");
        pump_until_msg(&mut app, &mut rx, "Tools enabled: edit, read, shell, write").await;
        assert_eq!(app.enabled_tools.as_ref().map(Vec::len), Some(4));
    }

    #[tokio::test]
    async fn usage_command_opens_the_usage_panel() {
        let (mut app, mut rx) = make_app(100, 30);
        app.handle_submit("/usage");
        // get_state goes to the real client (no agent) → error path.
        pump(&mut app, &mut rx).await;
        let messages = system_messages(&app);
        assert!(
            messages.iter().any(|m| m.contains("Failed to load usage")),
            "{messages:?}"
        );

        // With a live mock the panel opens.
        let (addr, _seen) = spawn_app_mock().await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.handle_submit("/usage");
        pump_until_overlay(&mut app, &mut rx).await;
        assert!(top_overlay_is::<crate::components::usage_view::UsageOverlay>(&app));
        let rows = app.overlay_stack[0].component.render(76);
        assert!(!rows.is_empty());
        // Escape closes it.
        app.handle_key("escape");
        assert!(app.overlay_stack.is_empty());
    }

    fn sample_session(id: &str, name: &str, cwd: &str, parent: Option<&str>) -> SessionSummary {
        SessionSummary {
            id: id.into(),
            cwd: cwd.into(),
            updated_at_ms: 1_700_000_000_000,
            model: "future/deepseek-flash".into(),
            session_name: Some(name.into()),
            parent_session_id: parent.map(|p| p.into()),
            is_streaming: None,
            first_message: None,
        }
    }

    /// Objective (1): `/sessions` must be a `MenuState` picker, not the legacy
    /// `SelectList`, so it gets incremental search and the scroll window.
    #[tokio::test]
    async fn sessions_command_opens_the_menu_framework() {
        let (mut app, rx) = make_app(100, 30);
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(vec![
                sample_session("s1", "alpha work", "/tmp/a", None),
                sample_session("s2", "beta work", "/tmp/b", None),
            ]),
            purpose: SessionsPurpose::Browse,
        });
        assert!(
            top_overlay_is::<MenuOverlay>(&app),
            "/sessions must use the popup-menu framework"
        );
        let menu = app.overlay_stack[0]
            .component
            .as_any()
            .downcast_ref::<MenuOverlay>()
            .unwrap();
        assert_eq!(menu.state().visible_len(), 2);
        // Incremental search filters sessions by name.
        app.overlay_stack[0].component.handle_input("b");
        let menu = app.overlay_stack[0]
            .component
            .as_any()
            .downcast_ref::<MenuOverlay>()
            .unwrap();
        assert_eq!(menu.state().visible_len(), 1);
        assert_eq!(menu.state().highlighted().unwrap().value, "s2");
        // Every rendered row is exactly the overlay width.
        let rows = app.overlay_stack[0].component.render(76);
        assert!(rows.iter().all(|r| visible_width(r) <= 76));
        let _ = rx;
    }

    /// The tree renders depth via `MenuItem::indent` (not ASCII art) and keeps
    /// parents before their children.
    #[tokio::test]
    async fn tree_command_indents_the_hierarchy() {
        let (mut app, rx) = make_app(100, 30);
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(vec![
                sample_session("root", "parent", "/tmp/a", None),
                sample_session("child", "fork child", "/tmp/a", Some("root")),
            ]),
            purpose: SessionsPurpose::Tree,
        });
        assert!(top_overlay_is::<MenuOverlay>(&app));
        let menu = app.overlay_stack[0]
            .component
            .as_any()
            .downcast_ref::<MenuOverlay>()
            .unwrap();
        assert_eq!(menu.state().visible_len(), 2);
        let options = menu.state().options();
        let items: Vec<&MenuItem> = options.sections[0].items.iter().collect();
        assert_eq!(items[0].value, "root");
        assert_eq!(items[0].indent, 0);
        assert_eq!(items[1].value, "child");
        assert_eq!(items[1].indent, 1, "the forked session is nested one level");
        let _ = rx;
    }

    /// Confirming the *current* session is a no-op; another id switches.
    #[tokio::test]
    async fn session_menu_confirmation_switches_or_ignores_the_current() {
        let (mut app, mut rx) = make_app(100, 30);
        app.state.session_id = "s1".into();
        app.handle_cmd(UiCmd::MenuSelected {
            purpose: MenuPurpose::Sessions,
            values: vec!["s1".into()],
        });
        assert!(!app.state.streaming);
        // Same session: nothing happens, no switch command is emitted.
        pump(&mut app, &mut rx).await;
        let pending: Vec<UiCmd> = rx.try_recv().into_iter().collect();
        let switched = pending
            .iter()
            .any(|cmd| matches!(cmd, UiCmd::SessionSwitched { .. }));
        assert!(
            !switched,
            "re-selecting the current session must not switch"
        );
        // A different id performs the switch and reports the label.
        app.session_labels.insert("s2".into(), "beta work".into());
        app.handle_cmd(UiCmd::MenuSelected {
            purpose: MenuPurpose::Sessions,
            values: vec!["s2".into()],
        });
        let _ = rx;
    }

    /// An empty selection (menu closed without a value) is a no-op: no
    /// overlay is popped, no session is switched and no message is added.
    #[tokio::test]
    async fn session_menu_confirmation_without_a_value_is_a_no_op() {
        let (mut app, _rx) = make_app(100, 30);
        app.state.session_id = "s1".into();
        app.chat.clear_messages();
        app.add_system_message("sentinel".into());
        let overlays_before = app.overlay_stack.len();
        app.handle_cmd(UiCmd::MenuSelected {
            purpose: MenuPurpose::SessionTree,
            values: vec![],
        });
        assert_eq!(app.overlay_stack.len(), overlays_before, "no overlay pops");
        assert_eq!(app.state.session_id, "s1", "no session switch");
        assert_eq!(
            last_system(&app),
            "sentinel",
            "an empty selection must not print anything"
        );
    }

    /// `/sessions` on an empty list still opens (the menu shows its empty state).
    #[tokio::test]
    async fn sessions_menu_with_no_sessions_renders_the_empty_state() {
        let (mut app, _rx) = make_app(100, 30);
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(vec![]),
            purpose: SessionsPurpose::Browse,
        });
        assert!(top_overlay_is::<MenuOverlay>(&app));
        let rows = app.overlay_stack[0].component.render(76);
        let text: String = rows
            .iter()
            .map(|r| strip_ansi_codes(r))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("No matches"), "{text}");
    }

    #[test]
    fn session_display_name_falls_back_to_first_message_then_id() {
        let mut s = sample_session("sid", "named", "/tmp", None);
        assert_eq!(session_display_name(&s), "named");
        s.session_name = None;
        s.first_message = Some("hello there".into());
        assert_eq!(session_display_name(&s), "hello there");
        s.first_message = None;
        assert_eq!(session_display_name(&s), "sid");
    }

    #[test]
    fn shorten_cwd_is_home_relative_and_width_capped() {
        assert_eq!(shorten_cwd("/home/u/work", "/home/u"), "~/work");
        // A path outside home — or a host with no `HOME` at all — is left alone.
        assert_eq!(shorten_cwd("/opt/x", "/home/u"), "/opt/x");
        assert_eq!(shorten_cwd("/opt/x", ""), "/opt/x");
        // A long path is tail-truncated with a leading ellipsis.
        let long = format!("/very/{}", "x".repeat(80));
        let short = shorten_cwd(&long, "/home/u");
        assert_eq!(short.chars().count(), 40);
        assert!(short.starts_with('…'), "{short}");
    }

    /// An approval request must notify, because the run parks until answered.
    #[tokio::test]
    async fn approval_request_notifies_the_user() {
        let (mut app, _rx) = make_app(100, 30);
        app.show_approval_overlay(ApprovalEvent {
            request_id: "req-1".into(),
            tool_id: "t1".into(),
            tool_name: "shell".into(),
            kind: "shell".into(),
            risk_level: "high".into(),
            title: "Run rm -rf".into(),
            summary: "destructive command".into(),
            requested_action: Some(Value::String("rm -rf /".into())),
        });
        let writes = terminal_writes(&app);
        assert!(
            writes.contains("\x1b]9;"),
            "approval must emit an OSC 9 notification: {writes:?}"
        );
        assert!(
            writes.contains("req-1"),
            "the notification names the request"
        );
        assert!(app.pending_approval.is_some());
        assert!(app.input.get_value().starts_with("/approve req-1"));
    }

    /// Notifications respect the settings gate: disabling them silences the
    /// approval notice as well.
    #[tokio::test]
    async fn approval_notification_respects_the_settings_gate() {
        let (mut app, _rx) = make_app(100, 30);
        app.tui_settings.notify = Some(crate::notifications::NotifyConfig {
            enabled: false,
            bell: true,
            osc9: true,
            title: true,
        });
        app.show_approval_overlay(ApprovalEvent {
            request_id: "req-2".into(),
            tool_id: "t1".into(),
            tool_name: "shell".into(),
            kind: "shell".into(),
            risk_level: "high".into(),
            title: "Run rm -rf".into(),
            summary: "destructive command".into(),
            requested_action: None,
        });
        assert!(
            !terminal_writes(&app).contains("\x1b]9;"),
            "a disabled config must stay silent"
        );
    }

    #[tokio::test]
    async fn transcript_command_opens_the_pager_and_copies_a_line() {
        let (mut app, mut rx) = make_app(100, 30);
        // Nothing to show yet.
        app.handle_submit("/transcript");
        assert!(app.overlay_stack.is_empty());
        let message = last_system(&app);
        assert!(message.contains("Nothing to show"), "{message}");

        app.chat.add_message(ChatMessage::new(
            "m1".into(),
            ChatRole::Assistant,
            "a long answer line",
        ));
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        app.clipboard = recording_clipboard(&log);
        app.chat.clear_messages();
        app.chat.add_message(ChatMessage::new(
            "m2".into(),
            ChatRole::Assistant,
            "a long answer line",
        ));
        app.handle_submit("/transcript");
        assert!(top_overlay_is::<PagerOverlay>(&app));
        let rows = app.overlay_stack[0].component.render(100);
        let text = rows
            .iter()
            .map(|row| strip_ansi_codes(row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("a long answer line"), "{text}");
        assert!(
            text.contains("search"),
            "the status row shows hints: {text}"
        );

        // `y` copies the current line through the clipboard backend.
        press_on_overlay(&mut app, &mut rx, "y");
        let copied = clipboard_log(&log);
        assert_eq!(copied.len(), 1, "{copied:?}");
        assert!(copied[0].contains("a long answer line"), "{copied:?}");
        assert!(app.overlay_stack.is_empty(), "the copy closes the pager");
        let message = last_system(&app);
        assert!(message.contains("Copied"), "{message}");

        // `q` closes without copying.
        app.handle_submit("/transcript");
        log.lock().unwrap().clear();
        press_on_overlay(&mut app, &mut rx, "q");
        assert!(app.overlay_stack.is_empty());
        assert!(clipboard_log(&log).is_empty());

        // Search inside the pager: `/` + query + enter marks the match.
        app.handle_submit("/transcript");
        for key in ["/", "l", "o", "n", "g", "enter"] {
            press_on_overlay(&mut app, &mut rx, key);
        }
        let pager = app.overlay_stack[0]
            .component
            .as_any()
            .downcast_ref::<PagerOverlay>()
            .unwrap()
            .pager();
        assert_eq!(pager.match_count(), 1);
        assert_eq!(pager.search_query(), "long");
    }

    #[tokio::test]
    async fn copy_command_and_ctrl_x_copy_the_last_assistant_message() {
        let (mut app, rx) = make_app(100, 30);
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        app.clipboard = recording_clipboard(&log);

        // Nothing to copy yet.
        app.handle_submit("/copy");
        assert!(clipboard_log(&log).is_empty());
        let message = last_system(&app);
        assert!(message.contains("Nothing to copy"), "{message}");

        app.chat.add_message(ChatMessage::new(
            "a1".into(),
            ChatRole::Assistant,
            "first answer  ",
        ));
        app.chat
            .add_message(ChatMessage::new("u1".into(), ChatRole::User, "a follow-up"));
        app.handle_submit("/copy");
        let copied = clipboard_log(&log);
        assert_eq!(copied.len(), 1);
        let first = copied[0].clone();
        assert!(
            first.ends_with("first answer"),
            "trailing whitespace is trimmed: {first:?}"
        );
        let message = last_system(&app);
        assert!(message.contains("Copied"), "{message}");

        // ctrl+x is the shortcut for the same action.
        log.lock().unwrap().clear();
        app.handle_agent_event(&make_event("text_chunk", r#"{"text":"second"}"#));
        app.chat.mark_last_message_complete();
        app.handle_key_action(KeyAction::CopyLastMessage);
        let copied = clipboard_log(&log);
        assert_eq!(copied.len(), 1);
        assert!(copied[0].contains("second"), "{copied:?}");
        let _ = rx;
    }

    #[tokio::test]
    async fn copy_without_a_native_backend_requests_osc52() {
        let (mut app, rx) = make_app(100, 30);
        // Empty candidate list + no tmux ⇒ the OSC 52 request path.
        app.clipboard = crate::clipboard::Clipboard::with_parts(
            Box::new(|_, _, _| Err("no backend".into())),
            Vec::new(),
            false,
        );
        app.chat.add_message(ChatMessage::new(
            "a1".into(),
            ChatRole::Assistant,
            "payload",
        ));
        app.handle_submit("/copy");
        let writes = terminal_writes(&app);
        assert!(
            writes.contains("\x1b]52;"),
            "the OSC 52 request must be written: {writes:?}"
        );
        let message = last_system(&app);
        assert!(message.contains("OSC 52"), "{message}");

        // An empty message never touches the clipboard.
        app.chat.clear_messages();
        app.handle_submit("/copy");
        let message = last_system(&app);
        assert!(message.contains("Nothing to copy"), "{message}");
        let _ = rx;
    }

    #[tokio::test]
    async fn copy_is_not_reported_as_confirmed_when_the_backend_fails() {
        let (mut app, rx) = make_app(100, 30);
        app.clipboard = crate::clipboard::Clipboard::with_parts(
            Box::new(|_, _, _| Err("boom".into())),
            Vec::new(),
            false,
        );
        // Too large for OSC 52 as well → Failed.
        app.chat.add_message(ChatMessage::new(
            "a1".into(),
            ChatRole::Assistant,
            &"x".repeat(crate::clipboard::OSC52_MAX_RAW_BYTES + 1),
        ));
        app.handle_submit("/copy");
        let message = last_system(&app);
        assert!(message.contains("Copy failed"), "{message}");
        let _ = rx;
    }

    #[tokio::test]
    async fn editor_command_refills_the_draft_from_the_temp_file() {
        let _guard = crate::test_env::lock();
        let (mut app, rx) = make_app(100, 30);
        let dir = std::env::temp_dir().join(format!("tui-editor-{}", random_id()));
        std::fs::create_dir_all(&dir).unwrap();
        app.tui_settings_path = dir.join("settings.json");
        app.input.set_value("draft text", None);

        let previous = std::env::var("EDITOR").ok();
        std::env::set_var("EDITOR", "true");
        std::env::remove_var("VISUAL");

        // The injected "editor" appends a line to the draft file.
        let result = app.edit_draft_with(&mut |command: &mut std::process::Command| {
            let args: Vec<String> = command
                .get_args()
                .map(|arg| arg.to_string_lossy().to_string())
                .collect();
            let path = args.last().expect("the draft path is the last argument");
            let mut existing = std::fs::read_to_string(path).unwrap_or_default();
            existing.push_str("\nedited body");
            std::fs::write(path, existing).unwrap();
            Ok(())
        });
        let text = result.expect("the editor flow succeeds");
        assert_eq!(text, "draft text\nedited body");
        assert!(dir.join("editor").read_dir().unwrap().next().is_none());

        // `/editor` wires the same flow into the input line.
        app.input.set_value("start", None);
        app.handle_submit("/editor");
        assert_eq!(
            app.input.get_value(),
            "start",
            "a no-op editor leaves the draft untouched"
        );
        let message = last_system(&app);
        assert!(message.contains("Draft updated"), "{message}");

        // No editor configured → an actionable message, not a panic.
        std::env::remove_var("EDITOR");
        app.input.set_value("kept", None);
        app.handle_submit("/editor");
        let message = last_system(&app);
        assert!(message.contains("$VISUAL"), "{message}");
        assert_eq!(app.input.get_value(), "kept");

        restore_env("EDITOR", previous);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = rx;
    }

    #[tokio::test]
    async fn editor_flow_reports_a_spawn_failure_and_cleans_up() {
        let _guard = crate::test_env::lock();
        let (mut app, _rx) = make_app(100, 30);
        let dir = std::env::temp_dir().join(format!("tui-editor-fail-{}", random_id()));
        std::fs::create_dir_all(&dir).unwrap();
        app.tui_settings_path = dir.join("settings.json");
        let previous = std::env::var("EDITOR").ok();
        std::env::set_var("EDITOR", "true");
        let result = app.edit_draft_with(&mut |_command: &mut std::process::Command| {
            Err(crate::external_editor::EditorError::Io("no tty".into()))
        });
        assert!(matches!(
            result,
            Err(crate::external_editor::EditorError::Io(_))
        ));
        // The draft file was removed even though the editor failed.
        let draft_dir = dir.join("editor");
        let leftovers = std::fs::read_dir(&draft_dir)
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(leftovers, 0, "an aborted spawn must not leak the draft");
        match previous {
            Some(value) => std::env::set_var("EDITOR", value),
            None => std::env::remove_var("EDITOR"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn editor_resumes_the_terminal_and_forces_a_redraw() {
        let _guard = crate::test_env::lock();
        let (mut app, _rx) = make_app(100, 30);
        let dir = std::env::temp_dir().join(format!("tui-editor-term-{}", random_id()));
        std::fs::create_dir_all(&dir).unwrap();
        app.tui_settings_path = dir.join("settings.json");
        let previous = std::env::var("EDITOR").ok();
        std::env::set_var("EDITOR", "true");
        // `start` is what installs `input_tx`; install it directly instead
        // (a real `start` would block on the agent handshake).
        let (input_tx, _input_rx) = mpsc::unbounded_channel();
        app.input_tx = Some(input_tx);
        assert!(app.previous_width == 0);
        app.edit_draft_with(&mut |_| Ok(())).unwrap();
        assert!(
            app.render_now || app.render_deadline.is_some(),
            "resuming forces a full redraw"
        );
        match previous {
            Some(value) => std::env::set_var("EDITOR", value),
            None => std::env::remove_var("EDITOR"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn providers_command_lists_and_reports_providers() {
        let mut mock = AppMockAgent::default();
        mock.overrides.insert(
            "list_providers".into(),
            r#"{"builtin":[{"id":"openai","name":"OpenAI","baseUrl":"https://api.openai.com/v1","hasApiKey":true,"modelCount":3}],"custom":[{"id":"acme","name":"Acme","api":"openai-completions","baseUrl":"http://localhost:8080","hasApiKey":false,"models":[{"id":"a1","name":"A1"}]}]}"#.to_string(),
        );
        let (addr, seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());

        app.handle_submit("/providers");
        pump_until_overlay(&mut app, &mut rx).await;
        assert!(
            top_overlay_is::<ProviderListOverlay>(&app),
            "the provider picker is the provider_dialogs overlay"
        );
        let rows = app.overlay_stack[0].component.render(80);
        let text = rows
            .iter()
            .map(|row| strip_ansi_codes(row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("OpenAI"), "{text}");
        assert!(text.contains("Built-in"), "{text}");

        // Adding opens the form overlay; cancelling returns to the list.
        press_on_overlay(&mut app, &mut rx, "a");
        assert!(top_overlay_is::<ProviderFormOverlay>(&app));
        press_on_overlay(&mut app, &mut rx, "escape");
        pump_until_overlay(&mut app, &mut rx).await;
        assert!(top_overlay_is::<ProviderListOverlay>(&app));

        // `r` reloads the auth store and reports the outcome.
        press_on_overlay(&mut app, &mut rx, "r");
        pump_until_msg(&mut app, &mut rx, "reload auth").await;
        let sent = seen.lock().unwrap().clone();
        assert!(
            sent.iter().any(|(t, _)| t == "reload_auth"),
            "reload_auth must reach the agent: {sent:?}"
        );
        assert!(
            sent.iter().any(|(t, _)| t == "list_providers"),
            "the list refreshes after the mutation: {sent:?}"
        );
    }

    #[tokio::test]
    async fn providers_list_reports_a_failed_load() {
        let (mut app, mut rx) = make_app(100, 30);
        app.handle_submit("/providers");
        pump(&mut app, &mut rx).await;
        let messages = system_messages(&app);
        assert!(
            messages
                .iter()
                .any(|m| m.contains("Failed to load providers")),
            "{messages:?}"
        );
        assert!(app.overlay_stack.is_empty());
    }

    #[tokio::test]
    async fn provider_form_submit_upserts_and_reports() {
        let mut mock = AppMockAgent::default();
        mock.overrides.insert(
            "list_providers".into(),
            r#"{"builtin":[],"custom":[]}"#.to_string(),
        );
        let (addr, seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.handle_submit("/providers");
        pump_until_overlay(&mut app, &mut rx).await;
        press_on_overlay(&mut app, &mut rx, "a");
        assert!(top_overlay_is::<ProviderFormOverlay>(&app));

        // A client-side validation failure stays on the form and explains why
        // (the form's own `validate_provider_input` rejects the empty id).
        press_on_overlay(&mut app, &mut rx, "enter");
        assert!(top_overlay_is::<ProviderFormOverlay>(&app));
        let error = app.overlay_stack[0]
            .component
            .as_any()
            .downcast_ref::<ProviderFormOverlay>()
            .unwrap()
            .form()
            .error
            .clone();
        assert!(error.is_some(), "the form explains the rejection");
        let rendered = app.overlay_stack[0].component.render(80);
        let text = rendered
            .iter()
            .map(|row| strip_ansi_codes(row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains(&error.clone().unwrap()), "{text}");

        // Fill the form through its own API and submit again.
        let idx = app.get_top_overlay_index().unwrap();
        {
            let form = app.overlay_stack[idx]
                .component
                .as_any_mut()
                .downcast_mut::<ProviderFormOverlay>()
                .unwrap();
            form.form_mut().id = "acme".into();
            form.form_mut().name = "Acme".into();
            form.form_mut().base_url = "http://localhost:8080".into();
        }
        press_on_overlay(&mut app, &mut rx, "enter");
        pump_until_msg(&mut app, &mut rx, "provider acme saved").await;
        assert!(app.overlay_stack.is_empty() || top_overlay_is::<ProviderListOverlay>(&app));
        let sent = seen.lock().unwrap().clone();
        assert!(
            sent.iter().any(|(t, _)| t == "upsert_provider"),
            "upsert_provider must reach the agent: {sent:?}"
        );
    }

    #[tokio::test]
    async fn provider_keys_are_captured_from_the_next_submission() {
        let (addr, seen) = spawn_app_mock().await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.handle_submit("/provider-key acme");
        assert_eq!(app.pending_secret.as_deref(), Some("acme"));
        let message = last_system(&app);
        assert!(message.contains("API key for acme"), "{message}");

        // The next submission is the key, not a prompt.
        app.handle_submit("sk-secret");
        pump_until_msg(&mut app, &mut rx, "key updated").await;
        let sent = seen.lock().unwrap().clone();
        assert!(
            sent.iter().any(|(t, _)| t == "set_auth"),
            "set_auth must reach the agent: {sent:?}"
        );
        assert!(app.pending_secret.is_none());

        // An empty submission clears the stored key; /cancel-input aborts.
        app.handle_submit("/provider-key acme");
        app.handle_submit("");
        assert_eq!(
            app.pending_secret, None,
            "a blank submit is captured by the key prompt (it clears the key)"
        );
        pump(&mut app, &mut rx).await;
        // Hoisted out of the assert's lazy arguments (taxonomy (e)): an
        // expression argument is only evaluated when the assertion fails, so
        // leaving it inline makes its line permanently uncovered.
        let after_blank = system_messages(&app);
        let cleared = after_blank.iter().any(|m| m.contains("key cleared"));
        let message = format!("the blank submit reports the clear: {after_blank:?}");
        assert!(cleared, "{message}");

        // `/cancel-input` aborts a pending prompt without touching the key.
        app.handle_submit("/provider-key acme");
        app.handle_submit("/cancel-input");
        assert!(app.pending_secret.is_none());
        assert!(last_system(&app).contains("Cancelled the key prompt"));

        // A blank submit with NO prompt pending is still a no-op: it must not
        // fall through to the prompt path.
        let before = system_messages(&app).len();
        app.handle_submit("");
        pump(&mut app, &mut rx).await;
        assert_eq!(system_messages(&app).len(), before);
    }

    #[tokio::test]
    async fn notifications_fire_on_run_state_changes() {
        let (mut app, _rx) = make_app(100, 30);
        app.state.cwd = "/tmp/project".into();
        app.state.model = "openai/gpt-4o".into();
        app.state.session_name = Some("main".into());
        // Default settings (no `notify` key): bell + OSC 9 + run-state title.
        assert!(app.tui_settings.notify_config().title);
        assert!(app.tui_settings.notify_config().bell);

        app.handle_agent_event(&make_event_with_run("agent_start", "{}", "run-1"));
        let started = terminal_writes(&app);
        assert!(started.contains("\x1b]0;"), "the title is set: {started:?}");
        assert!(started.contains("[>]"), "streaming marks the title");
        assert!(started.contains("gpt-4o"), "{started:?}");

        app.terminal.writes.borrow_mut().clear();
        bind_our_run(&mut app, "run-1");
        app.handle_agent_event(&make_event_with_run(
            "agent_end",
            r#"{"state":"completed"}"#,
            "run-1",
        ));
        let ended = terminal_writes(&app);
        assert!(
            ended.contains("\x1b]9;"),
            "OSC 9 desktop notification: {ended:?}"
        );
        assert!(wrote_bell(&app), "the bell still rings: {ended:?}");
        assert!(
            !ended.contains("[>]"),
            "the title drops the streaming marker: {ended:?}"
        );
    }

    #[tokio::test]
    async fn notification_settings_disable_channels() {
        let (mut app, _rx) = make_app(100, 30);
        app.tui_settings.notify = Some(NotifyConfig {
            enabled: false,
            bell: true,
            osc9: true,
            title: true,
        });
        bind_our_run(&mut app, "run-1");
        app.handle_agent_event(&make_event_with_run(
            "agent_end",
            r#"{"state":"completed"}"#,
            "run-1",
        ));
        let writes = terminal_writes(&app);
        assert!(
            !wrote_bell(&app) && !writes.contains("\x1b]9;"),
            "notify.enabled=false silences every channel: {writes:?}"
        );

        // bellOnComplete=false keeps silencing the bell (legacy key).
        let (mut app, _rx) = make_app(100, 30);
        app.tui_settings.bell_on_complete = Some(false);
        bind_our_run(&mut app, "run-1");
        app.handle_agent_event(&make_event_with_run(
            "agent_end",
            r#"{"state":"completed"}"#,
            "run-1",
        ));
        assert!(!wrote_bell(&app));
        assert!(
            terminal_writes(&app).contains("\x1b]9;"),
            "the OSC 9 channel is independent of bellOnComplete"
        );
    }

    #[tokio::test]
    async fn tui_settings_persist_theme_and_notify_config() {
        let settings = TuiSettings {
            theme_id: Some("light".into()),
            notify: Some(NotifyConfig {
                enabled: true,
                bell: false,
                osc9: true,
                title: true,
            }),
            ..TuiSettings::default()
        };
        let json = serde_json::to_string_pretty(&settings.to_json()).unwrap();
        assert!(json.contains("\"themeId\": \"light\""), "{json}");
        assert!(json.contains("\"bell\": false"), "{json}");
        let parsed: Value = serde_json::from_str(&json).unwrap();
        let back = TuiSettings::from_json(&parsed);
        assert_eq!(back.theme_id.as_deref(), Some("light"));
        let config = back.notify_config();
        assert!(!config.bell && config.osc9 && config.title && config.enabled);

        // A malformed notify object falls back to the defaults.
        let v: Value = serde_json::from_str(r#"{"notify":"nonsense"}"#).unwrap();
        let fallback = TuiSettings::from_json(&v);
        assert!(fallback.notify.is_none());
        assert!(fallback.notify_config().enabled);

        // The legacy bellOnComplete=false still silences the bell.
        let v: Value = serde_json::from_str(r#"{"bellOnComplete":false}"#).unwrap();
        assert!(!TuiSettings::from_json(&v).notify_config().bell);
    }

    #[tokio::test]
    async fn loaded_settings_apply_the_persisted_theme() {
        let dir = std::env::temp_dir().join(format!("tui-theme-load-{}", random_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"themeId":"dracula","enabledModelIds":["a"],"bellOnComplete":false}"#,
        )
        .unwrap();
        let (mut app, _rx) = make_app(100, 30);
        app.tui_settings_path = path.clone();
        app.load_tui_settings();
        assert_eq!(app.tui_settings.theme_id.as_deref(), Some("dracula"));
        assert_eq!(app.theme, crate::themes::theme_by_id("dracula").unwrap());
        assert_eq!(app.chat.theme(), app.theme);
        // Startup reaches the whole chrome, not only the conversation.
        assert_eq!(app.footer.theme(), app.theme);
        assert_eq!(app.input.theme(), app.theme);
        assert_eq!(app.enabled_model_ids, Some(vec!["a".to_string()]));
        assert!(!app.tui_settings.notify_config().bell);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tool_output_toggle_rerenders_the_chat() {
        let (mut app, rx) = make_app(100, 30);
        app.chat.render(100);
        app.chat
            .add_tool_start("t1", "shell", Some(r#"{"command":"ls"}"#.into()));
        app.chat
            .append_tool_delta("t1", "one\ntwo\nthree\nfour\nfive\nsix\n");
        app.chat.finish_tool("t1", None, false);
        let collapsed = app.chat.render_all(100).len();
        assert_eq!(collapsed, 1 + 1, "row + blank");
        app.handle_key_action(KeyAction::ToggleToolOutput);
        assert!(app.chat.tool_output_expanded());
        let expanded = app.chat.render_all(100).len();
        assert_eq!(expanded, 1 + 6 + 1, "row + 6 lines + blank");
        app.handle_key_action(KeyAction::ToggleToolOutput);
        assert!(!app.chat.tool_output_expanded());
        assert_eq!(app.chat.render_all(100).len(), collapsed);
        let _ = rx;
    }

    #[tokio::test]
    async fn key_strokes_route_ctrl_g_and_ctrl_x_to_the_actions() {
        let (mut app, mut rx) = make_app(100, 30);
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        app.clipboard = recording_clipboard(&log);
        app.chat
            .add_message(ChatMessage::new("a1".into(), ChatRole::Assistant, "answer"));
        // 0x18 = ctrl+x, 0x07 = ctrl+g — the raw bytes a terminal sends, run
        // through `parse_key` and the global keybinding table.
        app.handle_input("\u{18}");
        while let Ok(cmd) = rx.try_recv() {
            app.handle_cmd(cmd);
        }
        assert_eq!(
            clipboard_log(&log).len(),
            1,
            "ctrl+x copies the last answer"
        );
        app.handle_input("\u{7}");
        while let Ok(cmd) = rx.try_recv() {
            app.handle_cmd(cmd);
        }
        assert!(
            app.chat.tool_output_expanded(),
            "ctrl+g expands tool output"
        );
        app.handle_input("\u{7}");
        while let Ok(cmd) = rx.try_recv() {
            app.handle_cmd(cmd);
        }
        assert!(!app.chat.tool_output_expanded());
    }

    // ─── /agent, /stats, /history, /tool-output, /export, toggles ──────

    /// Rendered overlay text (ANSI stripped); the overlay may have been
    /// repainted by a previous test step, so read it fresh.
    fn overlay_text(app: &mut App<FakeTerminal>, width: usize) -> String {
        app.overlay_stack
            .iter_mut()
            .map(|entry| entry.component.render(width).join("\n"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn agent_info_lines_render_every_field_or_a_dash() {
        let lines = agent_info_lines(&json_parse(
            r#"{"version":"1.2.3","agentInstanceId":"inst-9","skillsCount":7}"#,
        ));
        assert_eq!(lines.len(), 4);
        assert!(lines[0].contains("Agent information"), "{lines:?}");
        assert_eq!(lines[1], "  version: 1.2.3");
        assert_eq!(lines[2], "  instance id: inst-9");
        assert_eq!(lines[3], "  skills discovered: 7");

        // An older/newer agent that omits the fields still renders.
        let lines = agent_info_lines(&json_parse("{}"));
        assert_eq!(lines[1], "  version: ");
        assert_eq!(lines[2], "  instance id: ");
        assert_eq!(lines[3], "  skills discovered: -");
    }

    #[test]
    fn session_ledger_lines_render_counters_and_default_the_missing_half() {
        let full = json_parse(
            r#"{"sessionId":"s1","sessionFile":"/tmp/s1.jsonl","userMessages":2,"assistantMessages":3,"toolCalls":4,"toolResults":5,"totalMessages":9,"tokens":{"input":10,"output":20,"cacheRead":1,"cacheWrite":2,"total":30},"cost":1.5}"#,
        );
        let text = session_ledger_lines(&full).join("\n");
        for needle in [
            "file: /tmp/s1.jsonl",
            "messages: 9",
            "user/assistant: 2/3",
            "tool calls/results: 4/5",
            "tokens in/out: 10/20",
            "tokens cache read/write: 1/2",
            "tokens total: 30",
            "cost: 1.5",
        ] {
            assert!(text.contains(needle), "missing {needle} in {text}");
        }
        // The old panel's header and its `session:` row are gone on purpose:
        // `/status` already prints `Session:`, and a section does not need a
        // title when its first label is self-describing.
        assert!(!text.contains("Session statistics:"), "{text}");
        assert!(!text.contains("session: "), "{text}");

        // Counters and the whole `tokens` object may be absent: `-`, not 0.
        let sparse = session_ledger_lines(&json_parse(r#"{"cost":0}"#)).join("\n");
        for needle in [
            "messages: -",
            "user/assistant: -/-",
            "tool calls/results: -/-",
            "tokens in/out: -/-",
            "tokens cache read/write: -/-",
            "tokens total: -",
            "cost: 0",
        ] {
            assert!(sparse.contains(needle), "missing {needle} in {sparse}");
        }
    }

    #[test]
    fn history_match_lines_render_snippets_blocks_and_the_more_marker() {
        let bg = Chrome::LEGACY.highlight_bg;
        let payload = json_parse(
            r#"{"matches":[{"entryId":"e1","role":"user","kind":"text","snippet":"first\nmatch"},{"entryId":"e2","role":"assistant","kind":"tool_call","snippet":"call","toolCallId":"c1","toolName":"read"}],"hasMore":true}"#,
        );
        let lines = history_match_lines("needle", &payload, bg);
        assert_eq!(lines[0], "History matches for 'needle': 2");
        // One blank separator, then a three-row block per match: the human row
        // (time, role, tool), the snippet, the dimmed machine ids.
        assert_eq!(lines[1], "");
        assert_eq!(
            lines[2],
            format!("{} · {}", crate::theme::dim("--:--:--"), bold("user"))
        );
        // (no "needle" in "first match" — the window can slice the hit away —
        // so this row renders unmarked)
        assert_eq!(lines[3], "  first match", "snippets stay one line each");
        assert_eq!(lines[4], format!("  {}", crate::theme::dim("#e1 · text")));
        assert_eq!(lines[5], "");
        assert_eq!(
            lines[6],
            format!(
                "{} · {} · read",
                crate::theme::dim("--:--:--"),
                bold("assistant")
            )
        );
        assert_eq!(lines[7], "  call");
        assert_eq!(
            lines[8],
            format!("  {}", crate::theme::dim("#e2 · tool_call · call c1"))
        );
        assert_eq!(lines[9], "");
        assert!(lines[10].contains("more matches"), "{lines:?}");
        assert_eq!(lines.len(), 11, "{lines:?}");

        // No matches at all → no lines, so the caller emits the miss instead.
        let empty = history_match_lines("nope", &json_parse(r#"{"matches":[]}"#), bg);
        assert!(empty.is_empty(), "{empty:?}");
        assert!(history_match_lines("nope", &json_parse("{}"), bg).is_empty());
    }

    #[test]
    fn history_match_lines_highlight_every_hit_and_survive_a_sliced_one() {
        let bg = Chrome::LEGACY.highlight_bg;
        let mark = |text: &str| format!("\x1b[48;5;{bg}m\x1b[1m{text}\x1b[0m");
        let payload = json_parse(
            r#"{"matches":[{"entryId":"e1","kind":"text","role":"user","snippet":"中文 needle 尾巴 needle 🚀"}]}"#,
        );
        let lines = history_match_lines("Needle", &payload, bg);
        // Case-insensitive (the agent's search is), every occurrence, and the
        // CJK/emoji neighbours are outside the marks — a byte-offset splice
        // would put one inside 尾 or split the rocket.
        assert_eq!(
            lines[3],
            format!("  中文 {} 尾巴 {} 🚀", mark("needle"), mark("needle"))
        );
        assert_eq!(
            strip_ansi_codes(&lines[3]),
            "  中文 needle 尾巴 needle 🚀",
            "the pager's `y` copy must stay clean"
        );

        // A payload whose snippet no longer holds the query (the agent's window
        // sliced it off) renders unmarked — and reaches the pager as plain text.
        let sliced = json_parse(
            r#"{"matches":[{"entryId":"e1","kind":"text","role":"user","snippet":"…tail only"}]}"#,
        );
        let lines = history_match_lines("needle", &sliced, bg);
        assert_eq!(lines[3], "  …tail only");
    }

    #[test]
    fn history_match_lines_render_intact_cjk_through_the_pager() {
        let bg = Chrome::LEGACY.highlight_bg;
        let payload = json_parse(
            r#"{"matches":[{"entryId":"e1","kind":"text","role":"user","snippet":"中文 needle 尾巴 needle 🚀"}]}"#,
        );
        let mut pager = Pager::new();
        pager.set_content(history_match_lines("needle", &payload, bg));
        let rows = pager.render(40, 8);
        // The marks carry no columns: every row still lands on exactly the
        // pager's width, so a CJK/emoji hit cannot push the layout sideways.
        for row in &rows {
            assert_eq!(visible_width(row), 40, "{row:?}");
        }
        // …and the wide characters come through the renderer whole, both hits on
        // one row between them (a byte-offset splice would cut 尾 or 🚀 apart).
        assert_eq!(
            strip_ansi_codes(&rows[3]).trim_end(),
            "  中文 needle 尾巴 needle 🚀"
        );
    }

    #[test]
    fn history_match_lines_render_a_timestamp_or_a_placeholder() {
        let bg = Chrome::LEGACY.highlight_bg;
        // A real ms value is rendered in the local zone; the mock pins
        // `2025-06-15T07:06:40Z`, and the expected text is derived from the
        // same ms through the same zone so the assertion holds in any TZ.
        let ms = 1_750_000_000_000i64;
        let want = chrono::DateTime::from_timestamp_millis(ms)
            .unwrap()
            .with_timezone(&chrono::Local)
            .format("%H:%M:%S")
            .to_string();
        let payload = json_parse(
            r#"{"matches":[{"entryId":"e1","kind":"text","role":"user","snippet":"x","timestampMs":1750000000000}]}"#,
        );
        let lines = history_match_lines("x", &payload, bg);
        assert_eq!(
            lines[2],
            format!("{} · {}", crate::theme::dim(&want), bold("user"))
        );
        assert_eq!(want.len(), 8, "{want}");

        // No timestamp (null, or the key absent entirely): a placeholder, never
        // a `0`, an epoch time or a `NaN`.
        for payload in [
            json_parse(
                r#"{"matches":[{"entryId":"e1","kind":"text","role":"tool","snippet":"x","timestampMs":null}]}"#,
            ),
            json_parse(
                r#"{"matches":[{"entryId":"e1","kind":"text","role":"tool","snippet":"x"}]}"#,
            ),
        ] {
            let lines = history_match_lines("x", &payload, bg);
            assert_eq!(
                lines[2],
                format!("{} · {}", crate::theme::dim("--:--:--"), bold("tool")),
                "{payload}"
            );
        }
    }

    #[test]
    fn tool_call_lines_list_calls_and_an_empty_page() {
        let page = json_parse(
            r#"{"tools":[{"toolCallId":"c1","name":"read","status":"completed"}],"hasMore":true,"nextOffset":1}"#,
        );
        let lines = tool_call_lines(&page);
        assert!(lines[0].contains("Tool calls in this run"), "{lines:?}");
        assert_eq!(lines[1], "c1  read  completed");
        assert!(lines[2].contains("more calls"), "{lines:?}");
        assert!(lines[3].contains("/tool-output <call-id>"), "{lines:?}");

        let empty = tool_call_lines(&json_parse(r#"{"tools":[]}"#));
        assert_eq!(empty.len(), 3);
        assert_eq!(empty[1], "  (none recorded yet)");
        assert!(empty[2].contains("/tool-output <call-id>"), "{empty:?}");
    }

    #[test]
    fn tool_output_lines_include_every_line_and_report_a_missing_output() {
        let payload = serde_json::json!({
            "output": {"toolCallId": "c1", "text": "line one\nline two", "isError": false}
        });
        let lines = tool_output_lines("c1", &payload);
        assert_eq!(lines[0], "Output of c1 (ok):");
        assert_eq!(
            &lines[1..],
            &["line one".to_string(), "line two".to_string()]
        );

        let failed = serde_json::json!({
            "output": {"toolCallId": "c2", "text": "boom", "isError": true}
        });
        assert_eq!(tool_output_lines("c2", &failed)[0], "Output of c2 (error):");

        // `output: null` (no stored result) and a missing key both yield no
        // lines, which the caller turns into a system message.
        let null = serde_json::json!({"output": null});
        assert!(tool_output_lines("c3", &null).is_empty());
        assert!(tool_output_lines("c4", &json_parse("{}")).is_empty());
    }

    #[test]
    fn export_result_message_reports_the_path_size_and_html_fallbacks() {
        let dir = std::env::temp_dir().join(format!("tui-export-{}", random_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("session.html");
        std::fs::write(&file, "<html></html>").unwrap();
        let path = file.display().to_string();

        let message = export_result_message(&serde_json::json!({"path": path}));
        assert_eq!(message, format!("Exported session to {path} (13 bytes)."));

        // A path this process cannot stat (agent and TUI need not share a
        // filesystem) still reports the file the agent wrote.
        let absent = dir.join("absent.html").display().to_string();
        let message = export_result_message(&serde_json::json!({"path": absent}));
        assert_eq!(message, format!("Exported session to {absent}."));

        // An agent that answers with the HTML body and writes no file.
        let message = export_result_message(&serde_json::json!({"html": "<html></html>"}));
        assert!(message.contains("HTML text"), "{message}");

        let message = export_result_message(&json_parse("{}"));
        assert!(message.contains("no file path"), "{message}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_toggle_arg_understands_on_off_and_rejects_junk() {
        assert_eq!(App::<FakeTerminal>::parse_toggle_arg(""), Ok(None));
        assert_eq!(App::<FakeTerminal>::parse_toggle_arg("   "), Ok(None));
        assert_eq!(App::<FakeTerminal>::parse_toggle_arg("on"), Ok(Some(true)));
        assert_eq!(App::<FakeTerminal>::parse_toggle_arg("ON"), Ok(Some(true)));
        assert_eq!(
            App::<FakeTerminal>::parse_toggle_arg(" Enabled "),
            Ok(Some(true))
        );
        assert_eq!(
            App::<FakeTerminal>::parse_toggle_arg("off"),
            Ok(Some(false))
        );
        assert_eq!(
            App::<FakeTerminal>::parse_toggle_arg("Disabled"),
            Ok(Some(false))
        );
        assert_eq!(
            App::<FakeTerminal>::parse_toggle_arg("maybe"),
            Err("maybe".to_string())
        );
    }

    #[tokio::test]
    async fn inspection_results_open_the_pager_and_failures_report_why() {
        let (mut app, _rx) = make_app(100, 30);

        app.handle_cmd(UiCmd::AgentInfoLoaded(Ok(json_parse(
            r#"{"version":"1.2.3","agentInstanceId":"inst-9","skillsCount":7}"#,
        ))));
        let text = overlay_text(&mut app, 90);
        assert!(text.contains("inst-9"), "{text}");
        assert!(
            text.contains("session: "),
            "the panel adds local facts: {text}"
        );
        app.handle_cmd(UiCmd::OverlayCancel);
        app.handle_cmd(UiCmd::AgentInfoLoaded(Err("down".into())));
        assert!(last_system(&app).contains("Failed to load agent info: down"));

        // `/stats` used to open a second pager for these counters; `/status`
        // carries them now (covered by the StatusLoaded cases above), so the
        // panel-level assertions live there.

        // A query with no match is a system message, not an empty pager.
        app.handle_cmd(UiCmd::HistorySearched {
            query: "q".into(),
            result: Ok(json_parse(r#"{"matches":[]}"#)),
        });
        assert!(app.overlay_stack.is_empty(), "no empty pager");
        assert!(last_system(&app).contains("No history matches for 'q'"));
        app.handle_cmd(UiCmd::HistorySearched {
            query: "q".into(),
            result: Ok(json_parse(
                r#"{"matches":[{"entryId":"e1","kind":"text","role":"user","snippet":"hit"}]}"#,
            )),
        });
        assert!(overlay_text(&mut app, 90).contains("hit"));
        app.handle_cmd(UiCmd::OverlayCancel);
        app.handle_cmd(UiCmd::HistorySearched {
            query: "q".into(),
            result: Err("boom".into()),
        });
        assert!(last_system(&app).contains("History search failed: boom"));

        app.handle_cmd(UiCmd::ToolCallsLoaded(Ok(json_parse(r#"{"tools":[]}"#))));
        assert!(overlay_text(&mut app, 90).contains("none recorded"));
        app.handle_cmd(UiCmd::OverlayCancel);
        app.handle_cmd(UiCmd::ToolCallsLoaded(Err("x".into())));
        assert!(last_system(&app).contains("Failed to list tool calls: x"));

        app.handle_cmd(UiCmd::ToolOutputLoaded {
            tool_call_id: "c1".into(),
            result: Ok(json_parse(r#"{"output":{"text":"full stored body"}}"#)),
        });
        assert!(overlay_text(&mut app, 90).contains("full stored body"));
        app.handle_cmd(UiCmd::OverlayCancel);
        app.handle_cmd(UiCmd::ToolOutputLoaded {
            tool_call_id: "c1".into(),
            result: Err("x".into()),
        });
        assert!(last_system(&app).contains("Failed to read tool output: x"));
        app.handle_cmd(UiCmd::ToolOutputLoaded {
            tool_call_id: "c9".into(),
            result: Ok(json_parse(r#"{"output":null}"#)),
        });
        assert!(
            app.overlay_stack.is_empty(),
            "a missing output is not a pager"
        );
        assert!(last_system(&app).contains("No stored output for tool call c9"));

        app.handle_cmd(UiCmd::SessionExported(Ok(
            serde_json::json!({"html": "<html/>"}),
        )));
        // Read the message into a binding first: an `assert!` format argument
        // is only evaluated on failure, so it would never be covered.
        let text = last_system(&app);
        assert!(text.contains("HTML text"), "{text}");
        app.handle_cmd(UiCmd::SessionExported(Err("full disk".into())));
        assert!(last_system(&app).contains("Failed to export session: full disk"));
    }

    #[tokio::test]
    async fn auto_compaction_retry_and_setting_writes_report_their_outcome() {
        let (mut app, _rx) = make_app(100, 30);
        assert!(app.state.auto_compaction_enabled);
        assert!(app.state.auto_retry_enabled);

        app.handle_cmd(UiCmd::AutoCompactionSet {
            enabled: false,
            result: Ok(()),
        });
        assert!(!app.state.auto_compaction_enabled);
        assert_eq!(last_system(&app), "Auto compaction disabled.");
        app.handle_cmd(UiCmd::AutoCompactionSet {
            enabled: true,
            result: Ok(()),
        });
        assert!(app.state.auto_compaction_enabled);
        assert_eq!(last_system(&app), "Auto compaction enabled.");
        // A rejected write must not flip the local mirror (the footer renders
        // it, and the agent kept the old value).
        app.handle_cmd(UiCmd::AutoCompactionSet {
            enabled: false,
            result: Err("busy".into()),
        });
        assert!(app.state.auto_compaction_enabled);
        assert!(last_system(&app).contains("Failed to set auto compaction: busy"));

        app.handle_cmd(UiCmd::AutoRetrySet {
            enabled: false,
            result: Ok(()),
        });
        assert!(!app.state.auto_retry_enabled);
        assert_eq!(last_system(&app), "Auto retry disabled.");
        app.handle_cmd(UiCmd::AutoRetrySet {
            enabled: true,
            result: Ok(()),
        });
        assert!(app.state.auto_retry_enabled);
        assert_eq!(last_system(&app), "Auto retry enabled.");
        app.handle_cmd(UiCmd::AutoRetrySet {
            enabled: false,
            result: Err("busy".into()),
        });
        assert!(app.state.auto_retry_enabled);
        assert!(last_system(&app).contains("Failed to set auto retry: busy"));

        app.handle_cmd(UiCmd::SessionSettingWritten {
            success_message: "System prompt appended.".into(),
            error_prefix: "Failed to append to the system prompt",
            result: Ok(()),
        });
        assert_eq!(last_system(&app), "System prompt appended.");
        app.handle_cmd(UiCmd::SessionSettingWritten {
            success_message: "System prompt appended.".into(),
            error_prefix: "Failed to append to the system prompt",
            result: Err("denied".into()),
        });
        assert_eq!(
            last_system(&app),
            "Failed to append to the system prompt: denied"
        );
    }

    #[tokio::test]
    async fn new_commands_without_their_argument_print_usage() {
        let (mut app, _rx) = make_app(100, 30);
        for (command, needle) in [
            ("/history", "Usage: /history <query>"),
            ("/history   ", "Usage: /history <query>"),
            ("/autocompact maybe", "Usage: /autocompact [on|off]"),
            ("/autoretry maybe", "Usage: /autoretry [on|off]"),
        ] {
            app.handle_submit(command);
            let last = last_system(&app);
            assert!(last.contains(needle), "{command} → {last}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn inspection_commands_round_trip_through_a_live_agent() {
        let mock = AppMockAgent {
            overrides: std::collections::HashMap::from([
                (
                    "get_agent_info".to_string(),
                    r#"{"version":"1.2.3","agentInstanceId":"inst-9","skillsCount":7}"#
                        .to_string(),
                ),
                (
                    "get_session_stats".to_string(),
                    r#"{"totalMessages":3,"toolCalls":1,"tokens":{"total":42},"cost":1.5}"#
                        .to_string(),
                ),
                (
                    "search_session_history".to_string(),
                    r#"{"matches":[{"entryId":"e1","kind":"text","role":"user","snippet":"the needle"}]}"#
                        .to_string(),
                ),
                (
                    "list_tool_calls".to_string(),
                    r#"{"tools":[{"toolCallId":"c1","name":"read","status":"completed"}]}"#
                        .to_string(),
                ),
                (
                    "get_tool_output".to_string(),
                    r#"{"output":{"text":"full stored body"}}"#.to_string(),
                ),
                (
                    "export_html".to_string(),
                    r#"{"path":"no/such/export.html"}"#.to_string(),
                ),
            ]),
            // `list_tool_calls`/`get_tool_output` are run-scoped: the wrapper
            // needs a run id, which the client learns from `agent_start`.
            events: vec![future_rpc::proto::StreamEvent {
                r#type: "agent_start".into(),
                session_id: "s1".into(),
                run_id: "run-1".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();

        for (command, needle) in [
            ("/agent", "inst-9"),
            ("/history needle", "the needle"),
            ("/tool-output", "c1  read  completed"),
            ("/tool-output c1", "full stored body"),
        ] {
            app.handle_submit(command);
            pump_until_overlay(&mut app, &mut rx).await;
            let raw = overlay_text(&mut app, 90);
            // The panes carry styling (the `/history` match mark, the pager's
            // dimmed hints), so the readable-text assertion runs on the bytes
            // the copy path would hand to a clipboard.
            let text = strip_ansi_codes(&raw);
            assert!(text.contains(needle), "{command} rendered {text}");
            if command == "/history needle" {
                // …and the query is marked, not just echoed back in the header.
                let mark = format!(
                    "\x1b[48;5;{}m\x1b[1mneedle\x1b[0m",
                    Chrome::LEGACY.highlight_bg
                );
                assert!(raw.contains(&mark), "{command} rendered {raw}");
            }
            app.handle_cmd(UiCmd::OverlayCancel);
        }

        // The usage ledger that `/stats` used to show now rides in `/status`,
        // which answers in the transcript rather than in a pager.
        app.handle_submit("/status");
        pump_until_msg(&mut app, &mut rx, "tokens total: 42").await;
        // Bound first: a `assert!(cond(x), "{}", cond(x))` re-reads the value
        // and — because the format argument is only evaluated on failure —
        // leaves that second call as a line no passing test ever executes.
        let status = last_system(&app);
        assert!(status.contains("file: "), "{status}");

        // `/export` reports in a system message rather than in a pager.
        app.handle_submit("/export");
        pump_until_msg(&mut app, &mut rx, "Exported session to").await;

        // The wrappers reached the wire with the arguments the user typed.
        let sent = requests.lock().unwrap().clone();
        let by_type = |command: &str| {
            sent.iter()
                .filter(|cmd| cmd.r#type == command)
                .cloned()
                .collect::<Vec<_>>()
        };
        for expected in [
            "get_agent_info",
            "get_session_stats",
            "search_session_history",
            "list_tool_calls",
            "get_tool_output",
            "export_html",
        ] {
            assert!(
                !by_type(expected).is_empty(),
                "{expected} never reached the agent"
            );
        }
        let search = by_type("search_session_history");
        assert_eq!(search[0].message, "needle");
        assert_eq!(search[0].limit, Some(20));
        assert_eq!(
            by_type("get_tool_output")[0].tool_call_id.as_deref(),
            Some("c1")
        );
        // Both run-scoped reads addressed the run the client learned about.
        assert_eq!(by_type("list_tool_calls")[0].run_id, "run-1");
        assert_eq!(by_type("get_tool_output")[0].run_id, "run-1");
        app.stop();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn setting_commands_round_trip_through_a_live_agent() {
        let mock = AppMockAgent::default();
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();

        app.handle_submit("/autocompact off");
        pump_until_msg(&mut app, &mut rx, "Auto compaction disabled.").await;
        assert!(!app.state.auto_compaction_enabled);
        // No argument toggles the current state back on.
        app.handle_submit("/autocompact");
        pump_until_msg(&mut app, &mut rx, "Auto compaction enabled.").await;
        assert!(app.state.auto_compaction_enabled);

        app.handle_submit("/autoretry off");
        pump_until_msg(&mut app, &mut rx, "Auto retry disabled.").await;
        assert!(!app.state.auto_retry_enabled);
        app.handle_submit("/autoretry on");
        pump_until_msg(&mut app, &mut rx, "Auto retry enabled.").await;

        let sent = requests.lock().unwrap().clone();
        let by_type = |command: &str| {
            sent.iter()
                .filter(|cmd| cmd.r#type == command)
                .cloned()
                .collect::<Vec<_>>()
        };
        let enabled: Vec<bool> = by_type("set_auto_compaction")
            .iter()
            .map(|cmd| cmd.enabled)
            .collect();
        assert_eq!(enabled, vec![false, true]);
        let enabled: Vec<bool> = by_type("set_auto_retry")
            .iter()
            .map(|cmd| cmd.enabled)
            .collect();
        assert_eq!(enabled, vec![false, true]);
        app.stop();
    }

    #[tokio::test]
    async fn slash_command_table_covers_the_new_commands() {
        let (app, _rx) = make_app(100, 30);
        let values: Vec<String> = app
            .slash_commands
            .iter()
            .map(|command| command.value.clone())
            .collect();
        for expected in [
            "/theme",
            "/providers",
            "/models",
            "/skills",
            "/tools",
            "/usage",
            "/transcript",
            "/copy",
            "/editor",
            "/export",
            "/agent",
            "/history",
            "/autocompact",
            "/autoretry",
            "/keymap",
            "/tool-output",
        ] {
            assert!(values.contains(&expected.to_string()), "missing {expected}");
        }
        // Every entry has a non-empty description (the autocomplete popup
        // renders it) and the table has no duplicates.
        for command in &app.slash_commands {
            assert!(!command.description.is_empty(), "{}", command.value);
        }
        let mut sorted = values.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), values.len(), "duplicate slash commands");
    }

    /// The card is taller than the pane, so `/help` is a scrolling viewport:
    /// the frame stays pinned, the bottom border is drawn, and the commands
    /// below the fold are one key away. Before this, the tail did not exist on
    /// screen and no key could reach it.
    #[tokio::test]
    async fn help_scrolls_to_the_commands_below_the_fold() {
        let (mut app, mut rx) = make_app(80, 36);
        app.handle_submit("/help");
        assert!(top_overlay_is::<HelpOverlay>(&app));

        let first = top_overlay_text(&mut app, 80);
        let rows: Vec<&str> = first.lines().collect();
        assert_eq!(rows.len(), 36, "the overlay owns the pane, no more");
        assert!(rows[0].starts_with('┌'), "top border: {:?}", rows[0]);
        assert!(rows[35].starts_with('└'), "bottom border: {:?}", rows[35]);
        let hint = rows[34].to_string();
        assert!(hint.contains("more · ↑↓ scroll"), "{hint}");
        assert!(rows[3].contains("Shortcuts:"), "{:?}", rows[3]);
        assert!(
            !first.contains("/usage"),
            "the last commands start below the fold"
        );

        // `down` scrolls the body by exactly one row and leaves the frame put.
        press_on_overlay(&mut app, &mut rx, "down");
        let scrolled = top_overlay_text(&mut app, 80);
        let frame = |text: &str| text.lines().take(3).collect::<Vec<_>>().join("\n");
        assert!(scrolled
            .lines()
            .nth(3)
            .unwrap_or_default()
            .contains("ctrl+c"));
        assert_eq!(frame(&scrolled), frame(&first), "the frame stays pinned");

        // `end` jumps to the tail, and the hint stops promising rows below.
        press_on_overlay(&mut app, &mut rx, "end");
        let last = top_overlay_text(&mut app, 80);
        assert!(last.contains("/usage"), "{last}");
        assert!(last.contains("/tree"), "{last}");
        let hint = last.lines().nth(34).unwrap_or_default().to_string();
        assert!(hint.contains("above · ↑↓ scroll"), "{hint}");
        assert!(!hint.contains("more"), "nothing is left below: {hint}");

        // `home` comes back to the very first window, and a key the viewport
        // does not own leaves it exactly where it was (the card must not react
        // to the chat's keys).
        press_on_overlay(&mut app, &mut rx, "home");
        assert_eq!(top_overlay_text(&mut app, 80), first);
        press_on_overlay(&mut app, &mut rx, "x");
        assert_eq!(top_overlay_text(&mut app, 80), first);
        // `escape` still closes the card (the app intercepts it first).
        app.handle_key("escape");
        assert!(app.overlay_stack.is_empty());
    }

    /// A pane that can show the whole card gets the whole card: no viewport,
    /// no hint row, nothing shifted.
    #[tokio::test]
    async fn a_tall_pane_gets_the_whole_help_card() {
        let (mut app, _rx) = make_app(80, 72);
        app.handle_submit("/help");
        let text = top_overlay_text(&mut app, 80);
        assert!(text.contains("/usage"), "{text}");
        assert!(!text.contains("more ·"), "no hint row is needed");
        assert!(text.lines().last().unwrap_or_default().starts_with('└'));
    }

    /// Every row of the `/help` overlay the user can actually reach, in order:
    /// render a window, page down, repeat until the window stops moving.
    ///
    /// A single render only shows the pane's worth of the card (that is the
    /// D3 fix), so a test that wants to see the whole card has to scroll it —
    /// which is also the thing worth testing: nothing may be *reachable only*
    /// by rendering.
    fn scroll_through_help(app: &mut App<FakeTerminal>, width: usize) -> String {
        let mut seen: Vec<String> = Vec::new();
        let mut previous: Option<Vec<String>> = None;
        for _ in 0..64 {
            let rows: Vec<String> = app.overlay_stack[0]
                .component
                .render(width)
                .iter()
                .map(|row| strip_ansi_codes(row))
                .collect();
            if previous.as_ref() == Some(&rows) {
                break; // the window stopped moving: the end of the card
            }
            seen.extend(rows.iter().cloned());
            previous = Some(rows);
            app.overlay_stack[0].component.handle_input("pagedown");
        }
        seen.join("\n")
    }

    #[tokio::test]
    async fn help_overlay_mentions_the_new_commands() {
        let (mut app, _rx) = make_app(100, 30);
        app.handle_submit("/help");
        let text = scroll_through_help(&mut app, 90);
        for needle in [
            "/theme",
            "/providers",
            "/usage",
            "/export",
            "/agent",
            "/history",
            "/autocompact",
            "/autoretry",
            "/keymap",
            "/tool-output",
            "ctrl+g",
            "ctrl+x",
        ] {
            assert!(text.contains(needle), "help is missing {needle}: {text}");
        }
        // The card and the autocomplete table are both derived from the same
        // dispatcher; drift between them is what this test exists to catch.
        // Scrolling reaches every row, so this covers all 45 commands, not the
        // first screenful.
        let mut from_help: std::collections::BTreeSet<String> = text
            .lines()
            .filter_map(|line| {
                let body = strip_ansi_codes(line)
                    .split('│')
                    .nth(1)?
                    .trim_start()
                    .to_string();
                let token = body.split_whitespace().next()?;
                token.starts_with('/').then(|| token.to_string())
            })
            .collect();
        assert!(
            from_help.remove("/commands:"),
            "the card keeps its section header"
        );
        let from_table: std::collections::BTreeSet<String> = app
            .slash_commands
            .iter()
            .map(|command| command.value.clone())
            .collect();
        assert_eq!(from_help, from_table, "help card and autocomplete drifted");
    }

    /// The three commands this branch deleted must not survive as ghosts.
    ///
    /// A name the autocomplete popup still offers while the dispatcher has no
    /// arm for it is worse than a name that is simply gone: the user picks it,
    /// and either nothing happens or their text goes somewhere they did not
    /// mean. Both surfaces are checked, because they are what the user sees —
    /// the card is the reference and the popup is the path.
    ///
    /// Typing one anyway is not a dead end either, and that is the other half
    /// of the contract: it falls through to the prompt like every other
    /// unknown slash command (`submit_unknown_slash_falls_through_to_prompt`
    /// pins that behaviour for an arbitrary unknown name), so the text shows up
    /// as a user message instead of vanishing.
    #[tokio::test]
    async fn removed_commands_are_gone_from_the_card_and_the_table() {
        const REMOVED: [&str; 3] = ["/rule", "/ephemeral", "/quit-agent"];
        let (mut app, _rx) = make_app(100, 30);
        for removed in REMOVED {
            assert!(
                !app.slash_commands
                    .iter()
                    .any(|command| command.value == removed),
                "{removed} is still offered by autocomplete"
            );
        }

        app.handle_submit("/help");
        let card = scroll_through_help(&mut app, 90);
        for removed in REMOVED {
            assert!(!card.contains(removed), "{removed} is still in /help");
        }
        app.handle_key("escape");

        app.handle_submit("/ephemeral on");
        let last = app.chat.last_message().cloned().unwrap();
        assert_eq!(last.role, ChatRole::User);
        assert_eq!(last.content, "/ephemeral on");
    }

    // ─── provider overlay callbacks ────────────────────────────────────

    fn provider_info(id: &str) -> ProviderInfo {
        ProviderInfo {
            id: id.to_string(),
            name: format!("{id} inc"),
            api_type: "openai-completions".into(),
            base_url: "http://localhost:8080".into(),
            models: Vec::new(),
            has_api_key: false,
            builtin: false,
            model_count: 0,
        }
    }

    /// A mock `list_providers` payload with a single custom provider.
    fn provider_list_json(id: &str) -> String {
        format!(
            r#"{{"builtin":[],"custom":[{{"id":"{id}","name":"{id} inc","api":"openai-completions","baseUrl":"http://localhost:8080","hasApiKey":false,"models":[]}}]}}"#
        )
    }

    /// Rendered text of the top overlay, ANSI stripped.
    fn top_overlay_text(app: &mut App<FakeTerminal>, width: usize) -> String {
        let idx = app.get_top_overlay_index().expect("an overlay is open");
        app.overlay_stack[idx]
            .component
            .render(width)
            .iter()
            .map(|row| strip_ansi_codes(row))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Restore an environment variable to its captured value (`None` = unset).
    /// Both arms exist because a test host may or may not define `EDITOR`.
    fn restore_env(key: &str, previous: Option<String>) {
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    /// The two overlay sinks are the only translation from a keypress to a
    /// `UiCmd`: every payable action has to reach the channel, and the
    /// redraw-only actions must stay silent.
    #[test]
    fn provider_sinks_translate_every_overlay_action() {
        use crate::components::provider_dialogs::{ProviderFormAction, ProviderListAction};

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut list = provider_list_sink(tx);
        list(ProviderListAction::None);
        list(ProviderListAction::Moved);
        assert!(rx.try_recv().is_err(), "None/Moved only redraw");
        list(ProviderListAction::Edit(provider_info("acme")));
        assert!(matches!(rx.try_recv(), Ok(UiCmd::ProviderEdit(info)) if info.id == "acme"));
        list(ProviderListAction::Delete(provider_info("acme")));
        assert!(matches!(rx.try_recv(), Ok(UiCmd::ProviderDelete(id)) if id == "acme"));
        list(ProviderListAction::SetKey(provider_info("acme")));
        assert!(matches!(rx.try_recv(), Ok(UiCmd::ProviderSetKey(id)) if id == "acme"));
        list(ProviderListAction::Add);
        assert!(matches!(rx.try_recv(), Ok(UiCmd::ProviderAdd)));
        list(ProviderListAction::SyncModels);
        assert!(matches!(rx.try_recv(), Ok(UiCmd::ProviderSync(a)) if a == "sync_future_models"));
        list(ProviderListAction::ReloadAuth);
        assert!(matches!(rx.try_recv(), Ok(UiCmd::ProviderSync(a)) if a == "reload_auth"));
        list(ProviderListAction::Cancelled);
        assert!(matches!(rx.try_recv(), Ok(UiCmd::OverlayCancel)));

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut form = provider_form_sink(tx);
        form(ProviderFormAction::None);
        form(ProviderFormAction::Changed);
        assert!(rx.try_recv().is_err(), "a form edit only redraws");
        form(ProviderFormAction::Cancel);
        assert!(matches!(rx.try_recv(), Ok(UiCmd::ProviderFormCancelled)));
        let mut input = ProviderInput::create();
        input.id = "acme".into();
        form(ProviderFormAction::Submit(input));
        assert!(matches!(rx.try_recv(), Ok(UiCmd::ProviderSubmit(b)) if b.id == "acme"));
    }

    /// Every `ProviderListAction` a keypress can produce drives the matching
    /// flow against a live agent: edit opens the prefilled form, delete and
    /// sync go through `ProviderActionDone` and refresh the list in place,
    /// `k` starts the key capture.
    #[tokio::test]
    async fn provider_overlay_actions_edit_delete_sync_and_set_the_key() {
        let mut mock = AppMockAgent::default();
        mock.overrides
            .insert("list_providers".into(), provider_list_json("acme"));
        let (addr, seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());

        app.handle_submit("/providers");
        pump_until_overlay(&mut app, &mut rx).await;
        assert!(top_overlay_is::<ProviderListOverlay>(&app));
        // The custom provider lives on the second tab (built-ins come first).
        press_on_overlay(&mut app, &mut rx, "tab");
        assert!(
            top_overlay_text(&mut app, 80).contains("acme"),
            "custom tab"
        );

        // `e` → the edit form, prefilled with the highlighted provider.
        press_on_overlay(&mut app, &mut rx, "e");
        let form_id = app
            .overlay_stack
            .last()
            .unwrap()
            .component
            .as_any()
            .downcast_ref::<ProviderFormOverlay>()
            .expect("edit opens the form")
            .form()
            .id
            .clone();
        assert_eq!(form_id, "acme", "the form is prefilled");

        // `escape` returns to the list (it stays underneath).
        press_on_overlay(&mut app, &mut rx, "escape");
        pump_until_overlay(&mut app, &mut rx).await;
        assert!(top_overlay_is::<ProviderListOverlay>(&app));
        // The rebuilt list opens on the built-in tab again.
        press_on_overlay(&mut app, &mut rx, "tab");

        // `s` → sync_future_models → the list is refreshed from the agent.
        press_on_overlay(&mut app, &mut rx, "s");
        pump_until_msg(&mut app, &mut rx, "sync future models").await;
        let sent = seen.lock().unwrap().clone();
        assert!(
            sent.iter().any(|(t, _)| t == "sync_future_models"),
            "sync_future_models must reach the agent: {sent:?}"
        );

        // `k` → start the API-key capture for the highlighted provider.
        let list_before = seen.lock().unwrap().len();
        press_on_overlay(&mut app, &mut rx, "k");
        assert_eq!(app.pending_secret.as_deref(), Some("acme"));
        let message = last_system(&app);
        assert!(message.contains("API key for acme"), "{message}");
        app.handle_submit("/cancel-input");
        assert!(app.pending_secret.is_none());

        // `d` → delete_provider; the follow-up refresh reuses the open list.
        press_on_overlay(&mut app, &mut rx, "d");
        pump_until_msg(&mut app, &mut rx, "provider acme deleted").await;
        let sent = seen.lock().unwrap().clone();
        assert!(
            sent.iter().any(|(t, _)| t == "delete_provider"),
            "delete_provider must reach the agent: {sent:?}"
        );
        assert!(
            sent[list_before..]
                .iter()
                .any(|(t, _)| t == "list_providers"),
            "the list refreshes after the mutation: {sent:?}"
        );
        assert!(
            top_overlay_is::<ProviderListOverlay>(&app),
            "the list stays open"
        );
    }

    /// A failed mutation has to be visible where the user is looking: the form
    /// error when the form is open, the list notice when it is a list action.
    #[tokio::test]
    async fn provider_mutation_failures_reach_the_form_and_the_list() {
        let mut mock = AppMockAgent::default();
        mock.overrides
            .insert("list_providers".into(), provider_list_json("acme"));
        mock.fail.insert("delete_provider".into());
        mock.fail.insert("upsert_provider".into());
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());

        app.handle_submit("/providers");
        pump_until_overlay(&mut app, &mut rx).await;

        // The list owns the failure: notice set, message printed.
        press_on_overlay(&mut app, &mut rx, "tab");
        press_on_overlay(&mut app, &mut rx, "d");
        pump_until_msg(&mut app, &mut rx, "Failed: nope").await;
        assert_eq!(app.provider_notice.as_deref(), Some("nope"));
        let text = top_overlay_text(&mut app, 80);
        assert!(text.contains("nope"), "the list shows the notice: {text}");

        // The form owns it instead when a submit fails. Clear the transcript
        // first so the second failure is unambiguous.
        app.chat.clear_messages();
        press_on_overlay(&mut app, &mut rx, "a");
        assert!(top_overlay_is::<ProviderFormOverlay>(&app));
        let idx = app.get_top_overlay_index().unwrap();
        {
            let form = app.overlay_stack[idx]
                .component
                .as_any_mut()
                .downcast_mut::<ProviderFormOverlay>()
                .unwrap();
            form.form_mut().id = "acme".into();
            form.form_mut().name = "Acme".into();
            form.form_mut().base_url = "http://localhost:8080".into();
        }
        press_on_overlay(&mut app, &mut rx, "enter");
        pump_until_msg(&mut app, &mut rx, "Failed: nope").await;
        let idx = app.get_top_overlay_index().unwrap();
        let error = app.overlay_stack[idx]
            .component
            .as_any()
            .downcast_ref::<ProviderFormOverlay>()
            .unwrap()
            .form()
            .error
            .clone();
        assert_eq!(error.as_deref(), Some("nope"), "the form shows the failure");
    }

    /// A mutation that fails once the user has closed the form *and* the list
    /// has no overlay left to render the error on: the transcript still carries
    /// it, and the empty stack is left empty (no panic, no resurrected overlay).
    #[tokio::test]
    async fn provider_failure_with_every_overlay_closed_still_reports_it() {
        let mut mock = AppMockAgent::default();
        mock.overrides
            .insert("list_providers".into(), provider_list_json("acme"));
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());

        app.handle_submit("/providers");
        pump_until_overlay(&mut app, &mut rx).await;
        assert!(top_overlay_is::<ProviderListOverlay>(&app));
        // Close the list; the second cancel finds nothing and is a no-op.
        app.handle_cmd(UiCmd::OverlayCancel);
        app.handle_cmd(UiCmd::OverlayCancel);
        assert!(app.overlay_stack.is_empty());
        app.chat.clear_messages();

        app.handle_cmd(UiCmd::ProviderActionDone {
            action: "provider acme deleted".into(),
            result: Err("gone".into()),
        });

        assert_eq!(app.provider_notice.as_deref(), Some("gone"));
        assert_eq!(last_system(&app), "Failed: gone");
        assert!(
            app.overlay_stack.is_empty(),
            "a failure must not resurrect an overlay"
        );
    }

    /// The app re-validates a submit that reaches it as a command, so a bad
    /// payload is rejected locally instead of being sent to the agent.
    #[tokio::test]
    async fn provider_submit_rejects_invalid_input_before_the_wire() {
        let mut mock = AppMockAgent::default();
        mock.overrides
            .insert("list_providers".into(), provider_list_json("acme"));
        let (addr, seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.handle_submit("/providers");
        pump_until_overlay(&mut app, &mut rx).await;
        press_on_overlay(&mut app, &mut rx, "a");

        let mut bad = ProviderInput::create();
        bad.id = "acme".into();
        bad.api_type = "not-a-dialect".into();
        app.handle_cmd(UiCmd::ProviderSubmit(Box::new(bad)));

        assert_eq!(
            app.provider_notice.as_deref(),
            Some("unsupported provider API type")
        );
        let idx = app.get_top_overlay_index().unwrap();
        let error = app.overlay_stack[idx]
            .component
            .as_any()
            .downcast_ref::<ProviderFormOverlay>()
            .unwrap()
            .form()
            .error
            .clone();
        assert_eq!(error.as_deref(), Some("unsupported provider API type"));
        let message = last_system(&app);
        assert!(message.contains("Invalid provider"), "{message}");
        let sent = seen.lock().unwrap().clone();
        assert!(
            !sent.iter().any(|(t, _)| t == "upsert_provider"),
            "a rejected payload never reaches the agent: {sent:?}"
        );

        // Cancelling the form returns to the list, which still shows the
        // notice the rejection left behind.
        app.handle_cmd(UiCmd::ProviderFormCancelled);
        pump_until_overlay(&mut app, &mut rx).await;
        assert!(top_overlay_is::<ProviderListOverlay>(&app));
        let text = top_overlay_text(&mut app, 80);
        assert!(
            text.contains("unsupported provider API type"),
            "the reopened list carries the notice: {text}"
        );
        let _ = rx;
    }

    // ─── model menus, menu commands, small paths ───────────────────────

    /// `/models default` loads the catalog into a menu and writes the choice
    /// back through `set_default_model`; a cancel writes nothing.
    #[tokio::test]
    async fn models_default_menu_writes_the_default_model() {
        let mut mock = AppMockAgent::default();
        mock.overrides.insert(
            "list_models".into(),
            r#"{"models":[{"id":"gpt-4o","label":"GPT-4o","provider":"openai","isDefault":true},{"id":"claude-sonnet-4","label":"Claude Sonnet 4","provider":"anthropic"}]}"#.to_string(),
        );
        let (addr, seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.handle_submit("/models default");
        pump_until_overlay(&mut app, &mut rx).await;
        assert!(top_overlay_is::<MenuOverlay>(&app));
        let text = top_overlay_text(&mut app, 80);
        assert!(text.contains("Default Model"), "{text}");
        assert!(text.contains("gpt-4o"), "{text}");
        assert!(text.contains("default"), "the default is badged: {text}");

        // Moving the highlight is not a confirmation, and escape cancels.
        press_on_overlay(&mut app, &mut rx, "down");
        assert!(
            top_overlay_is::<MenuOverlay>(&app),
            "navigation keeps the menu open"
        );
        press_on_overlay(&mut app, &mut rx, "escape");
        assert!(app.overlay_stack.is_empty());
        let sent = seen.lock().unwrap().clone();
        assert!(
            !sent.iter().any(|(t, _)| t == "set_default_model"),
            "cancelling writes nothing: {sent:?}"
        );

        // Confirming sends the highlighted model.
        app.handle_submit("/models default");
        pump_until_overlay(&mut app, &mut rx).await;
        press_on_overlay(&mut app, &mut rx, "enter");
        pump_until_msg(&mut app, &mut rx, "Model:").await;
        let sent = seen.lock().unwrap().clone();
        assert!(
            sent.iter()
                .any(|(t, a)| t == "set_default_model" && a.contains("claude-sonnet-4")),
            "the confirmed row is written through set_default_model: {sent:?}"
        );
    }

    /// Menu commands that reach the app without a value are local no-ops: a
    /// theme pick with nothing to pick, a cancel, and an empty model write.
    #[tokio::test]
    async fn menu_commands_without_a_value_are_no_ops() {
        let (mut app, _rx) = make_app(100, 30);

        app.show_theme_menu();
        app.handle_cmd(UiCmd::MenuSelected {
            purpose: MenuPurpose::Theme,
            values: Vec::new(),
        });
        assert!(app.overlay_stack.is_empty(), "the menu closes");
        assert!(app.tui_settings.theme_id.is_none(), "nothing persisted");
        assert_eq!(app.theme, crate::theme::DARK_THEME);

        app.show_theme_menu();
        assert!(!app.overlay_stack.is_empty());
        app.handle_cmd(UiCmd::MenuCancelled(MenuPurpose::Theme));
        assert!(app.overlay_stack.is_empty(), "cancel closes the overlay");

        app.show_theme_menu();
        app.handle_cmd(UiCmd::SetDefaultModel(Vec::new()));
        assert!(app.overlay_stack.is_empty(), "the empty write still closes");
    }

    /// `menu_sink` is the shared callback for every popup: cancel and confirm
    /// translate, navigation only redraws.
    #[test]
    fn menu_sink_translates_confirm_cancel_and_toggle() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut sink = App::<FakeTerminal>::menu_sink(tx, MenuPurpose::Theme);
        sink(MenuAction::None);
        sink(MenuAction::Moved);
        sink(MenuAction::TabChanged);
        assert!(rx.try_recv().is_err(), "navigation only redraws");
        sink(MenuAction::Cancelled);
        assert!(matches!(
            rx.try_recv(),
            Ok(UiCmd::MenuCancelled(MenuPurpose::Theme))
        ));
        sink(MenuAction::Confirmed(vec!["light".into()]));
        assert!(matches!(
            rx.try_recv(),
            Ok(UiCmd::MenuSelected { purpose: MenuPurpose::Theme, values }) if values == vec!["light".to_string()]
        ));
        sink(MenuAction::Toggled(vec!["light".into(), "dark".into()]));
        assert!(matches!(
            rx.try_recv(),
            Ok(UiCmd::MenuSelected { values, .. }) if values.len() == 2
        ));
    }

    /// A model row whose label is empty or repeats the id must not print a
    /// second copy of the id.
    #[test]
    fn model_rows_fold_a_redundant_label_into_an_empty_one() {
        let mut bare = sample_models()[1].clone();
        bare.label = String::new();
        let mut same = sample_models()[0].clone();
        same.label = same.full_id();
        let mut nice = sample_models()[0].clone();
        nice.id = "nice".into();
        nice.label = "Nice Name".into();

        let rows = App::<FakeTerminal>::model_rows(vec![bare, same, nice]);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].0, "anthropic/claude-sonnet-4");
        assert!(rows[0].1.is_empty(), "an empty label stays empty");
        assert_eq!(rows[1].0, "openai/gpt-4o");
        assert!(rows[1].1.is_empty(), "a label equal to the id is dropped");
        assert_eq!(rows[2].1, "Nice Name", "a real label is kept");
        assert!(rows.iter().all(|row| !row.2), "no row is a default here");
    }

    /// The scope menu badges the model the agent reports as the default.
    #[test]
    fn scope_menu_badges_the_default_model() {
        let mut models = sample_models();
        models[0].is_default = true;
        let mut state = App::<FakeTerminal>::build_scope_menu(models, None, "openai/gpt-4o");
        let text = state
            .render(80, 20)
            .iter()
            .map(|row| strip_ansi_codes(row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("default"), "{text}");
        assert!(text.contains("openai/gpt-4o"), "{text}");
    }

    /// The sessions menu badges a streaming row and the current one.
    #[tokio::test]
    async fn sessions_menu_marks_the_streaming_and_the_current_rows() {
        let (mut app, _rx) = make_app(100, 30);
        let mut sessions = sample_sessions();
        // s2 streams, s1 is the current session.
        sessions[1].is_streaming = Some(true);
        app.state.session_id = "s1".into();
        app.handle_cmd(UiCmd::SessionsLoaded {
            result: Ok(sessions),
            purpose: SessionsPurpose::Browse,
        });
        let text = top_overlay_text(&mut app, 80);
        assert!(text.contains("streaming"), "{text}");
        assert!(text.contains("current"), "{text}");
    }

    /// Rows that are *not* in the enabled set still render (the `/tools` menu
    /// shows every built-in with the unchecked ones plain).
    #[tokio::test]
    async fn tools_menu_renders_the_disabled_rows_and_reports_a_failed_write() {
        let (mut app, _rx) = make_app(100, 30);
        app.enabled_tools = Some(vec!["read".to_string()]);
        app.handle_submit("/tools");
        let text = top_overlay_text(&mut app, 70);
        for tool in BUILTIN_TOOLS {
            assert!(text.contains(tool), "{tool} missing: {text}");
        }

        // A rejected `disable_tools` is reported as a system message.
        app.handle_cmd(UiCmd::ToolsChanged(Err("nope".into())));
        assert!(
            last_system(&app).contains("Failed to set tools"),
            "{}",
            last_system(&app)
        );
    }

    /// Escape falls through to the generic overlay close when no overlay on the
    /// stack claims it (an idle pager, the usage panel, …), and when every
    /// overlay is hidden. A pager with a live `/` editor claims it instead —
    /// see `escape_closes_the_pager_search_editor_then_the_pager`.
    #[tokio::test]
    async fn escape_closes_a_non_menu_overlay() {
        let (mut app, _rx) = make_app(100, 30);
        app.show_pager_text(vec!["a line".to_string()], "nothing");
        assert!(top_overlay_is::<PagerOverlay>(&app));
        app.handle_key("escape");
        assert!(app.overlay_stack.is_empty(), "escape closes the pager");

        // A stack whose entries are all hidden has no top to hand the escape
        // to: the app-level close still runs.
        app.show_pager_text(vec!["a line".to_string()], "nothing");
        app.overlay_stack[0].hidden = true;
        app.handle_key("escape");
        assert!(app.overlay_stack.is_empty(), "the hidden overlay is popped");
    }

    /// A refresh that arrives when the provider list is *not* the top overlay
    /// leaves the notice alone (the caller opens a fresh list instead).
    #[tokio::test]
    async fn refreshing_the_provider_list_without_one_open_is_a_no_op() {
        let (mut app, _rx) = make_app(100, 30);
        app.provider_notice = Some("kept".to_string());
        // Nothing open at all.
        app.refresh_provider_list(vec![provider_info("acme")]);
        assert_eq!(app.provider_notice.as_deref(), Some("kept"));
        assert!(app.overlay_stack.is_empty());
        // Something else on top.
        app.show_pager_text(vec!["a line".to_string()], "nothing");
        app.refresh_provider_list(vec![provider_info("acme")]);
        assert_eq!(app.provider_notice.as_deref(), Some("kept"));
        assert!(
            top_overlay_is::<PagerOverlay>(&app),
            "the pager is untouched"
        );
    }

    /// A close delivered through the command channel is awaited, not assumed:
    /// the helper drains it, and keeps polling while it is still pending.
    #[tokio::test]
    async fn pump_until_no_overlay_waits_for_a_channel_delivered_close() {
        let (mut app, mut rx) = make_app(100, 30);
        app.show_theme_menu();
        assert!(!app.overlay_stack.is_empty());
        let tx = app.op_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(60)).await;
            let _ = tx.send(UiCmd::OverlayCancel);
        });
        pump_until_no_overlay(&mut app, &mut rx).await;
        assert!(app.overlay_stack.is_empty(), "the queued close was applied");
    }

    /// The cursor-tracking fake has to implement the whole `TerminalIo`
    /// contract the app uses when it stops and restarts the terminal.
    #[tokio::test]
    async fn tracking_terminal_supports_stop_resume_and_exit_signals() {
        let (mut app, _rx, _row) = make_tracking_app(80, 24);
        app.running = true;
        // `stop` drains stdin, stops the terminal and puts the cursor back.
        app.stop();
        assert!(!app.is_running());
        // Resuming reinstalls the input/resize callbacks.
        let (input_tx, _input_rx) = mpsc::unbounded_channel();
        app.input_tx = Some(input_tx);
        app.resume_terminal();
        assert!(app.terminal.on_input.is_some());
        assert!(app.terminal.on_resize.is_some());
        app.suspend_terminal();
        app.terminal.hide_cursor();
        app.terminal.set_exit_signal_callback(None);
        app.terminal.drain_input(0, 0);
        assert_eq!(app.terminal.cols, 80, "the fake keeps its geometry");
        assert_eq!(app.terminal.rows, 24);
    }

    /// `/provider-key` without an id explains the syntax instead of capturing
    /// the next submission.
    #[tokio::test]
    async fn provider_key_without_an_argument_prints_usage() {
        let (mut app, _rx) = make_app(100, 30);
        app.handle_submit("/provider-key");
        assert!(app.pending_secret.is_none(), "nothing was captured");
        assert!(
            last_system(&app).contains("Usage: /provider-key"),
            "{}",
            last_system(&app)
        );
    }

    /// A `/editor` that cannot be spawned is reported, not swallowed.
    #[tokio::test]
    async fn editor_reports_a_spawn_failure() {
        let _guard = crate::test_env::lock();
        let (mut app, _rx) = make_app(100, 30);
        let dir = std::env::temp_dir().join(format!("tui-editor-fail-{}", random_id()));
        std::fs::create_dir_all(&dir).unwrap();
        app.tui_settings_path = dir.join("settings.json");
        let previous_editor = std::env::var("EDITOR").ok();
        let previous_visual = std::env::var("VISUAL").ok();
        std::env::remove_var("VISUAL");
        std::env::set_var("EDITOR", "future-tui-no-such-editor-binary");
        app.handle_submit("/editor");
        let message = last_system(&app);
        assert!(message.contains("Editor failed"), "{message}");
        restore_env("EDITOR", previous_editor);
        restore_env("VISUAL", previous_visual);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Both arms of [`restore_env`] are exercised: a test host may or may not
    /// have the variable set.
    #[test]
    fn restore_env_sets_a_value_and_unset_is_a_removal() {
        let _guard = crate::test_env::lock();
        restore_env("FUTURE_TUI_ENV_PROBE", Some("kept".into()));
        assert_eq!(std::env::var("FUTURE_TUI_ENV_PROBE").as_deref(), Ok("kept"));
        restore_env("FUTURE_TUI_ENV_PROBE", None);
        assert!(std::env::var("FUTURE_TUI_ENV_PROBE").is_err());
    }

    /// Reinstate the terminal after a suspended child: the callbacks handed to
    /// the terminal must forward to the UI channel, and the screen is redrawn.
    #[tokio::test]
    async fn resume_terminal_forwards_input_and_resize_to_the_ui_channel() {
        let (mut app, _rx) = make_app(100, 30);
        let (input_tx, mut input_rx) = mpsc::unbounded_channel();
        app.input_tx = Some(input_tx);
        app.resume_terminal();
        assert!(
            app.render_now || app.render_deadline.is_some(),
            "resuming forces a redraw"
        );

        let on_input = app
            .terminal
            .on_input
            .as_mut()
            .expect("the input callback is installed");
        on_input("typed".to_string());
        let on_resize = app
            .terminal
            .on_resize
            .as_mut()
            .expect("the resize callback is installed");
        on_resize();
        let forwarded = input_rx.try_recv();
        assert!(
            matches!(forwarded, Ok(UiInput::Input(ref text)) if text == "typed"),
            "the typed input is forwarded to the UI channel"
        );
        assert!(matches!(input_rx.try_recv(), Ok(UiInput::Resize)));
        assert!(input_rx.try_recv().is_err());
    }

    /// The cursor tracker has to follow every row-changing sequence the app
    /// emits, and ignore a sequence that never terminates.
    #[test]
    fn track_row_follows_cursor_moves_and_ignores_truncated_sequences() {
        assert_eq!(track_row(2, "x\ny"), 3, "LF advances one row");
        assert_eq!(track_row(2, "\x1b[3A"), 0, "CUU clamps at the top");
        assert_eq!(track_row(2, "\x1b[3B"), 5, "CUD moves down three rows");
        assert_eq!(track_row(2, "\x1b[B"), 3, "CUD without a count is one row");
        assert_eq!(track_row(7, "\x1b[H"), 0, "CUP without params is row 0");
        assert_eq!(track_row(7, "\x1b[4;9H"), 3, "CUP rows are 1-based");
        assert_eq!(track_row(7, "\x1b[4;9f"), 3, "`f` is the same sequence");
        assert_eq!(track_row(5, "\x1b[3"), 5, "a truncated sequence is ignored");
    }

    /// A focusable overlay takes the keyboard away from the editor; a pager
    /// (not focusable) does not.
    #[tokio::test]
    async fn a_focusable_overlay_owns_the_keyboard() {
        let (mut app, _rx) = make_app(100, 30);
        let id = app.show_overlay(
            Box::new(crate::components::input::Input::new()),
            OverlayOptions::default(),
        );
        assert!(
            matches!(app.focused, FocusTarget::Overlay(current) if current == id),
            "the new overlay is focused"
        );
        let focused = app.overlay_stack[0]
            .component
            .as_any()
            .downcast_ref::<crate::components::input::Input>()
            .expect("input overlay")
            .focused;
        assert!(focused, "the focusable overlay was told it has focus");
    }

    /// The replay path accepts tool arguments delivered as a JSON *string* (the
    /// agent sends both shapes) and skips a message whose `blocks` is missing.
    #[tokio::test]
    async fn session_replay_accepts_string_arguments_and_skips_blockless_messages() {
        let (mut app, _rx) = make_app(100, 30);
        app.apply_history_page(
            "s1",
            Ok(json_parse(
                r#"{"entries":[
              {"id":"m0","kind":"user","role":"user","blocks":"not-an-array"},
              {"id":"m1","kind":"assistant","role":"assistant","blocks":[{"kind":"tool_call","toolCallId":"c1","name":"read","arguments":"{\"path\":\"/tmp/x\"}"},{"kind":"tool_call","name":"ghost"},{"kind":"tool_call","toolCallId":"","name":"ghost"}]},
              {"id":"m2","kind":"tool","role":"tool","blocks":[{"kind":"tool_result","toolCallId":"c1","text":"tool out","isError":false}]}
            ]}"#,
            )),
        );
        // The body is only in the transcript once it is asked for (`ctrl+g`).
        app.chat.set_tool_output_expanded(true);
        let text = crate::utils::strip_ansi_codes(&app.chat.render_all(100).join("\n"));
        assert!(text.contains("tool out"), "{text}");
        assert!(
            text.contains("/tmp/x"),
            "the string-encoded arguments survive the replay: {text}"
        );
        assert!(
            !text.contains("not-an-array"),
            "a message without a block array is skipped: {text}"
        );
    }

    /// The page keys as a terminal *sends* them must scroll the card.
    ///
    /// `keys::parse_key("\x1b[6~")` is `"pageDown"` and `"\x1b[5~"` is
    /// `"pageUp"` — camelCase, not the pager's lowercase ids — so this drives
    /// the app's real input path with those bytes: a key the view names
    /// wrongly fails here instead of on a user's terminal.
    #[tokio::test]
    async fn the_page_keys_a_terminal_sends_scroll_the_help_card() {
        let (mut app, _rx) = make_app(80, 36);
        app.handle_submit("/help");
        let first = top_overlay_text(&mut app, 80);

        app.handle_input("\x1b[6~"); // PageDown
        let paged = top_overlay_text(&mut app, 80);
        assert_ne!(paged, first, "PageDown must scroll the card");
        assert!(paged.contains("/usage"), "{paged}");

        app.handle_input("\x1b[5~"); // PageUp
        assert_eq!(top_overlay_text(&mut app, 80), first, "PageUp pages back");

        app.handle_input("\x1b[6~");
        app.handle_input("\x1bOH"); // Home
        assert_eq!(top_overlay_text(&mut app, 80), first, "Home returns");
        app.handle_input("\x1bOF"); // End
        assert!(
            top_overlay_text(&mut app, 80).contains("/usage"),
            "End jumps to the tail"
        );

        // The fallbacks that work on every terminal — including the ones whose
        // Home/End form `parse_key` does not know (`\x1b[F` and `\x1b[H` parse
        // to `None`, and tmux's own `Home`/`End` key names send those).
        app.handle_input("g");
        assert_eq!(top_overlay_text(&mut app, 80), first, "g is the top");
        app.handle_input("G");
        assert!(
            top_overlay_text(&mut app, 80).contains("/usage"),
            "G is the tail"
        );
        app.handle_input("b");
        assert_eq!(top_overlay_text(&mut app, 80), first, "b pages up");
        app.handle_input(" ");
        assert!(
            top_overlay_text(&mut app, 80).contains("/usage"),
            "space pages down"
        );
    }

    // ─── /sandbox and /permission ──────────────────────────────────────

    /// Both catalogue answers repaint the open panel.
    ///
    /// Neither arm writes to the transcript, so neither used to trigger a
    /// render: the panel kept the frame it was opened with — the loading row
    /// stayed on screen and the merged rows never appeared — until the user
    /// pressed another key. The unit tests hid it because `top_overlay_text`
    /// renders on demand; a real tmux pane did not.
    #[tokio::test(flavor = "multi_thread")]
    async fn skills_answers_repaint_the_panel() {
        let (mut app, _rx, _requests) = app_with_live_agent(std::collections::HashMap::new()).await;
        app.handle_submit("/skills");

        app.render_requested = false;
        app.handle_cmd(UiCmd::SkillsLoaded(Ok(json_parse(r#"{"commands":[]}"#))));
        assert!(
            app.render_requested,
            "the get_commands answer repaints the panel"
        );

        app.render_requested = false;
        app.handle_cmd(UiCmd::SkillsCatalogueLoaded(Ok(skills_catalogue_fixture())));
        assert!(
            app.render_requested,
            "the catalogue answer repaints the panel"
        );
        app.stop();
    }

    /// `/sandbox` and `/permission` open one card — the agent owns both halves
    /// of the policy, so there is one screen — but not in the same state: the
    /// tier list has the highlight under `/sandbox`, the tool-permission block
    /// under `/permission`. Two commands opening byte-identical panels is what
    /// this pins down (the card itself is unchanged, and nothing is hidden
    /// either way).
    #[tokio::test(flavor = "multi_thread")]
    async fn permission_opens_the_permission_half_of_the_shared_card() {
        let (mut sandbox_app, _rx) = make_app(100, 30);
        sandbox_app.handle_submit("/sandbox");
        let sandbox = top_overlay_text(&mut sandbox_app, 80);

        let mock = AppMockAgent::default();
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        app.handle_submit("/permission");
        let permission = top_overlay_text(&mut app, 80);

        // One card, both blocks, two entry states.
        for text in [&sandbox, &permission] {
            assert!(text.contains("Sandbox & permissions"), "{text}");
            assert!(text.contains("Approval mode"), "{text}");
            assert!(text.contains("Tool permissions"), "{text}");
        }
        assert_ne!(
            sandbox, permission,
            "the two commands must not open the same panel"
        );
        let marked = |text: &str| {
            text.lines()
                .find(|line| line.trim_start().starts_with('›'))
                .unwrap_or_default()
                .to_string()
        };
        assert!(marked(&sandbox).contains("Sandboxed"), "{sandbox}");
        assert!(marked(&permission).contains("All"), "{permission}");

        // The block `/permission` lands on is live: ↓ to Workspace, enter
        // applies it through the agent.
        press_on_overlay(&mut app, &mut rx, "down");
        press_on_overlay(&mut app, &mut rx, "enter");
        pump_until_msg(&mut app, &mut rx, "Tool permission level: Workspace").await;
        assert_eq!(app.state.permission_level, PermissionKind::Workspace);

        // …and the tier half is still reachable: `↑` walks back through the
        // block and then out of it into the tier list, whose rows apply a tier
        // (so `/permission` reaches everything `/sandbox` does).
        press_on_overlay(&mut app, &mut rx, "up"); // Workspace → All
        press_on_overlay(&mut app, &mut rx, "up"); // …and out of the block
        let back = top_overlay_text(&mut app, 80);
        assert!(marked(&back).contains("Unrestricted"), "{back}");
        press_on_overlay(&mut app, &mut rx, "enter");
        // Wait for the command to arrive, not for the command *channel* to go
        // quiet: the RPC runs in a spawned task, so `pump` could return during
        // the connect and the count below would read zero on a busy host (the
        // assertion is the same, it just stops racing the transport).
        pump_until_agent_commands(&mut app, &mut rx, &requests, "set_sandbox_policy", 1).await;
        assert_eq!(
            agent_command_count(&requests, "set_sandbox_policy"),
            1,
            "the tier row still writes the policy"
        );
        app.stop();
    }

    /// `/sandbox` opens the tier + permission panel with the platform's initial
    /// status, says out loud that the tier is the platform default (the agent
    /// has no policy read-back), applies a tier through the agent, and turns a
    /// silent downgrade into the panel's fallback banner.
    #[tokio::test]
    async fn sandbox_panel_warns_about_the_unknown_tier_and_shows_a_downgrade() {
        let (mut app, mut rx) = make_app(100, 30);
        app.handle_submit("/sandbox");
        assert!(top_overlay_is::<SandboxOverlay>(&app));
        let text = top_overlay_text(&mut app, 80);
        assert!(text.contains("Sandbox & permissions"), "{text}");
        assert!(text.contains("Approval mode"), "{text}");
        assert!(text.contains("Tool permissions"), "{text}");
        // macOS is sandboxed by construction, and no policy has been read.
        assert!(text.contains("current: Sandboxed"), "{text}");
        assert!(
            text.contains("Tier shown is this platform's default"),
            "the panel must not present the platform default as the session policy: {text}"
        );

        // Applying a tier goes through `set_sandbox_policy`…
        press_on_overlay(&mut app, &mut rx, "enter");
        // …and the agent's answer (a downgrade) is what the panel then shows.
        app.handle_cmd(UiCmd::SandboxPolicySet {
            result: Ok(json_parse(
                r#"{"tier":"manual","requestedTier":"sandbox","sandboxAvailable":false,"sandboxCode":"binary_missing","sandboxBackend":"none"}"#,
            )),
        });
        let text = top_overlay_text(&mut app, 80);
        assert!(text.contains("current: Manual"), "{text}");
        assert!(
            text.contains("fell back to Manual"),
            "a downgraded request must never be reported as applied: {text}"
        );
        assert!(
            !text.contains("Tier shown is this platform's default"),
            "the answer removes the caveat: {text}"
        );
        // The cached status is what a later open reuses.
        assert_eq!(
            app.sandbox.as_ref().map(|status| status.tier),
            Some(SandboxTier::Manual)
        );

        // The panel's refresh key reaches the app as a probe request (the dead
        // client reports the failure instead of inventing an answer), and the
        // app's own escape closes the panel — the app intercepts escape before
        // any component sees it.
        press_on_overlay(&mut app, &mut rx, "r");
        app.handle_key("escape");
        assert!(app.overlay_stack.is_empty());
        pump(&mut app, &mut rx).await;
        assert!(last_system(&app).contains("Sandbox probe failed"));
    }

    /// A policy write that fails is reported, and the cached status is left
    /// alone (a failed request must not look applied).
    #[tokio::test]
    async fn sandbox_policy_failure_is_reported_without_touching_the_cache() {
        let (mut app, _rx) = make_app(100, 30);
        app.handle_submit("/sandbox");
        let opening = app.sandbox.clone().expect("the panel caches its status");
        app.handle_cmd(UiCmd::SandboxPolicySet {
            result: Err("session gone".into()),
        });
        assert!(last_system(&app).contains("Failed to set the sandbox tier: session gone"));
        assert_eq!(
            app.sandbox.as_ref().map(|status| status.tier),
            Some(opening.tier),
            "a failed request must not move the cached tier"
        );
        assert!(
            app.sandbox
                .as_ref()
                .is_some_and(|status| status.requested_tier.is_none()),
            "…and must not be recorded as applied"
        );
        let text = top_overlay_text(&mut app, 80);
        assert!(
            text.contains("Tier shown is this platform's default"),
            "{text}"
        );
    }

    /// Linux reaches the sandbox through a probe RPC: the panel renders
    /// "checking" first and the diagnostic reason once the agent answers. The
    /// Windows probe is a different command (`probe_windows_sandbox`), so the
    /// two platforms are driven separately.
    #[tokio::test(flavor = "multi_thread")]
    async fn linux_and_windows_sandbox_panels_use_their_own_probe_rpcs() {
        let mock = AppMockAgent {
            overrides: std::collections::HashMap::from([(
                "probe_sandbox".to_string(),
                r#"{"available":false,"code":"binary_missing","backend":"none","path":"/usr/bin/bwrap"}"#
                    .to_string(),
            )]),
            ..Default::default()
        };
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();

        app.open_sandbox_panel(SandboxPlatform::Linux);
        let opening = top_overlay_text(&mut app, 80);
        assert!(
            opening.contains("checking the system sandbox"),
            "Linux has no answer before the probe: {opening}"
        );
        pump_until_overlay_text(&mut app, &mut rx, "diagnostic: binary_missing").await;
        let text = top_overlay_text(&mut app, 80);
        assert!(
            text.contains("unavailable — Bubblewrap is not installed"),
            "the diagnostic reason must reach the panel: {text}"
        );
        assert!(
            text.contains("Sandboxing is native only to macOS"),
            "the platform caveat is rendered: {text}"
        );

        // The Windows probe is its own command. The wait is on the *condition*,
        // with the file's shared patience rather than a fresh 5 s guess: one
        // round trip to the in-process mock, on a host that is also running the
        // rest of this suite, is not a five-second operation — and `Err(Elapsed)`
        // here would report a slow machine as a broken panel.
        app.request_sandbox_probe(SandboxPlatform::Windows);
        let sent = tokio::time::timeout(
            Duration::from_millis(PUMP_BUDGET_ITERS as u64 * PUMP_INTERVAL_MS),
            async {
                loop {
                    if requests
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|cmd| cmd.r#type == "probe_windows_sandbox")
                    {
                        break true;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            },
        )
        .await;
        assert_eq!(sent, Ok(true));
        app.stop();
    }

    /// The sandbox tier also arrives on the event stream — the only read-back
    /// the agent offers — so an open panel follows a change made elsewhere.
    #[tokio::test]
    async fn sandbox_policy_event_updates_the_cached_tier() {
        let (mut app, mut rx) = make_app(100, 30);
        app.handle_submit("/sandbox");
        app.handle_agent_event(&make_event("sandbox_policy_changed", r#"{"tier":"off"}"#));
        let text = top_overlay_text(&mut app, 80);
        assert!(text.contains("current: Unrestricted"), "{text}");
        assert!(
            !text.contains("Tier shown is this platform's default"),
            "an event-confirmed tier is not a guess: {text}"
        );
        // An event without a usable tier keeps the last known status.
        app.handle_agent_event(&make_event("sandbox_policy_changed", r#"{"tier":"junk"}"#));
        assert_eq!(
            app.sandbox.as_ref().map(|status| status.tier),
            Some(SandboxTier::Off)
        );
        pump(&mut app, &mut rx).await;
    }

    /// The caveat row is a pure function of the status: present while the tier
    /// is the platform's guess, gone once something authoritative set it.
    #[test]
    fn sandbox_tier_caveat_tracks_whether_the_tier_is_known() {
        let platform = SandboxPlatform::current();
        let fresh = SandboxStatus::new(platform);
        assert!(sandbox_tier_caveat(&fresh).is_some());

        let mut applied = fresh.clone();
        applied.requested_tier = Some(platform_default_tier(platform));
        applied.tier = platform_default_tier(platform);
        assert!(sandbox_tier_caveat(&applied).is_none());

        let other = SandboxTier::ALL
            .iter()
            .copied()
            .find(|tier| *tier != platform_default_tier(platform))
            .expect("a tier differs from the platform default");
        let mut changed = fresh;
        changed.tier = other;
        assert!(sandbox_tier_caveat(&changed).is_none());
    }

    /// `/permission <level>` applies and persists the level; a bad value is a
    /// usage error, and no argument opens the panel that owns the picker.
    #[tokio::test(flavor = "multi_thread")]
    async fn permission_command_sets_persists_and_opens_the_picker() {
        let mock = AppMockAgent::default();
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();

        app.handle_submit("/permission workspace");
        pump_until_msg(&mut app, &mut rx, "Tool permission level: Workspace").await;
        assert_eq!(app.state.permission_level, PermissionKind::Workspace);
        assert_eq!(
            app.tui_settings.default_permission_level.as_deref(),
            Some("workspace")
        );
        // Persisted, so the next launch applies it (`apply_tui_defaults`).
        let saved = std::fs::read_to_string(&app.tui_settings_path).unwrap_or_default();
        assert!(saved.contains("defaultPermissionLevel"), "{saved}");
        assert!(saved.contains("workspace"), "{saved}");

        app.handle_submit("/permission none");
        pump_until_msg(&mut app, &mut rx, "Tool permission level: None").await;
        assert_eq!(app.state.permission_level, PermissionKind::None);

        app.handle_submit("/permission  sometimes");
        assert!(last_system(&app).contains("Usage: /permission [all|workspace|none]"));
        // No argument: the panel with the permission picker opens instead.
        app.handle_submit("/permission");
        assert!(top_overlay_is::<SandboxOverlay>(&app));
        let text = top_overlay_text(&mut app, 80);
        assert!(text.contains("current: None"), "{text}");

        let sent = requests.lock().unwrap().clone();
        let levels: Vec<String> = sent
            .iter()
            .filter(|cmd| cmd.r#type == "set_permission_level")
            .map(|cmd| cmd.level.clone())
            .collect();
        assert_eq!(levels, vec!["workspace".to_string(), "none".to_string()]);
        app.stop();
    }

    /// A level picked inside the panel is applied without persisting it, the
    /// open panel follows the answer, and a failure leaves the mirror alone.
    #[tokio::test(flavor = "multi_thread")]
    async fn permission_selected_in_the_panel_applies_and_mirrors() {
        let (mut app, mut rx) = make_app(100, 30);
        app.handle_submit("/sandbox");
        app.handle_cmd(UiCmd::PermissionLevelRequested(PermissionKind::Workspace));
        // The dead client's failure must not move the mirrored level.
        pump(&mut app, &mut rx).await;
        assert!(last_system(&app).contains("Failed to set the permission level"));
        assert_eq!(app.state.permission_level, PermissionKind::All);

        app.handle_cmd(UiCmd::PermissionLevelSet {
            level: PermissionKind::None,
            result: Ok(()),
        });
        assert_eq!(app.state.permission_level, PermissionKind::None);
        assert!(
            app.tui_settings.default_permission_level.is_none(),
            "a panel pick is not the startup default"
        );
        let text = top_overlay_text(&mut app, 80);
        assert!(
            text.contains("current: None"),
            "the open panel follows the applied level: {text}"
        );

        // With no panel open the same command still mirrors the level.
        app.hide_overlay();
        app.handle_cmd(UiCmd::PermissionLevelSet {
            level: PermissionKind::All,
            result: Ok(()),
        });
        assert_eq!(app.state.permission_level, PermissionKind::All);

        // The agent's own snapshot is the other source of the mirrored level
        // (an unknown value keeps the last known one).
        let mut snapshot = sample_state();
        snapshot.permission_level = Some("workspace".into());
        app.handle_cmd(UiCmd::Refreshed(Ok(snapshot)));
        assert_eq!(app.state.permission_level, PermissionKind::Workspace);
        let mut unknown = sample_state();
        unknown.permission_level = Some("sometimes".into());
        app.handle_cmd(UiCmd::Refreshed(Ok(unknown)));
        assert_eq!(app.state.permission_level, PermissionKind::Workspace);
    }

    // ─── /skills (browser, refresh, install/uninstall/upgrade) ─────────

    /// The catalogue fills in the descriptions the local `get_state` names do
    /// not carry, `r` re-scans the skill directories, and a failed panel action
    /// is refused in the panel's own status row.
    #[tokio::test(flavor = "multi_thread")]
    async fn skills_catalogue_enriches_the_panel_and_refresh_rescans() {
        let mock = AppMockAgent {
            overrides: std::collections::HashMap::from([(
                "get_commands".to_string(),
                r#"{"commands":[{"name":"alpha","description":"does alpha things","nameZh":"阿尔法","descriptionZh":"做阿尔法的事","source":"skill"}]}"#
                    .to_string(),
            )]),
            ..Default::default()
        };
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();

        app.handle_submit("/skills");
        // `get_state` already reported one skill, so the panel is populated at
        // once — the *description* is what the catalogue RPC adds.
        let opening = top_overlay_text(&mut app, 80);
        assert!(
            opening.contains("alpha"),
            "the panel opens with the names the session already knew: {opening}"
        );
        // The row carries both languages; the panel shows the Chinese one
        // because the skill has it, while `enter` still inserts the canonical
        // English name a prompt has to reference.
        pump_until_overlay_text(&mut app, &mut rx, "做阿尔法的事").await;
        let text = top_overlay_text(&mut app, 80);
        assert!(text.contains("阿尔法"), "{text}");

        // `r` re-scans, then re-reads the catalogue (refresh_skills alone does
        // not carry the rows).
        press_on_overlay(&mut app, &mut rx, "r");
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                // The refresh answer arrives on the command channel: it is
                // what triggers the follow-up `get_commands`.
                while let Ok(cmd) = rx.try_recv() {
                    app.handle_cmd(cmd);
                }
                let seen = requests.lock().unwrap().clone();
                let refreshes = seen.iter().filter(|c| c.r#type == "refresh_skills").count();
                let loads = seen.iter().filter(|c| c.r#type == "get_commands").count();
                if refreshes >= 1 && loads >= 2 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("refresh_skills and a second get_commands must reach the agent");
        // The panel is still open and still shows the row.
        assert!(top_overlay_is::<SkillsOverlay>(&app));

        // The panel's preview key opens the pager over the panel.
        press_on_overlay(&mut app, &mut rx, "ctrl+o");
        assert!(top_overlay_is::<PagerOverlay>(&app));
        let preview = top_overlay_text(&mut app, 80);
        assert!(preview.contains("阿尔法"), "{preview}");
        press_on_overlay(&mut app, &mut rx, "escape");
        assert!(
            top_overlay_is::<SkillsOverlay>(&app),
            "the panel is still open"
        );

        // `enter` inserts the canonical (English) skill name — the one a
        // prompt has to reference — and closes the panel.
        press_on_overlay(&mut app, &mut rx, "enter");
        assert_eq!(app.input.get_value(), "alpha");
        assert!(last_system(&app).contains("Inserted skill: alpha"));

        // A failed refresh and a failed catalogue load are reported.
        app.handle_cmd(UiCmd::SkillsRefreshed(Err("busy".into())));
        assert!(last_system(&app).contains("Failed to refresh skills: busy"));
        app.handle_cmd(UiCmd::SkillsLoaded(Err("busy".into())));
        assert!(last_system(&app).contains("Failed to load the skill catalogue: busy"));
        // …and an answer that arrives after the panel closed is ignored.
        assert!(app.overlay_stack.is_empty());
        app.handle_cmd(UiCmd::SkillsLoaded(Ok(json_parse(r#"{"commands":[]}"#))));
        assert!(app.overlay_stack.is_empty());
        // A successful refresh asks for the catalogue again.
        app.handle_cmd(UiCmd::SkillsRefreshed(Ok(json_parse("{}"))));
        app.stop();
    }

    /// The panel says it is waiting for the installable-skills catalogue, and
    /// stops saying it when the answer lands.
    ///
    /// The child is gated on a channel, so the in-flight state is observable
    /// without racing an instant answer (and bounded, so a failure cannot hang).
    #[tokio::test(flavor = "multi_thread")]
    async fn skills_panel_says_it_is_loading_the_catalogue() {
        let (mut app, mut rx, _requests) =
            app_with_live_agent(std::collections::HashMap::new()).await;
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let gate = Arc::new(std::sync::Mutex::new(release_rx));
        let waiting = Arc::clone(&gate);
        app.skills_cli = Some(fake_skills_cli(
            &Arc::new(Mutex::new(Vec::new())),
            move |_args| {
                let _ = waiting
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10));
                Ok((0, SKILLS_CATALOGUE_JSON.to_string(), String::new()))
            },
        ));

        app.handle_submit("/skills");
        let pending = top_overlay_text(&mut app, 80);
        assert!(pending.contains(SKILLS_LOADING), "{pending}");
        // The row comes out of the panel's budget, never on top of it: an
        // overlay taller than its `max_height` gets clipped, and the row that
        // would be clipped is this one.
        assert!(pending.lines().count() <= 28, "{}", pending.lines().count());

        let _ = release_tx.send(());
        pump_until_overlay_text(&mut app, &mut rx, "v1.2.0").await;
        let settled = top_overlay_text(&mut app, 80);
        assert!(!settled.contains(SKILLS_LOADING), "{settled}");
        assert!(settled.contains("v1.2.0"), "{settled}");
        app.stop();
    }

    /// A catalogue call that hit its short timeout is retryable, and the panel
    /// that asked for it says so; any other failure keeps its own reason and no
    /// key advice. The transcript is deliberately silent while that panel is on
    /// screen (see the sibling test below for the no-panel case).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_timed_out_catalogue_is_reported_as_retryable() {
        let (mut app, mut rx, _requests) =
            app_with_live_agent(std::collections::HashMap::new()).await;
        // The runner answers `skills list` the way a killed child does: the
        // marker `wait_for` writes plus the short budget `spawn_runner` gave it.
        let timeout = format!(
            "failed to run `future skills list --json`: {} after {}s",
            crate::skills_cli::TIMEOUT_MARKER,
            crate::skills_cli::SKILL_LIST_TIMEOUT.as_secs()
        );
        app.skills_cli = Some(fake_skills_cli(
            &Arc::new(Mutex::new(Vec::new())),
            move |_args| Err(timeout.clone()),
        ));
        app.handle_submit("/skills");
        // The panel carries this now, so wait on the PANEL row rather than the
        // transcript: the row is what holds the retry advice, and the rendered
        // panel cuts it off at ~78 columns.
        pump_until_skills_row(&mut app, &mut rx, "press r to retry").await;
        let row = skills_row(&mut app).expect("a failed catalogue fills the status row");
        assert!(row.contains("press r to retry"), "{row}");
        assert!(row.contains("timed out after 15s"), "{row}");
        // ONE report, not two: the panel is on screen and carries the retry
        // hint next to the list it is about. Repeating it as a system message
        // made the same problem look like two failures — round-3 acceptance
        // read that pair as "the skills are broken".
        // Hoisted into a binding: `system_messages` as a lazy assert argument on
        // its own line would never execute on the passing path, so the line
        // would sit at DA:0 forever — the taxonomy (e) trap this branch keeps
        // hitting (see COVERAGE-CONTRACT.md).
        let messages = system_messages(&app);
        assert!(
            !messages
                .iter()
                .any(|line| line.contains("press r to retry")),
            "the open panel already reports this; the transcript must not repeat it: {messages:?}"
        );

        // A failure that is not a timeout is reported as what it is: the cause
        // is already on the line, and no key would fix it.
        app.handle_cmd(UiCmd::SkillsCatalogueLoaded(Err("no such file".into())));
        let row = skills_row(&mut app).expect("the row is reused");
        assert!(row.contains("no such file"), "{row}");
        assert!(!row.contains("press r to retry"), "{row}");
        app.stop();
    }

    /// The other half of the one-report rule: with no skills panel on screen —
    /// the load outlived it, or the user pressed `r` and moved on — the
    /// transcript is the only place the failure can surface, so it is used.
    /// Dropping it there would hide the failure entirely.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_catalogue_failure_with_no_panel_is_reported_in_the_transcript() {
        let (mut app, mut rx, _requests) =
            app_with_live_agent(std::collections::HashMap::new()).await;
        app.skills_cli = Some(fake_skills_cli(
            &Arc::new(Mutex::new(Vec::new())),
            move |_args| Err("failed to run `future skills list --json`: no such file".into()),
        ));
        // `/skills` is never opened, so no panel can carry the failure.
        app.load_cli_catalogue();
        pump_until_msg(&mut app, &mut rx, "no such file").await;
        let messages = system_messages(&app);
        assert!(
            messages.iter().any(|line| line.contains("no such file")),
            "the transcript is the only reporter here: {messages:?}"
        );
        assert!(skills_row(&mut app).is_none(), "no panel, so no status row");
        app.stop();
    }

    /// `i` runs `future skills install <id> --version <latest>` on the blocking
    /// pool, marks the row in flight while it runs, and — because the agent
    /// caches the skills it discovered — re-scans only *after* the child
    /// succeeded, so the panel's two sources are re-read from the re-scan's
    /// answer rather than from the key press.
    #[tokio::test(flavor = "multi_thread")]
    async fn skill_install_runs_the_cli_and_rescans_the_agent() {
        let (mut app, mut rx, requests) = app_with_live_agent(std::collections::HashMap::from([(
            "get_commands".to_string(),
            r#"{"commands":[{"name":"alpha","description":"does alpha things","source":"skill"}]}"#
                .to_string(),
        )]))
        .await;

        // The fake blocks on this gate so the in-flight state is observable
        // without racing an instant answer (bounded, so a failure cannot hang).
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let gate = Arc::new(std::sync::Mutex::new(release_rx));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let waiting = Arc::clone(&gate);
        app.skills_cli = Some(fake_skills_cli(&calls, move |args| {
            if args.get(1).map(String::as_str) == Some("list") {
                return Ok((0, SKILLS_CATALOGUE_JSON.to_string(), String::new()));
            }
            let _ = waiting
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10));
            Ok((0, "installed alpha".to_string(), String::new()))
        }));

        app.handle_submit("/skills");
        // get_commands reports alpha as installed; the catalogue knows 1.0.0
        // locally against a 1.2.0 release, so the row is an upgrade.
        pump_until_overlay_text(&mut app, &mut rx, "does alpha things").await;
        pump_until_overlay_text(&mut app, &mut rx, "1.2.0").await;
        let text = top_overlay_text(&mut app, 90);
        assert!(
            text.contains("does alpha things") && text.contains("1.2.0"),
            "the panel merges both sources: {text}"
        );

        press_on_overlay(&mut app, &mut rx, "i");
        pump_until_skill_call(&mut app, &mut rx, &calls, |args| skill_op_count(args) >= 1).await;
        let text = top_overlay_text(&mut app, 90);
        assert!(
            text.contains("working…"),
            "the row being installed is marked in flight: {text}"
        );
        assert!(last_system(&app).contains("Installing skill alpha v1.2.0"));

        release_tx.send(()).expect("the fake runner is waiting");
        pump_until_msg(&mut app, &mut rx, "install alpha@1.2.0 succeeded").await;
        // The re-scan is what makes the new skill visible to the agent's own
        // cache — and its *answer* is what re-pulls both sources: the agent's
        // commands and the CLI's catalogue.
        pump_until_agent_commands(&mut app, &mut rx, &requests, "refresh_skills", 1).await;
        pump_until_agent_commands(&mut app, &mut rx, &requests, "get_commands", 2).await;
        pump_until_skill_call(&mut app, &mut rx, &calls, |args| {
            skill_list_count(args) >= 2
        })
        .await;

        assert!(
            !top_overlay_text(&mut app, 90).contains("working…"),
            "the in-flight mark is cleared once the child reported"
        );
        assert_eq!(skills_row(&mut app), None, "a success leaves no error row");
        assert!(!app.skills_op_running, "the operation is no longer running");
        // The command line is what the CLI documents: the id plus the version
        // the catalogue reported, never a bare name.
        let list = crate::skills_cli::list_args();
        let install = vec![
            "skills".to_string(),
            "install".to_string(),
            "alpha".to_string(),
            "--version".to_string(),
            "1.2.0".to_string(),
        ];
        let args = skill_args(&calls);
        assert_eq!(args[0], list, "the panel opens by reading the catalogue");
        assert!(args.contains(&install), "the install argv: {args:?}");
        assert!(
            skill_list_count(&args) >= 2,
            "the re-scan re-reads the catalogue too: {args:?}"
        );
        app.stop();
    }

    /// A non-zero exit is reported with the CLI's own stderr excerpt, the row is
    /// released, and nothing is re-pulled: a failed child leaves the installed
    /// set exactly as it was.
    #[tokio::test(flavor = "multi_thread")]
    async fn skill_op_failure_is_reported_and_does_not_rescan() {
        let (mut app, mut rx, requests) = app_with_live_agent(std::collections::HashMap::from([(
            "get_commands".to_string(),
            r#"{"commands":[{"name":"alpha","description":"does alpha things","source":"skill"}]}"#
                .to_string(),
        )]))
        .await;

        let calls = Arc::new(Mutex::new(Vec::new()));
        app.skills_cli = Some(fake_skills_cli(&calls, |args| {
            if args.get(1).map(String::as_str) == Some("list") {
                return Ok((0, SKILLS_CATALOGUE_JSON.to_string(), String::new()));
            }
            Ok((
                1,
                String::new(),
                "  permission denied:\n  /home/u/.future/agent/skills/alpha ".to_string(),
            ))
        }));

        app.handle_submit("/skills");
        // The two initial fetches have to land before the keys can act on a
        // row: alpha only exists once the catalogue answered.
        pump_until_overlay_text(&mut app, &mut rx, "v1.2.0").await;
        // `u` twice: the panel's own uninstall confirmation, then the run.
        press_on_overlay(&mut app, &mut rx, "u");
        press_on_overlay(&mut app, &mut rx, "u");
        pump_until_overlay_text(&mut app, &mut rx, "permission denied").await;

        let row = skills_row(&mut app).expect("a failure fills the status row");
        assert!(
            row.contains("uninstall alpha failed (exit code 1)"),
            "the summary names the operation and its exit code: {row}"
        );
        assert!(
            row.contains("permission denied: /home/u/.future/agent/skills/alpha"),
            "the child's stderr is flattened into the row: {row}"
        );
        assert!(
            !top_overlay_text(&mut app, 120).contains("working…"),
            "the row is released"
        );
        assert!(last_system(&app).contains("uninstall alpha failed"));
        let args = skill_args(&calls);
        assert!(
            args.contains(&vec![
                "skills".to_string(),
                "uninstall".to_string(),
                "alpha".to_string()
            ]),
            "the uninstall argv is the id alone: {args:?}"
        );
        // A failure is not a reason to re-read anything.
        assert_eq!(agent_command_count(&requests, "refresh_skills"), 0);
        assert!(!app.skills_op_running);
        app.stop();
    }

    /// A child that had to be killed gets its own sentence: `summarize_outcome`
    /// says which operation timed out, and the app names the deadline that
    /// killed it.
    #[tokio::test(flavor = "multi_thread")]
    async fn skill_op_timeout_says_it_was_killed() {
        let (mut app, mut rx, _requests) =
            app_with_live_agent(std::collections::HashMap::new()).await;
        app.handle_submit("/skills");
        // The panel's other source — the `future skills list --json` child —
        // fails asynchronously (no `future` binary here), and it writes the very
        // rows this test reads. Wait for it *before* injecting the runner and
        // pressing `i`, otherwise it can land after the operation's own row and
        // overwrite it (the two writers are not ordered). The panel is open, so
        // this failure lands in the panel's status row, not the transcript.
        pump_until_skills_row(&mut app, &mut rx, "Failed to list installable skills").await;
        let timeout = format!("{} after 120s", crate::skills_cli::TIMEOUT_MARKER);
        app.skills_cli = Some(fake_skills_cli(
            &Arc::new(Mutex::new(Vec::new())),
            move |_args| Err(timeout.clone()),
        ));
        app.handle_cmd(UiCmd::SkillsMutationRequested(SkillsMutation::Install(
            "alpha".to_string(),
        )));
        pump_until_msg(&mut app, &mut rx, "install alpha timed out").await;
        let row = skills_row(&mut app).expect("a killed child fills the status row");
        assert!(
            row.contains("install alpha timed out (killed after 120s)"),
            "a killed child is named as such: {row}"
        );
        assert!(!app.skills_op_running);
        app.stop();
    }

    /// An id the CLI refuses is rejected before any child is spawned, and the
    /// panel says why instead of showing a failure that never ran.
    #[tokio::test(flavor = "multi_thread")]
    async fn skill_op_refuses_an_invalid_id_without_spawning() {
        let (mut app, mut rx, requests) =
            app_with_live_agent(std::collections::HashMap::new()).await;
        let calls = Arc::new(Mutex::new(Vec::new()));
        app.skills_cli = Some(fake_skills_cli(&calls, |_args| {
            Ok((0, SKILLS_CATALOGUE_JSON.to_string(), String::new()))
        }));
        app.handle_submit("/skills");
        app.handle_cmd(UiCmd::SkillsMutationRequested(SkillsMutation::Uninstall(
            "../escape".to_string(),
        )));
        pump_until_msg(&mut app, &mut rx, "invalid skill id").await;
        let ops = skill_op_args(&calls);
        assert!(
            ops.is_empty(),
            "a rejected id must not reach the process runner: {ops:?}"
        );
        let row = skills_row(&mut app).expect("the rejection fills the status row");
        assert!(
            row.contains("uninstall ../escape failed: invalid skill id"),
            "the rejection reason is the CLI's own: {row}"
        );
        assert_eq!(agent_command_count(&requests, "refresh_skills"), 0);
        assert!(!app.skills_op_running);
        app.stop();
    }

    /// `U` upgrades everything at once, so the first press only names the set;
    /// the second one runs it. A catalogue that moved between the two presses
    /// asks for a fresh confirmation instead of running.
    #[tokio::test(flavor = "multi_thread")]
    async fn upgrade_all_needs_a_second_press_and_names_the_skills() {
        let (mut app, mut rx, _requests) =
            app_with_live_agent(std::collections::HashMap::new()).await;
        let calls = Arc::new(Mutex::new(Vec::new()));
        app.skills_cli = Some(fake_skills_cli(&calls, |args| {
            if args.get(1).map(String::as_str) == Some("list") {
                return Ok((0, SKILLS_CATALOGUE_JSON.to_string(), String::new()));
            }
            Ok((0, "updated".to_string(), String::new()))
        }));
        app.handle_submit("/skills");
        // Let the panel's own fetches land first: they are what the injected
        // rows below are stacked on top of.
        pump(&mut app, &mut rx).await;
        app.handle_cmd(UiCmd::SkillsLoaded(Ok(json_parse(
            r#"{"commands":[{"name":"alpha","description":"a","source":"skill"},{"name":"beta","description":"b","source":"skill"}]}"#,
        ))));
        app.handle_cmd(UiCmd::SkillsCatalogueLoaded(Ok(skills_catalogue_fixture())));

        press_on_overlay(&mut app, &mut rx, "U");
        let row = skills_row(&mut app).expect("the first press asks for confirmation");
        assert!(
            row.contains("This will upgrade 1 installed skill (alpha)"),
            "the prompt names every skill it would upgrade: {row}"
        );
        assert!(row.contains("Press U again to confirm"));
        let ops = skill_op_args(&calls);
        assert!(ops.is_empty(), "the first press runs nothing: {ops:?}");
        assert_eq!(
            app.skills_upgrade_armed,
            Some(vec!["alpha".to_string()]),
            "the arm carries the set the prompt listed"
        );

        press_on_overlay(&mut app, &mut rx, "U");
        pump_until_skill_call(&mut app, &mut rx, &calls, |args| skill_op_count(args) >= 1).await;
        let args = skill_args(&calls);
        assert!(
            args.contains(&vec!["skills".to_string(), "update".to_string()]),
            "the confirmed press runs the update: {args:?}"
        );
        assert!(app.skills_upgrade_armed.is_none(), "the arm is spent");
        pump_until_msg(&mut app, &mut rx, "update all skills succeeded").await;
        app.stop();
    }

    /// The arm is compared against the *current* upgradable set: a catalogue
    /// that moved under the confirmation re-arms rather than upgrading
    /// something the prompt never listed.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stale_upgrade_confirmation_is_not_reused() {
        let (mut app, mut rx, _requests) =
            app_with_live_agent(std::collections::HashMap::new()).await;
        let calls = Arc::new(Mutex::new(Vec::new()));
        app.skills_cli = Some(fake_skills_cli(&calls, |_args| {
            Ok((0, SKILLS_CATALOGUE_JSON.to_string(), String::new()))
        }));
        app.handle_submit("/skills");
        pump(&mut app, &mut rx).await;
        app.handle_cmd(UiCmd::SkillsLoaded(Ok(json_parse(
            r#"{"commands":[{"name":"alpha","description":"a","source":"skill"},{"name":"beta","description":"b","source":"skill"}]}"#,
        ))));
        app.handle_cmd(UiCmd::SkillsCatalogueLoaded(Ok(skills_catalogue_fixture())));
        press_on_overlay(&mut app, &mut rx, "U");
        assert_eq!(app.skills_upgrade_armed, Some(vec!["alpha".to_string()]));

        // The catalogue moved: alpha is current now, beta is behind. The next
        // `U` must not run the upgrade the old prompt asked about.
        let moved = crate::skills_cli::parse_catalogue(
            r#"{"skills":[{"id":"alpha","latestVersion":"1.2.0","installedVersion":"1.2.0"},
                         {"id":"beta","latestVersion":"3.0.0","installedVersion":"2.0.0"}]}"#,
        )
        .expect("the second fixture parses");
        app.handle_cmd(UiCmd::SkillsCatalogueLoaded(Ok(moved)));
        let before = skill_op_args(&calls).len();
        press_on_overlay(&mut app, &mut rx, "U");
        let row = skills_row(&mut app).expect("the stale arm asks again");
        assert!(
            row.contains("This will upgrade 1 installed skill (beta)"),
            "the prompt names the *new* set: {row}"
        );
        assert_eq!(
            skill_op_args(&calls).len(),
            before,
            "a stale confirmation spawns nothing"
        );
        assert_eq!(app.skills_upgrade_armed, Some(vec!["beta".to_string()]));
    }

    /// A mutation request can outlive the panel it came from: the operation
    /// still runs (the id is self-contained) and its result goes to the
    /// transcript, because there is no status row left to write it into.
    #[tokio::test(flavor = "multi_thread")]
    async fn skill_op_without_a_panel_reports_to_the_transcript() {
        let (mut app, mut rx, _requests) =
            app_with_live_agent(std::collections::HashMap::new()).await;
        let calls = Arc::new(Mutex::new(Vec::new()));
        app.skills_cli = Some(fake_skills_cli(&calls, |args| {
            if args.get(1).map(String::as_str) == Some("list") {
                return Ok((0, SKILLS_CATALOGUE_JSON.to_string(), String::new()));
            }
            Ok((0, "removed".to_string(), String::new()))
        }));
        assert!(app.overlay_stack.is_empty(), "no panel is open");
        app.handle_cmd(UiCmd::SkillsMutationRequested(SkillsMutation::Uninstall(
            "alpha".to_string(),
        )));
        pump_until_msg(&mut app, &mut rx, "uninstall alpha succeeded").await;
        // The success path re-reads the catalogue even with no panel to show
        // it in: the app's cache is what the next install pins.
        pump_until_skill_call(&mut app, &mut rx, &calls, |args| {
            skill_list_count(args) >= 1
        })
        .await;
        let ops = skill_op_args(&calls);
        assert!(
            ops.contains(&vec![
                "skills".to_string(),
                "uninstall".to_string(),
                "alpha".to_string()
            ]),
            "the operation runs without the panel: {ops:?}"
        );
        assert!(app.overlay_stack.is_empty(), "and it opens nothing");
        app.stop();
    }

    /// `U` with nothing to upgrade says so — the panel refuses the key itself,
    /// but a mutation that arrives without the confirmation cannot run either.
    #[tokio::test(flavor = "multi_thread")]
    async fn upgrade_all_with_nothing_to_upgrade_says_so() {
        let (mut app, _rx, _requests) = app_with_live_agent(std::collections::HashMap::new()).await;
        let calls = Arc::new(Mutex::new(Vec::new()));
        app.skills_cli = Some(fake_skills_cli(&calls, |_args| {
            Ok((0, String::new(), String::new()))
        }));
        app.handle_submit("/skills");
        app.handle_cmd(UiCmd::SkillsMutationRequested(SkillsMutation::UpgradeAll));
        let row = skills_row(&mut app).expect("the refusal explains itself");
        assert_eq!(row, "No installed skill has a newer version to upgrade.");
        assert!(skill_op_args(&calls).is_empty());
        assert!(app.skills_upgrade_armed.is_none());

        // With the panel gone there is no set to confirm against at all: the
        // same sentence goes to the transcript instead of guessing.
        app.hide_overlay();
        app.handle_cmd(UiCmd::SkillsMutationRequested(SkillsMutation::UpgradeAll));
        assert_eq!(
            last_system(&app),
            "No installed skill has a newer version to upgrade."
        );
        assert!(skill_op_args(&calls).is_empty());
    }

    /// One child at a time: a second mutation while one is running is refused
    /// out loud rather than starting a second `future skills …` over the same
    /// package directory.
    #[tokio::test(flavor = "multi_thread")]
    async fn skill_op_refuses_a_second_one_while_running() {
        let (mut app, _rx, _requests) = app_with_live_agent(std::collections::HashMap::new()).await;
        let calls = Arc::new(Mutex::new(Vec::new()));
        app.skills_cli = Some(fake_skills_cli(&calls, |_args| {
            Ok((0, String::new(), String::new()))
        }));
        app.handle_submit("/skills");
        app.skills_op_running = true;
        app.handle_cmd(UiCmd::SkillsMutationRequested(SkillsMutation::Install(
            "alpha".to_string(),
        )));
        let row = skills_row(&mut app).expect("the refusal explains itself");
        assert_eq!(
            row,
            "A skill operation is already running — wait for it to finish."
        );
        let ops = skill_op_args(&calls);
        assert!(ops.is_empty(), "no second child is started: {ops:?}");
        assert!(!last_system(&app).contains("Installing skill alpha"));
    }

    /// With no `future` executable to run, the keys and the catalogue fetch say
    /// so — in the panel while it is open, in the transcript otherwise. This is
    /// the standalone `future-tui` install, where a bare-name spawn would fail
    /// with an OS error nobody can act on.
    #[tokio::test(flavor = "multi_thread")]
    async fn skills_without_a_future_binary_say_so() {
        let (mut app, mut rx, _requests) =
            app_with_live_agent(std::collections::HashMap::new()).await;
        app.skills_cli = None;
        app.handle_submit("/skills");
        pump(&mut app, &mut rx).await;
        let row = skills_row(&mut app).expect("opening the panel reports the missing binary");
        assert_eq!(
            row,
            "The `future` executable was not found — installing or removing skills is unavailable.",
            "the panel names the reason instead of failing later"
        );

        // The install key reaches the same sentence, and nothing is started.
        app.handle_cmd(UiCmd::SkillsMutationRequested(SkillsMutation::Install(
            "alpha".to_string(),
        )));
        assert_eq!(skills_row(&mut app).as_deref(), Some(row.as_str()));
        assert!(!app.skills_op_running, "no operation was started");

        // With no panel open the same notice goes to the transcript, so a
        // refresh that outlived its panel is still explained.
        app.hide_overlay();
        app.load_cli_catalogue();
        assert!(last_system(&app).contains("The `future` executable was not found"));
        app.stop();
    }

    /// The test build never hands an app the host's `future` (see
    /// [`skills_cli_for_host`]): a test that reaches a skill operation without
    /// injecting its own fake reports the spawn attempt instead of performing
    /// it, which is what keeps this suite from running a real installer on a
    /// machine that happens to have one on `PATH`.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_test_build_never_resolves_a_real_future_binary() {
        let (mut app, mut rx, _requests) =
            app_with_live_agent(std::collections::HashMap::new()).await;
        assert!(
            app.skills_cli.is_some(),
            "the app is offered an installer, just not a real one"
        );
        app.handle_submit("/skills");
        app.handle_cmd(UiCmd::SkillsMutationRequested(SkillsMutation::Install(
            "alpha".to_string(),
        )));
        pump_until_msg(
            &mut app,
            &mut rx,
            "install alpha failed: a test tried to spawn",
        )
        .await;
        // The panel's own catalogue fetch goes through the same runner, so the
        // op's line is not necessarily the last one: assert over all of them.
        let messages = system_messages(&app);
        assert!(
            messages
                .iter()
                .any(|message| message.starts_with("install alpha failed: a test tried to spawn")),
            "the fake runner names the command it refused: {messages:?}"
        );
        assert!(!app.skills_op_running);
        app.stop();
    }

    /// Either source may fail without taking the other down: the panel keeps
    /// listing what it has and names the fetch it could not reach.
    #[tokio::test]
    async fn a_failed_skill_source_is_named_in_the_panel() {
        let (mut app, _rx) = make_app(100, 30);
        app.handle_submit("/skills");
        app.handle_cmd(UiCmd::SkillsCatalogueLoaded(Ok(skills_catalogue_fixture())));
        app.handle_cmd(UiCmd::SkillsLoaded(Err("agent busy".to_string())));
        assert_eq!(
            skills_row(&mut app).as_deref(),
            Some("Failed to load the skill catalogue: agent busy"),
            "the agent-side failure is visible where the rows are"
        );
        let text = top_overlay_text(&mut app, 120);
        assert!(
            text.contains("alpha"),
            "the catalogue rows survive the other fetch failing: {text}"
        );
        app.handle_cmd(UiCmd::SkillsCatalogueLoaded(
            Err("no such file".to_string()),
        ));
        assert_eq!(
            skills_row(&mut app).as_deref(),
            Some("Failed to list installable skills: no such file"),
            "the CLI-side failure replaces the row"
        );
        // The panel is open, so it is the one place this is reported (see
        // `a_catalogue_failure_with_no_panel_is_reported_in_the_transcript`
        // for the no-panel half of the rule).
        assert!(
            !system_messages(&app)
                .iter()
                .any(|line| line.contains("Failed to list installable skills")),
            "the open panel carries it; the transcript must not repeat it: {:?}",
            system_messages(&app)
        );
    }

    /// The mutation helpers, one by one: what the panel marks in flight, what
    /// the transcript says was started, and which set `U` confirms against.
    #[test]
    fn skill_op_texts_and_targets() {
        let install = SkillOp::Install {
            id: "alpha".to_string(),
            version: Some("1.2.0".to_string()),
        };
        let unversioned = SkillOp::Install {
            id: "alpha".to_string(),
            version: None,
        };
        let uninstall = SkillOp::Uninstall {
            id: "alpha".to_string(),
        };
        assert_eq!(skill_op_pending_id(&install).as_deref(), Some("alpha"));
        assert_eq!(skill_op_pending_id(&uninstall).as_deref(), Some("alpha"));
        assert!(skill_op_pending_id(&SkillOp::UpdateAll).is_none());
        assert_eq!(
            skill_op_start_message(&install),
            "Installing skill alpha v1.2.0…"
        );
        assert_eq!(
            skill_op_start_message(&unversioned),
            "Installing skill alpha…"
        );
        assert_eq!(skill_op_start_message(&uninstall), "Removing skill alpha…");
        assert_eq!(
            skill_op_start_message(&SkillOp::UpdateAll),
            "Upgrading every installed skill…"
        );
        assert_eq!(
            upgrade_confirmation(&["alpha".to_string()]),
            "This will upgrade 1 installed skill (alpha). Press U again to confirm."
        );
        assert_eq!(
            upgrade_confirmation(&["alpha".to_string(), "beta".to_string()]),
            "This will upgrade 2 installed skills (alpha, beta). Press U again to confirm."
        );
    }

    /// A successful operation clears the panel's error row; every kind of
    /// failure fills it, and a killed child is named as killed.
    #[test]
    fn skill_op_error_row_reflects_the_outcome() {
        let op = SkillOp::Uninstall {
            id: "alpha".to_string(),
        };
        let ok = SkillOpOutcome {
            op: op.clone(),
            ok: true,
            exit_code: Some(0),
            stdout: "removed".to_string(),
            stderr: String::new(),
            timed_out: false,
        };
        assert!(skill_op_error(&ok).is_none(), "success clears the row");
        assert_eq!(describe_outcome(&ok), "uninstall alpha succeeded");

        let failed = SkillOpOutcome {
            ok: false,
            exit_code: Some(2),
            stderr: "disk full".to_string(),
            ..ok.clone()
        };
        assert_eq!(
            skill_op_error(&failed).as_deref(),
            Some("uninstall alpha failed (exit code 2): disk full")
        );

        let killed = SkillOpOutcome {
            ok: false,
            exit_code: None,
            timed_out: true,
            stderr: "future skills: timed out after 120s".to_string(),
            ..ok
        };
        assert_eq!(
            describe_outcome(&killed),
            "uninstall alpha timed out (killed after 120s)"
        );
    }

    /// The catalogue answer is what an install pins, and an id it does not know
    /// (or no answer at all) installs whatever the platform serves as current.
    #[tokio::test]
    async fn install_version_comes_from_the_last_catalogue() {
        let (mut app, _rx) = make_app(100, 30);
        assert_eq!(app.install_version_for("alpha"), None);
        app.handle_cmd(UiCmd::SkillsCatalogueLoaded(Ok(skills_catalogue_fixture())));
        assert_eq!(app.install_version_for("alpha").as_deref(), Some("1.2.0"));
        assert_eq!(app.install_version_for("gamma"), None);
        assert_eq!(app.install_version_for("beta").as_deref(), Some("2.0.0"));
    }

    /// The missing-binary test is the bare fallback name, not a path: a located
    /// `future` (`future tui`'s sibling, or one off `PATH`) is never "missing".
    #[test]
    fn only_the_bare_program_name_counts_as_missing() {
        assert!(skills_binary_missing(std::path::Path::new("future")));
        assert!(!skills_binary_missing(std::path::Path::new(
            "/usr/local/bin/future"
        )));
        assert!(!skills_binary_missing(std::path::Path::new("future-tui")));
    }

    /// Both new panels translate every action they can emit — value-carrying
    /// ones into a `UiCmd`, the redraw-only ones into silence — and the skills
    /// mutations into the explicit refusal (never into a spawn).
    #[test]
    fn panel_sinks_translate_every_action() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut sandbox_sink = App::<FakeTerminal>::sandbox_sink(tx.clone());
        sandbox_sink(SandboxAction::None);
        sandbox_sink(SandboxAction::Moved);
        assert!(rx.try_recv().is_err(), "redraw-only actions stay silent");
        sandbox_sink(SandboxAction::TierSelected(SandboxTier::Off));
        assert!(matches!(
            rx.try_recv(),
            Ok(UiCmd::SandboxTierRequested(SandboxTier::Off))
        ));
        sandbox_sink(SandboxAction::PermissionSelected(PermissionKind::None));
        assert!(matches!(
            rx.try_recv(),
            Ok(UiCmd::PermissionLevelRequested(PermissionKind::None))
        ));
        sandbox_sink(SandboxAction::RefreshProbe);
        assert!(matches!(rx.try_recv(), Ok(UiCmd::SandboxProbeRequested)));
        sandbox_sink(SandboxAction::Cancelled);
        assert!(matches!(rx.try_recv(), Ok(UiCmd::OverlayCancel)));

        let mut skills_sink = App::<FakeTerminal>::skills_sink(tx);
        for silent in [
            SkillsAction::None,
            SkillsAction::Moved,
            SkillsAction::TabChanged,
        ] {
            skills_sink(silent);
        }
        assert!(rx.try_recv().is_err(), "redraw-only actions stay silent");
        skills_sink(SkillsAction::Use("alpha".into()));
        assert!(matches!(rx.try_recv(), Ok(UiCmd::SkillChosen(name)) if name == "alpha"));
        skills_sink(SkillsAction::Detail);
        assert!(matches!(rx.try_recv(), Ok(UiCmd::SkillsDetailRequested)));
        skills_sink(SkillsAction::Refresh);
        assert!(matches!(rx.try_recv(), Ok(UiCmd::SkillsRefreshRequested)));
        skills_sink(SkillsAction::Cancelled);
        assert!(matches!(rx.try_recv(), Ok(UiCmd::OverlayCancel)));
        // Every mutation the panel can emit reaches the app as the operation
        // to run — `i`/`u` with the id they were aimed at, `U` with nothing
        // (the app resolves the upgrade set from the panel it confirms with).
        skills_sink(SkillsAction::Install("alpha".into()));
        assert!(matches!(
            rx.try_recv(),
            Ok(UiCmd::SkillsMutationRequested(SkillsMutation::Install(id))) if id == "alpha"
        ));
        skills_sink(SkillsAction::Uninstall("alpha".into()));
        assert!(matches!(
            rx.try_recv(),
            Ok(UiCmd::SkillsMutationRequested(SkillsMutation::Uninstall(id))) if id == "alpha"
        ));
        skills_sink(SkillsAction::UpgradeAll);
        assert!(matches!(
            rx.try_recv(),
            Ok(UiCmd::SkillsMutationRequested(SkillsMutation::UpgradeAll))
        ));
    }

    /// `/skills` `ctrl+o` with nothing highlighted is a message, not an empty
    /// pager.
    #[tokio::test]
    async fn skills_detail_without_a_panel_or_a_row_says_so() {
        let (mut app, _rx) = make_app(100, 30);
        app.handle_cmd(UiCmd::SkillsDetailRequested);
        assert_eq!(last_system(&app), "No skill is highlighted.");
    }

    /// The names `get_state` reports are enough to browse and to insert; the
    /// descriptions arrive with the catalogue.
    #[test]
    fn local_skill_rows_carry_the_name_as_id_and_source() {
        let rows = local_skill_rows(&["alpha".to_string(), "beta".to_string()]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "alpha");
        assert_eq!(rows[0].name, "alpha");
        assert_eq!(rows[0].source, "skill");
        assert!(rows[0].description.is_empty());
        assert!(local_skill_rows(&[]).is_empty());
    }

    /// The bilingual surfaces follow the host locale — the TUI has no language
    /// setting of its own.
    #[test]
    fn prefers_chinese_reads_the_locale_environment() {
        let _guard = crate::test_env::lock();
        let keys = ["LC_ALL", "LC_MESSAGES", "LANG"];
        let saved: Vec<Option<std::ffi::OsString>> = keys.iter().map(std::env::var_os).collect();
        for key in keys {
            std::env::remove_var(key);
        }
        assert!(!prefers_chinese());
        std::env::set_var("LANG", "zh_CN.UTF-8");
        assert!(prefers_chinese());
        std::env::set_var("LANG", "en_US.UTF-8");
        assert!(!prefers_chinese());
        std::env::set_var("LC_ALL", "CN");
        assert!(prefers_chinese());
        for (key, value) in keys.iter().zip(saved) {
            restore_env2(key, value);
        }
    }

    // ─── /shell, /metrics, /snapshot, /context, /system-prompt … ───────

    /// `/shell` captures the agent's output, and `/metrics` / `/snapshot` land
    /// in the pager with the arguments the user typed on the wire.
    #[tokio::test(flavor = "multi_thread")]
    async fn shell_metrics_and_snapshot_round_trip_through_a_live_agent() {
        let mock = AppMockAgent {
            overrides: std::collections::HashMap::from([
                (
                    "shell".to_string(),
                    r#"{"output":"one\ntwo\n","exitCode":3}"#.to_string(),
                ),
                (
                    "get_runtime_metrics".to_string(),
                    r#"{"activeRunGauge":2,"eventJournalHealthy":true,"eventJournalError":null}"#
                        .to_string(),
                ),
                (
                    "get_run_snapshot".to_string(),
                    r#"{"runSnapshot":true,"watermark":7,"nextSinceIdx":7,"hasMore":false,"projection":{"runId":"run-1","cursor":7,"events":[{"idx":1,"type":"text_chunk"}]}}"#
                        .to_string(),
                ),
            ]),
            events: vec![future_rpc::proto::StreamEvent {
                r#type: "agent_start".into(),
                session_id: "s1".into(),
                run_id: "run-1".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();

        app.handle_submit("/shell echo  hi");
        pump_until_overlay(&mut app, &mut rx).await;
        let text = top_overlay_text(&mut app, 90);
        assert!(text.contains("one") && text.contains("two"), "{text}");
        assert!(text.contains("exit code: 3"), "{text}");
        app.handle_cmd(UiCmd::OverlayCancel);

        app.handle_submit("/metrics");
        pump_until_overlay(&mut app, &mut rx).await;
        let text = top_overlay_text(&mut app, 90);
        // The pager paints its content verbatim: a markdown `**` in the title
        // reaches the screen as two asterisks (the rows are padded to width).
        let title = text.lines().next().unwrap_or_default().to_string();
        assert_eq!(title.trim_end(), "Runtime metrics", "{text}");
        assert!(text.contains("activeRunGauge: 2"), "{text}");
        assert!(text.contains("eventJournalError: (none)"), "{text}");
        app.handle_cmd(UiCmd::OverlayCancel);

        app.handle_submit("/snapshot");
        pump_until_overlay(&mut app, &mut rx).await;
        let text = top_overlay_text(&mut app, 90);
        let title = text.lines().next().unwrap_or_default().to_string();
        assert_eq!(title.trim_end(), "Run snapshot", "{text}");
        assert!(text.contains("watermark: 7"), "{text}");
        assert!(text.contains("projection run: run-1"), "{text}");
        assert!(text.contains("#1 text_chunk"), "{text}");
        app.handle_cmd(UiCmd::OverlayCancel);

        let sent = requests.lock().unwrap().clone();
        let last = |kind: &str| {
            sent.iter()
                .rfind(|cmd| cmd.r#type == kind)
                .cloned()
                .unwrap_or_else(|| panic!("{kind} never reached the agent"))
        };
        assert_eq!(last("shell").command, "echo  hi", "the command is verbatim");
        assert_eq!(last("shell").shell_timeout_ms, 0, "0 = the agent's default");
        assert_eq!(last("get_run_snapshot").run_id, "run-1");
        app.stop();
    }

    /// The three diagnostics renderers, including the shapes the agent only
    /// produces on an edge case.
    #[test]
    fn diagnostics_renderers_handle_every_payload_shape() {
        // Metrics: an ordered object, an empty object, and a bare scalar.
        let lines = App::<FakeTerminal>::metrics_lines(&json_parse(
            r#"{"a":1,"b":"x","c":null,"d":[1,2]}"#,
        ));
        assert_eq!(
            lines,
            vec![
                "Runtime metrics".to_string(),
                String::new(),
                "a: 1".to_string(),
                "b: x".to_string(),
                "c: (none)".to_string(),
                "d: [1,2]".to_string(),
            ]
        );
        // The pager paints its content verbatim, so a markdown `**` in a title
        // reaches the screen as two asterisks.
        assert!(!lines[0].contains("**"), "{}", lines[0]);
        assert!(App::<FakeTerminal>::metrics_lines(&json_parse("{}"))
            .contains(&"The agent reported no counters.".to_string()));
        assert!(
            App::<FakeTerminal>::metrics_lines(&Value::String("nothing".into()))
                .contains(&"nothing".to_string())
        );

        // Snapshot: every envelope field, the projection, and the missing-list
        // fallback.
        let lines = App::<FakeTerminal>::snapshot_lines(&json_parse(
            r#"{"runSnapshot":true,"watermark":3,"nextSinceIdx":3,"hasMore":true,"projection":{"runId":"r","cursor":3,"events":[{"idx":2,"type":"tool_end"},{"idx":"x"}]}}"#,
        ));
        let joined = lines.join("\n");
        assert_eq!(lines[0], "Run snapshot", "plain-text title: {joined}");
        assert!(joined.contains("runSnapshot: true"), "{joined}");
        assert!(joined.contains("hasMore: true"), "{joined}");
        assert!(joined.contains("projection cursor: 3"), "{joined}");
        assert!(joined.contains("events: 2"), "{joined}");
        assert!(joined.contains("#2 tool_end"), "{joined}");
        assert!(joined.contains("#x -"), "{joined}");
        let sparse = App::<FakeTerminal>::snapshot_lines(&json_parse(r#"{}"#));
        assert!(
            sparse.contains(&"events: (not reported)".to_string()),
            "{sparse:?}"
        );

        // Shell: an empty body still shows the exit code, and a payload without
        // one says so.
        assert_eq!(
            App::<FakeTerminal>::shell_lines(&json_parse(r#"{"output":"","exitCode":0}"#)),
            vec![String::new(), "exit code: 0".to_string()]
        );
        let no_echo = App::<FakeTerminal>::shell_lines(&json_parse(r#"{"other":1}"#));
        assert!(no_echo.contains(&"exit code: (not reported)".to_string()));

        // All three kinds reach their renderer from the pager arm.
        for (kind, needle) in [
            (DiagnosticsKind::Metrics, "a: 1"),
            (DiagnosticsKind::Snapshot, "watermark: 3"),
            (DiagnosticsKind::Shell, "exit code: 0"),
        ] {
            let payload = match kind {
                DiagnosticsKind::Metrics => json_parse(r#"{"a":1}"#),
                DiagnosticsKind::Snapshot => json_parse(r#"{"watermark":3}"#),
                DiagnosticsKind::Shell => json_parse(r#"{"output":"","exitCode":0}"#),
            };
            let lines = App::<FakeTerminal>::diagnostics_lines(kind, &payload);
            assert!(lines.iter().any(|line| line.contains(needle)), "{lines:?}");
        }
    }

    /// A failed diagnostics RPC names the command and the reason; `/snapshot`
    /// without a run says that instead of blaming the agent.
    #[tokio::test]
    async fn diagnostics_failures_are_named() {
        let (mut app, mut rx) = make_app(100, 30);
        for (kind, needle) in [
            (
                DiagnosticsKind::Metrics,
                "Failed to load runtime metrics: boom",
            ),
            (
                DiagnosticsKind::Snapshot,
                "Failed to load the run snapshot: boom",
            ),
            (DiagnosticsKind::Shell, "Shell command failed: boom"),
        ] {
            app.handle_cmd(UiCmd::DiagnosticsLoaded {
                kind,
                result: Err("boom".into()),
            });
            assert_eq!(last_system(&app), needle);
        }
        app.handle_submit("/snapshot");
        pump_until_msg(&mut app, &mut rx, "no run to snapshot yet").await;
        app.handle_submit("/shell");
        assert_eq!(last_system(&app), "Usage: /shell <command>");
        // A payload that renders to nothing still opens something usable.
        app.handle_cmd(UiCmd::DiagnosticsLoaded {
            kind: DiagnosticsKind::Metrics,
            result: Ok(json_parse("{}")),
        });
        assert!(top_overlay_is::<PagerOverlay>(&app));
    }

    /// `/title` generates and applies a name, refuses a locale the agent does
    /// not accept, and never writes an empty name.
    #[tokio::test(flavor = "multi_thread")]
    async fn title_generates_applies_and_validates() {
        let mock = AppMockAgent {
            overrides: std::collections::HashMap::from([(
                "generate_session_title".to_string(),
                r#"{"title":"A useful name","model":"m"}"#.to_string(),
            )]),
            ..Default::default()
        };
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();

        app.handle_submit("/title zh");
        pump_until_msg(&mut app, &mut rx, "Session renamed to A useful name.").await;
        assert_eq!(app.state.session_name.as_deref(), Some("A useful name"));
        let sent = requests.lock().unwrap().clone();
        assert_eq!(
            sent.iter()
                .rfind(|cmd| cmd.r#type == "generate_session_title")
                .map(|cmd| cmd.mode.clone()),
            Some("zh".to_string())
        );
        assert_eq!(
            sent.iter()
                .rfind(|cmd| cmd.r#type == "set_session_name")
                .map(|cmd| cmd.name.clone()),
            Some("A useful name".to_string())
        );

        app.handle_submit("/title german");
        assert!(last_system(&app).contains("Usage: /title [zh|en] (got 'german')"));

        // The agent's edge cases: no title at all, and a blank one.
        app.handle_cmd(UiCmd::SessionTitleGenerated(Ok(json_parse(
            r#"{"model":"m"}"#,
        ))));
        assert_eq!(last_system(&app), "The agent returned no title.");
        app.handle_cmd(UiCmd::SessionTitleGenerated(Ok(json_parse(
            r#"{"title":"   "}"#,
        ))));
        assert!(last_system(&app).contains("returned an empty title"));
        app.handle_cmd(UiCmd::SessionTitleGenerated(Err("model down".into())));
        assert!(last_system(&app).contains("Failed to generate a title: model down"));
        app.handle_cmd(UiCmd::SessionTitleApplied {
            title: "T".into(),
            result: Err("read-only".into()),
        });
        assert!(last_system(&app).contains("Failed to apply the title: read-only"));
        app.stop();
    }

    /// `/title`'s locale: an explicit argument wins; with none the host locale
    /// decides — the TUI has no language setting of its own, and the agent
    /// rejects anything outside `zh`/`en`.
    #[test]
    fn title_mode_follows_the_argument_then_the_host_locale() {
        let _guard = crate::test_env::lock();
        let saved: Vec<Option<std::ffi::OsString>> = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .map(std::env::var_os)
            .collect();
        let mode = App::<FakeTerminal>::title_mode;

        for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            std::env::remove_var(key);
        }
        assert_eq!(mode(""), Ok("en"));
        std::env::set_var("LANG", "zh_CN.UTF-8");
        assert_eq!(mode("  "), Ok("zh"));
        // An explicit argument overrides the locale in both directions.
        assert_eq!(mode("en"), Ok("en"));
        std::env::set_var("LANG", "en_US.UTF-8");
        assert_eq!(mode(" ZH "), Ok("zh"));
        assert_eq!(mode("german"), Err("german".to_string()));

        for (key, value) in ["LC_ALL", "LC_MESSAGES", "LANG"].iter().zip(saved) {
            restore_env2(key, value);
        }
    }

    /// `/context` lists what the agent reported and toggles the switch; a bad
    /// value is a usage error.
    #[tokio::test(flavor = "multi_thread")]
    async fn context_lists_files_and_toggles_loading() {
        let mock = AppMockAgent::default();
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();

        app.state.context_files = vec!["/w/AGENTS.md".into()];
        app.handle_submit("/context");
        assert!(top_overlay_is::<PagerOverlay>(&app));
        let text = top_overlay_text(&mut app, 90);
        let title = text.lines().next().unwrap_or_default().to_string();
        assert_eq!(title.trim_end(), "Context files", "{text}");
        assert!(text.contains("/w/AGENTS.md"), "{text}");
        assert!(text.contains("/context on|off"), "{text}");
        app.handle_cmd(UiCmd::OverlayCancel);

        app.handle_submit("/context off");
        pump_until_msg(&mut app, &mut rx, "Context files disabled.").await;
        app.handle_submit("/context on");
        pump_until_msg(&mut app, &mut rx, "Context files enabled.").await;
        assert!(requests
            .lock()
            .unwrap()
            .iter()
            .any(|cmd| cmd.r#type == "set_context_files" && !cmd.enabled));

        app.handle_submit("/context maybe");
        assert!(last_system(&app).contains("Usage: /context [on|off] (got 'maybe')"));
        app.handle_cmd(UiCmd::ContextFilesSet {
            enabled: true,
            result: Err("denied".into()),
        });
        assert_eq!(last_system(&app), "Failed to set context files: denied");

        // Nothing configured (or loading off): say both, don't guess.
        app.state.context_files.clear();
        app.handle_submit("/context");
        let text = top_overlay_text(&mut app, 90);
        assert!(
            text.contains("may be absent, or loading may be off"),
            "{text}"
        );
        app.stop();
    }

    // ─── /delete (needs a confirmation) ───────────────────────────────

    /// `/delete` explains itself first, then deletes the session it named and
    /// leaves the TUI on a fresh one.
    #[tokio::test(flavor = "multi_thread")]
    async fn delete_needs_confirmation_then_starts_a_new_session() {
        let mock = AppMockAgent::default();
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();

        app.handle_submit("/delete");
        let warning = last_system(&app);
        assert!(warning.contains("Re-run as /delete --yes"), "{warning}");
        assert!(
            !requests
                .lock()
                .unwrap()
                .iter()
                .any(|cmd| cmd.r#type == "delete_session"),
            "an unconfirmed /delete must not reach the agent"
        );

        app.handle_submit("/delete --yes");
        pump_until_msg(&mut app, &mut rx, "Starting a new one.").await;
        // The replacement session is created right after the delete: wait for
        // one that comes *after* the delete rather than for the one `start()`
        // made.
        let replaced = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let seen = requests.lock().unwrap().clone();
                let delete_at = seen.iter().position(|cmd| cmd.r#type == "delete_session");
                if delete_at
                    .is_some_and(|at| seen.iter().skip(at).any(|cmd| cmd.r#type == "new_session"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await;
        assert!(replaced.is_ok(), "the TUI must end up on a live session");

        let sent = requests.lock().unwrap().clone();
        let deleted_at = sent
            .iter()
            .position(|cmd| cmd.r#type == "delete_session")
            .expect("delete_session reached the agent");
        assert_eq!(sent[deleted_at].session_id, "s1");
        assert!(
            sent.iter()
                .skip(deleted_at)
                .any(|cmd| cmd.r#type == "new_session"),
            "a fresh session must be created after the delete"
        );

        app.handle_cmd(UiCmd::SessionDeleted {
            session_id: "s9".into(),
            result: Err("busy".into()),
        });
        assert_eq!(last_system(&app), "Failed to delete the session: busy");
        app.stop();
    }

    /// Without a session there is nothing to confirm or delete.
    #[tokio::test]
    async fn delete_without_a_session_is_a_message() {
        let (mut app, _rx) = make_app(100, 30);
        app.handle_submit("/delete");
        assert!(last_system(&app).contains("(no session yet)"));
        app.handle_submit("/delete --yes");
        assert_eq!(last_system(&app), "No session to delete.");
    }

    // ─── Overlay plumbing for the two new panels ──────────────────────

    /// Both panels take the palette — when they are open and when they are
    /// created after a switch — and every row they draw is width-exact.
    #[tokio::test]
    async fn new_panels_follow_the_palette_and_fit_their_rows() {
        let (mut app, _rx) = make_app(100, 30);
        app.handle_submit("/sandbox");
        let idx = app.get_top_overlay_index().unwrap();
        let dark = app.overlay_stack[idx].component.render(80);
        let (_, light) = crate::themes::resolve_theme(Some("light"));
        app.apply_theme(light);
        let lit = app.overlay_stack[idx].component.render(80);
        assert_ne!(dark, lit, "an open panel repaints on /theme");
        assert!(
            dark.iter()
                .chain(lit.iter())
                .all(|row| visible_width(row) == 80),
            "every overlay row must be width-exact"
        );

        // Created *after* the switch: the palette is applied at construction,
        // so the first render already matches the themed one.
        app.hide_overlay();
        app.handle_submit("/sandbox");
        let idx = app.get_top_overlay_index().unwrap();
        assert_eq!(app.overlay_stack[idx].component.render(80), lit);

        // The skills browser is the other arm of the same fan-out.
        app.hide_overlay();
        app.state.skills = vec!["alpha".into()];
        app.handle_submit("/skills");
        let idx = app.get_top_overlay_index().unwrap();
        let lit_skills = app.overlay_stack[idx].component.render(80);
        app.apply_theme(DARK_THEME);
        let dark_skills = app.overlay_stack[idx].component.render(80);
        assert_ne!(lit_skills, dark_skills);
        assert!(dark_skills.iter().all(|row| visible_width(row) == 80));
    }

    /// A panel built when the terminal is 1 column wide still renders rows the
    /// compositor can place (the caveat row is wrapped, not truncated away).
    #[tokio::test]
    async fn panel_rows_fit_a_narrow_terminal() {
        let (mut app, _rx) = make_app(1, 3);
        app.handle_submit("/sandbox");
        let idx = app.get_top_overlay_index().unwrap();
        let rows = app.overlay_stack[idx].component.render(1);
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|row| visible_width(row) == 1), "{rows:?}");
    }

    /// `fit_overlay_row` pads to the exact width and never drops the styling
    /// that precedes the cut.
    #[test]
    fn fit_overlay_row_pads_and_truncates() {
        assert_eq!(fit_overlay_row("ab", 4), "ab  ");
        assert_eq!(fit_overlay_row("abcdef", 3), "abc");
        assert_eq!(fit_overlay_row("", 0), "");
        let styled = "\u{1b}[31mabc\u{1b}[0m";
        let fitted = fit_overlay_row(styled, 5);
        assert_eq!(visible_width(&fitted), 5);
        assert!(
            fitted.starts_with("\u{1b}[31m"),
            "styling survives: {fitted:?}"
        );
    }

    // ─── /worktree ─────────────────────────────────────────────────────

    /// The `git` invocations an injected runner saw, in order.
    type GitCalls = Arc<Mutex<Vec<Vec<String>>>>;

    /// A [`GitCli`] whose process boundary records its argv and answers with
    /// `answer`: no unit test of `/worktree` runs the real git through it.
    fn fake_git<F>(calls: &GitCalls, answer: F) -> Arc<GitCli>
    where
        F: Fn(&[String]) -> Result<crate::worktree::GitOutput, String> + Send + Sync + 'static,
    {
        let sink = Arc::clone(calls);
        let runner: GitRunner = Box::new(move |args: &[String], _timeout: Duration| {
            sink.lock().unwrap().push(args.to_vec());
            answer(args)
        });
        Arc::new(GitCli::with_runner(runner, PathBuf::from("/fake/git")))
    }

    /// The recorded git argv list.
    fn git_args(calls: &GitCalls) -> Vec<Vec<String>> {
        calls.lock().unwrap().clone()
    }

    /// A successful `git` answer.
    fn git_ok(stdout: &str) -> Result<crate::worktree::GitOutput, String> {
        Ok(crate::worktree::GitOutput {
            code: 0,
            stdout: stdout.to_string(),
            stderr: String::new(),
        })
    }

    /// The listing the fake git reports: a main worktree and one branch
    /// worktree under `.worktrees/`, exactly the layout this repository uses.
    const GIT_PORCELAIN: &str = "worktree /repo\nHEAD 1\nbranch refs/heads/main\n\nworktree /repo/.worktrees/demo\nHEAD 2\nbranch refs/heads/feat/demo\n\n";

    /// The fake git's canned answers: `worktree list` reports
    /// [`GIT_PORCELAIN`], `status` calls the `demo` worktree dirty,
    /// `rev-parse` says `main` and `worktree add` succeeds.
    fn worktree_git_canned(args: &[String]) -> Result<crate::worktree::GitOutput, String> {
        let rest = &args[2..];
        match rest.first().map(String::as_str) {
            Some("worktree") if rest.get(1).map(String::as_str) == Some("list") => {
                git_ok(GIT_PORCELAIN)
            }
            Some("worktree") => git_ok("Preparing worktree\n"),
            Some("status") if args.iter().any(|arg| arg.contains("demo")) => git_ok(" M x\n"),
            Some("status") => git_ok(""),
            Some("rev-parse") => git_ok("main\n"),
            _ => git_ok(""),
        }
    }

    /// The `/worktree` panel of the top overlay.
    fn top_worktree_view(app: &mut App<FakeTerminal>) -> Option<&mut WorktreeView> {
        app.top_worktree_overlay().map(|overlay| overlay.view_mut())
    }

    /// The overlay sinks are the only translation from a keypress to a
    /// `UiCmd`: every payable action has to reach the channel, and the
    /// redraw-only actions must stay silent.
    #[test]
    fn worktree_sink_translates_every_action() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut sink = App::<FakeTerminal>::worktree_sink(tx);
        sink(WorktreeAction::None);
        assert!(rx.try_recv().is_err(), "a redraw is not a command");
        sink(WorktreeAction::Select("/repo".into()));
        assert!(matches!(
            rx.try_recv(),
            Ok(UiCmd::WorktreeSwitchRequested(path)) if path == "/repo"
        ));
        sink(WorktreeAction::New);
        assert!(matches!(rx.try_recv(), Ok(UiCmd::WorktreeNewRequested)));
        sink(WorktreeAction::Refresh);
        assert!(matches!(rx.try_recv(), Ok(UiCmd::WorktreeRefreshRequested)));
        sink(WorktreeAction::Cancelled);
        assert!(matches!(rx.try_recv(), Ok(UiCmd::OverlayCancel)));
    }

    /// `/worktree` opens the panel before git has answered — the create row is
    /// usable at once — and fills the list in when the blocking-pool call
    /// reports back.
    #[tokio::test(flavor = "multi_thread")]
    async fn worktree_panel_opens_with_the_create_row_then_fills_in() {
        let (mut app, mut rx) = make_app(100, 30);
        let calls: GitCalls = Arc::new(Mutex::new(Vec::new()));
        app.git_cli = fake_git(&calls, worktree_git_canned);
        app.state.cwd = "/repo".into();

        app.handle_submit("/worktree");
        let pending = top_overlay_text(&mut app, 76);
        assert!(pending.contains(WORKTREE_LOADING), "{pending}");
        assert!(pending.contains("New worktree…"), "{pending}");

        pump_until_overlay_text(&mut app, &mut rx, "feat/demo · dirty").await;
        let settled = top_overlay_text(&mut app, 76);
        assert!(!settled.contains(WORKTREE_LOADING), "{settled}");
        assert!(settled.contains("main · clean"), "{settled}");
        assert!(
            settled.contains("current"),
            "the session's worktree: {settled}"
        );
        // The listing is one `worktree list` plus one `status` per entry.
        assert_eq!(
            git_args(&calls),
            vec![
                crate::worktree::list_args(std::path::Path::new("/repo")),
                crate::worktree::status_args(std::path::Path::new("/repo")),
                crate::worktree::status_args(std::path::Path::new("/repo/.worktrees/demo")),
            ]
        );
        app.stop();
    }

    /// `ctrl+r` re-reads git's list and raises the notice while it runs.
    #[tokio::test(flavor = "multi_thread")]
    async fn worktree_panel_reloads_the_listing() {
        let (mut app, mut rx) = make_app(100, 30);
        let calls: GitCalls = Arc::new(Mutex::new(Vec::new()));
        app.git_cli = fake_git(&calls, worktree_git_canned);
        app.state.cwd = "/repo".into();
        app.handle_submit("/worktree");
        pump_until_overlay_text(&mut app, &mut rx, "feat/demo").await;
        let first = git_args(&calls).len();

        press_on_overlay(&mut app, &mut rx, "ctrl+r");
        pump_until_overlay_text(&mut app, &mut rx, "feat/demo").await;
        assert!(git_args(&calls).len() > first, "the reload asked git again");
        assert!(
            top_overlay_text(&mut app, 76).contains("feat/demo"),
            "the panel keeps its rows while the reload runs"
        );
        app.stop();
    }

    /// A failed `git worktree list` is reported in the panel, not swallowed
    /// and not turned into a panic.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_listing_is_reported_in_the_panel() {
        let (mut app, mut rx) = make_app(100, 30);
        let calls: GitCalls = Arc::new(Mutex::new(Vec::new()));
        app.git_cli = fake_git(&calls, |_args| {
            Ok(crate::worktree::GitOutput {
                code: 128,
                stdout: String::new(),
                stderr: "fatal: not a git repository\n".to_string(),
            })
        });
        app.state.cwd = "/not-a-repo".into();
        app.handle_submit("/worktree");
        pump_until_overlay_text(&mut app, &mut rx, "Failed to read git worktrees").await;
        // The notice row is the panel's own row, so it carries the whole
        // message (the rendered row is truncated to the panel width).
        let notice = top_worktree_view(&mut app)
            .unwrap()
            .notice()
            .unwrap()
            .to_string();
        assert!(notice.contains("fatal: not a git repository"), "{notice}");
        let text = top_overlay_text(&mut app, 100);
        assert!(
            text.contains("New worktree…"),
            "the create row survives a failed listing: {text}"
        );
        app.stop();
    }

    /// An answer that lands after the panel was closed is dropped: it must not
    /// repaint whatever the user opened in the meantime, and it must not write
    /// a transcript line (a listing is a read).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_listing_that_arrives_after_the_panel_closed_is_dropped() {
        let (mut app, _rx) = make_app(100, 30);
        app.handle_submit("/worktree");
        assert!(top_overlay_is::<WorktreeOverlay>(&app));
        app.handle_key("escape");
        assert!(app.overlay_stack.is_empty(), "escape closed the panel");
        app.handle_cmd(UiCmd::WorktreesLoaded {
            result: Ok(vec![WorktreeInfo {
                path: "/repo".into(),
                branch: Some("main".into()),
                ..WorktreeInfo::default()
            }]),
        });
        assert!(app.overlay_stack.is_empty());
        assert!(
            !system_messages(&app)
                .iter()
                .any(|message| message.contains("worktree")),
            "a listing writes no transcript line"
        );
    }

    /// The panel's first escape clears an active search instead of closing it
    /// (the app's `wants_escape` gate reaches this overlay).
    #[tokio::test(flavor = "multi_thread")]
    async fn the_worktree_search_owns_the_first_escape() {
        let (mut app, mut rx) = make_app(100, 30);
        app.handle_submit("/worktree");
        press_on_overlay(&mut app, &mut rx, "d");
        assert_eq!(top_worktree_view(&mut app).unwrap().filter(), "d");
        app.handle_key("escape");
        assert!(top_overlay_is::<WorktreeOverlay>(&app), "still open");
        assert_eq!(top_worktree_view(&mut app).unwrap().filter(), "");
        app.handle_key("escape");
        assert!(app.overlay_stack.is_empty(), "the second escape closes");
    }

    /// Confirming a worktree row switches the session's cwd through the same
    /// `set_cwd` RPC `/cwd` sends, with the path git reported verbatim.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_panel_switches_the_session_to_the_selected_worktree() {
        let (mut app, mut rx, requests) =
            app_with_live_agent(std::collections::HashMap::new()).await;
        let calls: GitCalls = Arc::new(Mutex::new(Vec::new()));
        app.git_cli = fake_git(&calls, worktree_git_canned);
        app.state.cwd = "/repo".into();

        app.handle_submit("/worktree");
        pump_until_overlay_text(&mut app, &mut rx, "feat/demo").await;
        press_on_overlay(&mut app, &mut rx, "down");
        press_on_overlay(&mut app, &mut rx, "enter");
        pump_until_msg(
            &mut app,
            &mut rx,
            "Working directory: /repo/.worktrees/demo",
        )
        .await;

        assert_eq!(app.state.cwd, "/repo/.worktrees/demo");
        assert!(app.overlay_stack.is_empty(), "switching closes the panel");
        let seen: Vec<String> = requests
            .lock()
            .unwrap()
            .iter()
            .filter(|cmd| cmd.r#type == "set_cwd")
            .map(|cmd| cmd.cwd.clone())
            .collect();
        assert_eq!(seen, vec!["/repo/.worktrees/demo".to_string()]);
        app.stop();
    }

    /// The create row names no branch, so it puts the command in the prompt
    /// instead of guessing one.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_create_row_inserts_the_command_into_the_prompt() {
        let (mut app, mut rx) = make_app(100, 30);
        let calls: GitCalls = Arc::new(Mutex::new(Vec::new()));
        app.git_cli = fake_git(&calls, worktree_git_canned);
        app.state.cwd = "/repo".into();
        app.handle_submit("/worktree");
        pump_until_overlay_text(&mut app, &mut rx, "feat/demo").await;
        // The create row is the last row of the panel.
        press_on_overlay(&mut app, &mut rx, "end");
        press_on_overlay(&mut app, &mut rx, "enter");
        assert!(app.overlay_stack.is_empty(), "the panel closes");
        assert_eq!(app.input.get_value(), NEW_WORKTREE_COMMAND);
        let last = last_system(&app);
        assert!(
            last.contains("/worktree new feat/tui-worktree"),
            "last system message: {last}"
        );
        app.stop();
    }

    /// `/worktree new <branch>` runs the whole git whitelist, in order, and
    /// moves the session into the worktree it created.
    #[tokio::test(flavor = "multi_thread")]
    async fn creating_a_worktree_runs_git_then_switches_the_session() {
        let (mut app, mut rx, requests) =
            app_with_live_agent(std::collections::HashMap::new()).await;
        let calls: GitCalls = Arc::new(Mutex::new(Vec::new()));
        app.git_cli = fake_git(&calls, worktree_git_canned);
        app.state.cwd = "/repo".into();

        app.handle_submit("/worktree new feat/demo");
        pump_until_msg(
            &mut app,
            &mut rx,
            "Created worktree demo on branch feat/demo from main.",
        )
        .await;
        pump_until_msg(
            &mut app,
            &mut rx,
            "Working directory: /repo/.worktrees/demo",
        )
        .await;

        assert_eq!(
            git_args(&calls),
            vec![
                crate::worktree::list_args(std::path::Path::new("/repo")),
                crate::worktree::status_args(std::path::Path::new("/repo")),
                crate::worktree::status_args(std::path::Path::new("/repo/.worktrees/demo")),
                crate::worktree::branch_args(std::path::Path::new("/repo")),
                crate::worktree::add_args(
                    std::path::Path::new("/repo"),
                    &crate::worktree::plan_worktree(std::path::Path::new("/repo"), "feat/demo")
                        .unwrap()
                ),
            ]
        );
        let seen: Vec<String> = requests
            .lock()
            .unwrap()
            .iter()
            .filter(|cmd| cmd.r#type == "set_cwd")
            .map(|cmd| cmd.cwd.clone())
            .collect();
        assert_eq!(seen, vec!["/repo/.worktrees/demo".to_string()]);
        app.stop();
    }

    /// A name git would refuse (or that escapes the repository) is rejected by
    /// the app: the message says why and *no process runs at all*.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_invalid_worktree_name_never_reaches_git() {
        let (mut app, _rx) = make_app(100, 30);
        let calls: GitCalls = Arc::new(Mutex::new(Vec::new()));
        app.git_cli = fake_git(&calls, worktree_git_canned);
        app.state.cwd = "/repo".into();

        app.handle_submit("/worktree new ../escape");
        let message = last_system(&app);
        assert!(message.contains("must not contain '..'"), "{message}");
        app.handle_submit("/worktree new");
        assert!(last_system(&app).contains("Usage: /worktree new <branch>"));
        app.handle_submit("/worktree /some/path");
        assert!(last_system(&app).contains(WORKTREE_USAGE));
        let spawned = git_args(&calls);
        assert!(
            spawned.is_empty(),
            "a rejected name spawns nothing: {spawned:?}"
        );
    }

    /// Moving the session's cwd under a running turn is refused, exactly as
    /// `/cwd` refuses it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_worktree_create_is_refused_while_streaming() {
        let (mut app, _rx) = make_app(100, 30);
        let calls: GitCalls = Arc::new(Mutex::new(Vec::new()));
        app.git_cli = fake_git(&calls, worktree_git_canned);
        app.state.cwd = "/repo".into();
        app.state.streaming = true;

        app.handle_submit("/worktree new feat/demo");
        assert_eq!(last_system(&app), CWD_STREAMING_ERROR);
        assert!(git_args(&calls).is_empty());
    }

    /// A failure that arrives with no panel to report into (a create is typed
    /// at the prompt, which has no panel) goes to the transcript.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_create_failure_with_no_panel_goes_to_the_transcript() {
        let (mut app, _rx) = make_app(100, 30);
        app.handle_cmd(UiCmd::WorktreeAdded {
            result: Err("git worktree add: fatal: '/repo/.worktrees/demo' already exists".into()),
        });
        assert_eq!(
            last_system(&app),
            "Failed to create the worktree: git worktree add: fatal: '/repo/.worktrees/demo' already exists"
        );
    }

    /// The panel's notice is *also* reachable for a create that fails while it
    /// is open (a reload and a create can overlap).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_create_failure_lands_in_the_open_panel() {
        let (mut app, _rx) = make_app(100, 30);
        app.handle_submit("/worktree");
        app.handle_cmd(UiCmd::WorktreeAdded {
            result: Err("already exists".into()),
        });
        let text = top_overlay_text(&mut app, 100);
        assert!(
            text.contains("Failed to create the worktree: already exists"),
            "{text}"
        );
    }

    // ─── /worktree against the real git ────────────────────────────────

    /// A throwaway repository with one commit on `main`, removed on drop.
    struct TempGitRepo {
        root: PathBuf,
    }

    impl TempGitRepo {
        fn new() -> Self {
            let dir =
                std::env::temp_dir().join(format!("future-app-worktree-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            // `git worktree list` prints resolved paths; canonicalise once so
            // nothing below compares a resolved path with the macOS
            // `/var` → `/private/var` symlink.
            let root = std::fs::canonicalize(&dir).unwrap();
            git_setup(&root, &["init", "-q", "-b", "main"]);
            git_setup(&root, &["config", "user.email", "tui@example.com"]);
            git_setup(&root, &["config", "user.name", "TUI Test"]);
            std::fs::write(root.join("README.md"), "hello\n").unwrap();
            git_setup(&root, &["add", "README.md"]);
            git_setup(
                &root,
                &["-c", "commit.gpgsign=false", "commit", "-q", "-m", "init"],
            );
            Self { root }
        }
    }

    impl Drop for TempGitRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// One `git` command for the fixture. Test *setup* only: the module under
    /// test can never build anything but list / status / rev-parse / add.
    fn git_setup(dir: &std::path::Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git runs in this test");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(out.status.success(), "git {args:?} failed: {stderr}");
    }

    /// The canned git answers an invocation it has no case for with an empty
    /// success: a future test that adds a git call fails on *its* assertion,
    /// not on a fixture that panicked.
    #[test]
    fn the_canned_git_answers_an_invocation_it_does_not_know() {
        let args: Vec<String> = ["-C", "/repo", "fetch"]
            .iter()
            .map(|arg| (*arg).to_string())
            .collect();
        assert_eq!(worktree_git_canned(&args), git_ok(""));
    }

    /// The end-to-end flow with the real `git`: `/worktree new feat/demo`
    /// creates the directory, checks out the branch, and the session's cwd
    /// reaches the agent as a `set_cwd` with that path.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_real_repository_creates_a_worktree_and_switches_to_it() {
        let repo = TempGitRepo::new();
        let (mut app, mut rx, requests) =
            app_with_live_agent(std::collections::HashMap::new()).await;
        app.git_cli = Arc::new(GitCli::new());
        app.state.cwd = repo.root.display().to_string();

        app.handle_submit("/worktree new feat/demo");
        pump_until_msg(
            &mut app,
            &mut rx,
            "Created worktree demo on branch feat/demo from main.",
        )
        .await;
        let path = repo.root.join(".worktrees").join("demo");
        assert!(path.is_dir(), "git created the worktree directory");
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "feat/demo");
        pump_until_msg(
            &mut app,
            &mut rx,
            &format!("Working directory: {}", path.display()),
        )
        .await;
        assert_eq!(app.state.cwd, path.display().to_string());
        let seen: Vec<String> = requests
            .lock()
            .unwrap()
            .iter()
            .filter(|cmd| cmd.r#type == "set_cwd")
            .map(|cmd| cmd.cwd.clone())
            .collect();
        assert_eq!(seen, vec![path.display().to_string()]);
        app.stop();
    }

    // ─── /keymap ───────────────────────────────────────────────────────

    /// An app whose `keybindings.json` lives in `dir` (the file may or may not
    /// exist yet). Every keymap test builds its own directory: the shared
    /// `make_app` temp path would make one test's bindings another test's
    /// startup config.
    fn app_in_keybindings_dir(
        dir: &std::path::Path,
    ) -> (App<FakeTerminal>, mpsc::UnboundedReceiver<UiCmd>) {
        let (op_tx, op_rx) = mpsc::unbounded_channel();
        let (client, _events, _conn) = GrpcClient::new("127.0.0.1:1");
        let app = App::new(
            FakeTerminal {
                writes: Rc::new(RefCell::new(Vec::new())),
                cols: 100,
                rows: 30,
                on_input: None,
                on_resize: None,
            },
            Arc::new(client),
            op_tx,
            &CliOptions::default(),
            dir.join("settings.json"),
        );
        (app, op_rx)
    }

    /// The same, with `body` already written to the keybinding file (the app
    /// therefore reads it during `setup`). The `TempDir` is returned so it
    /// outlives the app.
    fn app_with_keybindings(
        body: &str,
    ) -> (
        App<FakeTerminal>,
        mpsc::UnboundedReceiver<UiCmd>,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(crate::keybindings::KEYBINDINGS_FILE), body).unwrap();
        let (app, rx) = app_in_keybindings_dir(dir.path());
        (app, rx, dir)
    }

    /// The top overlay as the keymap panel (panics when something else is up).
    fn top_keymap_view(app: &mut App<FakeTerminal>) -> &mut KeymapView {
        app.top_keymap_overlay()
            .expect("the keymap panel is open")
            .view_mut()
    }

    /// Walk the panel's highlight onto `description`, then press `keys`.
    fn keymap_press(
        app: &mut App<FakeTerminal>,
        rx: &mut mpsc::UnboundedReceiver<UiCmd>,
        description: &str,
        keys: &[&str],
    ) {
        for _ in 0..64 {
            if top_keymap_view(app).highlighted().map(|item| item.value)
                == Some(description.to_string())
            {
                break;
            }
            press_on_overlay(app, rx, "down");
        }
        assert_eq!(
            top_keymap_view(app).highlighted().map(|item| item.value),
            Some(description.to_string()),
            "«{description}» is not in the panel"
        );
        for key in keys {
            press_on_overlay(app, rx, key);
        }
    }

    fn keybindings_file(dir: &std::path::Path) -> String {
        std::fs::read_to_string(dir.join(crate::keybindings::KEYBINDINGS_FILE))
            .unwrap_or_else(|_| "<missing>".to_string())
    }

    /// The panel's catalogue and the actions `setup` registers are the same
    /// list: a renamed description in either place fails here instead of
    /// quietly dropping a row from `/keymap`.
    #[tokio::test]
    async fn keymap_catalog_covers_every_registered_action() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _rx) = app_in_keybindings_dir(dir.path());
        let mut registered: Vec<String> = app
            .keybindings
            .action_bindings()
            .into_iter()
            .map(|action| action.description)
            .collect();
        registered.sort();
        let mut catalogued: Vec<String> = crate::components::keymap_view::ACTION_GROUPS
            .iter()
            .flat_map(|group| group.descriptions.iter().map(|d| d.to_string()))
            .collect();
        catalogued.sort();
        assert_eq!(registered, catalogued);
        assert_eq!(registered.len(), 13, "14 entries, two of them one action");
    }

    /// No keybindings file, no change: the built-in map is exactly what
    /// `setup` registered, nothing is written and nothing conflicts.
    #[tokio::test]
    async fn the_built_in_keymap_is_unchanged_without_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _rx) = app_in_keybindings_dir(dir.path());
        assert!(app.keybinding_problems.is_empty());
        assert!(app.keybinding_unknown.is_empty());
        assert_eq!(app.keybindings.assignment_overrides(), Vec::new());
        assert!(app.keybindings.action_conflicts().is_empty());
        assert!(app
            .keybindings
            .action_bindings()
            .iter()
            .all(|action| action.is_default()));
        let map = app.keybindings.get_binding_map();
        assert_eq!(
            map.get("ctrl+p").map(Vec::as_slice),
            Some(&["Cycle model".to_string()][..])
        );
        assert_eq!(
            map.get("shift+tab").map(Vec::as_slice),
            Some(&["Cycle thinking".to_string()][..])
        );
        assert_eq!(
            map.get("pageDown").map(Vec::as_slice),
            Some(&["Scroll chat down".to_string()][..])
        );
        // 14 registrations: 13 actions, one of which ("Cycle thinking") answers
        // on two keys, so 14 keys with exactly one action each.
        let owners = app.keybindings.key_owners();
        assert_eq!(owners.len(), 14);
        assert!(owners.iter().all(|(_, actions)| actions.len() == 1));
        // …and nothing was created on disk by the read.
        assert_eq!(keybindings_file(dir.path()), "<missing>");
    }

    #[tokio::test]
    async fn the_keybindings_file_sits_beside_settings_json() {
        let expected = std::path::Path::new("/home/dev/.future/tui").join("keybindings.json");
        assert_eq!(
            keybindings_path_for(std::path::Path::new("/home/dev/.future/tui/settings.json")),
            expected
        );
        // A bare file name (an in-memory settings path) still lands somewhere
        // usable rather than at the filesystem root.
        assert_eq!(
            keybindings_path_for(std::path::Path::new("settings.json")),
            std::path::Path::new(".").join("keybindings.json")
        );
    }

    /// A file written by a previous run is applied before the first key is
    /// ever handled.
    #[tokio::test]
    async fn a_saved_file_rebinds_at_startup() {
        let (app, _rx, _dir) = app_with_keybindings(r#"{"Cycle model": "ctrl+y"}"#);
        assert!(
            app.keybinding_problems.is_empty(),
            "{:?}",
            app.keybinding_problems
        );
        assert_eq!(
            app.keybindings.actions_on_key("ctrl+y"),
            vec!["Cycle model"]
        );
        assert!(app.keybindings.actions_on_key("ctrl+p").is_empty());
        assert_eq!(
            app.keybindings.assignment_overrides(),
            vec![("Cycle model".to_string(), "ctrl+y".to_string())]
        );
        // The other actions are untouched.
        assert_eq!(
            app.keybindings.actions_on_key("ctrl+r"),
            vec!["Browse sessions"]
        );
    }

    /// A file that only restates a built-in binding is not a problem and not a
    /// change — including the one key the panel refuses to *capture*.
    #[tokio::test]
    async fn restating_a_built_in_key_is_a_no_op() {
        let (app, _rx, _dir) = app_with_keybindings(r#"{"Interrupt / exit": "ctrl+c"}"#);
        assert!(
            app.keybinding_problems.is_empty(),
            "{:?}",
            app.keybinding_problems
        );
        assert!(app
            .keybindings
            .action_bindings()
            .iter()
            .all(|a| a.is_default()));
        assert_eq!(app.keybindings.assignment_overrides(), Vec::new());
    }

    /// One action, one key: a file entry names *the* key an action answers on,
    /// so an action this build registered twice (ctrl+t and shift+tab both
    /// cycle thinking) ends up on the one key the file names. That is the rule
    /// the panel's rows follow too ("ctrl+t, shift+tab" is one action).
    #[tokio::test]
    async fn one_action_is_one_key_in_the_file() {
        let (app, _rx, _dir) = app_with_keybindings(r#"{"Cycle thinking": "ctrl+t"}"#);
        assert!(
            app.keybinding_problems.is_empty(),
            "{:?}",
            app.keybinding_problems
        );
        let thinking = app
            .keybindings
            .action_bindings()
            .into_iter()
            .find(|action| action.description == "Cycle thinking")
            .unwrap();
        assert_eq!(thinking.keys, vec!["ctrl+t"]);
        assert!(app.keybindings.actions_on_key("shift+tab").is_empty());
        assert_eq!(
            app.keybindings.assignment_overrides(),
            vec![("Cycle thinking".to_string(), "ctrl+t".to_string())]
        );
    }

    /// A broken file never panics, never half-applies and is reported to the
    /// user rather than silently ignored.
    #[tokio::test]
    async fn a_broken_file_keeps_the_defaults_and_says_so() {
        let (app, _rx, _dir) = app_with_keybindings("{not json");
        assert_eq!(app.keybinding_problems.len(), 1);
        assert!(
            app.keybinding_problems[0].contains("not valid JSON"),
            "{:?}",
            app.keybinding_problems
        );
        assert_eq!(
            app.keybindings.actions_on_key("ctrl+p"),
            vec!["Cycle model"]
        );
        // Reported at startup, not only when `/keymap` happens to be opened:
        // the panel is empty until it is, and a silently ignored config is how
        // a typo survives for weeks.
        let last = app.chat.last_message().expect("the startup warning");
        assert_eq!(last.role, ChatRole::System);
        assert!(last.content.contains("not valid JSON"), "{}", last.content);
    }

    /// Keys no terminal can send are refused with a suggestion instead of being
    /// written into the dispatcher (where they would be silently dead).
    #[tokio::test]
    async fn a_key_no_terminal_sends_is_reported_and_not_applied() {
        let (app, _rx, _dir) =
            app_with_keybindings(r#"{"Cycle model": "PageDown", "Browse sessions": "esc"}"#);
        assert_eq!(app.keybinding_problems.len(), 2);
        let joined = app.keybinding_problems.join(" · ");
        assert!(joined.contains("did you mean \"pageDown\""), "{joined}");
        assert!(joined.contains("\"esc\""), "{joined}");
        assert_eq!(
            app.keybindings.actions_on_key("ctrl+p"),
            vec!["Cycle model"]
        );
        assert_eq!(
            app.keybindings.actions_on_key("ctrl+r"),
            vec!["Browse sessions"]
        );
        assert!(app.keybindings.actions_on_key("esc").is_empty());
        // `pageDown` itself is fine — it is "Scroll chat down"'s built-in key.
        // What must never appear is the mis-cased spelling the file asked for.
        assert_eq!(
            app.keybindings.actions_on_key("pageDown"),
            vec!["Scroll chat down"]
        );
        assert!(app
            .keybindings
            .key_owners()
            .iter()
            .all(|(key, _)| key != "PageDown"));
    }

    /// A reserved key in the file is refused for the same reason the panel
    /// refuses to capture it.
    #[tokio::test]
    async fn a_reserved_key_in_the_file_is_refused() {
        let (app, _rx, _dir) = app_with_keybindings(r#"{"Cycle model": "ctrl+c"}"#);
        assert_eq!(app.keybinding_problems.len(), 1);
        assert!(
            app.keybinding_problems[0].contains("interrupt byte"),
            "{:?}",
            app.keybinding_problems
        );
    }

    /// An entry naming an action this build does not have is kept, not dropped:
    /// saving must not delete a binding a different build wrote.
    #[tokio::test]
    async fn an_unknown_action_survives_a_save() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(crate::keybindings::KEYBINDINGS_FILE),
            r#"{"Cycle model": "ctrl+y", "Ghost action": "f9"}"#,
        )
        .unwrap();
        let (mut app, mut rx) = app_in_keybindings_dir(dir.path());
        assert_eq!(app.keybinding_unknown.len(), 1);
        assert!(app.keybinding_problems[0].contains("not an action in this build"));

        // Restoring everything rewrites the file — the ghost entry included.
        app.handle_submit("/keymap");
        keymap_press(
            &mut app,
            &mut rx,
            crate::components::keymap_view::RESET_ALL,
            &["enter"],
        );
        assert_eq!(
            app.keybindings.actions_on_key("ctrl+p"),
            vec!["Cycle model"]
        );
        assert!(app.keybindings.actions_on_key("ctrl+y").is_empty());
        assert_eq!(
            keybindings_file(dir.path()),
            "{\n  \"Ghost action\": \"f9\"\n}\n"
        );
        app.stop();
    }

    /// The whole way round: `/keymap` → capture a key → the live dispatcher
    /// answers on it → the file on disk says so.
    #[tokio::test]
    async fn a_captured_key_is_applied_persisted_and_shown() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, mut rx) = app_in_keybindings_dir(dir.path());
        app.handle_submit("/keymap");
        let text = top_overlay_text(&mut app, 76);
        assert!(text.contains("Key bindings"), "{text}");
        assert!(text.contains("Cycle model"), "{text}");
        assert!(text.contains("ctrl+p"), "{text}");
        assert!(!top_keymap_view(&mut app).is_capturing());

        keymap_press(&mut app, &mut rx, "Cycle model", &["enter"]);
        assert!(top_keymap_view(&mut app).is_capturing());
        let text = top_overlay_text(&mut app, 76);
        assert!(
            text.contains("Press the new key for «Cycle model»"),
            "{text}"
        );

        press_on_overlay(&mut app, &mut rx, "ctrl+y");
        assert!(!top_keymap_view(&mut app).is_capturing());
        // The live dispatcher moved with it.
        assert_eq!(
            app.keybindings.actions_on_key("ctrl+y"),
            vec!["Cycle model"]
        );
        assert!(app.keybindings.actions_on_key("ctrl+p").is_empty());
        assert_eq!(
            keybindings_file(dir.path()),
            "{\n  \"Cycle model\": \"ctrl+y\"\n}\n"
        );
        // The panel shows the new key on the row and says what it did.
        let text = top_overlay_text(&mut app, 76);
        assert!(text.contains("ctrl+y"), "{text}");
        assert!(text.contains("Bound «Cycle model» to ctrl+y"), "{text}");
        // And it really dispatches: the built-in key is free, the new one runs
        // the action through the manager the app uses.
        assert!(!app.keybindings.dispatch("ctrl+p", None));
        assert!(app.keybindings.dispatch("ctrl+y", None));
        app.stop();
    }

    /// Capture → a key another action owns → the panel asks. "o" applies the
    /// override (the new action wins, as the prompt says) and persists it.
    #[tokio::test]
    async fn a_conflicting_key_asks_before_it_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, mut rx) = app_in_keybindings_dir(dir.path());
        app.handle_submit("/keymap");
        keymap_press(&mut app, &mut rx, "Cycle model", &["enter", "ctrl+r"]);
        let asking = top_overlay_text(&mut app, 90);
        assert!(
            asking.contains("already bound to «Browse sessions»"),
            "{asking}"
        );
        assert!(asking.contains("o = override"), "{asking}");
        assert!(top_keymap_view(&mut app).is_capturing(), "still waiting");
        assert_eq!(
            keybindings_file(dir.path()),
            "<missing>",
            "nothing written yet"
        );

        press_on_overlay(&mut app, &mut rx, "o");
        assert_eq!(
            app.keybindings.actions_on_key("ctrl+r"),
            vec!["Cycle model", "Browse sessions"],
            "the action the user moved wins the key"
        );
        assert_eq!(
            keybindings_file(dir.path()),
            "{\n  \"Cycle model\": \"ctrl+r\"\n}\n"
        );
        // The row that lost the key is badged, and the notice explains it.
        let text = top_overlay_text(&mut app, 90);
        assert!(text.contains("no longer answers on it"), "{text}");
        app.stop();
    }

    /// "c" (or escape) leaves the conflict alone: no binding, no file.
    #[tokio::test]
    async fn cancelling_a_conflict_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, mut rx) = app_in_keybindings_dir(dir.path());
        app.handle_submit("/keymap");
        keymap_press(&mut app, &mut rx, "Cycle model", &["enter", "ctrl+r"]);
        press_on_overlay(&mut app, &mut rx, "c");
        assert_eq!(
            app.keybindings.actions_on_key("ctrl+r"),
            vec!["Browse sessions"]
        );
        assert_eq!(
            app.keybindings.actions_on_key("ctrl+p"),
            vec!["Cycle model"]
        );
        assert_eq!(keybindings_file(dir.path()), "<missing>");
        app.stop();
    }

    /// `ctrl+r` restores one action; the sentinel row restores every one, and
    /// the file goes back to an empty object so the next launch starts from the
    /// built-ins too.
    #[tokio::test]
    async fn restoring_defaults_rewrites_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(crate::keybindings::KEYBINDINGS_FILE),
            r#"{"Cycle model": "ctrl+y", "Browse sessions": ""}"#,
        )
        .unwrap();
        let (mut app, mut rx) = app_in_keybindings_dir(dir.path());
        assert!(
            app.keybindings.actions_on_key("ctrl+r").is_empty(),
            "unbound"
        );
        app.handle_submit("/keymap");

        // One action, through the chord.
        keymap_press(&mut app, &mut rx, "Cycle model", &["ctrl+r"]);
        assert_eq!(
            app.keybindings.actions_on_key("ctrl+p"),
            vec!["Cycle model"]
        );
        assert_eq!(
            keybindings_file(dir.path()),
            "{\n  \"Browse sessions\": \"\"\n}\n",
            "the other override is still there"
        );

        // Everything, through the row.
        keymap_press(
            &mut app,
            &mut rx,
            crate::components::keymap_view::RESET_ALL,
            &["enter"],
        );
        assert_eq!(
            app.keybindings.actions_on_key("ctrl+r"),
            vec!["Browse sessions"]
        );
        assert_eq!(keybindings_file(dir.path()), "{}\n");
        assert_eq!(app.keybindings.assignment_overrides(), Vec::new());
        app.stop();
    }

    /// A `fixed` action — the one the raw interrupt byte also answers — is
    /// refused with its reason, both on `enter` and on the reset chord.
    #[tokio::test]
    async fn a_fixed_action_is_refused_in_the_panel() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, mut rx) = app_in_keybindings_dir(dir.path());
        app.handle_submit("/keymap");
        keymap_press(&mut app, &mut rx, "Interrupt / exit", &["enter"]);
        assert!(!top_keymap_view(&mut app).is_capturing());
        let text = top_overlay_text(&mut app, 90);
        assert!(text.contains("raw interrupt byte"), "{text}");
        keymap_press(&mut app, &mut rx, "Interrupt / exit", &["ctrl+r"]);
        let text = top_overlay_text(&mut app, 90);
        assert!(text.contains("is fixed"), "{text}");
        assert_eq!(keybindings_file(dir.path()), "<missing>");
        app.stop();
    }

    /// A keymap command can outlive the panel it was asked for: the sink is a
    /// channel, the panel is closed (or has another overlay on top) while it is
    /// drained, and a command that the user cannot see is a change the user
    /// will not trust. The transcript is where it lands instead.
    #[tokio::test]
    async fn a_keymap_command_outliving_its_panel_reports_in_the_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _rx) = app_in_keybindings_dir(dir.path());
        assert!(app.top_keymap_overlay().is_none(), "no panel is open");

        app.handle_cmd(UiCmd::KeymapBind {
            description: "Cycle model".to_string(),
            key: "ctrl+y".to_string(),
        });
        assert_eq!(
            app.keybindings.actions_on_key("ctrl+y"),
            vec!["Cycle model"]
        );
        let last = app.chat.last_message().unwrap();
        assert_eq!(last.role, ChatRole::System);
        let said = last.content.clone();
        assert!(said.contains("Bound «Cycle model» to ctrl+y"), "{said}");

        // A description this build does not carry is refused here too, not
        // only in the panel: the command is a public entry point.
        app.handle_cmd(UiCmd::KeymapBind {
            description: "Ghost action".to_string(),
            key: "ctrl+l".to_string(),
        });
        let last = app.chat.last_message().unwrap();
        let refused = last.content.contains("is not an action in this build");
        assert!(refused, "the transcript says why: {:?}", last.content);
        assert_eq!(
            app.keybindings.actions_on_key("ctrl+l"),
            vec!["Clear screen / redraw"],
            "the key it asked for is untouched"
        );
        let written = "{\n  \"Cycle model\": \"ctrl+y\"\n}\n";
        assert_eq!(keybindings_file(dir.path()), written, "nothing was written");

        // The same for the reset side, which is a different command.
        app.handle_cmd(UiCmd::KeymapReset {
            description: Some("Ghost action".to_string()),
        });
        let last = app.chat.last_message().unwrap();
        let refused = last.content.contains("is not an action in this build");
        assert!(refused, "the transcript says why: {:?}", last.content);
        assert_eq!(keybindings_file(dir.path()), written);

        // A key no terminal can send never reaches the dispatcher, whatever the
        // caller is (`apply_keymap_binding` runs the panel's own check).
        app.handle_cmd(UiCmd::KeymapBind {
            description: "Cycle model".to_string(),
            key: "ctrl+c".to_string(),
        });
        let last = app.chat.last_message().unwrap();
        let refused = last.content.contains("interrupt byte");
        assert!(refused, "the transcript says why: {:?}", last.content);
        assert_eq!(
            app.keybindings.actions_on_key("ctrl+c"),
            vec!["Interrupt / exit"]
        );
        assert_eq!(
            app.keybindings.actions_on_key("ctrl+y"),
            vec!["Cycle model"]
        );
        app.stop();
    }

    /// A binding that only lives in this process is a lie the user would
    /// discover after a restart, so a write that fails is appended to the
    /// message instead of being dropped.
    #[tokio::test]
    async fn a_binding_that_cannot_be_written_says_so() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file belongs: `write` on it fails on every
        // platform, with no permissions to set up and nothing to clean up.
        std::fs::create_dir(dir.path().join(crate::keybindings::KEYBINDINGS_FILE)).unwrap();
        let (mut app, mut rx) = app_in_keybindings_dir(dir.path());
        app.handle_submit("/keymap");
        keymap_press(&mut app, &mut rx, "Cycle model", &["enter"]);
        press_on_overlay(&mut app, &mut rx, "ctrl+y");
        // The binding is live — the panel applied it before persisting it —
        // and the panel says the file did not take it.
        assert_eq!(
            app.keybindings.actions_on_key("ctrl+y"),
            vec!["Cycle model"]
        );
        let text = top_overlay_text(&mut app, 90);
        assert!(text.contains("could not write"), "{text}");
        assert!(
            text.contains(crate::keybindings::KEYBINDINGS_FILE),
            "{text}"
        );
        app.stop();
    }

    /// The panel's sink is the only translation from a keypress to a command:
    /// every payable action reaches the channel and the redraw-only ones stay
    /// silent.
    #[tokio::test]
    async fn keymap_sink_translates_every_action() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut sink = App::<FakeTerminal>::keymap_sink(tx);
        sink(KeymapAction::None);
        assert!(rx.try_recv().is_err(), "a redraw is not a command");
        sink(KeymapAction::Bind {
            description: "Cycle model".into(),
            key: "ctrl+y".into(),
        });
        assert!(matches!(
            rx.try_recv(),
            Ok(UiCmd::KeymapBind { description, key })
                if description == "Cycle model" && key == "ctrl+y"
        ));
        sink(KeymapAction::ResetAction("Cycle model".into()));
        assert!(matches!(
            rx.try_recv(),
            Ok(UiCmd::KeymapReset { description: Some(d) }) if d == "Cycle model"
        ));
        sink(KeymapAction::ResetAll);
        assert!(matches!(
            rx.try_recv(),
            Ok(UiCmd::KeymapReset { description: None })
        ));
        sink(KeymapAction::Cancelled);
        assert!(matches!(rx.try_recv(), Ok(UiCmd::OverlayCancel)));
    }

    /// `/keymap <something>` is a usage message, not a silently ignored panel
    /// open: the command takes no arguments.
    #[tokio::test]
    async fn keymap_arguments_are_rejected_with_usage() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _rx) = app_in_keybindings_dir(dir.path());
        app.handle_submit("/keymap reset");
        assert!(app.top_keymap_overlay().is_none(), "no panel opened");
        let last = app.chat.last_message().unwrap();
        assert!(last.content.contains("Usage: /keymap"), "{}", last.content);
    }

    // ─── pastes and image attachments on the wire ─────────────────────

    /// A paste long enough to fold, with an unmistakable tail so a test (or a
    /// pane capture) can tell the stored text from its placeholder.
    fn foldable_paste() -> String {
        let mut text = "P".repeat(1_000);
        text.push_str("\nPASTE-TAIL\n");
        assert!(text.chars().count() > crate::paste::FOLD_THRESHOLD);
        text
    }

    /// A real PNG on disk (magic bytes only — no test decodes it).
    fn png_fixture(dir: &tempfile::TempDir, name: &str) -> String {
        let path = dir.path().join(name);
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&[0u8; 24]);
        std::fs::write(&path, bytes).expect("write fixture");
        path.to_string_lossy().to_string()
    }

    /// Paste `text` the way the terminal delivers one, then submit with Enter —
    /// the whole path a user drives, not `handle_submit` directly.
    fn paste_into(app: &mut App<FakeTerminal>, text: &str) {
        app.handle_input(&format!("\x1b[200~{text}\x1b[201~"));
    }

    /// Type `text` the way the terminal delivers typing: one key at a time.
    /// A multi-character `handle_input` is not a thing — the app parses keys,
    /// so anything longer than one character has to arrive as a paste.
    fn type_text(app: &mut App<FakeTerminal>, text: &str) {
        for ch in text.chars() {
            app.handle_input(&ch.to_string());
        }
    }

    fn sent_prompts(
        requests: &std::sync::Arc<std::sync::Mutex<Vec<RpcCommand>>>,
    ) -> Vec<RpcCommand> {
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|cmd| cmd.r#type == "prompt")
            .cloned()
            .collect()
    }

    fn user_messages(app: &App<FakeTerminal>) -> Vec<String> {
        app.chat
            .plain_messages()
            .iter()
            .filter(|(role, _)| *role == ChatRole::User)
            .map(|(_, content)| content.clone())
            .collect()
    }

    /// A folded paste is one display unit in the box, and the *whole text* on
    /// the wire — the placeholder is never sent, and the transcript shows what
    /// was sent (the agent's own echo of the prompt has to match it, or the
    /// dedup would print the message twice).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_folded_paste_reaches_the_agent_as_the_whole_text() {
        let mock = AppMockAgent::default();
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();

        let pasted = foldable_paste();
        paste_into(&mut app, &pasted);
        // One placeholder, and none of the text in the box.
        assert_eq!(
            app.input.get_value(),
            crate::paste::placeholder_name(pasted.chars().count(), 1)
        );

        app.handle_input("\r");
        pump_until_agent_commands(&mut app, &mut rx, &requests, "prompt", 1).await;
        let prompts = sent_prompts(&requests);
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].message, pasted, "the wire gets the whole paste");
        assert!(prompts[0].attachments.is_empty());
        assert_eq!(user_messages(&app), vec![pasted.clone()]);
        // The draft is spent: the box is empty and holds nothing to expand.
        assert_eq!(app.input.get_value(), "");
    }

    /// The same round trip for an image: the box shows a marker, the wire
    /// carries a path attachment, and the message text keeps the marker the
    /// user saw (the agent's path manifest is what tells the model where it is).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pasted_image_path_travels_as_an_attachment() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = png_fixture(&dir, "shot.png");
        let mock = AppMockAgent::default();
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();

        type_text(&mut app, "what is this? ");
        paste_into(&mut app, &path);
        assert_eq!(app.input.get_value(), "what is this? [Image #1]");
        // The box says how many are waiting (the golden pins the pixels).
        let lines = app.input.render(80);
        assert_eq!(lines[0], "📎 1 image attached");

        app.handle_input("\r");
        pump_until_agent_commands(&mut app, &mut rx, &requests, "prompt", 1).await;
        let prompts = sent_prompts(&requests);
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].message, "what is this? [Image #1]");
        assert_eq!(prompts[0].attachments.len(), 1);
        assert_eq!(prompts[0].attachments[0].path, path);
        assert_eq!(prompts[0].attachments[0].kind, "image");
        assert_eq!(prompts[0].attachments[0].name, "shot.png");
        // …and the next submission starts clean: no attachment, no marker.
        assert!(app.input.take_pending().attachments().is_empty());
    }

    /// Nothing the user deleted reaches the model: a paste whose placeholder is
    /// gone is not expanded, and an image whose marker is gone is not attached.
    #[tokio::test(flavor = "multi_thread")]
    async fn deleted_placeholders_and_markers_never_reach_the_agent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let image = png_fixture(&dir, "shot.png");
        let mock = AppMockAgent::default();
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();

        let pasted = foldable_paste();
        paste_into(&mut app, &pasted);
        paste_into(&mut app, &format!(" {image}"));
        type_text(&mut app, "keep this");
        let placeholder = crate::paste::placeholder_name(pasted.chars().count(), 1);
        assert_eq!(
            app.input.get_value(),
            format!("{placeholder}{}keep this", crate::paste::image_marker(1))
        );

        // Delete the folded paste and the image marker, keep the typed tail.
        app.handle_key("ctrl+a");
        let doomed = placeholder.chars().count() + crate::paste::image_marker(1).chars().count();
        for _ in 0..doomed {
            app.handle_key("delete");
        }
        assert_eq!(app.input.get_value(), "keep this");

        app.handle_input("\r");
        pump_until_agent_commands(&mut app, &mut rx, &requests, "prompt", 1).await;
        let prompts = sent_prompts(&requests);
        assert_eq!(prompts[0].message, "keep this");
        assert!(prompts[0].attachments.is_empty());
        assert_eq!(user_messages(&app), vec!["keep this".to_string()]);
    }

    /// A model that cannot take images is named before the message goes out,
    /// because the agent degrades the image to a path and the user would
    /// otherwise believe the model saw a picture.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_image_sent_to_a_text_only_model_is_called_out() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = png_fixture(&dir, "shot.png");
        let mock = AppMockAgent::default();
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();
        // Startup already fetched the catalog, and the mock's models carry no
        // `modalities` — so every model it lists is text-only, which is the
        // case this test is about.
        assert_eq!(app.current_model_image_support(), Some(false));

        paste_into(&mut app, &path);
        app.do_render();
        let lines = app.input.render(80);
        assert!(lines[0].contains("cannot view images"), "{lines:?}");

        app.handle_input("\r");
        pump_until_agent_commands(&mut app, &mut rx, &requests, "prompt", 1).await;
        pump_until_msg(&mut app, &mut rx, "cannot view images").await;
        // Warned *and* sent: the agent still puts the path in front of the
        // model, which is worth more than refusing to attach it.
        let prompts = sent_prompts(&requests);
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].attachments.len(), 1);
        assert_eq!(prompts[0].attachments[0].path, path);
    }

    /// A folded paste and an image survive a session switch: the box's draft is
    /// cached per session, and a marker cached without the thing it stands for
    /// would come back as text that means nothing (and send nothing).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_switch_keeps_a_drafts_pastes_and_images_with_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = png_fixture(&dir, "shot.png");
        let mock = AppMockAgent::default();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();

        let pasted = foldable_paste();
        paste_into(&mut app, &pasted);
        paste_into(&mut app, &format!(" {path}"));
        let draft = app.input.get_value().to_string();
        assert!(draft.contains("[Image #1]"), "{draft}");

        // Away to another session, and back to this one — through the same
        // save/restore pair a real switch uses.
        app.save_session_input();
        app.state.session_id = "s0".into();
        app.restore_session_input();
        assert_eq!(app.input.get_value(), "");
        app.save_session_input();
        app.state.session_id = "s1".into();
        app.restore_session_input();
        assert_eq!(app.input.get_value(), draft);
        assert!(app.input.expanded(&draft).contains("PASTE-TAIL"));
        assert_eq!(app.input.take_pending().attachments().len(), 1);
        let _ = rx.try_recv();
    }

    /// A paste past the message cap is refused in the transcript — never
    /// truncated, never sent.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_oversized_paste_is_refused_with_a_reason() {
        let mock = AppMockAgent::default();
        let requests = mock.requests.clone();
        let (addr, _seen) = spawn_app_mock_with(mock).await;
        let (mut app, mut rx) = make_app_at(&addr, &CliOptions::default());
        app.start(mpsc::unbounded_channel().0).await.unwrap();

        let huge = "x".repeat(crate::paste::MAX_MESSAGE_CHARS + 1);
        paste_into(&mut app, &huge);
        pump_until_msg(&mut app, &mut rx, "exceeds the maximum length").await;
        assert_eq!(app.input.get_value(), "");
        assert!(sent_prompts(&requests).is_empty());
        // The phrasing names both numbers, so the user can act on it.
        let notice = last_system(&app);
        assert!(notice.contains("100000 characters"), "{notice}");
        assert!(notice.contains("100001 provided"), "{notice}");
    }

    /// The cap is checked again at submission, so a draft that assembles past
    /// it (the same placeholder copied twice) is refused rather than billed.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_draft_that_assembles_past_the_cap_is_refused_at_submit() {
        let (mut app, _rx) = make_app(100, 30);
        app.input.insert_text(&"y".repeat(60_000));
        let placeholder = crate::paste::placeholder_name(60_000, 1);
        assert_eq!(app.input.get_value(), placeholder);
        // A hand-copied placeholder names the same stored text twice, so the
        // message would assemble to more than the cap while the box stays small.
        let draft = format!("{placeholder} {placeholder}");
        app.input.set_value(&draft, None);
        assert_eq!(app.input.expanded(&draft).chars().count(), 120_001);

        app.handle_submit(&draft);
        let notice = last_system(&app);
        assert!(notice.contains("exceeds the maximum length"), "{notice}");
        // The draft is still there (nothing was sent, nothing was lost)…
        assert_eq!(app.input.get_value(), draft);
        // …and the text it stands for is still expandable.
        assert_eq!(app.input.expanded(&draft).chars().count(), 120_001);
    }

    // ─── ctrl+v: reading the clipboard ─────────────────────────────────

    /// Programs an injected clipboard reader was asked to run.
    type ClipboardPrograms = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

    /// The path a macOS image probe was told to write (`POSIX file "<path>"`).
    fn clipboard_probe_target(args: &[String]) -> PathBuf {
        let joined = args.join("\n");
        let marker = "POSIX file \"";
        let start = joined
            .find(marker)
            .unwrap_or_else(|| panic!("no target in {joined}"))
            + marker.len();
        let rest = &joined[start..];
        let end = rest.find('"').expect("the literal is closed");
        PathBuf::from(&rest[..end])
    }

    /// The platform the clipboard-paste tests pin: macOS, whose probe commands
    /// (`osascript`/`pbpaste`) are the names the scripted clipboard answers.
    /// Pinning keeps the assertions identical on a Linux CI host, where the
    /// real probes would be `wl-paste`/`xclip` and the assertions would not
    /// hold.
    const CLIPBOARD_TEST_OS: &str = "macos";

    /// A clipboard reader backed by a scripted runner: the image probe writes
    /// `bytes` to the path it names (nothing for empty `bytes`), the text tool
    /// answers `text`. No test here ever runs a real clipboard program.
    fn scripted_clipboard(
        dir: &std::path::Path,
        bytes: &[u8],
        text: &str,
    ) -> (crate::paste::ClipboardCapture, ClipboardPrograms) {
        let programs: ClipboardPrograms = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&programs);
        let bytes = bytes.to_vec();
        let text = text.to_string();
        let capture = crate::paste::ClipboardCapture::with_runner(
            Box::new(move |program: &str, args: &[String]| {
                sink.lock().unwrap().push(program.to_string());
                if program == "osascript" {
                    if !bytes.is_empty() {
                        std::fs::write(clipboard_probe_target(args), &bytes)
                            .map_err(|error| error.to_string())?;
                    }
                    return Ok((0, String::new(), String::new()));
                }
                Ok((0, text.clone(), String::new()))
            }),
            dir.to_path_buf(),
        );
        (capture, programs)
    }

    /// A real PNG, magic bytes included.
    fn clipboard_png() -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"rest of the file");
        bytes
    }

    /// The programs a scripted clipboard ran.
    fn clipboard_programs(programs: &ClipboardPrograms) -> Vec<String> {
        programs.lock().unwrap().clone()
    }

    #[tokio::test]
    async fn ctrl_v_attaches_the_image_on_the_clipboard() {
        let (mut app, _rx) = make_app(100, 30);
        let dir = tempfile::tempdir().expect("tempdir");
        let (capture, programs) = scripted_clipboard(dir.path(), &clipboard_png(), "never read");
        app.clipboard_capture = capture;

        app.paste_clipboard_for_os(CLIPBOARD_TEST_OS);

        // The image became an attachment behind a marker — the P1 mechanism,
        // numbering and all — and its file is still where the agent will read
        // it after the prompt goes out.
        assert_eq!(app.input.get_value(), "[Image #1]");
        let pending = app.input.take_pending();
        assert_eq!(pending.attachments().len(), 1);
        let attachment = &pending.attachments()[0];
        assert_eq!(attachment.kind, "image");
        assert!(attachment.path.ends_with(".png"), "{}", attachment.path);
        assert!(
            std::path::Path::new(&attachment.path).is_file(),
            "{}",
            attachment.path
        );
        assert_eq!(attachment.name, attachment.path.rsplit('/').next().unwrap());
        // The image answered the question, so the text tool was never asked, and
        // nothing was reported to the user.
        assert_eq!(clipboard_programs(&programs), vec!["osascript"]);
        assert!(
            system_messages(&app).is_empty(),
            "{:?}",
            system_messages(&app)
        );
    }

    #[tokio::test]
    async fn super_v_pastes_too_because_that_is_what_a_command_press_arrives_as() {
        // Upstream of a Kitty-protocol terminal a Command press is reported as
        // `super+v`; the terminal that keeps Command for its own Paste menu
        // never sends anything at all. Binding the spelling that arrives is the
        // only way the key can honestly be offered on macOS.
        let (mut app, _rx) = make_app(100, 30);
        let dir = tempfile::tempdir().expect("tempdir");
        let (capture, programs) = scripted_clipboard(dir.path(), &clipboard_png(), "never read");
        app.clipboard_capture = capture;

        app.paste_clipboard_for_os(CLIPBOARD_TEST_OS);

        assert_eq!(app.input.get_value(), "[Image #1]");
        assert_eq!(app.input.take_pending().attachments().len(), 1);
        assert_eq!(clipboard_programs(&programs), vec!["osascript"]);
    }

    #[tokio::test]
    async fn a_user_binding_on_super_v_wins_over_the_clipboard() {
        // Same contract as ctrl+v: the clipboard is the default, not an
        // override the user cannot take back.
        let (mut app, _rx, _dir) = app_with_keybindings(r#"{"Cycle model": "super+v"}"#);
        let dir = tempfile::tempdir().expect("tempdir");
        let (capture, programs) = scripted_clipboard(dir.path(), &clipboard_png(), "never read");
        app.clipboard_capture = capture;

        app.handle_key("super+v");

        assert!(
            clipboard_programs(&programs).is_empty(),
            "the clipboard must not be read when the user bound the key"
        );
    }

    #[tokio::test]
    async fn ctrl_v_with_text_on_the_clipboard_inserts_the_text() {
        let (mut app, _rx) = make_app(100, 30);
        let dir = tempfile::tempdir().expect("tempdir");
        // No image: the probe answers success but writes nothing (a text-only
        // clipboard), which is the ordinary case, not a failure.
        let (capture, programs) = scripted_clipboard(dir.path(), &[], "a line from the clipboard");
        app.clipboard_capture = capture;

        app.paste_clipboard_for_os(CLIPBOARD_TEST_OS);

        assert_eq!(app.input.get_value(), "a line from the clipboard");
        assert!(app.input.take_pending().attachments().is_empty());
        assert_eq!(clipboard_programs(&programs), vec!["osascript", "pbpaste"]);
        assert!(system_messages(&app).is_empty(), "no notice for plain text");
    }

    #[tokio::test]
    async fn ctrl_v_with_nothing_to_paste_says_so_and_keeps_the_draft() {
        let (mut app, _rx) = make_app(100, 30);
        app.input.set_value("half typed", None);
        let dir = tempfile::tempdir().expect("tempdir");
        let (capture, _programs) = scripted_clipboard(dir.path(), &[], "");
        app.clipboard_capture = capture;

        app.handle_key(Key::CTRL_V);

        // The draft is untouched and the refusal is readable in the transcript.
        assert_eq!(app.input.get_value(), "half typed");
        let messages = system_messages(&app);
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert!(
            messages[0].contains("Clipboard paste failed"),
            "{messages:?}"
        );
        assert!(
            messages[0].contains("neither an image nor text"),
            "{messages:?}"
        );
    }

    #[tokio::test]
    async fn ctrl_v_with_no_clipboard_tool_reports_the_tool() {
        let (mut app, _rx) = make_app(100, 30);
        let capture = crate::paste::ClipboardCapture::with_runner(
            Box::new(|program: &str, _args: &[String]| {
                Err(format!(
                    "failed to spawn {program}: No such file or directory"
                ))
            }),
            tempfile::tempdir().expect("tempdir").path().to_path_buf(),
        );
        app.clipboard_capture = capture;

        app.paste_clipboard_for_os(CLIPBOARD_TEST_OS);

        assert!(app.input.get_value().is_empty());
        let message = last_system(&app);
        assert!(message.contains("failed to spawn pbpaste"), "{message}");
    }

    #[tokio::test]
    async fn ctrl_v_with_an_overlay_open_belongs_to_the_overlay() {
        let (mut app, _rx) = make_app(100, 30);
        let dir = tempfile::tempdir().expect("tempdir");
        let (capture, programs) = scripted_clipboard(dir.path(), &clipboard_png(), "text");
        app.clipboard_capture = capture;
        app.handle_submit("/help");
        assert!(!app.overlay_stack.is_empty(), "the card is open");

        app.handle_key(Key::CTRL_V);

        // A panel owns the keyboard: the clipboard is not even read, and nothing
        // reaches the input box behind it.
        assert!(clipboard_programs(&programs).is_empty());
        assert!(app.input.get_value().is_empty());
        assert!(
            system_messages(&app).is_empty(),
            "{:?}",
            system_messages(&app)
        );
    }

    #[tokio::test]
    async fn a_user_binding_on_ctrl_v_wins_over_the_clipboard() {
        let (mut app, _rx, _dir) = app_with_keybindings(r#"{"Cycle model": "ctrl+v"}"#);
        let dir = tempfile::tempdir().expect("tempdir");
        let (capture, programs) = scripted_clipboard(dir.path(), &clipboard_png(), "text");
        app.clipboard_capture = capture;
        assert!(
            !app.keybindings.actions_on_key(Key::CTRL_V).is_empty(),
            "the key is bound for this test to mean anything"
        );

        app.handle_key(Key::CTRL_V);

        assert!(clipboard_programs(&programs).is_empty());
        assert!(app.input.get_value().is_empty());
    }
}
