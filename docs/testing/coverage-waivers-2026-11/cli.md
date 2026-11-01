# cli (rust, llvm-cov) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The
llvm-cov *summary* percentages are not the acceptance number.

**Lane**: `cargo llvm-cov -p future-cli` (crate `future-cli`, lib + every integration
target), rust **1.99.1** pinned by `rust-toolchain.toml`. The relay lane also runs
`--test relay_nats_auth -- --ignored` against a real `nats-server`
(`FUTURE_RELAY_NATS_SERVER`), and the profile of both passes is merged into one report —
`coverage/2026-11/after/cli/measure15.sh` is the whole thing, `cli-lcov-15.info` the artifact.

**This file's status: 19 rows / 62 lines registered, and it is not a claim of per-line 100 %.** The
lane reads **250 `DA:0`** out of 55 163 lines (99.55 %): 62 are registered here, **8 await a
product decision** (a section of their own, below), and **180 are work items** with a
falsifier each. By line shape the 250 divide into **76 closing tokens** (one class, below: 5 of
them sit inside registered rows, 2 in the decision block, 69 in work items) and **174
statements** (57 registered, 6 decision, 111 work items). Nothing in the 250 is silently
missing — the section *Not waived* exists so that "not covered" and "cannot be covered" are
never the same sentence.

Wave 15 closes **nine** lines with tests, registers **five** as a dead accessor, and inherits
**two** from another lane:

| what | lines | how |
|---|---:|---|
| `relay/config.rs` **4 → 0** | 116, 119, 127, 132 | three tests over `validate()`: a whitespace seed, a TTL of zero, and a certificate field that is set but empty |
| `relay/sign.rs` **2 → 0** | 80, 82 | one handler test over the roster route: a signature header that is not base64 (`signature_malformed`), and one made by another key (`signature_invalid`) — both leave through `VerifyError::code`, which only `Missing`/`Stale` had reached |
| `commands/data.rs` **21 → 19** | 638 + 1 closing token | a restore into a home nested past `PATH_MAX`, so `<home>/.restore-work-<id>` is the one path the filesystem refuses |
| `headless/mod.rs` **18 → 17** | 451 | a launch: the switch is off, a login appears under the running host, and the adopt arm reports `remote::status()` instead of starting anything |
| `relay/jwt.rs` **5, now registered** | 45-48, 50 | `Role::as_str` has no caller in this tree — row 19, a W8 row, not a test |
| `browser/safari/safari_manager.rs` **11 → 13** | 287, 357, 460-469, 625, 1110-1111, 1117, 1132 | **another lane's**: the flake workstream's rewrite (`5b6b574f5`, `4887e65fb`) landed after wave 14's report was taken, and every added line is a diagnostic arm that runs only when a launch has already failed |

The arithmetic is therefore −9 covered by this wave, +2 inherited, −1 closing token that the
long-home restore also re-enters: 257 → 250.

Wave 14 closes six lines of `headless/mod.rs` (24 → 18) with two **launch** arms rather than
any lib test: a `--relay` run, and a model-catalogue refresh the *Agent* refuses
(§*Two launch arms that had never run*, below). Both are integration targets, so they add no
instrumented lines of their own — every one of the six is a line a product path now really
executes. Three more come off `browser/safari/safari_manager.rs` (14 → 11) from **another
lane's** rewrite of the two launch tests (`0900e710c`, merged into the base this wave was
finally measured on); those three are that lane's test-side arms, and this file lists them
only so the total is not read as this wave's doing.

## Headline

| | `DA:0` | of |
|---|---:|---:|
| lane start, wave 1 report (`cli-lcov.info`) | **894** | 53 830 |
| wave 2 (`cli-lcov-4.info`) | **788** | — |
| wave 3 (`cli-lcov-7.info`) | **429** | 54 561 |
| wave 10 (`cli-lcov-10.info`) | **355** | 54 882 |
| wave 11 (`cli-lcov-11.info`) | **288** | 54 892 |
| wave 12 (`cli-lcov-12.info`) | **284** | 54 893 |
| wave 13 (`cli-lcov-13.info`) | **266** | 55 051 |
| **wave 14 (`cli-lcov-14.info`)** | **257** | 55 033 |
| **this wave (`cli-lcov-15.info`)** | **250** | 55 163 |
| — registered here (19 rows, 62 lines) | 62 | |
| — of those 62, closing tokens | 5 | |
| — awaiting a product decision (below) | 8 | |
| — work items named below (69 of them closing tokens) | 180 | |

Per file, wave 13 → this wave (top of the list only; `delta.py cli-lcov-14.info
cli-lcov-15.info w14 w15` prints the whole table and both totals):

```
file                                    w14    w15   delta
commands/remote.rs                        31     31      +0
commands/remote_peers.rs                  20     20      +0
commands/data.rs                          21     19      -2   (the restore work directory + one closing token)
headless/mod.rs                           18     17      -1   (451, the login adopted while the switch is off)
browser/safari/safari_manager.rs           11     13      +2   (another lane's rewrite, 5b6b574f5/4887e65fb)
relay/config.rs                             4      0      -4   (three validate() tests)
relay/sign.rs                               2      0      -2   (the two codes on the wire)
```

This wave was measured **once**, on the base wave 14 merged into (`origin/dev-huichen` at
`2b9e9ef36`). The `DA:0` count moved 257 → 250 and the instrumented total 55 033 → 55 163:
the 130 extra lines are the new tests' own bodies (they live inside the lib, so they are
instrumented) plus the Safari rewrite named above. The net **−7** is **−9** this wave's own and
**+2** inherited from the other lane; no line of this wave's product code was written, and no
line of it was renumbered.

Wave 13 was measured twice, because the integration branch moved by twelve commits between
the first report and the hand-off: on wave 12's base the lane read **254 / 55 028**, and on
wave 13's base (`origin/dev-huichen` at `71f93f804`) it reads **266 / 55 051**. The
difference is not this lane's: three upstream commits touched this crate (`9fdabcb03`,
`71f93f804`, `5d371857d`), which added the eleven `test_server.rs` lines above and moved
line numbers inside `commands/remote.rs` and `headless/mod.rs` — the registered rows below
were re-mapped onto the new numbers and re-checked (`checked=57 stale=0`), rather than left
describing the old file.

That line total grew by 158 in wave 13, and none of it is product code: the nine tests added
then live inside the lib (five in `session_journal`'s and `session`'s test modules, two in
`models`', one in `remote_peers`'), so their bodies are instrumented and the denominator
grows with them. `commands/models.rs` and `commands/session.rs` are therefore both at
metric A's floor: `models.rs` reads **0**, and `session.rs`'s single remaining line is a
closing brace. Wave 14 adds no line at all: both of its tests are **integration targets**
(`cli/tests/`), which the report does not instrument, so its whole arithmetic is the six
lines it closes.

