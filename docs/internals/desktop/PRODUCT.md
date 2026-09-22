# FutureOS product description

> ([中文](PRODUCT.zh-CN.md))

## 1. Product positioning

FutureOS is a workspace where "one AI agent runs everywhere": the desktop GUI is
the primary experience, with phone, terminal, CLI, and IM bots as companion
interfaces connected to the same Agent backend. It targets people who need to
push complex tasks forward continuously: software engineering, research,
data analysis, document writing, report generation, and automated debugging
should all be doable within one set of work objects.

FutureOS's core goal is to make the AI's work process inspectable, recoverable,
and reviewable. Users do not only see the final answer — they see what the
Agent read, which commands it ran, which approvals it is waiting on, which
files it produced, and how to continue that work afterwards.

Phase one focuses on the desktop experience, workspace-bound Agent execution,
the approval loop, and phone remote control (continue/control desktop
conversations after QR pairing — see 5.3).

## 2. Current module boundaries

FutureOS is a multi-surface system built around "one AI agent runs everywhere":
the core is the Rust gRPC agent backend; the desktop, phone, terminal, CLI, and
IM bots are all interfaces that connect to it.

- `desktop/`: Tauri + React + TypeScript desktop GUI, the primary experience
  entry.
- `mobile/`: React Native (Expo) mobile client (Android / iOS), the native
  phone terminal for desktop Remote (see 5.3).
- `agent/`: the Rust `future-agent` gRPC service — sessions, LLM streaming,
  tool execution, approvals, and event projection.
- `tui/`: the Rust `future-tui` terminal UI.
- `cli/`: Rust `future-cli`, building the unified `future` binary that
  aggregates component management.
- `channels/`: Rust `future-channel`, the Feishu / DingTalk IM bridge.
- `orchestration/loop/`: Rust `future-loop`, the loop control plane.
- `packages/rpc/`: Rust `future-rpc`, the protobuf wire contract (single source
  of truth).

The GUI does not compile the Agent into the desktop process as a Tauri crate
dependency. At runtime the GUI connects to the Agent gRPC service over
per-user local IPC by default: on Unix `FUTURE_AGENT_SOCKET` is preferred, a
redirected `FUTURE_HOME` (`future agent --home`) owns
`<FUTURE_HOME>/run/agent.sock`, Linux otherwise uses
`$XDG_RUNTIME_DIR/future/agent.sock`, then falls back to
`~/.future/run/agent.sock` (the macOS default); Windows uses a current-user
named pipe. `FUTURE_AGENT_GRPC_ADDR` can explicitly specify TCP, which requires
the Agent to be started with `--grpc-addr`. With no reachable Agent, the
desktop launches the unified `future agent` sidecar and only owns the lifecycle
of the process it started.

## 3. Product principles

### 3.1 The GUI is the primary experience

The desktop GUI should offer the most complete, most refined experience:
conversations, workspaces, background program states, tool execution results,
instant approvals, reviews, and long-task progress. The GUI must not be a
terminal wrapper — it renders the Agent's work process into an understandable,
operable interface.

### 3.2 Agent work must be inspectable

Users should be able to inspect:

- What plan the Agent made.
- Which tools it called.
- Which files it read.
- Which commands it executed.
- Which files changed.
- Which steps failed.
- Which operations need approval.
- Which tasks can be continued/resumed.

### 3.3 Conversations are work objects

A conversation is not a temporary chat log but a recoverable, continuable,
manageable work object. FutureOS supports both ordinary Chats and
Workspace-bound conversations.

- Chat: the system auto-creates a temporary workspace — good for quick starts.
- Workspace conversations: the user picks a local directory — good for
  long-term projects and real file work.

Every Thread must use an independent Agent session, so different Chat or
Workspace conversations never share run context.

### 3.4 The CLI is an auxiliary entry

The CLI is for login, service management, terminal automation, and debugging.
The CLI does not duplicate the GUI's full experience, nor replace the GUI as
the product's main line. Whether CLI-created tasks appear in the GUI needs
separate definition later; phase one does not make it a default sync target.

### 3.5 The phone is the companion Remote terminal

The phone (Android / iOS) is a native client of Desktop Remote, not a
standalone product. After QR pairing it can view the desktop's online status
and session list, create or continue desktop conversations, stream
replies / thinking / tool execution, and handle approval, stop, model, and
thinking-level switching. Tools always execute locally on the desktop machine;
the phone only views and controls remotely — see 5.3.

### 3.6 The system is not just for coding

FutureOS must support coding, but must not be designed only as a code tool.
Research, literature organization, data analysis, and report writing are all
first-class product scenarios.

## 4. Core work objects

### 4.1 Workspace

A Workspace is a project or work context: a local project directory, a research
directory, a data-analysis directory, a writing directory, or a temporary
directory FutureOS auto-creates.

A Workspace can hold multiple child conversations. Deleting a workspace
conversation only deletes the conversation itself, never the workspace
directory or files inside it.

The Workspace itself supports rename and delete (the operation menu on the
workspace group header in the left navigation). Workspace deletion is a **soft
delete**: the workspace and its child conversations are removed from the
sidebar, but **the on-disk directory and files are not deleted**. When "Open
workspace" picks a directory already registered as a workspace, it prompts and
opens the existing one directly (deduplicated by path, no duplicate records).

### 4.2 Chat

Chat is an ordinary conversation not explicitly bound to a user directory. To
give tools and scripts a stable execution space, every Chat auto-creates a
temporary workspace behind the scenes.

Ordinary Chat file artifacts must be inspectable and cleanable. Cleaning an
ordinary Chat must prompt for confirmation if temporary workspace files exist.

