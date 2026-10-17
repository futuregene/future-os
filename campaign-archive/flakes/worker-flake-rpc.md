# F-RPC — flake report channel (agent `worker-flake-rpc`)

Unit: the `future-rpc` local-endpoint lease race. Full record: `docs/testing/flaky-rpc.md`.
Command under measurement, before and after, unchanged:
`cargo test -p future-rpc --lib -j 4` (199 → 201 lib tests), private
`CARGO_TARGET_DIR=target/cov-rpc`, isolated `HOME`, `ulimit -n 10240`.

## Rates

* **before** (unmodified tree): 1/30 (`flake-rate`, logs `/tmp/flake-rpc-before`) and
  19/390 in diagnostic loops (`/tmp/rpc-diag{,2,3}`) → pooled **20/420 ≈ 4.8 %**.
* **after** (fix + tests): **0/200** on the final tree (`/tmp/flake-rpc-final2`, load
  13.5–26.8, no rebuild inside the loop) and **0/1100** across all fixed-tree sweeps.

## Failing tests seen before the fix

| test | panic site | error |
|---|---|---|
| `transport::unix_endpoint::tests::active_legacy_listener_is_preserved_but_stale_socket_is_reclaimed` | `unix_endpoint.rs:310` final acquire | `AddrInUse: local endpoint is active or cannot be classified as stale` |
| same test, other runs | `unix_endpoint.rs:310` | `AddrInUse: local endpoint ownership is occupied` |
| `transport::unix_endpoint::tests::lease_excludes_other_binders_and_allows_clean_restart` | final re-acquire after `drop(listener)` | `…ownership is occupied` |
| `transport::tests::unpolled_local_listener_releases_its_endpoint_on_drop` | `transport.rs:694` | `…ownership is occupied` |
| `transport::tests::the_default_endpoint_reserves_binds_and_connects_under_its_own_home` | `transport.rs:1043` | `…ownership is occupied` |

No timeouts (deadlocks) were observed in 1500+ runs: every failure is a fast
`AddrInUse`, never a hang.

## Mechanism

`flock` lives on the open file description; `fork` copies the whole descriptor table, so
a child that has not `exec`ed yet keeps the description (and the lock, and a listening
socket) alive after the owner's `close()`. The crate's one `Command::spawn` test
(`unix_endpoint.rs:300`) is the only fork window in the suite. Window length: ~70 µs warm,
>300 ms cold on a loaded host.

## Fixes

* production: `LeaseFile::drop` unlocks explicitly (`flock(fd, LOCK_UN)`) before close —
  `packages/rpc/src/transport/unix_endpoint.rs`.
* tests: `fork_guard()` serializes the one spawning test against the test that asserts a
  released listener is reclaimable (removing it → 30/100 failures).
* regression tests: `dropping_a_reservation_releases_the_lease_a_shared_description_references`
  (negative control fails 1/1 with the flake's exact message) and
  `a_duplicated_listener_descriptor_keeps_a_dropped_socket_connectable`.

## Consequences for other units

* **Measurement hazard for myself, worth generalising**: a `flake-rate` sweep of 100 runs
  printed **24/100** failures and that number was invalid — the build was stale. I had
  restored the source with `mv` (mtime-preserving), cargo considered it fresh, and the
  logs contain no `Compiling future-rpc` line; the panic line in them (488) belongs to the
  pre-restore revision (the current one is 489). **Every flake number should be reported
  with evidence that the binary matches the source** (a `Compiling <crate>` line, or a
  panic line that moved with the edit).
* `packages/rpc/src/compat.rs:235` (steering item 16) does not exist: the file is
  `orchestration/goal/src/compat.rs`, registered as a scheduling race by its own unit in
  `docs/testing/module-goal.md` §1099, and measured by the goal crate's run — it cannot
  move any `packages/rpc` number.
* Coverage interaction: my change adds **0** uncovered lines; the instrumented check shows
  `unix_endpoint.rs` still at exactly its 5 pre-existing residuals (114,115,116,129,157).
