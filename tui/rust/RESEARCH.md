# future-tui Rust port — Research & Architecture (P0)

Goal (goal_6b1065901442): translate `tui/` (TypeScript, run by bun/node) 1:1 into a
Rust crate at `tui/rust/` — identical UI rendering, key handling, interaction,
argument parsing, and help text — with a **self-implemented terminal backend**
(no crossterm; `libc` on POSIX, `windows-sys` for the later Windows phase), TS
unit tests ported, a render-diff harness, and tmux screen-consistency tests.
All work happens in the `claude/tui-rust-port` worktree; `main` is untouched.

This document records the P0 research decisions: terminal-backend framework
comparison, the self-implemented backend design, and the markdown plan
(consumed by a later phase). It is the decision log the rest of the port
references.

---

## 1. Terminal backend: framework comparison

The TS TUI writes raw ANSI escape sequences through a thin `NodeTerminal`
class (`tui/src/tui.ts`) and parses raw stdin bytes itself
(`tui/src/stdin-buffer.ts` + `tui/src/keys.ts`). That byte pipeline is the
behavioral contract we must reproduce exactly, so the backend choice is driven
by "can it hand us raw bytes and let us write our own escape strings".

| Option | Raw bytes from terminal? | Our escape strings verbatim? | Event model matches TS? | Verdict |
|---|---|---|---|---|
| **crossterm** | No — it pre-parses `KeyEvent`s on a reader thread; the raw byte stream is consumed internally | No — it owns cursor/color writes; we'd bypass it and write raw strings anyway, keeping only its raw-mode toggle | No — its event enum is a different contract than `parseKey`'s string key-ids | **Rejected.** It would sit between us and the bytes we must parse ourselves; a 1:1 port would fight it |
| **ratatui** | No — buffer/cell rendering model, widgets, layout engine | No | No — completely different rendering paradigm | **Rejected.** A rewrite, not a port; byte-identical output is impossible to guarantee through a cell-buffer diff |
| **termion** | Yes (raw reads) | Partial | Close-ish, but its own parser + dead/unmaintained ecosystem | Rejected — unmaintained, Windows story weak |
| **termwiz** (wezterm) | Yes | Partial | Its own parser, very heavy dependency tree | Rejected — overkill; its parser would be a second source of truth |
| **Self-implemented** (`libc` + `windows-sys`) | Yes — `poll(2)` + `read(2)` on fd 0 | Yes — every write is our string | Yes — we feed our own `StdinBuffer` → `keys::parse_key` pipeline, 1:1 with TS | **Chosen.** Also mandated by the task |

### Decision

Self-implemented terminal backend in `tui/rust/src/terminal.rs`. Scope per
phase:

