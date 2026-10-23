# Host mode guide

[中文](desktop-host.zh-CN.md)

Host mode lets the FutureOS Desktop backend provide phone remote access from a
terminal, without opening a window or WebView. It works over SSH and on local
machines that do not need the desktop UI. It reuses Desktop's sessions,
workspaces, approvals and file handling; it is not a separate Remote service or
SDK.

**The process stays in the foreground. Press Ctrl+C to close remote access and exit.**
It does not daemonize or install a system service.

The unified entry ships with the CLI. Release packaging, signing and same-candidate acceptance are tracked in the [Headless extraction plan](../architecture/headless-package-development.md).

## 1. Before you start

- Install the unified `future` CLI, including from a CLI-only archive. Run
  `future host`; no separate server executable or Desktop installation is needed.
- The complete Agent runs inside this process when no external service owns the
  default endpoint/data directory. Maintenance invokes this same CLI executable.
  An external Agent is attached only after compatibility/readiness checks.
- Install a FutureOS mobile app matching the host's production/test environment.
- Allow the host to reach the Future OS platform and configured NATS relay. The
  phone also needs network access.
- Run as an ordinary system user. Platform credentials, pairing and tool execution
  belong to that user; do not use root just for convenience.
- Only one Desktop backend may own a data directory. Close the GUI using that
  directory before starting host mode.

Platform access requires valid Future OS account credentials, but **no browser is
needed on the server** and no account is needed to start: a signed-out host runs
everything local (Agent, runs, tasks, its own providers) and keeps the remote
desktop closed. Signing in is a separate command — `future auth login`, which
prints an authorization link and user code — and a running host picks that login
up and starts serving without a restart. Pairing a device is separate again:
`future remote devices add` prints the invitation in the terminal that runs it.

## 2. Start in the foreground

After a `.deb` install, run `future host` directly. For a portable or source
build, change to the directory containing the executable.

Linux / macOS:

```bash
./future host
```

Windows PowerShell:

```powershell
.\future.exe host
```

No `--headless` flag is needed or accepted. `futureos` starts only the graphical
Desktop; its former `--headless` option has been removed. Replace old commands
such as `futureos --headless --no-qr` with `future host`.
Neither entry point takes pairing flags any more: devices are managed with
`future remote devices` (section 3.1), which asks the running Desktop.
Both executables share the backend, data and phone protocol; this is not a second
implementation.

No additional `--pair --qr` options are needed, and none exist: the entry serves
the devices already paired to this machine, and pairing a new one is a separate
command (`future remote devices add`, section 3.1). Pairing does not end anything
or return a shell prompt: the process keeps running and serves the phone.

If the remote desktop switch is off — turned off in the window's Remote page or
with `future remote disable` — the entry starts idle instead of serving: it stays
up as the machine's control endpoint, and `future remote enable` starts serving
without a restart (section 3.2).

### Step one: platform authorization

The host does not run a login of its own. A server that is signed out starts
anyway, says so, and serves everything local; only the remote desktop stays
closed. Sign in from a second terminal on the same machine:

```bash
future auth login
```

1. Scan with your phone camera/browser, or open the URL in a browser on your own
   computer or phone.
2. Sign in to Future OS and authorize this server. Enter the terminal's user code
   if the web page requests it.
3. The login is saved through the running Agent, and the host adopts it within a
   second: it prints `Platform account signed in`, fetches the model catalogue
   and starts serving the devices already paired to this machine — no restart.

This authorizes the server to access your platform account. It does not yet allow
the mobile app to control the server. No server browser, SSH graphical forwarding
or inbound browser callback port is required.

A *different* login or a logout while the host is running closes remote access and
asks for a restart: the previous account's remote entry must not stay open until
its own credential expires. Everything local keeps running either way.

A stored credential is still verified against the platform at startup, exactly as
before: accepted means it serves; refused (401/403) prints `Platform authorization
is no longer valid. Please sign in again.` and continues in the signed-out state
until `future auth login` stores a new one; a transport failure is still reported
as an error rather than being mistaken for a logout.

### Step two: mobile app pairing

The host prints no pairing code of its own. Pairing is a request the operator
makes, in a second terminal on the same machine:

```bash
future remote devices add
```

That command asks the running host to mint an invitation and prints it where it
was asked for: a QR code, the complete `futureos://remote/pair?...` link, and the
time left before it expires.

