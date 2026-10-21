//! Help text — verbatim port of `cli/src/help.ts` (`printHelp`) plus the
//! per-group help strings inlined in `cli/src/index.ts`. Bytes must match the
//! TypeScript CLI exactly (golden-tested in P4).

/// `printHelp()` from cli/src/help.ts.
pub const MAIN_HELP: &str = r#"Future OS CLI — agent gateway for the Future Agent gRPC server (per-user local IPC by default).

Usage:
  future <group> <command> [options] [args...]

Groups:
  init      Install built-in skills and initialize local commands
  config    Configure a model provider, and read/write global settings
  desktop   Read and change the desktop app's settings
  workspace List, and add, the desktop app's workspaces
  remote    Manage the devices paired to this machine's remote desktop
  auth      Authentication & API key management
  account   Platform account info
  run       Send a prompt to the agent (one-shot, non-interactive)
  skills    Install & manage agent skills
  task      Manage scheduled tasks (reusable prompt + trigger + runs)
  tools     List, describe, and call platform & browser tools
  mcp       Configure MCP servers and call their tools
  models    List available AI models from the agent
  session   List, inspect, update, rename, and delete agent sessions
  data      Offline history backup, verification and recovery
  doctor    Environment diagnostic
  version   Print the build identity (version, commit, target)

Apps (run the FutureOS components — same as their standalone binaries):
  agent     Start the agent gRPC server (future-agent)
  tui       Launch the terminal UI (future-tui)
  channel   Start the IM channel bridge (future-channel)
  goal      Goal control plane: goals/todos/gates (future-goal)
  headless  Foreground application host for mobile remote access
  relay     Self-hosted, account-free NATS relay for remote control

Quick start:
  future init                                Initialize Future OS
  future config                              Configure a model provider
  future auth login                          Sign in to the Future platform
  future headless                            Start the foreground application host
  future agent                               Start the agent server
  future tui                                 Launch the terminal UI
  future run "Explain this project"          One-shot agent prompt
  future run @README.md "Summarize this"     Include files in prompt
  future skills install-builtin              Install all built-in skills
  future doctor                              Check everything is working

Run 'future <group> --help' for per-group details.
  future headless --help     Foreground host, login and pairing
  future init --help         Initialization behavior
  future config --help       Interactive model-provider setup
  future desktop --help      Desktop app settings
  future workspace --help    Desktop app workspaces (list/add)
  future remote --help       Paired remote devices (list/remove/use)
  future run --help          All run options (model, fork, thinking, tools, etc.)
  future auth --help         Auth subcommands
  future account --help      Account subcommands
  future skills --help       Skills subcommands
  future task --help         Scheduled-task management (list/show/add/run/runs)
  future tools --help        Tool subcommands
  future mcp --help          MCP server and tool subcommands
  future models --help       Model listing options
  future session --help      Session management options
  future agent --help        Agent server options (gRPC addr, logging, profiling)
  future relay --help        Self-hosted relay (init/serve)
  future tui --help          TUI options (print mode, list models, etc.)
  future goal --help         Goal control plane commands
  future version --json      Build identity: version, commit, target
  future --version           Print version and exit"#;

/// `future init --help` output (index.ts).
pub const INIT_HELP: &str = r#"future init — initialize Future OS

Usage:
  future init

Installs all built-in skills. On macOS and Linux, also links future and, when
available, its sibling future-agent into ~/.future/bin/ and prints a PATH setup hint."#;

/// `future config --help` output.
pub const CONFIG_HELP: &str = r#"future config — configure a model provider, or read/write global settings

Usage:
  future config                       Interactive model-provider setup
  future config get [<key>] [--json]  Show the effective agent settings
  future config set <key> <value>     Change one setting
  future config get --help            All settings keys and their defaults

Subcommands:
  get       With no key, prints the settings file path and every effective
            setting value (defaults included). With a key, prints that value
            alone; --json prints it typed, or the whole document.
  set       Writes one key into ~/.future/agent/settings.json, creating the
            file if needed. Other keys, and any key this build does not know
            about, are left untouched.

Interactive setup (no subcommand):
  FutureOS  Reuse the device-code login flow. If a token is already configured,
            asks before replacing it.
  Custom    Prompt for provider ID, API protocol, base URL and API key, then
            keep, edit, delete or add models with their own token limits, image
            support and per-1M-token prices; then update
            ~/.future/agent/models.json and ~/.future/agent/auth.json.

API keys are read without terminal echo. If Future Agent is running, custom
provider changes take effect immediately; otherwise they apply on its next start.
Settings reads and writes never need the Agent: settings are read from disk when
they are used, so a `set` applies to the next new session (or the next Agent
start, for the compaction and retry policy).

Settable keys (the dotted names are the exact keys in settings.json):
  compaction.enabled                  true|false  Auto-compaction on/off
  compaction.reserve_tokens           integer     Context reserved for the reply
  compaction.keep_recent_tokens       integer     Recent tokens kept verbatim
  retry.enabled                       true|false  Automatic retry on/off
  retry.max_retries                   integer     Retries per request
  retry.base_delay_ms                 integer     Base backoff delay (ms)
  retry.provider.max_retry_delay_ms   integer     Provider-level retry cap (ms)
  maxTurns                            integer     Model+tool turns per prompt (0 = unlimited)
  defaultPermissionLevel              all|workspace|none
  defaultModel                        model id    Global default model ("provider/id"; "" = none)"#;

