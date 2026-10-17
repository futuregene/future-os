# Flake workstream — `future-rpc` local-endpoint lease race (unit F-RPC)

Status: **fixed in production code, with a measured before/after rate**.
Everything below was measured in this worktree (`.worktrees/cov100-r2`,
branch `test/cov100-r2`, base `8449acf6`); no state-changing git command was run.

## 0. Run identity and environment

| field | value |
|---|---|
| unit / agent id | F-RPC (`worker-flake-rpc`) |
| worktree | `/Users/geilige/future-os/.worktrees/cov100-r2` |
| revision measured | `f32dd96a` / `642c66f6` snapshots of base `8449acf6`; `packages/rpc/**` unmodified by peers throughout |
| host | macOS (darwin, arm64), 32 CPUs; load average **10–27** while measuring (eight campaign workers share the host) |
| toolchain | `cargo 1.97.0 (c980f4866 2026-06-30)` from the pinned toolchain |

Every shell used:

```bash
ulimit -n 10240
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=$PWD/target/test-home          # the real ~/.future must not leak into tests
export CARGO_TARGET_DIR=$PWD/target/cov-rpc # private, uninstrumented
mkdir -p "$HOME"
```

Flake work needs no instrumentation (`FLAKES.md` §2.3), so all rates below come from
`cargo test` in `target/cov-rpc`. The one instrumented run (§6) used a separate
`target/cov-rpc-instr`, which was deleted immediately after the lcov was copied to `/tmp`.

**The command under measurement, before and after, unchanged:**

```bash
timeout 300 cargo test -p future-rpc --lib -j 4          # 199 → 201 lib tests
```

Driver: `.future/cov100/flake-rate.sh --label <label> --runs <N> --timeout 300 -- <cmd>`.

## 1. Rates

### Before — production code and tests as committed (199 lib tests)

| measurement | runs | failures | timeouts | failing test | error |
|---|---|---|---|---|---|
| `flake-rate --label rpc-lib-before` | 30 | **1** | 0 | `transport::unix_endpoint::tests::active_legacy_listener_is_preserved_but_stale_socket_is_reclaimed` (unix_endpoint.rs:310) | `local endpoint is active or cannot be classified as stale` |
| loop, probe diagnostics only | 120 | **4** | 0 | lease tests, 4 sites | `local endpoint ownership is occupied` |
| loop, lock/fd diagnostics, `--test-threads=32` | 150 | **10** | 0 | 4 sites | `local endpoint ownership is occupied` |
| loop, + timestamps | 120 | **5** | 0 | 3 sites | `local endpoint ownership is occupied` |

Pooled before: **20 failures / 420 runs ≈ 4.8 %** (1/30 on the harness, 19/390 in the
diagnostic loops; the loops differ from the harness run only by `--nocapture` and by
`eprintln!` diagnostics added *inside* the lock path — they are listed separately above
rather than folded into one number). Logs: `/tmp/flake-rpc-before`, `/tmp/rpc-diag{,2,3}`.

The four failing sites the flake surfaced, in both symptoms:

* `transport::unix_endpoint::tests::lease_excludes_other_binders_and_allows_clean_restart` — final re-acquire after `drop(listener)`
* `transport::tests::unpolled_local_listener_releases_its_endpoint_on_drop` (transport.rs:694) — second `bind_local_at` after `drop(incoming)`
* `transport::tests::the_default_endpoint_reserves_binds_and_connects_under_its_own_home` (transport.rs:1043) — `bind_local()` after `drop(reserved)`
* `transport::unix_endpoint::tests::active_legacy_listener_is_preserved_but_stale_socket_is_reclaimed` — final re-acquire after `drop(listener)`

### After — `LeaseFile` + `fork_guard` + two regression tests (201 lib tests)

| measurement | runs | failures | timeouts | note |
|---|---|---|---|---|
| `flake-rate --label rpc-final2` (final tree) | 200 | **0** | 0 | load 13.5–26.8; **0 rebuilds inside the loop** (binary verified to match the source) |
| `flake-rate --label rpc-verified` (fork-test variant) | 200 | 0 | 0 | load 13.5 |
| `flake-rate --label rpc-lib-after2` (fork-test variant) | 200 | 0 | 0 | |
| loop, instrumented build | 300 | 0 | 0 | |
| `flake-rate --label rpc-B` / loop `--nocapture` | 100 + 100 | 0 | 0 | instrumented |
| `flake-rate --label rpc-all-targets`, `cargo test -p future-rpc -j 4` (lib **and** the integration targets) | 30 | 0 | 0 | load average 38.6 |