`delta.py` keyed its rows on the absolute `SF:` path until this wave, which made every file
look new whenever the two reports were measured in different worktrees (`cov-cli`, then
`cov-cli-new`): the first run of it printed `+31` for `commands/remote.rs` and `0` for every
row on the left. It now keys on the path inside `cli/src/`.

## How this lane is measured

```bash
cd coverage/2026-11/after/cli
./measure15.sh                                # lib + every integration target, then the nats-only pass
python3 perfile.py cli-lcov-15.info           # per file, highest first
python3 missed.py cli-lcov-15.info "cli/src/headless/mod.rs"
                                              # the DA:0 lines of one file
python3 classify.py cli-lcov-15.info          # closing-token class vs statements
python3 delta.py cli-lcov-14.info cli-lcov-15.info w14 w15   # per-file movement between two waves
python3 waiver_check.py cli-lcov-15.info "relay/cli.rs:519-522,526-528,530,532,584,588-590,421,245" \
    "relay/server.rs:239" "relay/room.rs:168-170" "prompt.rs:37,59,67-74,104,127,140,151" \
    "relay/jwt.rs:45-48,50" \
    "commands/remote.rs:533,1341,1578,2474,2432-2433,2807,2809-2810,2627-2631" \
    "commands/remote_peers.rs:684,687-690,692,985-987,991"
                                              # every registered row, re-read against this report
./safari-load-repro.sh 3 8                    # the gate flake's reproduction (see cli-safari-gate-flake.md)
```

