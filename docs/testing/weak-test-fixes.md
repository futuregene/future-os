# Weak-test fixes — the 真弱 rows of `docs/testing/weak-test-audit.md`

Owner: the weak-test-fix task (`verify.py weak`). This file records what each
**真弱 (real weak)** row of the audit received. The audit's own rows were left as
found during the first pass; after the `is_approved_outside_path` decision
recorded below, the three rpc rows were also closed in
`docs/testing/weak-test-audit.md` as `真弱 → 已修`.

Classification, test names and file paths come from the audit; the assertions
below are new in this change. **One production line changed, and only its
visibility**: `fn is_approved_outside_path` → `pub(crate) fn` (no signature
change, no behaviour change, no new crate-public API). No test was deleted, no
`#[ignore]` was added, and no existing assertion was weakened.

## Line-number drift

Every edit added lines, so rows below the first edit in a file moved. The test
name is the stable key; the numbers here are the **post-fix** anchors (the audit
quotes the pre-fix ones).

| audit anchor (pre-fix) | post-fix line | test |
|---|---|---|
| `agent/src/models/mod.rs:1771` | 1771 | `model_accepts_images_returns_bool` |
| `agent/src/models/mod.rs:1844` | 1880 | `get_default_model_returns_something` |
| `agent/src/models/mod.rs:1934` | 2028 | `registry_resolve_scope_with_star` |
| `agent/src/tools/mod.rs:2953` | 2953 | `approve_outside_path_adds_to_approved_list` |
| `agent/src/sandbox/mod.rs:1458` | 1458 | `legacy_bash_probe_runs_real_bash_version_check` |
| `agent/src/sandbox/mod.rs:2049` | 2061 | `hydrate_skips_when_shell_spawn_fails` |
| `agent/src/sandbox/mod.rs:2064` | 2085 | `hydrate_ignores_dump_without_marker` |
| `agent/src/sandbox/mod.rs:2080` | 2109 | `hydrate_times_out_and_kills_hung_shell` |
| `channels/src/providers/cli.rs:342` | 381 | `the_pump_stops_when_the_consumer_goes_away` |
| `agent/src/rpc/prompt_helpers.rs:782` | 797 | `approve_tool_path_write_and_edit` |
| `agent/src/rpc/prompt_helpers.rs:797` | 836 | `approve_tool_path_other_tools_noop` |
| `agent/src/rpc/prompt_helpers.rs:808` | 871 | `approve_tool_path_no_path_field` |

## Fixed rows (13 of 13)

`file:line | test | assertion added | branch/line it pins`

