//! The agent adapter: fills [`WorkbenchData`] from a real session and applies
//! the surface's intents back to the agent.
//!
//! This is the other half of the workbench contract. The surface (`screen`,
//! `panels`, `launchpad`, …) paints from [`WorkbenchData`] and returns
//! [`WbAction`]s; everything about *where the data comes from* lives here, so a
//! drawing bug and a wiring bug can never be confused for each other.
//!
//! It reuses the leaves the inline TUI already has, deliberately:
//!
//! * [`ChatArea`] renders the conversation — markdown, tool rows, thinking and
//!   the final-answer fold are the same code, so a reply looks identical in both
//!   surfaces.
//! * [`crate::session_rows::rows_to_messages`] maps journal entries to messages,
//!   including its structural rules (an empty `tool_call` step is not a message).
//! * [`GrpcClient`] talks to the agent; this module never touches proto or a
//!   socket directly.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;

use crate::components::chat_area::{ChatArea, ChatMessage, ChatRole, ReplyDetail, RunState};
use crate::paste;
use crate::rpc::types::{
    AgentEvent, ModelInfo, PendingApprovalState, RpcSessionState, SessionEntriesPage,
    SessionSummary,
};
use crate::rpc::GrpcClient;

use super::i18n::{self, t, Key, Language, LanguagePreference};
use super::layout::Frame;
use super::state::{
    effective_thinking_levels, legacy_wire_thinking_level, model_reference, next_thinking_level,
    Activity, ActivityKind, InputView, ModelChoice, PreviewView, QueuedPrompt, SessionMeta,
    SessionRow, SkillRow, UsageData, WorkbenchData, WorkspaceGroup, DEFAULT_THINKING_LEVEL_NAME,
};
use super::WbAction;

/// What an applied action asks the driver loop to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    Quit,
}

/// A request the agent is parked on, waiting for a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalPrompt {
    pub request_id: String,
    pub title: String,
    pub summary: String,
    /// Full agent payload: risk, action, boundary and suggested allow rules.
    pub raw: Value,
}

/// Which session this adapter is attached to, and how much history is loaded.
#[derive(Debug, Clone, Default)]
struct Paging {
    session_id: String,
    /// Older rows exist above what is loaded.
    has_more: bool,
    /// Exclusive backward cursor for the next older page.
    next_offset: i64,
}

/// The palette verbs the *app* carries out — through the agent (`/new`, a
/// `/model` with an argument, `/export`) or in the process (`/quit`).
///
/// Declared here, beside the `command()` match that implements them, so
/// `every_palette_entry_has_a_handler` can fail when a palette entry has no
/// handler at all: the palette ([`super::COMMANDS`]), the pickers
/// ([`super::picker::PickerKind::from_command`]), the surface's own verbs
/// ([`super::Workbench::surface_command`]) and this list have to account for
/// every entry exactly once. That is the check whose absence let five entries
/// advertise themselves and then answer `Unknown command`.
pub const APP_COMMANDS: [&str; 21] = [
    "/new",
    "/reply-detail",
    "/tool-output",
    "/export",
    "/quit",
    "/copy",
    "/paste",
    "/preview",
    "/refresh",
    "/mouse",
    "/compact",
    "/tools",
    "/providers",
    "/diff",
    "/git",
    "/files",
    "/approval",
    "/history",
    "/tree",
    "/fork",
    "/worktree",
];

#[derive(Clone)]
struct DraftSnapshot {
    editor: super::editor::EditorState,
    pending: crate::components::input::PendingDraft,
}

/// How many conversations the adapter keeps ready to paint.
///
/// The rail is a list the reader walks with `shift+↑/↓`, so the conversation
/// they come back to is almost always one of the last few. Eight covers a
/// there-and-back over a window's worth of neighbours and costs one transcript's
/// worth of rows each (a page is ten exchanges; the rendered lines dominate).
const CACHED_SESSIONS: usize = 8;

/// A conversation that has been on screen, kept so that coming back to it is a
/// repaint instead of a round trip.
///
/// The rows are the expensive half of a switch and the half that does not
/// change: a cached `ChatArea` holds what the reader last read, so a switch
/// that hits one paints immediately and then reconciles in the background
/// (`Live::finish_switch`). `page` is the tail page those rows were built from
/// — the reconcile compares it against the page the agent returns and keeps the
/// rows when nothing moved.
struct CachedSession {
    chat: ChatArea,
    paging: Paging,
    state: RpcSessionState,
    page: SessionEntriesPage,
}

/// The tail page the rows on screen were built from, and the conversation it
/// belongs to.
///
/// The id is not redundant with the header's: a client with no session (the
/// launchpad, or a `future tui` that attached to nothing) loads an empty page for
/// an empty id while `get_state` still names a session, and caching those rows
/// under that name would bring the reader back to a blank transcript.
#[derive(Clone)]
struct ShownPage {
    session_id: String,
    page: SessionEntriesPage,
}

/// One attached session plus everything the surface needs to paint it.
pub struct Live {
    client: Arc<GrpcClient>,
    chat: ChatArea,
    state: RpcSessionState,
    paging: Paging,
    /// Conversations the reader has already been shown, oldest first (see
    /// [`CachedSession`]).
    cached: VecDeque<(String, CachedSession)>,
    /// The tail page the transcript on screen was built from, if it is known.
    ///
    /// A switch fetches the same page again and compares: unchanged means the
    /// rows on screen are still the conversation's, so they are kept (that is
    /// what makes a cached conversation cost no re-render at all); changed means
    /// the conversation moved on while the reader was away and the transcript is
    /// rebuilt from the new page.
    shown_page: Option<ShownPage>,
    sessions: SessionList,
    /// Bumped when the pinned workspaces change (a pin reorders and re-files the
    /// 工作区 tab, so the view it feeds is stale from that moment).
    pins_rev: u64,
    /// The rail's rows and filing, built for the key below.
    session_view: Option<SessionView>,
    /// How many times [`Self::build_session_view`] has run — the meter its
    /// contract is measured with (see [`Self::session_view_builds`]).
    session_view_builds: u64,
    models: Vec<ModelInfo>,
    approval: Option<ApprovalPrompt>,
    diff: Option<super::state::DiffView>,
    fork_points: Vec<super::state::ForkPoint>,
    worktrees: Vec<super::state::WorktreeView>,
    /// The `/git` panel's content, once it has been read for this workspace.
    git: Option<super::state::GitView>,
    /// A failed request or command, shown above the status bar until the next
    /// successful action replaces it.
    pub notice: Option<String>,
    activity: Activity,
    activity_start: Option<Instant>,
    /// The tool calls this run has made, by the agent's own call id — what the
    /// row's `· {n} 工具` counts.
    ///
    /// A set, because the agent announces one call twice: `phase: "input"`
    /// while its arguments stream, then `phase: "execution"` when it runs
    /// (`agent/src/rpc/prompt_helpers.rs`). Counting the events made the row
    /// claim twice the calls the transcript beneath it showed — 53 for the 26
    /// that ran.
    run_tools: HashSet<String>,
    draft: InputView,
    /// Drafts are local to this client, one per session and launchpad.
    drafts: HashMap<String, DraftSnapshot>,
    revision: u64,
    mouse_capture: bool,
    /// Recent runs this client submitted, awaiting a terminal state. Bounded
    /// because cancellation and incomplete streams need not produce agent_end.
    completion_candidates: std::collections::VecDeque<String>,
    detail: ReplyDetail,
    /// Palette id, so a `/theme` cycle does not need the surface's own state.
    pub theme_id: String,
    /// The language the adapter's own sentences are written in (the input
    /// hint, an approval prompt, an export notice). The surface keeps its own
    /// copy of the resolved language; this one is for the data it hands over.
    lang: Language,
    /// The surface is on the launchpad: `/new` detached from every session
    /// without creating one, so the header's own state (connection, version)
    /// stays and the first message creates the session the reader wanted.
    launchpad: bool,
    /// The model and reasoning level chosen while nothing was attached. The
    /// agent cannot be told `set_model` without a session, so the picker (and
    /// the launchpad's own chips) stage the choice here instead: the header
    /// shows it, and the session the next message creates is asked for with it.
    /// Cleared once that session exists — from then on the agent is the source.
    pending_model: Option<String>,
    pending_effort: Option<String>,
    /// The approval mode chosen while nothing was attached. `set_sandbox_policy`
    /// needs a session like the other two setters, so the launchpad's chip and
    /// the `/mode` menu stage it here and the session the next message creates
    /// is told once it exists.
    pending_mode: Option<String>,
    /// The workspace the next session will run in, chosen on the launchpad.
    /// Nothing is sent: the agent has no session to set a directory on until
    /// the first message creates one.
    pending_cwd: Option<String>,
    /// When the current run started and how many output tokens it has reported:
    /// tok/s is the two divided. Cleared at `agent_start`, frozen at
    /// `agent_end` so an idle surface still shows the turn it just finished.
    run_started: Option<Instant>,
    run_output_tokens: u64,
    run_tps: f64,
    /// A `usage`/`agent_end` event asked for the agent's cumulative usage and
    /// the driver has not fetched it yet. The surface is synchronous and the
    /// fetch is not, so the event sets the flag and the driver loop drains it
    /// (`take_state_refresh`) — that is what makes the footer move after a turn
    /// instead of only when someone presses ctrl+k.
    state_refresh_pending: bool,
    /// The workspace the local facts below were read for. A worktree move is the
    /// one thing that has to re-read the working directory's files, so the menu
    /// follows the session instead of the process's own directory.
    local_cwd: String,
    /// The git branch of `local_cwd`, read once per workspace (see
    /// [`super::git`]): the header names it after the cwd, and a per-frame
    /// `git` spawn would be a stutter on a large repository.
    branch: super::git::BranchCache,
    /// The installed skills (machine-wide: read once, then reused for every
    /// frame), which the `/` palette offers beside its commands.
    local_skills: Option<Vec<SkillRow>>,
    /// Workspace paths the reader pinned in the rail (the 置顶 section), as
    /// `~/.future/tui/settings.json` last recorded them.
    pinned_workspaces: Vec<String>,
    /// The `@` menu's candidates for `local_cwd`, read once per directory: a
    /// listing per keystroke would be a `git ls-files` per keystroke. Empty
    /// until [`LocalReads`] answers for the directory in force.
    files: Vec<String>,
    /// The process-bound reads for `local_cwd` — the `@` menu's files and the
    /// header's branch — running off the frame (see [`LocalReads`]).
    local_reads: Option<LocalReads>,
    /// The 文件 panel's tree, rooted at the session workspace. Scanned when the
    /// directory moves (beside the frame, see [`LocalReads`]) and when the reader
    /// asks for it — never per frame.
    tree: crate::workbench::files::FileTree,
    /// What the preview box is holding, if anything.
    preview: Option<PreviewView>,
    /// The clipboard reader. Holds the directory its temporary images go to; a
    /// capture that finds no image leaves nothing behind.
    clipboard: paste::ClipboardCapture,
    /// The header's dot: the event stream's latest answer, or a successful
    /// startup read when this client has not subscribed yet (a launchpad owns
    /// no session to subscribe to).
    connected: bool,
    /// The gRPC stream's own edges, watched so a disconnect reaches the dot.
    connection: tokio::sync::watch::Receiver<bool>,
    /// False once the watch's sender is gone, so a closed channel cannot wake
    /// the driver in a tight loop.
    connection_open: bool,
    /// One preview being read and decoded on a blocking thread. Starting a
    /// second load replaces it; an answer for an older request is dropped.
    preview_read: Option<(u64, std::sync::mpsc::Receiver<PreviewLoaded>)>,
    preview_revision: u64,
    /// The one file-tree read in flight, keyed by the revision that asked for
    /// it. An answer for an older request is dropped.
    tree_read: Option<(u64, std::sync::mpsc::Receiver<super::files::FileTree>)>,
    tree_revision: u64,
    /// The siblings a finished preview brought, waiting for the driver to hand
    /// them to the preview box (`take_preview_siblings`).
    preview_siblings: Option<(Vec<String>, usize)>,
}

struct PreviewLoaded {
    revision: u64,
    view: PreviewView,
    siblings: (Vec<String>, usize),
}

impl Live {
    /// Attach to the agent and load the session's tail plus the lists the
    /// surface shows. `session` names a session to restore; `None` keeps
    /// whatever the agent has current.
    pub async fn attach(
        addr: &str,
        session: Option<&str>,
        no_context_files: bool,
        settings: Option<&crate::settings::TuiSettings>,
    ) -> Result<(Self, tokio::sync::mpsc::UnboundedReceiver<AgentEvent>), String> {
        let (client, events, connection) = GrpcClient::new(addr);
        let client = Arc::new(client.with_no_context_files(no_context_files));

        // The reading level is not persisted: every terminal starts folded, and
        // `ctrl+e` expands for the run it is pressed in. See `ReplyDetail`.
        let detail = ReplyDetail::default();
        // The stored preference decides the adapter's language before anything
        // is painted, so the first frame is already in the reader's language.
        let lang = LanguagePreference::from_wire(
            settings
                .and_then(|settings| settings.language.as_deref())
                .unwrap_or("system"),
        )
        .resolve();
        let theme = settings
            .map(persisted_palette)
            .unwrap_or(super::theme::FUTURE_DARK);
        let mouse_capture = settings.and_then(|s| s.mouse_capture).unwrap_or(true);
        let mut chat = ChatArea::new(80, None);
        chat.set_reply_detail(detail);

        let mut live = Self {
            client,
            chat,
            state: RpcSessionState::default(),
            paging: Paging::default(),
            cached: VecDeque::new(),
            shown_page: None,
            sessions: SessionList::default(),
            pins_rev: 0,
            session_view: None,
            session_view_builds: 0,
            models: Vec::new(),
            approval: None,
            diff: None,
            fork_points: Vec::new(),
            worktrees: Vec::new(),
            git: None,
            notice: None,
            activity: Activity::default(),
            activity_start: None,
            run_tools: HashSet::new(),
            draft: InputView::from_editor(
                super::editor::EditorState::default(),
                t(lang, Key::InputHintLaunchpad).into(),
            ),
            drafts: HashMap::new(),
            revision: 0,
            mouse_capture,
            completion_candidates: std::collections::VecDeque::new(),
            detail,
            theme_id: theme.id.to_string(),
            lang,
            launchpad: false,
            pending_model: None,
            pending_effort: None,
            pending_mode: None,
            pending_cwd: None,
            run_started: None,
            run_output_tokens: 0,
            run_tps: 0.0,
            state_refresh_pending: false,
            local_cwd: String::new(),
            branch: super::git::BranchCache::new(),
            local_skills: None,
            pinned_workspaces: settings
                .and_then(|settings| settings.pinned_workspaces.clone())
                .unwrap_or_default(),
            files: Vec::new(),
            local_reads: None,
            tree: crate::workbench::files::FileTree::default(),
            preview: None,
            clipboard: paste::ClipboardCapture::default(),
            connection,
            connected: false,
            connection_open: true,
            preview_read: None,
            preview_revision: 0,
            tree_read: None,
            tree_revision: 0,
            preview_siblings: None,
        };

        if let Some(id) = session.filter(|id| !id.is_empty()) {
            live.client.switch_session(id).await?;
        }
        // The first screen waits for these three reads. They do not depend on
        // one another, so they go out together instead of costing three
        // deadlines in sequence on a slow link.
        let (state, sessions, models) = tokio::join!(
            live.client.get_state(),
            live.client.list_sessions(),
            live.client.list_models()
        );
        if let Ok(state) = state {
            live.install_state(state);
            live.connected = true;
        }
        if let Ok(sessions) = sessions {
            live.sessions.replace(sessions);
            live.connected = true;
        }
        if let Ok(models) = models {
            live.models = models;
            live.connected = true;
        }
        // A conversation that is already writing when the terminal attaches
        // owns the row from the first frame (see [`Live::adopt_run`]).
        live.adopt_run();
        live.load_tail().await;
        Ok((live, events))
    }

    // ─── loading ──────────────────────────────────────────────────────

    /// Ask the agent for the session state — the single source of model,
    /// thinking level, permission, usage and the approvals it still holds.
    pub async fn refresh_state(&mut self) {
        let Ok(state) = self.client.get_state().await else {
            return;
        };
        self.install_state(state);
    }

    /// Put one `get_state` answer on screen, and point the client at the
    /// session it names.
    ///
    /// Split from [`Self::refresh_state`] because a switch reads the state of
    /// the conversation it is opening in the same round trip as that
    /// conversation's page (see [`Self::finish_switch`]) and then installs it
    /// here — one place that decides what a state answer means, so the two
    /// paths cannot drift.
    fn install_state(&mut self, state: RpcSessionState) {
        // A `get_state` naming a live approval is the reconnect path: the prompt
        // has to come back, or nothing but another client could free the parked
        // run.
        if self.approval.is_none() {
            if let Some(pending) = state.pending_approvals.first() {
                self.approval = Some(prompt_of(pending));
            }
        }
        // The client's stream manager subscribes to *its* current session and
        // refuses to subscribe at all while that is empty (an empty id would
        // leak every session's events). Attaching to a session that already
        // exists — which is the normal `future tui` start — therefore has to
        // publish the id, not just paint it: without this line the transcript
        // loads from history and then never moves again, because no run's
        // `text_chunk` was ever subscribed to (measured against the mock agent:
        // the reply never arrived and the activity row sat on 思考 forever).
        if !state.session_id.is_empty() && self.client.get_current_session_id() != state.session_id
        {
            self.client.set_current_session_id(&state.session_id);
        }
        self.state = state;
        self.refresh_local();
    }

    /// Read the panels' local facts for the workspace the session is in.
    ///
    /// The todo list belongs to the *session's* cwd, not to the directory the
    /// TUI was started in: attaching to a conversation that ran in
    /// `.worktrees/<name>` must show that checkout's goal, and moving to another
    /// workspace must re-read rather than keep painting the old one's list.
    fn refresh_local(&mut self) {
        // The directory in force — the staged one on the launchpad, the session's
        // own once one exists. A staged workspace the local facts did not follow
        // made `@` list (and `@a-file` expand against) the directory `/new` left
        // behind, while the chip and the session that got created named another.
        let cwd = self.shown_cwd().unwrap_or_default();
        self.local_cwd = cwd;
        // The workspace's process-bound facts belong to the directory, not to the
        // session, and they are read on their own threads: a workspace change
        // used to spawn `git ls-files` (24 ms in this repository) and
        // `git rev-parse` (up to `BRANCH_TIMEOUT` when a repository is locked)
        // *inside the driver loop*, so the first frame after a switch to another
        // workspace waited for them. Nothing of the previous workspace may stay
        // on screen while the answer is out: the files and the branch go with it.
        if self.local_reads.as_ref().map(|reads| reads.cwd.as_str())
            != Some(self.local_cwd.as_str())
        {
            self.files = Vec::new();
            // The 文件 panel's tree is a directory listing too, and a directory
            // with thousands of entries in it (`/tmp`) takes as long as the git
            // read this replaced. It arrives with the listing.
            self.tree = super::files::FileTree::default();
            self.local_reads = Some(LocalReads::start(&self.local_cwd));
        }
        if self.local_skills.is_none() {
            self.local_skills = Some(super::local::installed_skills());
        }
    }

    /// Install whatever a workspace's background reads have answered, and answer
    /// whether anything arrived — which is when the driver owes a frame.
    ///
    /// Called from the loop's clock (see `run::TICK`), never from `data`: one
    /// place decides that a frame is owed, and a read that lands between two
    /// keystrokes still reaches the screen.
    pub fn poll_local_reads(&mut self) -> bool {
        let Some(reads) = self.local_reads.as_mut() else {
            return self.poll_tree_read();
        };
        let cwd = reads.cwd.clone();
        let mut landed = false;
        if let Some(rx) = reads.branch.take() {
            match rx.try_recv() {
                Ok(branch) => {
                    self.branch.install(&cwd, branch);
                    landed = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => reads.branch = Some(rx),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
            }
        }
        if let Some(rx) = reads.files.take() {
            match rx.try_recv() {
                Ok((files, tree)) => {
                    self.files = files;
                    // An explicit tree read the reader started while this
                    // workspace scan was running is newer: do not let the
                    // slower startup answer paint over it.
                    if self.tree_read.is_none() {
                        self.tree = tree;
                    }
                    landed = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => reads.files = Some(rx),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
            }
        }
        landed || self.poll_tree_read()
    }

    fn poll_tree_read(&mut self) -> bool {
        let Some((revision, rx)) = self.tree_read.as_ref() else {
            return false;
        };
        let revision = *revision;
        match rx.try_recv() {
            Ok(tree) => {
                self.tree_read = None;
                if revision != self.tree_revision {
                    return false;
                }
                if let Some(error) = tree.error.clone() {
                    self.notice = Some(error);
                }
                self.tree = tree;
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.tree_read = None;
                false
            }
        }
    }

    /// Re-read the conversation list, and answer whether anything the rail
    /// paints changed — a row arrived, an age moved, a run started or stopped.
    ///
    /// The answer is what lets the driver poll (see `run::SESSIONS_POLL`)
    /// without repainting a list that is already on screen: only a real change
    /// owes a frame.
    pub async fn refresh_sessions(&mut self) -> bool {
        let Ok(sessions) = self.client.list_sessions().await else {
            return false;
        };
        self.sessions.replace(sessions)
    }

    /// Is any listed conversation still writing? The rail's spinner animates on
    /// it, and the driver keeps the list warm while it holds.
    pub fn sessions_streaming(&self) -> bool {
        self.sessions
            .iter()
            .any(|session| session.is_streaming.unwrap_or(false))
    }

    pub async fn refresh_models(&mut self) {
        if let Ok(models) = self.client.list_models().await {
            self.models = models;
        }
    }

    /// The rows and the 工作区 filing a frame paints, cloned out of the cached
    /// view (see [`SessionView`]).
    ///
    /// A frame asks for these on every paint, and rebuilding them is the one
    /// part of a frame that grows with the reader's history: sorting, naming and
    /// filing every conversation. It used to be quadratic — 0.5 ms per frame at
    /// a hundred conversations and 10 ms at five hundred, measured — and the
    /// driver composes a frame's data several times per keystroke, so the view
    /// is built when one of its inputs moves and cloned otherwise (see
    /// `docs/testing/tui-session-rail-frame-cost-2026-10-27.md`).
    fn session_view_parts(&mut self) -> (Vec<SessionRow>, Vec<WorkspaceGroup>, Vec<String>) {
        let key = SessionViewKey {
            sessions: self.sessions.rev(),
            pins: self.pins_rev,
            current: self.state.session_id.clone(),
            launchpad: self.launchpad,
            lang: self.lang,
            // `age` reads in minutes: a view built in another minute may print a
            // different age for the same rows, so the minute is an input.
            minute: now_ms() / 60_000,
        };
        if self.session_view.as_ref().map(|view| &view.key) != Some(&key) {
            let view = self.build_session_view(key);
            self.session_view = Some(view);
        }
        let view = self.session_view.as_ref();
        match view {
            Some(view) => (
                view.rows.clone(),
                view.workspaces.clone(),
                view.chats.clone(),
            ),
            // Unreachable: the block above always leaves a view for this key.
            // A missing one paints an empty rail instead of panicking a
            // terminal, and the next frame rebuilds it.
            None => (Vec::new(), Vec::new(), Vec::new()),
        }
    }

    /// How many times the rail's view has been rebuilt — the count a frame pays
    /// only when one of [`SessionViewKey`]'s inputs moves.
    ///
    /// Exists for the contract's test: the view is the one part of a frame that
    /// grows with the reader's history, and a change that makes a frame rebuild
    /// it every time (a key field, a missing input) costs milliseconds per
    /// keystroke at a few hundred conversations without failing anything else.
    pub fn session_view_builds(&self) -> u64 {
        self.session_view_builds
    }

    /// Build the rail's view for `key`: the rows most recently spoken first, and
    /// their 工作区 filing.
    fn build_session_view(&mut self, key: SessionViewKey) -> SessionView {
        self.session_view_builds = self.session_view_builds.saturating_add(1);
        // The rail's list is the desktop's recent view: most recently spoken
        // first. Only the `活跃` stretch is re-ordered after this, by when each
        // conversation started (`panels::rows_with`) — an activity stamp moves
        // every second while a run streams, so ordering *that* stretch by it
        // would shuffle the rows under the reader, and a run would jump to the
        // top of a list the reader is walking with the arrow keys.
        //
        // `updated_at_ms` is not "when I was last here" — it is the newest
        // entry's stamp, and a `session_info` snapshot is an entry, so a storage
        // migration that rewrites every snapshot restamps the whole list at
        // once. That is why the running stretch, the one a reader watches, does
        // not depend on it.
        let mut sessions: Vec<&SessionSummary> = self.sessions.iter().collect();
        sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at_ms));
        // One read of the clock and one of the app's own directories: `age` and
        // `is_chat_directory` used to resolve both per row, which put a
        // `SystemTime::now` and two `format!`s on every conversation in the list.
        let now = now_ms();
        let roots = ChatRoots::detect();
        let current = &key.current;
        let rows: Vec<SessionRow> = sessions
            .into_iter()
            .map(|session| SessionRow {
                id: session.id.clone(),
                title: display_name(session),
                // The workspace's *name*, not its path: the rail and the
                // `ctrl+r` list are 30-odd columns wide, and a path spent on
                // the tail of `D:/work/…/future-os` says less than the word
                // itself. The path is still the group's identity (see
                // `WorkspaceGroup`) and the header still names it in full.
                //
                // A plain chat has no workspace to name: its directory is the
                // app's own scratch tree (`…/workspaces/chat/thread-…`), and
                // printing that directory's name is how `thread-20261026-…`
                // came to read like a project name. Blank, so the row says only
                // what a chat has — its title and its age.
                workspace: roots.workspace_label(&session.cwd),
                parent_id: session.parent_session_id.clone(),
                age: age_at(now, session.updated_at_ms),
                // 0 from an agent that predates the stamp: the activity stamp
                // stands in, so the 活跃 stretch is ordered rather than tied.
                created_at_ms: match session.created_at_ms {
                    0 => session.updated_at_ms,
                    created => created,
                },
                // On the launchpad nothing is attached, so no row claims to be
                // the current one.
                current: !key.launchpad && &session.id == current,
                // The agent's own flag, not a local run row: a conversation
                // another client started writes here too, which is what makes
                // 最近's 活跃 stretch a view of the agent and not of this
                // terminal's own typing.
                streaming: session.is_streaming.unwrap_or(false),
            })
            .collect();
        // The 工作区 tab: the same rows, filed the way the desktop's rail files
        // them. A conversation that ran in one of the app's own scratch
        // directories belongs to no workspace, so it is a plain chat.
        let (workspaces, chats) =
            file_sessions(&self.sessions, &rows, &self.pinned_workspaces, &roots);
        SessionView {
            key,
            rows,
            workspaces,
            chats,
        }
    }