`measure15.sh` pins `HOME` to `target/test-home`, clears `CARGO_BUILD_TARGET`/
`CARGO_TARGET_DIR` (they leak from other sessions' shells) and puts the 1.99.1 toolchain
first on `PATH` (Homebrew's rustup shim shadows it otherwise). It must be run as
`sh ./measure15.sh` (or after `chmod +x`): the file is checked in without the executable
bit, and `./measure15.sh` then fails in milliseconds with no output at all — which reads
exactly like "the measurement finished instantly".

The wave-15 report was taken on the base wave 14 merged into (`2b9e9ef36`): `1549 passed;
0 failed` in **179.13 s** for the lib target, every integration target green
(`bin` 28, `headless_cli` 18, `remote_start_refusal` 14, `remote_unpair_prompt` 5,
`relay_serve` 4, and six single- or zero-test targets), and the nats-only pass `2 passed`.
The previous wave's numbers are kept below rather than replaced, because the two runs are
the record of a lane that was measured across a moving base.

The wave-14 report was first taken at load average **38**, and then re-taken after the
integration branch moved ~80 commits (load **45–80**): `1543 passed; 0 failed` in
**102.66 s** and then **173.75 s** for the lib target, every integration target green both
times, and the nats-only pass `2 passed`. No run of this wave had to be re-taken for a
failure, which is itself part of the answer to the safari question: the wave-13 diagnostic
is in place, and six runs of this wave (two measured, four hunted) did not produce the red
it is for — see `cli-safari-gate-flake.md` §7.

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
| 10 | `commands/remote.rs:533` | **W2** | `Book::set_controller`'s `if self.legacy { return; }` — "a legacy file has one device, which controls by virtue of being alone". Its only caller is `take_control`'s `book.edit` closure (2605), inside `if !recorded`, and a legacy book *is* one credential: `Book::read` sets `active = slots.first()` (434-435), so `recorded` is already true whenever a legacy book is in hand and the call never happens. | a second caller of `set_controller` that does not compare against `active` first — or a legacy document that yields no `Device`, which fails `read` before the caller runs | `use_persists_the_controller_and_keeps_unmodelled_fields` drives the multi-device arm of the same function; `a_legacy_single_pairing_reads_as_one_slot_that_cannot_serve` pins the single-device shape that makes the guard true |
| 11 | `commands/remote.rs:1341` | **W2** | `Observed::headline`'s `(other, _, _) => (*other).to_string()`. The match is on `self.state()`, and `state()` is `state_of(running, phase)` (1081-1083) — a `&'static str` whose every arm returns one of the eight literals matched above it (`off`, `unknown`, `on`, `connecting`, `serving`, `degraded`, `failed`, `revoked`). A phase this build does not know is `"unknown"` (1063), which has its own arm at 1333-1334. | an arm added to `state_of` — or `state()` ceasing to be its only source | `every_phase_maps_to_the_state_the_document_promises` pins the closed vocabulary, including `state_of(true, Some("draining")) == "unknown"`: the unknown phase lands on a named arm, never on this one |
| 12 | `commands/remote.rs:1578` | **W2** | `switch`'s `out.log_err(silence)` inside `"off" if on =>`. `silence` is set only on the paths where `reported` stays `None` (1523, 1534), and a running Desktop whose state is `"off"` must have answered with `phase: "stopped"` (1054) — i.e. `reported` is `Some`. With no Desktop running `silence` is never set at all (1517). The two conditions are mutually exclusive by construction, so the arm is entered (`DA:26` on 1576) and its body never is. | an answer that carries a status *and* an error, or a `state_of` that maps a missing status to `"off"` | `a_desktop_that_refuses_the_switch_still_stores_it_and_exits_one` takes the `"unknown"` arm above it (1521-1526 → 1570-1574); `enable_without_a_desktop_stores_the_switch_and_says_it_is_not_serving` takes `"off"` with no silence |
| 13 | `commands/remote.rs:2474` | **W2** | `queue_pending_revoke`'s `None => return Err(… "is not a JSON object")` on `pending.as_object_mut()`. `pending` has exactly two producers: `json!({})` for a missing file, and the document that survived `.filter(Value::is_object)` (2434-2438) — a third shape would have to be constructed below that filter. | a third way to build `pending` that is not an object (e.g. reading the queue through a helper that returns `Value`) | `an_undeliverable_platform_revoke_is_queued_for_the_next_desktop` writes the queue through this function, taking the `Some` arm, and reads the file back as an object |
| 14 | `commands/remote.rs:2432-2433, 2807, 2809-2810` | **W4** | `platform_api_key`'s `NotFound` and other-I/O arms, and `revoke`'s arm that queues the revoke for that error. The caller has already proved this file readable: `future_api_key()` (2390) returned the key out of `<home>/agent/auth.json` through `auth_store::read`, and `platform_api_key()` resolves the *same* path (`future_agent::utils::default_config_dir()` = `future_home()/agent`, the same root as `auth_store::agent_dir()`) and reads it again microseconds later, with no `await` in between. Any of these five lines needs the file to be deleted, replaced or emptied exactly in that window. | a concurrent `future auth logout` (or an editor) landing between the two reads — which a test cannot place without a seam — or the two functions resolving different paths, which would be a defect of its own rather than a line to cover | `an_undeliverable_platform_revoke_is_queued_for_the_next_desktop` pins the guard's own outcome: the "not signed in" warning is what a machine with no `auth.json` gets, and it is produced at 2391-2400, before either read can differ |
| 15 | `commands/remote.rs:2627-2631` | **W4** | `take_control`'s "no longer paired to this machine: the pairing book changed while this ran" — `book.edit`'s closure finding the wanted device gone. The book is read twice in this function (`Book::load()` at 2580, `Self::load()` inside `edit`), and nothing between them awaits: `desktop_running()` and `switch_is_on()` are file reads, so another process cannot be placed in that gap by a test. | an `await` between the two reads (the Desktop round trip moved before the write, which the comment at 2593-2595 says is deliberate), or a load seam | `use_persists_the_controller_and_keeps_unmodelled_fields` takes the `Ok` arm of the same closure — `book.edit` writing a controller that is still there; `use_accepts_a_stripped_or_partial_id` reaches the same write by prefix |
| 16 | `commands/remote_peers.rs:684, 687-690, 692` | **W2** | `add`'s `Some(live) => "The pairing is stored, but the connection did not come up{…}"`. `peer.live.connected` is `PeerSummary.connected` as `pair_with_emitter` returned it, and the only assignment to that field on an `Ok` path is `summary.connected = true` (`packages/remote-client/src/runtime.rs:1806`); every failed connection returns `Err` (1765-1789), which `add` turns into `describe_pairing_error` and a failed command instead of this arm. The arm reports a state the runtime cannot produce. | a runtime that returns an `Ok` summary with `connected == false` — i.e. stores the pairing and answers before the first connect, which is the contract this arm was written for | the connected arm is driven twice, by `add_says_which_machine_it_paired_and_who_connects_it` (text) and `add_json_prints_the_machine_it_paired_and_who_keeps_it_connected` (JSON, `cli/tests/`) |
| 17 | `commands/remote_peers.rs:985-987` | **W4** | `rename`'s `return Err("{desktop_id} is no longer paired with this machine: the book changed while this ran.")` inside `book.edit`'s closure. The id is read from a book loaded at 967 (`book.position(&selector)`), and the closure re-reads the same file; nothing between the two reads awaits — `Book::load` and `edit` are both file I/O — so the row has to be *removed* by another process in that microsecond window. The line is then the one arm `edit` cannot avoid reaching if it did. | a concurrent `remote peers remove` (or an editor, or a Desktop round trip inserted between the reads) landing between `Book::load()` and `book.edit`'s re-read, which no test can place without a seam | `rename_writes_the_label_and_keeps_the_credential` and `rename_clear_goes_back_to_the_machine_id` take the closure's success arm — the row is found, written or cleared, and the credential is read back; `an_ambiguous_prefix_asks_for_a_longer_one` pins the selector refusal that is the *other* way this arm could have been reached, and it happens in `position` before the closure runs || 18 | `commands/remote_peers.rs:991` | **W2** | `record.as_object_mut().ok_or_else(|| "The pairing book holds an entry that is not a JSON object.")` — the *entry* the `find` above returned is by construction an object: the predicate is `peer.get("desktopId").and_then(Value::as_str) == Some(id)`, and `Value::get` with a string key answers `None` for an array, a string, a number, a bool and null alike. So the row that satisfied `find` has an object at its root, and `as_object_mut` cannot be `None`. | a `find` predicate widened to something `Value::get` does not bound (e.g. matching a nested path, or `== Value::String(id)` on the peer itself) | every test in `remote_peers/tests.rs` goes through this closure: `rename_writes_the_label_and_keeps_the_credential` and `rename_clear_goes_back_to_the_machine_id` both take the `Some` side of the same `ok_or_else`, and the entries they carry came from the product's own writers |
| 19 | `relay/jwt.rs:45-48, 50` | **W8** | `Role::as_str`, the render direction of the role vocabulary. Nothing in this tree calls it: `Role::parse` is the only direction a token request uses (`server.rs` parses `role` and never renders one), and no log line, claim or error message formats a `Role`. The role string that crosses the wire is written by the *clients*, not by this crate. | a second consumer that renders a role — a log line, a JWT claim, or an error message naming the caller's role. If the honest answer is "there will never be one", the accessor is dead code and deleting it is the fix, not covering it. | `a_correctly_signed_request_verifies_and_reports_its_key` and `sign_user`'s claim assertions pin the *parse* direction and the permissions each role gets; nothing asserts the string back because nothing produces one |

> A line whose text is nothing but a closing token (`}`, `})`, `});`, `))`, `));`, `)?;`,
> `}),`, `],`) reads `DA:0` whenever the *construct it closes* is not re-entered on the last
> iteration. llvm-cov attributes the end of a multi-line expression/block to that line, and a
> statement whose only execution is "fall through to the brace" has no counter to increment.

Nothing is lost by excluding it from the numerator's accounting **as a class** — and nothing
is gained by waiving it line by line, which would turn 76 measurements into 76 assertions.
The count is printed by `classify.py` on every run so it cannot grow silently: `76` at this
measurement, `77` at wave 14, `79` at wave 13, `86` at wave 10, `97` at wave 3, `119` at
wave 2.

All 62 registered lines were re-read against `cli-lcov-15.info` when this wave landed
(`waiver_check.py`, `checked=62 stale=0`), so no row is stale. Five of them
(`relay/cli.rs:245`, `relay/cli.rs:522`, `commands/remote.rs:2631`,
`commands/remote.rs:2810`, `relay/jwt.rs:50`) are also closing tokens, which is why the
class count is 76 of the 250 and the registered statements are 57 and not 62. Two more of
the 76 are the decision block's (`headless/mod.rs:494-495`), so the 250 divides as
**62 registered + 8 decision + 180 work items**, and the 180 work items carry **111
statements and 69 closing tokens**.

Rows 17 and 18 are the last unregistered lines of `commands/remote_peers.rs`: with them the
file's 20 `DA:0` lines divide exactly into **11 closing tokens** (`411`, `671`, `692`, `806`,
`906`, `988`, `992`, `1009`, `1045`, `1065`, `1118`), **5 statements in row 16**
(684, 687-690) and **4 statements in rows 17-18** (985-987, 991). The file has no work item
left, which is why it moved from the *Not waived* table to a sentence in this one.

Four of wave 13's seven new rows were written *after* trying to cover them, and each of
them names the measurement that changed the answer rather than a guess:

* row 16 was assigned as "add's disconnected arm, build it a second bridge". The arm cannot
  be reached at all: `pair_with_emitter` returns `Ok` only from `connect_attempt`, and the
  field is set to `true` on the line that follows a successful handshake. A test that
  manufactured a `connected: false` summary would be testing a mock, not the runtime.
* row 10 was assigned to the same fixture. A legacy book already *is* the controller, so
  `take_control` skips the write entirely — the guard is reached only by a caller that does
  not exist.
* rows 14 and 15 were the two places a "live session" was expected to help. Both are two
  reads of one file in one function with nothing awaiting between them, which is a race
  window, not a scenario.

The lines that were in `commands/remote.rs`'s work-item row at wave 11 were therefore
**mis-classified as reachable-with-a-live-session**, and the wave-10 sentence that the
`host::remote_peer::testing` bridge "is what supplies one" was wrong about them: it supplies
the *pairing* (`add`), which is a different file. The file's 31 `DA:0` lines now divide
exactly: **14 registered** here (4 W2 + 10 W4: 533, 1341, 1578, 2474, 2432-2433, 2627-2631,
2807, 2809-2810), **2 closing tokens** (520, 1366) and **15 test-side lines** (below).

### The fifteen test-side lines in `commands/remote.rs`

`DA:0` inside `#[cfg(test)]` is not a coverage gap in the product, and the lane instruments
it anyway, so it is counted here rather than in a separate file:

| lines | what they are | why 0 |
|---|---|---|
| 3739, 3781, 3786, 5164, 5175, 5180 | the `recorded[0]` / `platform_requests[0]` / `relay_requests[0]` arguments of six `assert!` messages | an assertion's message arguments are evaluated **only when the assertion fails**; every one of these six assertions passes on every run |
| 5098 | `a_real_ca_bundle_builds_a_trusting_client`'s `bundle.display()` message argument | same rule: the assertion passes |
| 3895, 3904 | the closing braces of a test-side `retain` in `answer_removals` | closing tokens, in test code |
| 3946-3947, 5668-5669 | two fixture loops' "not there yet" / "claimed" arms (`answer_takeovers`, the silent-Desktop responder) | the first poll of each task already finds the request, and the loop's iteration cost is attributed to the `read_dir` line above it (the same attribution shape as the closing-token class) |
| 5092-5093 | the "no platform CA bundle on this host" fallback of the same test | macOS ships `/etc/ssl/cert.pem`, so the `let Some(..) else` never takes its else arm on this lane's host |

They are left uncovered on purpose: making them `DA:1` would mean writing assertions that
fail (to evaluate their messages) or a host with no CA store — both worse than the
measurement they would buy. `classify.py` prints them as ordinary statements, so they stay
visible in this file rather than disappearing into a class.

## The launch test that could not say why it failed (5 test-side lines, wave 13)

`browser::safari::safari_manager::tests::launch_then_become_reachable_reports_started`
waited 20 s for its driver script's marker and then panicked with its **own** sentence —
"the driver was never spawned, so the poll was never reached" — while throwing away
whatever `safari_start` had actually answered. That is why the two red runs in wave 12
could not say whether the product refused before the spawn or the host starved it
([`cli-safari-gate-flake.md`](cli-safari-gate-flake.md) is the conclusion record).

Its sibling `start_launch_success_against_fake_driver` has checked
`started.is_finished()` since its driver-port record replaced a sleep. This wave ported
that check (test code only; no product line, no timeout, no seam). The five lines it adds
(`safari_manager.rs:508-513`) read `DA:0` in every green run **by construction**: they are
the arms reached only when the launch has already failed, so a passing run can never
execute them. They are counted here rather than waived, exactly like `commands/remote.rs`'s
assertion-message arguments: `classify.py` prints them as ordinary statements, so they stay
visible instead of disappearing into a class.

Measured, not assumed: with the platform gate forced false for one run, the same test now
reports `safari_start failed before spawning a driver: Safari is only available on macOS.`
in **0.04 s**; before the change the same input printed the marker-wait sentence after the
full 20 s. That is the diagnostic's positive control, and the reason this wave's per-file
count for the file went *up* by five while the lane's went down by thirty.

