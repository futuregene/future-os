# REV-S6 — adversarial review of the three fixtures segment S6 added

Reviewer `rev-goal-s6`, session `20261017-011151-ee1e305898964d70b6817c79680c1efa`, 2026-10-17.
Host macOS 26 arm64 (Darwin, `rustc 1.97.0` from `rust-toolchain.toml`), load average **11–16**
while this review ran (the checkout is shared with peer units, §11.1).

Subject (`todo_a7bdb3b3979a`): the three fixtures of `docs/testing/module-goal.md` §11.13 —
`orchestration/goal/src/console.rs::rate_limit_backoff_tests` (+ its `#[path]` mock),
`orchestration/goal/tests/webui_open_browser_drive.rs`,
`orchestration/goal/tests/supervision_future_stem_drive.rs` — at the revision the record pins.
The record and the module sources were **not** edited; the only write target of this review is this
file. Every counterexample was built in the scratch tree `/tmp/rev-goal-s6`
(`git archive HEAD`, then nothing else from the worktree) with its own target dir
(`/tmp/rev-goal-s6-target`) and its own private `HOME`.

## 0. Verdict

**All three fixtures stand: every claim they carry is either reproduced or explained, and none of
the ten covered lines is claimed on evidence that does not hold.** I falsified nothing about the
*coverage* they report (every `DA` in §11.13 reproduces), and I broke two *statements in the record*
that surround them — one mechanism and one disclosure sentence — plus one cross-platform category
inconsistency. None of those three changes the ten lines.

| # | the control as handed to me | what I measured | verdict |
|---|---|---|---|
| **1a** | "delete the `is_rate_limit_error` conjunct … confirm the POSITIVE test fails while the control still passes" | deleting the conjunct (`console.rs:5101-5102` → `if record.terminal_state == "error"`) makes **both** drives sleep: the **positive test passes and the control FAILS** at `12016` | **the instructed edit produces the inverse pair** — see §2.1 |
| **1b** | (the edit that produces the instructed pair) | neutralising the whole guard (`if … "error" && false`) makes the **positive test FAIL** at `11974` and the control pass, in 0.02 s | **SURVIVED** — the fixture is falsifiable in both directions, so it is not a tautology (§2.2) |
| **1c** | §11.13(1)'s paused-clock evidence ("`AgentClient::connect` dies in 0.01 s, Timeout expired") | the *conclusion* holds (over 20 paused drive runs the guard is evaluated **0** times and the ten lines stay `DA:0`), but the *mechanism* is one of two shapes and is **load-dependent**: one instrumented binary, one session — connect failed 5/5 on the box as it stood (load 11.6), then succeeded 2/4 once 6 CPU burners were added | **SURVIVED with the mechanism corrected** — see §2.4 |
| **2a** | "point PATH at the REAL opener directory and confirm the record stays empty" | executed with an opener directory that is not the recorder: the non-recorder received `http://127.0.0.1:59886/`, the reviewed record stayed **absent**, and the same production lines (`server.rs:87,88,102-104`) were driven | **SURVIVED** — the assertion can only be satisfied by the fixture's own recorder (§3.1). The literal `/usr/bin/open` was **not** executed; the reason is in §3.3 |
| **2b** | (the other direction, mine) | launcher neutralised → the positive test FAILS at `webui_open_browser_drive.rs:96` after **10.01 s**; the control passes | **SURVIVED** — and it shows the `DA` evidence for `102-104` alone cannot see this (§3.2) |
| **3** | "re-run the sabotage of `supervision.rs:281` and re-check that no watcher is leaked" | with `cmd.arg("goal")` removed the test FAILS at `supervision_future_stem_drive.rs:215` reporting exactly `<copy> supervisor watch --goal goal_future_stem_probe`; the three earlier assertions (register exit 0 + `supervisor registered`, watcher running, `watch.log`) all pass; no `WARNING` from the teardown and no surviving process from the copy | **SURVIVED** (§4) |
| **4** | the two Unix-only assertions have no Windows reading | correct disposition for the **macOS** claim; but the Windows counterpart of the browser arm is already registered as `unreachable-in-this-environment` in §6 — a category §11.12 moved the *macOS* twin **out of**. Parity item, not a coverage hole (§5) |
| **5** | "the 5 s WATCH is the ONLY intentional wall-clock wait in the crate" | **falsified by census**: 23 `sleep` sites in 16 files under `tests/`, several ≥ 1 s (1100 ms, 1200 ms ×4, 2 s ×2), plus 10 ms polls with 5 s deadlines in `src/validator.rs`'s tests — and the same segment's own two new fixtures wait on purpose as well | **FALSIFIED as written**; it does not weaken the paused-clock rejection, which rests on the `DA` result, not on cost (§6) |

Independent machine check on the record's own ledger, with the supervisor's **fixed**
`reconcile.py` (the STALE half that could previously skip a row whose uncovered set had become
empty):

```bash
python3 .future/cov100/reconcile.py --label rev-s6 --report /tmp/r2c-lcov.info \
  --scope /Users/geilige/future-os/.worktrees/cov100-r2/packages \
  --scope /Users/geilige/future-os/.worktrees/cov100-r2/orchestration/tasks \
  --scope /Users/geilige/future-os/.worktrees/cov100-r2/orchestration/goal \
  --doc docs/testing/module-goal.md --section "Round 2"
# [rev-s6] files with uncovered lines=13 uncovered lines=82
#   mentioned in module-goal.md: 82
#   MISSING (uncovered, not mentioned): 0          # exit 0, no STALE block
```

