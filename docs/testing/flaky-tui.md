# flaky-tui.md — the `tui` unit's flake record

Scope: `flake`-class defects that are **measurement-level** in the `future-tui`
test suite: they do not make an assertion red, they make the *number* unreachable
(the binary aborts, cargo-llvm-cov writes no lcov, or the run hangs and the
timeout kills it). Written per `FLAKES.md` §3 — every claim here has an
invocation, a run count and the mechanism, and everything I could not reproduce
says so.

Host: macOS 26.6.2 (25G83), arm64 (Mac15,14), load average 9–20 during these
measurements, other campaign workers active.

## 1. Item 18 — the fd-1 trap, the part that was **not** fixed

`FLAKES.md` item 18 records this class as "**fixed by the tui unit**", with
before-rates of 5/8 (instrumented) and 2/7 (plain) cited in
`docs/testing/module-tui.md` §9.7(e). **That status is wrong** and this section
is the correction: the class reproduces after those fixes, at the rate below.

### 1.1 Invocation and rate (same command before and after)

```
.future/cov100/flake-rate.sh --label tui-fd1 --runs 8 --timeout 300 -- \
    timeout -k 5 280 cargo test -p future-tui --lib -j 4
```

`timeout -k 5` is added around cargo on purpose: **this suite catches SIGTERM**
(it installs its own handler for `SIGINT`/`SIGTERM`/`SIGHUP`/`SIGQUIT`), so a
plain `timeout` does not end a hung test binary — measured: a run that stalled at
`timeout 240` was still alive 26 minutes later, and only `SIGKILL` ended it. Run
counts are therefore equally comparable before and after (`-k 5` = `SIGKILL`
5 s after the `SIGTERM`).

| | rate (same command, same `-j 4`) |
|---|---|
| **before** (tree = `d9049279`, the test as the unit left it) | **1 of 8** runs — `rc=101`, verbatim summary below |
| **after** (this unit's fix, §1.4 — same command, same 8 runs) | **1 of 8** runs — verbatim summary below |

**So the rate did not move: this unit does *not* claim the flake is fixed.** What
it does claim is in §1.3/§1.4: the kill route the probes prove (a write that is
asleep inside the pipe is broken by the `dup2` that hands fd 1 back) is real, the
fix removes it, and yet the suite rate is unchanged — i.e. either that route is
not the dominant one in a full run, or a second, so far unidentified route
produces the same `BrokenPipe`. Both readings are consistent with everything
measured, and neither is proved; §1.5 lists what that leaves open.

Earlier measurements of the same class: 2 of 7 plain full-suite runs and 5 of 8
instrumented attempts (`docs/testing/module-tui.md` §9.7(e)), and 2 of 4 in this
unit's own direct-binary witness loop (1 death, 1 stall) — the class is stable at
roughly 1 in 4 to 1 in 8 runs depending on load.

### 1.2 What the tool printed (verbatim)

**Before** (`--label tui-fd1`, tree `d9049279`):

```
tui-fd1 run   1/8  pass
tui-fd1 run   2/8  pass
tui-fd1 run   3/8  pass
tui-fd1 run   4/8  pass
tui-fd1 run   5/8  pass
tui-fd1 run   6/8  FAIL(rc=101)
tui-fd1 run   7/8  pass
tui-fd1 run   8/8  pass

=== flake-rate [tui-fd1] ===
runs=8 failures=1 timeouts=0
logs: /var/folders/hy/h2vs6ytn5lb97xcx8ykzcxhh0000gn/T//flake-tui-fd1
```

**After** (`--label tui-fd1-after`, this unit's fix):

```
tui-fd1-after run   1/8  FAIL(rc=101)
tui-fd1-after run   2/8  pass
tui-fd1-after run   3/8  pass
tui-fd1-after run   4/8  pass
tui-fd1-after run   5/8  pass
tui-fd1-after run   6/8  pass
tui-fd1-after run   7/8  pass
tui-fd1-after run   8/8  pass

=== flake-rate [tui-fd1-after] ===
runs=8 failures=1 timeouts=0
logs: /var/folders/hy/h2vs6ytn5lb97xcx8ykzcxhh0000gn/T//flake-tui-fd1-after
```

The failures are **not** failing *tests*: both runs ended with

```
error: io error when listing tests: Os { code: 32, kind: BrokenPipe, message: "Broken pipe" }
error: test failed, to rerun pass `-p future-tui --lib`
```

i.e. libtest's **own** stdout write failed and libtest aborted the whole binary
(`process::exit(101)`), which is why `cargo llvm-cov` writes no lcov at all for
that target: **the run is lost, the number does not exist**. `flake-rate.sh`
has no category for "the harness died", so it counts these as `failures` with no
named test and prints no "distinct failing tests" block — that is the signature
to look for.

### 1.3 The mechanism, named

Everything below is measured on this host, not inferred.

**Which fd.** fd **1** — libtest's stdout fd, which is *process-global*.
`write_stdout_retries_on_eintr` points it at a pipe's write end for ~0.5 s:

* external witness (`libproc`, one `PROC_PIDLISTFDS` sample every ~150 µs, no
  write into the target), green run:
  `fd1=VNODE(/…/run-N.log)` → **`fd1=PIPE`** at 66.746 s → `VNODE` again 509 ms
  later → a PTY slave for 12 ms (the two size tests) → gone.
* in-process witness (poll `POLLOUT` + `fstat` on fd 1, 200 µs, no write),
  green run: fd 1 is a **FIFO whose POLLOUT is clear for the whole 507 ms** —
  i.e. the pipe is *full*, so every other thread's `write(1, …)` **blocks inside
  the pipe**; then `kind=0o10000 → 0o100000` (restored to the log file) at
  507.7 ms; the read end (fd 5) and the write end (fd 7) stay open until the
  teardown.

**Which other writers.** Two, and both are visible in the failing logs:

1. **libtest's own progress writer.** In a failing run the log is missing the
   `test … ok` lines for *exactly* this window (2 021 lines vs 2 274 in the
   neighbouring green run), and the last line is a truncated
   `test rpc::…watchdog_fires_on_dataless_stream ... ` with no result: those
   bytes were written into the pipe and discarded by the test's drainer. That is
   the direct evidence that the harness writes to fd 1 while the test owns it.
2. **`write_stdout_writes_all_bytes`** — an *unlocked* test (`tui/src/terminal_posix.rs:725`)
   that calls the production `write_stdout`, i.e. writes to fd 1, with no
   `test_env::lock()`. It is a second, independent victim of the same hijack:
   with a DYLD interposer attached (see below) this test went **red** in a run
   where another fd-1 test owned fd 1 — `write_stdout` returned `Err` and the
   test's `.unwrap()` panicked (`test terminal::terminal_posix::tests::write_stdout_writes_all_bytes ... FAILED`,
   `2192 passed; 1 failed`). So there are two symptoms of one defect: a red
   test when a *test* is the writer, a lost run when *libtest* is.

**Which drainer.** The test's own drainer thread (`tui/src/terminal_posix.rs`,
`write_stdout_retries_on_eintr`): it sleeps 500 ms (that is what makes the
payload write block and be interrupted), drains, and stops on the
`drained_enough` flag — which is set **immediately after the payload write
returns**, i.e. while the pipe may still hold bytes that libtest queued.

**The ordering, measured.** install (t) → pipe **full** → other threads' writes
to fd 1 queue inside it → drainer wakes (t+500 ms) → payload returns (t+507 ms)
→ fd 1 restored (t+507.7 ms) → **teardown closes the read end (t+1217 ms)**.

**What goes wrong — measured, and *not* what §9.7(e) guessed.** The failing
write is not aimed at a broken pipe: fd 1 still points at the pipe, the pipe
still has a reader, and libtest dies of `EPIPE` anyway. The kernel behaviour
under it, measured with small C probes:

* a write to a pipe whose read end is gone returns **`EPIPE` (32)** — Rust
  ignores `SIGPIPE`, so it is an `Err`, not a signal death — and
  `poll(POLLOUT)` on that write end reports **`POLLHUP`**, *not* `POLLERR`;
* **a write that is already asleep inside a full pipe is *not* woken by the read
  end closing** (probe: still blocked 30 s later). That is the **hang** face of
  this family: the writer never resumes, the suite stalls, `timeout` cannot kill
  it (§1.1) and the run is lost just as completely;
* **and the decisive one: such a sleeping write is broken by `dup2` itself.**
  Probe `/tmp/restore-breaks-queued-writer.c` (fill a pipe → make its write end
  blocking → `dup2` it onto fd 1 → let a second thread write to fd 1, which
  sleeps inside the pipe → `dup2` the original stdout back): the sleeping write
  is woken by the restore and **fails — `SIGPIPE` in the probe (rc 141), i.e.
  `EPIPE` in Rust — although the pipe's read end is open the whole time.**

That is the whole failure, and it is self-inflicted by the test's own teardown
step: libtest's progress line, queued in the pipe while fd 1 was the full pipe,
is woken and fails at the moment `dup2(saved, 1)` hands fd 1 back. libtest then
aborts the binary (`io error when listing tests: Os { code: 32, kind: BrokenPipe }`)
and the run, and its lcov, are gone.

The instruments agree with this and rule out the earlier "the read end was
closed" story:

* the DYLD interposer trace of a **failing** run ends with
  `dup2(5[FIFO …] -> 1[was REG])` (the install), then
  `dup2(6[REG …] -> 1[was FIFO …])` (the restore, 509 ms later), **and then
  nothing** — no `close()` of the pipe's read end, no foreign close of it, and
  the process is gone ~1 ms later;
* the external witness of the same run shows `fd1type=6` (a pipe) with the read
  end **and** the drainer's parked reference present, then `fd1type=1` (the log
  file) — i.e. fd 1 was a *valid, readable* pipe right up to the restore;
* two earlier explanations are therefore retired: this is **not** "the harness
  wrote to a readerless pipe" (it did not — the reader was there) and **not** "a
  stale close of the read end" (no close happened). §9.7(e)'s watcher saw nothing
  because there is nothing to see on fd 1: the victim is a *queued write*, which
  is exactly the note that section ends on.

One genuine stale-number hazard does exist in the same file and is recorded for
whoever takes the sweep, without being the cause here:
`PtyStdin::close_master()` (`terminal.rs:860`) closes `master`, and
`PtyStdin::drop` (`terminal.rs:874`) closes the same number again — and the
interposer shows fd numbers being recycled across tests at 20 µs granularity
(`dup(0) -> 6` right after the fd-1 test closed 6).

### 1.4 The fix (mechanism change, not a retry)

Both parts are in `tui/src/terminal_posix.rs::write_stdout_retries_on_eintr`.

1. **fd 1 is handed back only once the pipe has been drained empty.** The
   drainer publishes a counter every time it looks and finds the pipe empty
   (`read` → `EAGAIN`); the main thread waits for a *fresh* count before its
   `dup2(saved, 1)`. A write that does not sleep inside the pipe is unaffected by
   replacing the descriptor (it copies its bytes and returns), and with the pipe
   empty and being drained continuously a line-sized write cannot sleep in it —
   so the restore can no longer break one. The wait terminates by construction:
   `write_stdout` can only return once the drainer has drained the payload,
   after which the drainer necessarily reads `EAGAIN`.
2. **The reader is parked out of the low-fd recycling path, and the drainer runs
   until EOF.** The drainer reads through a second reference taken with
   `fcntl(read_fd, F_DUPFD, 100)`, and it never stops on a flag: a write end held
   by a blocked writer keeps EOF from being reported, so EOF means "no writer
   exists", and only then are the read end and its parked reference closed. This
   removes the class's *other* face as well: no writer can be left orphaned in
   the pipe, and a stale `close()` of a recycled low fd number can no longer make
   the pipe readerless (`F_DUPFD` is also a precondition assert, so a missing
   slot fails loudly instead of silently losing the protection).

The first part is the fix for the route the probes prove; the second is what stops
the *same* test from arriving at the failure through a neighbouring route. **The
measured outcome, stated plainly: 1/8 before, 1/8 after — the fix does not change
the suite rate, so the flake stays open.** What the two probes do establish is
that the "sleeping write broken by the restore" route is real and that the drain
removes it for a stand-alone reproduction of the sequence; whether a full-suite
run reaches the failure through that route or through another one is exactly what
the unchanged rate leaves undecided.

What is deliberately unchanged: the 500 ms delay (it is what creates the blocked
write the test exists for — that blocked write, and its `EINTR` retry, are still
covered), the assertion (`result.unwrap() == 200_000`), the
`posix_test_lock()`, and the pipe's own `filled`-to-capacity setup. No retry, no
`--skip`, no `#[ignore]`, no serialisation of the suite, no weakened assertion,
and no `sleep` added to paper over a window.

One intermediate revision of this fix is worth recording as a rejected approach:
a version that waited on a counter of "the drainer saw the pipe empty" hung the
whole suite (`flake-rate` run 1 `TIMEOUT`, `write_stdout_retries_on_eintr` never
printed) because the drainer's poll is *blocking*, so its `read` never returned
`EAGAIN` and the counter never advanced. The published fix drains from the main
thread instead, which cannot deadlock on the drainer's progress. And an earlier
revision that had *only* the reader protection (`F_DUPFD` + drain-to-EOF, no
drain-before-restore) measured **2 of 8** — no better than the original, which is
the measurement that sent me looking for the real route instead of the reader
story §9.7(e) had assumed.

### 1.5 Residual, and what I could not do