**Where that landed (wave 14).** The flake workstream closed the item on a different branch
(`0900e710c`, in this wave's final base): the 20 s wait is gone, the wait now ends when the
fixture's record appears *or* when the launch task finishes, and its bound is the product's
probe budget — a count, not a duration. Its injected control (a driver that sleeps 25 s
before touching its marker) was **2/2 red** with the old shape and green with the new one, so
the arm was (e), `/bin/sh` starved past the test's own clock, and never a foreign listener.
The `DA:0` lines this section counted moved with that rewrite: the file reads 11, not 14, on
the base this wave is measured against, and they remain test-side arms. `$2` was therefore
not ported — see `cli-safari-gate-flake.md` §7.



## Two launch arms that had never run (6 lines, wave 14)

Both of these are in `headless::session`, and both were registered as "needs the whole
heavyweight `headless_cli` fixture, not a unit test". They were right about the *shape* and
wrong about the cost: the fixture existed, and each arm is **one** more launch in it.

**`--relay` (302/306).** The flag had only ever been *parsed*:
`relay_flags_are_accepted_in_both_spellings` stops at `--help`, and no test in this crate had
started a relay run. A launch now really runs: `future host --relay
http://127.0.0.1:1` serves, prints "Self-hosted relay (…). No Future account is used for phone
pairing", takes the `None` account branch and stays up (`No device is paired yet`), then
stops cleanly on SIGINT. The port is a loopback one nothing listens on **on purpose**: the
decision is printed before any connection is attempted, so a reachable relay would add a race
without adding a property. The test asserts the *absence* of the signed-out sentence (the
other branch of the same decision) and reads the credential file back — the embedded Agent
writes a placeholder `auth.json` of its own, so the file's existence means nothing, while a
key in it would prove the account branch had run. Measured: the first version asserted the
file's *non-existence* and failed in 5.42 s on that placeholder; the key is the assertion
that measures the decision.

**The catalogue the *Agent* refuses (656-658).** This one is not reachable by an unreachable
Agent: `host::agent_bridge::queries::sync_future_models` answers a **zeroed result**
(`synced: false`) for an Agent it cannot connect to — "best-effort" by its own doc comment,
pinned by `sync_future_models_variants` (`Down agent → zeroed result`). The only input that
reaches the error arm is an Agent that answers and *rejects*, which is exactly what
`future_host::test_support::agent`'s scripted mock is for. So the test pins
`FUTURE_AGENT_GRPC_ADDR` at that mock, scripts
`reject_default("sync_future_models", …)`, serves the platform's profile endpoint from a
loopback listener (the account must be adopted for a refresh to happen at all), and waits for
the product's own sentence:

```
Model catalog refresh failed; existing configured models remain available: the platform\
catalogue is unavailable
```

It then asserts the run went on to `Preparing the remote connection...`, that it did not call
itself signed out, and that SIGINT still stops it cleanly. The target is
`cli/tests/headless_catalogue_refusal.rs` and it holds **one** test: `mock_agent()` points a
process-wide `FUTURE_AGENT_GRPC_ADDR` at itself, and a single-test process is the only way to
use it without the value leaking into another test's child. Row 16's lesson, applied instead
of re-learned.

Neither target adds an instrumented line: both live in `cli/tests/`, which the report does
not cover. So the wave's arithmetic is the six lines it closes and nothing else.

## Rows this wave closed

Deleted from the work-item list, not reworded — each is now `DA:1` and each has a test that
names the input that reaches it:

| was | now covered by |
|---|---|
| `relay/config.rs` **4 → 0** (wave 15): the seed refusal (116), the two TTL refusals (119, 127), and the empty certificate field (132) | `an_account_seed_nobody_generated_is_refused_by_name`, `a_ttl_that_has_already_elapsed_is_refused`, `a_certificate_field_that_is_set_but_empty_is_refused` — three tests over `validate()`, each asserting the sentence *and* the field it names, because an operator reads the sentence and greps for the field. None of them is a new rule: the four refusals existed and had never been run. |
| `relay/sign.rs` **2 → 0** (wave 15): `VerifyError::code`'s `Malformed` (80) and `BadSignature` (82) arms | `an_unreadable_or_foreign_signature_is_named_on_the_wire` — two requests to the roster route: a signature header that is not base64, and a genuine signature made by a key other than the one the header names. `Missing` and `Stale` were already on the wire; these two were the codes no test had asked for. The same test then takes the host's own signature and gets `200`, so both refusals are about the signature and not the request. |
| `commands/data.rs` **21 → 19** (wave 15): `work`'s `symlink_metadata` error arm (638) and one closing token | `a_restore_work_directory_the_filesystem_cannot_name_is_an_error_not_an_absence` — a restore target home nested to 985 bytes, so `<home>/.restore-work-<id>` (46 bytes below it) is the one path the filesystem refuses while the home, its `app`/`agent` children and the pending marker all still resolve. Falsified before it was kept: at 300 bytes the same test panics on `unwrap_err()` of an `Ok`, i.e. the restore really succeeds and the assertion is not vacuous. |
| `headless/mod.rs` **18 → 17** (wave 15): 451, `AccountAction::Adopt`'s `remote::status()` — the switch-off arm of "a login appeared" | `a_login_adopted_while_the_switch_is_off_serves_nothing_until_the_switch_returns` (`cli/tests/headless_cli.rs`) — a launch: the switch is stored off, the entry starts signed out, `auth.json` appears under the running host, and the run adopts it **without** asking the platform for a token; `future remote enable` afterwards starts serving from the same process, which is what proves the adopted account is still there. Falsified: with the switch left on, the same test fails on the no-token assertion because the arm took `remote::enable()` instead. |
| `headless/mod.rs` **24 → 18** (wave 14): the `--relay` sentence and the `None` account it decides (302, 306), and the catalogue-refusal report (656-657; 658 `);` and 456 `}` are closing tokens) | `a_relay_launch_serves_without_a_future_account` (`cli/tests/headless_cli.rs`) and `a_catalogue_refresh_the_agent_refuses_is_reported_and_the_run_keeps_serving` (`cli/tests/headless_catalogue_refusal.rs`) — see *Two launch arms that had never run*. |
| `commands/models.rs` **12 lines → 0** (wave 13): `refresh_failure_reason`'s platform-failure sentence (380-381), the arm that reports it (350), and the `source: "cache"` document a script reads instead of the sentence (354-362) | `a_failed_platform_fetch_is_reported_as_the_network_not_the_login` — a credential pointing at a refused port, so the local fetch fails *and* the credential exists, which is the only input that separates the network sentence from the login one (the file's existing tests reached neither: one has no credential, the other's fetch succeeds). `a_local_refresh_reports_itself_as_the_cache_in_json` — the same real platform fixture, asking for the document. The file now reads 0 `DA:0`. |
| `commands/session.rs` **11 → 1** (wave 13): `session prune`'s dispatch (1471), `list`'s own argument shape — `--parent` with no value (1514-1515), `--help` (1520-1522), an unknown option (1524-1526) — and `option_label`'s unmodelled-key label (1427) | `the_dispatcher_reaches_prune`, `list_refuses_its_own_unknown_and_valueless_options`, `an_unmodelled_setting_keeps_a_generic_label`. The one line left is a closing brace (1338). |
| `commands/session_journal.rs` **15 → 8** (wave 13): `number`'s string arm and its unknown-shape arm (121-122), `short`'s truncation (134), `local_time`'s "never" (140), the report's "never" and "none recorded" rows (222, 260), and a kept journal's reason (273) | `counters_read_a_string_and_degrade_an_unknown_shape_to_zero`, `long_run_ids_are_shortened_and_short_ones_are_not`, `an_absent_instant_reads_as_never`, `a_report_with_no_history_says_never_and_none_recorded`, `a_kept_journal_prints_the_reason_it_was_kept`. All five are pure functions of a report document, so none of them needs an Agent. |
| `commands/remote_peers.rs` **1105-1108** (wave 13) — `remove`'s "Could not remove the pairing: … The book is unchanged", the arm behind a client that cannot rewrite the book | `a_book_only_this_command_can_read_is_not_reported_as_removed`: a `remote_peers.json` whose row carries a `desktopId` and nothing else — readable by this command's own loader (which needs only the id to print a row) and rejected by the client's typed book. No race and no permission trick: the row is on disk for the whole command, and the test asserts the file came back byte-identical. |
| `commands/remote_peers.rs` **668-672** (wave 12) — `add --json`'s document: `desktopRunning`, the `paired` row and the return that keeps the document whole | `add_json_prints_the_machine_it_paired_and_who_keeps_it_connected` (`cli/tests/remote_peers_add_json.rs`): the same **real** pairing as the lib's text test, then `serde_json::from_str` over the *whole* of stdout — which is also the assertion that nothing follows the document on this branch. |
| `commands/remote_peers.rs` **694-695** (wave 12) — "The Desktop running here keeps it connected from now on." | `add_with_a_desktop_running_says_who_keeps_it_connected` (`cli/tests/remote_peers_add_running_desktop.rs`): one real pairing while this process holds `<future>/app/desktop.lock`, which is the file `desktop_running()` opens; the test asserts the sentence **and** the absence of its opposite. |
| `commands/remote_peers.rs` 78 → 29 (49 lines) — `add`'s pairing and its report (638-700), `takeover` on an empty book (862-866), `remove` on a Desktop that holds the lock and never answers (1084-1087), and the row a Desktop reports that the book does not hold (545) | `add_says_which_machine_it_paired_and_who_connects_it` — a **real** pairing: a host bridge on the in-process broker, a claim the mock platform grants, one Noise handshake, and the book written where the next start reads it. Plus `takeover` in `an_action_on_an_empty_book_names_the_paired_command`, `remove_proceeds_here_when_the_desktop_never_answers`, `list_keeps_a_pairing_the_book_and_the_desktop_disagree_about`. |
| `headless/mod.rs` 42 → 24 (18 lines) — the switch-off report (382/387), the idle notice (511-517), the refused-key refusal (326/329/330) and the transport failure while verifying (334) | `a_switched_off_remote_desktop_stays_up_and_comes_back_without_a_restart` (one run: the switch is off, nothing is served, and `future remote enable` from another terminal starts serving **without a restart**), `a_stored_key_the_platform_refuses_leaves_the_machine_serving_locally`, `an_unreachable_platform_while_verifying_is_reported_as_a_failure`. |
| `headless/mod.rs` 249 + 951-953 — the loop's own stop cause | `a_loop_that_returns_is_its_own_stop_cause`: the session arm of `until_shutdown` returns `StopCause::Session`, whose reason is `headless_exit` and whose terminal words are "an internal stop" — not one of the three signal names. |

## Nine lines in four places, and the arms that stayed out of reach (wave 15)

Each of these was a *statement* that read `DA:0` and that no wave had written down, so the
finding worth keeping is not the test — it is which part of the arm was the obstacle.

**The relay's four config refusals and its two signature codes had never run.**
`RelayConfig::validate` refuses a whitespace `account_seed`, a TTL of zero and a certificate
field that is set but empty; every one of the four `bail!` lines was uncovered, because the
file's tests cover the *other* four refusals (empty endpoint, long-lived JWT, half a
certificate, and the round trip). Nothing new is measured by running them — which is exactly
why they had drifted, and why the tests assert the sentence *and* the field name: the sentence
is what an operator reads and the field is what they grep for.

