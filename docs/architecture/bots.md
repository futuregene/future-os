# Bots and bot groups: the v1 minimal slice

[中文](bots.zh-CN.md)

Status: design draft. **Implementation has not started**: this document is for
review and scheduling only; the work begins as a separate task. Updated:
2026-11-04.

This document is design only: no code changes, no configuration changes, no bot
is created, and nothing under `~/.future/` is touched.

The object is a **bot group** (*group* below), named `group` in code and the CLI,
so it cannot be confused with the repository's existing `channels/` crate for
Feishu and DingTalk IM bridges.

---

## 1. Why a minimal slice first

The things to validate are not architectural. They are three product facts:

1. Will a person really organise work by **role** ("launch post", "review")
   rather than continuing to open a conversation per task?
2. Does one bot `@`-ing another create real value, or merely the appearance of
   activity?
3. Is the user willing to pay for the extra runs and tokens each handoff costs?

So v1 contains exactly: **one bot + one group (two bots and a person) +
deterministic routing + serial execution within the group + a visible
transcript**. A separate agent instance, a separate database, hoisted config, a
routing judge, monetary metering, model-generated job skeletons and three
creation paths are all outside v1 (§14).

Acceptance is not "all these modules exist". It is the week of real usage in §13.
If the minimal combination is not used, the candidate mechanisms behind it have
no reason to exist.

---

## 2. The v1 product

### 2.1 What the user sees

- **The bots surface** replaces the ordinary conversation list and shows only
  bots and bot groups; they do not appear in Recent or Workspaces.
- **A bot has a name, a one-line job, a private conversation and memory**. The
  private conversation is for the long-term relationship and for an explicit
  "remember this".
- **A group has at least two bots, one shared workspace and one transcript**.
  The transcript holds user messages, bot replies, failures and system notes.
- **`@` inside a group calls bots only**, not files. Files arrive by drag and
  drop or by typing a path.
- **One-line creation**: typing `New bot: owns the launch post` on the bots
  surface creates it, with no directory, model or job form.

### 2.2 How a conversation flows

| Situation | v1 behaviour |
|---|---|
| The user says `@Writer write a draft` | Only Writer wakes |
| The user says "take a look at this draft" with no `@` | The **group owner** (the chief by default) takes it, answers itself, or `@`-s a better specialist |
| A bot's reply contains `@Reviewer` | Reviewer wakes and the chain depth increases by one |
| A bot's reply contains no `@` | The chain ends |
| A message `@`-s a non-member | No wake; a system note goes into the transcript |
| The group is switched to "mentions only" | An unaddressed user message starts no bot run and leaves a system note |

The user does not have to decide whom to call for every message, while **bots
still only hand off to each other through `@`**, so the room does not turn into
everyone talking at once. v1 has neither "every member judges whether to speak"
nor a separate routing judge.

### 2.3 Group conversations are not copied

The group transcript is the only source of truth. Every turn reads the messages
it needs from that ledger; **no separate conversation is kept for each
`bot × group` pair**. A bot's long-term context lives only in its own private
conversation.

---

## 3. Concepts and data model

### 3.1 Concepts

| Concept | Definition |
|---|---|
| **Bot** | A role profile: name, one-line job, model, default workspace, permissions and a private thread. The default workspace holds `AGENTS.md`, `FUTURE.md` and `.future/memory/` |
| **Group** | At least two bots, an owner, a shared workspace, speaking rules and a transcript |
| **Owner** | The bot named by `owner_bot_id`; it handles unaddressed user messages. New groups default to the chief |
| **Turn** | One real run. A group turn uses an **ephemeral, non-persisted session**; its final text, wake reason, run id and token usage go into the ledger |

`AGENTS.md` is formed from the user's own words, not expanded by a model.
`FUTURE.md` changes only when the user or the bot explicitly performs "remember
this".

### 3.2 All tables live in the existing `app.db`