### 4.3 Message

A Message is a conversation entry, not just a piece of text. In the future it
may contain markdown, images, file references, attachments, tool results, and
diffs.

The input-box attachment entry supports local files. Images are passed to the
model as multimodal input; other files are passed to the Agent as a structured
path list for the model to read on demand with tools like `read` / `shell`.
Attachments are not copied into the working directory — see 4.9 Attachment.

### 4.4 Run

A Run is one Agent execution, usually triggered by one user message. In the GUI
a Run is presented as a "background program" recording status, model, start and
end times, error info, tool-call process, and output results.

Users should be able to see whether the current background program is running,
failed, waiting for approval, or can be terminated, retried, or resumed. The
right-side Runs panel only shows the program list and result status — not full
events, tool payloads, or long output details — to keep the right side from
becoming a debugging log collection.

Background-program list product rules:

- Running, queued, and waiting-for-approval programs use a blue dot.
- Completed, failed, and cancelled programs use a gray dot.
- Running programs can be force-terminated, with a second confirmation required
  first.
- Termination calls Agent abort and marks the run `cancelled`.
- If the run is waiting for approval, termination must also cancel that run's
  pending approval, avoiding a stale approval lingering above the composer.
- Completed programs show the result status: success, failure, or cancelled.
- Completed, failed, and cancelled programs can be cleared in one click;
  clearing only deletes the run and its event/tool/approval/review relations —
  never user messages.

Failed / cancelled Runs support recovery with three actions:

- **Retry**: re-launch with the original user message (including attachments)
  that triggered the run — equivalent to re-running from scratch.
- **Continue**: re-launch starting with "continue the previous task" plus a
  summary of the failure message; when triggered from a run inspection in the
  Runs panel, it additionally attaches a summary of what that run already did
  (tool calls and recent events) so the Agent continues instead of re-running.
- **Fork**: on any ended assistant message, copy out a new Thread (with an
  independent Agent session) from the user message that triggered it, leaving
  the original conversation untouched.

Retry / Continue only appear on the **latest round's** failure that was not
interrupted; failures from earlier rounds or interrupted runs get no in-thread
retry (avoiding racing a run still active on the Agent side) — use Fork
instead.

### 4.5 Tool Call

A Tool Call is one invocation of an external capability by the Agent. The
default tool set is a long-term fixed constraint: `read`, `shell`, `edit`,
`write`.

Directory browsing and search do not add `ls` or `grep` tools; they go through
`shell` system capabilities, with approval and observable UI making sure users
understand the risk.

Tool calls in the GUI show a short title, status, duration, and a path or
command summary. Command details are single-line truncated, with the full
content visible on hover.

### 4.6 Approval

Approval answers "may this execute" when the selected mode and rules require
asking; it does not mean every high-risk operation is automatically identified
or asked about. Desktop currently defaults to `off`; the corresponding
protections apply only after manual/sandbox is enabled.

Ordinary file approvals target **file-path access** (read / write); manual-mode
shell approvals and macOS / Linux single de-sandbox approvals target the
**whole command** — the paths listed on the card must not be read as the
command's complete permission boundary. Shared rules, protocol, and approvals:
[SANDBOX/COMMON.md](SANDBOX/COMMON.md); per-platform implementation,
differences, and acceptance: [macOS](SANDBOX/MACOS.md),
[Linux](SANDBOX/LINUX.md), [Windows](SANDBOX/WINDOWS.md). Key points:

- **Layered path rules** yield `ask / allow / deny`, first match returns:
  built-in security overrides (immutable) → sensitive-file guards (immutable) →
  this conversation/workspace temporary rules → rule files
  (`${WS}/.future/approval_rule.json`, `~/.future/approval_rule.json`) →
  fallback (reads open, writes limited to workspace/temp).
- **Three approval tiers** (input-box dropdown / settings page switch, global):
  - **Manual approval** (opt-in, all platforms): read/write/edit prompt per
    rules; shell read-only commands (`ls/cat/grep/git status` etc.) run without
    asking, other commands show a confirmation card; no OS sandbox enabled.
  - **Sandbox protection** (macOS): shell runs automatically in a Seatbelt
    sandbox; on out-of-boundary failure it goes through escalation approval;
    read/write/edit still prompt per rules.
  - **Write protection** (Windows): shell runs inside an unelevated
    RestrictedToken + NTFS ACL write boundary, with the workspace, actual temp
    roots, and allowed paths open; extra external write paths go through
    up-front approval. **Shell reads (including sensitive files) and the
    network are not intercepted by write protection**; native read/write/edit
    still follow the sensitive guards. Ordinary directory ACLs cannot fully
    protect future filenames or globs; the current user's existing parent-
    directory delete permissions and wide ACLs can also weaken the boundary —
    a single-file approval must not be read as absolutely forbidding deletion,
    and no macOS equivalence is claimed.
  - **Sandbox protection** (Linux merged; the pre-release real-machine matrix
    still needs candidate-version evidence): shell runs in system Bubblewrap,
    network open; exact protection targets existing at launch and glob matches
    are protected by the OS sandbox. Missing exact targets and new glob matches
    within a command are currently end-detection only — creation inside a
    writable domain cannot be prevented, and it is not a complete access audit;
    missing Deny targets do not promise hard protection — an accepted
    limitation below. Full probe failure explicitly falls back to manual
    approval, never bare runs. Installation and limits in
    [`SANDBOX/LINUX.md`](SANDBOX/LINUX.md).
  - **Fully open** (current desktop default): everything allowed, no prompts,
    no sandbox.
