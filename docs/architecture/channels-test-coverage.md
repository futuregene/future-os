# Channel framework test coverage

What the channel framework promises about tests, and the handful of lines that
are deliberately not covered.

## Where the authoritative numbers live now

This page states the *policy* and the historical waivers. The measurement, the
per-file inventory and the current ledger live in
[`../testing/module-channels.md`](../testing/module-channels.md), which is the
document the goal gate reads (`verify.py crate future-channels …`).

Four things this page predates and that you should know before quoting it:

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
3. **Round 2 (2026-10-16).** The campaign after this page was written measured
   the crate on **macOS arm64** by per-line LCOV `DA:<line>,0`, found **85**
   uncovered lines of 25 531, and took them to **24 of 25 740** (99.9068 %).
   That pass fixed a real defect in the iMessage fixture (three macOS-only
   polling tests had been asserting against messages older than the poll
   loop's watermark) and a real defect in the process-test harness (a SIGINT
   that is missed hangs the test binary forever). Run identity, the covered
   list and the per-line ledger are in `docs/testing/module-channels.md` §10;
   the tables below are that measurement.
4. **Round 2b (2026-10-16).** The adversarial review `rev-ch`
   (`docs/testing/review-rust.md`, round 2) then falsified **16 of the 24** rows
   by building an executing counterexample for each, and that rework is now
   shipped: the crate reads **8 uncovered lines of 25 841 = 99.9690 %**, with
   `attribution-artifact` and `unreachable-in-this-environment` empty for the
   framework and two rows re-registered with corrected reasons. The tables below
   are the round-2b state; `docs/testing/module-channels.md` §10.7 has the
   covering test of every row that fell, §10.4 the current ledger.

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

**Round-2b measurement (2026-10-16, macOS arm64, `cargo llvm-cov -p
future-channel --lcov`, per-line `DA:<line>,0`): 8 lines in 8 files.** That is the
current state, and it is the result of two rounds: round 2 took this crate from
85 to 24 uncovered lines, and round 2b executed the rework that the adversarial
review `rev-ch` forced by falsifying **16 of those 24 rows** with an executing
counterexample each (`docs/testing/review-rust.md`; the covering test of every
fallen row is named in `docs/testing/module-channels.md` §10.7.2, the current
per-line ledger is §10.4, and a race line that wobbles on its own is discussed at
the end of this section). **Two of the three groups below are now empty for this
crate**: every `attribution-artifact` and every `unreachable-in-this-environment`
row this page used to carry was *executable* once the right input was built. What
remains is one platform row and seven control-flow rows, each with the invariant
that makes every input avoid it.

### `platform-unmeasured` (1)

| Line | Why it cannot be executed by a test here |
|---|---|
| `providers/imessage.rs` 282 | `sender()`'s `!platform_supported()` arm. The predicate is `cfg!(target_os = "macos")` (`imessage.rs` 256–259), a compile-time constant, and coverage is measured on macOS, where it is true by definition; the Windows/Linux build executes this line. The other macOS-only arms of the same file are covered (`run_polling`'s whole body, `probe`, `send_text`). The review confirmed this as the one row whose category needed no correction, and nothing in the tree stubs the predicate. |

### `unreachable-in-this-environment` (0)

This group is empty for the framework. What used to be in it, and what covered
each line in round 2b:

| Former row | Now covered by |
|---|---|
| `transport/webhook.rs` 149, 150, 151 | `channels/tests/webhook_emfile.rs` — one test in its own process: `RLIMIT_NOFILE` lowered to the number of descriptors the process already holds, the table filled, one connection queued, then `serve()`. The per-process limit is why the fixture is contained; `147 → 34` against `148 → 33` shows the `Err` arm really was taken. (Closing the listener's fd behind `serve`'s back was measured and does *not* reach the arm — the poll pends.) |
| `providers/imessage.rs` 391 | `channels/tests/imessage_send_success.rs` — a `PATH` shim whose `osascript` exits 0, driven through the public provider API, so the success half is asserted with **no message sent**. |
| `providers/mattermost.rs` 552 | `a_reset_socket_fails_the_pong_reply`, with the frame order the review named (`WaitForReceived{1}` → ping → reset), plus a positive control that the auth challenge reached the mock. |
| `test_support.rs` 780 | `kill_write_half_reports_an_error_that_is_not_notconnected` — `dup2(/dev/null)` over the test's own socket descriptor makes `shutdown(Write)` answer ENOTSOCK; the `#[should_panic]` message *is* the assertion. |
| `providers/slack.rs` 934, 935, 936 | `slack_events_webhook_starts_in_the_binary` (`tests/channel_bin.rs`) — runs the **binary**, the only build that compiles the `#[cfg(not(test))]` twin, and waits for the server's own log line before SIGINT. |

### `unreachable-by-construction` (7)

| Lines | The invariant that makes every input avoid them |
|---|---|
| `feishu/mod.rs` 95, `dingtalk/mod.rs` 94 | The `Ok(())` arm of the connection monitor. `AgentClient::wait_for_disconnect` has no success path: after the stream ends it returns `Err(anyhow!("Agent connection closed"))` (`grpc_client.rs` 108–125) — there is no statement that yields `Ok(())`, so no stream shape can select the arm. The `Err` arm below is the only one that runs (it carries count 1), and `an_ended_stream_is_a_disconnect` asserts the error for an ended stream. |
| `providers/mattermost.rs` 527, `providers/slack.rs` 733 | The session-level `stream.next() -> None` arm. `None` needs a drained close handshake **and** the peer's FIN, and tungstenite hands the `Close` message to the loop first — the session returns at the `Ok(WsMessage::Close(_))` arm above (mattermost 555 / slack 775, both covered by the session close tests) and never reaches the `None` test. Measured directly by the review with a raw stream: `[Close]` → `Close(None)` then the poll **pends**; `[Close, FIN]` → `Close(None)` then `None`; `[FIN]` alone → `Err(ResetWithoutClosingHandshake)`; a reset → `Err(Connection reset)`. Ten session constructions in total (both providers: close, fin, both, ping+reset) all landed on the `Close` arm, the read-error arm or a bail. Both facts are why the earlier "drive it with `ShutdownWrite`" next step cannot work. |
| `providers/qq.rs` 626 | The skip edge of `if let Some(interval) = heartbeat_interval` (`qq.rs` 615–626). The enclosing `select!` arm's own guard — `_ = tokio::time::sleep(50 ms), if heartbeat_interval.is_some()` (`qq.rs` 612) — means entering the arm implies `Some`, so the false edge is unreachable **by construction**. The round-2 page called this an `attribution-artifact` and offered the "a tick where `due` is false" hypothesis; the review took exactly those ticks (a 30 s interval held open 600 ms ≈ 11 ticks, `625 → 11`) and 626 did not move, so the hypothesis is refuted by measurement rather than left untested. **Category and mechanism corrected here.** |
| `outbox.rs` 210 | The "channel not built" guard in `Outbox::context`. Resolving a channel by id goes through the registry, whose entries are all `is_implemented()` and which ships no `Planned` channel, so `ensure_usable()` cannot fail there — a **data-dependent dead guard, not a type-system impossibility**, and it is a **duplicate**: the same check *is* covered where a definition can be handed in directly (`lib.rs::entry_action`, `an_enabled_channel_that_is_not_built_is_reported_and_skipped`). The review proved it by executed enumeration over `providers::all_definitions()` — everything is either implemented or absent from the registry, and the absent ones bail one line earlier (`208 → 2`). |
| `test_support.rs` 770 | `_ => unreachable!("tests dial plain ws only")` in `kill_write_half`: the mock's sockets are always `MaybeTlsStream::Plain` (`769 → 4`), so the TLS arm cannot be taken. The `unreachable!` exists to fail loudly if that ever changes. Source-level result only: the three call sites (`slack_tests.rs` 1477/1503, `mattermost_tests.rs` 652) all dial `ws://`, and no TLS-capable mock exists — the crate does own a real TLS client path (`feishu_ws.rs` 183, `dingtalk_ws.rs` 131 use `crate::tls::ws_connector()`), so the missing piece is a mock server, not a property of the type system. |