v1 adds no database. Definitions, membership and the ledger share a database
with `workspaces` and `threads`, so isolation filters and foreign keys live in
one place. A released database needs a versioned migration in
`host/src/store/migrations.rs`; `host/src/store/schema.rs` stays the source of
truth for fresh databases.

```sql
bots(
  id TEXT PRIMARY KEY, name TEXT NOT NULL, job TEXT NOT NULL,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id),
  model_id TEXT, thinking_level TEXT,
  permission_level TEXT NOT NULL DEFAULT 'all',  -- private chat: full access
  dm_thread_id TEXT REFERENCES threads(id),
  is_primary INTEGER NOT NULL DEFAULT 0,         -- at most one chief per home
  hidden INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER
);
CREATE UNIQUE INDEX bots_one_primary ON bots(is_primary) WHERE is_primary = 1;

bot_groups(
  id TEXT PRIMARY KEY, name TEXT NOT NULL,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id),
  owner_bot_id TEXT NOT NULL REFERENCES bots(id),
  speak_policy TEXT NOT NULL DEFAULT 'owner',     -- owner | mention_only
  permission_level TEXT NOT NULL DEFAULT 'workspace',
  max_turns INTEGER NOT NULL DEFAULT 8,
  max_depth INTEGER NOT NULL DEFAULT 2,
  transcript_window_tokens INTEGER NOT NULL DEFAULT 3000,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
  archived_at INTEGER, deleted_at INTEGER
);

group_members(
  group_id TEXT NOT NULL REFERENCES bot_groups(id),
  bot_id TEXT NOT NULL REFERENCES bots(id),
  role_note TEXT NOT NULL DEFAULT '', joined_at INTEGER NOT NULL,
  PRIMARY KEY (group_id, bot_id)
);

group_turns(
  id TEXT PRIMARY KEY, group_id TEXT NOT NULL REFERENCES bot_groups(id),
  seq INTEGER NOT NULL,
  author_kind TEXT NOT NULL,                 -- user | bot | system
  bot_id TEXT, text TEXT NOT NULL,
  run_id TEXT,                                -- one-shot run; no durable binding session
  status TEXT NOT NULL,                      -- queued | running | done | failed | cancelled | waiting_approval
  mentioned_json TEXT NOT NULL DEFAULT '[]',
  wake_reason TEXT,                          -- user_mention | owner | bot_mention | manual
  input_tokens INTEGER, output_tokens INTEGER, cache_read_tokens INTEGER,
  error_message TEXT, created_at INTEGER NOT NULL, finished_at INTEGER
);
CREATE UNIQUE INDEX group_turns_seq ON group_turns(group_id, seq);
```

The only change to an existing table:

```sql
ALTER TABLE threads ADD COLUMN scope TEXT NOT NULL DEFAULT 'user';  -- user | bot | group
```

A bot's private conversation is an ordinary thread with `scope='bot'`; group runs
create no thread.

### 3.3 Consistency across the two stores

v1 has only `app.db` and the shared `agent.db`, not a third store. Write order is
chosen for recoverability:

| Operation | Order | Repair after a crash |
|---|---|---|
| Create a bot | ① create the private session in the shared agent (`created_by='bot'`) → ② in **one `app.db` transaction**, write the workspace, the thread (`scope='bot'`) and the bots row | If only ① finished, the startup sweep finds the `created_by='bot'` session with no bots row and deletes it |
| Create a group | one `app.db` transaction writes the workspace, `bot_groups` and `group_members` | the whole transaction rolls back; no half-created group |
| Delete a bot or group | ① stop related turns and clear the queue → ② soft-delete the `app.db` rows → ③ delete the agent's private session | after ②, the sweep removes orphaned bot sessions and threads; ③ is retried idempotently |
| Group turn | one-shot run → write `group_turns` | no session is persisted, so there is no session orphan; only the ledger row needs a terminal state |

Rules: **a dangling reference degrades, never errors**. The startup sweep is
idempotent and only reclaims and finishes deletes; it does not infer intent or
recreate conversations.

