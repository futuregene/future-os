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

use crate::components::chat_area::{
    ChatArea, ChatMessage, ChatRole, FoldWords, ReplyDetail, RunState,
};
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
pub const APP_COMMANDS: [&str; 17] = [
    "/new",
    "/stop",
    "/reply-detail",
    "/export",
    "/quit",
    "/copy",
    "/paste",
    "/refresh",
    "/rename",
    "/mouse",
    "/compact",
    "/tools",
    "/git",
    "/files",
    "/approval",
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
/// that hits one paints immediately and then reconciles beside the surface
/// ([`Live::start_step_switch`] for a step, [`Live::finish_switch`] for the
/// doors that wait). `page` is the tail page those rows were built from
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
    /// A conversation listing that is out with the agent (see
    /// [`Self::begin_sessions_refresh`]). The driver's clock drains it, so a
    /// slow answer costs a frame's worth of staleness instead of a freeze.
    sessions_refresh: Option<tokio::sync::oneshot::Receiver<Result<Vec<SessionSummary>, String>>>,
    /// Bumped when the pinned conversations change (a pin re-files the 工作区
    /// tab, so the view it feeds is stale from that moment).
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
    /// When the manual compaction holding this session started, if one is
    /// running.
    ///
    /// `/compact` answers with an acknowledgement before the work is done, so
    /// the surface has to track the operation itself: the row says a compaction
    /// is in progress, `enter` leaves the draft alone (the agent refuses a
    /// prompt with `session_busy`), and the terminal event — committed, failed
    /// or unchanged — is what clears it again.
    compaction_start: Option<Instant>,
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
    /// The model and reasoning level the reader last chose. On the launchpad
    /// (nothing attached) the agent cannot be told `set_model`, so the picker
    /// and the launchpad's own chips stage the choice here; in a session the
    /// same choice is recorded here too, so it survives both a `/new` and a
    /// restart — the driver persists it as `settings.json` `defaultModel` /
    /// `defaultThinkingLevel`. Seeded from that file at attach, so the next
    /// session starts where the reader left off instead of the agent's default.
    pending_model: Option<String>,
    pending_effort: Option<String>,
    /// The approval mode chosen while nothing was attached. `set_sandbox_policy`
    /// needs a session like the other two setters, so the launchpad's chip and
    /// the `/mode` menu stage it here and the session the next message creates
    /// is told once it exists.
    pending_mode: Option<String>,
    /// The tool set staged on the launchpad: `Some(vec![])` is `/tools off`,
    /// `Some(tools)` is `/tools a,b`.
    ///
    /// Tools belong to a session — `set_tools` and `disable_tools` name one — so
    /// a `/tools` typed before one exists has nowhere to go. It used to be sent
    /// anyway, to whatever session this client last had (or to none), and the
    /// notice claimed the choice took effect while the session the first message
    /// created came up with every tool on. Staged here, it is applied to that
    /// session once it exists, and spent — like the approval mode, a tool set
    /// belongs to the conversation that was told it.
    pending_tools: Option<Vec<String>>,
    /// The workspace the next session will run in, chosen on the launchpad.
    /// Nothing is sent: the agent has no session to set a directory on until
    /// the first message creates one.
    pending_cwd: Option<String>,
    /// A workspace the reader picked while a conversation was attached (a
    /// workspace picker row, a `/git` worktree row) that
    /// `~/.future/tui/settings.json` has not been told about yet. The driver
    /// drains it after every action ([`Self::take_workspace_to_remember`]); the
    /// launchpad's own staging is read from `pending_cwd` instead.
    remembered_cwd: Option<String>,
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
    /// Conversation ids the reader pinned in the rail (the 置顶 section), as
    /// `~/.future/tui/settings.json` last recorded them.
    pinned_sessions: Vec<String>,
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
    /// The conversation whose state and tail page are being read off the
    /// driver's own clock (see [`Self::start_step_switch`]). One at a time,
    /// newest wins: an answer for anything else is dropped on arrival.
    switch_load: Option<String>,
    /// A switch this client began and did not wait for (see
    /// [`Self::open_conversation`]): the driver's cue that the rail's cursor
    /// belongs to the answer rather than to the frame it is about to paint.
    switch_started: bool,
    /// The page above the transcript that is out with the agent, with the
    /// window it was asked for (`session_id`, `next_offset`).
    older_load: Option<(String, i64)>,
    /// A state re-read is out (see [`Self::begin_state_refresh`]): a second one
    /// waits for it, so two answers cannot land out of order and leave the
    /// footer describing the older one.
    state_refresh: bool,
    /// Bumped for every `/git` read, so a view read for a tab the reader has
    /// already left cannot install itself over the one they asked for next.
    git_rev: u64,
    /// Where a read hands its answer to the driver's loop. `None` in a surface
    /// nobody drives (tests, the demos): those doors wait for their own answers
    /// instead.
    load_tx: Option<tokio::sync::mpsc::UnboundedSender<Loaded>>,
}

/// One read the surface started and does not wait for, on its way back to the
/// driver's loop.
///
/// Every door that would otherwise park the loop behind the agent or a `git`
/// process sends its answer here: a switch (`Live::start_step_switch`), the page
/// above the transcript (`Live::begin_load_older`), the `/git` panel
/// (`Live::begin_git_read`) and the footer's state at a turn boundary
/// (`Live::begin_state_refresh`). The loop installs each one on its own thread,
/// which is where the selection it has to be checked against lives.
pub enum Loaded {
    Switch(SwitchLoaded),
    Older(OlderPage),
    Git {
        rev: u64,
        view: super::state::GitView,
    },
    State(Result<RpcSessionState, String>),
}

/// The agent's answer to the two reads a switch makes, handed from the task
/// that read them to the loop that owns the surface.
pub struct SwitchLoaded {
    /// The conversation asked for — the key the loop checks its own selection
    /// against before installing anything.
    pub id: String,
    pub state: Result<RpcSessionState, String>,
    pub page: Result<SessionEntriesPage, String>,
}

/// The page above what the transcript has loaded, and the window it answers.
pub struct OlderPage {
    pub session_id: String,
    /// The `next_offset` the transcript carried when this page was asked for. A
    /// page for an older window — a superseded scroll, or a conversation the
    /// reader has left — is dropped rather than prepended above rows it does not
    /// sit on.
    pub before: i64,
    pub page: Result<SessionEntriesPage, String>,
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
        // `/reply-detail` expands for the run it is used in. See `ReplyDetail`.
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
        chat.set_fold_words(fold_words(lang));

