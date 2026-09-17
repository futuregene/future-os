# Headless Desktop guide

[中文](desktop-headless.zh-CN.md)

Headless mode lets FutureOS Desktop provide phone remote access from a terminal,
without opening a window or WebView. It works over SSH and on local machines that
do not need the desktop UI. It reuses Desktop's sessions, workspaces, approvals and
file handling; it is not a separate Remote service or SDK.

**The process stays in the foreground. Press Ctrl+C to close remote access and exit.**
It does not daemonize or install a system service.

## 1. Before you start

- Build `futureos-headless` and a matching `future` CLI (section 5). The standalone
  entrypoint needs no GTK/WebKit and starts headless by default. Current release
  workflows do not produce this binary or a separate headless archive; the GUI
  portable and CLI-only packages do not provide this standalone entrypoint.
- Put the CLI beside `futureos-headless` or on PATH so it can start the local Agent
  when needed. You can also run `future agent` independently first; the headless
  backend connects without starting a duplicate.
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

Change to the directory containing the built `futureos-headless` executable.

Linux / macOS:

```bash
./futureos-headless
```

Windows PowerShell:

```powershell
.\futureos-headless.exe
```

No `--headless` flag is needed or accepted. `futureos` starts only the graphical
Desktop; its former `--headless` option has been removed. Replace old commands
such as `futureos --headless --no-qr` with `futureos-headless --no-qr`.
Both executables share the backend, data and phone protocol; this is not a second
implementation.

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
| `--no-qr` | Print authorization/pairing links without drawing QR codes |
| `--re-pair` | Explicitly revoke the saved phone pairing and create a new invitation |
| `--help` | Show help without starting Desktop or Agent |

For example:

```bash
./futureos-headless --no-qr
./futureos-headless --re-pair
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
- **Saved login and completed pairing:** retain them for the next `futureos-headless`
  launch. Exiting is not unpairing.

Unix also handles SIGTERM and SIGHUP. Continuity after SSH disconnect is not
promised. If you deliberately need it, use tmux or a service manager; this is not
the default behavior of headless mode.

## 5. Server build without graphical dependencies

The graphical `futureos` binary links GUI libraries, which the Linux loader needs
**before** parsing arguments. The standalone `futureos-headless` build excludes
Tauri/GTK/WebKit and needs no X11/Wayland session. Its other runtime requirements
(such as glibc) depend on the build target and environment; source builds are not
necessarily fully static. No release workflow changes are required for this
source-build path.

To build both binaries from the repository root without npm/Tauri packaging:

```bash
make build-desktop-headless
```

Or build explicitly:

```bash
cargo build --release --no-default-features --features headless \
  --bin futureos-headless --manifest-path desktop/src-tauri/Cargo.toml
cargo build --release -p future-cli
```

The default output is `desktop/src-tauri/target/release/futureos-headless`, with
`.exe` on Windows. Put `target/release/future` (Windows: `future.exe`) beside it or
on PATH; the Make target copies it beside the server automatically. If using
custom Cargo output directories, build and copy the outputs explicitly.
The `headless` feature selects the standalone entrypoint; combining it with the
`gui` feature is rejected rather than producing a misleading GUI-linked server.
See [Build & Install](build-and-install.md) for general build requirements.

`--release` selects compiler optimization, not the production/test platform
channel. The existing project version and environment policy still applies. Do
not edit pairing links to bypass the mobile app's environment checks.

For local validation, run the standalone CLI/lifecycle tests with:

```bash
cargo test --no-default-features --features headless \
  --manifest-path desktop/src-tauri/Cargo.toml --test headless_cli
```

On native Linux with Docker running, you can also manually check the ELF
libraries and startup in a clean Ubuntu 24.04 container (no GUI stack):

```bash
bash scripts/ci/check-headless-linux.sh desktop/src-tauri/target/release/futureos-headless
```

Use a build compatible with Ubuntu 24.04 for this check. It is a manual verifier,
not a step in the current CI or release workflows.

## 6. Data, permissions and troubleshooting

Headless mode uses the running user's existing directories, including
`~/.future/agent/auth.json`, `~/.future/remote_pairing.json` and `~/.future/app/`.
Being signed in on your laptop does not sign in the SSH server. Changing the
system user or HOME does not automatically inherit credentials. For later service
hosting, use the same user and directories as the initial interactive setup.

| Symptom | Action |
|---|---|
| `libwebkit2gtk-4.1.so.0` missing | This is the GUI binary; build and run `futureos-headless` instead |
| `--headless` option removed or unknown | Use `futureos-headless` without that flag; `--no-qr` and `--re-pair` belong to the standalone entrypoint |
| Agent not found | Place a matching `future` CLI or start the Agent independently; an unreachable explicitly configured Agent endpoint is not automatically taken over |
| Interactive terminal required | Complete initial login/pairing in a real terminal, without pipes, redirection or non-interactive service startup |
| Platform authorization denied or expired | Restart and sign in as instructed; the login code is not the app pairing code |
| Pairing invitation expired | Restart to request an invitation; completed pairings do not require this |
| Another Desktop owns the directory | Exit its GUI or headless process; do not delete the lock file to bypass mutual exclusion |
| Phone cannot claim an invitation | Check matching production/test environments and that the invitation is unused and unexpired |
| Platform logout, account switch or refresh authorization rejected | The entry closes or reports an actionable error; restore the correct login and restart |

Phone-triggered tools run on the host under the existing approval and file-access
rules. Headless mode does not grant extra permissions or provide additional
sandbox isolation. See [Phone remote access](../wiki/en/Remote.md) for transport and
relay trust boundaries and [Sandbox](../wiki/en/Sandbox.md) for permissions.