The default workspace is `~/.future/bots/workspaces/<slug>/`, never under
`~/.future/workspaces/chat/`, which is reclaimed. A bot may also point at an
existing user workspace.

---

## 4. Context assembly

### 4.1 The bot's private conversation

The private conversation is an ordinary persistent session in the shared agent,
with `cwd` set to the bot's default workspace. The existing prompt builder reads
on every run:

- `AGENTS.md`: the role;
- `FUTURE.md`: the memory index (still capped at 30 entries / 100 lines / 12 KB);
- `.future/memory/*.md`: details read on demand.

Evidence: `agent/src/rpc/session_prompt.rs::build_system_prompt` and
`agent/src/prompt/project_context.rs::load_project_context`. Models, credentials
and skills live in the same agent instance, so v1 needs no configuration
override.

### 4.2 A group turn: assembled once, with no session history

Each turn creates a fresh ephemeral session. `cwd` is still the bot workspace,
and the system prompt has three layers:

```
<the existing identity, tools and memory layers — injected from cwd>

<append_system_prompt: the group layer>
# Group: Launch
Owner: @Chief
Members: @Writer (writes the launch post), @Reviewer (blocking issues only), @you (Research)
Group workspace: <absolute path> (shared; deliverables go here)
# Rules
- Your turn ends with one group message; do not restate what others said
- To involve someone, @ them and say what you need; not @-ing means they stay out
- Never list, open or reference the user's conversations
- Keep private material out of the group workspace
```

The user message carries only what this turn needs:

```
[the most recent N group messages, at most 3000 tokens, trimmed whole]
[now] @Research said: <text>
[you were @-ed] Answer that; if nobody else is needed, @ no one.
```

- The group layer is refreshed only when the job, members, workspace or rules
  change; the transcript window always stays in the user message, so the system
  prompt is not rewritten every turn.
- A turn may carry roughly the latest 20 messages, capped at 3000 tokens, with
  whole oldest messages trimmed first. v1 has no rolling summary.
- Private-chat context does not enter a group by default; the group workspace
  and the transcript are the two shared channels between bots.
- Memory is written back only on an explicit "remember this"; group turns do not
  automatically sediment memory.

### 4.3 The existing one-shot entry point

A prototype or manual test can use the CLI as it exists today:

```bash
future run --no-session \
  --cwd <bot-workspace> \
  --model <bot-model> --thinking <level> \
  --append-system-prompt "<group layer>" \
  --message-file <turn-envelope.md>
```

`--no-session` means the session is not persisted; `--cwd` and
`--append-system-prompt` already exist (see `future run --help`). In production
the host executor starts the same run and adds the permission parameters in §8.

---

## 5. Speaking and routing

Routing lives in a deterministic kernel, not in a model's guess about who was
addressed.

### 5.1 Wake rules

In priority order:

1. A user message mentions members with `@`: dispatch in mention order and wake
   only those bots.
2. A user message has no `@` and the policy is `owner`: dispatch only to
   `owner_bot_id`. The owner may answer itself or `@` a specialist in its reply.
3. A user message has no `@` and the policy is `mention_only`: start no run and
   write a system note explaining the current mode.
4. A bot's reply mentions a member: wake that bot and increase the chain depth;
   with no mention, end the chain.
5. A mention of a non-member: no wake, and a system note.
6. If the owner is deleted or absent from the group, the group enters "an owner
   must be chosen"; it does not silently substitute another speaker.

### 5.2 Parsing `@`

1. `@` plus the longest whole-name match in the roster; the name must be followed
   by whitespace, line end or punctuation.
2. `@` must be preceded by whitespace or line start; `mail@host` and paths do not
   count.
3. Fenced code blocks and `>` quote lines are not parsed.
4. Names match case-insensitively but resolve to the canonical name; the author
   is excluded; a repeated mention counts once.
5. `@` plus anything that is not a roster name is plain text: no error, no wake.

### 5.3 Stop conditions