- **Network is fully open and never approved.**
- **Linux shell's explicit limits (accepted, no macOS equivalence promised)**:
  protection targets that do not exist at launch (including the two
  `approval_rule.json` files and user-custom `deny` paths) do not promise
  creation prevention — only end-of-run existence detection; if they end up in
  a writable domain, creation may succeed. Existing rule files are still
  protected by read-only mounts, but a missing one can be created with writes
  that take effect later — an accepted security trade-off, not the absence of
  risk. Native `read/write/edit` rule checks are unchanged. Complex
  "wide-deny, narrow-allow" combinations and not-yet-existing writable allow
  targets are not promised to work — access or initialization may fail. No
  parent-directory read-only, placeholder objects, or auto-degradation is added
  this cycle for these scenarios; macOS's original path enforcement is
  unchanged.
- **Rule semantics**: sensitive files (`.env`, `*.pem`, `*.key`, `~/.ssh`,
  credentials, etc.) default to ask, cannot be overridden by user rules, and
  can only be "allowed once" — never persistently allowed; models.json
  read/write deny, rule-file write deny. Native tools enforce this judgment;
  shell platform differences follow the boundaries above, and a macOS/Linux
  approved whole command out of the sandbox is not bound by the OS rules.
- Windows shows "Write protection" only after the full host probe passes; Linux
  keeps the "Sandbox protection" option for a stable layout — disabled while
  detecting or unavailable, selectable only after the full probe passes. A
  clear Linux probe failure falls back and persists as "Manual approval",
  showing "reason + remedy + code" with a hint to fully restart FutureOS after
  installation; transient connection errors do not rewrite the setting.
  TUI / CLI / channels use their own `permission_level` and do not participate
  in this approval system.
- Desktop's macOS, Windows, and manual approvals reuse one approval card;
  mobile's native cards consume the same trusted semantics. Ordinary users
  first see the trusted-backend-generated "behavior + target" title; single
  targets do not duplicate fields, command/file previews collapse into "view
  command / view details", and ACL, SID, backend, hash, and rule-layer
  technical data are not shown.
- Approval offers three per-scenario semantics: **not allowed / allow this once
  / always allow in this project** (exact wording can keep evolving). "Always
  allow in this project" saves the same behavior and target expressed on the
  card as a workspace allow rule and applies it this turn immediately;
  sensitive files, macOS/Linux whole-command escalation, and
  non-persistable requests do not offer this option.
- Multi-target approvals list every target, at most 8, decided as a whole
  group; when no trusted behavior/target can be generated or payload parsing
  fails, fail closed — no approve button.

**Model-initiated single de-sandbox request (macOS / Linux, existing
capability)**:

- After a command fails in the sandbox, the model can judge whether sandbox-
  external permission is truly needed, then issue a new shell request with
  `escalated: true` and one sentence of `justification`. That request enters
  approval before execution; the model does not grant itself permission, and
  the original failure need not have been recognized by the system as an
  auto-escalatable denial first.
- The card shows the full command and the model-provided reason (when
  non-empty). When failure diagnostics are parseable, paths mentioned in the
  error can also be shown; active requests usually have no failure output, so a
  path list is not guaranteed. That list is neither the complete set of access
  targets nor an authorization of only those paths.
- The title distinguishes the source: model-initiated is "Model requests
  running this command outside the sandbox"; system-triggered after a sandbox
  failure is "Running this command outside the sandbox is needed". Desktop and
  phone agree. The execution branch sets `action.escalation_trigger`
  (`model_request` / `sandbox_failure`) — never guessed from the reason text or
  path presence; old Agents / historical data without a source keep "Run this
  command outside the sandbox". This only distinguishes the approval source —
  it does not change the grant scope or rejection judgment; passive triggers
  still rely on the existing classifier, not a new precise-attribution
  guarantee.
- After user approval, **the current whole command executes outside the OS
  sandbox once**; rejection means that de-sandboxed execution does not run.
  With no escalation-approval channel available, nothing runs bare — the
  current implementation keeps trying ordinary sandbox execution. A single
  approval does not change the global approval mode to "manual" or "fully
  open"; later ordinary commands still follow the original mode.
- This is a request capability available to the model; not every sandbox
  failure is guaranteed to trigger a request or a successful retry. An
  initialization failure by itself does not mean authorization was obtained; if
  it is unclear whether the original command executed, check side effects first
  — exit 125 alone must not be treated as proof a safe re-run is possible.
- No separate command-level auto-fallback, temporary manual mode, or dedicated
  recovery UI. The existing basic-probe-unavailable handling is unchanged.
  Windows still uses concrete write-path capabilities, not this whole-command
  de-sandbox semantic.

The GUI shows approval requests, passes decisions back to the Agent, and — for
persistent allows — writes rule files through a trusted path (agent tools
cannot write that file).

Approval is a critical product path and must follow these rules:

- Approval is not placed in the right context panel and is not a history-tab
  action.
- The current pending approval appears at the bottom of the middle
  conversation area, above the composer — more prominent than ordinary context
  information.
- The UI shows at most one pending approval at a time; the Agent backend also
  waits serially, and does not continue executing later dangerous operations
  while the current approval is undecided.
- Approvals never time out and always wait for an explicit allow or deny; the
  Agent / GUI must not end a run waiting for approval because of a fixed HTTP,
  SSE idle, or event-collection timeout.
- Keyboard shortcuts: `Esc` = not allowed, `Cmd/Ctrl + Enter` = allow this
  once.
- Command and file previews wrap in the collapsed detail, at most one third of
  the window height, scrolling inside when exceeded; raw approval JSON must not
  be shown as user-approvable content.
