# cli (rust, llvm-cov) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The
llvm-cov *summary* percentages are not the acceptance number.

**Lane**: `cargo llvm-cov -p future-cli` (crate `future-cli`, lib + every integration
target), rust **1.99.1** pinned by `rust-toolchain.toml`. The relay lane also runs
`--test relay_nats_auth -- --ignored` against a real `nats-server`
(`FUTURE_RELAY_NATS_SERVER`), and the profile of both passes is merged into one report —
`coverage/2026-11/after/cli/measure11.sh` is the whole thing, `cli-lcov-11.info` the artifact.

**This file's status: 9 rows / 33 lines registered, and it is not a claim of per-line 100 %.** The
lane reads **288 `DA:0`** out of 54 892 lines (99.48 %): 80 of those are one class
(closing tokens, below), 31 are statements registered here, and the remaining **177 are
named work items** with a falsifier each. Nothing in the 288 is silently missing — the
section *Not waived* exists so that "not covered" and "cannot be covered" are never the
same sentence.

## Headline

| | `DA:0` | of |
|---|---:|---:|
| lane start, wave 1 report (`cli-lcov.info`) | **894** | 53 830 |
| wave 2 (`cli-lcov-4.info`) | **788** | — |
| wave 3 (`cli-lcov-7.info`) | **429** | 54 561 |
| wave 10 (`cli-lcov-10.info`) | **355** | 54 882 |
| **this wave (`cli-lcov-11.info`)** | **288** | 54 892 |
| — closing tokens (one class, see below) | 80 | |
| — statements | 208 | |
| — of those, registered as waivers (9 rows, this file) | **31** | |
| — of those, work items named below (not waived) | 177 | |

Per file, wave 10 → this wave (top of the list only; `delta.py cli-lcov-10.info
cli-lcov-11.info w10 w11` prints the whole table and both totals):

```
file                                    w10    w11   delta
commands/remote_peers.rs                   78     29     -49
headless/mod.rs                            42     24     -18
commands/remote.rs                         31     31      +0
prompt.rs                                  15     15      +0   (now a registered row, not a work item)
commands/data.rs                           21     21      +0
relay/cli.rs                               15     15      +0
commands/session_journal.rs                15     15      +0
```

The line total grew by 10 between wave 10 and this wave: the tests added in this wave are
themselves instrumented (`cli/src/...` includes `#[cfg(test)]` modules), so a falling
numerator with a growing denominator is that, not a regression.

## How this lane is measured

```bash
cd coverage/2026-11/after/cli
./measure11.sh                                # lib + every integration target, then the nats-only pass
python3 perfile.py cli-lcov-11.info           # per file, highest first
python3 missed.py cli-lcov-11.info relay/server.rs /path/to/cli/src/relay/server.rs
                                              # the DA:0 lines of one file, with their text
python3 classify.py cli-lcov-11.info          # closing-token class vs statements
python3 delta.py cli-lcov-10.info cli-lcov-11.info w10 w11   # per-file movement between two waves
python3 waiver_check.py cli-lcov-11.info "relay/cli.rs:519-522,526-528,530,532,584,588-590,421,245" \
    "relay/server.rs:239" "relay/room.rs:168-170" "prompt.rs:37,59,67-74,104,127,140,151"
                                              # every registered row, re-read against this report
```

