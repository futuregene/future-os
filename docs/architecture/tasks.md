# Scheduled tasks architecture

> A scheduled task is a reusable unit of work: a prompt, working directory, model/thinking level and full permissions. A **trigger** decides when it runs, and any of them can also be run on demand. Each run creates or reuses an ordinary conversation and records its outcome and prompt version.

`future goal` manages a long-term goal, evidence, gates and multiple agents. Scheduled tasks run a **fixed action when triggered**. They have separate tables and purposes.

## 0. Scope

- **Crate:** `orchestration/tasks` (`future-tasks`) holds the kernel, types and store, alongside `orchestration/goal`. The executor and its tick loop live in `future-host` (`host/src/tasks.rs`, `host/src/tasks/service.rs`), which is what both desktop hosts and the `future task` CLI share.
- **Hosts:** GUI and headless desktop share the same tick loop. TUI hosting is not connected. Mobile uses the desktop remote bridge.
- **Layers:** the deterministic `next_due`, truncation and prompt-revision helpers are host-independent pure functions. One executor serves both desktop hosts; their difference is a `Notifier` closure (§8).

Implemented: kernel, store, the `future task` CLI (§9), desktop tick loop and panel, headless hosting, mobile bridge/pages, and the `future-scheduled-task` skill. Reserved work: TUI hosting, a `threads.task_id` badge (§2), and an instance-level `.lock`. Current overlap detection assumes one host per home.

## 1. Lessons from the goal control plane

| Goal-plane design | Tasks equivalent |
|---|---|
| Deterministic kernel; agents supply judgments | `next_due`, truncation and `prompt_change_revisions` are pure functions |
| Commit first; refresh compatibility state afterward (`sync_compat`) | notification failure does not roll back a committed run |
| Turn envelope with a schema header and a completion contract | `future_tasks_run_envelope_v1`: identity/run/trigger/settings blocks, the instruction and the completion contract; full-output pointer: `future task output <run-id>` |
| Distinct task classes prevent incorrect frontier claims | `task_runs.kind` distinguishes `main` and `manual` (`chain` is a legacy kind of earlier runs) |
| Separate state root (`<cwd>/.future/goal/`) | `<home>/.future/tasks/`, with its own SQLite database |

Scheduled tasks do not adopt fencing tokens, multi-agent competition, claim stealing or JSONL event sourcing. SQLite supplies durability. The goal plane's monitoring cadence stays separate; task eligibility is decided at tick/claim time instead of adding quota or should-run layers.

## 2. Data model

State lives in `<home>/.future/tasks/`. `<home>` is the host's FutureOS home; `FUTURE_HOME` redirects the entire root, as it does for the Agent. The database is `tasks.db`, using WAL and `busy_timeout=5000`.

The proposed `<home>/.future/tasks/.lock` is **not implemented**. A single host plus `has_running_run` prevents overlapping runs of a task. Two hosts ticking against the same home remain an uncovered case.