- After a GUI restart, leftover pending approvals are **not** unconditionally
  cancelled: startup convergence keeps them, and reconciliation once the Agent
  is reachable (`reconcile_pending_approvals`, once at startup + every watchdog
  round) decides against the Agent's authoritative pending set — cards the
  Agent is still waiting on are kept (the run revives and returns to
  `waiting_approval`), and only those the Agent no longer holds (decided
  elsewhere, aborted, or lost in an Agent restart) become `cancelled`. This
  guarantees approvals the Agent hung on during a GUI crash remain displayed
  and decidable after restart.
- If the user clicks an old pending approval that no longer exists on the
  backend, the GUI turns its local state to `cancelled` — never exposing an
  internal error like "not pending" to the user.

### 4.7 Review

Review answers "what changed". It is different from Approval: Approval is
allow/deny before execution; Review is inspecting changes after execution.

Review covers all Workspace conversations (`mode = workspace`), whether or not
the directory is a Git repository; ordinary Chats show no Review, and their
file artifacts are visible through the Files panel (see 5.5). A dropdown at the
top of the Review panel switches views, offering by workspace type:

- **Git changes** (Git workspaces only): based on a real `git diff`, showing
  the current work tree's uncommitted changes against the selected base
  (default `HEAD`) — additions, deletions, modifications, renames, and
  untracked text files — with file-level additions / deletions.
- **Last-round changes** (both workspace types): the workspace file changes
  between before and after the current Thread's **latest ended Run**
  (`completed` / `failed` / `cancelled`).

"Last-round changes" product semantics:

- Strictly the latest ended Run; running Runs do not enter, and the previous
  round's results stay visible during the run.
- When the latest round has no file changes, show "no file changes last round"
  — do not fall back to an earlier round with changes, or "last round" loses
  its meaning.
- It means "file changes inside the workspace during that round's run", not an
  absolute claim every change came from the Agent (user edits, IDE formatters,
  and background processes within the same window are also included) — the UI
  does not lie about attribution.
- Failed, cancelled, or partially completed Runs still show what was written to
  disk as best as possible; when the snapshot fails it is explicitly marked
  "Unavailable" — never disguised as "no changes".

"Last-round changes" is produced by FutureOS snapshotting the workspace before
and after each Run and diffing them, read-only throughout: it writes no
commit / index / ref / object into the user's real Git repository and creates
no `.git` in non-Git directories (implementation in ER.md §4.10, §6.8).

"Last-round changes" overlays status hints at the top as applicable:

- **Concurrent**: other Runs ran concurrently in this workspace during this
  round; some changes may come from concurrent runs; the diff is still shown.
- **Recovered**: snapshots recovered after an app restart; attribution may be
  imprecise.
- **Unavailable**: a last-round Run exists but its snapshots are unavailable —
  distinct from "no changes".
- **Directory too large**: change preview is turned off for non-Git workspaces
  above the size red line, showing a static hint instead of an empty state.

Display conventions: diffs are unified (single column) only; files are
collapsed by default with an "expand all / collapse all" toggle at the top;
binary files show only path, status, before/after sizes, and type — no text
diff; sensitive files matching credential rules (`.env`, `*.pem`, `*.key`,
etc.) are only marked "sensitive file changed, content not saved" — the content
is not saved.

In the long-term goal, Review will also cover change review of text files like
markdown, documents, and tables.

### 4.8 Skill

A Skill is a capability unit the Agent can use, coming from the **official
platform catalog**, not stored in the repo. Users can browse, install, and
uninstall skills on the Skills page; after installation, `/技能名` triggers
them in the conversation input box.

The input box's `/` menu carries not only Skills but a few high-frequency
**context tools**. Context tools execute FutureOS's own session operations —
they are not sent to the model as user prompts and do not pretend to be Skills.
The first context tool is **Compact**: offered only in conversations that
already have an Agent session; selecting it immediately triggers a standalone
context compaction. It creates no user message, ordinary Agent reply, or new
Run; the summary request is the operation's only model communication. Manual
compaction skips the automatic threshold and uses the complete recent-turn tail
of at most 15K tokens; short conversations get a summary of the whole thing,
with no duplicate copy of the original kept. A successful checkpoint records
`trigger: manual` and shows the user's own choice via the "you manually
compacted this conversation's context" divider; failure also reports in place.

The `/` menu searches by Chinese or English name and description. When filtered
results contain both context tools and Skills, context tools stay pinned on
top, Skills pinned below, with the "技能 / Skills" divider text shown only
before the Skill list; a single-type result shows no divider. Selecting a
context tool removes the typed `/query` and executes immediately; selecting a
Skill keeps inserting the original `/skill-name` pill, sent with the next user
message.

