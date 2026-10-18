# Review — `module-tauri-headless` (§4.1: 195 waived + 88 OPEN = 283 residual, twelve scopes)

**Reviewer unit:** REV-TAURI-HL · **Date:** 2026-10-18 · **Verdict: 2 rows / 3 lines FELL on
measurement; 11 rows / 17 lines fall as registrations; the oscillating pair is real (533/539) but
the ledger claims the wrong line and leaves 533 unclassified; the remaining rows survive a source
check; 4 families were not attacked (stated in §7).**

Reviewed record: `docs/testing/module-tauri-headless.md` §4.1 (the waiver table) + §4.2 (OPEN) +
§8.3 (the "closed" crosswalk), against the pairing's pinned report
**`.future/cov100/tauri-tau-d8-union-lcov.info`** (TAU-D8, `36c568972`). Gate re-run by me:
`staletable.py --gate` → **PASS**, `tauri-headless  282 claims / 283 uncovered / 0 stale / 0
covered / 0 absent`. My own scan of the twelve scopes confirms **283 `DA:0` lines**, per file:
server 58, pty 18, session 14, manager 8, shell 11, cwd 4, headless/mod 51, headless/agent 1,
cli_tool 11, agent_host 23, agent_supervisor 67, skills_bootstrap 17.

## 0. Which figure, and the record moving under the brief

The brief says **300** (TAU-D4, `aff0501b3`). The record and its pairing are at **283** (TAU-D8,
`36c568972`), which is the revision I reviewed. Both numbers are correct for their revisions, and
the 300 → 283 move is the one D8 documented (agent_supervisor 81 → 67, agent_host 26 → 23). I am
**not** reporting the move as a defect; I name it so the final report does not carry 300.

The ticket's own "261" is two revisions older than both and was not used.

Two things I verified about the pairing because the ticket says its predecessor could not check the
figure:

* the registry entry now names **twelve file scopes**, not two prefixes, and resolves to the
  retained union — the gate reads the twelve-scope figure, so the "gate could not check it" defect
  is **closed**, not inherited;
* `claims 282` vs `uncovered 283` is the basename-keying effect the pairing note describes
  (`headless/mod.rs`, `headless/agent.rs` are keyed by basename in the report). `STALE 0` /
  `MISSING 0` hold under suffix resolution, so the 283 is the number to read.

`git diff --stat 36c568972..HEAD` over the twelve scopes is **empty**, so this review's line numbers
are also HEAD's — there is no drift to report.

## 1. Method and scratch tree

`git archive` at the pinned revision, segment-unique directory, probes appended at the end of files
in the scratch tree only:

```bash
R=/tmp/rev-tauri-hl-s1-20261018
git archive 36c568972 | tar -x -C $R/tree
mkdir -p $R/tree/desktop/src-tauri/binaries   # tauri-build sidecar
: > $R/tree/desktop/src-tauri/binaries/future-aarch64-apple-darwin
cp -Rc target/cov-tauri-shell $R/target       # APFS clone of a warm deps tree
export LLVM_COV=~/.rustup/toolchains/1.97.0-aarch64-apple-darwin/lib/rustlib/aarch64-apple-darwin/bin/llvm-cov
export LLVM_PROFDATA=…/llvm-profdata
CARGO_TARGET_DIR=$R/target cargo llvm-cov --no-report --lib -- rev_pr --test-threads=1
CARGO_TARGET_DIR=$R/target cargo llvm-cov report --lcov --output-path $R/probe-fresh.info
```

`test result: ok. 2 passed; 0 failed; 2156 filtered out; finished in 0.02s` (`$R/probe3.log`),
`RUN_EXIT=0`. The profile directory was wiped between runs so the AFTER numbers are one execution
(the first render had accumulated a failed probe run's profraw; that is why the table below quotes
`probe-fresh.info`).

The oscillation measurement runs the **built instrumented test binary** directly, one profile
directory per run, then merges and exports each run separately:

```bash
BIN=$R/target/llvm-cov-target/debug/deps/futureos_lib-45852c9dd7624648
for i in 1..5; do HOME=$R/home LLVM_PROFILE_FILE=$R/osc/srv$i/p-%p.profraw \
    $BIN 'terminal::server::' --test-threads=1; done
for i in 1..5; do llvm-profdata merge -sparse $R/osc/srv$i/*.profraw -o …/pd; \
    llvm-cov export -format=lcov -instr-profile=…/pd $BIN > $R/osc/srv$i/lcov.info; done
```

## 2. Measured falls (2 rows / 3 lines)

| row | lines | before (pinned) | after (one probe run) | probe |
|---|---|---|---|---|
| `terminal/cwd.rs:206` (`ThreadNotWritable`) | 1 | **0** (205=0 too) | **206 = 1**, 205 = 2 | P1 |
| `agent_supervisor.rs:65-66` (`is_stopping()` → `"stopping"`) | 2 | **0, 0** | **65 = 1, 66 = 1** | P2 |

Both probes pass as assertions on behaviour, so the counts are not "the test merely ran": P1 asserts
the refusal variant and its `THREAD_READONLY` code; P2 asserts `phase == "stopping"` **and**
`status.shutdown.is_none()`, and the report shows 58-62 = **0** in the same run — i.e. the arm that
the record says always wins did not run, which is exactly the state the row calls impossible.

### 2.1 `cwd.rs:206` — "no store API writes `readonly`" is a statement about today's writers

The row: `unreachable-by-construction` | "`resolve_for_thread`'s `ThreadNotWritable` arm needs a
thread row with `readonly = 1` or `deleted_at` set. **No store API writes either** (`delete_thread`
hard-deletes), which the file's own test already documents."

Both halves are true and neither is an invariant. `readonly` is a `TEXT`-table column
(`schema.rs:41`) that `get_thread` reads into a `bool`, and this crate's fixtures write store rows
over a **raw connection** wherever the store API does not offer the transition (the record's own
`docs/testing/module-tauri-appdata.md` §S19.1 does exactly this with `PRAGMA foreign_keys = OFF`;
`review.rs`, `store/runs.rs` and `store/cleanup.rs` here all open raw connections in tests). So the
state is a *construction*, not an impossibility, and the arm exists precisely because a row can be
in that state (the session `readonly` spelling is also the one a future pin/read-only feature
writes).

```rust
#[cfg(test)]
mod rev_probe {
    use super::*;

    /// REVIEW PROBE (REV-TAURI-HL): `terminal/cwd.rs:206` (`ThreadNotWritable`)
    /// is registered `unreachable-by-construction` because "no store API writes
    /// `readonly`". The column exists with a default, and the crate's own
    /// fixtures write store rows over a raw connection, so the state is a
    /// construction, not an invariant.
    #[test]
    fn rev_pr_a_read_only_thread_is_refused_by_name() {
        use crate::auth_store::test_support::HomeGuard;

        let _home = HomeGuard::new("rev-pr-cwd-readonly");
        crate::store::initialize_app_store().expect("init store");
        let dir = std::env::temp_dir().join(format!("futureos-rev-cwd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("workspace dir");
        let workspace = crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some("REV".into()),
            path: dir.display().to_string(),
            description: None,
            create_directory: Some(true),
        })
        .expect("create workspace");
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "workspace".into(),
            title: Some("REV".into()),
            workspace_id: Some(workspace.id.clone()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .expect("create thread");
        // The thread resolves before the flag is set.
        resolve_for_thread(&thread.id).expect("writable thread resolves");

        let db = future_app_settings::app_db_path().expect("db path");
        let conn = rusqlite::Connection::open(db).expect("open db");
        let updated = conn
            .execute("UPDATE threads SET readonly = 1 WHERE id = ?1", [&thread.id])
            .expect("mark read-only");
        assert_eq!(updated, 1, "the thread row must actually be flagged");
        drop(conn);

        let error = resolve_for_thread(&thread.id).expect_err("a read-only thread is refused");
        assert!(matches!(error, CwdError::ThreadNotWritable(_)), "{error:?}");
        assert_eq!(error.code(), "THREAD_READONLY");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
```