`measure11.sh` pins `HOME` to `target/test-home`, clears `CARGO_BUILD_TARGET`/
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
| 8 | `prompt.rs:37, 59, 67-74` | **W2 + W4** | the *line* fallback: `if !stdin().is_terminal() { return ask_line(..) }`, the terminal-refused-the-mode fallthrough, and `ask_line`'s body. The only caller (`remote::ask`) checks the same predicate first and **refuses** a redirected stdin — deliberately, so "the answer to a prompt nobody saw" is never read — which makes both the fallback and the function it calls unreachable from this tree. | a second caller of `prompt::ask_yes_no` that does not pre-check; or stdin ceasing to be a terminal between `remote::ask`'s check and `ask_yes_no`'s (microseconds apart, and the reason the arm is kept) | `remove_without_yes_refuses_a_prompt_nobody_saw` pins the refusal that makes it unreachable — a piped answer is refused, not read; the pty fixtures pin the terminal path this file's module doc is about |
| 9 | `prompt.rs:104, 127, 140, 151` | **W2 + W3** | `read_keys`' end-of-input `return false` (the pty master closing under a raw read), and `Raw::enable`/`Raw::read`'s failures: `tcgetattr` != 0, `tcsetattr` != 0, `read` != 1. | an `EIO` from `read(2)` on stdin, or a `termios` call that fails on a descriptor `is_terminal()` just accepted — both are the OS refusing a descriptor it just blessed | every pty fixture (`remote_unpair_prompt.rs`) drives `read_keys`' success arms: a decided key, an ignored key, an Enter; `only_the_letters_decide` pins `decide` itself |

> A line whose text is nothing but a closing token (`}`, `})`, `});`, `))`, `));`, `)?;`,
> `}),`, `],`) reads `DA:0` whenever the *construct it closes* is not re-entered on the last
> iteration. llvm-cov attributes the end of a multi-line expression/block to that line, and a
> statement whose only execution is "fall through to the brace" has no counter to increment.

Nothing is lost by excluding it from the numerator's accounting **as a class** — and nothing
is gained by waiving it line by line, which would turn 80 measurements into 80 assertions.
The count is printed by `classify.py` on every run so it cannot grow silently: `80` at this
measurement, `86` at wave 10, `97` at wave 3, `119` at wave 2.

All 33 registered lines were re-read against `cli-lcov-11.info` when this wave landed
(`waiver_check.py`, `stale=0`), so no row is stale. Two of the 19 relay rows
(`relay/cli.rs:245`, `relay/cli.rs:522`) are also closing tokens, which is why the class
count is 80 of the 288 and not 80 plus 33.

## Rows this wave closed

Deleted from the work-item list, not reworded — each is now `DA:1` and each has a test that
names the input that reaches it:

| was | now covered by |
|---|---|
| `commands/remote_peers.rs` 78 → 29 (49 lines) — `add`'s pairing and its report (638-700), `takeover` on an empty book (862-866), `remove` on a Desktop that holds the lock and never answers (1084-1087), and the row a Desktop reports that the book does not hold (545) | `add_says_which_machine_it_paired_and_who_connects_it` — a **real** pairing: a host bridge on the in-process broker, a claim the mock platform grants, one Noise handshake, and the book written where the next start reads it. Plus `takeover` in `an_action_on_an_empty_book_names_the_paired_command`, `remove_proceeds_here_when_the_desktop_never_answers`, `list_keeps_a_pairing_the_book_and_the_desktop_disagree_about`. |
| `headless/mod.rs` 42 → 24 (18 lines) — the switch-off report (382/387), the idle notice (511-517), the refused-key refusal (326/329/330) and the transport failure while verifying (334) | `a_switched_off_remote_desktop_stays_up_and_comes_back_without_a_restart` (one run: the switch is off, nothing is served, and `future remote enable` from another terminal starts serving **without a restart**), `a_stored_key_the_platform_refuses_leaves_the_machine_serving_locally`, `an_unreachable_platform_while_verifying_is_reported_as_a_failure`. |
| `headless/mod.rs` 249 + 951-953 — the loop's own stop cause | `a_loop_that_returns_is_its_own_stop_cause`: the session arm of `until_shutdown` returns `StopCause::Session`, whose reason is `headless_exit` and whose terminal words are "an internal stop" — not one of the three signal names. |

## The `future-host/test-support` decision — taken, in this wave

Wave 10 left this open and framed it as a lane decision. It is settled:

1. **The feature was already on.** `cli`'s `[dev-dependencies]` has carried
   `future-host = { features = ["test-support"] }` since `a5f91752b` (the host extraction),
   and this crate already consumed it (`test_support::configure_entry_fixture`,
   `test_support::agent::mock_agent`). Using `remote_peer::testing` adds no dependency, no
   feature, no `pub` and no seam: the product source is untouched, which is why it is not
   "changing the product for a test". The whole diff is test code.
2. **It works, and it was worth 49 lines** in `remote_peers.rs` alone — `add`'s claim and
   handshake cannot be faked, and a mocked platform answers only the HTTP half.
3. **Three properties of the fixture cost real time and are now recorded** where a reader
   will hit them (`cli/src/commands/remote_peers/tests.rs`, above `Served`):
   * `testing::fixture` **claims** the bridge for a device; a second claim against the same
     bridge answers `peer_mismatch`. `add` *is* the claim, so the fixture has to be rebuilt
     one step earlier (`start_host` without `claim`).
   * **One bridge per process.** `add` shuts the peer runtime down before it returns (it is
     a one-shot command), and a second `start_host` in the same process then fails. That is
     why the `--json` branch of the same command has no test here: it would need a second
     bridge, i.e. a second test binary.
   * **The runtime the bridge spawns on must outlive the test that installed it.**
     `future_host::runtime::set` is a `OnceLock`, so a test-scoped handle stays installed and
     goes dead with the test. Measured: with a test-scoped handle, a full lib run failed
     three `headless::tests` CLI-discovery probes with "discovery timed out"; with a leaked
     process-lifetime runtime the same run passes 1532/1532. The test installs the latter and
     says so.

## Not waived — reachable, and how

These are work items. Each names the input that would make the line run; none of them is
claimed as unreachable.

| area | `DA:0` | the input that would take it |
|---|---:|---|
| `commands/remote.rs` | 31 | eight separate report paths (`remove`/`add`/`headline` arms at 520-5641), each needing a paired device whose credentials a fake platform answers for; `evt`/journal arms at 2404-2779 need a live remote session. The same `host::remote_peer::testing` bridge taken above is what supplies one — this is now the **next** consumer of it, not a third problem. |
| `commands/remote_peers.rs` | 29 | three groups. (a) `add --json` (668-672): needs a second bridge in the same process — see the decision above, so the branch belongs to a new test *binary*. (b) `add`'s two remaining report arms (684-692, 695): a pairing whose connection came up *disconnected* (the `live.error` sentence), and one with a Desktop running to take it over — each needs its own bridge. (c) `rename`'s 985-992 and `remove`'s 1105-1108: `Book::edit` re-loads the file and finds the row again, so the "no longer paired" arm needs a concurrent writer between two reads in one function (W4), and `unpair` returns a *warning* rather than an `Err` when only the platform fails — the `Err` arm needs the client's own book write to fail. |
| `headless/mod.rs` | 24 | five groups. (a) 128: `start_logging` failing — a read-only data root or a full disk, while the entry keeps serving. (b) 302/306: a `--relay` launch (the "no Future account is used" line, and the `None` account it decides). (c) 451/456: a login adopted while the switch is **off** (adopted, but `remote::status()` instead of a start). (d) 488-495: the `NeedsAccount` notice — `Failed(AccountAuthorization)` on a run with no account. Both CLI commands that could ask for a start refuse before they reach the host (`future remote enable` and `future remote devices add` each say "sign in first"), so today the producer is the **window's** Remote page through the control channel; the CLI cannot reach it. (e) 523 (`Stopped` with the switch on and something to serve), 656-658 (a model-catalogue refresh the *Agent* refuses), 857-859 (`stop`'s runtime build failing — W2), and 951-953/1178, which are two test-side arms: the closure body a "startup was dropped" test must never poll, and a fixture-discovery `panic!`. |
| `commands/data.rs` | 21 | **4 statements** are the `Err(error) => return Err(error.into())` arms of `symlink_metadata` in `digest`/`collect`/`inventory`/`work` (293, 359, 591, 638) — **W3**: reached when the OS fails with something other than `NotFound` (`EIO`, `ESTALE`, `ENAMETOOLONG`). Permission denial does **not** reach them, and that was measured, not assumed: with the parent at `0o444` macOS still resolves `lstat` of a child, `read_dir` opens, and the *iteration* is what fails — a line `every_move_boundary…` already covers. The other **17** are closing tokens. |
| `commands/doctor.rs` | 12 | **W1 + W2**: the **OS-sandbox probe** arms. On macOS `platform_sandbox_probe_product()` always returns `Ok`, always reports `version: None`, and its `code` is only `available`/`binary_missing` — so `Status::Warn` (173) needs `/usr/bin/sandbox-exec` **absent**, `version={version}` (183) can never print, `available_write_only` (186) and `sandbox_remedy` (189) are Linux-only, and the `Err` arm (197-201) has no macOS producer. The remaining two are defensive: an empty key the accessor has already normalised to `None` (140) and a `Stdio::piped` pair that is piped by construction (387), plus one `join` error from a copy task that cannot panic (419). |
| `relay/cli.rs` | 15 | registered above (rows 1-5): process lifetime, W3, W1 |
| `commands/session_journal.rs`, `models.rs`, `session.rs`, `configure.rs`, `mcp_*`, `browser/*`, `headless/agent.rs` | 15+12+11+7+18+14+14 | one refusal/report arm each. `headless/agent.rs`'s 14 are named in the `supervise` section below; `session_journal.rs`'s are a journal the real Agent writes. |