**Launch and onboarding**: Skills is live as a standalone left-nav entry. New
users entering for the first time see a skills onboarding banner on the new
conversation page ("Start learning" auto-starts one conversation with the
getting-started guidance; "Got it" collapses it); the left-nav Skills entry
shows an installed-count badge and a one-time guide bubble ("N common skills
installed" / "No skills installed yet"), and the Skills page offers guidance
text with a "Try it" prefill.

### 4.9 Attachment

An Attachment is a local file the user attaches in the conversation input box
for the model to understand. Images go through the multimodal channel
separately; other files are uniformly handled as files the Agent tools can read
on demand.

Attachment sources and upload paths:

- Any ordinary file is accepted, directories are not. jpg, jpeg, png, gif,
  webp, bmp are images; SVG is given to the Agent as a text file to read.
- Three upload methods: the input box's attachment button, copy-paste, and
  drag-and-drop. When the web clipboard provides a `file://` URI, decode it
  into a normal local path first (including Windows drive letters, UNC, and
  Linux paths); otherwise read the system file-manager clipboard and uniformly
  parse macOS Finder file-reference URLs, Windows Explorer's `CF_HDROP`, and
  Linux file managers' URI lists. After resolving to a real absolute path, the
  backend checks protected paths, existence, directory-ness, and readability;
  on success only the path is recorded — the file is not copied.
- At most 4 images per message; non-image files have no count or size limit. A
  single image is at most 25 MiB, validated on select, drag, and paste.
- Images must be successfully read and decoded before sending; on failure the
  message is not sent, the input draft is kept, and the specific file is called
  out.
- Models supporting image input receive the image content; models that do not
  receive the image path. Other files reach the model through a one-line JSON
  attachment list with name, type, and absolute path — strings escaped per JSON
  rules and explicitly marked untrusted; the GUI does not extract or inline
  file content.

Ordinary Chats and Workspace conversations share the same file policy: nothing
is copied into the working directory; message metadata records the original
absolute path. Images additionally generate persistent thumbnails; only when
the clipboard has pathless `File`/Blobs (e.g. browser or remote sources) does
FutureOS copy into a private temporary directory — a single ordinary file at
most 10 MiB, at most 10 per paste totaling at most 20 MiB — and persists to the
thread's `images/<threadId>/origin` on send. Pathless images follow the image
paste flow and the 25 MiB per-image limit.

In history, image, Markdown, and JSON files use in-app preview (by extension);
other files are opened by the system default app. When the original path is
invalid, explicitly show "file moved or deleted".

Mobile attachments transfer in shards over the NATS relay: validated by
original size on selection first (single file 10 MiB, single message total
20 MiB, at most 10 attachments / 4 images); images with a longest edge over
1600px are downsampled to 1600px on the phone, not rejected. JPEG/BMP are
encoded as JPEG quality 65 on the phone before upload, HEIC/HEIF are input-only
and also converted to JPEG 65, PNG/WebP/GIF stay as originals, SVG is an
ordinary file; the UI shows only the original size. GIF, animated WebP, and
APNG offer no mobile preview, with a uniform "view on desktop" hint. The phone
downloads desktop attachments only on tap, with a second confirmation on
cellular and unknown network types; downloads auto-retry on failure; on iOS,
after download the system share sheet handles save/open. Static image previews
use a 600px longest edge; Markdown fully renders at most 2 MiB; previews are
cached by content hash with validation and a 100 MiB cache cap. Phone-uploaded
files persist into the thread's `images/<threadId>/origin` file tree once the
prompt binds the thread, cleaned up with the thread.

## 5. Desktop experience

### 5.1 Three-column layout

The GUI uses a three-column structure:

```text
left navigation   middle conversation area   right context panel
```

The left column holds feature entries and workspace/thread navigation. The
middle holds the conversation and instant approval. The right holds inspectable
context like background programs and review. The left rail's right edge
supports drag-resize, and arrow keys work when the divider is focused; the
width persists across restarts, and it auto-constrains when the window shrinks
to leave room for the conversation area and an open right panel.

The right panel must not steal user attention and must not carry approval
actions needing immediate response. Ordinary background refreshes must not
flicker or switch tabs frequently.

### 5.2 Left navigation

The left navigation supports:

- New Chat.
- Skill (live as a standalone nav entry with installed-count badge and
  first-time guidance — see 4.8).
- Remote (phone remote control; the entry shows only after signing in to
  FutureOS — see 5.3).
- The pinned section (all pinned conversations).
- Workspace list.
- Child conversations under workspaces.
- Chat list.
- Settings.
- Expand, collapse, and archive display.

Pinning is **global**: every pinned conversation — whether belonging to a
workspace or an ordinary Chat — is gathered in the top "Pinned" section;
unpinning returns it to its own group. Workspace **groups** can be pinned too
(from the group menu): pinned groups lead the workspace list, unpinned ones
keep the store's recency order below them, and the header shows a pin marker.
The flag is stored on the workspace (`workspaces.pinned`), so the phone's
workspace tab and the desktop rail order the same groups the same way.

Conversations display as a tree of at most three levels following the Agent's
parent-child session relations (excluding the Workspace title level), collapsed
by default; clicking the title
still opens the conversation directly. Forks and loop-derived execution
sessions use the same relations. Child conversations group with their root even
when their working directories differ; pinned children enter the pinned section
independently and keep their own subtree. Missing, archived, or deleted parents
do not hide surviving children; deeper historical levels are flattened into the
third level, with no Agent relations deleted or rewritten. Batch select-all
includes collapsed children in the current group; delete does not auto-cascade.
A pinned conversation belongs to the pinned section rather than to any group,
so it is never part of a batch: it carries no checkbox and select-all (in a
workspace group or in Chat) skips it on both desktop and mobile.

A child conversation's expand toggle must be a **+ / − tree-node toggle**
(collapsed `+`, expanded `−`), which is a different icon from the workspace /
section-header collapse **chevron**; when they share a column (on mobile both
controls sit in columns 8–24) only the icon distinguishes them, so they must not
use the same icon, nor a filled glyph that breaks the product's hairline style.

**Column rule**: a row = `[toggle column 16px][gap 4px][title]`, and **a child
row's start equals its parent's title column**, so a parent title and its child
titles share a column at every level; a leaf row has no toggle column, so its
title starts at the row start.
Desktop row starts: conversations (including pinned) 16, workspaces 28 (both
unchanged), +20 per level; a group header is
`[chevron 8][4][folder 28][4][name 48]` — the folder occupies the toggle column
so the name's 48 lines up with first-level conversation titles (see
ActivityRail).
Mobile starts at 8 with +20 per level; the toggle layout is 16 wide (not 44),
with the 44×44 touch target completed by `hitSlop` (16 on the left falls inside
the list padding, 4 on the right stops at the title column).