An invitation is minted by the platform, so this is the one local-looking command
that needs an account: signed out, `add` refuses with `future auth login`. The
host itself is not broken when that happens — sessions, runs, tasks and your own
model providers keep working, and `add` succeeds once a login has been adopted.

- Scan with **Add device / Scan inside the FutureOS app**.
- Alternatively, paste the complete link into the app's **Paste pairing code**
  entry.

The pairing code is the complete invitation link, not a short numeric code.
Invitations are valid for five minutes and are single-use. Do not share them,
post screenshots or save them in shared logs. The host keeps running while you
pair, and starts serving the new device as soon as the phone claims the
invitation — no restart, and nothing else to run.

### Already signed in or paired

Valid login and pairing are reused, skipping the corresponding steps. The paired
phone can reconnect without scanning every time. A ready entry or saved pairing
does not prove the phone is currently online: open the app and connect to the host.
Ordinary network outages do not deliberately replace valid pairings; the existing
Remote connection logic handles recovery.
### What the server maintains on its own

The host serves the same backend as the window, so the phone sees the same
sessions, settings, skills, models and tasks. Two steps the window's interface
drives are performed by this entry point itself, because a server has no window
to render them:

- **Model catalogue refresh.** The Future catalogue is fetched at startup and
then every 24 hours, on the same schedule policy as the window's job. The Agent
announces the change itself, so a phone that is connected picks up new models
through the same invalidation stream the windowed desktop uses.
- **Built-in skills.** The bundled `future init` runs once after the first
successful login (and on later launches — it is idempotent), installing the
platform's built-in skills exactly as the window's post-login onboarding step
does. Its output is printed with a `[skills]` prefix; a failure is reported and
does not stop the server.

The window's other two scheduled jobs have no server equivalent: the app-update
check needs the desktop updater plugin, and the balance refresh feeds a window
event the phone does not read.

## 3. Startup options

| Option | Behavior |
|---|---|
| `--relay URL` | Point new invitations at a self-hosted relay instead of the Future platform, for this launch only |
| `--relay-ca` | PEM file with that relay's CA, when it is not issued by a public CA |
| `--help` | Show help without starting Desktop or Agent |

For example:

```bash
./future host --relay https://relay.example.com
./future host --help
```

`future remote devices add` prints the invitation as a QR code with the complete
link beside it. A narrow terminal automatically falls back to the link alone and
says why; there is no flag for it, and no flag for pairing on this entry point
either — see below.

### 3.1 Managing paired devices from the terminal

One machine can hold several paired devices (phones, another computer), but
exactly one of them is **in control**: only it may command this machine, and the
others stay paired in standby. The `future` CLI manages them:

```bash
future remote devices                  # every paired device, who is in control, issued by what
future remote devices add              # pair a new device: print its invitation
future remote devices remove <id|#>    # unpair this one; the others stay paired
future remote devices remove --all     # unpair every device
future remote devices use <id|#>       # choose the one in control
```

- A device is named by its pair id — the `pair_` prefix and letter case are
  decoration, and a unique prefix is enough — or by the row number the list prints.
- `remove` revokes the device where it was minted (the Future platform, or its
  room on a self-hosted relay) and only then drops the local slot. When a Desktop
  is running it is asked to perform the removal, which is what stops serving the
  device immediately; without one (or with a Desktop too old to know the request)
  the book is edited here and the device is served until its next credential
  refresh. A revoke that cannot be delivered is queued for the next Desktop to
  retry; the local unpair still happens.
- `use` writes the same pointer the window's "Let this control" writes, and with a
  Desktop running it also asks it to hand the machine over: control there is an
  in-memory decision (a device asking to take over moves it too), so a takeover is
  what makes the choice apply now rather than at the next start. The Desktop
  publishes the new controller, stands the previous one down and stores the same
  pointer. A device this Desktop has no bridge for cannot be handed the machine —
  there is no channel to send it commands — and the choice is still recorded and
  applies when the Desktop next starts serving. A device whose bridge is up but
  whose app is closed is handed the machine too, and the answer says that nothing
  will command it until it reconnects.
- `list` reads and `use` writes `~/.future/remote_pairings.json` (an older
  single-pairing file is read as one slot), so both work with no running Desktop
  and, on a self-hosted relay, no account.