### `attribution-artifact` (0)

Empty for this crate. All four rows this page used to carry were **executable, not
span ends**: `feishu/mod.rs` 111 and `dingtalk/mod.rs` 110 needed the *inner*
backoff wait to elapse (the round-2 "other edge" taken was the `if let`'s
**outer** false edge, which skips the region entirely), `feishu/prompt_loop.rs`
449 needed an errored `AgentEnd` with streaming off, and `providers/qq.rs` 626 was
misclassified outright (see the guard above). Seven other lines of this shape
**were** closed in round 2 by writing a test for the other edge of the construct
and no longer appear on this page: `feishu/bridge.rs` 933 (a `post` with no image
element), `feishu/prompt_loop.rs` 53 (a superseded turn with no ack reaction),
`providers/slack.rs` 763 (an `events_api` envelope with no `payload.event`) and
763's sibling 903 (an `event_callback` with no `event` field).

Lines this page used to waive that are **now covered** and no longer appear:
`lib.rs` 172, 173, 190, 191 (the supervisor spawn closures — the review showed
that awaiting the join handles executes the line; the `inspect_err` *closure body*
stays dead because `run` has no `Err` path, which is recorded in
`docs/testing/module-channels.md` §10.7.3), `lib.rs` 386 and 514 (the flusher's
failure brace and the `Started`-vs-`Running` operand), `providers/imessage.rs`
464–498 (the whole poll-loop body, which the fixed fixture uncovered),
`providers/cli.rs` 362–368, `session_store.rs` 199–202, `policy.rs` 427/447,
`feishu/bridge.rs` 139/880, `providers/slack.rs` 763/903 and the task-level
shutdown arms.

### The one race line (not a framework line)

`feishu/feishu_ws.rs` 254 is the `warn!` inside the pong send: its test tolerates
both orderings of a pong-send/read race on purpose, so the line executes only when
the send wins. A crate-wide total that moves by exactly one line, with this line
as the difference, is that race — not a regression. It is covered in the round-2b
run (which is why it is not in the 8 above) and was uncovered in earlier ones; the
row is kept here so the wobble is not rediscovered as a bug.


## Lines outside this scope

The Feishu and DingTalk bridges (`channels/src/feishu/`,
`channels/src/dingtalk/`) and the agent gRPC client (`channels/src/grpc_client.rs`)
predate the channel framework and are not part of it. **As of round 2b none of
them has an uncovered line**: the whole `channels/src` subtree reads 8 uncovered
lines, all of them listed in the groups above, and the round-2 measurement had
already cleared this surface (the 24 lines of that run contained no
`feishu/bridge.rs`, `dingtalk/` or `grpc_client.rs` line either). The one line
that can appear here is the race at `feishu/feishu_ws.rs` 254 discussed
above: it is executed only when the pong send wins a race its own test tolerates
in both orders, so two measurements of this crate can differ by exactly that line
and neither is wrong.

## Keeping the guard testable

`start_all` decides each registered channel's fate in `entry_action`, which
takes a definition rather than reaching into the registry. That is what lets a
test exercise "enabled but not built is reported, not silently skipped" without
the build having to ship a planned channel:
`an_enabled_channel_that_is_not_built_is_reported_and_skipped` constructs one.

`pump_lines` takes any `BufRead`, so its read-error arm is covered by
`a_read_error_ends_the_pump_instead_of_retrying_it` rather than by breaking the
test's own stdin.