Title-space priority: rows without children reserve no expand-arrow placeholder,
and child rows align to their parent title by row-start indent (no placeholder
whitespace). Ordinary conversations and the pinned section (non-workspace) keep
a 16px starting margin between the title and the highlight left edge — titles
are not flush against the edge; other sessions appearing or expanding children
never change unrelated top-level rows' title origins.

A conversation row's **whole row is clickable** to enter it; the row-end
operation menu (rename / pin / delete) does not accidentally enter. The
workspace group header offers an "Open workspace" entry beside it plus a group
menu (rename / pin / open folder / select chats / delete workspace); no "new
workspace" entry is offered.

**Unread indicator** right of conversations: after a background run ends, an
unread dot appears (green for normal completion, red for failure); while
running, a blue waiting icon; opening or leaving the conversation clears the
unread dot, and the idle state shows no dot. Unread state lives in the current
session (sessionStorage) and clears on app restart.

### 5.3 Phone remote control

The phone is the Desktop's remote view and control surface; tools execute on
the computer locally. Remote capability must be visible and controllable:
closing the Desktop in GUI mode disconnects Remote; standalone `futureos-headless`
creates no window, runs in the terminal foreground, shows the platform login
and phone-pairing QR code and link on demand, and Ctrl+C closes the entry.
Headless mode is not silently hiding a window or auto-background keeping alive.
In both modes, the Agent owned by the Desktop and in-progress conversations end
when the Desktop exits; existing external Agents are not terminated by it.

The complete product background, interaction rules, network wiring, pairing
and permissions, architecture plans, self-recovery and sync contracts, support
codes, and development plans for this feature are maintained in
[remote connection design](CONNECTION.md); this section does not duplicate
them.

### 5.4 Middle conversation area

The middle area shows user messages, assistant streaming output, plans, tool
calls, command previews, error states, and follow-up interactions.

Message bodies render as Markdown: beyond ordinary rich text, inline and block
LaTeX math (KaTeX) are supported; links are filtered through a protocol
whitelist (non-http(s) protocols degrade to plain text), external links open in
the system default browser, and `[text](http…)` links in user messages are
clickable too. Markdown images load inline automatically, including resolved
local paths outside the workspace; local images still pass backend path and
size validation before an asset URL is exposed. Clicking an unlinked image opens
a viewport-sized preview, dismissed with Escape, the close button, or the
backdrop. Images wrapped in links retain their link action. Inline images stay
height-limited without separate expand/collapse links; click-to-preview is the
single enlargement action.

Failed / ended assistant messages offer recovery actions below: retry /
continue (latest-round failure only) and fork (any ended message) — semantics
in 4.4.

The assistant's **thinking process** and tool activity stay **inline** in
occurrence order, never gathered at the message top. Matching mobile, two or
more consecutive settled steps collapse into a muted, right-aligned summary
(tool glyph ×N · brain glyph ×N). Counts refer to projected step rows; an
already grouped same-kind tool burst retains its own nested count. Failed
steps may fold, but the summary keeps an alert glyph and an accessible failure
count. Prose, compaction markers, running tools, and the last segment of a
streaming reply interrupt aggregation.

Opening the summary keeps its header on the right and reveals individual steps
in the left-aligned reading column. Each step independently expands its full
reasoning or wrapping command/path details. Standalone steps also start
collapsed on the right. Reasoning can always be expanded in place; there is
no separate setting that hides or gates its content. A "Thinking…" footer hint
remains available when thinking has not yet produced an inline reasoning segment.

The copy control, elapsed duration and output-token footer stay visible on the
same right rail after completion. During streaming, the amber generating dot
and live timer occupy that rail instead of the copy control.

The input box stays floating at the bottom, with the model selector in the
input area. On send, the GUI creates a Run record and hands the prompt to the
Agent; user messages and events are uniquely persisted by Agent SQLite, and GUI
observers project the event stream in real time onto the UI and additional
storage (approvals, etc.). Desktop keeps its own workspace, Thread, Run
display, and approval store — never a second copy of conversation bodies.

History display and persistence rules:

- First open reads only the latest 10 user rounds, older pages load on
  scroll-up; the tool panel queries independently on demand without slowing the
  body's first load. A single exceptionally large round is still returned as a
  complete round.
- A new conversation still binds its Agent session on first send.
  `null → sessionId` is binding completion, not a conversation switch:
  background calibration does not clear messages or disable the input box.
  Existing view and cache-refresh failures are kept, with error and retry
  offered.
- When the same Thread changes from existing session A to B, explicitly notify
  the context change; keep the old view and block sending until the new history
  is ready; after success switch wholesale — never mix the two sessions.
  Different Threads keep independent view state.
- Old-page loads use a reading anchor to hold position; later layout changes
  like images are corrected by the same viewport logic; out-of-order requests
  and callbacks from invalidated sessions must not overwrite current history.
- First upgrade auto-imports old JSONL, keeping the originals but no longer
  updating them. Corrupt sessions are skipped wholesale and reported; other
  sessions remain usable; a global database failure must not masquerade as an
  empty conversation or fall back to old files. Uncommitted streaming
  increments may be lost on abnormal exit; committed history is the recovery
  basis. Queued prompts are memory-only; the queue is not restored on restart.