```sql
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);   -- schema_version

CREATE TABLE tasks (
  id               TEXT PRIMARY KEY,          -- tsk_...
  name             TEXT NOT NULL,
  enabled          INTEGER NOT NULL DEFAULT 1,
  prompt           TEXT NOT NULL,
  prompt_version   INTEGER NOT NULL DEFAULT 1,
  cwd              TEXT NOT NULL,             -- working directory; an unspecified chat directory uses the conversation workspace
  model_id         TEXT,                      -- provider/model; required in forms, optional in CLI (app default)
  thinking_level   TEXT,                      -- off|minimal|low|medium|high|xhigh; required in forms
  timezone         TEXT,                      -- IANA zone the trigger times are read in; NULL = host's zone, else Asia/Shanghai
  session_policy   TEXT NOT NULL DEFAULT 'new',  -- new | existing
  session_retention TEXT NOT NULL DEFAULT 'keep', -- keep | delete (section 6.1; new sessions only)
  conversation_mode TEXT NOT NULL DEFAULT 'workspace',  -- workspace | chat (new forms default to chat)
  thread_id        TEXT,                      -- host conversation binding for existing policy (created lazily)
  trigger_kind     TEXT NOT NULL,             -- manual | schedule
  trigger_json     TEXT NOT NULL,
  dep_join         TEXT NOT NULL DEFAULT 'all',  -- legacy, unread
  next_due_at      INTEGER,                   -- schedule only; NULL means no future due time
  last_run_at      INTEGER,
  pending_request_at INTEGER,                 -- pending explicit request consumed by tick
  pending_origin   TEXT,                      -- ui | cli | schedule
  pending_actor    TEXT,                      -- agent:<sid> / user / cli / task:<id>
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER
);
CREATE UNIQUE INDEX tasks_active_name ON tasks(name) WHERE deleted_at IS NULL;

-- `task_deps` and `task_dep_state` are legacy: the dependency graph was
-- removed (see §12), nothing reads or writes them any more, and they are kept
-- so an existing database loses no rows to a migration nobody asked for.

CREATE TABLE task_runs (
  id             TEXT PRIMARY KEY,            -- trn_...
  task_id        TEXT NOT NULL REFERENCES tasks(id),
  kind           TEXT NOT NULL,               -- main | manual (chain is legacy)
  origin         TEXT NOT NULL,               -- schedule | ui | cli (chain is legacy)
  actor          TEXT,                        -- agent:<sid> / user / cli / task:<id>
  due_at         INTEGER,                     -- kind=main only
  status         TEXT NOT NULL,               -- running|completed|failed|skipped
  thread_id      TEXT, session_id TEXT, run_id TEXT,
  prompt_version INTEGER,
  result_summary TEXT,                        -- 2000 characters, head…tail; used by lists/downstream envelopes
  result_text    TEXT,                        -- answer (16000 limit, head…tail); fetched separately with run_output
  session_deleted INTEGER NOT NULL DEFAULT 0,  -- conversation deleted under section 6.1; bindings cleared
  feedback       TEXT, feedback_note TEXT,    -- good | bad plus note
  started_at INTEGER, finished_at INTEGER, error_message TEXT,
  UNIQUE(task_id, due_at)                     -- NULL values do not conflict; manual runs are unconstrained
);

CREATE TABLE task_prompt_revisions (
  id         TEXT PRIMARY KEY,
  task_id    TEXT NOT NULL REFERENCES tasks(id),
  version    INTEGER NOT NULL,
  prompt     TEXT NOT NULL,
  source     TEXT NOT NULL,                   -- user | rollback | reflection(historical rows only) | superseded
  status     TEXT NOT NULL,                   -- active | superseded
  reason     TEXT, confidence REAL, source_run_id TEXT,
  created_at INTEGER NOT NULL,
  UNIQUE(task_id, version)
);
```

Scheduled tasks do **not** live in `app.db`. A scheduled task's conversations are ordinary conversations, identified by a title such as `Task name · timestamp`; `task_runs.thread_id` provides the mapping and lets the Runs panel open them.

A nullable `threads.task_id` badge is deferred until needed. It would require a versioned GUI migration under rule 7 in `desktop/CLAUDE.md`, plus the ER documentation update; titles already satisfy the conversation-list requirement.

## 3. Triggers

```jsonc
{ "kind": "manual" }

{ "kind": "schedule", "mode": "once",     "date": "2026-12-24", "time": "09:00" }
{ "kind": "schedule", "mode": "interval", "every_minutes": 120, "anchor": 1760000000000 }
{ "kind": "schedule", "mode": "daily",    "time": "09:00" }
{ "kind": "schedule", "mode": "weekly",   "days": ["mon","wed","fri"], "time": "10:00" }
{ "kind": "schedule", "mode": "monthly",  "day": 31, "time": "09:00" }   // months without day 31 use their final day
```

`next_due(trigger, after)` is a pure function with unit tests:

- **Interval:** `anchor + k·every`, selecting the first future grid point. Scheduling follows the anchor rather than drifting or replaying missed intervals.
- **Daily/weekly/monthly:** calendar scheduling on the task's own clock (see "Time zone" below); a missed calendar slot is caught up once.
- **Short months:** a requested day beyond the month's length uses its final day. UI and CLI expose this rule.
- **Once:** completion sets `enabled=0`.