sha256 of the appended block (`#[cfg(test)] mod rev_probe { … }`, 51 lines):
**`80ef53c8258df609c5aa00a1f8c7860642fc49b4f03a1664fa2d2397f462f372`**.

**Category the row should carry:** `unreachable-in-this-environment` at best
("no *current* writer produces it; a raw connection does"), and the honest 口径 A answer is that the
line is coverable today. **Invalidation token:** any store API that marks a thread read-only (a pin
feature) — the token is already half-present, since the loader materialises the field.

### 2.2 `agent_supervisor.rs:65-66` — the "first branch always wins" invariant is false

The row: `unreachable-by-construction` | "requires `snapshot() == None` **and**
`is_stopping() == true`. `coordinator::begin` sets `ADMISSION.closed = true` and registers the
operation in one critical section, and **neither is ever cleared**, so the first branch (57) always
wins."

The two globals are not one invariant. `is_stopping()` reads `ADMISSION.closed`
(`desktop_shutdown.rs:46-48`); `snapshot()` reads a *different* global,
`OPERATION` (`desktop_shutdown/coordinator.rs:102-104`). They are set together by `begin`, and the
suite's own test **`closing_admission_rejects_new_commands_without_losing_active_guards`
(`desktop_shutdown.rs:544-553`) already produces `closed = true` with no operation registered** —
it asserts `is_stopping()` and never asks `agent_status()`, which is the only reason 65-66 stay
`DA:0`. `reset_for_test` clears *both*, so that route does not work (my first probe tried it and was
refuted — recorded in §9, it is why the arm's real construction is the existing test's state).

```rust
#[cfg(test)]
mod rev_probe {
    /// REVIEW PROBE (REV-TAURI-HL): `agent_supervisor.rs:65-66` is registered
    /// `unreachable-by-construction` because "the first branch always wins".
    /// `closed` and the registered operation are separate globals, and the
    /// suite's own `closing_admission_rejects_new_commands_without_losing_active_guards`
    /// already produces `closed == true` with no operation at all.
    #[tokio::test]
    async fn rev_pr_a_closed_admission_with_no_operation_reports_stopping() {
        let _home = crate::auth_store::test_support::HomeGuard::new("rev-pr-status-stopping");
        crate::store::initialize_app_store().expect("init store");
        super::ADMISSION.lock().unwrap().closed = true;
        assert!(super::is_stopping());
        assert!(crate::desktop_shutdown::snapshot().is_none(), "no operation is registered");
        let status = crate::agent_supervisor::agent_status().await;
        assert_eq!(status.phase, "stopping");
        assert!(status.shutdown.is_none(), "the arm under test has no snapshot");
        super::ADMISSION.lock().unwrap().closed = false;
    }
}
```

sha256 of the appended block (20 lines):
**`068e425cfa8f610565f6c90eddde68a945f325ebfb7935013307cadf8c96436c`**.
It lives in `desktop_shutdown.rs`'s own test module because `ADMISSION` is private to it — the
landable version is those three added lines in the *existing* test at `:544`, not a new fixture.

**Invalidation token:** `agent_status()` being asked after any admission closure without an
operation; or, in production, a `stop` whose operation is dropped before the status is read.

## 3. The rows that move — measured twice (and then fifteen times)

The ticket names `server.rs` 533/539. The record's own §2.0 says the pair is 533 and 539; §4.1's
row at `:468` claims **535** and says so in its own text: "In the D8 union **533 and 535 are
uncovered** while **539** … is covered — so only 535 is claimed here."

Measured on the fixed tree, same binary, `terminal::server::` filter, five runs:

| run | 531 | **533** | **535** | **539** | 547 | 548 |
|---|---|---|---|---|---|---|
| srv1 | 1 | **0** | **0** | 0 | 1 | 1 |
| srv2 | 1 | **0** | **0** | 0 | 1 | 1 |
| srv3 | 2 | **1** | **0** | 0 | 1 | 1 |
| srv4 | 1 | **0** | **0** | **1** | 1 | 1 |
| srv5 | 2 | **1** | **0** | 0 | 1 | 1 |