| Rule | Default |
|---|---|
| Bot turns per user message | 8 |
| Bot-to-bot `@` chain depth | 2 |
| A bot may not speak twice in a row | hard rule |
| A turn that `@`-s nobody | ends the chain |
| Timeout, failure, cancellation | write a system note, then end or continue by rule |

`wake_reason` records whether the wake came from a user mention, the owner rule or
a bot handoff, so "why did it answer" is explainable.

---

## 6. Execution and transcript

The executor is `host/src/bots/executor.rs`, shaped like the tasks tick loop.

- **Serial within a group** to keep transcript order; **at most two groups in
  parallel**.
- One turn is one run; the final text goes into `group_turns`, and its `@`
  mentions decide the next speaker.
- A turn times out on the order of 600 seconds: abort first, write a system note,
  then continue or end by the queue rules.
- **Failure is visible**: failure, cancellation and an empty reply all produce a
  system note; the group must never look as though nobody is listening.
- **Approvals**: when a turn parks on approval, the group shows "Waiting for
  approval" and the card remains in the bot's private conversation and the global
  pending list. That turn pauses the group until the timeout aborts it and the
  queue continues; other groups are unaffected. There is no batch approval in a
  group.
- Deleting a bot or group stops the executor and clears the queue before data is
  deleted.

The transcript stores final text and a small amount of run metadata. The
ephemeral session's process history is not kept as a second transcript.

---

## 7. Conversation isolation

All three are hard requirements:

1. Recent and Workspaces must not contain conversations created by bots or
   groups.
2. The bots surface must not contain the user's Recent or Workspace
   conversations.
3. The agent in a user conversation (the session steward) must not reach bot
   conversations through lists, search or mentions.

### 7.1 Mechanism

- `threads.scope` is the single source of truth at the thread layer: ordinary
  lists query `scope='user'`; the bots surface queries `scope<>'user'`.
  `host/src/store/threads.rs::list_threads` currently filters only deletion
  state, so this must be enforced in the query layer, not in one component.
- A bot's private session carries `created_by='bot'` and
  `source_meta.scope='bot'`; the agent-side session list excludes it by default,
  with no switch that lists bot sessions.
- Group turns are not persisted, so they cannot appear in any session list.
- `#` session mentions are disabled entirely on the bot and group surfaces; `@`
  lists bots only.

### 7.2 Entry points to verify individually

1. the Workspace view and its archived section;
2. Recent, including paging and ordering;
3. the `#` session-mention menu;
4. conversation search;
5. unread dots and run-status polling;
6. approval and pending counts;
7. merged remote rows;
8. the mobile conversation catalogue and Recent;
9. the user-side `future session list`.

### 7.3 Boundary

This is **organisational and enumeration isolation, not permission isolation**.
In the shared agent, someone who knows a private session id can still read it
explicitly; full access in a bot's private chat also means the bot can read files
on the machine. v1 does not describe this isolation as a security boundary. If
bots are ever offered to other people, a separate instance or authenticated
refusal can be considered then; neither is promised in this slice.

---

## 8. Group-turn write permissions, using existing mechanisms

A bot's private conversation has full access by default; **a group turn defaults
to `permission_level='workspace'`**. The goal is not read-only: reads follow the
existing defaults, while writes are allowed only in **the bot workspace and the
group workspace**; other writes still go to approval.

It takes two existing client parameters and one rule file:

1. the host sets `permission_level='workspace'` in the run request;
2. it also sets the desktop's existing `sandbox_tier` (using the platform's
   current values), **because otherwise the rule layers are not enabled**;
3. the host idempotently maintains this entry in
   `<bot-workspace>/.future/approval_rule.json`:

```json
{
  "rules": [
    { "path": "<group-workspace-absolute-path>", "access": "both", "action": "allow" }
  ]
}
```

Evidence:

- `ScheduledSettingsSnapshot` in `agent/src/rpc/session_prompt.rs` contains both
  `permission_level` and `sandbox_tier`, and the sandbox resolves against
  `session_cwd`;
