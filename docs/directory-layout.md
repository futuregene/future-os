# Directory layout: what lives under `~/.future/`

FutureOS keeps all of its user state under `~/.future/` (on Windows:
`%USERPROFILE%\.future\`). This page maps each subdirectory to the component
that owns it and what it stores. Paths below use the macOS/Linux form; the
Windows layout is identical with `%USERPROFILE%\.future\` as the root.

```text
~/.future/
├── agent/                     # the agent backend (future-agent)
│   ├── settings.json          # agent settings (model defaults, sandbox, …)
│   ├── models.json            # provider/model catalog: apiKey, baseUrl, models[]
│   ├── auth.json              # credentials, keyed by model id or provider
│   ├── sessions/              # flat JSONL session store (one file per session)
│   ├── skills/                # installed user skills (APP_SKILLS_DIR)
│   ├── browser/               # CLI browser-tool state (config.json, profile/, artifacts/)
│   ├── images/                # CLI image-tool output directory
│   └── logs/agent.log         # agent log (when logging is enabled)
├── agent-app/                 # legacy credential dir (auth.json) read for back-compat
├── channels/
│   ├── config.json            # Feishu / DingTalk bridge config (see channels-config.md)
│   └── feishu/                # Feishu bridge data (session file, received files)
├── tui/                       # the terminal UI (future-tui)
│   ├── settings.json          # defaultModel, defaultThinkingLevel, … (see tui.md)
│   ├── keybindings.json       # optional keybinding overrides
│   ├── debug.log              # TUI runtime log
│   ├── write.log              # raw screen-write log (PI_TUI_WRITE_LOG=1 only)
│   └── crash.log              # panic backtrace appended on crash
├── app/                       # the desktop GUI (FutureOS app)
│   ├── app.db                 # SQLite database (threads, runs, approvals, …)
│   ├── images/                # per-thread image tree (thumb/ + origin/)
│   ├── review/                # per-workspace shadow git review repos
│   └── run_events/            # per-run event logs (JSONL)
├── workspaces/
│   └── chat/                  # per-thread chat workspaces (agent session / thread id)
├── remote_pairing.json        # desktop remote-bridge identity (nkey_seed + user_jwt)
└── bin/                       # CLI / agent links: `future`, `future-agent` (see below)
```

## `~/.future/agent/` — agent backend

Owned by `future-agent` (the gRPC backend on `127.0.0.1:50051`). Reads its
config purely from files here — there are no model-related CLI flags or env
vars:

- `settings.json` — agent settings.
- `models.json` — provider catalog in the shape
  `{"providers": {"<provider>": {"apiKey": …, "baseUrl": …, "models": [{"id", "name", "contextWindow"}]}}}`.
  `future auth login` syncs this automatically; it can also be hand-edited.
- `auth.json` — credentials, keyed by model id first, then provider, then a
  default entry: `{"<provider>": {"type": "api_key", "key": …, "baseUrl": …}}`.
- `sessions/` — flat directory of JSONL session files (the agent's default
  session dir).
- `skills/` — one of the two skill discovery directories
  (`APP_SKILLS_DIR`); the other is `~/.agents/skills/` (`AGENTS_SKILLS_DIR`).
  Skills are plain directories with a `SKILL.md` + YAML frontmatter.
- `browser/` — CLI browser-tool state (`config.json`, `profile/` for the
  Chromium profile, `artifacts/` for screenshots). Honors `FUTURE_HOME`.
- `images/` — output directory for the CLI image generation/editing tools
  (`future tools call image …`).
- `logs/agent.log` — written when logging is enabled.

## `~/.future/agent-app/` — legacy credential directory

The agent resolves `auth.json` from `~/.future/agent-app/auth.json` before
`~/.future/agent/auth.json` (back-compat with credentials written by older
GUI builds); both `agent/` and `agent-app/` are treated as credential
locations by the GUI's file-access guard. New writes go to `~/.future/agent/`.

## `~/.future/channels/` — channel bridges

Owned by `future-channel` (the Feishu / DingTalk bridge). `config.json`
holds the `agent`, `feishu` and `dingtalk` blocks — see
[channels-config.md](channels-config.md) for the full schema and defaults.
If the file is missing, the bridge writes a default template and exits,
asking you to edit it and restart. `feishu/` is the Feishu bridge's data
directory (session file and received files/images).

## `~/.future/tui/` — terminal UI

Owned by `future-tui`. `settings.json` persists client-side settings
(`defaultModel`, `defaultThinkingLevel`, `defaultPermissionLevel`,
`enabledModelIds`); optional keybinding overrides go in `keybindings.json`;
`debug.log` is written when `PI_DEBUG_REDRAW=1`, and `write.log` records raw
screen writes when `PI_TUI_WRITE_LOG=1`; `crash.log` receives the panic
backtrace when the TUI crashes. See [tui.md](tui.md).

## `~/.future/app/` — desktop GUI

Owned by the Tauri desktop app (see `desktop/`):

- `app.db` — the SQLite database (threads, runs, approval requests, …).
- `images/` — persistent per-thread image tree (`<thread_id>/thumb/` and,
  for workspace conversations, `<thread_id>/origin/`). Kept under `~/.future`
  rather than the OS cache dir because macOS may purge the cache.
- `review/` — shadow git repositories used for the review feature, one
  `<workspace_id>` subdir per workspace's runs.
- `run_events/` — per-run event logs (JSONL), derived from the agent's JSONL
  sessions.

The desktop remote bridge keeps its pairing identity (`nkey_seed` +
`user_jwt` + NATS addresses) in `~/.future/remote_pairing.json` (at the
`~/.future` root, not under `app/`).

## `~/.future/workspaces/chat/` — chat workspaces

Per-thread chat workspaces for the GUI, one subdir per agent session id (when
known, e.g. from an import) or GUI thread id. User-chosen workspaces live
elsewhere and are never touched by reclamation of this directory.

## Loop control plane — project-local, not under `~/.future/`

The `future-loop` state is **project-local**: run it from the project
directory and everything lives under `<cwd>/.future/loop/`
(`FUTURE_LOOP_ROOT` overrides the root for special setups; `~/.future/loop/`
is not used). See [loop-control-plane.md](loop-control-plane.md).

```text
<cwd>/.future/loop/
├── registry.json                  # goal registry (one entry per goal)
├── goals/<goal_id>/
│   ├── events.jsonl               # event-sourced ledger (the authoritative state)
│   ├── runs.jsonl                 # authoritative spend/run ledger
│   ├── next_action.txt            # kernel should-run decision snapshot
│   ├── schema.json                # event-store schema version stamp
│   ├── ACTIVE_GOAL_STATE.md       # human-readable active-state projection
│   ├── status-cache.json          # status projection cache
│   ├── read_diagnostics.json      # ledger-read diagnostics (unknown event kinds)
│   ├── scheduler-state/           # scheduler state (backed up with the goal)
│   └── runs/                      # run-history (compaction/retention, LoopX-style)
│       └── index.jsonl            # append-only run index
├── runs/
│   └── <run_id>.live.jsonl        # live in-flight worker run logs
├── inbox/
│   └── *.json                     # operator inbox (liveness alerts, …)
└── backups/
    └── <ts>-<goal_id>/            # per-goal backup (ledger + scheduler-state + registry entry)
```

## `~/.future/bin/` — CLI links

`future init` installs the built-in skills and (macOS/Linux) symlinks
`future` — plus `future-agent`, when it sits next to the `future` executable
— into `~/.future/bin/`, printing a PATH setup hint. On a default install
only `future` is linked (the standalone binaries are no longer installed).
The Windows installer installs to `%USERPROFILE%\.future\bin` as well.

## Related

- `~/.agents/skills/` — second skill discovery directory (`AGENTS_SKILLS_DIR`),
  e.g. for machine-wide skills.
- Project-local `.future/` — GUI chat workspaces and the loop control plane
  also use a `.future/` directory inside a project (e.g. `.future/loop/`,
  `.future/approval_rule.json`). It is meant to be gitignored.