    /// Pin or unpin one workspace, and answer what belongs on the notice row.
    pub fn set_workspace_pinned(&mut self, path: &str, pinned: bool) -> String {
        self.pinned_workspaces
            .retain(|known| !same_path(known, path));
        if pinned {
            self.pinned_workspaces.push(path.to_string());
        }
        self.pins_rev = self.pins_rev.wrapping_add(1);
        let name = workspace_name(path);
        if pinned {
            i18n::render(self.lang, Key::NoticePinned, &[&name])
        } else {
            i18n::render(self.lang, Key::NoticeUnpinned, &[&name])
        }
    }

    /// The workspace paths the rail currently pins, for the settings file.
    pub fn pinned_workspaces(&self) -> &[String] {
        &self.pinned_workspaces
    }

    /// `--continue`: attach to the most recently active session.
    pub async fn attach_most_recent(&mut self) {
        self.refresh_sessions().await;
        let newest = self
            .sessions
            .iter()
            .max_by_key(|session| session.updated_at_ms)
            .map(|session| session.id.clone());
        if let Some(id) = newest {
            if id != self.state.session_id {
                self.switch(&id).await;
            }
        }
    }

    /// Load the newest page of the transcript, replacing what is on screen.
    pub async fn load_tail(&mut self) {
        let session_id = self.client.get_current_session_id();
        if session_id.is_empty() {
            self.show_page(session_id, SessionEntriesPage::default());
            return;
        }
        match self.client.session_tail_page(&session_id).await {
            Ok(page) => self.show_page(session_id, page),
            Err(error) => {
                self.show_page(session_id, SessionEntriesPage::default());
                self.notice = Some(format!("Could not load the transcript: {error}"));
            }
        }
    }

    /// Put `page` on screen in place of whatever was there.
    ///
    /// The page is also remembered as the one these rows were built from, which
    /// is what lets the next switch to this conversation tell "the same page,
    /// keep the rows" from "it moved on while you were away, rebuild".
    fn show_page(&mut self, session_id: String, page: SessionEntriesPage) {
        self.chat.clear_messages();
        self.reset_scroll_revision();
        self.shown_page = Some(ShownPage {
            session_id: session_id.clone(),
            page: page.clone(),
        });
        self.paging = Paging {
            session_id,
            has_more: false,
            next_offset: 0,
        };
        self.apply_page(page, false);
    }

    /// Fetch the page above what is loaded and prepend it, keeping the reader's
    /// place on the row they were looking at.
    pub async fn load_older(&mut self) {
        if !self.paging.has_more {
            return;
        }
        let session_id = self.paging.session_id.clone();
        let before = self.paging.next_offset;
        match self.client.session_older_page(&session_id, before).await {
            // Prepending does not change the *tail* page, which is why
            // `shown_page` is left alone here: a switch back to this
            // conversation that finds the same tail keeps the older rows the
            // reader loaded as well.
            Ok(page) => self.apply_page(page, true),
            Err(error) => self.notice = Some(format!("Could not load older rows: {error}")),
        }
    }

    fn apply_page(&mut self, page: SessionEntriesPage, prepend: bool) {
        let messages = crate::session_rows::rows_to_messages(&page.entries);
        if prepend {
            self.chat.prepend_messages(messages);
        } else {
            for message in messages {
                self.chat.add_message(message);
            }
        }
        self.paging.has_more = page.has_more;
        self.paging.next_offset = page.next_offset;
    }

    // ─── events ───────────────────────────────────────────────────────

