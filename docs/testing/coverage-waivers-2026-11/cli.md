# cli (rust, llvm-cov) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The
llvm-cov *summary* percentages are not the acceptance number.

**Lane**: `cargo llvm-cov -p future-cli` (crate `future-cli`, lib + every integration
target), rust **1.99.1** pinned by `rust-toolchain.toml`. The relay lane also runs
`--test relay_nats_auth -- --ignored` against a real `nats-server`
(`FUTURE_RELAY_NATS_SERVER`), and the profile of both passes is merged into one report —
`coverage/2026-11/after/cli/measure10.sh` is the whole thing, `cli-lcov-10.info` the artifact.

**This file's status: 7 rows / 19 lines registered, and it is not a claim of per-line 100 %.** The
lane reads **355 `DA:0`** out of 54 882 lines (99.35 %): 86 of those are one class
(closing tokens, below), 17 are statements registered here, and the remaining **252 are
named work items** with a falsifier each. Nothing in the 355 is silently missing — the
section *Not waived* exists so that "not covered" and "cannot be covered" are never the
same sentence.

## Headline

| | `DA:0` | of |
|---|---:|---:|
| lane start, wave 1 report (`cli-lcov.info`) | **894** | 53 830 |
| wave 2 (`cli-lcov-4.info`) | **788** | — |
| wave 3 (`cli-lcov-7.info`) | **429** | 54 561 |
| **this wave (`cli-lcov-10.info`)** | **355** | 54 882 |
| — closing tokens (one class, see below) | 86 | |
| — statements | 269 | |
| — of those, registered as waivers (7 rows, this file) | **17** | |
| — of those, work items named below (not waived) | 252 | |

Per file, wave 3 → this wave (top of the list only; `delta.py cli-lcov-7.info cli-lcov-10.info`
prints the whole table and both totals):

```
file                                    w3     now   delta
headless/agent.rs                          66     14     -52
commands/data.rs                           33     21     -12
commands/doctor.rs                         22     12     -10
relay/server.rs                             2      1      -1
headless/mod.rs                            41     42      +1   (attribution, named below)
commands/remote_peers.rs                   78     78      +0
commands/remote.rs                         31     31      +0
relay/cli.rs                               15     15      +0
```

The line total grew by 321 between wave 3 and this wave: the tests added in this wave are
themselves instrumented (`cli/src/...` includes `#[cfg(test)]` modules), so a falling
numerator with a growing denominator is that, not a regression. The one row that grew is a
test's own `panic!` failure arm (`headless/mod.rs:1156`) whose count flipped between two runs
in which the test passed both times — it is named in the *Not waived* table.

## How this lane is measured

```bash
cd coverage/2026-11/after/cli
./measure10.sh                                # lib + every integration target, then the nats-only pass
python3 perfile.py cli-lcov-10.info           # per file, highest first
python3 missed-lines.py cli-lcov-10.info relay/server.rs   # DA:0 lines as ranges
python3 classify.py cli-lcov-10.info          # closing-token class vs statements
python3 delta.py cli-lcov-7.info cli-lcov-10.info w3 w10   # per-file movement between two waves
```

`measure10.sh` pins `HOME` to `target/test-home`, clears `CARGO_BUILD_TARGET`/
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

The class below is why the 86 are not 86 rows: they are one mechanism, and waiving them
individually would turn one measurement into 86 assertions. Two of the 19 registered lines
(`relay/cli.rs:245`, `relay/cli.rs:522`) are also closing tokens; the class count is
therefore 86 of the 355, not 86 *plus* 19.

All 19 registered lines were re-read against `cli-lcov-10.info` when this wave landed:
every one still reads `DA:0`, so no row is stale. (Two of them moved *out* of this file's
scope in this wave and one was deleted rather than reworded — see
*Rows this wave closed* below.)

> A line whose text is nothing but a closing token (`}`, `})`, `});`, `))`, `));`, `)?;`,
> `}),`, `],`) reads `DA:0` whenever the *construct it closes* is not re-entered on the last
> iteration. llvm-cov attributes the end of a multi-line expression/block to that line, and a
> statement whose only execution is "fall through to the brace" has no counter to increment.

Nothing is lost by excluding it from the numerator's accounting **as a class** — and nothing
is gained by waiving it line by line, which would turn 86 measurements into 86 assertions.
The count is printed by `classify.py` on every run so it cannot grow silently: `86` at this
measurement, `97` at wave 3, `119` at wave 2 (the absolute number fell because the
constructs closed above them are now executed).

## Rows this wave closed

Deleted from the work-item list, not reworded — each is now `DA:1` and each has a test that
names the input that reaches it:

| was | now covered by |
|---|---|
| `relay/server.rs:442` — a client revoke against an **expired room** (1 line, and with it the last line in that file) | `a_revoke_against_an_expired_room_says_the_room_is_gone`: `relay_with_room_ttl(1)` + a 1.1 s wait. See the wall-clock note below. |
| `headless/agent.rs` 66 lines — `supervise`'s `Startable`→`repair`, `Busy`/wait arms, both long-wait exits | three new `block_fixtures` children (`startable`, `waiting`, `blocked`); 52 lines. |
| `commands/doctor.rs` 10 lines — the zero-session sentence, the damaged-storage counters and the `Issue` status | `zero_sessions_in_the_database_is_its_own_answer`, `storage_damage_is_an_issue_that_names_every_counter`. |
| `commands/data.rs` 12 lines — the manifest-path walk and the link-as-preimage digest | `a_manifest_path_below_a_file_is_refused_by_the_filesystem`, `a_linked_preimage_is_hashed_as_a_link_and_not_followed`. |

### The relay expiry test's one wall-clock assumption

`room_ttl_seconds: 1` and a 1.1 s sleep are the only wall clock this file depends on, and
they are on the correct side of it rather than racing it: `expires_at = t + 1` for the whole
second `t` the registration lands in, the refusal fires at `now >= expires_at`, and `now()`
is whole seconds — so `floor(t + 1.1) >= t + 1` for **every** `t`. The sleep is a lower bound
(tokio never returns early), so a loaded host widens the margin; nothing retries or polls,
so there is no window in which a slower run flips the result. What would make it a flake is
a clock that could return early or a fractional `now()` — neither is in this code.

## Not waived — reachable, and how

These are work items. Each names the input that would make the line run; none of them is
claimed as unreachable.

| area | `DA:0` | the input that would take it |
|---|---:|---|
| `commands/remote_peers.rs` | 78 | `add`'s happy path (618-700) needs a **real pairing peer**: `pair_with_emitter` runs a claim against a platform plus one Noise handshake, so a mock platform is not enough — the other side must answer the handshake. See *Change lane* below. The rest is small: `reach`'s 806, `takeover`'s 862-866/906, `rename`'s 985-992 (see below), `remove`'s declined-prompt arm 1042-1045 (needs a TTY — `ask` refuses without one, which is what `remove_without_yes_refuses_a_prompt_nobody_saw` pins instead), and 1084-1087/1105-1108 |
| `headless/mod.rs` | 42 | the entry's console report lines, one per phase (`host log unavailable` 128, the stop reasons 249/302-306, the remote notices 656/951), and a refusal arm beside each (334, 523, 740, 857). None needs a new harness; each needs the phase it reports. One of the 42 is not code: 1156 is a test's `panic!` failure arm that read `1` in wave 3 and `0` here **while the test passed in both runs** (`run7.log:1584`, `run10.log:1596`) — the +1 in the per-file table is that line, and it is an attribution artifact (W5), not a regression. |
| `commands/remote.rs` | 31 | eight separate report paths (`remove`/`add`/`headline` arms at 520-5641), each needing a paired device whose credentials a fake platform answers for; `evt`/journal arms at 2404-2779 need a live remote session |
| `commands/data.rs` | 21 | **4 statements** are the `Err(error) => return Err(error.into())` arms of `symlink_metadata` in `digest`/`collect`/`inventory`/`work` (293, 359, 591, 638) — **W3**: reached when the OS fails with something other than `NotFound` (`EIO`, `ESTALE`, `ENAMETOOLONG`). Permission denial does **not** reach them, and that was measured, not assumed: with the parent at `0o444` macOS still resolves `lstat` of a child, `read_dir` opens, and the *iteration* is what fails — a line `every_move_boundary…` already covers. The other **17** are closing tokens. |
| `commands/doctor.rs` | 12 | **W1 + W2**, and this reverses wave 3's plan for the file: the 22 lines were never the platform-reachability arms, they are the **OS-sandbox probe** arms. On macOS `platform_sandbox_probe_product()` always returns `Ok`, always reports `version: None`, and its `code` is only `available`/`binary_missing` — so `Status::Warn` (173) needs `/usr/bin/sandbox-exec` **absent**, `version={version}` (183) can never print, `available_write_only` (186) and `sandbox_remedy` (189) are Linux-only, and the `Err` arm (197-201) has no macOS producer. The remaining two are defensive: an empty key that the accessor has already normalised to `None` (140) and a `Stdio::piped` pair that is piped by construction (387), plus one `join` error from a copy task that cannot panic (419). |
| `relay/cli.rs` | 15 | registered above (rows 1-5): process lifetime, W3, W1 |
| `commands/session_journal.rs`, `models.rs`, `session.rs`, `configure.rs`, `mcp_*`, `browser/*`, `prompt.rs` | 15+12+11+7+18+14+15 | one refusal/report arm each; `prompt.rs`'s TTY branches are the one group that needs a terminal |

### `supervise` — the wave-3 plan, executed

Wave 3 left this as a work item with a plan; this wave ran it. The three children live in
`headless::agent::block_fixtures` and every arm they drive is a **real** transition, not a
setter:

* `startable` — a free lock and no answer. Two rounds open the grace window
  (`UNSEEN_TOLERANCE`, the product's 3 s), the third `repair`s into a real embedded Agent;
  the assertions are `owns_agent()` and `Phase::Ready`, i.e. an Agent that serves.
* `waiting` — another process holds the lock. The wait is recorded once and not restarted
  per round; a wait past `WAIT_NOTICE_AFTER` prints why; a wait past `WAIT_FOR_OWNER` is
  refused by name. The two long arms **age the recorded start** instead of sleeping 30 s and
  180 s — the probe still runs for real against a lock this process holds, and the aged
  `Instant` is the same state a long wait produces.
* `blocked` — an explicit, unreachable endpoint is returned to the caller as the probe
  described it, and no Agent is started behind it.

What is left in the file (14 lines): `check_running`'s `Failed`-and-restart arm (43), the
*owned*-Agent `Busy`/`Unresponsive` repair (79-84, needs an Agent this host owns to stop
answering), restart-budget exhaustion (124, needs several real restarts to burn the budget),
the two `recovery_requires_restart` arms (146, 151), and the fixture's own 120 s timeout
handler (394-395, 399) which is unreachable while the children pass.

## Change lane — the three groups that need something other than a bigger test

Wave 3's todo asked for the `remote peers add` happy path and then withdrew it to its own
item, correctly: 618-700 of `commands/remote_peers.rs` cannot be closed by writing a test in
this lane. What follows is what a future lane would have to *add*, with the code that says
so, so nobody re-derives it.

1. **`remote peers add` (618-700, ~30 of the 78).** `add` calls
   `future_host::remote_peer::runtime::pair_with_emitter(&invitation, None)`, which is a claim
   against the platform **plus one Noise handshake with the other machine**: the other side
   must answer with its own key. A mocked platform answers the HTTP half and nothing else, so
   the handshake fails and the rest of the command never runs.
   The fixture exists, but not in this crate: `host::remote_peer::testing`
   (`start(..)`, `MockPlatform`, the in-process fake broker, `invitation_for`) is gated on
   `#[cfg(any(test, feature = "test-support"))]` and already drives this exact call end to end
   in `host/src/remote_peer/runtime_tests.rs:104`. Consuming it here means turning on
   `future-host/test-support` in `cli`'s **dev-dependencies** and running a real host bridge
   inside the cli test process — which is a decision about the lane (a product crate gaining a
   fixture seam in its dev-deps), not a test to write. Two further hazards if it is taken:
   the bridge installs the process-global host runtime, and `HostGuard`/`HomeGuard` mutate
   `HOME`, so it has to sit behind `test_env::lock_env()` and never overlap another case.
2. **`prompt.rs` (15) and `remove`'s declined-prompt arm.** `ask_yes_no` reads a terminal;
   without one it refuses, and that refusal is what the existing test pins instead. Closing
   the interactive arms needs a pty — a different harness, and one `cli` has no other reason
   to have.
3. **`remote.rs`'s `evt`/journal arms (2404-2779).** They report a **live** remote session.
   The same `host::remote_peer::testing` bridge would supply one, so this is the second
   consumer of item 1 rather than a third problem.

What is *not* in this list: `headless/agent.rs`, `commands/data.rs` and `commands/doctor.rs`
were on it in wave 3 and are now measured (74 of their 121 lines closed).

## Two product observations made while writing these tests

Neither is a coverage matter, and neither was fixed here (no product semantics were changed
in this lane):

1. `remote peers remove --json` prints the JSON document **and then** the sentence "… is no
   longer paired with this machine" on the Desktop-removed path, while every other `--json`
   path returns after the document. A script that parses `stdout` as one JSON document
   breaks on that branch. The test reads the first document with
   `serde_json::Deserializer::into_iter()` and says why.
2. `serve` installs no shutdown signal, so the relay cannot be stopped except by a signal
   whose handler does not exist. That is what makes registered rows 2 and 3 unmeasurable, and
   it is worth a decision independent of coverage.

## What this file cannot claim

- **Not per-line 100 %.** 355 lines read 0. The lane's compliance statement is the one
  `docs/testing/coverage-waivers-2026-11/README.md` makes for the web lane: no silent gap
  (86 one-class + 17 registered statements + 252 work items named above = 355), no stale row
  (each of the 19 registered lines was re-read against `cli-lcov-10.info`), no product change
  made for coverage (no seam, no `pub`, no signature — the `test-support` option in *Change
  lane* is exactly the decision that was **not** taken here).
- **No stale exemptions.** If a later wave covers a registered line, the row is wrong rather
  than old and must be deleted, not reworded: `classify.py` prints the registered lines'
  current counts on every run, so the check is one command.
- **Two of this wave's own claims were falsified before they were kept**, and the file says so
  rather than hiding the retries: the long-wait notice is asserted on the child's stderr only
  because swapping the expected string for a sentinel makes the test fail, and two
  permission-based data tests were **written, run, and deleted** once they proved to reach
  `entry?` instead of the `lstat` arms they were meant for (see the W3 row above).