* **The flake is still open** (1/8 both before and after). What is ruled out for
  the failing runs, with the instrument that ruled it out:
  * "the harness wrote to a *readerless* pipe" — the external witness shows fd 1
    a valid pipe (read end *and* the drainer's parked reference present) until the
    restore, then the log file, and the DYLD trace shows **no** `close()` of the
    read end anywhere in the window;
  * "the read end was closed by a stale `close()` of a recycled fd number" — the
    trace shows no foreign close during the window (it does show fd numbers being
    recycled *between* tests, 20 µs apart, which is why that story was tempting);
  * "the test's own teardown closed the read end too early" — the death lands
    ~1 ms after `dup2(saved, 1)`, not at the teardown.
* What the trace *does* show, in both a green and a failing run, is the same
  install → 509 ms → restore sequence with the harness's lines for exactly that
  window missing from the log; and the probe pair shows a sleeping write on fd 1
  being broken by that restore (SIGPIPE/EPIPE) and completing (`n=80 errno=0`)
  when the pipe is drained first. The fix implements the latter. Since the rate
  did not move, the next useful check is to attach the same two instruments to
  the *fixed* tree and confirm whether a failing run still dies *at* the restore;
  if it does, the drain's condition is not being met in a full run and the
  question is what refills the pipe between the drain and the `dup2` (the one
  writer that can move 64 KiB at once is the payload itself, which has already
  returned — so this is the narrow spot to look at).
* Two other fd-1 routes exist in the same file and are *not* touched here:
  `write_stdout_reports_errors` puts a **directory** on fd 1 for ~31 µs
  (interposer-measured), which fails a writer with `EISDIR` (code 21 — a
  different error than the one observed), and the two PTY tests put a **slave**
  on fd 1, which fails a writer with `EIO` (code 5, measured). Neither matches the
  observed `BrokenPipe`, but both are the same class and would each abort a run
  the same way libtest does here.
* `write_stdout_writes_all_bytes` is unlocked and writes to the process-global
  fd 1, so it is a *victim* of any fd-1 hijack — observed red under the
  interposer (`write_stdout` returned `Err`, `.unwrap()` panicked). It is a
  candidate for the same one-line lock the other fd-1 tests hold; I did not
  change it because nothing in the before/after rates needed it.
* `PtyStdin::close_master()` (`terminal.rs:860`) plus `PtyStdin::drop`
  (`terminal.rs:874`) close the same fd number twice, i.e. the stale-close
  factory the sweep should remove; it is *not* the cause of this flake (its test
  holds the same lock as the fd-1 test).
* libtest's own lines for the 507 ms window are still *discarded* by the drainer
  (they are written into the pipe). Cosmetic — the run no longer dies *from
  that* — but a reader of the log still sees a gap; forwarding them to `saved`
  would close it.

## 2. How to reproduce, and how to instrument

### 2.1 Reproduce

```bash
cd /Users/geilige/future-os/.worktrees/<wt>
timeout -k 5 280 cargo test -p future-tui --lib -j 4        # ~1 in 4 dies
```
The signature is the two `error:` lines of §1.2 and **no lcov** under
`cargo llvm-cov`; the `cargo test` exit status is 101.

### 2.2 Watch fd 1 without perturbing it

Two witnesses are in `/tmp` on this host and both are read-only w.r.t. the
process under test (`fd1witness.c` — `libproc` fd-table sampling from outside;
`fd1witness2.c` — same plus the set of `PIPE`/`SOCKET` fds, 30 µs sampling).
Two traps: (a) an in-process watcher must not *write* a probe byte into fd 1 — on
a full blocking pipe that write blocks the watcher itself; (b) `poll` on a pipe
write end reports `POLLHUP`, never `POLLERR`, on macOS.

### 2.3 Name the fd's owner: DYLD interposer

`/tmp/fdspy.c` interposes `dup`/`dup2`/`close`, logging timestamp, thread id,
the `fstat` kind+inode of the fd and fd 1's identity at that instant. Two rules
that cost time to learn: the original functions must be reached through the raw
trap (`syscall(SYS_close, …)`, `SYS_dup2`) because **dyld applies interposing to
`dlsym` lookups too** (a `dlsym(RTLD_NEXT, "close")` build recursed 58 k frames
into libSystem's initialiser), and `SYS_pipe` is *not* `pipe(int[2])` on this OS
(returns 4, leaves the array alone) — do not interpose `pipe`.

```
DYLD_INSERT_LIBRARIES=/tmp/fdspy.dylib FDSPY_OUT=/tmp/spy.log \
  FDSPY_ALL_CLOSES=1 <test-binary> --test-threads=4
```

## 3. Item 2nd — `app::tests::wait_for_agent_retries_until_agent_appears`

`tui/src/app.rs:15598`. Observed red once in 8 **instrumented** full-suite
attempts (`/tmp/tui-cov-crateB-1.log`: `test app::tests::wait_for_agent_retries_until_agent_appears ... FAILED`,
`2192 passed; 1 failed`, suite `finished in 24.38s`); the panic text itself is
not in that log, and the test passes alone.

**Mechanism (code-reading + the shape of the failure, not yet reproduced under a
controlled rate):** the test bounds real startup with a **wall-clock budget** —
`tokio::time::timeout(Duration::from_secs(10), app.start(…))` with
`.expect("retrying startup should complete after readiness recovers")` — around
a startup loop whose retry cadence is **1 s** and whose success depends on a
second tokio task being scheduled (the mock's `readiness_rejected` `Notify` →
`ready.store(true)`). Under a loaded host (load 16–20 with four test threads in
one process) the probe, the notify and the retry can each be delayed past the
budget, and the test then fails *by timeout*, which is a property of the host,
not of the code under test. It is a load-dependent wall-clock budget, i.e. the
same class as `FLAKES.md` items 6 and 8.

**Rates:** 1/8 instrumented (pre-unit, `/tmp/tui-cov-crateB-1.log`); **0/16**
full-suite runs during this unit's own before/after measurements (§1.1) and 0 in
the extra witness-loop runs — so the effect is small and load-correlated.

**Not fixed here, deliberately:** I did not reproduce it under a controlled rate
(nor did any of my 16 full-suite runs hit it), and `FLAKES.md` §4 is explicit
that a flake that cannot be reproduced is recorded, not "fixed". Registered with
its mechanism and rate above. If someone takes it: the honest fix is to make the
budget a *condition* (wait for readiness/the retry state) rather than 10 s of
wall clock, and to state the budget's relation to the 1 s cadence.

## 4. The corrected `FLAKES.md` item 18 line (for the supervisor)

Because `.future/cov100/` is held by another writer, this is handed over rather
than applied:

> | 18 | **the fd-1 trap in the `tui` suite is only partly fixed** — it still kills runs | `tui/src/terminal_posix.rs` (`write_stdout_retries_on_eintr`) | fd 1 is process-global and this test points it at a **full** pipe for ~0.5 s, so libtest's own progress line (and the unlocked `write_stdout_writes_all_bytes`) queues inside that pipe; on macOS a write that is *asleep* in a full pipe is broken by the `dup2` that hands fd 1 back — `EPIPE`/`SIGPIPE` with the read end open (measured with two standalone probes) — and libtest then aborts the binary (`io error when listing tests: … BrokenPipe`: **no lcov**); a sleeping write orphaned by a read-end close instead **hangs** the suite, which `timeout` cannot end because the suite catches `SIGTERM`. **Measured before: 1/8** (`--runs 8`, same command; 2/7 plain and 5/8 instrumented earlier in the campaign). This unit's fix (drain the pipe to empty before the restore; park the reader at fd>=100; run the drainer to EOF) removes that route but the rate is **unchanged: 1/8 after** — the flake is **still open**, with the failing run's fd-1 state, the windows ruled out and the next check written down in `docs/testing/flaky-tui.md` §1.3–§1.5 | measurement-level flake (item 12 family), **not fixed** |

## 5. Item 18, second pass — the route measured on the *fixed* tree, and the fix that moves it

Section §1 left two readings open ("either the proven route is not the dominant
one, or a second route exists"). This section separates them, by instrumenting
the **fixed** tree with a `write` trace, and then removes the route it names.
A predecessor's §1–§4 are untouched; §1.5's third bullet (`write_stdout_writes_all_bytes`,
the unlocked fd-1 writer) and its `PtyStdin::close_master` note are answered in
§5.4 and §5.6.

### 5.1 Instrument: a `write` trace, not just the descriptor handover

`/tmp/fdspy2.c` is §2.3's DYLD interposer (`dup`/`dup2`/`close`, raw-trap originals,
same two rules: never reach the originals through `dlsym`, never interpose `pipe`)
extended with `write`: it logs `WRITE_ENTER fd=1 … n=<len> first="<40 bytes>"` and
`WRITE_EXIT fd=1 … ret=<n> errno=<e> dt=<seconds> t_enter=<seconds>`, so a failing
run names **which writer** fails and **when relative to `dup2`** — the two things
§1.5 said were still missing. Log path is `O_APPEND`-opened, because cargo and the
test binary are two processes injecting the same `FDSPY_OUT`.

```
cd <worktree>
DYLD_INSERT_LIBRARIES=/tmp/fdspy2.dylib FDSPY_OUT=/tmp/fd1diag/<batch>/spy-N.log \
  FDSPY_WRITES=1 FDSPY_ALL_CLOSES=1 \
  timeout -k 5 280 cargo test -p future-tui --lib -j 4
```

Two harnesses in `/tmp`: `fd1run.sh <label> <n>` (sequential, stops at the first
non-zero run) and `fd1par.sh <label> <n>` (n at once, own spy log and rc file per
run). 12 instrumented runs of the tree §1.4 left (P1: 4 sequential, P2: 8
parallel) plus 3 of the fixed tree (P3, §5.5). On the pre-fix tree the flake did
**not** fire instrumented in any of the 12 (0/12 — §1.3's 5/8 was measured under
`cargo llvm-cov`, i.e. against a much slower suite). What they show instead is the
race itself, present in **12 of 12** runs.

### 5.2 The victim, and the ~100 µs margin (measured on a green run)

```
19.616728 tid=8336368000  fd1=[REG]                  WRITE_ENTER fd=1 n=71 first="test terminal::tests::terminal_default_s"
19.616733 tid=12942831616 fd1=[FIFO ino=1040184262265991747] dup2(8[FIFO ino=1040184262265991747] -> 1[was REG]) -> 1
…   (the test's payload write: ret=-1 errno=4 (EINTR) every ~13 ms for ~500 ms)
20.123246 tid=8336368000  fd1=[FIFO ino=1040184262265991747] WRITE_EXIT fd=1 ret=71 errno=0 dt=0.506517 t_enter=19.616725
20.123259 tid=8336368000  fd1=[FIFO ino=1040184262265991747] WRITE_ENTER fd=1 n=2 first="ok"
20.123353 tid=12942831616 fd1=[REG]                  dup2(10[REG] -> 1[was FIFO ino=1040184262265991747]) -> 1
```

* The queued writer is always **`tid=8336368000`, libtest's console thread**, and
  always **one** write per run: a 70/71/80-byte `test … ` piece, or its 2-byte
  `ok`. Its `dt` is **0.500–0.510 s in every run** — asleep inside the pipe for the
  whole window, released by the drain.
* Its `WRITE_ENTER` line shows `fd1=[REG]` and the install follows **5 µs** later:
  the harness thread was already past Rust's stdout flush when fd 1 changed under
  it, and its `write` syscall resolved fd 1 **into the just-installed pipe**. So
  the victim is not "a write that started inside the window" but "**any write to
  fd 1 that the kernel resolves while fd 1 is the full pipe**" — which, for those
  ~0.5 s, is every write to fd 1 in the process.
* It returns **107 µs before** the restore (§1.4's drain released it). That margin
  *is* the flake: the drain makes the queued writer **runnable**, it does not
  order it before the `dup2`, and when the restore wins the write fails.

So §1.5's first reading is the wrong one and the second is right in a stronger
form: the proven route *is* the dominant one — it is simply not
"asleep at the restore", it is "**woken by the drain but not yet scheduled**".

### 5.3 Why draining first cannot fix it: the trigger is the *fd number*

`/tmp/probe-v2.c` — full pipe, a second thread sleeps in `write`, then
`dup2(saved, 1)`; `SIGPIPE` ignored (as Rust does), so the writer reports instead
of dying:

| the sleeping writer writes to | after `dup2(saved, 1)` |
|---|---|
| fd 1, the hijacked number | `n=-1 errno=32 (Broken pipe)` — **reproducible** |
| fd 5, the **same pipe's** write end, still open | `STILL BLOCKED` — no error |

So it is **not** "the pipe lost its last writer" (fd 5 keeps the write side alive
in *both* rows). A sleeping write is broken **exactly when `dup2` replaces the
descriptor it is writing to**. Two consequences, both measured here:

* keeping an extra reference to the pipe cannot help — which is also why §1.4's
  `F_DUPFD(100)`-parked reader did not move the rate (`2/8` in that revision);
* **no ordering of the drain and the restore is safe from userspace**: "the pipe
  is empty" is not "no writer is in flight". The drain's condition *is* met; what
  is unmet is the schedulability. §1.5's "what refills the pipe between the drain
  and the `dup2`" is therefore the wrong question — nothing refills it, the
  already-queued writer is simply not run yet.

### 5.4 The fix: keep writers out of fd 1 for the window

`tui/src/terminal_posix.rs::write_stdout_retries_on_eintr` now holds
`io::stdout().lock()` from before the pipe is filled to immediately after
`dup2(saved, 1)`. Rust's stdout lock is the **process-global** lock every
Rust-stdout writer takes per write (`write!`/`flush` on `io::stdout()`), so the
harness's console — the thread the trace names — now blocks in *userspace* instead
of queueing in the pipe: **a write that never enters the pipe cannot be broken by
the restore**, whatever the scheduling does. The production `write_stdout` writes
with raw `write(2)` on fd 1 and takes no such lock, so there is no self-deadlock,
and there is no acquisition hazard: `posix_test_lock()` is held first, so no other
fd-1 hijacker can have a full pipe installed when the lock is taken. The guard is
dropped immediately after the restore; the drain-before-restore from §1.4 is kept
(it is what lets an already-queued writer finish promptly).

§1.5's third bullet, answered: `write_stdout_writes_all_bytes` writes to the
**process-global fd 1** through the production `write_stdout` and now takes the
same `posix_test_lock()` as the other fd-1 tests. It matters *more* than §1.5
judged, not less: the drainer's drain-to-EOF keeps the window open until EOF (the
opposite of shortening it), and §1.3 already observed this test red under the
interposer when another fd-1 test owned fd 1.

### 5.5 Verification and the rate

**Route gone (per-run evidence, not a rate).** 3 instrumented runs on the fixed
tree (P3) contain **no fd-1 write from any thread other than the test's own
payload** inside the window: the `dt > 0.05 s` list is empty (baseline 12/12 had
exactly one entry, always `tid=8336368000`, `dt≈0.50 s`), and no `WRITE_EXIT`
anywhere in those logs returns `EPIPE`.

**Rate** — same command, same `-j 4`, sequential, same host, verbatim tool output
(logs under `${TMPDIR}/flake-tui-fd1-*`):

*before* (`--label tui-fd1-before3`, this tree as §1.4 left it; tool lines 1/8…8/8,
run 3 `FAIL(rc=101)`; `runs=8 failures=1 timeouts=0`):

```
test rpc::grpc_client::tests::wait_for_backoff_above_gives_up_at_a_bound_the_backoff_cannot_reach ... ok
test terminal::terminal_posix::tests::wait_eintr_maps_to_timeout ... ok
error: io error when listing tests: Os { code: 32, kind: BrokenPipe, message: "Broken pipe" }
error: test failed, to rerun pass `-p future-tui --lib`
```

(2030 log lines, against 2280 in the neighbouring green run — the same signature
as §1.2, and again the last complete line is a test whose `ok` never arrives.)

*after* (`--label tui-fd1-after3`, with §5.4): `runs=8 failures=0 timeouts=0`,
all eight `pass`.

| tree | runs | failures |
|---|---|---|
| before (§1.4's fix only) | 8 | **1** (`rc=101`, BrokenPipe, no lcov) |
| after (§5.4) | 8 | **0** |
| after, second batch on the final tree incl. §5.6 | 8 | **0** |

The two after-batches are 0/16 while the class's recorded history on this host is
1/8, 2/7, 2/4, 2/8 and 5/8 (here and in `module-tui.md` 9.7(e)). The honest
statement: the *route* is closed by construction and the rate is 0/16 after
1/8 before, but 16 runs cannot exclude a residual of ~1/8 on their own — the
evidence that this is the route is §5.2 (12/12 queued writes, every one of them
the harness's own fd-1 write) and §5.5's empty `dt > 0.05 s` list (0/3).

**One unrelated death, recorded so it is not mistaken for this flake.** In the
3-way *parallel* instrumented batch (P3) one run went red on
`app::tests::a_real_repository_creates_a_worktree_and_switches_to_it` with
`finished in 134.20s` (vs 21–24 s normally) and its log contains **no**
`io error when listing tests` — a load-induced real-git failure, assertion-level:
the run, its output and its lcov were all written. Different class (item 6/8
family), different symptom.

### 5.6 Also in scope, confirmed and fixed: `PtyStdin::close_master()`'s double close

§1.5 registered this as real but not the flake's cause. Both halves hold; the
first half is now measured. `reader_loop_full_cycle_on_pty` (`terminal.rs:1724`)
calls `pty.close_master()` and the `PtyStdin` is dropped at the end of the test,
whose `Drop` closed `self.master` **again** — interposer trace of that test alone:

```
0.073785 tid=6169866240 close(6)[CHR] -> 0 errno=0                 <- close_master(), after the reader reached EOF
0.383171 tid=6169866240 close(8)[CHR] -> 0 errno=0                 <- Drop: slave
0.383188 tid=6169866240 close(6)[CLOSED(EBADF)] -> -1 errno=9      <- Drop: the master, a second time
```

The second close is 309 ms after the first, in a process where other test threads
open and close fd numbers constantly (an interposer trace of the fd-1 window alone
shows single-digit numbers reused at 20 µs granularity, §1.3) — so it is a stale
close of whatever else holds that number, and it returned `EBADF` here only
because nothing had taken fd 6 yet. `master` is now an `AtomicI32` and both paths
close through one `close_master()` that swaps in `-1` first, so the number can be
closed at most once; the same instrumented run afterwards contains **zero**
`CLOSED(EBADF)` closes (was exactly one). It remains *not* the fd-1 flake's cause
(that test holds the terminal lock), exactly as §1.5 said.

### 5.7 Also in scope, **not** fixed: `app::tests::wait_for_agent_retries_until_agent_appears`

`tui/src/app.rs:15598`. **Rate:** 1 in 8 *instrumented* attempts in the campaign's
own log (`/tmp/tui-cov-crateB-1.log`: `… FAILED`, `2192 passed; 1 failed`, suite
`finished in 24.38s`); **0** in the 24 full-suite rate runs (8 before + 16 after,
§5.5), **0** in the 15 instrumented runs (12 on the pre-fix tree, 3 on the fixed
tree, §5.1/§5.5), **0** in the gate runs of this
section — 0/42 full-suite executions. It passes alone.

**Mechanism:** `tokio::time::timeout(Duration::from_secs(10), app.start(…))` bounds
a startup loop whose retry cadence is **1 s** and whose success depends on a
*second* tokio task being scheduled (the mock's `readiness_rejected` `Notify` →
`ready.store(true)`). Note the failing run was a *fast* one (`finished in
24.38s`), so "the whole host was 10× slow" is not an adequate explanation; the
detail section is absent from the log (no `panicked at` line survives, so the
panic text is unknown — the same gap §3 records).

**Not fixed, deliberately.** No reproduction was obtained (0/42), and `FLAKES.md`
§4 is explicit that an unreproducible flake is recorded, not "fixed". The only
change I can justify without one is diagnostics (report the retry count and
readiness state on expiry, so the next worker gets the panic text), and a longer
bound is explicitly out of bounds. Handed over with the above; the honest fix
remains "make the budget a condition" — for which the *condition* to wait on has to
be identified from a reproduction first.

### 5.8 What is still open after this pass

* **Raw `write(2)` writers that take no lock at all** — the TUI's own render path
  (`tui/src/terminal.rs:77` `write_str` → `write_stdout`) is called by other,
  *unlocked* tests, so a render inside the window can still be queued in the pipe
  and broken at the restore. Its failure mode is benign, which is why it is not a
  measurement-level route: that call site discards the error (`let _ = …`), so the
  bytes are lost silently and no test goes red. A reader of a run's log therefore
  still sees gaps (libtest's own lines are discarded by the drainer, §1.5).
* **The other two fd-1 hijacks in the same file are the same class**: a directory
  on fd 1 for 41 µs (`write_stdout_reports_errors`; the writer's error is `EBADF`
  here, measured, not the `EISDIR` §1.5 recorded) and a PTY **slave** on fd 1 for
  ~12 ms in the two size tests (`dup2(9[CHR] -> 1)` twice per run, measured). Both
  hold the posix lock, and no foreign write landed inside either window in any of
  the 15 instrumented runs — so neither has produced a lost run, but both are the
  same "replace the process-global fd 1 under the harness" hazard.
* **A child spawned during the window** inherits fd 1 = the pipe. It cannot break
  a run the way the harness does: it is a *different* descriptor slot in a
  *different* process, and §5.3's probe shows the breakage needs the `dup2` to
  replace the very fd the sleeper writes to (the fd-5 row). Its bytes are drained
  and discarded.
* A run costs 21–25 s idle, 85–135 s under the campaign's load spikes; the rate
  measurements in §5.5 are therefore sequential and paired
  (before/after on the same host within minutes), and `timeout -k 5` is still
  mandatory (§1.1: the suite catches `SIGTERM`).

## 6. The residual windows, the raw-writer path, and the app.rs budget (F-TUI-2's §5.8 items)

§5 closed the route that kills runs and named four residuals. This section closes
the two that were still *reachable* and quantifies the other two.

### 6.1 The raw-writer path: `write_stdout` now takes the process-global lock

`tui/src/terminal_posix.rs::write_stdout` acquires `io::stdout().lock()` for the
duration of its raw `write(2)` on fd 1. This is the TUI's render path
(`terminal::write_str`), which until now was serialized only by its own
per-`Terminal` `write_lock` — i.e. a render from a test that takes no lock of its
own was one more writer that the kernel could resolve into fd 1 while another
test had a **full pipe** installed there (§5.2). It is now in the same rendezvous
as the harness: the fd-1 tests hold that lock from before the install to after the
restore, the harness takes it per `println!`, and this path takes it per render, so
a write is either before or after the window — never inside it.

Three properties make it safe, checked rather than assumed:

* the lock is **reentrant by construction** — `std`'s `STDOUT` is a
  `ReentrantLock` (`library/std/src/io/stdio.rs:612/719/730` in the 1.96.0
  sysroot), so the one caller that writes to fd 1 while already holding it (the
  fd-1 test's own `write_stdout(payload)`) proceeds instead of self-blocking;
* dropping `StdoutLock` is **pure lock release** — there is no `impl Drop for
  StdoutLock` in `stdio.rs`, so no buffer flush lands inside the window;
* lock order is consistent everywhere (`write_lock` → stdout lock), and the only
  paths that hold the stdout lock are the fd-1 tests themselves, which take
  `posix_test_lock()`/`terminal_test_lock()` first, so there is no cycle.

Verified by the §5.1 interposer on the fixed tree, full suite, 3 parallel runs
(`/tmp/fd1diag/P4`): **inside the pipe window only the payload's own thread writes**
(36/38/39 writes, all of them its `EINTR` retries), no foreign writer at all — the
pre-fix baseline (§5.2) had exactly one foreign write inside the window in **12 of
12** runs. No `WRITE_EXIT` anywhere in those logs returns `EPIPE`.

### 6.2 The three remaining fd-1 windows

Instrumented window-by-window census (same 3 logs, lines between each install
`dup2(… -> 1[was X])` and its restore):

| window | duration | writers inside | foreign writer? |
|---|---|---|---|
| full pipe (`write_stdout_retries_on_eintr`) | ~500 ms | payload thread only (36–39 writes) | **no** (baseline 12/12 had one) |
| directory (`write_stdout_reports_errors`) | 41 µs | the test's own failing probe (`errno=9`) | no |
| PTY slave, `refresh_size_picks_up_pty_dimensions` | ~40 µs | none | no |
| PTY slave, `reader_resize_updates_cached_size` | 12–15.5 ms | 4–7 writes from the test thread, **+54 writes from the libtest console in 1 of 3 runs** | **yes, sometimes** |

So the first three are now closed and the fourth is the exception §6.3 deals with.

**What I changed there.** `write_stdout_reports_errors` (directory) and
`refresh_size_picks_up_pty_dimensions` hold the standard lock across their window,
so nothing else can be inside fd 1 while it points at the directory/PTY. Both PTY
tests also hand fd 1 back **before** the drainer stops (ordering), which is not
cosmetic — see §6.4.

### 6.3 Why the second PTY window cannot take that lock

`reader_resize_updates_cached_size` needs fd 1 pointing at the PTY slave for the
whole window (the initial `Terminal::new()` size, and again when the reader
refreshes it after `SIGWINCH`), and it needs the reader thread alive to observe
that `SIGWINCH`. That same reader writes to fd 1 by itself:

* its one-shot modifyOtherKeys fallback fires **150 ms** after `t.start()`
  (`terminal.rs:389`: `write_str(&write_lock, "\x1b[>4;2m")` when neither kitty nor
  mOK is active), and
* an incoming kitty response makes it write `\x1b[>7u` (`terminal.rs:769`).

A lock held by the test thread across the window would block that write and hang
`t.stop()`'s join of the reader — i.e. it would trade a rare absorbed-bytes window
for a rare **hang**, which the campaign ranks strictly worse (§1.1: a hang cannot
be ended by `SIGTERM`). The measured window is 12–15.5 ms, so the 150 ms fallback
did not actually fire in any of the 44 instrumented runs observed — the argument
is about what a *starved* run can reach, not about what has happened yet. Hence:
ordering, not a lock, and the residual stated as absorbed bytes (a lost line in
the run log, cosmetic, §1.5's note) rather than as a lost run.

### 6.4 The PTY windows were **hang**-able, not EPIPE-able (new probes)

§5's probes were all pipes. Two new probes answer the PTY case, and they change
the classification of both PTY windows:

* `/tmp/ptycap.c` — a PTY slave's output buffer is **1024 bytes** on this host
  (measured, two runs). The console's 54-write burst in §6.2 is ~3.2 KB, so a
  writer inside that window *is* blocked by the buffer at least transiently.
* `/tmp/probe-pty.c` — fill the slave's buffer, `dup2(slave, 1)`, let a thread
  block in `write(1, …)`, then restore fd 1:

  | shape | result |
  |---|---|
  | a drainer is reading the master (the two tests' shape) | **`n=80 errno=0`** — the sleeper completes, 3/3 runs |
  | no drainer, buffer still full (the old teardown shape) | the restore **never returns** — killed at 8/15 s, 3/3 runs |

  So a PTY write is *not* broken by the restore the way a pipe write is (§5.3):
  the PTY windows were never an `EPIPE` route. What they were is worse in one
  respect — **restoring fd 1 while the buffer is full and nothing is draining
  hangs the restore itself**. Both PTY tests' teardown used to stop the drainer
  and join it *before* `dup2(saved, 1)`, which is exactly that shape; they now
  restore first and let the drainer outlive the window. That is a real fix, not
  tidying.

### 6.5 The app.rs budget: the loop's own wall time exceeds the old bound

`app::tests::wait_for_agent_retries_until_agent_appears` (`tui/src/app.rs:15598`)
was §5.7's "0/42, not fixed". Running the single test in a loop (same host, same
code, `./target/debug/deps/future_tui-… --exact …`) gives a **heavy-tailed**
startup instead of a constant. Of **45 runs** (four loops: 8 with timing
instrumentation, 16 bare, 20 capturing status, 1 with the fix in the binary),
**36 finished in 1.48–1.51 s**, **7 took 3.49–6.50 s**, and **two took 16.50 s and
16.52 s** — i.e. two runs in 45 exceeded the old bound by ~65 %. The old assertion
wrapped `app.start` in a **10 s** wall-clock budget with
`.expect("retrying startup should complete after readiness recovers")`, so those
runs *are* the recorded failure: the loop was still making progress (it retries
every 1 s until the agent answers) and the bound, not the loop, would have decided
the test. Both of them passed with the backstop in place, as did the whole 20-run
status loop (20/20, tails 3.49/6.48/6.50 s).

The numbers have a mechanism. One attempt costs the client's connect budget
(`rpc/grpc_client.rs:46` `TRY_CONNECT_TIMEOUT_SEC = 3`) plus the loop's 1 s cadence
— 4 s — and the observed tail is ~4 attempt-quanta (16.5 s ≈ 4 × 4 s). A 10 s
budget therefore covers about **2.5** worst-case attempts, i.e. it was not a
deadlock backstop but a bound on how unlucky the host may be.

**The change**, in the test module only:

* the contract is the *condition* the body already asserts (the
  "retrying every 1s" line, the "Connected to agent" line, `is_running`);
* the bound is now a named backstop derived from the cadence —
  `STARTUP_BACKSTOP = 60 s`, fifteen worst-case attempts, which can only fire when
  the loop cannot settle at all — with the relation and the measured tail written
  next to it;
* expiry now reports the transcript **on stderr**, because a failing test's panic
  text is part of the harness's *stdout* section and fd 1 is process-global in
  this suite — and the one recorded failure's log contains **zero** `panicked`
  lines (`/tmp/tui-cov-crateB-1.log`: `… FAILED` at line 2029, the `failures:`
  list at 2276, no panic text anywhere), so the next occurrence must leave its
  evidence on fd 2;
* the new expiry path is covered by
  `startup_reports_the_transcript_when_the_retry_loop_cannot_settle` (a
  never-ready mock + a 300 ms backstop, `#[should_panic]`, deterministic), which
  keeps the campaign's own "no uncovered line" rule satisfied.

**The alternative I measured and did *not* take**, so the next worker does not have
to rediscover it: this same test with `#[tokio::test(start_paused = true)]` (no
`flavor`, i.e. the current-thread runtime; `test-util` is already a dev-dependency
and a paused clock is already used at `app.rs:25483` for the heartbeat loop)
**passes in 0.07 s** — virtual time makes the 1 s cadence free, so the whole
wall-clock tail disappears and the test stops being a host measurement at all. It
is a bigger change than it looks (the virtual clock also governs every other timer
in the test body, including the render/pump timers, and it removes the real-time
meaning of any backstop, so a genuine hang would be reported instantly rather than
as a stall) and it would need its own rate before replacing the backstop; the
20-run rate above is what justifies the change I did make.

**Not** a longer timeout dressed up: no attempt-chain rewrite was done (the retry
loop is unchanged), and the reproduction the same item asked for — a *failing*
run with its panic text — was not obtained as a *failure*: no failure in the 44
single-test runs, nor in any full-suite run of §5.5–§6.6 (32 rate-run suites, 18
instrumented, and the gate suites). What §6.5 has instead is a run whose startup was
*longer than the old bound* (16.50 s vs 10 s) and which the backstop let finish —
the mechanism, measured, one step short of the red test. The panic text itself
stays unobserved, which is why the diagnostic now goes to fd 2.

### 6.6 Rate, and the item this is not

Same command as every other rate here, final tree, sequential, `-j 4`,
`timeout -k 5 280`, host load 24–34 while it ran (other campaign workers):
`--label tui-fd1-r3 --runs 8` → `runs=8 failures=1 timeouts=0`, and **no log
contains `io error when listing tests`** — the fd-1 route did **not** fire in any
of the 8. The single failure is a different item: `a_real_repository_creates_a_
worktree_and_switches_to_it`, `2192 passed; 1 failed`, `finished in 136.98s`
against 21–30 s for the other seven runs — an assertion-level, load-correlated
real-git timeout (second sighting; the first was §5.5's parallel batch). Registered
for the sweep, **not** attributed to item 18.

Running total for the fd-1 class after §5–§6: **1/8** before the lock, **0/8**,
**0/8**, **0/3** (instrumented) and **0/8** after — 0/27 across the three after
batches, with the route itself proven closed in §6.1 rather than only uncounted.

### 6.7 What is still open

* The console's absorption route in `reader_resize_updates_cached_size` (§6.2/6.3):
  witnessed, cosmetic, and left with its mechanism named. The two candidate fixes,
  for whoever takes it: a **forwarding** drainer (re-emit what it drains to the
  saved fd 1 — heals the log gap, does nothing for the hang face, which the
  ordering already removed) or a redesigned window that arms the kitty protocol
  first and then takes the stdout lock (heals the absorption, but the reader's
  in-window write has to be made impossible first — see §6.3 — and
  `Terminal::start`'s kitty flag is stored *before* its `\x1b[>7u` write, so that
  ordering would have to change too).
* `app::tests::a_real_repository_creates_a_worktree_and_switches_to_it` under load
  (§6.6, §5.5): two sightings, assertion-level, not yet reproduced on demand.
* `write_stdout_writes_all_bytes` (unlocked, writes fd 1 through the production
  path) now holds `posix_test_lock()` **and** goes through the locked
  `write_stdout`; §5.8's concern about the drainer running to EOF is therefore
  closed twice over, but the pair is worth re-checking if the drainer is ever
  changed again.

### 6.8 Two reds that are *not* item 18, met while running this section's gates

The gate command is `cargo test -p future-tui --no-fail-fast -j 4`. On this host
(load 24–48, other campaign workers active) it came back **rc=0 on four of six
attempts and rc=101 on two** (`/tmp/gate1*.log`), and neither red is the fd-1 class
(no log contains `io error when listing tests`):

* `/tmp/gate1-*.log` first attempt — the **lib** target red once at
  `2193 passed; 1 failed; finished in 138.27s` under load ~48 (the 21 s shape
  inflated 6×). It is *not* an abort: the summary is complete (2193 + 1 = 2194),
  and a `BrokenPipe` death never prints a completed `test result:` line with
  counts (§1.2) — so the fd-1 route is excluded by the shape of the red. I did not
  capture which test failed, so it stays unnamed; the two *named* load-stall
  failures in this suite are `a_real_repository_creates_a_worktree_and_switches_to_it`
  (§5.5, §6.6).
* `/tmp/gate1-2.log` — `tests/agent_sidecar_readiness.rs` red, same command, load
  47, while the **lib** target in that same run passed (2194 passed, 21.51 s):

  ```
  test a_candidate_that_answers_readiness_is_returned_as_an_owned_child ... FAILED
  panicked at tui/tests/agent_sidecar_readiness.rs:235:23:
  a candidate that answers readiness must be accepted: no Future Agent was reachable
  and no sidecar could be started (…/deps/future: No such file or directory (os error 2);
  …/deps/future-agent: No such file or directory (os error 2); future-agent: No such file or directory (os error 2))
  test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; finished in 6.57s
  ```

  Mechanism (read from the fixture, not yet proved by instrumenting it): the
  candidate on `PATH` is a `future` stub that writes a sentinel and then
  `exec /bin/sleep 30`, and the fixture's server deliberately binds its socket only
  **after** that sentinel appears (`wait_for_path(&spawned, Duration::from_secs(6))`,
  `agent_sidecar_readiness.rs:196`). Measured here the target takes 6.5 s when it
  fails and passes when the host is idle (it passed in the run at load ~9 and in
  the §5/§6 rate runs), so this reads as *the same class as items 6/8*: a real-time
  budget in a fixture (the launch loop's probe budget against a 6 s
  sentinel wait), not a regression — nothing in this section touches `PATH`, the
  supervisor or the readiness probe, and the third candidate's `ENOENT` is the
  *unix socket* not existing yet, not a missing binary (`…/deps/future` really is
  absent, as the fixture asserts).

  Registered for the sweep rather than fixed here: it is outside this todo's
  remit (it is not the fd-1 trap and not the `app.rs` budget item), and a red
  integration target is *not* measurement-level in item 12's sense — the run's
  lcov for the lib target is written normally, only the run's exit status is lost.

## 7. `tui/tests/agent_sidecar_readiness.rs` — fixed, and the launch window it was really racing

§6.8 registered this as a new flake, 1 of 2 gate runs at load 47, reading it as
"the fixture's own timing budget vs a loaded host". The measurement below says the
losing party is **the launch loop's own probe window**, not the fixture's budget,
and the fix removes the test's dependence on it without changing what a user
waits for.

### 7.1 Rate, mechanism, fix, rate

**Rate.** Before: the gate red of §6.8 (`/tmp/gate1-2.log`, load 47, the lib target
passing in the same run) — the target red in **1 of 2** observations at that load.
In isolation it is rare: **0 of 20** instrumented runs at load ~80 (§6.8's own
loop, `--nocapture`), and every one of those 20 runs shows the same timings, which
is what made the mechanism worth measuring directly (below).

**Mechanism, measured under control.** The fixture could not bind before the child
was observable:

| stub's sentinel delayed by | fixture bound at | loop returned at | result |
|---|---|---|---|
| 0 ms | 385 ms | 471 ms | pass |
| 1 s | 1395 ms | 1446 ms | pass |
| 2 s | 2481 ms | 2579 ms | pass |
| 3 s | 3558 ms | 3627 ms | pass |
| 4 s | 4489 ms | 4660 ms | pass |
| **5 s** | never | **5109 ms** | **FAIL** |

and the failure is the recorded message, verbatim:

```
test a_candidate_that_answers_readiness_is_returned_as_an_owned_child ... FAILED
a candidate that answers readiness must be accepted: no Future Agent was reachable and no
sidecar could be started (…/deps/future: No such file or directory (os error 2);
…/deps/future-agent: No such file or directory (os error 2); future-agent: No such file or
directory (os error 2))
```

So the loop's per-candidate window is **5.0–5.1 s** (`40 × 125 ms`, measured, not
read off the constant), and the run fails when the child — a `/bin/sh` script,
i.e. a fork/exec plus one filesystem write, measured at 0.38–1.07 s in 20 runs —
takes longer than that. The 6 s fixture budget in the old code was a *symptom*:
the fixture had to be allowed to wait longer than the loop does, and the loop was
always the party that gave up. Under the load spike that produced §6.8's red the
fork/exec of the stand-in child took > 5 s, which is a 13x outlier but not an
impossible one — the host was at load 47 with several other test binaries.

The endpoint offers no way out of the gate: `select_agent` authorizes a local
launch only for a `Transport` error whose kind is `NotFound` or `ConnectionRefused`
(`packages/rpc/src/connection.rs:405-436`), so the fixture must be *silent* while
the loop decides and *answering* once the child has spawned — and the child is the
only observable in between (`check_instance_lock` creates no file, and the
loop's probes use the same endpoint as the decision does).

**Fix.** The window is now a parameter of the loop, with the product default
unchanged:

* `tui/src/agent_supervisor.rs`: `LAUNCH_PROBE_WINDOW = 5 s` and
  `LAUNCH_PROBE_INTERVAL = 125 ms` are named once, `ensure_agent_running` keeps its
  signature and behaviour, and the loop body moved to
  `ensure_agent_running_within(configured, window)` — the caller that has to
  fork/exec a stand-in candidate supplies its own window instead of asserting how
  fast the host schedules that child;
* `tui/tests/agent_sidecar_readiness.rs`: the fixture passes
  `LAUNCH_FIXTURE_WINDOW = 60 s` and the sentinel gate keeps its meaning, its bound
  (`SENTINEL_WAIT = 30 s`) now sitting *inside* the window it hands the loop. That
  restores §1.1's invariant — the guard is still alive when the loop returns, so
  "the sentinel exists" needs no wait — instead of the bounded proof-wait the
  first attempt at this fix needed.

**Verified end to end.** Same tree, same target, stub's sentinel delayed past the
old window: **6 s → 3 tests pass (13.3 s); 12 s → 3 tests pass (25.3 s)** — both
shapes were red before the fix (5 s was red, §7.1's sweep).

**Rate after.** `cargo test -p future-tui --test agent_sidecar_readiness` under
load (three whole-suite `cargo test -p future-tui --no-fail-fast -j 4` runs in the
background, then 14 iterations): **14/14 pass**, and all three background suites
passed too (`rc=0`). Verbatim: `=== readiness rate after the fix: 14/14 pass,
failures=0 ===`.

### 7.2 The give-up arm is now tested instead of stumbled into

The loop's "no candidate answered" arm used to be reachable only when a loaded
host happened to exceed the window (that is what §6.8's red actually *was*), which
made it both rare and, as the missing panic text shows, hard to read. It is now a
deterministic test: `a_candidate_that_never_answers_is_reported_after_the_probe_window`
drives a stub that spawns and stays alive but never serves anything, with a 250 ms
window, and asserts the reported message, the candidates it lists, that no
handshake happened and that the silent candidate never answered. **3 tests pass in
the target** (1.3 s), up from 2.

### 7.3 What is not fixed, and the uncertainty

* The test still forks a child; on a host that cannot schedule a fork/exec within
  **60 s** it fails again. That is 156x the measured 0.38–1.07 s, and — unlike the
  old shape — the failure it reports is now about the *fixture's* candidate
  (`the PATH stub must be the candidate that was launched`) rather than about the
  loop's window. A test that forks anything cannot be fully host-independent; what
  can be removed is the *arbitrary coupling* between the fixture's child and a
  product budget, and that is what §7.1 does.
* The production window is deliberately **not** widened: it is what a user waits
  when the sidecar genuinely cannot start, and there is no measurement in this
  campaign that a real `future agent` needs more than 5 s. If that is wrong, it is
  a product decision with its own evidence, not a fix for this flake — and note
  that widening it would not have made the test host-independent either, only
  moved the outlier.
* The first attempt at this fix (bind after `min(sentinel, 2 s)`, prove the
  sentinel afterwards) is a **rejected approach** worth recording, because it looks
  right and measurably fails: with the bind no longer gated on the child, the loop
  handshakes as soon as the endpoint answers, returns, and the guard drops — the
  candidate is killed *before* it can write its sentinel, so a late-but-legitimate
  child turns into a sentinel-assertion failure. Measured: stub delays of 3, 5, 8
  and 20 s all failed at the assertion, while the loop itself returned `Ok(true)`
  every time (loop windows were 2001–2019 ms). The ordering "sentinel first, then
  answer" is load-bearing, which is why the fix keeps it and moves the *window*
  instead.

## 8. Item 24 (routed to this scope by the supervisor): the live-agent real-git test

`app::tests::a_real_repository_creates_a_worktree_and_switches_to_it`
(`tui/src/app.rs:23124`) — the supervisor's FLAKES item 24, folded in here.

**What is already recorded.** Found by the supervisor while independently
rate-checking item 18: 1 failure in 4 runs of `timeout -k 5 280 cargo test -p
future-tui --lib -j 4` at load 34, with the same test passing 3/3 standalone at
load 6 (5.30 s, 5.23 s, 0.26 s — the spread is the tell). Its harness is
wall-clock bounded: `PUMP_BUDGET_ITERS 4_800 × PUMP_INTERVAL_MS 25 = 120 s`
(`app.rs:11942`), spent in `pump_until_msg`/`pump_until_workdir` waiting for real
TCP + HTTP/2 round trips against an in-process mock plus **real `git` subprocess
work** (`git init`/`commit`/`worktree add`/`rev-parse`).

**What this pass adds.**

* **Four more sightings, with timings** — 134.20 s (`/tmp/fd1diag/P3/run-1.log`),
  136.98 s (`${TMPDIR}/flake-tui-fd1-r3/tui-fd1-r3-3.log`), 138.27 s
  (`/tmp/gate1-1.log` · the lib target's own run, §6.8) and **153.18 s**
  (`/tmp/readiness/suite-1.log`, the largest yet) against a 21–30 s baseline for
  the same target. That is six sightings in one evening at load 34–48, so the item
  is corroborated, not a one-off.
* **A second, different flake in the same runs, for the sweep:**
  `/tmp/readiness/suite-2.log` (rc=101, `60.87 s`) failed
  `rpc::grpc_client::tests::subscribe_failure_modes` — not item 24, not the fd-1
  class (no `BrokenPipe`), and not registered anywhere yet. Same evidence gap: no
  panic section.
* **The evidence gap is now characterised, and it is worse than "the log ends at
  the summary".** In three of my logs the failing test has **no
  `---- <test> stdout ----` section at all** — not merely no `panicked` line: the
  result line and the `failures:` list are there, and the section that would carry
  the assertion is absent (`grep -n "stdout ----"` and `grep -n "panicked at"` both
  find nothing in `/tmp/readiness/suite-1.log`, `${TMPDIR}/flake-tui-fd1-r3/…-3.log`
  and `/tmp/fd1diag/P3/run-1.log`). **The section is not systematically missing**,
  and I refuted my own first explanation for it:
  * the crate installs a process-global panic hook (`crash::install`,
    `tui/src/crash.rs`) that **replaces** libtest's capturing hook, and under
    `cfg(test)` its body returns early unless the reporting is armed — so a panic
    that happens while that hook is installed is *swallowed*, not captured. Proven
    in a controlled pair in one binary (`/tmp/probe-…` runs): a failing test
    prints `---- … stdout ----` + `thread … panicked at …` normally, and the same
    test after `crash::install()` produces **no section and no message**;
  * **Corrected in §10: that swallow *is* the cause, and my "nothing in the lib
    target calls `index::run`" was simply wrong.** About twenty `index.rs` tests
    call the production entry point `run(...)`, which used to install the crash hook
    with a **bare, unscoped `set_hook`** — so in a lib test run the crash hook
    replaced libtest's capturing hook for the rest of the process. That is what
    removed the sections, and §10 fixes it (deterministic A/B: the same panic shows
    its message before any `run(...)` and loses it after one).
  Practical consequence for whoever takes this: **force the text onto the log**
  rather than relying on libtest's sections — `cargo test -p future-tui --lib -j 4
  -- --nocapture` puts the panic on stderr as it happens, so it cannot be dropped
  by a section that is never printed.
  * **New correlation, five occurrences:** every log in this campaign with a
    section-less failure is a *long* test run — 110.14 s, 134.20 s, 136.98 s,
    138.27 s, 153.18 s — while the controlled *instant* failure printed its section
    normally in the same command on the same host. That is the shape to explain,
    and it points at something in the run's *end game* rather than at the hook
    (which a normal run restores). It is registered, not fixed.
* **A reproduction attempt, and it falsifies the obvious recipe.** 14 concurrent
  copies of the single test with `--nocapture` (`/tmp/item24-repro.sh`, logs in
  `/tmp/item24/`): **14/14 pass**, and the slowest copy took **2.50 s** against the
  120 s budget (typical 0.66–0.68 s; alone, 0.20 s). So concurrency on *this* test
  is not the pressure that breaks it — the `66–77 s` figure in the record belongs
  to `spawn_paths_succeed_against_live_agent`, not to this one. Every sighting so
  far came from a **whole-suite** run at load 34–48, i.e. the pressure is 2 194
  tests on 4 threads *plus* external load, which is a 48x gap that one more copy
  will not close.
* **Next useful check, sharpened by the two points above.** Take a *whole-suite*
  run with the text forced onto the log rather than relying on libtest's sections:
  `cargo test -p future-tui --lib -j 4 -- --nocapture` under external load, and
  grep for the assertion. A single-target loop cannot reach it (§8's 14 copies), and
  the failing suite's own log does not carry the section.
* **Not fixed, and deliberately not guessed at.** The item names three candidate
  losses — the 120 s pump timeout, `path.is_dir()`, or `seen.len() == 1`. Two of
  the three are *not* wall-clock: they fail when the app never delivered the
  `set_cwd`, which under load could be a genuinely lost command rather than a slow
  one. Removing the wall-clock dependency without knowing which one fires would be
  guessing at the fix, and for this test a paused clock is not obviously available
  either: the wait covers real `git` subprocess time, which virtual time does not
  compress (unlike the *local* conditions §6.5's `start_paused` experiment
  addressed). The next useful check is the one the item states, with `--nocapture`
  and enough concurrency to make the round trips slow, and then the fix is
  whichever of the three it names.

## 9. Status of the other three items in this pass

Recorded so the next worker does not re-derive them.

### 9.1 §6.7 option 1 (a forwarding drainer) is not as clean as it reads

The `reader_resize_updates_cached_size` window (§6.2/§6.3) is the one window that
cannot hold the process-global stdout lock, and it absorbs a concurrent writer's
bytes — witnessed, cosmetic. §6.7 offered a forwarding drainer (re-emit drained
bytes to the saved fd 1) as the fix that heals the log gap. Working through it:
the drainer reads **one** byte stream off the PTY master, and that stream contains
*both* the foreign console lines *and* the test's own terminal control sequences
(`\x1b[?1049h`, `\x1b[?2004h`, `\x1b[?1004h`, `\x1b[?u`, the pop sequences — the
bytes `Terminal::start`/`stop` write, which this window exists to give a real tty
to). Forwarding forwards both, so it would heal the missing `test … ok` lines by
writing ~40 bytes of escape sequences into the run log for every one of them —
trading a silence for noise, in a log the module record reads. There is no
in-band way to tell the two apart (same fd, same buffered stream), so option 1 is
**not** the cheap fix §6.7 assumed. Option 2 (arm the kitty protocol before taking
the lock, then hold the lock) removes the reader's need to write inside the window
but needs `Terminal::start`'s ordering changed (the kitty flag is stored *before*
its `\x1b[>7u` write, `terminal.rs:764-769`) and the 150 ms modifyOtherKeys
fallback to be impossible inside the window — a change to production behaviour for
a cosmetic gain. **Recommendation: leave the absorption, with this reasoning
recorded**; the measurement-level harm (a lost run) was already removed by the
ordering fix in §6.4.

### 9.2 §6.5's paused clock: measured once, not adopted

`#[tokio::test(start_paused = true)]` makes the `app.rs` retry test pass in 0.07 s
(one observation). Not adopted, and not verified over 20+ runs: the virtual clock
also governs every other timer in that test body (`pump`, the render deadlines),
so the honest next step is the 20-run comparison against the 60 s backstop, not a
swap on one data point.

### 9.3 Not done in this pass: the `cargo llvm-cov` rate

The item-18 rate has not been re-measured under `cargo llvm-cov`, which is where
the class was first seen (the campaign's own failure came from an instrumented
run, and instrumented tails are longer). Post-fix evidence so far is 0/27 of the
plain `--lib` command plus §6.1's route check (0 foreign writers inside the window
in 3 instrumented runs, against 12/12 before). Eight instrumented runs did not fit
this pass's budget, and the honest statement is that the instrumented rate is
**unmeasured after the fix**.

### 9.4 New flake found while rating this pass's own suite: `subscribe_failure_modes`

`rpc::grpc_client::tests::subscribe_failure_modes` (`tui/src/rpc/grpc_client.rs:3832`)
**red the gate command twice** this pass, and it is not item 18, not item 24 and not
the fd-1 class (no `BrokenPipe` anywhere):

| run | log | suite time | load |
|---|---|---|---|
| 1 | `/tmp/readiness/suite-2.log` | 60.87 s | ~24–34 |
| 2 | `/tmp/g8.log` (this pass's gate) | 110.14 s | ~67 |

`2193 passed; 1 failed` both times, and — like item 24's logs — **no panic section
and no panic text**, so which assertion lost is unknown. What the test does is
readable even so: four short blocks each `connect_events()`, `sleep(300 ms)`, then
assert something about the client's state, with `stream_fails: true` and a
server-side failure mixed in, i.e. it is a *fixed-sleep → assert* shape against
live gRPC, which is the same family as items 6/8/14/24 rather than a new class.
Registered for the sweep with its two sightings.

**Rate, and the assertion is still unnamed — stated plainly.** This pass could not
reproduce it: **0 reds in 48 concurrent single-test copies** (3 rounds x 16,
`--nocapture`, every copy `rc=0`) and **0 in 3 further whole-suite runs**
(load 20–47, all green). So, like item 24, the trigger is *whole-suite plus external
load*, not copies — the two sightings stay 2 of the ~14 suite runs that have ever
been observed at load >= 24. Per the item's own protocol (rate -> mechanism -> fix
-> rate) that leaves the fix unmade on purpose: without a red there is no assertion
to name, and rewriting four `connect_events(); sleep(300 ms); assert!(...)` blocks
would be a guess dressed as a fix.

**What is now different.** §10's fix means the next red carries its `panicked at …`
line and the assertion **without** `--nocapture` — so the missing step is cheap, and
the next whole-suite red at load is the whole cost of naming it.

**Hypothesis, explicitly not measured.** Each block is a *fixed sleep standing in for
a state transition* on live gRPC (`stream_fails: true`, a server-side stream failure,
`127.0.0.1:1`, an unparseable address), asserting the client's state 300 ms later.
Under load the transition can be slower than the sleep, which is the family items
6/8/14/24 belong to; the same family's fix is to wait for the condition with a
generous bound instead of sleeping a fixed 300 ms and asserting.

### 9.5 The whole-suite rate of this pass, stated plainly

Rule 4b (a fix can introduce a flake — rate the whole suite) applied to the three
fixes of this pass (§6.1's stdout lock, §6.4's ordering, §7.1's launch window):
**4 green / 3 red across 7 whole-suite runs** of
`cargo test -p future-tui --no-fail-fast -j 4`:

* green: 4 runs (load 2.7–20; `/tmp/r7-suite-*.log`, `/tmp/g7-1.log`);
* red: the three at load ≥24, and **every red was in a test this pass did not
  touch** — `a_real_repository_creates_a_worktree_and_switches_to_it` ×1 (item 24,
  153.18 s) and `subscribe_failure_modes` ×2 (§9.4, 60.87 s / 110.14 s).

So the gate command is **not reliably green on a loaded host**, independently of
anything in this pass: the two failing tests are a real-`git` subprocess test and a
live-gRPC test, and both fail in their long-run shape. I cannot prove a negative
here — what I can say is that the failures are in files this pass does not touch,
that they are load-correlated, and that the green runs include three that were
started *alongside* each other. This is exactly the finding rule 4b is for, and it
belongs to whoever owns those two tests rather than to these three fixes.

## 10. The evidence gap is fixed: the entry point leaked the process-global panic hook

§8 registered this anomaly (five logs with a `FAILED` line, no
`---- <test> stdout ----` section and no panic text, every one of them a long
failing run) and — under a corrected heading, see §8 — my first explanation was
"the crash hook swallows panics, but a normal full run restores it". The first half
was right and the second half was wrong: the hook is installed by the **production
entry point**, which about twenty `index.rs` tests call.

### 10.1 The mechanism, proven in one command

`index::run` (`tui/src/index.rs:1070`) called `crate::crash::install()`, i.e. a bare
`std::panic::set_hook(Box::new(hook_body))` with no restore. `hook_body` returns
early unless the reporting is armed (`#[cfg(test)]`, deliberately: it keeps other
tests' deliberate panics out of `crash.log` and out of the gate log). So once any
`run(...)`-calling test had executed, the crash hook **was** the process's hook, and
every later panic went to a body that returns early — libtest's capturing hook was
gone, so no section body and no message.

Two probes, each run alone so scheduling cannot be blamed:

| probe (run alone) | output |
|---|---|
| `b_probe_plain` — `panic!` with no entry point run | `---- crash::hook_probe_test::b_probe_plain stdout ----` · `thread … panicked at tui/src/crash.rs:412:9` · `PROBE_B plain` |
| `a_probe_after_index_run` — `index::run(&["--version"])`, then the same `panic!` | `---- … stdout ----` · **no panic text at all** |

That is the exact signature of the five logs, and it also explains the *long-run*
correlation without appealing to timing luck: the hook is installed whenever a
`run(...)`-calling test executes, and under `-j 4` test order is interleaved, so a
panic that happens two minutes in (item 24, `subscribe_failure_modes`) essentially
always lands after that install, while a panic in the first moments of the run may
not. My instant probe in `--exact` mode kept its section only because it ran with
no entry point.

### 10.2 The second face of the same defect

The test-side guard was `InstalledHook(Hook)`: `take_hook()` before, `set_hook` the
saved one after. With two guards overlapping — two `run(...)` tests on different
threads, which the suite does all the time — the second guard saves the **crash**
hook (installed by the first) and puts it *back* when it drops, so the crash hook
outlives both. Same swallow, reached from the other direction.

### 10.3 The fix

`tui/src/crash.rs` now owns the scoped install (and the bare `install()` is
**private**, reachable only from `InstalledHook::install`, so the footgun is no
longer part of the crate's API), and it is **reference-counted**
rather than save-and-restore-per-call:

* `HOOK_STATE` (a `Mutex<HookState>` holding `depth` and the captured `original`)
  means the hook that was in place before the first install is captured **once** and
  restored only when the outermost guard drops, so neither face above can leak it;
* the mutex is poison-recovering (`unwrap_or_else(|e| e.into_inner())`) — refusing to
  restore because an earlier panic poisoned the lock would leak the hook, which is
  the bug being fixed;
* `InstalledHook::install()` is the public scoped API, and `index::run` uses it, so
  the entry point keeps its real purpose (a panic *inside* `run` is still reported)
  while no longer deciding what happens to panics after it returns.

**Verified in both directions.** The two new regression tests in
`crash::scoped_install_tests` each fail on the pre-fix code and pass after — I ran
that check rather than assuming it:

| regression test | on the pre-fix code | on the fix |
|---|---|---|
| `a_panic_after_the_entry_point_returns_reaches_the_previous_hook` (a counter hook is the observable) | **FAILED** (bare install restored in `run`) | ok |
| `overlapping_scoped_installs_do_not_leak_the_hook` (two guards overlap on a barrier) | **FAILED** (per-call save/restore restored in the guard) | ok |

The counters are the point: `catch_unwind` returns the payload whether or not the
hook captured it, so only "did the hook that was installed before see it" can
distinguish captured from swallowed.

### 10.4 Residual, stated rather than papered over

A panic raised **inside** the entry point while it holds the guard is still
swallowed in a test build: the crash hook is installed for `run`'s scope and returns
early when unarmed. That is narrow — it needs a panic in `run` itself (an internal
unwrap), not in the test's assertions, which happen after `run` returns and are now
captured — and the alternative closes it at a stated cost: making the unarmed body
**forward** to the captured hook would also let every `#[should_panic]` test's
deliberate panic be captured again, which is the noise the module's `report_armed`
gate exists to avoid, and it would invalidate
`dropping_the_guard_restores_the_hook_that_was_installed_before` (its observable is
exactly "the probe stays silent while the crash hook is installed"), so that test
would have to be rebuilt around the crash-log path instead. Recorded with both
sides; the delegation is the next step if a swallowed panic inside `run` is ever
observed.

### 10.5 What this changes for the campaign

Every `future-tui` **lib** failure from now on carries its `panicked at …` line and
the assertion, which is the thing `FLAKES.md` items 18, 24, 25 and 26 all record as
missing, and §8/§9.4 could not name. The next red in this target should be readable
directly from the gate log; `--nocapture` is no longer required to force the text
out. Two consequences worth stating: (a) any *older* log in this campaign that shows
the section-less signature was very likely this defect rather than a mystery, so it
is worth re-reading those items' evidence with that in mind; (b) `docs/testing/`
items whose "next check" is "get the panic text" can now be done by simply running
the failing target.

### 10.6 The `reconcile.py` STALE change (supervisor steer) — nothing fired in this scope

Checked rather than assumed: this pass's deliverable is a **flake record**, and the
newly visible STALE rows are ledger claims resolved against a report's uncovered
map. `docs/testing/flaky-tui.md` carries no ledger rows, and the `tui` ledger
(`docs/testing/module-tui.md`) plus `.future/cov100/` are outside this todo's write
set, so nothing here could fire and nothing was touched. If a STALE row does appear
for the `tui` module when the gate next runs, it is the supervisor's to route — the
one thing I can say from inside this work is that none of §1–§10 relies on a
line-level uncovered claim, only on counts and on measurements with a stated
command.

## 11. What §10's fix exposed, and the root causes it let me name

§10 removed the reason a `future-tui` lib failure had no message. This section is
what that bought, plus the one regression it introduced and the fix for it.

### 11.1 Rule 4b on my own fix: §10 introduced a crash-test flake, fixed here

The first thing the readable logs showed was a red **in my own change**. At 6
concurrent whole suites, **3 of 6** failed, and every failed test was in the crash
module (`panic_hook_restores_terminal_and_writes_crash_log`,
`panic_hook_survives_unwritable_crash_log`,
`dropping_the_guard_restores_the_hook_that_was_installed_before`, and my new
`overlapping_scoped_installs_do_not_leak_the_hook`), with — of course — no text,
because those tests *are* the hook.

**Found with a temporary diagnostic** (a `write_all` to `/tmp/hookdiag.log` inside
`hook_body`, reverted afterwards): the swallowed message was
`assertion `left == right` failed: the last guard must restore the hook it replaced,
not an inner install` with `left: 0, right: 1` — my counter probe saw nothing,
i.e. the process had been left holding a **swallowing** hook.

**Mechanism.** `index::run` installed the hook, and about twenty `index.rs` tests
call `run`, several from threads that hold no test lock. So an index test could
enter `InstalledHook::install()` while a crash test was between its own
`take_hook()` and `set_hook()` (both under `test_env::lock`, which the index tests
do not take):

1. crash test: `take_hook()` → the *crash* hook is what comes back; `set_hook(probe)`;
2. index test: `install()` sees `depth == 0`, captures **the probe** as `original`,
   installs the crash hook;
3. index test drops: `depth` 0 → restores `original` = the probe;
4. crash test then restores its own captured `previous` — which was the crash hook;
5. the crash test's guard drops at `depth` already 0, so the outer restore is
   skipped and the swallowing hook stays installed for the rest of the run.

**Fix (structural, not a counter tweak).** New
`crash::install_scoped_for_entry_point()`, used by `index::run`: **a test build
installs nothing**. The hook is a single process-global slot, so the entry point has
no business replacing libtest's hook under other tests; a test that wants the real
hook still installs it explicitly (`InstalledHook::install`), and production keeps
the behaviour that matters (`run` reporting its own panic). With the entry point out
of the picture the only hook mutations are the deliberate ones in the crash tests,
which all hold `test_env::lock`, so the counter can no longer be desynchronized.

**Rates.** Before the fix: **3 of 6** concurrent suites red, all in the crash
module. After: **0 crash failures in 34 whole-suite runs** (6 at 6-way, then 12 at
`-j 4`, 4 at `-j 12`, 12 at `-j 8`, every one 4- or 6-way concurrent) plus the 8
crash tests green standalone. The two new
`crash::scoped_install_tests` regression tests (each failing on the pre-fix code —
§10.3) still pass, and `a_panic_after_the_entry_point_returns_reaches_the_previous_hook`
now asserts the *entry point must not leave a swallowing hook installed*, which is
the invariant this defect violated.

### 11.2 Item 24: root cause named, fixed, and proven with a positive control

Six sightings of this test existed with **no message**, so the record carried three
guesses (the 120 s pump, `path.is_dir()`, `seen.len() == 1`). Once §10 was in, the
same load reproduced it twice in one batch, with **two distinct sites**:

```
thread 'app::tests::a_real_repository_creates_a_worktree_and_switches_to_it' panicked at tui/src/app.rs:12019:9:
timed out waiting for system message containing "Created worktree demo on branch feat/demo from main."
                                                    (suite 137.25 s, and 147.53 s, and 147.00 s at -j 12)

thread '…' panicked at tui/src/app.rs:23200:14:
called `Result::unwrap()` on an `Err` value: Os { code: 2, kind: NotFound, message: "No such file or directory" }
                                                    (suite 70.00 s)
```

**Root cause: another test in the same binary swaps the process-global `PATH`.**
`components/autocomplete.rs`'s `with_stubbed_path` (`:1709`) sets
`PATH` to a temp directory holding only its own stubs, and while it does, `git`
cannot be resolved anywhere. It does hold `test_env::lock()`, but *this* test does
not, so for the length of its run any `git` spawn can land in that window:
`Err(ENOENT)` at the test's own `rev-parse` (site 2), or — when the `/worktree new`
task is the one that cannot spawn `git` — no success message ever arrives and the
120 s `pump_until_msg` budget expires (site 1). Both sites are one cause, and it is
the fd-1 trap's pattern (`docs/testing/flaky-tui.md` §1–§6) applied to an env var:
**process-global state hijacked by a test, breaking an unrelated test.**

**Positive control, then the fix.** Poisoning `PATH` mid-flow from the test itself
reproduces the recorded signature exactly:

| tree | with `PATH` poisoned during the worktree flow |
|---|---|
| before the fix | **FAILED** — `called `Result::unwrap()` on an `Err` value: Os { code: 2, kind: NotFound }` |
| after the fix | **ok** (0.23 s) |

The fix is the test declining to depend on hijackable state:
`absolute_git()` resolves `git` once from the standard locations (then `PATH`, then
the bare name), and that absolute path drives **both** the test's own `git` commands
and the app's `GitCli` — the latter through the seam that already existed for
exactly this (`GitCli::with_program`, `tui/src/worktree.rs:146`). The production
default is untouched: `GitCli::new()` still means bare `git` on `PATH`, because in
the product the user's `PATH` is the right answer.

**Rate after the fix: 0 item-24 reds in 12 whole-suite runs** (3 rounds x 4
concurrent at `-j 8`), against **4 of 16** before it (3 of 12 at `-j 4` 4-way, 1 of
4 at `-j 12`) — and the red runs' suite times fell back from 137-147 s to 25-30 s. Unchanged and
explicitly out of scope: the other tests in this binary still swap `PATH`, so a
*future* test that spawns `git` without pinning it has the same exposure — that is
a suite-wide convention question, not this test's.

### 11.3 A sibling of §7's readiness flake, in `tui/tests/agent_sidecar_launch.rs`

One red in the same batch, and it is **not** item 24 or item 18:

```
test a_sidecar_candidate_that_exits_is_reported_and_the_next_one_is_tried ... FAILED
panicked at tui/tests/agent_sidecar_launch.rs:59:5:
the candidate's exit status must be reported verbatim: no Future Agent was reachable and no sidecar
could be started (…/deps/future: No such file or directory (os error 2); …/deps/future-agent:
No such file or directory (os error 2); future-agent: No such file or directory (os error 2))
```

The fixture points `PATH` at a directory holding one `future` stub that `exit 3`s,
and the assertion wants `future exited with exit status: 3` in the report. The
report instead lists the *three* unrunnable candidates and **omits the spawned
stub's entry entirely** — which the loop only does when a candidate consumed its
whole probe window without `try_wait` observing an exit (`ensure_agent_running_within`
kills the child after the window and pushes no failure line). So: the same family as
§7 (a fixture racing the launch loop's window), in a *different* file, with an
open question I could not close — why a stub that exits immediately is not seen by
`try_wait` in a 5 s window. **Rate: 0 of 20 runs alone** (load ~10), **1 of 34** in the
whole-suite batches (load 36), so it needs the batch conditions to appear. Registered with
that next check rather than "fixed" by widening anything — **both the question and the
flavour of the answer are in §12.1**, which closes it.

### 11.4 What is still unrated, and one measurement caution

* `subscribe_failure_modes` (§9.4) did **not** red in any of this pass's **34
  whole-suite runs** (6-way, then 4-way at `-j 4`/`-j 8`/`-j 12`), so it remains
  **unreproduced** and unfixed — 2 sightings ever. The panic text is now available
  the moment it does red, which is all §9.4 was waiting for.
* **Concurrency caution for whoever reproduces any of these:** at **6** concurrent
  whole suites this host stopped finishing them at all — all six hit a 560 s cap
  (`rc=124`) with the suite only part-finishable, so a "hang" there is contention,
  not a defect. 4 concurrent suites at `-j 8` is the useful setting: it reproduced
  both item 24 sites and the sidecar red within 12 runs, and each suite finished in
  20–150 s.
* The evidence gap is closed *for this binary*: every red in §11.2/§11.3 carried its
  assertion, which is the first time in this campaign that an item previously
  recorded as "panic text never captured" could be diagnosed from the log at all.

## 12. The sidecar-launch flake closed, and the supervisor's two steering items

### 12.1 `agent_sidecar_launch.rs`: the fixture's child raced the loop's *product* window

§11.3 registered this with an open question ("why is a stub that exits immediately
not seen by `try_wait` in a 5 s window?"). A temporary diagnostic inside
`ensure_agent_running_within` (a line per spawn / per `try_wait` / per expiry to
`$TUI_LAUNCH_DIAG`, reverted) answered it on the first green run:

```
spawn FAIL: …/deps/future: No such file or directory (os error 2)
spawn FAIL: …/deps/future-agent: No such file or directory (os error 2)
spawn ok: future
candidate future: attempts=40 window=5s
future: alive at attempt 0
future: alive at attempt 1
future: alive at attempt 2
future: exit seen at attempt 3: exit status: 3
```

The stub is `/bin/sh`; **the exit is observed at attempt 3, i.e. ~375 ms after the
spawn**, and the window is 5 s. So the margin was ~13x — and the mechanism is the
one §7.1 already named in the sibling file, not a `try_wait` defect: the fixture
raced the loop's **product** budget against a host-dependent fork/exec. When the
host is 13x slower (this suite's own suite-times inflate 6.5x at the loads that
reproduced it), the loop exhausts its 40 attempts and **kills the child silently**,
so the report lists the candidates that failed to *spawn* and omits the one that
spawned — which is precisely what the assertion is about.

**Positive controls, both reproducible on demand:**

| control | shape | result |
|---|---|---|
| window shorter than the child's latency (a 300 ms window, public API) | 2 attempts, expiry | **the recorded message, character for character** |
| A: product default (5 s) + an 8 s stub (`FIX_STUB_DELAY`, temporary) | expiry at 5 s, 5.13 s run | **FAIL**, same message |
| B: fixture window (60 s) + the *same* 8 s stub | observed at 8 s, 8.18 s run | **PASS** |

**Fix** (one file, test only): the fixture passes its own window —
`ensure_agent_running_within("auto", LAUNCH_FIXTURE_WINDOW)`, 60 s, documented as
160x the measured latency — exactly the pattern §7.1 established for
`agent_sidecar_readiness.rs`. The production default is unchanged, and the loop's
behaviour on a *real* sidecar is untouched: a real agent that cannot answer in 5 s
still gets killed at 5 s.

**Rates.** Before: 1 of 34 whole-suite batches (0 of 20 alone). After: **0 of 8
whole-suite runs** (§12.4), 4-way concurrent at `-j 8`, load ~18 — the A/B pair above
is the load-independent evidence, and it shows the failure gone for a child **21x
slower** than the one that triggered it.

**Corrected 2026-10-17 by §14: that is not the whole story.** In one later 4-way batch
at a much higher load this target failed **4 of 4 runs**, each **61.35 s** — my own
window expiring — while the lib target in those same runs passed 4 of 4 (2198 tests,
23-31 s). So this fix closes the *slow-but-progressing* child and does **not** close a
child the host starves so completely that it never reaches its `exit`: no window length
can, because the loop is waiting for an exit the OS has not delivered. §14 has the
measurement and what it implies.

**Not taken (recorded, with the reason).** The loop could push a failure line of its
own when a candidate's window expires, instead of killing it silently — that would
have made this failure self-explanatory and is arguably better product reporting
("tried X, it did not answer in 5 s"). I did not do it: it changes a user-visible
message for a defect that is now fixed and readable, it is not needed by any
assertion, and the project's rule is not to add beyond what the task requires. If a
future flake again shows a report that omits a candidate it tried, that line is the
first thing to add.

### 12.2 Steering item: `crash::install()` is now private

The bare, unrestoring install is now `fn install()` (was `pub`), with a doc comment
saying why: it is the footgun that caused the missing-evidence defect (§10), the
hook is a single process-global slot, and nothing outside the module has a reason to
install it without a scope. Its only caller is `InstalledHook::install`, which owns
the reference count and the captured previous hook — checked across the repo, not
assumed: `grep -rn "crash::install\|\binstall()" tui/src tui/tests` finds no other
caller, and the crate builds with no unused-item warning. Behaviour is identical;
the two characters close the hole. §10.3's text says so too, so the record does not
still advertise the footgun as API.

### 12.3 Steering item, adopted as a rule here: no red without its text

The supervisor's instruction — never report a red run without its panic text, and
treat missing text as itself a finding — is now how this record works, and the
scripts behind it print the assertion rather than a bare `FAIL` (`/tmp/hunt-red.sh`,
`/tmp/readiness-rate.sh`, and the batch loops all grep `panicked at` and dump the
`---- <test> stdout ----` body). Two consequences are already visible in §11.2/§12.1:
the item-24 root cause and this file's mechanism were both found *because* the text
was there. If a `future-tui` lib red is textless again, that is a regression of §10 —
the first checks are (a) whether `run(...)` is installing a hook in a test build
again (`install_scoped_for_entry_point`) and (b) whether the failing test is one of
the hook tests themselves, whose own text is swallowed by construction (§10.4).

### 12.4 Rates and gates for this pass

* `agent_sidecar_launch.rs`: 0 of 8 whole-suite runs, 4-way concurrent at `-j 8`
  (a third round of 4 was planned; the background batch was reaped after two, so the
  count is stated as what ran, not as what was intended); A/B as above. 20/20 alone.
* Item 24 (fixed in §11.2): 0 reds in the same 8 whole-suite runs.
* crash-module (fixed in §11.1): 0 reds in the same 8 runs.
* `subscribe_failure_modes` (§9.4): still **0 reds in 42 whole-suite runs** across
  §11 and §12 — unreproduced, so unfixed, with the text available the moment it reds.
* Gates: `cargo clippy -p future-tui --all-targets -- -D warnings` 0;
  `cargo fmt -p future-tui -- --check` 0; `cargo test -p future-tui --no-fail-fast
  -j 4` 0 (lib 2196). Workspace `cargo fmt --check` was red in **peer** files
  (`agent/src/rpc/commands/session_lifecycle_tests.rs:776`,
  `agent/src/rpc/commands/session_title.rs:377`) — reported, not formatted.
* No temporary code left: the three diagnostic hooks of this pass (the panic-hook
  file writer, the launch-loop `$TUI_LAUNCH_DIAG` writer, the stub-delay/window
  switch) are all reverted, and the only `TEMPORARY` strings in the tree are in this
  record describing them.

## 13. The `PATH`-and-spawn convention in the `tui` crate (the census, the choices, and a deterministic reproduction)

`FLAKES.md` item 27 named the class (a test leaks a process-global, and other tests
fail) and item 24 was its largest tui consumer. This section is the census the
supervisor asked for, bounded to the `tui` crate: **which sites swap `PATH`, which
tests can actually be reached by that swap, what was changed for each, and why.**
It is a *decision* record: a convention cannot be rate-tested, so the evidence is
the enumeration, the choices, and the one reproduction that does not need load.

### 13.1 The two rules

> **A test that mutates process-global state must restore it through RAII, not on the
> success path — a mutex is not a restore.**
>
> **A test that spawns an external program must pin an absolute path** (or hold
> `test_env::lock()` for the whole window in which it could spawn).

They are two different jobs, which is why this fix has two halves rather than one:
RAII removes the **destructive leak** (a panic leaves the mutation in place for every
later test in the binary — item 27 measured 27 cascading failures and a voided
instrumented run), while pinning removes the **legitimate-window** failure (the
swapping test is allowed to hold its mutation while it runs, so a concurrent spawner
still loses its `PATH` — that is what item 24 measured at 3 of 12 concurrent
whole-suite runs, 137-147 s instead of 25-30 s). Pinning cannot fix a leak, and RAII
cannot fix a window.

### 13.2 The census (tui crate only)

**A. Sites that swap the process-global `PATH`** — 5:

| # | site | helper | lock held | restore | after |
|---|---|---|---|---|---|
| A1 | `components/autocomplete.rs:1721` | `with_stubbed_path` | `test_env::lock()` | after `f()` | **`EnvVarGuard`** (RAII) |
| A2 | `components/autocomplete.rs:1898` | `with_path_prepended` | `test_env::lock()` | after `body()` | **`EnvVarGuard`** (RAII) |
| A3 | `components/autocomplete.rs:1999` | inline in `attachment_completions_fall_back_to_find_inside_the_cwd` | `test_env::lock()` | after the block | **`EnvVarGuard`** (RAII) |
| A4 | `tests/agent_sidecar_launch.rs:59` | inline; **one** test in its own binary | none needed | at the end | unchanged — blast radius is its own process, and the file has a single test |
| A5 | `tests/agent_sidecar_readiness.rs:193` | inline in the fixture helper; 3 tests serialize on `ENV_LOCK` | its own `ENV_LOCK` | at the end | unchanged — every test in that file re-sets `PATH` itself, and (b) below is what protects the spawn |

**B. Spawners that a swapped `PATH` can break** (same binary, no absolute pin, no
lock) — the reachable set:

| # | site | program | after |
|---|---|---|---|
| B1 | `worktree.rs::git_in` (fixture setup) | `git` | **pinned** `absolute_tool("git")` |
| B2 | `worktree.rs` `real_git_lists_creates_and_reports_a_worktree` | `git` (via `GitCli::new`) | **pinned** `GitCli::with_program` |
| B3 | `worktree.rs` `real_git_attaches_a_branch_that_already_exists` | `git` (via `GitCli::new`) | **pinned** `GitCli::with_program` |
| B4 | `agent_supervisor.rs::spawn_sleeper` | `sleep` / `cmd` | **pinned** |
| B5 | `agent_supervisor.rs::process_is_running` | `ps` / `tasklist` | **pinned** |
| B6–B9 | `clipboard.rs` tests: `cat`, `true`, `false`, `sh` + the two `Command::new` children | | **pinned** |
| B10 | `autocomplete.rs:626/653` (`fd`, `find`) | | **intended** — these are the swapper's own consumer; the stub `PATH` is the point |
| B11 | `terminal_posix.rs:1041` | `current_exe()` | already absolute — no change |
| B12 | `tests/cli_smoke.rs:11` | `CARGO_BIN_EXE_future-tui` | already absolute, and a separate binary |
| B13 | `app.rs::git_for_host` / `skills_cli_for_host` | `git`, `future` | **safe by design**: both are `cfg(test)`-overridden to a runner that refuses to spawn, so app tests never run a real tool |
| B14 | `skills_cli.rs` `resolve_program()` | `future` | production resolution, deliberately `PATH`-aware; its tests inject `SkillsCli::with_runner`, and the one that does not (`:1080`) only *resolves*, it never spawns |
| B15 | `external_editor.rs:401/403` | the user's editor | production by design; no test spawns one |

Two rows are the reason this is a census and not a grep: **B13** looks like a
spawner and is not (the test override refuses), and **B10** looks like a victim and
is the point of the test.

### 13.3 The reproduction, without load

No rate was needed: a pair of filters reproduces the class deterministically.

```
cargo test -p future-tui --lib -j 4 -- worktree:: components::autocomplete::
```

| tree | result |
|---|---|
| before (bare `GitCli::new()`, i.e. `git` from `PATH`) | **FAILED. 88 passed; 2 failed** — both `failed to spawn git: No such file or directory (os error 2)`, `finished in 1.15s` |
| after (pinned) | **ok. 90 passed; 0 failed**, `finished in 0.96s` |

Each filter alone is green (`worktree::` alone: 41 passed), which is what makes it a
*concurrent* defect rather than a broken test: `autocomplete`'s completion tests hold
`PATH` = a stub directory while they run, and a `git` spawn in that window fails.
This is item 24's mechanism reproduced in one second instead of one load spike, and
it is the check a future worker should run before claiming this class is clean.

**The RAII half is proven at the mechanism, not end to end**, and the doc says so:
`test_env::tests::env_var_guard_restores_the_value_after_a_panic` panics inside a
`set`, catches it and asserts the old value is back; `absolute_tool_ignores_a_swapped_path`
asserts the resolver answers from `/usr/bin`-style locations while `PATH` is
`/nonexistent-stub-dir`. An end-to-end version that panics inside
`with_stubbed_path` and then reads `PATH` **deadlocks** — the helper takes
`test_env::lock()` itself and the test would take it again — which is itself worth
knowing: **a helper that locks must not be called from a test that already holds the
lock** (the lock is a plain `Mutex`, deliberately non-reentrant, and the same trap
that made `report_armed` a thread-local in §10). I recorded the mechanism test rather
than adding a racy end-to-end one.

### 13.4 What a worker must do when adding a test here

1. If the test sets a process-global (`PATH`, `HOME`, `FUTURE_*`, the cwd, a signal
   disposition), do it with `test_env::EnvVarGuard` (or another RAII type), never
   with a `set_var` … `restore` pair — and take `test_env::lock()` as well if the
   value must be stable for the window, since the guard protects the *leak*, not the
   *race*.
2. If the test spawns a program: `crate::test_env::absolute_tool("name")` for a
   system tool, or the module's own `with_program`/`with_runner` seam for a
   production CLI (`GitCli::with_program`, `SkillsCli::with_runner`). The bare name
   is only correct when the test's *subject* is `PATH` resolution itself.
3. Do not add a second `test_env::lock()` on a thread that already holds it.
4. `app.rs::absolute_git` is the template this convention grew out of; it now
   delegates to `test_env::absolute_tool`, so there is one implementation.

### 13.5 Rates, and what is deliberately not claimed

* **Rule 4b** (a fix can introduce a flake) on this pass: **two** 4-way concurrent
  whole-suite batches at `-j 8`, 8 runs in total — 4/4 green at load ~18, and in the
  high-load batch all four red on `agent_sidecar_launch`, which is **not** a module
  this section touched (it is §12.1's flake, and §14 records what that says about the
  §12.1 fix; the lib target passed in all eight runs, 2198 tests, 23-31 s).
* The touched surface: 146 tests in `clipboard`, `worktree`, `agent_supervisor`,
  `autocomplete` and `test_env`, all green, plus the two new `test_env` guards and the
  deterministic before/after pair of §13.3.
* **Not claimed:** a rate for the class. The deterministic pair in §13.3 is stronger
  evidence than a rate for *this* mechanism, and the convention itself (§13.4) is
  untestable by construction.
* **Left alone on purpose**, with the reason in §13.2: the two integration-test files
  A4/A5 (own process, no cross-test reach in the same binary), the `cfg(test)`
  overrides B13, and the production `PATH`-aware resolutions B14/B15.
* **Outside this todo**: the same class exists in other crates (`FLAKES.md` item 27
  names `desktop/src-tauri/src/cli_tool.rs`, and the item's census counted 218 sites
  repo-wide). Those are other units' files; this section is the `tui` crate's census
  and the convention other units can adopt.

## 14. Rule 4b on my own §12.1 fix: it closes the slow case, not the starved one

The honest re-measurement of §12.1 under load, which **corrects the rate claim there**
(that one was taken at load ~18).

### 14.1 What was measured

| shape | result |
|---|---|
| `agent_sidecar_launch` alone, 3 runs | **3/3 pass** (0.26-0.64 s) |
| 4 concurrent copies of the target, no other load | **4/4 pass** (0.64-1.03 s) |
| 4 copies + **2 whole lib suites** running | **4/4 pass** (0.65-1.03 s) |
| whole-suite batch, 4 concurrent `--no-fail-fast -j 8`, **high load** | **all 4 red**, each `FAILED ... finished in 61.35 s`; the lib target passed in all four (2198 passed, 23-31 s) |
| the same batch again, load ~18 | **4/4 pass** (lib 2198 passed, 27-29 s) |

The red message is §12.1's verbatim, and the 61 s is my `LAUNCH_FIXTURE_WINDOW`: the
loop ran its 480 attempts (~127 ms each) and never saw the spawned stub's exit, so it
killed the child **silently** — which is why the report lists only the candidates that
failed to *spawn* and omits the one that spawned.

### 14.2 What that means, stated carefully

* **§12.1's fix is real but partial.** The A/B pair still holds (an 8 s stub passes with
  the fixture's window, fails with the production one), so the fix does what it claims
  for a child that is *slow*: it is observed, and the test passes.
* **A starved child is a different sub-mode and no window length fixes it.** The loop
  waits for an exit the OS has not delivered; 5 s, 60 s or 600 s all end the same way,
  one of them just slower. The 4-of-4 batch is the evidence: 61 s is my window
  expiring, not the child finishing.
* **The rate is steeper in load than the old record suggested.** 1 of 34 (§11.3) → 0 of
  8 at load ~18 (§12.4) → 4 of 4 in one high-load batch (§14.1), *same tree*. So the
  honest statement is "reduced at moderate load, still reproducible at high load", not
  "closed".
* **The cost of the current shape:** in the starvation case the target now burns **61 s**
  (instead of ~6 s with the old 5 s window) and still fails. Faster failure is not a fix
  either, but anyone tempted to raise the window further should know the remaining mode
  is not a latency problem.

### 14.3 The next check, and the change I still decline

The narrow question is **whether the stub's child ever runs** in the starved case. The
fixture can answer it without touching production: log the child's pid, the `try_wait`
results and the per-attempt wall clock (the reverted `$TUI_LAUNCH_DIAG` shape of
§12.1), then re-run the 4-way batch at high load. If `try_wait` reports `Ok(None)` for
the whole window while the pid is alive, the child is starved before it can exit — and
the only fixes left are to stop depending on a spawned child (a runner seam for the
candidate list, which the loop does not have) or to accept the flake with its load
profile recorded, which is what this section does.

I still do **not** add a "window expired" failure line to
`ensure_agent_running_within`, for the reason §12.1 gives (a user-visible message for a
defect no assertion needs). §14.1 strengthens the case for it — it would have made the
61 s batch self-explanatory — so it stays recorded as the first thing to add if this
sub-mode is hit again.

## 15. A second rule-4b finding on my own work: one of §11.1's two new tests is itself load-sensitive

The final gate run of §13's pass came back red, and the failing test is one I added in
§11.1:

```
test crash::scoped_install_tests::overlapping_scoped_installs_do_not_leak_the_hook ... FAILED
test result: FAILED. 2197 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 21.75s
```

**The text is missing, and that is itself the predicted finding.** §12.3 says a
textless red is a finding and names the two first checks; this run satisfies the
second one: the failing test **is** a hook test, so its own panic is swallowed by
construction (§10.4). Both §12.3 checks confirmed rather than assumed:
`install_scoped_for_entry_point` is still the entry point's only install path (check a),
and the failing test orchestrates the crash hook itself (check b).

**Rates, this pass:**

| shape | runs | failures |
|---|---|---|
| whole-suite lib target (`--no-fail-fast -j 8`, 4-way) | 8 | 0 |
| whole-suite lib target (single, `-j 4`) | 6 | **1** (this gate run, `finished in 21.75s`, at a higher load) |
| `crash::scoped_install_tests` alone, `-j 4` | 12 | 0 |
| that filter plus `terminal::tests::term_signal_failsafe_runs_restore_and_die_path` (the crate's other hook mutator) | 8 | 0 |
| full lib target at load ~4 | 4 | 0 |

So: **1 of ~14 whole-suite lib runs**, correlated with load (every green run above was
at load 4-18; the red one came in the same window as §14.1's 4-of-4 batch).

**Mechanism: not proven, and I am not going to claim one.** The test asserts on the
*process-global* hook slot (a probe counter), so anything that installs a hook between
"both guards drop" and the final `catch_unwind` makes `seen` 0. Every hook mutator I
can find holds `test_env::lock()` — `crash::tests::*`, `crash::scoped_install_tests::*`
and `terminal::tests::term_signal_failsafe_runs_restore_and_die_path` (checked with
`grep -rn "take_hook\|set_hook" tui/src`) — and my test holds it too, so the
interference is *not* explained by the code I can see. What the numbers do say is that
it needs a whole-suite + load to appear, which is as far as this pass got.

**What this does and does not change about §11.1.** The fix stands and its measurement
was real (3 of 6 concurrent suites red, all in the crash module, before; 0 of 34 after,
plus every green run above). What is now visible is that my *second* new test
over-reaches: it exists to prove "two overlapping guards restore exactly once", which is
**bookkeeping the module owns** (`HOOK_STATE.depth`/`original`), and it proves it by
observing the process-global slot — a shared resource other tests legitimately mutate.

**Next check, with one trap named.** Assert on the bookkeeping as well
(`depth == 0`, `original == None` after the overlap) — but **bookkeeping alone cannot
replace the probe**: under the per-call save/restore bug this test exists to catch,
`original` is taken on every drop, so the bookkeeping looks clean while the *crash hook
is the installed hook*. A robust version therefore needs to observe hook *installs*
(a counter incremented in `InstalledHook::install`, not in the panic hook) so the
assertion no longer depends on what the process-global slot happens to hold at one
instant. That is the change to make, and it is why this is handed over rather than
patched blind at the end of a long pass.

**Left deliberately unchanged:** the test as it stands still catches the original bug
(its two regression checks in §10.3 were run and failed on the pre-fix code), so
weakening it now would trade a real guard for a quieter log.

## 16. The three backlog items: the instrumented rate (closed), the paused clock (verified, not adopted), and the last unreproduced flake

### 16.1 §9.3 closed: the instrumented rate is **0 of 8**, and every run kept its coverage

The fd-1 class was first seen under `cargo llvm-cov`, where a dead run writes **no
lcov** ("the number does not exist"), so the plain-suite rate did not settle it. I ran
the module record's own command (`docs/testing/module-tui.md` §1.1) eight times:

```
CARGO_TARGET_DIR=target/cov-tui2 cargo llvm-cov -p future-tui --lib --no-report -- --test-threads=4
```

| run | rc | secs | `io error when listing tests` | ok summary | new profraw |
|---|---|---|---|---|---|
| 1 (incl. build) | 0 | 218 | 0 | 2 | 28 |
| 2 | 0 | 79 | 0 | 2 | 2 |
| 3 | 0 | 73 | 0 | 2 | 2 |
| 4 | 0 | 84 | 0 | 2 | 2 |
| 5 | 0 | 71 | 0 | 2 | 2 |
| 6 | 0 | 69 | 0 | 2 | 2 |
| 7 | 0 | 126 | 0 | 2 | 2 |
| 8 | 0 | 65 | 0 | 2 | 2 |

**`runs=8 failures=0`**, no `BrokenPipe` in any log, and `profraw_new=2` every time —
the run's coverage profile was written on every single run (the "lost lcov" face of
this class did not appear). Before fix B the same command produced **5 of 8**
instrumented attempts (§1.3), so this is the rate that was missing: instrumented
**0/8** now, **5/8** before.

Two honest scope notes:

* **Load was 4-10** for all eight runs (a quiet window — the 218 s first run is the
  instrumented build, not load). The campaign's instrumented failures were recorded at
  higher load, and §14 shows this host's behaviour is load-sensitive, so the correct
  claim is "0/8 at load 4-10", not "0/8 period". A high-load instrumented batch is the
  remaining measurement, and it needs other workers active.
* **Environment gotcha for the next worker (worth one minute to read).** This
  worktree's `cargo-llvm-cov` is **0.8.7**, while `docs/testing/module-tui.md` §1.1
  documents **0.9.1**. 0.8.7 looks for the component named `llvm-tools-preview`, but
  the pinned 1.97.0 toolchain ships it as **`llvm-tools`** (renamed), so the plain
  command fails with *"failed to find llvm-tools-preview"* and every run returns
  `rc=1` in 0 s — which is exactly what my first harness did before I noticed. The
  documented override fixes it and needs no install:

  ```bash
  tc="$HOME/.rustup/toolchains/1.97.0-aarch64-apple-darwin/lib/rustlib/aarch64-apple-darwin/bin"
  export LLVM_COV="$tc/llvm-cov" LLVM_PROFDATA="$tc/llvm-profdata"   # LLVM 22.1.6-rust
  ```

  The toolchain's own binaries are the right choice (they match the rustc that
  instrumented the code). **Consequence to flag:** the campaign's published llvm-cov
  numbers were produced by a *different* cargo-llvm-cov version than the one available
  here, so a re-measurement of any module's lcov figure in this worktree should state
  which pair (tool version + LLVM) produced it.

### 16.2 §9.2 verified: the paused clock passes 75 runs — and I still do not adopt it

`#[tokio::test(start_paused = true)]` (no `flavor`, i.e. the current-thread runtime)
makes `app::tests::wait_for_agent_retries_until_agent_appears` finish in **~0.07 s**
instead of ~1.5 s. This pass gives it the 20+ runs the item asks for, plus load:

| shape | runs | result |
|---|---|---|
| single test, quiet | 25 | **25/25 pass** |
| single test, under 2 whole lib suites | 14 | **14/14 pass** |
| 16 concurrent copies, under 4 whole lib suites | 16 | **16/16 pass** |
| 12 concurrent copies, under **8 CPU burners** (load ~17) | 12 | **12/12 pass** |
| inside a 4-way whole-suite batch, twice | 8 suite runs | the test ok in all 8 |

**Not adopted, and the reason is mechanical, not timid.** With a paused clock the
retry loop's `tokio::time::sleep(1 s)` completes immediately whenever the runtime has
nothing else ready, so under starvation the loop stops *sleeping* and starts
**spinning** — it would burn CPU as fast as the host allows while waiting for the very
task it is competing with. That is a strictly worse failure mode than the current
"waits politely, may exceed its budget". The measured runs cannot see it (they all
have an early readiness flip, so no spin occurs), and the regime that would show it —
genuine starvation, load >= 30 on this host — is the one window I could not reach
today (my burners topped out at ~17, and §14's collapse happened at a load this
session never reproduced).

Second, smaller objection: the paused clock also removes any *real-time* signal from
the test, so a future regression that makes startup genuinely slow would no longer be
visible anywhere. The 60 s backstop with its stated relation to the 1 s cadence (§6.5)
keeps that signal while removing the false failures that were actually measured, which
is the better trade until the spin is ruled out.

**One process note.** A supervisor safety snapshot captured the *temporary* flip
(`HEAD` briefly had `start_paused = true` on this test); the working tree restores
`flavor = "multi_thread"` and the test passes in 1.50 s. Nothing in the tree now
depends on the experiment.

### 16.3 §9.4 unchanged: `subscribe_failure_modes` is still unreproduced

**0 reds in the 14 whole-suite runs of this pass** (including a load-~20 window), and
0 in **56** whole-suite runs to date across §9-§16, against 2 sightings ever (60.87 s
and 110.14 s, both at load 24-67). Nothing new to fix: no red means no assertion to
name, and the four `connect_events(); sleep(300 ms); assert!(...)` blocks should be
rewritten only once the failing one is known (the expected shape remains a bounded
wait for the condition, not a weakened assertion).

**Next check for whoever gets a red:** the panic text now survives (§10), so the
assertion is the whole answer; the file is `tui/src/rpc/grpc_client.rs:3832`.

### 16.4 The two items of this todo already settled elsewhere

* **Item (5), the suite-wide convention** — done in §13 (this was
  `todo_b95513d17deb`, a separate todo; its census, its two rules, the
  `test_env::EnvVarGuard` / `absolute_tool` implementation and the deterministic
  reproduction all landed there). No new work here.
* **Item (6), the silent kill in `ensure_agent_running_within`** — decision unchanged
  and re-justified by §14: a "window expired" failure line would have made the 61 s
  batch self-explanatory, but no assertion needs it and it is a user-visible message.
  It stays the first thing to add if that sub-mode is hit again.

### 16.5 Gates for this pass

| command | exit | result |
|---|---|---|
| `cargo clippy -p future-tui --all-targets -- -D warnings` | 0 | clean |
| `cargo fmt -p future-tui -- --check` | 0 | clean |
| workspace `cargo fmt --check` | 0 | clean |
| `cargo test -p future-tui --no-fail-fast -j 4` | 0 | lib 2198 passed, integration targets ok |
| 4-way whole-suite batches | — | 3 batches, all green (12 suite runs); the retry test ok in each |
| instrumented (`cargo llvm-cov`, module-record command) | — | 8 runs, 0 failures, coverage written every time |

No source change survives this pass except the removal of the temporary
`start_paused` flip; the tree's only working-tree delta is the flake record itself.

## 17. The scanner's census of the `tui` crate, classified by blast radius; and item §15 rewritten

The supervisor copied `global-leak-scan.py` into the worktree (it had been outside every
unit's write set, which is why §13's census was by hand). This section is the scanner's
answer for `tui`, **classified** — the tool reports *"mutates a global with a
panic-capable call before the last restore"*, which is a shape; whether a test that does
not take `test_env::lock()` can be **hit** is the part it cannot answer, and it is the
part that decides.

### 17.1 The steering item: two HOME sites, converted (and a third the scanner then caught)

Both sites the supervisor named are the same shape as the PATH sites of §13, one
variable over, and both are the destructive-leak half (§13.1 rule i):

| site | was | now |
|---|---|---|
| `components/autocomplete.rs:1310` `file_path_home_unset_falls_back_to_root` | `remove_var("HOME")` + `remove_var("USERPROFILE")`, restored at the end (7 panic-capable calls before it) | `EnvVarGuard::remove("HOME")` + `EnvVarGuard::remove("USERPROFILE")` |
| `components/autocomplete.rs:1647` `file_path_completions_expand_tilde_and_keep_absolute_display` | `set_var("HOME", …)`, restored at the end (10 panic-capable calls before it) | `EnvVarGuard::set("HOME", …)` |

Converting the first one exposed a **third** raw site inside it that the scanner had
folded into the same function span: the same test sets `HOME` again later for its
"with a home that exists" phase (`env::set_var("HOME", cwd.path())`). That one is now a
nested `EnvVarGuard` too, and `global-leak-scan.py tui/src/components/autocomplete.rs`
reports **`total candidates: 0`**. The file's 49 tests pass.

**Tool note for the next census, in the other direction.** The scanner's `GLOBALS` regex
only matches raw `set_var(`/`remove_var(`, so it is blind to mutation through a helper —
after this conversion its answer for the file is "0 candidates", which means *"no raw
mutation"*, not *"no process-global is touched"*. Read it as a triage filter, exactly as
§4c says, and never as a clean bill.

### 17.2 The census, classified (25 candidates, `tui` crate)

| variable(s) | sites | reachable by a test that skips the lock? | decision |
|---|---|---|---|
| **`PATH`** | 3 autocomplete helpers + 2 integration files | **yes** — measured (item 24; §13.3 reproduces it in 1 s) | **done in §13**: helpers → RAII, consumers → pinned |
| **`HOME` / `USERPROFILE`** | `terminal.rs:1121`, `index.rs:1469/2618/2656/2687/2774/2835`, `app.rs:15517` | **yes** — the home is resolved by `crate::home` / `future_home()` (`tui_settings_path`, the crash-log path, skills) for anything that reads the user's `.future`, and many `app::`/`index::` tests do that without setting it and without the lock | **converted in §17.1 for the two autocomplete sites**; the nine here are the next item (same mechanical change, different files) |
| `COLUMNS` / `LINES` | `terminal.rs:1047/1066/1167` | narrow — read only as the *fallback* in `columns_with_ioctl`/`rows_with_ioctl`, and consumers that care pass an explicit size | recorded; low value |
| `TERM_PROGRAM` | `terminal_image.rs:669` | narrow — the capability detector; every image test sets its own | recorded |
| `EDITOR` / `VISUAL` | `app.rs:18879/18932/18961/20750` | narrow — resolved per test, and each editor test sets it | recorded |
| `LANG` / `LC_ALL` | `app.rs:22227/22489` | narrow — locale detection; both tests set them | recorded |
| `PI_TUI_DEBUG`, `PI_DEBUG_REDRAW`, `PI_TUI_WRITE_LOG`, `TERMUX_VERSION` | `app.rs:15517`, `terminal.rs:1121` | narrow — read only by the feature under test | recorded (rides along with the HOME conversion of those two rows) |
| `FUTURE_HOME`, `FUTURE_AGENT_SOCKET` | `tui/tests/agent_sidecar_{launch,readiness}.rs` | no — separate binaries, one/test-file serialized (§13.2 A4/A5) | unchanged |
| `FUTURE_AGENT_GRPC_ADDR` | `rpc/grpc_client.rs:2687` | n/a — the flagged function is a **helper** named `restore`, not a test; the scanner's function-level heuristic mislabels it | recorded |

So the *reachable* set is `PATH` (done), `HOME`/`USERPROFILE` (2 done, 9 to go), and
nothing else rises above "narrow" — which is the classification the tool cannot make.

### 17.3 §15's flake fixed: the regression guard no longer reads the shared slot

§15 left `crash::scoped_install_tests::overlapping_scoped_installs_do_not_leak_the_hook`
load-sensitive (1 of ~14 whole-suite runs) because it asserted on the **process-global
hook slot** — "the hook installed before the overlap saw exactly one panic" — a resource
other tests legitimately mutate.

`crash.rs` now counts **hook replacements** instead: `HOOK_INSTALLS` is incremented in
`InstalledHook::install` exactly when it actually replaces the hook, exposed as
`hook_installs()`. The test asserts

* `hook_installs()` advanced by **exactly 1** for two overlapping guards (two means the
  second guard saved the *crash* hook and will put it back — the leak), and
* `hook_depth() == 0` afterwards (secondary, and asserted for §15's stated reason: under
  the per-call bug the bookkeeping reads clean while the crash hook is what is installed,
  so bookkeeping alone cannot catch it).

The panic-based probe is gone, so the assertion touches no shared state.

**Verified in both directions** — run, not assumed:

| tree | result |
|---|---|
| pre-fix shape (save-and-restore on every call) | **FAILED** |
| current | **ok** |

**Re-rated at load: 12 whole-suite runs** (3 rounds x 4 concurrent, `-j 8`) — **12/12
green**, no red anywhere in the batch, and the rewritten test `... ok` in all 12, against
1 of ~14 before. That is the flake from §15 closed by construction rather than by luck.

### 17.4 Items (2) and (4) of this todo: unchanged, and stated as such

* **`agent_sidecar_launch.rs` starved-child sub-mode (§14)** — **not re-measured this
  pass**: the instrument-it-then-run-at-high-load check needs a load this session did not
  reach (my burners top out near 17; §14's collapse needed higher). The instrumentation
  recipe stays exactly as §14.3 wrote it, and the decision it feeds (a runner seam vs
  accepting the flake with its profile) is unchanged.
* **`subscribe_failure_modes` (§9.4)** — still unreproduced: 0 reds in this pass's 12
  whole-suite runs, so **0 of 68** whole-suite runs to date against 2 sightings ever. No
  red means no assertion to name; nothing to fix yet.
* **The backlog** (instrumented rate, `start_paused`, the §9.1 drainer, the silent-kill
  line) was closed or decided in §16 and is not re-opened here.

### 17.5 Gates for this pass

| command | exit | result |
|---|---|---|
| `cargo clippy -p future-tui --all-targets -- -D warnings` | 0 | clean |
| `cargo fmt -p future-tui -- --check` | 0 | clean |
| workspace `cargo fmt --check` | 0 | clean |
| `cargo test -p future-tui --no-fail-fast -j 4` | 0 | lib 2198 passed, integration targets ok |
| 3 x 4-way whole-suite batches (`-j 8`) | — | 12 suite runs, all green |
| `global-leak-scan.py tui/src/components/autocomplete.rs` | — | `total candidates: 0` (was 2) |