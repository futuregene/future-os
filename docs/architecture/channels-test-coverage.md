# Channel framework test coverage

What the channel framework promises about tests, and the handful of lines that
are deliberately not covered.

## The measure

Coverage is per line, from llvm-cov's `DA:<line>,0` records — not a percentage
summary. A line is covered when a test executes it; nothing is measured by
proxy.

```bash
bash scripts/chan-cov.sh                          # whole crate: report + missed list
bash scripts/chan-cov.sh --check FILE [FILE...]    # fail if a named file has an uncovered line
python3 scripts/chan-missed.py [substring]        # grouped list of uncovered lines
python3 scripts/show-lines.py FILE LINE [LINE...] # print the lines with context
```

The script sanitises the environment (`CARGO_HOME`, `CARGO_TARGET_DIR`, `HOME`)
and takes an exclusive lock: a leaked variable changes the instrumented unit
hash and llvm-cov then reports every line as never executed, and concurrent
instrumented runs thrash the machine.

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

Measured on the channel crate. 22 lines, in four groups.

### Environmental (7)

| Lines | Why it cannot be executed by a test |
|---|---|
| `transport/webhook.rs` 149–151 | The `accept()` failure arm. It needs the listener to break or the process to run out of file descriptors — not reproducible deterministically in-process, and forcing it (fd exhaustion) would break other tests running in parallel. |
| `providers/imessage.rs` 282 | `sender()`'s `!platform_supported()` arm. Coverage is measured on macOS, where the predicate is true by definition. |
| `providers/imessage.rs` 391 | The tail of `send()` after `run_osascript`. Reaching it runs the real `osascript` and would send a real iMessage. |
| `providers/slack.rs` 937–939 | `#[cfg(not(test))] webhook_test_slot`. The measured binary is a test build, so this body is not compiled into it. |

### Unreachable by construction (4)

| Lines | Why |
|---|---|
| `providers/slack.rs` 736, `providers/mattermost.rs` 527 | `let Some(message) = stream.next() else { bail!("… closed by the platform") }`. Verified by experiment: a server that drops the connection **without** a close frame does not end the stream — the client reports `Err(Protocol(ResetWithoutClosingHandshake))` on the read arm. A connection that closes properly yields `Ok(Close)`, which bails on its own arm. The `None` arm is therefore not reachable through this client; it is kept because `Stream::next` is typed as `Option`. |
| `providers/slack.rs` 979 | `unreachable!("tests dial plain ws only")` in a test-only helper: the tests hand it a plain socket, so the TLS arm cannot be taken. |
| `outbox.rs` 210 | The "not built" guard in `Outbox::context`. Resolving a channel by id goes through the registry, which ships no planned channel, so the guard cannot fire. The same guard **is** covered in the starter, where a definition can be handed in directly (see below). |

### Ordering and environment (1)

| Line | Why |
|---|---|
| `providers/mattermost.rs` 552 | The pong write on a socket that died between the frame being read and the reply being written. The mock cannot order those two events: a socket that is already dead fails the earlier authentication write instead (covered), and a reset that arrives while the session is reading is reported by the read arm first. The existing reset tests assert the same user-visible outcome (the session reports an unwritable socket). |

### Attribution artifacts (10)

Each of these is a brace or a span end; the neighbouring statements are covered
by the named test.

| Lines | What they are | Proof the code runs |
|---|---|---|
| `lib.rs` 172–173, 190–191 | The `inspect_err` closures of the Feishu and DingTalk supervisors | Both bridges retry on error and return `Ok` on shutdown, so the closure only runs if a bridge is aborted before its first exit. Nothing in the suite aborts them mid-run. |
| `lib.rs` 386 | `}` closing the flusher's flush-failure block | `the_flusher_survives_an_unwritable_snapshot` makes the flush fail and asserts the flusher keeps running. |
| `lib.rs` 514 | The `Some(Running)` operand of the state assertion in `starting_publishes_a_state_for_every_channel` | The snapshot is taken before the supervisors run, so every enabled row is `Starting` and the second operand is never evaluated. It stays in the assertion so the test remains valid if that changes. |
| `providers/slack.rs` 766, 906 | The `}` after the webhook's `if let Some(event)` and after the dispatch spawn | `the_events_webhook_verifies_signatures_and_answers_the_challenge` waits for the accepted prompt's acknowledgement, which `dispatch_event` only sends once the event has been through the bridge pipeline. |
| `providers/qq.rs` 626 | `}` closing the heartbeat branch | `heartbeats_echo_the_last_sequence_and_stop_after_a_missed_ack` and `an_ack_clears_the_heartbeat_flag_and_the_connection_survives` assert the heartbeat frame and the cleared ack flag. |

## Lines outside this scope

The Feishu and DingTalk bridges (`channels/src/feishu/`, `channels/src/dingtalk/`)
and the agent gRPC client (`channels/src/grpc_client.rs`) carry 25 further
uncovered lines. They predate the channel framework and are not part of it; they
are listed here only so the crate-wide number is not mistaken for framework
debt.

## Keeping the guard testable

`start_all` decides each registered channel's fate in `entry_action`, which
takes a definition rather than reaching into the registry. That is what lets a
test exercise "enabled but not built is reported, not silently skipped" without
the build having to ship a planned channel:
`an_enabled_channel_that_is_not_built_is_reported_and_skipped` constructs one.

`pump_lines` takes any `BufRead`, so its read-error arm is covered by
`a_read_error_ends_the_pump_instead_of_retrying_it` rather than by breaking the
test's own stdin.