| file:line | test | assertion added | branch covered |
|---|---|---|---|
| `agent/src/models/mod.rs:1771` | `model_accepts_images_returns_bool` | Builds a `Registry` fixture (`cov100-vision` with `input=["text","image"]`, `cov100-text-only`, credentialed via an inline `AuthStore`-free catalog) and asserts `model_accepts_images_with` is **true** for the bare id **and** the `provider/id` form, **false** for the text-only model, **false** for `cov100-no-such-model-xyz`; keeps the catalog-backed `assert!(model_accepts_images("gpt-4o"))`. | Both arms of `model_accepts_images_with` (`agent/src/models/mod.rs:290`): the `.map(|m| m.input.iter().any(…))` arm and the `unwrap_or(false)` miss arm. Fails if either is inverted, and if the `input` list stops being consulted. |
| `agent/src/models/mod.rs:1880` | `get_default_model_returns_something` | (a) If the global entry point names a model, `assert!(Registry::new().resolve(&id).is_some())` — a stale or malformed id now fails. (b) Hermetic `get_default_model_with(&registry)` fixtures: the first **credentialed text** model wins while an uncredentialed earlier entry and a credentialed image-only entry lose; (c) with `auth["future"]` present, the fixture asserts the `future/deepseek-v4-pro` short-circuit wins **over an earlier credentialed text model**. | `get_default_model_with` (`:369`): the credential+text filter, the `auth.get("future").is_some()` preferred arm, and the `preferred.or_else(…)` fallback arm. (c) fails if the short-circuit is removed; (b) fails if the credential filter is dropped (it would then return `cov100-unconfigured/cov100-text`). |
| `agent/src/models/mod.rs:2028` | `registry_resolve_scope_with_star` | Fixture registry + `AuthStore::from_json` fixture. Asserts `resolve_scope(["*"])` and `resolve_scope(["<provider>"])` return exactly the two credentialed ids **in catalog order**, and that a non-matching pattern yields an **empty** vec (not the whole catalog). | `resolve_scope` (`:1115`): the `retain` credential filter, the `glob_match` hit arm for `*`, the provider-name pattern arm, and the no-match arm. Fails if the credential filter is dropped (extra id) or if a miss returned everything. |
| `agent/src/tools/mod.rs:2953` | `approve_outside_path_adds_to_approved_list` | Now `#[tokio::test]`. Outside any scope: `approve_outside_path` must not become visible (`!is_approved_outside_path`). Inside `with_tool_scope`: empty at scope start → `true` after approving → still `false` for an unrelated path → a `cov100-sub/../file` spelling resolves to the same approved path. | `approve_outside_path` (`:117`), the `TOOL_SCOPE.try_with` `unwrap_or(false)` arm (`:1695`), and the `canonicalize_lenient` normalization at `:120`. The last assertion fails if the raw spelling is stored instead of the normalized path. |
| `agent/src/sandbox/mod.rs:1458` | `legacy_bash_probe_runs_real_bash_version_check` | `assert!(!legacy_bash_probe("sh"))` and `assert!(!legacy_bash_probe("/bin/zsh"))` (the name gate, no spawn), plus `assert!(legacy || on_path("bash"))` on the host-dependent call. | The `name == "bash" && …` short-circuit (`:750`) and the `unwrap_or(true)` conservative fallback: a `false` verdict is only reachable through a probe that really ran. |
| `agent/src/sandbox/mod.rs:2061` | `hydrate_skips_when_shell_spawn_fails` | Snapshots `std::env::var_os("PATH")` before `hydrate_from_login_shell()` and `assert_eq!` after. | The spawn-failure early `return` at `:813` (before `plan_env_merge` / `set_var`). It now fails if a failed spawn still rewrites PATH. |
| `agent/src/sandbox/mod.rs:2085` | `hydrate_ignores_dump_without_marker` | Same PATH before/after `assert_eq!` around the marker-less dump. | `plan_env_merge` returning `(None, vec![])` (`:848` `if let Some(value) = path` not taken). Fails if a marker-less dump gets applied. |
| `agent/src/sandbox/mod.rs:2109` | `hydrate_times_out_and_kills_hung_shell` | Same PATH before/after `assert_eq!`, **plus** `assert!(started.elapsed() < 20s)` on a shell that would otherwise hold stdout open for 30 s. | The `rx.recv_timeout` timeout arm (`:836-838`). The elapsed bound converts "the timeout arm was deleted" from an unbounded hang into a clean failure. |
| `channels/src/providers/cli.rs:381` | `the_pump_stops_when_the_consumer_goes_away` | Replaced `Cursor::new(b"one\ntwo\n")` with an `EndlessLines` `BufRead` fixture that never reports EOF, counts `read_line` calls and errors after a 1000-line budget; then `assert_eq!(reads, 1)`. A second, live-consumer run with `budget: 3` pins the fixture itself (`reads == 4`, first line received), so `reads == 1` can only mean "the pump stopped", never "the reader stopped early". | The closed-consumer arm of `pump_lines` (`channels/src/providers/cli.rs:93`). With EOF-fed input the old test reached `:91` regardless, so this is the first assertion that distinguishes the two exits. A pump that ignores the closed consumer records >1 reads and fails the assertion instead of hanging. |
| `agent/src/rpc/prompt_helpers.rs:797` | `approve_tool_path_write_and_edit` | Now `#[tokio::test]` inside `with_tool_scope`: the fresh scope is empty; `write` with `{"path":"test.txt"}` approves the workspace's `test.txt` (asserted by concrete path); then a **different** file (`edited.txt`) is asserted unapproved and `edit` approves it, so a `write`-only guard fails. Needed `is_approved_outside_path` (`agent/src/tools/mod.rs:1686`) widened to `pub(crate)` (visibility only). | `approve_tool_path_if_present`'s `write|edit` arm and its `approve_outside_path` call (`prompt_helpers.rs:424`); it fails if the wrapper drops the approval, resolves the wrong path, or narrows the tool set. Paired `!X` before / `X` after in the same scope prove the predicate is not vacuously false. |
| `agent/src/rpc/prompt_helpers.rs:836` | `approve_tool_path_other_tools_noop` | Inside `with_tool_scope`: `read` (with a `path` argument) and `shell` leave the approved list empty; a control `write` in the same scope records the same path. | The tool-name early `return` at `agent/src/rpc/prompt_helpers.rs:416` (the `read`/`shell` leg). Fails if the guard is widened to those tools; the control makes the emptiness attributable to the guard rather than to a dead list. |
| `agent/src/rpc/prompt_helpers.rs:871` | `approve_tool_path_no_path_field` | Inside `with_tool_scope`: `write` with `{}` and with a present-but-non-string `{"path":42}` leaves the list empty; the control `write` with a string path records it. | The `let Some(path) = super::argument_path(arguments) else { return; }` early return at `agent/src/rpc/prompt_helpers.rs:420`, including `as_str()`'s `None` for a non-string value. |