- the fallback in `agent/src/sandbox/rules.rs` allows writes in the workspace or
  temp and otherwise returns `ask`, with the workspace rule file as its own
  layer;
- `agent/src/sandbox/seatbelt.rs::build_profile` emits
  `(allow file-write* …)` for allow rules in addition to the base writable
  roots, and the test `session_allow_rules_emit_allow_verbs` pins that behaviour.

Therefore **no new tier and no typed roots in `SandboxPolicy` are needed**. The
CLI's `--permission workspace` covers only item 1; it exposes no `sandbox_tier`,
so a single CLI command cannot be cited as proof that the permission took
effect. The host uses RPC for group turns.

This is separate from full access in a bot's private chat: the private chat can
still use the whole machine; a group turn writes deliverables to the shared
board and stops for approval on other writes.

---

## 9. The bot surface: creation, memory, notifications and metering

### 9.1 One creation path

On the bots surface, type:

```text
New bot: owns the launch post, writes plainly, never sends externally on its own
```

The system does four things: derives an editable name and slug from the job;
creates the default workspace; writes **the user's own words** into `AGENTS.md`;
and creates and opens the private conversation. There is no model-generated job
skeleton, no directory or model form, and no second or third creation path.

A default chief may be offered on first use; later groups default to it as the
owner. The chief is only a bot whose job is coordination and intake, with no
extra permissions.

### 9.2 Memory needs an explicit entry point

The private conversation and the bot page offer **"Remember this"**: the current
fact or preference is written into the bot's `FUTURE.md` or `.future/memory/`,
and the new entry is shown immediately.

This is a design conclusion: **there is no "recently learned" list based on
automatic sedimentation**. The existing memory discipline deliberately records
less, so such a list would be empty for long periods without an explicit entry
point. Writes are visible, editable and removable, which is more trustworthy
than an automatic recap.

### 9.3 Notifications are quiet by default

Only three things notify: being `@`-ed, a failed turn and a pending approval. A
"only what needs my decision" filter keeps those three; everything else goes to
the transcript without raising a badge.

### 9.4 Show tokens, not money

Each turn writes one line: `N runs · X input / Y output tokens · Z cache-read
tokens`. Without a reliable price table there is no currency conversion, which
would look precise while being unreliable. If usage is unavailable, the line
says "unknown" rather than zero.

---

## 10. Where the code lives

v1 has neither a separate store nor a separate agent instance.

| Location | Contents |
|---|---|
| `orchestration/bots/` | crate `future-bots`: the pure routing kernel and types only; it owns no database |
| `host/src/bots/model.rs` | app.db models and views |
| `host/src/bots/store.rs` | app.db reads/writes and migration support |
| `host/src/bots/service.rs` | create, edit, delete and the startup sweep; shared by Tauri, CLI and remote |
| `host/src/bots/executor.rs` | serial group execution, turns, timeouts, permission parameters and the ledger |
| `host/src/bots/notify.rs` | `bots-updated` and notifications |
| `cli/src/commands/bot.rs`, `group.rs` | `future bot …`, `future group …`; calls the service only |
| `desktop/src-tauri/src/commands/bots.rs` | thin Tauri adapter |
| `desktop/src/features/bots/` | bots surface, group view, creation and "remember this" |
| `desktop/src/i18n/locales/{en,zh}/bots.json` | UI strings |
| `desktop/src/components/layout/ActivityRail.tsx` | navigation and list switching |
| `packages/remote-protocol/src/handshake.rs` | the `bots_v1` capability when mobile support lands |

Dependency direction: `future-bots` depends on neither host nor agent; `host`
owns the existing `app.db` write path; the CLI never assembles SQL itself.

v1 does not change `packages/rpc/proto/future.proto`: sessions, runs,
`permission_level` and `sandbox_tier` already exist. Only a later permission tier
or a separate-instance protocol would require a change.

---

## 11. Boundaries and non-goals

### 11.1 No cross-machine bots

"A bot on machine A joins a group on machine B" lacks five foundations, not one
switch:

1. an addressable cross-machine bot handle;
2. a cross-machine protocol for turn ordering, deduplication and budgets (turn
   id, idempotency key and a clock ruling);
3. a member-authorisation layer for "this machine's user allows an agent on
   another machine to write local files";
4. cost ownership across the two machines;
5. defined semantics for whether a disconnected member is "temporarily
   unavailable" or a failed turn.

None has an existing implementation, so v1 is local-only. A bot on another
machine gets an explicit unavailable message.

### 11.2 Other explicit non-goals

Voice, browser and desktop control, demonstration recording, triggering from X,
IM-bridge integration, sharing bots with a team and cloud long-running work. If
the host is not running, a local bot does not continue either.

---

## 12. Implementation order

| Phase | Contents | Observable acceptance |
|---|---|---|
| **M1 Bot** | app.db migration, private bot chat, navigation and list isolation, one-line creation, explicit memory | create a bot, talk three turns, restart and both role and memory remain; its private chat is absent from Recent and Workspaces; `future session list` excludes it |
| **M2 Group** | groups and members, routing kernel, one-shot turns, serial execution, group transcript, `@` menu | two bots and a person: `@`-ing one gets exactly one reply; with no `@`, the owner takes it; a bot-to-bot chain stops at depth 2; a 9-turn message is cut by the cap with a stated reason; failure and timeout are visible |
| **M3 Permissions and metering** | group-turn workspace permission, the group-workspace allow rule, the token line, the startup sweep | the shared board is writable; writes elsewhere go to approval; usage matches the provider; killing the process between writes leaves an idempotent repair |

Land the phases separately; do not implement every candidate mechanism at once.

---

## 13. Acceptance and the go/no-go rule for the minimal slice

The primary designer/user runs it in **real work for seven consecutive days**;
demo tasks do not count. Record four things:

1. **Group usage**: at least five real group conversations covering at least
   three different tasks, and whether less context had to be ferried than in a
   single conversation.
2. **Bot-to-bot handoffs**: at least three naturally occurring bot `@` handoffs,
   not manufactured for the test, and how many of their results the person used
   directly.
3. **Roles and routing**: across at least ten real tasks, the proportion where
   the user felt "I know whom to ask"; for unaddressed messages, how often the
   owner had to be redirected. If redirection exceeds 30%, the `owner` rule
   fails and the group returns to mention-only.
4. **Cost**: input, output and cache-read tokens per turn, compared with doing
   the same task in one conversation. If the median exceeds 2× the
   single-conversation cost, reduce context and turn count before adding
   members.

These are also mandatory: no leak at the nine isolation entry points; failure
and approval are visible; and one real "remember this" is used correctly in a new
group the next day.

Go/no-go:

- Fewer than three group uses in the week and no natural bot-to-bot handoff:
  **stop the group feature** and keep only private bot chats.
- The group is used but the owner is often wrong: **keep `@` routing and remove
  unaddressed intake**.
- Cost exceeds the threshold: first reduce the transcript window and turn count;
  do not hide the problem with more memory or more instances.
- Only when roles are demonstrably better than tasks, handoffs produce value and
  cost is acceptable do we consider the candidates in §14.

---

## 14. Only after validation (not in v1)

The later order is recorded as order only, with no design; each phase starts as its own
task, and no phase begins before the previous one's criteria are met.

| Phase | One line |
|---|---|
| **P1** | Bot as an app: `bot.toml`, interface contracts, local invocation and audit |
| **P2** | Memory classes and evolution proposals: facts/preferences/skills, diff, trial run and rollback |
| **P3** | Template and fork sharing: an installed bot carries neither the source instance's memory nor its credentials |
| **P4** | Authorized invocation within an account or team: relay authentication, quota, revocation and audit |
| **P5** | Cross-user groups: a signed event log on the relay, with membership changes per epoch |