**Time zone.** `tasks.timezone` names the IANA zone (`Asia/Shanghai`, `Europe/Berlin`, `UTC`) the wall-clock times above are read in, so a schedule keeps its meaning when the host's own zone changes or the task is managed from another machine. Every writer stores a concrete name: the CLI flag `--timezone`, the desktop form and the phone editor all send one, and a client that sends nothing is resolved at write time to the host's own zone — **Asia/Shanghai when that cannot be read**, because an unreadable zone must not silently mean UTC. A row with no stored zone (written before the setting existed) resolves the same way each time the schedule is recomputed. The chosen zone is printed by `future task show`, included in the run envelope (`run settings: … | timezone=…`) and used to render `due=` and the CLI's date columns; the task detail panel shows it next to the working directory. An unknown name is refused with an error rather than replaced. The identifier is what gets stored, not necessarily what is read: the desktop picker names each zone in the UI's language with its offset at the next run (`中国时间 · UTC+8`, `Central European Time · UTC+2`), leads with this machine's zone and the common ones, and answers a search by name, identifier, city or offset. That is display only — the saved value stays the IANA name, so switching the interface language never rewrites a schedule.

## 5. Execution: tick → claim → run

```text
tick (30-second wall-clock interval; Run now also wakes it immediately)
 └─ select enabled tasks with a pending request or an expired schedule
      └─ claim:
           1. read due time
           2. INSERT task_runs(status=running, kind/origin/actor/due_at)
           3. advance next_due_at / clear pending_request_at
           4. spawn execution
      └─ execute:
           1. new session / existing tasks.thread_id (create lazily)
           2. existing session: wait for idle and retain its context
           3. build future_tasks_run_envelope_v1
           4. provision and run_prompt (permission=all, sandbox tier=off)
           5. save status, finished_at, result_summary and result_text
           6. session_retention=delete: delete this run's conversation,
              mark session_deleted and clear its bindings on success
           7. notify (GUI events or headless ledger only)
```

Claim is described as four separate steps, rather than one atomic transaction. Startup reconciliation marks abandoned `running` runs `failed("interrupted")`.

New triggers while a task runs are recorded as `skipped`, rather than queued. Failed conversation deletion retains its bindings and logs the failure; the saved run answer remains available.

### 5.1 Run envelope (`future_tasks_run_envelope_v1`)

Each initial prompt contains the envelope, the task's instruction and its completion contract. Like the goal plane's turn envelope, this uses plain-text blocks rather than XML:

```text
── future_tasks_run_envelope_v1 ──
task: Weekly progress report | id: tsk_... | prompt-version: 3
run: kind=main | due=2026-10-07T09:00:00+08:00
trigger: {"mode":"weekly","days":["mon","fri"],"time":"10:00"}
run settings: cwd=/Users/... | timezone=Asia/Shanghai | conversation=workspace | session=new (fresh conversation per run) | model=... | thinking=high

Instruction:
<original task prompt>

Completion contract:
- This run is unattended. Make and record decisions without ending with a question.
- Finish with a short report: work done, artifacts and paths, changes since earlier runs,
  and remaining uncertainty. It becomes the run's result_summary.
```

The envelope supplies trusted identity and run context, including execution time, model and directory. A chat task without a directory omits an empty `cwd=`. `session=existing` identifies the reused conversation and retains earlier context.

## 6. Session policy

| `session_policy` | Behavior |
|---|---|
| `new` (default) | Create a fresh conversation per trigger, titled `Task name · 2026-10-07 09:00` |
| `existing` | Reuse one conversation, creating `tasks.thread_id` lazily; append directly without pre-compaction |

`conversation_mode=workspace` requires `cwd` and associates the conversation with that workspace. The server rejects an empty directory. `chat` uses the conversation's temporary workspace and needs no directory; both client forms default to chat. The CLI can still supply `--cwd`.

Reusing a conversation is intended to preserve earlier results. Pre-compaction would process the entire history and remove context the next run needs. Users can manually compact when desired; manual `compact` RPC remains mutually exclusive with a run.

### 6.1 Conversation retention