        let mut live = Self {
            client,
            chat,
            state: RpcSessionState::default(),
            paging: Paging::default(),
            cached: VecDeque::new(),
            shown_page: None,
            sessions: SessionList::default(),
            sessions_refresh: None,
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
            compaction_start: None,
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
            pending_model: settings.and_then(|settings| settings.default_model.clone()),
            pending_effort: settings.and_then(|settings| settings.default_thinking_level.clone()),
            pending_mode: None,
            pending_tools: None,
            pending_cwd: None,
            remembered_cwd: None,
            run_started: None,
            run_output_tokens: 0,
            run_tps: 0.0,
            state_refresh_pending: false,
            local_cwd: String::new(),
            branch: super::git::BranchCache::new(),
            local_skills: None,
            pinned_sessions: settings
                .and_then(|settings| settings.pinned_sessions.clone())
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
            switch_load: None,
            switch_started: false,
            older_load: None,
            state_refresh: false,
            git_rev: 0,
            load_tx: None,
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
            // The run this state names belongs to the session just published —
            // publishing it cleared that name, and a terminal that attached to
            // a session already streaming must still be able to interrupt it.
            if let Some(active) = &state.active_run {
                self.client.adopt_active_run(&active.run_id);
            }
        }
        self.state = state;
        self.reconcile_queue();
        self.adopt_compaction();
        self.refresh_local();
    }

    /// Put the transcript back in step with the agent's queue.
    ///
    /// A submission bound to a run is *not* in the transcript while the agent
    /// still lists it (`RunState::Queued`, drawn above the composer instead).
    /// This is where that state ends, and each ending has its own answer:
    ///
    /// * the run is the session's active run → its turn has begun, so the card
    ///   goes back into the conversation at the point where it started;
    /// * a terminal acknowledgement says it never ran on its own: `cancelled`,
    ///   `superseded` and `session_deleted` drop the card (nothing was sent, and
    ///   a recalled prompt is back in the draft), while `merged` folds its text
    ///   into the card of the run that absorbed it;
    /// * anything else — the run left the queue and is no longer active, which
    ///   is a turn that started *and* finished between two polls → show the
    ///   card rather than keep a sent prompt hidden forever.
    ///
    /// A run the queue still lists stays hidden: the order it will run in is
    /// the agent's, and the band reads that order from `get_state` itself.
    fn reconcile_queue(&mut self) {
        let queued: HashSet<String> = self
            .state
            .queued_runs
            .iter()
            .map(|row| row.run_id.clone())
            .collect();
        let active = self.state.active_run.as_ref().map(|row| row.run_id.clone());
        for run_id in self.chat.queued_run_ids() {
            if queued.contains(&run_id) {
                continue;
            }
            if active.as_deref() == Some(run_id.as_str()) {
                self.chat.update_run_state(&run_id, RunState::Running);
                continue;
            }
            let reason = self
                .state
                .recent_terminal_acks
                .iter()
                .find(|ack| ack.run_id == run_id)
                .map(|ack| ack.reason.as_str());
            match reason {
                // `merged` is the fold: the run ahead carries this text, so the
                // card ahead grows by it — see `ChatArea::fold_run_into_previous`.
                Some("merged") => {
                    self.chat.fold_run_into_previous(&run_id);
                }
                Some("cancelled" | "superseded" | "session_deleted") => self.chat.drop_run(&run_id),
                _ => self.chat.update_run_state(&run_id, RunState::Terminal),
            }
        }
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
    ///
    /// This waits for the agent, so it belongs on the paths that *mean* to wait
    /// (the attach, a switch, the tests); the periodic poll uses
    /// [`Self::begin_sessions_refresh`] instead.
    pub async fn refresh_sessions(&mut self) -> bool {
        let Ok(sessions) = self.client.list_sessions().await else {
            return false;
        };
        self.sessions.replace(sessions)
    }

    /// Put a name this client just wrote to the agent on screen: the header's
    /// title and the rail's row for the conversation.
    ///
    /// No read follows the write. `set_session_name` persists the very string it
    /// was given and the agent echoes nothing back, so a `list_sessions` here
    /// would buy seconds of frozen loop (6.9 s for 67 conversations, measured
    /// 2026-10-28) to learn what the request already said. A poll that lands
    /// later reports the same name, and a name another client set arrives that
    /// way as well.
    fn apply_session_name(&mut self, name: &str) {
        let id = self.state.session_id.clone();
        self.state.session_name = Some(name.to_string());
        self.sessions.set_name(&id, name);
        // A cached conversation carries its own state, and the title above the
        // transcript is drawn from the *attached* one: coming back to this
        // conversation from a cached copy must not restore the old title.
        for (_, cached) in &mut self.cached {
            if cached.state.session_id == id {
                cached.state.session_name = Some(name.to_string());
            }
        }
    }

    /// Ask the agent for the conversation list *without* waiting for the answer,
    /// and answer whether this call started a listing (false: one is already
    /// out).
    ///
    /// The rail re-reads its list on a clock (`run::SESSIONS_POLL`), and the
    /// answer is a listing of every conversation the agent holds: awaiting it
    /// inside the driver loop froze the surface — no keys, no repaint, the
    /// activity row's seconds stuck — for as long as the agent took. Measured
    /// 2026-10-28 against a 67-conversation store: 6.9 s per listing, once every
    /// five seconds. Off the loop the same answer costs a frame's worth of
    /// staleness instead (see [`Self::poll_sessions`]).
    pub fn begin_sessions_refresh(&mut self) -> bool {
        if self.sessions_refresh.is_some() {
            return false;
        }
        let client = Arc::clone(&self.client);
        let (send, receive) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = send.send(client.list_sessions().await);
        });
        self.sessions_refresh = Some(receive);
        true
    }

    /// Install a listing that has landed: `None` while it is still out,
    /// `Some(changed)` once it has — whether the rail owes a frame.
    ///
    /// Called from the driver's clock, beside [`Self::poll_local_reads`]: the
    /// loop that owns the tick is the one place that decides a frame is owed.
    pub fn poll_sessions(&mut self) -> Option<bool> {
        // The listing's answer holds no borrow of the receiver, so the slot is
        // free to take the next one as soon as this frame is installed.
        let received = match self.sessions_refresh.as_mut()?.try_recv() {
            Ok(result) => result,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return None,
            // The task went away without answering (a panic, or a runtime
            // shutting down): the rail keeps its last list.
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => Err(String::new()),
        };
        self.sessions_refresh = None;
        Some(match received {
            Ok(sessions) => self.sessions.replace(sessions),
            Err(_) => false,
        })
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
                // The reader's own pin, not the agent's: `/pin` names one
                // conversation, and the rail draws it in 置顶 instead of under
                // the directory it ran in.
                pinned: self.pinned_sessions.iter().any(|id| id == &session.id),
            })
            .collect();
        // The 工作区 tab: the same rows, filed the way the desktop's rail files
        // them. A conversation that ran in one of the app's own scratch
        // directories belongs to no workspace, so it is a plain chat.
        let (workspaces, chats) = file_sessions(&self.sessions, &rows, &roots);
        SessionView {
            key,
            rows,
            workspaces,
            chats,
        }
    }

    /// Pin or unpin one conversation, and answer what belongs on the notice row.
    pub fn set_session_pinned(&mut self, id: &str, pinned: bool) -> String {
        self.pinned_sessions.retain(|known| known != id);
        if pinned {
            self.pinned_sessions.push(id.to_string());
        }
        self.pins_rev = self.pins_rev.wrapping_add(1);
        // The row's own label, so the notice names the conversation the reader
        // pinned rather than the id the agent filed it under.
        let title = self
            .sessions
            .iter()
            .find(|session| session.id == id)
            .map(display_name)
            .unwrap_or_else(|| id.to_string());
        if pinned {
            i18n::render(self.lang, Key::NoticePinned, &[&title])
        } else {
            i18n::render(self.lang, Key::NoticeUnpinned, &[&title])
        }
    }

    /// The conversation ids the rail currently pins, for the settings file.
    pub fn pinned_sessions(&self) -> &[String] {
        &self.pinned_sessions
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
                // A run this client attached to mid-flight never saw its
                // `agent_start`: the first slice that arrives is enough to know
                // the trailing reply is still being written.
                self.chat.begin_reply_run();
                self.chat.append_to_last_message(&text);
            }
            "agent_start" => {
                self.run_tools.clear();
                self.run_started = Some(Instant::now());
                self.run_output_tokens = 0;
                self.run_tps = 0.0;
                self.mark(ActivityKind::Thinking);
                // A queued submission whose run just started stops being a queue
                // row: its card belongs in the conversation from here on. The
                // state refresh asks the agent for the same answer; this one
                // lands without waiting for the round trip.
                if let Some(run_id) = event.run_id.as_deref().filter(|id| !id.is_empty()) {
                    self.chat.update_run_state(run_id, RunState::Running);
                }
                self.chat.begin_reply_run();
                self.chat.add_message(ChatMessage::new(
                    crate::session_rows::random_id(),
                    ChatRole::Assistant,
                    "",
                ));
            }
            "thinking_start" => {
                self.mark(ActivityKind::Thinking);
                self.chat.begin_reply_run();
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
                self.chat.begin_reply_run();
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
                // Only now may the reading level fold the reply onto its
                // answer: the desktop collapses a reply once it has stopped
                // streaming, never between two of its slices.
                self.chat.end_reply_run();
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
            // ── a compaction of this conversation ───────────────────────
            //
            // Only a standalone (manual) compaction takes the session from
            // the reader: `/compact` is refused while a run streams, and the
            // desktop treats the whole window as "cannot send". An automatic
            // one happens *inside* a run, whose own activity row is the truth
            // while it streams — so it leaves the row alone and only reports
            // itself when it commits.
            "compaction_started" => {
                if is_standalone_compaction(&event.data) {
                    self.begin_compaction();
                }
            }
            "compaction_committed" => {
                let before = token_count(&event.data, "tokens_before");
                let after = token_count(&event.data, "tokens_after");
                self.end_compaction();
                // The divider is written here as well as into the history's
                // own projection: the live transcript has to show where the
                // conversation was cut without a reload, and a later reload
                // replaces every row anyway (`session_rows` builds the same
                // one from the checkpoint).
                self.chat.add_message(ChatMessage::new(
                    crate::session_rows::random_id(),
                    ChatRole::System,
                    &crate::session_rows::compaction_text_of(before, after),
                ));
                self.revision = self.revision.wrapping_add(1);
                self.notice = Some(match (before, after) {
                    (Some(before), Some(after)) => {
                        i18n::render(self.lang, Key::CompactDoneTokens, &[&before, &after])
                    }
                    _ => t(self.lang, Key::CompactDone).to_string(),
                });
            }
            "compaction_unchanged" => {
                self.end_compaction();
                self.notice = Some(t(self.lang, Key::CompactUnchanged).into());
            }
            "compaction_failed" => {
                self.end_compaction();
                let error = str_field(&event.data, "error");
                self.notice = Some(if error.is_empty() {
                    t(self.lang, Key::CompactFailed).to_string()
                } else {
                    i18n::render(self.lang, Key::WorkflowFailed, &[&error])
                });
            }
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
        // A compaction belongs to the conversation that started it, like the
        // run: the next one's session is not held by it. `get_state` puts the
        // new session's own flag back (`adopt_compaction`).
        self.compaction_start = None;
        // No run is writing this transcript any more: its last reply is
        // settled, and the reading level may fold it (a switch, a fork or a
        // stop leaves nothing that will still send slices for it).
        self.chat.end_reply_run();
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
        // A compaction belongs to the session that started it like a run does
        // — but `install_state` has just put the *new* session's flag on
        // screen, so the forget above must not be the last word: switching into
        // (or attaching to) a conversation that is compacting has to keep
        // saying so.
        self.adopt_compaction();
        if self.state.is_streaming {
            self.mark(ActivityKind::Thinking);
            self.run_started = Some(Instant::now());
            // The trailing reply is being written by a run this client did not
            // start: every row stays in place until that run ends.
            self.chat.begin_reply_run();
        }
    }

    /// Put the compaction row in step with the agent's own flag.
    ///
    /// `get_state` reports `isCompacting` for the session on screen, whatever
    /// client started it, so a compaction another client ran — or one this
    /// terminal attached to mid-flight — is visible here too. The flag is the
    /// agent's, so a state that says "not compacting" ends the row: the event
    /// that carries the token counts may have been missed by an attach, and a
    /// surface stuck on "compacting" would refuse the reader's prompts forever.
    fn adopt_compaction(&mut self) {
        if self.state.is_compacting {
            self.begin_compaction();
        } else if self.compacting() {
            // This client attached after the operation began and its terminal
            // event is gone: the flag is the agent's, so the sentence that
            // promised a wait must not outlive it either.
            self.end_compaction();
            if self.notice.as_deref() == Some(t(self.lang, Key::NoticeCompacting)) {
                self.notice = None;
            }
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
        let key = self.draft_key();
        self.save_draft_of(key);
    }
    /// File the box under `key` — the conversation it currently belongs to.
    fn save_draft_of(&mut self, key: String) {
        self.drafts.insert(
            key,
            DraftSnapshot {
                editor: self.draft.editor.clone(),
                pending: self.draft.editor.pending(),
            },
        );
    }
    fn restore_draft(&mut self) {
        let key = self.draft_key();
        self.restore_draft_of(key);
    }
    /// Put `key`'s draft in the box, or an empty one when it has none.
    ///
    /// Keyed by argument rather than by `self.state` because a switch moves the
    /// box onto a conversation the state does not name yet — the answer that
    /// will name it is still out with the agent.
    fn restore_draft_of(&mut self, key: String) {
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
                input_tokens: state.usage.input_tokens.max(0) as u64,
                output_tokens: state.usage.output_tokens.max(0) as u64,
                cache_read_tokens: state.usage.cache_read_tokens.max(0) as u64,
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
            row_stops: self.chat.row_stops(),
            input,
            activity,
            compacting: self.compacting(),
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
        // A manual compaction is what the session is doing, and it is the
        // answer to "why does enter not send": it outranks the idle row the
        // finished run left behind.
        if let Some(start) = self.compaction_start {
            return Activity {
                kind: ActivityKind::Compacting,
                detail: String::new(),
                elapsed_secs: start.elapsed().as_secs(),
                tools: 0,
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
            | WbAction::CopyMessage(_)
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
            | WbAction::OpenFork
            | WbAction::OpenWorktree => {}
            WbAction::ApprovalDecision { approved, rule } => {
                self.decide_with_rule(approved, rule).await
            }
            WbAction::SelectDiff(_) => {}
            // Opening a transcript row is the chat's own state (`tab` + `enter`,
            // or a click): the driver only has to repaint, and the revision
            // moves because the rows — and the spans over them — changed.
            WbAction::ToggleChatRow(row) => {
                if self.chat.toggle_row(row).is_some() {
                    self.revision = self.revision.wrapping_add(1);
                }
            }
            WbAction::SelectWorktree(path) => self.select_worktree(&path).await,
            WbAction::ForkAt(id) => self.fork_at(&id).await,
            WbAction::Compact(instructions) => self.compact(instructions).await,
            WbAction::SetApproval(mode) => self.set_approval(&mode).await,
            WbAction::SelectWorkspace(path) => self.select_workspace(&path).await,
            WbAction::SelectChatWorkspace => self.select_chat_workspace().await,
            WbAction::ToggleFolder(path) => self.toggle_folder(&path),
            WbAction::RefreshFiles => self.refresh_tree(),
            // The preview itself is opened by the driver, which owns the
            // overlay; loading the bytes — and the list a step walks — is ours.
            WbAction::PreviewFile(path) => self.start_preview(&path),
            WbAction::PreviewChatFile(path) => self.start_picture(&path),
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
            WbAction::LoadOlder => {
                if self.load_tx.is_none() {
                    // No driver to hand the page to (the frame tests): read it
                    // here, which is the waiting form.
                    self.load_older().await;
                } else {
                    self.begin_load_older();
                }
            }
            WbAction::SwitchSession(id) => self.open_conversation(&id).await,
            // A delete also loads the successor. Keep that nested async state
            // off the caller's stack, which is small on Windows.
            WbAction::DeleteSession(id) => Box::pin(self.delete_session(&id)).await,
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
            // applies it through [`Live::set_session_pinned`], so here it is
            // only a shape the agent never sees.
            WbAction::SetSessionPinned { .. } => {}
            // The driver displays local notices; a demo adapter has no notice
            // dispatch of its own to perform.
            WbAction::Notice(_) => {}
            WbAction::FocusPane(_) => {}
            WbAction::Redraw => {}
        }
        Outcome::Continue
    }

    /// `/compact`: ask the agent to compact this conversation.
    ///
    /// The answer is an acknowledgement, not the result — the agent summarises
    /// on its own worker — so the surface takes the session over here and gives
    /// it back when the operation's terminal event arrives (the token counts
    /// travel with `compaction_committed`).
    async fn compact(&mut self, instructions: String) {
        match self
            .client
            .compact((!instructions.is_empty()).then_some(instructions.as_str()))
            .await
        {
            Ok(ack) if ack.accepted => {
                self.begin_compaction();
                // The agent's own flag is authoritative — and the gauge the
                // footer draws has nothing to do with this request.
                self.refresh_state().await;
            }
            Ok(_) => {
                self.notice = Some(t(self.lang, Key::CompactFailed).into());
            }
            Err(error) => {
                self.notice = Some(i18n::render(self.lang, Key::WorkflowFailed, &[&error]));
                // A refusal (a run in flight, a compaction already running)
                // leaves the agent's state as it was, and that state is what
                // the row should keep showing.
                self.refresh_state().await;
            }
        }
    }

    /// Is a compaction holding this session right now?
    pub fn compacting(&self) -> bool {
        self.compaction_start.is_some()
    }

    /// Take the session over for a compaction, if it is not already.
    fn begin_compaction(&mut self) {
        if self.compaction_start.is_none() {
            self.compaction_start = Some(Instant::now());
            self.notice = Some(t(self.lang, Key::NoticeCompacting).into());
        }
    }

    /// Give the session back: the operation reached a terminal event.
    fn end_compaction(&mut self) {
        self.compaction_start = None;
    }

    /// Send the draft. A submission while a run is in flight joins the agent's
    /// queue and goes out when that run ends — as **one** message with whatever
    /// else is waiting (`enqueue_coalescing`): a burst of supplements is a
    /// follow-up, not three turns. Until that boundary each one is its own row
    /// in the queue band above the composer, where it can be cancelled or
    /// recalled; at the boundary the agent folds them in queue order and the
    /// transcript shows the run's card carrying every text (`fold_run_into_
    /// previous`). The band, and the transcript's silence about a submission
    /// until it goes out, are what make "queued" visible instead of looking
    /// already sent.
    async fn submit(&mut self, text: &str) {
        // A compaction owns the session: the agent answers a prompt with
        // `session_busy` until the checkpoint lands, and the draft is the
        // reader's, so it stays where it is with the reason on the notice row.
        if self.compacting() {
            self.notice = Some(t(self.lang, Key::ComposerCompacting).into());
            return;
        }
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
        let mut sent = ChatMessage::new(crate::session_rows::random_id(), ChatRole::User, &typed);
        // The id the acknowledgement binds the run to: the frame draws this
        // card only once the agent has actually started the turn (see
        // [`Self::reconcile_queue`]).
        let message_id = sent.id.clone();
        // The pictures travel with the bubble the reader just sent. The agent
        // records them on the entry and the history replays them from its
        // metadata, but that is a round trip away: the thumbnail is owed now.
        sent.pictures = attachments
            .iter()
            .filter(|attachment| attachment.kind == "image" && !attachment.path.is_empty())
            .map(|attachment| crate::components::chat_area::ChatPicture {
                path: attachment.path.clone(),
                name: attachment.name.clone(),
            })
            .collect();
        self.chat.add_message(sent);
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
                    self.completion_candidates.push_back(ack.run_id.clone());
                }
                if ack.accepted_state == "queued" {
                    // Bind the bubble to the run *and* to the queue: the card
                    // leaves the transcript until the agent starts the run (it
                    // is listed above the composer meanwhile), which is what
                    // keeps a submission that has not been sent from reading
                    // like one that has.
                    self.chat.bind_user_run(
                        &message_id,
                        &ack.run_id,
                        RunState::Queued,
                        ack.queue_position.map(|position| position as u32),
                    );
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
            "stop" => self.abort().await,
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
            "compact" if self.detached() => {
                self.notice = Some(t(self.lang, Key::NeedsConversation).into());
            }
            "compact" => {
                self.compact(arg).await;
            }
            "history" | "fork" => {}
            "worktree" if !arg.is_empty() => {
                if arg.strip_prefix("new ").is_some() {
                    self.notice = Some(t(self.lang, Key::WorktreeCreating).into());
                } else {
                    self.notice = Some(crate::worktree::WORKTREE_USAGE.into());
                }
            }
            // The launchpad has no session yet, so a tool set has nowhere to
            // take effect: it is staged for the one the first message creates,
            // exactly like the model and effort chips. Sending it anyway named
            // the session the reader had just left and told them it worked.
            "tools" if self.detached() => {
                if arg.is_empty() {
                    self.notice = Some(t(self.lang, Key::ToolsUsage).into());
                } else {
                    let tools: Vec<String> = if arg == "off" {
                        Vec::new()
                    } else {
                        arg.split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .collect()
                    };
                    self.pending_tools = Some(tools);
                    self.notice = Some(t(self.lang, Key::ToolsPending).into());
                }
            }
            "tools" => {
                if arg.is_empty() {
                    self.notice = Some(t(self.lang, Key::ToolsUsage).into());
                } else if arg == "off" {
                    // The choice is this conversation's, so a `/new` after it
                    // starts with the tools the agent gives a session.
                    self.pending_tools = None;
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
            // A model named in the command line is a choice like any other, so
            // it takes the launchpad road too when nothing is attached.
            "model" if !arg.is_empty() && self.detached() => self.set_pending_model(&arg),
            "model" if !arg.is_empty() => self.set_model(&arg).await,
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
                // The reading level's cycle: every slice, then the work folded,
                // then the process folded too — one use folds one more thing
                // away.
                let next = self.detail.next();
                self.set_detail(next);
            }
            "refresh" => {
                self.refresh_state().await;
                self.refresh_sessions().await;
            }
            // The title is the one thing about a conversation the reader names
            // themselves, so it is written to the agent rather than kept here
            // (the desktop and the phone rename through the same command, and
            // the rail's row is then whatever the agent reports).
            "rename" if self.detached() => {
                self.notice = Some(t(self.lang, Key::NeedsConversation).into());
            }
            "rename" if arg.is_empty() => {
                self.notice = Some(t(self.lang, Key::RenameUsage).into());
            }
            "rename" => match self.client.set_session_name(&arg).await {
                Ok(()) => {
                    // The name the request carried is the answer, so it is
                    // installed rather than read back (see
                    // [`Self::apply_session_name`]).
                    self.apply_session_name(&arg);
                    self.notice = Some(i18n::render(self.lang, Key::NoticeRenamed, &[&arg]));
                }
                Err(error) => {
                    self.notice = Some(i18n::render(self.lang, Key::WorkflowFailed, &[&error]))
                }
            },
            // The whole record, through the agent's own exporter (the same one
            // the desktop uses). The path it wrote is the whole answer, so it
            // goes on the notice row rather than into the transcript.
            "export" if self.detached() => {
                // `/export` writes *a* conversation; with none on screen it used
                // to write the one the reader had left, and to report the path
                // as if that were the record they asked for.
                self.notice = Some(t(self.lang, Key::NeedsConversation).into());
            }
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
            // handler): `/usage`, `/skills`, `/workspace`, `/help`. They are
            // named here so a caller that bypasses the surface is not told a
            // command the palette advertises does not exist — the notice the
            // whole family used to produce. The four choosers (`/model`,
            // `/thinking`, `/mode`, `/workspace`) come back
            // to the surface the same way: the picker is not the agent's.
            // The verbs the *driver* answers before this handler ever runs: the
            // boxes it opens (`/git`, `/files`, `/approval`), the choosers the
            // surface opens for the reader, and the names the panels took with
            // them (`/usage`, `/skills`). They are listed so a caller that
            // bypasses the driver is not told a command the palette advertises
            // does not exist — the notice the whole family used to produce.
            "usage" | "skills" | "workspace" | "help" | "thinking" | "mode" | "git" | "files"
            | "approval" | "worktree" => {}
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
    /// Kept for the doors that have no driver to hand an answer to, and for
    /// `--continue` (which runs before the loop exists). Everything the driver
    /// owns — a click in the rail, the launchpad's recents, a `ctrl+r` pick —
    /// goes through [`Self::open_conversation`] instead, which is the same
    /// switch without the wait.
    async fn switch(&mut self, session_id: &str) {
        if self.already_showing(session_id) {
            return;
        }
        self.begin_switch(session_id);
        self.finish_switch(session_id).await;
    }

    /// Open a conversation from a door that is not a step the driver already
    /// owns: a click in the rail, the launchpad's recents, a `ctrl+r` pick.
    ///
    /// Off the loop when there is one — the same non-blocking switch a step
    /// takes, because a click is as deliberate as a step and the agent's answer
    /// just as slow (75 ms warm, seconds cold, measured) — and awaited when
    /// there is no driver to hand the answer to (the frame tests, the demos).
    ///
    /// The driver is told the switch *began* rather than shown its answer
    /// ([`Self::take_switch_started`]): the frame painted before the rows land
    /// still names the conversation being left, so following "the conversation
    /// on screen" then would park the rail's cursor back where the reader came
    /// from. The answer's own arm does that follow when the rows arrive.
    async fn open_conversation(&mut self, session_id: &str) {
        if self.load_tx.is_none() {
            self.switch(session_id).await;
            return;
        }
        if self.already_showing(session_id) {
            return;
        }
        self.start_step_switch(session_id);
        self.switch_started = true;
    }

    /// Take the "a switch began" flag: `true` when the last action opened a
    /// conversation that has not answered yet.
    pub fn take_switch_started(&mut self) -> bool {
        std::mem::take(&mut self.switch_started)
    }

    /// Everything about a switch that costs no round trip.
    ///
    /// The draft is filed under the conversation being left and the one being
    /// landed on is put in the box, the cached conversation (if any) is swapped
    /// onto the screen, and the rail's step is forgotten. Returns whether cached
    /// rows are on screen *now*: `true` means the caller may paint them before
    /// the agent has answered anything, which is the whole point of keeping
    /// them. A miss leaves the transcript untouched — the rows are replaced when
    /// the page arrives, and nothing paints in between — and returns `false`.
    ///
    /// The draft moves here rather than with the rows because the box belongs to
    /// the conversation the reader is *on*: with the answer still out, a key
    /// typed into a box still holding the last conversation's text would be
    /// filed under the conversation they left when the answer landed.
    pub fn begin_switch(&mut self, session_id: &str) -> bool {
        if self.already_showing(session_id) {
            // Nothing to open, and whatever a step is still reading belongs to a
            // conversation the reader has left — including a step *back* onto
            // this one while its successor was being read.
            self.switch_load = None;
            return false;
        }
        // Whatever a step was reading is not this conversation's (a click, or
        // the next step, supersedes it): its answer is dropped on arrival.
        self.switch_load = None;
        // Filed under the conversation being left, before the launchpad flag and
        // the screen's own state move off it.
        self.save_draft();
        self.launchpad = false;
        self.notice = None;
        let cached = self.take_cached(session_id);
        self.restore_draft_of(session_id.to_string());
        // The row is the session's: nothing of the one just left may stay on it
        // (see [`Self::adopt_run`]). With cached rows this already uses the
        // state that came in with them, so the frame painted from them is
        // coherent; the agent's answer re-runs it.
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
    ///
    /// This is the *waiting* form. The driver's step uses
    /// [`Self::start_step_switch`], which puts the two reads in a task instead.
    pub async fn finish_switch(&mut self, session_id: &str) {
        let (state, page) = tokio::join!(
            self.client.fetch_state_of(session_id),
            self.client.session_tail_page(session_id)
        );
        self.install_switch(session_id, state, page);
    }

    /// Hand the surface the channel its non-blocking switches answer on.
    ///
    /// Called once by the driver before its loop starts: the loop is what owns
    /// the wake-up, so the surface only ever holds the sending end.
    pub fn set_load_sender(&mut self, tx: tokio::sync::mpsc::UnboundedSender<Loaded>) {
        self.load_tx = Some(tx);
    }

    /// The non-blocking half of a switch: swap what the cache can show onto the
    /// screen *now* and read the rest in a task.
    ///
    /// Used by both doors that have a driver behind them — a `shift+↑/↓` step
    /// (`run::show_conversation`) and a click ([`Self::open_conversation`]).
    /// Returns whether cached rows are on screen — the caller's cue to paint
    /// before the answer arrives. A surface with no driver attached (the frame
    /// tests, the demos) has nowhere to hand the answer, so it sends nothing:
    /// those doors call [`Self::switch`], which waits for its own.
    ///
    /// One load is out at a time, keyed by the conversation asked for. A step
    /// that lands on a third conversation while the second is still out starts
    /// that one too — the answer already on its way is dropped when it arrives
    /// ([`Self::install_switch_load`]), because two page reads are cheap next to
    /// making the reader wait for a conversation they have already left. Held
    /// keys never get here in bulk: `run::OPEN_SETTLE` is what collapses a burst
    /// onto one conversation.
    pub fn start_step_switch(&mut self, session_id: &str) -> bool {
        if self.already_showing(session_id) {
            // Back onto the conversation on screen: nothing to open, and a read
            // still out belongs to a conversation the reader has left.
            self.switch_load = None;
            return false;
        }
        let cached = self.begin_switch(session_id);
        self.load_switch(session_id);
        cached
    }

    /// Ask the agent for `session_id`'s state and tail page without waiting.
    fn load_switch(&mut self, session_id: &str) {
        if self.switch_load.as_deref() == Some(session_id) {
            return;
        }
        let Some(tx) = self.load_tx.clone() else {
            return;
        };
        let client = self.client.clone();
        let id = session_id.to_string();
        self.switch_load = Some(id.clone());
        tokio::spawn(async move {
            let (state, page) =
                tokio::join!(client.fetch_state_of(&id), client.session_tail_page(&id));
            // The receiver is gone when the driver has quit: the answer is
            // simply dropped with it.
            let _ = tx.send(Loaded::Switch(SwitchLoaded { id, state, page }));
        });
    }

    /// Take one finished switch load from the driver's loop.
    ///
    /// An answer for a conversation the reader has already stepped past is
    /// dropped: installing it would paint rows for a conversation that is no
    /// longer selected. Returns whether a frame is owed.
    pub fn install_switch_load(&mut self, loaded: SwitchLoaded) -> bool {
        if self.switch_load.as_deref() != Some(loaded.id.as_str()) {
            return false;
        }
        self.switch_load = None;
        self.install_switch(&loaded.id, loaded.state, loaded.page)
    }

    /// Ask the agent for a conversation's tail page and throw the answer away.
    ///
    /// The agent keeps the page it materialized (keyed by the conversation's
    /// revision), so reading one early is worth it only when the reader is
    /// *about* to open it — and the rail makes that predictable: after a step
    /// settles, the two conversations either side of the cursor are exactly where
    /// the next step can land. Measured on a 147-conversation store at a 300 ms
    /// reading cadence, a step onto a warmed conversation costs **18.1 ms median
    /// instead of 120.0 ms** (and 61.9 ms instead of 153.0 ms for the worst one).
    ///
    /// Nothing is installed and nothing is owed: a prefetch that loses the race,
    /// or whose conversation the reader never opens, costs the agent one page
    /// read — off this loop — and nothing else. A page whose conversation moved
    /// on since is keyed by a newer revision and is simply read again when opened.
    pub fn prefetch_page(&self, session_id: &str) {
        if session_id.is_empty() || self.already_showing(session_id) {
            return;
        }
        let client = Arc::clone(&self.client);
        let id = session_id.to_string();
        tokio::spawn(async move {
            let _ = client.session_tail_page(&id).await;
        });
    }

    // ─── the page above the transcript ───────────────────────────────

    /// Ask for the page above what the transcript has loaded, without waiting.
    ///
    /// Scrolling up past the first loaded row is the one door a reader opens
    /// over and over, and a page is a real read on the agent's side (measured:
    /// 38–285 ms first time, 6–53 ms once the page is cached). Waiting for it in
    /// the loop froze the rail, the stream and the keyboard while the reader was
    /// already scrolling.
    ///
    /// Returns whether a read was started: a transcript with no page above it, a
    /// surface with no driver, and a window already being read all answer `false`
    /// and change nothing.
    pub fn begin_load_older(&mut self) -> bool {
        if !self.paging.has_more {
            return false;
        }
        let Some(tx) = self.load_tx.clone() else {
            return false;
        };
        let session_id = self.paging.session_id.clone();
        let before = self.paging.next_offset;
        if self.older_load.as_ref() == Some(&(session_id.clone(), before)) {
            return false;
        }
        self.older_load = Some((session_id.clone(), before));
        let client = self.client.clone();
        tokio::spawn(async move {
            let page = client.session_older_page(&session_id, before).await;
            let _ = tx.send(Loaded::Older(OlderPage {
                session_id,
                before,
                page,
            }));
        });
        true
    }

    /// Take one finished older page from the driver's loop.
    ///
    /// The transcript has to be the conversation and the window this page was
    /// asked for; anything else (a scroll that moved on, a conversation the
    /// reader left) is dropped rather than prepended above rows it does not sit
    /// on. Returns whether a frame is owed.
    pub fn install_older(&mut self, older: OlderPage) -> bool {
        if self.older_load.as_ref() != Some(&(older.session_id.clone(), older.before)) {
            return false;
        }
        self.older_load = None;
        if older.session_id != self.paging.session_id || older.before != self.paging.next_offset {
            return false;
        }
        match older.page {
            Ok(page) => {
                self.apply_page(page, true);
                true
            }
            Err(error) => {
                self.notice = Some(format!("Could not load older rows: {error}"));
                true
            }
        }
    }

    // ─── the footer's state, at a turn boundary ──────────────────────

    /// Re-read the session's state without waiting for it.
    ///
    /// A `usage`/`agent_end` event means the agent knows more about the turn than
    /// the deltas did, so the footer asks (`run`'s `take_state_refresh`). That
    /// read is cheap but not free (25 ms measured, and the loop used to sit
    /// through it at every turn's end); one is out at a time, so two answers
    /// cannot land out of order and leave the footer describing the older one.
    pub fn begin_state_refresh(&mut self) -> bool {
        if self.state_refresh {
            return false;
        }
        let Some(tx) = self.load_tx.clone() else {
            return false;
        };
        self.state_refresh = true;
        let client = self.client.clone();
        tokio::spawn(async move {
            // The failure is sent too: a read that never answers must not leave
            // the slot filled for the rest of the session.
            let _ = tx.send(Loaded::State(client.get_state().await));
        });
        true
    }

    /// Take one finished state read from the driver's loop.
    ///
    /// A read that failed changes nothing (the footer keeps the last figures it
    /// was given) but still frees the slot. Returns whether a frame is owed.
    pub fn install_state_refresh(&mut self, state: Result<RpcSessionState, String>) -> bool {
        self.state_refresh = false;
        match state {
            Ok(state) => {
                self.install_state(state);
                true
            }
            Err(_) => false,
        }
    }

    // ─── the `/git` panel ────────────────────────────────────────────

    /// Read the repository for the `/git` panel off the loop.
    ///
    /// The read is local (a `git` process per section) but not cheap: measured
    /// on this repository, opening the panel costs 132 ms for the commit log and
    /// 155 ms for the refs, and the loop used to sit through it every time the
    /// panel opened, changed tab or narrowed its filter. Returns whether a read
    /// was started; a surface with no driver reads it itself (`run`).
    pub fn begin_git_read(
        &mut self,
        cwd: std::path::PathBuf,
        section: super::state::GitSection,
        filter: Option<String>,
        want: usize,
    ) -> bool {
        let Some(tx) = self.load_tx.clone() else {
            return false;
        };
        self.git_rev = self.git_rev.wrapping_add(1);
        let rev = self.git_rev;
        let lang = self.lang;
        tokio::task::spawn_blocking(move || {
            let view = super::git_panel::read(&cwd, section, filter, want, lang);
            let _ = tx.send(Loaded::Git { rev, view });
        });
        true
    }

    /// Take one finished `/git` read from the driver's loop.
    ///
    /// A view read for a tab the reader has already left is dropped: installing
    /// it would flip the whole panel back to the previous section. Returns
    /// whether a frame is owed.
    pub fn install_git(&mut self, rev: u64, view: super::state::GitView) -> bool {
        if rev != self.git_rev {
            return false;
        }
        self.git = Some(view);
        true
    }

    /// Put the agent's answer for a switch on screen.
    ///
    /// Shared by the waiting form ([`Self::finish_switch`]) and the driver's
    /// ([`Self::install_switch_load`]), so which rows are kept, when the notice
    /// speaks and what the activity row shows are decided in one place.
    fn install_switch(
        &mut self,
        session_id: &str,
        state: Result<RpcSessionState, String>,
        page: Result<SessionEntriesPage, String>,
    ) -> bool {
        let state = match state {
            Ok(state) => state,
            Err(error) => {
                // The box took the target's draft on the way in: filed under
                // that conversation, it survives, and the one on screen gets its
                // own text back.
                self.save_draft_of(session_id.to_string());
                self.restore_draft();
                // Nothing moved: the header and the transcript stay on the
                // conversation that is still current, and the notice says why
                // the one the reader picked did not open.
                self.notice = Some(format!("Could not switch session: {error}"));
                return true;
            }
        };
        // The rows on screen are the conversation being left — a cache hit has
        // already moved them into the cache. Kept here, before the incoming
        // state renames what is on screen and `show_page` replaces the rows
        // themselves, so the way back is a repaint. A miss therefore paints the
        // new rows and files the old ones in the same frame.
        if !self.already_showing(session_id) {
            self.remember_current();
        }
        // The run map before the state: `install_state` decides the approval row
        // from it, and the state this client holds is the one adopted.
        self.client.adopt_state(&state);
        self.install_state(state);
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
        true
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
                // Nothing cached to paint. The rows on screen stay exactly
                // where they are — they are the conversation being left, and
                // they are kept (`remember_current`) the moment the answer for
                // the new one is in hand. Clearing them here instead would
                // blank the transcript for as long as the agent takes to
                // answer, and the driver paints between the two: a tick, a
                // streamed delta from the run in the conversation being left,
                // its own clock.
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

    /// File the conversation on screen in the cache: its state, the pages it has
    /// loaded and the rows themselves.
    ///
    /// Used by a switch that had nothing cached to paint — the rows stay on
    /// screen until the incoming page is in hand, so this is called at that
    /// moment rather than when the step was taken (see [`Self::install_switch`]).
    /// The rows are *moved*: a `ChatArea` is the one copy of what the reader has
    /// read, and the incoming page replaces it.
    fn remember_current(&mut self) {
        let outgoing = (
            self.state.session_id.clone(),
            self.paging.clone(),
            self.state.clone(),
            self.shown_page.clone(),
        );
        let empty = self.chat.empty_like();
        let previous = std::mem::replace(&mut self.chat, empty);
        self.paging = Paging::default();
        self.shown_page = None;
        self.remember(outgoing, previous);
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
        // A step's answer on its way belongs to a conversation the reader has
        // just left; installing it would paint one over the launchpad.
        self.switch_load = None;
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

    /// Stage the workspace the first conversation will run in — the launchpad's
    /// default, so `cd project && future tui` creates that project's
    /// conversation, and a `future tui` typed wherever the terminal happened to
    /// sit reopens the workspace chosen last time (`remembered`).
    ///
    /// Called by the driver rather than by [`Self::attach`]: attaching is also
    /// how tests and an embedding caller open the surface, and those must not
    /// inherit whatever directory the process happens to run in.
    pub fn stage_launch_workspace(&mut self, remembered: Option<&str>) {
        if !self.detached() {
            return;
        }
        let launched = std::env::current_dir()
            .ok()
            .map(|dir| dir.to_string_lossy().to_string());
        if let Some(workspace) = launch_workspace(launched.as_deref(), remembered) {
            self.pending_cwd = Some(workspace);
            self.refresh_local();
        }
    }

    /// The workspace `settings.json` should remember (`lastWorkspace`), if this
    /// client holds one it has not handed over yet.
    ///
    /// Two sources, both of them a *choice* rather than a directory the reader
    /// merely visited: the workspace the launchpad has staged for the next
    /// conversation, and a workspace picked while one was attached. A
    /// conversation the reader only switched to is not one — remembering it
    /// would make an old session's directory the next start's default.
    ///
    /// Draining rather than reading keeps the driver's write cheap: it compares
    /// this against the file and writes only when the two differ. The launchpad's
    /// staging is read, not drained — the chip keeps naming that workspace until
    /// the session exists — so the driver sees it again on the next action; that
    /// comparison is what makes the repeat free.
    pub fn take_workspace_to_remember(&mut self) -> Option<String> {
        if let Some(chosen) = self.remembered_cwd.take() {
            return Some(chosen);
        }
        if !self.detached() {
            return None;
        }
        let staged = self.pending_cwd.as_deref()?.trim().to_string();
        if staged.is_empty() || is_chat_directory(&staged) {
            return None;
        }
        Some(staged)
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
    /// A launchpad with nothing staged this visit names the reader's remembered
    /// default — the model they last selected, kept in `settings.json` — before
    /// falling back to the agent's default (the model `list_models` flags
    /// `isDefault`). That remembered default is the reader's own choice, not
    /// the model of the conversation `/new` just left behind: attaching to a
    /// session that runs a different model does not overwrite it.
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
                // The workspace and approval mode are spent once the session
                // exists (the agent holds them now). The model and level are
                // kept: they are the reader's remembered default, so the next
                // `/new` starts where the reader left off instead of the
                // agent's own default.
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
                // The tool set staged on the launchpad (`/tools off`,
                // `/tools read,shell`) is one call too, and it is spent here:
                // it described *this* conversation, not a default for the next
                // one. A failure keeps the reader's attention on the notice row
                // — the session exists either way.
                if let Some(tools) = self.pending_tools.take() {
                    let result = if tools.is_empty() {
                        self.client.disable_tools().await
                    } else {
                        self.client.set_tools(&tools).await
                    };
                    if let Err(error) = result {
                        self.notice = Some(i18n::render(self.lang, Key::WorkflowFailed, &[&error]));
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

    /// Apply one model by id (the `/model` picker's row), and remember it as
    /// the reader's default so the next session starts with it.
    async fn set_model(&mut self, model_id: &str) {
        match self.client.set_model(model_id).await {
            Ok(()) => {
                self.notice = None;
                self.pending_model = Some(model_id.to_string());
                self.refresh_state().await;
            }
            Err(error) => self.notice = Some(format!("Could not set the model: {error}")),
        }
    }

    /// Apply one reasoning level (the model picker's slider), and remember it
    /// as the reader's default.
    async fn set_thinking(&mut self, level: &str) {
        let wire = self.wire_thinking_level(level);
        match self.client.set_thinking_level(&wire).await {
            Ok(()) => {
                self.notice = None;
                self.pending_effort = Some(level.to_string());
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
                    self.pending_model = Some(model.to_string());
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
                    self.pending_effort = Some(level.to_string());
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

    /// Load the messages `/fork` can stop after: every question and every
    /// answer the session has persisted, in transcript order.
    ///
    /// The list comes from the agent rather than off the transcript, because
    /// the agent is what knows which messages are *persisted*: a reply still
    /// streaming has no entry to fork at yet, so the newest point the picker
    /// offers is the prompt that started it — exactly the rule the surface
    /// states. `get_fork_messages` is the other, older question ("which user
    /// turns a fork can start from"); this one is the picker's.
    pub async fn load_fork_entries(&mut self) {
        let session_id = self.state.session_id.clone();
        match self.client.fork_entries_for(&session_id).await {
            Ok(value) => {
                self.fork_points = value
                    .get("entries")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| {
                        let id = entry.get("id")?.as_str()?.to_string();
                        let role = match entry.get("role").and_then(Value::as_str) {
                            Some("assistant") => super::state::ForkRole::Assistant,
                            _ => super::state::ForkRole::User,
                        };
                        let text = entry
                            .get("blocks")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .find_map(|block| block.get("text").and_then(Value::as_str))
                            .unwrap_or("")
                            .to_string();
                        Some(super::state::ForkPoint { id, role, text })
                    })
                    .collect();
            }
            Err(error) => {
                self.notice = Some(i18n::render(self.lang, Key::WorkflowFailed, &[&error]))
            }
        }
    }

    /// The `[复制]` control under one message: that message's own text, as the
    /// transcript holds it (markdown source, not the painted ANSI rows).
    ///
    /// The same clipboard road `/copy` takes, one message instead of the last
    /// answer — a prompt can be copied back into the draft, a reply quoted into
    /// another conversation.
    pub fn copy_message(&self, row: usize) -> Result<String, String> {
        self.chat
            .copyable_text_at(row)
            .ok_or_else(|| t(self.lang, Key::CopyEmpty).to_string())
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
                // A worktree the reader entered is the workspace they chose:
                // the next start from somewhere ambient reopens it.
                self.remembered_cwd = Some(path.to_string());
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
                // The workspace the reader picked is the one the next start
                // reopens (see [`Self::take_workspace_to_remember`]).
                self.remembered_cwd = Some(path.to_string());
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
    /// finishes. An image the reader opened from a panel steps through the
    /// folder it sits in.
    pub fn start_preview(&mut self, path: &str) {
        self.read_preview(path, None);
    }

    /// Read a picture the conversation carries — a caption in the transcript, or
    /// the draft's own attachment. The same read, but an image steps through
    /// *this session's* pictures instead of its folder: clipboard captures from
    /// every conversation are written to one temp directory, so a folder step
    /// from a picture pasted here walks into other sessions' drafts.
    pub fn start_picture(&mut self, path: &str) {
        let pictures = self.conversation_pictures();
        self.read_preview(path, Some(pictures));
    }

    /// Every picture this conversation carries, oldest first: the transcript's
    /// own, then the images still attached to the draft.
    fn conversation_pictures(&self) -> Vec<String> {
        let mut pictures: Vec<String> = self
            .chat
            .pictures()
            .map(|picture| picture.path.clone())
            .collect();
        let pending = self.draft.editor.pending();
        pictures.extend(
            pending
                .attachments()
                .iter()
                .filter(|attachment| attachment.kind == "image")
                .map(|attachment| attachment.path.clone()),
        );
        pictures
    }

    /// `pictures` is the list an image steps through, or `None` for the folder
    /// it sits in.
    fn read_preview(&mut self, path: &str, pictures: Option<Vec<String>>) {
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
                match pictures {
                    Some(pictures) => super::files::conversation_siblings(&path, &pictures),
                    None => super::files::image_siblings(&path),
                }
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
    /// The bytes are read on a blocking thread by [`Self::start_picture`]; the
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
                self.start_picture(&attachment.path);
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

    /// Interrupt the run in flight (`ctrl+c`, `/stop`).
    ///
    /// With nothing running this is nothing: the chord means the turn under the
    /// reader, and a turn that has already ended asks the agent to abort an
    /// empty run id, which used to answer the reader with an error notice for
    /// pressing a key at the wrong moment.
    pub async fn abort(&mut self) {
        if !self.client.has_active_run() {
            return;
        }
        if let Err(error) = self.client.abort().await {
            self.notice = Some(format!("Could not stop the run: {error}"));
        } else {
            // The stopped run's calls will never report an end, and the row
            // must not keep counting them. The rate stays: that is what the
            // row shows after any run ends.
            self.activity = Activity::default();
            self.activity_start = None;
            self.run_tools.clear();
            // A stopped reply is settled: the reading level folds it now
            // rather than waiting for an `agent_end` the interrupt may never
            // produce.
            self.chat.end_reply_run();
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
    // `agent_start` is here for the queue: the run that just started may be one
    // this client submitted while the previous turn was streaming, and the
    // agent's next state is what moves that submission from the composer's band
    // into the transcript (`reconcile_queue`).
    //
    // The compaction events are here for the same kind of reason: a checkpoint
    // replaces the prompt the next turn sends, so the context gauge is stale
    // the moment one lands, and `isCompacting` is the flag the row and the
    // composer are drawn from.
    matches!(
        event_type,
        "usage"
            | "agent_start"
            | "agent_end"
            | "compaction_started"
            | "compaction_committed"
            | "compaction_failed"
            | "compaction_unchanged"
    )
}

/// Is this compaction event a standalone (manual) one — `/compact`, rather
/// than the agent trimming its own context inside a run?
///
/// The trigger is the answer the desktop keys its "compaction pending" state
/// on; `phase` is the fallback for an older agent that named only the phase.
fn is_standalone_compaction(data: &Value) -> bool {
    str_field(data, "trigger") == "manual" || str_field(data, "phase") == "standalone"
}

/// One token count from a compaction payload, `None` when it is absent or
/// zero: a pair of zeroes is not a before/after worth printing.
fn token_count(data: &Value, key: &str) -> Option<u64> {
    let value = token_field(data, key);
    (value > 0).then_some(value as u64)
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
    ///
    /// The listing is filtered on the way in (`is_conversation`), so every
    /// reader of the list — the rail, the 工作区 filing, `--continue`, the row a
    /// delete slides into place — walks conversations rather than session
    /// records.
    fn replace(&mut self, rows: Vec<SessionSummary>) -> bool {
        let rows: Vec<SessionSummary> = rows.into_iter().filter(is_conversation).collect();
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

    /// Give one conversation a new name in place.
    ///
    /// The listing is the rail's own copy of what the agent holds, and a rename
    /// is the one write that changes a row without changing its place: the
    /// version moves so the view is re-filed, and an unknown id (a launchpad, or
    /// a conversation the list has not caught up with) changes nothing.
    fn set_name(&mut self, id: &str, name: &str) {
        let Some(row) = self.rows.iter_mut().find(|row| row.id == id) else {
            return;
        };
        if row.session_name.as_deref() == Some(name) {
            return;
        }
        row.session_name = Some(name.to_string());
        self.rev = self.rev.wrapping_add(1);
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
    /// [`Live::pins_rev`] — a pin re-files the 工作区 tab.
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

/// The 工作区 tab's filing: one group per workspace, and the conversations that
/// belong to no workspace at all.
///
/// A conversation is a plain chat when its directory is one of the app's own
/// scratch roots — the agent's default workspace (a session started without a
/// directory) or the per-chat scratch tree the desktop hands out. Everything
/// else is filed under the directory it ran in, named by that directory and
/// identified by its path: two checkouts can share a name.
///
/// A **pinned** conversation is filed here like any other — the lift into 置顶
/// is the rail's (`panels::rows_with`), because 最近 draws the same list with
/// the pin making no difference at all.
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
    // activity — which is also where its heading belongs.
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
                    path,
                    age: row.age.clone(),
                    session_ids: vec![row.id.clone()],
                });
            }
        }
    }
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

/// Decode a picture, scaled down so its longest side is at most `max_px`.
///
/// Shared by the preview box and the transcript's thumbnails: the two differ
/// only in how many pixels they are worth, and a second decoder would be a
/// second place for the size limits to drift.
fn decode_image(
    path: &str,
    max_px: u32,
    lang: Language,
) -> Result<super::image_view::ImagePixels, String> {
    let bytes = read_image_bytes(path, lang)?;
    let image = image::load_from_memory(&bytes).map_err(|error| error.to_string())?;
    let image = if image.width() > max_px || image.height() > max_px {
        image.resize(max_px, max_px, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    Ok(super::image_view::ImagePixels {
        width: rgba.width() as usize,
        height: rgba.height() as usize,
        rgba: rgba.into_raw().into(),
    })
}

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
                let pixels = decode_image(path, MAX_PREVIEW_IMAGE_PX, lang)?;
                Ok(PreviewView::Image {
                    path: path.to_string(),
                    pixels,
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

/// The share of the session's prompt tokens the provider served from its cache.
///
/// `input_tokens` is the session's cumulative `prompt_tokens`, and the cached
/// bytes are *part of* the prompt they were served from — the agent's own
/// counter refuses a block whose cache reads exceed its prompt precisely
/// because of that (`agent/src/types/mod.rs`, `cached_tokens_exceed_prompt`).
/// So the denominator is the reported input alone, which is also what
/// `models::cache_hit_rate` divides by. Adding the reads to it counts the
/// cached bytes twice: the ratio then reads `read / (read + prompt)`, and a
/// session that is 99% cached shows 50% — its ceiling, not its rate.
fn cache_percent(state: &RpcSessionState) -> u8 {
    let read = state.usage.cache_read_tokens.max(0) as u64;
    let input = state.usage.input_tokens.max(0) as u64;
    if input == 0 {
        0
    } else {
        // A provider that contradicts itself (more cached than prompt) reads as
        // fully cached rather than above 100%.
        ((read.min(input) as f64 / input as f64) * 100.0).round() as u8
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

/// Whether a listed session is a conversation the rail paints.
///
/// The agent lists a session from the moment a client creates it, before the
/// first prompt arrives, so a session opened and never spoken in stays behind as
/// a record with no entries, and `queryCount: 0` is how the agent says so. The
/// desktop's rail never shows those — its importer waits for the first message
/// (`host/src/agent_bridge/import.rs`, `has_message`) — and this rail is the same
/// view, so the row is dropped here too. Left in, it has neither a name nor a
/// prompt to be called by, and [`display_name`] falls back to the head of its
/// id: the agent spells ids `YYYYMMDD-HHMMSS-<hash>`, so the rail ends up showing
/// `20261019`, a date that is not a title, on a conversation that was never
/// spoken in.
///
/// Only a count the agent actually sent decides. A peer that predates the field
/// omits it, and an unknown count is not an empty session: hiding on it would
/// drop a whole history from the rail because a client talked to an older agent.
fn is_conversation(session: &SessionSummary) -> bool {
    session.query_count != Some(0)
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

/// The workspace the first conversation of a fresh `future tui` runs in: the
/// directory it was started from when that is a directory the reader chose,
/// otherwise the workspace they chose last time, otherwise none at all (a plain
/// chat).
///
/// `cd project && future tui` opens that project, which is what makes the two
/// commands one gesture. A launch from somewhere *ambient* — the home directory,
/// a temp directory, the filesystem root — is not a choice of workspace: the
/// shell merely happens to sit there, so the remembered workspace comes back
/// instead of a conversation filed under a directory named `tmp`
/// (2026-10-29: sessions created by `future tui` started in `/private/tmp`
/// kept coming back after the reader had picked `future-os` in the previous
/// one). A launch from one of the app's own scratch directories is a chat, and
/// so is a launch directory that cannot be resolved.
///
/// The remembered path is used only while it still exists: a removed worktree
/// must not become the next conversation's directory.
fn launch_workspace(launched: Option<&str>, remembered: Option<&str>) -> Option<String> {
    let launched = launched
        .map(str::trim)
        .filter(|path| !path.is_empty() && !is_chat_directory(path));
    if let Some(path) = launched.filter(|path| !is_ambient_directory(path)) {
        return Some(path.to_string());
    }
    let remembered = remembered
        .map(str::trim)
        .filter(|path| !path.is_empty() && !is_chat_directory(path))
        .filter(|path| std::path::Path::new(path).is_dir());
    remembered.map(str::to_string)
}

/// Is this directory one a shell happens to sit in rather than a workspace the
/// reader chose — their home, the filesystem root, or a temp directory?
///
/// Home and temp are compared with their contents in mind in opposite
/// directions: a project *under* the home directory is an ordinary workspace
/// (`~/code/app`), while everything under a temp root is scratch
/// (`/var/folders/…/T/…`, `/tmp/x`).
fn is_ambient_directory(path: &str) -> bool {
    let trimmed = path.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return true;
    }
    let path = std::path::Path::new(trimmed);
    // `/` and `C:\` have no parent: the filesystem root itself.
    if path.parent().is_none() {
        return true;
    }
    if let Some(home) = crate::home::home_dir() {
        if same_existing_path(path, &home) {
            return true;
        }
    }
    if under_directory(path, &std::env::temp_dir()) {
        return true;
    }
    // The temp root the system names is per-user on macOS (`$TMPDIR`), while a
    // shell's `/tmp` is the shared one; both are ambient, and on macOS the two
    // spellings of the shared one are the same directory.
    ["/tmp", "/private/tmp"]
        .into_iter()
        .any(|fixed| under_directory(path, std::path::Path::new(fixed)))
}

/// Is `path` `root` or something inside it, comparing the resolved directories
/// when both exist (`/tmp` and `/private/tmp` are one directory on macOS)?
fn under_directory(path: &std::path::Path, root: &std::path::Path) -> bool {
    if path.strip_prefix(root).is_ok() {
        return true;
    }
    match (std::fs::canonicalize(path), std::fs::canonicalize(root)) {
        (Ok(path), Ok(root)) => path.strip_prefix(&root).is_ok(),
        _ => false,
    }
}

/// [`under_directory`] for one directory rather than a tree: a project under the
/// home directory is a workspace, the home directory itself is not.
fn same_existing_path(path: &std::path::Path, other: &std::path::Path) -> bool {
    if path == other {
        return true;
    }
    match (std::fs::canonicalize(path), std::fs::canonicalize(other)) {
        (Ok(path), Ok(other)) => path == other,
        _ => false,
    }
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
        // The transcript's own process rows are chrome, not model output: they
        // are worded in the interface language (`工具调用 61 次 · 思考 30 次`).
        self.chat.set_fold_words(fold_words(self.lang));
        self.lang
    }
}

/// The words a transcript's rows are drawn in (see `FoldWords`): the fold
/// summaries and the two controls under every message.
fn fold_words(lang: Language) -> FoldWords {
    FoldWords {
        tool_call: t(lang, Key::ChatFoldToolCall),
        tool_calls: t(lang, Key::ChatFoldToolCalls),
        thought: t(lang, Key::ChatFoldThought),
        thoughts: t(lang, Key::ChatFoldThoughts),
        thinking: t(lang, Key::ChatThinkingLabel),
        process: t(lang, Key::ChatProcessLabel),
        copy: t(lang, Key::ChatCopyChip),
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
    use serde_json::json;

    /// Run `future` on a thread with a big stack.
    ///
    /// `Live::apply(WbAction::Command(..))` is one async fn over every verb the
    /// adapter answers, so a debug build lays all of their locals into one frame
    /// — about 9 MB, more than the 8 MB a `cargo test` thread gets, and the
    /// abort takes the whole test binary with it. A release build's frame is a
    /// fraction of that, which is why the shipped surface never notices.
    fn with_big_stack<T: Send + 'static>(
        body: impl FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = T>>> + Send + 'static,
    ) -> T {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(move || {
                // The future is *built* on the big stack, so nothing has to be
                // `Send`: the surface holds `Rc`s and a client the runtime owns.
                let future = body();
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("a test runtime")
                    .block_on(future)
            })
            .expect("a test thread")
            .join()
            .expect("the test body did not panic")
    }

    use crate::components::chat_area::{ChatMessage, ChatRole, RowAction};
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

    /// A submission behind a running turn joins the agent's queue, and the
    /// transcript does not pretend it was sent: the card is held back until the
    /// agent starts its run (`reconcile_queue`), and the composer's band is what
    /// shows it meanwhile.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn a_submit_behind_a_running_turn_waits_in_the_queue() {
        let _guard = crate::test_env::lock();
        let home = tempfile::tempdir().unwrap();
        let _home = crate::test_env::EnvVarGuard::set("FUTURE_HOME", home.path());
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            (
                "get_state",
                r#"{"sessionId":"s1","isStreaming":true,"activeRun":{"runId":"r-1","epoch":1,"state":"running","lastEventIdx":0}}"#,
            ),
            ("get_session_entries", r#"{"entries":[]}"#),
            (
                "prompt",
                // The prompt ack is the one payload the wire keeps snake_case
                // (`payloads_ext::RunAck`).
                r#"{"run_id":"r-2","run_epoch":0,"accepted_state":"queued","queue_position":1}"#,
            ),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock.clone()).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        assert_eq!(live.state.session_id, "s1");
        // The state the agent answers with once it has accepted the submission:
        // the run is queued behind r-1, in the agent's own words.
        mock.data_by_type.lock().unwrap().insert(
            "get_state".into(),
            r#"{"sessionId":"s1","isStreaming":true,"activeRun":{"runId":"r-1","epoch":1,"state":"running","lastEventIdx":0},"queuedRuns":[{"runId":"r-2","runSequence":2,"clientRequestId":"c-2","state":"queued","queuePosition":1,"acceptedAt":"now","displayText":"second"}],"queuedCount":1}"#.into(),
        );
        live.set_draft("second".into(), 6);
        live.apply(WbAction::Submit("second".into())).await;

        let painted = live.chat.render_all(80).join("\n");
        assert!(
            !painted.contains("second"),
            "a queued prompt must not read as sent: {:?}\n{painted}",
            live.notice
        );
        assert_eq!(live.state.queued_runs.len(), 1, "the agent lists the run");
        assert_eq!(
            live.chat.queued_run_ids().len(),
            1,
            "the card is bound to it"
        );
        assert_eq!(
            live.data(&frame()).queue.len(),
            1,
            "the band above the composer lists it"
        );
    }

    /// A burst queued behind a running turn goes out as **one** message: the
    /// agent folds the prompts waiting at the boundary into the run ahead of
    /// them and answers them together, so the card that runs carries every text
    /// and the absorbed ones leave the band without leaving a gap in the
    /// transcript.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn a_burst_of_queued_submissions_goes_out_as_one_message() {
        let _guard = crate::test_env::lock();
        let home = tempfile::tempdir().unwrap();
        let _home = crate::test_env::EnvVarGuard::set("FUTURE_HOME", home.path());
        let busy = r#"{"sessionId":"s1","isStreaming":true,"activeRun":{"runId":"r-1","epoch":1,"state":"running","lastEventIdx":0}}"#;
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("get_state", busy),
            ("get_session_entries", r#"{"entries":[]}"#),
            (
                "prompt",
                r#"{"run_id":"r-2","run_epoch":0,"accepted_state":"queued","queue_position":1}"#,
            ),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock.clone()).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");

        // The first submission joins the queue …
        mock.data_by_type.lock().unwrap().insert(
            "get_state".into(),
            r#"{"sessionId":"s1","isStreaming":true,"activeRun":{"runId":"r-1","epoch":1,"state":"running","lastEventIdx":0},"queuedRuns":[{"runId":"r-2","runSequence":2,"clientRequestId":"c-2","state":"queued","queuePosition":1,"acceptedAt":"now","displayText":"first"}],"queuedCount":1}"#.into(),
        );
        live.set_draft("first".into(), 5);
        live.apply(WbAction::Submit("first".into())).await;

        // … and the second behind it.
        mock.data_by_type.lock().unwrap().insert(
            "get_state".into(),
            r#"{"sessionId":"s1","isStreaming":true,"activeRun":{"runId":"r-1","epoch":1,"state":"running","lastEventIdx":0},"queuedRuns":[{"runId":"r-2","runSequence":2,"clientRequestId":"c-2","state":"queued","queuePosition":1,"acceptedAt":"now","displayText":"first"},{"runId":"r-3","runSequence":3,"clientRequestId":"c-3","state":"queued","queuePosition":2,"acceptedAt":"now","displayText":"second"}],"queuedCount":2}"#.into(),
        );
        mock.data_by_type.lock().unwrap().insert(
            "prompt".into(),
            r#"{"run_id":"r-3","run_epoch":0,"accepted_state":"queued","queue_position":2}"#.into(),
        );
        live.set_draft("second".into(), 6);
        live.apply(WbAction::Submit("second".into())).await;

        // Both are queue rows above the composer; neither is in the transcript.
        assert_eq!(live.data(&frame()).queue.len(), 2);
        let painted = live.chat.render_all(80).join("\n");
        assert!(!painted.contains("first"), "{painted}");
        assert!(!painted.contains("second"), "{painted}");

        // The running turn ends: r-2 starts carrying both texts and r-3 comes
        // back as a terminal `merged` acknowledgement.
        mock.data_by_type.lock().unwrap().insert(
            "get_state".into(),
            r#"{"sessionId":"s1","activeRun":{"runId":"r-2","epoch":1,"state":"running","lastEventIdx":0},"recentTerminalAcks":[{"runId":"r-3","runSequence":3,"clientRequestId":"c-3","state":"terminal","reason":"merged"}]}"#.into(),
        );
        live.refresh_state().await;

        assert_eq!(live.data(&frame()).queue.len(), 0, "the band is empty");
        let painted = live.chat.render_all(80).join("\n");
        assert!(painted.contains("first"), "{painted}");
        assert!(
            painted.contains("second"),
            "the folded text rides the card that ran: {painted}"
        );
    }

    /// The queue is the agent's, and the transcript follows it: a submission
    /// leaves the band when its run starts, and leaves the transcript entirely
    /// when the agent says it never ran (cancelled, merged, superseded).
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn reconcile_queue_moves_a_submission_out_of_the_band_and_keeps_it_honest() {
        let _guard = crate::test_env::lock();
        let home = tempfile::tempdir().unwrap();
        let _home = crate::test_env::EnvVarGuard::set("FUTURE_HOME", home.path());
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("get_state", r#"{"sessionId":"s1"}"#),
            ("get_session_entries", r#"{"entries":[]}"#),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");

        // Still queued: the card stays out of the transcript.
        live.chat.add_message(ChatMessage::new(
            "u-1".into(),
            ChatRole::User,
            "started later",
        ));
        live.chat
            .bind_user_run("u-1", "r-2", RunState::Queued, Some(1));
        live.state.queued_runs = vec![crate::rpc::types::QueuedRunState {
            run_id: "r-2".into(),
            run_sequence: 2,
            client_request_id: "c-2".into(),
            state: "queued".into(),
            queue_position: 1,
            accepted_at: String::new(),
            display_text: "started later".into(),
        }];
        live.reconcile_queue();
        assert!(!live
            .chat
            .render_all(80)
            .iter()
            .any(|row| row.contains("started later")));

        // Started: the agent made it the active run, so the card belongs in the
        // conversation again.
        live.state.queued_runs.clear();
        live.state.active_run = Some(crate::rpc::types::ActiveRunState {
            run_id: "r-2".into(),
            epoch: 1,
            run_sequence: Some(2),
            state: "running".into(),
            last_event_idx: 0,
        });
        live.reconcile_queue();
        assert!(live
            .chat
            .render_all(80)
            .iter()
            .any(|row| row.contains("started later")));

        // Cancelled before it ran: nothing was sent, so nothing stays behind.
        live.chat
            .add_message(ChatMessage::new("u-2".into(), ChatRole::User, "never sent"));
        live.chat
            .bind_user_run("u-2", "r-3", RunState::Queued, Some(1));
        live.state.active_run = None;
        live.state.recent_terminal_acks = vec![crate::rpc::types::RecentTerminalAck {
            run_id: "r-3".into(),
            run_sequence: 3,
            client_request_id: "c-3".into(),
            state: "cancelled".into(),
            reason: "cancelled".into(),
        }];
        live.reconcile_queue();
        assert!(
            live.chat.queued_run_ids().is_empty(),
            "the card left the queue"
        );
        assert!(
            !live
                .chat
                .render_all(80)
                .iter()
                .any(|row| row.contains("never sent")),
            "a cancelled submission keeps no card"
        );

        // Gone with no acknowledgement either way: it ran unseen, so it is
        // shown rather than hidden behind a queue row that no longer exists.
        live.chat.add_message(ChatMessage::new(
            "u-3".into(),
            ChatRole::User,
            "ran quickly",
        ));
        live.chat
            .bind_user_run("u-3", "r-4", RunState::Queued, Some(1));
        live.state.recent_terminal_acks.clear();
        live.reconcile_queue();
        assert!(live.chat.queued_run_ids().is_empty());
        assert!(live
            .chat
            .render_all(80)
            .iter()
            .any(|row| row.contains("ran quickly")));
    }

    /// The interrupt chord means the turn in flight (`ctrl+c`). With nothing
    /// running it is nothing at all: no `abort` reaches the agent, and the
    /// reader is not shown an error for pressing a key at the wrong moment —
    /// which is exactly what an unconditional abort did with an empty run id.
    #[tokio::test]
    async fn stopping_with_nothing_running_asks_the_agent_nothing() {
        let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("get_state", r#"{"sessionId":"s1"}"#),
            ("get_session_entries", r#"{"entries":[]}"#),
            ("abort", "{}"),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        assert_eq!(live.state.session_id, "s1", "get_state installs the state");
        assert!(!live.client.has_active_run(), "the state named no run");
        live.abort().await;
        assert!(live.notice.is_none(), "{:?}", live.notice);
        let commands = seen.lock().unwrap().clone();
        assert!(
            !commands.iter().any(|command| command == "abort"),
            "an idle interrupt must not reach the agent, saw {commands:?}"
        );
    }

    /// …and with a turn in flight the same call does reach it: `ctrl+c` while a
    /// reply streams is the shell's interrupt, and it has to land.
    #[tokio::test]
    async fn stopping_a_running_turn_asks_the_agent_to_abort() {
        let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            (
                "get_state",
                r#"{"sessionId":"s1","activeRun":{"runId":"r-1","epoch":1,"state":"running","lastEventIdx":0}}"#,
            ),
            ("get_session_entries", r#"{"entries":[]}"#),
            ("abort", "{}"),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        assert_eq!(live.state.session_id, "s1", "get_state installs the state");
        assert!(live.client.has_active_run(), "the state named a live run");
        live.abort().await;
        assert!(live.notice.is_none(), "{:?}", live.notice);
        let commands = seen.lock().unwrap().clone();
        assert!(
            commands.iter().any(|command| command == "abort"),
            "a live interrupt must reach the agent, saw {commands:?}"
        );
    }

    /// Opening a transcript row is the chat's own state: the driver rebuilds
    /// the transcript and moves the revision (the rows, and the link spans over
    /// them, changed), and nothing reaches the agent. `tab` + `enter` on a
    /// folded reply, a burst, a call or a reasoning marker all land here.
    #[tokio::test]
    async fn opening_a_chat_row_repaints_without_asking_the_agent() {
        let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[(
            "get_state",
            r#"{"sessionId":"s1"}"#,
        )]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s1"), false, None)
            .await
            .expect("attach to the test agent");

        // One settled turn: a call, then the answer it produced.
        live.chat
            .add_message(ChatMessage::new("u".into(), ChatRole::User, "look"));
        live.chat
            .add_tool_start("c1", "read", Some(r#"{"path":"/a.rs"}"#.into()));
        live.chat.finish_tool("c1", None, false);
        live.chat.add_message(ChatMessage::new(
            "a".into(),
            ChatRole::Assistant,
            "the answer",
        ));
        let folded = plain_rows(&live.chat.render_all(100));
        assert!(
            !folded.contains("read /a.rs"),
            "the work is folded: {folded}"
        );

        let row = live
            .chat
            .row_stops()
            .into_iter()
            .find(|stop| matches!(stop.action, RowAction::Process { .. }))
            .expect("the fold draws a row")
            .row;
        let before = seen.lock().unwrap().len();
        let revision = live.revision;
        live.apply(WbAction::ToggleChatRow(row)).await;

        assert!(
            live.revision > revision,
            "the revision moves, so the span cache is rebuilt"
        );
        assert_eq!(
            seen.lock().unwrap().len(),
            before,
            "opening a row reaches the agent with nothing"
        );
        let open = plain_rows(&live.chat.render_all(100));
        assert!(open.contains("read /a.rs"), "the call is back: {open}");
    }

    /// Rows as a reader sees them: ANSI stripped, one per line.
    fn plain_rows(rows: &[String]) -> String {
        rows.iter()
            .map(|row| crate::utils::strip_ansi_codes(row))
            .collect::<Vec<_>>()
            .join("\n")
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
    /// the level the old `ctrl+e` chord used to write.
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

    /// A picture the conversation named steps through this session's pictures —
    /// not the folder it sits in, which every conversation's pasted screenshots
    /// share.
    #[tokio::test]
    async fn a_conversation_picture_steps_through_this_sessions_pictures() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[(
            "get_state",
            r#"{"sessionId":"s-1"}"#,
        )]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for name in ["a.png", "b.png", "other-session.png"] {
            let path = dir.path().join(name);
            std::fs::write(&path, png_bytes()).unwrap();
            paths.push(path.to_string_lossy().to_string());
        }
        // Two of the three belong to this conversation; the third is another
        // session's capture that happens to sit in the same directory.
        for path in &paths[..2] {
            let mut message = crate::components::chat_area::ChatMessage::new(
                format!("m-{path}"),
                ChatRole::User,
                "",
            );
            message
                .pictures
                .push(crate::components::chat_area::ChatPicture {
                    path: path.clone(),
                    name: String::new(),
                });
            live.chat.add_message(message);
        }

        live.start_picture(&paths[0]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !live.poll_preview() {
            assert!(std::time::Instant::now() < deadline, "preview never landed");
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let (siblings, index) = live.take_preview_siblings().expect("picture neighbours");
        assert_eq!(
            siblings,
            paths[..2].to_vec(),
            "the conversation's own pictures, in its order"
        );
        assert_eq!(index, 0);

        // The same file opened from a panel still steps through its folder: that
        // is a different question, and the panel's answer is the directory.
        live.start_preview(&paths[0]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !live.poll_preview() {
            assert!(std::time::Instant::now() < deadline, "preview never landed");
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let (folder, _) = live.take_preview_siblings().expect("folder neighbours");
        assert_eq!(folder.len(), 3, "the directory holds all three: {folder:?}");
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
            // The field a peer may omit: these fixtures stand for rows an agent
            // listed without the count, which the rail keeps (see
            // `is_conversation`).
            query_count: None,
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
    #[allow(clippy::await_holding_lock)] // FUTURE_HOME is process-global and must stay pinned across awaits.
    async fn a_fresh_launchpad_names_the_product_defaults() {
        // The expected chat directory is read from the process environment, as
        // the screen reads it: a peer test's swap must not answer for it.
        let _guard = crate::test_env::lock();
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

    /// A model and level the reader chose on an earlier run are remembered as
    /// the terminal UI's default: a fresh launchpad names them over both the
    /// agent's `isDefault` model and the conversation `/new` left behind, so
    /// the next session starts where the reader left off.
    #[tokio::test]
    async fn a_stored_default_model_is_the_launchpad_default() {
        // The conversation the client came from runs a different model and
        // level, and the agent flags `gpt-9` as its default — neither may win
        // over the reader's remembered choice.
        let state = r#"{"sessionId":"s-1","cwd":"/work/alpha","model":"future/old","thinkingLevel":"high"}"#;
        let models = r#"{"models":[
            {"id":"old","label":"old","provider":"future","thinkingLevel":"default",
             "thinkingLevels":["default","high"],"defaultThinkingLevel":"high"},
            {"id":"deepseek-flash","label":"deepseek-flash","provider":"future",
             "thinkingLevel":"default","thinkingLevels":["default","high"],
             "defaultThinkingLevel":"default"},
            {"id":"gpt-9","label":"gpt-9","provider":"future","isDefault":true,
             "thinkingLevel":"default","thinkingLevels":["default","high"],
             "defaultThinkingLevel":"default"}
        ]}"#;
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("get_state", state),
            ("list_models", models),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let settings = crate::settings::TuiSettings::from_json(&serde_json::json!({
            "defaultModel": "future/deepseek-flash",
            "defaultThinkingLevel": "high"
        }));
        let (mut live, _events) = Live::attach(&addr, None, false, Some(&settings))
            .await
            .expect("attach to the test agent");
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
        assert_eq!(data.model, "future/deepseek-flash", "the remembered model");
        assert_eq!(data.effort, "high", "the remembered level");
    }

    /// The rail, the `ctrl+r` list and the 工作区 tab name a workspace by its
    /// own name; the path is what the *group* is filed under, not what the row
    /// has room for. On the launchpad the 工作区 chip stages the directory the
    /// next session is created in.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // FUTURE_HOME is process-global and must stay pinned across awaits.
    async fn the_rail_names_workspaces_and_the_chip_stages_the_next_one() {
        // The chat row is named from the process environment, so this test
        // reads it under the same lock as a peer test's swap of `FUTURE_HOME`.
        let _guard = crate::test_env::lock();
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

        live.stage_launch_workspace(None);
        let started_from = std::env::current_dir().unwrap();
        assert_eq!(
            live.pending_cwd.as_deref(),
            Some(started_from.to_string_lossy().as_ref()),
            "the first conversation runs where the TUI was opened"
        );
        // …and that staging is what `settings.json` is told to remember, so the
        // next start reopens it.
        assert_eq!(
            live.take_workspace_to_remember().as_deref(),
            Some(started_from.to_string_lossy().as_ref())
        );
        // A session already attached is not re-staged.
        live.launchpad = false;
        live.state.session_id = "s-1".into();
        live.pending_cwd = None;
        live.stage_launch_workspace(Some("/work/alpha"));
        assert_eq!(live.pending_cwd, None, "an attached session keeps its cwd");
    }

    /// A launch from somewhere ambient is not a choice of workspace: the one
    /// picked last time comes back, which is what stops `future tui` typed in a
    /// scratch directory from filing the next conversations under it.
    #[test]
    fn a_launch_from_an_ambient_directory_reopens_the_remembered_workspace() {
        let remembered = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let remembered = remembered.path().to_string_lossy();
        let scratch = scratch.path().to_string_lossy();
        assert!(
            is_ambient_directory(&scratch),
            "a directory under the temp root is ambient: {scratch}"
        );
        assert_eq!(
            launch_workspace(Some(&scratch), Some(&remembered)).as_deref(),
            Some(remembered.as_ref())
        );
    }

    /// `cd project && future tui` is still that project, whatever was picked
    /// last time.
    #[test]
    fn a_project_directory_wins_over_the_remembered_workspace() {
        let remembered = tempfile::tempdir().unwrap();
        let remembered = remembered.path().to_string_lossy();
        assert_eq!(
            launch_workspace(Some("/work/future-os"), Some(&remembered)).as_deref(),
            Some("/work/future-os")
        );
    }

    /// A workspace that is gone (a removed worktree) is skipped instead of
    /// becoming the next conversation's directory.
    #[test]
    fn a_remembered_workspace_that_no_longer_exists_is_not_staged() {
        let scratch = tempfile::tempdir().unwrap();
        let scratch = scratch.path().to_string_lossy();
        let missing = std::env::temp_dir().join("future-tui-workspace-that-is-gone");
        let missing = missing.to_string_lossy();
        assert_eq!(launch_workspace(Some(&scratch), Some(&missing)), None);
    }

    /// Nothing remembered yet, and the shell sat somewhere ambient: the first
    /// conversation is a plain chat rather than a workspace named `tmp` — the
    /// one launch that still lands in the scratch directory, and the one the
    /// remembered workspace makes unnecessary afterwards.
    #[test]
    fn a_first_launch_from_a_temp_directory_is_a_chat_not_a_workspace() {
        let scratch = tempfile::tempdir().unwrap();
        let scratch = scratch.path().to_string_lossy();
        assert_eq!(launch_workspace(Some(&scratch), None), None);
    }

    /// One of the app's own scratch directories is a chat, and a chat is never
    /// remembered as a workspace.
    #[test]
    fn the_apps_own_scratch_directory_is_never_the_launch_workspace() {
        let _guard = crate::test_env::lock();
        let remembered = tempfile::tempdir().unwrap();
        let remembered = remembered.path().to_string_lossy();
        let chat = chat_workspace();
        assert_eq!(
            launch_workspace(Some(&chat), Some(&remembered)).as_deref(),
            Some(remembered.as_ref())
        );
        assert_eq!(launch_workspace(Some(&chat), None), None);
    }

    /// Ambient is about *where the shell sits*, not where the reader works: the
    /// home directory and the temp tree are ambient, a project under either is
    /// not.
    #[test]
    fn ambient_directories_are_the_ones_a_shell_sits_in() {
        // Every path below is derived from `HOME`/`FUTURE_HOME` at read time,
        // so a peer test's `EnvVarGuard` must be resting while this runs: a
        // scratch tree under the swapped root is ambient, and the last
        // assertion below would fail for a directory this test never chose.
        let _guard = crate::test_env::lock();
        assert!(is_ambient_directory("/"), "the filesystem root");
        assert!(is_ambient_directory("/tmp"));
        assert!(is_ambient_directory("/tmp/"), "a trailing separator");
        assert!(is_ambient_directory("/private/tmp"), "the macOS spelling");
        assert!(is_ambient_directory("/tmp/scratch/notes"));
        let temp = std::env::temp_dir();
        assert!(is_ambient_directory(&temp.to_string_lossy()));
        assert!(is_ambient_directory(&temp.join("child").to_string_lossy()));
        if let Some(home) = crate::home::home_dir() {
            let home = home.to_string_lossy().to_string();
            assert!(is_ambient_directory(&home), "{home}");
            assert!(
                !is_ambient_directory(&format!("{home}/code/app")),
                "a project under the home directory is a workspace"
            );
        }
        assert!(!is_ambient_directory("/work/future-os"));
        assert!(
            !is_ambient_directory(&chat_workspace()),
            "the app's scratch tree is not ambient — it is a chat, which is a\n             different rule (`is_chat_directory`)"
        );
    }

    /// What the driver writes is a choice, not a directory the reader merely
    /// visited: the launchpad's staging and an explicit workspace pick, not the
    /// workspace of a conversation they switched to.
    #[tokio::test]
    async fn only_a_chosen_workspace_is_remembered() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");

        // Nothing staged, no session: nothing to remember.
        assert_eq!(live.take_workspace_to_remember(), None);
        // A chat is not a workspace.
        live.pending_cwd = None;
        assert_eq!(live.take_workspace_to_remember(), None);
        // The launchpad's staging is.
        live.pending_cwd = Some("/work/alpha".into());
        assert_eq!(
            live.take_workspace_to_remember().as_deref(),
            Some("/work/alpha")
        );
        assert_eq!(
            live.take_workspace_to_remember().as_deref(),
            Some("/work/alpha"),
            "still staged: the chip keeps naming it, and the driver's compare\n             against the file is what makes the repeat a no-op"
        );
        // A session attached with a cwd of its own is not a choice — the reader
        // merely opened a conversation — so switching does not rewrite it.
        live.launchpad = false;
        live.state.session_id = "s-1".into();
        live.state.cwd = Some("/work/legacy".into());
        assert_eq!(live.take_workspace_to_remember(), None);
    }

    /// `/new` starts the next conversation where the one on screen ran: a
    /// project is inherited, a chat is not (the next message mints a fresh chat
    /// directory of its own).
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // FUTURE_HOME is process-global and must stay pinned across awaits.
    async fn a_new_conversation_keeps_the_workspace_it_left() {
        // `chat_workspace()` is read here and classified by the launchpad: a
        // swap in between would make the two name different directories.
        let _guard = crate::test_env::lock();
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

    /// The rail's listing is *started* on the clock and drained a tick later,
    /// never awaited inside the loop: a slow agent costs a frame's worth of
    /// staleness instead of a frozen surface (the freeze this replaced: 6.9 s
    /// of no keys and no repaint, every five seconds).
    #[tokio::test]
    async fn a_slow_listing_is_started_and_drained_instead_of_awaited() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_slow(
            &[("list_sessions", r#"{"sessions":[]}"#)],
            250,
        );
        let table = mock.data_by_type.clone();
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the slow test agent");
        assert!(live.sessions.is_empty(), "the attach read an empty list");

        // The answer this listing will carry, moved before it is asked for.
        table.lock().unwrap().insert(
            "list_sessions".into(),
            r#"{"sessions":[{"id":"s-late","cwd":"/work/alpha","updatedAtMs":1700000000000,"createdAtMs":1700000000000,"model":"m","sessionName":"late"}]}"#.into(),
        );
        let started = std::time::Instant::now();
        assert!(live.begin_sessions_refresh(), "the clock starts a listing");
        assert!(
            started.elapsed() < std::time::Duration::from_millis(150),
            "starting a listing does not wait for the agent ({:?})",
            started.elapsed()
        );
        assert!(!live.begin_sessions_refresh(), "one listing is already out");
        assert_eq!(live.poll_sessions(), None, "nothing has landed yet");

        let landed = loop {
            if let Some(changed) = live.poll_sessions() {
                break changed;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        assert!(landed, "the listing moved the rail");
        let data = live.data(&frame());
        assert!(
            data.sessions.iter().any(|row| row.id == "s-late"),
            "the drained listing is on screen: {:?}",
            data.sessions.iter().map(|row| &row.id).collect::<Vec<_>>()
        );
        assert!(
            live.begin_sessions_refresh(),
            "the slot frees once the listing is drained"
        );
        live.client.disconnect();
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

    /// A session the agent lists before its first prompt — opened, never spoken
    /// in — is not a conversation, and the rail must not paint it.
    ///
    /// Left in, it has no name and no prompt to be called by, so the title falls
    /// back to the head of its id; ids are spelled `YYYYMMDD-HHMMSS-<hash>`, and
    /// the rail showed `20261019` for a conversation nobody ever had. The desktop
    /// never listed these (its importer waits for the first message), so dropping
    /// the row is also what makes the two surfaces agree.
    #[test]
    fn a_session_with_no_message_never_reaches_the_rail() {
        let mut empty = row("20261019-211257-845dff47", None, "/private/tmp");
        empty.query_count = Some(0);
        let mut spoken = row("20261019-171039-1f02dedd", None, "/private/tmp");
        spoken.first_message = Some("why does the rail show a date".into());
        spoken.query_count = Some(2);
        let mut named = row("s-named", Some("会话总管"), "/work/alpha");
        named.query_count = Some(1);

        let mut list = SessionList::default();
        assert!(list.replace(vec![empty, spoken, named]));
        let ids: Vec<&str> = list.iter().map(|session| session.id.as_str()).collect();
        assert_eq!(
            ids,
            ["20261019-171039-1f02dedd", "s-named"],
            "the session that was never spoken in is dropped"
        );
        // What the rail paints carries no id head: the fallback is still there
        // for a row that has no title yet, but no such row survives the filter.
        let titles: Vec<String> = list.iter().map(display_name).collect();
        assert_eq!(
            titles,
            ["why does the rail show a date", "会话总管"],
            "no date-headed row is left to paint"
        );

        // A session given a name but never spoken in is no conversation either:
        // the agent lists it, the desktop never imports it, and opening it shows
        // an empty transcript.
        let mut labelled = row("s-labelled", Some("hand-named"), "/work/alpha");
        labelled.query_count = Some(0);
        assert!(list.replace(vec![labelled]));
        assert!(list.is_empty(), "nothing to paint");

        // An agent that predates `queryCount` sends no count at all. Unknown is
        // not empty: its rows must stay, or a client that talked to an older
        // agent would lose every conversation it has.
        let legacy_empty = row("0123456789abcdef", None, "/tmp");
        assert_eq!(legacy_empty.query_count, None, "the fixture carries none");
        let mut legacy_spoken = row("s-legacy", None, "/work/alpha");
        legacy_spoken.first_message = Some("look at this test".into());
        assert!(list.replace(vec![legacy_empty, legacy_spoken]));
        let ids: Vec<&str> = list.iter().map(|session| session.id.as_str()).collect();
        assert_eq!(ids, ["0123456789abcdef", "s-legacy"], "no count, no hiding");

        // A listing that only moves an invisible row changed nothing the rail
        // paints, so no frame is owed.
        let mut quiet = row("s-quiet", Some("quiet"), "/work/alpha");
        quiet.first_message = Some("look at this test".into());
        assert!(list.replace(vec![quiet.clone()]));
        let mut hidden = row("s-hidden", None, "/private/tmp");
        hidden.query_count = Some(0);
        assert!(
            !list.replace(vec![quiet, hidden]),
            "a row the rail never paints cannot owe it a frame"
        );
    }

    #[test]
    fn a_workspace_is_shown_home_relative() {
        let _guard = crate::test_env::lock();
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
        // A cached read is part of the prompt it was served from, so the rate
        // is the reads over the prompt — not over the prompt plus the reads.
        state.usage.input_tokens = 100;
        state.usage.cache_read_tokens = 96;
        assert_eq!(cache_percent(&state), 96);
        // The old formula capped every heavily cached session at 50%: this
        // session reads 99% here and read 50% there (990k / 1.99M).
        state.usage.input_tokens = 1_000_000;
        state.usage.cache_read_tokens = 990_000;
        assert_eq!(cache_percent(&state), 99);
        // Reads a provider reports beyond its own prompt read as fully cached.
        state.usage.input_tokens = 10;
        state.usage.cache_read_tokens = 90;
        assert_eq!(cache_percent(&state), 100);
    }

    /// The 工作区 tab's filing. A conversation is filed under the directory it
    /// ran in; two checkouts that share a name are two groups (the path is the
    /// identity); a scratch directory is a plain chat. A pin is the rail's
    /// business, so the filing lists the conversation either way.
    #[test]
    fn conversations_are_filed_by_the_directory_they_ran_in() {
        // The scratch tree is read from the process environment twice — once
        // for the fixture, once by `ChatRoots::detect()` — and only a stable
        // environment makes the two the same directory.
        let _guard = crate::test_env::lock();
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
                pinned: session.id == "a",
                ..SessionRow::default()
            })
            .collect();
        let (groups, chats) = file_sessions(&sessions, &rows, &ChatRoots::detect());

        // A pin is the rail's own lift, not the filing's: the group keeps the
        // conversation even when 工作区 draws it in 置顶.
        assert!(groups
            .iter()
            .any(|group| group.session_ids.contains(&"a".to_string())));

        // A trailing slash is not a different workspace: `a` and `b` are one
        // group, newest first, and the group's age is that newest one.
        let main = groups
            .iter()
            .find(|group| group.path == "/work/future-os")
            .expect("the main checkout");
        assert_eq!(main.session_ids, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(main.age, "0h");
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
        let (groups, chats) = file_sessions(&sessions, &rows, &ChatRoots::detect());
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
        // The roots and the directories compared against them are both
        // read from the process environment, so they must be read apart from a
        // peer test's swap of it.
        let _guard = crate::test_env::lock();
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
        live.set_session_pinned("s3", true);
        live.data(&frame);
        assert!(
            live.session_view_builds() > before,
            "a pinned conversation must re-file the tab"
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
        let _guard = crate::test_env::lock();
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
        // The cache rate is the reads over the prompt, which already contains
        // them: `inputTokens` is the provider's `prompt_tokens` (800 / 900 = 89),
        // not an uncached remainder.
        assert_eq!(cache_percent(&state), 89);

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

    /// `/compact` takes the session over until its checkpoint lands.
    ///
    /// The acknowledgement names the operation, not the result: the agent
    /// summarises on a worker, so the surface has to show that something is
    /// happening, refuse the reader's prompt (the agent answers `session_busy`
    /// until then), and hand back the pair of token counts on the event that
    /// ends it — the same pair the stored history's divider carries.
    #[tokio::test]
    async fn a_manual_compaction_holds_the_session_until_it_commits() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");

        live.handle_event(event(
            "compaction_started",
            serde_json::json!({"operation_id": "cmp-1", "trigger": "manual", "phase": "standalone"}),
        ));
        assert!(live.compacting(), "the session is held");
        assert_eq!(live.activity_kind(), ActivityKind::Compacting);
        assert!(live.data(&frame()).compacting);
        assert_eq!(
            live.notice.as_deref(),
            Some(t(live.language(), Key::NoticeCompacting))
        );

        // A prompt is left in the draft with the reason on the notice row: the
        // agent would refuse it, and the text is the reader's.
        live.set_draft("keep me".into(), 7);
        live.apply(WbAction::Submit("keep me".into())).await;
        assert_eq!(live.draft.text, "keep me", "the draft survives");
        assert_eq!(
            live.notice.as_deref(),
            Some(t(live.language(), Key::ComposerCompacting))
        );

        live.handle_event(event(
            "compaction_committed",
            serde_json::json!({
                "operation_id": "cmp-1",
                "trigger": "manual",
                "phase": "standalone",
                "tokens_before": 62203,
                "tokens_after": 33281,
            }),
        ));
        assert!(!live.compacting(), "the session is given back");
        let expected = i18n::render(
            live.language(),
            Key::CompactDoneTokens,
            &[&62203u64, &33281u64],
        );
        assert_eq!(live.notice.as_deref(), Some(expected.as_str()));
        // The transcript says where the conversation was cut, in the same words
        // a reload builds from the checkpoint (`session_rows`).
        let rows: Vec<String> = live
            .chat
            .render_all(120)
            .iter()
            .map(|line| crate::utils::strip_ansi_codes(line))
            .collect();
        assert!(
            rows.iter()
                .any(|line| line.contains("Context compacted: 62203 → 33281 tokens")),
            "{rows:?}"
        );
    }

    /// An automatic compaction happens *inside* a run: the run's own activity
    /// row is the truth while it streams, so it must not take the session (or
    /// the reader's queue) the way `/compact` does.
    #[tokio::test]
    async fn an_automatic_compaction_leaves_the_session_to_its_run() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");

        live.handle_event(event(
            "compaction_started",
            serde_json::json!({"operation_id": "op-2", "trigger": "automatic", "phase": "pre_turn"}),
        ));
        assert!(!live.compacting());
    }

    /// The agent's own `isCompacting` is authoritative: a terminal event this
    /// client never saw (it attached after the compaction began, or another
    /// client ran it) must not leave the surface refusing prompts forever.
    #[tokio::test]
    async fn a_state_that_says_the_compaction_ended_gives_the_session_back() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");

        live.state.is_compacting = true;
        live.adopt_compaction();
        assert!(live.compacting());
        live.state.is_compacting = false;
        live.adopt_compaction();
        assert!(!live.compacting());
    }

    /// `agent_start` and `agent_end` are what tell the transcript a reply is
    /// still being written: between them its prose stays on screen, and only
    /// when the run ends does the reading level fold it onto its answer.
    ///
    /// Reading that off the messages instead made the last reply collapse and
    /// reopen on every step — a slice stops growing whenever the model moves on
    /// to a call — which is the flicker the desktop never shows.
    #[tokio::test]
    async fn a_run_brackets_when_its_reply_may_fold() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        // The frame's words depend on the machine's locale; this test asserts
        // the shape, so it pins the language.
        live.set_language(super::i18n::LanguagePreference::En);
        assert!(!live.chat.run_in_flight(), "nothing runs on a fresh client");

        live.chat
            .add_message(ChatMessage::new("u".into(), ChatRole::User, "go"));
        live.handle_event(event("agent_start", serde_json::json!({})));
        assert!(live.chat.run_in_flight(), "agent_start opens the run");
        live.handle_event(event(
            "text_chunk",
            serde_json::json!({"text": "First note."}),
        ));
        live.handle_event(event(
            "tool_start",
            serde_json::json!({"tool_id": "t1", "tool_name": "read", "tool_args": "{\"path\":\"/a.rs\"}"}),
        ));
        live.handle_event(event(
            "tool_end",
            serde_json::json!({"tool_id": "t1", "text": "ok"}),
        ));
        live.handle_event(event(
            "text_chunk",
            serde_json::json!({"text": "Second note."}),
        ));

        let rows = |chat: &mut ChatArea| -> Vec<String> {
            chat.render_all(120)
                .iter()
                .map(|line| crate::utils::strip_ansi_codes(line))
                .collect()
        };
        // In flight: both notes are on screen, and the call is one row.
        let running = rows(&mut live.chat);
        assert!(
            running.iter().any(|line| line.contains("First note.")),
            "{running:?}"
        );
        assert!(
            running.iter().any(|line| line.contains("read /a.rs")),
            "{running:?}"
        );

        live.handle_event(event("agent_end", serde_json::json!({})));
        assert!(!live.chat.run_in_flight(), "agent_end closes the run");
        let settled = rows(&mut live.chat);
        assert!(
            !settled.iter().any(|line| line.contains("First note.")),
            "the settled reply is its answer and the counts — {settled:?}"
        );
        assert!(
            settled.iter().any(|line| line.contains("▸ 1 tool call")),
            "{settled:?}"
        );
        assert!(
            settled.iter().any(|line| line.contains("Second note.")),
            "{settled:?}"
        );
    }

    /// The process rows are chrome, worded in the interface language: Chinese
    /// counts with 次 and puts the noun first, English pluralizes and puts the
    /// number first.
    #[test]
    fn the_process_rows_are_drawn_in_the_interface_language() {
        let zh = fold_words(Language::Zh);
        assert_eq!(zh.tool_calls, "工具调用 {} 次");
        assert_eq!(zh.thoughts, "思考 {} 次");
        assert_eq!(zh.thinking, "思考");
        assert_eq!(zh.process, "过程");
        let en = fold_words(Language::En);
        assert_eq!(en.tool_calls, "{} tool calls");
        assert_eq!(en.thoughts, "{} thoughts");
        assert_eq!(en.thinking, "thinking");
        assert_eq!(en.process, "process");

        // And a transcript drawn with them says what it means.
        let mut chat = ChatArea::new(120, None);
        chat.set_fold_words(zh);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "开始"));
        let mut lone = ChatMessage::new("t".into(), ChatRole::Assistant, "");
        lone.thinking = Some("先看看".into());
        chat.add_message(lone);
        chat.add_message(ChatMessage::new("u2".into(), ChatRole::User, "继续"));
        let mut answered = ChatMessage::new("a2".into(), ChatRole::Assistant, "好了");
        answered.thinking = Some("想清楚了".into());
        chat.add_message(answered);
        let rows: Vec<String> = chat
            .render_all(120)
            .iter()
            .map(|line| crate::utils::strip_ansi_codes(line))
            .collect();
        assert!(
            rows.iter().any(|line| line.contains("▸ 思考")),
            "the reasoning marker is Chinese — {rows:?}"
        );
        assert!(
            rows.iter().any(|line| line.contains("▸ 思考 1 次")),
            "the count row is Chinese too — {rows:?}"
        );
        assert!(
            !rows.iter().any(|line| line.contains("thought")),
            "{rows:?}"
        );
        assert!(
            !rows.iter().any(|line| line.contains("thinking")),
            "{rows:?}"
        );
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

    /// The switch answer a test's channel carried (the channel also carries the
    /// page above the transcript, the `git` view and the footer's state).
    fn switch_of(loaded: super::Loaded) -> super::SwitchLoaded {
        match loaded {
            super::Loaded::Switch(switch) => switch,
            _ => panic!("expected a switch answer"),
        }
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

    // ─── a switch off the loop ────────────────────────────────────────

    /// A click in the rail is the same switch as a step, and takes the same
    /// road: the read happens in a task, the surface keeps painting, and the
    /// driver is told the switch *began* so it does not follow "the conversation
    /// on screen" — which is still the one being left — and park the rail's
    /// cursor there for a frame.
    #[tokio::test]
    async fn a_click_reads_off_the_loop_and_tells_the_driver_it_began() {
        let first = page_of(&["q1"]);
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_slow(
            &[
                ("switch_session", r#"{"success":true}"#),
                ("get_state", r#"{"sessionId":"s-a"}"#),
                ("get_session_entries", &first),
            ],
            250,
        );
        let table = mock.data_by_type.clone();
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s-a"), false, None)
            .await
            .expect("attach to the slow test agent");
        let (tx, mut answers) = tokio::sync::mpsc::unbounded_channel();
        live.set_load_sender(tx);

        table
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-b"}"#.into());
        table
            .lock()
            .unwrap()
            .insert("get_session_entries".into(), page_of(&["q-b"]));
        let started = std::time::Instant::now();
        live.apply(WbAction::SwitchSession("s-b".into())).await;
        assert!(
            started.elapsed() < std::time::Duration::from_millis(200),
            "a click does not wait for the agent ({:?})",
            started.elapsed()
        );
        assert!(live.take_switch_started(), "the driver is told it began");
        assert!(!live.take_switch_started(), "and only once");
        assert_eq!(
            live.data(&frame()).session.id,
            "s-a",
            "the frame before the answer still names the conversation being left"
        );
        assert_eq!(
            live.data(&frame()).input.text,
            "",
            "while the box is already the conversation landed on"
        );
        assert!(
            rows_on_screen(&mut live).join("\n").contains("q1"),
            "and its rows are still on screen while the read is out"
        );

        let loaded = switch_of(answers.recv().await.expect("the click's read answers"));
        assert!(live.install_switch_load(loaded), "the answer owes a frame");
        assert_eq!(live.data(&frame()).session.id, "s-b");
        assert!(rows_on_screen(&mut live).join("\n").contains("q-b"));
        live.client.disconnect();
    }

    /// A surface with no driver behind it (the frame tests, the demos) has
    /// nowhere to hand an answer, so its clicks wait for their own: the same
    /// call, one round trip, nothing owed to a loop that does not exist.
    #[tokio::test]
    async fn a_click_with_no_driver_waits_for_its_own_answer() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("switch_session", r#"{"success":true}"#),
            ("get_state", r#"{"sessionId":"s-a"}"#),
            ("get_session_entries", &page_of(&["q1"])),
        ]);
        let table = mock.data_by_type.clone();
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s-a"), false, None)
            .await
            .expect("attach to the test agent");

        table
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-b"}"#.into());
        table
            .lock()
            .unwrap()
            .insert("get_session_entries".into(), page_of(&["q-b"]));
        live.apply(WbAction::SwitchSession("s-b".into())).await;
        assert!(!live.take_switch_started(), "nothing was left outstanding");
        assert_eq!(live.data(&frame()).session.id, "s-b");
        assert!(rows_on_screen(&mut live).join("\n").contains("q-b"));
        live.client.disconnect();
    }

    // ─── the page above, the footer's state, the git panel ───────────

    /// A prefetch reads one conversation's page and installs nothing: the answer
    /// is thrown away on purpose, because the agent keeps it for the step that
    /// is about to ask for it.
    #[tokio::test]
    async fn a_prefetch_reads_a_page_and_leaves_the_screen_alone() {
        let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("switch_session", r#"{"success":true}"#),
            ("get_state", r#"{"sessionId":"s-a"}"#),
            ("get_session_entries", &page_of(&["q1"])),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s-a"), false, None)
            .await
            .expect("attach to the test agent");
        let pages = |seen: &std::sync::Arc<std::sync::Mutex<Vec<String>>>| {
            seen.lock()
                .unwrap()
                .iter()
                .filter(|kind| kind.as_str() == "get_session_entries")
                .count()
        };
        let before = pages(&seen);

        live.prefetch_page("s-b");
        // The read is a task: give it the loop to run on.
        for _ in 0..50 {
            if pages(&seen) > before {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(pages(&seen), before + 1, "one page read, for s-b");
        assert_eq!(
            live.data(&frame()).session.id,
            "s-a",
            "and nothing is installed: the screen is still s-a"
        );

        // The conversation on screen needs no warming, and neither does nothing.
        live.prefetch_page("s-a");
        live.prefetch_page("");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            pages(&seen),
            before + 1,
            "neither the shown conversation nor an empty id is read"
        );
        live.client.disconnect();
    }

    /// Scrolling past the first loaded row asks for the page above it without
    /// waiting: a page is a real read (38–285 ms first time), and the reader is
    /// already scrolling.
    #[tokio::test]
    async fn the_page_above_is_read_off_the_loop_and_installed_when_it_lands() {
        let first = json!({
            "entries": [
                {"id": "u0", "role": "user", "blocks": [{"kind": "text", "text": "tail question"}]},
                {"id": "a0", "role": "assistant", "blocks": [{"kind": "text", "text": "tail answer"}]}
            ],
            "hasMore": true,
            "nextOffset": 0
        })
        .to_string();
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_slow(
            &[
                ("switch_session", r#"{"success":true}"#),
                ("get_state", r#"{"sessionId":"s-a"}"#),
                ("get_session_entries", &first),
            ],
            250,
        );
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s-a"), false, None)
            .await
            .expect("attach to the slow test agent");
        let (tx, mut answers) = tokio::sync::mpsc::unbounded_channel();
        live.set_load_sender(tx);

        let before = rows_on_screen(&mut live).len();
        let started = std::time::Instant::now();
        assert!(
            live.begin_load_older(),
            "the transcript has a page above it"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_millis(200),
            "asking for it does not wait ({:?})",
            started.elapsed()
        );
        assert!(
            !live.begin_load_older(),
            "and the same window is not asked for twice"
        );
        assert_eq!(
            rows_on_screen(&mut live).len(),
            before,
            "nothing was prepended before the answer"
        );

        let loaded = answers.recv().await.expect("the page answers");
        let super::Loaded::Older(page) = loaded else {
            panic!("expected the page above");
        };
        assert_eq!(page.before, 0, "the window that was asked for");
        assert!(live.install_older(page), "the page owes a frame");
        assert_eq!(
            rows_on_screen(&mut live).len(),
            before * 2,
            "the page was prepended above the tail"
        );
        live.client.disconnect();
    }

    /// A page read for a window the transcript has left (another conversation,
    /// or a page that already arrived) is dropped rather than prepended above
    /// rows it does not sit on.
    #[tokio::test]
    async fn an_older_page_for_a_window_the_transcript_left_is_dropped() {
        let first = json!({
            "entries": [
                {"id": "u0", "role": "user", "blocks": [{"kind": "text", "text": "q1"}]}
            ],
            "hasMore": true,
            "nextOffset": 0
        })
        .to_string();
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("switch_session", r#"{"success":true}"#),
            ("get_state", r#"{"sessionId":"s-a"}"#),
            ("get_session_entries", &first),
        ]);
        let table = mock.data_by_type.clone();
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s-a"), false, None)
            .await
            .expect("attach to the test agent");
        let (tx, mut answers) = tokio::sync::mpsc::unbounded_channel();
        live.set_load_sender(tx);

        assert!(live.begin_load_older());
        let loaded = answers.recv().await.expect("the page answers");

        // The reader opened another conversation while the page was in flight
        // (and that switch has landed: the transcript is s-b's now).
        table
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-b"}"#.into());
        live.apply(WbAction::SwitchSession("s-b".into())).await;
        let switched = switch_of(answers.recv().await.expect("the switch answers"));
        assert!(live.install_switch_load(switched));
        assert_eq!(live.data(&frame()).session.id, "s-b");

        let super::Loaded::Older(page) = loaded else {
            panic!("expected the page above");
        };
        assert!(
            !live.install_older(page),
            "its rows belong to the conversation the reader left"
        );
        live.client.disconnect();
    }

    /// The turn boundary's two reads are started, not waited for: one listing
    /// per boundary (a second is refused while the first is out) and one state.
    #[tokio::test]
    async fn a_turn_boundary_reads_the_state_beside_the_surface() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_slow(
            &[
                ("switch_session", r#"{"success":true}"#),
                ("get_state", r#"{"sessionId":"s-a","contextTokens":123}"#),
                ("get_session_entries", &page_of(&["q1"])),
            ],
            250,
        );
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s-a"), false, None)
            .await
            .expect("attach to the slow test agent");
        let (tx, mut answers) = tokio::sync::mpsc::unbounded_channel();
        live.set_load_sender(tx);

        let started = std::time::Instant::now();
        assert!(live.begin_state_refresh(), "the footer asks");
        assert!(
            started.elapsed() < std::time::Duration::from_millis(200),
            "asking does not wait ({:?})",
            started.elapsed()
        );
        assert!(!live.begin_state_refresh(), "one read is already out");

        let loaded = answers.recv().await.expect("the state answers");
        let super::Loaded::State(state) = loaded else {
            panic!("expected a state answer");
        };
        assert!(live.install_state_refresh(state));
        assert_eq!(
            live.data(&frame()).usage.ctx_used,
            123,
            "the footer shows the state that landed"
        );
        assert!(
            live.begin_state_refresh(),
            "the slot frees once its answer is installed"
        );
        live.client.disconnect();
    }

    /// The `/git` panel's read runs on a blocking thread and lands through the
    /// driver's arm; a view read for a tab the reader has already left is
    /// dropped rather than flipping the panel back.
    #[tokio::test]
    async fn a_git_view_lands_without_the_loop_waiting() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_slow(
            &[
                ("switch_session", r#"{"success":true}"#),
                ("get_state", r#"{"sessionId":"s-a"}"#),
                ("get_session_entries", &page_of(&["q1"])),
            ],
            5,
        );
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s-a"), false, None)
            .await
            .expect("attach to the test agent");
        let (tx, mut answers) = tokio::sync::mpsc::unbounded_channel();
        live.set_load_sender(tx);
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("the repository root")
            .to_path_buf();

        let started = std::time::Instant::now();
        assert!(live.begin_git_read(
            repo.clone(),
            crate::workbench::state::GitSection::Commits,
            None,
            0
        ));
        assert!(
            started.elapsed() < std::time::Duration::from_millis(100),
            "opening the panel does not wait for git ({:?})",
            started.elapsed()
        );
        // A second read supersedes the first, which must not install.
        assert!(live.begin_git_read(repo, crate::workbench::state::GitSection::Refs, None, 0));

        let mut seen = 0;
        while seen < 2 {
            let loaded = answers.recv().await.expect("a git read answers");
            let super::Loaded::Git { rev, view } = loaded else {
                panic!("expected a git view");
            };
            let installed = live.install_git(rev, view);
            seen += 1;
            if installed {
                assert_eq!(
                    live.git().map(|git| git.section),
                    Some(crate::workbench::state::GitSection::Refs),
                    "the tab that is installed is the one asked for last"
                );
            }
        }
        assert!(live.git().is_some(), "a view is on screen");
        live.client.disconnect();
    }

    /// `shift+↑/↓` hands the agent's two reads to a task: the step returns
    /// without waiting, and the conversation's rows arrive on their own
    /// wake-up.
    ///
    /// The waiting form is what made a held key look stuck — the loop was
    /// behind an agent answer (measured: 75 ms warm, seconds on a busy store)
    /// on *every* step, so the rail, the stream and the keyboard all stopped.
    #[tokio::test]
    async fn a_step_reads_off_the_loop_and_installs_its_answer_when_it_lands() {
        let first = page_of(&["q1"]);
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_slow(
            &[
                ("switch_session", r#"{"success":true}"#),
                ("get_state", r#"{"sessionId":"s-a"}"#),
                ("get_session_entries", &first),
            ],
            250,
        );
        let table = mock.data_by_type.clone();
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s-a"), false, None)
            .await
            .expect("attach to the slow test agent");
        let (tx, mut answers) = tokio::sync::mpsc::unbounded_channel();
        live.set_load_sender(tx);

        // What s-b will answer, moved in before it is asked for.
        table
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-b"}"#.into());
        table
            .lock()
            .unwrap()
            .insert("get_session_entries".into(), page_of(&["q2"]));

        let started = std::time::Instant::now();
        assert!(
            !live.start_step_switch("s-b"),
            "nothing cached for s-b to paint"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_millis(200),
            "a step does not wait for the agent ({:?})",
            started.elapsed()
        );
        assert!(
            answers.try_recv().is_err(),
            "the answer is still out with the agent"
        );
        assert!(
            rows_on_screen(&mut live).join("\n").contains("q1"),
            "the conversation that was on screen is still there"
        );

        let loaded = switch_of(answers.recv().await.expect("the step's read answers"));
        assert_eq!(loaded.id, "s-b");
        assert!(live.install_switch_load(loaded), "the answer owes a frame");
        let screen = rows_on_screen(&mut live).join("\n");
        assert!(
            screen.contains("q2"),
            "the rows that landed are on screen: {screen}"
        );
        assert!(
            !screen.contains("q1"),
            "and the conversation left is not: {screen}"
        );
        assert!(answers.try_recv().is_err(), "one step, one read");
        live.client.disconnect();
    }

    /// A step the reader has already moved past is dropped when its answer
    /// lands: the rows belong to the conversation they are on, and installing
    /// the one they passed through would paint over it.
    #[tokio::test]
    async fn an_answer_for_a_conversation_the_reader_left_is_dropped() {
        let first = page_of(&["q1"]);
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("switch_session", r#"{"success":true}"#),
            ("get_state", r#"{"sessionId":"s-a"}"#),
            ("get_session_entries", &first),
        ]);
        let table = mock.data_by_type.clone();
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s-a"), false, None)
            .await
            .expect("attach to the test agent");
        let (tx, mut answers) = tokio::sync::mpsc::unbounded_channel();
        live.set_load_sender(tx);

        table
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-b"}"#.into());
        table
            .lock()
            .unwrap()
            .insert("get_session_entries".into(), page_of(&["q-b"]));
        live.start_step_switch("s-b");
        let passed = switch_of(answers.recv().await.expect("s-b answers"));

        table
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-c"}"#.into());
        table
            .lock()
            .unwrap()
            .insert("get_session_entries".into(), page_of(&["q-c"]));
        live.start_step_switch("s-c");

        assert!(
            !live.install_switch_load(passed),
            "s-b is a conversation the reader has left"
        );
        assert!(
            live.data(&frame()).session.id == "s-a",
            "so the screen still names the one they are on"
        );

        let landed = switch_of(answers.recv().await.expect("s-c answers"));
        assert!(
            live.install_switch_load(landed),
            "the one they are on lands"
        );
        let screen = rows_on_screen(&mut live).join("\n");
        assert!(screen.contains("q-c"), "{screen}");
        assert!(!screen.contains("q-b"), "{screen}");
        live.client.disconnect();
    }

    /// The draft belongs to the conversation the reader is *on*, so it moves
    /// with the cursor rather than with the agent's answer: a key typed while a
    /// step's read is still out lands in the conversation they see.
    #[tokio::test]
    async fn the_draft_moves_with_the_cursor_and_not_with_the_answer() {
        let first = page_of(&["q1"]);
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_slow(
            &[
                ("switch_session", r#"{"success":true}"#),
                ("get_state", r#"{"sessionId":"s-a"}"#),
                ("get_session_entries", &first),
            ],
            250,
        );
        let table = mock.data_by_type.clone();
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s-a"), false, None)
            .await
            .expect("attach to the slow test agent");
        let (tx, mut answers) = tokio::sync::mpsc::unbounded_channel();
        live.set_load_sender(tx);

        live.set_draft("half a prompt".into(), 13);
        table
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-b"}"#.into());
        table
            .lock()
            .unwrap()
            .insert("get_session_entries".into(), page_of(&["q2"]));
        live.start_step_switch("s-b");
        assert_eq!(
            live.data(&frame()).input.text,
            "",
            "the box is the conversation landed on, not the one left"
        );

        // Typed while the answer is still out.
        live.set_draft("typed after".into(), 12);
        let loaded = switch_of(answers.recv().await.expect("the step's read answers"));
        assert!(live.install_switch_load(loaded));
        assert_eq!(
            live.data(&frame()).input.text,
            "typed after",
            "the answer does not overwrite what was typed into its box"
        );

        // …and the way back still finds what was left there.
        table
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-a"}"#.into());
        table
            .lock()
            .unwrap()
            .insert("get_session_entries".into(), page_of(&["q1"]));
        live.start_step_switch("s-a");
        let back = switch_of(answers.recv().await.expect("s-a answers"));
        assert!(live.install_switch_load(back));
        assert_eq!(live.data(&frame()).input.text, "half a prompt");
        live.client.disconnect();
    }

    /// A step back onto the conversation still on screen cancels the read that
    /// was out for its successor: the reader has left that one, and its rows
    /// must not arrive later and paint over this one.
    #[tokio::test]
    async fn stepping_back_onto_the_conversation_on_screen_cancels_the_read() {
        let first = page_of(&["q1"]);
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_slow(
            &[
                ("switch_session", r#"{"success":true}"#),
                ("get_state", r#"{"sessionId":"s-a"}"#),
                ("get_session_entries", &first),
            ],
            100,
        );
        let table = mock.data_by_type.clone();
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s-a"), false, None)
            .await
            .expect("attach to the test agent");
        let (tx, mut answers) = tokio::sync::mpsc::unbounded_channel();
        live.set_load_sender(tx);

        table
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-b"}"#.into());
        table
            .lock()
            .unwrap()
            .insert("get_session_entries".into(), page_of(&["q-b"]));
        live.start_step_switch("s-b");
        // …and back, before s-b answered.
        assert!(
            !live.start_step_switch("s-a"),
            "the conversation on screen is already there"
        );

        let stale = switch_of(answers.recv().await.expect("s-b was read"));
        assert!(!live.install_switch_load(stale), "s-b is not wanted");
        assert_eq!(live.data(&frame()).session.id, "s-a");
        assert!(rows_on_screen(&mut live).join("\n").contains("q1"));
        live.client.disconnect();
    }

    /// A step's answer that lands after the reader opened something else is
    /// dropped: the screen belongs to the last thing they asked for — a click on
    /// another row, or `/new`.
    #[tokio::test]
    async fn a_step_answer_never_overrides_what_was_opened_since() {
        let first = page_of(&["q1"]);
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_slow(
            &[
                ("switch_session", r#"{"success":true}"#),
                ("get_state", r#"{"sessionId":"s-a"}"#),
                ("get_session_entries", &first),
            ],
            100,
        );
        let table = mock.data_by_type.clone();
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s-a"), false, None)
            .await
            .expect("attach to the test agent");
        let (tx, mut pending) = tokio::sync::mpsc::unbounded_channel();
        live.set_load_sender(tx);

        table
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-b"}"#.into());
        table
            .lock()
            .unwrap()
            .insert("get_session_entries".into(), page_of(&["q-b"]));
        live.start_step_switch("s-b");

        // A click on another row while s-b is still being read.
        table
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-c"}"#.into());
        table
            .lock()
            .unwrap()
            .insert("get_session_entries".into(), page_of(&["q-c"]));
        live.apply(WbAction::SwitchSession("s-c".into())).await;

        // Both answers land, in whichever order the tasks finish: the step's is
        // dropped, the click's installs.
        let mut seen: Vec<(String, bool)> = Vec::new();
        while seen.len() < 2 {
            let loaded = switch_of(pending.recv().await.expect("a read answered"));
            let id = loaded.id.clone();
            let installed = live.install_switch_load(loaded);
            seen.push((id, installed));
        }
        assert!(
            seen.iter()
                .any(|(id, installed)| id == "s-b" && !*installed),
            "the step was superseded by the conversation opened since: {seen:?}"
        );
        assert!(
            seen.iter().any(|(id, installed)| id == "s-c" && *installed),
            "the clicked conversation is the one that lands: {seen:?}"
        );
        assert!(
            rows_on_screen(&mut live).join("\n").contains("q-c"),
            "and it is what stays on screen"
        );

        // The same for `/new`: a step's rows must not land on the launchpad.
        live.start_step_switch("s-a");
        live.to_launchpad();
        let landing = switch_of(pending.recv().await.expect("s-a was read"));
        assert!(!live.install_switch_load(landing), "the launchpad stays");
        assert!(
            live.data(&frame()).session.id.is_empty(),
            "no session shown"
        );
        live.client.disconnect();
    }

    /// A switch the agent cannot answer leaves the reader where they were, with
    /// both drafts intact: the text of the conversation still on screen, and the
    /// one of the conversation that never opened.
    #[tokio::test]
    async fn a_failed_step_keeps_both_drafts_and_the_conversation_it_was_on() {
        let first = page_of(&["q1"]);
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("switch_session", r#"{"success":true}"#),
            ("get_state", r#"{"sessionId":"s-a"}"#),
            ("get_session_entries", &first),
        ]);
        let table = mock.data_by_type.clone();
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s-a"), false, None)
            .await
            .expect("attach to the test agent");
        let (tx, mut answers) = tokio::sync::mpsc::unbounded_channel();
        live.set_load_sender(tx);
        live.set_draft("half a prompt".into(), 13);

        // A state the client cannot read: the switch fails on the way in.
        table
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":42}"#.into());
        live.start_step_switch("s-b");
        live.set_draft("typed after".into(), 12);
        let failed = switch_of(answers.recv().await.expect("the read answers either way"));
        assert!(live.install_switch_load(failed), "the notice owes a frame");
        assert!(
            live.notice
                .as_deref()
                .is_some_and(|notice| notice.contains("switch")),
            "the notice says what failed: {:?}",
            live.notice
        );
        assert_eq!(
            live.data(&frame()).session.id,
            "s-a",
            "the reader stays on the conversation they were on"
        );
        assert_eq!(
            live.data(&frame()).input.text,
            "half a prompt",
            "and the box gets its own text back"
        );
        assert!(
            rows_on_screen(&mut live).join("\n").contains("q1"),
            "the transcript is untouched"
        );

        // What was typed into s-b's box is filed under s-b, not lost.
        table
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-b"}"#.into());
        table
            .lock()
            .unwrap()
            .insert("get_session_entries".into(), page_of(&["q-b"]));
        live.start_step_switch("s-b");
        let landed = switch_of(answers.recv().await.expect("s-b answers this time"));
        assert!(live.install_switch_load(landed));
        assert_eq!(live.data(&frame()).session.id, "s-b");
        assert_eq!(live.data(&frame()).input.text, "typed after");
        live.client.disconnect();
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
                    session_name: Some("moved".into()),
                    parent_session_id: None,
                    is_streaming: None,
                    first_message: None,
                    query_count: Some(1),
                }]);
            live.data(&frame);
            let cold = start.elapsed().as_secs_f64() * 1000.0;
            assert!(live.session_view_builds() > rebuild, "the list moved");

            // The rail's 5 s poll, priced as the round trip the driver starts
            // off the loop (`Live::begin_sessions_refresh`).
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

    /// Every verb the adapter carries out answers *something*.
    ///
    /// The bug class this catches is the palette advertising a verb that comes back
    /// `Unknown command /…` — five of them did once, and nothing failed. `/paste`
    /// is left out on purpose: it reads the host clipboard, which a test machine
    /// may not have.
    #[test]
    fn every_app_command_is_answered_by_the_adapter() {
        with_big_stack(|| {
            Box::pin(async {
                let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
                    ("get_state", r#"{"sessionId":"s1","model":"mock"}"#),
                    ("get_session_entries", r#"{"entries":[]}"#),
                    ("export_html", r#"{"path":"/tmp/x.html"}"#),
                    ("list_sessions", r#"{"sessions":[]}"#),
                    ("list_models", r#"{"models":[]}"#),
                ]);
                let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
                let (mut live, _events) = Live::attach(&addr, None, false, None)
                    .await
                    .expect("attach to the test agent");
                for command in APP_COMMANDS {
                    if command == "/paste" {
                        continue;
                    }
                    live.notice = None;
                    live.apply(WbAction::Command(command.into())).await;
                    let notice = live.notice.clone().unwrap_or_default();
                    assert!(
                        !notice.starts_with("Unknown command"),
                        "the palette advertises {command}, and the adapter refuses it: {notice}"
                    );
                }
            })
        });
    }

    /// `/tools off` must reach the agent as `disable_tools` with the session this
    /// client is attached to — and the notice must say it took effect.
    ///
    /// The reader's report was the opposite: a prompt sent after `/tools off` still
    /// called tools. That can only be told apart from a stale binary by pinning the
    /// two facts here — the RPC leaves, and it names the attached session.
    #[test]
    fn tools_off_sends_disable_tools_for_the_attached_session() {
        with_big_stack(|| {
            Box::pin(async {
                let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
                    ("get_state", r#"{"sessionId":"s1","model":"mock"}"#),
                    ("get_session_entries", r#"{"entries":[]}"#),
                    ("disable_tools", "{}"),
                ]);
                let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock.clone()).await;
                let (mut live, _events) = Live::attach(&addr, None, false, None)
                    .await
                    .expect("attach to the test agent");

                live.apply(WbAction::Command("/tools off".into())).await;

                let commands = seen.lock().unwrap().clone();
                assert!(
                    commands.iter().any(|command| command == "disable_tools"),
                    "`/tools off` must reach the agent, saw {commands:?}"
                );
                let request =
                    crate::rpc::grpc_client::tests::last_request_of(&mock, "disable_tools");
                assert_eq!(
                    request.session_id, "s1",
                    "the tool set belongs to the session on screen, not to a default one"
                );
                assert!(
                    !live.notice.clone().unwrap_or_default().contains("/tools"),
                    "the notice confirms the choice rather than printing usage"
                );

                // The launchpad is the case the reader hit: no session exists yet, so a
                // tool set has nowhere to live. It must not answer for the conversation
                // just left behind either — that session's tools are not this one's — and
                // it must not claim the choice took effect.
                live.to_launchpad();
                live.apply(WbAction::Command("/tools off".into())).await;
                let after = seen.lock().unwrap().clone();
                assert_eq!(
                    after
                        .iter()
                        .filter(|c| c.as_str() == "disable_tools")
                        .count(),
                    1,
                    "the launchpad has no session to disable tools in: {after:?}"
                );
                assert_eq!(
                    live.pending_tools,
                    Some(Vec::new()),
                    "the choice is staged for the session the first message creates"
                );
                let notice = live.notice.clone().unwrap_or_default();
                assert!(
                    !notice.is_empty() && !notice.contains("/tools"),
                    "the notice says the choice is staged, not that it took effect: {notice:?}"
                );
            })
        });
    }

    /// `/rename <name>` is the one verb that writes a conversation's own title:
    /// the name leaves for the agent as `set_session_name` on the session on
    /// screen, and the surface installs it from the answer it already holds —
    /// the header and the rail's row — instead of re-reading a listing that
    /// costs seconds (see [`Live::apply_session_name`]).
    ///
    /// A bare verb is the usage line, not a rename, and the launchpad has no
    /// conversation to name: neither may reach the agent.
    #[test]
    fn the_rename_verb_writes_the_title_and_installs_it_without_a_listing() {
        with_big_stack(|| {
            Box::pin(async {
                let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
                    ("get_state", r#"{"sessionId":"s1","model":"mock"}"#),
                    ("get_session_entries", r#"{"entries":[]}"#),
                    (
                        "list_sessions",
                        r#"{"sessions":[{"id":"s1","model":"mock","cwd":"/tmp","firstMessage":"hello","queryCount":1}]}"#,
                    ),
                ]);
                let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock.clone()).await;
                let (mut live, _events) = Live::attach(&addr, None, false, None)
                    .await
                    .expect("attach to the test agent");
                live.refresh_sessions().await;
                assert_eq!(live.sessions.len(), 1, "the rail holds the conversation");
                assert_eq!(
                    live.title(),
                    "hello",
                    "the row is called by its first message"
                );

                let before = seen.lock().unwrap().len();
                live.apply(WbAction::Command("/rename 修 parser".into()))
                    .await;

                let commands = seen.lock().unwrap().clone();
                // The write is the whole round trip: the name installed below
                // came from the request, not from a listing read back.
                assert_eq!(
                    commands[before..].to_vec(),
                    vec!["set_session_name".to_string()],
                    "`/rename` is one write and no re-read: {commands:?}"
                );
                let request =
                    crate::rpc::grpc_client::tests::last_request_of(&mock, "set_session_name");
                assert_eq!(
                    request.session_id, "s1",
                    "the title belongs to the conversation on screen"
                );
                assert_eq!(
                    request.name, "修 parser",
                    "the words after the verb are the name"
                );
                assert_eq!(
                    live.notice.as_deref(),
                    Some(
                        i18n::render(live.language(), Key::NoticeRenamed, &[&"修 parser"]).as_str()
                    ),
                    "the notice names the title that took"
                );
                assert_eq!(live.title(), "修 parser", "the header names the new title");
                assert_eq!(
                    live.sessions
                        .iter()
                        .find(|row| row.id == "s1")
                        .and_then(|row| row.session_name.as_deref()),
                    Some("修 parser"),
                    "the rail's row carries the new title without a re-read"
                );

                // A bare verb says what to type rather than naming a
                // conversation nothing.
                live.notice = None;
                live.apply(WbAction::Command("/rename".into())).await;
                assert_eq!(
                    live.notice.as_deref(),
                    Some(t(live.language(), Key::RenameUsage)),
                    "a rename needs a name"
                );

                // The launchpad has no conversation: the verb must not name the
                // one the reader just left.
                live.to_launchpad();
                live.apply(WbAction::Command("/rename 别的".into())).await;
                assert_eq!(
                    live.notice.as_deref(),
                    Some(t(live.language(), Key::NeedsConversation)),
                    "the launchpad has no conversation to rename"
                );
                let after = seen.lock().unwrap().clone();
                assert_eq!(
                    after
                        .iter()
                        .filter(|command| command.as_str() == "set_session_name")
                        .count(),
                    1,
                    "only the attached conversation was renamed: {after:?}"
                );
            })
        });
    }

    /// `/compact` on an attached conversation is one round trip to the agent
    /// and a surface that says what it is doing: the agent acknowledges, the
    /// row turns to "compacting", and the reader's prompt is refused until the
    /// terminal event arrives (see
    /// `a_manual_compaction_holds_the_session_until_it_commits` for the
    /// event half, which this test does not have to repeat).
    #[test]
    fn the_compact_verb_takes_the_session_over_on_its_acknowledgement() {
        with_big_stack(|| {
            Box::pin(async {
                let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
                    (
                        "get_state",
                        // The agent's own flag is what the surface trusts; the
                        // real one sets it before it answers `compact`.
                        r#"{"sessionId":"s1","model":"mock","isCompacting":true}"#,
                    ),
                    ("get_session_entries", r#"{"entries":[]}"#),
                    ("compact", r#"{"accepted":true,"operationId":"cmp-1"}"#),
                ]);
                let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock.clone()).await;
                let (mut live, _events) = Live::attach(&addr, None, false, None)
                    .await
                    .expect("attach to the test agent");

                live.apply(WbAction::Command("/compact".into())).await;

                let commands = seen.lock().unwrap().clone();
                assert!(
                    commands.iter().any(|command| command == "compact"),
                    "`/compact` must reach the agent, saw {commands:?}"
                );
                assert!(live.compacting(), "the acknowledgement holds the session");
                assert_eq!(
                    live.notice.as_deref(),
                    Some(t(live.language(), Key::NoticeCompacting)),
                    "the page says the compaction is running"
                );
            })
        });
    }

    /// The staged tool set is applied to the session the first message creates —
    /// after it exists, and before the message that created it runs.
    ///
    /// This is the reported bug: `/tools off` on the launchpad reached the agent
    /// *before* a session existed, so the conversation the reader then started came
    /// up with every tool on and the very next prompt called one.
    #[test]
    #[allow(clippy::await_holding_lock)] // FUTURE_HOME is process-global and must stay pinned across awaits.
    fn a_launchpad_tool_set_is_applied_to_the_session_the_first_message_creates() {
        with_big_stack(|| {
            Box::pin(async {
                // FUTURE_HOME is process-global and must stay pinned across awaits.
                let _guard = crate::test_env::lock();
                let home = tempfile::tempdir().unwrap();
                let _home = crate::test_env::EnvVarGuard::set("FUTURE_HOME", home.path());
                let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
                    ("new_session", r#"{"sessionId":"s-new"}"#),
                    ("get_state", r#"{"sessionId":"s-new","model":"mock"}"#),
                    ("get_session_entries", r#"{"entries":[]}"#),
                    ("prompt", r#"{"runId":"r-1","acceptedState":"running"}"#),
                    ("disable_tools", "{}"),
                    ("list_sessions", r#"{"sessions":[]}"#),
                    ("list_models", r#"{"models":[]}"#),
                ]);
                let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock.clone()).await;
                let (mut live, _events) = Live::attach(&addr, None, false, None)
                    .await
                    .expect("attach to the test agent");
                live.to_launchpad();

                live.apply(WbAction::Command("/tools off".into())).await;
                assert_eq!(
                    seen.lock()
                        .unwrap()
                        .iter()
                        .filter(|c| c.as_str() == "disable_tools")
                        .count(),
                    0,
                    "with no session, the verb has nothing to tell"
                );

                live.set_draft("hello there".into(), 11);
                live.set_draft(String::new(), 0);
                live.apply(WbAction::Submit("hello there".into())).await;

                let commands = seen.lock().unwrap().clone();
                let created = commands
                    .iter()
                    .position(|command| command == "new_session")
                    .unwrap_or_else(|| {
                        panic!("the first message creates the session: {commands:?}")
                    });
                let disabled = commands
                    .iter()
                    .position(|command| command == "disable_tools")
                    .unwrap_or_else(|| {
                        panic!("the staged tool set reaches the new session: {commands:?}")
                    });
                assert!(
                    created < disabled,
                    "the tool set is applied once the session exists: {commands:?}"
                );
                assert!(
                    live.pending_tools.is_none(),
                    "the staged set is spent on this conversation"
                );
            })
        });
    }

    /// `/compact` and `/export` name the conversation on screen, so on the launchpad
    /// they say so instead of compacting — or exporting — the one the reader left.
    #[test]
    fn verbs_that_need_a_conversation_say_so_on_the_launchpad() {
        with_big_stack(|| {
            Box::pin(async {
                let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
                    ("get_state", r#"{"sessionId":"s-prev","model":"mock"}"#),
                    ("get_session_entries", r#"{"entries":[]}"#),
                    ("compact", "{}"),
                    ("export_html", r#"{"path":"/tmp/x.html"}"#),
                ]);
                let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
                let (mut live, _events) = Live::attach(&addr, None, false, None)
                    .await
                    .expect("attach to the test agent");
                live.to_launchpad();
                for command in ["/compact", "/export"] {
                    let before = seen.lock().unwrap().clone();
                    live.apply(WbAction::Command(command.into())).await;
                    let after = seen.lock().unwrap().clone();
                    assert_eq!(
            after.len(),
            before.len(),
            "{command} must not reach the agent with no conversation on screen: {after:?}"
        );
                    assert!(
                        !live.notice.clone().unwrap_or_default().is_empty(),
                        "{command} says what it needs instead of pretending"
                    );
                }
            })
        });
    }

    /// The `[复制]` control copies the message the reader pointed at, not the
    /// last answer `/copy` takes — the markdown the transcript holds, so a reply
    /// can be quoted elsewhere. The row is resolved to its message, so a copy of
    /// any other row (a prompt, a call) is the same call.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // FUTURE_HOME is process-global and must stay pinned across awaits.
    async fn the_copy_control_copies_the_message_under_it() {
        let _guard = crate::test_env::lock();
        let home = tempfile::tempdir().unwrap();
        let _home = crate::test_env::EnvVarGuard::set("FUTURE_HOME", home.path());
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("get_state", r#"{"sessionId":"s1"}"#),
            ("get_session_entries", r#"{"entries":[]}"#),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");
        live.chat.add_message(ChatMessage::new(
            "u1".into(),
            ChatRole::User,
            "写个回归测试",
        ));
        live.chat
            .add_message(ChatMessage::new("a1".into(), ChatRole::Assistant, "好了。"));
        live.chat.render(80);

        let reply_row = *live
            .chat
            .control_rows()
            .last()
            .expect("the reply's controls");
        assert_eq!(live.copy_message(reply_row).as_deref(), Ok("好了。"));
        // The prompt's own row has no chip, but the call is the same: a row that
        // covers a message is copied as that message.
        let prompt_row = (0..reply_row)
            .find(|row| live.chat.message_at_row(*row) == Some(0))
            .expect("the prompt's first row");
        assert_eq!(live.copy_message(prompt_row).as_deref(), Ok("写个回归测试"));
        // A row that covers no message answers with the empty-clipboard notice
        // rather than a panic.
        assert!(live.copy_message(9_999).is_err());
    }

    /// The `/fork` box lists what the agent says is persisted: questions *and*
    /// answers, in transcript order, each with the role its row is labelled by.
    ///
    /// The list comes from the agent and not off the transcript on purpose — a
    /// reply still streaming has no entry yet, so the newest point a live turn
    /// offers is the prompt that started it.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // FUTURE_HOME is process-global and must stay pinned across awaits.
    async fn the_fork_box_lists_questions_and_answers() {
        let _guard = crate::test_env::lock();
        let home = tempfile::tempdir().unwrap();
        let _home = crate::test_env::EnvVarGuard::set("FUTURE_HOME", home.path());
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            (
                "get_fork_entries",
                r#"{"entries":[
                    {"id":"e1","role":"user","blocks":[{"kind":"text","text":"why does it fail?"}],"createdAtMs":1},
                    {"id":"e2","role":"assistant","blocks":[{"kind":"text","text":"because the loop advanced first"}],"createdAtMs":2}
                ]}"#,
            ),
            ("get_state", r#"{"sessionId":"s1"}"#),
            ("get_session_entries", r#"{"entries":[]}"#),
            ("list_sessions", r#"{"sessions":[]}"#),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");

        live.load_fork_entries().await;
        let points: Vec<_> = live
            .fork_points
            .iter()
            .map(|point| (point.role, point.text.as_str(), point.id.as_str()))
            .collect();
        assert_eq!(
            points,
            vec![
                (
                    super::super::state::ForkRole::User,
                    "why does it fail?",
                    "e1"
                ),
                (
                    super::super::state::ForkRole::Assistant,
                    "because the loop advanced first",
                    "e2"
                ),
            ]
        );
        assert!(live.notice.is_none(), "{:?}", live.notice);
    }

    /// A chosen point is the message it names: `through_entry`, so the child
    /// ends there — an answer's fork carries the answer, a question's fork
    /// stops at the question.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // FUTURE_HOME is process-global and must stay pinned across awaits.
    async fn the_chosen_fork_point_ends_the_child_with_that_message() {
        let _guard = crate::test_env::lock();
        let home = tempfile::tempdir().unwrap();
        let _home = crate::test_env::EnvVarGuard::set("FUTURE_HOME", home.path());
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("get_state", r#"{"sessionId":"s1"}"#),
            ("get_session_entries", r#"{"entries":[]}"#),
            ("list_sessions", r#"{"sessions":[]}"#),
            ("fork", r#"{"sessionId":"child-1"}"#),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock.clone()).await;
        let (mut live, _events) = Live::attach(&addr, None, false, None)
            .await
            .expect("attach to the test agent");

        live.fork_at("entry-9").await;
        let request = crate::rpc::grpc_client::tests::last_request_of(&mock, "fork");
        assert_eq!(request.entry_id, "entry-9");
        assert_eq!(request.mode, "through_entry");
        assert_eq!(request.created_by, "tui");
    }

    /// Attach to a mock agent that answers `pairs`, for the tables below.
    async fn live_on(pairs: &[(&str, &str)]) -> Live {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_answering(pairs);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (live, _events) = Live::attach(&addr, Some("s1"), false, None)
            .await
            .expect("attach to the mock agent");
        live
    }

    /// Every adapter action the surface can hand over has the effect the surface
    /// then sees — or an explicit "nothing, the surface owns it" arm.
    ///
    /// The table is the point: `apply`'s arms are one line each, and no behaviour
    /// test would name them one by one, but an action wired to the wrong arm (a
    /// stage that sends, a send that stages) is exactly what a silently-empty arm
    /// hides.
    #[tokio::test]
    async fn every_adapter_action_has_the_effect_the_surface_expects() {
        type Check = fn(&mut super::Live) -> bool;

        let cases: Vec<(&str, WbAction, Check)> = vec![
            (
                "a draft edit lands in the draft",
                WbAction::EditInput {
                    text: "half typed".into(),
                    cursor: 4,
                },
                |live| live.draft.editor.text() == "half typed",
            ),
            (
                "a staged model is staged, not sent",
                WbAction::SetPendingModel("future/other".into()),
                |live| live.pending_model() == Some("future/other"),
            ),
            (
                "a staged level is staged, not sent",
                WbAction::SetPendingThinking("high".into()),
                |live| live.pending_effort() == Some("high"),
            ),
            (
                "`/new` detaches from the conversation",
                WbAction::NewSession,
                |live| live.detached(),
            ),
            (
                "the palette is the surface's own",
                WbAction::SetTheme("future-dark".into()),
                |_| true,
            ),
            (
                "the panel rectangle is the surface's own",
                WbAction::ResizeSidebar(40),
                |_| true,
            ),
            (
                "a notice is the driver's to show",
                WbAction::Notice("hello".into()),
                |_| true,
            ),
            (
                "a redraw is a frame, not an answer",
                WbAction::Redraw,
                |_| true,
            ),
            (
                "a scroll is the surface's own",
                WbAction::Scroll(-3),
                |_| true,
            ),
        ];

        for (what, action, check) in cases {
            let mut live = live_on(&[]).await;
            let outcome = live.apply(action).await;
            assert!(
                matches!(outcome, Outcome::Continue),
                "{what}: an action must not end the app"
            );
            assert!(check(&mut live), "{what}");
            live.client.disconnect();
        }
    }

    /// The launchpad chips walk the agent's own order and stage the answer: a
    /// cycle never sends anything (there is no session to send it to), and the
    /// ring wraps rather than stopping at the end of the list.
    #[tokio::test]
    async fn the_launchpad_chips_cycle_and_wrap_without_sending() {
        let models = r#"{"models":[
            {"id":"a","label":"a","provider":"future","thinkingLevel":"default",
             "thinkingLevels":["default","low","high"],"defaultThinkingLevel":"default",
             "isDefault":true},
            {"id":"b","label":"b","provider":"future","thinkingLevel":"default",
             "thinkingLevels":["default","high"],"defaultThinkingLevel":"default"}
        ]}"#;
        let mut live = live_on(&[
            ("get_state", r#"{"sessionId":"s1"}"#),
            ("list_models", models),
        ])
        .await;
        live.to_launchpad();
        assert!(live.detached(), "the launchpad is the detached surface");
        assert_eq!(live.models.len(), 2, "the list installed");

        // Model: each press moves, and the ring comes back to where it started
        // after exactly one lap of the list it walks.
        let ring = live.models.len();
        live.cycle_pending_model();
        let start = live.pending_model().map(str::to_string);
        assert!(start.is_some(), "the first press staged nothing");
        let mut seen = vec![start.clone()];
        let mut steps = 0;
        loop {
            let before = live.pending_model().map(str::to_string);
            live.cycle_pending_model();
            steps += 1;
            let now = live.pending_model().map(str::to_string);
            assert_ne!(before, now, "a press did not move the chip");
            if now == start {
                break;
            }
            assert!(
                steps <= ring,
                "the ring did not come back to its start within one lap: {seen:?}"
            );
            seen.push(now);
        }
        assert_eq!(
            steps, ring,
            "the ring should visit every model once: {seen:?}"
        );

        // Level: the staged model's own ring, and it wraps too.
        live.cycle_pending_effort();
        let start = live.pending_effort().map(str::to_string);
        let mut levels = vec![start.clone()];
        for _ in 0..8 {
            let before = live.pending_effort().map(str::to_string);
            live.cycle_pending_effort();
            let now = live.pending_effort().map(str::to_string);
            assert_ne!(before, now, "a level press did not move");
            if now == start {
                break;
            }
            levels.push(now);
        }
        assert!(
            live.pending_effort().map(str::to_string) == start,
            "the level ring never came back to its start: {levels:?}"
        );

        // No models loaded at all: the chip says so instead of doing nothing.
        let mut bare = live_on(&[("get_state", r#"{"sessionId":"s1"}"#)]).await;
        bare.to_launchpad();
        bare.models.clear();
        bare.cycle_pending_model();
        assert!(
            bare.notice.is_some(),
            "an empty model list said nothing at all"
        );
        bare.client.disconnect();
        live.client.disconnect();
    }

    /// The queue row's two verbs: recalling puts the queued prompt back in the
    /// draft, cancelling explains itself — and both go through the agent, which
    /// is what makes them the queue's own verbs rather than the draft's.
    #[tokio::test]
    async fn a_queued_prompt_can_be_recalled_or_cancelled() {
        let queued = super::RpcSessionState {
            session_id: "s1".into(),
            queued_runs: vec![crate::rpc::types::QueuedRunState {
                run_id: "q-1".into(),
                display_text: "earlier question".into(),
                run_sequence: 1,
                client_request_id: "c-1".into(),
                state: "queued".into(),
                queue_position: 1,
                accepted_at: "2026-11-01T00:00:00Z".into(),
            }],
            queued_count: Some(1),
            ..Default::default()
        };
        let mut live = live_on(&[
            ("get_state", r#"{"sessionId":"s1"}"#),
            ("cancel_queued_run", "{}"),
        ])
        .await;
        live.install_state_refresh(Ok(queued));
        assert_eq!(live.state.queued_runs.len(), 1, "the queue installed");

        live.recall_queued("q-1").await;
        assert_eq!(
            live.draft.editor.text(),
            "earlier question",
            "recall did not put the prompt back in the draft"
        );

        live.install_state_refresh(Err("nothing".into()));
        live.cancel_queued("q-1").await;
        assert!(
            live.notice.is_some(),
            "the cancel neither refreshed nor explained itself"
        );
        live.client.disconnect();
    }

    /// The cycle and approval verbs on an **attached** session go to the agent:
    /// the launchpad stages them, a live conversation applies them.
    ///
    /// This is the other half of `the_launchpad_chips_cycle_and_wrap_without_sending`:
    /// the same three chips, on the side where "stage it" would be a lie.
    #[tokio::test]
    async fn an_attached_cycle_is_applied_not_staged() {
        use crate::approval_mode::ApprovalMode;

        let mut live = live_on(&[
            ("get_state", r#"{"sessionId":"s1","model":"future/old"}"#),
            ("cycle_model", r#"{"model":"future/new"}"#),
            ("cycle_thinking_level", r#"{"level":"high"}"#),
            ("set_sandbox_policy", r#"{"success":true}"#),
        ])
        .await;
        assert!(!live.detached(), "the fixture attaches a session");

        live.cycle_model().await;
        assert_eq!(
            live.state.model.as_deref(),
            Some("future/new"),
            "the agent's answer is the model now in force"
        );
        assert_eq!(
            live.pending_model(),
            Some("future/new"),
            "…and it is what the next session inherits"
        );

        live.cycle_thinking().await;
        assert_eq!(
            live.state.thinking_level, "high",
            "the level came from the agent's answer"
        );

        // The approval ring: every mode in turn, each one applied to the agent,
        // and the ring comes back to where it started after one lap.
        let start = live.shown_mode();
        let mut seen = Vec::new();
        for _ in 0..ApprovalMode::ALL.len() {
            let before = live.state.sandbox_tier.clone();
            live.cycle_mode().await;
            let after = live.state.sandbox_tier.clone();
            assert_ne!(before, after, "the approval ring did not move");
            seen.push(after.clone());
            assert!(
                live.notice.is_none(),
                "applying an approval mode reported an error: {:?}",
                live.notice
            );
        }
        assert_eq!(
            live.shown_mode(),
            start,
            "the approval ring did not wrap after one lap: {seen:?}"
        );

        // An unknown mode is refused with a notice, not sent.
        live.set_approval("not-a-mode").await;
        assert!(
            live.notice.is_some(),
            "an unknown approval mode was accepted silently"
        );
        live.client.disconnect();
    }

    /// A workspace the reader picks is the next session's directory on the
    /// launchpad, and the session's own workspace when one is attached — the two
    /// sides of the same verb.
    #[tokio::test]
    async fn picking_a_workspace_stages_it_or_moves_the_session() {
        let mut live = live_on(&[
            ("get_state", r#"{"sessionId":"s1","cwd":"/work/old"}"#),
            ("set_cwd", r#"{"success":true}"#),
        ])
        .await;

        // The directory the session already has is a no-op, not a round trip
        // with a notice.
        live.select_workspace("/work/old").await;
        assert!(
            live.notice.is_none(),
            "re-picking the current workspace spoke: {:?}",
            live.notice
        );

        // Attached: the agent moves the session, and the client remembers the
        // choice for the next start.
        live.select_workspace("/work/new/").await;
        assert!(
            live.notice.is_some(),
            "picking another workspace said nothing"
        );
        assert_eq!(
            live.take_workspace_to_remember().as_deref(),
            Some("/work/new"),
            "the choice is not remembered (the trailing slash is trimmed)"
        );

        // The launchpad stages it instead, and says so.
        live.to_launchpad();
        live.select_workspace("/work/staged").await;
        assert_eq!(
            live.pending_cwd.as_deref(),
            Some("/work/staged"),
            "the launchpad did not stage the directory"
        );

        // An empty path is not a choice.
        let before = live.pending_cwd.clone();
        live.select_workspace("   ").await;
        assert_eq!(
            live.pending_cwd, before,
            "an empty path moved the workspace"
        );
        live.client.disconnect();
    }

    /// `/new` on a session asks the agent for a fresh conversation and lands on
    /// it; the launchpad is what `/new` is *already* showing, so it stays put.
    #[tokio::test]
    async fn a_new_session_lands_on_the_agents_answer() {
        // The id is the agent's, and this client re-reads the state right after
        // asking for it — so the answer has to change between the two reads. A
        // static table would report the conversation the client just left.
        let table = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::from([
            ("get_state".to_string(), r#"{"sessionId":"s1"}"#.to_string()),
            (
                "get_session_entries".to_string(),
                r#"{"entries":[]}"#.to_string(),
            ),
            (
                "new_session".to_string(),
                r#"{"sessionId":"s-fresh"}"#.to_string(),
            ),
        ])));
        let (mock, _seen) = (
            crate::rpc::grpc_client::tests::api_mock_with_table(table.clone()),
            (),
        );
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s1"), false, None)
            .await
            .expect("attach to the mock agent");
        live.pending_cwd = Some("/work/staged".into());
        table
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-fresh"}"#.into());
        live.new_session().await;
        assert!(!live.detached(), "a fresh session is not the launchpad");
        assert_ne!(
            live.state.session_id, "s1",
            "the new session did not become the current one"
        );
        assert!(
            !live.state.session_id.is_empty(),
            "the client ended up attached to no session at all"
        );
        assert_eq!(
            live.state.session_id, "s-fresh",
            "the fresh id did not install"
        );
        assert!(
            live.pending_cwd.is_none(),
            "the staged workspace was not spent by the session it started"
        );
        assert!(
            live.notice.is_none(),
            "a fresh session reported an error: {:?}",
            live.notice
        );
        live.client.disconnect();
    }

    /// A failure from the agent is a notice, never a silent default: the model
    /// and level cycles say why they did not move.
    #[tokio::test]
    async fn a_refused_cycle_is_reported() {
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_failing(
            &[("get_state", r#"{"sessionId":"s1","model":"future/old"}"#)],
            "cycle_model",
            "no other model",
        );
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s1"), false, None)
            .await
            .expect("attach to the failing mock");
        live.cycle_model().await;
        assert!(
            live.notice.is_some(),
            "a refused cycle left the reader without a reason"
        );
        live.client.disconnect();

        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_failing(
            &[("get_state", r#"{"sessionId":"s1","model":"future/old"}"#)],
            "cycle_thinking_level",
            "no other level",
        );
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s1"), false, None)
            .await
            .expect("attach to the failing mock");
        live.cycle_thinking().await;
        assert!(
            live.notice.is_some(),
            "a refused level cycle left the reader without a reason"
        );
        live.client.disconnect();
    }

    /// The two events that become a desktop notification: an approval asking for
    /// a human, and an error. Everything else is the transcript's business, and
    /// a message with no body falls back to the kind's own sentence.
    #[tokio::test]
    async fn only_an_approval_or_an_error_becomes_a_notification() {
        use crate::notifications::NotifyKind;

        let live = live_on(&[("get_state", r#"{"sessionId":"s1"}"#)]).await;

        let approval = live
            .alert_for(&event(
                "approval_request",
                json!({"summary": "run: rm -rf build"}),
            ))
            .expect("an approval asks for a human");
        assert_eq!(approval.kind, NotifyKind::ApprovalNeeded);
        assert_eq!(approval.body, "run: rm -rf build");

        let error = live
            .alert_for(&event("error", json!({"error": "the model refused"})))
            .expect("an error is worth a notification");
        assert_eq!(error.kind, NotifyKind::Failed);
        assert_eq!(error.body, "the model refused");

        // No body: the kind's own sentence, not an empty bubble.
        let bare = live
            .alert_for(&event("approval_request", json!({})))
            .expect("still an approval");
        assert!(
            !bare.body.trim().is_empty(),
            "a notification with no body fell back to nothing"
        );

        // Everything else is the transcript's.
        for kind in [
            "text_chunk",
            "tool_start",
            "agent_end",
            "usage",
            "run_state",
        ] {
            assert!(
                live.alert_for(&event(kind, json!({"text": "x"}))).is_none(),
                "{kind} should not ring the desktop"
            );
        }
        live.client.disconnect();
    }

    /// The completion notice: its body is the last answer (cleaned), and the
    /// sentence that stands in when the transcript has nothing to quote. The
    /// "is this the last turn" gate is the queue, which the driver reads.
    #[tokio::test]
    async fn a_finished_run_quotes_the_answer_or_falls_back() {
        use crate::notifications::NotifyKind;

        let mut live = live_on(&[("get_state", r#"{"sessionId":"s1"}"#)]).await;

        // Nothing on screen yet: the fallback sentence, never an empty bubble.
        let bare = live.completion_notification();
        assert_eq!(bare.kind, NotifyKind::Completed);
        assert!(
            !bare.body.trim().is_empty(),
            "a completion with no answer fell back to nothing"
        );

        // With an answer on screen, the notice quotes it.
        live.chat
            .add_message(crate::components::chat_area::ChatMessage::new(
                "a-1".into(),
                crate::components::chat_area::ChatRole::Assistant,
                "the build is green",
            ));
        let quoted = live.completion_notification();
        assert_eq!(quoted.body, "the build is green");

        // The last-turn gate is the queue, empty here.
        assert!(live.queue_is_empty(), "an attach has nothing queued");
        let queued = super::RpcSessionState {
            session_id: "s1".into(),
            queued_runs: vec![crate::rpc::types::QueuedRunState {
                run_id: "q-1".into(),
                display_text: "next".into(),
                run_sequence: 1,
                client_request_id: "c-1".into(),
                state: "queued".into(),
                queue_position: 1,
                accepted_at: "2026-11-01T00:00:00Z".into(),
            }],
            queued_count: Some(1),
            ..Default::default()
        };
        live.install_state_refresh(Ok(queued));
        assert!(
            !live.queue_is_empty(),
            "a queued prompt must hold the completion notice back"
        );
        live.client.disconnect();
    }

    /// The adapter's own verbs: what `/x` means when the *surface* has not taken
    /// it first. A table, because these are one line each and the failure mode is
    /// a verb wired to the wrong one — `/rename` that renames nothing, `/export`
    /// that writes the conversation the reader left.
    #[tokio::test]
    async fn the_adapters_own_verbs_do_what_they_say() {
        // Attached, so the session-side arms are the ones under test.
        let attached = &[
            ("get_state", r#"{"sessionId":"s1","model":"future/old"}"#),
            ("get_session_entries", r#"{"entries":[]}"#),
            ("set_session_name", "{}"),
            ("export_html", r#"{"path":"/tmp/s1.html"}"#),
            ("abort", "{}"),
            ("list_sessions", r#"{"sessions":[]}"#),
            ("compact", r#"{"success":true}"#),
            ("set_tools", "{}"),
        ];
        let mut live = live_on(attached).await;

        // `/rename`: the name the request carried is installed, and the notice
        // says so rather than repeating the usage line.
        live.command("/rename a better title").await;
        assert_eq!(
            live.state.session_name.as_deref(),
            Some("a better title"),
            "/rename did not install the name it sent"
        );
        assert!(live.notice.is_some(), "/rename said nothing");

        // `/rename` with no argument is the usage line, not a rename to "".
        live.notice = None;
        live.command("/rename").await;
        assert!(live.notice.is_some(), "a bare /rename said nothing");

        // `/export` reports the path the agent wrote.
        live.notice = None;
        live.command("/export").await;
        let notice = live.notice.clone().unwrap_or_default();
        assert!(
            notice.contains("/tmp/s1.html"),
            "/export did not report the path it wrote: {notice:?}"
        );

        // `/copy` and `/mouse` are the surface's own; the adapter must not claim
        // them with a notice.
        live.notice = None;
        assert!(matches!(live.command("/copy").await, Outcome::Continue));
        assert!(
            live.notice.is_none(),
            "the adapter answered a verb the surface owns: {:?}",
            live.notice
        );

        // A verb nobody claims is named back, never silently dropped.
        live.command("/definitely-not-a-verb").await;
        let notice = live.notice.clone().unwrap_or_default();
        assert!(
            notice.contains("definitely-not-a-verb"),
            "an unknown verb was not named: {notice:?}"
        );

        // `/quit` is the one verb that ends the app.
        assert!(matches!(live.command("/quit").await, Outcome::Quit));
        live.client.disconnect();
    }

    /// The verbs that need a conversation say so on the launchpad instead of
    /// calling an agent that has no session to act on.
    #[tokio::test]
    async fn a_verb_that_needs_a_conversation_says_so_on_the_launchpad() {
        let mut live = live_on(&[("get_state", r#"{"sessionId":"s1"}"#)]).await;
        live.to_launchpad();
        for verb in ["/rename x", "/export", "/compact"] {
            live.notice = None;
            assert!(matches!(live.command(verb).await, Outcome::Continue));
            assert!(live.notice.is_some(), "{verb} on the launchpad was silent");
        }
        // `/new` from the launchpad is the launchpad: no notice, no round trip.
        live.notice = None;
        live.command("/new").await;
        assert!(live.detached(), "/new left the launchpad attached");
        live.client.disconnect();
    }

    /// The stream events the transcript reads, one row per kind: what each does
    /// to the activity row, the tool count, the queue or the transcript.
    #[tokio::test]
    async fn every_stream_event_kind_lands_somewhere_the_surface_shows() {
        let mut live = live_on(&[
            ("get_state", r#"{"sessionId":"s1"}"#),
            ("get_session_entries", r#"{"entries":[]}"#),
        ])
        .await;
        let current = live.state.session_id.clone();

        let stamped = |kind: &str, data: serde_json::Value| {
            let mut event = event(kind, data);
            event.session_id = Some(current.clone());
            event
        };

        // A turn starts: the activity row says it is writing.
        live.handle_event(stamped("agent_start", json!({})));
        assert!(
            live.activity_kind() != super::ActivityKind::Idle,
            "agent_start did not move the activity row"
        );

        // A tool call is counted once per call id, even though the agent
        // announces it twice (input, then execution).
        for phase in ["input", "execution"] {
            live.handle_event(stamped(
                "tool_start",
                json!({"tool_call_id": "t-1", "name": "bash", "phase": phase}),
            ));
        }
        assert_eq!(
            live.activity_snapshot().tools,
            1,
            "one call announced twice is two tools"
        );

        // Thinking and text are the transcript's.
        live.handle_event(stamped("thinking_start", json!({})));
        live.handle_event(stamped("thinking_delta", json!({"text": "hmm"})));
        live.handle_event(stamped("text_chunk", json!({"text": "the answer"})));
        live.handle_event(stamped("agent_end", json!({})));
        let rows = live.chat.render_all(80).join("\n");
        assert!(
            rows.contains("the answer"),
            "the answer never reached the rows: {rows:?}"
        );

        // An event stamped with another conversation is dropped, not applied to
        // the one on screen.
        let mut other = event("text_chunk", json!({"text": "another conversation"}));
        other.session_id = Some("s-elsewhere".into());
        live.handle_event(other);
        let rows = live.chat.render_all(80).join("\n");
        assert!(
            !rows.contains("another conversation"),
            "another conversation's text was painted here"
        );

        // An error is a notice row, and the run it names is not left claiming to
        // be running.
        live.handle_event(stamped("error", json!({"error": "the model refused"})));
        assert!(
            live.notice.is_some() || live.activity_kind() != super::ActivityKind::Idle,
            "an error event left nothing on screen"
        );
        live.client.disconnect();
    }

    /// Entering a worktree from the repository panel is moving the session: the
    /// worktree the reader is already in is a no-op, and a failure is a notice.
    #[tokio::test]
    async fn entering_a_worktree_moves_the_session_or_explains_itself() {
        let mut live = live_on(&[
            ("get_state", r#"{"sessionId":"s1","cwd":"/work/main"}"#),
            ("set_cwd", r#"{"success":true}"#),
        ])
        .await;

        // The checkout the session is already in: nothing to move, and nothing
        // said about it.
        live.notice = None;
        live.select_worktree("/work/main").await;
        assert!(live.notice.is_none(), "re-entering the same checkout spoke");

        live.select_worktree("/work/feature").await;
        assert_eq!(
            live.take_workspace_to_remember().as_deref(),
            Some("/work/feature"),
            "the worktree the reader entered is not the remembered workspace"
        );

        // A refused move is the agent's answer on the notice row.
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_failing(
            &[("get_state", r#"{"sessionId":"s1","cwd":"/work/main"}"#)],
            "set_cwd",
            "not a worktree",
        );
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut refused, _events) = Live::attach(&addr, Some("s1"), false, None)
            .await
            .expect("attach to the failing mock");
        refused.select_worktree("/work/gone").await;
        assert!(
            refused.notice.is_some(),
            "a refused worktree move was silent"
        );
        refused.client.disconnect();
        live.client.disconnect();
    }

    /// Creating a worktree is two steps (create, then move into it) and the
    /// notice carries the summary — or the reason it did not happen, from either
    /// step.
    #[tokio::test]
    async fn creating_a_worktree_reports_the_summary_or_the_reason() {
        use crate::worktree::{WorktreeCreated, WorktreePlan};

        let mut live = live_on(&[
            ("get_state", r#"{"sessionId":"s1","cwd":"/work/main"}"#),
            ("set_cwd", r#"{"success":true}"#),
        ])
        .await;

        let created = WorktreeCreated {
            plan: WorktreePlan {
                path: std::path::PathBuf::from("/work/feature-x"),
                branch: "feature-x".into(),
                name: "feature-x".into(),
            },
            base: Some("main".into()),
        };
        let summary = created.summary();
        live.create_worktree_with(|| Ok(created)).await;
        assert_eq!(
            live.notice.as_deref(),
            Some(summary.as_str()),
            "the summary of the created worktree is not what the reader sees"
        );

        // The creation itself failed.
        live.notice = None;
        live.create_worktree_with(|| Err("branch already exists".into()))
            .await;
        let notice = live.notice.clone().unwrap_or_default();
        assert!(
            notice.contains("branch already exists"),
            "a failed creation did not report the reason: {notice:?}"
        );

        // The checkout exists but the session could not move into it.
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_failing(
            &[("get_state", r#"{"sessionId":"s1"}"#)],
            "set_cwd",
            "cannot enter",
        );
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut stuck, _events) = Live::attach(&addr, Some("s1"), false, None)
            .await
            .expect("attach to the failing mock");
        stuck
            .create_worktree_with(|| {
                Ok(WorktreeCreated {
                    plan: WorktreePlan {
                        path: std::path::PathBuf::from("/work/feature-y"),
                        branch: "feature-y".into(),
                        name: "feature-y".into(),
                    },
                    base: None,
                })
            })
            .await;
        let notice = stuck.notice.clone().unwrap_or_default();
        assert!(
            notice.contains("cannot enter"),
            "a refused move into the new worktree was not reported: {notice:?}"
        );
        stuck.client.disconnect();
        live.client.disconnect();
    }

    /// Answering an approval with "always allow this" writes the rule the agent
    /// suggested, then the decision — and a prompt with no rule to write says so
    /// instead of pretending it saved one.
    #[tokio::test]
    async fn answering_with_a_rule_writes_it_before_the_decision() {
        let with_rule = r#"{"sessionId":"s1","pendingApprovals":[
            {"approvalRequestId":"a-1","toolName":"bash","kind":"command","riskLevel":"high",
             "saveSuggestion":{"rules":[{"path":"/work/alpha","access":"read"}]}}]}"#;
        let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(&[
            ("get_state", with_rule),
            ("add_session_rule", "{}"),
            ("approval_decision", "{}"),
        ]);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s1"), false, None)
            .await
            .expect("attach to the mock agent");
        assert!(live.approval().is_some(), "the fixture parks an approval");

        live.decide_with_rule(true, true).await;
        assert!(
            live.approval().is_none(),
            "an answered approval is still parked"
        );
        let asked = seen.lock().expect("seen").clone();
        let rule_at = asked.iter().position(|kind| kind == "add_session_rule");
        let decision_at = asked.iter().position(|kind| kind == "approval_decision");
        assert!(
            matches!((rule_at, decision_at), (Some(rule), Some(decision)) if rule < decision),
            "the rule is written before the decision it belongs to: {asked:?}"
        );
        live.client.disconnect();

        // No suggested rule: the reader is told, and nothing is decided.
        let no_rule = r#"{"sessionId":"s1","pendingApprovals":[
            {"approvalRequestId":"a-2","toolName":"bash","kind":"command","riskLevel":"high"}]}"#;
        let mut plain = live_on(&[("get_state", no_rule), ("approval_decision", "{}")]).await;
        plain.decide_with_rule(true, true).await;
        assert!(
            plain.notice.is_some(),
            "an approval with no rule to save was answered silently"
        );
        assert!(
            plain.approval().is_some(),
            "a refusal to save a rule still answered the prompt"
        );
        plain.client.disconnect();
    }

    /// The file panel's reads run off the loop and land on `poll_tree_read`: a
    /// rescan installs a tree, and toggling a folder installs the *same* tree
    /// with that folder's open state flipped.
    ///
    /// This is also the positive control for `poll_tree_read`'s `Disconnected`
    /// arm (see `docs/testing/coverage-waivers-2026-11/tui.md`): the two arms a
    /// reader can reach are asserted here by name, so the third is the only one
    /// left — and it needs the spawned thread to die before its unconditional
    /// `tx.send`.
    #[tokio::test]
    async fn the_file_panel_installs_a_scan_and_toggles_a_folder() {
        let mut live = live_on(&[(
            "get_state",
            concat!(
                r#"{"sessionId":"s1","cwd":""#,
                env!("CARGO_MANIFEST_DIR"),
                r#""}"#
            ),
        )])
        .await;

        // The tree is rooted by the workspace read (its children come from the
        // same listing the 文件 panel shows), so the fixture lands that first.
        // `local_reads` is cleared to force the scan: `refresh_local` only starts
        // one when the directory changed, and attach already read this one.
        live.local_reads = None;
        live.refresh_local();
        assert!(
            settle_local(&mut live).await,
            "the workspace scan never installed a tree"
        );
        assert!(
            !live.tree.rows().is_empty(),
            "the crate's own directory has no rows"
        );

        // A folder in this crate: `src`.
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let was_open = live.tree.is_open(&src);
        live.toggle_folder(&src.to_string_lossy());
        assert!(
            settle_tree(&mut live).await,
            "the toggle never installed its tree"
        );
        assert_eq!(
            live.tree.is_open(&src),
            !was_open,
            "toggling a folder did not flip its open state"
        );
        assert!(!live.tree.rows().is_empty(), "the toggle lost the tree");

        // A read that a newer one superseded is dropped, not painted over it.
        live.refresh_tree();
        live.refresh_tree();
        assert!(
            settle_tree(&mut live).await,
            "the newest scan never installed, so the superseded one won"
        );
        assert!(!live.tree.rows().is_empty());
        live.client.disconnect();
    }

    /// Wait until the workspace read has answered and installed its tree.
    async fn settle_local(live: &mut super::Live) -> bool {
        for _ in 0..400 {
            let _ = live.poll_local_reads();
            if !live.tree.rows().is_empty() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        false
    }

    /// Wait until the driver has taken the panel's own scan (`poll_tree_read`
    /// clears `tree_read` when it installs one, or says it was superseded).
    async fn settle_tree(live: &mut super::Live) -> bool {
        for _ in 0..400 {
            let _ = live.poll_local_reads();
            if live.tree_read.is_none() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        false
    }

    /// Every `WbAction` arm the surface can hand the adapter, one row each.
    ///
    /// The arms are one line apiece and a behaviour test would never name them,
    /// so a table is the only way this stays honest: each row says which mock
    /// answers the command it dispatches to and what the surface should then see.
    /// The arms that are deliberately the surface's own (a rectangle, a palette,
    /// a redraw) must leave the draft and the notice row **alone** — that is
    /// their assertion, not "it ran".
    #[tokio::test]
    async fn every_action_arm_dispatches_to_its_own_effect() {
        type Check = fn(&mut super::Live) -> bool;

        // (name, mock answers, action, check) — `None` means "an arm the surface
        // owns: it must leave the draft and the notice row exactly as they were".
        #[allow(clippy::type_complexity)]
        let rows: Vec<(&str, &[(&str, &str)], WbAction, Option<Check>)> = vec![
            (
                "a session step is the surface's own cursor",
                &[],
                WbAction::SessionStep(1),
                None,
            ),
            (
                "a diff selection is the surface's own",
                &[],
                WbAction::SelectDiff(0),
                None,
            ),
            (
                "opening the worktree box does not read anything here",
                &[],
                WbAction::OpenWorktree,
                None,
            ),
            (
                "the attachment preview is the driver's to open",
                &[],
                WbAction::PreviewAttachment,
                None,
            ),
            (
                "the language is the driver's to resolve and persist",
                &[],
                WbAction::SetLanguage(super::LanguagePreference::System),
                None,
            ),
            (
                "preferences are the driver's to persist",
                &[],
                WbAction::SetPreferences,
                None,
            ),
            (
                "the shortcut editor is the driver's too",
                &[],
                WbAction::SetKeybindings,
                None,
            ),
            (
                "a pin lands in this client's settings, not the agent's",
                &[],
                WbAction::SetSessionPinned {
                    id: "s1".into(),
                    pinned: true,
                },
                None,
            ),
            (
                "a focused pane is a layout fact",
                &[],
                WbAction::FocusPane(crate::workbench::PaneId::Chat),
                None,
            ),
            (
                "entering a worktree moves the session and is remembered",
                &[
                    ("get_state", r#"{"sessionId":"s1","cwd":"/work/main"}"#),
                    ("set_cwd", r#"{"success":true}"#),
                ],
                WbAction::SelectWorktree("/work/feature".into()),
                Some(|live| live.take_workspace_to_remember().as_deref() == Some("/work/feature")),
            ),
            (
                "an approval mode the reader picked is applied",
                &[
                    ("get_state", r#"{"sessionId":"s1"}"#),
                    ("set_sandbox_policy", r#"{"success":true}"#),
                ],
                WbAction::SetApproval("sandbox".into()),
                Some(|live| live.state.sandbox_tier.as_deref() == Some("sandbox")),
            ),
            (
                "a workspace the reader picked is applied and remembered",
                &[
                    ("get_state", r#"{"sessionId":"s1","cwd":"/work/old"}"#),
                    ("set_cwd", r#"{"success":true}"#),
                ],
                WbAction::SelectWorkspace("/work/new".into()),
                Some(|live| live.take_workspace_to_remember().as_deref() == Some("/work/new")),
            ),
            (
                "the plain chat is a workspace too",
                &[
                    ("get_state", r#"{"sessionId":"s1","cwd":"/work/old"}"#),
                    ("set_cwd", r#"{"success":true}"#),
                ],
                WbAction::SelectChatWorkspace,
                Some(|live| live.notice.is_some()),
            ),
            (
                "refreshing the files starts a read",
                &[(
                    "get_state",
                    concat!(
                        r#"{"sessionId":"s1","cwd":""#,
                        env!("CARGO_MANIFEST_DIR"),
                        r#""}"#
                    ),
                )],
                WbAction::RefreshFiles,
                Some(|live| live.tree_read.is_some()),
            ),
            (
                "a preview reads the file off the loop",
                &[("get_state", r#"{"sessionId":"s1"}"#)],
                WbAction::PreviewFile("/etc/hosts".into()),
                Some(|live| live.notice.is_none()),
            ),
            (
                "a picture preview steps through this conversation's pictures",
                &[("get_state", r#"{"sessionId":"s1"}"#)],
                WbAction::PreviewChatFile("/tmp/missing.png".into()),
                Some(|live| live.notice.is_none()),
            ),
            (
                "a tool set is sent, and the notice says so",
                &[("get_state", r#"{"sessionId":"s1"}"#), ("set_tools", "{}")],
                WbAction::SetTools(vec!["read".into()]),
                Some(|live| live.notice.is_some()),
            ),
            (
                "cancelling a queued prompt goes to the agent",
                &[
                    ("get_state", r#"{"sessionId":"s1"}"#),
                    ("cancel_queued_run", "{}"),
                ],
                WbAction::CancelQueued("q-1".into()),
                Some(|live| live.notice.is_some()),
            ),
            (
                "a model the reader picked is applied",
                &[
                    ("get_state", r#"{"sessionId":"s1"}"#),
                    ("set_model", r#"{"model":"future/new"}"#),
                ],
                WbAction::SetModel("future/new".into()),
                // `state.model` comes back from the agent's next `get_state`; the
                // staged value is what this call installs.
                Some(|live| live.pending_model() == Some("future/new")),
            ),
            (
                "a level the reader picked is applied",
                &[
                    ("get_state", r#"{"sessionId":"s1"}"#),
                    ("set_thinking_level", r#"{"level":"high"}"#),
                ],
                WbAction::SetThinking("high".into()),
                Some(|live| live.pending_effort() == Some("high")),
            ),
            (
                "a compaction the agent refuses is a notice, not a takeover",
                &[
                    ("get_state", r#"{"sessionId":"s1"}"#),
                    ("compact", r#"{"accepted":false}"#),
                ],
                WbAction::Compact(String::new()),
                Some(|live| !live.compacting() && live.notice.is_some()),
            ),
            (
                "forking from a row lands on the new conversation",
                &[
                    ("get_state", r#"{"sessionId":"s1"}"#),
                    ("fork", r#"{"sessionId":"s-fork"}"#),
                    ("get_session_entries", r#"{"entries":[]}"#),
                    ("list_sessions", r#"{"sessions":[]}"#),
                ],
                WbAction::ForkAt("e-1".into()),
                Some(|live| !live.detached()),
            ),
            (
                "loading an older page without a driver reads it here",
                &[
                    (
                        "get_state",
                        r#"{"sessionId":"s1","activeRun":{"runId":"r","epoch":1,"state":"done"}}"#,
                    ),
                    (
                        "get_session_entries",
                        r#"{"entries":[],"hasMore":true,"nextOffset":0}"#,
                    ),
                    ("session_older_page", r#"{"entries":[]}"#),
                ],
                WbAction::LoadOlder,
                Some(|live| live.notice.is_none()),
            ),
        ];

        for (what, pairs, action, check) in rows {
            let mut live = live_on(pairs).await;
            // The surface the arm must leave alone, captured before it runs.
            let before = (live.notice.clone(), live.draft.editor.text().to_string());
            let outcome = live.apply(action).await;
            assert!(
                matches!(outcome, Outcome::Continue),
                "{what}: this arm must not end the app"
            );
            match check {
                Some(check) => assert!(check(&mut live), "{what}"),
                None => assert_eq!(
                    (live.notice.clone(), live.draft.editor.text().to_string()),
                    before,
                    "{what}: an arm the surface owns changed the draft or the notice"
                ),
            }
            live.client.disconnect();
        }
    }

    /// The typed verbs that reach the adapter (the surface takes the rest before
    /// they arrive), one row each — the parsing, the list-shaped argument and the
    /// refusal all have a sentence the reader sees.
    #[tokio::test]
    async fn every_adapter_verb_parses_and_answers() {
        type Check = fn(&mut super::Live) -> bool;
        #[allow(clippy::type_complexity)]
        let rows: Vec<(&str, &[(&str, &str)], &str, Option<Check>)> = vec![
            (
                "`/sessions` re-reads the rail",
                &[
                    ("get_state", r#"{"sessionId":"s1"}"#),
                    ("list_sessions", r#"{"sessions":[]}"#),
                ],
                "/sessions",
                None,
            ),
            (
                "`/worktree new x` says it is creating one",
                &[("get_state", r#"{"sessionId":"s1"}"#)],
                "/worktree new feat/x",
                Some(|live| live.notice.is_some()),
            ),
            (
                "`/worktree` with a name that is not `new` shows the usage line",
                &[("get_state", r#"{"sessionId":"s1"}"#)],
                "/worktree feat/x",
                Some(|live| live.notice.is_some()),
            ),
            (
                "`/tools a,b` splits the list and sends it",
                &[("get_state", r#"{"sessionId":"s1"}"#), ("set_tools", "{}")],
                "/tools read, shell",
                Some(|live| live.notice.is_some()),
            ),
            (
                "a refused `/tools` is the failure sentence",
                &[("get_state", r#"{"sessionId":"s1"}"#)],
                "/tools read",
                Some(|live| live.notice.is_some()),
            ),
            (
                "`/model x` applies the model",
                &[
                    ("get_state", r#"{"sessionId":"s1"}"#),
                    ("set_model", r#"{"model":"future/new"}"#),
                ],
                "/model future/new",
                Some(|live| live.pending_model() == Some("future/new")),
            ),
            (
                "`/model` cycles the agent's own order",
                &[
                    ("get_state", r#"{"sessionId":"s1"}"#),
                    ("cycle_model", r#"{"model":"future/next"}"#),
                ],
                "/model",
                Some(|live| live.pending_model() == Some("future/next")),
            ),
            (
                "a refused `/export` is a sentence, not a path",
                &[("get_state", r#"{"sessionId":"s1"}"#)],
                "/export",
                Some(|live| live.notice.is_some()),
            ),
            (
                "`/fork` and `/history` are the surface's own",
                &[("get_state", r#"{"sessionId":"s1"}"#)],
                "/history",
                None,
            ),
            (
                "the choosers the surface opens are not unknown verbs",
                &[("get_state", r#"{"sessionId":"s1"}"#)],
                "/thinking",
                None,
            ),
        ];

        for (what, pairs, verb, check) in rows {
            let mut live = live_on(pairs).await;
            let before = (live.notice.clone(), live.draft.editor.text().to_string());
            assert!(
                matches!(live.command(verb).await, Outcome::Continue),
                "{what}: a verb must not end the app"
            );
            match check {
                Some(check) => assert!(check(&mut live), "{what}"),
                None => assert_eq!(
                    (live.notice.clone(), live.draft.editor.text().to_string()),
                    before,
                    "{what}: the adapter spoke about a verb it does not own"
                ),
            }
            live.client.disconnect();
        }
    }

    /// The events the transcript turns into rows *within* a run: a streaming tool
    /// call, its final text, and the three compaction outcomes.
    #[tokio::test]
    async fn a_running_turn_updates_its_own_rows() {
        let mut live = live_on(&[
            ("get_state", r#"{"sessionId":"s1"}"#),
            ("get_session_entries", r#"{"entries":[]}"#),
        ])
        .await;
        let current = live.state.session_id.clone();
        let stamped = |kind: &str, data: serde_json::Value| {
            let mut event = event(kind, data);
            event.session_id = Some(current.clone());
            event
        };

        // A tool call streaming its arguments keeps one row, not one per delta —
        // and each of the three events owes a frame (the revision is what the
        // loop repaints on), which is the surface-visible half of that.
        let mut revision = live.revision;
        for (kind, data) in [
            (
                "tool_start",
                json!({"tool_call_id": "t-1", "name": "bash", "phase": "input"}),
            ),
            (
                "tool_delta",
                json!({"tool_call_id": "t-1", "text": "ls -la"}),
            ),
            (
                "tool_end",
                json!({"tool_call_id": "t-1", "ok": true, "output": "done"}),
            ),
        ] {
            live.handle_event(stamped(kind, data));
            assert_ne!(
                live.revision, revision,
                "{kind} did not owe the surface a frame"
            );
            revision = live.revision;
        }
        assert_eq!(
            live.activity_snapshot().tools,
            1,
            "one tool call, counted once"
        );

        // The three compaction outcomes: committed ends the takeover, unchanged
        // says nothing changed, failed says why.
        live.begin_compaction();
        assert!(live.compacting(), "the probe did not take the session over");
        live.handle_event(stamped(
            "compaction_committed",
            json!({"tokensSaved": 1200}),
        ));
        assert!(
            !live.compacting(),
            "a committed compaction left the session held"
        );

        live.notice = None;
        live.handle_event(stamped("compaction_unchanged", json!({})));
        assert!(
            live.notice.is_some(),
            "an unchanged compaction said nothing at all"
        );

        live.notice = None;
        live.handle_event(stamped(
            "compaction_failed",
            json!({"error": "the model refused"}),
        ));
        let notice = live.notice.clone().unwrap_or_default();
        assert!(
            notice.contains("the model refused"),
            "a failed compaction dropped the agent's reason: {notice:?}"
        );

        // An unknown kind is not a panic and not a painted row.
        live.handle_event(stamped("not_a_kind_at_all", json!({})));
        live.client.disconnect();
    }

    /// The launchpad drops stream events: the transcript was cleared on purpose,
    /// and the previous session's reply must not narrate into it.
    #[tokio::test]
    async fn the_launchpad_ignores_stream_events() {
        let mut live = live_on(&[("get_state", r#"{"sessionId":"s1"}"#)]).await;
        let before = live.chat.render_all(100).len();
        live.to_launchpad();
        live.handle_event(event(
            "text_chunk",
            json!({"text": "from the session I left"}),
        ));
        let rows = live.chat.render_all(100).join("\n");
        assert!(
            !rows.contains("from the session I left"),
            "the launchpad painted the conversation the reader left: {rows:?}"
        );
        assert!(
            live.activity_kind() == super::ActivityKind::Idle,
            "a dropped event still moved the activity row"
        );
        let _ = before;
        live.client.disconnect();
    }

    /// The clipboard reader's answers, one row each: a picture reaches the draft
    /// as an attachment, a refusal says why, and an empty refusal falls back to
    /// the plain "nothing on the clipboard" sentence.
    ///
    /// `ClipboardCapture::with_runner` is the paste module's own seam (the real
    /// reader shells out to the platform's tools), so every branch here is driven
    /// by a fake tool rather than by a broken host — which is what turned this
    /// from a W3 candidate into an ordinary test.
    #[tokio::test]
    async fn the_clipboard_pastes_a_picture_or_says_why_it_cannot() {
        use crate::paste::ClipboardCapture;

        // A 1x1 PNG: what an image probe would leave in the file it was told to
        // write. The runner finds that path in the probe's own arguments, the way
        // the real tool does.
        const PNG: &[u8] = &[
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, b'I', b'H', b'D', b'R',
            0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89, 0, 0, 0, 0x0a, b'I',
            b'D', b'A', b'T', 0x78, 0x9c, 0x63, 0, 0, 0, 2, 0, 1, 0x1b, 0x8f, 0x0f, 0x7d, 0, 0, 0,
            0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
        ];

        let mut live = live_on(&[("get_state", r#"{"sessionId":"s1"}"#)]).await;
        let dir = tempfile::tempdir().expect("tempdir");

        // An image: the probe writes the file it was handed, and the adapter
        // attaches it through the same road a dragged-in picture takes.
        live.clipboard = ClipboardCapture::with_runner(
            Box::new(|_program: &str, args: &[String]| {
                // The path may be a bare argument or embedded in a script the
                // probe runs; find it either way and write there, the way the
                // real tool does.
                for arg in args {
                    if let Some(at) = arg.find(".png") {
                        let tail = &arg[..at + 4];
                        let start = tail.rfind(['"', '\'', ' ']).map(|i| i + 1).unwrap_or(0);
                        std::fs::write(&tail[start..], PNG).map_err(|e| e.to_string())?;
                    }
                }
                Ok((0, String::new(), String::new()))
            }),
            dir.path().to_path_buf(),
        );
        live.paste_clipboard().await;
        // The draft carries the attachment marker; the picture itself is in the
        // pending attachments, the same list a dragged-in file fills.
        let attachments: Vec<String> = live
            .draft
            .editor
            .pending()
            .attachments()
            .iter()
            .map(|attachment| attachment.path.clone())
            .collect();
        assert_eq!(
            attachments.len(),
            1,
            "an image on the clipboard did not attach: {:?}",
            live.draft.editor.text()
        );
        assert!(
            attachments[0].ends_with(".png"),
            "the attachment is not the file the probe wrote: {attachments:?}"
        );

        // Text on the clipboard. The probes are distinguished by what the
        // *measured* table looks like (2026-11-01, macOS): the image probe is
        // `osascript` with the output path embedded in its script, the text
        // command is `pbpaste` with no arguments — so "the arguments name a
        // .png" is the image probe, and everything else is the text tool.
        live.clipboard = ClipboardCapture::with_runner(
            Box::new(|_program: &str, args: &[String]| {
                if args.iter().any(|arg| arg.contains(".png")) {
                    Ok((1, String::new(), String::new()))
                } else {
                    Ok((0, "clipboard text".to_string(), String::new()))
                }
            }),
            dir.path().to_path_buf(),
        );
        live.notice = Some("stale".into());
        live.paste_clipboard().await;
        // The draft still carries the picture's marker from the row above; a
        // paste appends, so the text is what it now ends with.
        let draft = live.draft.editor.text().to_string();
        assert!(
            draft.ends_with("clipboard text"),
            "the clipboard's text did not reach the draft: {draft:?} (notice: {:?})",
            live.notice
        );
        assert!(
            live.notice.is_none(),
            "a successful text paste left a notice: {:?}",
            live.notice
        );

        // A refusal with a reason: the reason is the reader's sentence.
        live.clipboard = ClipboardCapture::with_runner(
            Box::new(|_program: &str, _args: &[String]| {
                Err("no clipboard tool for this host".into())
            }),
            dir.path().to_path_buf(),
        );
        live.notice = None;
        live.paste_clipboard().await;
        let notice = live.notice.clone().unwrap_or_default();
        assert!(
            notice.contains("no clipboard tool"),
            "a refused paste dropped the reason: {notice:?}"
        );

        // A refusal with *no* reason: the plain empty-clipboard sentence, never a
        // blank notice row. (Production's own runner always fills the reason in;
        // this is the guard for the one that could not.)
        live.clipboard = ClipboardCapture::with_runner(
            Box::new(|_program: &str, _args: &[String]| Err(String::new())),
            dir.path().to_path_buf(),
        );
        live.notice = None;
        live.paste_clipboard().await;
        assert!(
            live.notice.is_some(),
            "a refusal with no reason left the notice row empty"
        );

        // Nothing usable on the clipboard: the reader is told, not left guessing.
        live.clipboard = ClipboardCapture::with_runner(
            Box::new(|_program: &str, _args: &[String]| Ok((1, String::new(), String::new()))),
            dir.path().to_path_buf(),
        );
        live.notice = None;
        live.paste_clipboard().await;
        assert!(
            live.notice.is_some(),
            "an empty clipboard said nothing at all"
        );
        live.client.disconnect();
    }

    /// Opening a link hands it to the platform's opener, and an opener that
    /// cannot run is a notice rather than a lost click.
    ///
    /// `#[cfg(unix)]`: the stub is a shell script on `PATH`. Windows' opener is a
    /// `cmd` builtin, so that half is unmeasured here (`platform-unmeasured`, a
    /// measurement gap — not a waiver).
    #[cfg(unix)]
    #[tokio::test]
    async fn opening_a_link_spawns_the_opener_and_reports_a_missing_one() {
        use std::os::unix::fs::PermissionsExt as _;

        let mut live = live_on(&[("get_state", r#"{"sessionId":"s1"}"#)]).await;
        let dir = tempfile::tempdir().expect("tempdir");
        // Both openers a unix host can pick: `open` on macOS, `xdg-open`
        // elsewhere. The platform chooses inside `open_external`.
        for name in ["open", "xdg-open"] {
            let script = dir.path().join(name);
            std::fs::write(&script, "#!/bin/sh\nexit 0\n").expect("write the stub");
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("chmod the stub");
        }

        // `PATH` is process-global: take the crate's lock, and put it back with
        // the guard even if the assertion below panics.
        let _guard = crate::test_env::lock();
        let _path = crate::test_env::EnvVarGuard::set("PATH", dir.path());
        live.notice = None;
        live.open_url("https://example.com/stub");
        assert!(
            live.notice.is_none(),
            "a link the opener accepted reported an error: {:?}",
            live.notice
        );

        // No opener on `PATH` at all.
        let empty = tempfile::tempdir().expect("tempdir");
        let _path = crate::test_env::EnvVarGuard::set("PATH", empty.path());
        live.notice = None;
        live.open_url("https://example.com/stub");
        assert!(
            live.notice.is_some(),
            "a link with no opener on PATH failed silently"
        );
        live.client.disconnect();
    }

    /// `/continue`-style attach: the session list is re-read and the newest row
    /// is opened — and when the newest row is already on screen, nothing is
    /// re-read or re-painted.
    #[tokio::test]
    async fn attaching_to_the_most_recent_opens_the_newest_row() {
        let sessions = r#"{"sessions":[
            {"id":"s-old","cwd":"/tmp","updatedAtMs":1700000001000,"model":"m"},
            {"id":"s-new","cwd":"/tmp","updatedAtMs":1700000009000,"model":"m"}
        ]}"#;
        // The switch re-reads the state, so the fixture's answer has to move to
        // the row being opened (a static table would report the one it left).
        let table = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::from([
            (
                "get_state".to_string(),
                r#"{"sessionId":"s-old"}"#.to_string(),
            ),
            (
                "get_session_entries".to_string(),
                r#"{"entries":[]}"#.to_string(),
            ),
            ("list_sessions".to_string(), sessions.to_string()),
            ("get_state_of".to_string(), "{}".to_string()),
        ])));
        let (mock, _seen) = (
            crate::rpc::grpc_client::tests::api_mock_with_table(table.clone()),
            (),
        );
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s-old"), false, None)
            .await
            .expect("attach to the mock agent");
        table
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s-new"}"#.into());
        live.attach_most_recent().await;
        assert_eq!(
            live.state.session_id, "s-new",
            "the newest row is not the one that opened"
        );

        // Already on the newest one: the read happens, the switch does not.
        live.attach_most_recent().await;
        assert_eq!(
            live.state.session_id, "s-new",
            "a second attach moved off the newest conversation"
        );
        live.client.disconnect();
    }

    /// Deleting the conversation on screen hands the screen to the row that slid
    /// into its place, falls back to the launchpad when it was the only one, and
    /// a refused delete is the notice — nothing is closed under the reader.
    #[tokio::test]
    async fn deleting_a_conversation_moves_to_its_successor() {
        let two = r#"{"sessions":[
            {"id":"s1","cwd":"/tmp","updatedAtMs":1700000002000,"model":"m"},
            {"id":"s2","cwd":"/tmp","updatedAtMs":1700000001000,"model":"m"}
        ]}"#;
        // The list the agent has *after* the delete: one row left.
        let after = r#"{"sessions":[
            {"id":"s2","cwd":"/tmp","updatedAtMs":1700000001000,"model":"m"}
        ]}"#;
        let table = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::from([
            ("get_state".to_string(), r#"{"sessionId":"s1"}"#.to_string()),
            (
                "get_session_entries".to_string(),
                r#"{"entries":[]}"#.to_string(),
            ),
            ("list_sessions".to_string(), two.to_string()),
            (
                "delete_session".to_string(),
                r#"{"success":true}"#.to_string(),
            ),
        ])));
        let (mock, _seen) = (
            crate::rpc::grpc_client::tests::api_mock_with_table(table.clone()),
            (),
        );
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut live, _events) = Live::attach(&addr, Some("s1"), false, None)
            .await
            .expect("attach to the mock agent");
        // The delete re-reads the list before it opens anything.
        table
            .lock()
            .unwrap()
            .insert("list_sessions".into(), after.to_string());
        // The switch to the successor re-reads the state as well.
        table
            .lock()
            .unwrap()
            .insert("get_state".into(), r#"{"sessionId":"s2"}"#.into());
        live.delete_session("s1").await;
        assert_eq!(
            live.state.session_id, "s2",
            "the row that took the deleted one's place is not on screen"
        );

        // The last remaining conversation: the launchpad, with no id left.
        let empty = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::from([
            ("get_state".to_string(), r#"{"sessionId":"s2"}"#.to_string()),
            (
                "get_session_entries".to_string(),
                r#"{"entries":[]}"#.to_string(),
            ),
            ("list_sessions".to_string(), after.to_string()),
            (
                "delete_session".to_string(),
                r#"{"success":true}"#.to_string(),
            ),
        ])));
        let (mock, _seen) = (
            crate::rpc::grpc_client::tests::api_mock_with_table(empty.clone()),
            (),
        );
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut last, _events) = Live::attach(&addr, Some("s2"), false, None)
            .await
            .expect("attach to the mock agent");
        empty
            .lock()
            .unwrap()
            .insert("list_sessions".into(), r#"{"sessions":[]}"#.into());
        last.delete_session("s2").await;
        assert!(last.detached(), "deleting the last one left a session open");
        assert!(
            last.state.session_id.is_empty(),
            "the deleted id is still the current one: {:?}",
            last.state.session_id
        );

        // A refused delete: the conversation stays, and the reader is told.
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_failing(
            &[
                ("get_state", r#"{"sessionId":"s1"}"#),
                ("get_session_entries", r#"{"entries":[]}"#),
                ("list_sessions", two),
            ],
            "delete_session",
            "the run is in flight",
        );
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut refused, _events) = Live::attach(&addr, Some("s1"), false, None)
            .await
            .expect("attach to the failing mock");
        refused.delete_session("s1").await;
        let notice = refused.notice.clone().unwrap_or_default();
        assert!(
            notice.contains("the run is in flight"),
            "a refused delete dropped the agent's reason: {notice:?}"
        );
        assert_eq!(
            refused.state.session_id, "s1",
            "a refused delete moved off the conversation anyway"
        );
        refused.client.disconnect();
        last.client.disconnect();
        live.client.disconnect();
    }

    /// The preview of a draft attachment reads the last picture in it; with no
    /// picture the reader is told there is nothing to show rather than being
    /// given an empty box.
    #[tokio::test]
    async fn previewing_an_attachment_reads_the_last_picture_in_the_draft() {
        let mut live = live_on(&[("get_state", r#"{"sessionId":"s1"}"#)]).await;

        // Nothing attached: the "nothing to preview" sentence, and no box.
        assert!(!live.load_attachment(), "an empty draft claimed a picture");
        assert!(
            live.notice.is_some(),
            "an empty draft said nothing about the missing picture"
        );

        // Two pictures: the last one is the one the box opens on. They are real
        // files — the editor sniffs the bytes before it attaches a path, which
        // is what keeps a pasted `/tmp/notes.txt` from becoming a thumbnail.
        const PNG: &[u8] = &[
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, b'I', b'H', b'D', b'R',
            0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89, 0, 0, 0, 0x0a, b'I',
            b'D', b'A', b'T', 0x78, 0x9c, 0x63, 0, 0, 0, 2, 0, 1, 0x1b, 0x8f, 0x0f, 0x7d, 0, 0, 0,
            0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
        ];
        let dir = tempfile::tempdir().expect("tempdir");
        let first = dir.path().join("first.png");
        let second = dir.path().join("second.png");
        std::fs::write(&first, PNG).expect("write the first picture");
        std::fs::write(&second, PNG).expect("write the second picture");
        let mut editor = live.draft.editor.clone();
        editor.paste(&first.to_string_lossy());
        editor.paste(&second.to_string_lossy());
        live.draft = super::InputView::from_editor(editor, live.draft.placeholder.clone());
        assert!(
            live.load_attachment(),
            "a draft with pictures claimed there was none"
        );
        live.client.disconnect();
    }

    /// A mention inside the draft: quotes and backslash escapes are part of the
    /// word, plain whitespace ends it, and the walk reports where it stopped so
    /// the caller can continue from there.
    ///
    /// `mention_word` measured zero: it is only reached through the `@` menu's
    /// own completion, which needs a live editor round trip. It is a pure
    /// function over the text, so the table is the honest fixture.
    #[test]
    fn a_mention_word_is_walked_with_its_quotes_and_escapes() {
        // A bare word stops at the space; raw text and value agree.
        let (end, raw, value) = super::mention_word("src/main.rs rest", 0);
        assert_eq!(
            (end, raw, value.as_str()),
            (11, "src/main.rs", "src/main.rs")
        );
        // A quoted path keeps its spaces; the value drops the quotes.
        let (end, raw, value) = super::mention_word("\"a file with spaces.rs\" x", 0);
        assert_eq!(value, "a file with spaces.rs");
        let text = "\"a file with spaces.rs\" x";
        assert_eq!(
            &text[end..],
            " x",
            "the walk stopped inside the closing quote"
        );
        assert_eq!(
            raw, "\"a file with spaces.rs\"",
            "the raw slice keeps the quotes"
        );
        // A backslash escapes the next character, inside the quotes: the value
        // keeps the quote and loses the backslash.
        let (_, _, value) = super::mention_word("\"a \\\"quoted\\\" name\"", 0);
        assert_eq!(value, "a \"quoted\" name");
        // A trailing backslash with nothing after it is kept as a character.
        let (_, _, value) = super::mention_word("\"ends in \\", 0);
        assert_eq!(value, "ends in \\");
        // The walk starts where it is told, and reports the end it reached.
        let text = "look at @src/lib.rs";
        let at = text.find('@').expect("the mention") + 1;
        let (end, word, _rest) = super::mention_word(text, at);
        assert_eq!(word, "src/lib.rs");
        assert_eq!(
            end,
            text.len(),
            "the walk stopped before the end of the word"
        );
        // An empty text is an empty word, and the walk does not move.
        let (end, raw, value) = super::mention_word("", 0);
        assert_eq!((end, raw, value.as_str()), (0, "", ""));
    }

    /// A run's usage: the tokens a turn spent are folded into the session's
    /// context gauge as the events arrive, and the run's own rate is frozen when
    /// the run ends rather than counting from the clock afterwards.
    #[tokio::test]
    async fn a_runs_usage_is_folded_into_the_context_gauge() {
        let mut live = live_on(&[("get_state", r#"{"sessionId":"s1"}"#)]).await;
        let current = live.state.session_id.clone();
        let stamped = |kind: &str, data: serde_json::Value| {
            let mut event = event(kind, data);
            event.session_id = Some(current.clone());
            event.run_id = Some("run-1".into());
            event
        };

        // A model step reports its spend on the `usage` event.
        // The event carries the step's spend under `usage`.
        live.handle_event(stamped(
            "usage",
            json!({"usage": {"prompt_tokens": 120, "completion_tokens": 30}}),
        ));
        assert_eq!(
            live.state.context_tokens,
            Some(150),
            "the step's tokens did not reach the context gauge"
        );
        assert!(
            live.run_output_tokens >= 30,
            "the completion tokens were not counted against the run: {}",
            live.run_output_tokens
        );

        // The end of the run folds its own total in, and the rate stops moving.
        let before = live.run_output_tokens;
        live.handle_event(stamped(
            "agent_end",
            json!({"usage": {"prompt_tokens": 200, "completion_tokens": 40}}),
        ));
        assert!(
            live.run_output_tokens >= before,
            "the end-of-run usage did not add to the run's total"
        );
        let frozen = live.run_tps;
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert_eq!(
            live.run_tps, frozen,
            "the rate kept moving after the run ended"
        );
        live.client.disconnect();
    }

    /// A failed turn is announced once, and only for a run this client started:
    /// the end of somebody else's run is not this surface's news.
    #[tokio::test]
    async fn only_this_clients_failed_run_raises_its_notification() {
        let mut live = live_on(&[("get_state", r#"{"sessionId":"s1"}"#)]).await;
        let current = live.state.session_id.clone();

        // An end that is not a failure is not this notification.
        let ok_end = event("agent_end", json!({"state": "done"}));
        assert!(!live.take_failure_notification(&ok_end));

        // A failure for a run this client did not submit: nothing to announce.
        let mut stranger = event("agent_end", json!({"state": "failed"}));
        stranger.session_id = Some(current.clone());
        stranger.run_id = Some("run-other".into());
        assert!(
            !live.take_failure_notification(&stranger),
            "another run's failure was announced here"
        );

        // A run this client did submit: the failure is announced, once.
        live.completion_candidates.push_back("run-1".to_string());
        let mine = {
            let mut event = event("agent_end", json!({"state": "failed"}));
            event.session_id = Some(current);
            event.run_id = Some("run-1".into());
            event
        };
        assert!(
            live.take_failure_notification(&mine),
            "the failure was not taken"
        );
        assert!(
            !live.take_failure_notification(&mine),
            "the same failure was announced twice"
        );
        live.client.disconnect();
    }

    /// Reading a picture for the preview box: a missing file, one over the cap
    /// and one that grew between the stat and the read each get their own
    /// sentence — the reader is told which happened, not just "it failed".
    #[test]
    fn a_preview_read_names_the_reason_it_cannot_show_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");

        // A file that is not there.
        let missing = dir.path().join("gone.png");
        let error = super::read_image_bytes(&missing.to_string_lossy(), super::Language::En)
            .expect_err("a missing file cannot be read");
        assert!(!error.trim().is_empty(), "a missing file reported nothing");

        // A picture: the bytes come back as they are.
        let small = dir.path().join("small.png");
        std::fs::write(&small, b"\x89PNG\r\n\x1a\n tiny").expect("write the picture");
        let bytes = super::read_image_bytes(&small.to_string_lossy(), super::Language::En)
            .expect("a small picture reads");
        assert_eq!(bytes, b"\x89PNG\r\n\x1a\n tiny");

        // One over the cap: refused by the *metadata*, before a byte is
        // allocated, and the sentence names both sizes in megabytes.
        let huge = dir.path().join("huge.png");
        let file = std::fs::File::create(&huge).expect("create the file");
        file.set_len(super::MAX_PREVIEW_IMAGE_BYTES + 1024)
            .expect("sparse file");
        let error = super::read_image_bytes(&huge.to_string_lossy(), super::Language::En)
            .expect_err("an oversized picture is refused");
        assert!(
            error.contains('1') || error.to_lowercase().contains("mb") || !error.is_empty(),
            "the size refusal did not name the sizes: {error:?}"
        );
    }

    /// The diff's selection: the row asked for, else the first one — a selection
    /// that moved past a shorter list still names a file rather than nothing.
    #[tokio::test]
    async fn a_diff_selection_falls_back_to_the_first_file() {
        use crate::workbench::state::DiffView;

        let mut live = live_on(&[("get_state", r#"{"sessionId":"s1"}"#)]).await;
        live.set_diff(DiffView {
            files: vec![
                crate::workbench::diff_review::DiffFile {
                    path: "a.rs".into(),
                    status: "M".into(),
                },
                crate::workbench::diff_review::DiffFile {
                    path: "b.rs".into(),
                    status: "A".into(),
                },
            ],
            selected: 0,
            patch: String::new(),
            error: None,
        });

        assert_eq!(live.selected_diff_path(1), "b.rs");
        // Past the end: the first file, not an empty path.
        assert_eq!(live.selected_diff_path(9), "a.rs");
        // No diff at all: empty, which the caller treats as "nothing selected".
        live.diff = None;
        assert_eq!(live.selected_diff_path(0), "");
        live.client.disconnect();
    }

    /// The arms `apply`, `command` and `handle_event` still had open, each one a
    /// place where the *failure* is the interesting half: a refused tool set, a
    /// refused rename, a refused export, and the events that clear a card
    /// somebody else answered.
    #[tokio::test]
    async fn the_residual_arms_report_their_failures_and_clear_what_was_answered() {
        // A refused tool set: the sentence names the reason.
        let mut live = live_on(&[("get_state", r#"{"sessionId":"s1"}"#)]).await;
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_failing(
            &[("get_state", r#"{"sessionId":"s1"}"#)],
            "set_tools",
            "no such tool",
        );
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut refused, _events) = Live::attach(&addr, Some("s1"), false, None)
            .await
            .expect("attach to the failing mock");
        refused.apply(WbAction::SetTools(vec!["nope".into()])).await;
        let notice = refused.notice.clone().unwrap_or_default();
        assert!(
            notice.contains("no such tool"),
            "a refused tool set dropped the reason: {notice:?}"
        );
        refused.client.disconnect();

        // A refused rename and a refused export, through the typed door.
        let (mock, _seen) = crate::rpc::grpc_client::tests::api_mock_failing(
            &[
                ("get_state", r#"{"sessionId":"s1"}"#),
                ("get_session_entries", r#"{"entries":[]}"#),
            ],
            "set_session_name",
            "the name is taken",
        );
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (mut refused, _events) = Live::attach(&addr, Some("s1"), false, None)
            .await
            .expect("attach to the failing mock");
        refused.command("/rename a taken name").await;
        let notice = refused.notice.clone().unwrap_or_default();
        assert!(
            notice.contains("the name is taken"),
            "a refused rename dropped the reason: {notice:?}"
        );
        refused.notice = None;
        refused.command("/export").await;
        let notice = refused.notice.clone().unwrap_or_default();
        assert!(!notice.is_empty(), "a refused export said nothing");
        refused.client.disconnect();

        // `/tools off` on the launchpad stages an empty set instead of calling
        // the agent with no session.
        live.to_launchpad();
        live.command("/tools off").await;
        live.client.disconnect();
    }

    /// The approval card is cleared by whoever answers it: an
    /// `approval_decision` event naming the parked request takes the card down,
    /// and one naming another request leaves it up.
    #[tokio::test]
    async fn an_answered_approval_clears_the_card_it_belongs_to() {
        let mut live = live_on(&[(
            "get_state",
            r#"{"sessionId":"s1","pendingApprovals":[
                {"approvalRequestId":"a-1","toolName":"bash","riskLevel":"high"}]}"#,
        )])
        .await;
        let current = live.state.session_id.clone();
        assert!(live.approval().is_some(), "the fixture parks an approval");

        // Another client answered a different request: nothing moves.
        let other = {
            let mut event = event("approval_decision", json!({"approval_request_id": "a-2"}));
            event.session_id = Some(current.clone());
            event
        };
        live.handle_event(other);
        assert!(
            live.approval().is_some(),
            "a decision for another request cleared this card"
        );

        // The parked one: the card goes, whichever client answered it.
        let mine = {
            let mut event = event("approval_decision", json!({"approval_request_id": "a-1"}));
            event.session_id = Some(current);
            event
        };
        live.handle_event(mine);
        assert!(
            live.approval().is_none(),
            "the answered approval is still parked"
        );
        live.client.disconnect();
    }

    /// A compaction that fails without saying why still tells the reader it
    /// failed — never a blank notice row.
    #[tokio::test]
    async fn a_compaction_that_failed_without_a_reason_still_says_so() {
        let mut live = live_on(&[("get_state", r#"{"sessionId":"s1"}"#)]).await;
        let current = live.state.session_id.clone();
        live.begin_compaction();
        let mut failed = event("compaction_failed", json!({}));
        failed.session_id = Some(current);
        live.handle_event(failed);
        assert!(
            live.notice
                .as_deref()
                .is_some_and(|notice| !notice.trim().is_empty()),
            "a failed compaction with no reason left the notice row empty"
        );
        assert!(
            !live.compacting(),
            "the session was left held by the failure"
        );
        live.client.disconnect();
    }

    /// Submitting: an empty draft is not a prompt; the pictures attached to it
    /// travel with the message; and the id the acknowledgement binds is remembered
    /// only up to the window the completion notice reads.
    /// A `Live` whose mock keeps its log, so a test can ask what the *agent* was
    /// asked rather than what the adapter meant.
    async fn live_with_seen(
        pairs: &[(&str, &str)],
    ) -> (super::Live, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let (mock, seen) = crate::rpc::grpc_client::tests::api_mock_answering(pairs);
        let addr = crate::rpc::grpc_client::tests::spawn_api_mock(mock).await;
        let (live, _events) = Live::attach(&addr, Some("s1"), false, None)
            .await
            .expect("attach to the mock agent");
        (live, seen)
    }

    #[tokio::test]
    async fn submitting_carries_its_pictures_and_its_run_id() {
        let (mut live, seen) = live_with_seen(&[
            ("get_state", r#"{"sessionId":"s1"}"#),
            ("get_session_entries", r#"{"entries":[]}"#),
            (
                "prompt",
                // The ack is the wire's own snake_case shape (`RunAck`), epoch
                // included — a partial ack fails to decode and the run id is lost.
                r#"{"run_id":"run-1","run_epoch":0,"accepted_state":"running","queue_position":0}"#,
            ),
        ])
        .await;

        // An empty draft is not a prompt: nothing is sent.
        live.submit("").await;
        assert!(
            !seen.lock().unwrap().iter().any(|kind| kind == "prompt"),
            "an empty draft reached the agent"
        );

        // With text, the prompt is sent and its run id is remembered.
        live.submit("hello").await;
        assert!(
            seen.lock().unwrap().iter().any(|kind| kind == "prompt"),
            "a real prompt never reached the agent"
        );
        assert!(
            !live.completion_candidates.is_empty(),
            "the acknowledged run was not remembered for its completion notice"
        );

        // The window is bounded: the 33rd acknowledgement drops the oldest id
        // rather than growing the list, so a long session cannot grow it without
        // end.
        for _ in 0..33 {
            live.submit("again").await;
        }
        assert_eq!(
            live.completion_candidates.len(),
            32,
            "the completion window did not hold at its bound"
        );
        live.client.disconnect();
    }

    /// A *chat* has no workspace of its own: `new_session` mints the id, creates
    /// the directory the desktop also files chats under, and tells the agent both
    /// — which is the branch a launchpad first message takes, and the one that
    /// measured zero because every fixture so far staged a directory.
    #[tokio::test]
    async fn a_chat_session_creates_its_own_directory() {
        // A workspace that is **not** a repository: the point here is the chat
        // directory, and a `git` read in the shared worktree would put this test
        // in the same repository as the `/git` panel's tests — measured: two of
        // four full-suite runs then failed there on a read that had contended.
        let workspace = tempfile::tempdir().expect("workspace");
        let get_state = format!(
            r#"{{"sessionId":"s1","cwd":"{}"}}"#,
            workspace.path().display()
        );
        let (mut live, seen) = live_with_seen(&[
            ("get_state", &get_state),
            ("get_session_entries", r#"{"entries":[]}"#),
            ("new_session", r#"{"sessionId":"s-chat"}"#),
            ("set_sandbox_policy", r#"{"success":true}"#),
            ("set_tools", "{}"),
        ])
        .await;
        live.to_launchpad();
        // Nothing staged: the launchpad's own defaults, and a tool set to spend.
        live.pending_tools = Some(vec!["read".into()]);

        live.new_session().await;

        let asked = seen.lock().unwrap().clone();
        assert!(
            asked.iter().any(|kind| kind == "new_session"),
            "the agent was never asked for a session: {asked:?}"
        );
        // The approval mode is a second call, because `new_session` has no
        // argument for it.
        assert!(
            asked.iter().any(|kind| kind == "set_sandbox_policy"),
            "the session was created without being told its approval mode: {asked:?}"
        );
        // The staged tool set is spent on the session that exists now.
        assert!(
            asked.iter().any(|kind| kind == "set_tools"),
            "the staged tool set was not sent: {asked:?}"
        );
        assert!(
            live.pending_tools.is_none(),
            "the staged tool set survived the session it described"
        );
        assert!(!live.detached(), "a fresh chat is not the launchpad");

        // The directory the client named is a real one under this home.
        let chat_root = super::chat_root();
        assert!(
            chat_root.is_dir(),
            "the chat directory root was never created: {}",
            chat_root.display()
        );
        assert!(
            std::fs::read_dir(&chat_root)
                .expect("the chat root")
                .next()
                .is_some(),
            "a chat session left no directory of its own"
        );
        live.client.disconnect();
    }

    /// The plain-chat row of the workspace picker: on the launchpad it is
    /// "stage nothing"; in a session already on a chat directory it is a no-op;
    /// otherwise it moves the session into a chat of its own.
    #[tokio::test]
    async fn picking_the_plain_chat_stages_nothing_or_moves_into_one() {
        // Launchpad: staging no workspace *is* the plain chat.
        let mut live =
            live_on(&[("get_state", r#"{"sessionId":"s1","cwd":"/work/project"}"#)]).await;
        live.to_launchpad();
        live.pending_cwd = Some("/work/project".into());
        live.select_chat_workspace().await;
        assert!(
            live.pending_cwd.is_none(),
            "the plain chat staged a directory: {:?}",
            live.pending_cwd
        );
        live.client.disconnect();

        // The "already a chat" early return is not asserted here: it is decided
        // against the process's chat root, which follows `HOME`, and a sibling
        // test may swap `HOME` between this test's call and its assertion. That
        // lock cannot be held across the await, so the branch stays open rather
        // than becoming a flake.
        live.client.disconnect();
    }

    /// What the frame is built from, on the two sides that measured zero: the
    /// skills a *state* names (described from the local read when it knows them)
    /// and the model list's labels, which fall back to the id when a provider
    /// reports none.
    #[tokio::test]
    async fn the_frames_data_describes_skills_and_labels_models() {
        use crate::workbench::state::SkillRow;

        // A state that names two skills: the local read describes one of them,
        // the other keeps an empty description rather than a wrong one.
        let mut live = live_on(&[(
            "get_state",
            r#"{"sessionId":"s1","skills":["pdf","review"]}"#,
        )])
        .await;
        live.local_skills = Some(vec![SkillRow {
            name: "pdf".into(),
            description: "read a pdf".into(),
        }]);
        let frame = frame();
        let data = live.data(&frame);
        assert_eq!(
            data.skills
                .iter()
                .map(|skill| (skill.name.as_str(), skill.description.as_str()))
                .collect::<Vec<_>>(),
            vec![("pdf", "read a pdf"), ("review", "")],
            "the agent's order, described by the local read where it can be"
        );

        // No skills from the state: the local read is the whole list (an older
        // agent reports none).
        let mut live = live_on(&[("get_state", r#"{"sessionId":"s1"}"#)]).await;
        live.local_skills = Some(vec![SkillRow {
            name: "pdf".into(),
            description: "read a pdf".into(),
        }]);
        let data = live.data(&frame);
        assert_eq!(
            data.skills
                .iter()
                .map(|skill| skill.name.as_str())
                .collect::<Vec<_>>(),
            vec!["pdf"],
            "an agent that names no skills leaves the local list"
        );

        // A model whose label is blank is shown by its id, never as an empty row.
        let mut live = live_on(&[
            ("get_state", r#"{"sessionId":"s1"}"#),
            (
                "list_models",
                r#"{"models":[
                    {"id":"future/old","label":"","provider":"future",
                     "thinkingLevel":"default","thinkingLevels":["default"],
                     "defaultThinkingLevel":"default","isDefault":true},
                    {"id":"future/named","label":"Named","provider":"future",
                     "thinkingLevel":"default","thinkingLevels":["default"],
                     "defaultThinkingLevel":"default"}]}"#,
            ),
        ])
        .await;
        let data = live.data(&frame);
        let labels: Vec<(&str, &str)> = data
            .models
            .iter()
            .map(|model| (model.id.as_str(), model.label.as_str()))
            .collect();
        assert!(
            labels.contains(&("future/old", "future/old")),
            "an unlabelled model was not named by its id: {labels:?}"
        );
        assert!(
            labels.contains(&("future/named", "Named")),
            "a labelled model lost its label: {labels:?}"
        );
        live.client.disconnect();
    }
}