- The list adds a `STATE` column beside the existing `CONTROL` one, and its
  headline gives the total plus one count per state (`2 entries in the pairing
  book: 1 paired / 1 unclaimed, expired`) so a row that cannot serve is never
  folded into a number that implies it can. The states are annotation only — no
  row is removed, and `control` keeps its meaning (the stored pointer): `paired`,
  `unclaimed` (an invitation no phone has scanned yet), `unclaimed, expired` (an
  invitation nobody ever claimed, past its deadline — it can neither be claimed nor
  connect, so nothing happens behind it until `remove` clears it, and `remove` does
  clear it on the platform: the invitation's row counts against the platform's
  per-desktop cap whether or not anyone scans it, so discarding it is a real
  deletion rather than bookkeeping), and `legacy` (a
  credential from before the v2 handshake, which cannot connect on this version).
  The `expiresAt` in a pairing is the invitation's deadline, not the pairing's: a
  device that claimed its invitation keeps working after that instant, because only
  the claim path checks it — which is why a `paired` row can carry a deadline in
  the past.
- `add` and `remove` (and the immediate half of `use`) are the exceptions, because
  the running process is what actually serves a device. An invitation is only
  usable while the bridge for that pairing is connected, and a removal only takes
  effect now if that bridge is stopped, so all three ask the running Desktop —
  window or host — and carry its answer back. `add` prints the invitation it
  returns as a QR code and link in the terminal that asked. Existing devices stay
  paired; this adds beside them.
  `remove --all` followed by `add` is how a machine's pairing is replaced, and the
  `add` stops any stale bridge left by the removal before minting the replacement.

"Connected right now" and the name a device gave itself exist only in a running
Desktop process — they are not in the pairing book. A machine with an empty book
stays unmatched until somebody runs `add`; that is also the recovery path after
`remove --all`.

### 3.2 The remote desktop switch

A machine serves its paired devices only while its remote desktop switch is on.
It is one preference, shared by the window's Remote page, `future host` and
the CLI, and it decides both "serve now" and "serve again at the next launch":

```bash
future remote status               # is it on now? what is the stored switch?
future remote enable               # turn it on now, and keep it on
future remote disable              # turn it off now, and keep it off
```

**Signing in comes first.** Without `--relay`, everything Remote does runs on your
Future OS account: the platform mints the invitation, the credential and the
relay connection. So `future remote enable` checks for a login before it touches
the switch and refuses with `future auth login` when there is none — a switch
that is on but unsigned-in serves nobody. `future host` itself does **not** need
an account: it starts signed out, serves everything local, and turns the remote
desktop on by itself the moment `future auth login` stores an account. `future
remote disable` needs no account: turning serving off is never blocked by being
unable to serve. With `--relay` there is no account at all, and none of this
applies.

- `status` prints the two halves under their own labels — the live state
  (`phase`, which devices are connected, what is in control) and the stored
  `autoConnectRemote` — because a machine can be "on" as a preference and "off"
  as a process. `--json` prints one document whose `state` is the word to test
  (`off`, `unknown`, `on`, `connecting`, `serving`, `degraded`, `failed`,
  `revoked`); see `future remote status --help`.
- With a Desktop running (window or host), `enable`/`disable` take effect at
  once: the request goes to that process, which starts or gracefully stops the
  bridge. Nothing is restarted, and a connected device is told the machine went
  away when the switch goes off.
- With none running there is no bridge to move, so only the switch is stored for
  the next start. `disable` exits 0 — off is the state that was asked for —
  while `enable` exits 1 and says nothing is serving yet.
- A host entry with the switch off starts (or stays) idle instead of exiting:
  it is the machine's control endpoint, so `future remote enable` turns it back on
  without a restart. A stop while the switch is *on* is still a fault and still
  ends the entry. "On and empty" is not a fault by itself — a book emptied by your
  own `remove` keeps the entry running, waiting to be asked for a new device — but
  a device the platform **revoked** is: the unusable credential is dropped, nothing
  is left to serve, and the entry stops naming `CredentialRevoked` instead of
  waiting for an invitation nobody asked for.
- Switching off keeps every pairing. `future remote devices remove` is what
  unpairs.

### 3.3 Driving another desktop (the client role)

The commands above manage devices paired *into* this machine. The mirror image is
a desktop this machine drives: pair it once from an invitation, and this Desktop
keeps a connection to it.