    /// Apply one event from the agent's stream. Events stamped with another
    /// session are dropped: after a switch the previous session's run would
    /// otherwise narrate into the transcript that is now on screen.
    pub fn handle_event(&mut self, event: AgentEvent) {
        // On the launchpad the transcript was cleared on purpose: the previous
        // session's run would otherwise narrate into a screen that says "no
        // session yet".
        if self.launchpad {
            return;
        }
        let current = self.client.get_current_session_id();
        if let Some(session) = event.session_id.as_deref() {
            if !current.is_empty() && session != current {
                return;
            }
        }
        if matches!(
            event.r#type.as_str(),
            "text_chunk"
                | "thinking_delta"
                | "thinking_start"
                | "thinking_end"
                | "tool_start"
                | "tool_delta"
                | "tool_end"
                | "agent_start"
                | "agent_end"
        ) {
            self.revision = self.revision.wrapping_add(1);
        }
        let text = event.text().to_string();
        // The turn's spend is the agent's to total up, and these are the two
        // events that say a number moved: a model step's `usage`, and the end of
        // the run. The driver loop turns the flag into one `get_state`.
        if wants_state_refresh(&event.r#type) {
            self.state_refresh_pending = true;
        }
        match event.r#type.as_str() {
            "text_chunk" => {
                self.mark(ActivityKind::Streaming);
                self.chat.append_to_last_message(&text);
            }
            "agent_start" => {
                self.run_tools.clear();
                self.run_started = Some(Instant::now());
                self.run_output_tokens = 0;
                self.run_tps = 0.0;
                self.mark(ActivityKind::Thinking);
                self.chat.add_message(ChatMessage::new(
                    crate::session_rows::random_id(),
                    ChatRole::Assistant,
                    "",
                ));
            }
            "thinking_start" => {
                self.mark(ActivityKind::Thinking);
                self.chat.start_thinking();
            }
            "thinking_delta" => self.chat.append_thinking_delta(&text),
            "thinking_end" => self.chat.end_thinking(),
            "tool_start" => {
                let tool_id = str_field(&event.data, "tool_id");
                let name = str_field(&event.data, "tool_name");
                let args = event.data.get("tool_args").map(|value| match value {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                });
                // One call, however many phases announce it — see the field.
                self.run_tools.insert(tool_id.clone());
                self.chat.add_tool_start(&tool_id, &name, args);
                self.mark(ActivityKind::Tool);
            }
            "tool_delta" => {
                let tool_id = str_field(&event.data, "tool_id");
                self.chat.append_tool_delta(&tool_id, &text);
            }
            "tool_end" => {
                let tool_id = str_field(&event.data, "tool_id");
                let failed = event
                    .data
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .or_else(|| event.data.get("isError").and_then(Value::as_bool))
                    .unwrap_or(false);
                let output = event.data.get("text").and_then(Value::as_str);
                self.chat.finish_tool(&tool_id, output, failed);
                self.mark(ActivityKind::Thinking);
            }
            "agent_end" => {
                let final_text = event.text();
                if !final_text.is_empty() {
                    self.chat.update_last_message(final_text);
                }
                self.chat.mark_last_message_complete();
                self.approval = None;
                self.activity = Activity::default();
                // The end of a run states its own totals, which is what the
                // rate is measured over when the provider reported no per-step
                // `usage` events at all.
                self.finish_run_usage(event.data.get("usage"));
                self.run_tps = self.live_tps();
                self.run_started = None;
                self.activity_start = None;
                self.run_tools.clear();
            }
            "usage" => self.apply_usage(event.data.get("usage")),
            "approval_request" => {
                self.approval = Some(ApprovalPrompt {
                    request_id: str_field(&event.data, "approval_request_id"),
                    title: {
                        let title = str_field(&event.data, "title");
                        if title.is_empty() {
                            "Approve tool execution".to_string()
                        } else {
                            title
                        }
                    },
                    summary: str_field(&event.data, "summary"),
                    raw: event.data.clone(),
                });
            }
            "approval_decision" => {
                // Whichever client answered — this one, the desktop, the phone —
                // the prompt goes, or it would ask about a request the agent no
                // longer holds.
                let id = str_field(&event.data, "approval_request_id");
                if self.approval.as_ref().is_some_and(|p| p.request_id == id) {
                    self.approval = None;
                }
            }
            "error" => {
                let error = str_field(&event.data, "error");
                if !error.is_empty() {
                    self.notice = Some(error);
                }
                self.activity = Activity::default();
                self.activity_start = None;
            }
            _ => {}
        }
    }

    fn mark(&mut self, kind: ActivityKind) {
        if self.activity_start.is_none() {
            self.activity_start = Some(Instant::now());
        }
        self.activity.kind = kind;
    }

    /// Forget the run this client was narrating.
    ///
    /// The activity row describes the session on screen, and a run belongs to
    /// the session that started it: a switch, a fork or `/new` has to drop the
    /// previous conversation's kind, clock, tool tally and rate, or the next
    /// one's footer keeps showing them (`思考 · 1071s · 53 工具`, minutes after
    /// the reader left that conversation).
    ///
    /// `agent_end` deliberately does not use this: the run is over, but its
    /// rate is what the row keeps showing until the next one.
    fn forget_run(&mut self) {
        self.activity = Activity::default();
        self.activity_start = None;
        self.run_tools.clear();
        self.run_started = None;
        self.run_output_tokens = 0;
        self.run_tps = 0.0;
    }

    /// Make the activity row describe the session this client just attached to.
    ///
    /// `get_state` is the other half of it: a conversation whose run is still in
    /// flight (`is_streaming`) — started here, by the desktop, by a queued run —
    /// owns the spinner again. Its clock starts now, because that is when this
    /// client began watching it, and its tool tally starts empty: the calls the
    /// run made before the switch are the transcript's to show, not this row's.
    fn adopt_run(&mut self) {
        self.forget_run();
        if self.state.is_streaming {
            self.mark(ActivityKind::Thinking);
            self.run_started = Some(Instant::now());
        }
    }

    /// Fold one `usage` event's numbers into the state the footer paints.
    ///
    /// The event carries the *step's* tokens (`prompt_tokens`,
    /// `completion_tokens`, the two cache counts), the same shape the inline TUI
    /// reads; a run emits one per model call, so the totals are sums and the
    /// context is the last step's prompt plus its completion — the next request
    /// is exactly that plus the new turns.
    /// Fold the *run's* own usage — the one `agent_end` carries.
    ///
    /// These are totals the per-step `usage` events already added up (a real
    /// run emits one of those per model call), so this may only drive the
    /// context gauge and the rate: adding them again would double the session's
    /// token and cost bookkeeping.
    fn finish_run_usage(&mut self, usage: Option<&Value>) {
        let Some(usage) = usage else { return };
        let prompt = token_field(usage, "prompt_tokens");
        let completion = token_field(usage, "completion_tokens");
        if prompt + completion > 0 {
            self.state.context_tokens = Some(prompt + completion);
        }
        if completion > 0 {
            self.run_output_tokens = completion as u64;
        }
    }

    fn apply_usage(&mut self, usage: Option<&Value>) {
        let Some((completion, ctx)) = fold_usage(&mut self.state, usage) else {
            return;
        };
        self.state.context_tokens = Some(ctx);
        self.run_output_tokens += completion;
        self.run_tps = self.live_tps();
    }

    /// tok/s for the run in flight (0 before enough of it has happened to mean
    /// anything, and the frozen rate once it is over).
    fn live_tps(&self) -> f64 {
        let Some(start) = self.run_started else {
            return self.run_tps;
        };
        rate_per_second(self.run_output_tokens, start.elapsed().as_secs_f64())
            .filter(|rate| *rate > 0.0)
            .unwrap_or(self.run_tps)
    }

    /// Drain the "go ask the agent for its totals" flag. The driver loop calls
    /// this after every agent event and performs the fetch when it is set.
    pub fn take_state_refresh(&mut self) -> bool {
        std::mem::take(&mut self.state_refresh_pending)
    }

    /// Is the agent reachable right now?
    pub fn connected(&self) -> bool {
        self.connected
    }

    /// Consume a connection-state edge for the driver, which owes a frame when
    /// this answers `true`. A closed watch is reported once and then stays
    /// quiet instead of waking the loop forever.
    pub fn connection_changed(&mut self) -> bool {
        if !self.connection_open {
            return false;
        }
        match self.connection.has_changed() {
            Ok(changed) => {
                if changed {
                    let connected = *self.connection.borrow_and_update();
                    self.connected = connected;
                }
                changed
            }
            Err(_) => {
                self.connection_open = false;
                true
            }
        }
    }

    /// Is the agent parked on an approval right now?
    pub fn approval(&self) -> Option<&ApprovalPrompt> {
        self.approval.as_ref()
    }

    pub fn set_draft(&mut self, text: String, cursor: usize) {
        if self.draft.text != text {
            self.draft.editor.set(&text, cursor);
        }
        self.draft.text = text;
        self.draft.cursor = cursor;
    }

    pub fn set_detail(&mut self, detail: ReplyDetail) {
        self.detail = detail;
        self.revision = self.revision.wrapping_add(1);
        self.chat.set_reply_detail(detail);
    }

    pub fn detail(&self) -> ReplyDetail {
        self.detail
    }

    /// Expand or collapse every tool call's body (`/tool-output`).
    ///
    /// A view toggle like `ctrl+e`, not a preference, and — unlike a turn — not
    /// new output either: the revision is left alone, so a reader scrolled up
    /// does not get the `↓ new output` marker for rows they opened themselves.
    pub fn toggle_tool_output(&mut self) -> bool {
        self.chat.toggle_tool_output_expanded()
    }

    // ─── painting ─────────────────────────────────────────────────────

    /// What the activity row is showing, without building a frame.
    ///
    /// The driver loop only needs this *one field* to notice that a turn
    /// started or ended (which is when the session list has to be re-read) and
    /// whether the row is animating (which is when the spinner owes a frame).
    /// Asking for a whole [`WorkbenchData`] to get it used to cost a transcript
    /// render per agent event — see [`Self::data`].
    pub fn activity_kind(&self) -> ActivityKind {
        self.activity_snapshot().kind
    }

    /// Is anything on the activity row moving with time — a run in flight, or
    /// the elapsed/tool counter the row keeps showing? That is the whole of the
    /// frame's animation (see `run::TickClock`); the only other clock in a frame
    /// is the sidebar's session ages, which read in minutes.
    pub fn activity_animating(&self) -> bool {
        let activity = self.activity_snapshot();
        activity.is_busy() || activity.tools > 0
    }

    /// Everything the surface paints, composed for one [`Frame`].
    ///
    /// The frame — not a loose width — is the argument because the transcript
    /// caches its rendered rows per width ([`ChatArea::render_all`]): a call at
    /// a width the last frame did not use re-wraps and re-parses every message.
    /// The driver used to ask for a whole frame at width `1` just to read the
    /// activity kind, on every key and every streamed token, so nothing within
    /// a keystroke ever hit that cache.
    fn draft_key(&self) -> String {
        if self.launchpad {
            "launchpad".into()
        } else {
            self.state.session_id.clone()
        }
    }
    fn save_draft(&mut self) {
        self.drafts.insert(
            self.draft_key(),
            DraftSnapshot {
                editor: self.draft.editor.clone(),
                pending: self.draft.editor.pending(),
            },
        );
    }
    fn restore_draft(&mut self) {
        let key = self.draft_key();
        if let Some(saved) = self.drafts.remove(&key) {
            let mut editor = saved.editor;
            let text = editor.text().to_string();
            editor.replace(&text, saved.pending);
            self.draft = InputView::from_editor(editor, self.draft.placeholder.clone());
        } else {
            self.draft = InputView::from_editor(
                super::editor::EditorState::default(),
                self.draft.placeholder.clone(),
            );
        }
    }
    fn reset_scroll_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub async fn cancel_queued(&mut self, run_id: &str) {
        match self.client.cancel_queued_run(run_id).await {
            Ok(()) => {
                self.refresh_state().await;
                self.notice = Some(t(self.lang, Key::QueueCancelled).into());
            }
            Err(error) => {
                self.notice = Some(i18n::render(self.lang, Key::QueueCancelFailed, &[&error]))
            }
        }
    }
    pub async fn recall_queued(&mut self, run_id: &str) {
        let Some(item) = self
            .state
            .queued_runs
            .iter()
            .find(|row| row.run_id == run_id)
            .cloned()
        else {
            return;
        };
        match self.client.cancel_queued_run(run_id).await {
            Ok(()) => {
                self.save_draft();
                self.draft
                    .editor
                    .replace(&item.display_text, Default::default());
                self.draft = InputView::from_editor(
                    self.draft.editor.clone(),
                    self.draft.placeholder.clone(),
                );
                self.refresh_state().await;
                self.notice = Some(t(self.lang, Key::QueueRecalled).into());
            }
            Err(error) => {
                self.notice = Some(i18n::render(self.lang, Key::QueueRecallFailed, &[&error]))
            }
        }
    }

    pub fn language(&self) -> Language {
        self.lang
    }

    pub fn copy_last(&self) -> Result<String, String> {
        self.chat
            .last_assistant_text()
            .map(|text| crate::clipboard::clean_assistant_text(&text))
            .filter(|text| !text.is_empty())
            .ok_or_else(|| t(self.lang, Key::CopyEmpty).to_string())
    }

    pub fn toggle_mouse(&mut self) -> bool {
        self.mouse_capture = !self.mouse_capture;
        self.mouse_capture
    }

    pub fn set_mouse_capture(&mut self, enabled: bool) {
        self.mouse_capture = enabled;
    }

    pub fn mouse_capture(&self) -> bool {
        self.mouse_capture
    }

    /// A completion belongs on the terminal only when this client requested
    /// the run, it succeeded, and no queued follow-up remains.
    pub fn take_completion_notification(&mut self, event: &AgentEvent) -> bool {
        if event.r#type != "agent_end" {
            return false;
        }
        let Some(run_id) = event.run_id.as_deref().filter(|id| !id.is_empty()) else {
            return false;
        };
        let own = self
            .completion_candidates
            .iter()
            .position(|id| id == run_id)
            .and_then(|index| self.completion_candidates.remove(index))
            .is_some();
        // The driver checks the queue after its state refresh; the state here
        // may still predate the agent_end event and must not reject early.
        own && crate::notifications::should_notify(
            true,
            event.data.get("state").and_then(Value::as_str),
            true,
        )
    }

    /// A failed terminal state also consumes the owned-run candidate.
    pub fn take_failure_notification(&mut self, event: &AgentEvent) -> bool {
        if event.r#type != "agent_end"
            || event.data.get("state").and_then(Value::as_str) != Some("failed")
        {
            return false;
        }
        event.run_id.as_deref().is_some_and(|run_id| {
            self.completion_candidates
                .iter()
                .position(|id| id == run_id)
                .map(|index| self.completion_candidates.remove(index))
                .is_some()
        })
    }

    pub fn completion_notification(&self) -> crate::notifications::NotifyEvent {
        crate::notifications::NotifyEvent::new(
            crate::notifications::NotifyKind::Completed,
            t(self.lang, Key::NotificationTitle),
            self.chat
                .last_assistant_text()
                .map(|text| crate::clipboard::clean_assistant_text(&text))
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| t(self.lang, Key::NotificationBody).into()),
        )
    }

    pub fn queue_is_empty(&self) -> bool {
        self.state.queued_runs.is_empty()
    }

    /// A non-recurring alert for this client, before `handle_event` consumes
    /// the event. Completion has its own owned-run and queue gate.
    pub fn alert_for(&self, event: &AgentEvent) -> Option<crate::notifications::NotifyEvent> {
        let kind = match event.r#type.as_str() {
            "approval_request" => crate::notifications::NotifyKind::ApprovalNeeded,
            "error" => crate::notifications::NotifyKind::Failed,
            _ => return None,
        };
        let body = if kind == crate::notifications::NotifyKind::ApprovalNeeded {
            event
                .data
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or("")
        } else {
            event
                .data
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("")
        };
        Some(crate::notifications::NotifyEvent::new(
            kind,
            t(self.lang, Key::NotificationTitle),
            if body.trim().is_empty() {
                kind.default_body().to_string()
            } else {
                body.into()
            },
        ))
    }

    pub fn data(&mut self, frame: &Frame) -> WorkbenchData {
        let state = self.state.clone();
        let messages = self.chat.render_all_shared(frame.inner);
        let activity = self.activity_snapshot();
        let input = self.draft.clone();
        // The rail's rows and its 工作区 filing, built only when one of their
        // inputs moved (see [`SessionView`]) — everything outside `messages`
        // here is per-frame work that this used to repeat for every
        // conversation the reader had ever had.
        let (rows, workspaces, chats) = self.session_view_parts();
        // On the launchpad there is no session, so no session's title: the
        // header must not keep naming the conversation `/new` left behind.
        let title = if self.launchpad {
            String::new()
        } else {
            self.title()
        };
        let current = state.session_id.clone();
        let skill_names = state.skills.clone();
        // The agent names the skills it discovered; the sidebar's local read adds
        // the descriptions (and is the fallback when an older agent reports
        // none). One list, deduplicated by name, in the agent's order.
        let local_skills = self.local_skills.clone().unwrap_or_default();
        let skills: Vec<SkillRow> = if skill_names.is_empty() {
            local_skills
        } else {
            skill_names
                .iter()
                .map(|name| SkillRow {
                    name: name.clone(),
                    description: local_skills
                        .iter()
                        .find(|skill| &skill.name == name)
                        .map(|skill| skill.description.clone())
                        .unwrap_or_default(),
                })
                .collect()
        };

        // The directory the header and the launchpad chip name: the staged one
        // while nothing is attached, the agent's own otherwise.
        let cwd = self.shown_cwd().unwrap_or_default();

        WorkbenchData {
            app_name: "future-tui".into(),
            version: crate::version::VERSION.to_string(),
            connected: self.connected(),
            session: SessionMeta {
                id: if self.launchpad {
                    String::new()
                } else {
                    current
                },
                title,
                // On the launchpad the chip names the workspace the first
                // message will create the session in, not whatever session the
                // client left behind.
                cwd: cwd.clone(),
                // A directory this client has not chosen (the launchpad's empty
                // cwd) is not a chat yet: `is_chat_directory` answers "no
                // workspace" for the empty string too, and a surface that has
                // decided nothing must not tick a row either.
                chat: !cwd.trim().is_empty() && is_chat_directory(&cwd),
                branch: self.branch.get(&self.local_cwd).to_string(),
            },
            model: self
                .shown_model()
                .filter(|model| !model.trim().is_empty())
                .unwrap_or_else(|| "no model".to_string()),
            effort: self.shown_effort(),
            mode: self.shown_mode(),
            usage: UsageData {
                ctx_used: state.context_tokens.unwrap_or(0).max(0) as u64,
                ctx_total: state.context_window.unwrap_or(0).max(0) as u64,
                cost: state.usage.cost_cny,
                cache_pct: cache_percent(&state),
                tps: self.live_tps(),
            },
            files: self.files.clone(),
            tree: self.tree.clone(),
            preview: self.preview.clone(),
            sessions: rows,
            workspaces,
            chats,
            models: self
                .models
                .iter()
                .map(|model| ModelChoice {
                    id: model.id.clone(),
                    label: if model.label.trim().is_empty() {
                        model.id.clone()
                    } else {
                        model.label.clone()
                    },
                    provider: model.provider.clone(),
                    context_window: model.context_window,
                    thinking_level: model.thinking_level.clone(),
                    thinking_levels: model.thinking_levels.clone(),
                    default_thinking_level: model.default_thinking_level.clone(),
                    is_default: model.is_default,
                })
                .collect(),
            skills,
            messages,
            input,
            activity,
            notice: self.notice.clone(),
            queue: self
                .state
                .queued_runs
                .iter()
                .map(|row| QueuedPrompt {
                    run_id: row.run_id.clone(),
                    text: row.display_text.clone(),
                })
                .collect(),
            revision: self.revision,
            mouse_capture: self.mouse_capture,
            approval: self
                .approval
                .as_ref()
                .map(|prompt| super::workflows::approval_of(&prompt.raw)),
            diff: self.diff.clone(),
            fork_points: self.fork_points.clone(),
            worktrees: self.worktrees.clone(),
            git: self.git.clone(),
        }
    }

    fn activity_snapshot(&self) -> Activity {
        if let Some(approval) = &self.approval {
            return Activity {
                kind: ActivityKind::Tool,
                detail: i18n::render(
                    self.lang,
                    Key::ApprovalWaiting,
                    &[
                        &approval.title,
                        &if approval.summary.is_empty() {
                            String::new()
                        } else {
                            format!(" · {}", approval.summary)
                        },
                    ],
                ),
                elapsed_secs: 0,
                tools: self.run_tools.len(),
            };
        }
        Activity {
            kind: self.activity.kind,
            detail: String::new(),
            elapsed_secs: self
                .activity_start
                .map(|start| start.elapsed().as_secs())
                .unwrap_or(0),
            tools: self.run_tools.len(),
        }
    }

    /// The session's title: its name, else the first prompt, else the id tail.
    fn title(&self) -> String {
        if let Some(name) = self.state.session_name.as_deref() {
            if !name.trim().is_empty() {
                return name.to_string();
            }
        }
        self.sessions
            .iter()
            .find(|row| row.id == self.state.session_id)
            .map(display_name)
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| "new session".to_string())
    }

    // ─── actions ──────────────────────────────────────────────────────

    /// Apply one surface intent. Returns whether the loop should keep going.
    pub async fn apply(&mut self, action: WbAction) -> Outcome {
        match action {
            WbAction::Quit => return Outcome::Quit,
            // The surface's own cursor move, resolved by whoever holds the data
            // (`run::quit`): this layer has no rail to walk.
            WbAction::SessionStep(_) => {}
            WbAction::EditInput { text, cursor } => self.set_draft(text, cursor),
            WbAction::EditEditor(editor) => {
                self.draft = InputView::from_editor(*editor, self.draft.placeholder.clone())
            }
            WbAction::CopyLast
            | WbAction::ToggleMouse
            | WbAction::OpenQueue
            | WbAction::OpenApproval
            | WbAction::OpenDiff
            | WbAction::OpenGit
            | WbAction::OpenFiles
            | WbAction::GitSection(_)
            | WbAction::GitFilter(_)
            | WbAction::GitRefresh
            | WbAction::SelectGit(_)
            | WbAction::GitWorktree(_)
            | WbAction::CopyText(_)
            | WbAction::OpenHistory
            | WbAction::OpenTree
            | WbAction::OpenFork
            | WbAction::OpenWorktree => {}
            WbAction::ApprovalDecision { approved, rule } => {
                self.decide_with_rule(approved, rule).await
            }
            WbAction::SelectDiff(_) => {}
            WbAction::SelectWorktree(path) => self.select_worktree(&path).await,
            WbAction::ForkAt(id) => self.fork_at(&id).await,
            WbAction::Compact(instructions) => self.compact(instructions).await,
            WbAction::SetApproval(mode) => self.set_approval(&mode).await,
            WbAction::SelectWorkspace(path) => self.select_workspace(&path).await,
            WbAction::SelectChatWorkspace => self.select_chat_workspace().await,
            WbAction::ToggleFolder(path) => self.toggle_folder(&path),
            WbAction::RefreshFiles => self.refresh_tree(),
            // The preview itself is opened by the driver, which owns the
            // overlay; loading the bytes is ours.
            WbAction::PreviewFile(path) => self.start_preview(&path),
            WbAction::PasteClipboard => self.paste_clipboard().await, // The box is the driver's to open (`run::quit`): this layer reads
            // the bytes, the surface paints them.
            WbAction::PreviewAttachment => {}
            WbAction::OpenUrl(url) => self.open_url(&url),
            WbAction::SetTools(tools) => match self.client.set_tools(&tools).await {
                Ok(()) => self.notice = Some(t(self.lang, Key::ToolsDone).into()),
                Err(error) => {
                    self.notice = Some(i18n::render(self.lang, Key::WorkflowFailed, &[&error]))
                }
            },
            WbAction::CancelQueued(id) => self.cancel_queued(&id).await,
            WbAction::RecallQueued(id) => self.recall_queued(&id).await,
            WbAction::Submit(text) => self.submit(&text).await,
            WbAction::Command(command) => return self.command(&command).await,
            WbAction::Scroll(_) => {}
            WbAction::LoadOlder => self.load_older().await,
            WbAction::SwitchSession(id) => self.switch(&id).await,
            WbAction::DeleteSession(id) => self.delete_session(&id).await,
            WbAction::SetModel(id) => self.set_model(&id).await,
            WbAction::SetThinking(level) => self.set_thinking(&level).await,
            // No session to apply them to: stage, do not call.
            WbAction::SetPendingModel(id) => self.set_pending_model(&id),
            WbAction::SetPendingThinking(level) => self.set_pending_effort(&level),
            WbAction::NewSession => self.to_launchpad(),
            // The surface owns its own palette; the driver persists it (it is
            // the one that knows where `settings.json` is). `ResetTheme` is the
            // same split: forgetting the stored id is the driver's business.
            WbAction::SetTheme(_) | WbAction::ResetTheme => {}
            // Same split for the language: the driver resolves the preference,
            // hands the bundle back here and persists it.
            WbAction::SetLanguage(_) => {}
            WbAction::SwitchPanel(_) | WbAction::ResizeSidebar(_) | WbAction::PersistSidebar => {}
            // Surface-only state: the driver owns persistence, the agent does
            // not need a round trip for a rectangle or a local shortcut.
            WbAction::SetPreferences | WbAction::SetKeybindings => {}
            // The rail's pin lands in this client's settings file; the driver
            // applies it through [`Live::set_workspace_pinned`], so here it is
            // only a shape the agent never sees.
            WbAction::SetWorkspacePinned { .. } => {}
            // The driver displays local notices; a demo adapter has no notice
            // dispatch of its own to perform.
            WbAction::Notice(_) => {}
            WbAction::FocusPane(_) => {}
            WbAction::Redraw => {}
        }
        Outcome::Continue
    }

    async fn compact(&mut self, instructions: String) {
        match self
            .client
            .compact((!instructions.is_empty()).then_some(instructions.as_str()))
            .await
        {
            Ok(_) => {
                self.notice = Some(t(self.lang, Key::CompactDone).into());
                self.refresh_state().await;
                self.load_tail().await;
            }
            Err(error) => {
                self.notice = Some(i18n::render(self.lang, Key::WorkflowFailed, &[&error]))
            }
        }
    }

    /// Send the draft. A submission while a run is in flight queues behind it
    /// (the agent's coalescing policy), which is what the inline TUI does too.
    async fn submit(&mut self, text: &str) {
        self.draft.editor.record_submission();
        // The draft is read first because it is what carries the folded pastes
        // that `pending()` expands into the prompt. It is *not* the only source:
        // the driver hands this action over after clearing the surface, and a
        // `Submit` that found an empty draft used to return without ever
        // reaching the agent — the launchpad's first message did nothing at all.
        // The action's own text is the surface's reading of the prompt, so it
        // stands in whenever the draft has already been reset.
        let drafted = self.draft.editor.text().to_string();
        let original_text = if drafted.trim().is_empty() {
            text.to_string()
        } else {
            drafted
        };
        // The draft is what the reader typed; the prompt is that draft with its
        // `@path` mentions inlined. The transcript keeps the draft (that is what
        // was said), the agent gets the files (that is what was meant).
        let typed = self
            .draft
            .editor
            .pending()
            .expand(&original_text)
            .trim()
            .to_string();
        if typed.is_empty() {
            return;
        }
        let message = expand_mentions(&typed, &self.local_cwd);
        // The launchpad's first message: there is no session to send it to yet,
        // so create one in the folder the surface is showing before sending.
        // `/new` left the previous session attached on the agent's side, which
        // is exactly why this asks for a *new* one instead of reusing it.
        if self.launchpad || self.state.session_id.is_empty() {
            self.new_session().await;
            if self.state.session_id.is_empty() || self.launchpad {
                // `new_session` reports its own failure; this is the other
                // half — the agent made a session but this client did not end
                // up attached to one. Returning silently here is what makes the
                // launchpad's first message look like a key that does nothing:
                // the draft stays (that is deliberate, it can be sent again) and
                // the reader is owed the reason.
                if self.notice.is_none() {
                    self.notice = Some(t(self.lang, Key::NoticeSessionNotStarted).into());
                }
                return;
            }
        }
        let pending = self.draft.editor.take_pending();
        let attachments = pending.attachments().to_vec();
        self.drafts.remove(&self.draft_key());
        self.draft = InputView::from_editor(
            super::editor::EditorState::default(),
            self.draft.placeholder.clone(),
        );
        self.revision = self.revision.wrapping_add(1);
        self.chat.add_message(ChatMessage::new(
            crate::session_rows::random_id(),
            ChatRole::User,
            &typed,
        ));
        self.mark(ActivityKind::Thinking);
        match self
            .client
            .prompt(
                &message,
                crate::rpc::types::BUSY_ENQUEUE_COALESCING,
                attachments,
            )
            .await
        {
            Ok(ack) => {
                if !ack.run_id.is_empty() {
                    if self.completion_candidates.len() == 32 {
                        self.completion_candidates.pop_front();
                    }
                    self.completion_candidates.push_back(ack.run_id);
                }
                if ack.accepted_state == "queued" {
                    self.refresh_state().await;
                }
            }
            Err(error) => {
                // The request may have reached the agent before the reply was
                // lost. Keeping the message on screen preserves what the reader
                // said; restoring the draft as well made a retry look harmless
                // and could send the same prompt twice. The marker says the
                // acknowledgement never arrived, and the notice says to check
                // before retrying.
                self.chat
                    .mark_last_message_run_state(RunState::NotAcknowledged);
                self.notice = Some(i18n::render(
                    self.lang,
                    Key::NoticeSendUnacknowledged,
                    &[&error],
                ));
                self.activity = Activity::default();
                self.activity_start = None;
            }
        }
    }

    /// The `/…` commands the palette and the key bindings can run. Anything else
    /// is reported rather than silently dropped: a command that does nothing is
    /// worse than one that says it is unknown.
    async fn command(&mut self, command: &str) -> Outcome {
        let mut parts = command.trim().splitn(2, ' ');
        let name = parts
            .next()
            .unwrap_or("")
            .trim_start_matches('/')
            .to_string();
        let arg = parts.next().unwrap_or("").trim().to_string();
        match name.as_str() {
            "quit" => return Outcome::Quit,
            "queue" => self.refresh_state().await,
            "copy" | "mouse" => {}
            "paste" => self.paste_clipboard().await,
            "new" => {
                self.notice = None;
                // `/new` is the launchpad, not an immediate session: the
                // session is created by the first message, with whatever model
                // and workspace the reader chose there.
                self.to_launchpad();
            }
            "sessions" => {
                self.refresh_sessions().await;
            }
            "compact" => {
                self.compact(arg).await;
            }
            "sandbox" => {
                // `/sandbox <mode>` sets it; a bare `/sandbox` is the menu the
                // driver opens before this handler ever sees it.
                if crate::approval_mode::ApprovalMode::from_wire(&arg).is_some() {
                    self.set_approval(&arg).await;
                } else {
                    self.cycle_mode().await;
                }
            }
            "diff" | "history" | "fork" => {}
            "worktree" if !arg.is_empty() => {
                if arg.strip_prefix("new ").is_some() {
                    self.notice = Some(t(self.lang, Key::WorktreeCreating).into());
                } else {
                    self.notice = Some(crate::worktree::WORKTREE_USAGE.into());
                }
            }
            "tools" => {
                if arg.is_empty() {
                    self.notice = Some(t(self.lang, Key::ToolsUsage).into());
                } else if arg == "off" {
                    match self.client.disable_tools().await {
                        Ok(()) => self.notice = Some(t(self.lang, Key::ToolsDone).into()),
                        Err(error) => {
                            self.notice =
                                Some(i18n::render(self.lang, Key::WorkflowFailed, &[&error]))
                        }
                    }
                } else {
                    let tools: Vec<String> = arg
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect();
                    match self.client.set_tools(&tools).await {
                        Ok(()) => self.notice = Some(t(self.lang, Key::ToolsDone).into()),
                        Err(error) => {
                            self.notice =
                                Some(i18n::render(self.lang, Key::WorkflowFailed, &[&error]))
                        }
                    }
                }
            }
            "providers" => match self.client.list_providers().await {
                Ok(providers) => {
                    self.notice = Some(
                        providers
                            .iter()
                            .map(|provider| format!("{} ({})", provider.id, provider.model_count))
                            .collect::<Vec<_>>()
                            .join(" · "),
                    );
                }
                Err(error) => {
                    self.notice = Some(i18n::render(self.lang, Key::WorkflowFailed, &[&error]))
                }
            },
            // A model named in the command line is a choice like any other, so
            // it takes the launchpad road too when nothing is attached.
            "model" if !arg.is_empty() && self.detached() => self.set_pending_model(&arg),
            "model" if !arg.is_empty() => match self.client.set_model(&arg).await {
                Ok(()) => self.refresh_state().await,
                Err(error) => self.notice = Some(format!("Could not set the model: {error}")),
            },
            "model" => self.cycle_model().await,
            // The launchpad's parameter chips. With a session attached each
            // floats a wire call rather than carrying state of its own: the
            // agent stays the single source of what the next message runs with.
            // With none, the model, effort and approval chips stage the choice
            // locally — the agent has no session to accept any of them.
            "launchpad.model" => self.cycle_model().await,
            "launchpad.effort" => self.cycle_thinking().await,
            "launchpad.approval" => self.cycle_mode().await,
            "launchpad.workspace" => self.cycle_pending_workspace(),
            "reply-detail" => {
                let next = match self.detail {
                    ReplyDetail::Default => ReplyDetail::Detailed,
                    ReplyDetail::Detailed => ReplyDetail::Default,
                };
                self.set_detail(next);
            }
            // The bodies under the tool rows: one row per call is the point of
            // the transcript, and this is the door that opens a body (`ctrl+g`
            // answered it before the chord became the git browser).
            "tool-output" => {
                self.toggle_tool_output();
            }
            "refresh" => {
                self.refresh_state().await;
                self.refresh_sessions().await;
            }
            // The whole record, through the agent's own exporter (the same one
            // the desktop uses). The path it wrote is the whole answer, so it
            // goes on the notice row rather than into the transcript.
            "export" => match self.client.export_html().await {
                Ok(value) => {
                    let path = value
                        .get("path")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    self.notice = Some(if path.is_empty() {
                        t(self.lang, Key::ExportDone).to_string()
                    } else {
                        i18n::render(self.lang, Key::ExportDonePath, &[&path])
                    });
                }
                Err(error) => self.notice = Some(format!("Could not export: {error}")),
            },
            // The verbs the *surface* answers (`run.rs` calls it before this
            // handler): `/usage`, `/skills`, `/workspace`, `/transcript`,
            // `/help`. They are named here so a caller that bypasses the
            // surface is not told a command the palette advertises does not
            // exist — the notice the whole family used to produce. The four
            // choosers (`/model`, `/thinking`, `/mode`, `/workspace`) come back
            // to the surface the same way: the picker is not the agent's.
            "usage" | "skills" | "workspace" | "transcript" | "help" | "thinking" | "mode" => {}
            other => self.notice = Some(format!("Unknown command /{other}")),
        }
        Outcome::Continue
    }

    /// Already on this conversation, and not on the launchpad?
    ///
    /// A click on the row the transcript is already showing, or a step that
    /// found nowhere to go, must not re-read anything: the rows on screen are
    /// this conversation's, and a rebuild would throw away the scroll position
    /// and the older pages the reader loaded.
    pub fn already_showing(&self, session_id: &str) -> bool {
        !self.launchpad && session_id == self.state.session_id
    }

    /// Show one conversation: the synchronous half and the round trip.
    ///
    /// Kept as one call for every door that is not a held key (a click in the
    /// rail, the launchpad's recents, `--continue`, a deleted row's
    /// successor). Those doors paint once after this returns, so a cache hit's
    /// rows and the reconcile's are one frame; `run::show_conversation` splits
    /// the two only for `shift+↑/↓`, where painting the cache first is the
    /// whole point.
    async fn switch(&mut self, session_id: &str) {
        if self.already_showing(session_id) {
            return;
        }
        self.begin_switch(session_id);
        self.finish_switch(session_id).await;
    }

    /// Everything about a switch that costs no round trip.
    ///
    /// The draft is filed under the conversation being left, the cached
    /// conversation (if any) is swapped onto the screen, and the rail's step is
    /// forgotten. Returns whether cached rows are on screen *now*: `true` means
    /// the caller may paint them before [`Self::finish_switch`] asks the agent
    /// anything, which is the whole point of keeping them. A miss leaves the
    /// transcript untouched — the rows are replaced when the page arrives, and
    /// nothing paints in between — and returns `false`.
    pub fn begin_switch(&mut self, session_id: &str) -> bool {
        if self.already_showing(session_id) {
            return false;
        }
        self.save_draft();
        self.launchpad = false;
        self.notice = None;
        let cached = self.take_cached(session_id);
        // The row is the session's: nothing of the one just left may stay on it
        // (see [`Self::adopt_run`]). With cached rows this already uses the
        // state that came in with them, so the frame painted from them is
        // coherent; `finish_switch` re-runs it against the agent's answer.
        self.adopt_run();
        self.reset_scroll_revision();
        cached
    }

    /// Point the client at `session_id` and reconcile what is on screen.
    ///
    /// The state and the tail page both address the session by name, so they go
    /// out together — one round trip instead of the three a switch used to take
    /// (`switch_session`, then `get_state`, then the page). `get_state` is what
    /// hydrates the session on the agent's side as well, which is why the
    /// `switch_session` RPC has no work left to do: it resolves the very same
    /// `try_get_session`. A cached conversation whose page the agent answers
    /// with unchanged keeps the rows it already has, so coming back to one is a
    /// repaint and not a rebuild.
    pub async fn finish_switch(&mut self, session_id: &str) {
        let (state, page) = tokio::join!(
            self.client.get_state_of(session_id),
            self.client.session_tail_page(session_id)
        );
        match state {
            Ok(state) => self.install_state(state),
            // Nothing moved: the header and the transcript stay on the
            // conversation that is still current, and the notice says why the
            // one the reader picked did not open.
            Err(error) => {
                self.notice = Some(format!("Could not switch session: {error}"));
                return;
            }
        }
        match page {
            Ok(page) => {
                // The rows on screen are worth keeping only when they are this
                // conversation's and built from this very page.
                let unchanged = self
                    .shown_page
                    .as_ref()
                    .is_some_and(|shown| shown.session_id == session_id && shown.page == page);
                if !unchanged {
                    self.show_page(session_id.to_string(), page);
                }
            }
            Err(error) => self.notice = Some(format!("Could not load the transcript: {error}")),
        }
        self.adopt_run();
        self.restore_draft();
    }

    /// Is this conversation's transcript still in the cache — that is, can a
    /// switch to it paint without asking the agent for anything?
    pub fn is_cached(&self, session_id: &str) -> bool {
        self.cached.iter().any(|(id, _)| id == session_id)
    }

    /// Swap the cached conversation for `session_id` onto the screen, keeping
    /// what is there now for the way back.
    fn take_cached(&mut self, session_id: &str) -> bool {
        // What is on screen now, and everything needed to put it back.
        let outgoing = (
            self.state.session_id.clone(),
            self.paging.clone(),
            self.state.clone(),
            self.shown_page.clone(),
        );
        match self.cached.iter().position(|(id, _)| id == session_id) {
            Some(index) => {
                let Some((_, cached)) = self.cached.remove(index) else {
                    return false;
                };
                let CachedSession {
                    chat,
                    paging,
                    state,
                    page,
                } = cached;
                let previous = std::mem::replace(&mut self.chat, chat);
                self.paging = paging;
                self.state = state;
                // The cached page stays the one these rows were built from, so
                // the reconcile below can tell "unchanged, keep them" from
                // "moved on, rebuild".
                self.shown_page = Some(ShownPage {
                    session_id: session_id.to_string(),
                    page,
                });
                self.remember(outgoing, previous);
                true
            }
            None => {
                let fresh = self.chat.empty_like();
                let previous = std::mem::replace(&mut self.chat, fresh);
                self.paging = Paging::default();
                self.shown_page = None;
                self.remember(outgoing, previous);
                false
            }
        }
    }

    /// Keep the conversation that was just on screen, so coming back to it is a
    /// repaint.
    ///
    /// A conversation whose page this client cannot name — one it never finished
    /// loading, or the launchpad — is dropped instead: the cache promises rows,
    /// and rows it cannot check against the agent's page would be worse than a
    /// round trip.
    fn remember(
        &mut self,
        outgoing: (String, Paging, RpcSessionState, Option<ShownPage>),
        chat: ChatArea,
    ) {
        let (id, paging, state, Some(shown)) = outgoing else {
            return;
        };
        // Rows that belong to another conversation (or to none) are not this
        // one's to keep.
        if id.is_empty() || shown.session_id != id {
            return;
        }
        self.cached.retain(|(known, _)| known != &id);
        self.cached.push_back((
            id,
            CachedSession {
                chat,
                paging,
                state,
                page: shown.page,
            },
        ));
        while self.cached.len() > CACHED_SESSIONS {
            self.cached.pop_front();
        }
    }

    /// Delete one conversation — its record and its file.
    ///
    /// The conversation on screen cannot be deleted out from under the
    /// transcript, so the row that slides into its place in the list takes
    /// over; deleting the last one leaves the launchpad, which is where a
    /// client with no session belongs. A refusal (the agent does not delete a
    /// session a compaction holds) keeps everything as it was and says why on
    /// the notice row.
    async fn delete_session(&mut self, session_id: &str) {
        let was_current = !self.detached() && session_id == self.state.session_id;
        let position = self.sessions.iter().position(|row| row.id == session_id);
        let title = self
            .sessions
            .iter()
            .find(|row| row.id == session_id)
            .map(display_name)
            .unwrap_or_else(|| session_id.to_string());
        if let Err(error) = self.client.delete_session(session_id).await {
            self.notice = Some(i18n::render(self.lang, Key::NoticeDeleteFailed, &[&error]));
            return;
        }
        // The list is re-read before anything is opened: the successor is named
        // from what the agent has now, not from what the rail painted a moment
        // ago.
        self.refresh_sessions().await;
        if was_current {
            // The row that slid into the deleted one's place takes over, and
            // the walk wraps: the last row has no successor, so it falls back
            // to the first. The deleted id is skipped wherever it lands — a
            // list that still names it (a stale read) must not reopen it.
            let start = position.unwrap_or(0);
            let successor = (start..self.sessions.len())
                .chain(0..start)
                .map(|index| &self.sessions[index])
                .find(|row| row.id != session_id)
                .map(|row| row.id.clone());
            match successor {
                Some(next) => self.switch(&next).await,
                None => {
                    self.to_launchpad();
                    // Nothing is attached any more, and the id the agent still
                    // knows as "current" names the conversation that is gone.
                    self.state.session_id.clear();
                    self.client.set_current_session_id("");
                }
            }
        }
        // Last: opening a conversation clears the notice row, and "deleted" is
        // what the reader just asked for, so it has to survive the switch.
        self.notice = Some(i18n::render(self.lang, Key::NoticeDeleted, &[&title]));
    }

    /// `/new`: detach from the session and show the launchpad.
    /// Nothing is created and nothing is asked of the agent — the header keeps
    /// its connection state and version, the transcript goes, and the first
    /// message starts the session (`submit` already handled that on a fresh
    /// start). A run still streaming in the session left behind is muted by the
    /// `launchpad` guard in `handle_event`.
    pub fn to_launchpad(&mut self) {
        // `/new` starts where the conversation was: the workspace the reader is
        // leaving is staged for the one the first message creates, so a new
        // conversation does not silently fall back to the app's scratch tree.
        // A chat stages no workspace at all — the next message mints its own
        // chat directory (see [`Self::new_session`]).
        let inherited = self.shown_cwd().filter(|cwd| {
            let cwd = cwd.trim();
            !cwd.is_empty() && !is_chat_directory(cwd)
        });
        self.save_draft();
        self.launchpad = true;
        // The conversation being left keeps its rows for the way back: `/new`
        // is not a reason to make the rail pay a round trip for it.
        let outgoing = (
            self.state.session_id.clone(),
            self.paging.clone(),
            self.state.clone(),
            self.shown_page.take(),
        );
        let fresh = self.chat.empty_like();
        let previous = std::mem::replace(&mut self.chat, fresh);
        self.remember(outgoing, previous);
        self.paging = Paging::default();
        self.restore_draft();
        self.reset_scroll_revision();
        self.forget_run();
        self.approval = None;
        self.notice = None;
        // The next conversation runs where this one ran; `None` is the chat
        // default, whose directory the first message names after its session.
        self.pending_cwd = inherited;
        // The launchpad's own directory (`@` and the files panel read it, see
        // `refresh_local`): the workspace the chip now names.
        self.refresh_local();
    }

    /// Stage the directory this process was started from for the first
    /// conversation — the launchpad's default, so `cd project && future tui`
    /// creates that project's conversation.
    ///
    /// Called by the driver rather than by [`Self::attach`]: attaching is also
    /// how tests and an embedding caller open the surface, and those must not
    /// inherit whatever directory the process happens to run in.
    pub fn stage_launch_workspace(&mut self) {
        if !self.detached() {
            return;
        }
        if let Some(workspace) = launch_workspace() {
            self.pending_cwd = Some(workspace);
            self.refresh_local();
        }
    }

    /// Is the surface on the launchpad (no session attached)?
    pub fn on_launchpad(&self) -> bool {
        self.detached()
    }

    /// Nothing is attached — `/new` detached this client, or the agent never had
    /// a session for it. The agent answers `set_model`/`set_thinking_level` with
    /// "session not found" in both cases, and the surface paints the launchpad in
    /// both (`WorkbenchData::on_launchpad` looks at the empty id), so one
    /// predicate decides both what is painted and where a choice goes.
    fn detached(&self) -> bool {
        self.launchpad || self.state.session_id.is_empty()
    }

    /// The model the surface should name: the staged one while the launchpad
    /// has no session to read it from, otherwise the agent's own.
    ///
    /// A launchpad with nothing staged names the agent's *default* model — the
    /// one `new_session` would pick for a session that asks for none (the same
    /// model `list_models` flags `isDefault`) — rather than the model of the
    /// conversation `/new` just left behind. A fresh screen is a fresh choice:
    /// carrying the last conversation's model into the next one made `/new` a
    /// copy of what came before instead of a clean start.
    fn shown_model(&self) -> Option<String> {
        if self.detached() {
            return self
                .pending_model
                .clone()
                .or_else(|| self.default_model_id())
                .or_else(|| self.state.model.clone());
        }
        self.state.model.clone()
    }

    /// The agent's default model, as `list_models` flags it (`isDefault` — the
    /// agent resolves it with the same function `new_session` uses, so the chip
    /// and the session that gets created agree). `None` on an agent that flags
    /// nothing.
    fn default_model_id(&self) -> Option<String> {
        let model = self.models.iter().find(|model| model.is_default)?;
        Some(if model.provider.is_empty() {
            model.id.clone()
        } else {
            format!("{}/{}", model.provider, model.id)
        })
    }

    /// The reasoning level the surface should name (see [`Self::shown_model`]).
    ///
    /// A launchpad with nothing staged starts at the reserved `default` (the
    /// provider default, i.e. no thinking parameter): an empty agent state is
    /// not "no level", it is the implicit one. The default model's own starting
    /// level is what the agent would hand a session created without one, so that
    /// is what the chip names.
    fn shown_effort(&self) -> String {
        if self.detached() {
            if let Some(level) = self.pending_effort.clone() {
                return level;
            }
            return self
                .models
                .iter()
                .find(|model| model.is_default)
                .and_then(|model| model.default_thinking_level.clone())
                .filter(|level| !level.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_THINKING_LEVEL_NAME.to_string());
        }
        self.state.thinking_level.clone()
    }

    /// The approval mode the surface should name (see [`Self::shown_model`]).
    ///
    /// The agent reports the sandbox *tier* only, and `auto` is that tier with
    /// the model reviewer — so a mode chosen in this process is named from what
    /// was chosen, and a fresh attach reads the tier (`sandbox` rather than
    /// `auto`). The launchpad starts from the product default it was asked for —
    /// 完全访问 (`off`) — and [`Self::new_session`] applies it, so the chip is not
    /// naming something the session will not run under.
    fn shown_mode(&self) -> String {
        if self.detached() {
            return self
                .pending_mode
                .clone()
                .unwrap_or_else(|| "off".to_string());
        }
        self.state
            .sandbox_tier
            .clone()
            .filter(|tier| !tier.trim().is_empty())
            .unwrap_or_else(|| "off".to_string())
    }

    /// The workspace the surface should name (see [`Self::shown_model`]).
    ///
    /// A launchpad with nothing staged names the plain chat (see
    /// [`chat_workspace`]); `/new` stages the workspace the last conversation
    /// ran in, and a fresh terminal stages the directory `future tui` was
    /// started from (see [`launch_workspace`]).
    fn shown_cwd(&self) -> Option<String> {
        if self.detached() {
            return Some(self.pending_cwd.clone().unwrap_or_else(chat_workspace));
        }
        self.state.cwd.clone()
    }

    /// The directories a new session can be started in, as one *stable* ring:
    /// the plain chat first (the launchpad's own default, the same row the
    /// workspace picker lists), then every workspace this client has a
    /// conversation in, newest first. Deduplicated by the same
    /// trailing-slash-insensitive comparison the 工作区 filing uses, so one
    /// directory is one choice.
    ///
    /// Stable rather than "the one in force first": the chip's `enter` walks
    /// this list, and a list that reordered itself around the current choice made
    /// two presses bounce between the same pair instead of walking the ring — and
    /// it left the chat (the default it starts on) unreachable once the reader
    /// stepped off it.
    pub fn workspace_choices(&self) -> Vec<String> {
        let mut choices: Vec<String> = Vec::new();
        let mut push = |path: &str| {
            if path.trim().is_empty() {
                return;
            }
            if choices.iter().any(|known| same_path(known, path)) {
                return;
            }
            choices.push(path.to_string());
        };
        push(&chat_workspace());
        for session in self.sessions.iter() {
            // Every chat is the one 对话会话 entry above: the per-chat scratch
            // directories are not workspaces the chip should cycle through.
            if is_chat_directory(&session.cwd) {
                continue;
            }
            push(&session.cwd);
        }
        choices
    }

    /// Stage the next workspace of the launchpad's 工作区 chip.
    ///
    /// The chip cycles the directories the reader already works in; with none
    /// to cycle it says so rather than doing nothing. The notice names the
    /// directory's *path*, because two checkouts can share a name and the chip
    /// itself only has room for the name.
    pub fn cycle_pending_workspace(&mut self) {
        let choices = self.workspace_choices();
        if choices.len() < 2 {
            self.notice = Some(t(self.lang, Key::WorkspaceNone).into());
            return;
        }
        let current = self.shown_cwd().unwrap_or_default();
        // Every chat directory is the one 对话会话 choice, whichever session's
        // scratch tree the conversation currently runs in.
        let next = choices
            .iter()
            .position(|path| {
                same_path(path, &current)
                    || (is_chat_directory(path) && is_chat_directory(&current))
            })
            .map(|index| (index + 1) % choices.len())
            .unwrap_or(0);
        let path = choices[next].clone();
        self.pending_cwd = Some(path.clone());
        self.notice = Some(i18n::render(
            self.lang,
            Key::WorkspaceChosen,
            &[&shorten(&path)],
        ));
        // `@` and the files panel follow the directory the next session will run
        // in (see `refresh_local`).
        self.refresh_local();
    }

    /// The model a picker choice is made for: the staged one while detached,
    /// otherwise the agent's own.
    fn choice_model(&self) -> Option<&ModelInfo> {
        // The model in force, staged or not: on the launchpad that is the
        // agent's default (see `shown_model`), and the launchpad's own choice of
        // wire vocabulary has to follow the model the chip names.
        let id = self.shown_model()?;
        find_model(&self.models, &id)
    }

    /// The level to send over the wire.
    ///
    /// The reserved `default` is only understood by an agent that advertises
    /// `thinkingLevels`; an older agent's models fall back to the legacy
    /// vocabulary, whose default is `medium` (see [`legacy_wire_thinking_level`]).
    /// An agent whose model list has not loaded yet is assumed current.
    fn wire_thinking_level(&self, level: &str) -> String {
        let current_agent = self
            .choice_model()
            .map(|model| model.thinking_levels.is_some())
            .unwrap_or(true);
        if current_agent {
            level.to_string()
        } else {
            legacy_wire_thinking_level(level).to_string()
        }
    }

    /// Stage a model chosen while nothing is attached — the picker's row or the
    /// launchpad's own chip. Nothing is sent: the agent has no session to set a
    /// model on ("session not found"), and the session the next message creates
    /// is asked for with this.
    pub fn set_pending_model(&mut self, model_id: &str) {
        self.pending_model = Some(model_id.to_string());
        self.notice = None;
    }

    /// Stage a reasoning level chosen while nothing is attached.
    pub fn set_pending_effort(&mut self, level: &str) {
        self.pending_effort = Some(level.to_string());
        self.notice = None;
    }

    /// The staged model, if any — what the driver persists as the terminal UI's
    /// default (`settings.json` `defaultModel`).
    pub fn pending_model(&self) -> Option<&str> {
        self.pending_model.as_deref()
    }

    /// The staged reasoning level, if any (`defaultThinkingLevel`).
    pub fn pending_effort(&self) -> Option<&str> {
        self.pending_effort.as_deref()
    }

    async fn new_session(&mut self) {
        let cwd = self.shown_cwd().unwrap_or_default();
        // What the reader chose on the launchpad is what this session starts
        // with; with nothing staged those are the product defaults the chips
        // already name — the agent's default model and starting level, the
        // 对话会话 chat, 完全访问.
        let model = self.shown_model().unwrap_or_default();
        let level = self.wire_thinking_level(&self.shown_effort());
        let mode = crate::approval_mode::ApprovalMode::from_wire(&self.shown_mode());
        // A chat's directory is named after its session id — the desktop app's
        // own `~/.future/workspaces/chat/<id>` convention, so a chat created
        // here is filed as a chat there instead of as a project called
        // `workspace`. The id is minted here so the directory exists before the
        // agent opens the session. A directory the reader typed is theirs to
        // have made, and a typo deserves the agent's error.
        let chat = cwd.trim().is_empty() || is_chat_directory(&cwd);
        let (session_id, cwd) = if chat {
            let id = crate::session_rows::random_id();
            let directory = chat_dir_for(&id);
            if let Err(error) = std::fs::create_dir_all(&directory) {
                self.notice = Some(i18n::render(
                    self.lang,
                    Key::WorkflowFailed,
                    &[&error.to_string()],
                ));
                return;
            }
            (Some(id), directory)
        } else {
            (None, cwd)
        };
        match self
            .client
            .new_session(
                session_id.as_deref(),
                (!cwd.is_empty()).then_some(cwd.as_str()),
                (!model.is_empty()).then_some(model.as_str()),
                (!level.is_empty()).then_some(level.as_str()),
            )
            .await
        {
            Ok(_) => {
                self.launchpad = false;
                self.drafts.remove("launchpad");
                // The agent now owns the choice (the session was created with
                // it), so the staging is spent — keeping it would let a stale
                // pick override the session's own on the next `/new`.
                self.pending_model = None;
                self.pending_effort = None;
                self.pending_cwd = None;
                self.pending_mode = None;
                // The approval mode has no `new_session` argument: it is one
                // call after the session exists. The launchpad always names one
                // (its default is 完全访问), so every session this surface
                // starts is told — a chip that named a mode the session did not
                // run under would be a lie.
                if let Some(mode) = mode {
                    match self.client.set_sandbox_policy(mode).await {
                        Ok(_) => self.state.sandbox_tier = Some(mode.as_str().to_string()),
                        Err(error) => {
                            self.notice = Some(format!("Could not set the approval mode: {error}"))
                        }
                    }
                }
                self.refresh_state().await;
                // A brand-new session has no run of its own, and the one this
                // client left behind (the launchpad's `/new`) is not its.
                self.forget_run();
                self.refresh_sessions().await;
                self.load_tail().await;
            }
            Err(error) => self.notice = Some(format!("Could not start a session: {error}")),
        }
    }

    /// Apply one model by id (the `/model` picker's row).
    async fn set_model(&mut self, model_id: &str) {
        match self.client.set_model(model_id).await {
            Ok(()) => {
                self.notice = None;
                self.refresh_state().await;
            }
            Err(error) => self.notice = Some(format!("Could not set the model: {error}")),
        }
    }

    /// Apply one reasoning level (the model picker's slider).
    async fn set_thinking(&mut self, level: &str) {
        let level = self.wire_thinking_level(level);
        match self.client.set_thinking_level(&level).await {
            Ok(()) => {
                self.notice = None;
                self.refresh_state().await;
            }
            Err(error) => self.notice = Some(format!("Could not set thinking: {error}")),
        }
    }

    async fn cycle_model(&mut self) {
        // The launchpad's chip: same gesture, staged instead of sent.
        if self.detached() {
            self.cycle_pending_model();
            return;
        }
        match self.client.cycle_model().await {
            Ok(value) => {
                if let Some(model) = value.get("model").and_then(Value::as_str) {
                    self.state.model = Some(model.to_string());
                }
            }
            Err(error) => self.notice = Some(format!("Could not cycle the model: {error}")),
        }
    }

    async fn cycle_thinking(&mut self) {
        if self.detached() {
            self.cycle_pending_effort();
            return;
        }
        match self.client.cycle_thinking_level().await {
            Ok(value) => {
                if let Some(level) = value.get("level").and_then(Value::as_str) {
                    self.state.thinking_level = level.to_string();
                }
            }
            Err(error) => self.notice = Some(format!("Could not cycle thinking: {error}")),
        }
    }

    /// The next model in the agent's own order, staged for the next session.
    ///
    /// The list is walked as references (`future/deepseek-flash`), and the
    /// current one is resolved the same way so a state naming its model with
    /// the provider still finds its place in the list.
    fn cycle_pending_model(&mut self) {
        // The references, not the bare ids: two providers may offer one id, and
        // staging the bare id would land the session on whichever of them the
        // agent resolves first.
        let references: Vec<String> = self
            .models
            .iter()
            .map(|model| model_reference(&model.provider, &model.id))
            .collect();
        if references.is_empty() {
            // Not loaded yet (`refresh_models` runs after the first state):
            // say so rather than silently doing nothing.
            self.notice = Some("No models to choose from yet".into());
            return;
        }
        let current = self
            .pending_model
            .clone()
            .or_else(|| self.state.model.clone());
        let here = current
            .as_deref()
            .and_then(|current| find_model(&self.models, current))
            .map(|model| model_reference(&model.provider, &model.id));
        let index = here
            .and_then(|reference| {
                references
                    .iter()
                    .position(|candidate| *candidate == reference)
            })
            .map(|index| index + 1)
            .unwrap_or(0);
        let next = references[index % references.len()].clone();
        self.set_pending_model(&next);
    }

    /// The next reasoning level, staged for the next session.
    ///
    /// The ring is the staged model's own declaration (the launchpad can pick a
    /// model before a session exists), falling back to the legacy vocabulary for
    /// an older agent. An unknown current level starts the ring at `default`, so
    /// the first press lands on the first real choice rather than skipping it.
    fn cycle_pending_effort(&mut self) {
        let model_id = self
            .pending_model
            .as_deref()
            .or(self.state.model.as_deref());
        let levels = level_ring(&self.models, model_id);
        let current = self
            .pending_effort
            .clone()
            .unwrap_or_else(|| self.shown_effort());
        if let Some(next) = next_thinking_level(&levels, &current) {
            self.set_pending_effort(&next);
        }
    }

    /// The approval mode, in the desktop composer's order.
    async fn cycle_mode(&mut self) {
        use crate::approval_mode::ApprovalMode;
        let current = self.shown_mode();
        let index = ApprovalMode::ALL
            .iter()
            .position(|mode| mode.as_str() == current)
            .unwrap_or(ApprovalMode::ALL.len() - 1);
        let next = ApprovalMode::ALL[(index + 1) % ApprovalMode::ALL.len()];
        self.set_approval(next.as_str()).await;
    }

    /// Apply one approval mode — the `/mode` menu's row, a click on the
    /// launchpad's chip, or the next step of the cycle.
    ///
    /// With no session attached it is staged like the model and the level: the
    /// agent answers `set_sandbox_policy` with "session not found" otherwise,
    /// and the reader would see a red row for a choice that was perfectly fine.
    pub async fn set_approval(&mut self, mode: &str) {
        use crate::approval_mode::ApprovalMode;
        let Some(mode) = ApprovalMode::from_wire(mode) else {
            self.notice = Some(i18n::render(self.lang, Key::NoticeModeUnknown, &[&mode]));
            return;
        };
        if self.detached() {
            self.pending_mode = Some(mode.as_str().to_string());
            self.notice = None;
            return;
        }
        match self.client.set_sandbox_policy(mode).await {
            Ok(_) => {
                self.state.sandbox_tier = Some(mode.as_str().to_string());
                self.notice = None;
            }
            Err(error) => self.notice = Some(format!("Could not set the approval mode: {error}")),
        }
    }

    /// Answer the parked approval. Rules are installed first; any failure
    /// leaves the request pending rather than silently degrading to allow-once.
    pub async fn decide(&mut self, approved: bool) {
        self.decide_with_rule(approved, false).await;
    }

    pub async fn decide_with_rule(&mut self, approved: bool, with_rule: bool) {
        let Some(prompt) = self.approval.clone() else {
            return;
        };
        if with_rule {
            let rules = super::workflows::approval_of(&prompt.raw).rules;
            if rules.is_empty() {
                self.notice = Some(t(self.lang, Key::ApprovalNoRule).into());
                return;
            }
            for rule in rules {
                if let Err(error) = self.client.add_session_rule(&rule.path, &rule.access).await {
                    self.notice = Some(i18n::render(self.lang, Key::ApprovalRuleFailed, &[&error]));
                    return;
                }
            }
        }
        if let Err(error) = self
            .client
            .approval_decision(&prompt.request_id, approved, "")
            .await
        {
            self.notice = Some(format!("Could not answer the approval: {error}"));
            return;
        }
        self.approval = None;
        self.notice = if with_rule {
            Some(t(self.lang, Key::ApprovalRuleAdded).into())
        } else {
            None
        };
    }

    pub fn selected_diff_path(&self, selected: usize) -> String {
        self.diff
            .as_ref()
            .and_then(|diff| diff.files.get(selected).or_else(|| diff.files.first()))
            .map(|file| file.path.clone())
            .unwrap_or_default()
    }

    pub async fn load_fork_points(&mut self) {
        let session_id = self.state.session_id.clone();
        match self.client.fork_messages_for(&session_id).await {
            Ok(value) => {
                self.fork_points = value
                    .get("messages")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|message| {
                        let id = message.get("id")?.as_str()?.to_string();
                        let text = message
                            .get("blocks")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .find_map(|block| block.get("text").and_then(Value::as_str))
                            .or_else(|| message.get("content").and_then(Value::as_str))
                            .unwrap_or("")
                            .to_string();
                        Some(super::state::ForkPoint { id, text })
                    })
                    .collect();
            }
            Err(error) => {
                self.notice = Some(i18n::render(self.lang, Key::WorkflowFailed, &[&error]))
            }
        }
    }

    pub async fn fork_at(&mut self, entry_id: &str) {
        let parent = self.state.session_id.clone();
        self.save_draft();
        match self.client.fork_from(&parent, entry_id).await {
            Ok(value) => {
                self.launchpad = false;
                self.notice = None;
                self.refresh_state().await;
                // A fork is another conversation: the run in flight here is
                // the parent's to finish, not something to show on it.
                self.adopt_run();
                self.refresh_sessions().await;
                self.load_tail().await;
                self.restore_draft();
                if value.get("sessionId").and_then(Value::as_str).is_none() {
                    self.notice = Some(t(self.lang, Key::ForkEmptyId).into());
                }
            }
            Err(error) => {
                self.notice = Some(i18n::render(self.lang, Key::WorkflowFailed, &[&error]))
            }
        }
    }

    pub fn set_diff(&mut self, diff: super::state::DiffView) {
        self.diff = Some(diff);
    }

    /// Install a freshly read `/git` panel. The view carries its own section and
    /// selection, so the surface keeps painting what the driver last read while
    /// a reload is in flight.
    pub fn set_git(&mut self, git: Option<super::state::GitView>) {
        self.git = git;
    }

    /// The `/git` view as the driver last read it.
    pub fn git(&self) -> Option<&super::state::GitView> {
        self.git.as_ref()
    }

    pub fn set_worktrees(&mut self, rows: Vec<crate::worktree::WorktreeInfo>) {
        let cwd = self.state.cwd.clone().unwrap_or_default();
        self.worktrees = rows
            .iter()
            .map(|row| super::state::WorktreeView {
                path: row.path.clone(),
                label: row.name(),
                description: row.describe(),
                current: row.is_current(&cwd),
            })
            .collect();
    }

    pub fn refresh_worktrees(&mut self) {
        let cwd = self.state.cwd.clone().unwrap_or_default();
        let result = match crate::worktree::probe_repo(
            &crate::worktree::GitCli::default(),
            &cwd,
            std::env::current_dir().ok().as_deref(),
        ) {
            Ok(rows) => {
                self.worktrees = rows
                    .iter()
                    .map(|row| super::state::WorktreeView {
                        path: row.path.clone(),
                        label: row.name(),
                        description: row.describe(),
                        current: row.is_current(&cwd),
                    })
                    .collect();
                Ok(())
            }
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            self.notice = Some(i18n::render(self.lang, Key::WorkflowFailed, &[&error]));
        }
    }

    pub async fn select_worktree(&mut self, path: &str) {
        if path == self.state.cwd.clone().unwrap_or_default() {
            return;
        }
        match self.client.set_cwd(path).await {
            Ok(()) => {
                self.refresh_state().await;
                self.refresh_worktrees();
                self.diff = None;
            }
            Err(error) => {
                self.notice = Some(i18n::render(self.lang, Key::WorkflowFailed, &[&error]))
            }
        }
    }

    /// Move the session to no workspace at all — the plain chat the workspace
    /// picker lists above the directories.
    ///
    /// Two roads again, decided by whether anything is attached. With no
    /// session the chat is staged and the first message mints its directory
    /// (see [`Self::new_session`]); with one, the agent moves it to *this
    /// session's* chat directory, `~/.future/workspaces/chat/<id>` — the
    /// desktop app's own convention, so the two surfaces file the conversation
    /// in the same place. The directory is created here rather than left to the
    /// agent: a session parked in a directory that is not there has no tool that
    /// works, and nobody else is going to make this one — a directory the reader
    /// typed is theirs, but this path is the app's.
    pub async fn select_chat_workspace(&mut self) {
        if !self.detached() && is_chat_directory(&self.state.cwd.clone().unwrap_or_default()) {
            // Already a chat: the row the reader would leave is the row they are
            // on, which is what `esc` gives too.
            self.notice = None;
            return;
        }
        let notice = i18n::render(
            self.lang,
            Key::WorkspaceChosen,
            &[&t(self.lang, Key::WorkspaceChat)],
        );
        if self.detached() {
            // No session yet, so no id to name the chat directory after:
            // staging no workspace *is* the chat, and the first message mints
            // the directory.
            self.pending_cwd = None;
            self.notice = Some(notice);
            // `@` and the files panel follow the directory the next session
            // will run in (see `refresh_local`).
            self.refresh_local();
            return;
        }
        let path = chat_dir_for(&self.state.session_id);
        if let Err(error) = std::fs::create_dir_all(&path) {
            self.notice = Some(i18n::render(
                self.lang,
                Key::WorkflowFailed,
                &[&error.to_string()],
            ));
            return;
        }
        match self.client.set_cwd(&path).await {
            Ok(()) => {
                self.refresh_state().await;
                self.refresh_worktrees();
                self.diff = None;
                self.notice = Some(notice);
            }
            Err(error) => {
                self.notice = Some(i18n::render(self.lang, Key::WorkflowFailed, &[&error]))
            }
        }
    }

    /// Move the session to a workspace the reader picked (or typed).
    ///
    /// Two roads, decided by whether anything is attached — the same split the
    /// model and the effort take. With no session the directory is staged for
    /// the one the next message creates; with one, the agent moves it, and the
    /// rail re-files the conversation under its new workspace.
    pub async fn select_workspace(&mut self, path: &str) {
        let path = path.trim().trim_end_matches(['/', '\\']);
        if path.is_empty() {
            return;
        }
        let notice = i18n::render(self.lang, Key::WorkspaceChosen, &[&shorten(path)]);
        if self.detached() {
            self.pending_cwd = Some(path.to_string());
            self.notice = Some(notice);
            // `@` and the files panel follow the directory the next session
            // will run in (see `refresh_local`).
            self.refresh_local();
            return;
        }
        if same_path(&self.state.cwd.clone().unwrap_or_default(), path) {
            self.notice = None;
            return;
        }
        match self.client.set_cwd(path).await {
            Ok(()) => {
                self.refresh_state().await;
                self.refresh_worktrees();
                self.diff = None;
                self.notice = Some(notice);
            }
            Err(error) => {
                self.notice = Some(i18n::render(self.lang, Key::WorkflowFailed, &[&error]))
            }
        }
    }

    // ─── the 文件 panel, the preview box and the clipboard ──────────────

    /// Hand a link to the desktop's own handler — a browser for an address, a
    /// mail client for `mailto:`.
    ///
    /// Started, not waited on: the browser is the reader's program, and a TUI
    /// that blocked until it exited would freeze for as long as they kept a tab
    /// open.
    fn open_url(&mut self, url: &str) {
        if let Err(error) = open_external(url) {
            self.notice = Some(i18n::render(
                self.lang,
                Key::NoticeOpenLinkFailed,
                &[&url, &error.to_string()],
            ));
        }
    }

    /// Open or close a directory in the tree.
    ///
    /// The read happens on a blocking thread; the panel keeps the rows it has
    /// until the answer lands (`poll_local_reads`).
    pub fn toggle_folder(&mut self, path: &str) {
        let mut tree = self.tree.clone();
        let path = std::path::PathBuf::from(path);
        self.tree_revision = self.tree_revision.wrapping_add(1);
        let revision = self.tree_revision;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            tree.toggle(&path);
            let _ = tx.send(tree);
        });
        self.tree_read = Some((revision, rx));
    }

    /// Read a file for the preview box. The box opens immediately with a
    /// loading row; `poll_preview` installs the bytes when the blocking read
    /// finishes.
    pub fn start_preview(&mut self, path: &str) {
        let path = path.to_string();
        self.preview_revision = self.preview_revision.wrapping_add(1);
        let revision = self.preview_revision;
        self.preview = Some(PreviewView::Loading { path: path.clone() });
        self.notice = None;
        let lang = self.lang;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let kind = super::files::preview_kind(std::path::Path::new(&path));
            let view = read_preview_lang(&path, kind, lang);
            let siblings = if matches!(view, PreviewView::Image { .. }) {
                super::files::image_siblings(&path)
            } else {
                (Vec::new(), 0)
            };
            let _ = tx.send(PreviewLoaded {
                revision,
                view,
                siblings,
            });
        });
        self.preview_read = Some((revision, rx));
    }

    /// Install a finished preview and answer whether the frame changed.
    pub fn poll_preview(&mut self) -> bool {
        let Some((_revision, rx)) = self.preview_read.as_ref() else {
            return false;
        };
        match rx.try_recv() {
            Ok(loaded) => {
                self.preview_read = None;
                if loaded.revision != self.preview_revision {
                    return false;
                }
                self.preview = Some(loaded.view);
                self.preview_siblings = Some(loaded.siblings);
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.preview_read = None;
                false
            }
        }
    }

    /// Take the sibling list a finished preview brought, for the preview box's
    /// step buttons. Kept here until the driver asks for it.
    pub fn take_preview_siblings(&mut self) -> Option<(Vec<String>, usize)> {
        self.preview_siblings.take()
    }

    /// Show the newest image already attached to the draft.
    ///
    /// The bytes are read on a blocking thread by [`Self::start_preview`]; the
    /// box opens at once with its loading row. Returns whether there is a box
    /// to open.
    pub fn load_attachment(&mut self) -> bool {
        let attachments = self.draft.editor.pending().attachments().to_vec();
        match attachments
            .iter()
            .rev()
            .find(|attachment| attachment.kind == "image")
        {
            Some(attachment) => {
                self.start_preview(&attachment.path);
                true
            }
            None => {
                self.notice = Some(t(self.lang, Key::PreviewNothing).into());
                false
            }
        }
    }

    /// Read the clipboard: an image becomes a draft attachment (the same
    /// `[Image #N]` marker a pasted image path produces), text is pasted.
    pub async fn paste_clipboard(&mut self) {
        let capture = self.clipboard.clone();
        let os = std::env::consts::OS;
        let pid = std::process::id();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let outcome = tokio::task::spawn_blocking(move || capture.capture(os, pid, now)).await;
        match outcome {
            Ok(paste::ClipboardPaste::Image { path, .. }) => {
                // `insert_text` on an image path is the existing attach road:
                // one marker, one attachment, one entry in the attachment line.
                let mut editor = self.draft.editor.clone();
                editor.paste(&path);
                self.draft = InputView::from_editor(editor, self.draft.placeholder.clone());
                self.notice = None;
            }
            Ok(paste::ClipboardPaste::Text(text)) => {
                let mut editor = self.draft.editor.clone();
                editor.paste(&text);
                self.draft = InputView::from_editor(editor, self.draft.placeholder.clone());
                self.notice = None;
            }
            Ok(paste::ClipboardPaste::Unavailable(reason)) if reason.is_empty() => {
                self.notice = Some(t(self.lang, Key::NoticeClipboardEmpty).into());
            }
            Ok(paste::ClipboardPaste::Unavailable(reason)) => {
                self.notice = Some(i18n::render(
                    self.lang,
                    Key::NoticeClipboardUnavailable,
                    &[&reason],
                ));
            }
            Err(error) => {
                self.notice = Some(i18n::render(
                    self.lang,
                    Key::NoticeClipboardUnavailable,
                    &[&error.to_string()],
                ));
            }
        }
    }

    /// Re-read the tree in the background, keeping the open directories.
    pub fn refresh_tree(&mut self) {
        let tree = self.tree.clone();
        self.tree_revision = self.tree_revision.wrapping_add(1);
        let revision = self.tree_revision;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut tree = tree;
            tree.rescan();
            let _ = tx.send(tree);
        });
        self.tree_read = Some((revision, rx));
    }

    pub async fn create_worktree_with<F>(&mut self, create: F)
    where
        F: FnOnce() -> Result<crate::worktree::WorktreeCreated, String>,
    {
        let result = match create() {
            Ok(created) => match self
                .client
                .set_cwd(&created.plan.path.to_string_lossy())
                .await
            {
                Ok(()) => {
                    self.refresh_state().await;
                    self.refresh_worktrees();
                    Ok(created.summary())
                }
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        self.notice = Some(match result {
            Ok(summary) => summary,
            Err(error) => i18n::render(self.lang, Key::WorkflowFailed, &[&error]),
        });
    }

    /// Interrupt the run in flight (`ctrl+x`).
    pub async fn abort(&mut self) {
        if let Err(error) = self.client.abort().await {
            self.notice = Some(format!("Could not stop the run: {error}"));
        } else {
            // The stopped run's calls will never report an end, and the row
            // must not keep counting them. The rate stays: that is what the
            // row shows after any run ends.
            self.activity = Activity::default();
            self.activity_start = None;
            self.run_tools.clear();
        }
    }

    /// The models, for a picker row the surface may grow later.
    pub fn models(&self) -> &[ModelInfo] {
        &self.models
    }
}

fn prompt_of(pending: &PendingApprovalState) -> ApprovalPrompt {
    ApprovalPrompt {
        request_id: pending.approval_request_id.clone(),
        title: if pending.title.is_empty() {
            "Approve tool execution".to_string()
        } else {
            pending.title.clone()
        },
        summary: pending.summary.clone(),
        raw: serde_json::to_value(pending).unwrap_or_default(),
    }
}

fn str_field(data: &Value, key: &str) -> String {
    data.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Does this event type mean the agent's cumulative usage changed?
fn wants_state_refresh(event_type: &str) -> bool {
    matches!(event_type, "usage" | "agent_end")
}

/// Fold one `usage` event into `state`: the two token totals add up, the context
/// gauge takes this step's prompt-plus-completion (the next request is exactly
/// that), and the cache counts add up so the footer's hit rate follows the run.
///
/// Returns `(completion_tokens, context_tokens)` for the caller's own tok/s
/// bookkeeping, or `None` when the payload carried no numbers at all.
fn fold_usage(state: &mut RpcSessionState, usage: Option<&Value>) -> Option<(u64, i64)> {
    let usage = usage?;
    let prompt = token_field(usage, "prompt_tokens");
    let completion = token_field(usage, "completion_tokens");
    let cache_read = token_field(usage, "cache_read_tokens");
    let cache_write = token_field(usage, "cache_write_tokens");
    if prompt == 0 && completion == 0 && cache_read == 0 && cache_write == 0 {
        return None;
    }
    state.usage.input_tokens += prompt;
    state.usage.output_tokens += completion;
    state.usage.cache_read_tokens += cache_read;
    state.usage.cache_write_tokens += cache_write;
    Some((completion.max(0) as u64, prompt + completion))
}

/// tok/s, or `None` while the sample is too short to mean anything (a rate
/// divided by a few milliseconds reads as a million tokens a second).
fn rate_per_second(tokens: u64, secs: f64) -> Option<f64> {
    if tokens == 0 || secs < 0.5 {
        return None;
    }
    Some(tokens as f64 / secs)
}

/// One numeric field of a `usage` payload, tolerating the camelCase spelling a
/// typed payload may use.
fn token_field(usage: &Value, key: &str) -> i64 {
    let camel = {
        let mut chars = key.chars();
        match chars.next() {
            Some(first) => first.to_string() + &chars.as_str().replace('_', ""),
            None => String::new(),
        }
    };
    usage
        .get(key)
        .or_else(|| usage.get(camel.as_str()))
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

/// The reads that cost a process, running beside the frame instead of in it.
///
/// Both belong to a workspace, both answer "what is in this directory", and both
/// were paid on the frame path: `git ls-files` for the `@` menu's candidates and
/// `git rev-parse` for the header's branch — 24 ms and up to
/// [`super::git::BRANCH_TIMEOUT`] respectively, spawned inside the driver loop by
/// [`Live::refresh_local`]. A reader who stepped through the rail's 工作区 tab paid
/// that per workspace, on the frame that opened the conversation.
///
/// Two threads rather than one: the branch is a fast read the header wants now,
/// the listing is a slow one only `@` needs, and coupling them would blank the
/// header while a large repository was listed.
struct LocalReads {
    /// The workspace these are for. An answer for another one is dropped: the
    /// reader has moved on and it must not paint over the workspace on screen.
    cwd: String,
    branch: Option<std::sync::mpsc::Receiver<String>>,
    files: Option<std::sync::mpsc::Receiver<(Vec<String>, super::files::FileTree)>>,
}

impl LocalReads {
    fn start(cwd: &str) -> Self {
        Self {
            cwd: cwd.to_string(),
            branch: Self::spawn(cwd, super::git::branch),
            files: Self::spawn(cwd, |path| {
                (
                    super::local::workspace_files(path),
                    super::files::FileTree::scan(&path.display().to_string()),
                )
            }),
        }
    }

    /// Run `read` on its own thread and hand back the channel its answer arrives
    /// on. A directory that is not there has nothing to read, so no thread is
    /// started and no answer is owed.
    fn spawn<T: Send + 'static>(
        cwd: &str,
        read: impl FnOnce(&std::path::Path) -> T + Send + 'static,
    ) -> Option<std::sync::mpsc::Receiver<T>> {
        let path = std::path::PathBuf::from(cwd);
        if cwd.trim().is_empty() || !path.is_dir() {
            return None;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // A reader that has moved on is gone: sending into a closed channel
            // is how the thread learns to stop caring, not an error.
            let _ = tx.send(read(&path));
        });
        Some(rx)
    }
}

/// The conversation list as the client holds it, with the version a cached view
/// of it is keyed on.
///
/// A plain `Vec` would let a caller assign a fresh list and leave the rail
/// painting the previous one: [`Live::data`] rebuilds its rows only when
/// [`Self::rev`] moves, so replacing the list *has* to move it, and this type is
/// what makes forgetting unrepresentable. The `Deref` is read-only for the same
/// reason — read through it, write through [`Self::replace`].
#[derive(Default)]
struct SessionList {
    rows: Vec<SessionSummary>,
    rev: u64,
}

impl SessionList {
    /// Replace the list, answering whether anything changed — an unchanged list
    /// keeps its version, so the rail is not re-filed (nor repainted) for a poll
    /// that read the same rows.
    fn replace(&mut self, rows: Vec<SessionSummary>) -> bool {
        if rows == self.rows {
            return false;
        }
        self.rows = rows;
        self.rev = self.rev.wrapping_add(1);
        true
    }

    fn rev(&self) -> u64 {
        self.rev
    }
}

impl std::ops::Deref for SessionList {
    type Target = [SessionSummary];

    fn deref(&self) -> &Self::Target {
        &self.rows
    }
}

/// The rail's view of the session list: rows in the order the panels want
/// them, the 工作区 filing, and the plain chats.
///
/// A frame asks for these on every paint, and building them is the part of a
/// frame that grows with the reader's history ([`Live::build_session_view`]).
/// So they are built when one of the inputs below moves and cloned otherwise.
/// A field added to [`SessionRow`] that reads anything outside those inputs
/// belongs in [`SessionViewKey`] too.
struct SessionView {
    key: SessionViewKey,
    rows: Vec<SessionRow>,
    workspaces: Vec<WorkspaceGroup>,
    chats: Vec<String>,
}

/// Everything [`SessionView`] is built from, compared per frame.
#[derive(PartialEq, Eq)]
struct SessionViewKey {
    /// [`SessionList::rev`] — the list itself.
    sessions: u64,
    /// [`Live::pins_rev`] — a pin reorders and re-files the 工作区 tab.
    pins: u64,
    /// The attached conversation (`current` on its row).
    current: String,
    /// The launchpad attaches to nothing, so no row claims to be current.
    launchpad: bool,
    lang: Language,
    /// [`age_at`] reads in minutes, so a view built in an earlier minute may
    /// print a different age for the same rows.
    minute: i64,
}

/// The 工作区 tab's filing: one group per workspace, pinned first, and the
/// conversations that belong to no workspace at all.
///
/// A conversation is a plain chat when its directory is one of the app's own
/// scratch roots — the agent's default workspace (a session started without a
/// directory) or the per-chat scratch tree the desktop hands out. Everything
/// else is filed under the directory it ran in, named by that directory and
/// identified by its path: two checkouts can share a name.
///
/// Two indexes, not two scans: each row used to look its summary up with a
/// linear walk of the session list and its group up with a linear walk of the
/// groups built so far, which made filing O(sessions²) — at five hundred
/// conversations it was most of a 10 ms frame. The identity of a workspace is
/// [`path_key`], the same normalisation [`same_path`] compares by, so two
/// spellings of one directory (`C:\work\x`, `C:/work/x/`) still land in one
/// group.
fn file_sessions(
    sessions: &[SessionSummary],
    rows: &[SessionRow],
    pinned: &[String],
    roots: &ChatRoots,
) -> (Vec<WorkspaceGroup>, Vec<String>) {
    let mut by_id: HashMap<&str, &SessionSummary> = HashMap::with_capacity(sessions.len());
    for session in sessions {
        by_id.entry(session.id.as_str()).or_insert(session);
    }
    let mut groups: Vec<WorkspaceGroup> = Vec::new();
    let mut chats: Vec<String> = Vec::new();
    let mut group_at: HashMap<String, usize> = HashMap::new();
    // `rows` is most recently spoken first, so a group keeps that order inside
    // itself and the first time a path is seen is that workspace's newest
    // activity.
    for row in rows {
        let Some(summary) = by_id.get(row.id.as_str()) else {
            continue;
        };
        if roots.is_chat(&summary.cwd) {
            chats.push(row.id.clone());
            continue;
        }
        let path = path_key(&summary.cwd);
        if path.is_empty() {
            chats.push(row.id.clone());
            continue;
        }
        match group_at.get(&path) {
            Some(&index) => groups[index].session_ids.push(row.id.clone()),
            None => {
                group_at.insert(path.clone(), groups.len());
                groups.push(WorkspaceGroup {
                    name: workspace_name(&path),
                    pinned: pinned.iter().any(|pin| same_path(pin, &path)),
                    path,
                    age: row.age.clone(),
                    session_ids: vec![row.id.clone()],
                });
            }
        }
    }
    // Pinned workspaces lead (the desktop's rail does the same); the rest keep
    // the newest-activity order the rows already carry.
    groups.sort_by_key(|group| !group.pinned);
    (groups, chats)
}

/// A workspace's identity for filing: `same_path`'s normalisation, computed
/// once per path instead of twice per comparison.
fn path_key(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.contains('\\') {
        trimmed.replace('\\', "/")
    } else {
        trimmed.to_string()
    }
}

/// Start the reader's handler for `target`: `open` on macOS, the shell's `start`
/// on Windows, `xdg-open` everywhere else.
///
/// Not a library dependency: each platform has had its own opener since long
/// before this program existed, and the one a reader configured is the one they
/// expect a link to land in.
fn open_external(target: &str) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(target);
        command
    } else if cfg!(windows) {
        // `start` is a shell builtin, and the empty string is the window title
        // it would otherwise take the URL for.
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", target]);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(target);
        command
    };
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Reaped on a thread of its own: nobody waits for a browser, and an
    // unreaped child is a zombie for as long as the TUI stays open.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// The longest side a preview image is decoded to, in pixels.