/// `future config get --help` output.
///
/// This is the canonical list of settable keys: `CONFIG_HELP` and
/// `CONFIG_SET_HELP` both point readers here ("Run `future config get --help`
/// for the settable keys"), so it has to carry them. `config_keys_are_documented`
/// pins that every key the command accepts appears here.
pub const CONFIG_GET_HELP: &str = r#"future config get — show the effective agent settings

Usage:
  future config get [<key>] [--json]

With no key, prints the settings file path and every effective value, including
the defaults for keys the file omits. With a key, prints that value alone, which
makes it usable in a script. --json prints the value typed (or, with no key, the
whole effective document as JSON).

Reads ~/.future/agent/settings.json and never writes it, so it works with the
Agent stopped. It does not read auth.json: credentials are never part of the
settings document.

Settable keys (the dotted names are the exact keys in settings.json, which nests
snake_case inside the camelCase top level):
  compaction.enabled                  true|false  Auto-compaction on/off
  compaction.reserve_tokens           integer     Context reserved for the reply
  compaction.keep_recent_tokens       integer     Recent tokens kept verbatim
  retry.enabled                       true|false  Automatic retry on/off
  retry.max_retries                   integer     Retries per request
  retry.base_delay_ms                 integer     Base backoff delay (ms)
  retry.provider.max_retry_delay_ms   integer     Provider-level retry cap (ms)
  maxTurns                            integer     Model+tool turns per prompt (0 = unlimited)
  defaultPermissionLevel              all|workspace|none
  defaultModel                        model id    Global default model ("provider/id"; "" = none)

When a change takes effect:
  defaultModel, defaultPermissionLevel   the next new session
  compaction.*, retry.*, maxTurns        the next Agent start

Change a value with `future config set <key> <value>`."#;

/// `future config set --help` output.
pub const CONFIG_SET_HELP: &str = r#"future config set — change one global agent setting

Usage:
  future config set <key> <value> [--json]

Writes one key into ~/.future/agent/settings.json (created if missing) and
prints the new value. Other keys are preserved, including keys this build does
not know about, and the file keeps the Agent's own formatting.

Run `future config get --help` for the settable keys, their accepted values and
their defaults.

Values are validated before the file is touched, and an invalid value or an
unknown key leaves the file exactly as it was.

When a change takes effect:
  defaultModel, defaultPermissionLevel   the next new session
  compaction.*, retry.*, maxTurns        the next Agent start

No Agent is required; a running one is unaffected, since these settings are read
from disk when they are used."#;

/// `future desktop --help` output.
pub const DESKTOP_HELP: &str = r#"future desktop — read and change the desktop app's settings

Usage:
  future desktop settings [<key>] [--json]   Show the effective settings
  future desktop settings get [<key>] [--json]
  future desktop settings set <key> <value>

The desktop app keeps its own preferences in an `app_settings` table in
~/.future/app/app.db. They are separate from the agent settings document that
`future config` writes, and from models/providers/auth, which the app and the
agent share.

With no key, prints the database path and every effective value (defaults
included). A database the desktop app has never written reports the defaults and
is not created. With a key, prints that value alone, which makes it usable in a
script; --json prints it typed, or the whole document.

No running desktop app is required. A running one picks the change up the next
time it reads the settings.

Settable keys (camelCase, as the desktop API spells them):
  approvalTier            off|manual|sandbox|auto  Approval tier for file access and shell
  hiddenModels            list               Model ids hidden from the model picker
  autoUpgradeSkills       true|false         Upgrade installed skills on app open
  autoConnectRemote       true|false         Keep the remote desktop on after a restart
  skillGuideDismissed     true|false         Skills onboarding banner dismissed
  skillIntroDismissed     true|false         Skills intro bubble acknowledged
  bellOnComplete          true|false         Bell when a run finishes
  autoTitleFirstTurn      true|false         Generate a title after the first answer
  titleLanguage           en|zh              Language used for generated titles
  communityEdition        true|false         Use the community-edition UI
  skillRecommend          true|false         Recommend one uninstalled skill
  autoResumeInterrupted   true|false         Continue interrupted conversations on app open

A list value is a JSON array or a comma-separated list; an empty value clears it."#;

/// `future workspace --help` output.
pub const WORKSPACE_HELP: &str = r#"future workspace — manage the desktop app's workspaces

Usage:
  future workspace list [--json]                    List the user's workspaces
  future workspace add <path> [--name <name>] [--json]
                                                   Add a directory as a workspace

A workspace is a directory on this machine that workspace conversations are
filed under. The records live in the desktop app's own database
(~/.future/app/app.db) — the same store the app's new-conversation dialog and a
paired phone write — so a workspace added here appears in the desktop app's
sidebar and on the phone once the app publishes its catalogue. No running
desktop app is required for the write.

The directory must already exist: a workspace points at a directory, it does not
create one. Adding a path that already has a workspace reopens that workspace
instead of creating a second one for the same directory, however the path is
spelled (~, a symlinked path, a trailing separator) — one directory is always
one workspace.

Workspaces also appear on their own: `future session set <id> --cwd <dir>` files
that conversation under the directory's workspace, and the desktop app does the
same whenever a cwd changes. `future workspace list` is then the place to see
where conversations are filed.

Examples:
  future workspace list
  future workspace add ~/projects/demo
  future workspace add /srv/app --name "Service"

With --json, list prints an array of workspace records and add prints the
record it created or reopened, with "created" saying which."#;

/// `future desktop settings get --help` output.
pub const DESKTOP_GET_HELP: &str = r#"future desktop settings get — show the desktop app's effective settings

Usage:
  future desktop settings [<key>] [--json]
  future desktop settings get [<key>] [--json]

