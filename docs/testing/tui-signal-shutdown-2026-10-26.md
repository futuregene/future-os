# `future tui` ignores `SIGTERM` once its terminal is gone (2026-10-26)

A one-off record, not a design document: the reproduction, the root cause with
line numbers, the fix, and what the tests do and do not cover. Everything below
was measured on this machine; the commands are written out so each row can be
re-checked.

The defect was handed over by the session that fixed the event-loop frame cost
(`2a856ab72`) and left it alone: *"`SIGTERM` in the no-tty state is still
ignored, identically before and after: the terminal reader exits at EOF, so
nothing polls the signal self-pipe — pre-existing terminal-lifecycle bug."*

## 0. Summary

| | |
|---|---|
| State | a `future tui` whose terminal went away still runs, with the termination-signal handlers installed and nobody draining the pipe they write to |
| Effect | `SIGTERM` / `SIGINT` / `SIGHUP` / `SIGQUIT` are **captured and silently dropped**; `kill` returns 0, the process stays; only `SIGKILL` removes it |
| Root cause | `tui/src/terminal.rs:372` (`if n == 0 { break; }`, base `ec891652f`) — the reader thread was the only reader of the self-pipe, and it exited at EOF |
| Fix | the reader keeps running for the self-pipe alone after stdin is gone, and a termination signal arms an exit deadline so a blocked cleanup cannot recreate "never exits" |
| Covered by | `tui/src/terminal.rs` unit test + `tui/tests/signal_shutdown.rs` (six real-process tests) |
| Not covered | a reader blocked *inside* a write to a terminal nobody drains, and `future agent`'s own signal path (checked by inspection only — it has no such hole; see §6) |

## 1. Reproduction

The terminal has to be *taken away* from a running TUI, not withheld at start —
those are two different states, and only one of them is broken.

| state | how it arises | behaviour |
|---|---|---|
| **no tty at start** (`stdin` is not a tty) | `future tui </dev/null` | `Terminal::start` refuses (`tui/src/terminal.rs:328`, `NotConnected`), the process exits at once with code 1 and `terminal: stdin is not a TTY — the interactive TUI needs a terminal`. **Not a bug** — nothing lingers, and no handler was ever installed, so the signal's default action still applies |
| **tty that died later** | a terminal emulator, `tmux` pane, SSH session or test harness goes away while the TUI runs (a `script -q /dev/null …` whose `script` is killed is one way; that is how the six orphans in the hand-over report were produced) | `start` already succeeded and installed the handlers; stdin now reads EOF/EIO; the signal is swallowed. **The bug** |