### `supervise` — the wave-3 plan, executed in wave 10

The three children live in `headless::agent::block_fixtures` and every arm they drive is a
**real** transition, not a setter:

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

### The relay expiry test's one wall-clock assumption

`room_ttl_seconds: 1` and a 1.1 s sleep are the only wall clock this file depends on, and
they are on the correct side of it rather than racing it: `expires_at = t + 1` for the whole
second `t` the registration lands in, the refusal fires at `now >= expires_at`, and `now()`
is whole seconds — so `floor(t + 1.1) >= t + 1` for **every** `t`. The sleep is a lower bound
(tokio never returns early), so a loaded host widens the margin; nothing retries or polls,
so there is no window in which a slower run flips the result. What would make it a flake is
a clock that could return early or a fractional `now()` — neither is in this code.

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
3. `prompt`'s module doc says "a redirected stdin still reads a line, which keeps `printf 'y\n'
   | future …` scripts working" — but the only caller refuses a redirected stdin before it
   calls `ask_yes_no` (`remote::ask`), and a test pins that refusal. One of the two sentences
   is wrong; the refusal is the deliberate one (rows 8 and 9 are waivers because of it), so
   the module doc is the statement to fix.

## What this file cannot claim

- **Not per-line 100 %.** 288 lines read 0. The lane's compliance statement is the one
  `docs/testing/coverage-waivers-2026-11/README.md` makes for the web lane: no silent gap
  (80 one-class + 31 registered statements + 177 work items named above = 288), no stale row
  (all 33 registered lines re-read against `cli-lcov-11.info` by `waiver_check.py`), no
  product change made for coverage (no seam, no `pub`, no signature — the `test-support`
  decision taken in this wave is a dev-dependency this crate already had, and its whole diff
  is test code).
- **No stale exemptions.** If a later wave covers a registered line, the row is wrong rather
  than old and must be deleted, not reworded: `classify.py` prints the registered lines'
  current counts on every run, so the check is one command.
- **Two of this lane's own claims were falsified before they were kept**, and the file says
  so rather than hiding the retries: the long-wait notice is asserted on the child's stderr
  only because swapping the expected string for a sentinel makes the test fail, and two
  permission-based data tests were **written, run, and deleted** once they proved to reach
  `entry?` instead of the `lstat` arms they were meant for. In this wave, the first version
  of the `add` test drove **two** pairings in one process and failed with
  `remote_client_stopped`; the cause was not the command but the fixture's one-bridge rule,
  and the test is now one pairing with that rule written down where it bit.