///
/// This is the resolution a *zoom* sees, so it decides how much a reader can
/// read in a dense screenshot: at 1024 a picture zoomed twice was already an
/// upscaled guess. 2048 is 16 MB of RGBA in the surface's own state — one
/// picture at a time, and the transmitted bitmap is downsampled to the cells on
/// screen anyway (see `run::transmit_bitmap`), so the extra pixels cost memory
/// when zoomed and nothing at all when fitted.
pub const MAX_PREVIEW_IMAGE_PX: u32 = 2048;

/// Largest encoded picture the preview will load. Decoding dimensions are
/// bounded below, but the compressed bytes arrive before that check: an
/// arbitrarily large file must not become an arbitrarily large allocation just
/// to be refused.
const MAX_PREVIEW_IMAGE_BYTES: u64 = 64 * 1024 * 1024;

/// Read an image file without trusting its size on disk.
fn read_image_bytes(path: &str, lang: Language) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let meta = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if meta.len() > MAX_PREVIEW_IMAGE_BYTES {
        return Err(i18n::render(
            lang,
            Key::PreviewImageTooLarge,
            &[
                &(meta.len() / (1024 * 1024)).to_string(),
                &(MAX_PREVIEW_IMAGE_BYTES / (1024 * 1024)).to_string(),
            ],
        ));
    }
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_PREVIEW_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_PREVIEW_IMAGE_BYTES {
        return Err(i18n::render(lang, Key::PreviewImageChanged, &[]));
    }
    Ok(bytes)
}