## The three `agent/src/rpc/prompt_helpers.rs` rows: how they were decided

## The three `agent/src/rpc/prompt_helpers.rs` rows: decision and outcome

`approve_tool_path_write_and_edit`, `approve_tool_path_other_tools_noop` and
`approve_tool_path_no_path_field` were the only rows where the audit's suggested
fix needed a production change, so they were left as-is and escalated. The
evidence for the escalation:

* The only effect of the subject is `crate::tools::approve_outside_path(...)`
  (`agent/src/rpc/prompt_helpers.rs:424`), which pushes into the **task-local**
  `TOOL_SCOPE` and is a silent no-op outside a scope (`agent/src/tools/mod.rs:121`).
* The only reader was `fn is_approved_outside_path` — **private** to
  `agent/src/tools/mod.rs:1686`; a crate-wide grep finds exactly two references,
  both inside `tools/mod.rs` (`:1660` consumer, `:1686` definition). Its other
  consumer, `ensure_workspace_access` (`:1650`), and the handlers
  (`write_handler` `:382`, `edit_handler` `:434`) are private too.
* `agent/src/rpc/prompt_helpers.rs` could not reach any of them, so **no
  assertion from that file could observe the effect**; merely asserting "does
  not panic" is exactly the weak test being replaced.

**Decision (supervisor, 2026-09-26): option 1.** Widen `fn
is_approved_outside_path` to `pub(crate)` — visibility only, signature and
behaviour untouched, and `agent/src/tools/mod.rs` is inside the `future-agent`
crate so the crate's public API is unchanged (same precedent as the
`tau-terminal` `on_data`/`on_eof` widening) — then make the three tests assert
the approved list inside `with_tool_scope`. Option 2 (re-classifying them as
合理豁免 without assertions) was rejected: three tests that still assert nothing
is what this goal exists to remove, and the wrapper-level assertions are
real — they catch "the wrapper passed the wrong tool name / dropped the path".

Outcome: all three now assert observable state (see the "Fixed rows" table
above), and a single deliberate mutation of the wrapper's tool-name guard
(`matches!(tool_name, "write" | "edit")` → `matches!(tool_name, "read" |
"edit")`) makes **all three fail**, each with its own message — so none of them
can pass by accident. The mutation was reverted immediately.

## The last row: fixed by a successor task

* `tui/src/components/chat_area.rs:2611` `tiny_terminal_width_does_not_underflow`
  — **FIXED** once the tui module stopped changing, by the follow-up task (`todo_43d631e0363d`, agent `w-weak`). The assertion is no longer 'does not panic':
  the test is table-driven over widths `[0,1,2,3,4,40,1000]`, asserts both renders are
  non-empty, asserts the viewport is a genuine **window of the transcript**
  (`all.windows(view.len()).any(|w| w == view)` — an underflowed `viewport_top` would
  slice out of range and fail), asserts every row's visible width is `<= width.max(3)`
  cells, and asserts the 1-column gutter survives.

## Verification performed