The measurement used a parent that starts the binary on a fresh PTY, closes the
master (the child's stdin becomes EOF on macOS, `EIO` on Linux), waits, then
sends one signal per process and reports the exit status. `</dev/null>` alone
cannot reproduce it, which is worth knowing before trying.

```bash
# pre-fix and post-fix binaries, same build flags (FUTURE_VERSION=0.0.2-sigtest)
for s in TERM INT HUP QUIT; do python3 sig-one.py /tmp/future-prefix "$s"; done
for s in TERM INT HUP QUIT; do python3 sig-one.py /tmp/future-postfix "$s"; done
```

The pre-fix binary is the base commit with the fix's EOF arm reverted to the
original `break` (the same edit that makes the new tests red, §5).

### 1.1 Measured before vs after (dead PTY, one process per signal)

| signal | before (`ec891652f`) | after (this branch) |
|---|---|---|
| `SIGTERM` | **ignored** — still alive after 12 s, `SIGKILL`ed for cleanup | exits, code **0**, in 0.1 s |
| `SIGINT` | **ignored** — same | exits, code **0**, in 0.1 s |
| `SIGHUP` | **ignored** — same | exits, code **0**, in 0.1 s |
| `SIGQUIT` | **ignored** — same | exits, code **0**, in 0.1 s |

Exit code 0 (not "killed by signal 15") is the intended shape: the signal is
turned into the app's own ctrl+c path, so the terminal is restored and the run
ends as a normal exit. That was already the behaviour for a *live* terminal
before this fix (measured: `SIGTERM` on a live PTY exits, code 0, both before
and after).

The other two states the task named are fine in both builds:

| state | before | after |
|---|---|---|
| live PTY, agent connected | `SIGTERM` → exits, code 0 | same |
| live PTY, **agent connection failed** (`FUTURE_HOME` with no agent) | `SIGTERM` → exits, code 0 | same |

### 1.2 One correction to the hand-over report

The report also recorded 100 % CPU. That was true for the orphans it found at
00:36, before the frame-cost fix merged; with `2a856ab72` in the tree the same
dead-PTY process idles at **0.01 s of CPU per 5 s** (~0 %) on both the pre-fix
and the post-fix binary. The signal hole is independent of that: the process is
cheap to leave running, which if anything makes it easier to forget.

## 2. Root cause

Signal handling is a self-pipe: `enable_raw` installs handlers with `sigaction`
(`tui/src/terminal_posix.rs:190`, `install_signal_handlers` at `:153`) and the
handler only does an async-signal-safe `write(2)` of the signal number into a
non-blocking pipe (`tui/src/terminal_posix.rs:53`). Everything real — resize,
exit callback, restore-and-re-raise — happens on the **reader thread**, which
`poll(2)`s `{stdin, signal-pipe}` (`tui/src/terminal_posix.rs:250`, dispatch at
`tui/src/terminal.rs:400` and `:407`).

That thread is also the only thing that leaves the loop early:

- `tui/src/terminal.rs:363` (`ec891652f`): a read error → `break`;
- `tui/src/terminal.rs:372` (`ec891652f`): **stdin at EOF → `break`** — the
  ordinary case for a TUI whose PTY master closed.

From the moment it breaks, nobody `poll`s the pipe. The handlers are still
installed (`restore_raw`, which restores `SIG_DFL`, only runs from `stop()`), so
`SIGTERM` no longer performs its default action; it is written into a pipe with
no reader and forgotten. The main loop never learns that stdin is gone either,
so the process sits there — the one state where `SIGTERM` is neither handled nor
defaulted, and only `SIGKILL` works.

## 3. The fix

Two parts, both in the terminal layer; nothing about input, drawing, layout or
the frame cost changes, and `terminal_windows.rs` only gains the probe §3.2 needs.

### 3.1 The reader outlives stdin

`tui/src/terminal.rs:81` (`wait_next`) — when stdin reports EOF or a read error
the loop no longer breaks: it sets `stdin_gone` (`:450`, `:476`) and keeps going.
What changes is the poll set, not the loop:

- `tui/src/terminal_posix.rs:321` `wait_for_signals` polls the **self-pipe only**.
  fd 0 has to leave the set — a hung-up PTY stays readable forever, so keeping it
  would spin — and the pipe has to stay, because the handlers are still installed.
- The dispatch arms are unchanged, so `SIGWINCH`, the exit callback and the
  restore-and-re-raise failsafe all still work in that state. The exit callback
  is what makes a signal an ordinary graceful stop instead of a kill.

`stop()` is unaffected: it sets `stop_flag` and `wake()`s the pipe, which is
exactly what the signal-only poll waits on (`tui/src/terminal.rs:619`).

### 3.2 Cleanup cannot resurrect "never exits"

The graceful path is "hand the app a ctrl+c and let it exit". That is right when
it works and no path at all when the app loop is blocked — on an agent read, on
a write to a terminal nobody drains, or because it is already gone. So a caught
termination signal arms a deadline before the callback runs
(`tui/src/terminal.rs:120`, `arm_exit_deadline`, called at `:525`):

- after `SIGNAL_EXIT_DEADLINE` (10 s in production,
  `FUTURE_TUI_EXIT_DEADLINE_MS` for the tests that cannot wait that long) the
  thread checks `Backend::raw_active()` (`tui/src/terminal_posix.rs:341`,
  `terminal_windows.rs`);
- if the terminal is **still in raw mode**, cleanup has not got as far as
  restoring it: run the existing failsafe `restore_terminal_for_exit`
  (`tui/src/terminal.rs:979` — cursor, bracketed paste, mouse and keyboard
  protocols, progress indicator, termios, stderr) and die with the signal's
  default disposition, so the exit status still says "SIGTERM";
- if raw mode is already off, the terminal was handed back and the rest of the
  exit (settings, agent disconnect, lock release) gets to finish.

The deadline thread only sleeps and writes — it can never block on the state the
graceful path is stuck on. Nothing switched `SIGTERM` back to `SIG_DFL` as a
shortcut: the handler stays installed, and the graceful path is what runs first.

## 4. Behaviour worth knowing

- A signal after EOF still reaches the app's exit callback, i.e. the same
  graceful path as before: terminal restored, session persisted, agent
  disconnected, lock released (verified end to end: the exit status is 0, not a
  signal death).
- The reader wakes 10×/s after EOF (the loop's 100 ms poll cap), which is what
  the loop already did before EOF. It does not repaint anything.
- EOF itself is still not an exit. That is not part of this fix: a surface with
  no keyboard still has a screen and an agent stream, and turning "terminal gone"
  into "process gone" would be an interaction change, not a lifecycle fix.

## 5. Tests

| test | where | what it pins |
|---|---|---|
| `reader_loop_keeps_handling_signals_after_stdin_eof` | `tui/src/terminal.rs` (unit, real PTY) | after the master closes, `SIGTERM` still reaches the exit callback within 2 s, and `stop()` still joins the reader loop |
| `sigterm_stops_a_tui_whose_terminal_is_gone` | `tui/tests/signal_shutdown.rs` | a **real process** on a PTY: master closed → `SIGTERM` → exits, code 0, within 15 s |
| `sigint_and_sighup_stop_it_too` | same | the same for `SIGINT` and `SIGHUP`, one process per signal |
| `a_blocked_cleanup_still_exits_on_sigterm` | same | an exit callback that never finishes is force-ended by the deadline (set to 800 ms for the test); asserts both that the process goes and that it did not go before the deadline could have run |
| `a_tui_with_no_tty_does_not_linger` | same | the no-tty state exits on its own instead of lingering |
| `eof_alone_does_not_stop_the_process` | same | closes the terminal and waits 500 ms: no exit. Keeps the tests above honest — if EOF alone ended the process they would pass without touching the signal path |

The child in the integration tests is the test binary re-executed
(`--exact signal_shutdown_child`) with an isolated `FUTURE_HOME`, a PTY on stdin
(with `FD_CLOEXEC`, so the parent's close really is the last master) and its
output in a temp file. A `Drop` guard SIGKILLs and reaps the child if a test
panics, so a red run cannot leave the very orphan being fixed.

**Falsified, not just green:** reverting the EOF arm to the original `break`
(`tui/src/terminal.rs:372` at `ec891652f`) turns
`reader_loop_keeps_handling_signals_after_stdin_eof`,
`sigterm_stops_a_tui_whose_terminal_is_gone`, `sigint_and_sighup_stop_it_too` and
`a_blocked_cleanup_still_exits_on_sigterm` red (measured; the other two stay
green, as they should), and restoring it makes them green again.

### What the tests do not cover

- **A reader blocked inside a write.** If the reader is stuck in `write(2)` to a
  terminal whose buffer is full (a suspended `tmux` client), the signal byte sits
  in the pipe undrained and the deadline never arms. That is a different stall —
  the fix's boundary is "the reader is running".
- **`future agent` / `future host`.** §6 is inspection, not a test.
- **Windows.** The console path has no signals; Ctrl+C and console-close arrive
  as input events on the reader thread, which is why `wait_next` is a no-op
  there. The new `raw_active()` in `terminal_windows.rs` is compiled by CI only
  (cross-`clippy` for `future-tui` stops in `libsqlite3-sys`'s C build on this
  machine).

## 6. `future agent` / `future host`: no same-shaped hole

Neither builds a `Terminal` (no raw mode, no self-pipe), so the reader-thread
structure cannot occur:

- the headless entry (`cli/src/headless/mod.rs:780` `ShutdownSignals`) uses
  `tokio::signal` for SIGINT/SIGTERM/SIGHUP (`:797`) on a signal stream `select!`ed
  against the serving loop (`:263` `until_shutdown`), and records which signal
  ended the run (`StopCause`, `:213`);
- `future agent` (`agent/src/cli/shutdown.rs:24`) watches ctrl+c on a dedicated
  thread with its own runtime, and after a 5 s grace calls `std::process::exit(130)`
  instead of waiting for a teardown that may be stuck. `SIGTERM` is not captured
  there at all, so its default action terminates the process — the opposite
  failure mode from this report (no cleanup, but it does stop).

Nothing was changed in either.

## 7. Environment

| | |
|---|---|
| Machine | macOS 26.6.2 (25G83), Apple silicon |
| Base commit | `ec891652f` on `origin/dev-huichen`; branch `fix/tui-signal-shutdown` |
| Toolchain | pinned `1.97.0` (`rust-toolchain.toml`) |
| Binaries | `cargo build -p future-cli` (debug), `FUTURE_VERSION=0.0.2-sigtest` for both the before and the after build |
| Checks | `cargo test -p future-tui` (1056 lib tests + the integration targets), `cargo fmt --check`, `cargo clippy -p future-tui --all-targets -- -D warnings` |