pub fn read_preview(path: &str, kind: super::files::PreviewKind) -> PreviewView {
    read_preview_lang(path, kind, Language::En)
}

fn read_preview_lang(path: &str, kind: super::files::PreviewKind, lang: Language) -> PreviewView {
    use super::files::PreviewKind;
    let preview = || -> Result<PreviewView, String> {
        match kind {
            PreviewKind::Image => {
                let bytes = read_image_bytes(path, lang)?;
                let image = image::load_from_memory(&bytes).map_err(|error| error.to_string())?;
                let image = if image.width() > MAX_PREVIEW_IMAGE_PX
                    || image.height() > MAX_PREVIEW_IMAGE_PX
                {
                    image.resize(
                        MAX_PREVIEW_IMAGE_PX,
                        MAX_PREVIEW_IMAGE_PX,
                        image::imageops::FilterType::Triangle,
                    )
                } else {
                    image
                };
                let rgba = image.to_rgba8();
                Ok(PreviewView::Image {
                    path: path.to_string(),
                    pixels: super::image_view::ImagePixels {
                        width: rgba.width() as usize,
                        height: rgba.height() as usize,
                        rgba: rgba.into_raw().into(),
                    },
                })
            }
            PreviewKind::Markdown => {
                let (text, _) = read_text(path)?;
                Ok(PreviewView::Markdown {
                    path: path.to_string(),
                    text: text.into(),
                })
            }
            PreviewKind::Text => {
                let (text, truncated) = read_text(path)?;
                Ok(PreviewView::Text {
                    path: path.to_string(),
                    lines: super::syntax::highlight(path, &text).into(),
                    truncated,
                })
            }
            PreviewKind::Opaque => Ok(PreviewView::Unavailable {
                path: path.to_string(),
                // Empty: the box's own "nothing to preview" line is the text,
                // and it is drawn in the reader's language.
                reason: String::new(),
            }),
        }
    };
    preview().unwrap_or_else(|reason| PreviewView::Unavailable {
        path: path.to_string(),
        reason,
    })
}

/// Read at most [`files::MAX_TEXT_PREVIEW_BYTES`] of a file as text.
///
/// Bounded on the *read*, not after it: a 2 GB log is never in memory, and a
/// preview is not a reason to build it.
fn read_text(path: &str) -> Result<(String, bool), String> {
    use std::io::Read;
    let cap = super::files::MAX_TEXT_PREVIEW_BYTES as usize;
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes: Vec<u8> = Vec::with_capacity(cap.min(64 * 1024));
    file.take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let truncated = bytes.len() > cap;
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(cap)]).to_string();
    let (text, cut) = bound(&text, cap);
    Ok((text, truncated || cut))
}

/// Cut `text` at a character boundary at or below `limit` bytes.
fn bound(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_string(), false);
    }
    let mut cut = limit;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    (text[..cut].to_string(), true)
}

fn cache_percent(state: &RpcSessionState) -> u8 {
    let read = state.usage.cache_read_tokens.max(0) as u64;
    let input = state.usage.input_tokens.max(0) as u64;
    let total = read + input;
    if total == 0 {
        0
    } else {
        ((read as f64 / total as f64) * 100.0).round() as u8
    }
}

fn display_name(session: &SessionSummary) -> String {
    session
        .session_name
        .clone()
        .or_else(|| session.first_message.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| session.id.chars().take(8).collect())
}

/// `~/future-os` — the row's workspace column has room for the home-relative
/// form only.
fn shorten(cwd: &str) -> String {
    let home = crate::home::home_dir_or_default().display().to_string();
    if !home.is_empty() && cwd == home {
        "~".to_string()
    } else if !home.is_empty() && cwd.starts_with(&format!("{home}/")) {
        format!("~{}", &cwd[home.len()..])
    } else {
        cwd.to_string()
    }
}

/// A workspace's heading name: the directory's own name, or the whole path
/// when there is no last segment to take (`/`).
fn workspace_name(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let name = trimmed
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(trimmed)
        .to_string();
    if name.is_empty() {
        path.to_string()
    } else {
        name
    }
}

/// Do two spellings name the same directory? Trailing separators and the two
/// separator characters are the only differences a pin has to survive; a
/// case-insensitive filesystem is not assumed (this is not a filesystem check).
fn same_path(left: &str, right: &str) -> bool {
    let normalise = |path: &str| path.replace('\\', "/").trim_end_matches('/').to_string();
    normalise(left) == normalise(right)
}

/// Where this installation keeps the conversations that belong to no
/// workspace, resolved once and then asked about every row.
///
/// [`is_chat_directory`] answers the same question, but it reads the
/// installation's directory and builds two `format!`s on *every* call — and a
/// frame asks it once per conversation in the rail (twice, before
/// [`Live::build_session_view`] hoisted the clock and this). One read per
/// rebuild, one comparison per row.
struct ChatRoots {
    /// `{state}/agent/workspace` — the placeholder a chat names before it has a
    /// session, and where chats created by older clients still run.
    workspace: String,
    /// `{state}/workspaces/chat/` — the per-chat scratch tree the desktop makes.
    chat_prefix: String,
}

impl ChatRoots {
    fn detect() -> Self {
        let root = crate::home::future_state_dir()
            .display()
            .to_string()
            .replace('\\', "/");
        let root = root.trim_end_matches('/').to_string();
        Self {
            workspace: format!("{root}/agent/workspace"),
            chat_prefix: format!("{root}/workspaces/chat/"),
        }
    }

    /// Is `cwd` one of the app's own scratch directories — a conversation with
    /// no workspace of its own?
    ///
    /// Two of them exist: the agent's default workspace and the per-chat
    /// scratch tree. A directory the reader chose is never one of these, so a
    /// project's conversation is always filed.
    fn is_chat(&self, cwd: &str) -> bool {
        let cwd = cwd.trim_end_matches(['/', '\\']);
        if cwd.is_empty() {
            return true;
        }
        // Only a Windows spelling needs the `\\` → `/` rewrite, and that
        // rewrite allocates: the common path compares in place.
        if cwd.contains('\\') {
            let normalized = cwd.replace('\\', "/");
            return self.matches(&normalized);
        }
        self.matches(cwd)
    }

    fn matches(&self, normalized: &str) -> bool {
        normalized == self.workspace || normalized.starts_with(&self.chat_prefix)
    }

    /// The workspace label a conversation row prints: its directory's name, or
    /// nothing at all for a plain chat.
    fn workspace_label(&self, cwd: &str) -> String {
        if self.is_chat(cwd) {
            String::new()
        } else {
            workspace_name(cwd)
        }
    }
}

/// Is `cwd` one of the app's own scratch directories — a conversation with no
/// workspace of its own? See [`ChatRoots`], which is what a loop should hold.
fn is_chat_directory(cwd: &str) -> bool {
    ChatRoots::detect().is_chat(cwd)
}

/// The placeholder a chat names before it has a session id, and where chats
/// created before this client filed them under the app's scratch tree still
/// run.
///
/// A chat's real directory is [`chat_dir_for`] — `<state>/workspaces/chat/<id>`,
/// the desktop app's own convention — but the launchpad stages "no workspace"
/// before the id exists, and the header, the `@` list and the files panel need
/// some directory to read. This is that directory: the agent's own default
/// workspace, which [`is_chat_directory`] still classifies as a chat.
fn chat_workspace() -> String {
    crate::home::future_state_dir()
        .join("agent")
        .join("workspace")
        .to_string_lossy()
        .to_string()
}

/// The root of the app's per-conversation scratch tree — the same directory the
/// desktop app files a chat under, so both surfaces name one conversation's
/// directory the same way.
fn chat_root() -> std::path::PathBuf {
    crate::home::future_state_dir()
        .join("workspaces")
        .join("chat")
}

/// The directory a chat runs in: `<chat root>/<session id>`.
///
/// The id is the agent session id (see [`Live::new_session`]), which is also
/// the name the desktop gives the imported chat workspace; the two surfaces
/// therefore agree on where a conversation lives.
fn chat_dir_for(session_id: &str) -> String {
    chat_root().join(session_id).to_string_lossy().to_string()
}

/// The directory the TUI process was started from, when it can be a workspace.
///
/// `cd project && future tui` starts the first conversation in that project
/// rather than in the app's scratch tree; a launch from one of those scratch
/// directories is still a chat, and so is a directory that cannot be resolved.
fn launch_workspace() -> Option<String> {
    let dir = std::env::current_dir().ok()?;
    let path = dir.to_string_lossy().to_string();
    if path.trim().is_empty() || is_chat_directory(&path) {
        return None;
    }
    Some(path)
}

/// Milliseconds since the epoch, read once per frame rather than once per row.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// `age`, against a clock the caller already read — a list of conversations
/// asks this once per row, and a `SystemTime::now` per row is a syscall per row.
fn age_at(now: i64, updated_at_ms: i64) -> String {
    let delta = now.saturating_sub(updated_at_ms);
    if delta < 60_000 {
        return "now".to_string();
    }
    let minutes = delta / 60_000;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = hours / 24;
    if days < 7 {
        return format!("{days}d");
    }
    format!("{}w", days / 7)
}

/// The palette persisted in settings, or the workbench default.
pub fn persisted_palette(settings: &crate::settings::TuiSettings) -> super::theme::Palette {
    super::theme::resolve(settings.theme_id.as_deref())
}

/// The language preference `settings.json` carries. An absent key means `system`
/// — the reader has not chosen, so the locale decides.
pub fn persisted_language(settings: &crate::settings::TuiSettings) -> LanguagePreference {
    LanguagePreference::from_wire(settings.language.as_deref().unwrap_or("system"))
}

/// The text a completed file mention inserts into the draft.
///
/// A path with whitespace, a quote or a backslash is quoted, because the
/// submission parser has to be able to tell where the path ends. Escape and
/// quote are the only characters that need `\\` inside the quotes.
pub(crate) fn mention_insert(path: &str) -> String {
    if path
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '"' | '\\'))
    {
        let mut quoted = String::with_capacity(path.len() + 4);
        for ch in path.chars() {
            if matches!(ch, '"' | '\\') {
                quoted.push('\\');
            }
            quoted.push(ch);
        }
        format!("@\"{quoted}\" ")
    } else {
        format!("@{path} ")
    }
}

/// A draft with its `@path` mentions inlined.
///
/// `@` completion is a *local* affordance: the agent receives a prompt, not a
/// path it would resolve itself, and only this side knows which directory the
/// reader means. So every `@path` that names a readable file under the session's
/// working directory becomes a `<file name="…">…</file>` block — exactly the
/// shape `future run @README.md` sends. A path with spaces is written
/// `@"my file.md"`; the quotes are syntax, not part of the path.
///
/// The limits are stated rather than silent: at most [`MAX_MENTION_FILES`] files
/// and [`MAX_MENTION_BYTES`] each; a file that is skipped (too large, binary,
/// unreadable) is named in the prompt with the reason, so the model is told what
/// it did not get. A token that names no file (`@team`) stays as typed — it may
/// still mean something to the reader's sentence.
pub fn expand_mentions(text: &str, cwd: &str) -> String {
    const MAX_MENTION_FILES: usize = 8;
    const MAX_MENTION_BYTES: usize = 256 * 1024;

    if !text.contains('@') {
        return text.to_string();
    }
    let root = std::path::Path::new(cwd);
    let mut out = String::with_capacity(text.len());
    let mut inlined = 0usize;
    let mut at = 0usize;
    while at < text.len() {
        let ch = text[at..].chars().next().unwrap_or(' ');
        if ch.is_whitespace() {
            out.push(ch);
            at += ch.len_utf8();
            continue;
        }
        let (end, raw, value) = mention_word(text, at);
        let Some(rest) = value.strip_prefix('@') else {
            out.push_str(raw);
            at = end;
            continue;
        };
        // Trailing punctuation belongs to the sentence, not the path — and it is
        // put back after the block, so the reader's comma survives the inlining.
        let path = rest.trim_end_matches([',', '。', ':', ';', ')']);
        let tail = &rest[path.len()..];
        if path.is_empty() || inlined >= MAX_MENTION_FILES {
            out.push_str(raw);
            at = end;
            continue;
        }
        let absolute = if std::path::Path::new(path).is_absolute() {
            std::path::PathBuf::from(path)
        } else {
            root.join(path)
        };
        match read_mention(&absolute, MAX_MENTION_BYTES) {
            Ok(MentionRead::TooLarge(bytes)) => {
                out.push_str(&format!(
                    "[{path}: {} KB, not inlined - read it in pieces]", // i18n: prompt text for the model
                    bytes / 1024
                ));
                out.push_str(tail);
            }
            Ok(MentionRead::Binary) => {
                out.push_str(&format!(
                    "[{path}: binary, not inlined]" // i18n: prompt text for the model
                ));
                out.push_str(tail);
            }
            Ok(MentionRead::Text(bytes)) => {
                out.push_str(&format!(
                    "<file name=\"{}\">\n{}\n</file>",
                    absolute.display(),
                    String::from_utf8_lossy(&bytes)
                ));
                out.push_str(tail);
                inlined += 1;
            }
            Err(_) => out.push_str(raw),
        }
        at = end;
    }
    out
}

