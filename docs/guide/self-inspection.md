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
| `future doctor` | One pass over login, agent connectivity, sandbox, providers, sessions and skills |
| `future models --json` | Models this agent can use |
| `future auth status` | Whether a platform login is configured (and to what) |
| `future account profile` / `balance` | The user's account and remaining credits |
| `future skills list` | Installed vs. catalogued skills |
| `future tools list` / `describe <name>` | The tool surface, with arguments and examples |
| `future session list --json` | Every recorded session, newest first |
| `future session info <id>` | One session's model, cwd, message/tool counts, tokens and cost |
| `future loop status` | Long-running goals for the current project directory |

`future config get` reads `~/.future/agent/settings.json` through the same typed
loader the Agent uses, so it reports what the Agent would actually apply rather
than what the file literally contains: a key the file omits still shows its
documented default. With a key it prints that value alone, which makes it usable
in a script.

Credentials are a deliberate exception. `auth.json` is never part of this
surface: `future config get` contains no key material, and the settings document
has no field for one.

## Reading the implementation

The commands above say *what* is configured; the source says *why* it behaves
that way. When a behaviour is surprising, when a setting's effect is unclear, or
before claiming a limit, the answer is usually in the implementation — and the
repository carries its own orientation for that:

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

Session-scoped settings stay with `future session set <id>` (`--model`,
`--thinking`, `--cwd`, `--title`), which reaches the running session at once.
Capabilities move with `future skills install|uninstall|update`.

Changing a setting changes how every later session behaves, so the skill treats
it as the user's decision: state the old value and the new one, and make the
change when the user agrees.

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