Six further runs of the single named pump test
(`end_to_end::the_pump_answers_pings_carries_input_and_ends_with_the_shell`) read
**533 = 535 = 539 = 0** every time. The three full-suite artifacts agree:

| artifact (revision) | 533 | 535 | 539 |
|---|---|---|---|
| `tauri-tau-after2` (20:54, second segment) | 0 | 0 | 0 |
| `tauri-tau3-union` (`aff0501b3`) | **1** | 0 | 0 |
| `tauri-tau-d8-union` (`36c568972`, pinned) | 0 | 0 | **1** |

**Fifteen readings, and 535 is 0 in all fifteen.** 533 is covered in 2/5 of my runs and 1/3 of the
full-suite artifacts; 539 in 1/5 and 1/3. So:

* **535's tag is stable** — the row that survives is the brace, not the break, and its
  `unreachable-in-this-environment` category (an ordering between a client frame and
  `SessionEvent::Exited`) matches the evidence;
* **the family that moves is 533/539**, exactly as §2.0 says — and one of them, **533, is `DA:0` in
  the pinned report and has no ledger row at all.** The only claim cell that mentions this family
  (`:468`) enumerates 535 and then states that 533 is uncovered. Prose at `:129`, `:191`, `:496`
  and `:867` mentions 533, which is why `reconcile.py` reports `MISSING 0`: the parser counts a
  mention anywhere in the document, so **a line with no category still passes the gate**. That is
  a 口径 A gap on the one line I checked line-by-line: 533 is `DA:0`, has no row, and `MISSING 0`
  does not see it. (The pairing's `claims 282` is a separate, documented keying effect, not a
  count of unclassified lines.)

### 3.1 A crosswalk disposition that is false as written

§8.3 (`:867`) records the segment-3 closures as "**533, 548** — the oscillating pump arm (§2.0) and
the 'any other client frame is ignored' arm". Measured: **548 = 1 in 5/5 runs** (the disposition is
true, and it is a real closure), **533 = 0 in 3/5** (the disposition is a run-dependent cover read as
a closure). The ticket's rule applies exactly here: "covered by the segment-3 tests" is a claim
about a test, and for 533 the test covers it only under one scheduling. **Verdict: FELL — §8.3's
533 disposition** (and the ledger must then either claim 533 or list it as OPEN/oscillating; it
currently does neither).

## 4. The registration falls (11 rows / 17 lines)

Standard applied, from `docs/testing/waiver-ledger.md`: `unreachable-by-construction` requires an
invariant **in the code** (types, control flow, exhaustive match). A claim of the form "no current
API writes this", "the OS cannot be made to do this here", or "a test binary takes the other cfg
arm" is `unreachable-in-this-environment`, and when a fixture exists it is simply reachable.