**82 of 82 cited, 0 MISSING, 0 STALE** — the record's ledger does not overstate its residual against
the report it pins, under the corpus-wide STALE loop. So the new coverage this review is about is
*additive*: it moved ten lines from `coverable` to covered, and this is the first certificate of
that claim taken with the fixed tool.

## 1. The frozen artifact, re-verified before attacking it

```
$ md5 /tmp/r2c-lcov.info
MD5 (/tmp/r2c-lcov.info) = a236d44d7a4ef788381252611b04d3c4        # matches the record's pin
$ find orchestration packages -newer /tmp/r2c-lcov.info -type f -name '*.rs'   # empty
$ python3 daread.py /tmp/r2c-lcov.info console.rs 5104 5108 5109 5110 5111
5104=1 5108=1 5109=1 5110=1 5111=1
$ python3 daread.py /tmp/r2c-lcov.info server.rs 88 102 103 104
88=1 102=1 103=1 104=1
$ python3 daread.py /tmp/r2c-lcov.info supervision.rs 281
281=1
```

No goal-scope source is newer than the report, and every `DA` §11.13 quotes reproduces. Two
adjacent facts I needed for the review and checked while I was here:

* the guard the fixture covers is **pre-existing production code**, not something this segment
  added: `git blame` puts `console.rs:5101-5112` on `de19b99cc` (2026-08-24, the then-path
  `orchestration/loop/src/console.rs`);
* the segment's own source change is additive and test-only: `d8c5e4395` adds **176 lines and
  deletes 0** in `console.rs` (the `#[cfg(test)]` fixture + the top-level `#[path]` declaration),
  `34cb6ce2f` rewrites that fixture **inside the same test module** (the `Drive` tuple → struct,
  the `WATCH` const, the assertions), and `1d31503a` adds the two `tests/*.rs` files (423
  insertions, 0 deletions). The record's "no production line was touched" holds.

One thing worth recording for the next reader: the fixture's docblock *was first written the other
way round* — `34cb6ce2f` **deletes** a paragraph asserting that "under a paused tokio clock the drive
is cheap *and* the cool-down becomes observable … `start_paused` auto-advances to the next timer
deadline whenever the runtime has no other work, which is what carries the 45 s `sleep` inside
`run_turns`". The shipped text is the after-the-experiment version. §2.4 shows the experiment was
right to be believed and imprecise about *why* — the exact shape of a claim that needs an
independent verdict.

## 2. Control 1 — the rate-limit cool-down fixture (`console.rs:5104, 5108-5111`)

Command of record for every number in this section (filters the lib target to the two fixture tests,
so the report isolates the fixture rather than the whole suite; the unfiltered full-run numbers are
quoted from the record's own `/tmp/r2c-lcov.info`):

```bash
# in /tmp/rev-goal-s6, with HOME=/tmp/rev-goal-s6-home CARGO_TARGET_DIR=/tmp/rev-goal-s6-target
cargo llvm-cov -p future-goal --no-fail-fast --lcov --output-path <OUT> -j 4 \
  --lib -- rate_limit_backoff_tests
```

**Baseline, unmodified (both tests green, `finished in 5.03s`):**

| line | `console.rs` statement | filtered baseline | record's full run 4 |
|---|---|---|---|
| 5101 | `if record.terminal_state == "error"` | 2 | 68 |
| 5102 | `&& is_rate_limit_error(record.error.as_deref())` | 2 | 5 |
| 5104 | `println!("⏳ rate-limited (HTTP 429) …")` | **1** | **1** |
| 5108-5111 | the `sleep(...)` call, argument, `)` and `.await` | **1,1,1,1** | **1,1,1,1** |
| 5112 | `let mut g = store` after the block | 1 | 67 |

The **5.03 s** is the pair's own cost, and it is the `WATCH` timing out in the positive test: the two
tests run concurrently, the control returns in milliseconds, the positive is dropped by
`tokio::time::timeout(WATCH, run)`.

### 2.1 The instructed edit, run as instructed — the pair comes out inverted

`sabotage-console.py A` deletes the conjunct, i.e. the guard becomes

```rust
        if record.terminal_state == "error"
        {
```

(file sha256 `574fe104e124323fe52a1ba772508fa002fd4c83640abbade90232e3226c7e8f`, one line shorter).

```
test console::rate_limit_backoff_tests::a_non_throttled_error_drives_the_same_path_without_the_cool_down ... FAILED
thread … panicked at orchestration/goal/src/console.rs:12016:14:
only a throttle earns the cool-down; this drive must finish inside WATCH
test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 444 filtered out; finished in 5.01s
```

`12016` is the **control's** `.expect(...)`; the positive test **passed**. With the conjunct gone,
`terminal_state == "error"` is true for *both* drives, so both sleep — `DA` in the sabotaged file
(line numbers shifted by one after the deletion, so they are read against the sabotaged source):

```
  5101: DA=   2          if record.terminal_state == "error"
  5103: DA=   2              println!(
  5107: DA=   2              tokio::time::sleep(std::time::Duration::from_secs(
  5108: DA=   2  5109: DA=   2  5110: DA=   2       # the sleep's argument, `)` and `.await`
  5111: DA=   0          }
```

Two drives reached the cool-down; neither finished it (the `}` is 0 because both futures were
dropped by the timeout). **So the control as worded identifies the wrong edit for its own stated
expectation.** That is not a defect of the fixture — it is evidence *for* it in one direction: the
two tests are separated by exactly this conjunct, and the control is not a second copy of the
positive.

### 2.2 The edit that produces the instructed pair

`sabotage-console.py B` neutralises the whole guard (`if record.terminal_state == "error" && false`;
sha256 `b3f9267fd7d79392426b7cf3461c1919443bedafcf7dfdb15c2654f4f4dfc7bd`):

