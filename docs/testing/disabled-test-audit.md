# Disabled-test audit

**Acceptance contract: `disabled-test-audit`.** Declared gate:
`python .future/cov100/verify.py skipped` → **PASS (exit 0)**. Sibling artifact:
`weak-test-audit` (gate `python .future/cov100/verify.py weak`).

Audit of every *disabled* test in the tree: `#[ignore]`, `it.skip(` /
`test.skip(` / `describe.skip(`, `xit(` / `xdescribe(` / `xtest(`,
`@pytest.mark.skip` / `@unittest.skip`, and `cfg_attr(..., ignore)`.

Owner: cov100 audit task. No test or source file was modified to produce this
document — it is a fact-finding artifact only.

## Conclusion (the headline)

* **This branch introduces zero newly disabled tests.** The set of `#[ignore]`d
  Rust test *functions* is identical to `origin/main`'s: **26 now, 26 on
  `origin/main`, name-for-name (added = 0, removed = 0)**.
* **No test was disabled by this goal to make a run go green.** Every disabled
  test in the tree predates the branch, and each carries a reason that is about
  environment/measurement, never about a failing assertion (see the inventory).
* The only `#[ignore]`-related edits on this branch are two *reason string*
  updates (a moved measurement-script path) on tests that were already ignored;
  their names and bodies are unchanged (details below).

## The check

```bash
python .future/cov100/verify.py skipped
```

Before this file existed the command failed with (the 28 marker lines it prints
are reproduced in the inventory below):

```
FAIL: 6 file(s) with disabled tests are not in the audit doc
disabled markers found: 28; files referenced by the audit doc: 6
  [UNAUDITED] agent/tests/linux_sandbox_smoke.rs:70  #[ignore = "requires a native Linux host with a working system bwrap"]
  ... (26 more) ...
```

The scanner's patterns (`verify.py` `DISABLED_PATTERNS`) are
`#\[ignore\b`, `\b(it|test|describe)\.skip\b`, `\b(xit|xdescribe|xtest)\(`.

### Branch-vs-`origin/main` comparison (the part the scanner does not do)

The scanner counts what is in the tree *now*; "new on this branch" needs a
diff against `origin/main`. Two independent checks were run:

1. **Marker-line diff.** Scan every `.rs/.ts/.tsx/.py` under
   `agent/ channels/ cli/ orchestration/ packages/ tui/ desktop/ mobile/` and
   compare each matched line against `git show origin/main:<path>`:

   ```
   current markers: 28   origin/main markers: 27
   markers not present verbatim in origin/main:
     desktop/src-tauri/src/remote_host/sync_measurement.rs:584
     desktop/src-tauri/src/remote/publisher/coalesce.rs:891
     desktop/src-tauri/src/remote/publisher/coalesce.rs:1142
   ```
   All three are explained below — two are reworded reason strings on tests that
   were already `#[ignore]`d in `origin/main`, one is a doc comment.

2. **Ignored-test-name set diff** (the authoritative one — it compares test
   identities, not text). For each file, collect the name of the `fn` that
   follows each `#[ignore]` attribute, then diff the sets:

   ```
   ignored tests now: 26   on origin/main: 26
   ADDED  (ignored now, not ignored on origin/main): (none)
   REMOVED/renamed: 0
   ```

   The equivalent script (run from the worktree root):

   ```python
   import re, subprocess
   from pathlib import Path
   ign, fn = re.compile(r"#\[ignore\b"), re.compile(r"\bfn\s+(\w+)")
   def names(text):
       ls, out = text.splitlines(), []
       for i, l in enumerate(ls):
           if ign.search(l):
               for j in range(i, min(i + 6, len(ls))):
                   m = fn.search(ls[j])
                   if m:
                       out.append(m.group(1)); break
       return out
   roots = ["agent","channels","cli","orchestration","packages","tui","desktop","mobile"]
   for p in (q for r in roots for q in Path(r).rglob("*")
             if q.suffix in (".rs",".ts",".tsx",".py") and "node_modules" not in q.parts):
       now = names(p.read_text(encoding="utf-8", errors="replace"))
       if not now:
           continue
       rel = p.as_posix()
       old = names(subprocess.run(["git","show","origin/main:"+rel],
                                  capture_output=True, text=True).stdout)
       for n in now:
           if n not in old:
               print("ADDED", rel, n)
   ```

3. **The other disabled-test forms yield zero hits repo-wide** —
   `.skip(` / `xit(` / `xdescribe(` / `xtest(` / `@pytest.mark.skip` /
   `@unittest.skip` / `cfg_attr(..., ignore)`: no match in any `.rs`, `.ts`,
   `.tsx`, `.py`. All 28 markers are Rust `#[ignore]` attributes or prose that
   mentions `#[ignore]`.

## Inventory — the 26 pre-existing ignored tests (not introduced here)