`session_retention` applies only to `session_policy=new`: `keep` is the default; `delete` removes the run's conversation during completion. Save `result_text` before deleting. On successful deletion, set `session_deleted` and clear thread/session/run bindings. On failure, retain the bindings and record the error. The `existing` policy requires `keep`.

## 7. Prompt version history

CLI `edit --prompt`, desktop edits and mobile edits retain the replaced prompt and record a new version, allowing rollback to the initial version. There is one write-decision entry point: pure `kernel::prompt_change_revisions`. It returns the one or two rows to persist and the resulting version; callers must not separately increment `prompt_version`.

- **Backfill:** if the current version was never recorded, the first edit records it before the new version. This fills older v1 gaps without a migration.
- **Sources:** `user`, `rollback`, `reflection` (historical rows only), and `superseded` for a replaced version whose original source is unknown.
- **One active version:** `store.insert_revision` supersedes earlier active rows in the same write.
- **Apply:** `prompt apply` (CLI) calls `tasks::accept_revision`, recording a new `rollback` version and retaining the replaced version as superseded. Applying the active version is a no-op. The desktop and mobile UIs once carried their own Apply button; they no longer read the revision list at all (§11), so rolling a prompt back is a CLI/skill operation.

### 7a. Removed: reflection-based prompt proposals

Automatic post-run reflection, proposed prompt rewrites, proposal cards and acceptance controls, CLI `--reflection` and the proposal portion of `prompt log` have all been removed.

- `store::MIGRATIONS` Exec steps remove `tasks.reflection`, `kind='reflection'` run rows and `status='proposed'` revision rows. Removing old running reflection rows also prevents a permanent `has_running_run` block.
- Keep accepted historical `source='reflection'` active/superseded rows: they are real prompt history, so `prompt log` still names that source; Rust retains a read-only constant.
- `kernel::LEGACY_PROPOSED_STATUS` skips old proposals when deciding whether the current version was recorded. New code never writes that status.
- Wire/UI cleanup removed `pendingProposals`, revision `sourceRunId`, CLI list/show `reflection`, and proposal-specific translation keys. The app UIs no longer read the revision list at all (§11); `prompt log|apply` is the only reader of it again. The desktop host still answers `list_task_revisions`/`apply_task_revision` over its remote protocol, because paired clients of older builds may still ask.

## 8. Hosts: `Notifier`, not an Executor trait

One executor lives in `future_host::tasks` (`host/src/tasks.rs`); the desktop module of the same name is only the notification adapter. Only its completion notification differs between hosts:

```rust
pub type Notifier = Arc<dyn Fn(Option<&str /* thread_id */>) + Send + Sync>;

// future-host: the executor and its loop
pub fn start_with_notifier(notify: Notifier)       // resident supervision, 30s tick
pub fn start_headless()                            // ledger-only notification
async fn run_loop(notify: Notifier)                // reconcile, then tick every 30s

// desktop/src-tauri/src/tasks.rs: the GUI adapter
pub fn start<R: tauri::Runtime>(app: AppHandle<R>) // emit_remote_activity + emit_threads_updated
```

An Executor trait would add indirection around five thin `agent_bridge` wrappers used by this one implementation. GUI and headless both call `run_loop`; headless omits the notification outlet.

- **GUI desktop:** `desktop/src-tauri/src/lib.rs` setup calls the local `tasks::start`, which installs the notifier and calls `future_host::tasks::start_with_notifier`.
- **Headless desktop:** `cli/src/headless/mod.rs` calls `future_host::tasks::start_headless`.
- **Mobile:** manages tasks through the remote bridge (`tasks_v1`), without hosting.
- **TUI:** hosting is not connected. The crate and CLI already work; a panel is future work.

## 9. CLI (`future task`)

```
future task list [--all] [--json]
future task show <id|name> [--json] [--prompt]
future task add --name N --prompt P|--prompt-file F [--cwd D]
                [--model M] [--thinking L] [--session new|existing]
                [--conversation workspace|chat]
                (--manual | --at | --every | --daily | --weekly | --monthly)
future task edit <id|name> [any add flag] [--enable|--disable]
future task enable|disable <id|name>
future task remove <id|name> [--yes]
future task run <id|name> [--wait] [--timeout 15m] [--json]
future task runs <id|name> [--limit N] [--json]
future task output <run-id> [--tail N] [--json]
future task feedback <run-id> good|bad [--note "…"]
future task prompt log|apply|revert <id|name> [revision-id]
```