| row | category claimed | what the source shows | verdict |
|---|---|---|---|
| `server.rs` 533 (1) | *(none — no row)* | `break` on a failed `attachment.write`; `DA:0` in the pinned report | **Fr** (§3) |
| `server.rs` 115-117 (3) | by-construction | `#[cfg(not(test))] { crate::runtime::spawn(future) }` — cfg-split. The record's own §4.2 last paragraph says it "can ever only be executed by a **real application run**, and the campaign treats that as a blind spot … rather than as waivable work" — that is a reachable-in-this-environment statement contradicted by the category one screen above. The sibling module's review measured the route for the same shape (spawned binaries run the non-test lib, `review-tauri-remote.md` §4.1) | **Fr** |
| `agent_supervisor.rs` 33 (1) | by-construction | same cfg-split (`host().start(...)`); same §4.2 sentence names it | **Fr** |
| `agent_supervisor.rs` 41, 45 (2) | by-construction | `!matches!(spawn_blocking(ensure_agent_running).await, Ok(Ok(())))`: in the **bin** copy `ensure_agent_running` is the real start, so an embedded-agent start failure returns `false` at 45. The record's mechanism ("in `cfg(test)` the callee is a constant `Ok(())`") is a statement about the test copy — a cfg fact, not an invariant | **Fr** |
| `agent_supervisor.rs` 65-66 (2) | by-construction | measured, §2.2 | **M** |
| `manager.rs` 307-310 (4) | by-construction | "a process that survives `SIGKILL`" — that is an OS state (a child in an uninterruptible wait, a zombie held by another reaper), not a code invariant; the ledger's own definition puts "a broken filesystem, a full pipe, ENOSPC" in `unreachable-in-this-environment` | **Fr** |
| `manager.rs` 361 (1) | by-construction | `if session.is_running() { continue; }` — the guard exists because the invariant is **not** structural: `exit_order` is a plain `VecDeque<String>` and `note_exit` (same file, `:328`) never checks that the session has exited; a same-module fixture that calls `note_exit` on a live session and pushes past `EXITED_LIMIT` reaches the line. The record's argument ("a running session can never be in that queue") is a convention over three functions | **Fr** |
| `cwd.rs` 253 (1) | attribution-artifact | the mechanism text is "the message argument of an `assert!` … evaluated only on failure" — that is the **failure-only** class, which this same record labels `unreachable-by-construction` for its identical siblings (`session.rs` 611, `manager.rs` 533). `attribution-artifact` is defined as "a region llvm-cov attributes to a macro or a closing brace"; no neighbour evidence is offered for 253 (251 = 1, 254 has no `DA` record) | **Fr** (category token) |
| `manager.rs` 533 (1) | by-construction | correct category, correct mechanism — listed here only because its sibling `cwd.rs:253` uses the other label for the same construct | **S** (not a fall) |
| `headless/mod.rs` 281 (1) | by-construction | `require_terminal`'s `Ok(())` "needs both stdin and stdout to be terminals, **which a libtest process never has**" — a statement about the harness. Worse, this record's own next step for the neighbouring `147-149` row is "a **PTY fixture** (`portable-pty` is a dependency) so `require_terminal` passes", i.e. the fixture that reaches 281 is already prescribed three rows below | **Fr** |
| `shell.rs` 315 (1) | by-construction | `Some(path.clone())` for a relative candidate; the stated reason is "needs the account login shell (always absolute and executable) **to be rejected first**" — a host state. The very next row (`shell.rs` 325, 329-332) carries the **same** missing thing under `unreachable-in-this-environment`. One of the two categories must be wrong | **Fr** |
| `headless/mod.rs` 1131 (1) | by-construction | `Err(_) => return` of the mock server's accept loop, after a `WouldBlock` arm — a non-`WouldBlock` accept error (`ECONNABORTED`, `EMFILE`) is the ledger's own "broken host" class | **Fr** |

`cwd.rs:206` is the other measured fall (§2.1). Counts: **measured 2 rows / 3 lines; registration
11 rows / 17 lines** (533; 115-117; 33; 41/45; 65-66 measured; 307-310; 361; 253; 281; 315; 1131).

**What this pattern is.** Eight of these ten rows use `unreachable-by-construction` for a
*current-writers*, *current-harness* or *current-cfg* fact (the ninth is the `attribution-artifact` token, the tenth has no row at all). That is the same category drift REV-CH and REV-AGENT
found elsewhere; it does not make the lines covered, but it makes "no input reaches it" false, and
each row's invalidation token is a small, plausible change (a pin/read-only feature, a PTY fixture,
a real app run, a same-module fixture).

## 5. `platform-unmeasured` rows — shape and condition

The ledger now defines two shapes; these rows are **shape (b)**, all of them, with one platform
condition, and they are correctly registered:

| row | what compiles here | what the other platform does |
|---|---|---|
| `pty.rs` 161-165, 621-622 (7) | `if cfg!(windows) { command.env(…) }` — `cfg!` compiles both arms, so the five `env` calls and their mirror assertion exist in this binary | a **Windows** host (ConPTY / `UTF8INPUT`/`UTF8OUTPUT` code page) runs them; macOS takes the other arm |
| `cli_tool.rs` 83 (1) | `if cfg!(windows) { "future.exe" } else { "future" }` | **Windows** spawns the `.exe` spelling |
| `server.rs` 1349-1353, 1962-1966, 1992-1996 (15) | the e2e's `answered_dsr = true; socket.send(Text("\u{1b}[1;1R"))` blocks | a **Windows ConPTY** shell emits the DSR query and blocks on it; the unix fixture shell never asks |