With no key, prints the database path (default ~/.future/app/app.db) and every
effective value, including the defaults for keys never written. With a key,
prints that value alone, which makes it usable in a script. --json prints the
value typed (or, with no key, the whole effective document as JSON).

Reads only: a database the desktop app has never written reports the defaults
rather than creating one. Credentials are never part of this document."#;

/// `future desktop settings set --help` output.
pub const DESKTOP_SET_HELP: &str = r#"future desktop settings set — change one desktop app setting

Usage:
  future desktop settings set <key> <value> [--json]

Writes one key into ~/.future/app/app.db (created if missing) and prints the new
value. Other keys are preserved. Values are validated before the database is
touched, so an invalid value or an unknown key leaves it exactly as it was.

Run `future desktop --help` for the settable keys, their accepted values and
their defaults.

The running desktop app is unaffected until it next reads the settings; a change
made in the app's own Settings screen applies immediately."#;

/// `future remote --help` output.
pub const REMOTE_HELP: &str = r#"future remote — manage the devices paired to this machine's remote desktop

Usage:
  future remote status [--json]                      Whether the remote desktop is on, and who is connected
  future remote enable [--json]                      Turn it on now (no restart), and keep it on
  future remote disable [--json]                     Turn it off now (no restart), and keep it off
  future remote devices [list] [--json]              Every paired device, and the one in control
  future remote devices add [--json]                 Pair a new device: print its invitation
  future remote devices remove <id|#> [--yes]        Unpair one device; the others stay paired
  future remote devices remove --all [--yes]         Unpair every device
  future remote devices use <id|#>                   Make a paired device the one in control
  future remote peers [list] [--json]                Remote desktops THIS machine connects to
  future remote peers add <invitation>               Pair with one by pasting its link
  future remote peers connect|disconnect <id|#>      Take one up or down now
  future remote peers remove <id|#> [--yes]          Unpair it

status answers two different questions and keeps them apart: whether this
machine is serving its devices right now (runtime state, which only a running
Desktop knows), and the stored autoConnectRemote switch that decides whether it
serves again after a restart. Run `future remote status --help` for its fields,
its --json document and its exit codes.

enable and disable are that switch, from a terminal: they take effect at once
when a Desktop is running (the window, or `future headless`) and are stored for
the next launch either way, so neither needs a restart.

Every action takes --json and prints the same document `future remote devices
--json` prints (except `add`, which prints the invitation: pairId, link,
expiresAt).

A device is a phone or another computer that paired with this machine's remote
desktop — the Remote page in the window, or future headless. The list reads the
pairing book the Desktop itself reads (~/.future/remote_pairings.json), and use
writes it, so neither needs a running Desktop; a self-hosted relay needs no
account either.

add, remove and use are the exceptions to "the book is enough", because the
Desktop is what actually serves a device. An invitation is only usable while the
bridge for that pairing is connected, and a removal only takes effect *now* if
that bridge is stopped — so both ask the running Desktop (the window, or future
headless) and carry its answer back. use asks it too: control is an in-memory
pointer in that process, so handing the machine over is what makes the choice
take effect before the next restart. With no Desktop running, remove and use edit
the book itself, which is what the next start reads. A machine with no device at
all prints an invitation as the Desktop starts.

Self-hosted relays work the same way, and a machine may hold an official device
and a self-hosted one at the same time: `list` names each device's endpoint, and
`remove` sends it to the issuer that knows it — the platform API, or the relay's
own room. A relay with a private CA needs no extra flag here: name it with
FUTURE_RELAY_CA and it is applied to that relay's requests. Nothing is
remembered between runs — a self-hosted relay is chosen per launch with
`--relay`, and a launch without it uses the Future platform.

remove revokes the device where it was minted before dropping it locally. A
platform revoke that cannot be delivered now is queued and retried by the next
Desktop that runs; a relay has no account to queue against, so an undeliverable
room deletion is reported and the room expires on its own.

use writes the same pointer the window's "Let this control" writes, and with a
Desktop running it moves the controller at once: the Desktop is asked to hand the
machine over, which publishes the new controller, stands the previous one down
and persists the same choice. A device this Desktop has no bridge for cannot be
handed the machine (there is no channel to send it commands), and with the switch
off nobody is being served — in both cases the pointer is recorded and applies when
the Desktop next starts serving. A device whose bridge is up but whose client is
closed is handed the machine too, and the answer says that nothing will command it
until that device reconnects.

Run `future remote devices --help` for the ids remove and use accept."#;

/// The one-line usage every `future remote` argument error repeats.
pub const REMOTE_USAGE: &str = r#"Usage:
  future remote status [--json]
  future remote enable [--json]
  future remote disable [--json]
  future remote devices [list] [--json]
  future remote devices add [--json]
  future remote devices remove <id|#> [--yes] [--json]
  future remote devices remove --all [--yes] [--json]
  future remote devices use <id|#> [--json]
  future remote peers [list] [--json]
  future remote peers add <invitation> [--json]
  future remote peers connect <id|#> [--json]
  future remote peers disconnect <id|#> [--json]
  future remote peers rename <id|#> <name> [--json]
  future remote peers remove <id|#> [--yes] [--json]"#;

/// `future remote status --help` output.
pub const REMOTE_STATUS_HELP: &str = r#"future remote status — is the remote desktop on, and who is connected

Usage:
  future remote status [--json]

Two facts, printed separately because they are different facts:

  Remote desktop      Whether this machine is serving its devices right now.
                      Only a running Desktop (the window, or `future headless`)
                      can answer; with none running this reads "off — no
                      Desktop is running for this machine".
  Saved switch        autoConnectRemote, the same value the window's Remote page
                      stores. It decides whether the machine serves again at the
                      next launch.