Reply endings must be accounted for to the user, using in-message status
dividers — no full-page popups or blank replies to mask errors: manual stop,
output-length limit, and the compaction flow each keep their own hints;
anomalies are shown by evidenced cause — model, network/upstream connection,
timeout, or software/backend-service — with generated content kept. When a
normal end signal is missing and no attribution evidence exists, show "reason
unconfirmed" — never blame the model or the network wholesale. Each protocol's
recognized normal end, refusal, content filter, and pause are handled
separately; no single end marker is required of all models. See
[reply outcome semantics](../../architecture/response-outcomes.md) for the
specific judgments.

Typing `/` opens the unified context-tool and Skill menu. The current context
tool is "Compact"; it only operates on the current conversation, creates no
user message, and reports its lifecycle via a three-state divider in the
message area. Mixed-result ordering and divider rules in 4.8.

Besides the model and thinking-level pickers, the input area offers an
**approval mode** quick-switch dropdown; it is the same global selection as the
settings "General" page's approval mode. macOS shows "Manual approval / Sandbox
protection / Fully open"; Windows shows "Manual approval / Write protection /
Fully open" after the host probe passes; Linux shows "Manual approval / Sandbox
protection / Fully open" after the Bubblewrap host probe passes.

Each message has a **copy button** below it that copies its plain-text content.
The user-message copy button appears on hover; the assistant's copy button and
**duration·output tokens** stay visible on the right after completion. During
assistant **streaming**, an amber **generating indicator** replaces the copy
button alongside the live duration. A **"Thinking…"** hint appears while the
model is thinking but has not yet produced an inline reasoning segment.

The input box supports local file attachments via three methods: attachment
button, copy-paste, and drag-drop; at most 4 images per message, non-images
unlimited. Images go multimodal; other files reach the Agent as a
safely-escaped structured path list — see 4.9 Attachment.

When the Agent requests approval, the approval card inserts above the composer,
not in the right panel. The approval card belongs to the current conversation's
immediate interaction layer; the Agent run stays waiting until the user allows
or denies.

### 5.5 Right context panel

The right panel inspects the current Thread's run context but carries no
immediate approvals. The top shows no separate title or current
session / thread name; a dropdown serves as the current view title, switching
available views by workspace type.

Current focus:

- Runs: the background-program list, distinguishing running and completed
  programs, supporting terminating running programs and clearing completed
  ones. Each program card's main content shows only the `command` field from
  the tool input JSON, displayed as the real command text — never the full
  JSON, escaped text, model names, or internal numbers like `Program <id>`;
  runs without `command` do not enter the Runs list.
- Workspace conversations: show Files, Runs, and Review. Git workspaces'
  Review offers "Git changes" + "Last-round changes"; non-Git workspaces offer
  only "Last-round changes".
- Ordinary Chats: show Files and Runs, no Review.
- Review: change review for the current Workspace conversation, including file
  lists, statistics, and diffs — see 4.7.

The right panel can collapse. Collapsed, it keeps only a lightweight entry
without affecting main-conversation reading.

The Runs panel explicitly does not:

- Show the full run event timeline.
- Show full tool payloads.
- Show long stdout / stderr.
- Show approval history or approval actions.

Those details belong to future dedicated debug / timeline / review views and
must not pollute the daily background-program list.

### 5.6 Colors and design tokens

GUI colors uniformly use the **semantic tokens** defined in
`desktop/tailwind.config.js` (neutral/surface, accent/interaction, the status
triple, diff, shadows) — no raw Tailwind named colors written in components.
Status badges uniformly use the `<Badge tone>` component; colors
distinguishing **sibling categories** (event categories, error subtypes) are an
intentional exception.

The color list, usage quick reference, and anti-patterns are in
[`desktop/COLOR.md`](COLOR.md) — follow it when picking colors for new or
changed components.

### 5.7 Settings: providers and models

The settings panel (gear at the bottom of the left nav; the "Models" shortcut
below New Chat jumps straight to the models page) has three pages:

- **General**: UI language switch (中文 / English, default Chinese, saved
  locally); **approval mode** by platform (macOS: Manual approval / Sandbox
  protection / Fully open; Windows with host probe passed: Manual approval /
  Write protection / Fully open; Linux with Bubblewrap host probe passed:
  Manual approval / Sandbox protection / Fully open — on failure show the
  stable diagnostic code and apt/dnf install hints and keep Manual approval;
  default Fully open `off`, falling back to Manual approval only when sandbox
  is clearly unavailable); **Generate a title after the first answer**
  (on by default; an explicitly saved off choice is preserved). This generates and saves a title in the background using the
  same title-suggestion API as the rename dialog, once after a new conversation's
  first successful run. It never compacts or changes conversation context. Later
  turns, failed/cancelled first runs, and replayed completion events do not trigger
  it. Enabling it does not backfill conversations whose first answer already ended.
  Generation failure leaves the title and successful answer unchanged. A title
  edited while generation is in flight is not overwritten.

