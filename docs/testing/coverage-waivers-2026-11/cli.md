# cli (rust, llvm-cov) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The
llvm-cov *summary* percentages are not the acceptance number.

**Lane**: `cargo llvm-cov -p future-cli` (crate `future-cli`, lib + every integration
target), rust **1.99.1** pinned by `rust-toolchain.toml`. The relay lane also runs
`--test relay_nats_auth -- --ignored` against a real `nats-server`
(`FUTURE_RELAY_NATS_SERVER`), and the profile of both passes is merged into one report —
`coverage/2026-11/after/cli/measure7.sh` is the whole thing, `cli-lcov-7.info` the artifact.

**This file's status: 7 rows / 19 lines registered, and it is not a claim of per-line 100 %.** The
lane reads **429 `DA:0`** out of 54 561 lines (99.21 %): 97 of those are one class
(closing tokens, below), 17 are statements registered here, and the remaining **315 are
named work items** with a falsifier each. Nothing in the 429 is silently missing — the
section *Not waived* exists so that "not covered" and "cannot be covered" are never the
same sentence.

## Headline

| | `DA:0` | of |
|---|---:|---:|
| lane start, wave 1 report (`cli-lcov.info`) | **894** | 53 830 |
| wave 2 (`cli-lcov-4.info`, the previous session's last measurement) | **788** | — |
| **this wave (`cli-lcov-7.info`)** | **429** | 54 561 |
| — closing tokens (one class, see below) | 97 | |
| — statements | 332 | |
| — of those, registered as waivers (7 rows, this file) | **17** | |
| — of those, work items named below (not waived) | 315 | |

Per file, wave 2 → this wave:

```
file                                    wave2  now   delta
relay/server.rs                            86     2     -84
relay/cli.rs                               71    15     -56
relay/room.rs                              33     3     -30
commands/remote_peers.rs                  110    78     -32
headless/agent.rs                          66    66      +0   (test added, no line closed — see below)
```

The line total grew by 731 between the two reports: the tests added in this wave are
themselves instrumented (`cli/src/...` includes `#[cfg(test)]` modules). A ratio that moves
down while the numerator falls is that, not a regression.

## How this lane is measured

```bash
cd coverage/2026-11/after/cli
./measure7.sh                     # lib + every integration target, then the nats-only pass, merged
python3 perfile.py cli-lcov-7.info            # per file, highest first
python3 missed-lines.py cli-lcov-7.info relay/server.rs   # DA:0 lines as ranges
python3 classify.py cli-lcov-7.info           # closing-token class vs statements
```

`measure7.sh` pins `HOME` to `target/test-home`, clears `CARGO_BUILD_TARGET`/
`CARGO_TARGET_DIR` (they leak from other sessions' shells) and puts the 1.99.1 toolchain
first on `PATH` (Homebrew's rustup shim shadows it otherwise).

## Rows registered as waivers

Category names are `.future/memory/coverage-llvm-cov.md`'s: W1 platform, **W2 defensive
dead arm**, W3 OS failure injection, W4 race window, W5 attribution artifact, W6 ghost
line, W7 mock closure, W8 dead producer.

| # | file:line | cat | the code that would have to be defeated | falsifier | positive control |
|---|---|---|---|---|---|
| 1 | `relay/cli.rs:519-522` | **process lifetime** | the room-purge task inside `serve`'s runtime: `rooms.purge(now())`, `if dropped > 0`, the count message. The task is spawned immediately before the bind; in the only runs that reach it the bind fails and the runtime is dropped, so it is never polled — and the printing arm needs a *second* tick, i.e. ≥ 60 s of wall clock inside a process that runs until it is killed. | a `RoomStore` whose `purge` returns > 0 at the next tick, in a process that outlives one interval — reachable only by shortening the interval, which is a product change made for the test | `purge_drops_only_rooms_past_their_ttl` (`relay/room.rs`) pins `purge` itself; `serve_announces_*` pins everything up to the bind |
| 2 | `relay/cli.rs:526-528, 530, 532` | **process lifetime** | `axum::serve(listener, app).await?` / `serve_tls(..).await?` and the `Ok(())` after them. Both run until the process is stopped; `serve` installs no shutdown signal, so there is no state in which this function returns. | a graceful-shutdown seam in the product (a `oneshot` receiver and `with_graceful_shutdown`), or a test that kills the process — which also discards the coverage profile of the very lines it exercised | `serve_tls` (the loop's body) is driven end to end over a real TLS handshake by `serve_terminates_tls_with_the_configured_certificate`; the plaintext equivalent is `axum::serve`'s own |
| 3 | `relay/cli.rs:584, 588-590` | **W3** | `serve_tls`'s accept-failure arm and its 100 ms back-off: reached only when `listener.accept()` returns `Err` — in practice `EMFILE`/`ENFILE` during a burst (the code says so). | an `RLIMIT_NOFILE` ceiling low enough that the listener's `accept` fails while the test still holds descriptors — `setrlimit` is in-process and global, so it would have to be a child process | `serve_terminates_tls_with_the_configured_certificate` drives the same loop's success arm, including a refused handshake and a page after it |
| 4 | `relay/cli.rs:421` | **W1** | `path_text`'s `\\?\`-prefix strip: `Some(stripped) => stripped.to_string()`. On macOS `canonicalize` never returns a verbatim path, so the arm cannot execute on this lane's host. | the same lane on Windows (verbatim paths are the default there) | the `None` arm is what every macOS run takes, and the function's output is asserted through `relay init`'s printed setup |
| 5 | `relay/cli.rs:245` | **closing token** | `)?;` at the end of the `nats_conf(..)` call in `init` — the multi-line call's last token. | — (see the class note) | the call's arguments and its output are asserted by `the_generated_nats_config_uses_the_ports_it_advertises` |
| 6 | `relay/server.rs:239` | **W2** | `describe_room`'s `Err(_) => error(500, "internal", …)`: `serde_json::to_value(&room)` over `RoomDescription`, a struct of `String`/`i64`/`bool`/`Vec<DeviceDescription>`. Serializing it cannot fail. | a `RoomDescription` field whose `Serialize` can error (a map with non-string keys, a custom impl) — i.e. a change to the type, not an input | `the_roster_is_only_readable_by_the_desktop_that_owns_the_room` takes the `Ok` arm of the same `match` |
| 7 | `relay/room.rs:168-170` | **W8** | `impl Default for RoomStore` → `Self::new()`. Nothing in the tree constructs it: `Relay::new` uses `RoomStore::new()` and `RoomStore::open(..)` for a served relay. | a caller that needs `RoomStore: Default` (e.g. a `#[derive(Default)]` struct holding one) | `RoomStore::new()`'s own behaviour is pinned by every test in `relay/room.rs` |

The class below is why the 97 are not 97 rows: they are one mechanism, and waiving them
individually would turn one measurement into 97 assertions. Two of the 19 registered lines
(`relay/cli.rs:245`, `relay/cli.rs:522`) are also closing tokens; the class count is
therefore 97 of the 429, not 97 *plus* 19.

> A line whose text is nothing but a closing token (`}`, `})`, `});`, `))`, `));`, `)?;`,
> `}),`, `],`) reads `DA:0` whenever the *construct it closes* is not re-entered on the last
> iteration. llvm-cov attributes the end of a multi-line expression/block to that line, and a
> statement whose only execution is "fall through to the brace" has no counter to increment.

Nothing is lost by excluding it from the numerator's accounting **as a class** — and nothing
is gained by waiving it line by line, which would turn 97 measurements into 97 assertions.
The count is printed by `classify.py` on every run so it cannot grow silently: `97` at this
measurement, `119` at wave 2 (the absolute number fell because the constructs closed above
them are now executed).

## Not waived — reachable, and how

These are work items. Each names the input that would make the line run; none of them is
claimed as unreachable.

| area | `DA:0` | the input that would take it |
|---|---:|---|
| `headless/agent.rs` (`supervise`'s recovery arms) | 66 | see *supervise, conclusion* below |
| `commands/remote_peers.rs` | 78 | `add`'s happy path (618-700) needs a **real pairing peer**: `pair_with_emitter` runs a claim against a platform plus one Noise handshake, so a mock platform is not enough — the other side must answer the handshake. The rest is small: `reach`'s 806, `takeover`'s 862-866/906, `rename`'s 985-992 (see below), `remove`'s declined-prompt arm 1042-1045 (needs a TTY — `ask` refuses without one, which is what `remove_without_yes_refuses_a_prompt_nobody_saw` pins instead), and 1084-1087/1105-1108 |
| `commands/remote.rs` | 31 | eight separate report paths (`remove`/`add`/`headline` arms at 520-5641), each needing a paired device whose credentials a fake platform answers for; `evt`/journal arms at 2404-2779 need a live remote session |
| `commands/data.rs` | 33 | the `restore`/`rollback` failure arms — a prepared restore directory that is unusable mid-way |
| `commands/doctor.rs` | 22 | the platform-reachability arms (a stubbed HTTP endpoint is enough; `test_server::spawn_http` already exists) |
| `prompt.rs` | 15 | `ask_yes_no`'s TTY branches — the same missing terminal as above |
| `relay/server.rs:442` | 1 | a **client revoke against an expired room**: `Relay::new` with `room_ttl_seconds: 1`, register, sleep 1.1 s, `client_revoke` → `RoomError::Expired` (404). Written but not yet added: 1.1 s of wall clock is the only cost |
| `commands/session_journal.rs`, `models.rs`, `session.rs`, `configure.rs`, `mcp_*`, `browser/*` | 15+12+11+7+18+14 | one refusal/report arm each; none of them needs a new harness |

### `supervise`, conclusion (the todo's item 5)

**Neither a probe seam nor a waiver.** `supervise` is a `match` over
`probe_local_agent(..)` plus `host()` — the process-global `AgentHost` singleton — and its
arms are gated on **real time**: `ReadinessLoss::confirmed` needs 3 failed probes *and* 15 s,
`UNSEEN_TOLERANCE` is 3 s, `WAIT_FOR_OWNER` is a minute. A probe seam would not help: the
seam only replaces the probe, while each arm also asks `host().owns_agent()`,
`budget.allow(now)` and `recover(address)`, so the test would still need a host it owns, a
real Agent to lose and real seconds to wait. A probe seam is therefore *more* product code
for the same missing fixture.

The honest route already exists in this file: `block_fixtures` re-execs the libtest binary
with its own `HOME`/`FUTURE_HOME`/socket and asserts inside the child, because a failed start
poisons the global. The same fixture, driving `supervise` instead of `check_running`, covers
the `Startable` → `repair` → owned-Agent path in ~5 s of real time and the `Busy`/wait arms
with a second child holding the instance lock. That is the next work item; it is not
registered as a waiver.

What **is** covered now: the ordinary round — `a_live_agent_is_observed_and_never_started_twice`
drives `supervise` against the mock Agent, asserts the probe answered `Attached` before
calling it (so a vanished fixture fails there instead of starting an Agent in the test
process), and asserts supervision never takes ownership of an Agent another process serves.
That test closed **no line** (the `Attached` arm was already exercised); it closed a hole in
what the suite *asserts*, which is why it is listed as +0 in the per-file table rather than
counted as progress.

## Two product observations made while writing these tests

Neither is a coverage matter, and neither was fixed here (no product semantics were changed
in this lane):

1. `remote peers remove --json` prints the JSON document **and then** the sentence "… is no
   longer paired with this machine" on the Desktop-removed path, while every other `--json`
   path returns after the document. A script that parses `stdout` as one JSON document
   breaks on that branch. The test reads the first document with
   `serde_json::Deserializer::into_iter()` and says why.
2. `serve` installs no shutdown signal, so the relay cannot be stopped except by a signal
   whose handler does not exist. That is what makes rows 2 and 3 above unmeasurable, and it
   is worth a decision independent of coverage.

## What this file cannot claim

- **Not per-line 100 %.** 429 lines read 0. The lane's compliance statement is the one
  `docs/testing/coverage-waivers-2026-11/README.md` makes for the web lane: no silent gap
  (97 one-class + 17 registered statements + 315 work items named above = 429), no stale row
  (each row in the table was re-read against `cli-lcov-7.info`), no product change made for
  coverage (no seam, no `pub`, no signature).
- **No stale exemptions.** If a later wave covers a registered line, the row is wrong rather
  than old and must be deleted, not reworded: `classify.py` prints the registered lines'
  current counts on every run, so the check is one command.