No row in this ledger is shape (a) (a line absent from this report because it is `cfg`-gated out);
all three families are compiled here and refused by this host's runtime. That matches the record's
own wording.

## 6. Per-row verdicts

`M` measured fall, `Fr` registration fall, `S` survived a source check, `N` read but not attacked,
`C` corrected mechanism (survives). Counts in brackets are the row's own line count.

| # | row | n | verdict | basis |
|---|---|---|---|---|
| 1 | `pty.rs` 161-165, 621-622 | 7 | S | `platform-unmeasured` shape (b), Windows ConPTY (§5) |
| 2 | `pty.rs` 434, 456 | 2 | S | helper timeout returns; failure-only |
| 3 | `pty.rs` 745-747, 787-789, 792 | 6 | N | the reap-spin arm; the segment-3 run spun it, this union did not. Token: a slower reaper (§8.3) |
| 4 | `pty.rs` 843, 886 | 2 | S | `match saved { Some => set, None => remove }` with `PATH` always set |
| 5 | `skills_bootstrap.rs` 223, 253, 271 | 3 | S | same restore shape |
| 6 | `headless/mod.rs` 952, 956 | 2 | S | same restore shape; 951/955 covered 4× |
| 7 | `cwd.rs` 451, 454 | 2 | S | same restore shape |
| 8 | `session.rs` 526 | 1 | S | keys re-looked-up **under the same guard** (522 held through 533); 523-525 covered 147× — a real invariant |
| 9 | `session.rs` 611, 796, 853, 1022, 1046, 1223, 1264, 1284, 1289 | 9 | S | failure-only (`panic!` message, assert-message arguments, the untaken `matches!` arm, the post-miss break). 1022's mechanism ("reaching it means the assertion after it fails") is the same class |
| 10 | `manager.rs` 533 | 1 | S | failure-only assertion message (its sibling `cwd.rs:253` is mislabelled, see #34) |
| 11 | `manager.rs` 307-310 | 4 | **Fr** | "a process that survives SIGKILL" is an OS state; §4 |
| 12 | `manager.rs` 355, 361, 365 | 3 | **Fr** (361) / S (355, 365) | 355 is `pop_front` under `len > LIMIT` (structural); 365 is its brace; 361 is the convention guard |
| 13 | `cli_tool.rs` 83 | 1 | S | shape (b), Windows |
| 14 | `cli_tool.rs` 92 | 1 | S | `"bundled"` needs a `future` beside the libtest binary; the record names the test it would break. Token: a per-target sidecar |
| 15 | `cli_tool.rs` 285-288 | 4 | N | `try_wait` `Err` for a live child; plausible, not attacked |
| 16 | `cli_tool.rs` 293, 305 | 2 | N | the post-exit stream re-checks; microsecond window |
| 17 | `cli_tool.rs` 356 | 1 | S | brace artifact; 355 = 274, 357 = 14 |
| 18 | `cli_tool.rs` 672 | 1 | S | assert-message argument, failure-only |
| 19 | `cli_tool.rs` 759 | 1 | S | the NEW-B seam's fail-only `panic!` body |
| 20 | `skills_bootstrap.rs` 104 | 1 | S | `#[non_exhaustive]` wildcard; the compiler requires it |
| 21 | `agent_supervisor.rs` 33 | 1 | **Fr** | cfg-split, and the record says a real run reaches it (§4) |
| 22 | `agent_supervisor.rs` 41, 45 | 2 | **Fr** | the bin copy can fail the start (§4) |
| 23 | `agent_supervisor.rs` 65-66 | 2 | **M** | §2.2 |
| 24 | `agent_supervisor.rs` 157-160, 162, 164-182 | 24 | S | `rfd` native modal; no seam in this crate. Token: an injected dialog |
| 25 | `agent_supervisor.rs` 200-211, 216-218 | 15 | S | same dialog class; the rest of the waiter is covered (188-220 measured > 0) |
| 26 | `server.rs` 115-117 | 3 | **Fr** | cfg-split; §4 |
| 27 | `server.rs` 149, 157, 525, 704 (+168, 1184, 1900 artifacts) | 7 | S | semaphore owned by `SERVER`; `OnceLock`; `Attachment` owns the `Arc<Session>`; the header slice ends before `CRLFCRLF`. Braces: 169 = 2, 1183 = 1 / 1185 = 1, 1899 = 1 / 1901 = 1 |
| 28 | `server.rs` 535 | 1 | S | stable 15/15; ordering mechanism (§3) |
| 29 | `server.rs` 533 | 1 | **Fr** | `DA:0` in the pinned report and unclassified (§3) |
| 30 | `server.rs` 140-142, 163-164, 496-497, 519-523 | 13 | S | transient accept errors, a window the pump keeps empty, and > 1 MiB unread — all `unreachable-in-this-environment` with the state named |
| 31 | `server.rs` 1349-1353, 1962-1966, 1992-1996 | 15 | S | shape (b), Windows ConPTY DSR |
| 32 | `server.rs` 1342-1346, 1135, 1953, 1956, 1988, 2025, 2061 | 11 | N | "some other frame arrived" arms; the fixture shell emits no Text frame. 1135 needs a second `shared_server` caller |
| 33 | `server.rs` 1756, 1957-1959, 1989, 2026, 2062, 2617 | 9 | S | failure-only e2e assertions |
| 34 | `cwd.rs` 253 | 1 | **Fr** | failure-only labelled attribution, no neighbour evidence (§4) |
| 35 | `cwd.rs` 206 | 1 | **M** | §2.1 |
| 36 | `shell.rs` 108, 113 | 2 | S | `getpwuid_r` failure / empty login shell — this host always resolves one |
| 37 | `shell.rs` 303, 315 | 2 | **Fr** (315) / S (303) | 303 is structural (both producers reject empty); 315 is a host state (§4) |
| 38 | `shell.rs` 325, 329-332 | 5 | S | every candidate unusable, incl. `/bin/bash` and `/bin/sh` |
| 39 | `shell.rs` 296, 352 | 2 | S | brace artifacts; 295 = 10 / 297 = 20, 349-350 = 14 / 353 absent |
| 40 | `headless/mod.rs` 281, 352, 649-651, 981, 1131 | 7 | **Fr** (281, 1131) / S (352, 649-651, 981) | 352 is an exhaustive match region; 649-651 is a test future it asserts never completes; 981 is a failure-only `panic!` |
| 41 | `headless/mod.rs` 836 | 1 | S | brace of a `pending().await` block; 835 = 1, 837 = 1 |
| 42 | `headless/mod.rs` 313-315 | 3 | S | spawn-failure after the identity re-check (TOCTOU it closes) |
| 43 | `headless/mod.rs` 354-360, 364, 368-372, 374 | 14 | S | the device-code flow; needs a terminal **and** a live platform — the terminal half is now a PTY fixture, the platform half is not |
| 44 | §8.3 "533, 548 closed" | (disposition) | **Fr** (533) / S (548) | §3.1: 548 = 1 in 5/5; 533 = 0 in 3/5 |

## 7. What I did not reach

* **`pty.rs` 745-747, 787-789, 792** (the reap-spin arm). Landing it needs a child whose reaping is
  slower than one poll — a scheduling property, not a fixture. The record's own note (it spun in the
  tau3 run) is the only evidence.