```
test console::rate_limit_backoff_tests::a_throttled_turn_holds_the_writeback_for_the_whole_cool_down ... FAILED
thread … panicked at orchestration/goal/src/console.rs:11974:9:
a 429 turn must still be inside RATE_LIMIT_BACKOFF_SECS after 5s; the run returned
Some(Err("max-turns (1) reached without validated closure"))
test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 444 filtered out; finished in 0.02s
```

The positive's own stdout in that run shows the turn completing immediately
(`✔ writeback ok — next action synced`), and `DA` in the same run:

```
  5101: DA=   2          if record.terminal_state == "error" && false
  5103: DA=   0              println!("
  5107: DA=   0  5108: DA=   0  5109: DA=   0  5110: DA=   0
  5111: DA=   2          }
```

So the line evidence is symmetric with §2.1: with the guard armed and a throttle text, the cool-down
body executes (1 each); with the guard neutralised, it never does (0 each) **and the positive test
fails**. The fixture is therefore falsifiable in both directions — it is a test of the guard, not a
recording of whatever the drive happened to do.

### 2.3 What the fixture pins, and what it does not

Pinned (measured): (i) the guard is entered and the cool-down body runs for a 429 turn; (ii) the
trigger is the error *text*, since the identical drive with `validator exited 1` finishes inside the
same budget, classifies `HardError` and writes its record (the control, green in both sabotages);
(iii) the cool-down **precedes** the writeback — a regression that moved the `sleep` after the
writeback would write `goal.history` at ~0 s and fail `assert!(goal.history.is_empty())` inside the
5 s window. Not pinned: the cool-down's *duration* (a regression changing `RATE_LIMIT_BACKOFF_SECS`
from 45 s to 3600 s still passes — nothing asserts the sleep's length) and the post-cool-down half of
the run. Those are bounded claims, not defects; the record does not claim them.

### 2.4 §11.13(1)'s paused-clock evidence: conclusion holds, mechanism corrected

The record's disclosure says the free route was tried and fails because
"`AgentClient::connect` dies in 0.01 s with `Failed to connect to agent: Agent readiness could not be
verified: status: Cancelled, message: "Timeout expired"`". I re-ran that experiment in two probes,
added to the fixture module in the scratch tree only (module sha256
`7923e95d3583b700357e23ee4a6d8c0da80566c1654d3e1639354c2a7fce4b33` for the shipped file plus both
probes):

* **probe 1** (`rev_probe_paused_clock_cannot_carry_the_turn`): `AgentClient::connect` alone under
  `#[tokio::test(start_paused = true)]`, panicking if it *succeeds*;
* **probe 2** (`rev_probe_paused_drive_outcome`): the whole `drive_error_turn` — the same call the
  positive test makes — under the paused clock, appending what it observed to
  `/tmp/rev-goal-s6-paused-drive.txt`.

**The record's quoted error text is real and reproducible** (probe 1, connect-failure shape):

```
rev-s6 probe paused-clock: connect failed after 1.234958ms with Failed to connect to agent:
Agent readiness could not be verified: status: Cancelled, message: "Timeout expired", details: [],
metadata: MetadataMap { headers: {} }; local startup is forbidden
```

**But it is one of two shapes, and which one you get depends on machine load.** One instrumented
binary (sha256 `fc5c4f901f90c9bc2803c7f66939dbaf0a412e25970790d130290da5270ad95c`), probed
separately, seconds apart, each `1` = connect succeeded (probe 1 panics), `0` = connect failed:

```
--- 5 runs, box as left by the peers (load 11.6) ---   0 0 0 0 0
--- 6 CPU burners started, 4 runs ---                 1 1 0 0
```

and earlier in the same session, against the same *path*, 5/5 succeeded. A measurement whose
outcome flips like that is not a property of the paused clock.

**Probe 2 shows why the record is right anyway.** Twenty direct runs of the probe-2 binary
(sha256 `e13e2b9bf1152558ee6b7e21c7466af6156d535e247daa7b7e42aec4899903ea`) — all 20 past
`connect` (the `.expect("the scripted mock is reachable")` never fired), and **none** reaching a turn:

```
prompts=0 completed=Some(Err("gRPC 'get_state' failed: status: Unavailable, message:
  \"Agent readiness could not be verified: status: Cancelled, message: \"Timeout expired\" …
  ; local startup is forbidden"))  failure_kind=Some(InfraRecoverable) history=0 events=7   × 6
prompts=0 completed=None failure_kind=None history=0 events=6                              × 14
```

and, merging those 20 runs' profraws:

```
console.rs: 5101=0 5102=0 5104=0 5108=0 5109=0 5110=0 5111=0 5112=0
```

**The guard is never evaluated in a paused-clock drive.** Corrected mechanism, for the record:

> the paused clock auto-advances to whatever gRPC deadline is armed when the runtime parks. The
> failure therefore surfaces at the readiness verification of the first gRPC call rather than at a
> fixed place: in the 20-run batch the `connect` step passed every time and the drive died at the
> first `get_state` (6 runs, reported as `InfraRecoverable`; the other 14 were still pending when the
> 5 s window closed), while probe 1 shows `connect` itself failing in other runs. Either way the drive
> never reaches a turn, so the ten lines stay `DA:0`. The failure's *location* is load-sensitive; the
> *guard never being reached* is what the rejection rests on, and that is measured 20/20.

**Effect on the fixture's justification: none.** The 5 s is not a price paid for convenience — it is
the only route that covers these lines short of the injectable constant the unit declined to take
(PLAN 4b). What needs a one-sentence fix in §11.13(1) is *which call dies*, and the claim that the
connect failure is deterministic.

## 3. Control 2 — the `--open` launcher fixture (`webui/server.rs:88, 102-104`)