| Candidate | When to build it |
|---|---|
| A separate agent instance, separate `agent.db` and a second supervision chain | when bots are offered to other people, or a real incident proves shared-instance isolation insufficient |
| A separate `bots.db` | when measurement shows an `app.db` write hotspot or migration cost that justifies it |
| Hoisting configuration into `~/.future/config/` | when a second instance really needs shared config; do layout migration, backup and hard-deny paths together |
| A `router` judge or `eager` lightweight judging of every member | when §13 shows users frequently do not want to name speakers and `owner` is insufficient; measure accuracy and cost first |
| Monetary metering and a budget gate | when token usage proves a budget is needed and a reliable price source exists |
| Model-generated job skeletons, create-then-grow and three creation paths | when one-line creation is used and hand-written jobs become a real bottleneck |
| Automatic "recently learned" | only after there is a steady stream of explicit memories worth summarising |
| Extracting skills from sessions and running them on a schedule | after bots and groups prove durable value; not part of this slice |
| Durable binding sessions per bot and group | only if one-shot assembly is shown to lose decisive context; do not return to N×M duplication by default |

---

## 15. What was checked, and what was not

This separates code facts from behavioural promises.

### Checked by reading code

- `agent/src/rpc/session_prompt.rs`: accepted run settings contain
  `permission_level` and `sandbox_tier`; the sandbox resolves against
  `session_cwd`; the append prompt is re-read each run.
- `agent/src/prompt/mod.rs` and `agent/src/prompt/project_context.rs`:
  `AGENTS.md` and `FUTURE.md` are injected from cwd; the append segment comes
  after project context and memory.
- `agent/src/sandbox/rules.rs`: rule-layer order, the workspace rule file, and
  `ask` by default for writes outside cwd.
- `agent/src/sandbox/seatbelt.rs::build_profile`: allow rules enter the seatbelt
  profile, and the test `session_allow_rules_emit_allow_verbs` exists.
- `host/src/store/threads.rs::list_threads`: it currently filters only deletion
  state, so `scope` filtering belongs in the query.

### Checked by running commands

- `future run --help`: confirmed `--no-session`, `--cwd`,
  `--append-system-prompt`, `--permission` and `--message-file`.
- `future session new --help` and `future session set --help`: confirmed the
  persistence semantics of `--append-system-prompt`.
- `future skills --help`: confirmed the skills directory and installation
  mechanism.
- `make check-docs`: run and passed before this document was committed.

### Not checked (and therefore not promised as verified)

- No real run has yet verified end-to-end that the rule file allows the group
  workspace under macOS seatbelt.
- Linux and Windows rule and OS-sandbox behaviour have not been verified.
- Parallel groups, one bot `@`-ed by two groups, approval parking and timeout
  races have not been verified.
- Cross-machine behaviour has not been verified.
- Token cost, routing experience, memory-write frequency and every week-long
  usage metric remain unmeasured.

These belong to the implementation test plan, not to claims made by this
document.

---

## Appendix: external material

- Grok Bot documentation: [Bots](https://docs.x.ai/grok-bot/bots),
  [chat and collaboration](https://docs.x.ai/grok-bot/chat-and-collaboration),
  [skills and routines](https://docs.x.ai/grok-bot/skills-routines-and-automations),
  [computer and apps](https://docs.x.ai/grok-bot/computer-and-apps).
- Launch discussion, including a month-long user:
  <https://news.ycombinator.com/item?id=49261514>
- Two-week hands-on review: <https://www.youtube.com/watch?v=d9c5gloGPMU>
- Limit-change record and reporting:
  <https://www.youtube.com/watch?v=IULY-h-KwrI>,
  <https://techcrunch.com/2026/11/02/elon-musk-throttles-grok-bot-limits-due-to-unsustainably-high-costs/>
- Running a bot fleet: <https://granda.org/en/2026/09/06/how-i-run-grok-bot/>
- Field Notes (third party):
  [ORCHESTRATION](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/agents/ORCHESTRATION.md),
  [ANTIPATTERNS](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/ANTIPATTERNS.md),
  [PRODUCT](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/reference/PRODUCT.md)
