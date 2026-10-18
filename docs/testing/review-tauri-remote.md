# Review — `module-tauri-remote` (§4.1: 1195 waived, 0 OPEN)

**Reviewer unit:** REV-TAURI-REMOTE · **Date:** 2026-10-17/18 · **Verdict: 1 row / 2 lines FELL on
measurement; 2 rows / 146 lines fail as registrations; 3 rows carry a wrong mechanism; the
remaining rows survive a source check, and 2 families were not attacked (stated in §7).**

Reviewed record: `docs/testing/module-tauri-remote.md` §4.1 (the 45-row ledger; 1195 residual
lines over `remote/`, `remote_host/`, `remote_peer/`), pinned report
**`/tmp/tauri-tau-d7b-lcov.info`** (2026-10-17 16:02). Tripwires re-run by me: `reconcile.py
--section "4.1"` → **1195 uncovered / 1195 mentioned / MISSING 0 / COVERED 0**;
`staletable.py --gate` for `tauri-remote` → **PASS** (`1484 claims / 1195 uncov / 289 ABSENT /
0 COVERED`). The 289 ABSENT are the `real_network.rs` range claim (§5.2).

The ticket's two shape questions are answered in §3 and §4; the per-row table is §6.

## 0. Pairing, and the revision the report actually describes

### 0.1 The report is pinned to a revision that is not HEAD, and it is not a single commit

I identified the report's source revision from the line tables rather than assuming it:

