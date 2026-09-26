# policy.rs default-value mutants — raw evidence

Companion to the `channels/src/policy.rs` section of
`docs/testing/mutation-report.md` (item `todo_8dcc25abfda1`, agent `w-chan3`).
Kept under `mutation/` because that file is shared and may be rewritten by the
mutation job. Everything below is either extracted verbatim from the artifacts or
measured on this checkout; commands are given so it can be re-derived.

Artifacts analysed (both gitignored):

| run | dir | mutants | recorded | completed? |
|---|---|---|---|---|
| `out-policy` | `mutation/out-policy/mutants.out` | 12 diffs, 8 outcomes | 6 caught / 2 missed / 0 unviable | **no** — `outcomes.json` has `"end_time": null`, `"success": 0` |
| `timing-probe` | `mutation/timing-probe/mutants.out` | 1 | 0 caught / 1 missed | yes (baseline clean, `1451 passed; 0 failed`) |

## 1. Outcomes, and what each mutant's own log says

Extracted with:

```python
import re, glob, os
for p in sorted(glob.glob('mutation/out-policy/mutants.out/log/*.log')):
    t = open(p, encoding='utf-8', errors='replace').read()
    print(os.path.basename(p),
          sorted(set(re.findall(r'(\S+) \.\.\. FAILED', t))),
          re.findall(r'test result: (.*)', t))
```

| mutant | recorded | its own log's `test result` | failing test in the log |
|---|---|---|---|
| `52:5` dm = `"xyzzy"` | **missed** | `FAILED. 1450 passed; 1 failed` | `providers::telegram::tests::the_webhook_serves_verified_deliveries_and_rejects_the_rest` |
| `52:5` dm = `String::new()` | **caught** | `ok. 1451 passed; 0 failed` | — |
| `56:5` group = `String::new()` | **missed** | `ok. 1451 passed; 0 failed` | — |
| `56:5` group = `"xyzzy"` | **caught** | `FAILED. 1449 passed; 2 failed` | both telegram webhook tests |
| `60:5` require_mention = `false` | **caught** | `FAILED. 1449 passed; 2 failed` | both telegram webhook tests |
| `90:9` `check_dm -> Default::default()` | **caught** | `FAILED. 1450 passed; 1 failed` | `transport::ws::tests::a_healthy_connection_resets_the_backoff_before_the_next_failure` |
| `91:13` delete arm `"open"` | **caught** | `FAILED. 1450 passed; 1 failed` | telegram webhook test (see §3) |
| `92:13` delete arm `"disabled"` | **caught** | `FAILED. 1450 passed; 1 failed` | telegram webhook test |
| `112:9`, `119:13`, `96:21`, `96:68` | *(no outcome)* | *(empty log, never run)* | — |

Two mutants are recorded **caught** with a fully green log, and one is recorded
**missed** with a red log: the verdicts are not derived from the logs they sit
next to.

## 2. The only failures in the set are environment flakes

Verbatim from `log/channels__src__policy.rs_line_91_col_13.log`:

```
thread 'providers::telegram::tests::the_webhook_serves_verified_deliveries_and_rejects_the_rest' (8816) panicked at channels\src\providers\telegram_tests.rs:1414:5:
Err(webhook server cannot bind 127.0.0.1:18787: ... (os error 10048))
```

`os error 10048` is `WSAEADDRINUSE` — the port collision the plan already lists as
a pre-existing `future-channel` Windows failure. The same test name is the only
failure in six of the eight runs. No `policy::tests::*` test ever failed in any
log.

## 3. The `91:13` log cannot describe the `91:13` mutant

The log `log/channels__src__policy.rs_line_91_col_13.log` — named after
`delete match arm "open" in PolicyEngine::check_dm` — contains:

```
test policy::tests::dm_open_allows_anyone ... ok
```

Reproduced by hand on this checkout (mutated, run, reverted):

```
$ git diff -U0 -- channels/src/policy.rs     # arm deleted
$ CARGO_TARGET_DIR=target/cov-w-chan3 cargo test -p future-channel --lib -j 3 -- policy
test policy::tests::dm_open_allows_anyone ... FAILED
thread 'policy::tests::dm_open_allows_anyone' (16936) panicked at channels\src\policy.rs:207:9:
test result: FAILED. 25 passed; 1 failed; 1432 filtered out; finished in 0.13s
```

`config("open", …)` with an empty DM allowlist falls into the `_` arm once the
`"open"` arm is gone, so the test cannot pass. It passed in that log.

## 4. `unviable.txt` is empty although one mutant cannot build

`90:9` replaces the whole `check_dm` body with `Default::default()`. In
`channels/src/policy.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access { Allowed, Denied(String) }
```

`rg "Default for Access" channels/src` matches only `impl Default for
AccessPolicyConfig`. `Access` therefore has no `Default`, the mutant cannot
compile, and cargo-mutants reports build failures as *unviable* — yet
`unviable.txt` is empty and this mutant carries a `CaughtMutant` verdict with a
137 KB test-run log. (Read from the sources; not reproduced by a build.)

## 5. Measured verdicts for the `default_*` mutants (this checkout)

```
CARGO_TARGET_DIR=target/cov-w-chan3 cargo test -p future-channel --lib -j 3 -- policy
```

| step | mutation | result |
|---|---|---|
| baseline | none | 19 passed, 0 failed |
| gap proof | dm→`"xyzzy"`, group→`String::new()`, require_mention→`false` | **19 passed, 0 failed** — all three unpinned |
| after fix | same three | 6 failed (the new tests; per-test panic messages in the report's §4) |
| behavioural | dm→`"open"` only | `an_unconfigured_engine_refuses_a_stranger_in_dm` failed: `got Allowed` |
| arm probe | delete `"open"` arm only | `dm_open_allows_anyone` failed (`policy.rs:207`) |
| restored | none | full `--lib`: **1458 passed, 0 failed**; `git diff` = one insertion-only hunk in `mod tests` |

## 6. What this means for scoring

* `out-policy` for `channels/src/policy.rs` must **not** be read as 6/8 = 75 %.
  The 6 "caught" verdicts come from a port collision and two of them contradict
  their own logs; the honest count is **0/8 genuinely caught** before the fix.
* `mutation/summary.json` was deliberately **not** written by this item. Whoever
  produces it must re-run on the post-fix commit (the recorded verdicts predate
  it) and must neutralise the flaky Telegram-webhook / ws-timing tests first —
  otherwise the score measures `WSAEADDRINUSE`, not the tests.