/// One whitespace-delimited word, with quoted runs allowed to contain spaces.
///
/// Returns `(byte offset after the word, raw source slice, unquoted value)`.
/// Only `\"` and `\\` are escapes, and only inside quotes: every other
/// backslash belongs to the path exactly as typed.
fn mention_word(text: &str, start: usize) -> (usize, &str, String) {
    let mut at = start;
    let mut value = String::new();
    let mut quoted = false;
    while at < text.len() {
        let ch = text[at..].chars().next().unwrap_or(' ');
        if ch == '"' {
            quoted = !quoted;
            at += 1;
            continue;
        }
        if ch == '\\' && quoted {
            at += 1;
            if at < text.len() {
                let escaped = text[at..].chars().next().unwrap_or('\\');
                value.push(escaped);
                at += escaped.len_utf8();
            } else {
                value.push('\\');
            }
            continue;
        }
        if !quoted && ch.is_whitespace() {
            break;
        }
        value.push(ch);
        at += ch.len_utf8();
    }
    (at, &text[start..at], value)
}

enum MentionRead {
    Text(Vec<u8>),
    TooLarge(u64),
    Binary,
}

/// Read a mention without trusting the file's size: metadata decides whether a
/// file is too large before a byte is allocated, and the read itself is capped.
fn read_mention(path: &std::path::Path, limit: usize) -> std::io::Result<MentionRead> {
    use std::io::Read;
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(std::io::Error::other("not a file")); // i18n: internal control flow, never drawn
    }
    if meta.len() > limit as u64 {
        return Ok(MentionRead::TooLarge(meta.len()));
    }
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Ok(MentionRead::TooLarge(bytes.len() as u64));
    }
    if bytes.iter().take(4096).any(|byte| *byte == 0) {
        return Ok(MentionRead::Binary);
    }
    Ok(MentionRead::Text(bytes))
}

#[cfg(test)]
mod mention_tests {
    use super::{expand_mentions, mention_insert};

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a scratch directory");
        std::fs::write(dir.path().join("note.md"), "hello from the file").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/deep.rs"), "fn main() {}").unwrap();
        dir
    }

    /// The prompt the agent reads is the CLI's `<file>` block, not a path it has
    /// to go and open itself: the reader's sentence stays, the token does not.
    #[test]
    fn a_quoted_mention_keeps_a_path_with_spaces_in_one_piece() {
        let dir = workspace();
        std::fs::write(dir.path().join("my note.md"), "quoted contents").unwrap();
        let cwd = dir.path().display().to_string();
        let prompt = expand_mentions("read @\"my note.md\", please", &cwd);
        assert!(
            prompt.contains("<file name=\"") && prompt.contains("my note.md\">\nquoted contents"),
            "{prompt}"
        );
        assert!(!prompt.contains("@\"my"), "the token is consumed: {prompt}");

        assert_eq!(mention_insert("src/main.rs"), "@src/main.rs ");
        assert_eq!(mention_insert("my note.md"), "@\"my note.md\" ");
        assert_eq!(mention_insert("a\\b.md"), "@\"a\\\\b.md\" ");
    }

    /// The byte cap is decided from metadata and the read itself is bounded:
    /// a huge file is never allocated just to be rejected.
    #[test]
    fn an_oversized_mention_is_named_without_reading_the_whole_file() {
        let dir = workspace();
        let huge = dir.path().join("huge.txt");
        let file = std::fs::File::create(&huge).unwrap();
        file.set_len(300 * 1024).unwrap();
        let prompt = expand_mentions("@huge.txt", &dir.path().display().to_string());
        assert!(prompt.contains("not inlined"), "{prompt}");
        assert!(!prompt.contains("<file"), "{prompt}");
    }

    #[test]
    fn a_mention_becomes_the_same_file_block_the_cli_sends() {
        let dir = workspace();
        let cwd = dir.path().display().to_string();
        let prompt = expand_mentions("看一下 @note.md 的结论", &cwd);
        assert!(prompt.starts_with("看一下 "), "{prompt}");
        assert!(prompt.contains("hello from the file"), "{prompt}");
        // The name in the block is the absolute path the code actually opened,
        // so build the expectation the same way instead of pasting a separator:
        // `Path::join` yields `\` on Windows and `/` elsewhere.
        assert!(
            prompt.contains(&format!(
                "<file name=\"{}\">",
                dir.path().join("note.md").display()
            )),
            "{prompt}"
        );
        assert!(prompt.contains("</file>"), "{prompt}");
        assert!(
            !prompt.contains("@note.md"),
            "the token is replaced: {prompt}"
        );
    }

    /// Every mention that cannot be inlined says why in the prompt, and a token
    /// that names nothing stays exactly as the reader typed it.
    #[test]
    fn every_mention_kind_that_cannot_be_inlined_says_so_in_the_prompt() {
        let dir = workspace();
        let cwd = dir.path().display().to_string();
        std::fs::write(dir.path().join("blob.bin"), [0u8, 1, 2, 3]).unwrap();
        let prompt = expand_mentions("@missing.rs 和 @blob.bin 以及 @team 都在这里", &cwd);
        assert!(prompt.contains("@missing.rs"), "{prompt}");
        assert!(prompt.contains("blob.bin: binary"), "{prompt}");
        assert!(prompt.contains("@team"), "{prompt}");
    }

    /// A file past the size limit is named rather than silently dropped — and a
    /// draft with no mention is returned untouched, byte for byte.
    #[test]
    fn a_large_file_is_named_and_a_draft_without_mentions_is_unchanged() {
        let dir = workspace();
        let cwd = dir.path().display().to_string();
        std::fs::write(dir.path().join("big.txt"), "x".repeat(300 * 1024)).unwrap();
        let prompt = expand_mentions("@big.txt", &cwd);
        assert!(prompt.contains("big.txt: 300 KB"), "{prompt}");
        assert!(!prompt.contains("<file"), "{prompt}");
        assert_eq!(
            expand_mentions("no mentions here", &cwd),
            "no mentions here"
        );
        // And an e-mail address is not a mention either: the token does not start
        // with the sigil.
        assert_eq!(
            expand_mentions("mail me@example.com", &cwd),
            "mail me@example.com"
        );
        // Trailing sentence punctuation is dropped from the path, because a
        // comma after a filename is the reader's, not the file's.
        let prompt = expand_mentions("see @note.md, then the test", &cwd);
        assert!(prompt.contains("</file>, then the test"), "{prompt}");
        // A full stop is deliberately *not* trimmed: it is part of a filename as
        // often as it is the end of a sentence. What that costs depends on the
        // filesystem — and in both cases the assertions below hold the same
        // invariant, that the stop reached the filesystem instead of being cut.
        let prompt = expand_mentions("see @note.md.", &cwd);
        #[cfg(windows)]
        {
            // Win32 strips trailing dots, so `note.md.` *does* resolve here. The
            // block therefore names the path with the stop still on it: a trimmer
            // would have produced `note.md` and left the stop outside the tag.
            assert!(
                prompt.contains(&format!(
                    "<file name=\"{}\">",
                    dir.path().join("note.md.").display()
                )),
                "the trailing stop is passed through untrimmed: {prompt}"
            );
            assert!(!prompt.contains("@note.md."), "{prompt}");
        }
        #[cfg(not(windows))]
        {
            // A trailing dot is a legal file name here, so this names nothing in
            // the directory: the token stays exactly as the reader typed it.
            assert!(prompt.contains("@note.md."), "{prompt}");
            assert!(!prompt.contains("<file"), "{prompt}");
        }
    }
}

impl Live {
    /// Adopt a language preference: the adapter's own sentences (the input hint,
    /// an approval prompt) and the panels' local facts are re-read in it, and the
    /// resolved bundle is returned for the surface.
    pub fn set_language(&mut self, preference: LanguagePreference) -> Language {
        self.lang = preference.resolve();
        self.draft.placeholder = t(self.lang, Key::InputHintLaunchpad).into();
        self.lang
    }
}

/// The level ring for `model_id`: its declared list from the agent's model
/// list, or the legacy fallback when the id is unknown (not loaded yet, or from
/// an older agent that never sent one).
fn level_ring(models: &[ModelInfo], model_id: Option<&str>) -> Vec<String> {
    let declared = model_id
        .and_then(|id| find_model(models, id))
        .and_then(|model| model.thinking_levels.as_deref());
    effective_thinking_levels(declared)
}