- The CLI reads/writes the shared `tasks.db`; writes take effect on the host's next tick. It never executes a run itself.
- `run` queues by default and reports that fact. `--wait` polls the ledger until terminal; JSON includes `status`, `runId`, `threadId` and `resultSummary`.
- `output <run-id>` supplies the envelope's full-output pointer. Prefer the answer saved on that run, which survives conversation deletion. Only older runs lacking a saved answer fall back to the conversation's last assistant text. Print task/status/prompt-version/session identity before the body; when the fallback conversation has been reused by a newer run, identify that run to avoid misattribution. `--tail N` prints only the end.
- `prompt log` lists versions and identifies the active one. `prompt apply <revision-id>` creates a new rollback version unless that version is already active; `prompt revert` applies the previous version.
- `runs` and `prompt log` remain readable for soft-deleted tasks by ID or name; they no longer appear in `list`.
- `enable` recalculates the next scheduled time; `remove` soft-deletes the task (its runs and conversations are kept).
- Help distinguishes tasks from `future goal todo`.

A test in `cli/src/commands/task.rs` parses `help::TASK_HELP` and requires the same subcommand set as the dispatcher, in both directions. This guards against documenting commands that produce `Unknown argument`.

## 10. The `future-scheduled-task` skill

`skills/builtin/future-scheduled-task/SKILL.md` lives in the skills submodule and has its own PR. It focuses on writing prompts: eight unattended-run constraints, trigger selection and the short-month rule, the `run → assess → feedback → edit → rerun` loop, and using `future task run --wait --json` from other agents. External side effects require user consent.

Real Agent/model verification is manual, outside CI. Desktop `tasks.rs` has ignored `a_queued_run_against_a_real_agent_*` tests and `serve_pending_runs_until_idle`, a one-shot host using the same tick as headless without login or the remote bridge:

```bash
# Agent and host must share HOME so managed chat workspaces agree.
HOME=/tmp/try FUTURE_HOME=/tmp/try/.future future agent --grpc-addr 127.0.0.1:5099 &
FUTURE_AGENT_GRPC_ADDR=127.0.0.1:5099 future task add … && future task run NAME
HOME=/tmp/try FUTURE_AGENT_GRPC_ADDR=127.0.0.1:5099 \
  cargo test --lib tasks::tests::serve_pending_runs_until_idle -- --ignored --nocapture
```

Skill commands must match implemented commands in §9. Earlier skill instructions named commands before implementation; those were aligned with the actual command surface. Implemented companion commands now include `edit`, `enable`, `disable`, `remove`, `feedback` and `prompt log|apply|revert`.

There is no automated skill/CLI version guard: skills are a separate repository and parent CI does not check out that submodule. Help/dispatcher tests cannot catch cross-repository drift. Update this document and the skill together when changing that contract. Built-in reflection was removed (§7a); the skill must not promise automatic prompt proposals.

## 11. UI

- **Desktop:** Scheduled tasks appear after Skills and before phone control in both ActivityRail states. Its panel has a list, editor and run ledger. This is a two-column task view; it omits the conversation's right context panel.
- **Detail order:** show run results first. Settings and prompt are lower-frequency definition work, reached through Edit. A new scheduled task opens its form immediately because it has no history.
- **Desktop definition:** a Settings disclosure renders definition nodes only while open, rather than putting a long form below a scrolling history.
- **Mobile definition:** Settings pushes a separate `TaskForm` layer (`settingsOpen`), hiding runs while editing. Back and successful Save return to detail; failed Save preserves the form/draft for retry. A separate layer avoids placing a multi-screen form after run history.
- **Mobile entry:** “Scheduled task settings” in the Settings stack, gated by `tasks_v1`. Lists omit prompt bodies to meet wire budgets; detail fetches them separately and shows the run ledger.
Both forms default new scheduled tasks to **chat conversations**, requiring a directory only for workspace mode. Model and thinking level must be selected explicitly; there is no Default option, and validation identifies the missing field. Model lists include enabled models plus the task's currently bound model even if disabled, avoiding an accidental model change while editing other fields.