`VerifyError::code` had the same shape from the other side. `Missing` and `Stale` reach the
wire (the roster route answers `401 signature_missing` / `401 signature_stale`), so the two
arms that a client-visible code had never produced were `Malformed` and `BadSignature`. Both
are input-driven: a signature header that is not base64, and a genuine signature made by a key
other than the one the header names. The test drives the real handler twice and then takes the
host's own signature to a `200`, so the refusals cannot be about the room or the request.

**A restore work directory the filesystem cannot name (`commands/data.rs:638`).** `work` reads
an absent `.restore-work-<id>` as "no restore has started", so the arm between that and a real
error needs an OS failure other than `NotFound`. `ENAMETOOLONG` is one a *path* produces
without any injection: on macOS the limit is 1023 bytes, the work directory is 46 bytes below
the home, and the pending marker is 25, so a home of 985 bytes leaves the work directory as the
single path over the limit while everything around it still resolves. The test builds exactly
that (`nested_home`) and calls `begin_restore`; the error is `File name too long (os error 63)`.

It was falsified before it was kept: at 300 bytes the same test panics on `unwrap_err()` of an
`Ok` — the restore really succeeds — so the assertion is not "some error happened".

**Why the same trick does not reach the other three (`293`, `359`, `591`).** This is the
measurement this wave adds to `commands/data.rs`'s row, and it is negative on purpose. All four
lines look alike — `Err(error) => return Err(error.into())` after a `symlink_metadata` — but only
`work`'s is reachable that way. `collect` and `inventory` both resolve their path through
`relative(..)`, which calls `without_links`, and `without_links` `lstat`s *every component in
order*: at the component where the path crosses the limit it returns its own error, one frame
before the arm under discussion. `root_digest`'s caller (`finish_restore_with`) builds its path
from `work(home, ..)`, which is under the limit or the restore never started. So those three
need a *transient* failure of a path the filesystem just resolved — `EIO`, `ESTALE`, or the
directory disappearing between two reads — which is a race window (W4), not an input. The row
now says so instead of implying the whole family is one `ENAMETOOLONG` away.

**The login that arrives while the switch is off (`headless/mod.rs:451`).** The adopt arm has
two halves — `remote::enable()` when the switch is on, `remote::status()` when it is off — and
only the first had a launch. The new target is the existing switched-off test crossed with the
existing login-appears test: start signed out with the switch stored off, write `auth.json`
under the running host, and require that nothing is asked of the platform for the next three
seconds. Falsified the same way: with the switch left on, the identical test fails on precisely
that assertion (`["/client/v1/remote/auth/token", …]`), because the arm took `enable`. Then
`future remote enable` from another terminal starts serving from the same process, which is what
proves the adopted account was kept rather than merely reported.

**What that test cannot say.** The sentence this arm prints is
"Platform account signed in (…): starting the remote desktop for it." — and when the switch is
off it starts nothing (see observation 5 below). The test therefore does not assert the sentence;
it asserts the two effects that distinguish the arms (no token request now, a token request after
the switch returns). A message can lie about which line ran; the platform's request log cannot.

## The `future-host/test-support` decision — taken, in this wave

Wave 10 left this open and framed it as a lane decision. It is settled:

1. **The feature was already on.** `cli`'s `[dev-dependencies]` has carried
   `future-host = { features = ["test-support"] }` since `a5f91752b` (the host extraction),
   and this crate already consumed it (`test_support::configure_entry_fixture`,
   `test_support::agent::mock_agent`). Using `remote_peer::testing` adds no dependency, no
   feature, no `pub` and no seam: the product source is untouched, which is why it is not
   "changing the product for a test". The whole diff is test code.
2. **It works, and it was worth 49 lines** in `remote_peers.rs` alone — `add`'s claim and
   handshake cannot be faked, and a mocked platform answers only the HTTP half. In wave 12 it
   was worth 5 more `DA:0` lines off the count (`add --json`'s document and the
   running-Desktop sentence — six lines of code, one of which is a closing token that stays 0),
   reached from two **integration** targets that share a copy of the fixture in
   `cli/tests/support/mod.rs`.
