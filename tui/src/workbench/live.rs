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

use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;

use crate::components::chat_area::{ChatArea, ChatMessage, ChatRole, ReplyDetail};
use crate::rpc::types::{
    AgentEvent, ModelInfo, PendingApprovalState, RpcSessionState, SessionEntriesPage,
    SessionSummary,
};
use crate::rpc::GrpcClient;

use super::i18n::{self, t, Key, Language, LanguagePreference};
use super::state::{
    Activity, ActivityKind, InputView, JobRow, ModelChoice, SessionMeta, SessionRow, SkillRow,
    TodoItem, UsageData, WorkbenchData, THINKING_LEVELS,
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
pub const APP_COMMANDS: [&str; 4] = ["/new", "/reply-detail", "/export", "/quit"];

/// One attached session plus everything the surface needs to paint it.
pub struct Live {
    client: Arc<GrpcClient>,
    chat: ChatArea,
    state: RpcSessionState,
    paging: Paging,
    sessions: Vec<SessionSummary>,
    models: Vec<ModelInfo>,
    approval: Option<ApprovalPrompt>,
    /// A failed request or command, shown above the status bar until the next
    /// successful action replaces it.
    pub notice: Option<String>,
    activity: Activity,
    activity_start: Option<Instant>,
    open_tools: usize,
    draft: InputView,
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
    /// one thing that has to re-read the goal store, so the panel follows the
    /// session instead of the process's own directory.
    local_cwd: String,
    /// The git branch of `local_cwd`, read once per workspace (see
    /// [`super::git`]): the header names it after the cwd, and a per-frame
    /// `git` spawn would be a stutter on a large repository.
    branch: super::git::BranchCache,
    /// The goal store's todos for `local_cwd` (see [`super::local`]).
    todos: Vec<TodoItem>,
    /// Installed skills and scheduled jobs are machine-wide: read once, then
    /// reused for every frame.
    local_skills: Option<Vec<SkillRow>>,
    local_jobs: Option<Vec<JobRow>>,
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
        let (client, events, _connection) = GrpcClient::new(addr);
        let client = Arc::new(client.with_no_context_files(no_context_files));

        let detail = settings.map(persisted_detail).unwrap_or_default();
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
        let mut chat = ChatArea::new(80, None);
        chat.set_reply_detail(detail);

        let mut live = Self {
            client,
            chat,
            state: RpcSessionState::default(),
            paging: Paging::default(),
            sessions: Vec::new(),
            models: Vec::new(),
            approval: None,
            notice: None,
            activity: Activity::default(),
            activity_start: None,
            open_tools: 0,
            draft: InputView {
                placeholder: t(lang, Key::InputHintLaunchpad).into(),
                ..InputView::default()
            },
            detail,
            theme_id: theme.id.to_string(),
            lang,
            launchpad: false,
            pending_model: None,
            pending_effort: None,
            run_started: None,
            run_output_tokens: 0,
            run_tps: 0.0,
            state_refresh_pending: false,
            local_cwd: String::new(),
            branch: super::git::BranchCache::new(),
            todos: Vec::new(),
            local_skills: None,
            local_jobs: None,
        };

        if let Some(id) = session.filter(|id| !id.is_empty()) {
            live.client.switch_session(id).await?;
        }
        live.refresh_state().await;
        live.refresh_sessions().await;
        live.refresh_models().await;
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
        let cwd = self.state.cwd.clone().unwrap_or_default();
        if cwd != self.local_cwd {
            self.todos = if cwd.is_empty() {
                Vec::new()
            } else {
                super::local::goal_todos(std::path::Path::new(&cwd))
            };
            self.local_cwd = cwd;
        }
        // The header's second line names the workspace *and its branch*. The
        // cache keys on the cwd, so this is a no-op on every refresh that did
        // not move the session.
        self.branch.for_cwd(&self.local_cwd);
        if self.local_skills.is_none() {
            self.local_skills = Some(super::local::installed_skills());
        }
        if self.local_jobs.is_none() {
            self.local_jobs = Some(super::local::scheduled_jobs(self.lang));
        }
    }

    pub async fn refresh_sessions(&mut self) {
        if let Ok(sessions) = self.client.list_sessions().await {
            self.sessions = sessions;
        }
    }

    pub async fn refresh_models(&mut self) {
        if let Ok(models) = self.client.list_models().await {
            self.models = models;
        }
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
        self.chat.clear_messages();
        self.paging = Paging {
            session_id: session_id.clone(),
            has_more: false,
            next_offset: 0,
        };
        if session_id.is_empty() {
            return;
        }
        match self.client.session_tail_page(&session_id).await {
            Ok(page) => self.apply_page(page, false),
            Err(error) => self.notice = Some(format!("Could not load the transcript: {error}")),
        }
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
                self.open_tools = 0;
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
                self.chat.add_tool_start(&tool_id, &name, args);
                self.open_tools += 1;
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
                self.open_tools = self.open_tools.saturating_sub(1);
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
                self.open_tools = 0;
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

    /// Is the agent parked on an approval right now?
    pub fn approval(&self) -> Option<&ApprovalPrompt> {
        self.approval.as_ref()
    }

    pub fn set_draft(&mut self, text: String, cursor: usize) {
        self.draft.text = text;
        self.draft.cursor = cursor;
    }

    pub fn set_detail(&mut self, detail: ReplyDetail) {
        self.detail = detail;
        self.chat.set_reply_detail(detail);
    }

    pub fn detail(&self) -> ReplyDetail {
        self.detail
    }

    // ─── painting ─────────────────────────────────────────────────────

    /// Everything the surface paints. `inner_width` is the conversation band's
    /// text width: the renderer needs it, and passing the real one keeps a
    /// markdown reply wrapped the way the terminal draws it.
    pub fn data(&mut self, inner_width: usize) -> WorkbenchData {
        let state = self.state.clone();
        let messages = self.chat.render_all(inner_width.max(1));
        let activity = self.activity_snapshot();
        let input = self.draft.clone();
        let sessions = self.sessions.clone();
        // On the launchpad there is no session, so no session's title: the
        // header must not keep naming the conversation `/new` left behind.
        let title = if self.launchpad {
            String::new()
        } else {
            self.title()
        };
        let current = state.session_id.clone();
        let mut rows: Vec<SessionRow> = sessions
            .iter()
            .map(|session| SessionRow {
                id: session.id.clone(),
                title: display_name(session),
                workspace: shorten(&session.cwd),
                age: age(session.updated_at_ms),
                // On the launchpad nothing is attached, so no row claims to be
                // the current one.
                current: !self.launchpad && session.id == current,
            })
            .collect();
        rows.sort_by(|a, b| {
            let at = |row: &SessionRow| {
                sessions
                    .iter()
                    .find(|s| s.id == row.id)
                    .map(|s| s.updated_at_ms)
                    .unwrap_or(0)
            };
            at(b).cmp(&at(a))
        });
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

        WorkbenchData {
            app_name: "future-tui".into(),
            version: crate::version::VERSION.to_string(),
            connected: true,
            session: SessionMeta {
                id: if self.launchpad {
                    String::new()
                } else {
                    current
                },
                title,
                cwd: state.cwd.clone().unwrap_or_default(),
                branch: self.branch.for_cwd(&self.local_cwd).to_string(),
            },
            model: self
                .shown_model()
                .filter(|model| !model.trim().is_empty())
                .unwrap_or_else(|| "no model".to_string()),
            effort: self.shown_effort(),
            mode: state.sandbox_tier.clone().unwrap_or_else(|| "open".into()),
            permission: state
                .permission_level
                .clone()
                .unwrap_or_else(|| "all".into()),
            usage: UsageData {
                ctx_used: state.context_tokens.unwrap_or(0).max(0) as u64,
                ctx_total: state.context_window.unwrap_or(0).max(0) as u64,
                cost: state.usage.cost_cny,
                cache_pct: cache_percent(&state),
                tps: self.live_tps(),
            },
            todos: self.todos.clone(),
            sessions: rows,
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
                })
                .collect(),
            skills,
            jobs: self.local_jobs.clone().unwrap_or_default(),
            messages,
            input,
            activity,
            notice: self.notice.clone(),
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
                tools: self.open_tools,
            };
        }
        Activity {
            kind: self.activity.kind,
            detail: String::new(),
            elapsed_secs: self
                .activity_start
                .map(|start| start.elapsed().as_secs())
                .unwrap_or(0),
            tools: self.open_tools,
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
            WbAction::EditInput { text, cursor } => self.set_draft(text, cursor),
            WbAction::Submit(text) => self.submit(&text).await,
            WbAction::Command(command) => return self.command(&command).await,
            WbAction::Scroll(_) => {}
            WbAction::LoadOlder => self.load_older().await,
            WbAction::SwitchSession(id) => self.switch(&id).await,
            WbAction::SetModel(id) => self.set_model(&id).await,
            WbAction::SetThinking(level) => self.set_thinking(&level).await,
            // No session to apply them to: stage, do not call.
            WbAction::SetPendingModel(id) => self.set_pending_model(&id),
            WbAction::SetPendingThinking(level) => self.set_pending_effort(&level),
            WbAction::NewSession => self.to_launchpad(),
            // The surface owns its own palette; the driver persists it (it is
            // the one that knows where `settings.json` is).
            WbAction::SetTheme(_) => {}
            // Same split for the language: the driver resolves the preference,
            // hands the bundle back here and persists it.
            WbAction::SetLanguage(_) => {}
            WbAction::SwitchPanel(_) | WbAction::ResizeSidebar(_) | WbAction::PersistSidebar => {}
            // Surface-only state: the driver owns persistence, the agent does
            // not need a round trip for a rectangle or a local shortcut.
            WbAction::SetPreferences | WbAction::SetKeybindings => {}
            // The driver displays local notices; a demo adapter has no notice
            // dispatch of its own to perform.
            WbAction::Notice(_) => {}
            WbAction::FocusPane(_) => {}
            WbAction::Redraw => {}
        }
        Outcome::Continue
    }

    /// Send the draft. A submission while a run is in flight queues behind it
    /// (the agent's coalescing policy), which is what the inline TUI does too.
    async fn submit(&mut self, text: &str) {
        let message = text.trim().to_string();
        if message.is_empty() {
            return;
        }
        // The launchpad's first message: there is no session to send it to yet,
        // so create one in the folder the surface is showing before sending.
        // `/new` left the previous session attached on the agent's side, which
        // is exactly why this asks for a *new* one instead of reusing it.
        if self.launchpad || self.state.session_id.is_empty() {
            self.new_session().await;
            if self.state.session_id.is_empty() || self.launchpad {
                return;
            }
        }
        self.draft = InputView {
            placeholder: self.draft.placeholder.clone(),
            ..InputView::default()
        };
        self.chat.add_message(ChatMessage::new(
            crate::session_rows::random_id(),
            ChatRole::User,
            &message,
        ));
        self.mark(ActivityKind::Thinking);
        match self
            .client
            .prompt(
                &message,
                crate::rpc::types::BUSY_ENQUEUE_COALESCING,
                Vec::new(),
            )
            .await
        {
            Ok(_ack) => {}
            Err(error) => {
                self.notice = Some(format!("Failed to send: {error}"));
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
            "new" => {
                self.notice = None;
                // `/new` is the launchpad, not an immediate session: the
                // session is created by the first message, with whatever model
                // and workspace the reader chose there.
                self.to_launchpad();
            }
            "sessions" => self.refresh_sessions().await,
            // A model named in the command line is a choice like any other, so
            // it takes the launchpad road too when nothing is attached.
            "model" if !arg.is_empty() && self.detached() => self.set_pending_model(&arg),
            "model" if !arg.is_empty() => match self.client.set_model(&arg).await {
                Ok(()) => self.refresh_state().await,
                Err(error) => self.notice = Some(format!("Could not set the model: {error}")),
            },
            "model" => self.cycle_model().await,
            "thinking" | "effort" => self.cycle_thinking().await,
            // The launchpad's parameter chips. With a session attached each
            // floats a wire call rather than carrying state of its own: the
            // agent stays the single source of what the next message runs with.
            // With none, the model and effort chips stage the choice locally —
            // the agent has no session to accept either.
            "launchpad.model" => self.cycle_model().await,
            "launchpad.effort" => self.cycle_thinking().await,
            "launchpad.permission" => self.cycle_permission().await,
            "launchpad.mode" => self.cycle_mode().await,
            "reply-detail" => {
                let next = match self.detail {
                    ReplyDetail::Default => ReplyDetail::Detailed,
                    ReplyDetail::Detailed => ReplyDetail::Default,
                };
                self.set_detail(next);
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
            // exist — the notice the whole family used to produce.
            "usage" | "skills" | "workspace" | "transcript" | "help" => {}
            other => self.notice = Some(format!("Unknown command /{other}")),
        }
        Outcome::Continue
    }

    async fn switch(&mut self, session_id: &str) {
        if !self.launchpad && session_id == self.state.session_id {
            return;
        }
        match self.client.switch_session(session_id).await {
            Ok(_) => {
                self.launchpad = false;
                self.notice = None;
                self.refresh_state().await;
                self.load_tail().await;
            }
            Err(error) => self.notice = Some(format!("Could not switch session: {error}")),
        }
    }

    /// `/new`: detach from the session and show the launchpad.
    ///
    /// Nothing is created and nothing is asked of the agent — the header keeps
    /// its connection state and version, the transcript goes, and the first
    /// message starts the session (`submit` already handled that on a fresh
    /// start). A run still streaming in the session left behind is muted by the
    /// `launchpad` guard in `handle_event`.
    pub fn to_launchpad(&mut self) {
        self.launchpad = true;
        self.chat.clear_messages();
        self.paging = Paging::default();
        self.draft = InputView {
            placeholder: self.draft.placeholder.clone(),
            ..InputView::default()
        };
        self.activity = Activity::default();
        self.activity_start = None;
        self.open_tools = 0;
        self.approval = None;
        self.notice = None;
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
    fn shown_model(&self) -> Option<String> {
        if self.detached() {
            return self
                .pending_model
                .clone()
                .or_else(|| self.state.model.clone());
        }
        self.state.model.clone()
    }

    /// The reasoning level the surface should name (see [`Self::shown_model`]).
    fn shown_effort(&self) -> String {
        if self.detached() {
            return self
                .pending_effort
                .clone()
                .unwrap_or_else(|| self.state.thinking_level.clone());
        }
        self.state.thinking_level.clone()
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
        let cwd = self.state.cwd.clone().unwrap_or_default();
        // What the reader chose on the launchpad is what this session starts
        // with; with nothing staged this is the state the agent already has.
        let model = self.shown_model().unwrap_or_default();
        let level = self.shown_effort();
        match self
            .client
            .new_session(
                (!cwd.is_empty()).then_some(cwd.as_str()),
                (!model.is_empty()).then_some(model.as_str()),
                (!level.is_empty()).then_some(level.as_str()),
            )
            .await
        {
            Ok(_) => {
                self.launchpad = false;
                // The agent now owns the choice (the session was created with
                // it), so the staging is spent — keeping it would let a stale
                // pick override the session's own on the next `/new`.
                self.pending_model = None;
                self.pending_effort = None;
                self.refresh_state().await;
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
        match self.client.set_thinking_level(level).await {
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
    /// Matched by id *tail* so a state that names the model with its provider
    /// (`future/deepseek-flash`) still finds the row the list holds.
    fn cycle_pending_model(&mut self) {
        let tail = |name: &str| name.rsplit('/').next().unwrap_or(name).to_lowercase();
        let current = self
            .pending_model
            .clone()
            .or_else(|| self.state.model.clone());
        let ids: Vec<String> = self.models.iter().map(|model| model.id.clone()).collect();
        if ids.is_empty() {
            // Not loaded yet (`refresh_models` runs after the first state):
            // say so rather than silently doing nothing.
            self.notice = Some("No models to choose from yet".into());
            return;
        }
        let here = current.as_deref().map(tail).unwrap_or_default();
        let index = ids
            .iter()
            .position(|id| tail(id) == here)
            .map(|index| index + 1)
            .unwrap_or(0);
        let next = ids[index % ids.len()].clone();
        self.set_pending_model(&next);
    }

    /// The next reasoning level, staged for the next session.
    fn cycle_pending_effort(&mut self) {
        let current = self
            .pending_effort
            .clone()
            .unwrap_or_else(|| self.state.thinking_level.clone());
        let index = THINKING_LEVELS
            .iter()
            .position(|level| *level == current)
            .map(|index| index + 1)
            .unwrap_or(0);
        let next = THINKING_LEVELS[index % THINKING_LEVELS.len()];
        self.set_pending_effort(next);
    }

    /// all → workspace → none → all: the level `/permission` sets, in the order
    /// the inline TUI offers them.
    async fn cycle_permission(&mut self) {
        const LEVELS: [&str; 3] = ["all", "workspace", "none"];
        let current = self
            .state
            .permission_level
            .clone()
            .unwrap_or_else(|| "all".to_string());
        let index = LEVELS
            .iter()
            .position(|level| *level == current)
            .unwrap_or(0);
        let next = LEVELS[(index + 1) % LEVELS.len()];
        match self.client.set_permission_level(next).await {
            Ok(()) => {
                self.state.permission_level = Some(next.to_string());
            }
            Err(error) => self.notice = Some(format!("Could not set the permission: {error}")),
        }
    }

    /// The approval mode, in the desktop composer's order.
    async fn cycle_mode(&mut self) {
        use crate::approval_mode::ApprovalMode;
        let current = self
            .state
            .sandbox_tier
            .clone()
            .unwrap_or_else(|| "off".into());
        let index = ApprovalMode::ALL
            .iter()
            .position(|mode| mode.as_str() == current)
            .unwrap_or(ApprovalMode::ALL.len() - 1);
        let next = ApprovalMode::ALL[(index + 1) % ApprovalMode::ALL.len()];
        match self.client.set_sandbox_policy(next).await {
            Ok(_) => self.state.sandbox_tier = Some(next.as_str().to_string()),
            Err(error) => self.notice = Some(format!("Could not set the approval mode: {error}")),
        }
    }

    /// Answer the parked approval (`y` / `n` on the surface).
    pub async fn decide(&mut self, approved: bool) {
        let Some(prompt) = self.approval.clone() else {
            return;
        };
        self.approval = None;
        if let Err(error) = self
            .client
            .approval_decision(&prompt.request_id, approved, "")
            .await
        {
            self.notice = Some(format!("Could not answer the approval: {error}"));
        }
    }

    /// Interrupt the run in flight (`ctrl+x`).
    pub async fn abort(&mut self) {
        if let Err(error) = self.client.abort().await {
            self.notice = Some(format!("Could not stop the run: {error}"));
        } else {
            self.activity = Activity::default();
            self.activity_start = None;
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

fn age(updated_at_ms: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0);
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

/// The palette the inline TUI persisted, or the workbench default.
pub fn persisted_palette(settings: &crate::settings::TuiSettings) -> super::theme::Palette {
    super::theme::resolve(settings.theme_id.as_deref())
}

/// The reading level the inline TUI persisted, or the default.
pub fn persisted_detail(settings: &crate::settings::TuiSettings) -> ReplyDetail {
    ReplyDetail::from_wire(settings.reply_detail.as_deref().unwrap_or("default"))
}

/// The language preference `settings.json` carries. An absent key means `system`
/// — the reader has not chosen, so the locale decides.
pub fn persisted_language(settings: &crate::settings::TuiSettings) -> LanguagePreference {
    LanguagePreference::from_wire(settings.language.as_deref().unwrap_or("system"))
}

impl Live {
    /// Adopt a language preference: the adapter's own sentences (the input hint,
    /// an approval prompt) and the panels' local facts are re-read in it, and the
    /// resolved bundle is returned for the surface.
    ///
    /// The scheduled jobs are read from `tasks.db` with their state already
    /// spelled out, so the cache is dropped rather than patched; the next frame
    /// refills it. Skills and todos carry no chrome, so they are untouched.
    pub fn set_language(&mut self, preference: LanguagePreference) -> Language {
        self.lang = preference.resolve();
        self.draft.placeholder = t(self.lang, Key::InputHintLaunchpad).into();
        self.local_jobs = None;
        self.lang
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::types::SessionSummary;

    fn row(id: &str, name: Option<&str>, cwd: &str) -> SessionSummary {
        SessionSummary {
            id: id.into(),
            cwd: cwd.into(),
            updated_at_ms: 0,
            model: "m".into(),
            session_name: name.map(str::to_string),
            parent_session_id: None,
            is_streaming: None,
            first_message: None,
        }
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
        assert_eq!(age(now), "now");
        assert_eq!(age(now - 5 * 60_000), "5m");
        assert_eq!(age(now - 3 * 3_600_000), "3h");
        assert_eq!(age(now - 2 * 86_400_000), "2d");
        assert_eq!(age(now - 10 * 86_400_000), "1w");
        // A clock that disagrees with a stored timestamp reads as "now" rather
        // than a negative age.
        assert_eq!(age(now + 60_000), "now");
    }

    #[test]
    fn a_cache_rate_never_divides_by_zero() {
        let mut state = RpcSessionState::default();
        assert_eq!(cache_percent(&state), 0);
        state.usage.cache_read_tokens = 96;
        state.usage.input_tokens = 4;
        assert_eq!(cache_percent(&state), 96);
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