| file | report max `DA:` line | at `0e80e4204` (14:59) | at HEAD | who moved |
|---|---|---|---|---|
| `remote/transport.rs` | **333** | **333** | 385 | `3662b442b` (16:23 merge, #1158) **+52** |
| `remote/supervisor/start.rs` | **652** | **652** | 661 | same merge, **+9** |
| `remote/publisher.rs` | 653 | 653 | 653 | unchanged |
| `remote/supervisor/shutdown.rs` | 286 | 286 | 286 | unchanged |
| `remote/verify_e2e.rs` | **2436** | 1971 | **2437** | the unit's own TAU-A3 work, committed at `d44870aed` |
| `remote_peer/runtime.rs` | **2732** | 2710 | 2736 | the unit's own tests |

**So the ledger is pinned to `0e80e4204` + the unit's uncommitted TAU-A3 test file** (a tree that no
single commit contains), and **two of the ledger's largest rows have drifted at HEAD**:
`transport.rs`'s 31 lines are numbered against a file 52 lines shorter, and `start.rs`'s 138 against
a file 9 lines shorter. This is drift, not a row defect — but the anchors must be re-read before the
final report quotes 1195, and every probe below is built on the revision its row describes
(verify_e2e at HEAD, which is byte-identical to the report's numbering; the rest at `0e80e4204`).

Verified against the source, at that revision: `transport.rs:231-244` **is** the
`Err(AppError::RemoteAuthorization)` arm of `connect_nats` (the broker rejecting the refreshed
credential), `:267-274` **is** `build_transport`'s error arm, and `:293-300` **is** the
generation-changed block. The ledger's three mechanisms for this file are **correct at its own
revision** — an earlier pass of mine read them against HEAD and produced a false mismatch; that is
recorded here because it is exactly the trap §0.1's re-pinning is meant to avoid.

### 0.2 Ledger integrity

`reconcile.py`: MISSING 0, COVERED 0 — no row claims a line the report shows covered, and every
uncovered line is registered. `staletable.py`: STALE 0. The only structural defect is the
`real_network.rs` range (§5.3).

## 1. Method and scratch tree

`git archive` (never `git worktree add`), a **segment-unique** directory:

```bash
R=/tmp/rev-tauri-remote-s1-20261017
git archive 0e80e4204 | tar -x -C $R
mkdir -p $R/desktop/src-tauri/binaries
: > $R/desktop/src-tauri/binaries/future-aarch64-apple-darwin     # tauri-build sidecar
cp -Rc target/cov-tauri-shell $R/target                            # APFS clone of a warm deps tree
# the unit's uncommitted TAU-A3 files, i.e. the revision the ledger's verify_e2e rows describe:
cp <worktree>/desktop/src-tauri/src/remote/{verify_e2e,test_support,tests}.rs $R/desktop/src-tauri/src/remote/
```

Run (toolchain 1.97.0, cargo-llvm-cov 0.8.7, `LLVM_COV`/`LLVM_PROFDATA` from the toolchain):

```bash
CARGO_TARGET_DIR=$R/target cargo llvm-cov --no-report --lib -- rev_pr_a_scripted_monitor --test-threads=1
CARGO_TARGET_DIR=$R/target cargo llvm-cov report --lcov --output-path $R/probe.info
```

Result: `test result: ok. 1 passed; 0 failed; 2121 filtered out; finished in 0.70s` (`$R/probe3.log`),
`RUN_EXIT=0`. The report contains one `/private/tmp/rev-tauri-remote-s1-…` SF block per file; every
AFTER number below is read from that block.

## 2. Verdict summary

| verdict | rows | lines |
|---|---|---|
| **FELL — measured** (the line's `DA` count moved 0 → 1 under a probe) | 1 | **2** |
| **FELL — registration** (category wrong; the missing thing is a fixture this machine can build, or a policy decision) | 2 | **146** |
| mechanism correction (line survives; stated reason wrong/incomplete) | 3 | 3 |
| SURVIVED (source-checked; token named) | 37 | 1037 |
| NOT ATTACKED this segment (carried; §7) | 2 | 7 |
| **total** | **45** | **1195** |

## 3. The measured fall — `remote/verify_e2e.rs:1429` and `:1572`

The ledger registers the pair as `unreachable-in-this-environment`: ":1429 (the baseline's `Some(url)`
arm) and :1572 (the `if let` that consumes it): reaching either requires a live monitor during a real
measurement, i.e. exactly the scripted-endpoint problem above. Registering two plumbing lines is
cheaper and more honest than a fixture that fabricates the numbers the assertion compares."

**Measured: both registered lines execute.** Before = the pinned full-suite report; after = a run
of **this one test alone** (`probe.info`), which is why the neighbours read differently — in the full
suite other tests already execute the `match`/`if let` headers, while the two arm bodies never did.

| line | source | before (full suite) | after (probe alone) |
|---|---|---|---|
| 1428 | `let before = match &monitor {` | 6 | 1 |
| **1429** | `Some(url) => Some(e2e.phone_accounting(url).await.expect("phone connection")),` | **0** | **1** |
| 1430 | `None => None,` | 6 | 0 |
| 1571 | `if let (Some(before), Some(url)) = (before, monitor.as_deref()) {` | 6 | 1 |
| **1572** | `assert_broker_accounting_against(&e2e, before, url, session).await;` | **0** | **1** |

The probe supplies a **scripted loopback monitor** and drives the shipping function
`run_real_traffic_with(.., Some(url))`. It answers every `verify-phone-pair_e2e-<pid>-<n>` name in a
window with zeroed counters, so the *baseline* arm runs; the arithmetic that follows then fails (a
zeroed monitor cannot match the receipt) and is caught, because **reachability is what the row
claims**. The probe passes.

```rust
    /// REVIEW PROBE (REV-TAURI-REMOTE): `verify_e2e.rs:1429` (the baseline's
    /// `Some(url)` arm) and `:1572` (the `if let` that consumes it) are
    /// registered `unreachable-in-this-environment`, on the argument that the
    /// window's second reading must be known before the run. The *baseline*
    /// reading has no such requirement: any monitor answer makes the arm run.
    #[tokio::test]
    async fn rev_pr_a_scripted_monitor_takes_the_baseline_arm() {
        // NB: run_real_traffic_with takes mock_agent_lock itself; taking it here
        // first deadlocks the non-reentrant mutex (measured: 3m41s, 0% CPU).
        let nats = FakeNats::start().await;
        let pid = std::process::id();
        // The publisher mints the pair id at E2e construction, so every
        // candidate id in a window is offered the same counters.
        let rows: Vec<serde_json::Value> = (1..600)
            .map(|n| {
                json!({
                    "name": format!("verify-phone-pair_e2e-{pid}-{n}"),
                    "in_msgs": 0u64, "in_bytes": 0u64,
                    "out_msgs": 0u64, "out_bytes": 0u64,
                })
            })
            .collect();
        let url = serve_connz(&json!({ "connections": rows }).to_string()).await;

        let journal = fixture_journal();
        let entries = traffic_entries();
        let broker = Broker::Real { url: nats.url().to_string() };
        // The arithmetic panics on a zeroed monitor; caught so that reachability,
        // not the numbers, is what this probe observes.
        let outcome = std::panic::AssertUnwindSafe(super::run_real_traffic_with(
            broker, &journal, "sess_rev_monitor", "run-verify", &entries, Some(url),
        ))
        .catch_unwind()
        .await;
        assert!(outcome.is_err(), "the zeroed monitor cannot satisfy the receipt arithmetic")
    }
```

**What this says about the mechanism.** The record is right that the *second* reading cannot be
scripted in advance; it is wrong that the lines are unreachable here. `:1429` is the **baseline**
read and `:1572` only needs `before` to be `Some` — neither requires the run's byte count. The
record's own §8.4 names the rest of the route itself ("a pre-scripted server would have to encode a
fixture-specific constant"); a constant read off `e2e.phone_rx_bytes` after one identical fixture run
is a two-phase fixture, and the existing sibling test
(`the_accounting_window_reads_the_monitor_and_accepts_a_truthful_broker`) already computes exactly
those numbers *after* driving the traffic. **A constant is not an impossibility.**

**Invalidation token:** deleting the `Some` arm, or making `phone_accounting` answer `None` for every
URL. Neither is needed — the lines are coverable today.

## 4. The two shape questions

### 4.1 The `cfg(not(test))` supervisor block (132 of the record's 138 `start.rs` lines)

**Is the tag falsifiable without the production seam?** **Yes — and the record already proved it for
a prefix without noticing what it had proved.** The 132 lines are not absent from a coverage build
because they are dead; they are absent from the **lib test binary** because `spawn_start_retry` and
its siblings put `#[cfg(test)] return;` beside a `#[cfg(not(test))]` real body (`start.rs:245-248`,
`:307-310`, `:361-423`, `:427-494`). The non-test copy is linked into every binary, and the record's
own measurement shows the harness already runs it: the integration tests spawn the instrumented
`futureos`/`futureos-headless` children, and those children reach `:250-258`, `:260-282`, `:638-648`.
So the route exists, is exercised, and is the *only* mechanism needed for the rest.

**Is calling it `unreachable-in-this-environment` honest, or a request in disguise?** It is a
**request in disguise**, for a fixture that drives a child process, and the honest categories are
two different ones:

* the **exit paths** (`:274` stop-or-Ready, `:283` non-retryable, `:286-298` the post-loop tail) and
  the **reconnect/supervisor bodies** (`:312-336`, `:369-421`, `:433-494`) are **reachable here with
  a better fixture**: a child process plus a broker whose lifetime the test controls. Two concrete
  constructs, no product change:
  1. **stop during the retry sleep** — `SUPERVISOR.start_requested` is cleared by
     `shutdown.rs:49` inside `stop_with`; the loop reads it at `:274` on its next wake. The child the
     record already runs is in that sleep (one retry iteration is measured); a test that asks the
     child to shut down while the retry task sleeps reaches `:274` and then `:286-298`.
  2. **a CONNECT refuser** — a ~30-line loopback responder that answers `CONNECT` with
     `-ERR 'Authorization Violation'` makes `classify_nats_connect_error` (its
     `Authentication | AuthorizationViolation` arm) returns `AppError::RemoteAuthorization`, i.e. a **non-retryable** start
     status, so the loop breaks at `:283` and runs the whole post-loop tail. The same responder is
     the missing fixture for `transport.rs:231-244` (§5.1) — one fixture, two families.
  Killing a live broker under the child (which `FakeNats::kill()` already does in-process) is the
  natural construct for `spawn_runtime_reconnect`/`spawn_runtime_supervisor`.
* therefore the correct registration is **"excluded by supervisor ruling (§6.2), fixture not built"**
  — a *policy* exclusion, which the waiver ledger has no category for and which must not be recorded
  as an environment property. The ruling itself may stand; **the row's category does not**.
  **Verdict: FELL as a registration (132 lines).**

**Invalidation token:** delete the `#[cfg(test)]`/`#[cfg(not(test))]` split (then the lib suite
compiles the body and the lines are ordinary coverage); or spawn the child with a failing broker.
I am not asking for the first — it is the one the supervisor refused, and it is not needed.

Two smaller rows in the same block survive on their own terms: `:70/:81/:93/:124/:215` (5 lines) are
state changes inside another task's window — a real `unreachable-in-this-environment`; `:158` (the
`else` of `web_client_enabled()`) is `unreachable-by-construction` because the test copy is the
constant `true` (`web_server.rs:34-40`) — correct as written.

