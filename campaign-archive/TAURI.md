# src-tauri coverage unit — measurement and classification notes

`desktop/src-tauri` is not a workspace member and is built in **two feature
flavors**; CI runs both (`Rust tests (Linux) (desktop gui|headless)`), and they
compile different code. A single-flavor measurement is not a coverage number for
this crate — the previous campaign reported one and had to list the headless
flavor as an open gap (`docs/testing/module-tauri.md` §5.3/§7.2).

## The measurement (both flavors, one report)

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=/Users/geilige/future-os/.worktrees/cov100-r2/target/test-home
ulimit -n 10240
export CARGO_TARGET_DIR=target/cov-tauri-<unit>      # YOUR private dir

make -C /Users/geilige/future-os/.worktrees/cov100-r2 desktop-sidecar-placeholder

# --test-threads=1 is MANDATORY: this crate's lib suite deadlocks in parallel on
# this host (HOME<->MOCK lock-order inversion). See .future/memory/testing-gotchas.md.
cargo llvm-cov clean
cargo llvm-cov --no-report          -j 4 -- --test-threads=1
cargo llvm-cov --no-report -j 4 --no-default-features --features headless -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/tauri-<unit>-lcov.info

python3 /Users/geilige/future-os/.worktrees/cov100-r2/.future/cov100/covlines.py \
    detail /tmp/tauri-<unit>-lcov.info --under desktop/src-tauri/src/<your subtree>
```

The `report` step merges the profraw of **both** runs — that is the point; do not
report after only one flavor. Iterate with the *uninstrumented* suite
(`cargo test --manifest-path desktop/src-tauri/Cargo.toml -- --test-threads=1`),
which is much faster, and measure once when you think you are done.

On this macOS host the suite is known-red in a few places that are unrelated to
coverage; do not fix them, and do not let them stop your measurement
(`cargo llvm-cov --no-fail-fast`, then `report` renders from the profraw even
when a test target failed):

* whole-lib parallel run deadlocks (hence `--test-threads=1`);
* a rotating set of `remote_peer::transfer*` / `commands::remote_peer::*download*`
  / `sandbox_availability` failures under a single serialized run.

## Classification: read these before calling anything unreachable

* `docs/testing/module-tauri.md` §5.1 — the waivers that were already checked
  against the code (`windows_power.rs`, `remote_host/sync_measurement.rs`'s
  ignored harness, `platform-unmeasured` rows). Reuse the reason, keep the
  category.
* `.future/memory/coverage-llvm-cov.md`, section "tauri / src-tauri 专坑" — the
  mechanisms that make whole classes of lines untestable without a production
  seam: `#[tauri::command]` async wrapper attribute lines (`AppHandle<Wry>`),
  `app.restart()` re-exec bodies, muda menu construction (main-thread assert),
  sidecar spawn arms, `CommandEvent`'s `#[non_exhaustive]` `_ => {}`.
* `docs/testing/platform-coverage.md` — which OS measures a cfg-gated file.

If a line is untestable **only because of the code's current shape**, say so and
propose the smallest seam (a generic `R: Runtime`, an injected side effect, a
`*_with` variant) in your report — do not invent a seam silently, and do not
delete the guard.

## Disk

`rm -rf target/cov-tauri-<unit>` as soon as your final measurement is captured
and its lcov copied to `/tmp`. Disk is the shared scarce resource; a src-tauri
target dir is ~10 GB.
