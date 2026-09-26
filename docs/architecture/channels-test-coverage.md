# Channel framework test coverage

What the channel framework promises about tests, and the handful of lines that
are deliberately not covered.

## Where the authoritative numbers live now

This page states the *policy* and the historical waivers. The measurement, the
per-file inventory and the current ledger live in
[`../testing/module-channels.md`](../testing/module-channels.md), which is the
document the goal gate reads (`verify.py crate future-channels …`).

Two things this page predates and that you should know before quoting it:

1. **Platform.** The counts below were produced on Linux, using the LCOV
   `DA:<line>,0` records the `chan-*` scripts read. On Windows the same crate
   measures **117 DA-uncovered lines**, and under llvm-cov's (stricter) JSON
   `summary.lines` metric — the one the goal gate uses — **222 uncovered lines in
   39 files at 99.1760%**. The difference is not behaviour: llvm-cov's line
   summary also counts a line that *executed* but carries a zero region (the
   `?`-error branch of an `await?` on its own line, a `}`/span end), which is the
   `attribution-artifact` class below. `docs/testing/module-channels.md` §1
   explains the split and §6 lists every file.
2. **Windows-red tests.** The seven tests that were red on `main` on Windows
   (Discord ×3, Mattermost, QQ, Slack, `transport/webhook`) are fixed; four of
   them were *test* bugs — including two that matched the OS's **localized**
   error text — and one was a harness bug (`WsAction::SendClose` silently ended
   the script). See `docs/testing/module-channels.md` §2, and re-run them with
   `python .future/cov100/verify.py windows-red-baseline future-channel`.

## The measure

Coverage is per line, from llvm-cov's `DA:<line>,0` records — not a percentage
summary. A line is covered when a test executes it; nothing is measured by
proxy.

```bash
bash scripts/measure/chan-cov.sh                          # whole crate: report + missed list
bash scripts/measure/chan-cov.sh --check FILE [FILE...]    # fail if a named file has an uncovered line
python3 scripts/measure/chan-missed.py [substring]        # grouped list of uncovered lines
python3 scripts/measure/show-lines.py FILE LINE [LINE...] # print the lines with context
```

The script sanitises the environment (`CARGO_HOME`, `CARGO_TARGET_DIR`, `HOME`)
and takes an exclusive lock: a leaked variable changes the instrumented unit
hash and llvm-cov then reports every line as never executed, and concurrent
instrumented runs thrash the machine.

The line numbers below are one measurement snapshot; they drift as the crate
changes. Re-run the commands above before relying on a specific number.

## The rule

New code is expected to have every line executed by a test. "Covered" means a
test that would fail if the line were wrong — not a line reached incidentally by
an unrelated test, and not a line made unreachable to improve the number.

When a line cannot be covered, it is one of three things, and it is written down
here instead of being hidden:

1. **Unreachable by construction** — the code is a guard the surrounding types
   make impossible. Prefer deleting it. If it is kept (a protocol edge, a
   defensive arm), say why the type system cannot express it.
2. **Unreachable in this environment** — the line needs a platform, a broken
   socket, or a real external service the harness cannot produce
   deterministically.
3. **Attribution artifact** — the line is a closing brace or a span end next to
   a macro expansion. llvm-cov attributes a separate region to it, so it reads
   as `0` even though the surrounding statements are covered. Every artifact
   listed below names the test that proves the code ran.

## Uncovered lines in the framework

Measured on the channel crate. 22 lines, in four groups. **Line numbers on this
page drift**; the authoritative, freshly measured per-file inventory (Windows:
222 lines in 39 files by llvm-cov's `summary.lines`, 117 by the DA records this
page uses) is `docs/testing/module-channels.md` §5–§6. The groups below are the
*reasons* that survive measurement on either platform.

### Environmental (8)

| Lines | Why it cannot be executed by a test |
|---|---|
| `transport/webhook.rs` 149–151 | The `accept()` failure arm. It needs the listener to break or the process to run out of file descriptors — not reproducible deterministically in-process, and forcing it (fd exhaustion) would break other tests running in parallel. |
| `providers/imessage.rs` 282 | `sender()`'s `!platform_supported()` arm. Coverage is measured on macOS, where the predicate is true by definition. |
| `providers/imessage.rs` 391 | The tail of `send()` after `run_osascript`. Reaching it runs the real `osascript` and would send a real iMessage. |
| `providers/slack.rs` 934–936 | `#[cfg(not(test))] webhook_test_slot`. The measured binary is a test build, so this body is not compiled into it. |

### Unreachable by construction (4)