### 4.2 The rows that move — measured twice (39 times, in fact)

The ticket names "the publisher.rs rows". I scanned **every lcov report under `/tmp`** (392 reports,
150 of which contain `remote/publisher.rs`) and read the row's lines in each. The finding inverts the
premise:

| line | construct | readings |
|---|---|---|
| 336 | `return` when `publish_batch` fails mid-flush | **0 in 150/150** |
| 532 | `return` on `spawn_blocking(..).await` `JoinError` | **0 in 150/150** |
| 612 | `Ok(false) => {}` on the activity lane | **0 in 150/150** |
| 613-619 | the activity lane's `Err` arm | **0 in 150/150** |
| 624 | `}` (block end) | **0, 1, 2, 3** across reports |
| 628 | `continue` on a `None` workspaces read | **0, 1, 2, 3** across reports |

Cleanest single observation — **one measurement pair, two readings**: `/tmp/d8-gui.info` (22:41)
reads 624=**0**, 628=**0**; `/tmp/d8-headless.info` (22:49, same source, same unit's pair) reads
624=**1**, 628=**1**. The same source also gives 08:15 gui = 0 and 08:31 headless = 2
(`appdata-s17` pair). So the *moving* lines are 624/628, **and the record already removed them from
the row** — its own text says they "read `DA:1` in lcov2 and `DA:0` at 20:54 and are dropped". The
ten lines the row actually claims are stable at 0 in every one of 39 independent runs.

**Verdict: SURVIVED (10 lines, 150 readings). Invalidated by:** calling `Transport::clear()` between
two ticker runs (the credential-refresh event) after a channel was established — `secure.rs:105-123`
drops `server.current`, so the next `secure::publish` returns `Ok(false)` at `:612`; a broker killed
between ticks gives the `Err` arm at `:613-619`. Both are production states, not seams — but the
activity lane is only reached after a *successful* session snapshot (`:560-563` must run first), so
a fixture must establish the channel, publish once, then clear. I did not build that fixture; the
row is reachable in principle and the token above is the exact change.

**A second correction the sweep produced:** 624/628's oscillation is the same phenomenon §S26.2 of
the appdata record documents (a brace/`continue` region attributed differently between two green
runs of one tree). The record is right not to claim them; it should also not carry the general
sentence "a line that was covered at 19:38 was `DA:0` at 20:54 with the same tests" as evidence that
*claimed* rows move — none of the claimed ones does.

## 5. The other registration findings and corrections

### 5.1 `remote/transport.rs:231-244` — FELL as a registration (14 lines)

The category is `unreachable-in-this-environment` ("needs a broker or a concurrent `stop()` to land
inside the refresh window"). The arm is the **refreshed credential rejected by the broker**
(`Err(AppError::RemoteAuthorization)` from `connect_nats`), and the missing thing is a *server that
answers CONNECT with `-ERR 'Authorization Violation'`* — a loopback socket this machine can host in
~30 lines, which `async_nats` maps through `ConnectErrorKind::AuthorizationViolation` to exactly
`RemoteAuthorization` (`classify_nats_connect_error`, its `Authentication | AuthorizationViolation`
arm). That is a fixture, not a host property. **Under the
ledger's own discriminator ("is the line reachable here with a better fixture?") the row cannot be
`unreachable-in-this-environment`.** The same responder closes `start.rs:283` (§4.1). I did not build
it; the verdict is a registration defect, with the construct named.

`transport.rs:267-274` (`build_transport` fails after a refresh) is the same family — the secure
handshake must refuse, which the same class of fake can do — but I did not establish that
`build_transport`'s failure is producible without a deeper fake, so I mark it **SURVIVED with the
token "a peer that refuses the secure handshake"** rather than a second fall. `:197` and `:293-300`
are genuine interleavings (two awaits, a concurrent `stop()`); **SURVIVED**.

### 5.2 Mechanism corrections (the line survives; the reason is wrong)

| row | recorded mechanism | what the source shows |
|---|---|---|
| `remote_host/files.rs:1145` | "a `std::fs::copy` that fails after a successful size check" | the copy is the **fallback when `hard_link` fails** (`:1144`); the size check is *after* it (`:1147`). The line needs a filesystem that refuses `link()` — cross-device (`transfer_root()` is `temp_dir()`, the cache is under `HOME`), or a non-POSIX-link volume. Not this host's default layout; keep the row, fix the words. |
| `remote_host/files.rs:1183` | "a `remove_file` on a path a concurrent writer already removed" | `:1183` is the `remove_file` **inside the rename-failure arm** (`:1182`); the line executes when `rename` fails, not when the remove does. Same verdict, corrected mechanism. |
| `remote/commands.rs:885` | `unreachable-in-this-environment`: "a codec-dependent size heuristic … no construction argument applies" | the argument *is* a construction argument: `plain` is `serde_json` text, so its per-byte entropy is bounded (ASCII ≤ 7 bits; raw UTF-8 ≈ 7), while the arm needs `compressed ≥ plain`, i.e. ≥ 8 bits/byte plus gzip's 18-byte trailer. The honest category is `unreachable-by-construction` **with the codec invariant named** (deflate cannot expand text past its own overhead). The record explicitly rejected that label on the grounds that no construction argument applies; this review disagrees and states the invariant. |
| `remote/real_network.rs` "1-1065" | a range | the report has **788** `DA` records there and `staletable.py` calls **289** absent: the row over-claims a range over lines with no record (the F.94 shape). Not a fall (the cell says "788 measured lines"), but the range should be the measured line set. |

### 5.3 Rows I checked and left standing

`remote/secure.rs:814` (an inert transport that opens — the `Ok` arm), `remote/lifecycle.rs:40`
(every `watch::Sender` dropped while the receiver is borrowed from the same struct — type-level),
`remote/web_server.rs:43` (the `cfg(not(test))` arm), `test_support.rs:1790/1804/2477` (failure
reports of invariants that hold), `remote_host/read_pages.rs:70` (the eviction loop's only exit),
`remote_host/business/providers.rs:58-64` (derived `Serialize` cannot fail), `remote_host/lean.rs:501`
(`as_object_mut` after a `kind` check) and `:1222` (a fixture validator's guard),
`remote_host/business/catalog.rs:104-106` (the fork contract), `remote_peer/session.rs:767` and
`remote_peer/runtime.rs:152-154` (cfg-split), `remote_peer/runtime.rs:2721` and
`remote/commands.rs:318/474`, `remote_host/pairing.rs:124`, `remote_host/business/history.rs:273`,
`remote/test_support.rs` artifacts (neighbour counts re-read: 318 → 313-317 covered, 474 → 469-473,
273 → statements above covered, 124 → 123 covered, 2721 → 2717-2720 covered). These are correct as
written; each row's invalidation token is the type/cfg/constant it names.

## 6. Per-row verdicts

`M` measured fall, `Fr` registration fall, `C` corrected mechanism, `S` survived source-checked,
`N` not attacked. Neighbour/count evidence is from `/tmp/tauri-tau-d7b-lcov.info`.

| # | row (lines) | n | verdict | basis |
|---|---|---|---|---|
| 1 | `supervisor/start.rs` 251…646 (the cfg-split block) | 132 | **Fr** | §4.1; the child route already covers a prefix; two constructs named; category is a ruling, not an environment |
| 2 | `supervisor/start.rs` 70, 81, 93, 124, 215 | 5 | S | state changes inside another task's window, no seam; token: a test-only pause point |
| 3 | `supervisor/start.rs` 158 | 1 | S | `web_client_enabled()` is constant `true` in the test copy; token: flip the constant |
| 4 | `verify_e2e.rs` 1429 | 1 | **M** | §3, 0 → 1 |
| 5 | `verify_e2e.rs` 1572 | 1 | **M** | §3, 0 → 1 |
| 6 | `verify_e2e.rs` 341-416 (InstalledHost) | 64 | S | only caller is the `#[ignore]`d real-network harness; token: a test that calls `install_host` with a fake broker |
| 7 | `real_network.rs` 1-1065 | 788 | S | every test `#[ignore]`d (needs nats-server + agent + model); token: remove the ignore |
| 8 | `verify_e2e.rs` 1927, 2125 | 2 | S | artifacts (env-restore `Some` arm; `/connz` accept block brace), neighbours covered |
| 9 | `transport.rs` 231-244 | 14 | **Fr** | §5.1; a CONNECT-refuser fixture is buildable here |
| 10 | `transport.rs` 267-274 | 8 | S | `build_transport` failure; token: a peer refusing the secure handshake |
| 11 | `transport.rs` 197, 293-300 | 9 | S | interleavings of two awaits with a concurrent `stop()` |
| 12 | `supervisor/shutdown.rs` 12, 26, 95-96, 106, 225, 233-250 | 19 | S | timing/state injections (`stop_gracefully` spawn fallback, `Some` mobile-disconnect arm, a `start_once` scripted to fail). Token: a `start_once` failure hook |
| 13 | `publisher.rs` 336, 532, 612-619 | 10 | S | 150 readings, all 0 (§4.2); tokens: `Transport::clear()` between ticks, a killed broker |
| 14 | `commands.rs` 318, 474 | 2 | S | closing braces of covered blocks |
| 15 | `commands.rs` 345, 800, 900, 903, 915 | 5 | S | epoch/seal/flush failures between statements; `:903` is the cfg-split arm (token: flip the cfg) |
| 16 | `commands.rs` 879, 882 | 2 | S | infallible `Vec` sink |
| 17 | `commands.rs` 885 | 1 | **C** | category argument is a construction argument (§5.2) |
| 18 | `transfer.rs` 44-48, 103 | 6 | N | not attacked (fake-broker refusal / seal failure) |
| 19 | `test_support.rs` 678, 683 | 2 | S | `platform-unmeasured` **shape (b)**: a Windows/Linux host whose `probe_sandbox` reports the other answer |
| 20 | `secure.rs` 814 | 1 | S | an inert transport cannot open (asserted three lines above) |
| 21 | `web_server.rs` 43 | 1 | S | the `cfg(not(test))` arm |
| 22 | `lifecycle.rs` 40 | 1 | S | type-level: the sender is the struct's own field |
| 23 | `test_support.rs` 1790, 1804, 2477 | 3 | S | failure reports of invariants that hold |
| 24 | `sync_measurement.rs` 822-861 | 40 | S | the `#[ignore]`d `serve_real_snapshot` harness (isolated DB, real agent, browser) |
| 25 | `lean.rs` 501, 1222 | 2 | S | object-mut after a `kind` check; a fixture validator's guard |
| 26 | `lean.rs` 1246-1249, 1414-1418, 1423-1426, 1428-1429 | 15 | S | the two `#[ignore]`d drivers' env gates |
| 27 | `lean.rs` 1354, 1453 | 2 | S | brace artifacts |
| 28 | `business/catalog.rs` 104-106 | 3 | S | the fork contract (`Ok` always carries an id) |
| 29 | `business/catalog.rs` 122-128, 130 | 8 | S | a concurrent delete/store failure between one function's statements |
| 30 | `business/settings.rs` 148-150, 176-180 | 8 | S | `platform-unmeasured` **shape (b)**: `product_sandbox_available()` is `Ok(true)` on macOS |
| 31 | `business/prompt.rs` 145-151 | 7 | S | the receipt task dying before its send; no caller retains it |
| 32 | `business/providers.rs` 58-64 | 7 | S | derived `Serialize` cannot fail |
| 33 | `business/wire_limits.rs` 717-722 | 6 | S | an `#[ignore]`d driver's env gate |
| 34 | `files.rs` 367 | 1 | S | shadowed by `MAX_ATTACHMENTS == MAX_IMAGES` |
| 35 | `files.rs` 1145 | 1 | **C** | it is the `hard_link` fallback (§5.2); token: a link-refusing filesystem |
| 36 | `files.rs` 1183 | 1 | **C** | the `remove_file` in the rename-failure arm (§5.2) |
| 37 | `session_files.rs` 74 | 1 | S | the entry removed between `canonicalize` and `metadata` |
| 38 | `read_pages.rs` 70 | 1 | S | the eviction loop's liveness guard |
| 39 | `pairing.rs` 124 | 1 | S | artifact: the `)?)` of a covered `ok_or_else` |
| 40 | `business/history.rs` 273 | 1 | S | brace artifact |
| 41 | `business/transfers.rs` 59 | 1 | N | `JoinError` of the blocking `list`; not attacked |
| 42 | `runtime.rs` 152-154 | 3 | S | `delay_unit()` cfg-split |
| 43 | `runtime.rs` 1659, 1723, 1736, 1761-1763 | 6 | S | the host must answer an oversized/mismatched chunk; in-process host cannot lie |
| 44 | `runtime.rs` 2721 | 1 | S | brace artifact (2717-2720 covered) |
| 45 | `session.rs` 767 | 1 | S | `require_tls(true)` under `cfg(not(test))` |

**Counts.** Measured falls **1 row / 2 lines**. Registration falls **2 rows / 146 lines** (132 + 14).
Mechanism corrections **3 rows / 3 lines** (plus the `real_network` range note). Survived **37 rows /
1037 lines**. Not attacked **2 rows / 7 lines** (`transfer.rs` 6, `business/transfers.rs` 1).
**45 rows / 1195 lines**, matching §4.1's ledger exactly (checked by summing the row cells).

## 7. What I did not reach, and what it would need

* **`transfer.rs:44-48, 103` (6 lines).** A fake broker refusing `queue_subscribe` — `FakeNats` does
  not model a refusal; the CONNECT-refuser of §4.1 is the nearest relative.
* **`business/transfers.rs:59` (1 line).** `spawn_blocking`'s `JoinError` needs the blocked
  `session_files::list` to panic. A probe would have to make `list` panic without editing it — no
  input does that; likely a genuine waiver.
* **`remote_peer/runtime.rs:1659/1723/1736/1761-1763` (6 lines).** Reaching them needs a host that
  lies about `totalBytes`; the in-process host computes it. The record's mechanism is plausible; I
  only read it. (These are inside row 43, which I marked survived on that reading.)
* **`supervisor/shutdown.rs` and `commands.rs` timing rows** were read, not driven.
* **Deliberately not re-reviewed:** `verify_e2e.rs:1838`/`:1896-1897` are **not** in this ledger —
  §4.2 records them as closed by two scanner tests (`a_fn_without_a_body_is_not_a_call_site`,
  `a_nested_block_comment_is_blanked_whole`). A first draft of this table listed them as a row; the
  ledger does not, and the report agrees (they have no `DA:0` record in `d7b`).

## 8. What the numbers become

At the record's revision (1195 waived / 0 OPEN):

* **2 lines are measurably coverable now** (`verify_e2e.rs:1429`, `:1572`) → 1193 if covered;
* **146 lines are registered under a category their mechanism does not support** (132 `start.rs`,
  14 `transport.rs`); they stay uncovered until a fixture is built, but the ledger must not call
  them an environment property. With the constructs named in §4.1/§5.1, a unit that builds one fake
  NATS responder and drives the existing child can close a large part of them;
* **do not quote 1195 at HEAD**: `transport.rs` and `start.rs` have drifted (+52/+9), so the anchors
  in those two rows no longer resolve. Re-pin first (both revisions are named in §0.1).

## 9. Provenance and handoff

| item | path / value |
|---|---|
| scratch tree | `/tmp/rev-tauri-remote-s1-20261017` (kept; base `0e80e4204` + the unit's TAU-A3 test files) |
| probe run | `$R/probe3.log` — 1 passed, 0 failed, **0.70 s**, `RUN_EXIT=0` |
| AFTER report | `/tmp/rev-tauri-remote-s1-20261017/probe.info` (`/private/tmp` SF block) |
| BEFORE report | `/tmp/tauri-tau-d7b-lcov.info` (2026-10-17 16:02) |
| publisher sweep | 392 reports under `/tmp`; 150 contain `remote/publisher.rs` (§4.2) |
| write target | `docs/testing/review-tauri-remote.md` only; no record or source edit |

**New evidence vs the previous attempt:** the first measured reachability of the `verify_e2e`
monitor lines (0 → 1 each), a 150-run measurement of the "moving" rows that shows the *claimed* lines
never move while 624/628 oscillate within a single pair, and the two constructs that make the
`start.rs` block reachable without the withdrawn seam.

**Rejected approaches:** reviewing at HEAD (the ledger's rows for `transport.rs`/`start.rs` describe
`0e80e4204`; HEAD verdicts would be false mismatches — the trap I hit once and recorded in §0.1);
taking `mock_agent_lock` in the probe (deadlocked for 3m41 s at 0% CPU — `run_real_traffic_with`
takes it itself); building the publisher `Transport::clear()` fixture (the activity lane needs a
successful snapshot first; the construct is stated instead of half-built).

**Next useful check:** (1) re-pin the record at HEAD and re-read the `transport.rs`/`start.rs`
anchors; (2) the two scanner probes in `test_support.rs` (expected falls); (3) the fake
CONNECT-refuser, which is the single fixture behind §4.1 and §5.1; (4) the `Transport::clear()`
publisher fixture for `:612`.
