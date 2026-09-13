# Headless Desktop guide

[中文](desktop-headless.zh-CN.md)

Headless mode lets FutureOS Desktop provide phone remote access from a terminal,
without opening a window or WebView. It works over SSH and on local machines that
do not need the desktop UI. It reuses Desktop's sessions, workspaces, approvals and
file handling; it is not a separate Remote service or SDK.

**The process stays in the foreground. Press Ctrl+C to close remote access and exit.**
It does not daemonize or install a system service.

## 1. Before you start

- Install or build a Desktop version supporting `--headless` and a matching
  `future` CLI. Put the CLI beside the Desktop executable or on PATH so Desktop
  can start the local Agent when needed. Alternatively, run `future agent`
  independently first; Desktop connects without starting a duplicate.
- Install a FutureOS mobile app matching the host's production/test environment.
- Allow the host to reach the Future OS platform and configured NATS relay. The
  phone also needs network access.
- Run as an ordinary system user. Platform credentials, pairing and tool execution
  belong to that user; do not use root just for convenience.
- Only one Desktop backend may own a data directory. Close the GUI using that
  directory before starting headless mode.

Platform access requires valid Future OS account credentials, but **no browser is
needed on the server**. Initial login and pairing require an interactive terminal;
do not redirect the output to a file.

## 2. Start in the foreground

Change to the directory containing the Desktop executable.

Linux / macOS:

```bash
./futureos --headless
```

Windows PowerShell:

```powershell
.\futureos.exe --headless
```

No additional `--pair --qr` options are needed. Desktop guides you according to the
saved login and pairing state. Pairing does not end the command or return a shell
prompt: the process continues serving the phone.

### Step one: platform authorization

Without a valid login, the terminal displays **PLATFORM LOGIN**, a QR code, an
authorization URL and a user code.

1. Scan with your phone camera/browser, or open the URL in a browser on your own
   computer or phone.
2. Sign in to Future OS and authorize this server. Enter the terminal's user code
   if the web page requests it.
3. The server waits for authorization, saves credentials through the Agent and
   proceeds to phone pairing.

This authorizes the server to access your platform account. It does not yet allow
the mobile app to control the server. No server browser, SSH graphical forwarding
or inbound browser callback port is required.

### Step two: mobile app pairing

If no pairing exists, once the remote entry and Agent are ready, the terminal
shows **PHONE PAIRING**, a second QR code and a complete
`futureos://remote/pair?...` invitation link.

- Scan with **Add device / Scan inside the FutureOS app**.
- Alternatively, paste the complete link into the app's **Paste pairing code**
  entry.

The pairing code is the complete invitation link, not a short numeric code.
Invitations are valid for five minutes and are single-use. Do not share them,
post screenshots or save them in shared logs.

### Already signed in or paired

Valid login and pairing are reused, skipping the corresponding steps. The paired
phone can reconnect without scanning every time. A ready entry or saved pairing
does not prove the phone is currently online: open the app and connect to the host.
Ordinary network outages do not deliberately replace valid pairings; the existing
Remote connection logic handles recovery.

## 3. Startup options

| Option | Behavior |
|---|---|
| `--headless` | Explicitly open phone remote access without a UI, in the foreground |
| `--no-qr` | With `--headless`, print authorization/pairing links without drawing QR codes |
| `--re-pair` | With `--headless`, explicitly revoke the saved phone pairing and create a new invitation |
| `--help` | Show help without starting Desktop or Agent |

For example:

```bash
./futureos --headless --no-qr
./futureos --headless --re-pair
```

`--re-pair` affects the previous phone binding. Use it to change phones or
intentionally replace a pairing, not to troubleshoot an ordinary network outage.
Narrow terminals automatically fall back to text links; copy the link, or widen
the terminal and restart.

## 4. Ctrl+C, SSH disconnects and restarting

Ctrl+C works while waiting for login, pairing, connection startup and normal
operation. Exit first stops new remote operations and connection recovery, then
performs bounded resource cleanup.

- **Agent started by this Desktop:** cancel active conversations and stop the
  Agent. In-progress tasks are interrupted.
- **Independently running Agent:** leave it running. This Desktop's remote entry
  still closes.
- **Saved login and completed pairing:** retain them for the next `--headless`
  launch. Exiting is not unpairing.

Unix also handles SIGTERM and SIGHUP. Continuity after SSH disconnect is not
promised. If you deliberately need it, use tmux or a service manager; this is not
the default behavior of headless mode.

## 5. Server build without graphical dependencies

The default Desktop build supports `--headless` without creating a UI at runtime,
but its executable still links GUI system libraries. For a Linux server without
GTK/WebKit, build the same backend without GUI support from the repository root:

```bash
cargo build --release --no-default-features --manifest-path desktop/src-tauri/Cargo.toml
```

The default output is `desktop/src-tauri/target/release/futureos`, with `.exe` on
Windows. If `CARGO_TARGET_DIR` is set, use that output directory instead. This build
needs no Tauri, GTK, WebKit or X11/Wayland session. It contains no GUI and requires
explicit headless startup.

A matching `future` CLI is still required to provide the Agent. Build it from the
repository root with `cargo build --release -p future-cli`, then put the CLI output
beside Desktop or on PATH. See [Build & Install](build-and-install.md) for general
build requirements.

`--release` selects compiler optimization, not the production/test platform
channel. The existing project version and environment policy still applies. Do
not edit pairing links to bypass the mobile app's environment checks.

## 6. Data, permissions and troubleshooting

Headless mode uses the running user's existing directories, including
`~/.future/agent/auth.json`, `~/.future/remote_pairing.json` and `~/.future/app/`.
Being signed in on your laptop does not sign in the SSH server. Changing the
system user or HOME does not automatically inherit credentials. For later service
hosting, use the same user and directories as the initial interactive setup.

| Symptom | Action |
|---|---|
| Agent not found | Place a matching `future` CLI or start the Agent independently; an unreachable explicitly configured Agent endpoint is not automatically taken over |
| Interactive terminal required | Complete initial login/pairing in a real terminal, without pipes, redirection or non-interactive service startup |
| Platform authorization denied or expired | Restart and sign in as instructed; the login code is not the app pairing code |
| Pairing invitation expired | Restart to request an invitation; completed pairings do not require this |
| Another Desktop owns the directory | Exit its GUI or headless process; do not delete the lock file to bypass mutual exclusion |
| Phone cannot claim an invitation | Check matching production/test environments and that the invitation is unused and unexpired |
| Platform logout, account switch or refresh authorization rejected | The entry closes or reports an actionable error; restore the correct login and restart |

Phone-triggered tools run on the host under the existing approval and file-access
rules. Headless mode does not grant extra permissions or provide additional
sandbox isolation. See [Phone remote access](wiki/en/Remote.md) for transport and
relay trust boundaries and [Sandbox](wiki/en/Sandbox.md) for permissions.
