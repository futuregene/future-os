# Embedded terminal

Status: implemented for **Linux** (verified by automated tests); macOS/Windows
paths exist but are **not verified** on this machine — see §Platforms.

The terminal is a PTY registry owned by the host process and served two ways: to
the app's own webview over a **loopback-only HTTP/WebSocket listener**, and to a
paired device over the remote control plane (§[Remote](#remote)). The registry
lives in `future-host`, so `future host` — a machine with no window at all —
serves the same shells. It is deliberately not a Tauri-IPC message pump.

## Why a socket

The design is opencode's, and the reason to copy it is not fashion:

* **Resume is one number.** Output is a byte stream with an absolute cursor. A
  view that reconnects (panel reopened, webview reloaded, tab remounted) asks
  for the bytes after the cursor it already applied, so nothing re-renders and
  nothing is lost.
* **The renderer keeps its own screen.** xterm's serialized buffer is persisted
  per tab, so the screen survives even when the server's bounded tail has moved
  past that client.
* **It is testable without a GUI.** `cargo test terminal::server` drives the
  real listener over real TCP and streams a real shell over a real WebSocket.
  The earlier IPC design could only be checked by a human clicking through the
  app.

## Layout

```
desktop/src-tauri/src/terminal/
  session.rs    PTY child + bounded output tail + absolute cursor + viewers
  manager.rs    registry, capacity limits, exit retention, conversation teardown
  pty.rs        portable-pty boundary; session-scoped process-tree teardown
  protocol.rs   wire helpers: control frame, replay chunking, input decoding
  server.rs     loopback listener: control routes, ticket issue, WebSocket pump
  ticket.rs     single-use, session-scoped, 60 s connect tickets
  cwd.rs        working-directory resolution (thread → workspace → home)
  shell.rs      default shell + shell list
desktop/src/features/terminal/
  client.ts         control routes; typed errors with the server's code
  tabs.ts           per-conversation tab state (persisted view + labels)
  useTerminalTabs.ts  reconciliation with the server, create/close/restart
  useTerminalPanel.ts open/height preferences + the global shortcut
  TerminalView.tsx  xterm + WebSocket (lazily loaded chunk)
  TerminalPanel.tsx tab strip, notices, exits
  TerminalToggleButton.tsx header affordance
```

## Wire protocol

Control routes are JSON over HTTP; output is a WebSocket.

| Route | Purpose |
| --- | --- |
| `GET /terminal/shells` | shells the client may offer |
| `GET /terminal?threadId=` | sessions for a conversation (running + retained exits) |
| `POST /terminal` | create `{threadId, title?, cols?, rows?}` |
| `GET /terminal/:id` | one session |
| `PATCH /terminal/:id` | `{title?, cols?, rows?}` |
| `DELETE /terminal/:id` | terminate the tree and forget the session |
| `POST /terminal/:id/connect-token` | issue a single-use connect ticket |
| `GET /terminal/:id/connect?cursor=&ticket=` | WebSocket upgrade |

On the socket:

* **server → client**: binary frames, each one type byte plus its payload:
  `0x01` + raw PTY bytes (forwarded verbatim, leading NUL included), or `0x00` +
  UTF-8 JSON `{cursor, start, exitCode?}` for the control frame;
* **client → server**: binary or text frames, decoded as UTF-8 (invalid UTF-8 is
  dropped, never replaced — a broken byte must not be typed into a shell).

`cursor` is the absolute end offset to resume from. `start` is the offset of the
first replayed byte: `start > requested` means the retained tail no longer
covers what the client asked for, and the view says so instead of showing a
hole. Close code `1000` means the shell exited; `4408` means the viewer fell
behind and must re-attach.

Buffers: the session retains 2 MiB of output; a viewer that has not activated
yet may accumulate 1 MiB before it is cut loose with `4408` (it re-attaches and
replays). A detached viewer never grows memory without bound.

## Security model

* Bound to `127.0.0.1` on an **ephemeral port**; never reachable from the LAN.
* A 32-byte random secret per process, handed to the webview through the single
  Tauri command `terminal_server_info`; required on every control route.
* The WebSocket cannot carry a header, so it redeems a **single-use,
  session-scoped, 60-second ticket** issued over an authenticated route.
* Requests from an origin that is not the app itself (or a loopback dev server)
  are refused even with a ticket. Origin checks are defence in depth: the secret
  and the ticket are the real gates.
* The listener serves one request per connection and caps concurrent
  connections; it accepts no keep-alive and no pipelining.
* Terminal output never reaches the agent, the RPC bridge, logs or SQLite. The
  child environment is the user's own environment minus the app's private
  plumbing (`FUTURE_AGENT_GRPC_ADDR`).
* A paired desktop may drive these shells, and only inside the sealed remote
  envelope (§[Remote](#remote)). Nothing about that path is a second transport:
  it is the same registry, the same stable error codes, and the same routing to
  the shell's own buffered tail.

## Lifecycle

* Sessions are **conversation-scoped**: `threadId` is required to create one, and
  deleting a conversation (or its workspace) closes its terminals. No shell can
  outlive the context it was opened in.
* App exit tears every session down with a bounded grace period.
* An exited shell stays addressable (final screen, exit code) until the tab is
  closed or 25 later sessions have exited.
* The panel auto-creates a tab only when it is opened with none. Every other
  shell is created by an explicit user action.
* Collapsing the panel **hides** it; it never closes anything. The shell keeps
  running, the tab keeps its serialized screen, and reopening reattaches to the
  same session. Only closing a tab (or deleting the conversation) ends a shell.
  Collapsing also hands the caret back to the composer: the panel's controller
  emits `futureos:focus-composer` and the Composer answers it (declining while
  it cannot hold a caret). Otherwise focus would sit on an unmounted terminal
  and the next keystrokes would go nowhere.
* The shortcut (`Ctrl+J`, `⌘J` on macOS, rebindable in Settings → Keyboard
  shortcuts) belongs to the app, not to the shell. `TerminalView` releases it from
  xterm's key handling so the window listener still sees it while the terminal has
  focus — without that, xterm would cancel the event and send a line feed to the
  shell instead. The binding is the command table's (`terminal.toggle`); the
  release predicate lives in `features/terminal/shortcut.ts`, and this is the
  **only** command the terminal lets through — every other shortcut is ignored
  there so `Ctrl+B`, `Ctrl+F`, `Ctrl+A` and `Ctrl+K` keep reaching readline. See
  [KEYBINDINGS.md](KEYBINDINGS.md).
* **The IME owns the keyboard while it is composing.** The same custom key
  handler also stands down for every event flagged `isComposing`, so a composing
  keystroke reaches the input method instead of the terminal. xterm's own
  composition heuristic only knows Chromium's convention for "this key belongs
  to the IME" (`keyCode === 229`); WebKit — the engine the desktop webview uses
  on Linux — reports consumed keys with their real keyCodes (a composing
  Backspace arrives as `keyCode 0`). On such a key xterm ran
  `_finalizeComposition(false)`, which commits whatever the textarea holds at
  that instant, so *typing pinyin, pressing Backspace to fix a letter and then
  committing sent the Chinese text to the shell twice* — the command line kept a
  leftover copy that deleting could not clear. The policy and the captured
  WebKit sequences are in `features/terminal/keyPolicy.ts`; the IME preedit box
  is painted with the terminal's own palette in `styles/globals.css` (xterm ships
  a black-on-white dark-theme default, and this app is light-only).
  Two measured details worth knowing: pressing a key with a modifier during a
  composition (e.g. `Ctrl+J`) makes the IME **cancel** the pending preedit, and
  the browser reports that key as `key: "Unidentified"` — so the panel shortcut
  cannot be recognised mid-composition and needs a second press (nothing leaks to
  the shell either way); and candidates are chosen/confirmed with Space or a
  digit, while a synthesised `Enter` makes this IME commit the raw pinyin
  (`nihao`) — verified to be the input method's own choice, since a plain
  `<input>` behaves the same.

### macOS WebKit overlapping input

With an IME input source, macOS WebKit can deliver committed `beforeinput` /
`input` **before** the character's `keydown`. When the previous key (or Shift /
Caps Lock) is still held, xterm 6.0's `_keyDownSeen` flag incorrectly rejects the
text as a duplicate. The later IME-managed keydown does not recover it, so fast
input such as `htop` can become `htp` (upstream
[xterm.js#5374](https://github.com/xtermjs/xterm.js/issues/5374)).

`features/terminal/macInput.ts` installs a macOS-WebKit-only `beforeinput`
workaround after `terminal.open()`, resetting that private guard for composed
`insertText` outside IME composition and screen-reader mode. xterm still sends
the text via its usual `onData` path; keypress deduplication, composition, and
other platforms are unchanged. The listener is removed when the view unmounts.
This isolated private-API workaround is tied to the pinned xterm version;
recheck it on upgrades. `macInput.test.ts` replays the upstream event ordering
through **real xterm in jsdom**, including an unpatched reproduction, overlapping
keys, Shift/Caps Lock, conventional input, Chinese composition and teardown.
This is event-sequence regression coverage, not native macOS IME verification.

## Working directory

Resolved server-side from the conversation; the client never sends a path.

1. the workspace strictly associated with the thread (`workspace_id`, so a
   chat's temporary workspace and a real workspace can never be confused) —
   for a standalone chat session that temporary workspace **is** the session's
   own directory;
2. the user's home directory, whenever the configured directory is missing or
   otherwise unusable.

The configured directory is validated before spawn and the fallback to home is
automatic (no confirmation step): a shell must land somewhere predictable, the
skipped path is logged, and the resolved directory travels back in the session
info. Validation itself stays because `portable-pty` silently substitutes
`$HOME` for a bad cwd (verified in the earlier spike) — here the choice is made
deliberately instead. Only an unusable home directory still errors
(`CWD_INVALID`).

## Shell resolution

`resolve_shell()` prefers the user's real shell and never substitutes one
silently: the account login shell (or `$SHELL`) first, then the well-known
fallbacks. On Windows every candidate is resolved to an **absolute path that
exists** before it is offered — `PATH` in the OS's own order, then the
well-known install locations (`%ProgramFiles%\PowerShell\7\pwsh.exe`,
`%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`,
`%SystemRoot%\System32\cmd.exe`).

That resolution is load-bearing, not cosmetic. `portable-pty` hands the program
to `CreateProcessW` as `lpApplicationName`, and Win32 does **not** search `PATH`
for that parameter: a bare `pwsh.exe` fails with `os error 2` ("The system
cannot find the file specified") even when PowerShell is installed. Machines
whose PowerShell 7 lives off `PATH` therefore failed every terminal tab with
`terminal.json`'s `createFailed` ("无法启动终端"). A shell that cannot be
located now falls through to the next candidate instead of being spawned.
`terminal::shell::windows_tests` pins the rules (`PATH` order, extension
appending, quoted `PATH` entries, well-known locations as last resort).

## Process teardown

A shell with job control puts background jobs in their **own** process groups,
so `killpg(shell)` alone leaves `sleep 300 &` running. Teardown therefore walks
the *session*: SIGTERM to the group, bounded wait, SIGKILL, then a sweep of
session members (Linux: `/proc`; other unix: `ps -o sess`). Windows uses
`taskkill /T /F`. This is a regression test, not a theory —
`terminal::pty::tests::teardown_reaches_background_jobs`.

## Platforms

| Platform | Spawn/IO | Teardown | GUI end-to-end |
| --- | --- | --- | --- |
| Linux | verified | verified (session sweep) | **not run** in this environment |
| macOS | not run | not run (`ps -o sess` path) | not run |
| Windows | not run | not run (`taskkill /T`) | not run |

The `GUI end-to-end` column means a packaged GUI build exercised end to end; it
is not keyboard or input-method coverage — that is the paragraph below.

Nothing above claims otherwise. A Job Object on Windows and `proc_listchildpids`
on macOS would be strictly better than the current best-effort paths.

The keyboard **is** covered on Linux beyond the app's own test suite: the real
`TerminalView` was driven in the system's WebKitGTK (the same engine the Tauri
webview uses here) against a real PTY running the user's login shell, with
**ibus-libpinyin** as the input method, through `WebKitWebDriver` — the IME
preedit, the commit, deleting during a composition and the panel shortcut all
observed as real events. That is how the composing-Backspace behaviour above was
found and fixed. macOS and Windows input methods are still **not run**; the
policy relies only on `KeyboardEvent.isComposing`, which every engine sets.

## Manual verification (5 minutes, GUI)

1. `npm run tauri:dev` (or run a packaged build), open a conversation.
2. Press **Ctrl+J** (⌘J on macOS) or click the terminal button in the header.
3. Type `pwd` — it must print the conversation's workspace directory.
4. `sleep 300 &` then close the tab; `pgrep -f "sleep 300"` must be empty.
5. Press **Ctrl+J** (⌘J) while the terminal itself has focus: the panel must
   collapse (it must not send a line feed to the shell) and the caret must land
   back in the message box. Press it again, or click the ✕ in the panel header:
   the screen and scrollback are still there, the shell is the same process, and
   a `cd` you made survives.
6. Reload the webview (⌘R / Ctrl+R): the panel restores the same screen and the
   shell keeps running (no new shell is spawned).
7. `exit` in the shell: the tab shows the exit code and offers a restart.
8. Type Chinese through your IME, correct a letter with Backspace, then commit:
   the characters must appear **once**, and deleting them must leave a clean
   line (no leftover copy, no stray backspace). The preedit must render in the
   terminal's own colours, not in a black box.
9. On macOS with a Chinese/Japanese input source in ASCII mode, type `htop`
   quickly with overlapping key presses (press the next key before releasing
   the previous one). Every character must arrive once. Also try the first
   Shift+3 character and a Caps Lock letter, then switch back to Chinese
   composition and repeat step 8.

## Remote

A paired desktop drives the *same* shells the person at this machine sees:
one registry, one conversation scope, one teardown rule (deleting a conversation
closes its shells, wherever the delete came from).

| Layer | Where |
| --- | --- |
| The registry | `host/src/terminal/` (`manager`, `session`, `pty`, `shell`, `cwd`) |
| It, as the control plane sees it | `host/src/terminal/service.rs`, trait + slot in `host/src/terminal_service.rs` |
| Installed by | `future_host::terminal::install_service()` — the window and `future host` alike |
| The wire commands | `host/src/remote_host/business/terminal.rs` |
| The capability | `future_remote_protocol::TERMINAL_FEATURE` (`terminal_v1`) |
| The client half | `desktop/src/features/terminal/remoteTerminal.ts` |
| The transport seam | `desktop/src/features/terminal/transport.ts` |

**Advertised, not assumed.** The handshake lists `terminal_v1` only from a
process that installed a registry (`services::set_terminal_available`), and both
entry points do: `AppState::initialize_process_state` in the window, and
`headless::session` before the remote bridge starts. A paired desktop shows the
panel only on a host that advertised it, so a build that never installed a
registry shows nothing rather than a button whose every press answers
`Unsupported command`.

**It is a paged pull, not a stream.** Commands travel the ordinary control-plane
lane: `terminal_list` / `terminal_create` / `terminal_update` / `terminal_delete`
/ `terminal_read` / `terminal_write`. Bytes travel as **URL-safe, unpadded
base64** in `terminal_data` — the host's `URL_SAFE_NO_PAD`, not the browser's
`btoa` alphabet: `-_8` is how `[0xFB, 0xFF]` is spelled, and both ends pin that
literal in a test because a mismatch here is invisible until it is not (padded
`bA==` is refused by the host, and `atob` throws on `-_`).

`terminal_read` returns the bytes *after* the cursor the client applied, plus
the cursor to ask from next time — the same absolute-offset contract the
loopback attachment has, so a client that reconnects asks for the
difference and is told when the retained tail no longer covers it
(`start > cursor`). A page is capped at 64 KiB and a write at 256 KiB.

**Order is a correctness property, not a nicety.** Commands are dispatched
concurrently on the host (one slow command must not block the rest), so
keystrokes could otherwise reach the shell out of order. The client therefore
keeps **one write in flight at a time**: input is batched for ~10 ms, sent, and
the next batch waits for the ack. That is why there are no interleaved writes to
merge — and why the cost below is what it is.

**What a paired device may do.** Anything the trait allows: spawn a shell, type
into it, resize it, kill it. That is deliberately wider than the agent's own
surface — a prompt passes through approval rules, a shell does not. The gate
that stays is *control*: `commands.rs` refuses every business command from a
standby device before a handler sees it, so only the device the user handed the
machine to can open or type into a shell.

**A headless host is a first-class target.** `future host` installs the registry
before it starts serving, so a server box reached from a paired desktop has real
shells in the same conversation directories the agent works in, and its shutdown
path freezes the registry and tears the children down with the rest of the
services (`cli/src/headless/mod.rs::stop`). The working directory still resolves
from the *store*, which is the same `~/.future` store the window uses.

**Cost, stated plainly.** Output costs one round trip per page (polled at 16 ms
while a shell is producing and 250 ms while it is idle — the client only polls
while its panel is open), and a keystroke costs one round trip before the shell
sees it and another before its echo arrives. On a relay that is a "watch the
build" terminal: fine for `make`, `git log`, `tail -f`; not `vim` at 200 ms.
A push lane (`p.{pair}.term.{id}` with the same cursor contract) is the upgrade
this design leaves room for — the transport seam is where it lands — and it is
not needed to make the feature correct.

### The panel is missing? Ask the host what it advertises

The panel exists only while the host's handshake lists `terminal_v1`, and a host
that does not is deliberately given no control at all — so "this machine serves
no shells" and "the window is broken" look exactly alike. Nothing in the window
says which it is, so ask the host:

```bash
cargo run -p future-host --example peer_features
desktop_AZYJSK5RT6PC5BQZ connected=true terminal=true features=["e2ee_v2", …, "terminal_v1"]
```

`terminal=false` is the host's own answer that it will not serve a shell, and
`features` is the rest of the list — the same list every other gated control
reads. The probe opens a client connection of its own (a host serves one live
session per pairing), so a window already connected to that host reconnects
afterwards; close the window first when the printed list has to be the only
connection.

Until 2026-11-11 that answer was `terminal=false` on **every** host, whatever
its build. The secure handshake (`secure_open` / `secure_finish`) — the only
path a current client takes — sealed `remote-protocol`'s own `SERVER_FEATURES`
literal, while the list that grows with each feature lived in
`remote-server::advertised_features`, which only the legacy
`pair_handshake_confirm` reply used. `terminal_v1` was added on 2026-11-04 and
never reached a paired desktop. Both paths now build one list, in
`remote-protocol`, and the confirmation takes it from the process that serves
the shells (`remote-server/src/secure.rs`); a test asserts the sealed body
equals what the process advertises, so the two cannot drift apart again.

`future remote peers list` still prints connection state rather than
capabilities; the probe above is the answer, and its output is what a support
note should quote.

## Differences from opencode

Same architecture; the deliberate deviations, so a future reader does not treat
them as accidents:

| Area | opencode | here | Why |
| --- | --- | --- | --- |
| Renderer | `ghostty-web` (libghostty WASM) | `@xterm/xterm` | The repo's dependency policy keeps WASM blobs out of the desktop bundle; xterm has no runtime dependencies and is already the app-facing contract here. The terminal component is renderer-agnostic apart from its imports. |
| Cursor unit | UTF-16 string length | bytes | The PTY produces bytes; counting decoded characters drifts on any non-ASCII output. |
| Scope key | directory | `threadId` → workspace | future-os has no worktree concept, and a conversation is what owns a workspace; deleting one must close its shells. |
| Attach to a dead session | refused | replays the final screen + exit code | The tab can show what happened instead of an empty error. |
| Teardown | `killpg` | session-scoped sweep | Verified: job-control background jobs live in their own process groups. |

## Known limitations

* No drag-to-reorder, no rename, no shell picker in the UI (the server already
  exposes `GET /terminal/shells`).
* No terminal search, no link handling, no paste confirmation yet.
* A paired **desktop** sees this machine's terminals (§[Remote](#remote)); the
  phone surface and the mobile app do not, and the phone has no terminal
  renderer at all.
* `future remote peers list` prints connection state, not the host's advertised
  capabilities — a missing panel is diagnosed with
  `cargo run -p future-host --example peer_features` (§[Remote](#remote)).
* The panel's open/height preferences and the tab view state live in
  `localStorage`; a cleared store loses the *view*, never the shell.