These are the `origin/main` baseline. Each row states why it is ignored and what
would lift it.

| file:line | ignored test | why ignored | lifted when |
|---|---|---|---|
| `agent/tests/linux_sandbox_smoke.rs:70` | `filesystem_no_new_privs_and_exit_status` | needs a native Linux host with a working system `bwrap` | the suite runs on a Linux box with `bwrap` installed (`--ignored`) |
| `agent/tests/linux_sandbox_smoke.rs:97` | `unreadable_mount_and_fd_allowlist_are_enforced` | same | same |
| `agent/tests/linux_sandbox_smoke.rs:128` | `command_signal_is_preserved` | same | same |
| `agent/tests/linux_sandbox_smoke.rs:145` | `helper_parent_death_does_not_leave_command_running` | same | same |
| `agent/tests/linux_sandbox_smoke.rs:216` | `omitted_missing_guard_created_by_command_is_reported_detection_only` | same | same |
| `agent/tests/linux_sandbox_smoke.rs:264` | `glob_rescan_failure_preserves_the_completed_command_status` | same | same |
| `agent/tests/linux_sandbox_smoke.rs:295` | `missing_scan_reports_partial_failure_after_unterminated_command_output` | same | same |
| `agent/tests/linux_sandbox_smoke.rs:337` | `production_plan_with_real_default_rules_starts_a_shell` | same | same |
| `agent/tests/linux_sandbox_smoke.rs:393` | `removing_existing_env_reports_a_busy_protection_mount` | same | same |
| `agent/tests/linux_sandbox_smoke.rs:430` | `command_cannot_write_or_forge_private_helper_report` | same | same |
| `agent/tests/linux_sandbox_smoke.rs:483` | `production_request_fd_transport_reaches_both_helper_phases` | same | same |
| `agent/tests/sandbox_smoke.rs:49` | `basic_shell_works` | runs real `sandbox-exec`; macOS only | run with `--ignored` on macOS |
| `agent/tests/sandbox_smoke.rs:62` | `workspace_and_tmp_writes_succeed` | same | same |
| `agent/tests/sandbox_smoke.rs:80` | `write_outside_roots_is_denied` | same | same |
| `agent/tests/sandbox_smoke.rs:101` | `credential_paths_are_unreadable` | same | same |
| `agent/tests/sandbox_smoke.rs:143` | `network_is_open_in_v2` | same | same |
| `agent/tests/sandbox_smoke.rs:175` | `git_operations_work_in_workspace` | same | same |
| `agent/tests/sandbox_smoke.rs:192` | `interpreters_and_devices_work` | same | same |
| `agent/tests/sandbox_smoke.rs:215` | `workspace_secret_and_rule_file_writes_are_denied` | same | same |
| `agent/tests/sandbox_smoke.rs:245` | `cargo_check_works_in_sandbox` | same | same |
| `agent/src/sandbox/linux/glob_scan.rs:506` | `large_workspace_exceeds_old_node_limit_without_failing` | creates >100 000 entries; large-workspace acceptance, too slow for the default run | a dedicated acceptance/perf job, or locally with `--ignored` |
| `channels/tests/agent_integration_test.rs:110` | `test_agent_prompt_flow` | requires a running agent | run against a live agent |
| `channels/tests/agent_integration_test.rs:235` | `test_old_session_prompt_flow` | requires a running agent | run against a live agent |
| `desktop/src-tauri/src/remote_host/sync_measurement.rs:584` | `serve_real_snapshot` | requires an isolated DB snapshot; driven by `scripts/measure-sync-browser.py` | a measurement run with the script |
| `desktop/src-tauri/src/remote/publisher/coalesce.rs:891` | `measure_real_journal` | driven by `scripts/measure-live-lane.py` with a real journal | a measurement run with the script |
| `desktop/src-tauri/src/remote/publisher/coalesce.rs:1031` | `measure_real_journal_lean` | needs `SYNC_MEASURE_JOURNAL` / `SESSION` / `RUN` in the env | a measurement run with those vars |

### Why the raw marker count is 28, not 26

Two of the 28 matched lines are **prose, not attributes**:

* `agent/tests/sandbox_smoke.rs:6` — the module doc comment
  (`//! … marked `#[ignore]` so they run …`).
* `desktop/src-tauri/src/remote/publisher/coalesce.rs:1142` — a doc comment on
  the synthetic-journal harness ("The two harnesses above are `#[ignore]`d …").

28 markers − 2 prose mentions = 26 ignored test functions, matching the name-set
diff in check 2.

### The two reworded reasons (not new disabled tests)

`git diff origin/main` for the two desktop files shows the reason strings moved
with the measurement scripts, while the ignored test functions themselves are
unchanged:

* `desktop/src-tauri/src/remote_host/sync_measurement.rs:584`:
  `#[ignore = "requires isolated DB snapshot; run scripts/measure/measure-sync-browser.py"]`
  → `… run scripts/measure-sync-browser.py"]`.
* `desktop/src-tauri/src/remote/publisher/coalesce.rs:891`:
  `#[ignore = "driven by scripts/measure/measure-live-lane.py with a real journal"]`
  → `… scripts/measure-live-lane.py with a real journal"]`.

Both files also gained **new, non-ignored** tests on this branch (e.g.
`the_measurement_harnesses_hold_over_a_synthetic_journal`,
`the_window_override_is_what_decides_a_merge`), which is the intended direction:
the measurement-only harnesses stay `#[ignore]`d while a synthetic-journal
counterpart is runnable.

## Independent re-verification (2026-09-26, by the verification pass)

The claims above were re-derived from scratch rather than trusted.

### 1. The ignored-test set — re-derived, not read

A fresh scan of every `.rs/.ts/.tsx/.py` under
`agent/ channels/ cli/ orchestration/ packages/ tui/ desktop/ mobile/`
(`node_modules` excluded), collecting the `fn` name that follows each `#[ignore]`
and diffing the **set** against `git show origin/main:<path>`:

```
ignored test fns now: 26   on origin/main: 26
ADDED:            0
REMOVED/RENAMED:  0
files with #[ignore] now:
  agent/src/sandbox/linux/glob_scan.rs
  agent/tests/linux_sandbox_smoke.rs
  agent/tests/sandbox_smoke.rs
  channels/tests/agent_integration_test.rs
  desktop/src-tauri/src/remote/publisher/coalesce.rs
  desktop/src-tauri/src/remote_host/sync_measurement.rs
```

That reproduces this document's headline claim independently: **the branch adds
no disabled test**, and none was removed either.

### 2. The 26 inventory anchors — every row resolves

Each row was re-checked mechanically: the named line must be the `#[ignore]`
attribute and the line directly after it must declare the named `fn`.

```
disabled-doc rows checked: 26   anchor mismatches: 0
```

(Convention note: a row's number is the **attribute** line, so the `fn` sits one
line below — e.g. `linux_sandbox_smoke.rs:70` is
`#[ignore = "requires a native Linux host with a working system bwrap"]` with the
test on `:71`. Unlike its sibling `weak-test-audit.md`, **no row here has
drifted**.)

## The platform red line, re-measured on Windows

"Disabled" has a second form this audit can also answer: a test that is not
`#[ignore]`d but is silently gated out by `#[cfg(unix)]`. The goal's Windows red
line is that those tests must now **compile and run** on Windows instead of being
skipped. Re-measured on this Windows host (private target dir, `-j 3`,
`RUST_TEST_THREADS=4`):

| command | result |
|---|---|
| `cargo test -p future-channel --lib` | **1451 passed; 0 failed; 0 ignored** (127.57 s) |
| `cargo test -p future-tui --lib` | **2122 passed; 0 failed; 0 ignored** (90.86 s) |

Both ran against the goal's working tree at `HEAD = c173e64e` (the goal's work is
uncommitted). `future-channel` has no live worker owning it. For `future-tui`,
**`w-tui` had a live run at the time**, so the 2122/0 figure describes an
in-flight revision — it is evidence that the suite passes on Windows at that
revision, not a claim about a frozen tree.

Caveat that matters: `--lib` covers a crate's unit targets only, so these are
**skip-checks**, not coverage measurements.

## Verification status / preconditions (2026-09-26)

The pass that produced the two sections above ran in a checkout where the
*freeze* preconditions for measurement-style work are **not** met:

* live worker runs: `w-ag-llm`, `w-audit2`, `w-desk-a`, `w-desk-b`,
  `w-mobile2`, `w-tau-rem2`, `w-tui`;
* open module todos under `w-tui` (`future-tui`), `w-ag-llm` (`ag-llm`),
  `w-tau-rem2` (`tau-*`), `w-mobile2` (`mobile`), `w-desk-a` / `w-desk-b`
  (desktop TS).

Consequently this pass produced **no** coverage reconciliation and ran **no**
`cargo-mutants` invocation: both would describe a moving tree. What it did
produce — the diff-level anti-pattern sweep, the ignored-test and assertion
audits, and the Windows re-runs above — is diff- and source-level, so each
section is valid for the revision it names. The full anti-pattern sweep (deleted
production lines, guard deletion, `#[cfg(test)]` seams, weakened assertions) is
in `docs/testing/weak-test-audit.md`.

## Statement

**No test was disabled in order to reach a coverage or green-suite target.**
The 26 ignored tests are all pre-existing on `origin/main`, are reproducible by
the reason string attached to each (platform, external service, or measurement
harness), and none of them was disabled by this goal. The branch's only
`#[ignore]`-adjacent change is the script-path rewording above.