* **`cli_tool.rs` 285-288, 293, 305.** `try_wait` failing for a live child, and the post-exit stream
  re-checks: I read them and accepted the mechanisms; I did not build a race.
* **`server.rs` 1342-1346 / 1953 / 1956 / 1988 / 2025 / 2061 and 1135.** The e2e frame-variant arms
  need a fixture shell that emits an extra frame, or a second `shared_server` caller.
* **§4.4's 27 renumberings.** The supervisor asked whether its *dispositions* are true. I attacked
  the two disposition kinds on the lines this review reached (`533` renumbered/closed → false;
  `548` closed → true) and the oscillation set; I did **not** re-derive all 27, and I did **not**
  flag its numbers-first shape (the supervisor's note, INCIDENTS F.95).
* **§4.2's 88 OPEN lines.** Read as the ledger's work list, not attacked — they are declared
  reachable by the record itself, so they are not waivers to falsify.

**Flake rates:** the only flakes I relied on are the oscillation readings, and those are reported as
a *distribution over 5 runs plus 15 total readings*, not as a pass/fail rate; no
`flake-rate.sh` before/after is claimed for them. The suite itself was not re-run in full, so no
host-load statement is needed for a number I quote — every AFTER number is one probe run and every
BEFORE number is the pinned artifact.

## 8. What the numbers become

At the pinned revision (283 = 195 waived + 88 OPEN, per the record's §4.3):

* **3 lines are measurably coverable now** (`cwd.rs:206`, `agent_supervisor.rs:65`, `:66`) → 280;
* **17 more lines are registered under a category their mechanism does not support** (10
  registration rows + the 7 that are cfg/host facts); they stay uncovered until the fixtures land,
  but the ledger must stop calling them constructions. With the two fixtures named in §2 and the PTY
  fixture the record already prescribes, a large part of them is reachable from tests this crate
  already writes;
* **1 line (`server.rs:533`) is unclassified** — it is `DA:0` in the pinned report, so 口径 A is not
  met for it until a row exists. Adding it moves the ledger to 283 classified of 283, and the
  oscillating pair to 533 + 535 (with 539 noted as the sibling that flips).
* Do not publish 300 or 261 for this module: 283 is the pinned figure (§0).

## 9. Provenance and handoff

| item | path / value |
|---|---|
| scratch tree | `/tmp/rev-tauri-hl-s1-20261018/tree` (base `36c568972`; twelve scopes identical to HEAD) |
| probe run | `$R/probe3.log` — 2 passed, 0 failed, 0.02 s, `RUN_EXIT=0` |
| AFTER report | `/tmp/rev-tauri-hl-s1-20261018/probe-fresh.info` (single run) |
| BEFORE report | `.future/cov100/tauri-tau-d8-union-lcov.info` |
| oscillation runs | `/tmp/rev-tauri-hl-s1-20261018/osc/srv{1..5}/lcov.info` + `osc/{1..6}/lcov.info` |
| probe hashes | cwd `80ef53c8…`, shutdown `068e425c…` |
| write target | `docs/testing/review-tauri-headless.md` only; no record or source edit |

**New evidence vs the previous attempt:** the first measured reachability of two rows the record
called constructions (`cwd.rs:206` and `agent_supervisor.rs:65-66`), a 15-reading measurement of the
oscillation that separates the stable line (535) from the moving pair (533/539) and shows that the
line the ledger declines to claim is the one that moves, and the refutation of a probe route I tried
first (`begin` + `reset_for_test` — `reset_for_test` clears `ADMISSION.closed` as well, so that
route is not the construction; the existing test's direct flag is).

**Rejected approaches:** assuming the brief's 300 (the record is at 283 and its pairing is PASS — a
review of 300 would have produced false line mismatches, the exact `review-tauri-remote.md` §0.1
trap); treating §4.4's numbers-first rows as claims (supervisor's F.95 note); reading 533's absence
from a claim cell as covered (it is `DA:0` and only prose-mentioning it is what makes `MISSING 0`
vacuous).

**Next useful check:** (1) add the 533 row and re-point its category with the oscillation evidence;
(2) land the `cwd.rs` read-only test and the three lines in the existing admission test; (3) the
PTY fixture for `headless` 147-149/281 — it closes two rows the record currently calls
constructions; (4) re-measure `pty.rs` 745-747 on a loaded host to see whether the spin arm is
worth a fixture.
