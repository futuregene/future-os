# Self-inspection

An agent that can look at itself can stop asking the user to repeat things it
already has. FutureOS already records everything worth knowing — how the agent
is configured, what it can do, and every conversation it took part in — and
exposes all of it through the same `future` CLI. There is no separate
"introspection" API and no new model tool: the agent reads itself with its
ordinary `shell` tool, so the same commands are auditable and reproducible by a
person.

The bundled `future-self` skill is the agent-facing layer on top of these
commands. It states which reads are safe, how bounded each one is, and which
changes need the user's agreement first. This page documents the commands
themselves.

## What is readable

| Command | What it reports |
|---|---|
| `future config get [<key>] [--json]` | The effective global settings, defaults included |
| `future desktop settings [<key>] [--json]` | The desktop app's own settings (approval tier, hidden models, …), defaults included |
| `future doctor` | One pass over login, agent connectivity, sandbox, providers, sessions and skills |
| `future models --json` | Models this agent can use |
| `future version --json` | Which build this is: version, commit, target, dirty state |
| `future auth status` | Whether a platform login is configured (and to what) |
| `future account profile` / `balance` | The user's account and remaining credits |
| `future skills list` | Installed vs. catalogued skills |
| `future tools list` / `describe <name>` | Platform and browser tools the **CLI** can call (not the model's `read`/`write`/`edit`/`shell`, which are set per session) |
| `future session list --json` | Every recorded session, newest first (each with its `cwd`, model and title) |
| `future session info <id> [--json]` | One session's model, cwd, message/tool counts, tokens and cost |
| `future session status <id> [--json] [--metrics]` | One session's **live** state: effective permission and sandbox tier, context occupancy, loaded context files and skills, active/queued runs, pending approvals |
| `future session transcript --session <id>` | One session's records, filtered and windowed — every user message, thinking, one tool's inputs or outputs, the paths it touched, and per-run outcomes (`--runs`) |
| `future session forks <id>` | The user turns that session can be branched at |
| `future session approvals <id>` | Approval requests the session is parked on right now |
| `future loop status` | Long-running goals for the current project directory |

Two of those are worth separating carefully, because they answer different
questions about the same session. `session info` reads the **persisted journal**
— what the conversation contains and what it has cost over its whole life.
`session status` reads the **live agent** — how the session is configured right
now and what it is doing. A session pinned to `workspace` permission while the
global default is `all`, or using 40% of its context window while having spent
40M tokens lifetime, is only correct in the second view; `config get` and
`session info` cannot express either.

`session status` also reports the sandbox tier a session's policy was set to
(`--sandbox` on `session set`), and distinguishes "never chose one" from an
explicit `off`.

`future config get` reads `~/.future/agent/settings.json` through the same typed
loader the Agent uses, so it reports what the Agent would actually apply rather
than what the file literally contains: a key the file omits still shows its
documented default. With a key it prints that value alone, which makes it usable
in a script. `future config` covers the *agent's* document only; the desktop
app's own preferences have their own command (see below).

Credentials are a deliberate exception. `auth.json` is never part of this
surface: `future config get` contains no key material, and the settings document
has no field for one. The account commands read that file themselves — the agent
never needs to, and never should.

## The account is the one remote read

`future account profile` and `future account balance` are the exception to
"everything here is on this disk". They reach the Future platform over the
network and need a login, so they fail offline and fail before `future auth
login` — a configuration state to report, not a broken account. Both are free
(reading a balance never spends credits), and a low balance is information to
pass on rather than a reason to create a recharge order.

## Finding your own session

An agent does not have to look up which session it is in — **its own system
prompt carries the id**, in the environment section:

```
Current session ID: <id>
You can reference this session ID when you need to identify or report which
conversation you are part of. This is your own session — you are self-aware of
this identifier.
```

Verified against a live run: the id in the outgoing system message is
byte-identical to that session's row in `agent.db`. So a search can use its own
id directly, with no lookup and no guessing.

```sh
future session list --json     # the *other* sessions; isStreaming marks the active one
```

`session list` is for sessions you are not in (a previous conversation to resume)
and for cross-checking: a session executing an active run reports
`isStreaming: true`, so during a turn exactly one row is `true`. Its summary
deliberately holds no usage — tokens and cost are in `session info <id>` — and its
rows are newest-first by `updatedAtMs`.

What does **not** work: the `shell` tool exports no `$FUTURE_SESSION_ID`, so the
environment carries nothing, and a title is not an identity (several sessions can
share one, and a fresh session has none).

## Which build is this

The version string is often not enough to identify the code, so
`future version --json` reports the facts it cannot carry:

```sh
future version --json
```

```json
{
  "version": "0.0.2-2a4df8a7+local.dirty",
  "isRelease": false,
  "bundleVersion": "0.0.2",
  "gitCommit": "2a4df8a738716ed63b933ad6bf488b975a4bd50b",
  "gitCommitShort": "2a4df8a7",
  "gitDirty": true,
  "buildTarget": "aarch64-apple-darwin",
  "buildProfile": "debug"
}
```

`gitCommit` is the point: a release tag (`1.2.3`) and a coordinated
test/nightly build (`0.0.2-<run>+test`) carry **no commit at all** in the
version, and a local dev build only an abbreviated hash. Comparing `gitCommit`
against `git rev-parse HEAD` is what answers "is the binary I am running the
commit I am reading?". `gitDirty` (uncommitted changes at build time) and
`buildTarget`/`buildProfile` are the rest of what a bug report otherwise has to
guess. When a build had no git checkout, the commit fields are `null` rather
than a placeholder, so a caller cannot mistake "not recorded" for a commit name.

The running Agent reports the same facts through `get_agent_info`
(`gitCommit`, `gitCommitShort`, `gitDirty`, `buildTarget`, `buildProfile`), which
is what makes the *process* comparable to a checkout or to the CLI at the other
end of the connection.

## Reading the implementation

The commands above say *what* is configured; the source says *why* it behaves
that way. When a behaviour is surprising, when a setting's effect is unclear, or
before claiming a limit, the answer is usually in the implementation — and the
repository carries its own orientation for that. The source lives at
[github.com/futuregene/future-os](https://github.com/futuregene/future-os), in a
checkout rather than in `~/.future/` (a session with no checkout should say so
rather than answer from memory); the skills list lives in
[futuregene/future-skills](https://github.com/futuregene/future-skills), the
`skills/` submodule.

| Where | What it gives you |
|---|---|
| `CLAUDE.md` | Workspace layout: which crate owns what, and which slice of `~/.future/` |
| `docs/README.md` | The docs index — guides, architecture, internals |
| `docs/guide/`, `docs/architecture/` | Released behaviour, and the design behind a subsystem |
| `FUTURE.md` + `.future/memory/` | Institutional gotchas recorded by earlier sessions |
| `packages/rpc/proto/future.proto` | The RPC wire contract (single source of truth) |

The usual questions map to one file: CLI surface in `cli/src/commands/` and
`cli/src/help.rs`, command routing in `agent/src/rpc/commands/mod.rs`, setting
semantics and defaults in `agent/src/config/mod.rs` (then `rg` the field for its
consumers), prompt assembly in `agent/src/prompt/mod.rs`, skills in
`agent/src/skills/`, session and history storage in `agent/src/session/`.

Two caveats matter more than the map. The running Agent is a built binary, so a
source claim is about the checkout, not about the process answering you — check
`future --version` before attributing behaviour to code. And a checkout may hold
another session's uncommitted work: read it, but never commit or reset in
someone else's tree.

Reading the source is for understanding, not for changing behaviour. Behaviour
that has a knob goes through `future config set`, `future session set` or a
skill; anything else is a bug to report, because a local edit forks the
installation from the release the user actually runs — and disappears at the
next update.

## Reading the user–agent record

`future session history` is the read-only recall surface
(see [Session history recall](session-history.md) for paging details):

```sh
future session history search --session <id> --query "text" [--limit 5] [--json]
future session history search --all --query "text" [--limit 5] [--sessions 50] [--json]
future session history get --session <id> --entry <entry-id> [--offset N] [--limit N] [--json]
```

`--all` is the difference between "what does this conversation say" and "have we
ever dealt with this". It searches across sessions rather than one, scanning the
`--sessions` most recently updated sessions (1..500, default 50) and labelling
each match with its `sessionId`. Because the scan is bounded, the response
reports how much of the history it covered:

- `scannedSessions` — how many sessions were actually searched;
- `truncated` — whether older sessions were left out;
- `hasMore` — whether more matches exist beyond `--limit`.

A caller that ignores those fields will confidently report "we never discussed
this" from an incomplete search. The skill requires the opposite: a negative
claim is only made when both flags are false.

Search is literal substring matching (ASCII case insensitive) over user and
assistant text, tool arguments and tool results — not semantic search, and not
over thinking/reasoning. When a query returns nothing, the query is usually the
problem; refine it rather than concluding the record is empty.

Search finds one passage; `future session transcript` is how a session is
*processed* rather than searched. It projects the same records through a
selectable lens — which roles and block kinds (`--select`, with thinking
included), which tool (`--tool`), a tool's arguments versus its result
(`--input` / `--output`), just the file paths either mentions (`--paths`), a
literal content filter (`--grep`) — and windows the result with `--cursor` /
`--limit` / `--all` and `--max-bytes`. Every emitted entry carries its run
outcome and token usage, `--runs` is a one-row-per-run ledger (status, duration,
tokens, error), and `--counts` answers "what is in here" in one cheap call.
`--json` makes every case scriptable. `session info <id> --json` is the same idea
for a session's identity: `cwd`, `model`, `thinkingLevel` and the computed stats
at the top level, the raw session metadata alongside them. See
[Session history recall](session-history.md#the-filtered-transcript) for the
option table and the two documented limits (heuristic `--paths`, and the
unattributed tool results `--tool` reports rather than mislabels).

What happened, run by run:

```sh
future session transcript --session <id> --runs --json   # which runs failed, and why
future session approvals <id>                            # what the session is waiting on
future session approve <id> <request-id> [--allow <glob> --access read|write]
future session abort <id>                                # stop the active run and clear the queue
```

## Changing settings

```sh
future config get                                   # everything, with defaults
future config get defaultPermissionLevel            # one value
future config set defaultPermissionLevel workspace  # change one key
future config set compaction.reserve_tokens 8192
```

`future config get --help` lists every settable key and its accepted values.
Values are validated before the file is touched: an invalid value or an unknown
key leaves `settings.json` exactly as it was. `set` edits the JSON document in
place, so keys written by a newer build — or by hand — survive a change made by
an older one.

Writes do not need a running Agent, and a running Agent does not need to be told:
settings are read from disk when they are used. The delay is therefore in *when
they are used*:

| Key | Takes effect |
|---|---|
| `defaultModel`, `defaultPermissionLevel` | The next new session |
| `compaction.*`, `retry.*`, `maxTurns` | The next Agent start |

Session-scoped settings stay with `future session set <id>`, which reaches the
running session at once and covers two groups:

```sh
# recorded with the session
future session set <id> --model <id> --thinking <level> --cwd <dir> --title <name> --parent <id>

# applied to the live session (until this agent stops)
future session set <id> --tools read,shell --permission workspace --sandbox manual
future session set <id> --system-prompt "…" --append-system-prompt "…"
future session set <id> --context-files off --auto-compact off --auto-retry on
```

`--tools` / `--no-tools` / `--no-builtin-tools` / `--system-prompt` /
`--append-system-prompt` / `--permission` / `--sandbox` / `--context-files` /
`--auto-compact` / `--auto-retry` are the live group. Read any of them back with
`future session status <id>` — which is why `--sandbox` is no longer
write-only. `--permission` is the approval gate and `--sandbox` the OS wrapping;
they are independent, and `sandbox` is refused when the platform cannot provide
one.

Plus `future session rename <id> <name>` for a title, `future session compact
--session <id>` for on-demand compaction (which acknowledges asynchronously — it
is not a completed summary, and it rejects a busy session), and the lifecycle
commands `future session new|fork|forks|clone|title|export`. Capabilities move
with `future skills install|uninstall|update`.

One gap is worth knowing rather than discovering: **a never-run session stores a
change only when it first runs.** A title or cwd on a session that already has a
record is written immediately; one that has never produced an entry stores it
with its first run, so a new session is absent from `session list` until it runs.

Two commands spend or act rather than read, and are explicit for that reason:
`future session title <id>` asks the session's model for a title (a model call —
it prints the suggestion and only renames with `--apply`), and
`future session abort|cancel|approve|reject` act on live work.

One more command acts, and it is the way one conversation reaches another:

```sh
future run --session <id> "<message>"     # start a run in THAT session
```

`--session` requires the session to exist (`switch_session` refuses an unknown
id rather than creating one) and the prompt is appended to it — behind an
in-progress run by default, or interrupting it with `--steer`. Which of those two
is right is the caller's judgement, not a fixed behaviour: appending leaves work
already under way alone, interrupting pre-empts it, and only the caller knows
what the user actually wanted. This is the send half of a `#`-picked conversation
reference: a user message can carry `[title](futureos://session/<id>)`, and the id
in that link is what `--session` takes. It is a full run in the other
conversation, so it spends credits, and the command returns when that run ends
(the shell tool's own timeout applies) — a long one needs a raised timeout. The
read half is `future session transcript --session <id>` /
`future session history search --session <id>` above.

Changing a setting changes how every later session behaves, so the skill treats
it as the user's decision: state the old value and the new one, and make the
change when the user agrees.

## The desktop app's settings

The desktop app keeps its own preferences — approval tier, hidden models, the
completion bell, the generated-title language and so on — in an `app_settings`
table in `~/.future/app/app.db`. They are neither the agent's settings document
(`future config`, `~/.future/agent/settings.json`) nor the models, providers
and auth files, which the app and the agent share.

```sh
future desktop settings                        # everything, with defaults
future desktop settings get approvalTier       # one value
future desktop settings set approvalTier manual
future desktop settings set hiddenModels "future/glm-5.3, future/kimi-k3"
```

Keys use the camelCase spelling of the desktop API; `future desktop --help`
lists every key with its accepted values, and each preference the app's Settings
screen can change is settable here too. A list value is a JSON array or a
comma-separated list. Values are validated before the database is touched, so an
invalid value or an unknown key leaves it exactly as it was; a read against a
database the app has never written reports the defaults and does not create one.

Reads and writes need no running desktop app, and the app picks the change up
the next time it reads the settings; a change made in the app's own Settings
screen applies immediately.

## Boundaries worth stating

- **Literal, not semantic.** No embeddings, synonyms or stemming; two words are
  one substring.
- **Bounded.** At most 20 matches per search, at most 500 sessions per
  cross-session scan, 4..32768 bytes per entry read.
- **Original records only.** Reasoning is excluded by design, media bodies and
  provider metadata are omitted, and compaction summaries are not exposed — so a
  recorded conversation is not byte-identical to what the model saw.
- **Local and single-machine.** Sessions under another `FUTURE_HOME`, or on
  another device, are not visible.
- **No credentials**, in reads, in summaries, or in anything written on the
  user's behalf.

## See also

- [Session history recall](session-history.md) — search/read semantics and byte
  paging.
- [Directory layout](directory-layout.md) — what lives where under `~/.future/`.