```bash
cargo llvm-cov -p future-goal --no-fail-fast --lcov --output-path <OUT> -j 4 \
  --test webui_open_browser_drive
```

Baseline: **2 passed, 0 failed, 0.38 s**; `DA server.rs 87=1 88=1 102=1 103=1 104=1` — identical to
the record's full run.

### 3.1 An opener directory that is not the recorder cannot write the record

Probe `orchestration/goal/tests/rev_s6_opener_dir_probe.rs` (scratch; source in §7). It runs the
**same production path** (`run_server(root, 0, open_browser = true)`) with `PATH` pointing at an
opener directory that is not the reviewed recorder, and checks both halves at once:

```
rev-s6 probe: the non-recorder opener on PATH received http://127.0.0.1:59886/
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.38s
$ daread.py /tmp/rev-goal-s6-p2-probe.info server.rs 87 88 102 103 104
87=1 88=1 102=1 103=1 104=1
```

The probe's own assertions: the non-recorder's file carries the URL, **the reviewed record
`opened.txt` does not exist**, the URL parses as `http://127.0.0.1:<port>/` and that port answers
`200 OK`. So the production call really calls whatever `open` is on `PATH`, and the reviewed
assertion — which reads a file whose path exists *only inside the fixture's own stub*
(`write_opener_stub` embeds `record.display()` in the script body) — cannot be satisfied by an opener
other than that recorder. The production call sites pass the URL alone
(`server.rs:87-88`), so no opener is ever handed the record's path.

### 3.2 The other direction, and a note about the evidence

`sabotage-server.py off` replaces the macOS body with `let _ = url;`
(sha256 `573ad67baaea6d321ed8484d2f0c52921b9cf2ab346e673fb18d0371d264c018`):

```
thread 'open_browser_hands_the_platform_opener_the_live_dashboard_url' panicked at
  orchestration/goal/tests/webui_open_browser_drive.rs:96:9:
the platform opener was never asked to open anything
  (/var/folders/…/T/.tmpeIUEQE/opened.txt absent after 10s)
test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.01s
```

The positive test fails, the negative control passes, and the fixture's own 10 s deadline is what the
cost becomes when the launcher does not run. Worth recording for the *evidence* half: in this
sabotaged run `DA server.rs 102=1 103=1 104=1` as well, because the no-op body still executes — so
the `DA` counts quoted for `102-104` show that `open_in_browser` was **called**, not that it spawned
anything. The spawn is pinned by the recorder's file, which is exactly why the fixture asserts on the
record rather than on the line count. Both halves are needed; neither alone is sufficient.

### 3.3 The literal instruction, and why I did not follow it

The instruction was to "point `PATH` at the REAL opener directory and confirm the record stays
empty". `/usr/bin/open` was **not executed**: on this host it hands the URL to the operator's default
browser, which the fixture's own header forbids ("the real opener must never be launched … a
regression that ignores the stub cannot open a browser on the operator's desktop"), and this box is a
live GUI session. The property under test does not depend on that: what §3.1 measures is that an
opener **which is not the recorder** cannot produce the record, and the real `/usr/bin/open` is such
an opener by construction — it writes no file, and the record's path is a fresh
`tempfile::tempdir()` path appearing nowhere but inside the fixture's stub. I chose the executed
substitution plus this mechanical argument over a browser window on the operator's desktop; if the
supervisor wants the literal run, it should be scheduled, not smuggled into a review.

## 4. Control 3 — the `future`-stem watchdog fixture (`agents/supervision.rs:281`)

```bash
cargo llvm-cov -p future-goal --no-fail-fast --lcov --output-path <OUT> -j 2 \
  --test supervision_future_stem_drive
```

Baseline: **1 passed, 7.35 s**; `DA supervision.rs 276=1 279=1 280=1 281=1 282=1 283=1` (and `281=1`
in the record's full run).

Sabotage (`sabotage-supervision.py off`, sha256
`4043f2cf3b984ea0d204161cc54ffb041ef09a139075868ca3d571086f4f5c77`), with `cmd.arg("goal")`
replaced by a comment:

```
thread 'a_future_named_binary_spawns_its_watchdog_with_the_goal_group_verb' panicked at
  orchestration/goal/tests/supervision_future_stem_drive.rs:215:9:
the detached watcher must be invoked with the `goal` group verb — that is the whole point of
supervision.rs:281: /var/folders/hy/…/T/.tmpMa5KOK/future supervisor watch --goal goal_future_stem_probe
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.87s
```

That is exactly the argv the record predicts (`<copy> supervisor watch --goal G`), read from the
child's own `ps` line — the assertion is the instrument that pins the *content* of line 281, and it
fires. Three things this run also measures:

1. **The premise is measured twice.** The failure is at the *argv* assertion, so the earlier
   assertions already passed in this run: the `register` drive exited 0 with `supervisor registered`,
   `supervision::running(&store, gid)` was true, and `watch.log` existed — i.e. the same copy, spawned
   as `<copy> supervisor watch --goal G`, accepted that spelling too. Both spellings therefore start a
   watcher from this binary, which is precisely why the fixture needed a `future`-named *copy* and an
   argv read instead of a coverage-visible effect.
2. **No watcher is leaked.** `grep -c WARNING` over the sabotaged run's log is **0** (the `Drop`
   guard's "did not release its lock" report never printed), and

   ```
   $ ps -eo pid,ppid,etime,args | grep -E "\.tmp[A-Za-z0-9]+/future"      # empty
   ```

   No process from the copy's directory survives the failing run. (Two `… goal supervisor watch`
   processes do exist on this box — `/opt/homebrew/bin/future … --goal goal_160bc247aa63` and
   `goal_5615137dbe95` — they are the campaign's own watchers, launched from the homebrew path, not
   from the fixture's temp dir; a leak check that greps for `supervisor watch` alone would have
   mis-read them as a leak, which is worth knowing for the next reviewer.)