```bash
future remote peers                     # every remote desktop this machine drives
future remote peers add <invitation>    # pair with one: paste the link it shows
future remote peers connect <id|#>      # connect now
future remote peers disconnect <id|#>   # drop the connection, keep the pairing
future remote peers takeover <id|#>     # take it back from the device driving it
future remote peers rename <id|#> <n>   # name it here (--clear for the machine id)
future remote peers remove <id|#>       # unpair it: stop connecting, revoke
```

- `add` runs here rather than in the Desktop. A pairing is a one-time claim plus a
  handshake with the other machine, and nothing local has to be serving for it, so
  a machine can be paired before its Desktop is ever opened. Paste the invitation
  that machine shows for connecting another computer: it is single-use and
  short-lived. The pairing is stored in `~/.future/remote_peers.json` and the
  command connects once, so a broken pairing is reported now instead of at the
  next start.
- `list` reads that file and, with a Desktop running, the connections that process
  holds. Without one it reports the stored pairings and no connection state,
  because a connection lives in that process and nowhere on disk.
- `connect` and `disconnect` are asked of the running Desktop: that is where the
  connection and the user's own "disconnected" mark live, and asking it is what
  keeps the automatic reconnect pass from undoing a deliberate stop. `takeover`
  is asked of it for the same reason: one device drives a machine at a time, a
  standby machine stays connected and refuses every command (`superseded`), and
  `list` draws that as "taken over" (naming the holder when it said who it is).
  The command is the same `takeover` a phone sends; a machine this one already
  drove answers that nothing moved, which is a success (`tookOver: false` in
  `--json`). `remove` asks the Desktop too when it can, and otherwise unpairs
  here and revokes from the terminal.
- A paired desktop is connected by the Desktop as it starts. A host
  serves devices but does not drive other desktops, so the client role needs the
  window.

## 4. Ctrl+C, SSH disconnects and restarting

Ctrl+C works while waiting for pairing, connection startup and normal
operation. Exit first stops new remote operations and connection recovery, then
performs bounded resource cleanup.

- **Embedded Agent:** cancel all clients' active/queued work, including CLI/TUI
  conversations, then stop the service.
- **Independently running Agent:** leave it running. This Desktop's remote entry
  still closes.
- **Saved login and completed pairing:** retain them for the next `future host`
  launch. Exiting is not unpairing.

Unix also handles SIGTERM and SIGHUP. Continuity after SSH disconnect is not
promised. If you deliberately need it, use tmux or a service manager; this is not
the default behavior of host mode.

## 5. Server build without graphical dependencies

The unified CLI calls the GUI-free `future-host` library directly. It does not
load Tauri, GTK or WebKit and needs no X11/Wayland session. Official Linux CLI
artifacts use static musl; local builds use the host toolchain and may be dynamic.

Build and install from the repository root without npm or Desktop:

```bash
make build-cli
make install-cli
make run-host ARGS=--help
# Compatibility aliases: make build-desktop-headless / install-desktop-headless
```

Or build explicitly:

```bash
cargo build --release -p future-cli --bin future
./target/release/future host
```

The output is `target/release/future` (`future.exe` on Windows). No separate
`futureos-headless` binary is required. Existing Future home paths and paired
credentials stay in place. See [Build & Install](build-and-install.md).

`--release` selects compiler optimization, not the production/test platform
channel. Keep the host and mobile environment consistent; do not edit invitations.

The process fixtures run the actual CLI:

```bash
cargo test -p future-cli --test headless_cli --test remote_start_refusal
```

On native Linux with Docker, verify a released static artifact in a networkless
Rocky Linux 8 container, including first noninteractive startup refusal:

```bash
bash scripts/ci/check-headless-linux.sh /path/to/future
```

The verifier rejects dynamic builds. Release CI applies it to CLI-only, deb and
portable payloads. See [Desktop Agent lifecycle](desktop-agent.md) for ownership,
logs and recovery.

## 6. Data, permissions and troubleshooting

Host mode uses the running user's existing directories, including
`~/.future/agent/auth.json`, `~/.future/remote_pairings.json` and `~/.future/app/`.
Being signed in on your laptop does not sign in the SSH server. Changing the
system user or HOME does not automatically inherit credentials. For later service
hosting, use the same user and directories as the initial interactive setup.

