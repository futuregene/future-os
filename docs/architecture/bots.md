# Bots and Channels design

[中文](bots.zh-CN.md)

Status: design draft. **Implementation has not started**: this document is for
review and scheduling only, and the work begins as a separate task. Updated:
2026-11-04.

This document is design only: no code changes, no configuration changes, no bot
is created, and nothing under `~/.future/` is touched. "Supported today" always
means a mechanism that already exists in this repository, with the code location
and the command line given so it can be checked; anything that needs a change
gets its own section and is never mixed into the "today" list.

The problem it solves: make "bot" and "channel" first-class objects in the
desktop app, so a user organises work by role rather than by task, while
guaranteeing that the bot side of the app and the user's own conversations are
**completely isolated**.

---

## 1. What Grok Bot does

Only claims with a source. The product documentation lives on `docs.x.ai`,
carried by Cursor's account and billing system. A third-party write-up of xAI's
72-hour launch stream (Field Notes) fills in failure modes the official docs do
not cover; it is **not** official documentation and is labelled wherever used.

| Fact | Source |
|---|---|
| A Bot is a durable teammate with a name, a job, its own conversation and context that compounds — not a chat thread | [Create and manage Bots](https://docs.x.ai/grok-bot/bots) |
| All bots share **one cloud computer**: files are visible to every bot, browser cookies and command-line credentials are shared; each bot gets its own screen, which is a parallel work surface and **not a security boundary** | [Use the computer and apps](https://docs.x.ai/grok-bot/computer-and-apps) |
| That computer has one shared workspace at `/workspace` | same page, "Work with files" |
| Memory is **per bot**, long-lived, and the bot decides on its own when to save; a few facts about you (name, time zone) are shared by all bots | [Create and manage Bots](https://docs.x.ai/grok-bot/bots), "What a Bot remembers" |
| In a group chat a bot **can use what you told it in your private chat**; each bot keeps its own memory and it is not shared with the other bots there; everyone in the group can read what it says | same page, "Across chats, Bots, and calls" |
| A group holds 2–6 bots; `@Bot` directs a message and `@everyone` addresses the group; writing normally lets "the participating Bots decide who should respond" | [Message and collaborate](https://docs.x.ai/grok-bot/chat-and-collaboration), "Start a group chat" / "Direct a message" |
| Bot-to-bot messaging is an **asynchronous handoff**: the receiver wakes, handles it and may reply later; the handoff shows in the bots' own chats, not in the group | same page, "Let Bots hand work off" |
| Approval requests, secret requests and email/Slack drafts **do not appear in group chats**; a bot's message to a group is text-only | same page, "Start a group chat" |
| There is a formal "primary Bot": the main point of contact, which **proactively** reports progress and coordinates with the other bots; without one the product shows an "Introducing your primary Bot" card to create it in one step or nominate an existing bot; it is replaceable; a shared bot cannot be primary | [Create and manage Bots](https://docs.x.ai/grok-bot/bots), "Your primary Bot" |
| Existing bots can **suggest or create** a focused new bot when a job deserves a long-lived owner; the docs also say to ask before creating several so the roster stays small | same page, "Create a Bot" |
| A skill library is **one per account** and every bot sees it; there is no per-bot attach | [Skills and routines](https://docs.x.ai/grok-bot/skills-routines-and-automations); see also [`grok-bot-cli`'s `gbot skills`](https://github.com/ScriptedAlchemy/grok-bot-cli) |
| A routine belongs to **one bot**, fires on a schedule or an event, and runs in the cloud | [Skills and routines](https://docs.x.ai/grok-bot/skills-routines-and-automations), "Create a routine" |
| Failure mode one: every bot in a group answers every message, so they are "eager", "love to talk" and talk over each other | [Field Notes · ANTIPATTERNS §35](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/ANTIPATTERNS.md) (third party) |
| Failure mode two: routine frequency is the biggest cost leak (every 15 minutes is hundreds of messages a day) | same file, §34 (third party) |
| Failure mode three: a **bot factory overfits** — "a bot-factory bot writes it for new bots and tends to overfit it to the one scenario it was created for" | [Field Notes · PRODUCT](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/reference/PRODUCT.md) (third party) |
| The practitioner "chief of staff" pattern: build every other bot *through* it, let it onboard new bots itself, and let it hold who is working on what | [Field Notes · ORCHESTRATION](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/agents/ORCHESTRATION.md) (third party) |
| Duplicating a bot carries profile, skills, routines and avatar, but **not** chat history, learned memory or attachments | [Create and manage Bots](https://docs.x.ai/grok-bot/bots), "Share a Bot" |

Three consequences decide this design: context is **layered** (bot-private ×
shared × situational); waking there is a **model judgement** rather than a kernel
rule; isolation lives at the **memory and conversation** layer, never at the
filesystem.

---

## 2. Three trade-offs against Grok Bot

| # | Trade-off | Grok Bot | This design | Cost |
|---|---|---|---|---|
| D1 | A bot has its own workspace | No: one computer and one `/workspace` for the account | One workspace per bot, holding its identity and memory | Handoffs must be explicit; files are not automatically visible |
| D2 | Waking is decided by the kernel | The participating bots decide; in practice they talk over each other | Three speaking policies; `eager` uses a **cheap judge** instead of a full run per member | Needs a judge, and judges are not always right (§5) |
| D3 | Memory is files | Server-side, no screen for it, only "what do you remember?" in chat | `FUTURE.md` plus `.future/memory/*.md`: greppable, diffable, hand-editable | Files do not summarise themselves; missed entries need a human |

**The one thing we do not copy**: sharing a single computer. It trades isolation
for convenience, while a FutureOS workspace *is* the unit of permission and
context; copying it would throw away the only structural advantage we have.

---

## 3. Concepts and data model

### 3.1 Concepts

| Concept | Definition |
|---|---|
| **Bot** | A workspace (`AGENTS.md` states the job, `FUTURE.md` is the memory index, `.future/memory/*.md` holds the details), a model and thinking level, a name and avatar, and **one private conversation** |
| **Channel** | Members (≥ 2 bots, plus people), a **channel workspace** (the shared board), a transcript ledger, a charter and a speaking policy |
| **Binding** | The `(bot × channel)` session: that bot's context container in that channel, created lazily |
| **Turn** | One utterance the routing kernel dispatches: a real run whose final text becomes a transcript entry |
| **Chief bot** | The bot whose `is_primary` is set: it routes, maintains the roster and creates new bots for the user. Not a privileged agent — a bot with a different job |
| **Session steward** | The agent inside the user's own ordinary conversations. It must be unable to see or reach bots' and channels' conversations (§8) |

A bot being exactly one workspace is deliberate: with `cwd` set to the bot
workspace, the existing prompt builder injects `AGENTS.md` (the job) and
`FUTURE.md` (the memory index). See §10.1.

### 3.2 Tables and database

Definitions, membership and the transcript ledger live in a **separate
database**, `~/.future/bots/bots.db`, a sibling of `tasks.db` and `fusion.db` and
built the same way: `store_dir` / `store_path` / `open` /
`open_existing_read_only`, `PRAGMA busy_timeout`, and `meta.schema_version`; the
template is `orchestration/tasks/src/store.rs`.

Why it is not folded into `app.db`: `app.db` carries startup, the three
conversation lists and run status and is the most-read database in the app, while
a transcript is a high-frequency append; putting these tables there would force
the kernel crate to depend on `host::store` and weld the control plane to the
presentation layer; `app.db` is a released schema where new tables need a
versioned migration, whereas a new database creates itself on first use with no
migration risk at all; and the transcript is append-only, may need its own
reclamation, and does not share threads' update/archive semantics.

Isolation does not require one database: the decision point is `threads.scope`
on the table the enumerator already reads (§8.2), not a join against `bots`.

```sql
bots(
  id TEXT PRIMARY KEY, name TEXT NOT NULL, job TEXT NOT NULL,
  avatar_shape TEXT, avatar_color TEXT,
  workspace_id TEXT NOT NULL,                   -- soft reference: app.db workspaces.id
  model_id TEXT, thinking_level TEXT, permission_level TEXT,
  dm_thread_id TEXT,                            -- soft reference: app.db threads.id
  is_primary INTEGER NOT NULL DEFAULT 0,        -- at most one per home
  created_by_bot_id TEXT REFERENCES bots(id),   -- audit only, grants nothing
  pinned INTEGER NOT NULL DEFAULT 0, hidden INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER
);
CREATE UNIQUE INDEX bots_one_primary ON bots(is_primary) WHERE is_primary = 1;

channels(
  id TEXT PRIMARY KEY, name TEXT NOT NULL,
  charter TEXT NOT NULL DEFAULT '',
  workspace_id TEXT NOT NULL,                    -- soft reference: app.db workspaces.id
  summary TEXT NOT NULL DEFAULT '',
  speak_policy TEXT NOT NULL DEFAULT 'router',   -- mention_only | router | eager
  router_bot_id TEXT REFERENCES bots(id),        -- NULL = use the chief bot
  allow_private_context INTEGER NOT NULL DEFAULT 0,
  max_turns INTEGER NOT NULL DEFAULT 8,
  max_depth INTEGER NOT NULL DEFAULT 2,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, archived_at INTEGER, deleted_at INTEGER
);

channel_members(channel_id TEXT NOT NULL, bot_id TEXT NOT NULL,
  role_note TEXT NOT NULL DEFAULT '', joined_at INTEGER NOT NULL,
  PRIMARY KEY (channel_id, bot_id));

channel_bindings(channel_id TEXT NOT NULL, bot_id TEXT NOT NULL,
  session_id TEXT, persona_hash TEXT,
  PRIMARY KEY (channel_id, bot_id));

channel_turns(
  id TEXT PRIMARY KEY, channel_id TEXT NOT NULL, seq INTEGER NOT NULL,
  author_kind TEXT NOT NULL,                 -- user | bot | system
  bot_id TEXT, text TEXT NOT NULL,
  run_id TEXT, session_id TEXT,
  status TEXT NOT NULL,                      -- queued | running | done | failed | cancelled
  mentioned_json TEXT NOT NULL DEFAULT '[]',
  wake_reason TEXT,                          -- which mention or rule woke it
  error_message TEXT, created_at INTEGER NOT NULL, finished_at INTEGER);
CREATE UNIQUE INDEX channel_turns_seq ON channel_turns(channel_id, seq);
```

The only change to an existing table, in `app.db`:

```sql
-- a versioned migration in host/src/store/migrations.rs; schema.rs mirrors it for fresh databases
ALTER TABLE threads ADD COLUMN scope TEXT NOT NULL DEFAULT 'user';  -- user | bot | channel
```

`scope` is the single source of truth for isolation (§8), not a derived field:
every thread recorded in `bots.dm_thread_id` or `channel_bindings` must carry the
matching `scope`, and the consistency sweep in §3.3 catches a mismatch.

Reuse rather than new tables: `workspaces` (one row per bot and per channel, each
`kind='user'`, so they show in the sidebar and the folder can be opened), and
`threads` (a bot's private conversation is an ordinary thread). Conversations
themselves stay in the agent's database; `created_by='bot'` is only a mark.

### 3.3 Cross-database consistency

The price of a separate database is **no cross-database transaction**. There are
exactly three such operations, and each is pinned down:

| Operation | Order | Crash in between: symptom and repair |
|---|---|---|
| Create a bot | ① write the workspaces row → ② create the thread (`scope='bot'`) → ③ write the bots row | Crash after ①/②: an orphan workspace or thread; the startup sweep reclaims any `scope='bot'` thread and workspace no bots row references |
| Create a channel | ① write the workspaces row → ② write the channels row | Orphan workspace, reclaimed the same way |
| Delete a bot or channel | ① stop the executor and clear the queue → ② soft-delete the bots.db row → ③ delete the thread and workspace | Crash after ②: the bot is gone but its thread remains — already excluded from user lists by `scope='bot'`, and the startup sweep deletes it |

Three rules:

1. **Single writer**: the host is the only writer of `bots.db` (the GUI and
   headless hosts share one service), the same assumption tasks makes. Other
   processes read it, or go through the service.
2. **A dangling reference degrades, never errors**: when the thread behind
   `dm_thread_id` is gone, the bot shows as "conversation deleted, can be
   recreated"; it never blocks a list and never throws. This is the same pattern
   as tasks' `session_deleted`.
3. **The startup sweep is idempotent**: it does two things only — reclaim
   unowned workspaces and threads, and finish deleting the threads of bots that
   are already gone. It does not infer intent and never recreates a conversation.

Directory layout:

```
~/.future/bots/
  bots.db                    # definitions, membership, bindings, transcript ledger
  agent-home/                # the bot agent instance (FUTURE_HOME; see §8.3)
  workspaces/<slug>/         # default workspace per bot (an ordinary user workspace)
```

The state directory and the workspace directory are separate, and neither lives
under `~/.future/workspaces/chat/` — that directory holds per-thread workspaces
and is reclaimed. A bot may also point at an existing user workspace, in which
case no directory is created.

### 3.4 CLI surface

Consistent with the other control planes: read and write the same `bots.db`, add
no backend.

```
future bot list|show|new|rename|set|hide|delete
future channel list|show|new|add|remove|post|policy
```

`future bot new` is the single entry point for "create a bot in one sentence": a
person and a bot use the same path.

---

## 4. Context assembly

### 4.1 Three layers

| Layer | Contents | Lifetime | Injection |
|---|---|---|---|
| **L1 bot-private** | The job (`AGENTS.md`), the memory index (`FUTURE.md`, linted to 30 entries / 100 lines / 12 KB), detail memory read on demand, the private conversation's history | Permanent | `cwd` injects the first two; the private history lives in the bot's own session |
| **L2 channel-shared** | Charter, member roster (name plus one-line job), the full bot roster, the channel workspace path, the channel summary, the speaking policy, the speaking rules | Channel lifetime; refreshed only when a content hash changes | Session-level `append_system_prompt` |
| **L3 turn envelope** | Who woke you, that message, the transcript window, what this turn asks | One turn | Prefix of the user message |

### 4.2 The assembled shape

System prompt (one per `(bot × channel)` binding):

```
<existing identity / tools / approval / communication sections — unchanged>

# Project Context            ← AGENTS.md from cwd (the bot's job)
# Workspace Memory           ← FUTURE.md from cwd (the bot's memory index)
# Available Skills           ← the global skill library (shared by the account)

<append_system_prompt — the channel layer>

# Channel: Launch
Charter: <charter>
Members (the speaking policy decides who answers):
- @Writer (writes the launch post)
- @Reviewer (lists blocking issues only)
- @you (Research)
Other bots on this machine (not in this channel; use only to route):
- @Support (customer issues)
Channel workspace: <channel-workspace> (shared by everyone)
Channel summary: <summary>

# Speaking rules
- Your turn ends with one channel message; lead with the result, do not restate
  what others already said
- To involve someone, @ them and say what you need; not @-ing means they stay out
- The channel workspace is yours to read and write; your own workspace is
  private, keep private material out of the channel
- If you need a file someone else holds, give an absolute path and @ them; do not
  assume you can reach it
- Never list, open or reference this machine user's conversations; you can only
  see this channel and your roster
```

User message (per turn):

```
[channel, most recent N messages]
Research · 10:02 · <text, truncated to 800 characters per message>
Writer   · 10:04 · <text>
[now] @Research said: <text>
[you were @-ed] Answer the message above; if nobody else is needed, @ no one.
```

### 4.3 Placement rules (backed by code)

1. **The append segment comes after project context and workspace memory**, so it
   can override the bot's own `AGENTS.md`. The channel layer must therefore be
   **purely additive**: it never restates or rewrites the job. Evidence: the
   section order in `agent/src/prompt/mod.rs::build_prompt`, and
   `agent/src/rpc/session_prompt.rs::build_system_prompt`, which reads the append
   segment after context and memory.
2. **Stable prefix first**: L1 and L2 go into the system prompt, L3 into the user
   message; L2 is refreshed only when `AGENTS.md` / `FUTURE.md` / the charter /
   the member list changes (by content hash). Measured in this repository:
   substituting the system prompt, dropping the tool definitions, or adding one
   line each gave **0%** cache on a primed prefix while an identical-shape
   request gave 93.7%, so the whole conversation is billed again
   ([compaction design](../internals/compaction/compaction.md), the cache and
   prefix section).
3. **Never inject identity twice**: L1 already comes from `cwd`; L2 must not copy
   it, or the job appears twice in one prompt and the model follows the later
   copy.

### 4.4 Budgets and truncation

| Item | Default | Note |
|---|---|---|
| Transcript window | 3k tokens, adapted to the model's window | Trim whole oldest messages, never half a message |
| One message | 800 characters, head…tail | Same shape as `task_runs.result_text` |
| Rolling summary | `channels.summary`, ≤ 1200 characters | Represents trimmed messages; written by a separate summary run |
| Summary trigger | > 2000 characters or > 8 trimmed messages | Stops summarising every turn |
| Private context | **Not** included by default | Channel flag `allow_private_context`, default 0 |

### 4.5 Fusion rules

- **A bot writes only its own memory**: it writes `FUTURE.md` and
  `.future/memory/` inside its own workspace. Context crosses bots only as
  spoken text or as files in the channel workspace, matching Grok Bot's "nothing
  is shared between bots unless one messages another"
  ([PRODUCT.md](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/reference/PRODUCT.md)).
- **The channel summary is not memory**: `channels.summary` is a channel fact
  injected only into turns taken in that channel; it is never written into a
  bot's `FUTURE.md`.
- **Process stays out of the transcript**: a turn contributes its final text plus
  one activity line, and the full tool process stays in the binding session
  behind a disclosure. The cost is that you cannot watch others work; a status
  line stands in.
- **Failures do enter the transcript**: a `failed` or `cancelled` turn writes a
  system message naming who failed, why, and how to retry. Grok Bot's group chat
  has no such slot, and the observed result is "it looks like nobody is answering
  you".

---

## 5. Speaking policies

`channels.speak_policy` has three settings; the difference is whether "deciding"
and "working" are separated:

| Policy | Model calls per user message | Feel | Cost order |
|---|---|---|---|
| `mention_only` | K real runs (K = the number of bots @-ed) | an extension number | 1× |
| `router` | 1 cheap judgement + K real runs | close to "it answered on its own" | ≈1× |
| `eager` | N cheap judgements (parallel) + K real runs | matches "let them decide" | ≈1×, N ≤ 6 |

The difference is one of magnitude: a real run carries the identity section, tool
definitions, the skill catalogue and the memory index — tens of kilobytes — while
a judgement carries one job line, the last three messages and the new message: a
few hundred tokens, and no tools. So `eager` asking every member costs far less
than "every member runs a full turn", which is exactly where Grok Bot's
group-chat cost comes from
([ANTIPATTERNS §35](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/ANTIPATTERNS.md)).
The exact ratio needs measurement (§15).

**The default is `router`.** A new channel includes the chief as a member (§9.2)
and lets it judge; when the user declines to create a chief the channel falls back
to `mention_only` rather than pretending to have a judge. `eager` is off by
default and stays available as a channel-level option.

The judge's contract:

- Input only: the machine's roster (name plus one-line job), this channel's
  members, the last three messages, the new message, and the judge's own one-line
  job.
- Output only: a list of `(bot_id, one-line reason)`, or "nobody".
- It has no tools, does not read memory or the workspace, and therefore misjudges
  sometimes. Two escape hatches: it may answer `maybe`, which becomes a real run
  only while budget remains; and a wrongly woken bot may say in its reply that
  the message was not its job, which still costs one run and counts against the
  budget.
- A failed judgement (timeout, model error) **falls back to `mention_only`** and
  writes a system message. It never goes silent.
- `wake_reason` is recorded in `channel_turns`, so "why did it answer" has an
  explanation.

---

## 6. The routing kernel

Lives in `orchestration/bots`, shaped like goal / tasks / fusion: a deterministic
kernel of pure functions over its store, with the executor on the host.

```rust
mentions(text: &str, roster: &[BotRef]) -> Vec<BotId>
enqueue(turn: &Turn, mentioned: Vec<BotId>) -> Vec<QueuedTurn>
judge(roster, recent, message) -> Vec<(BotId, String)>   // router / eager only
should_stop(queue, budget, seen) -> Option<StopReason>
```

`mentions` rules:

1. A mention is `@` plus a **whole-name** match against the roster, longest name
   first; the name must be followed by whitespace, line end or punctuation.
2. `@` must be preceded by whitespace or line start (`mail@host`, `src/@scope` do
   not count).
3. Fenced code blocks and `>` quote lines are not parsed.
4. Names match case-insensitively but resolve to the canonical name.
5. The author is excluded; a repeated mention counts once, at its first position.
6. `@` plus anything that is not a roster name is plain text: no error, no wake.

Stopping and budgets:

| Rule | Default | Why |
|---|---|---|
| Bot turns per user message | 8 | Directly answers the "every bot answers every message" cost pit |
| `@` chain depth | 2 | One bot tagging two others once is usually the whole need |
| A bot may not speak twice in a row | Hard rule | Prevents self-loops and two bots burning the budget on each other |
| A turn that `@`-s nobody | Ends the chain | Silence terminates |
| `@` a non-member | No wake; a system note | No implicit joining |
| `@all` | Allowed, counted against the budget | Keep the capability, make the cost visible |

"Who was @-ed" has an exact string answer and is therefore code; "should I answer
this" is a semantic judgement and is therefore the judge's, whose output must be
explainable and must fail back rather than fail silent.

---

## 7. The executor

`host/src/bots/executor.rs`, shaped like the tick loop in `host/src/tasks.rs`
(one service shared by the GUI and headless hosts; the Tauri side is a thin
adapter, as in `desktop/src-tauri/src/commands/tasks.rs`).

- **Serial within a channel** to keep transcript order; **at most two channels in
  parallel**, matching fusion's concurrency cap.
- **One turn is one real run**, addressed to the binding session:
  `future run --session <sid> --model … --thinking … "<envelope>"`.
- **Turn end**: the final text goes into `channel_turns` (`status='done'`); its
  `@` mentions are parsed, the speaking policy decides whether a judgement runs,
  and the next turns are dispatched. No mention and no judgement answer ends the
  chain.
- **Timeout**: on the order of 600 seconds; abort first, then write a system
  message.
- **Approvals**: a bot's action can park on one. The channel shows a single
  "Waiting for approval" line; the approval card stays in the bot's private
  conversation and the global pending list, matching Grok Bot's "approvals do not
  appear in a group". A channel therefore **stops when nobody is there** —
  intended, but it must be visible (§11.2).
- **Cancellation**: deleting a channel or closing a bot stops first, then
  deletes, following the goal plane's "delete must stop first" lesson.
- **Refresh**: every successful write publishes `bots-updated`, the same way
  `tasks-updated` works — otherwise "a bot created on the phone is missing on the
  desktop" reproduces the field report the task panel already had (see
  [tasks.md](tasks.md) §11).

---

## 8. Conversation isolation (hard requirement)

Three requirements, all mandatory:

1. The Recent and Workspace lists **must not** contain conversations created by
   bots.
2. The bots surface **must not** contain Recent or Workspace conversations.
3. The agent in the user's own conversations (the session steward) **must not**
   reach bots' or channels' conversations.

### 8.1 Two layers, two mechanisms

Conversations are enumerated through two independent paths, and they live in two
different databases:

| Layer | Enumerator | Location | Mechanism |
|---|---|---|---|
| **Thread layer** | the desktop's workspace / recent / archived views, merged remote rows, the mobile catalogue | `threads` in `app.db` | `threads.scope` filtering (§8.2) |
| **Session layer** | `future session list`, `#` session mentions, session search | the agent's own database | **bots run in a separate agent instance**, so the user-side instance does not hold these sessions at all (§8.3) |

The thread layer must be filtered: the UI lists read `threads`, and
`host/src/store/threads.rs::list_threads` today filters only
`status != 'deleted'` and returns everything. The session layer is not filtered
but **not co-located** — the only structural answer to requirement 3.

### 8.2 Thread layer

- `threads.scope` (`user` | `bot` | `channel`) is the single source of truth.
  `list_threads` and its siblings gain `scope = 'user'`; the bots surface queries
  `scope <> 'user'`. One place satisfies requirements 1 and 2.
- `#` session mentions are **disabled entirely** on the bot and channel surfaces
  (the landing point for requirement 2; bots address each other with `@`, so the
  semantics do not collide). The `@` menu in a channel lists bots only and never includes
  the user's conversations.
- `scope` is not a derived field: every thread recorded in `bots.dm_thread_id` or
  `channel_bindings` must carry the matching `scope`, backed by the consistency
  sweep in §3.3.

### 8.3 Session layer: the bot's own agent instance

Bot sessions run in a **second agent instance** whose state root is
`<home>/bots/agent-home/`. This is not a new mechanism: the documentation of
`FUTURE_HOME` describes exactly this — "replaces the whole root, which is how a
second, fully isolated Agent instance runs beside the default one: its own
singleton lock, database, sessions, logs and local IPC endpoint"
(`agent/src/utils/mod.rs::future_home`).

```
~/.future/bots/
  bots.db                       # definitions, membership, bindings, transcript ledger (§3.2)
  agent-home/                   # FUTURE_HOME: the bot agent instance
    agent/agent.db              # bot sessions and run events
    agent/sessions/             # bot session files
    agent/settings.json         # this instance's own settings
    run/agent.sock              # this instance's own IPC endpoint
  workspaces/<slug>/            # default workspace per bot
```

Three things follow: the user-side agent's session table does not contain bot
sessions (unenumerable rather than filtered); channel transcripts and bot
sessions do not slow the user's `agent.db` (this repository has seen a run
journal grow to 4.4 GiB before reclamation); and the two instances restart
independently, so a crash loop in one does not take the other down.

**Configuration sharing** (the only place this design needs an agent change):
`models.json`, `auth.json` and the skill directories all live under
`<home>/agent/`, and `FUTURE_HOME` moves them, so adding a provider, signing out
or installing a skill would have to happen twice. v1 adds a **configuration
directory override** (pointing at the user's `agent/` config) that affects reads
only and moves no data. Not a symlink (Windows needs a privilege or Developer
Mode), and not a copy (two copies drift). On skills: `~/.future/agent/skills`
moves with the home while `~/.agents/skills` already lives under the real user
home and is unaffected — the same override points both instances at the same
paths, so bots and the user see one skill set. The target shape after that is
`~/.future/config/` holding `models.json`, `auth.json` and `skills/`, shared by
construction with the override field retired; that is a task on a released layout
(migration, backup manifest, the sandbox's hard-deny paths, docs) and is
scheduled separately from the bot feature (§12 item 2). `settings.json` never
joins the shared set: each instance keeps its own (§12 item 3).

### 8.4 The shape of that instance

| Aspect | Design |
|---|---|
| Process and supervision | the existing watchdog and crash-recovery chain in `desktop/src-tauri/src/agent_supervisor.rs` takes a home parameter and runs two; the headless host likewise |
| Endpoint | the bot instance uses its own `run/agent.sock`; the executor connects to it and the UI never does (the UI knows only "bots") |
| Backup | `future data` gains `bots/bots.db` and the bot `agent-home` (today's list is `agent/agent.db`, `app/app.db`, `fusion/fusion.db`, `tasks/tasks.db`; see [data-backup.md](../guide/data-backup.md)) |
| Diagnostics | `future run / session / logs / doctor` gain a "which instance" dimension; `FUTURE_HOME=… future session list` already does this today, and the bot service wraps it instead of exposing it |
| Remote and mobile | still proxied by the host (`bots_v1`); the phone never learns there are two agents |

### 8.5 Entry points that must be verified one by one

Missing one produces "I never created this conversation". Every item below is part
of acceptance and needs a test:

1. the Workspace view and its tree (including the archived section);
2. the Recent view (including paging and the recency ordering);
3. the `#` session-mention menu;
4. conversation search;
5. unread dots and run-status polling;
6. approval and pending counts — a turn parked on approval must not raise a badge
   on a user conversation;
7. merged remote rows (including a conversation catalogue pushed from another
   desktop);
8. the mobile conversation list and its Recent;
9. the user-side `future session list` (the observable face of requirement 3: not
   "filtered out" but "not there in the first place").

### 8.6 Boundary and residual

**It is not a security boundary.** A bot and the user are the same OS user on the
same machine, and bots have full access by default (§10): a plain
`sqlite3 ~/.future/agent/agent.db` still reads the user's database. A separate
instance buys **structural unenumerability and fault isolation**, not "cannot
read". The only ways to get the latter are a sandbox policy for bots (which
conflicts with full access) or a separate OS user (out of scope). This sentence is
a design statement, not a security claim.

**Authenticated refusal is not part of this design.** Making "knowing a session id
is not enough" true requires the agent to refuse `transcript` / `run` / `delete`
for `created_by='bot'` until the caller is authenticated as the host executor, and
that depends on a trusted-CLI channel this repository records as **not designed**:
the layer-0 comment in `agent/src/sandbox/rules.rs` says the "proper fix is a
dedicated credential channel (agent injects a short-lived scoped token via env …)"
and marks it unscheduled. It is ordered after the separate instance.

---

## 9. The chief bot and the cost of creating one

### 9.1 The chief is a bot with a different job, not a privileged agent

Its power to create bots is the CLI it already has (`future bot new`,
`future channel new`). There is no second permission path and no back door.

| Item | Design |
|---|---|
| Uniqueness | `bots.is_primary`, enforced by a partial unique index: at most one per home |
| Creation | On first entry to the bots surface, a card reads "Set up your chief"; one click creates it (default workspace `<home>/bots/workspaces/chief/`, default model, default job template) or nominates an existing bot. Matches [Grok Bot's primary Bot](https://docs.x.ai/grok-bot/bots) |
| Job | Routing (it judges when `channels.router_bot_id` is null), maintaining the roster, creating bots and channels for the user, reporting progress. **Not** a general assistant |
| What it records | The roster, who is working on what, and the outcome of the last dispatch, in its own `FUTURE.md` |
| What it must not do | List, read or reference the user's conversations; answer questions raised in them |

### 9.2 Lowering the creation threshold

| Action | What it removes |
|---|---|
| One-click chief | The directory, the model and the job template all have defaults |
| One-sentence members | In the chief's private conversation: "add one for the launch post". The chief writes `AGENTS.md` from a **fixed skeleton** (job / boundaries / inputs / acceptance / when to ask whom) instead of improvising — exactly the cure for the "bot factory overfits" failure ([PRODUCT.md](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/reference/PRODUCT.md)) |
| Channels come with a default member | A new channel includes the chief, so "at least two bots" is satisfied by "chief + 1" |
| Growth from a conversation | When a conversation has clearly taken on a long-lived job, suggest "make this a bot?" |

One guardrail: **a roster that only grows is the characteristic disease of this
system.** The roster page shows who the chief is and who has not been used in
seven days, and suggests archiving rather than deleting — hide instead of delete
is what Grok Bot explicitly recommends
([Create and manage Bots](https://docs.x.ai/grok-bot/bots)). The chief asks before
creating several at once, also from its guidance.

### 9.3 Creating a bot in natural language

The chief calls `future bot new --name … --job …` in its own private
conversation; once the write succeeds the bot exists, and the desktop refreshes
on `bots-updated`. Because bot sessions are fully permitted by default (§10), the
command is not blocked by the sandbox.

### 9.4 One to three sentences: the three creation paths

The target feel: the user picks no directory, chooses no model and writes no job
template. The three paths differ only in **who produces the job**.

| Path | What the user does | What happens underneath | Cost |
|---|---|---|---|
| **A First-run card** | click "Set up your chief" | create the directory, write `AGENTS.md` from the skeleton, write the row, open the conversation | Covers only the first bot, and it is the one click worth paying |
| **B One line in the composer** | type `New bot: owns the launch post, takes Writer's tone, writes but does not review` | name derived from the job (numbered on clash) → create the directory → write `AGENTS.md` from the skeleton → write the row → open the chat | The job comes from the user's own words; nothing is inferred |
| **C Create, then grow** | create a placeholder bot and let **the first message be its job** | at the end of the first turn the executor compresses that message into `AGENTS.md` using the skeleton; the bot is greyed out until it has a job | Costs one conversation; depends on the skeleton's quality |

```
you:  You own turning each week's launch notes into a short post, in the style of the last one
it:   (writes that into its own AGENTS.md, restates three boundaries, asks one necessary question)
```

**Confirmation rule** (§12 item 8):

- The user **stated the job** ("add one that owns the launch post") → create it,
  no question.
- The chief **inferred** that a new bot is warranted ("this deserves a long-lived
  owner") → ask once before creating.

The "rosters only grow" guardrail stays, but no longer sits on the main path.

**Three guardrails**:

1. **The skeleton is slots, not prose** — the model fills fields, it does not
   write paragraphs. This is exactly the failure Grok Bot recorded: "a bot-factory
   bot writes it for new bots and tends to overfit it to the one scenario it was
   created for"
   ([PRODUCT](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/reference/PRODUCT.md)).
   The job file is an ordinary readable, editable file, which is the structural
   advantage over a server-side record.
2. **A placeholder bot with no job is greyed out**: it is not a default channel
   member and starts no routine.
3. **The default workspace is `<home>/bots/workspaces/<slug>/`**; pointing it at
   an existing project is `future bot set <id> --workspace <path>`, never a
   rebuild.

---

## 10. Permissions and sandbox

### 10.1 Default: full access

A bot session is created with `permission_level='all'` and **no `sandbox_tier`**.
Per `agent/src/rpc/session_prompt.rs`, no policy means
`ResolvedSandbox::disabled`, and `evaluate()` returns `Allow` on its first line:
the rule layers are never consulted.

Three consequences follow:

1. **The two-root problem disappears**: the bot workspace and the channel
   workspace are both directly readable and writable, with no `extra_roots` and no
   generated rule file.
2. **Layer 0 does not apply either**: the hard denies on `models.json` and
   `approval_rule.json` are not evaluated, so a bot can change its own model
   configuration and rule file. That is what "full access" means; it is written
   down here so nobody misreads it.
3. **`future bot new` and `future channel new` need no approval**, so "create a
   bot in one sentence" is frictionless.

### 10.2 The asymmetry this creates

There is no isolation at the file layer, while isolation at the conversation
layer is mandatory (§8). The two do not explain each other: what is isolated is
**conversations and context**, not files. A bot can read any file on the machine
and still cannot see or enter any of the user's conversations — the latter comes
from the separate agent instance (§8.3), not from file permissions. That is why
§8.6 states plainly that the separate instance is not a security boundary.

### 10.3 If tightening is ever wanted (not default)

| Lever | Mechanism | Cost |
|---|---|---|
| Layer 0 only | add a "layer 0 only" tier to the agent: workspace/user rules are not evaluated, but the hard denies on `models.json`, `approval_rule.json` and credential files stay | an agent change (new tier plus three-platform verification) |
| Two-root allowlist | `SandboxPolicy.extra_roots`, or a per-bot `approval_rule.json` | the former touches proto and agent; the latter is generated output whose stale paths silently fall back to approval |

Neither is in v1.

---

## 11. Failure surfaces

### 11.1 Two bots cannot see each other's workspace — do they deadlock?

No deadlock, but a silent stall is possible, and the two must not be conflated.

- A deadlock is structurally impossible: waking is triggered only by `@` or the
  judge, and is decided by the kernel; a bot's turn **cannot block waiting** on
  another bot's turn — it can only produce text. No wait edges means no deadlock.
- Three stalls are possible and each must be visible:
  1. **A writes its result into its own workspace and `@`-s B saying "see the
     result".** Under full access B can read it (§10), but "private" is then only
     a convention; if a rule ever denies it, B should answer "I need that file,
     paste it or put it in the channel workspace" rather than answer without
     evidence.
  2. **Two bots `@` each other until the budget runs out.** Termination is
     deterministic: the kernel stops at chain depth 2 or turn 8, writes a system
     message explaining why, and leaves both bots' last messages in the
     transcript. What the human sees is "they talked for four messages and
     stopped", not "it hung".
  3. **A turn ends with neither a `@` nor an answer.** The hardest to tell apart
     from a hang. Mitigation: put the turn contract in L2 ("your turn ends with
     one channel message") and write a system message whenever a turn finishes
     `done` with no text.

### 11.2 A turn parked on approval

- Symptom: the turn parks on `ask`, the channel shows "Waiting for approval", and
  the run does not end; with nobody there it stays parked until the turn times
  out and is aborted.
- Mitigation: the speaking rules make the bot judge the target before writing; a
  waiting turn in a channel is clickable and opens the approval card in the
  private conversation; the per-turn timeout aborts and writes a system message
  so one bot cannot stall the channel; and **no batch approval inside a
  channel**.

### 11.3 The `@` change and existing file references

Today, in a user conversation's editor, `@` mentions **workspace files**
(`desktop/src/features/agent/MentionEditor.tsx`; a file pill serialises to
`[name](./path)`), and `#` mentions sessions. **On the bot and channel surfaces
exactly one thing differs: `@` calls people, not files.**

| Existing behaviour | After the change | Verdict |
|---|---|---|
| File pills in stored drafts and messages | They serialise to markdown links and never go through mention parsing | Unaffected |
| `#` session mentions | unchanged in user conversations; disabled entirely on the bot and channel surfaces | Satisfies the isolation requirement (§8.2) |
| `@` in a user conversation | unchanged: still mentions workspace files | The two surfaces stay independent |
| `@` in the bot and channel editors | **bots only**: channel members → other bots | No file group in that menu (§12 item 6) |
| Giving a file to a bot | drag it in, or type its path | Files are a tool-layer concern, not a way to call someone |
| Hand-typed `@./src/main.rs` | matches no roster name, treated as plain text | No error, no wake |
| A bot writing `@Writer` as plain text | parsed by the kernel | Required: the CLI, the phone and bot-authored messages are all plain text |
| A machine and a file sharing a name (a bot called `docs`) | the roster match wins | Warn about the clash when the bot is created |

No data migration: no existing data carries the meaning "`@name` is a bot", and
`@` inside user conversations is untouched.

---

## 12. Decisions taken

Every item below is settled. It is written down so a later reader knows **why**,
and under what condition it should be reopened — not to recount how it was
reached.

| # | Decision | Reasoning | Reopen when |
|---|---|---|---|
| 1 | **Full access by default**: bot sessions carry no `sandbox_tier` and `permission_level='all'` (§10.1) | The person running the bot owns the machine; breaking a config costs a reinstall, while a policy layer that must be maintained forever costs continuously | The day bots are offered to someone other than the machine's owner: switch to "layer 0 only" (no touching model config or credentials) |
| 2 | **Point at the config first, hoist later**: v1 adds a config-directory override so the bot instance reads the user's `models.json`, `auth.json` and app-scope skills; moving all three into `~/.future/config/` is a separate follow-up task | The override is one change with the same behaviour as hoisting, while hoisting touches a released data layout and drags in migration, the backup manifest, the sandbox's hard-deny paths and the docs | Once the second instance has run stably, hoist as its own task; the two then share by construction and the override field retires |
| 3 | **`settings.json` stays per instance** | It carries the default model, the approval tier and compaction thresholds; a bot may want a cheaper model than you, and a bot editing it must not edit yours | If the two sides turn out to be identical for a long time and the upkeep becomes a burden, revisit sharing the read-only fields |
| 4 | **Default speaking policy `router`** (with a chief); `mention_only` without one; `eager` off by default (§5) | One judgement of a few hundred tokens removes the largest friction — the human doing the routing every turn — at almost no cost | The measured misjudgement rate (§15 item 6) is high enough that a user would rather name speakers |
| 5 | **`eager` is capped at 6 members and judged by a cheap model** (§5) | A group's value is a few viewpoints, not a crowd; the judge only needs to know whether a job is involved, not to reason hard | Measurement shows a cheap judge misjudges far more than a strong one *and* users turn `eager` on often |
| 6 | **`@` on the bot and channel surfaces lists bots only** (§11.3) | In a channel, `@` means "call someone"; making a person first think about what `@` points at now costs more than one missing menu group | Users need to cite files there often enough that dragging and typed paths are not enough, and the file group comes back |
| 7 | **Private-chat context is off by default inside a channel**, with a per-channel switch (§4.4) | The leak is silent: others in the group see a sentence and never learn it came from a private chat | Users complain about re-explaining background every time and accept a visible "this turn cited private context" mark |
| 8 | **The chief creates immediately when the user stated the job, and asks only when it inferred one itself** (§9.4) | A roster that only grows is this system's characteristic disease, but a confirmation on the path where the user has already decided is pure friction | Inferred creations become frequent and the question feels redundant: fall back to confirming only batch creation |
| 9 | **"Create, then grow" is allowed**: a placeholder bot with no job can exist, and the first turn generates its `AGENTS.md` from the skeleton (§9.4) | This is the step that makes "setup is a message" true: the user does not have to finish writing the job before creating | If placeholders pile up and never acquire a job, require a job before first use |

Of the nine, only the second carries real work (one config override); the rest
are parameter choices over mechanisms that already exist.

## 13. Cross-machine bots: not doing it, and what is missing

"A bot on machine A joins a channel on machine B". v1 is single-machine (the host
machine), not because it is complex but because none of the following five things
exists, and with any one missing the feature can only ship half-done:

1. **An addressable bot handle**: `future run --session <id>` resolves a session
   against the local agent; the remote protocol is a channel between paired
   desktops and has no notion of "a bot on another machine is an addressable
   object".
2. **A cross-machine turn protocol**: who holds transcript order, who decides the
   budget, and how a turn on A is written into B's transcript and deduplicated
   (it needs a `turn_id`, an idempotency key and a clock ruling). The remote
   protocol today has peer presence and request/response only.
3. **Cross-machine member authorisation**: handing a workspace path on B to a bot
   on A means letting another machine write local files; sandbox rules and
   approvals are local, and there is no layer for "this machine's user has
   authorised an agent on another machine".
4. **Cost ownership**: one turn spends two owners' credits and the attribution is
   undefined.
5. **Disconnection semantics**: when A's host drops, is that member "temporarily
   unavailable" or is the turn failed? The two answers differ for both transcript
   and budget, and neither is defined.

Conclusion: in the single-machine version, a bot whose executor is elsewhere gets
one explicit message instead of pretending it can participate.

---

## 14. Phases and acceptance

| Phase | Deliverable | Observable acceptance |
|---|---|---|
| **P1 Bot** | `bots.db` and its schema, **the bot's own agent instance and config override**, the private thread, navigation entry, bot list, one-click chief, one-line creation from the composer, `future bot` CLI | create a bot, talk three turns, restart and both name and memory are intact; its private conversation is absent from Recent and Workspace; **the user-side `future session list` does not contain it at all**; the bot can use the user's configured models and installed skills; **one line — `new bot: owns the launch post` — yields a working bot with no dialog, no naming step and no model choice**; `future data` backs up `bots/bots.db` and the bot `agent-home` |
| **P2 Channel** | `channels` / `members` / `bindings` / `turns`, `threads.scope`, routing kernel, three policies, executor, channel view, startup sweep | two bots and me: `@`-ing one gets exactly one reply; a bot `@`-ing another stops at chain depth 2; a 9-turn message is cut by the budget with a stated reason; under `router`, nobody @-ed still yields the right speaker with a visible reason; all nine entry points of §8.5 pass; deleting a bot's thread by hand degrades the bot to "conversation deleted, can be recreated" instead of erroring |
| **P3 Memory and routines** | observable memory writes, channel summary, routines bound to a bot, growth from a conversation | ask a bot to remember a preference and it holds in a new channel the next day; create a daily routine for a bot and read its run ledger; a conversation turned into a bot shows up on the roster |
| **P4 Optional tightening** | hoisting config into `~/.future/config/` (a separate task), the layer-0-only tier (only if bots are offered to others), cross-channel handoff, duplication and sharing | both instances read one config and the override field is gone; a bot cannot read `models.json`; A hands a result to B and B's answer lands where expected; a duplicated bot carries no memory |

**Not doing** (stated so it is not asked again): voice calls, browser/desktop
control, `@bot` from X, wiring a channel into an IM bridge, cross-machine bots.

---

## 15. Points that code must verify, not this document

Each is an **assumption** of this design. Paper cannot settle them; each needs a
measurement or a test.

1. **Is the prefix cache actually stable**: two adjacent turns with an unchanged
   L2 should reach the same `cache_read` magnitude; editing `FUTURE.md` should
   force one full recompute. Use the repository's existing measure
   (`cache_read / inputTokens`).
2. **Does layer 0 really not apply under a policy-free sandbox**: read
   `models.json` from a bot session and confirm it is readable — this decides
   §10.1 item 2.
3. **The nine entry points, in both directions**: verify each one, and verify
   absence rather than presence. Missing one is half the wall down.
4. **The separate instance actually runs**: the second agent gets its own lock,
   `agent.db` and socket; the user-side instance's session table does not contain
   it; the two restart independently and one crash does not affect the other.
5. **The config override works**: the bot instance sees a newly added provider and
   a freshly installed app-scope skill without a second configuration step, and a
   user sign-out takes the old credential away from the bot too.
6. **Judge accuracy**: run 20 real messages under `router` and under `eager`, and
   count "should have been called but was not" against "called but should not
   have been", to decide whether `eager` deserves to be on by default.
7. **Channel serialisation and budget truncation**: a 9-turn chain against a stub
   agent, verifying order, the stop point, and the system message.
8. **Answerability after summarising**: ask a question that depends on message 3
   once the channel is at message 40, and verify the summary plus window still
   answers correctly rather than appearing to.
9. **One bot `@`-ed by two channels at once**: the two binding sessions do not
   interfere; and measure whether the queueing semantics (`append` by default)
   ever need `--steer`.
10. **A parked approval does not stall the channel**: with one turn parked, other
    turns in the same channel proceed, and the timeout aborts and writes a system
    message.
11. **Chief creation is frictionless**: `future bot new` raises no approval inside
    the bot's own instance, and the desktop sees the new bot within a second via
    `bots-updated`.
12. **Mobile capability gate**: against an older host without `bots_v1`, the
    client says "this computer does not support it" rather than showing an empty
    list.
13. **Cross-database repair is genuinely idempotent**: kill the process between
    each of the three writes that create a bot, and verify the startup sweep
    reclaims the orphan and that a second run changes nothing.
14. **Does the generated job overfit**: create five bots with five different jobs
    in one line each and compare their `AGENTS.md` — if they collapse into the
    same boilerplate, the skeleton's slots are too soft and the template needs
    tightening rather than the user patching each file.
15. **Backup covers the new database and home**: `future data` restores
    `bots/bots.db`, the bot `agent-home` and the channel transcripts; deliberately
    omitting either must fail rather than pass silently.

---

## Appendix: relation to existing designs

| Existing capability | Relationship |
|---|---|
| Scheduled tasks ([tasks.md](tasks.md)) | routines are reused directly; a bot only gives a task an owner. `future task` runs at full permission by design, which matches a bot's own boundary and does not bypass it |
| Fusion ([fusion.md](fusion.md)) | multi-model discussion **inside one run**, not a multi-bot conversation; both share the "deterministic kernel plus host executor" shape, but fusion is read-only and single-turn |
| Goal control plane ([goal-control-plane.md](goal-control-plane.md)) | long-lived goals and human gates; a channel is several parties collaborating in one conversation and introduces no goal state |
| IM channels (the `channels/` crate) | shares only the word "channel": that bridges an external IM into the agent; this is conversations between agents and people |
| Workspaces and conversations | a bot's private conversation is an ordinary thread with `scope='bot'`, isolated in both directions from user conversations (§8) |