**Immediate runs:** desktop/mobile Run now writes `pending_request_at`, then calls `tasks::wake()`. The tick loop remains the sole claimer but wakes immediately rather than waiting for the 30-second interval. Start/end `threads-updated` events refresh the list and open detail so they do not stay at Queued.

**A definition changed elsewhere also refreshes:** `threads-updated` says a conversation moved, but when the phone disables, edits or deletes a task on its "Scheduled tasks" page it writes the same store *in this very process*, and the desktop panel had no signal at all — a manual refresh was the only way to see it, which is how one field report arrived. The remote task writes (create / update / delete / set_enabled / run / apply_revision) therefore publish `tasks-updated` on success, and the desktop's `useTasks` re-reads the list on it (local backend only: a panel pointed at another host is refreshed by that host's own pushes).

**Three requirements for the mobile form** (2026-10-22, from field reports):

- **The time is chosen, not typed.** The desktop is an `<input type="time">` and gets the system picker; the phone has no such element, and its text field needed the keyboard, which then covered the bottom of the form (Android edge-to-edge does not resize the window — see `lib/useKeyboardHeight`). `TimeField` now offers two strips of chips: all 0–23 hours expanded, and 0–59 minutes in one horizontal strip that opens scrolled to the current minute. A pick writes `HH:MM`; no keyboard is involved. The settings stack also gained keyboard avoidance, so the fields that still need typing (name, prompt) can no longer push Save under the keyboard.
- **The draft survives a cold start.** The form has a dozen fields and the phone can be killed mid-edit. The draft is keyed by "Desktop + task (`create` for a new one)" in AsyncStorage (`features/settings/taskDraftStorage.ts`), read back when the form opens and written 250 ms after a change. A successful save, cancel, delete or a step back clears it, so the next open cannot resume an edit the task no longer has. An edit draft carries `promptVersion` and is dropped once the prompt changed elsewhere. Stored fields are validated one by one; a bad value only resets that field to its default.
- **The entry button says Edit**, matching the desktop (`tasks.edit`), not "Settings".

**Prompt versions are not a client feature.** Both app UIs hide the history and offer no rollback: the version in force is a CLI/skill concern (`prompt log|apply|revert`), and a client that showed a list could only mislead about which version a run used. Edits through either client still record a version server-side, so the CLI sees the same history (§7). Each run still has its own card with status, kind, version, time and Open conversation; a long summary does not run into the next card's header.

Desktop opens the run's thread. Mobile opens `run.sessionId` when available, closes Settings through `SessionsScreen`, then selects that conversation. Older desktop hosts lacking this field have no mobile conversation link. Chat is a separate top-level screen rather than part of a mobile navigation library.

The run ledger follows the task's actions. Desktop's Settings disclosure precedes the ledger; mobile keeps definition editing on its own page. The task page registers list → detail → settings with `SettingsPageBack` (`features/settings/pageBack.ts`): Android back and the header arrow pop one layer, preserving list/detail state rather than returning directly to the Settings home.

## 12. Out of scope

Removed: task dependencies (edges, joins, chained runs, the `--depends-on`/`--join-any` flags and both dependency editors) — a scheduled task is started by its own schedule or on demand. The host IPC commands `list_task_deps`, `set_task_dep` and `remove_task_dep` still answer for clients of older builds (an empty list; an explicit refusal), and go away once no shipped client asks for them (`host/src/tasks/service.rs`). Out of scope: cron expressions, orchestration between tasks, OS background services, shell/goal/skill action kinds, system notifications and per-task tool allowlists. Full permissions remain the only policy.

## 13. Core invariants

- Desktop is the executor; headless shares the implementation with fewer notifications. CLI/mobile only queue work and read/write state.
- v1 uses `permission_level="all"` and sandbox tier `off`.
- Five schedule forms, plus on-demand runs; short months use the month's final day.
- A manual run consumes nothing: there are no dependency markers left to consume.
- `prompt_change_revisions` is the single prompt-version write-decision entry point, with exactly one active version at a time.