3. **Three properties of the fixture cost real time and are now recorded** where a reader
   will hit them (`cli/src/commands/remote_peers/tests.rs`, above `Served`, and
   `cli/tests/support/mod.rs`):
   * `testing::fixture` **claims** the bridge for a device; a second claim against the same
     bridge answers `peer_mismatch`. `add` *is* the claim, so the fixture has to be rebuilt
     one step earlier (`start_host` without `claim`).
   * **One bridge per process.** `add` shuts the peer runtime down before it returns (it is
     a one-shot command), and a second `start_host` in the same process then fails. That is
     why the `--json` branch and the running-Desktop sentence live in two integration
     targets of their own: one pairing per binary, three binaries for three reports. (An
     integration target satisfies this *without* a child process — it links the crate's own
     `dispatch`, and the lib it calls is the same instrumented one, so the counters land in
     the same report.)
   * **The runtime the bridge spawns on must outlive the test that installed it.**
     `future_host::runtime::set` is a `OnceLock`, so a test-scoped handle stays installed and
     goes dead with the test. Measured: with a test-scoped handle, a full lib run failed
     three `headless::tests` CLI-discovery probes with "discovery timed out"; with a leaked
     process-lifetime runtime the same run passes 1532/1532. Both fixtures install the latter
     and say so.

## Not waived — reachable, and how

These are work items. Each names the input that would make the line run; none of them is
claimed as unreachable. `commands/remote.rs` is **not** in this table any more: its 31
`DA:0` lines are 14 registered rows, 2 closing tokens and 15 test-side lines, all accounted
above. Neither is `commands/remote_peers.rs`: wave 14 registered its last four statements
(rows 17-18), and its 20 lines now divide into 11 closing tokens, 5 registered statements in
row 16 and 4 in rows 17-18 — every one of them named above.

| area | `DA:0` | the input that would take it |
|---|---:|---|
| `headless/mod.rs` | 17 | three groups, after the `NeedsAccount` notice (8 lines, below) and the login-while-off arm (451, **closed in wave 15**) are out of this table. (a) 128: `start_logging` failing — a read-only data root or a full disk, while the entry keeps serving. (b) 523 (`Stopped` with the switch on and something to serve), 857-859 (`stop`'s runtime build failing — W2), and 951-953 plus 1183, which are two test-side arms: the closure body a "startup was dropped" test must never poll, and a fixture-discovery `panic!` (covered until wave 13; an upstream commit re-shaped the fixture that used to take it, so it is uncovered again and belongs to that commit's owner). Two closing tokens (`494-495`) belong to the decision block rather than here. **Wave 14 closed 302/306 and 656-658; wave 15 closed 451** — see *Two launch arms that had never run* and *The login that arrives while the switch is off* below. |
| `test_server.rs` | 14 | **another lane's** (upstream `9fdabcb03`/`71f93f804`): the mock listener's fd-pressure report and its platform gates — diagnostic arms a passing run cannot execute. Listed so this lane's total is fully accounted for; it is not a work item here. |
| `commands/session_journal.rs` | 8 | three `serde_json::to_string_pretty(..).map_err(..)?` arms (300, 329, 364) — **W2**, serialising this module's own structs cannot fail —, the `MAX_SLICES` exhaustion refusal (350-353, a journal of 64+ slices), and one closing token (367). |
| `commands/data.rs` | 19 | **3 statements** are the `Err(error) => return Err(error.into())` arms of `symlink_metadata` in `root_digest`/`collect`/`inventory` (293, 359, 591), plus **16 closing tokens**. Wave 15 closed the fourth of that family (`work`, 638) with a long home *and* measured why the same trick does not reach these three — see *A restore work directory the filesystem cannot name* below: `relative`/`without_links` refuses the identical path one frame earlier, so the arm behind them needs a *transient* OS error (`EIO`, `ESTALE`), not a long one. Permission denial does not reach them either, and that was measured in wave 12, not assumed. |
| `commands/doctor.rs` | 12 | **W1 + W2**: the **OS-sandbox probe** arms. On macOS `platform_sandbox_probe_product()` always returns `Ok`, always reports `version: None`, and its `code` is only `available`/`binary_missing` — so `Status::Warn` (173) needs `/usr/bin/sandbox-exec` **absent**, `version={version}` (183) can never print, `available_write_only` (186) and `sandbox_remedy` (189) are Linux-only, and the `Err` arm (197-201) has no macOS producer. The remaining two are defensive: an empty key the accessor has already normalised to `None` (140) and a `Stdio::piped` pair that is piped by construction (387), plus one `join` error from a copy task that cannot panic (419). |
| `relay/cli.rs` | 15 | registered above (rows 1-5): process lifetime, W3, W1 |
| `relay/jwt.rs` | 5 | registered above (row 19), `Role::as_str`: a dead accessor, not a work item |
| `relay/config.rs`, `relay/sign.rs` | 0 + 0 | at metric A's floor since wave 15 (both were work items with no row: a validation refusal never run and two error codes never asked for) |
| `commands/session.rs`, `models.rs` | 1 + 0 | both are now at metric A's floor (`session.rs` has one closing brace left) — no work item remains in either |
| `commands/configure.rs`, `mcp_*`, `browser/*`, `headless/agent.rs` | 7+18+14+14 | one refusal/report arm each. `headless/agent.rs`'s 14 are named in the `supervise` section below. |

### A product decision, not a waiver: `headless/mod.rs:488-495`

Eight lines that are **neither registered as unreachable nor written off as a work item**,
because they are only unreachable while a product decision stands as it does:

```rust
PhaseAction::NeedsAccount => {            // Failed(AccountAuthorization) && !serving
    if !said_signed_out { ... "The remote desktop needs a FutureOS account" ... }
}
```

1. **What reaches the phase.** `phase_action` answers `NeedsAccount` for
   `RemotePhase::Failed(AccountAuthorization)` on a run with no account and no relay — the
   comment above the match calls this "this run's ordinary signed-out state (the remote
   desktop was switched on by `future remote enable` while nothing could serve it)".
