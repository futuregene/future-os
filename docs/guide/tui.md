# FutureOS Terminal UI (TUI)

The TUI is the terminal client: `future-tui`. It is a thin gRPC client that
connects over **per-user local IPC** by default (Unix-domain socket on
macOS/Linux, a current-user-only named pipe on Windows). Unix honors
`FUTURE_AGENT_SOCKET`; Linux otherwise uses `$XDG_RUNTIME_DIR/future/agent.sock`
when set, falling back to `~/.future/run/agent.sock` (the macOS default). With
`FUTURE_HOME` set it joins the Agent at `<FUTURE_HOME>/run/agent.sock` — see
[isolated instances](directory-layout.md#running-several-isolated-instances-future_home). If no agent is reachable, the TUI launches one as a sidecar and
shuts it down on exit — no manual startup needed. You can still run the
agent yourself:

```bash
future agent      # terminal 1: the agent
future tui        # terminal 2: the terminal UI
```

For remote/development setups pass `--grpc-addr <host:port>` to the agent to
serve TCP instead, and set `FUTURE_AGENT_GRPC_ADDR=<host:port>` on clients
(an explicit TCP target is authoritative and never falls back to local IPC).

`future tui <args>` runs the TUI in-process; the standalone `future-tui`
binary is equivalent but no longer installed by default (build it with
`cargo build -p future-tui` if you need it). `future tui --help` lists all
options (print mode, `--list-models`, `--session`, ...). One feature needs that
distinction: `/skills` installs and removes skills by spawning the unified
`future` binary, so a standalone `future-tui` needs `future` on `PATH` (see
[Skills](#skills)).

- Build / install: see [Build & Install](build-and-install.md).
- Session persistence, model config, tool approval and the sandbox all run
  through the agent; the TUI is a front-end. `/sandbox` and `/permission` edit
  the agent's session policy from here (see
  [Sandbox and tool permissions](#sandbox-and-tool-permissions)); nothing is
  changed behind your back at startup, apart from applying the TUI's own
  persisted `defaultPermissionLevel` (below).
- Fresh sessions default to permission `all`. The CLI's
  `future run --permission none` disables all tool calls, not just writes. See
  the [sandbox guide](../wiki/en/Sandbox.md) for the distinction.

## Project instructions

At each run, the agent reads the first readable file in the session's working
directory: `AGENTS.md` → `CLAUDE.md` → `GEMINI.md`. It does not merge files or
search parent directories; an empty readable file also stops the search.
`/reload` uses the same order, and context-file status lists file names, not contents.

Use `future tui --no-context-files` (short form `-nc`, also supported with `-p`)
to disable these project instructions. The opt-out is session-local, applies to
subsequent requests, and is reapplied when this TUI switches or creates sessions.
It does not erase earlier conversation content or disable the separate
`FUTURE.md` workspace memory layer. An agent that cannot apply the opt-out causes
the request to fail rather than silently ignoring the flag. `/context on|off`
toggles the same switch from inside the TUI.

## Slash commands

Every command below is intercepted by the TUI: it either opens a panel, changes
a setting through the agent, or runs one agent command — it is never sent to the
model as a prompt. Command names are case-insensitive; `arg` is everything after
the command name. This table is the authoritative dispatch set — the in-app help
overlay (`/help`) lists only a subset, and anything that is not a known command
(including a command missing its required argument, e.g. `/cwd`) is sent to the
model as an ordinary prompt.

| Command | Purpose |
|---|---|
| `/help` | Show the help overlay (shortcuts + core commands) |
| `/keymap` | Key-binding editor: list every action with its keys, rebind by pressing a key |
| `/model [name]` | Set the model directly, or open the searchable model selector with no arg |
| `/models` | Open the model enable-scope editor (same menu as `/scoped-models`) |
| `/models default` | Pick the agent-side default model for new sessions |
| `/scoped-models` | Configure the model enable/disable list |
| `/providers` | Manage providers: add/edit/delete, API keys, sync models |
| `/provider-key <id>` | Prompt for a provider API key (see below) |
| `/skills` | Skill browser: search, preview, insert, install / uninstall / upgrade |
| `/skill-recommend [on\|off]` | Offer a fitting skill before sending a message (on by default) |
| `/tools [none\|all]` | Multi-select the built-in tools; `none` disables all of them |
| `/permission [all\|workspace\|none]` | Set the tool permission level (and remember it); no arg opens the sandbox panel |
| `/sandbox` | Sandbox tier, backend availability and the permission picker |
| `/theme [id]` | Pick a theme, or set one directly by id |
| `/sessions` | Browse and switch sessions |
| `/new` | Start a new session |
| `/clone` | Clone the current session (continue in a new branch) |
| `/fork` | Fork from a chosen message |
| `/tree` | Session tree with fork/clone hierarchy |
| `/name <name>` | Set the session name |
| `/delete [--yes]` | Delete the current session (confirmed with `--yes`) and start a new one |
| `/title [zh\|en]` | Generate a session title with the model and apply it as the name |
| `/compact` | Compress the conversation context |
| `/status` | Session state, model, token usage, cost and this session's message/tool counters (printed into the chat) |
| `/usage` | Token, cost, context and quota panel |
| `/agent` | Agent version, instance id, discovered and loaded skills |
| `/metrics` | The agent's runtime counters |
| `/snapshot` | The current (or last) run's projection snapshot — it needs a run |
| `/history <query>` | Search this session's persisted history (up to 20 matches), snippets in the pager |
| `/tool-output [call-id]` | List this run's stored tool calls, or read one call's full output |
| `/transcript` | Full transcript in a searchable pager |
| `/copy` | Copy the last assistant message |
| `/export` | Ask the agent to write this session to an HTML file |
| `/import` | *Not available in the TUI* (stub, replies with a notice) |
| `/reload` | Reload skills + context files |
| `/cwd <dir>` | Change the working directory |
| `/worktree [new <branch>]` | List the repository's worktrees (branch, dirty state) and move this session into one; with `new` it adds a worktree under the main checkout's `.worktrees/` first |
| `/context [on\|off]` | List the context files, or turn loading them on/off |
| `/autocompact [on\|off]` | Turn automatic context compaction on/off |
| `/autoretry [on\|off]` | Turn automatic retry of failed runs on/off |
| `/shell <cmd>` | Run one command through the agent and show its output |
| `/stop` | Stop the current generation |
| `/cancel <run-id>` | Cancel a queued run |
| `/approve <request-id>` | Approve a pending tool execution |
| `/reject <request-id>` | Reject a pending tool execution |
| `/editor` | Edit the current draft in `$VISUAL` / `$EDITOR` |
| `/cancel-input` | Abort a pending `/provider-key` prompt |

`/editor` is handled before the input line is cleared, so it opens the
**current draft** and puts the edited text back into the input on exit; it does
not submit it.

`/provider-key <id>` starts a key prompt: the *next* submission is the key, not
a prompt — `/cancel-input` aborts the prompt instead. In the `/providers` list
the same flow is reached with `k`.

`/autocompact` and `/autoretry` accept `on`/`off` (also `true`/`enable` and
`false`/`disable`, case-insensitive); with no argument they flip the current
state. Both are agent-side settings for this session: the footer shows an
indicator while auto-compaction is on, and `/autoretry` is mirrored in the TUI
because the agent does not report it back. `/permission <level>`
also persists its argument as `defaultPermissionLevel` in the TUI's settings
file, so the next `future tui` starts from it (a level picked inside the
`/sandbox` panel is applied without persisting anything).

`/shell <cmd>` takes the raw rest of the line rather than the whitespace-collapsed
argument, so `printf '%s  %s'` reaches the agent unchanged. There is no PTY: it is
the agent's one-shot `shell` command — run in the session cwd under the agent's
own timeout (120 s by default) — and the captured output plus `exit code: N` open
in the pager.

`/delete` is the destructive command: without the literal `--yes` it only
explains itself and sends no request. `/delete --yes` removes the session file
and then starts a fresh session. `/worktree` is deliberately read-and-add only —
it runs `git worktree list`/`status`/`rev-parse` and `git worktree add`, and
refuses `remove`/`prune`/`reset`/`clean`/`checkout`/`gc`/`reflog` outright.

## Pop-up menus and the pager

The menu pickers — `/model`, `/models`, `/models default`, `/scoped-models`,
`/theme`, `/tools`, `/sessions`, `/tree` — share one pop-up-menu framework
(incremental search, tabs, multi-select, a fixed scroll window and footer
hints):

| Key | Action |
|---|---|
| typing, or `/` | Incremental search — filter the rows as you type |
| `↑↓` / `k` `j` | Move the highlight |
| `page up` / `page down`, `home` / `end` | Scroll the window; jump to an edge |
| `tab` / `shift+tab` | Switch tabs (menus that have them) |
| `space` | Toggle a row (multi-select menus, e.g. `/tools`) |
| `enter` | Confirm; in a multi-select menu applies the marked rows, and with nothing marked falls back to the highlighted row |
| `escape` | Clear the search query first, then close the menu |

An empty multi-select is not expressible that way, which is why "disable every
tool" has its own argument (`/tools none`).

`/providers`, `/skills` and `/sandbox` are richer panels with their own key
sets, documented in their sections below ([Providers and models](#providers-and-models),
[Skills](#skills), [Sandbox and tool permissions](#sandbox-and-tool-permissions)).

The session lists (`/sessions`, `/tree`, `/fork`) use a simpler filterable
list instead: type to filter (backspace deletes a character), `↑↓` moves the
highlight and wraps at the ends, `enter` selects and `escape` closes.

`/transcript`, `/agent`, `/metrics`, `/snapshot`, `/history` and
`/tool-output` open the whole result in a full-screen pager:

| Key | Action |
|---|---|
| `↑↓` / `k` `j`, `ctrl+u` `ctrl+b` / `ctrl+d` `ctrl+f`, `space` / `b` | Scroll by a line or a page |
| `g` / `home`, `G` / `end` | Jump to the top / bottom |
| `/` | Start an incremental search; `n` / `N` step to the next / previous match |
| `y` | Copy the current line (the pager closes on the copy) |
| `q` / `escape` | Close |

Its status row shows `[i/n] query` on the left and the scroll percentage on the
right. An empty result is reported as a chat message instead of opening an empty
pager (e.g. `No history matches for 'needle'.`).

## Keyboard shortcuts

| Key | Action |
|---|---|
| `ctrl+p` | Cycle model (within the scoped list when one is set) |
| `ctrl+t` | Cycle thinking level |
| `shift+tab` | Cycle thinking |
| `ctrl+o` | Expand / collapse thinking |
| `ctrl+g` | Expand / collapse tool output |
| `ctrl+d` | Compact view: fold runs of tool calls and thinking |
| `ctrl+x` | Copy the last assistant message |
| `ctrl+r` | Browse sessions |
| `ctrl+c` | Interrupt / exit |
| `ctrl+l` | Clear screen / redraw |
| `tab` | Autocomplete |
| `enter` | Submit / accept |
| `escape` | Close popup |
| `page up` / `page down` | Scroll chat up/down (scrolling up past the top loads older history) |
| `ctrl+↑` / `ctrl+↓` | Scroll chat up/down (line) |
| `↑↓` | Scroll / navigate lists |

While a popup or the pager is open every key goes to that overlay, so the global
shortcuts above apply to the chat input only.

### Compact view (`ctrl+d`)

A long agent run is mostly calls and reasoning, and each one costs rows. With
`ctrl+d` an *uninterrupted run* of completed calls to the same tool folds into
one summary row — `▸ read 3 files`, `▸ $ 4 commands` — and a thinking block
folds into a one-line marker (`▸ thinking`, `×N` when several blocks run
together). The same key expands everything again; the compact view is off by
default, and toggling it back restores the transcript exactly as it was.

A fold never hides something you need to read: a text answer, a user turn, a
call that is still running or one that failed all break the run, and a failed
call always keeps its own row and its body. Counting matches the desktop's
collapsed bursts — file tools count distinct files, shell commands count every
call. `ctrl+g` (expand every tool body) turns folding off, since asking for the
bodies and hiding the calls cannot both win; `ctrl+o` still hides thinking
entirely, marker included.

### History is paged

Switching to a session loads the newest ten exchanges; scrolling up past the
top of the transcript fetches the ten before those, and so on until the session
begins. Each page is one bounded read (the agent's indexed
`get_session_entries`), so a long session loads in a fraction of a second
instead of transferring its whole transcript in one response — and a session too
large for a single gRPC message is still readable. The older rows go above what
is on screen and the view shows them, so `page up` at the top is what walks back
through a conversation; a transcript whose top is the real beginning simply
stops there. `/transcript` shows the pages loaded so far, and `/export` always
writes the whole session (the agent reads it from its own store).

A tool call is one row: the call itself (`edit src/a.rs +12 -3`,
`$ make test`, `read src/main.rs:1-20`), with a diff's `+N -M` badge on that
same row — a transcript of twenty calls is twenty rows, not twenty previews.
`ctrl+g` expands the body beneath its row (up to 200 rows, with a
`… N more lines · truncated` marker at the cap) and collapses it again; a call
that *failed* keeps its body while collapsed, so the reason is readable without
knowing a key. `/tool-output <call-id>` prints one call's stored output in full.
Unified diffs
(`---`/`+++` headers, `@@` hunks) and `apply_patch` envelopes (the FREEFORM
provider tool whose grammar is captured under
`tests/provider-protocol/fixtures/`) get a line-number gutter and tinted
add/remove rows. Raw ANSI escapes inside tool
output are stripped so they cannot corrupt the screen — the diff renderer
supplies the only colour.

## Copying text

`/copy` and `ctrl+x` copy the most recent assistant message; `y` in the
`/transcript` pager (and in any other pager) copies the current line (and closes
the pager). Delivery is attempted in order:

1. a native clipboard program — `pbcopy` (macOS), `clip.exe` (Windows),
   `wl-copy` → `xclip` → `xsel` (Linux); this is the only path that *confirms*
   delivery,
2. tmux passthrough (`ESC Ptmux; … ESC \`) when the TUI runs inside tmux,
3. a plain OSC 52 request when the terminal understands it.

A native copy reports `Copied N characters.`; an OSC 52 copy only *requests* it
and the TUI says so (`the terminal has to apply it (unconfirmed)`). Payloads
above 100 KB are refused up front with an error. With no backend at all the TUI
reports `Copy failed: …`.

## Skills

`/skills` opens a browser over two sources: the skills the agent loaded for this
session (`get_commands`, which is what the panel has immediately, even offline)
and the installable catalogue (`future skills list --json`). The panel is opened
synchronously from the session's skills and then enriched by both fetches, so a
failed catalogue load is reported in the status row without hiding the rows that
did load.

| Key | Action |
|---|---|
| `/`, typing | Incremental search over the id, name, description and group (Chinese too, when the skill ships it) |
| `↑↓` / `k` `j` | Move the highlight |
| `tab` | Switch group tabs (`All`, `Installed`, `Available`, plus the skill's own group) |
| `enter` | Insert the highlighted skill's **canonical English name** into the prompt |
| `ctrl+o` | Show the highlighted skill's details in the pager |
| `i` | Install (or upgrade) the highlighted skill |
| `u`, `u` | Uninstall the highlighted skill — two presses confirm |
| `U`, `U` | Upgrade every installed skill that is behind — two presses confirm, and the prompt names them |
| `r` | Refresh: re-scan the agent's skill directories and re-read the catalogue |
| `s` | Cycle the scope (`All` / `Installed`) |
| `esc` | Close |

Rows merge the two sources: an installed skill shows its installed version
(`v1.2`), a skill the catalogue is ahead of shows `v1.2 → v1.3 ⬆`, a skill only
the catalogue knows shows `v1.4 · not installed`. `enter` on a row that is not
installed says so and points at `i` (it never inserts a name the agent cannot
resolve). While an operation runs, its row shows `working…` and a second
operation is refused until the first finishes.

Installing, uninstalling and upgrading are real, and go through the unified
binary: the TUI runs `future skills install <id> --version <v>` /
`future skills uninstall <id>` / `future skills update` as a child process
(120 s timeout). A standalone `future-tui` therefore needs `future` on `PATH`;
without it every entry point says
`The \`future\` executable was not found — installing or removing skills is
unavailable.` instead of failing later with an OS error. Only a *successful*
operation re-scans the agent (`refresh_skills`) and then re-reads both lists —
a failure, a timeout or a rejected id reports the outcome and re-reads nothing.

## Sandbox and tool permissions

`/sandbox` opens the sandbox panel. The same panel is what `/permission` opens
when you give it no argument, because the tier and the permission level belong
together.

| Key | Action |
|---|---|
| `↑↓` / `k` `j` | Move over the six rows (three tiers, then three permission levels) |
| `enter` / `space` | Apply the highlighted row |
| `r` | Re-probe the sandbox backend |
| `esc` | Close |

The tier rows are the desktop's three approval modes:

| Tier | Meaning |
|---|---|
| `Manual` | File access follows approval rules; allowlisted read-only commands run automatically, other commands ask. No OS sandbox. |
| `Sandboxed` | Commands run in the OS sandbox and ask for approval when needed. |
| `Unrestricted` (`off`) | No prompts, no sandbox — everything runs. |

The OS sandbox is native only to macOS (Seatbelt); on Linux the agent runs it
through the system Bubblewrap, and on Windows there is no sandbox tier at all
(commands run under FutureOS write protection instead). A panel opened on a
non-macOS host therefore says so, and the `Sandboxed` row is **disabled while the
backend is unavailable** — the highlight skips it and selecting it does nothing.

Availability has three states, and the panel never conflates them:

- **checking** — the probe has not answered yet (`Sandboxed (checking)`);
- **available** — a backend was found, with its name, path and version;
- **unavailable** — the probe came back without one, with the diagnostic reason
  and the stable diagnostic code (e.g. `bubblewrap` missing, not trusted, too
  old). A *failed* probe (transport error) stays "checking" instead of claiming
  the sandbox is missing, and the panel reports the error.

The agent downgrades rather than lying: asking for `sandbox` on a host whose
probe found no backend comes back as `manual`, and the panel prints the
fallback banner (`Sandbox requested but unavailable — fell back to Manual: …`)
instead of reporting success. The tier shown before you apply one here is this
platform's default and the panel labels it as such, unless a policy answer or a
`sandbox_policy_changed` event told it the session's real tier.

The permission rows are the agent's three levels for tool calls:

| Level | Meaning |
|---|---|
| `All` | Every tool call runs without asking. |
| `Workspace` | Tool calls ask for approval before they run; the grant is scoped to this workspace. |
| `None` | Every tool call is denied before it runs. |

`/permission <level>` applies one directly and persists it as
`defaultPermissionLevel` in `~/.future/tui/settings.json`; a level picked inside
the panel is applied without being persisted. `defaultPermissionLevel` is
re-applied to the agent the next time this TUI starts.

## Providers and models

The TUI edits the same agent-side configuration as `future models` and
`~/.future/agent/models.json`; every change goes through the agent, so it is
visible to the other clients too.

### `/providers`

The list has two tabs — **Built-in** (the catalog) and **Custom** — with
incremental search over the rows, plus these action keys (also listed in the
menu footer):

| Key | Action |
|---|---|
| `enter` | Edit the highlighted custom provider; on a built-in row it opens the key prompt instead |
| `e` | Edit the highlighted custom provider |
| `k` | Set / replace the highlighted provider's API key |
| `d` | Delete the highlighted custom provider |
| `a` | Add a custom provider |
| `s` | Sync the model list from `future models` |
| `r` | Reload `~/.future/agent/auth.json` |
| `escape` | Close |

Built-in providers are read-only apart from their key: `e` and `d` on a
built-in row only raise a notice, while `k` still works.

The add/edit form covers ID, name, API type, base URL, API key and the model
table:

| Key | Action |
|---|---|
| `tab` / `shift+tab`, `↑↓` | Move between fields |
| `←` / `→` | Cycle the API type (API row) |
| `ctrl+n` / `ctrl+d` | Add / remove a model row |
| `space` / `ctrl+t` | Toggle the highlighted model's image / thinking support |
| `enter` | Save (validated first), `escape` cancels |

A blank API-key field keeps the stored key; the agent never sends a key back, so
the form cannot show one.

### Selecting and scoping models

- `/model [name]` sets the session model; without an argument it opens the
  searchable model selector.
- `/models` and `/scoped-models` open the enable-scope editor — the models
  `ctrl+p` cycles through. The selection is persisted as `enabledModelIds` in
  `~/.future/tui/settings.json`.
- `/models default` picks the global default model used for new sessions —
  persisted in the **agent's** `~/.future/agent/settings.json`, not in the TUI
  settings file.
- `/status` prints session state (model, provider, image support, context
  window, token totals, cost) into the chat; `/usage` renders the same state as
  a panel: title row with the model and session name, the context bar, per-model
  token/cost rows, quota and queue warnings; `/agent` reports the agent's own
  version and instance facts. `/status` prints into the chat, `/usage` opens its
  own panel, and the rest open in the pager.

## Theme

`/theme` opens a searchable picker and `/theme <id>` sets a theme directly.
Known ids: `dark` (default), `light`, `one-dark`, `one-light`, `high-contrast`,
`dracula`. Ids are case-insensitive and `_` is accepted for `-`; an unknown id
falls back to the default. The choice is stored as `themeId` in
`~/.future/tui/settings.json` and reaches the whole interface: the chat area,
the footer, the pop-up menus, the provider list, the session lists, the pager
and the usage/sandbox panels. The default palette is unchanged from before the
palette was extended — `/theme dark` renders byte-identically to the built-in
colours. The input line's ported renderer emits no colour of its own (the prompt
is a plain `> `), so there is nothing there for a palette to change.

## Notifications and the terminal title

When one of *your* runs finishes or fails, the TUI can ring the terminal bell
and/or emit an `OSC 9` desktop notification (iTerm2, Ghostty, WezTerm, kitty,
warp). Runs started by another client on the same session never notify.

The channels live under `notify` in `~/.future/tui/settings.json`:

```json
{
  "themeId": "one-dark",
  "notify": { "enabled": true, "bell": true, "osc9": true, "title": false }
}
```

- `enabled` — master switch; `false` silences every channel.
- `bell` — emit `BEL`. The legacy top-level `bellOnComplete: false` still
  silences the bell.
- `osc9` — emit the desktop notification.
- `title` — additionally set the window title from each event. The run-state
  title is separate: while a run streams it reads `[>] <project> | <model> |
  <session>`, and it is written by default unless an explicit `notify` object is
  present (then `title` decides, and defaults to `false`).

Notification titles and bodies are sanitized — escape sequences, control
characters and bidi/invisible characters from a path or session name cannot
escape into the terminal — and titles are truncated to 100 columns. Nothing is
written when stdout is not a TTY, so print mode and piped runs stay clean.

## Settings & local files

The TUI persists client-side settings to `~/.future/tui/settings.json`:
`defaultModel`, `defaultThinkingLevel`, `defaultPermissionLevel`,
`enabledModelIds`, `themeId`, `bellOnComplete`, `skillRecommend` (set by
`/skill-recommend`, on by default) and the `notify` object above.
`defaultModel`, `defaultThinkingLevel` and `defaultPermissionLevel` are applied
to the agent when the TUI starts. Logs: `PI_DEBUG_REDRAW=1` writes debug redraw
logging to `~/.future/tui/debug.log`; `PI_TUI_WRITE_LOG=1` logs raw screen writes
to `~/.future/tui/write.log`.

The keys in the table above are the defaults. `/keymap` opens an editor over
every registered action: press a key on a highlighted row to rebind it, and the
difference from the defaults is written to `~/.future/tui/keybindings.json`
(beside `settings.json`), which is read at startup. A file that cannot be read,
or that names an action this build does not have, is reported inside that panel
and the defaults stay in force.

## Troubleshooting

| Symptom | Fix |
|---|---|
| Connection / gRPC error on startup | No agent could be found or started. Check the sidecar error, or start `future agent` yourself. In TCP mode, check nothing else holds the port: `lsof -i :<port>`. |
| Auth / "no model" error | No model configured. Run `future auth login`, add a provider through `/providers`, or edit `~/.future/agent/models.json` — see the repo README "Configure a model". |
| `/editor` reports a missing editor | Neither `$VISUAL` nor `$EDITOR` is set. Start with one, e.g. `EDITOR=vim future tui`. A blank `$VISUAL` does not fall back to `$EDITOR`. |
| `/copy` says "unconfirmed" or fails | The native clipboard program is unavailable (remote/SSH session?) and the copy went out as an OSC 52 request — the terminal has to apply it. A payload above 100 KB is refused; copy a smaller range. |
| `/skills` cannot install: "the `future` executable was not found" | The panel resolves `future` once, when this TUI starts. Run `future tui` (or put the unified binary on `PATH`) instead of the standalone `future-tui`; a restart is needed after installing it. |
| `/sandbox` says the sandbox is unavailable | The probe found no usable backend (macOS Seatbelt; Linux Bubblewrap). The reason and its diagnostic code are on the panel; `r` re-probes. On Linux the agent falls back to `Manual` and the panel says so rather than pretending the tier stuck. |
| A tool call is refused without asking | The permission level is `None` — check `/sandbox` → Tool permissions, or apply `/permission workspace`. |
| No bell / no desktop notification | Check `notify.enabled` (and `notify.bell` / `notify.osc9`, plus a `bellOnComplete: false` legacy key) in `~/.future/tui/settings.json`. A run started by another client, print mode and piped output never notify. |
| A theme does not repaint something | The chat area, footer, menus, provider list, session lists, pager and usage/sandbox panels all follow `/theme`. If a widget still looks unstyled, it is a bug — the input line is the only intentional non-participant, because it draws no colours. |

See also: [Directory layout](directory-layout.md) for what lives under
`~/.future/`, and the wiki [CLI](../wiki/en/CLI.md) / [Settings](../wiki/en/Settings.md)
pages for the desktop-app equivalents.