The Devices block lists every paired device: whether it is in control, whether
its bridge is connected (only a running Desktop knows that), the device's own
name when it declared one, and where it was minted (the Future platform, or a
self-hosted relay).

--json prints one object:
  desktopRunning             a Desktop owns this data directory (window or headless)
  state                      off | unknown | on | connecting | serving | degraded |
                             failed | revoked — the word to test in a script
  phase                      the Desktop's own phase, or null when it did not answer
  serving                    the machine is trying to serve (connecting, ready,
                             reconnecting or refreshing)
  reachable                  phase is ready: a device can connect right now
  reason                     why it is not serving (network, credential_expired, …)
  recovery                   a retry in progress: attempt, maxAttempts, nextRetryAt
  switch.autoConnectRemote   the stored switch
  signedIn                   this machine has an account (or a relay) to serve with
  agentAvailable             this machine's Agent, or null when unknown
  transport                  where it connects, with any credential in the URL removed
  active                     the device in control, or null
  devicesTotal, devices      every paired device: pairId, control, connected
                             (the bridge), present (the client is actually
                             there), phase, endpoint (platform | relay),
                             relayUrl, client
  warningCode                a non-fatal local problem (e.g. the web listener)
  invitationExpiresAt        when an unscanned invitation lapses; the code itself
                             is never printed by this command

Exit codes:
  0  the status was read — including "off", "failed" and "revoked": a machine
     that is off answered the question.
  1  it was not read: a Desktop that owns this directory did not answer (an
     older build, or one that died mid-request), or the settings store could not
     be read. --json still prints the document — state "unknown", plus the half
     read from disk — so a script gets a document either way.

Test `state`, not the exit code:
  future remote status --json | jq -e '.state == "serving"'"#;

/// `future remote enable --help` output.
pub const REMOTE_ENABLE_HELP: &str = r#"future remote enable — turn the remote desktop on, now

Usage:
  future remote enable [--json]

Turns on the same switch the window's Remote page has, and stores it: the
machine keeps serving its paired devices at the next launch.

With a Desktop running (the window, or `future headless`) it takes effect
immediately — the request goes to that process, which starts the bridge, so
nothing has to be restarted. With none running there is no bridge to start: the
switch is stored, the command says so, and it exits 1 because the machine is not
serving yet.

Enabling does not pair anything. A machine with no device comes up on and empty,
which is a valid state; pair the first one with `future remote devices add`.

--json prints the same document `future remote status --json` prints.

Exit codes:
  0  the machine is serving (or connecting, or retrying)
  1  it is not: no Desktop was running to start it, the running Desktop did not
     answer, or the start came up failed or revoked. The report says which, and
     whether the switch was stored anyway."#;

/// `future remote disable --help` output.
pub const REMOTE_DISABLE_HELP: &str = r#"future remote disable — turn the remote desktop off, now

Usage:
  future remote disable [--json]

Turns off the same switch the window's Remote page has, and stores it: the
machine does not serve again at the next launch.