2. **Why a CLI process cannot produce it.** Every CLI path that could put a remote desktop
   into that phase requires an account first:
   * `future remote enable` refuses before the host is asked — `remote.rs:1511`,
     `if on && !signed_in() { return Err(login_advice()) }`, pinned by
     `enable_without_a_login_points_at_auth_login_and_stores_nothing`;
   * `future remote devices add` cannot claim an invitation without an account
     (`describe_pairing_error`'s `NotSignedIn` arm);
   * the headless entry only calls `remote::enable()` when an account or a relay exists
     (388-396), and when a pinned account disappears mid-run `account_action` answers
     `Close`, which exits the entry with its own sentence (433-437) rather than looping on
     this one.
   * `docs/guide/desktop-host.md` states the same model, in the paragraph that begins
     **Signing in comes first** (line 268): "a switch that is on but unsigned-in serves
     nobody" — which is *why* `enable` refuses.
3. **What does produce it.** A **window** Desktop, which can switch the remote desktop on
   while signed out and then sits on `Failed(AccountAuthorization)`; its status is the one
   this loop prints only when that window's process owns the data directory, which is not
   the headless entry's own run.
4. **The decision, and who settles it.** Two of these three statements must be wrong, and
   which one is a product question this lane does not get to answer:
   * if the comment at 476-479 is the intended model, `future remote enable` should *not*
     refuse when signed out (the switch would then be storable without an account), this arm
     becomes reachable, and it must get a test;
   * if the refusal is the model (what `desktop-host.md` documents, and what
     `enable_without_a_login_points_at_auth_login_and_stores_nothing` pins), the arm is dead
     and the notice should be deleted rather than covered.

   **Who to confirm with**: whoever owns the remote control channel's signed-out contract —
   concretely the author of `future remote enable`'s refusal and of the window's Remote page
   (`desktop/src/features/remote/`), which is the only producer. There is no `CODEOWNERS`
   entry for this; the confirmation route is to state the conflict in the next remote or
   headless PR (and against the **Signing in comes first** paragraph of
   `docs/guide/desktop-host.md`, which is the document that would have to change) rather than
   to register a
   waiver here that presumes the answer. Recorded as a decision so it cannot quietly become
   a W2 row: a W2 row would claim "the input does not exist", and what is measured is
   narrower — "no CLI input exists *today*".

For the arithmetic: these 8 lines are neither in the 62 registered nor in the 180 work items
above; they are the third category (**62 registered + 8 decision-pending + 180 work items =
250**), and two of them (`494`, `495`) are closing tokens.

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

## Four product observations made while writing these tests

None is a coverage matter, and none was fixed here (no product semantics were changed in
this lane):

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
4. `remote peers add` tells the reader "The pairing is stored, but the connection did not
   come up: …" (wave 12, row 16) — a state the client runtime cannot produce, because a
   failed first connect is an `Err` and the command reports it as a failure with a `PA0xx`
   sentence. The two contracts disagree about what "paired but not reachable" *is*: a stored
   pairing with a warning, or a failed command. Whoever owns `packages/remote-client` should
   pick one; the coverage row follows from the answer, and the arm is the only place the
   first contract is written down.
5. `headless`'s adopt arm prints "Platform account signed in (…): starting the remote desktop
   for it." on **both** halves of its own decision — and the switch-off half starts nothing
   (it re-reads `remote::status()`). An operator who turns the switch off, then signs in from
   another terminal, is told the remote desktop is starting when it is not, and the next line
   they see is `Remote desktop is off (the switch is off)`. The sentence is the one place the
   contract is written down: either it should say "…: the switch is off, so nothing started"
   on that branch, or the branch itself is wrong. Wave 15's test asserts the *effects* (no
   token request while the switch is off, one afterwards) rather than the sentence, so fixing
   the wording does not invalidate the measurement.

## What this file cannot claim

- **Not per-line 100 %.** 250 lines read 0. The lane's compliance statement is the one
  `docs/testing/coverage-waivers-2026-11/README.md` makes for the web lane: no silent gap
  (62 registered + 8 decision-pending + 180 work items = 250, of which 76 are closing
  tokens), no stale row (all 62 registered lines re-read against `cli-lcov-15.info` by
  `waiver_check.py`, `checked=62 stale=0`), no product change made for coverage (no seam, no
  `pub`, no signature — the `test-support` decision is a dev-dependency this crate already
  had, and its whole diff is test code: the four integration targets added in waves 12 and 14,
  one more test inside the existing `headless_cli.rs` in wave 15, and five unit tests inside
  the lib in wave 15).
- **No stale exemptions.** If a later wave covers a registered line, the row is wrong rather
  than old and must be deleted, not reworded: `classify.py` prints the registered lines'
  current counts on every run, so the check is one command.
- **Two of this lane's own claims were falsified before they were kept**, and the file says
  so rather than hiding the retries: the long-wait notice is asserted on the child's stderr
  only because swapping the expected string for a sentinel makes the test fail, and two
  permission-based data tests were **written, run, and deleted** once they proved to reach
  `entry?` instead of the `lstat` arms they were meant for. In wave 11, the first version
  of the `add` test drove **two** pairings in one process and failed with
  `remote_client_stopped`; the cause was not the command but the fixture's one-bridge rule,
  and the test is now one pairing with that rule written down where it bit. Wave 15's two new
  arms were falsified the same way before they were kept: the long-home restore test panics on
  an `Ok` at 300 bytes, and the switch-off login test fails on its own no-token assertion when
  the switch is left on.
- **Wave 13's own first attempt broke a neighbouring test, and the fix is in the test, not in
  the report**: the new `remove` test reached `install_runtime()` before any test had
  installed a *process-lifetime* handle, so `future_host::runtime::set`'s `OnceLock` took the
  test-scoped one and `add_says_which_machine_it_paired_and_who_connects_it` then failed with
  `host shows an invitation`. The instrumented run that caught it is `run13.log` from the
  first attempt (1 failed of 1543); the test now installs the durable handle first, the same
  way `Served::start` does, and three consecutive module runs plus two full instrumented runs
  are green. This is the second time this lane has paid for that `OnceLock` (see the
  `test-support` section) — the rule is written where the next test will read it.
- **Wave 12 falsified this file's own plan for `commands/remote.rs`**, which is why its row
  moved out of *Not waived*: the eleven lines left there were each traced to the invariant
  that confines them (rows 10-15) instead of to a scenario, and the scenario the wave-10 text
  predicted — a live session through the `remote_peer::testing` bridge — cannot reach any of
  them. Writing five rows for one file and *not* writing tests for them is the honest version
  of that finding; a test would have needed a seam (a second `Book::load`, an `await` between
  two reads, a `platform_api_key` hook) and the rule for this lane is no seam.
- **The gate flake has a record of its own**, because it decides the gate and none of this
  lane's numbers: [`cli-safari-gate-flake.md`](cli-safari-gate-flake.md) carries the red
  runs — including one **plain** full `--lib` run of this wave that failed both launch tests
  together (1541/2, 141 s) and the three identical runs after it that were green —, the five
  loaded rounds that did **not** reproduce them, the arms of `safari_start` that each were
  forced (platform gate: refused in 0.04 s; `ulimit -n 64`: still green; the poll window: a
  different message by construction), and the one thing still missing — a red run *with* the
  new `started.is_finished()` diagnosis captured to a file, which is what will name whether
  the spawn, an external listener on the port, or the host's scheduler is what beat the
  bound. The reproduction is therefore open, not "fixed by a retry".