Pooled after, all with the production fix present: **0 failures / 1130 runs**, no timeouts.
Headline pair: **20/420 (≈4.8 %) → 0/200 on the final tree** (0/1130 overall, the last 30 of
which cover the crate's integration test targets too).

## 2. Mechanism

`flock` locks belong to the open **file description**, not to the descriptor.
`fork(2)` copies the caller's whole descriptor table into the child, and the inherited
descriptors are only closed when that child `exec`s (`O_CLOEXEC`; the lease is opened
with `O_CLOEXEC` at `unix_endpoint.rs:95`). Until then *every* descriptor another test
holds is duplicated in the child, and `close()` in the owner does not release the
description's lock. One mechanism, two symptoms:

* **(a) lease** — `LocalEndpointReservation::acquire` takes
  `flock(LOCK_EX|LOCK_NB)` on `<endpoint>.owner.lock` (`unix_endpoint.rs:75`). If the
  previous owner dropped its lease while a forked child still referenced the
  description, the flock fails with `EWOULDBLOCK` (errno 35) and the next acquire
  reports `AddrInUse: local endpoint ownership is occupied: <path>`.
* **(b) stale classification** — `stale_socket` decides "is this endpoint dead?" by
  connecting to it inside a 250 ms budget (`unix_endpoint.rs:141`). A *listening
  socket* descriptor that a forked child inherited keeps the socket connectable, so
  the probe connects instead of seeing `ECONNREFUSED` and the classification reports
  `AddrInUse: local endpoint is active or cannot be classified as stale: <path>`.

The only process-spawning test in the crate is
`transport::unix_endpoint::tests::endpoint_lease_is_exclusive_across_processes_even_before_binding`
(`Command::new(current_exe())`, `unix_endpoint.rs:300`); it is therefore the source of
every fork window in this suite. Its window is short when the child's image is warm and
long when it is cold: see §3.

### Evidence (each item measured, not inferred)

1. **The holder is another process, not a leaked fd in the test.** On every *unexpected*
   `flock` failure, an `fstat` sweep of the failing process's descriptors (0–511) found
   only the descriptor that had just been opened for the failing acquire; the owner's
   descriptor was already closed — so something else refers to the description.
2. **Same-millisecond correlation with the fork.** Instrumented runs printed a
   process-relative timestamp for every lock attempt and for the `spawn`. In all five
   instrumented failing runs the failing acquire and the run's single `spawn` fell in the
   same millisecond (e.g. `t=3ms lock_FAIL fd=6 thread=…active_legacy…` beside
   `t=3ms spawn_end pid=2620 child=2621`), while the *intentional* failures
   (`assert!(acquire(..).is_err())`) show a second, still-open local fd on the same inode.
3. **Deterministic demonstrations** (run before the final test shape changed):
   * a bare `fork()`ed child parked in `read` (never `exec`ed) that inherited the lease
     descriptor made the parent's re-acquire fail with
     `local endpoint ownership is occupied` *after* the parent closed its own descriptor;
     with the child gone the acquire succeeded;
   * a child holding an inherited *listener* descriptor made
     `UnixStream::connect(path)` succeed after `drop(listener)` (`DIAGLISTEN
     alive=Ok("connected") after=Err(ConnectionRefused)`, 3/3 runs).
4. **Fork-window length** (`Command::spawn`, measured by retrying the acquire in a tight
   loop after `drop`): warm child image → first success ≈ **70 µs**; cold page cache on a
   loaded host → **still blocked after 300 ms** (1805 failed attempts in the budget). This
   is also why the documented CI rate (`GOTCHAS.md` §38 / round-1 memory: 9/30 and 16/30)
   is far above what a warm local tree shows (4.8 %).
5. **The 250 ms classification budget is not the cause.** The victim test alone (no other
   test in the binary, hence no fork) passed **0 failures / 100 runs** — and again
   **0/100** with 24 CPU spinners running (`yes > /dev/null` ×24) to emulate a loaded host.
   The stale-classification symptom needs the concurrent suite, i.e. a fork window.
6. **Ruled out**: path collision (each case uses its own `tempfile::tempdir()`); fd
   exhaustion (the failing errno was always `EWOULDBLOCK` = 35, never `EMFILE`); a plain
   timeout (`timed_out=true` never appeared in several hundred instrumented probes; probes
   took 5–29 µs); a lock held by the same process through a second fd (§1 of the evidence).

## 3. Fix

### Production fix — explicit release (`packages/rpc/src/transport/unix_endpoint.rs`)

New `LeaseFile(File)` newtype owning the lease, with

```rust
impl Drop for LeaseFile {
    fn drop(&mut self) {
        // Unlock before the descriptor is closed: `close()` alone is not a
        // release while another descriptor — a `dup`, or a `fork()`ed child that
        // has not exec'd yet — still references this file description.
        unsafe { libc::flock(self.fd(), libc::LOCK_UN) };
    }
}
```

`LocalEndpointReservation` and `SocketCleanup` hold `LeaseFile` instead of `File`
(`acquire` uses `lease.try_lock()`), so both release paths — a reservation that is
dropped without binding, and the listener's teardown — end ownership at the drop. This is
`fork(2)`'s documented semantics: "if a process holding a lock on a file forks and the
child explicitly unlocks the file, the parent will lose its lock", i.e. an explicit
`LOCK_UN` clears the lock for the whole description even while a `dup` or a child's
inherited descriptor still exists.

**This is a production fix, not a test fix.** Who it affects in production: an agent
process that spawns children (tools, shells, sandboxes) forks them with the lease
descriptor open; if the agent then releases the endpoint (shutdown, or a reservation that
fails its staleness check) and something re-binds within the child's fork→exec window, the
old code reports `local endpoint ownership is occupied` and refuses to start — a
loaded/cold host widens the window (§2.4). The API, the error messages and the
classification rules are unchanged; only the release is now real.

### Test-topology fix — `fork_guard()`

The stale-classification symptom has no production fix: a listening socket that some
process still holds open *is* active, and refusing to steal it is correct. The defect is
in the suite's topology: `fork` duplicates the process's descriptors into a child, so a
test that asserts "the descriptor I dropped is released" is wrong for the whole window.
`tests::fork_guard()` is one crate-local mutex; the test that spawns a process holds it for
the child's whole fork window, and the test that asserts a released listener is
reclaimable holds it for its body. Tests that assert a released *lease* need no guard —
with `LOCK_UN` the release no longer depends on descriptors going away.

Evidence that it is load-bearing (A/B on the same command): removing the guard from the
vulnerable test → **30 failures / 100 runs**, all `local endpoint is active or cannot be
classified as stale`; with it → 0/1130. (Earlier, while the new fork-based regression test
took no guard, the suite failed **67/200** for the same reason — a fork window created by
my own test.)

### Regression tests (deterministic, no timing input)

1. `dropping_a_reservation_releases_the_lease_a_shared_description_references` — `dup()`s
   the lease descriptor (in-process equivalent of a fork-inherited descriptor), drops the
   reservation, and requires the next acquire+bind to succeed.
   **Negative control**: with the `LOCK_UN` line disabled, it fails 1/1 with exactly the
   flake's message — `AddrInUse: local endpoint ownership is occupied: <tempdir>/agent.sock`.
2. `a_duplicated_listener_descriptor_keeps_a_dropped_socket_connectable` — pins the
   premise of `fork_guard`: with a `dup` of the listener alive, `connect()` still succeeds
   after `drop(listener)`, and only fails once the last descriptor is gone.

Both use `dup` rather than `fork` deliberately: an honest fork-based version of test 1 was
written first and worked, but a forked child that `_exit`s writes no profile, so its body
appeared as 7 uncovered lines in `unix_endpoint.rs` (§6). `dup` exercises the identical
invariant — the lock lives on the shared description — inside the profiled process.

## 4. Invalidated measurement (recorded, not hidden)

A `flake-rate --runs 100` sweep reported **24/100** failures on what I believed was the
guarded build. It was not: I had restored the source with `mv`, which preserves the
pre-edit mtime, and cargo's mtime fingerprint treated the tree as fresh — the 100 logs
contain **no `Compiling future-rpc`** line, and the panic line in them is `unix_endpoint.rs:488`
(the guard-less revision) while the guarded source's corresponding `.unwrap()` is at 489.
That sweep measured the deliberately unguarded A build. Every "after" number in §1 is
taken on a build whose recompilation was confirmed (`Compiling future-rpc` present in the
manual run that preceded the sweep, absent inside the sweep).

Lesson worth carrying into the campaign: **`mv`-restoring a file can leave a stale
binary**, so a rate must be reported together with evidence that the binary matches the
source (the `Compiling <crate>` line, or a panic line number that moved with the edit).

## 5. Residual and open points

* **Not reproducible after the fix**: 0/1130 leaves nothing to observe for either
  symptom. The pre-fix reproduction is preserved as the rates in §1 and the logs in `/tmp`.
* **Same-mechanism residual (not observed, not fixed)**: if an agent is `SIGKILL`ed while
  one of its children is between `fork` and `exec`, that child still holds the lease
  description, and the killed process runs no `Drop`. A new instance starting inside that
  window can still report "occupied" until the child `exec`s. Nothing in the owner's code
  can run at that point; a bounded re-probe in the acquirer would be the only mitigation
  and was deliberately **not** added (it would slow down the "another agent owns this
  endpoint" verdict, which is the common case).
* **Solo, and not the cause**: the 250 ms `stale_socket` budget is unchanged. §2.5 shows
  it is not what these failures were.
* **Other units' mined candidates** (`FLAKES.md` §5b) were not touched:
  `agent/src/session/persistence.rs:1587`, `cli/src/commands/doctor.rs:815`,
  `channels/src/feishu/feishu_ws.rs:254`, and `packages/rpc/src/compat.rs:235` — the last
  one is a misattribution: there is **no `packages/rpc/src/compat.rs`**; the file is
  `orchestration/goal/src/compat.rs`, whose line 235 the goal unit's own record registers
  as a scheduling race (`docs/testing/module-goal.md` §1099). It is measured by the goal
  crate's run and therefore cannot move any `packages/rpc` number.

## 6. Coverage interaction (口径 A)

`cargo llvm-cov -p future-rpc --lib --lcov` on the final tree
(`/tmp/rpc-flake-lcov2.info`, instrumented dir deleted immediately):

```bash
python3 .future/cov100/covlines.py detail /tmp/rpc-flake-lcov2.info --under packages/rpc/src/transport/unix_endpoint.rs
# -> packages/rpc/src/transport/unix_endpoint.rs: 114,115,116,129,157
python3 .future/cov100/covlines.py total  /tmp/rpc-flake-lcov2.info --under packages/rpc/src | tail -1
# -> covered=5855 measured=6088 uncovered=233
```

The 5 residual lines are exactly the pre-existing ones the owning unit registered as
`unreachable-in-this-environment` (`docs/testing/module-goal.md` §1061-1062: the two
identity TOCTOU guards and `stale_socket`'s non-`NotFound` arm). **This change adds zero
uncovered lines**: the `LeaseFile` code and both new tests are exercised on every run, and
the fork-based variant that would have added 7 uncovered lines (a `_exit`ed child writes
no profile) was replaced by the `dup` construction for that reason.

## 7. Reproduce

```bash
cd .worktrees/cov100-r2
ulimit -n 10240
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=$PWD/target/test-home CARGO_TARGET_DIR=$PWD/target/cov-rpc
mkdir -p "$HOME"

cargo test -p future-rpc -j 4                      # all targets green (201 lib tests)
.future/cov100/flake-rate.sh --label rpc-check --runs 50 --timeout 300 \
    -- cargo test -p future-rpc --lib -j 4         # expect failures=0 timeouts=0

cargo clippy -p future-rpc --all-targets -j 4 -- -D warnings
rustfmt --edition 2021 --check packages/rpc/src/transport/unix_endpoint.rs
```

Negative controls (each must fail, then be reverted):

```bash
# 1. remove the explicit unlock: comment out the LOCK_UN call in LeaseFile::drop
cargo test -p future-rpc --lib -j 4 -- dropping_a_reservation_releases
#    -> FAILED: "local endpoint ownership is occupied: <tempdir>/agent.sock"

# 2. drop the serialization: delete `let _fork_window = fork_guard();` from
#    active_legacy_listener_is_preserved_but_stale_socket_is_reclaimed
.future/cov100/flake-rate.sh --label rpc-noguard --runs 100 --timeout 300 \
    -- cargo test -p future-rpc --lib -j 4
#    -> ~30/100 failures, all "local endpoint is active or cannot be classified as stale"
```