With a Desktop running it takes effect immediately — the bridge is stopped, and a
connected device is told the machine went away, so nothing has to be restarted. A
headless entry keeps running (it is the machine's control endpoint) and waits to
be enabled again. With none running there is nothing to stop: the switch is
stored, and that is the whole answer.

Paired devices are kept. Disabling serves nobody; it does not unpair anything —
`future remote devices remove` does that.

--json prints the same document `future remote status --json` prints.

Exit code:
  0  the remote desktop is off"#;

/// `future remote devices add --help` output.
pub const REMOTE_ADD_HELP: &str = r#"future remote devices add — pair a new device

Usage:
  future remote devices add [--json]

Prints a single-use invitation: a QR code (when the terminal is wide enough), the
complete `futureos://remote/pair` link, and when it expires. In the app, choose
Add device and scan it, or paste the complete link. The same invitation is also
on the Remote page while it lasts.

The invitation is minted by the running Desktop, which is the only process that
can serve the new device, so this needs one: a window, or future headless. With
none running, the answer is "start the Desktop" — and a machine with no device at
all prints an invitation as it starts, which is also the recovery path after
`remove --all`.

The invitation is whatever the Desktop is configured to mint: an official
`v=2` link, or a self-hosted relay's `v=3` one. Relay mode needs no FutureOS
account, and neither does this command — it only asks the Desktop.

An invitation is only usable while the Desktop serves, so with the remote desktop
switch off this refuses instead of minting one, and says to run
`future remote enable` (which starts serving at once when a Desktop is running).

Existing devices stay paired: this adds beside them. `remove --all` followed by
`add` is what replaces the pairing on a machine whose phone is being retired.

--json prints pairId, link and expiresAt for a script to render or forward.

When the invitation cannot be minted (the platform's device cap, a refused
credential, no sign-in) the reason is printed and the exit code is 1."#;

/// `future remote devices list --help` output.
pub const REMOTE_LIST_HELP: &str = r#"future remote devices list — every device paired to this machine

Usage:
  future remote devices [list] [--json]

Prints a row per device: its pair id, whether it is the one in control, and where
it was minted (the Future platform, or a self-hosted relay's URL). The `pair_`
prefix and the letter case of an id are decoration; remove and use accept either.

With one device there is no arbitration: it is in control. With several, the
device that last claimed the machine is; the others stay paired in standby and
cannot command it.

--json prints one object: desktopRunning, active (the pair id in control, or
null), and devices[] with pairId, desktopId, control, endpoint and relayUrl.
Credentials are never part of it.

Live connection state — who is connected at this moment, and the name a device
gave itself — is known only to a running Desktop: it is not in the pairing book.

The list says what is knowable without asking the Desktop: not serving because the
remote desktop switch is off — in which case it names `future remote enable`, the
command that turns it back on — nothing serving because no Desktop is running, or a
Desktop holding this data directory. Whether that Desktop is actually serving is
runtime state only it can answer: `future remote status` asks it."#;

/// `future remote devices remove --help` output.
pub const REMOTE_REMOVE_HELP: &str = r#"future remote devices remove — unpair one device, leaving the others paired

Usage:
  future remote devices remove <id|#> [--yes] [--json]
  future remote devices remove --all [--yes] [--json]

<id|#> is a pair id (the `pair_` prefix and letter case may be omitted, and a
unique prefix is enough) or the row number `future remote devices` prints.

--all removes every device at once — the "replace this machine's pairing" flow.
It takes no device argument; pair the first one back with `future remote devices
add`.

Asks for confirmation from the terminal unless --yes is given, and refuses to
read an answer from a redirected stdin: a prompt nobody saw must not become
"yes". One key answers — `y` or `Y` unpairs, `n`, `N`, Enter or Esc cancels —
so nothing has to be followed by Enter.

The device is revoked before the local slot is dropped — at the Future platform,
or by deleting its room at the relay that minted it. A running Desktop is asked to
do it first, which is the only way it stops being served *now*: the phone holds a
bearer credential that stays valid until it lapses, so editing the book alone
leaves the old bridge reading and answering it for minutes. With no Desktop
running (or one too old to know the request) the book is edited here, and the
device is served until its next credential refresh.
A revoke that cannot be delivered is queued in remote_pending_revokes.json for
the next Desktop to retry; the local removal still happens, because that is what
decides access to this machine.

Removing the device in control leaves nobody in control (with one device left,
that one controls anyway). Removing the last device removes the pairing file.

--json prints the resulting device document, like `future remote devices --json`."#;

/// `future remote devices use --help` output.
pub const REMOTE_USE_HELP: &str = r#"future remote devices use — choose the device that drives this machine

Usage:
  future remote devices use <id|#> [--json]

Writes the controlling device into the pairing book: the same pointer the
window's "Let this control" writes and the runtime restores at start. Control
moves the way an explicit takeover moves it, so the previous controller is no
longer allowed to command this machine.

With a Desktop running (the window, or future headless) it also asks that Desktop
to hand the machine over now, which is what makes the change immediate: control is
an in-memory pointer there, and only a takeover — the same one a phone performs —
moves it. Publishing the new controller and standing the previous one down is the
Desktop's own work, so a device this Desktop has no bridge for cannot be handed the
machine; the choice is still recorded, and the command says it applies when the
Desktop next starts serving. Nothing needs a restart otherwise.

Two notes can follow, on stderr, and neither means the choice was lost:

  - the device is not connected right now — the pointer moved, and nothing will
    command this machine until that device reconnects (the previous controller is
    in standby);
  - the running Desktop did not hand the machine over (no bridge for that device,
    or a build that predates the request) — the choice is recorded, applies at the
    next start, and the exit code is 1.

--json prints the resulting device document, like `future remote devices --json`."#;

/// `future task --help` output.
/// `future remote peers --help` output.
///
/// The client role, next to [`REMOTE_HELP`]'s host role. The two are one word
/// apart in the running product and mean opposite things ("who drives me" vs
/// "who do I drive"), so the help says which one this is before anything else.
pub const REMOTE_PEERS_HELP: &str = r#"future remote peers - the remote desktops this machine connects to

Usage:
  future remote peers [list] [--json]                 Every remote desktop this machine drives
  future remote peers add <invitation> [--json]       Pair with one: paste its invitation link
  future remote peers connect <id|#> [--json]         Connect to one now
  future remote peers disconnect <id|#> [--json]      Drop the connection, keep the pairing
  future remote peers rename <id|#> <name> [--json]   Name it here (--clear goes back to the id)
  future remote peers remove <id|#> [--yes] [--json]  Unpair it: stop connecting, revoke

This is the client role, the mirror of `future remote devices`. Devices are the
phones and computers allowed to drive THIS machine; peers are the machines this
one drives. The two pairings are independent: they have their own credentials,
their own runtimes, and their own files (~/.future/remote_peers.json here,
~/.future/remote_pairings.json there).

add is the only action that runs here instead of in the Desktop: pairing is a
one-time claim against the platform plus a handshake with the other machine, so
it needs nothing running on this one. On the other machine open Remote, take the
invitation it offers for connecting another computer, and paste the whole link
here. The pairing is stored and this command connects once, so the answer is
whether it works rather than whether it was written down. A running Desktop then
keeps it connected; with none running it connects the next time one starts (the
window - a headless host serves devices but does not drive other desktops).

Everything else reads the book, and connect, disconnect and remove also ask the
running Desktop, because a live connection and the user's own "disconnected"
mark live in that process and nowhere on disk. With no Desktop running:

  - list still shows what is paired, with no connection state to report;
  - rename still works: the name is this machine's private label for that host,
    never sent anywhere, and the Desktop reads it from the same file;
  - connect and disconnect have nothing to act on, and say so;
  - remove unpairs here and revokes where it can reach the platform, so nothing
    on this machine connects to it again.

The id is the other machine's desktopId, as `list` shows it; its `desktop_`
prefix is optional, and a row number (#1) names the same row.

Every action takes --json. list prints {desktopRunning, answered, peers}: a
peer's connected, stopped, error and agentAvailable are null when no Desktop
answered, which is the honest reading of "the book cannot know". add prints
{desktopRunning, paired}; the rest print the resulting list document, and remove
also names what it removed."#;

/// The one-line usage every `future remote peers` argument error repeats.
pub const REMOTE_PEERS_USAGE: &str = r#"Usage:
  future remote peers [list] [--json]
  future remote peers add <invitation> [--json]
  future remote peers connect <id|#> [--json]
  future remote peers disconnect <id|#> [--json]
  future remote peers rename <id|#> <name> [--json]
  future remote peers rename <id|#> --clear [--json]
  future remote peers remove <id|#> [--yes] [--json]"#;

/// `future remote peers list --help` output.
pub const REMOTE_PEERS_LIST_HELP: &str = r#"future remote peers list - every remote desktop this machine drives

Usage:
  future remote peers [list] [--json]

Reads the client-role book (~/.future/remote_peers.json) and, when a Desktop is
running, the connections that process holds. The two halves are kept apart in the
STATE column because only one of them is knowable from disk:

  connected       the Desktop holds a live connection to that machine
  disconnected    the user turned it off from this machine; the automatic
                  reconnect pass leaves it alone until `connect` asks for it
  not connected   paired, no live connection
  unknown         a Desktop is running but did not answer

With no Desktop running every row reads "not connected", because that is the
truth: a connection lives in that process.

--json prints one object:
  desktopRunning   a Desktop owns this data directory (window or headless)
  answered         that Desktop reported its connections
  peers[]          desktopId, pairId, name, icon, connected, stopped, error,
                   agentAvailable - the last four null when nothing answered"#;

/// `future remote peers add --help` output.
pub const REMOTE_PEERS_ADD_HELP: &str = r#"future remote peers add - pair this machine with another remote desktop

Usage:
  future remote peers add <invitation> [--json]

Takes the `futureos://remote/pair?...` link the other machine shows in its
Remote page ("connect another computer"). Paste it whole, in quotes: the link is
long and every parameter matters.

The pairing runs here, not in the Desktop: the claim and the handshake need
nothing local to be serving, which is what lets a machine be paired before its
Desktop is ever opened. The invitation is single-use and short-lived - an
expired one is refused, and a fresh link from the other machine is the only fix.

On success the pairing is stored in ~/.future/remote_peers.json and this command
connects once, so a broken pairing is reported now instead of at the next start.
A running Desktop keeps it connected from then on; with none running it connects
when one starts (the window; a headless host does not drive other desktops).

--json prints {desktopRunning, paired}, where paired is one peer row as
`future remote peers list --json` prints it."#;

/// `future remote peers connect|disconnect --help` output.
pub const REMOTE_PEERS_CONNECT_HELP: &str = r#"future remote peers connect|disconnect - take a pairing up or down now

Usage:
  future remote peers connect <id|#> [--json]
  future remote peers disconnect <id|#> [--json]

Asks the running Desktop to connect or drop one remote desktop. This is the same
call the window's own Connect and Disconnect buttons make, and it matters that it
is the same one: a disconnect also records the host as stopped in that process,
which is what keeps the automatic reconnect pass from undoing it a few seconds
later. Nothing about the pairing changes - connect brings it back.

With no Desktop running there is nothing to connect or drop (a connection lives
in that process), so both actions say so instead of pretending. A paired host is
connected by the Desktop as it starts, so start it and look at `list`.

connect reports the state the Desktop answered with, not what was asked: an
attempt that ended without a connection prints why, and exits 1."#;

/// `future remote peers rename --help` output.
pub const REMOTE_PEERS_RENAME_HELP: &str = r#"future remote peers rename - name a remote desktop here

Usage:
  future remote peers rename <id|#> <name> [--json]
  future remote peers rename <id|#> --clear [--json]

Writes this machine's private label for that desktop into the client-role book.
The name never leaves this machine: the other side neither sends nor stores it,
and the Desktop reads it from the same file, so a running one shows it on its
next refresh. --clear removes it and the machine id is shown again.

No Desktop is needed for this one."#;

/// `future remote peers remove --help` output.
pub const REMOTE_PEERS_REMOVE_HELP: &str = r#"future remote peers remove - unpair a remote desktop

Usage:
  future remote peers remove <id|#> [--yes] [--json]

Removes the pairing: this machine stops connecting to that desktop, and its
credential is revoked where the platform can be reached.

With a Desktop running it is asked first, because it owns the live connection and
can drop it now; its answer is what makes the removal immediate. With none
running, the entry is removed here and the revoke is attempted from this process,
so a machine without a Desktop is still cleanly unpaired. A revoke that cannot be
delivered is reported and is not fatal: the pairing is gone here, and the other
side stops accepting that credential when it expires.

Asks for confirmation; --yes skips the prompt, and a command without a terminal
on stdin must pass it."#;

pub const TASK_HELP: &str = r#"future task — manage scheduled tasks (reusable prompt + trigger + full-permission runs)

A scheduled task is a reusable work unit: prompt + working directory +
model/thinking level, run at full permission when its trigger fires — and on
demand whenever you ask. Each run produces an ordinary conversation and a
durable run-ledger entry.

Usage:
  future task list [--all] [--json]                 List scheduled tasks
  future task show <id|name> [--json] [--prompt]    Show one scheduled task
  future task add --name N --prompt P|--prompt-file F --cwd D
        [--model M] [--thinking L] [--session new|existing]
        [--session-retention keep|delete]
        [--conversation workspace|chat] [--disabled] [--json]
        (--manual | --at "YYYY-MM-DD HH:MM" | --every 30m
         | --daily [--time 09:00]
         | --weekly --days mon,wed,fri [--time 10:00]
         | --monthly --day 31 [--time 09:00]
         | --yearly --month 12 --day 31 [--time 22:00])
  future task edit <id|name> [any add flag] [--enable|--disable] [--json]
  future task enable|disable <id|name>
  future task remove <id|name> [--yes]              Soft-delete; runs are kept
  future task run <id|name> [--wait] [--timeout 15m] [--json]
  future task runs <id|name> [--limit N] [--json]
  future task output <run-id> [--tail N] [--json]   A run's full answer
  future task feedback <run-id> good|bad [--note "…"]
  future task prompt log <id|name> [--json]         The prompt version history
  future task prompt apply <id|name> <revision-id>  Put a stored version back in force
  future task prompt revert <id|name>               Back to the previous version

Triggers:
  --manual    Only runs when triggered: `future task run`, the desktop panel,
              the phone, or another agent. This is also what an add without any
              trigger flag produces.
  --at        One-shot on a calendar date/time (local time).
  --every     Every N minutes (30m, 2h, 1d). Anchored to a grid; no drift.
  --daily     Every day at --time (default 09:00).
  --weekly    Every selected weekday at --time (default 09:00).
  --monthly   Every month on --day (1-31) at --time (default 09:00).
              Months without that day run on the last day of the month.
  --yearly    Once a year on --month (1-12) and --day (1-31) at --time
              (default 09:00). A day the month cannot hold runs on the last
              day of that month, so 2/29 runs on the 28th in a common year.

Session:
  --session new       Each run opens a new conversation (default).
  --session existing  Reuse one conversation, so each run continues from what
                      the previous ones did.
  --session-retention keep    Keep the conversation (default).
  --session-retention delete  Delete it once the run settles, whether it
                      succeeded or failed. Only valid with `--session new`: a
                      conversation that is deleted cannot be the one the next
                      run continues.
                      What the run *did* is not lost — its status, its result
                      summary and its whole answer are recorded on the run, so
                      `future task runs` and `future task output <run-id>`
                      still answer. Only the conversation (its reasoning, its
                      tools, its transcript) goes.
  --conversation workspace  File the conversation under the working directory
                            (default).
  --conversation chat       Open it as a chat conversation instead (it runs in
                            the conversation's own temporary workspace, so
                            --cwd is optional).

Prompt versions:
  Editing a prompt (here or in the desktop) records a new version and keeps the
  one it replaced, so `prompt revert` can walk all the way back to the scheduled
  task's first prompt. Version source reads as: user (manual edit), rollback (a
  version re-applied), reflection (a version accepted from an older build's
  prompt suggestions), superseded (an outgoing version kept for the history).

Reading a run:
  `future task runs` lists the ledger (statuses, versions, summaries).
  `future task output <run-id>` prints the full answer behind a summary (the
  answer saved on the run; a run recorded before that was stored falls back to
  the conversation), and `feedback` records your verdict on a run.

Execution:
  Runs are executed by the desktop (or headless desktop) tick loop, not by
  this CLI. `future task run` queues a request; the next tick picks it up.
  `--wait` blocks until the run reaches a terminal state and prints its
  status, thread/session ids, and result summary.

Not to be confused with `future goal todo` (long-running goals with evidence
and gates). Scheduled tasks are fixed actions with triggers."#;

/// `future version --help` output.
pub const VERSION_HELP: &str = r#"future version — print the build identity of this CLI

Usage:
  future version [--json]
  future --version | -v | version      Same thing, plain output

Plain output is the display version (`future v0.0.2-479c8fee+local`), the same
string `future --version` has always printed. `--json` adds the facts that
string cannot carry:

  version         Display version
  isRelease       true when the version is a release (its first component is
                  non-zero); `0.*` is a dev build
  bundleVersion   Plain semver core, what installers use (they reject suffixes)
  gitCommit       Full commit this binary was built from, or null when the build
                  had no git checkout (tarball/vendored build)
  gitCommitShort  Abbreviated form
  gitDirty        Whether the tree had uncommitted changes at build time; null
                  when gitCommit is null
  buildTarget     Target triple the binary was compiled for
  buildProfile    Cargo profile (`debug` or `release`)

`gitCommit` is present even for release and coordinated test/nightly builds,
whose version string carries no hash at all. Use it to check whether the binary
you are running is the commit you are reading."#;

/// `future auth` group help (index.ts, no-command / --help branch).
pub const AUTH_GROUP_HELP: &str = r#"future auth — authenticate with the Future platform

Usage:
  future auth <command>

Commands:
  login       Device-code OAuth flow; saves API key to ~/.future/agent/auth.json
  status      Show whether logged in, and the platform URL in use
  credential  Output the API key + endpoint for shell scripts. Output is always JSON
              on success; use --json for consistent JSON error output when not logged in.
  logout      Remove the stored API key from auth.json

API key file: ~/.future/agent/auth.json
Environment override: FUTURE_API_KEY (takes precedence over auth.json)"#;

/// `future auth` group help shown after an unknown subcommand (index.ts) —
/// note the `credential` line differs from AUTH_GROUP_HELP.
pub const AUTH_GROUP_HELP_UNKNOWN: &str = r#"future auth — authenticate with the Future platform

Usage:
  future auth <command>

Commands:
  login       Device-code OAuth flow; saves API key to ~/.future/agent/auth.json
  status      Show whether logged in, and the platform URL in use
  credential  Output the raw API key + endpoint for shell scripts (--json not needed;
              output is always JSON: {"api_key":"...","endpoint":"..."})
  logout      Remove the stored API key from auth.json

API key file: ~/.future/agent/auth.json
Environment override: FUTURE_API_KEY (takes precedence over auth.json)"#;

/// `future auth login --help` output (index.ts).
pub const AUTH_LOGIN_HELP: &str = r#"future auth login — device-code OAuth flow

Usage:
  future auth login [--url <url>]

  --url <url>   Override the platform URL (default from DNS TXT record or built-in)
  --help, -h    Show this help

Opens a browser for you to sign in and authorize this CLI device.
Saves the resulting API key to ~/.future/agent/auth.json."#;

/// `future auth status --help` output (index.ts).
pub const AUTH_STATUS_HELP: &str = "future auth status — check current login state\n\nShows the platform URL and indicates whether an API key is stored.\nDoes not validate the key against the server.";

/// `future auth credential --help` output (index.ts).
pub const AUTH_CREDENTIAL_HELP: &str = r#"future auth credential — output API key for scripting

Usage:
  future auth credential [--json]

Output (always JSON on success):
  {"api_key":"...","endpoint":"..."}

  --json    When not logged in, emit JSON error instead of plain text.
            On success the output is always JSON regardless of this flag.

Useful for piping into other tools or CI/CD scripts."#;

/// `future auth logout --help` output (index.ts).
pub const AUTH_LOGOUT_HELP: &str = "future auth logout — remove stored API key\n\nDeletes the Future provider key from ~/.future/agent/auth.json.\nOther provider keys in the file are left untouched.";

/// `future tools` group help (index.ts).
pub const TOOLS_GROUP_HELP: &str = r#"future tools — list, describe, and call platform & browser tools

Usage:
  future tools list [--json]
  future tools describe <name>
  future tools call <name> --key1 val1 --key2 val2 [...]

Commands:
  list               Show available tools. --json for machine output.
  describe <name>    Show a tool's arguments and usage example.
  call <name>        Invoke a tool. Args as --key value. Use describe first to see
                     what arguments each tool accepts.

Requires authentication: future auth login, or set the FUTURE_API_KEY environment variable."#;

/// `future mcp` group help.
pub const MCP_GROUP_HELP: &str = r#"future mcp — configure MCP servers and call their tools

Servers are configured in ~/.future/mcp.json and run as local processes
speaking MCP over stdio. The FutureOS agent has no MCP client, so this CLI is
how an MCP server is reached from an agent session.

Usage:
  future mcp list [--json]
  future mcp status [<server>] [--json]
  future mcp tools <server> [--query <text>] [--limit <n>] [--offset <n>]
  future mcp describe <server> <tool>
  future mcp call <server> <tool> [--args '<json>'] [--out <file>] [--max-bytes <n>]
  future mcp serve <server>
  future mcp add <name> --command <program> [--arg <value>]... [--env KEY=VALUE]...
  future mcp remove <name>

Commands:
  list                     Configured servers, and the file they come from.
  status [<server>]        Start each server, shake hands, count tools, stop. Says
                           whether a configured server is actually usable.
  tools <server>           That server's tools, one line each, paged.
  describe <server> <tool> One tool's full description and argument schema.
  call <server> <tool>     Invoke one tool. Output is capped; --out writes all of it.
  serve <server>           Hold one session and take requests from stdin, one JSON
                           object per line: {"tool":"...","args":{...}}. Answers
                           with {"ok":true,"result":{...}} or {"ok":false,"error":"..."},
                           after a {"ok":true,"ready":true,...} ack. Use it when a
                           server's start-up is expensive: `call` starts the server
                           per invocation, `serve` pays that once.
  add / remove             Change the server list.

Two budgets, both tunable per command:
  --max-bytes <n>          How much of a result is *printed* (default 65536). The
                           rest is kept by --out.
  --max-frame-bytes <n>    How large one protocol frame may be *read* (default
                           33554432, or FUTURE_MCP_MAX_FRAME_BYTES). --max-bytes
                           cannot help when a frame does not fit at all; many
                           servers also have their own response budget.

`call` starts the server and stops it again; add --timeout <secs> when a server is
slow to start (default 60). An engine that must analyse the target on every launch
can need minutes, which is what `serve` is for."#;

/// `future skills` group help (index.ts).
pub const SKILLS_GROUP_HELP: &str = r#"future skills — install & manage agent skills

Skills are markdown instruction files the agent loads to handle specific tasks.
They live under ~/.future/agent/skills/<name>/SKILL.md.

Usage:
  future skills <command> [args]

Commands:
  list                    Show all skills available in the catalog (name, latest version,
                          installed version, description).
  install <name>          Install a specific skill by name. Use --version <ver> for a
                          specific version; omit for latest.
  install                 With no name argument, same as install-builtin.
  install-builtin         Install platform skills classified as builtin (from builtin/).
  uninstall <name>        Remove an installed skill.
  update                  Upgrade all installed skills to their latest versions.

Skills directory: ~/.future/agent/skills/
Catalog source: fetched from the Future platform API."#;

/// `future account` group help (index.ts).
pub const ACCOUNT_GROUP_HELP: &str = r#"future account — view platform account information

Usage:
  future account <command>

Commands:
  profile     Show account profile (email, user ID, verification status, creation date)
  balance     Show account credit balance. Use --json for machine-readable output.

Requires authentication: future auth login first."#;

/// `future models --help` output (index.ts).
pub const MODELS_HELP: &str = r#"future models — list available models from the running agent

Usage:
  future models [--json]

  --json    Output as JSON array with id, label, provider, contextWindow,
            supportsImages, thinkingLevel, and isDefault fields.
  --help    Show this help.

Requires a running agent (connects over per-user local IPC by default).
Set FUTURE_AGENT_GRPC_ADDR to an explicit TCP address for remote/development use."#;