3. **`DA` is blind here.** After the sabotage the statement does not exist, so there is no `DA`
   record for it (the comment occupying 281 reads `1` from the enclosing region). A `DA`-based check
   would have reported "line 281 covered" for a sabotage that removes it, which is why the fixture's
   argv assertion — not the coverage number — is the evidence for this row.

Production premise checked by reading, not by execution: the unified CLI dispatches *only* on the
group verb (`cli/src/main.rs:33-42`: `agent|tui|channel|channels|goal` → embedded component, anything
else → `future_cli::dispatch`, which has no `supervisor` group and errors with `Unknown command`). So
a `future`-stem binary really needs the `goal` prepend for the child to be a *goal* watcher. I did not
execute the real `/opt/homebrew/bin/future supervisor watch …`, because that would start a watcher
outside this unit's temp root.

## 5. The two Unix-only assertions — disposition, and one parity item

**`supervision_future_stem_drive.rs`'s argv read (`#[cfg(unix)]`) — the record's disposition is
right, and this is a strength gap, not a coverage gap.** Line 281 is platform-independent and the two
portable assertions above it (the register drive exits 0 and the watcher is running, `watch.log`
created) execute it on any platform; what Windows loses is the *discriminator*. A Windows-CI reading
of that assertion is *possible* without a new fixture — read the child's command line through the OS
instead of `ps` (`powershell Get-CimInstance Win32_Process` / `wmic process get commandline`) — but
it is optional, because the line is covered there either way. If the supervisor wants it, the owner
is the goal coverage unit (it owns this file), executed on a Windows host.

**`webui/server.rs`'s Unix-only positive arm — correct for the macOS claim, but the Windows twin now
carries an inconsistent category.** Two facts, both checked:

* on Windows the opener is `rundll32 url.dll,FileProtocolHandler`, i.e. different *lines*; a macOS
  report does not contain them at all, and the macOS `102-104` are `#[cfg(target_os = "macos")]`. So
  the macOS figure cannot speak for them, exactly as §11.13's "what this does not claim" says;
* the round-1 **Windows** record already registers them: `module-goal.md:615` files
  `open_in_browser` (88, 106-110) as `unreachable-in-this-environment`, with the reason "running it in
  a test would take over the operator's desktop".

That reason is *the same sentence* §11.12 used to move the macOS twin out of `unreachable-in-this-
environment` into `coverable` — "an intrusive side effect rather than an impossibility". So the two
platforms now disagree about the same shape, and the Windows row survives only because the round-1
audit predates §11.12. **Recommendation (supervisor):** either re-label the Windows row in §6 for
parity (the honest category is `coverable`-with-a-price, or a policy note that a recorder PE needs a
bin target, which is a PLAN-4b decision), or request the Windows reading — a copy of an existing
executable renamed `rundll32.exe` suffices to *execute* the arm (the same trick control 3 uses with
`future`), but recording its argv needs a dedicated recorder binary, i.e. a manifest change nobody's
coverage write set covers. Owner if a reading is requested: the goal coverage unit for the test, with
the PE-recorder question routed through the supervisor.

No *macOS-hidden* line is unregistered in either fixture: the two `#[cfg(unix)]` test *bodies* are
test code, and test code is not a measured line.

## 6. The "only intentional wall-clock wait in the crate" claim — falsified

§11.13's disclosure says: "The 5 s is real wall clock, spent by one test in the goal lib target. **It
is the only test in the crate that waits on purpose**". A census over the crate
(`sleep-census.py`, which splits each file at its first `#[cfg(test)]` and treats `tests/` as test
code) gives **23 `sleep` sites in 16 files under `tests/`, plus two in a lib test module**, all
deliberate. The table lists the distinct wait sites (a file may hold several) and omits the two new
fixtures of this segment, which are quoted underneath it:

| file:line | wait |
|---|---|
| `tests/public_api_drive.rs:107` | `thread::sleep(1100 ms)` |
| `tests/store_drive.rs:213`, `tests/console_drive_a.rs:1222`, `:1239`, `tests/claim_lease_contract.rs:79` | `thread::sleep(1200 ms)` ×4 |
| `tests/console_lease_lifecycle.rs:146`, `tests/console_writeback_refusals.rs:323` | `thread::sleep(2 s)` ×2 |
| `tests/supervision_watch_drive.rs:340` | `thread::sleep(300 ms)` (plus 2 s deadlines, a 10 s `timeout`) |
| `tests/schema_alignment_contract.rs:113`, `tests/decision_contract.rs:248/257/272`, `tests/store_contract.rs:211`, `tests/monitor_poll_contract.rs:195` | 80 ms, 50 ms ×3, 50 ms, 30 ms |
| `tests/run_notes_drive.rs:200/338`, `tests/reliability_contract.rs:347/365/373` | 20 ms poll ticks, with 30 s deadlines |
| `tests/common/mock_agent.rs:338` | the mock's own pacing sleep |
| `src/validator.rs:186/195` (lib tests) | 10 ms polls under 5 s timeouts; `:207` asserts a 50 ms timeout budget |

and the same segment's two new fixtures are further instances: the browser fixture polls for **10 s**
(`opened_url`), the supervision fixture polls at 100 ms under 15 s deadlines (teardown and the final
lock-release loop). So the sentence is false as written, including about its own segment.

