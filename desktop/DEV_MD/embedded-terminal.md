# Embedded terminal

Status: implemented for **Linux** (verified by automated tests); macOS/Windows
paths exist but are **not verified** on this machine — see §Platforms.

The terminal is a PTY registry owned by the desktop main process and served to
the app's own webview over a **loopback-only HTTP/WebSocket listener**. It is
deliberately not a Tauri-IPC message pump.

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
| `POST /terminal` | create `{threadId, title?, cols?, rows?, cwdPolicy?}` |
| `GET /terminal/:id` | one session |
| `PATCH /terminal/:id` | `{title?, cols?, rows?}` |
| `DELETE /terminal/:id` | terminate the tree and forget the session |
| `POST /terminal/:id/connect-token` | issue a single-use connect ticket |
| `GET /terminal/:id/connect?cursor=&ticket=` | WebSocket upgrade |

On the socket:

* **server → client**: binary frames of raw PTY bytes, plus one control frame —
  `0x00` followed by UTF-8 JSON `{cursor, start, exitCode?}`;
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
* Terminal output never reaches the agent, the RPC bridge, the remote control
  plane, logs or SQLite. The child environment is the user's own environment
  minus the app's private plumbing (`FUTURE_AGENT_GRPC_ADDR`).

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
* The shortcut (`Ctrl+J`, `⌘J` on macOS) belongs to the app, not to the shell.
  `TerminalView` releases it from xterm's key handling so the window listener
  still sees it while the terminal has focus — without that, xterm would cancel
  the event and send a line feed to the shell instead. The single definition
  lives in `features/terminal/shortcut.ts`.

## Working directory

Resolved server-side from the conversation; the client never sends a path.

1. the workspace strictly associated with the thread (`workspace_id`, so a
   chat's temporary workspace and a real workspace can never be confused);
2. the user's home directory — but **only** when nothing is configured, or after
   the user explicitly confirms the fallback.

A configured but missing/not-a-directory path fails loudly with `CWD_INVALID`.
This matters because `portable-pty` silently substitutes `$HOME` for a bad cwd
(verified in the earlier spike); the backend validates before spawning.

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

Nothing above claims otherwise. A Job Object on Windows and `proc_listchildpids`
on macOS would be strictly better than the current best-effort paths.

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
* Remote (phone-control) and mobile surfaces do not see terminals by design.
* The panel's open/height preferences and the tab view state live in
  `localStorage`; a cleared store loses the *view*, never the shell.