/// The model a reference names: `provider/id` exactly, and only then a bare id
/// or an id tail for a session (or a saved setting) that names the model
/// without its provider.
///
/// The order matters as much as the lookup: with two providers offering one
/// id, a tail match can land on the other provider's model, which is how a
/// picker row ended up describing a different model's thinking levels.
fn find_model<'a>(models: &'a [ModelInfo], reference: &str) -> Option<&'a ModelInfo> {
    let reference = reference.trim();
    if reference.is_empty() {
        return None;
    }
    let qualified = |model: &ModelInfo| {
        !model.provider.is_empty()
            && reference.eq_ignore_ascii_case(&model_reference(&model.provider, &model.id))
    };
    let tail = |name: &str| name.rsplit('/').next().unwrap_or(name).to_lowercase();
    let needle = tail(reference);
    models
        .iter()
        .find(|model| qualified(model))
        .or_else(|| {
            models
                .iter()
                .find(|model| reference.eq_ignore_ascii_case(&model.id))
        })
        .or_else(|| models.iter().find(|model| tail(&model.id) == needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::types::SessionSummary;

    /// The launchpad's first message must create the session and send the
    /// prompt.
    ///
    /// `Submit` carries the text the surface read, and it must stand on its own
    /// without the draft: the driver used to clear the draft *before* handing
    /// the action over, which made every submit read an empty draft and return
    /// silently — the reported "type on the home screen, press enter, nothing
    /// happens". The sequence below is that order, kept as the contract.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // FUTURE_HOME is process-global and must stay pinned across awaits.
    async fn a_launchpad_submit_creates_the_session_and_sends_the_prompt() {
        let _guard = crate::test_env::lock();
        let home = tempfile::tempdir().unwrap();
        let _home = crate::test_env::EnvVarGuard::set("FUTURE_HOME", home.path());
        let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("new_session", r#"{"sessionId":"s-new"}"#),
            ("get_state", r#"{"sessionId":"s-new"}"#),
            ("prompt", r#"{"runId":"r-1","acceptedState":"running"}"#),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock.clone()).await;

        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        // The home screen: nothing attached, so the next message must create
        // the session.
        live.to_launchpad();
        live.set_draft("hello there".into(), 11);

        // …and the driver's own order, verbatim.
        live.set_draft(String::new(), 0);
        live.apply(WbAction::Submit("hello there".into())).await;

        let commands = seen.lock().unwrap().clone();
        assert!(
            commands.iter().any(|command| command == "new_session"),
            "the first message must create a session, saw {commands:?}"
        );
        assert!(
            commands.iter().any(|command| command == "prompt"),
            "the prompt must reach the agent, saw {commands:?}"
        );
        // A chat is filed where the desktop app files its own: under the app's
        // chat tree, named after the session id this client minted.
        let request = crate::rpc::grpc_client::tests::last_request_of(&mock, "new_session");
        assert!(
            !request.session_id.is_empty(),
            "a chat mints its session id"
        );
        let directory = home
            .path()
            .join("workspaces")
            .join("chat")
            .join(&request.session_id);
        assert_eq!(request.cwd, directory.to_string_lossy());
        assert!(directory.is_dir(), "the chat directory is created for it");
        let painted = live.chat.render_all(80).join("\n");
        assert!(
            painted.contains("hello there"),
            "the transcript keeps what was said:\n{painted}"
        );
    }

    /// `/tool-output` is the one door to a tool call's body: the chord that used
    /// to open it (`ctrl+g`) is the git browser now, and the transcript's own
    /// hint names this verb instead. It is a view toggle — the reader is asking
    /// to see what this client already has, so nothing goes to the agent — and
    /// pressing it again puts the transcript back to one row per call.
    #[tokio::test]
    async fn tool_output_toggles_the_bodies_without_asking_the_agent() {
        let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[(
            "get_state",
            r#"{"sessionId":"s1"}"#,
        )]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s1"), false, None)
            .await
            .expect("attach to the test agent");
        let before = seen.lock().unwrap().len();
        // One paint, so the toggle has a transcript to rebuild (an unpainted
        // chat only records that it owes a frame).
        live.chat.render_all(100);
        let renders = live.chat.full_renders();

        live.apply(WbAction::Command("/tool-output".into())).await;
        assert!(live.chat.tool_output_expanded(), "the bodies are open");
        assert_eq!(
            seen.lock().unwrap().len(),
            before,
            "a view toggle reaches the agent with nothing"
        );
        assert!(
            live.chat.full_renders() > renders,
            "the transcript is rebuilt for the rows it just gained"
        );

        live.apply(WbAction::Command("/tool-output".into())).await;
        assert!(!live.chat.tool_output_expanded(), "and closed again");
    }

    /// Deleting the conversation on screen opens the one that takes its place.
    ///
    /// The transcript must never stay on a conversation the agent no longer
    /// has: `delete_session` reaches the agent, and the successor in the list is
    /// opened in the same breath — which is the observable half of "the row that
    /// slid into its place opens". Opening it is a read of *that* session's
    /// state (the switch addresses both of its reads by name); it used to be a
    /// `switch_session` RPC as well, one of the three round trips a switch took.
    #[tokio::test]
    async fn deleting_the_current_session_opens_its_successor() {
        let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("get_state", r#"{"sessionId":"s1"}"#),
            (
                "list_sessions",
                r#"{"sessions":[
                    {"id":"s1","cwd":"/tmp","updatedAt":"2026-01-02","model":"m"},
                    {"id":"s2","cwd":"/tmp","updatedAt":"2026-01-01","model":"m"}
                ]}"#,
            ),
            ("delete_session", r#"{"sessionId":"s1"}"#),
            ("switch_session", r#"{"cancelled":false}"#),
        ]);
        let asked = mock.seen_sessions.clone();
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        assert_eq!(live.state.session_id, "s1", "the fixture attaches to s1");

        live.apply(WbAction::DeleteSession("s1".into())).await;

        let commands = seen.lock().unwrap().clone();
        assert!(
            commands.iter().any(|command| command == "delete_session"),
            "the delete must reach the agent, saw {commands:?}"
        );
        let opened = asked.lock().unwrap().clone();
        assert!(
            opened
                .iter()
                .any(|(command, session)| command == "get_state" && session == "s2"),
            "the successor must open, saw {opened:?}"
        );
        // The reader is told what happened — and it has to survive the switch,
        // which clears the notice row when it opens a conversation.
        let notice = live.notice.unwrap_or_default();
        assert!(
            notice.contains("Deleted") || notice.contains("已删除"),
            "{notice}"
        );
    }

    /// A restart opens folded, even from a `settings.json` that still carries
    /// the level the old `ctrl+e` used to write.
    ///
    /// `replyDetail` is retired with the persistence: peeking at the process is
    /// a view toggle for the run it happens in, and once it wrote itself back
    /// the next terminal had no way of knowing the reader had only *looked*.
    /// The key is therefore ignored on the way in and dropped on the next write
    /// (see `TuiSettings`), which makes this test the guard against a future
    /// "remember the level again" change landing without the product decision.
    #[tokio::test]
    async fn a_legacy_stored_level_does_not_open_the_transcript_expanded() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[(
            "get_state",
            r#"{"sessionId":"s-1"}"#,
        )]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let settings = crate::settings::TuiSettings::from_json(&serde_json::json!({
            "replyDetail": "detailed"
        }));

        let (live, _events) = Live::attach(&addr, None, false, Some(&settings))
            .await
            .expect("attach to the test agent");

        assert_eq!(
            live.detail(),
            ReplyDetail::Default,
            "a stored reading level must not survive into the next terminal"
        );
    }

    /// When no session can be started, the reader's text must survive: the
    /// whole point of the launchpad's first message is that it can be retried
    /// after whatever stopped the session is fixed.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // FUTURE_HOME is process-global and must stay pinned across awaits.
    async fn a_submit_that_cannot_start_a_session_keeps_the_draft() {
        let _guard = crate::test_env::lock();
        let home = tempfile::tempdir().unwrap();
        let _home = crate::test_env::EnvVarGuard::set("FUTURE_HOME", home.path());
        let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            // A `new_session` that hands back no id, and a state with none
            // either: the adapter has nothing to send a prompt to.
            ("new_session", "{}"),
            ("get_state", r#"{"sessionId":""}"#),
            ("prompt", r#"{"runId":"r-1"}"#),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;

        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        live.to_launchpad();
        live.set_draft("do not lose this".into(), 16);
        live.apply(WbAction::Submit("do not lose this".into()))
            .await;

        assert_eq!(
            live.draft.text, "do not lose this",
            "the reader's text must survive a session that could not start"
        );
        // …and the key that did nothing but keep the text has to say why: a
        // silent return is the "press enter, nothing happens" report.
        let notice = live.notice.expect("a notice explains the missing session");
        assert!(
            notice.contains("新会话") || notice.contains("session"),
            "the notice names what did not happen: {notice}"
        );
        assert!(
            notice.contains("再发一次") || notice.contains("try again"),
            "and that the draft can be sent again: {notice}"
        );
        let commands = seen.lock().unwrap().clone();
        assert!(
            !commands.iter().any(|command| command == "prompt"),
            "nothing is sent without a session, saw {commands:?}"
        );
    }

    /// The same submission with the draft intact — the ordinary path once the
    /// driver stops clearing it first — attaches the folded pastes it carries.
    /// A prompt whose acknowledgement was lost is not silently retried: the
    /// message stays visible, the draft is not restored alongside it, and the
    /// row says the delivery is unconfirmed.
    #[tokio::test]
    async fn a_prompt_without_an_acknowledgement_is_marked_not_retried() {
        let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_failing(
            &[("get_state", r#"{"sessionId":"s-1"}"#)],
            "prompt",
            "connection reset after the request",
        );
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        live.set_draft("say this once".into(), 12);
        live.apply(WbAction::Submit("say this once".into())).await;

        assert_eq!(
            seen.lock()
                .unwrap()
                .iter()
                .filter(|c| *c == "prompt")
                .count(),
            1
        );
        assert!(
            live.draft.text.is_empty(),
            "restoring the draft invites a second copy of a prompt that may have arrived"
        );
        let last = live.chat.last_message().expect("the message stays visible");
        assert_eq!(last.role, ChatRole::User);
        assert_eq!(last.run_state, Some(RunState::NotAcknowledged));
        let notice = live.notice.as_deref().unwrap_or_default();
        assert!(notice.contains("not acknowledged") || notice.contains("未得到确认"));
        assert!(notice.contains("refresh") || notice.contains("刷新"));
    }

    /// The preview box opens now and fills itself when the blocking read lands;
    /// the event loop never waits for the decode.
    #[tokio::test]
    async fn a_preview_opens_while_its_bytes_are_read_off_thread() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[(
            "get_state",
            r#"{"sessionId":"s-1"}"#,
        )]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("shot.png");
        std::fs::write(&png, png_bytes()).unwrap();
        let path = png.to_string_lossy().to_string();

        live.start_preview(&path);
        assert!(matches!(live.preview, Some(PreviewView::Loading { .. })));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !live.poll_preview() {
            assert!(std::time::Instant::now() < deadline, "preview never landed");
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(matches!(live.preview, Some(PreviewView::Image { .. })));
        let siblings = live.take_preview_siblings().expect("picture neighbours");
        assert_eq!(siblings.1, 0);
        assert_eq!(siblings.0.len(), 1);
    }

    /// A launchpad has no session to subscribe to, but a successful attach
    /// still means the agent is there. A later stream edge is what turns the
    /// dot dark.
    #[tokio::test]
    async fn the_connection_dot_follows_the_stream_and_starts_from_attach() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[(
            "get_state",
            r#"{"sessionId":"s-1"}"#,
        )]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None).await.unwrap();
        assert!(live.connected(), "a successful attach reached the agent");

        let (tx, rx) = tokio::sync::watch::channel(true);
        live.connection = rx;
        live.connection_open = true;
        tx.send(false).unwrap();
        assert!(live.connection_changed());
        assert!(!live.connected());
        assert!(!live.data(&frame()).connected);
        tx.send(true).unwrap();
        assert!(live.connection_changed());
        assert!(live.connected());
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // FUTURE_HOME is process-global and must stay pinned across awaits.
    async fn a_submit_still_sends_when_the_draft_holds_the_text() {
        let _guard = crate::test_env::lock();
        let home = tempfile::tempdir().unwrap();
        let _home = crate::test_env::EnvVarGuard::set("FUTURE_HOME", home.path());
        let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("new_session", r#"{"sessionId":"s-new"}"#),
            ("get_state", r#"{"sessionId":"s-new"}"#),
            ("prompt", r#"{"runId":"r-1","acceptedState":"running"}"#),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;

        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        live.to_launchpad();
        live.set_draft("second try".into(), 10);
        live.apply(WbAction::Submit("second try".into())).await;

        let commands = seen.lock().unwrap().clone();
        assert!(
            commands.iter().any(|command| command == "prompt"),
            "{commands:?}"
        );
        let painted = live.chat.render_all(80).join("\n");
        assert!(
            painted.contains("second try"),
            "the transcript keeps what was said:\n{painted}"
        );
    }

    fn row(id: &str, name: Option<&str>, cwd: &str) -> SessionSummary {
        SessionSummary {
            id: id.into(),
            cwd: cwd.into(),
            updated_at_ms: 0,
            created_at_ms: 0,
            model: "m".into(),
            session_name: name.map(str::to_string),
            parent_session_id: None,
            is_streaming: None,
            first_message: None,
        }
    }

    /// The workspace picker's "no workspace" row: a chat, staged while nothing
    /// is attached and applied once a session exists, in a directory this
    /// surface makes itself under the app's chat tree — a fresh home has none,
    /// and a session parked in a directory that is not there has no tool that
    /// works.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // FUTURE_HOME is process-global and must stay pinned across awaits.
    async fn the_chat_row_stages_a_session_then_moves_it() {
        let _guard = crate::test_env::lock();
        let home = tempfile::tempdir().unwrap();
        let _home = crate::test_env::EnvVarGuard::set("FUTURE_HOME", home.path());
        let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock.clone()).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        let session_chat = home.path().join("workspaces").join("chat").join("s-1");
        let notice = i18n::render(
            live.lang,
            Key::WorkspaceChosen,
            &[&t(live.lang, Key::WorkspaceChat)],
        );

        // Nothing attached: the chat is staged as *no workspace at all* — the
        // first message mints the directory, named after the session it creates.
        live.to_launchpad();
        live.select_chat_workspace().await;
        assert_eq!(live.pending_cwd, None, "a chat stages no workspace");
        assert!(
            !session_chat.is_dir(),
            "the directory is named after a session that does not exist yet"
        );
        assert_eq!(live.notice.as_deref(), Some(notice.as_str()));

        // A session attached: the agent is told to move to *this session's* chat
        // directory, and the rail re-files the conversation under 对话会话
        // because the directory is the app's own scratch tree.
        live.launchpad = false;
        live.state.session_id = "s-1".into();
        live.state.cwd = Some("/work/alpha".into());
        live.select_chat_workspace().await;
        assert!(
            session_chat.is_dir(),
            "the chat's directory is created for it"
        );
        let commands = seen.lock().unwrap().clone();
        assert!(
            commands.iter().any(|command| command == "set_cwd"),
            "a chat moves an attached session: {commands:?}"
        );
        assert_eq!(
            crate::rpc::grpc_client::tests::last_request_of(&mock, "set_cwd").cwd,
            session_chat.to_string_lossy(),
            "the session moves to its own chat directory under the app's tree"
        );
        assert_eq!(live.notice.as_deref(), Some(notice.as_str()));

        // Already a chat: the row is where the reader stands, so nothing moves
        // (the same no-op `esc` gives).
        let before = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|command| *command == "set_cwd")
            .count();
        live.notice = None;
        live.state.session_id = "s-1".into();
        live.state.cwd = Some(session_chat.to_string_lossy().to_string());
        live.select_chat_workspace().await;
        let after = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|command| *command == "set_cwd")
            .count();
        assert_eq!(after, before, "already a chat: no second move");
        assert_eq!(live.notice, None);
    }

    /// A launchpad names the *product* defaults, not the conversation `/new`
    /// left behind: the plain chat, the agent's default model and the level that
    /// model starts at, and 完全访问. The state the client came from says
    /// otherwise on every one of those ("future/old", high thinking, manual
    /// approval in a project directory) — which is exactly what must not leak
    /// into a fresh screen.
    #[tokio::test]
    async fn a_fresh_launchpad_names_the_product_defaults() {
        let state = r#"{"sessionId":"s-1","cwd":"/work/alpha","model":"future/old","thinkingLevel":"high","sandboxTier":"manual"}"#;
        let models = r#"{"models":[
            {"id":"old","label":"old","provider":"future","thinkingLevel":"default",
             "thinkingLevels":["default","high"],"defaultThinkingLevel":"high"},
            {"id":"gpt-9","label":"gpt-9","provider":"future","isDefault":true,
             "thinkingLevel":"default","thinkingLevels":["default","high"],
             "defaultThinkingLevel":"default"}
        ]}"#;
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("get_state", state),
            ("list_models", models),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        // A first launch with nothing to inherit: the state the client came
        // from is not the next conversation's default (`/new` inherits it, a
        // fresh start does not).
        live.state.session_id.clear();
        live.state.cwd = None;
        live.to_launchpad();

        let frame = crate::workbench::layout::Frame::new(
            120,
            38,
            36,
            crate::workbench::preferences::SidebarPosition::Right,
        );
        let data = live.data(&frame);
        assert_eq!(data.session.cwd, chat_workspace());
        assert!(data.session.chat, "the chip names the plain chat");
        assert_eq!(data.model, "future/gpt-9", "the agent's default model");
        assert_eq!(data.effort, "default", "the level that model starts at");
        assert_eq!(data.mode, "off", "完全访问 / full access");

        // What the reader picks still wins — the defaults are a starting point,
        // not an override.
        live.set_pending_model("future/old");
        live.set_pending_effort("high");
        live.set_approval("manual").await;
        let data = live.data(&frame);
        assert_eq!(data.model, "future/old");
        assert_eq!(data.effort, "high");
        assert_eq!(data.mode, "manual");
    }

    /// The rail, the `ctrl+r` list and the 工作区 tab name a workspace by its
    /// own name; the path is what the *group* is filed under, not what the row
    /// has room for. On the launchpad the 工作区 chip stages the directory the
    /// next session is created in.
    #[tokio::test]
    async fn the_rail_names_workspaces_and_the_chip_stages_the_next_one() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        // Two workspaces, one of them named twice (a checkout and its trailing
        // slash), which is one choice and two rows.
        live.sessions.replace(vec![
            row("s-1", Some("first"), "/work/alpha"),
            row("s-2", Some("second"), "/work/alpha/"),
            row("s-3", Some("third"), "/other/beta"),
        ]);
        live.state.session_id = "s-1".into();
        live.state.cwd = Some("/work/alpha".into());
        live.to_launchpad();

        let frame = crate::workbench::layout::Frame::new(
            120,
            38,
            36,
            crate::workbench::preferences::SidebarPosition::Right,
        );
        let data = live.data(&frame);
        let names: Vec<&str> = data
            .sessions
            .iter()
            .map(|session| session.workspace.as_str())
            .collect();
        assert_eq!(names.len(), 3);
        for name in &names {
            assert!(!name.contains('/'), "a row names a workspace: {name:?}");
        }
        assert!(names.contains(&"alpha"), "{names:?}");
        assert!(names.contains(&"beta"), "{names:?}");

        // `/new` stages the workspace the conversation was just in — a new
        // conversation starts where the last one ran, not in a fixed scratch
        // directory.
        assert_eq!(data.session.cwd, "/work/alpha");
        assert!(!data.session.chat, "a workspace is not a chat");
        assert_eq!(
            live.workspace_choices(),
            vec![
                chat_workspace(),
                "/work/alpha".to_string(),
                "/other/beta".to_string()
            ]
        );

        // …cycles to the next *distinct* one…
        live.cycle_pending_workspace();
        assert_eq!(live.pending_cwd.as_deref(), Some("/other/beta"));
        let data = live.data(&frame);
        assert_eq!(data.session.cwd, "/other/beta");
        assert!(!data.session.chat, "a workspace is not a chat");

        // …and the `@` list, the files panel and the header's branch follow it.
        assert_eq!(live.local_cwd, "/other/beta");

        // …and on to the chat row, which stages no workspace at all.
        live.cycle_pending_workspace();
        assert_eq!(live.pending_cwd.as_deref(), Some(chat_workspace().as_str()));
        let data = live.data(&frame);
        assert!(data.session.chat, "the chip names it as 对话会话");
        // Standing on the chat, the next press leaves it again rather than
        // staging the same choice twice.
        live.cycle_pending_workspace();
        assert_eq!(live.pending_cwd.as_deref(), Some("/work/alpha"));
    }

    /// With a single choice there is nothing to cycle: the chip says so instead
    /// of silently keeping the only one.
    #[tokio::test]
    async fn cycling_one_workspace_reports_that_there_is_nothing_else() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        // No conversation anywhere: the launchpad's own chat is the only choice
        // left, and cycling it would be a chip that does nothing.
        live.sessions.replace(Vec::new());
        live.to_launchpad();
        live.cycle_pending_workspace();
        assert_eq!(live.pending_cwd, None);
        assert_eq!(
            live.notice.as_deref(),
            Some(t(live.lang, Key::WorkspaceNone))
        );
    }

    /// `cd project && future tui` opens that project: the driver stages the
    /// directory the process was started from, so the first message does not
    /// fall back to the app's scratch tree.
    #[tokio::test]
    async fn the_launchpad_stages_the_directory_the_tui_was_started_from() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        assert_eq!(live.pending_cwd, None, "attach stages nothing by itself");

        live.stage_launch_workspace();
        let started_from = std::env::current_dir().unwrap();
        assert_eq!(
            live.pending_cwd.as_deref(),
            Some(started_from.to_string_lossy().as_ref()),
            "the first conversation runs where the TUI was opened"
        );
        // A launch from one of the app's own scratch directories stays a chat
        // (see `launch_workspace`), and a session already attached is not
        // re-staged.
        live.launchpad = false;
        live.state.session_id = "s-1".into();
        live.pending_cwd = None;
        live.stage_launch_workspace();
        assert_eq!(live.pending_cwd, None, "an attached session keeps its cwd");
    }

    /// `/new` starts the next conversation where the one on screen ran: a
    /// project is inherited, a chat is not (the next message mints a fresh chat
    /// directory of its own).
    #[tokio::test]
    async fn a_new_conversation_keeps_the_workspace_it_left() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");

        live.state.session_id = "s-1".into();
        live.state.cwd = Some("/work/alpha".into());
        live.to_launchpad();
        assert_eq!(live.pending_cwd.as_deref(), Some("/work/alpha"));

        // A chat left behind stages no workspace: a new chat gets its own
        // directory rather than the previous conversation's.
        live.launchpad = false;
        live.state.session_id = "s-2".into();
        live.state.cwd = Some(chat_workspace());
        live.to_launchpad();
        assert_eq!(live.pending_cwd, None);
    }

    /// The rail's 活跃 stretch is the agent's own flag, not a run row of this
    /// client's: a conversation another client started reports `isStreaming` on
    /// its summary, and an agent that never sends the field (or sends false) is
    /// not "running".
    #[tokio::test]
    async fn a_streaming_summary_reaches_the_rail() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        let mut busy = row("s-1", Some("writing"), "/work/alpha");
        busy.is_streaming = Some(true);
        let mut quiet = row("s-2", Some("quiet"), "/work/alpha");
        quiet.is_streaming = Some(false);
        live.sessions.replace(vec![
            busy,
            quiet,
            row("s-3", Some("older agent"), "/work/alpha"),
        ]);
        assert!(live.sessions_streaming(), "one conversation is writing");

        let frame = crate::workbench::layout::Frame::new(
            120,
            38,
            36,
            crate::workbench::preferences::SidebarPosition::Right,
        );
        let data = live.data(&frame);
        let streaming: Vec<bool> = data.sessions.iter().map(|row| row.streaming).collect();
        assert_eq!(streaming.iter().filter(|on| **on).count(), 1);
        assert!(
            data.sessions
                .iter()
                .any(|row| row.streaming && row.title == "writing"),
            "the flag follows the conversation it was reported for"
        );

        // An agent that sends no flag at all is idle, not streaming.
        live.sessions
            .replace(vec![row("s-4", Some("legacy"), "/work/alpha")]);
        assert!(!live.sessions_streaming(), "no flag is not a run");
        let data = live.data(&frame);
        assert!(!data.sessions[0].streaming);
    }

    /// `最近` carries two stretches on two clocks. 活跃 (a run in flight) is
    /// ordered by when each conversation started — the one stamp a streaming run
    /// does not rewrite, so a running row keeps its place instead of trading
    /// with its neighbour every second. 更早 is ordered by when it last spoke,
    /// the same fact the row's age shows, because that stretch is a list a
    /// reader scans for "what did I just touch".
    #[tokio::test]
    async fn the_recent_panel_orders_active_rows_by_start_and_the_rest_by_activity() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as i64)
            .unwrap_or(0);
        let minute = 60_000;
        // Two running conversations whose creation order is the reverse of their
        // activity order: `s-speaking` moved a second ago, yet it started first.
        let mut speaking = row("s-speaking", Some("spoke a second ago"), "/work/alpha");
        speaking.created_at_ms = now - 300 * minute;
        speaking.updated_at_ms = now - minute;
        speaking.is_streaming = Some(true);
        let mut started = row("s-started", Some("started last"), "/work/alpha");
        started.created_at_ms = now - 30 * minute;
        started.updated_at_ms = now - 20 * minute;
        started.is_streaming = Some(true);
        // Two quiet ones, in activity order — and in the opposite creation order,
        // so the stretch cannot be passing this by accident.
        let mut newer = row("s-newer", Some("spoke an hour ago"), "/work/alpha");
        newer.created_at_ms = now - 900 * minute;
        newer.updated_at_ms = now - 60 * minute;
        let mut older = row("s-older", Some("spoke yesterday"), "/work/alpha");
        older.created_at_ms = now - 10 * minute;
        older.updated_at_ms = now - 24 * 60 * minute;
        // The list arrives in neither order: the panel does the sorting.
        live.sessions.replace(vec![older, speaking, newer, started]);

        let frame = crate::workbench::layout::Frame::new(
            120,
            38,
            36,
            crate::workbench::preferences::SidebarPosition::Right,
        );
        let data = live.data(&frame);
        let painted: Vec<&str> =
            crate::workbench::panels::rows(crate::workbench::state::PanelId::Recent, &data)
                .iter()
                .filter_map(|row| row.session())
                .map(|index| data.sessions[index].id.as_str())
                .collect();
        assert_eq!(
            painted,
            ["s-started", "s-speaking", "s-newer", "s-older"],
            "active rows newest-started first; the quiet ones by last activity"
        );

        // An agent that predates `createdAtMs` sends 0 for the running rows too:
        // the activity stamp stands in, so the stretch is ordered rather than a
        // page of ties.
        let mut old_a = row("s-old-a", Some("old agent"), "/work/alpha");
        old_a.is_streaming = Some(true);
        old_a.updated_at_ms = now - 10 * minute;
        let mut old_b = row("s-old-b", Some("old agent"), "/work/alpha");
        old_b.is_streaming = Some(true);
        old_b.updated_at_ms = now - minute;
        live.sessions.replace(vec![old_a, old_b]);
        let data = live.data(&frame);
        let painted: Vec<&str> =
            crate::workbench::panels::rows(crate::workbench::state::PanelId::Recent, &data)
                .iter()
                .filter_map(|row| row.session())
                .map(|index| data.sessions[index].id.as_str())
                .collect();
        assert_eq!(
            painted,
            ["s-old-b", "s-old-a"],
            "no stamp: activity orders it"
        );
    }

    #[test]
    fn a_session_row_is_named_by_its_name_then_its_first_prompt() {
        assert_eq!(
            display_name(&row("s1", Some("parser fix"), "/tmp")),
            "parser fix"
        );
        let mut first = row("s2", None, "/tmp");
        first.first_message = Some("look at this test".into());
        assert_eq!(display_name(&first), "look at this test");
        // Neither: the id's head, never a blank cell.
        assert_eq!(
            display_name(&row("0123456789abcdef", None, "/tmp")),
            "01234567"
        );
    }

    #[test]
    fn a_workspace_is_shown_home_relative() {
        let home = crate::home::home_dir_or_default().display().to_string();
        if home.is_empty() {
            return;
        }
        assert_eq!(shorten(&home), "~");
        assert_eq!(shorten(&format!("{home}/future-os")), "~/future-os");
        // Outside home it is left alone.
        assert_eq!(shorten("/opt/build"), "/opt/build");
    }

    #[test]
    fn ages_read_like_the_session_list() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        assert_eq!(age_at(now, now), "now");
        assert_eq!(age_at(now, now - 5 * 60_000), "5m");
        assert_eq!(age_at(now, now - 3 * 3_600_000), "3h");
        assert_eq!(age_at(now, now - 2 * 86_400_000), "2d");
        assert_eq!(age_at(now, now - 10 * 86_400_000), "1w");
        // A clock that disagrees with a stored timestamp reads as "now" rather
        // than a negative age.
        assert_eq!(age_at(now, now + 60_000), "now");
    }

    #[test]
    fn a_cache_rate_never_divides_by_zero() {
        let mut state = RpcSessionState::default();
        assert_eq!(cache_percent(&state), 0);
        state.usage.cache_read_tokens = 96;
        state.usage.input_tokens = 4;
        assert_eq!(cache_percent(&state), 96);
    }

    /// The 工作区 tab's filing. A conversation is filed under the directory it
    /// ran in; two checkouts that share a name are two groups (the path is the
    /// identity); a scratch directory is a plain chat; a pinned workspace leads
    /// even when it is not the freshest.
    #[test]
    fn conversations_are_filed_by_the_directory_they_ran_in() {
        // The scratch tree belongs to *this* home, so the fixture builds it the
        // way the app does rather than pasting another machine's path.
        let scratch = crate::home::future_state_dir()
            .join("workspaces")
            .join("chat")
            .join("abc")
            .display()
            .to_string();
        let sessions = vec![
            row("a", Some("newest in main"), "/work/future-os"),
            row("b", Some("older in main"), "/work/future-os/"),
            row("c", Some("a worktree"), "/work/future-os/.worktrees/rail"),
            row("d", Some("a sibling checkout"), "/other/future-os"),
            row("e", Some("scratch"), &scratch),
        ];
        let rows: Vec<SessionRow> = sessions
            .iter()
            .enumerate()
            .map(|(index, session)| SessionRow {
                id: session.id.clone(),
                title: display_name(session),
                workspace: ChatRoots::detect().workspace_label(&session.cwd),
                age: format!("{index}h"),
                ..SessionRow::default()
            })
            .collect();
        let pinned = vec!["/other/future-os/".to_string()];
        let (groups, chats) = file_sessions(&sessions, &rows, &pinned, &ChatRoots::detect());

        // The pinned checkout leads although it is not the freshest.
        assert_eq!(groups[0].path, "/other/future-os");
        assert!(groups[0].pinned);
        assert_eq!(groups[0].session_ids, vec!["d".to_string()]);

        // A trailing slash is not a different workspace: `a` and `b` are one
        // group, newest first, and the group's age is that newest one.
        let main = groups
            .iter()
            .find(|group| group.path == "/work/future-os")
            .expect("the main checkout");
        assert_eq!(main.session_ids, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(main.age, "0h");
        assert!(!main.pinned);
        assert_eq!(main.name, "future-os");

        // A worktree is its own group, and so is a second checkout with the
        // same name.
        assert!(groups
            .iter()
            .any(|group| group.path == "/work/future-os/.worktrees/rail"));
        assert_eq!(
            groups
                .iter()
                .filter(|group| group.name == "future-os")
                .count(),
            2,
            "two checkouts share a name and stay apart: {groups:?}"
        );

        // The scratch directory is a plain chat, not a workspace.
        assert_eq!(chats, vec!["e".to_string()]);
        assert!(!groups
            .iter()
            .any(|group| group.session_ids.contains(&"e".into())));
    }

    /// Two spellings of one directory are one workspace. `path_key` is what
    /// decides that now (the group lookup is a hash, not a `same_path` scan per
    /// row), so the normalisation it applies has to match the comparison it
    /// replaced.
    #[test]
    fn two_spellings_of_one_directory_are_one_workspace() {
        let sessions = vec![
            row("a", Some("newest"), "C:\\work\\os"),
            row("b", Some("older"), "C:/work/os/"),
        ];
        let rows: Vec<SessionRow> = sessions
            .iter()
            .map(|session| SessionRow {
                id: session.id.clone(),
                title: display_name(session),
                workspace: ChatRoots::detect().workspace_label(&session.cwd),
                age: "1h".to_string(),
                ..SessionRow::default()
            })
            .collect();
        let (groups, chats) = file_sessions(&sessions, &rows, &[], &ChatRoots::detect());
        assert!(chats.is_empty(), "neither is a scratch directory");
        assert_eq!(groups.len(), 1, "one directory, one group: {groups:?}");
        assert_eq!(
            groups[0].session_ids,
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(groups[0].path, "C:/work/os", "one spelling for the group");
        assert_eq!(groups[0].name, "os");
    }

    /// The hoisted roots answer what the per-row reader used to: the app's own
    /// two scratch directories are chats, a directory the reader chose is not.
    #[test]
    fn the_hoisted_roots_answer_which_directories_are_chats() {
        let roots = ChatRoots::detect();
        let state = crate::home::future_state_dir();
        let default_workspace = state.join("agent").join("workspace").display().to_string();
        let scratch = state
            .join("workspaces")
            .join("chat")
            .join("thread-1")
            .display()
            .to_string();
        assert!(roots.is_chat(""), "no directory yet is no workspace");
        assert!(roots.is_chat(&default_workspace));
        assert!(
            roots.is_chat(&format!("{default_workspace}/")),
            "a trailing separator is the same directory"
        );
        assert!(roots.is_chat(&scratch));
        assert!(!roots.is_chat("/work/future-os"));
        assert!(
            !roots.is_chat(&state.display().to_string()),
            "the state directory itself is not a chat"
        );
        // The row's label is blank exactly for those.
        assert_eq!(roots.workspace_label(&scratch), "");
        assert_eq!(roots.workspace_label("/work/future-os"), "future-os");
        // And the free reader, which other callers still use, agrees.
        assert!(is_chat_directory(&scratch));
        assert!(!is_chat_directory("/work/future-os"));
    }

    /// A frame asks for the rail's rows and filing every time it is composed,
    /// and building them is the one part of a frame that grows with history. The
    /// view is therefore rebuilt only when one of its inputs moves — a change
    /// that made it rebuild per frame would cost milliseconds per keystroke at a
    /// few hundred conversations without failing anything else.
    #[tokio::test]
    async fn the_rail_view_is_rebuilt_only_when_one_of_its_inputs_moves() {
        let body = format!(
            "{{\"sessions\":[{}]}}",
            (0..8)
                .map(|i| format!(
                    "{{\"id\":\"s{i}\",\"cwd\":\"/work/p{i}\",\"updatedAtMs\":{},\"model\":\"m\",\"sessionName\":\"c{i}\"}}",
                    1_700_000_000_000i64 + i as i64
                ))
                .collect::<Vec<_>>()
                .join(",")
        );
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("get_state", r#"{"sessionId":"s0"}"#),
            ("list_sessions", &body),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        assert_eq!(
            live.sessions.len(),
            8,
            "the fixture's list arrives at attach"
        );
        let frame = frame();
        live.data(&frame);

        // Steady state: composing a frame at the same inputs is a clone. At most
        // one rebuild is allowed, because the key carries the minute and a run
        // that crosses one legitimately re-reads its ages.
        let before = live.session_view_builds();
        for _ in 0..20 {
            live.data(&frame);
        }
        let rebuilt = live.session_view_builds() - before;
        assert!(
            rebuilt <= 1,
            "twenty frames that moved nothing rebuilt the rail's view {rebuilt} times"
        );

        // A pin re-files the 工作区 tab.
        let before = live.session_view_builds();
        live.set_workspace_pinned("/work/p3", true);
        live.data(&frame);
        assert!(
            live.session_view_builds() > before,
            "a pinned workspace must re-file the tab"
        );

        // Leaving for the launchpad moves `current` off every row.
        let before = live.session_view_builds();
        live.to_launchpad();
        live.data(&frame);
        assert!(
            live.session_view_builds() > before,
            "the launchpad claims no current conversation"
        );
        let data = live.data(&frame);
        assert!(
            data.sessions.iter().all(|row| !row.current),
            "no row is current on the launchpad"
        );
    }

    /// A workspace's process-bound facts are read *beside* the frame, not in it.
    ///
    /// Changing workspace used to spawn `git ls-files` (24 ms in this repository)
    /// and `git rev-parse` on the loop's own thread, so the frame that opened a
    /// conversation in another workspace waited for two processes — the stutter a
    /// reader felt when stepping through the rail's 工作区 tab. [`refresh_local`]
    /// starts the reads and returns; the clock that drives the loop
    /// ([`Live::poll_local_reads`]) installs the answers.
    #[tokio::test]
    async fn a_workspace_change_reads_off_the_frame() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("get_state", r#"{"sessionId":"s1"}"#),
            ("list_sessions", r#"{"sessions":[]}"#),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");

        // The checkout this test runs in: a real workspace with real files.
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("the crate sits inside the repository")
            .display()
            .to_string();
        live.state.cwd = Some(repo.clone());
        live.refresh_local();

        // Nothing was read on this thread: the call returned with the previous
        // workspace's facts gone rather than with the new one's read.
        assert_eq!(live.local_cwd, repo);
        assert!(
            live.files.is_empty(),
            "the listing must not be read on the frame's thread"
        );
        assert_eq!(
            live.branch.get(&live.local_cwd),
            "",
            "and neither must the branch"
        );

        // The answers arrive through the clock the driver keeps, one at a time
        // (`true` is the driver's "a frame is owed").
        let mut waited = std::time::Duration::ZERO;
        let mut landed = 0;
        while waited < std::time::Duration::from_secs(10) {
            if live.poll_local_reads() {
                landed += 1;
            }
            if !live.files.is_empty() && !live.branch.get(&live.local_cwd).is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            waited += std::time::Duration::from_millis(5);
        }
        assert!(
            live.files.iter().any(|file| file == "Cargo.toml"),
            "the listing is this repository's: {} entries after {landed} answers",
            live.files.len()
        );
        assert!(
            !live.branch.get(&live.local_cwd).is_empty(),
            "and the header can name the branch"
        );
        // A workspace that has not moved asks for nothing more.
        assert!(!live.poll_local_reads(), "nothing left to install");
    }

    /// A directory that is not there — or none at all — is not a read: the frame
    /// that shows an empty workspace spawns nothing.
    #[tokio::test]
    async fn an_emptied_workspace_starts_no_read() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("get_state", r#"{"sessionId":"s1"}"#),
            ("list_sessions", r#"{"sessions":[]}"#),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        live.state.cwd = Some("/definitely/not/a/workspace/here".into());
        live.refresh_local();
        assert!(live.files.is_empty());
        let reads = live
            .local_reads
            .as_ref()
            .expect("a workspace was asked for");
        assert!(
            reads.branch.is_none() && reads.files.is_none(),
            "a directory that is not there starts no read"
        );
        assert!(!live.poll_local_reads(), "there was nothing to read");
        assert!(live.branch.get(&live.local_cwd).is_empty());
    }

    /// A plain chat has no workspace to name. Its directory is the app's own
    /// scratch tree, and printing that directory's name is how
    /// `thread-20261026-121352-6627dd` came to read like a project in the rail.
    #[test]
    fn a_plain_chat_names_no_workspace() {
        let scratch = crate::home::future_state_dir()
            .join("workspaces")
            .join("chat")
            .join("thread-20261026-121352-6627dd")
            .display()
            .to_string();
        assert_eq!(ChatRoots::detect().workspace_label(&scratch), "");
        // Every other directory is named by its own last segment.
        assert_eq!(
            ChatRoots::detect().workspace_label("/work/future-os"),
            "future-os"
        );
        assert_eq!(
            ChatRoots::detect().workspace_label("/work/future-os/"),
            "future-os"
        );
    }

    /// The preview loader: pictures become pixels, documents become lines, and
    /// nothing it cannot read panics.
    #[test]
    fn a_preview_loads_pictures_documents_and_refusals() {
        use crate::workbench::files::{preview_kind, PreviewKind};
        let dir = tempfile::tempdir().expect("tempdir");
        let png = dir.path().join("shot.png");
        std::fs::write(&png, png_bytes()).unwrap();
        let path = png.to_string_lossy().to_string();
        let view = read_preview(&path, preview_kind(&png));
        match view {
            PreviewView::Image { pixels, .. } => {
                assert_eq!((pixels.width, pixels.height), (2, 2));
                assert_eq!(pixels.rgba.len(), 16);
            }
            other => panic!("a png must decode into pixels: {other:?}"),
        }

        // Text keeps its lines, and a long file says it is long.
        let text = dir.path().join("notes.txt");
        std::fs::write(&text, "one\ntwo\n").unwrap();
        let view = read_preview(&text.to_string_lossy(), preview_kind(&text));
        match view {
            PreviewView::Text {
                lines, truncated, ..
            } => {
                assert_eq!(
                    lines.iter().map(|line| line.text()).collect::<Vec<_>>(),
                    vec!["one", "two"]
                );
                assert!(!truncated);
            }
            other => panic!("text must stay text: {other:?}"),
        }
        // A file whose language is known arrives classified: the box paints
        // the runs, it does not scan the text again.
        let source = dir.path().join("lib.rs");
        std::fs::write(&source, "pub fn f() -> u32 { 1 } // done\n").unwrap();
        match read_preview(&source.to_string_lossy(), preview_kind(&source)) {
            PreviewView::Text { lines, .. } => {
                assert!(!lines[0].spans().is_empty(), "a .rs line is classified");
                assert_eq!(
                    lines[0].spans().last().map(|span| span.token),
                    Some(crate::workbench::syntax::Token::Comment)
                );
            }
            other => panic!("a .rs file must stay text: {other:?}"),
        }
        let long = dir.path().join("long.log");
        std::fs::write(
            &long,
            "x".repeat(crate::workbench::files::MAX_TEXT_PREVIEW_BYTES as usize + 16),
        )
        .unwrap();
        let view = read_preview(&long.to_string_lossy(), PreviewKind::Text);
        match view {
            PreviewView::Text { truncated, .. } => assert!(truncated),
            other => panic!("{other:?}"),
        }

        // Markdown is kept as its source, so the box can render it.
        let md = dir.path().join("README.md");
        std::fs::write(&md, "# title\n").unwrap();
        assert!(matches!(
            read_preview(&md.to_string_lossy(), preview_kind(&md)),
            PreviewView::Markdown { .. }
        ));

        // A file that is not there, and bytes that are not text, are both an
        // answer rather than a panic.
        assert!(matches!(
            read_preview("/definitely/not/here.png", PreviewKind::Image),
            PreviewView::Unavailable { .. }
        ));
        let binary = dir.path().join("blob.bin");
        std::fs::write(&binary, [0u8, 1, 2, 3]).unwrap();
        assert!(matches!(
            read_preview(&binary.to_string_lossy(), PreviewKind::Opaque),
            PreviewView::Unavailable { .. }
        ));
    }

    /// Cutting a long document never splits a character: a preview that ends in
    /// half a `中` would render as a replacement glyph.
    #[test]
    fn a_bounded_text_cut_keeps_whole_characters() {
        let text = "中".repeat(10);
        let (cut, truncated) = bound(&text, 7);
        assert!(truncated);
        assert_eq!(cut.len(), 6, "two three-byte characters fit in seven bytes");
        assert_eq!(cut.chars().count(), 2);
        let (whole, truncated) = bound("short", 64);
        assert_eq!((whole.as_str(), truncated), ("short", false));
    }

    /// A real 2×2 PNG, encoded here: it has to decode, not just sniff.
    fn png_bytes() -> Vec<u8> {
        let mut image = image::RgbaImage::new(2, 2);
        image.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        image.put_pixel(1, 1, image::Rgba([0, 0, 255, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut out, image::ImageFormat::Png)
            .expect("encode");
        out.into_inner()
    }

    #[test]
    fn a_workspace_is_named_by_its_last_segment() {
        assert_eq!(workspace_name("/home/dev/future-os"), "future-os");
        assert_eq!(workspace_name("/home/dev/future-os/"), "future-os");
        assert_eq!(workspace_name("D:\\work\\future-os"), "future-os");
        // Nothing to take: the path itself is the honest name.
        assert_eq!(workspace_name("/"), "/");
        for (left, right) in [("/a/b", "/a/b/"), ("D:\\a\\b", "D:/a/b"), ("/a/b/", "/a/b")] {
            assert!(same_path(left, right), "{left:?} vs {right:?}");
        }
        assert!(!same_path("/a/b", "/a/bc"));
    }

    /// A `usage` event is the only thing that moves the footer's ctx gauge
    /// mid-turn, so folding it wrong means the whole footer is wrong.
    #[test]
    fn a_usage_event_folds_into_the_gauges_it_feeds() {
        let mut state = RpcSessionState::default();
        let usage = serde_json::json!({
            "prompt_tokens": 900,
            "completion_tokens": 100,
            "cache_read_tokens": 800,
            "cache_write_tokens": 20
        });
        let (completion, ctx) = fold_usage(&mut state, Some(&usage)).expect("numbers fold");
        assert_eq!(completion, 100);
        // The next request is this prompt plus this completion.
        assert_eq!(ctx, 1000);
        assert_eq!(state.usage.input_tokens, 900);
        assert_eq!(state.usage.output_tokens, 100);
        assert_eq!(state.usage.cache_read_tokens, 800);
        assert_eq!(state.usage.cache_write_tokens, 20);
        // Cache hit rate follows the same accounting the crate already uses:
        // `cache_percent` divides the reads by the prompt *plus* the reads, so
        // it reads `inputTokens` as the uncached part (800 / 1700 = 47).
        assert_eq!(cache_percent(&state), 47);

        // A second step adds to the totals and replaces the context.
        let step = serde_json::json!({"prompt_tokens": 1100, "completion_tokens": 50});
        let (completion, ctx) = fold_usage(&mut state, Some(&step)).expect("numbers fold");
        assert_eq!((completion, ctx), (50, 1150));
        assert_eq!(state.usage.input_tokens, 2000);
        assert_eq!(state.usage.output_tokens, 150);

        // A payload with no numbers is not a step (and must not zero the gauge
        // or reset the cache counts to a division by zero).
        let before = state.usage.cache_read_tokens;
        assert_eq!(fold_usage(&mut state, Some(&serde_json::json!({}))), None);
        assert_eq!(fold_usage(&mut state, None), None);
        assert_eq!(state.usage.cache_read_tokens, before);
    }

    /// A rate over too short a window is noise, and a rate with no tokens is
    /// not a rate.
    #[test]
    fn a_rate_needs_a_meaningful_sample() {
        assert_eq!(rate_per_second(0, 5.0), None);
        assert_eq!(rate_per_second(100, 0.2), None);
        assert_eq!(rate_per_second(100, 2.0), Some(50.0));
    }

    fn model_info(id: &str, levels: Option<&[&str]>) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            label: id.into(),
            provider: String::new(),
            supports_images: false,
            thinking_level: "default".into(),
            thinking_levels: levels
                .map(|levels| levels.iter().map(|level| level.to_string()).collect()),
            default_thinking_level: Some("default".into()),
            context_window: 0,
            is_default: false,
        }
    }

    /// The launchpad's effort chip cycles the *selected model's* levels: a model
    /// that declares one level is cycled to `default` and back, never through a
    /// vocabulary it would only be clamped out of.
    #[test]
    fn the_launchpad_effort_ring_follows_the_selected_model() {
        let models = vec![
            model_info("future/k3", Some(&["default", "off", "low", "high"])),
            model_info("future/plain", Some(&["default"])),
        ];
        assert_eq!(
            level_ring(&models, Some("future/k3")),
            vec!["default", "off", "low", "high"]
        );
        // The session may name a model bare; the tail match still finds its row.
        assert_eq!(
            level_ring(&models, Some("k3")),
            vec!["default", "off", "low", "high"]
        );
        assert_eq!(level_ring(&models, Some("plain")), vec!["default"]);
        // An unknown model — the list has not loaded yet, or the agent predates
        // the field — falls back to `default` + the legacy six.
        let fallback = crate::workbench::state::fallback_thinking_levels();
        assert_eq!(level_ring(&models, Some("nope")), fallback);
        assert_eq!(level_ring(&models, None), fallback);
        // The first press from an unset level lands on the ring's first entry
        // (`default`); the next one advances to the first real choice.
        let ring = level_ring(&models, Some("k3"));
        assert_eq!(
            next_thinking_level(&ring, "").as_deref(),
            Some(DEFAULT_THINKING_LEVEL_NAME)
        );
        assert_eq!(
            next_thinking_level(&ring, DEFAULT_THINKING_LEVEL_NAME).as_deref(),
            Some("off")
        );
    }

    /// Two providers may offer one id, and a reference says which one: the
    /// lookup must not answer with the other provider's model, whose thinking
    /// ring (and therefore the staged effort) is a different set.
    #[test]
    fn a_reference_names_one_model_when_two_providers_share_its_id() {
        let mut westlake = model_info("deepseek-flash", Some(&["default", "off"]));
        westlake.provider = "westlakehpc".into();
        let mut future = model_info("deepseek-flash", Some(&["default", "low", "high"]));
        future.provider = "future".into();
        let models = vec![westlake, future];

        assert_eq!(
            find_model(&models, "future/deepseek-flash").map(|model| model.provider.as_str()),
            Some("future")
        );
        assert_eq!(
            level_ring(&models, Some("future/deepseek-flash")),
            vec!["default", "low", "high"]
        );
        // A bare id (or a tail) is all an older surface gives us: it keeps the
        // list's own order rather than failing.
        assert_eq!(
            find_model(&models, "deepseek-flash").map(|model| model.provider.as_str()),
            Some("westlakehpc")
        );
    }

    fn event(kind: &str, data: serde_json::Value) -> AgentEvent {
        AgentEvent {
            r#type: kind.into(),
            data,
            ..Default::default()
        }
    }

    fn frame() -> crate::workbench::layout::Frame {
        crate::workbench::layout::Frame::new(
            120,
            38,
            36,
            crate::workbench::preferences::SidebarPosition::Right,
        )
    }

    /// `· {n} 工具` counts the run's tool *calls*, and the agent announces one
    /// call twice (`phase: "input"` while its arguments stream, then
    /// `phase: "execution"` when it runs, see `prompt_helpers.rs`). Counting
    /// events made the row claim twice the calls the transcript showed — the
    /// reported `53 工具` on a run of 26 — so the count is a set of call ids.
    #[tokio::test]
    async fn a_tool_call_is_counted_once_though_the_agent_announces_it_twice() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        live.handle_event(event("agent_start", serde_json::json!({})));
        for phase in ["input", "execution"] {
            live.handle_event(event(
                "tool_start",
                serde_json::json!({"tool_id": "t1", "tool_name": "read", "phase": phase}),
            ));
            assert_eq!(live.data(&frame()).activity.tools, 1, "after {phase}");
        }
        // A second call is a second one, and the first one's end does not make
        // the run forget it: the row is a tally for the run, not a count of the
        // calls still open (that number would read `0` while the model thinks).
        live.handle_event(event(
            "tool_end",
            serde_json::json!({"tool_id": "t1", "text": "ok"}),
        ));
        live.handle_event(event(
            "tool_start",
            serde_json::json!({"tool_id": "t2", "tool_name": "shell", "phase": "input"}),
        ));
        live.handle_event(event(
            "tool_start",
            serde_json::json!({"tool_id": "t2", "tool_name": "shell", "phase": "execution"}),
        ));
        assert_eq!(live.data(&frame()).activity.tools, 2);
        // The next run starts its own tally.
        live.handle_event(event("agent_start", serde_json::json!({})));
        assert_eq!(live.data(&frame()).activity.tools, 0);
    }

    /// The activity row describes the conversation on screen. It used to carry
    /// the one the reader left: its kind (`思考`), its clock (`1071s`, still
    /// climbing) and its tool count sat on another conversation's footer because
    /// all three are the client's own run state and nothing dropped them at a
    /// switch.
    #[tokio::test]
    async fn a_switch_forgets_the_run_the_previous_session_was_narrating() {
        // Two conversations: the one this client is on runs, then the reader
        // clicks the other one in the rail.
        let answers =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::from([
                (
                    "switch_session".to_string(),
                    r#"{"success":true}"#.to_string(),
                ),
                (
                    "get_state".to_string(),
                    r#"{"sessionId":"s-a"}"#.to_string(),
                ),
                (
                    "get_session_entries".to_string(),
                    r#"{"entries":[]}"#.to_string(),
                ),
            ])));
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(
            crate::rpc::grpc_client::tests::api_mock_with_table(answers.clone()),
        )
        .await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        live.handle_event(event("agent_start", serde_json::json!({})));
        live.handle_event(event(
            "tool_start",
            serde_json::json!({"tool_id": "t1", "tool_name": "read", "phase": "execution"}),
        ));
        assert_eq!(live.activity_kind(), ActivityKind::Tool);
        assert_eq!(live.data(&frame()).activity.tools, 1);

        answers
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-b"}"#.into());
        live.apply(WbAction::SwitchSession("s-b".into())).await;

        let activity = live.data(&frame()).activity;
        assert_eq!(
            activity.kind,
            ActivityKind::Idle,
            "the row is the session's"
        );
        assert_eq!(activity.elapsed_secs, 0, "no clock from the session left");
        assert_eq!(activity.tools, 0, "no tool tally either");
        assert!(!live.activity_animating(), "and nothing left to animate");
    }

    /// The other half of a switch: a conversation whose run is still in flight
    /// (`get_state`'s `isStreaming` — started here, by the desktop, by a queued
    /// run) owns the row again, rather than showing `idle` until its next event.
    #[tokio::test]
    async fn a_session_already_writing_owns_the_row_after_a_switch() {
        // `get_state` answers for the session the client is on, so the map is
        // held here and rewritten at the switch: the conversation the reader
        // clicked is the one already writing (it may have been started by the
        // desktop, or by a queued run).
        let answers =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::from([
                (
                    "switch_session".to_string(),
                    r#"{"success":true}"#.to_string(),
                ),
                (
                    "get_state".to_string(),
                    r#"{"sessionId":"s-a"}"#.to_string(),
                ),
                (
                    "get_session_entries".to_string(),
                    r#"{"entries":[]}"#.to_string(),
                ),
            ])));
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(
            crate::rpc::grpc_client::tests::api_mock_with_table(answers.clone()),
        )
        .await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        assert_eq!(live.activity_kind(), ActivityKind::Idle);

        answers.lock().unwrap().insert(
            "get_state".into(),
            r#"{"sessionId":"s-b","isStreaming":true}"#.into(),
        );
        live.apply(WbAction::SwitchSession("s-b".into())).await;

        assert_eq!(live.activity_kind(), ActivityKind::Thinking);
        assert!(live.activity_animating(), "a run in flight spins the row");
        // The clock is this client's: it began watching at the switch, and the
        // calls made before it are the transcript's to show.
        let activity = live.data(&frame()).activity;
        assert_eq!(activity.elapsed_secs, 0);
        assert_eq!(activity.tools, 0);
    }

    // ─── the session cache ────────────────────────────────────────────

    /// One exchange of display history, shaped like the agent's page rows.
    fn page_of(questions: &[&str]) -> String {
        let rows: Vec<serde_json::Value> = questions
            .iter()
            .enumerate()
            .map(|(index, question)| {
                serde_json::json!({
                    "id": format!("e{index}"),
                    "role": "user",
                    "blocks": [{"kind": "text", "text": question}],
                })
            })
            .collect();
        serde_json::json!({"entries": rows}).to_string()
    }

    /// How many transcript pages the mock agent has been asked for.
    fn pages_fetched(seen: &std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> usize {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|kind| kind.as_str() == "get_session_entries")
            .count()
    }

    fn rows_on_screen(live: &mut Live) -> Vec<String> {
        live.data(&frame()).messages.to_vec()
    }

    /// A click on the row the transcript already shows reads nothing at all.
    ///
    /// The rows on screen are that conversation's, and the reader may have
    /// scrolled and paged older turns into them: re-reading would rebuild the
    /// transcript from the tail page and throw both away.
    #[tokio::test]
    async fn clicking_the_conversation_on_screen_reads_nothing() {
        let answers =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::from([
                (
                    "switch_session".to_string(),
                    r#"{"success":true}"#.to_string(),
                ),
                (
                    "get_state".to_string(),
                    r#"{"sessionId":"s-a"}"#.to_string(),
                ),
                ("get_session_entries".to_string(), page_of(&["q1"])),
            ])));
        let mock = crate::rpc::grpc_client::tests::api_mock_with_table(answers);
        let seen = mock.seen.clone();
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s-a"), false, None)
            .await
            .expect("attach to the test agent");
        let before = seen.lock().unwrap().len();
        let renders = live.chat.full_renders();

        live.apply(WbAction::SwitchSession("s-a".into())).await;

        assert_eq!(
            seen.lock().unwrap().len(),
            before,
            "the click reaches the agent with nothing"
        );
        assert_eq!(live.chat.full_renders(), renders, "and rebuilds nothing");
    }

    /// Coming back to a conversation the reader has already read is a repaint,
    /// not a reload: the rows are still here, so a switch to them paints before
    /// it asks the agent anything, and the reconcile that follows finds the same
    /// page and keeps them.
    ///
    /// This is the `shift+↑/↓` case the rail is built around — a bounce to the
    /// neighbour and back used to pay two round trips and a full re-render for
    /// the conversation it had just left.
    #[tokio::test]
    async fn a_cached_conversation_comes_back_without_reloading_its_rows() {
        let answers =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::from([
                (
                    "switch_session".to_string(),
                    r#"{"success":true}"#.to_string(),
                ),
                (
                    "get_state".to_string(),
                    r#"{"sessionId":"s-a"}"#.to_string(),
                ),
                ("get_session_entries".to_string(), page_of(&["q1"])),
            ])));
        let mock = crate::rpc::grpc_client::tests::api_mock_with_table(answers.clone());
        let seen = mock.seen.clone();
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s-a"), false, None)
            .await
            .expect("attach to the test agent");
        assert_eq!(pages_fetched(&seen), 1, "attach loads s-a's page");
        let first = rows_on_screen(&mut live);
        assert!(first.iter().any(|row| row.contains("q1")));
        // `data()` has now rendered at the frame's width, so a rebuild is
        // countable from here on.
        let renders = live.chat.full_renders();

        // To the neighbour, then straight back.
        answers
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-b"}"#.into());
        live.apply(WbAction::SwitchSession("s-b".into())).await;
        assert_eq!(pages_fetched(&seen), 2, "s-b had to be fetched");
        assert!(live.is_cached("s-a"), "the conversation just left is kept");

        answers
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-a"}"#.into());
        assert!(live.begin_switch("s-a"), "the cache has s-a's rows");
        assert_eq!(
            pages_fetched(&seen),
            2,
            "the switch paints before it asks the agent for anything"
        );
        assert_eq!(rows_on_screen(&mut live), first, "the rows came back");
        assert_eq!(
            live.chat.full_renders(),
            renders,
            "and they were not re-rendered"
        );

        // The reconcile still runs: it is what notices the conversation moving
        // on while the reader was away.
        live.finish_switch("s-a").await;
        assert_eq!(pages_fetched(&seen), 3);
        assert_eq!(
            live.chat.full_renders(),
            renders,
            "the same page keeps the rows it was built from"
        );
    }

    /// The other half of the cache: a conversation that gained turns while the
    /// reader was elsewhere is rebuilt from the agent's page rather than having
    /// that page *appended* to the rows the cache held — which would show the
    /// last exchange twice.
    #[tokio::test]
    async fn a_cached_conversation_that_moved_on_is_rebuilt_not_appended() {
        let answers =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::from([
                (
                    "switch_session".to_string(),
                    r#"{"success":true}"#.to_string(),
                ),
                (
                    "get_state".to_string(),
                    r#"{"sessionId":"s-a"}"#.to_string(),
                ),
                ("get_session_entries".to_string(), page_of(&["q1"])),
            ])));
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(
            crate::rpc::grpc_client::tests::api_mock_with_table(answers.clone()),
        )
        .await;
        let (mut live, _events) = Live::attach(&addr, Some("s-a"), false, None)
            .await
            .expect("attach to the test agent");

        answers
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-b"}"#.into());
        live.apply(WbAction::SwitchSession("s-b".into())).await;

        // s-a ran a turn while the reader was on s-b.
        answers
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-a"}"#.into());
        answers
            .lock()
            .unwrap()
            .insert("get_session_entries".into(), page_of(&["q1", "q2"]));
        assert!(live.begin_switch("s-a"), "the stale rows come back first");
        live.finish_switch("s-a").await;

        let screen = rows_on_screen(&mut live).join("\n");
        assert!(screen.contains("q2"), "the new turn is on screen");
        assert_eq!(
            screen.matches("q1").count(),
            1,
            "and the cached rows were replaced, not appended to: {screen}"
        );
    }

    /// `/new` is not a reason to make the way back cost a round trip: the
    /// conversation being left keeps its rows.
    #[tokio::test]
    async fn the_launchpad_keeps_the_conversation_it_left() {
        let answers =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::from([
                (
                    "switch_session".to_string(),
                    r#"{"success":true}"#.to_string(),
                ),
                (
                    "get_state".to_string(),
                    r#"{"sessionId":"s-a"}"#.to_string(),
                ),
                ("get_session_entries".to_string(), page_of(&["q1"])),
            ])));
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(
            crate::rpc::grpc_client::tests::api_mock_with_table(answers),
        )
        .await;
        let (mut live, _events) = Live::attach(&addr, Some("s-a"), false, None)
            .await
            .expect("attach to the test agent");

        live.to_launchpad();
        assert!(
            !rows_on_screen(&mut live).join("\n").contains("q1"),
            "the transcript is gone"
        );
        assert!(live.is_cached("s-a"), "but its rows are kept");
        assert!(live.begin_switch("s-a"), "and the way back is a repaint");
    }

    /// Meter, not a check: what a frame's `WorkbenchData` costs as the reader's
    /// conversation history grows.
    ///
    /// `Live::data` composes a frame's data, and the driver composes it once per
    /// agent-event batch and several times per keystroke — so whatever it costs
    /// is paid on the input path, not in the background. It used to be
    /// **quadratic**: `file_sessions` looked each row's summary up with a linear
    /// scan of the session list and each row's workspace up with a linear scan
    /// of the groups built so far, so 393 conversations cost 6.4 ms per call and
    /// 2000 cost 154 ms. Those scans are hashes now and the derived rows are
    /// cached against [`SessionViewKey`], which is what
    /// [`Self::session_view_builds`] measures.
    ///
    /// `cargo test -p future-tui --release --lib -- --ignored --nocapture session_rail_cost_meter`
    #[ignore = "meter, not a check"]
    #[tokio::test]
    async fn session_rail_cost_meter() {
        for n in [0usize, 20, 100, 393, 500, 2000] {
            let rows: Vec<String> = (0..n)
                .map(|i| {
                    format!(
                        r#"{{"id":"s{i}","cwd":"/Users/x/projects/project-{}","updatedAtMs":{},"createdAtMs":{},"model":"m","sessionName":"a conversation with a fairly long title {i}"}}"#,
                        i % 40,
                        1_700_000_000_000i64 + i as i64 * 1000,
                        1_700_000_000_000i64 + i as i64 * 1000
                    )
                })
                .collect();
            let body = format!(r#"{{"sessions":[{}]}}"#, rows.join(","));
            let (mock, _seen) =
                crate::rpc::grpc_client::tests::api_mock_answering(&[("list_sessions", &body)]);
            let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
            let (mut live, _events) = Live::attach(&addr, None, false, None).await.unwrap();
            live.refresh_sessions().await;
            let frame = frame();

            // The frame every event pays: unchanged state, one call.
            live.data(&frame);
            let start = std::time::Instant::now();
            for _ in 0..50 {
                live.data(&frame);
            }
            let steady = start.elapsed().as_secs_f64() * 1000.0 / 50.0;

            // The frame after a poll that moved the list: the view is rebuilt.
            let rebuild = live.session_view_builds();
            let start = std::time::Instant::now();
            live.sessions
                .replace(vec![crate::rpc::types::SessionSummary {
                    id: "s-moved".into(),
                    cwd: "/work/moved".into(),
                    updated_at_ms: 1_700_000_000_000,
                    created_at_ms: 0,
                    model: "m".into(),
                    session_name: None,
                    parent_session_id: None,
                    is_streaming: None,
                    first_message: None,
                }]);
            live.data(&frame);
            let cold = start.elapsed().as_secs_f64() * 1000.0;
            assert!(live.session_view_builds() > rebuild, "the list moved");

            // The rail's 5 s poll, which the driver awaits in the loop.
            let start = std::time::Instant::now();
            for _ in 0..20 {
                live.refresh_sessions().await;
            }
            let poll = start.elapsed().as_secs_f64() * 1000.0 / 20.0;

            println!(
                "sessions={n:>5}  data() = {steady:>7.3} ms  rebuild = {cold:>7.3} ms  poll = {poll:>6.3} ms"
            );
            live.client.disconnect();
        }
    }

    /// Meter, not a check: what a workspace change costs the frame, and what it
    /// costs the threads beside it.
    ///
    /// `cargo test -p future-tui --release --lib -- --ignored --nocapture workspace_change_cost_meter`
    #[ignore = "meter, not a check"]
    #[tokio::test]
    async fn workspace_change_cost_meter() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("get_state", r#"{"sessionId":"s1"}"#),
            ("list_sessions", r#"{"sessions":[]}"#),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None).await.unwrap();
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .display()
            .to_string();

        // The two reads on their own, in this repository.
        let start = std::time::Instant::now();
        let files = super::super::local::workspace_files(std::path::Path::new(&repo));
        let ls_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = std::time::Instant::now();
        let branch = super::super::git::branch(std::path::Path::new(&repo));
        let rev_ms = start.elapsed().as_secs_f64() * 1000.0;
        println!(
            "read on this thread: ls-files = {ls_ms:.2} ms ({} files), rev-parse = {rev_ms:.2} ms ({branch})",
            files.len()
        );

        // What the frame pays now: the change plus the first data() after it.
        for round in 0..3 {
            let target = if round % 2 == 0 { &repo } else { "/tmp" };
            live.state.cwd = Some(target.to_string());
            let start = std::time::Instant::now();
            live.refresh_local();
            let change_us = start.elapsed().as_secs_f64() * 1_000_000.0;
            let start = std::time::Instant::now();
            let _ = live.data(&frame());
            let frame_us = start.elapsed().as_secs_f64() * 1_000_000.0;
            // The answers, whenever their threads are done.
            let start = std::time::Instant::now();
            while start.elapsed() < std::time::Duration::from_secs(5)
                && !(live.poll_local_reads()
                    && !live.files.is_empty()
                    && !live.branch.get(&live.local_cwd).is_empty())
            {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            println!(
                "  workspace -> {target}: refresh_local = {change_us:.1} us, first frame = {frame_us:.1} us, background answered in {:.1} ms",
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
        live.client.disconnect();
    }

    /// Meter, not a check: the first frame after a conversation the reader has
    /// not opened yet (a switch cache miss) has to build and wrap the page it
    /// arrives with.
    ///
    /// `cargo test -p future-tui --release --lib -- --ignored --nocapture a_switched_conversations_first_frame_cost_meter`
    #[ignore = "meter, not a check"]
    #[tokio::test]
    async fn a_switched_conversations_first_frame_cost_meter() {
        let body: String = format!(
            r#"{{"sessions":[{}]}}"#,
            (0..393)
                .map(|i| format!(
                    r#"{{"id":"s{i}","cwd":"/work/p{}","updatedAtMs":{},"createdAtMs":{},"model":"m"}}"#,
                    i % 40,
                    1_700_000_000_000i64 + i as i64 * 1000,
                    1_700_000_000_000i64 + i as i64 * 1000
                ))
                .collect::<Vec<_>>()
                .join(",")
        );
        let (mock, _seen) =
            crate::rpc::grpc_client::tests::api_mock_answering(&[("list_sessions", &body)]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None).await.unwrap();
        live.refresh_sessions().await;
        let frame = frame();

        // A tail page is ten exchanges, so this is a whole page of history.
        for exchanges in [10usize, 50, 200] {
            let mut chat = crate::components::chat_area::ChatArea::new(frame.inner, None);
            for i in 0..exchanges {
                chat.add_message(crate::components::chat_area::ChatMessage::new(
                    format!("u{i}"),
                    crate::components::chat_area::ChatRole::User,
                    "看一下这个渲染路径为什么会慢，顺便解释一下 markdown 每帧重新解析的代价。",
                ));
                chat.add_message(crate::components::chat_area::ChatMessage::new(
                    format!("a{i}"),
                    crate::components::chat_area::ChatRole::Assistant,
                    "## 结论\n\n这一段讨论 **渲染** 的开销，含 `行内代码`。\n\n- 第一点\n- 第二点\n\n```rust\nfn main() {}\n```\n\n",
                ));
            }
            live.chat = chat;
            let start = std::time::Instant::now();
            let _ = live.data(&frame);
            let first = start.elapsed().as_secs_f64() * 1000.0;
            let start = std::time::Instant::now();
            for _ in 0..20 {
                let _ = live.data(&frame);
            }
            let next = start.elapsed().as_secs_f64() * 1000.0 / 20.0;
            println!("switch(miss, {exchanges:>3} exchanges): first frame = {first:>6.2} ms, then {next:>5.3} ms");
        }
        live.client.disconnect();
    }

    /// The two events that mean the agent knows more about the turn's spend than
    /// the deltas do: they are what makes the driver fetch `get_state`.
    #[test]
    fn only_usage_and_the_end_of_a_run_ask_for_a_state_refresh() {
        assert!(wants_state_refresh("usage"));
        assert!(wants_state_refresh("agent_end"));
        for quiet in ["text_chunk", "thinking_delta", "tool_start", "tool_end"] {
            assert!(
                !wants_state_refresh(quiet),
                "{quiet} must not add an RPC per token"
            );
        }
    }
}