| Lines | Why |
|---|---|
| `providers/slack.rs` 733, `providers/mattermost.rs` 527 | `let Some(message) = stream.next() else { bail!("… closed by the platform") }`. The original experiment here concluded the `None` arm was unreachable, because a server that drops the connection **without** a close frame yields `Err(Protocol(ResetWithoutClosingHandshake))` on the read arm. That is true of an abrupt drop but **not** of a drained close handshake: `a_drained_close_handshake_ends_the_message_stream_with_none` now drives a close frame, waits for the client's reply, half-closes the server's write half, and the client's stream yields `None` (proved against tungstenite 0.24's `ConnectionClosed → None` mapping). The `None` arm is therefore **reachable**; the *session*-level arm is simply not driven by a test yet, and it is listed as OPEN in the module ledger rather than waived here. |
| `test_support.rs` 741 | `unreachable!("tests dial plain ws only")` in `kill_write_half`: the tests hand it a plain socket, so the TLS arm cannot be taken. This helper moved out of `providers/slack.rs` (where it was `#[cfg(all(test, unix))]`) into `channels/src/test_support.rs` with `#[cfg(unix)]`/`#[cfg(windows)]` branches, so the Mattermost auth-challenge test can use it on Windows. |
| `outbox.rs` 210 | The "not built" guard in `Outbox::context`. Resolving a channel by id goes through the registry, which ships no planned channel, so the guard cannot fire. The same guard **is** covered in the starter, where a definition can be handed in directly (see below). |

### Ordering and environment (1)

| Line | Why |
|---|---|
| `providers/mattermost.rs` 552 | The pong write on a socket that died between the frame being read and the reply being written. The mock cannot order those two events: a socket that is already dead fails the earlier authentication write instead (covered), and a reset that arrives while the session is reading is reported by the read arm first. The existing reset tests assert the same user-visible outcome (the session reports an unwritable socket). |

### Attribution artifacts (9)

Each of these is a brace or a span end; the neighbouring statements are covered
by the named test.

| Lines | What they are | Proof the code runs |
|---|---|---|
| `lib.rs` 172–173, 190–191 | The `inspect_err` closures of the Feishu and DingTalk supervisors | Both bridges retry on error and return `Ok` on shutdown, so the closure only runs if a bridge is aborted before its first exit. Nothing in the suite aborts them mid-run (the process-level SIGINT tests that would are `#[cfg(unix)]`). |
| `lib.rs` 386 | `}` closing the flusher's flush-failure block | `the_flusher_survives_an_unwritable_snapshot` makes the flush fail and asserts the flusher keeps running. |
| `lib.rs` 514 | The `Some(Running)` operand of the state assertion in `starting_publishes_a_state_for_every_channel` | The snapshot is taken before the supervisors run, so every enabled row is `Starting` and the second operand is never evaluated. It stays in the assertion so the test remains valid if that changes. |
| `providers/slack.rs` 763, 903 | The `}` after the webhook's `if let Some(event)` and after the dispatch spawn | `the_events_webhook_verifies_signatures_and_answers_the_challenge` waits for the accepted prompt's acknowledgement, which `dispatch_event` only sends once the event has been through the bridge pipeline. |
| `providers/qq.rs` 626 | `}` closing the heartbeat branch | `heartbeats_echo_the_last_sequence_and_stop_after_a_missed_ack` and `an_ack_clears_the_heartbeat_flag_and_the_connection_survives` assert the heartbeat frame and the cleared ack flag. |

## Lines outside this scope

The Feishu and DingTalk bridges (`channels/src/feishu/`, `channels/src/dingtalk/`)
and the agent gRPC client (`channels/src/grpc_client.rs`) carry **25 or 26**
further uncovered lines — the count moves by one on its own, for a reason worth
knowing. `feishu/feishu_ws.rs:254` is the `warn!` inside the pong send, and its
test tolerates both orderings of a race on purpose ("either the pong-send warn
fires and the read errors, or the read errors first"); the line is executed only
when the send wins. A measurement of 47 and one of 48 can therefore both be
right, and a single line appearing there is not evidence that a change added it.
They predate the channel framework and are not part of it; they are listed here
only so the crate-wide number is not mistaken for framework debt.

## Keeping the guard testable

`start_all` decides each registered channel's fate in `entry_action`, which
takes a definition rather than reaching into the registry. That is what lets a
test exercise "enabled but not built is reported, not silently skipped" without
the build having to ship a planned channel:
`an_enabled_channel_that_is_not_built_is_reported_and_skipped` constructs one.

`pump_lines` takes any `BufRead`, so its read-error arm is covered by
`a_read_error_ends_the_pump_instead_of_retrying_it` rather than by breaking the
test's own stdin.
