# Bots and Channels design

[中文](bots.zh-CN.md)

Status: design draft, not implemented. Updated: 2026-11-04.

This document is design only: it contains no code changes, changes no actual
`sandbox` / `approval_rule` configuration, creates no bot, and touches nothing
under `~/.future/`. "Supported today" below means a mechanism that already
exists in this repository, with the code location and the command line spelled
out so it can be checked; everything that requires a change lives in its own
section and is never mixed into the "today" list.

It answers three questions:

1. How the xAI product *Grok Bot* organises bots, memory, group chats and
   context (§1, every claim linked to its source).
2. What a bot and a channel become in FutureOS, and how context is assembled
   (§3–§6).
3. Which part the current code already supports (§7), which part requires an
   agent or sandbox change (§8), how each part fails (§9), and the **five
   decisions that need a human answer** (§10).

---

## 1. What Grok Bot actually does

Only claims with a source. Grok Bot's product documentation lives on
`docs.x.ai`, carried by Cursor's account and billing system. A third-party
write-up of xAI's 72-hour launch stream (Field Notes) fills in failure modes
the official docs do not cover; it is **not** official documentation and is
labelled as such wherever it is used.

| Fact | Source |
|---|---|
| A Bot is a durable teammate with a name, a job, its own conversation and context that compounds — not a chat thread | [Create and manage Bots](https://docs.x.ai/grok-bot/bots) |
| All bots share **one cloud computer**: files are visible to every bot, browser cookies and command-line credentials are shared; each bot gets its own screen, which is a parallel work surface and **not a security boundary** | [Use the computer and apps](https://docs.x.ai/grok-bot/computer-and-apps) |
| That computer has one shared workspace at `/workspace`, where durable project files belong | same page, "Work with files" |
| Memory is **per bot**, long-lived, and the bot decides on its own when to save; a few facts about you (name, time zone) are shared by all bots | [Create and manage Bots](https://docs.x.ai/grok-bot/bots), "What a Bot remembers" |
| In a group chat a bot **can use what you told it in your private chat**; each bot keeps its own memory and it is not shared with the other bots there; everyone in the group can read what it says | same page, "Across chats, Bots, and calls" |
| A group holds 2–6 bots; `@Bot` directs a message and `@everyone` addresses the group; bots post into the group themselves and pass work among themselves | [Message and collaborate](https://docs.x.ai/grok-bot/chat-and-collaboration), "Start a group chat" / "Direct a message" |
| Bot-to-bot messaging is an **asynchronous handoff**: the receiver wakes, handles the request and may reply later; the handoff shows in the bots' own chats, not in the group | same page, "Let Bots hand work off" |
| Approval requests, secret requests and email/Slack drafts **do not appear in group chats**; a bot's handoff message to a group is text-only | same page, "Start a group chat" |
| A skill library is **one per account** and every bot sees it; there is no per-bot attach | [Skills and routines](https://docs.x.ai/grok-bot/skills-routines-and-automations), "Save a skill"; see also [`grok-bot-cli`'s `gbot skills`](https://github.com/ScriptedAlchemy/grok-bot-cli) |
| A routine belongs to **one bot**, fires on a schedule or an event, and runs in the cloud while the laptop is closed | [Skills and routines](https://docs.x.ai/grok-bot/skills-routines-and-automations), "Create a routine" |
| Failure mode one: bots in a group are "eager", "love to talk" and talk over each other, because **every bot answers every message** | [Field Notes · ANTIPATTERNS §35](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/ANTIPATTERNS.md) (third party) |
| Failure mode two: routine frequency is the biggest cost leak ("every 15 minutes is hundreds of messages a day") | same file, §34 (third party) |
| Duplicating a bot carries its profile, skills, routines and avatar but **not** chat history, learned memory or attachments; sharing a template keeps core memory and strips workspace-specific data | [Create and manage Bots](https://docs.x.ai/grok-bot/bots), "Share a Bot"; and [Field Notes · PRODUCT.md](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/reference/PRODUCT.md) (third party) |

Three consequences that decide the design:

- **Context is layered**: bot-private (memory plus its own conversation) ×
  shared (files and logins on the computer) × situational (the group transcript
  everyone can read).
- **Waking is explicit, but the default product behaviour was not**: `@`
  directs a message, yet "every bot answers every message" is the failure the
  team hit live and wrote down.
- **Isolation is at the memory and conversation layer, not the filesystem**:
  all bots share one `/workspace`.

---

## 2. Three divergences from the existing idea

| # | The idea | What Grok Bot does | What this design does | Cost |
|---|---|---|---|---|
| D1 | Each bot has its own workspace | No. One shared computer and one `/workspace`; isolation lives in memory and conversations | **Follow the idea**: the bot's workspace is where its identity and memory live | Handoffs between bots must be explicit; files and memory are not automatically visible |
| D2 | A bot joins only when `@`-ed | An expectation expressed in prompts; in practice bots talk over each other | **Make it a deterministic routing rule**, not a model judgement | Needs budgets and stop conditions, or it silently stops responding (§9.1) |
| D3 | Bot workspace plus channel workspace, two roots | Single root; the problem does not exist | Keep both roots, but pay once for "writable outside cwd" | See §8: today that lands on approval, so option B or A |

D1 brings a bonus, and it is the cheapest part of this design: **a bot's
identity and memory reuse the existing cwd-scoped context injection**, so no
new memory subsystem is needed (§4.1, §7.1).

---

## 3. Concepts and data model

### 3.1 Concepts

| Concept | Definition |
|---|---|
| **Bot** | A workspace (`AGENTS.md` states the job, `FUTURE.md` is the memory index, `.future/memory/*.md` holds the details), a model and thinking level, a name and avatar, and **one private conversation** (an ordinary thread) |
| **Channel** | Members (≥ 2 bots, plus people), a **channel workspace** (the shared board), a transcript ledger, and a charter |
| **Binding** | The `(bot × channel)` session: that bot's context container inside that channel, created lazily |
| **Turn** | One bot utterance dispatched by the routing kernel: a real agent run whose final text becomes a transcript entry |

A bot being exactly one workspace is a deliberate trade: with `cwd` set to the
bot workspace, the existing prompt builder already injects `AGENTS.md` (the
job) and `FUTURE.md` (the memory index). See §7.1.

### 3.2 Tables

These live in `app.db` (the desktop GUI store), **not** in a separate database
the way `tasks.db` does. The reason is filtering and joining: bots and channels
must be queried alongside `workspaces` and `threads` — "the bot's private
conversation must not appear in Recent" is not expressible across databases.
`host/src/store/schema.rs` is the single source of truth for a fresh database,
and a released database additionally needs a versioned migration in
`host/src/store/migrations.rs` (that file's own doc comment states the rule).

```sql
bots(
  id TEXT PRIMARY KEY, name TEXT NOT NULL, job TEXT NOT NULL,
  avatar_shape TEXT, avatar_color TEXT,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id),
  model_id TEXT, thinking_level TEXT, permission_level TEXT,
  dm_thread_id TEXT REFERENCES threads(id),
  pinned INTEGER NOT NULL DEFAULT 0, hidden INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER
);

channels(
  id TEXT PRIMARY KEY, name TEXT NOT NULL,
  charter TEXT NOT NULL DEFAULT '',          -- what this channel is for
  workspace_id TEXT NOT NULL REFERENCES workspaces(id),
  summary TEXT NOT NULL DEFAULT '',          -- rolling summary (§4.4)
  allow_private_context INTEGER NOT NULL DEFAULT 0,
  max_turns INTEGER NOT NULL DEFAULT 8,      -- bot turns per user message
  max_depth INTEGER NOT NULL DEFAULT 2,      -- @ chain depth
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, archived_at INTEGER, deleted_at INTEGER
);

channel_members(channel_id TEXT NOT NULL, bot_id TEXT NOT NULL,
  role_note TEXT NOT NULL DEFAULT '', joined_at INTEGER NOT NULL,
  PRIMARY KEY (channel_id, bot_id));

channel_bindings(channel_id TEXT NOT NULL, bot_id TEXT NOT NULL,
  thread_id TEXT, session_id TEXT, persona_hash TEXT,
  PRIMARY KEY (channel_id, bot_id));

channel_turns(
  id TEXT PRIMARY KEY, channel_id TEXT NOT NULL, seq INTEGER NOT NULL,
  author_kind TEXT NOT NULL,                 -- user | bot | system
  bot_id TEXT, text TEXT NOT NULL,
  run_id TEXT, session_id TEXT,
  status TEXT NOT NULL,                      -- queued | running | done | failed | cancelled
  mentioned_json TEXT NOT NULL DEFAULT '[]',
  error_message TEXT, created_at INTEGER NOT NULL, finished_at INTEGER);
CREATE UNIQUE INDEX channel_turns_seq ON channel_turns(channel_id, seq);
```

Reuse rather than new tables: `workspaces` (one row per bot and one per channel,
each `kind='user'`, so they show in the sidebar and the user can open the
folder), and `threads` (a bot's private conversation is an ordinary thread with
`mode='workspace'` and `workspace_id` set to the bot workspace).

**Invariant**: the session behind `channel_bindings.session_id` carries
`created_by='bot'` and `source_meta={botId, channelId}`. **Every entry point that
lists conversations** (Recent, `#` session mentions, session search, the mobile
conversation catalogue) must exclude them by default, or channel-internal
sessions leak into the user's view. This repository has already seen that class
of defect, so it must be verified at every entry point rather than filtered in
one list component.

### 3.3 CLI surface

Consistent with the other control planes: read the same store, add no backend.

```
future bot list|show|new|rename|delete        (v1: read-only plus create/rename)
future channel list|show|new|add|remove|post  (post = send one message as the human)
```

---

## 4. Context assembly

### 4.1 Three layers

| Layer | Contents | Lifetime | Injection |
|---|---|---|---|
| **L1 bot-private** | The job (`AGENTS.md`), the memory index (`FUTURE.md`, linted to 30 entries / 100 lines / 12 KB), detail memory read on demand, the private conversation's history | Permanent | `cwd` injects the first two; the private history lives in the bot's own session |
| **L2 channel-shared** | Charter, member roster (name plus one-line job), the full bot roster, the channel workspace path, the channel summary, speaking rules | Channel lifetime; refreshed only when a file hash changes | Session-level `append_system_prompt` |
| **L3 turn envelope** | Who `@`-ed you, that message, the transcript window, what this turn asks | One turn | Prefix of the user message |

### 4.2 The assembled shape

System prompt (one per `(bot × channel)` binding):

```
<existing identity / tools / approval / communication sections — unchanged>

# Project Context            ← AGENTS.md from cwd (the bot's job)
# Workspace Memory           ← FUTURE.md from cwd (the bot's memory index)
# Available Skills           ← the global skill library (shared, as in Grok Bot)

<append_system_prompt — the channel layer this design adds>

# Channel: Launch
Charter: <channels.charter>
Members (only speak when @-ed):
- @Writer (writes the launch post)
- @Reviewer (lists blocking issues only)
- @you (Research)
Other bots on this account (not in this channel; use only to route):
- @Support (customer issues)
Channel workspace: <channel-workspace> (shared by everyone)
Channel summary: <channels.summary>

# Speaking rules
- Your turn ends with one channel message; lead with the result, do not restate
  what others already said
- To involve someone, @ them and say what you need; not @-ing means they stay out
- Write files only in the channel workspace; your own workspace is private, keep
  private material out of the channel
- If you need a file someone else holds, give or ask for an absolute path; do not
  assume you can read their workspace
- For an action that needs human approval, say only "Waiting for approval" in the
  channel and keep the details in private
```

User message (per turn):

```
[channel, most recent N messages]
Research · 10:02 · <text, truncated to 800 characters per message>
Writer   · 10:04 · <text>
[now] @Research said: <text>
[you were @-ed] Answer the message above; if nobody else is needed, @ no one.
```

### 4.3 Placement rules (backed by code, not taste)

Three constraints that must hold:

1. **The append segment comes after project context and workspace memory**, so
   it can override the bot's own `AGENTS.md`. Therefore the channel layer must
   be **purely additive**: it never restates or rewrites the bot's job. The
   verified order is in `agent/src/prompt/mod.rs::build_prompt` (section-order
   comment) and `agent/src/rpc/session_prompt.rs::build_system_prompt`, which
   reads the append segment after project context and memory.
2. **Stable prefix first**: L1 and L2 go into the system prompt, L3 into the
   user message. L2 is refreshed only when `AGENTS.md` / `FUTURE.md` / the
   charter / the member list changes (by content hash); refreshing every turn
   would blow the prefix cache. This repository has measured it: substituting the
   system prompt, dropping the tool definitions, or adding one line each gave
   **0%** cache on a primed prefix while an identical-shape request gave 93.7%,
   so the whole conversation is billed again
   ([compaction design](../internals/compaction/compaction.md), the cache and
   prefix section).
3. **Never inject identity twice**: L1 already comes from `cwd`; L2 must not
   copy it, or the job appears twice in one prompt and the model follows the
   later copy.

### 4.4 Budgets and truncation

| Item | Default | Note |
|---|---|---|
| Transcript window | 3k tokens, adapted to the model's window | Trim whole oldest messages, never half a message |
| One message | 800 characters, head…tail | Same shape as `task_runs.result_text` |
| Rolling summary | `channels.summary`, ≤ 1200 characters | Represents trimmed messages; written by a separate summary run the kernel starts |
| Summary trigger | > 2000 characters or > 8 trimmed messages | Stops summarising every turn |
| Private context | **Not** included by default | Channel flag `allow_private_context`, default 0 (§10 item 5) |

### 4.5 Fusion rules (what goes in, what stays out)

- **A bot writes only its own memory**: it writes `FUTURE.md` and
  `.future/memory/` inside its own workspace with the ordinary tools. Context
  crosses bots only as spoken text or as files in the channel workspace, which
  matches Grok Bot's "nothing is shared between bots unless one messages another"
  ([PRODUCT.md](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/reference/PRODUCT.md)).
- **The summary is not memory**: `channels.summary` is a channel fact injected
  only into turns taken **in that channel**; it is never written into a bot's
  `FUTURE.md`.
- **Process stays out of the transcript**: a turn contributes its final text plus
  one activity line; the full tool process stays in the binding session behind a
  disclosure. Grok Bot shows tool activity, computer use and created files
  alongside messages; we trade that for a transcript that is not drowned in
  process noise, and pay for it with a single status line.
- **Failures do enter the transcript**: a `failed` or `cancelled` turn writes a
  system message saying who failed, why, and how to retry. Grok Bot's group chat
  has no such slot, and the observed result in the field notes is "it looks like
  nobody is answering you".

---

## 5. The routing kernel

Lives in `orchestration/bots`, shaped like goal / tasks / fusion: a deterministic
kernel of pure functions over an SQLite store, with the executor on the host.

```rust
// Pure functions, no IO, unit-testable
mentions(text: &str, roster: &[BotRef]) -> Vec<BotId>
enqueue(turn: &Turn, mentioned: Vec<BotId>) -> Vec<QueuedTurn>
should_stop(queue: &[QueuedTurn], budget: &Budget, seen: &[TurnRef]) -> Option<StopReason>
```

`mentions` rules, precise enough to test:

1. A mention is `@` plus a **whole-name** match against the roster, longest name
   first; the name must be followed by whitespace, line end, or punctuation.
2. `@` must be preceded by whitespace or line start (so `mail@host` and
   `src/@scope` are not mentions).
3. Fenced code blocks and `>` quote lines are not parsed.
4. Names match case-insensitively but resolve to the canonical name.
5. The author is excluded; a repeated mention counts once, at its first position.
6. `@` plus anything that is not a roster name is plain text: no error, no wake.

Stopping and budgets:

| Rule | Default | Why |
|---|---|---|
| Bot turns per user message | 8 | Directly answers the "every bot answers every message" failure in [ANTIPATTERNS §35](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/ANTIPATTERNS.md) |
| `@` chain depth | 2 | The documented conclusion is that "one bot tagging two others once" is usually enough (same file) |
| A bot may not speak twice in a row | Hard rule | Prevents self-loops and two bots burning the budget on each other |
| A turn that `@`-s nobody | Ends the chain | Silence terminates |
| `@` a non-member | No wake; a system note "X is not a member of this channel" | No implicit joining |
| `@all` | Allowed, counted against the budget, shown in red | Keep the capability, make the cost visible |

Why this is code and not a prompt: prompt-level "who should speak" is exactly
what failed in Grok Bot (§1, failure mode one). "Who was `@`-ed" is a string
question with an exact answer; leaving it to the model makes both cost and
ordering unpredictable.

---

## 6. The executor

`host/src/bots/executor.rs`, shaped like the tick loop in `host/src/tasks.rs`
(one service shared by the GUI and headless hosts; the Tauri side is a thin
adapter, as in `desktop/src-tauri/src/commands/tasks.rs`).

- **Serial within a channel** to keep transcript order and stop bots talking
  over each other. **At most two channels in parallel**, matching fusion's
  concurrency cap.
- **One turn is one real run**: `future run --session <binding-session> --model
  <bot.model> --thinking <bot.thinking> "<envelope>"`.
- **Turn end**: the run's final text is written to `channel_turns` with
  `status='done'`, its `@` mentions are parsed to extend the queue, and a turn
  that mentions nobody ends the chain.
- **Timeout**: on the order of fusion's 600 seconds; abort first, then write a
  system message.
- **Approvals**: a bot's action can park on one. The channel shows a single
  "Waiting for approval" line; the approval card stays in the bot's private
  conversation and the global pending list, matching Grok Bot's "approvals do
  not appear in a group". That means **a channel stops when nobody is there** —
  intended, but it must be visible (§9.2).
- **Cancellation**: deleting a channel or closing a bot stops first, then
  deletes, following the goal plane's "delete must stop first" lesson.

---

## 7. What today's code already supports (no agent change)

This section lists what already works, with the mechanism and the evidence. "It
works" means the mechanism exists — not that a usable product exists. The UI, the
router and the ledger are all unbuilt, and that is application work.

### 7.1 Identity and memory: injection from `cwd`

`agent/src/rpc/session_prompt.rs::build_system_prompt` re-reads from `cwd` on
every run: `AGENTS.md` (falling back to `CLAUDE.md`, then `GEMINI.md`, see
`agent/src/prompt/project_context.rs::load_project_context`) and `FUTURE.md`
(linted and injected by the Workspace Memory section of
`agent/src/prompt/mod.rs`). Therefore:

- the bot's job is the `AGENTS.md` in its workspace root;
- the bot's memory is `FUTURE.md` plus `.future/memory/*.md` in the same
  directory;
- writing memory back is the existing `write`/`edit` tool following the memory
  discipline already printed in its prompt (when to record, the caps, stale-entry
  removal).

**Zero changes**, and identical to the memory behaviour the user already gets in
any other conversation.

### 7.2 The channel layer: the session's append prompt

`future session new --cwd <bot-workspace> --name "Research" --append-system-prompt "<channel layer>" --json`,
then
`future session set <sid> --append-system-prompt "<new channel layer>"`
to refresh it. The segment is persisted with the session, re-read on every run,
and placed after project context and memory (§4.3).

### 7.3 Running a turn

`future run --session <sid> --model <id> --thinking <level> "<envelope>"`. Long
text goes through `--message-file <path>` (which also avoids PowerShell's
encoding trap). To not wait:
`future run --no-wait --session <sid> ...` and later `future session wait <sid>`.

### 7.4 Private conversation and routines

- A bot's private conversation is an ordinary thread plus a session; the existing
  conversation view works as is.
- "A bot's routine" is the existing `future task`:
  `future task add --name "Daily risk list" --prompt "<skill name + boundary>" --cwd <bot-workspace> --session existing --daily --time 09:00`.
  `tasks.cwd` and `session_policy=existing` already cover the three conditions —
  the bot's identity, the bot's workspace, the bot's session. Filtering by bot
  later only needs `tasks.owner_bot_id`.

### 7.5 What today's code does **not** cover (all application-side)

| Missing piece | Owner |
|---|---|
| `@` routing kernel and budgets | new crate `orchestration/bots` |
| Transcript ledger and executor | `host/src/bots/*` plus an `app.db` migration |
| Merged rendering across sessions (a channel is the output of N binding sessions) | new components; the existing projector is **single-session** (`packages/thread-projection/src/projection.ts`) |
| Navigation entry and the left list switching | `desktop/src/components/layout/ActivityRail.tsx` (the `ActivitySection` union is near line 61) |
| Access from the phone and another computer | a new capability in the remote protocol (the current list is in `packages/remote-protocol/src/handshake.rs`) |

---

## 8. P4: what the two-root permission needs

The requirement: while participating, a bot must read and write its own
workspace (memory, drafts) and the channel workspace (shared board,
deliverables). Today, a write outside the cwd falls through to approval
(`agent/src/sandbox/rules.rs` documents the layers: reads allow by default,
writes allow inside the workspace or temp, otherwise `ask`).

Two options:

| | Option B: generated per-bot rule file | Option A: `SandboxPolicy.extra_roots` |
|---|---|---|
| What changes | Nothing in the agent. On membership change the kernel writes `{path: <channel-workspace>, access: "both", action: "allow"}` into `<bot-workspace>/.future/approval_rule.json` (the rule file shape is `rules.rs::RawRule`) | `repeated Root extra_roots {path, access}` on `SandboxPolicy` in `packages/rpc/proto/future.proto`; `agent/src/sandbox` merges it into that session's runtime rule layer; `REGENERATE_PROTO=1` regenerates |
| Scope | Per bot: A's rule lives in A's workspace, so B gains nothing | Per `(bot × channel)` session: the most precise scope |
| Cost | The rule file is generated output that collides with hand edits, so it needs a `managed` marker and idempotent rewriting; copying or sharing a workspace carries stale paths | Agent plus protocol change, verified across Windows/macOS/Linux rule layers (seatbelt and the Windows path) |
| Risk | The file is readable by the bot and leaks "which channels am I in"; a stale path silently falls back to `ask`, so a channel suddenly starts asking for approval | Backward compatibility: an older agent that does not know the field must **degrade to ask**, never ignore it — ignoring it is a privilege escalation |

**Recommendation: B first, A as the real answer.** B keeps P1–P3 entirely free
of agent risk, and its per-bot scope cannot leak into another bot. A is cleaner,
but it introduces a security-relevant field whose cross-version degradation
semantics deserve their own PR and a three-platform sandbox regression.

One assumption that can be checked **today**: reads outside the cwd are allowed
by default, which means "B can read A's files if it has the path". That conflicts
with the intent that bots do not share memory, so it must be decided explicitly —
either the charter forbids cross-bot reads, or the bot workspace gets a `deny`
rule. This design's position is that **read-by-default is today's behaviour, not
a promise**, and therefore it must be verified in code (§13 item 3).

---

## 9. Failure surfaces

The three questions a review will ask, each with the symptom, the diagnosis and
the mitigation.

### 9.1 Two bots cannot see each other's workspace — do they deadlock?

**No deadlock, but a silent stall is possible, and the two must not be
conflated.**

- A deadlock is structurally impossible: waking is triggered only by `@` and is
  decided by the kernel; a bot's turn **cannot block waiting** on another bot's
  turn — it can only produce text. No wait edges means no deadlock. Grok Bot has
  the same structure (asynchronous handoff).
- Three stalls are possible and each must be visible:
  1. **A writes its result into its own workspace and `@`-s B saying "see the
     result".** B's turn fails to `read` that path (or is denied by the §8 rule),
     so B either answers "I need that file, paste it or put it in the channel
     workspace" or answers without evidence. **Mitigation**: the L2 speaking
     rules require an absolute path and confirming the reader can reach it, and
     the transcript keeps B's failed turn rather than swallowing it.
  2. **Two bots `@` each other until the budget runs out.** Termination is
     deterministic: the kernel stops at chain depth 2 or turn 8, writes a system
     message explaining why, and leaves both bots' last messages in the
     transcript. What the human sees is "they talked for four messages and
     stopped", not "it hung".
  3. **A turn ends with neither a `@` nor an answer.** This is the hardest to
     tell apart from a hang. **Mitigation**: put the turn contract in L2 ("your
     turn ends with one channel message") and have the kernel write a system
     message whenever a turn finishes `done` with no text.

### 9.2 What happens when a write to the channel workspace is refused?

- **Symptom**: the turn parks on approval (`ask`), the channel shows "Research is
  waiting for approval", and the run does not end. With nobody there it stays
  parked until the turn times out and is aborted.
- **Why it stops rather than fails**: `ask` is the approval layer, not a denial.
  This matches Grok Bot's decision to keep approvals out of the group, but our
  representation is heavier: Grok Bot moves the draft card to the private chat,
  while we only get one status line.
- **Mitigation**: (1) the speaking rules require writing only inside the channel
  workspace; (2) the channel makes a waiting turn clickable and opens the
  approval card in the bot's private conversation; (3) a per-turn timeout (on the
  order of 600 seconds) aborts and writes a system message, so one bot cannot
  stall the channel; (4) **no batch approval inside a channel** — every approval
  is read individually, or the red line means nothing.

### 9.3 Compatibility of the `@` change with existing file references

Today `@` in the editor mentions **workspace files**
(`desktop/src/features/agent/MentionEditor.tsx`; a file pill serialises to
`[name](./path)`), and `#` mentions sessions.

In a channel, `@` becomes bot-first. The compatibility matrix:

| Existing behaviour | After the change | Verdict |
|---|---|---|
| File pills in stored drafts and messages | They serialise to markdown links and never go through mention parsing | Unaffected |
| `#` session mentions | Untouched | Unaffected |
| `@` in the channel editor | Three groups: channel members → other bots → files | Files stay in the same menu, third |
| Hand-typed `@./src/main.rs` and similar paths | Matches no roster name, treated as plain text | No error, no wake |
| A bot writing `@Writer` as plain text | Parsed by the kernel | Required: the CLI, the phone and bot-authored messages are all plain text |
| A machine and a file sharing a name (a bot called `docs`) | The roster match wins, so `@docs` means the bot | Needs a UI warning; warn about the clash when the bot is created |

**No data migration**: no existing data carries the meaning "`@name` is a bot";
the change touches the editor menu and the kernel parser only.

---

## 10. Five decisions that need a human answer

This section can be forwarded as is. Each item gives both options, what each one
changes and costs, and the recommendation with its reason.

### 10.1 Two-root writes: B first, or A directly?

- **Option 1 (B)**: the kernel generates a per-bot `approval_rule.json`. Cost:
  generated output that needs idempotent rewriting and a `managed` marker; a
  stale path silently falls back to approval. Benefit: P1–P3 touch no agent code.
- **Option 2 (A)**: `SandboxPolicy.extra_roots`. Cost: proto, agent, and a
  three-platform sandbox regression; an older agent must degrade to `ask`.
  Benefit: scope is exactly `(bot × channel)`, and no generated file appears in
  the user's workspace.
- **Recommendation: B first, A as the P4 answer.** B leaves the first three
  phases free of agent risk; A's advantages (exact scope, no generated file)
  only pay off past roughly three bots and five channels, which is when the
  cross-platform verification cost is worth paying.

### 10.2 `@` in a channel: bots first, or a new trigger for files?

- **Option 1**: `@` matches bots first and files move to the third group of the
  same menu (§9.3).
- **Option 2**: keep `@` for files and give bots another symbol (for example
  `@@`).
- **Recommendation: option 1.** In a person's intuition `@` means "call
  someone", and files already have dragging and path entry; option 2 turns
  "call a bot" into something that must be taught, while it is the main path of
  this feature. The cost is that `@` behaves differently in different editors,
  which is why the menu must group the results rather than relying on docs.

### 10.3 Should a bot's private conversation appear in the normal list?

- **Option 1**: no; it is reachable only from the bots entry. This needs the
  §3.2 invariant enforced at **every** conversation-listing entry point.
- **Option 2**: yes, distinguished by workspace grouping.
- **Recommendation: option 1.** Grok Bot splits bots and conversations into two
  sidebar sections because they answer different questions ("who am I working
  with" versus "what am I doing"); a single list lets a bot's daily chatter push
  the user's real recent work out of view. The cost is that the filter must be
  complete — one missed entry point produces duplicate rows, which is verifiable
  (§13 item 5).

### 10.4 May a channel `@` a non-member and pull it in?

- **Option 1**: no; a system note plus an invite button.
- **Option 2**: add automatically.
- **Recommendation: option 1.** Automatic joining turns a visible fact ("who is
  in this channel") into an invisible side effect, and the member count directly
  sets the per-turn cost (§5 budgets scale with members). Making it a click
  costs one step.

### 10.5 May a bot use its private-chat context inside a channel?

- **Option 1**: off by default, per-channel switch. Grok Bot defaults to
  allowing it ([source](https://docs.x.ai/grok-bot/bots): "In a group chat, your
  Bot can use what you told it in your private chat").
- **Option 2**: on by default, matching Grok Bot.
- **Recommendation: option 1 (off by default).** Private chats often hold
  material that must not be published, and this leak is **silent**: others in
  the group see a sentence, never that it came from a private chat. What the
  feature buys (not repeating the background) is worth far less than one leak;
  when it is needed, turn it on and mark the turn as having used private
  context.

---

## 11. Cross-machine bots: not now, and what is missing

The scenario is "a bot on machine A joins a channel on machine B". This design
recommends **single-machine only** (the host machine), not because it is
"complex" but because the following five things do not exist yet, and without
any one of them the feature can only ship half-done:

1. **An addressable bot handle.** `future run --session <id>` resolves a session
   against the local agent; the remote protocol is a channel between two paired
   desktops and has no notion of "a bot on another machine is an addressable
   object".
2. **A cross-machine turn protocol.** Who holds transcript order, who decides the
   budget, and how a turn on machine A is written into B's transcript and
   deduplicated (it needs a `turn_id`, an idempotency key, and a clock ruling).
   The remote protocol today has peer presence and request/response, not
   multi-party ordering.
3. **Cross-machine member authorisation.** Handing a workspace path on machine B
   to a bot on machine A means letting another machine write local files; the
   sandbox rules and approvals are **local**, and there is no layer for "this
   machine's user has authorised an agent on another machine".
4. **Cost ownership.** One channel turn spends two machines' owners' credits and
   the billing attribution is undefined; today even "which channel this run
   belongs to" exists only in the local ledger.
5. **Disconnection semantics.** When A's host drops, is that member "temporarily
   unavailable" or is the turn failed? The two answers differ for both transcript
   and budget, and neither is defined.

Conclusion: in the single-machine version, a bot whose executor is elsewhere gets
one explicit message ("this bot runs on another computer; open it there"),
instead of pretending it can participate.

---

## 12. Phases and acceptance

| Phase | Deliverable | Observable acceptance |
|---|---|---|
| **P1 Bot** | `bots` table, private thread, navigation entry, bot list, creation (existing workspace or a new `<home>/bots/<id>/`) | Create a bot, talk three turns, restart and both the name and memory are intact; its private conversation is absent from Recent; `future bot list` shows it |
| **P2 Channel** | `channels`/`members`/`bindings`/`turns`, routing kernel, executor, channel view | Two bots and me: `@`-ing one gets exactly one reply; a bot `@`-ing another stops at chain depth 2; a 9-turn message is cut by the budget with a stated reason; closing the view keeps the transcript |
| **P3 Memory and routines** | Observable memory writes, channel summary, tasks bound to a bot | Ask a bot to remember a preference and see it hold in a new channel the next day; create a daily routine for a bot and read its run ledger |
| **P4 Isolation and handoff** | `extra_roots` (option A), cross-channel handoff, bot-to-bot private channel, duplicate/share | Two bots cannot read each other's manifest; A hands a result to B and B's answer lands where expected; a duplicated bot carries no memory |

**Not doing** (stated so reviews stop asking): voice calls, browser/desktop
control, `@bot` from X, wiring a channel into an IM bridge (Feishu/DingTalk),
cross-machine bots.

---

## 13. Points that code must verify, not this document

Each of these is an **assumption** of this design. They are listed because paper
cannot settle them; each needs a measurement or a test. A follow-up verification
session should work through them and write the results back into this file.

1. **Is the prefix cache actually stable?** Two adjacent turns with an unchanged
   L2 should reach the same `cache_read` magnitude, and editing `FUTURE.md`
   should force one full recompute. Use the repository's existing measure
   (`cache_read / inputTokens`).
2. **Does option B's rule file actually take effect?** With
   `{access: "both", action: "allow"}` pointing at the channel workspace, writes
   stop asking; and verify it does **not** give another bot on the same machine
   the same write access.
3. **Is read-outside-cwd allowed the same way under all three tiers?** In
   particular, under macOS seatbelt `tier=sandbox`, is reading another workspace
   still allowed? This decides the note at the end of §8.
4. **Channel serialisation plus budget truncation**: run a 9-turn chain against a
   stub agent and verify the order, the stop point, and the system message.
5. **Bot × channel sessions do not leak**: verify at every entry point (Recent,
   `#` mentions, session search, the mobile conversation catalogue,
   `future session list`), not only in the list component.
6. **Answerability after summarising**: ask a question that depends on message 3
   once the channel is at message 40, and verify the summary plus window still
   answers correctly rather than merely appearing to.
7. **One bot `@`-ed by two channels at once**: the two binding sessions do not
   interfere; and confirm by measurement whether the run queueing semantics
   (`append` by default) ever need `--steer`.
8. **A parked approval does not stall the channel**: with one turn parked on
   approval, other turns in the same channel still proceed, and the timeout
   aborts and writes a system message.
9. **Mobile capability gate**: against an older host without `bots_v1`, the
   client shows "this computer does not support it" rather than an empty list.

---

## Appendix: relation to existing designs

| Existing capability | Relationship |
|---|---|
| Scheduled tasks ([tasks.md](tasks.md)) | Routines are reused directly; a bot only gives a task an owner |
| Fusion ([fusion.md](fusion.md)) | Multi-model discussion **inside one run**, not a multi-bot conversation; both share the "deterministic kernel plus host executor" shape, but fusion is read-only and single-turn |
| Goal control plane ([goal-control-plane.md](goal-control-plane.md)) | Long-lived goals and human gates; a channel is several parties collaborating in one conversation and introduces no goal state |
| IM channels (the `channels/` crate) | Shares only the word "channel": that bridges an external IM into the agent, this is conversations between agents |