Does it change the suite-cost argument? **Only its framing.** The decisive reason the paused-clock
route was rejected is the measurement in §2.4 (`DA 5101=0` over 20 runs), not the price. What the
sentence was doing was making "5 s, not 45 s" look like the choice that mattered — and the crate
already spends ~10 s of deliberate wall clock in its other suites, so "45 s would have been
disproportionate" is not an argument this evidence set can carry. Suggested replacement, which is
what is actually true: *the fixture costs 5.03 s observed for the pair; the paused route is not a
cheaper route but no route at all — it never reaches the guard (20 runs, `DA 5101=0`) — so the only
alternative was the injectable constant, which the unit did not take.*

## 7. Probe inventory (sources, hashes, commands)

All probes live in `/tmp/rev-goal-s6` (kept alive for the supervisor to hash or re-run) and are
**not** part of the repository. Scratch tree: `git archive HEAD | tar -x -C /tmp/rev-goal-s6`
(HEAD `ed8d4528`; the goal scope is byte-identical to the report's revision, §1), plus the files
below and nothing else. Environment for every run: `HOME=/tmp/rev-goal-s6-home`,
`CARGO_TARGET_DIR=/tmp/rev-goal-s6-target`, `CARGO_HOME=$HOME/.cargo`, the pinned toolchain on
`PATH`, `ulimit -n 10240`.

| probe | sha256 | what it is |
|---|---|---|
| `sabotage-console.py` | `6dda4b025f2e0e7e71b8d37bd905ea7dc0cc8449ab9c2ca5a445db61847337fe` | guard edits A / B / restore for `console.rs` |
| `sabotage-server.py` | `495d81497d6b23ec63f793eef3e064052f197e1342185f495465584880b49235` | launcher off / restore for `webui/server.rs` |
| `sabotage-supervision.py` | `addd9c10dd89581827fc205c1ae64a478dea95dcc12e7ebc6c67b61b21c25cde` | goal-verb off / restore for `agents/supervision.rs` |
| `daread.py` | `90c004465c2bcac22e8b1672a2046b7f54568e5784563449d0cd253366e1bf9b` | `DA` readout by file suffix and line |
| `annotate.py` | `636e62defd852aab57d07b1c3111478647da31c3994a9fd6c2d22956654b4ed2` | a source range tagged with the `DA` counts of a (possibly shifted) file |
| `lcov-from-profraw.sh` | `44d409a2cb4c10c54eac0c0e2e1879bdd91b7b36afe4d44a162422cdb4726a6` | lcov from the profraw a **failing** run leaves behind (`cargo llvm-cov` aborts before its report step) |
| `sleep-census.py` | `3c61366ed2c32cf434873fd379d6f52272d6835439562ec0cf54c2d9223bbc40` | the wall-clock census of §6 |
| `tests/rev_s6_opener_dir_probe.rs` | `35d9bd79132c2db8e25b5f932ccd635a83e9c935f7aa27c3f85ecaa495a2d358` | control-2 probe |
| `src/console.rs` (shipped + both paused-clock probes) | `7923e95d3583b700357e23ee4a6d8c0da80566c1654d3e1639354c2a7fce4b33` | controls 1c |
| `src/console.rs` at HEAD (restored, verified) | `8c0f1a2db935d9e09a1b72bd2f2c5aeb300c21141dace6ef3cfd80357b0f2008` | unmodified anchor for the two sabotages |
| `src/webui/server.rs` at HEAD (restored, verified) | `f6a75890d4f10b75c5cf5aca786a89b89048b1ebd26d6b6d14f063c6e9e34903` | same |
| `src/agents/supervision.rs` at HEAD (restored, verified) | `23aff2ea3839bf59a75cbbe74548e21afceca4ce73f0aa9d406ebddc48d73221` | same |

Each restored file was verified by `shasum -a 256` against `git show HEAD:<path> | shasum -a 256`
in the campaign worktree; all three match. `tests/rev_s6_opener_dir_probe.rs` is an addition, so no
restore applies.

**Probe sources, embedded** (rule 5a/5b — these are the bytes the hashes above name):

`tests/rev_s6_opener_dir_probe.rs` (control 2, §3.1):

```rust
//! REV-S6 probe (scratch tree only).  The reviewed test asserts on `record` — a
//! file whose path is written *inside* the fixture's own recorder stub.  This
//! probe runs the same production path (`run_server(root, 0, open_browser = true)`)
//! with `PATH` pointing at an opener directory that is **not** the recorder.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use future_goal::store::Store;
use future_goal::webui::run_server;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

static PATH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct PathGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    saved: Option<std::ffi::OsString>,
}

impl PathGuard {
    fn only(dir: &Path) -> Self {
        let _lock = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::var_os("PATH");
        std::env::set_var("PATH", dir);
        Self { _lock, saved }
    }
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        match &self.saved {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
    }
}

fn opener_stub(dir: &Path, record: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let stub = dir.join("open");
    let mut file = std::fs::File::create(&stub).unwrap();
    file.write_all(format!("#!/bin/sh\nprintf '%s\\n' \"$1\" > '{}'\n", record.display()).as_bytes())
        .unwrap();
    drop(file);
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    stub
}

async fn wait_for(record: &Path, budget: Duration) -> Option<String> {
    let deadline = Instant::now() + budget;
    loop {
        if let Ok(text) = std::fs::read_to_string(record) {
            let text = text.trim().to_string();
            if !text.is_empty() { return Some(text); }
        }
        if Instant::now() >= deadline { return None; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn an_opener_directory_that_is_not_the_recorder_cannot_write_the_record() {
    let fixture = tempfile::tempdir().unwrap();
    let record = fixture.path().join("opened.txt");
    opener_stub(fixture.path(), &record);

    // The "opener directory": an opener that is not the reviewed recorder.
    let other = tempfile::tempdir().unwrap();
    let elsewhere = other.path().join("elsewhere.txt");
    opener_stub(other.path(), &elsewhere);
    let _path = PathGuard::only(other.path());

    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().to_string_lossy().into_owned();
    Store::open(&root_path).unwrap();
    let server = tokio::spawn(async move { run_server(root_path, 0, true).await });

    let url = wait_for(&elsewhere, Duration::from_secs(5))
        .await
        .expect("the production code must call whatever `open` is on PATH");
    println!("rev-s6 probe: the non-recorder opener on PATH received {url}");

    assert!(
        !record.exists(),
        "an opener other than the recorder must not be able to produce the \
         record the reviewed assertion reads"
    );

    let port: u16 = url
        .strip_prefix("http://127.0.0.1:")
        .and_then(|rest| rest.strip_suffix('/'))
        .and_then(|p| p.parse().ok())
        .unwrap_or_else(|| panic!("unexpected opener argument: {url}"));
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    client.write_all(b"GET /api/overview HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    assert!(String::from_utf8_lossy(&response).contains("200 OK"));

    server.abort();
    let _ = server.await;
}
```

The two paused-clock probes (controls 1c, §2.4), appended inside `mod rate_limit_backoff_tests` in
the scratch `src/console.rs` — they reuse that module's `drive_error_turn`, `spawn_mock`, `MockState`
and `ev` unchanged:

```rust
    /// REV-S6 probe: re-run of §11.13(1)'s paused-clock experiment, on its own terms.
    #[tokio::test(start_paused = true)]
    async fn rev_probe_paused_clock_cannot_carry_the_turn() {
        let (addr, _state) = spawn_mock(MockState {
            events: vec![
                ev("mock-run-1", 0, "text_chunk", "{\"text\":\"partial\"}"),
                ev("mock-run-1", 1, "error", "{\"error\":\"HTTP 429 too many requests\"}"),
            ],
            ..Default::default()
        })
        .await;
        let started = std::time::Instant::now();
        let outcome = crate::agent_client::AgentClient::connect(&addr).await;
        let elapsed = started.elapsed();
        match outcome {
            Ok(_) => panic!(
                "the paused clock carried the connect in {elapsed:?}: §11.13(1)'s \
                 rejection of the paused-clock route is falsified"
            ),
            Err(error) => println!(
                "rev-s6 probe paused-clock: connect failed after {elapsed:?} with {error:#}"
            ),
        }
    }

    /// REV-S6 probe: the *whole* drive — the same call the positive test makes —
    /// under the paused clock.  Records what it observed in a file, because the
    /// harness capture hides `println!`.
    #[tokio::test(start_paused = true)]
    async fn rev_probe_paused_drive_outcome() {
        let drive = drive_error_turn("{\"error\":\"HTTP 429 too many requests\"}").await;
        let goal = drive.store.replay("g").unwrap().unwrap();
        let report = format!(
            "prompts={} completed={:?} failure_kind={:?} history={} events={}\n",
            drive.prompts, drive.completed, drive.failure_kind,
            goal.history.len(), drive.store.events("g").unwrap().len()
        );
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true).append(true)
                .open("/tmp/rev-goal-s6-paused-drive.txt").unwrap();
            file.write_all(report.as_bytes()).unwrap();
        }
        assert!(
            drive.prompts == 1,
            "under a paused clock the drive must either die at connect or reach the \
             model; observed {report}"
        );
    }
```

The sabotages (all three are `python3 <script> <mode>` then re-run; each prints the sha256 of the
file it wrote):

```python
# sabotage-console.py — anchors (whichever is present is replaced by the requested one)
SHIPPED = '''        if record.terminal_state == "error"
            && crate::executor::is_rate_limit_error(record.error.as_deref())
        {
'''
MODE_A  = '''        if record.terminal_state == "error"
        {
'''
MODE_B  = '''        if record.terminal_state == "error" && false
        {
'''

# sabotage-server.py — macOS launcher
SHIPPED = '''#[cfg(target_os = "macos")]
fn open_in_browser(url: &str) {
    let _ = std::process::Command::new("open").arg(url).spawn();
}
'''
OFF     = '''#[cfg(target_os = "macos")]
fn open_in_browser(url: &str) {
    let _ = url;
}
'''

# sabotage-supervision.py — the `future`-only group verb
SHIPPED = '''    let mut cmd = std::process::Command::new(&exe);
    if stem == "future" {
        cmd.arg("goal");
    }
'''
OFF     = '''    let mut cmd = std::process::Command::new(&exe);
    if stem == "future" {
        // REV-S6 sabotage: the group verb is not prepended.
    }
'''
```

(the supervision sabotage was applied with the editor rather than this script — the script's
restore-then-apply order was written after that run; the bytes under measurement are the sha256 in
the table, which is what matters)

Reports produced (all in `/tmp`, read with `daread.py` / `annotate.py`):

| run | lcov | outcome |
|---|---|---|
| control-1 baseline | `/tmp/rev-goal-s6-base-lib.info` | 2 passed, 5.03 s |
| control-1 sabotage A | `/tmp/rev-goal-s6-p1a-conjunct-deleted.info` | 1 passed / 1 **failed** (control), 5.01 s |
| control-1 sabotage B | `/tmp/rev-goal-s6-p1b-guard-off.info` | 1 **failed** (positive) / 1 passed, 0.02 s |
| control-2 baseline | `/tmp/rev-goal-s6-p2-base.info` | 2 passed, 0.38 s |
| control-2 probe | `/tmp/rev-goal-s6-p2-probe.info` | 1 passed, 0.38 s |
| control-2 launcher off | `/tmp/rev-goal-s6-p2b-launcher-off.info` | 1 **failed** (positive) / 1 passed, 10.01 s |
| control-3 baseline | `/tmp/rev-goal-s6-p3-base.info` | 1 passed, 7.35 s |
| control-3 goal-verb off | `/tmp/rev-goal-s6-p3b-goal-arg-off.info` | 1 **failed** |
| paused probe 1 | `/tmp/rev-goal-s6-p4-paused.info`, `…-p4b-…` | 1 passed, 0.01 s (connect died both times) |
| paused probe 2 | `/tmp/rev-goal-s6-p5-paused-drive.info` ×20 profraws → `/tmp/rev-goal-s6-p5-paused20.info` | 20 runs: `prompts=0`; `DA 5101=0` |

## 8. What I could not check, and the uncertainties I am leaving open

* **The real `/usr/bin/open` was never executed** (by choice, §3.3). The claim it would falsify —
  "the record can be written by the real opener" — is argued mechanically plus by the §3.1
  substitution, not by running a browser.
* **The paused-clock connect failure is load-sensitive**, and my two probe batches ran on a box with
  load 11–16 from eleven peers. The load-independent result is the one the rejection needs
  (`DA 5101=0` in 20/20 paused drives, and `prompts=0` in 20/20); the *connect-died* observation is
  the part I would not quote as deterministic.
* **I did not re-run the record's full six-crate command.** The numbers I quote are from the pinned
  `/tmp/r2c-lcov.info` (re-read per line) and from my filtered runs, which is the point (they isolate
  the fixture); the record's suite totals are its own.
* **The Windows rows are read, not measured** (§5). No Windows host was used; the round-1 categories
  are quoted from `module-goal.md:615`.
* `cargo llvm-cov` aborts before its report step when a test fails — every failing-run `DA` above
  therefore comes from the `.profraw` the run left behind via `lcov-from-profraw.sh`, not from a
  `cargo llvm-cov` report. That is a tooling fact a re-runner will hit immediately; it is why the
  script exists and is hashed.

## 9. Handoff

* **Artifacts**: this file (new, the review's only write); scratch tree `/tmp/rev-goal-s6` with the
  probes, sabotages and readout tools above; reports and `*.profraw` under `/tmp/rev-goal-s6-*.info`
  / `/tmp/rev-goal-s6-runprof`; the instrumented target dir `/tmp/rev-goal-s6-target` was disposed at
  the end of the review (PLAN §2b — it held 1.0 GB, and the commands in §2/§7 rebuild it in ~35 s).
* **New evidence vs. the segment's own handoff**: (1) the §11.13(1) paused-clock experiment was
  re-run and its *conclusion* confirmed with a `DA` that the segment did not have — the guard is
  never evaluated under the paused clock (`5101=0`, 20 runs) — while its stated mechanism
  ("`connect` dies in 0.01 s") was shown to be one of two shapes and load-dependent; (2) the
  instructed control-1 edit was shown to produce the *inverse* outcome pair, and the edit that
  produces the instructed pair was run (guard neutralised → positive fails at `11974`, `DA` 0 across
  the cool-down); (3) the browser fixture was attacked from a direction the record did not test (an
  opener directory that is not the recorder: URL delivered, record absent), and the fact that `DA`
  for `102-104` cannot see a neutered launcher was measured; (4) control 3 reproduced exactly, with
  the leak check done by copy path rather than by the `supervisor watch` substring — the substring
  alone false-positives on the campaign's own watchers; (5) the "only intentional wall-clock wait"
  sentence is falsified by a census of 23 sites in 15 files; (6) the record's ledger passes the
  **fixed** `reconcile.py` with 0 MISSING / 0 STALE over 82 lines.
* **Rejected approaches**: (a) executing the literal `/usr/bin/open` check (opens the operator's
  browser — see §3.3); (b) attacking the browser row by *editing production code inside the reviewed
  fixture* (the sabotage is a scratch-tree edit, reverted, and the fixture was never modified);
  (c) a first attempt to classify the paused-clock result as deterministic — the 5/5-success batch
  and the 9/9-failure batch from the same path are what forced the load-controlled re-measurement;
  (d) `ps`-based leak detection by the string `supervisor watch` — it matched the campaign's two own
  watchers.
* **Uncertainties**: the load dependence above; whether the Windows `rundll32` arm should stay
  waived (a policy question for the supervisor, §5); and the exact reason `connect` fails in some runs
  and not others (I measured the correlation with load, not the mechanism inside tonic's readiness
  check).
* **Next useful checks, in order**:
  1. **Rework the three sentences** the review corrected (owner: supervisor's rework todo, since the
     reviewer may not edit the record): §11.13(1)'s "`connect` dies in 0.01 s" → "the readiness check
     of the first gRPC call dies; `connect` itself carried 20/20 in a later batch; the guard is
     evaluated 0 times under the paused clock (20 runs, `DA 5101=0`)"; the "only test in the crate
     that waits on purpose" sentence → the measured 5.03 s and the census; and §11.13's "Next useful
     checks" item 3 (the conjunct deletion) → the two edits and their actual outcome pair.
  2. Re-run the paused-clock probes on a **quiet** box, to confirm `DA 5101=0` there too (the clean,
     load-independent half of §2.4).
  3. Resolve the Windows parity item for `webui/server.rs`'s opener arm (§5) — re-label the round-1
     row or request the reading; the PE-recorder question goes to PLAN 4b.
  4. Not checked by anyone yet: whether the two other `attribution-artifact` siblings of
     `console.rs:10651`/`10566` still hold under the fixed STALE loop — `reconcile.py` sees them as
     five-digit citations the MISSING half cannot represent (`module-goal.md:1205`).