Startup names its own log file at launch and repeats the path when it exits with
an error. Desktop and its embedded Agent write to
`<root>/app/logs/desktop.log` (see
[Desktop Agent lifecycle and recovery](desktop-agent.md)), and the Agent logs its
startup phase by phase — model catalogue, database, legacy sessions, endpoint — so
a slow first launch on cold or network-mounted storage is distinguishable from a
hang.

| Symptom | Action |
|---|---|
| `libwebkit2gtk-4.1.so.0` missing | This is the GUI binary; build and run `future host` instead |
| `--headless` option removed or unknown | Use `future host` without that flag; `--re-pair`/`--no-qr` are gone — use `future remote devices add` and `remove --all` — while `--relay`/`--relay-ca` exist on both entry points |
| Agent unavailable/incompatible or instance lock occupied | Read the reported owner/version, check the same user/data root, and upgrade or exit the external owner; never delete the lock or bypass it. Explicit endpoints and external mode are attach-only |
| Built-in skills were not installed | Check the `[skills]` lines on stderr: maintenance runs this same executable as `future init` and needs a network route to the skill catalogue. The phone can still install skills itself |
| Interactive terminal required | Sign in from a real terminal, without pipes, redirection or non-interactive service startup; pairing a device needs no terminal |
| Platform authorization denied or expired | Sign in again with `future auth login`; a running host adopts it, or reports `Platform account signed in` and restarts serving. The login code is not the app pairing code |
| Pairing invitation expired | Run `future remote devices add` for a new one; completed pairings do not require this |
| `future remote devices add` times out | The running Desktop is asked, and it answers from *its own* build. If the message says nothing picked the request up, that process predates device requests — restart it with the current version (`futureos` or `future host`) and try again |
| Another Desktop owns the directory | Exit its GUI or host process; do not delete the lock file to bypass mutual exclusion |
| The process disappears, or a node operator reports it was killed | A host OOM killer or a login node's per-user CPU/memory budget sends SIGKILL, which no process can report itself. Read `<root>/app/logs/desktop.log`: the last startup phase it recorded shows where it stopped; then check the host (`dmesg -T`, `journalctl -k`, or the node's own quota/job logs). If the legacy import is what trips the budget, move the data root off shared storage with `FUTURE_HOME` |
| Phone cannot claim an invitation | Check matching production/test environments and that the invitation is unused and unexpired |
| Platform logout, account switch or refresh authorization rejected | The entry closes or reports an actionable error; restore the correct login and restart |

### Started from tmux or an SSH login: no window

`futureos` starts only a graphical window, so it needs a session that can
register one. A `tmux` server — and every pane it creates — keeps the launchd
session it was started in, and a server first started by an SSH login gives its
panes an SSH session rather than the desktop one.

Measured on macOS 26.6 with an SSH-rooted server: the window *is* created at the
right size and position and reports `kCGWindowIsOnscreen`, but the process never
appears in `lsappinfo` — no Dock icon, no ⌘-Tab entry — and nothing can raise it
above the app already on screen. That reads as "no window opened". A `tmux`
server started from a desktop terminal does not have this problem: the same
binary there registers the window normally and it can be raised. Note that
`launchctl managername` prints `Background` in both cases and
`CGSessionCopyCurrentDictionary()` is non-null in both, so neither one tells them
apart.

Ways out, cheapest first:

1. Give tmux a server of its own, started from a desktop terminal:
   `tmux -L gui`, then `tmux -L gui attach` to use it. Its panes register windows
   normally, and the old server keeps running for plain command-line work.
2. Let the desktop start the app: `open -a FutureOS`. LaunchServices starts it in
   the desktop session; rebuild the bundle with `make package-desktop` first if it
   is stale. `open` does not forward the environment, so `FUTURE_HOME` does not
   carry over — use option 1 for an isolated data root.
3. Restart the shared server from a desktop terminal: `tmux kill-server`, then
   start it again there. This drops every session the old server held.

A silent return is worth recognising too: while an instance is already running, a
second `futureos` — from any shell — only asks the running one to come to the
front and exits 0 after about a second, printing nothing.

Phone-triggered tools run on the host under the existing approval and file-access
rules. Host mode does not grant extra permissions or provide additional
sandbox isolation. See [Phone remote access](../wiki/en/Remote.md) for transport and
relay trust boundaries and [Sandbox](../wiki/en/Sandbox.md) for permissions.