- **P0 (this phase, POSIX-only):** raw mode via `tcgetattr`/`tcsetattr`
  (`cfmakeraw`-equivalent flags, matching Node's `setRawMode(true)`), window
  size via `ioctl(TIOCGWINSZ)` with `COLUMNS`/`LINES` env fallback then 80×24,
  signal handling via `sigaction` + **self-pipe** (async-signal-safe `write(2)`
  in the handler, decoded by the reader thread), and an input loop built on
  `poll(2)` over `{stdin, signal-pipe}` running on a background thread.
- **Windows (later phase):** `windows-sys` console API (`SetConsoleMode`,
  `ReadConsoleInputW`, `GetConsoleScreenBufferInfo`) behind the same
  `Terminal` trait. The TS code already branches on `process.platform !==
  "win32"` in the same places, so the surface is known.

### Why a background reader thread (not async I/O)

The TS TUI is single-threaded async: stdin `data` events arrive on the event
loop. The Rust equivalent that keeps the render/main thread free is a
dedicated reader thread doing blocking `poll` + `read`, dispatching through the
same callbacks the TS `start(onInput, onResize)` accepts. No tokio/mio needed:
the terminal contract is callback-based and synchronous from the app's point of
view. `drainInput` (used to disable keyboard protocols around modals) is
reproduced by a `draining` flag the reader thread honors, plus an idle/max
deadline polled by the caller.

### POSIX primitives used (all via `libc`)

- `tcgetattr`/`tcsetattr` — save/restore termios; raw flags mirror Node's
  `setRawMode(true)`: clear `BRKINT|ICRNL|INPCK|ISTRIP|IXON` (iflag),
  `OPOST` (oflag), `ECHO|ICANON|IEXTEN|ISIG` (lflag), set `CS8` (cflag).
  Note `ISIG` off means Ctrl+C arrives as byte `0x03` in the input stream —
  `keys::parse_key` maps it to `ctrl+c`, exactly like TS.
- `ioctl(TIOCGWINSZ)` — columns/rows; refreshed on `SIGWINCH`.
- `sigaction` + self-pipe — one global handler writes the signal number as a
  byte to a pipe; the reader thread decodes: `SIGWINCH` → resize callback +
  re-measure; `SIGINT`/`SIGTERM`/`SIGHUP`/`SIGQUIT` → async-signal-safe restore
  (tcsetattr + cursor-show + alt-screen-exit) from the *reader thread* (not the
  handler — no lock/alloc in the handler), then re-raise with `SIG_DFL` so the
  exit status matches a normal signal death. `SIGTSTP` is intentionally not
  handled (TS doesn't either).
- `poll(2)` — multiplex stdin + signal pipe + timer deadlines (kitty
  query fallback at 150 ms, progress keepalive at 1 s, StdinBuffer flush at
  10 ms idle).
- `isatty` — `start()` refuses non-tty stdin with a clear error.

### Escape-sequence surface (written verbatim, matching `tui.ts`)

`\x1b[?1049h/l` (alt screen), `\x1b[?2004h/l` (bracketed paste),
`\x1b[?25l/h` (cursor), `\x1b[?u` / `\x1b[?<n>u` / `\x1b[<u` / `\x1b[>7u`
(kitty protocol query/response/enable), `\x1b[>4;2m` / `\x1b[>4;0m`
(modifyOtherKeys), `\x1b]9;4;3\x07` / `\x1b]9;4;0;\x07` (progress),
`\x1b]0;<title>\x07` (title), `\x1b[<n>B/A` (moveBy), `\x1b[K`, `\x1b[J`,
`\x1b[2J\x1b[H`. The `PI_TUI_WRITE_LOG=1` write-log to
`~/.future/tui/write.log` is reproduced.

## 2. Input pipeline (ported 1:1, pure logic)

- `stdin-buffer.rs` — `StdinBuffer` port. Complete/incomplete escape-sequence
  detection (CSI/OSC/DCS/APC/SS3/meta), bracketed-paste buffering, kitty
  printable-codepoint duplicate suppression, and the 10 ms idle flush. The
  TS `setTimeout` flush is driven by the terminal loop (poll deadline) instead
  of an internal timer so the buffer stays a pure, synchronously testable
  state machine; `process()` returns `Vec<StdinEvent>` and `flush()` drains the
  remainder — observable behavior identical.
- `keys.rs` — `parse_key`, `matches_key`, `decode_kitty_printable`,
  `is_key_release/repeat`, kitty/modifyOtherKeys/legacy parsing. Regexes are
  ported with the `regex` crate (compiled once in `OnceLock`); the
  `_kittyProtocolActive` global is an `AtomicBool` shared with `terminal.rs`.
  `is_windows_terminal_session()` reads `WT_SESSION`/`SSH_*` env vars like TS.

## 3. Rendering utilities (ported 1:1, pure logic)

`utils.rs` ports `tui/src/utils.ts` byte-for-byte: grapheme-aware width
(`unicode-segmentation` in place of `Intl.Segmenter("en", {granularity:
"grapheme"})` — UAX #29 both; the tested cases CJK/emoji/combining/VS15/VS16
match), ANSI code extraction/tracking (`AnsiCodeTracker`), `stripAnsiCodes`,
word wrap with style carry-over (incl. the pure-ASCII fast path), truncation,
column slicing, and overlay segment extraction. Caches are `thread_local`
(JS is single-threaded; per-thread caches keep parallel unit tests isolated).

`theme.rs` ports `tui/src/theme.ts` data + helpers; `tui.ts`'s `DEFAULT_THEME`
lives here too (the TS file has two theme tables — the app uses the `theme.ts`
one, `tui.ts`'s is legacy but ported for completeness).

## 4. Markdown rendering plan (P2+ phase, decision recorded now)

TS uses `marked` (markedjs) with a custom strict-strikethrough tokenizer
(`tui/src/components/markdown.ts`). Options considered:

| Option | CommonMark | GFM tables/strike | Maturity | Notes |
|---|---|---|---|---|
| **pulldown-cmark** | yes | tables + strikethrough via `Options` | excellent (cargo, docs.rs) | event-based iterator; hand-written renderer ports `markdown.ts`'s styles (headings, bold/italic/strike, code, links/OSC 8, quotes, lists, tables w/ cell wrap) |
| comrak (cmark-gfm) | yes | full GFM | good | heavier; GFM extensions beyond what we render |
| markdown (cmark-gfm wrapper) | yes | full GFM | good | same caveat |
| markdown-it-rs | marked's algorithm family | partial | low activity | closest lineage to marked but riskier maintenance |

**Decision:** `pulldown-cmark` for parsing + a hand-written renderer that
reproduces `markdown.ts`'s exact output (the rendering logic — wrapping,
padding, per-style ANSI — is ported 1:1; only the parser is swapped).
Caveats recorded for the markdown phase:

1. The strict-strikethrough tokenizer regex
   `^(~~)(?=[^\s~])((?:\\.|[^\\])*?(?:\\.|[^\s~\\]))\1(?=[^~]|$)` uses a
   **backreference** (`\1`), which `regex` does not support. Plan: implement
   the strict check with a small hand-rolled matcher (or a preprocessing pass)
   before/alongside pulldown-cmark's `~~` handling, then diff against marked
   on edge cases.
2. `marked`'s table parsing is lenient; verify cell content + alignment
   handling against the TS component tests.
3. Terminal-image hyperlink sequences (`\x1b]8;;url\x07`) come from
   `terminal-image.ts` — ported in the same phase.

## 5. Test strategy (mirrors the cli-rust-port playbook)

- **P0:** inline `#[cfg(test)]` unit tests ported from `keys.test.ts` +
  `utils.test.ts` (incl. the xorshift32 fuzz of the ASCII wrap fast path and
  the reference `wrapAscii`), plus new tests for `stdin_buffer` (no TS test
  existed), `theme`, and the terminal size/env fallback (env writes guarded by
  a global `ENV_LOCK` mutex, same pattern as `cli/rust/src/test_env.rs`).
  Raw-mode/signal tests require a tty → skipped unless `stdin` is a tty.
- **P1+:** component render tests (`components.test.ts`, `footer.test.ts`,
  `input-bugs.test.ts`, `display-fixes.test.ts`, `chat-streaming.test.ts`,
  `help-screen.test.ts`) once components are ported.
- **Render diff:** drive TS (bun) and Rust renderers with identical inputs and
  byte-compare the produced lines (like `cli/rust/tests/diff-ts-rust.sh`).
- **tmux screen consistency:** run both binaries under `tmux` with a fixed
  size, feed scripted keys, capture panes, byte-compare.

## 6. Build & versioning

- `tui/rust` is a workspace member (bin `future-tui`, lib `future_tui`).
- `build.rs` injects the version via the same logic as
  `scripts/version.mjs` (port of the cli-rust `build.rs`), so
  `future-tui --version` prints `future-tui v<version>` exactly like TS
  (`console.log(\`future-tui v${VERSION}\`)`).
- Dependencies kept minimal: `libc`, `regex`, `unicode-segmentation`
  (all present in the local cargo cache); `parking_lot` (workspace) for the
  stdout write mutex. No crossterm/ratatui.
- Toolchain: `rustup run 1.97.0 cargo` (Homebrew cargo on PATH ignores
  `rust-toolchain.toml`).