| check | command | result |
|---|---|---|
| weak-test gate | `python .future/cov100/verify.py weak` | **PASS**, `assertion-free Rust tests: 56` (down from 70 — the 9 fixed rows are gone from the scanner output; the 3 unresolved rpc rows and the skipped `tui` row remain and are still `[ok]` because `docs/testing/weak-test-audit.md` names their file). |
| audit gate | `python .future/cov100/verify.py audit` | **PASS** (`assertion-free Rust tests: 56`; `disabled markers found: 28; files referenced by the audit doc: 6`). No `#[ignore]` was added by this change. |
| Windows-runnable new tests | `cargo test -p future-agent --lib -j 3 -- models::tests::model_accepts_images models::tests::get_default_model_returns_something models::tests::registry_resolve_scope_with_star tools::tests::approve_outside_path_adds_to_approved_list` | 5 passed, 0 failed (incl. the untouched sibling `model_accepts_images_unknown_returns_false`) |
| module-level regression | `cargo test -p future-agent --lib -j 3 -- models::tests tools::tests` | 137 passed, 0 failed, 1885 filtered out |
| " | `cargo test -p future-channel --lib -j 3 -- providers::cli::tests` | 13 passed, 0 failed |
| formatting | `rustfmt --edition 2021 --check` on the four changed files | clean (exit 0) |
| non-Windows test bodies | **could not be cross-compiled**: `cargo check --target x86_64-unknown-linux-gnu -p future-agent --tests` fails in the `ring` build script (`failed to find tool "x86_64-linux-gnu-gcc"`, no Linux C toolchain on this host, and `ring` is a transitive dependency). Substitute: the four `cfg(not(target_os = "windows"))` bodies were copied verbatim into a single-file scratch crate that stubs `legacy_bash_probe`, `on_path`, `hydrate_from_login_shell`, `ShellEnvGuard`, `fake_shell` and `hydrate_test_lock` with the **same signatures**, then checked with `rustc --emit=metadata` (`exit 0`) and `clippy-driver -W clippy::all` (`exit 0`). The scratch file lived under `%TEMP%` and was deleted. This proves the added statements are type- and lint-clean; it does **not** prove the surrounding non-Windows code compiles, so the three hydrate rows still need the first Linux run to confirm. |

The three `#[cfg(not(target_os = "windows"))]` hydrate assertions do not run on
this Windows host. They are the only part of this change that is not executed
by the local run; the Linux CI job (`cargo test -p future-agent`) is the first
place they actually run.

## Verification — second pass (the three rpc rows)

| check | command | result |
|---|---|---|
| targeted tests | `cargo test -p future-agent --lib -j 3 -- rpc::prompt_helpers::tests` | 21 passed, 0 failed, 2007 filtered out |
| mutation check (do the new assertions bite?) | guard at `agent/src/rpc/prompt_helpers.rs:416` changed `"write" \| "edit"` → `"read" \| "edit"`, then the command above | **3 failed / 0 passed**, each by its own message: `..._write_and_edit` → "write must approve … for the agent"; `..._other_tools_noop` → "read/shell must not approve …"; `..._no_path_field` → "the control write must be recorded in the same scope". Mutation reverted immediately; re-run after revert: 21 passed. |
| weak-test gate | `python .future/cov100/verify.py weak` | **PASS**, `assertion-free Rust tests: 58` (61 → 58; no `agent/src/rpc/` entry remains in the scanner output) |
| formatting | `rustfmt --edition 2021 --check agent/src/rpc/prompt_helpers.rs agent/src/tools/mod.rs` | clean (exit 0) |
| shared-checkout note | the mutation run's first attempt failed to compile | `agent/src/agent/run_loop.rs:5739` (`MutexGuard<CostSplit>` has no `total()`) — another worker mid-edit; a re-run 75 s later compiled and ran. Nothing outside this task's scope was touched. |

## Known limitations accepted rather than asserted

* `hydrate_times_out_and_kills_hung_shell`: the audit's `rev` note stands — the
  test has no handle on the child (`hydrate_from_login_shell` owns it), so "the
  hung shell was actually killed" cannot be observed from the test. What is now
  asserted is that the timeout arm returned before applying env *and* before the
  30 s the fake shell would keep stdout open for. A seam would be needed for the
  kill itself.
* `legacy_bash_probe_runs_real_bash_version_check`: the real-bash verdict stays
  host-dependent (bash 3 vs bash 4+); only the name gate and the
  "false ⇒ bash exists" invariant are asserted, which is what the audit asked
  for.