**Session title suggestions** can be requested from the rename window in Desktop
and mobile, or by Desktop's first-answer title generation. The generator
calls the conversation's selected model with at most its first three completed
question–answer pairs, excluding tools, reasoning and later exchanges. If none
of the first three user turns has a final answer (for example, a running or
cancelled conversation), it falls back to those turns' visible user text only;
tool commentary is not treated as a final answer. Each side is capped at 2000
characters. The independent, tool-free request uses the current
client's UI language and returns a suggestion of at most 32 display columns.
In the rename dialog it fills the editable input; only Save changes the stored
title. Generation errors leave the existing input unchanged, and late results
cannot overwrite a closed/reopened dialog. The automatic setting uses the same
generator and mirrored Desktop UI language, then saves the title to the Agent
and Desktop store without waiting for manual confirmation. There is no
conversation-prompt instruction or title-generation CLI command. Generating a
suggestion never appends a message or starts a chat run.
- **Providers**:
  - **Built-in FutureGene** (read-only): clicking "Connect" runs the GUI's
    built-in device-code OAuth login — authorization completes in the system
    browser, with a popup showing the verification code and a copyable link
    (the fallback when the browser does not auto-open), and credentials are
    written on success; after configuration it supports "Re-login / Log out".
    The GUI logs in on its own, independent of the CLI.
  - **Custom providers**: add / edit / delete OpenAI-compatible or
    Anthropic-compatible third-party providers (id / name / API type / Base
    URL / API Key), each with an editable model list — per model: id / name /
    image support / thinking support (default on) / context window / max output
    tokens, with field validation (id format and uniqueness, token cap positive
    and not exceeding the context window, model count cap).
- **Models**: lists the Agent's currently available models grouped **by
  provider** (showing the provider name, using the id only when nameless), with
  per-model visibility toggles and search.

A custom model's "thinking support" is a two-state capability toggle,
independent of the session's thinking intensity and the reasoning blocks' local
expand/collapse state.
Off disables the session thinking-intensity picker with an explanation; on
re-enables it. Custom models without `reasoning` filled in default to on;
explicit off is not overwritten by the built-in catalog; built-in and platform
models still follow catalog capabilities.

The dialog's model picker shares a source with the models page: both mark which
provider a model comes from. Provider keys belong to app / model settings.
Configuration storage and the login implementation are in ER.md §6.9.

#### New-user onboarding gate (OnboardingGate)

On first launch, if **no usable provider** is detected (FutureOS not signed in,
no built-in provider API key configured, and no custom provider), the app shows
a fullscreen onboarding overlay guiding the user to two ways of using it:

1. **Sign in to FutureOS** (primary): the main button runs the device-code
   OAuth login flow, with authorization completed in the system browser.
2. **Bring your own API key (BYOK)**: a de-emphasized secondary button; on
   click the overlay disappears and Settings → Providers opens automatically,
   guiding the user to add a custom OpenAI/Anthropic-compatible provider.

The overlay always shows a language switcher in the top-right corner
(Chinese/English), and dev/test builds additionally show an environment
switcher (production / test environment).

**Post-login initialization**: after FutureOS authorization succeeds, the
overlay does **not** disappear immediately — it shows a progress bar in place,
completing three initialization steps in order — ① wait for the Agent to be
ready; ② sync and load models (calling the dedicated `sync_future_models`,
letting the Agent pull the Future model catalog, write the cache, and rebuild
the model registry so the model list is complete); ③ install built-in skills.
The overlay closes only when "models are actually loaded", so users **never**
enter the main UI seeing an empty "no models configured" hint. Initialization
lasts at least 500ms to avoid flicker. The BYOK path does not trigger this
initialization (no built-in skills installed).

**Cancelling during login**: "Cancel" terminates an in-progress login. If a
usable provider already exists, the overlay closes back to the app; otherwise
the overlay returns to its initial state (login + BYOK buttons) without
entering the follow-up flow.

**Re-login from settings**: the Settings → Providers / Account page's
"Connect / Log in" buttons no longer each pop an inline login — they uniformly
bring up this onboarding overlay and **automatically start** the login flow,
reusing the same login + initialization logic.

Overlay display logic:
- No usable provider → show the onboarding overlay.
- FutureOS login succeeds → enter post-login initialization (progress bar);
  after initialization completes and models are ready the overlay disappears
  into the normal three-column layout.
- User successfully adds a custom provider → the overlay auto-disappears
  (re-detection triggered by the `future-auth-changed` event).
- User picks BYOK → the overlay disappears immediately, opening the provider
  settings page.
- Re-connect initiated from settings → the overlay is force-shown with
  auto-login, closing on completion or cancel.

This onboarding overlay **is not a forced login**: any usable provider (FutureOS
or custom) keeps the software fully functional.

## 6. Agent workflow

A typical flow:

1. The user opens FutureOS.
2. The user creates a Chat or picks a Workspace conversation.
3. The GUI creates Thread, Message, and Run.
4. The GUI calls `future-agent` over gRPC.
5. The Agent streams LLM output.
6. The Agent executes `read`, `shell`, `edit`, `write` per model output.
7. High-risk operations enter Approval; the Agent stops at the current tool call
   and waits for the user's decision, with no timeout.
8. The GUI shows the current approval above the composer; on allow the Agent
   continues, on deny the dangerous operation fails and the result feeds back
   to the Agent.
9. The GUI shows text increments, background-program status, tool-activity
   summaries, and end states.
10. After the Run completes, the assistant message, run events, tool calls,
    tool outputs, and approval records are persisted.

Agent tool execution defaults to the current session cwd as the workspace
boundary. Ordinary Chats use the system-created temporary workspace; Workspace
conversations use the user-chosen directory. Out-of-boundary access never
executes silently.

## 7. Phase-one non-goals

The following are out of scope for phase one:

- No compiling the Agent into the GUI Tauri crate.
- No CLI duplicating the GUI's full chat experience by default.
- No complex multi-agent service UI.
- No full PDF reader with fine annotation.
- No complex data-analysis workbench.
- No third-party paid Skill marketplace; Skills come only from the official
  platform catalog (see 4.8).
