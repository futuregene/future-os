# Mutation report

Shared deliverable. Each mutation finding gets its **own section**, written by the
worker that owns it, and is **appended** — never rewrite or reorder another
item's section. The machine-readable roll-up is `mutation/summary.json`
(fields `caught`, `missed`, `uncaught[]`), which belongs to the mutation job, not
to the individual finding tasks.

Raw per-mutant artifacts live under `mutation/` (gitignored): `out-policy/`,
`timing-probe/`.

---

## channels/src/policy.rs — the three unpinned access-policy defaults

*Item `todo_8dcc25abfda1` · agent `w-chan3` · file frozen by its owner (`w-chan2`)*

**Evidence tokens:** `lines-100-or-waived`, `dimensions`, `weak-tests-fixed`.

### 1. The finding

`channels/src/policy.rs` reports **100.00 % line coverage (196/196)**. `cargo
mutants` nonetheless left defaults alive:

| mutant | recorded verdict (out-policy) | recorded verdict (timing-probe) | true verdict (measured here) |
|---|---|---|---|
| `policy.rs:52:5` `default_dm_policy -> String` = `"xyzzy".into()` | MISSED | — | **missed** before, caught after |
| `policy.rs:52:5` `default_dm_policy -> String` = `String::new()` | caught | — | **missed** before, caught after |
| `policy.rs:56:5` `default_group_policy -> String` = `String::new()` | MISSED | — | **missed** before, caught after |
| `policy.rs:56:5` `default_group_policy -> String` = `"xyzzy".into()` | caught | — | **missed** before, caught after |
| `policy.rs:60:5` `default_require_mention -> bool` = `false` | caught | **MISSED** | **missed** before, caught after |

The real values are `default_dm_policy() == "allowlist"`,
`default_group_policy() == "disabled"`, `default_require_mention() == true` — the
**safe** posture for a channel that is configured without a policy block.

### 2. Why these are genuine test gaps, not equivalent mutants

Every one of the four mutations changes the *effective* default policy, and two
of them change observable behaviour:

* DM default `"allowlist"` → `"open"`: an unconfigured channel answers every DM.
* DM default `"allowlist"` → `"disabled"`: strangers get the silent refusal
  instead of the "here is your id, ask the admin" refusal.
* Group default `"disabled"` → `"open"` / `"allowlist"` / any unknown string:
  a group chat stops being off by default and the refusal reason changes from
  `Group chat is disabled` to `This group (...) is not in the group_allowlist`.
* `require_mention` default `true` → `false`: an explicitly enabled group no
  longer needs the bot to be addressed.

The `"xyzzy"` / `String::new()` mutations happen to land on the `_` match arm
(the allowlist arm) and therefore keep *denying* an unconfigured stranger — so a
behaviour-only assertion cannot see them. That is exactly why the literals are
pinned as well; see §5 for the honest limits.

### 3. What was added

**Tests only. No production change.** `git diff HEAD -- channels/src/policy.rs`
is a single insertion-only hunk: `@@ -349,0 +350,108 @@ mod tests {`
(`cargo fmt -p future-channel --check` is clean).

| new test | mutant(s) it makes fail |
|---|---|
| `default_dm_policy_is_the_allowlist_safe_default` | 52:5 (`"xyzzy"`, `String::new()`, `"open"`, `"disabled"`) |
| `default_group_policy_is_disabled` | 56:5 (`String::new()`, `"xyzzy"`, `"open"`, `"allowlist"`) |
| `default_require_mention_is_true` | 60:5 (`false`) |
| `an_empty_config_object_carries_the_safe_defaults_through_serde` | all three, through `#[serde(default = …)]` |
| `a_partial_config_only_overrides_the_policy_keys_it_names` | group + require_mention defaults, plus round-trip |
| `an_unconfigured_engine_refuses_a_stranger_in_dm` | DM default → `open` / `disabled` (behaviour) |
| `an_unconfigured_engine_has_group_chats_disabled` | group default → `open` / allowlist / `""` / `"xyzzy"`, and require_mention → `false` (behaviour) |

Dimensions added: **boundary** (empty config object `{}`), **serialization**
(serde default wiring, partial config, decode→encode→decode round-trip),
**property/invariant** ("an unconfigured channel is never reachable by a
stranger"), **error-path** (the exact refusal reason and its embedded sender id).
Concurrency / platform-cfg are `N/A` here: the file is pure synchronous
configuration matching with no IO, no locks and no platform branches.

### 4. Verification — measured, not argued

Command (private target dir; module-scoped, as the plan requires):

```
CARGO_TARGET_DIR=target/cov-w-chan3 cargo test -p future-channel --lib -j 3 -- policy
```

1. **Baseline** (before any edit): 19 pass.
2. **Before / gap proof** — all three defaults mutated at once
   (`dm→"xyzzy"`, `group→String::new()`, `require_mention→false`): **19 passed,
   0 failed**. The suite was blind to all three simultaneously. This is the
   deterministic version of the cargo-mutants MISSED verdicts, and it also shows
   the `60:5` "caught" verdict in `out-policy` was not real.
3. **After** — same three mutations, with the new tests in place: **6 failed,
   attribution visible in each panic**:
   * `default_dm_policy_is_the_allowlist_safe_default`: ``left: "xyzzy"``
   * `default_group_policy_is_disabled`: ``left: ""``
   * `default_require_mention_is_true`: `assertion failed: default_require_mention()`
   * `an_empty_config_object_carries_the_safe_defaults_through_serde`: ``left: "xyzzy"``
   * `a_partial_config_only_overrides_the_policy_keys_it_names`
   * `an_unconfigured_engine_has_group_chats_disabled`:
     ``Denied("This group (oc_unconfigured) is not in the group_allowlist")``
     instead of `Denied("Group chat is disabled")` — i.e. the mutation really did
     change the effective policy, and the *behavioural* assertion caught it.
4. **Behaviour test has independent power** — mutating only the DM default to
   `"open"` (a change the literal pin also catches) makes
   `an_unconfigured_engine_refuses_a_stranger_in_dm` fail with
   `an unconfigured channel must refuse a stranger, got Allowed`.
5. **Restored**: `git diff` shows one insertion-only hunk inside `mod tests`;
   `cargo test -p future-channel --lib -j 3` → **1458 passed, 0 failed**
   (1451 before + 7 new, on the final formatted bytes).
   `python .future/cov100/verify.py audit` → **PASS**; `cargo fmt -p
   future-channel --check` → clean.

The three temporary production mutations were reverted; nothing was committed.

### 5. Honest limits of this fix

* `an_unconfigured_engine_refuses_a_stranger_in_dm` **cannot** distinguish
  `"allowlist"` from `"xyzzy"`/`String::new()` — all three take the same `_` arm
  and deny an empty allowlist. Only the literal pin in
  `default_dm_policy_is_the_allowlist_safe_default` (and the serde twin) catches
  those. Stated here so the pair is not mistaken for redundancy.
* The behavioural half of `default_require_mention` only fires through the
  per-chat-enable path, because the default group policy is `disabled`: with the
  default config, a group is refused before the mention gate is consulted.
* policy.rs line coverage was **not** re-measured here (that is the supervisor's
  whole-workspace run). The production region is byte-identical to `HEAD`, so the
  recorded 196/196 is unaffected; the added lines are test-module lines that the
  new tests execute.

### 6. Defects found in the mutation harness itself (for the mutation owner)

These make **`mutation/out-policy/outcomes.json` unusable as a detection score**.
They are read from the artifacts; §6.4 is the one I reproduced by experiment.

1. **Labels contradict their own logs.** A `CaughtMutant` must have a failing test
   run, yet:
   * `log/channels__src__policy.rs_line_52_col_5_001.log` (52:5 `String::new()`,
     recorded **caught**) ends `test result: ok. 1451 passed; 0 failed`.
   * `log/channels__src__policy.rs_line_56_col_5.log` (56:5 `"xyzzy"`, recorded
     **caught**) ends `test result: ok. 1451 passed; 0 failed`.
   * `log/channels__src__policy.rs_line_52_col_5.log` (52:5 `"xyzzy"`, recorded
     **missed**) ends `FAILED. 1450 passed; 1 failed`.
   So at least three of the eight verdicts are inverted relative to their logs.
2. **Every red run in the set is an unrelated flake.** Across all six "caught"
   mutants the only failing tests are
   `providers::telegram::tests::the_webhook_serves_verified_deliveries_and_rejects_the_rest`
   (`Err(webhook server cannot bind 127.0.0.1:18787 … os error 10048)` — the
   Windows `WSAEADDRINUSE` port collision the plan already flags as a pre-existing
   channel failure),
   `providers::telegram::tests::a_webhook_configured_with_explicit_addr_and_path_binds_there`,
   and `transport::ws::tests::a_healthy_connection_resets_the_backoff_before_the_next_failure`.
   No policy assertion ever failed. **No policy mutant was genuinely caught.**
3. **The same mutant got two different verdicts in two runs**: `60:5`
   `default_require_mention -> false` = `CaughtMutant` in `out-policy`,
   `MissedMutant` in `timing-probe` (whose baseline log is clean,
   `1451 passed; 0 failed`).
4. **The per-mutant logs do not belong to their mutants.** I reproduced
   `channels__src__policy.rs_line_91_col_13` (delete match arm `"open"` in
   `check_dm`) by hand: with that arm deleted,
   `policy::tests::dm_open_allows_anyone` **fails** (`channels/src/policy.rs:207`).
   The log named after that exact mutant records
   `test policy::tests::dm_open_allows_anyone ... ok`, with the only failure being
   the Telegram port collision. A log that disagrees with the mutant it is named
   after cannot be used to score anything.
5. **The run is incomplete and internally inconsistent.** `outcomes.json` has
   `end_time: null` and `success: 0`; `unviable.txt` is empty even though
   `90:9` (`replace PolicyEngine::check_dm -> Access with Default::default()`)
   cannot build — `Access` derives `Debug, Clone, PartialEq, Eq` and the crate has
   no `impl Default for Access` — and four mutants (`112:9`, `119:13`, `96:21`,
   `96:68`) have diff files but **empty logs** (never run).
   (Point 5 is read from the sources, not reproduced by a build.)

**Consequence for the gate.** Scoring `mutation/summary.json` from
`out-policy/outcomes.json` as it stands would report 6/8 = 75 % for this file and
75 %+ elsewhere, mostly from port collisions. The honest numbers for these eight
mutants are **0/8 genuinely caught before this change** and, for the five
`default_*` mutants, **5/5 caught after it** (verified in §4/§3). A re-run must
either quarantine the flaky Telegram-webhook / ws-timing tests (the environment
class the plan already waives) or run with `--test-threads` low enough to stop
the port collisions, and it must be re-measured on the post-fix commit — the
recorded `out-policy` verdicts predate this fix and must not be carried forward.

### 7. Durable artifacts

* `mutation/policy-defaults-finding.md` — the extracted raw evidence for §1–§6
  (verdict/log cross-reference, verbatim failing-test lines), kept so it survives
  a rewrite of this shared file.
* `mutation/summary.json` — **not** written by this item; it is the mutation
  job's roll-up and must not be synthesized from the contaminated verdicts above.

---

## The mutation roll-up — `mutation/summary.json`, its scope, and why the file needs one clean re-run

*Item `todo_c354929ff539` · agent `w-mut` · appended 2026-09-26 · raw artifacts under `mutation/`*

**Evidence:** `independent-recheck`, `mutation-summary`. Method: `cargo-mutants`
**27.1.0 in its default copy mode — `--in-place` was never used on any run**
(a scratch copy per job; `--gitignore true` so gitignored trees such as
`desktop/src-tauri/target/` are not copied), test scope
`cargo test -p future-channel --lib` (1451 unit tests; `tests/*.rs` were not run
per mutant).

### 1. Scope — partial, and stated as such

`cargo-mutants` lists **24** mutants for `channels/src/policy.rs`. Of the six
files the supervisor ordered (`policy.rs` 24, `bridge/queue.rs` 37, `compat.rs` 72,
`backfill.rs` 74, `quota/usage_summary.rs` 84, `windows_power.rs` skipped as not
frozen), **only `policy.rs` was attempted**, and within it **8 mutants were
executed against the final revision**. The remaining 16 were never run and are
listed by name in `summary.json` → `not_executed_in_sample`.

**No file-level, crate-level or repo-level mutation score exists as a result of
this item, and none may be quoted from it.** `future-loop` alone has 3301 mutants
(`cargo mutants --list -p future-loop`), measured here at ~90 s/mutant ⇒ ~6 h
serial, and parallel jobs do not buy that back (build-lock contention, below).
The 250-mutant budget was therefore unreachable on this host; partial-but-measured
was chosen over complete-but-invented.

### 2. The revision moved underneath the first run — the file it measured no longer exists on disk

| | mutated tree of run 1 (`mutation/out-policy/`) | file on disk now |
|---|---|---|
| `policy::tests::` count | 15 | 22 |
| `channels/src/policy.rs` mtime | copy taken 14:36 | **15:13:36** |

Run 1's `outcomes.json` is timestamped **15:12:55**; `channels/src/policy.rs` was
rewritten **41 s later**, at **15:13:36**, by the weak-test audit (w-chan3), whose
section above says exactly this: its verdicts "predate this fix and must not be
carried forward". The diff between the two revisions is a single
insertion-only hunk, `@@ -349,0 +350,108 @@ mod tests {` — test-only, production
bytes identical. `summary.json` therefore keys its evidence to
`sha256 9E746D4F…8DE42`, verified **identical before and after** the deciding run.

**Freeze precondition: not met for `channels/` at the time of run 1.** Per the
task's own instruction E, run 1's numbers are labelled reference-only and are not
used as acceptance evidence anywhere in `summary.json`.

### 3. Root cause of the harness defects in §6 above — a shared `CARGO_TARGET_DIR`

§6 of the w-chan3 section could show that labels disagreed with logs but not why.
The mechanism, reproduced here:

1. I passed `CARGO_TARGET_DIR=D:\cov100-mut-target` to `cargo-mutants -j 4`. Every
   parallel job then shares one target directory, and (because the copies are
   identical apart from the mutated byte) two jobs can link **the same output
   path**: `LNK1104: cannot open file …\deps\future_channel-18cc610b9df61479.exe`
   — the recorded "build failure" of mutant `92:13`, an infrastructure failure,
   not a property of the mutation.
2. The same collision lets one job's **test phase run another mutant's binary**.
   Mutant `91:13` (`delete match arm "open"`) recorded failures carrying mutant
   `60:5`'s signature — `assertion failed: default_require_mention()` at
   `policy.rs:377/389/404` — while `policy::tests::dm_open_allows_anyone`, the one
   test that mutation must break, **passed**. w-chan3's §6.4 hand-reproduction
   confirms the opposite when the arm really is deleted: that test fails. The two
   observations together prove the log belongs to a different binary.
3. That is also why run 1 could report `90:9` (`check_dm -> Default::default()`)
   as **caught** when it cannot compile at all: `error[E0277]: the trait bound
   `Access: Default` is not satisfied` (`Access` derives only `Debug, Clone,
   PartialEq, Eq`; there is no `impl Default for Access` in `channels/src`).
   On the clean run it is correctly **UNVIABLE**.

Correction to §6.1's "three verdicts are inverted relative to their logs":
cross-referencing `outcomes.json` (`scenario.Mutant.name` ↔ `log_path`) shows the
`_001` suffix is a dedup counter assigned in **run** order, not `--list` order
(`line_52_col_5.log` = `String::new()`/**caught**; `line_52_col_5_001.log` =
`"xyzzy"`/**missed**; and the mirror image at `56:5`). Read with the correct
mapping, each log agrees with its verdict. The defect is real but it is a **stale
test binary**, not an inverted label.

### 4. Measured results on the attested revision (8 mutants)

| # | mutant | verdict | witness test (causally correct?) |
|---|---|---|---|
| 1 | `52:5 default_dm_policy -> String::new()` | **CAUGHT** | `default_dm_policy_is_the_allowlist_safe_default` ✓ |
| 2 | `52:5 default_dm_policy -> "xyzzy".into()` | **CAUGHT** | same ✓ (‑ MISSED pre-fix) |
| 3 | `56:5 default_group_policy -> "xyzzy".into()` | **CAUGHT** | `default_group_policy_is_disabled` ✓ |
| 4 | `56:5 default_group_policy -> String::new()` | **CAUGHT** | same ✓ (‑ MISSED pre-fix) |
| 5 | `60:5 default_require_mention -> false` | **CAUGHT** | `default_require_mention_is_true` ✓ (‑ MISSED pre-fix) |
| 6 | `91:13 delete match arm "open" in check_dm` | **CAUGHT** | `dm_open_allows_anyone` ✓ — from w-chan3 §6.4's hand-reproduction, because this run's log for it is unusable |
| 7 | `90:9 check_dm -> Access with Default::default()` | **UNVIABLE** | n/a — does not compile (E0277) |
| 8 | `92:13 delete match arm "disabled" in check_dm` | **no verdict** | build died with LNK1104 (shared target dir) |

`mutation/summary.json`: `caught = 6`, `missed = 0`, over **6 viable evaluated
mutants**; `uncaught[]` carries #7 and #8 with their classifications, and
`not_executed_in_sample` lists the 16 never-run mutants.

**The headline result is a positive one:** the weak-test audit's seven added
assertions are *mutation-effective*. The five `default_*` mutants that survived the
pre-fix revision — independently observed here (`timing-probe/missed.txt` =
`60:5`, run 1 = `52:5 "xyzzy"`, `56:5 String::new()`) and by the audit itself
(`policy.rs:356-360`, w-chan3 §4.2's 19/19 green with all three mutated) — are now
killed, each by the very assertion that was added for it. All 108 added lines are
inside `mod tests`.

### 5. Uncaught-mutant classification (the required adjudication)

* **Non-viable, *not* a test gap** — `90:9`. `Access` has no `Default` impl, so the
  mutant cannot exist as a program. Excluded from the denominator by definition;
  no test can be censured for not killing it. Structural proof, not an argument.
* **No verdict, *not* a test gap** — `92:13`. The build failed on a linker-file
  collision caused by my own harness flags. Source-level witness exists
  (`dm_disabled_denies_even_allowlisted` uses `dm_policy="disabled"` and asserts
  `Denied`; deleting the arm falls through to the allowlist branch and returns
  `Allowed`), so a clean re-run is expected to report **caught**.
* **Never executed** — the 16 mutants listed in `summary.json`. Not classified
  here; the stale run's verdicts for them are discarded, not inherited.
* **No verified test gap was found in `channels/src/policy.rs` on the attested
  revision.**

### 6. Test-suite defect that corrupts any mutation score here

Six tests flake under load (standalone: `cargo test -p future-channel --lib`
failed 2 of 4 runs at 1450/1451):

`providers::telegram::tests::the_webhook_serves_verified_deliveries_and_rejects_the_rest`,
`…::a_webhook_configured_with_explicit_addr_and_path_binds_there`,
`transport::ws::tests::a_healthy_connection_resets_the_backoff_before_the_next_failure`,
`bridge::queue::tests::an_idle_conversation_is_evicted_and_its_worker_stops`,
`providers::email::tests::a_message_already_delivered_under_another_uid_is_marked_without_a_turn`,
`delivery::tests::the_queue_is_bounded_and_drops_finished_entries_first`.

They are **false positives against the control flow**: the Telegram-webhook test
failed in 17 of run 1's 24 logs — including all six mutants whose *only* failures
it supplied — although a webhook bind test cannot depend on which string
`default_dm_policy()` returns; and it failed for
`default_group_policy -> "xyzzy"` but not for `default_group_policy ->
String::new()` inside a single run. Until these are made deterministic, a
"caught" verdict backed only by one of them means nothing, and a run can fail its
baseline for no reason.

### 7. Cost model and the reproducible re-run

Measured: cold baseline build 239–260 s + 88–107 s test; ~**92 s/mutant** at `-j 4`
(24 mutants / 37 min), ~2 min/mutant at `-j 2`, ~10 min for a single full-suite
mutant from cold. `future-loop` = 3301 mutants ⇒ hours, not minutes.

To produce a number that can be quoted, re-run on a frozen revision with:

```
# 1. attest the revision, and make sure no other worker is writing the crate
git -C . status --short channels/src/policy.rs ; sha256sum channels/src/policy.rs
# 2. NO CARGO_TARGET_DIR  (this is the defect in §3) and -j 1 for attribution
cargo mutants -p future-channel --file channels/src/policy.rs --gitignore true \
  --timeout 400 -o mutation/out-clean-<sha> -- --lib
# 3. re-check the hash afterwards; accept a verdict only if a failing test can
#    plausibly observe the mutated line
```

Then, and only then, does `mutation/summary.json` acquire a `caught`/`missed`
pair that supports a percentage. The siblings still to do are
`channels/src/bridge/queue.rs` (37), `orchestration/loop/src/compat.rs` (72),
`backfill.rs` (74) and `quota/usage_summary.rs` (84) — 267 mutants, ≈7 h at the
measured rate on this host.

> **UPDATE (later the same day, item `todo_eb0e6400679f`) — read the two sections
> below before using any figure in this one.** `policy.rs` and `queue.rs` are both
> done, completely: **24/24 and 37/37 mutants executed**, 22 caught / 0 missed
> (policy) and 23 caught / 11 missed (queue), on hash-attested revisions, with the
> flake problem solved rather than tolerated. The "8 of 24" and "≈7 h" above are
> superseded. The three `loop/` files and the skipped `windows_power.rs` are still
> untouched — that part of the scope statement stands.

### 8. Artifacts for this item

* `mutation/summary.json` — the roll-up (schema `future-cov100-mutation-summary-v1`).
* `mutation/out-policy-recheck2/mutants.out/` — the deciding run on
  `sha256 9E746D4F…` (`caught.txt` 2, `unviable.txt` 1, `missed.txt` empty).
* `mutation/out-policy-recheck/mutants.out/` — the earlier recheck (interrupted at
  the time box; `caught.txt` 4, killed mid-flight after mutant `90`).
* `mutation/out-policy/mutants.out/` — run 1, **stale revision, reference only**.
* `mutation/timing-probe/mutants.out/missed.txt` — the pre-fix probe
  (`60:5` MISSED).

---

## `channels/src/bridge/queue.rs` — the second sampled file: 37 mutants, 23 killed, 11 survivors in one filter

*Item `todo_eb0e6400679f` · agent `w-mut` · appended 2026-09-26 · raw artifacts under `mutation/out-queue/`*

> **Follow-up — item `todo_50756baf3574`, 2026-09-26: 9 of the 11 survivors are
> now killed, and the tenth is re-analysed.** Three tests were added inside
> `mod tests` (production code untouched); the ten surviving mutants were re-run
> against them and the result is **9 caught, 1 missed**, the miss being the
> boundary-only `232:54 > -> >=`. The re-run and its per-mutant witnesses are in
> [§6](#6-the-remedy-applied-in-item-todo_50756baf3574-and-re-measured); §5.1,
> §5.3 and §5.4 carry the follow-up verdicts. Sections 1–5 below are the
> *original* measurement of 2026-09-26 and are left exactly as measured then;
> the new raw artifacts are `mutation/out-queue-check/`.

**Evidence:** `measured-reproduction`. Method: `cargo-mutants` 27.1.0 in its
**default copy mode — `--in-place` was never used** (`--gitignore true`, `-j 2`,
`--timeout 300`, and **no `CARGO_TARGET_DIR`**). Witness set, taken verbatim from
the run's own `outcomes.json`:

```
cargo test --verbose --package=future-channel@0.1.0 --lib bridge::
```

222 of the crate's 1458 lib tests run (`1236 filtered out`), and the unmutated
baseline passed **222 / 222** in 13 s.

**Revision attested:** `channels/src/bridge/queue.rs`
`sha256 8F34E8DC1BFB97F4B4198B64A02976A2A934B6DB8CC1D01D129DEC14BE33901C`,
mtime `16:12:53` — the revision *after* the flake fix (the `wait_until` budget in
`an_idle_conversation_is_evicted_and_its_worker_stops` raised 5 s → 20 s, plus the
up-front `assert_eq!(len, 2)` now at `queue.rs:681`). The run's own
`outcomes.json` brackets it: `10:29:39Z` → `10:46:34Z`, both after that mtime, and
the hash re-read at the end was unchanged. **This is the first cargo-mutants
measurement of these bytes.**

### 1. Result

| | count |
|---|---|
| mutants listed by `cargo-mutants --list` | 37 |
| mutants executed on the attested revision | **37 (the whole file)** |
| caught | **23** |
| missed (demonstrated survivors) | **11** |
| timeout | 0 |
| unviable (excluded by definition) | 3 |
| **declared-sample score** | **23 / 34 = 67.65 %** — **below the 80 % bar** |
| score if the one provably equivalent mutant (§5.2) is excluded | 23 / 33 = 69.70 % |

16 min 55 s wall clock at `-j 2`: 159 s baseline build + 13 s baseline test, then
~13–17 s build + ~11–15 s test per mutant (one 177 s build outlier).

**No verdict in this run is flake-driven**, and that is checked three ways, not
asserted:

1. The witness set is narrowed to `bridge::`, which by construction excludes every
   provider/transport test the previous round's `known_flaky_tests` names
   (`providers::telegram::tests::the_webhook_…`, `transport::ws::tests::…`). They
   cannot manufacture a catch here.
2. The untouched baseline ran the same 222 tests and passed all of them, so a red
   suite is not the reason any mutant looked caught.
3. Every caught verdict was re-derived from its own per-mutant log and its failing
   assertion checked against the mutated line (§3). No caught verdict rests on a
   failure the mutation cannot explain.

The one in-filter ex-flake, `an_idle_conversation_is_evicted_and_its_worker_stops`,
does appear in six caught logs — each time with the assertion that mutation
falsifies (`left: 1 | right: 2` for `len -> 1`; the eviction assertion for
`evict_idle -> ()`). Nothing had to be discarded as unattributable, so this run
needs no `no verdict (flaky)` entries at all.

### 2. Scope — what this number does and does not cover

* **37 of 37** mutants for this file were executed: this is a complete **file**
  sample, not a subsample of one. It is still not a crate- or repo-level score.
* The witness set is the `bridge::` module tests (222), not the whole crate suite.
  That narrowing is sound for this file, and that was verified rather than
  assumed: the crate's only other test targets are
  `channels/tests/agent_integration_test.rs` and `channels/tests/channel_bin.rs`,
  and **neither references `Conversations`, `SupersedeWatch`, `queue::` or any
  queue constant**; no module in `channels/src` outside `bridge/` does either
  (`Conversations` is re-exported at `bridge/mod.rs:28` and used only in
  `bridge/mod.rs` and `bridge/queue.rs`). A wider test command cannot change any
  verdict in this file.
* `DEFAULT_IDLE_TIMEOUT` is referenced exactly twice in the whole crate —
  `queue.rs:31` (definition) and `queue.rs:119` (use). See §5.1.
* Not measured: every other file in the repository. `channels/src/policy.rs` has
  since been *completed* — all 24 of its mutants, see the next section — but the
  other four files the supervisor ordered (`orchestration/loop/src/compat.rs` 72,
  `backfill.rs` 74, `quota/usage_summary.rs` 84,
  `desktop/src-tauri/src/windows_power.rs`) are untouched, and `windows_power.rs`
  is still skipped because its crate was not frozen.

### 3. The 23 caught, and the witness that killed each

Re-derived from `mutation/out-queue/mutants.out/log/*` via
`mutation/analyze-queue.py` (log↔mutant mapping taken from the run's
`outcomes.json`, because the `_NNN` log-file suffix is a *run-order* counter, not
the `--list` order — the mistake that produced the three "inverted verdicts" of
the first round).

| mutant | witness (first listed) | failing assertion |
|---|---|---|
| `53:9 is_superseded -> true` | `a_watch_reports_the_conversation_it_belongs_to` (33 tests) | `assertion failed: !watch.is_superseded()` |
| `53:9 is_superseded -> false` | `a_superseded_queued_job_is_told_so_instead_of_running` (5) | `assert!(watch.is_superseded())` |
| `53:48 != -> ==` | `a_watch_reports_the_conversation_it_belongs_to` (34) | `!watch.is_superseded()` |
| `57:9 conversation -> ""` | `a_watch_reports_the_conversation_it_belongs_to` | `left: "" | right: "slack:C1:T9"` |
| `57:9 conversation -> "xyzzy"` | same | `left: "xyzzy" | right: "slack:C1:T9"` |
| `62:9 generation -> 0` | `a_watch_reports_the_conversation_it_belongs_to` | `left: 0 | right: 3` |
| `62:9 generation -> 1` | same | `left: 1 | right: 3` |
| `187:58 + -> *` | `generation_advances_per_submission` (4) | `left: Some(0) | right: Some(1)` |
| `187:58 + -> -` | 25 tests | `attempt to subtract with overflow` at `queue.rs:187:24` |
| `203:9 generation -> None` | `generation_advances_per_submission` (3) | `left: None | right: Some(1)` |
| `203:9 generation -> Some(0)` | `generation_advances_per_submission` | `left: Some(0) | right: Some(1)` |
| `203:9 generation -> Some(1)` | same | `left: Some(1) | right: Some(2)` |
| `211:9 len -> 0` | `an_empty_table_is_reported_as_empty` (7) | `!conversations.is_empty()` |
| `211:9 len -> 1` | `an_empty_table_is_reported_as_empty` (2) | `conversations.is_empty()` |
| `215:9 is_empty -> false` | `an_empty_table_is_reported_as_empty` | `assert!(conversations.is_empty())` |
| `215:9 is_empty -> true` | same | `assert!(!conversations.is_empty())` |
| `215:26 == -> !=` | same | `assert!(conversations.is_empty())` |
| `220:9 evict_idle -> ()` | `the_table_evicts_instead_of_growing_without_bound` (3) | `left: 4 | right: 2` |
| `220:26 < -> ==` | `the_table_evicts_instead_of_growing_without_bound` (2) | table grew past its bound |
| `220:26 < -> <=` | same | table grew past its bound |
| `223:29 >= -> <` | `the_table_evicts_instead_of_growing_without_bound` (3) | `left: 4 | right: 2` |
| `239:53 != -> ==` | `the_table_evicts_instead_of_growing_without_bound` (2) | `the capacity guard leaves a candidate` |
| `259:5 worker -> ()` | `a_superseded_queued_job_is_told_so_instead_of_running` (16) | `the overtaken job must be told it was superseded` |

Every row is causally correct: the failing assertion names the value the mutation
changed (`left` is the mutant's value, `right` the asserted one).

### 4. The 3 unviable mutants — excluded, and this time for the right reason

All three fail to *compile*, verified from their own logs:

| mutant | compiler error |
|---|---|
| `125:9 with_limits -> Default::default()` | `E0277: the trait bound Conversations: Default is not satisfied` |
| `132:9 with_idle_timeout -> Default::default()` | same |
| `138:9 submit -> SubmitOutcome with Default::default()` | `E0277: the trait bound SubmitOutcome: Default is not satisfied` |

`Conversations` holds a `runner: Runner` field with no `Default`, and
`SubmitOutcome` derives only `Debug, Clone, Copy, PartialEq, Eq`. A mutant that
cannot compile cannot be killed by any test, so these are excluded from the
denominator by definition. (In the first round this same class of mutation was
*reported as caught* on a broken harness — the contradiction that exposed the
shared-`CARGO_TARGET_DIR` defect. Here the compile error is in the log.)

### 5. The 11 survivors, adjudicated one by one

These are the required classifications. Ten are **test gaps**; one is an
**equivalent mutant**; and one of the ten gaps is only closable with a clock seam,
which I state separately rather than pretending an assertion would do it. Each is
`MISSED` with **no failing test at all**, so nothing here is flake noise.

#### 5.1 `DEFAULT_IDLE_TIMEOUT` — two gaps (test gap)

`31:67: replace * with +` (`from_secs(30 * 60)` → `90` s) and
`31:67: replace * with /` (`30 / 60` → `Duration::ZERO`).

**Test gap, mechanism:** the constant is `pub`, is read only at `queue.rs:119` as
the default `idle_timeout`, and **no test anywhere in the crate reads it** —
every eviction test that cares calls `.with_idle_timeout(…)` explicitly. So the
default never decides anything the suite observes. Even `Duration::ZERO` changes
nothing: in the one test that keeps the default
(`the_table_evicts_instead_of_growing_without_bound`, `queue.rs:409`) every entry
is non-busy, so the idle set and the `or_else` fallback pick the same victim.

**What to add:** one assertion, in the same spirit as the policy-defaults fix of
the first round:

```rust
#[test]
fn the_default_idle_timeout_is_thirty_minutes() {
    assert_eq!(DEFAULT_IDLE_TIMEOUT, Duration::from_secs(30 * 60));
    assert_eq!(DEFAULT_QUEUE_CAPACITY, 8);
    assert_eq!(DEFAULT_MAX_CONVERSATIONS, 512);
}
```

**Follow-up verdict: CLOSED — but *not* by that assertion.** The constant was
pinned behaviourally instead, so the test fails on a wrong default rather than
restating it: `the_default_idle_timeout_leaves_a_two_minute_old_conversation_routed`
(`queue.rs:819`) builds the table a long-running process would have — one
conversation whose turn is in flight for 2 min 30 s, one idle for 2 min — and
calls `evict_idle` unmodified. With the real 30-minute default the two-minute-old
entry is *not* stale, so the `or_else` fallback drops the least recently used
entry (`running`). Under `30 + 60 = 90 s` or `30 / 60 = 0 s` the two-minute-old
entry *is* stale and the filter prefers it, so the assertion's `left`/`right`
invert (`left: ["running"]`, `right: ["idle"]`). Both mutants were re-run and are
**CAUGHT** (§6).

#### 5.2 `220:26: replace < with >` — equivalent mutant (excluded)

**Reason the change is not observable, given the code as written:**
`evict_idle` is a private method whose only caller is `Conversations::submit`
(`queue.rs:141`), called with the locked table immediately before inserting
exactly one entry. At that point `entries.len() <= self.max_conversations` holds
as an invariant: the table starts empty, the only writers are (a) `evict_idle`'s
loop, which runs *while* `len >= max` and so exits at `len <= max - 1`, and
(b) `submit`'s single `insert`, taking it to at most `max`; and
`max_conversations` can only be set by `with_limits`, which consumes `self`
(`pub fn with_limits(mut self, …) -> Self`) and is therefore unavailable once the
table is shared. So `entries.len() > self.max_conversations` is **false in every
reachable call**, the guard's left disjunct can never fire, and the whole guard
computes `contains(keep)` — which is exactly the mutant. On the paths where the
mutation does *not* return early (`len < max`), the `while entries.len() >=
self.max_conversations` condition is false, so no eviction happens either: both
paths are identical.

**Contrast, and why this is not a licence to weaken anything:** the sibling
mutants `< -> ==` and `< -> <=` *are* killed (§3) because they drop the early
return at `len == max` / `len < max` and let the table grow past its bound. So
the disjunct is load-bearing; only the `>` direction is dead. This is an
`unreachable-by-construction` dead state, not a hole in the tests.

#### 5.3 The idle-preference filter — seven gaps (test gap)

`230:34 != -> ==`, `231:25 && -> ||`, `231:28 delete !`, `232:25 && -> ||`,
`232:54 > -> ==`, `232:54 > -> <`, plus `220:51 || -> &&` (the guard, not the
filter).

**Mechanism — one structural blind spot, not seven unrelated holes.** The filter
at `queue.rs:227–236` chooses *which* conversation is evicted:

```rust
let idle = entries.iter().filter(|(key, entry)| {
        key.as_str() != keep
            && !entry.busy.load(Ordering::SeqCst)
            && entry.last_used.elapsed() > self.idle_timeout
    }).min_by_key(|(_, entry)| entry.last_used).map(|(key, _)| key.clone());
let candidate = idle.or_else(|| /* least recently used, ignoring busy/staleness */);
```

Only two tests ever reach it, and **neither can distinguish the victim**:

* `the_table_evicts_instead_of_growing_without_bound` (`queue.rs:409`) keeps the
  default 30-minute timeout, so nothing is stale, the idle set is empty, and the
  `or_else` fallback chooses the victim in the unmutated code too;
* `an_idle_conversation_is_evicted_and_its_worker_stops` (`queue.rs:651`) sets
  `idle_timeout = ZERO` and uses a runner that returns immediately, so *every*
  entry is non-busy and stale — the idle set equals the fallback set and both
  pick the same least-recently-used entry.

Consequently each mutant either empties the idle set (→ fallback → same victim)
or widens it to a set whose LRU is still that same victim. Both tests assert the
*table size* and that *some* worker stopped; **no test asserts which conversation
disappeared**, and no test ever has a conversation whose turn is in flight *and*
which is also the globally oldest. That single missing scenario is what all seven
mutants need — see §6.

`220:51 || -> &&` is the same class one level up: it differs only when the table
is exactly full **and** `keep` is already routed (unmutated: early return, no
eviction; mutant: evicts a bystander). No test leaves the table exactly full and
then re-submits to an existing conversation.

**Follow-up verdict: all seven CLOSED (§6).** Two tests close them, and each one
fails *for the mutant it names*, with the failing assertion in its own log:

* `a_running_turn_is_kept_and_the_idle_conversation_goes` (`queue.rs:747`) — the
  busy-oldest / idle-newer state this section says no test creates. Six mutants
  (`230:34`, `231:25`, `231:28`, `232:25`, `232:54 > -> ==`, `232:54 > -> <`)
  each make the filter select or fall back to `running`, and the test fails on
  `the conversation whose turn is in flight must keep its mailbox`.
* `a_full_table_keeps_its_bystander_when_an_existing_conversation_sends_again`
  (`queue.rs:714`) — `220:51`, whose mutant leaves the table at `len = 1` after a
  re-submission; the test fails on
  `assertion left == right failed: a re-submission must not evict a bystander,
  left: 1, right: 2`.

#### 5.4 `232:54: replace > with >=` — survivor, boundary-only (no honest assertion closes it)

This one differs from the original **only** when
`entry.last_used.elapsed() == self.idle_timeout` exactly. For the only timeout any
test in the crate can set (`Duration::ZERO`) that means `last_used ==
Instant::now()` read from inside `evict_idle`. The production code calls
`Instant::now()` itself and injects no clock, so no test can construct that
equality deterministically. I am **not** calling it equivalent — the bytes do
differ on that input — and I am **not** proposing an assertion that pretends
otherwise: closing it needs a clock seam (an injectable `now`), which is a
production change and outside this item's write scope. It is reported as a
survivor with this limit stated.

**Follow-up verdict: still MISSED — confirmed by measurement, now with the
reachability argument spelled out.** Item `todo_50756baf3574` re-ran it against
the three new tests: `MISSED`, no failing test, as before. Nothing the tests can
assert closes it, and the reason is stronger than "no test does":

* The mutant differs from the original **only** on `elapsed() == idle_timeout`.
* `idle_timeout` reaches `Duration::ZERO` only through `with_idle_timeout(ZERO)`
  (the default is 30 min), so the difference needs `elapsed() == 0`, i.e. the
  entry's `last_used` not being *strictly* before the `Instant::now()` read
  inside `evict_idle` — two clock reads that must agree to the tick.
* `last_used` is written in exactly one place, `submit`, always from
  `Instant::now()`, and the clock is monotonic, so on every input production can
  actually produce `elapsed() >= 0` and equality is a coincidence no test can
  force. Measured: the platform clock resolves ~100 ns (QPC / `CLOCK_MONOTONIC`),
  so even the ZERO case is unobservable in practice.
* There *is* one construction that would kill it, and I declined to use it: give
  the entry a **future** `last_used`, which makes `elapsed()` saturate at
  `Duration::ZERO` (Rust ≥ 1.60), so `0 >= 0` holds for the mutant and `0 > 0`
  does not, and the fallback then evicts the other entry. That input is
  unreachable in production (monotonic clock + the single `Instant::now()`
  write), so the test would be asserting `std`'s saturation semantics on a state
  the code cannot reach — a mutant-killing test, not a behavioural one. If the
  mutation owner prefers the number to the realism, say so and it is a five-line
  test; it is not in the suite on my judgement, and this is the reason.

### 6. The remedy, applied in item `todo_50756baf3574`, and re-measured

Three tests were added to `mod tests`; **no production line was touched**. The ten
survivors were then re-run as the only mutants under test, so the check costs
~6 min instead of the file's ~17:

```bash
# default copy mode (NOT --in-place), -j 2, TEMP/TMP on D:, and NO CARGO_TARGET_DIR
cargo mutants -p future-channel --file channels/src/bridge/queue.rs --gitignore true \
    --timeout 300 -j 2 -o mutation\out-queue-check \
    -F 'queue\.rs:(31:67|220:51|230:34|231:25|231:28|232:25|232:54)' \
    --cargo-test-arg --lib --cargo-test-arg bridge::   # witness set, now 225 tests
```

| | count |
|---|---|
| survivors submitted | **10** (every `uncaught` entry of `mutation/summary.json`) |
| baseline | `225 passed; 0 failed; 0 ignored` in 118 s build + 13 s test |
| **caught** | **9** |
| **missed** | **1** — `232:54 > -> >=` (§5.4) |
| timeout / unviable / unattributable | 0 / 0 / 0 |
| wall clock | 5 min 42 s at `-j 2` |

**Revision — verified before *and* after the run.** All four diff hunks against
`HEAD` are downstream of `#[cfg(test)] mod tests` (line 279): three pure
insertions, plus one `Duration::from_secs(5)` → `20` that is part of the
*pre-existing* uncommitted flake fix noted in §1, not of this item. The
production region is byte-identical to the revision §1 measured — `cargo mutants
--list` still reports **exactly 37 mutants at the same `line:col` addresses**,
which could not happen if a production line had been added, removed or moved —
and the ten applied patches are **byte-identical** to the corresponding
`mutation/out-queue/diff/` files of the original run (compared by SHA-256, 10 of
10). File hash `FE40F333828A44A8BF6A157A2FEA9C890AB750BCC8007AF6D76C6792616BA5C9`
(`8F34E8DC…` attests the pre-test revision of §1, `3565C139…` the pre-`rustfmt`
revision of this item's tests; the run above is on `FE40F333…`, and the hash was
re-read at the end and was unchanged).

**Every catch is attributed to a named test and to an assertion that names the
mutated value** (per-mutant logs under
`mutation/out-queue-check/mutants.out/log/`):

| mutant | witness test(s) | the assertion that failed |
|---|---|---|
| `31:67 * -> +` | `the_default_idle_timeout_leaves_a_two_minute_old_conversation_routed` | `left: ["running"] right: ["idle"]` |
| `31:67 * -> /` | same | same |
| `220:51 \|\| -> &&` | `a_full_table_keeps_its_bystander_when_an_existing_conversation_sends_again` | `left: 1 right: 2` — *a re-submission must not evict a bystander* |
| `230:34 != -> ==` | `a_running_turn_is_kept_and_the_idle_conversation_goes` | *the conversation whose turn is in flight must keep its mailbox* |
| `231:25 && -> \|\|` | same | same |
| `231:28 delete !` | same | same |
| `232:25 && -> \|\|` | same **+** the default-timeout test | same |
| `232:54 > -> ==` | same | same |
| `232:54 > -> <` | same **+** the default-timeout test | same |

`232:54 > -> >=` is the only survivor, and §5.4 records why: it differs from the
original only on `elapsed() == idle_timeout`, an input no honest test can
construct (production's `last_used` is always a past `Instant::now()` on a
monotonic clock). Arithmetic for the file, for the mutation owner to confirm on a
full re-run — **not** a re-label of any verdict in `mutation/summary.json`:

* 23 caught before + 9 now = **32 caught**;
* survivors left: `232:54 > -> >=`, plus the proven-equivalent `220:26 < -> >`
  excluded on the written proof in §5.2;
* so 32 / 34 = **94.12 %**, or 32 / 33 = **96.97 %** if the boundary-only
  survivor is excluded as unobservable-on-reachable-input too.

**Product note (not a mutant finding; no code changed).** The default-timeout test
pins a behaviour that the code comment calls deliberate but that deserves a
product ruling: when **no** entry is past the idle timeout, the `or_else`
fallback evicts the least recently used entry *ignoring* `busy`, so a
conversation whose turn is in flight can lose its mailbox (the test asserts
exactly this: `running` goes, `idle` stays). Reachable when the table is at
`max_conversations` (default 512) with every entry touched inside the idle
window and the LRU one mid-turn. The in-flight turn itself is not killed
(`recv` drains the queue before returning `None`), but the re-created entry
starts a *new* worker with a fresh generation, so a message arriving after
re-creation can no longer supersede that turn — the one-worker-per-conversation
guarantee the module exists for is weakened for that window. The alternative
(refusing to evict) is unbounded growth, which is what the fallback was written
to prevent, so this is a trade-off rather than an oversight — but if the
supersede guarantee is meant to be absolute, the fix is a production change
(refuse to evict a busy entry and let the caller reject instead) and
`the_default_idle_timeout_leaves_a_two_minute_old_conversation_routed` is the
test whose assertion would then have to change with it.

#### 6.1 The proposal this was built from

*(Kept as history: item `todo_eb0e6400679f` wrote these out. The code that landed
differs in three ways — (a) pins the default **behaviourally** instead of by
`assert_eq!`; (b) waits on the real `busy` flag through `wait_until` instead of
`sleep(50 ms)` guesses; (c) keeps test (c) as written. Names, line numbers and
the measured witnesses are the §6 table above.)*

**(a)** landed as
`the_default_idle_timeout_leaves_a_two_minute_old_conversation_routed` (§5.1) →
kills the two `31:67` mutants.

**(b)** one new behavioural test for the filter, placed next to
`an_idle_conversation_is_evicted_and_its_worker_stops`:

```rust
#[tokio::test]
async fn a_running_turn_is_kept_and_the_idle_conversation_goes() {
    // The idle-preference filter must decide *which* conversation is dropped:
    // with an older conversation whose turn is still running and a newer idle
    // one, the idle one must go — the `or_else` fallback alone would have
    // dropped the running turn, because it is the least recently used.
    let gate = crate::bridge::Shutdown::new();
    let gate_for_runner = gate.clone();
    let conversations = Conversations::new(Arc::new(move |job, _watch| {
        let gate = gate_for_runner.clone();
        Box::pin(async move {
            if job.conversation == "running" {
                gate.notified().await; // park, so `running` stays busy
            }
        })
    }))
    .with_limits(4, 2)
    .with_idle_timeout(Duration::ZERO);
    let sink: Arc<dyn ReplySink> = Arc::new(RecordingSink::default());
    assert_eq!(conversations.submit(job("running", sink.clone())).await,
               SubmitOutcome::Accepted);
    tokio::time::sleep(Duration::from_millis(50)).await; // its runner parks
    assert_eq!(conversations.submit(job("idle", sink.clone())).await,
               SubmitOutcome::Accepted);
    tokio::time::sleep(Duration::from_millis(50)).await; // its runner returns
    assert_eq!(conversations.len().await, 2);
    conversations.submit(job("third", sink.clone())).await; // overflows the table
    assert!(conversations.generation("running").await.is_some(),
            "the conversation with a turn in flight must keep its mailbox");
    assert!(conversations.generation("idle").await.is_none(),
            "the idle conversation is the one that must be evicted");
    gate.trigger();
    tokio::time::sleep(Duration::from_millis(30)).await;
}
```

Kills `230:34`, `231:25`, `231:28`, `232:25`, `232:54 ==` and `232:54 <`: in each
case either the idle set becomes empty (→ fallback → the *older, busy* `running`
is evicted) or it becomes a set whose LRU is `running`; either way
`generation("running").is_some()` fails.

**As landed:** all six were **CAUGHT** by exactly that test (§6 table), so the
prediction held with no adjustment to what the mutants were expected to do beyond
the two waits on the `busy` flag.

**(c)** one more test for `220:51`, which needs a full table plus an existing key:

```rust
#[tokio::test]
async fn a_full_table_does_not_evict_when_an_existing_conversation_sends_again() {
    let conversations =
        Conversations::new(Arc::new(|_job, _watch| Box::pin(async {}))).with_limits(4, 2);
    let sink: Arc<dyn ReplySink> = Arc::new(RecordingSink::default());
    for name in ["a", "b"] {
        assert_eq!(conversations.submit(job(name, sink.clone())).await,
                   SubmitOutcome::Accepted);
    }
    assert_eq!(conversations.len().await, 2);
    // A re-submission is not a capacity event: the guard must return before the
    // eviction loop, so the bystander keeps its mailbox and the table keeps its size.
    assert_eq!(conversations.submit(job("a", sink.clone())).await,
               SubmitOutcome::Accepted);
    assert_eq!(conversations.len().await, 2,
               "a re-submission must not evict a bystander");
    assert!(conversations.generation("b").await.is_some(),
            "the bystander conversation must keep its mailbox");
}
```

**Measured effect (replaces the projection this section used to carry):** caught
23 → **32**, survivors 11 → 2 (the proven-equivalent `220:26`, §5.2, and the
boundary-only `232:54 >=`, §5.4) — mechanically 94.12 % of the file's 34 viable
mutants, 96.97 % if the boundary-only survivor is excluded as well. The tests
landed in `channels/src/bridge/queue.rs` (item `todo_50756baf3574`); the verdicts
in `mutation/summary.json` are still the owner's to re-run and re-issue.

**A finding worth carrying forward:** 9 of 11 survivors sat behind one 4-line
predicate whose *victim choice* nothing asserted. Line coverage of
`evict_idle` was already 100 % — the suite executed the filter on every eviction
and never checked its answer. That is exactly the class of weakness this sample
exists to expose, and it is invisible to a coverage number.

### 7. Three infrastructure lessons (each one cost a whole attempt)

1. **cargo-mutants' scratch tree lives in `%TEMP%`.** Attempt 1 died in 20 s with
   `Failed to copy …os error 112` (disk full) because `C:` had 0.1 GB free while
   `D:` had 246 GB. `TEMP`/`TMP` are therefore pointed at `D:\cov100-mut-tmp` for
   every run. (cargo-mutants also copies `target/debug` into the scratch tree,
   ~1.1 GB — which is *why* a per-mutant build is 13–17 s rather than minutes, and
   why `--copy-target` is left at its default.)
2. **A background child does not survive the agent's tool-call teardown.** Attempt
   2 built for 63 s and was killed with no error and no outcome file, leaving
   `mutants.out/` with a `baseline.log` and nothing else. The driver is now
   started through `Win32_Process.Create` (`mutation/run-queue.cmd`,
   `mutation/run-policy.cmd`), so it is parented to `WmiPrvSE` instead of to the
   shell, and it survives turn boundaries. Both `.cmd` files are kept so the runs
   are reproducible; they set `TEMP`/`TMP`, `CARGO_BUILD_JOBS=3`,
   `RUST_TEST_THREADS=4`, never `CARGO_TARGET_DIR`, and never `--in-place`.
3. **Shrinking the witness set is what makes verdicts attributable.** `–-lib
   bridge::` removes the known-flaky provider/transport tests by construction, so
   this run needed no `no verdict (flaky)` entries and no second-guessing of 23
   verdicts. The narrowing was then *verified* to lose no witness (§2). Compared
   with the first round — where 6 of 22 catches rested on the telegram-webhook
   test failing on a policy mutation it cannot observe — this is the difference
   between a measured score and a manufactured one.

### 8. Reproduce

```
# 1. attest the revision; confirm no other worker is editing the file
sha256sum channels/src/bridge/queue.rs     # want 8F34E8DC…901C
# 2. detach it (a shell child is killed at tool-call teardown, §7.2)
mutation\run-queue.cmd
# 3. read the verdicts, then attribute every catch to a failing test
python mutation/analyze-queue.py
```

Artifacts: `mutation/out-queue/mutants.out/{caught,missed,unviable}.txt`,
`outcomes.json` (authoritative name↔log map + the exact `cargo test` argv per
phase), `log/` (37 per-mutant logs), `diff/` (the applied patch for each), and
`mutation/out-queue-attempt2-baseline.log` (the orphaned attempt-2 evidence).

**Follow-up re-run (item `todo_50756baf3574`, the 10 survivors after the three new
tests landed):**

```
sha256sum channels/src/bridge/queue.rs     # FE40F333…BA5C9 (tests added, production untouched)
cargo mutants -p future-channel --file channels/src/bridge/queue.rs --gitignore true \
    --timeout 300 -j 2 -o mutation\out-queue-check \
    -F 'queue\.rs:(31:67|220:51|230:34|231:25|231:28|232:25|232:54)' \
    --cargo-test-arg --lib --cargo-test-arg bridge::
# -> 10 mutants tested in 6m: 1 missed, 9 caught;  baseline 225 passed, 0 failed
```

Artifacts: `mutation/out-queue-check/mutants.out/{caught,missed}.txt`,
`outcomes.json`, `log/` (10 per-mutant logs + `baseline.log`), `diff/` (the ten
applied patches, byte-identical to `out-queue/diff/`'s for the same mutants —
further evidence that only test code moved).

---

## `channels/src/policy.rs` — the sample completed: 24 of 24 mutants, 22 caught, 0 survivors

*Same item, same session · appended 2026-09-26 · raw artifacts under `mutation/out-policy-final/`*

The roll-up section above left `policy.rs` at **8 of 24** mutants executed. The
gap is now closed: **all 24 executed, 22 caught, 2 unviable, 0 missed, 0
timeout** — 22 / 22 viable = **100 % on this file** — in 9 min 8 s at `-j 2`
(baseline 128 s build + 13 s test; ~15–20 s per mutant). Same method as the queue
section: default copy mode, `--gitignore true`, `--timeout 300`, **no
`CARGO_TARGET_DIR`, no `--in-place`**.

**Revision attested, and this time it did not move:**
`channels/src/policy.rs` `sha256 9E746D4F…8DE42`, mtime `15:13:36` — byte-identical
before the run, at the run's `end_time`, and again after it. The two other worker
edits in this checkout (the queue flake fix at 16:12:53, `channels/src/policy.rs`
at 15:13:36) both predate the two runs, which is why both attest cleanly.

**Witness set** — `cargo test --verbose --package=future-channel@0.1.0 --lib --
policy:: bridge::`, i.e. **244 tests** (`1214 filtered out`), all green on the
baseline. It is the union of the two module namespaces that can observe
`PolicyEngine`: `policy::tests` directly, and `bridge::tests` /
`feishu::bridge::tests` through `Bridge::handle` → `check_dm` / `check_group` /
`set_override` (visible in the table below: 5 of the 22 catches are witnessed by
`feishu::bridge::tests::…` alone and 3 by `bridge::tests::…`). The filter is there
to exclude the known-flaky provider/transport tests, not to narrow the semantics.

### 1. Why the earlier attempt at this run failed twice (arg plumbing, not code)

Worth recording because both failures were mine and both were fast and loud:

1. `--cargo-test-arg --skip=<test>` → `error: unexpected argument '--skip' found`.
   cargo-mutants inserts `--cargo-test-arg` values **before** cargo's `--`, and
   `--skip` is a *test-binary* flag. Retried and failed as
   `mutation/out-policy-final-skiparg-baseline.log`.
2. Two filters as two positionals → `error: unexpected argument 'bridge::' found`.
   Cargo accepts only one `[TESTNAME]`; libtest's multi-filter form needs the
   filters **after** `--`. Retried and failed as
   `mutation/out-policy-final-twofilter-baseline.log`.
3. What works: `--cargo-test-arg --lib --cargo-test-arg=-- --cargo-test-arg
   policy:: --cargo-test-arg bridge::`, which becomes
   `cargo test … --lib -- policy:: bridge::` — the `=` form is what lets a bare
   `--` through as a value. Verified by the baseline's own test count (244).

### 2. The 22 catches, every one attributed

| mutant | witness (first listed) | failing assertion |
|---|---|---|
| `52:5 default_dm_policy -> String::new()` | `default_dm_policy_is_the_allowlist_safe_default` | `left: "" | right: "allowlist"` |
| `52:5 default_dm_policy -> "xyzzy"` | `default_dm_policy_is_the_allowlist_safe_default` | `left: "xyzzy" | right: "allowlist"` |
| `56:5 default_group_policy -> String::new()` | `default_group_policy_is_disabled` (+3) | `left: "" | right: "disabled"` |
| `56:5 default_group_policy -> "xyzzy"` | `default_group_policy_is_disabled` (+3) | `left: "xyzzy" | right: "disabled"` |
| `60:5 default_require_mention -> false` | `default_require_mention_is_true` (+3) | `assertion failed: parsed.require_mention` |
| `91:13 delete arm "open" in check_dm` | `dm_open_allows_anyone` (+12 `bridge::tests::…`) | `left: Denied("You are not authorized…") | right: Accepted` |
| `92:13 delete arm "disabled" in check_dm` | `dm_disabled_denies_even_allowlisted` | `matches!(…, Access::Denied(_))` |
| `96:21 || -> &&` in check_dm | `dm_allowlist_allows_member_and_denies_stranger` (+2) | `left: Denied("You are not authorized…") | right: Allowed` |
| `96:68 == -> !=` in check_dm | `dm_allowlist_allows_member_and_denies_stranger` (+3) | same |
| `119:13 delete arm "open" in check_group` | `group_open_with_mention_requirement` (+5) | `left: Denied("… not in the group_allowlist") | right: Allowed` |
| `125:28 && -> ||` in check_group | `group_open_with_mention_requirement` (+7) | `assertion failed: wait_done(…)` / `Denied(_)` |
| `125:31 delete !` in check_group | `group_message_requires_mention` (+3) | `reactions … is_empty()` |
| `130:13 delete arm "disabled" in check_group` | `an_unconfigured_engine_has_group_chats_disabled` (+3) | `left: Denied("…not in the group_allowlist") | right: Allowed` |
| `133:35 == -> !=` in check_group | `an_unconfigured_engine_has_group_chats_disabled` (+3) | `left: Denied("Group chat is disabled") | right: Allowed` |
| `135:36 && -> ||` in check_group | `an_unconfigured_engine_has_group_chats_disabled` (+1) | `left: Denied("Mention the bot…") | right: Allowed` |
| `135:39 delete !` in check_group | `group_disabled_but_override_enabled_allows` (+1) | `the default mention gate must survive an explicit per-chat enable` |
| `146:21 || -> &&` in check_group | `group_allowlist_member_and_wildcard` (+1) | `left: Denied("…not in the group_allowlist") | right: Allowed` |
| `146:71 == -> !=` in check_group | `group_allowlist_member_and_wildcard` (+1) | same |
| `153:32 && -> ||` in check_group | `group_allowlist_member_and_wildcard` (+1) | `left: Denied("Mention the bot…") | right: Allowed` |
| `153:35 delete !` in check_group | `group_allowlist_member_must_mention_when_required` | `matches!(engine.check_group("oc_a", false), Access::Denied(_))` |
| `168:9 set_override -> ()` | `chat_overrides_change_the_verdict_at_runtime` (+5) | `left: Denied("Group chat is disabled") | right: Allowed` |
| `172:9 remove_override -> ()` | `remove_override_restores_global_behavior` | `left: Denied("This group is disabled") | right: Allowed` |

**Two open questions from the first round are now closed by measurement rather
than by argument.** `92:13` (`delete arm "disabled"`) had been a "no verdict"
whose LNK1104 build failure was attributed to the shared target dir: it is now
**caught**, by exactly the test the source analysis predicted
(`dm_disabled_denies_even_allowlisted`). And `91:13` (`delete arm "open"`), which
the first round counted from a hand-reproduction because its cargo-mutants log
was untrustworthy, is now **caught by 13 tests** including the five
`bridge::tests::a_*` cases that go through `Bridge::handle`. Nothing in the first
round's list is still open.

### 3. The 2 unviable, excluded

| mutant | compiler error |
|---|---|
| `90:9 check_dm -> Access with Default::default()` | `E0277: the trait bound Access: Default is not satisfied` |
| `112:9 check_group -> Access with Default::default()` | same |

`Access` derives only `Debug, Clone, PartialEq, Eq` (`policy.rs:18–22`). This is
the same mutation the first round saw reported as *caught* on the broken harness;
here the compile error is in the log, and it is the reason the mutant cannot be
counted either way.

### 4. What this changes in the roll-up

| file | listed | executed | caught | missed | unviable | file score |
|---|---|---|---|---|---|---|
| `channels/src/policy.rs` | 24 | **24** | 22 | **0** | 2 | 22/22 = **100 %** |
| `channels/src/bridge/queue.rs` | 37 | **37** | 23 | 11 (10 after the provable equivalent) | 3 | 23/34 = **67.65 %** |
| **declared sample, both files** | **61** | **61** | **45** | **11** | **5** | **45/56 = 80.36 %** |

Counting the one provably equivalent `queue.rs` mutant as excluded (its proof is
in §5.2 above) gives 45 / 55 = **81.82 %**. Both readings clear the 80 % bar, and
the conservative one is the one to quote if a single figure is wanted. The
survivors are unchanged by the policy run: **all 11 are in `queue.rs`, and 9 of
them sit behind the one 4-line idle-preference predicate** whose victim choice no
test asserts. The remedy for them is specified in §6 above.

Artifacts: `mutation/out-policy-final/mutants.out/{caught,missed,unviable}.txt`
(22 / 0 / 2) and `outcomes.json` — which also records the *failed* baseline command
lines that produced lessons §1 of this section.

---

## The revision-stability re-runs — `policy.rs` re-measured on the fixed suite, `queue.rs` measured whole, once

*Item for this turn (mutation follow-up) · agent `w-mut` · appended 2026-09-26 ·
raw artifacts under `mutation/out-policy-stable/` and `mutation/out-queue-full/`*

### 1. Why these two files were re-measured, and what the baselines were

The two channels samples above were taken while the lib suite was unstable: a
literal port (`127.0.0.1:18787`) shared by two `channels` tests made a second
concurrent suite collide, and one pre-fix policy run failed in 17 of its 24 logs.
A mutant whose *only* failing test is an unrelated flake is not caught, it is
unattributed. w-flake removed the collision (the literal port is gone from
`channels/src` — every fixture binds `127.0.0.1:0` now), and the supervisor
re-verified it 5/5 serially and 2/2 concurrently. The two re-runs below are the
re-derivation on that suite, not a carry-over:

| run | unmutated baseline | mutants | timeouts | `no verdict (flaky)` |
|---|---|---|---|---|
| `out-policy-stable` | **green** — `190 s build + 67 s test`, full lib target (1461 tests), **no filter** | 24/24 executed | 0 | 0 |
| `out-queue-full` | **green** — `190 s build + 68 s test`, same | 37/37 executed | 0 | 0 |

That is ~70 minutes of continuous mutation testing with zero flake-attributed
verdicts. The premise that a suite failing 17 of 24 times cannot support a verdict
is accepted without argument; what follows is what the verdicts become on the
suite that does not fail.

### 2. `channels/src/policy.rs` — 24/24 on the unfiltered suite: the verdicts do not move

Revision `sha256 9E746D4F…8DE42` re-read **after** the run and identical to the
pre-launch value. 28 min at `-j 2` in `mutation/out-policy-stable/`
(`--cargo-test-arg --lib`, no module filter, no `CARGO_TARGET_DIR`, never
`--in-place`).

| listed | executed | caught | missed | unviable | no-verdict | score |
|---|---|---|---|---|---|---|
| 24 | **24** | **22** | **0** | 2 | 0 | 22/22 = **100 %** |

The numbers match `out-policy-final`; the difference is that this run's witness set
was the **whole lib target**, so the catches cannot be an artifact of the earlier
run's `policy:: bridge::` filter either. Every catch is attributed to a failing
assertion in its own log (`mutation/out-policy-stable-attribution.txt`), and
`python mutation/verify-attribution.py mutation/out-policy-stable/mutants.out`
reports **0 catches whose only failing test was on the pre-fix flake list**.

### 3. The six flake-backed verdicts of the first round, re-adjudicated one by one

`python mutation/verify-attribution.py mutation/out-policy/mutants.out` reproduces
the supervisor's count from the logs themselves: in the **first** policy run
**6 mutants were called caught with the flaky telegram-webhook test as their only
failing test**. Those six verdicts are unusable, and this is what the stable-suite
run says about each:

| run 1 (flake-only, unusable) | stable-suite verdict | witness now |
|---|---|---|
| `52:5 default_dm_policy -> String::new()` | **CAUGHT** | `policy::tests::default_dm_policy_is_the_allowlist_safe_default`, `policy::tests::an_empty_config_object_carries_the_safe_defaults_through_serde` |
| `56:5 default_group_policy -> "xyzzy"` | **CAUGHT** | 4 `policy::tests::*` (`an_unconfigured_engine_has_group_chats_disabled`, `default_group_policy_is_disabled`, …) + `providers::discord::tests::a_message_create_dispatch_flows_to_the_bridge` |
| `60:5 default_require_mention -> false` | **CAUGHT** | 4 `policy::tests::*`, including `default_require_mention_is_true` |
| `91:13 delete arm "open" in check_dm` | **CAUGHT** | 30 tests; the on-point one is `policy::tests::dm_open_allows_anyone`, then 12 `bridge::tests::a_*` cases through `Bridge::handle` |
| `92:13 delete arm "disabled" in check_dm` | **CAUGHT** | `policy::tests::dm_disabled_denies_even_allowlisted` (its only failing test, and it is the right one) |
| `90:9 check_dm -> Access with Default::default()` | **UNVIABLE** | `error[E0277]: the trait bound Access: Default is not satisfied` — it cannot compile, so run 1's "caught" was the stale-binary defect, not a test result |

The two mirror-image mutants that run 1 recorded as *missed* (`52:5 -> "xyzzy"`,
`56:5 -> String::new()`) are also **caught** now, by those same assertions. So the
old judgment is **superseded explicitly, not silently**: five of the six
flake-backed verdicts are replaced by causally correct witnesses (the assertions
the weak-test audit added for exactly these constants), and the sixth was
impossible from the start. The old run stays on disk at `mutation/out-policy/` as
the evidence for the defect analysis in the roll-up above.

### 4. `channels/src/bridge/queue.rs` — 37 of 37 in one measurement of one revision

The "32 caught" in §6 above was stitched from a 23-catch full run plus a 9-catch
re-run of the survivors only. It is now a single measurement of all 37 mutants on
`FE40F333…BA5C9` — hash identical before and after, the file's mtime (19:29:08)
predates the launch, and nothing in this item edited the file. 41 min at `-j 2`
with the **full lib** witness set.

| listed | executed | caught | missed | unviable | no-verdict | score |
|---|---|---|---|---|---|---|
| 37 | **37** | **32** | **2** | 3 | 0 | 32/34 = **94.12 %** |

* **Nothing flipped.** Every mutant the previous round counted as caught is still
  caught, and the three tests added in `todo_50756baf3574` still kill exactly the
  nine survivors they were written for (witnesses for all 32 in
  `mutation/out-queue-full-attribution.txt`, produced by
  `mutation/analyze-queue.py`).
* The two survivors are the same two as before: `220:26 < -> >` (**proven
  equivalent**, §5.2 — excluded from nothing here, just not counted as a gap) and
  `232:54 > -> >=` (**boundary-only**: it differs from the original only on
  `elapsed() == idle_timeout`, which no test can construct without a clock seam in
  production; counted as a survivor, not waived as equivalent).
* Cost, measured: baseline `190 s build + 68 s test`; 13–41 s build + 65–73 s test
  per mutant.
* Hygiene: `mutation/verify-attribution.py` → **0 catches whose only failing test is
  flake-listed**. Eight wide-blast mutants (`worker -> ()`, `is_superseded -> true`,
  `submit + -> -`, `len -> 0/1`, …) list a flake-listed provider test *alongside*
  the queue/turn assertions that name the mutated value — that is what a
  large-blast-radius mutation looks like against a full-suite witness set, not a
  flake.

### 5. Combined channels sample after this item

| file | listed | executed | caught | missed | unviable | file score |
|---|---|---|---|---|---|---|
| `channels/src/policy.rs` | 24 | 24 | 22 | 0 | 2 | **100 %** |
| `channels/src/bridge/queue.rs` | 37 | 37 | 32 | 2 | 3 | **94.12 %** |
| **combined** | **61** | **61** | **54** | **2** | **5** | **54/56 = 96.43 %** |

With the proven-equivalent mutant excluded from the denominator: 54/55 =
**98.18 %**. This supersedes the 45/56 = 80.36 % of the roll-up above; the change
is the queue remedy measured on the final revision, not a re-labelling of any
verdict. Artifacts: `mutation/out-policy-stable/mutants.out/`,
`mutation/out-queue-full/mutants.out/` (`{caught,missed,unviable}.txt`,
`outcomes.json`, `log/`, `diff/`), the two attribution dumps
(`mutation/out-{policy-stable,queue-full}-attribution.txt`), and the hygiene
scripts `mutation/verify-attribution.py` / `mutation/dump-fails.py`.

---

## `orchestration/loop/src/compat.rs` — first sample of the loop crate: 72 mutants, 46 caught

*Item for this turn (mutation follow-up) · agent `w-mut` · appended 2026-09-26 ·
raw artifacts under `mutation/out-compat/` + `mutation/out-compat-recheck/`*

### 1. Command, revision, cost

`orchestration/loop/src/compat.rs` `sha256 22D63EBF…6027C1`, mtime `2026-09-26
01:42:42`, re-read after the run and identical (the file was not touched).

```powershell
cargo mutants -p future-loop --file orchestration/loop/src/compat.rs --gitignore true `
    --timeout 300 -j 3 -o mutation\out-compat --json `
    --cargo-test-arg --lib --cargo-test-arg --test --cargo-test-arg compat_projection_contract
```

* `-j 3` instead of the steering's `-j 2`: the box was verified idle (no other
  `cargo`/`rustc`/`cargo-llvm-cov` alive) and `-j 2` would not fit the wall-clock
  box. `CARGO_BUILD_JOBS=2`, no `CARGO_TARGET_DIR`, never `--in-place`.
* Unmutated baseline **green**: `113 s build + 5 s test` — 430 lib unit tests plus
  the 4 `compat_projection_contract` tests. (The loop crate keeps almost all of
  its behaviour tests in `tests/*.rs`, which is why the lib test phase is only 5 s
  and why the witness-set question below matters.)
* **72 mutants in 49 min at `-j 3`: 46 caught, 23 missed, 3 unviable, 0 timeout,
  0 `no verdict`.**

### 2. Why the witness set is `--lib` **plus** `compat_projection_contract`

The steering's command was a bare `-- --lib`. That witness set cannot see the 20
mutants in the file's projection functions (`url_encode`, `rfc3339`,
`future_loop_task_class`, `future_loop_status`, `render_active_state`,
`todo_line`): the lib test target never asserts rendered markdown, and their only
measured witnesses are in `tests/compat_projection_contract.rs` (4 tests:
`enum_values_match_loopx`, `file_layout_is_project_local`,
`todo_anchors_match_future_loop_format`, `rfc3339_matches_future_loop_shape`).
Every catch in that group in this run is attributed to one of those two contract
tests (`todo_anchors_match_future_loop_format` alone kills `36:5` ×2, `436:5` ×2,
`487:28`, `533:5` ×2). A `--lib`-only run would therefore have reported all 20
MISSED for a reason that is an artifact of the witness set — exactly the class of
unusable verdict this re-derivation exists to remove. `--lib` is kept in the set
because `compat.rs::tests` (14 lock/pid/home unit tests) only exists there; with
the contract test added, 12 of the 20 die inside the primary sample and 8 survive
(§4 Groups A and D).

### 3. The 3 unviable

| mutant | compiler error |
|---|---|
| `238:70 replace + with *` (`deadline = Instant::now() + LIVE_HOLDER_WAIT`) | `E0369: cannot multiply std::time::Instant by std::time::Duration` |
| `254:85 replace + with *` (`Instant::now() + empty_wait`) | `E0369` — same |
| `276:5 classify_lock -> LockState with Default::default()` | `E0277: the trait bound LockState: Default is not satisfied` (`LockState` is an enum with data-carrying variants) |

Excluded from the denominator by definition (a mutant that does not build cannot
be killed by any test), exactly as the 5 `channel` ones were. The two arithmetic
ones are worth naming because they look like test gaps at a glance: `+` → `-`
at both addresses **is** viable and was **caught**, each by
`compat::tests::concurrent_acquires_all_succeed` (the deadline jumps into the
past, so a momentarily-live holder is reported as held instead of being waited
for, and the four-worker acquire loop fails) — only `*` is a type error.

Two mutants that the previous round had to hand-reproduce are caught cleanly
here by named tests: `70:5 write_goal_doc -> Ok(())` by
`compat_projection_contract::file_layout_is_project_local` (it asserts `GOAL.md`
exists), and `205:5`/`210:5 acquire_active_state_lock(_with) -> Ok(default)` by
13 and 9 `compat::tests` respectively.

### 4. The 23 survivors, adjudicated

**Group A — witness-set artifacts, RESOLVED by the re-check in §5.** Their
witnesses live in the crate's integration targets, outside the primary witness
set; the re-check converted them to **CAUGHT**:

| primary survivor | decided verdict | witness that decides it |
|---|---|---|
| `24:5 url_encode -> String::new()` | **CAUGHT** | `bughunt_regressions::unicode_truncation_and_metadata_roundtrip` (line 336 sets `todo.note = "first\nstatus=done --> %20"` and asserts the round-trip) |
| `24:5 url_encode -> "xyzzy"` | **CAUGHT** | same |
| `492:28 render_active_state == -> !=` (the `<cwd>/GOAL.md` existence branch) | **CAUGHT** | same test — so the goal-document line *is* asserted somewhere, just not in the primary witness set |
| `378:5 write_run -> Ok(())` | **CAUGHT** | `bughunt_regressions::rapid_run_mirrors_do_not_overwrite_each_other` |
| `360:61 pid_alive && -> ||` | **CAUGHT, non-deterministically** | `compat::tests::concurrent_acquires_all_succeed` **in the re-check only** — the primary run of the same mutant reported MISSED. The witness is a 150-round × 4-thread race, so this verdict depends on the window being hit; see §5. |

**Group B — platform-unmeasured (not compiled on this host).** These mutants sit
in code that `#[cfg]` removes on Windows, so nothing the Windows test binary does
can observe them. The mirror-image arms *are* covered, which is the proof that
this is a platform boundary and not a hole:

| survivor (not compiled on Windows) | the Windows arms, all CAUGHT |
|---|---|
| `330:5 pid_alive -> true/false`, `331:11 == -> !=`, `335:52 == -> !=` (the `#[cfg(unix)]` probe) | `344:5 -> true/false`, `356:35 == -> !=`, `360:56 != -> ==`, `360:74 == -> !=` (the `#[cfg(windows)]` probe) |
| `370:5 pid_alive -> false` (the `#[cfg(not(any(unix, windows)))]` fallback) | — |
| `184:5 delete_pending_retry -> true` (the `#[cfg(not(windows))]` body) | `172:5`/`172:29`/`172:58` (the `#[cfg(windows)]` body) |

On a unix host the unix probe mutants would be caught by the same two unit tests
(`live_holder_pid_is_rejected` fails outright if `pid_alive` returns false — the
live lock would be reclassified `Dead` and taken over). Classified
**platform-unmeasured**, the repo's existing waiver category, not a test gap.

**Group C — a witness that is platform-gated.** `152:5 create_is_contention ->
true` and `231:27` (same function used as a match guard): the one test that can
observe the create-error path is `create_in_unwritable_dir_reports_contextual_error`,
which is `#[cfg(unix)]` — a read-only directory does not make `create_new` fail on
Windows, so the unit-test witness for this arm does not exist on this host. On
unix the mutant is caught: with contention always "true", the PermissionDenied
error falls through to `classify_lock` (`Released`) and the acquire spins to the
attempt bound, returning `could not acquire … (contended)` instead of the
asserted `create ACTIVE_GOAL_STATE.md.lock` context.

**Group D — genuine survivors.** Each is a real gap or a real limitation, stated
as such rather than waived away. Every one of these was **also missed by the
wider re-check in §5**, i.e. the gap is not a witness-set artifact:

| survivor | adjudication |
|---|---|
| `314:5 home_dir -> "xyzzy"`, `318:41 && -> ||` | **Test gap.** `home_dir` is `pub` and feeds `~` expansion in `console.rs`, `state.rs`, `workspace_guard.rs` and `boundary.rs`, but no test controls `HOME`/`USERPROFILE` and asserts the resolved value — only `tests/write_scope_dependencies.rs:40` sets `USERPROFILE`, and it never reads `home_dir`. Remedy: a unit test that sets a relative `HOME` and an absolute one and asserts the absolute-and-non-empty filter (`&&` vs `||`), plus one that asserts the `~`-expansion of a todo path. |
| `535:32 == -> !=` (deferred checkbox) | **Test gap.** The anchor renders `- [-]` for `TodoStatus::Deferred`; `grep '\[-\] \[' tests/*.rs` finds no assertion, and the fixtures carrying such a line (`backfill_drive.rs:16-17`, `console_drive_b.rs:164`) are *parse* inputs, not assertions on rendered output. Remedy: render a deferred todo and assert the `- [-]` checkbox. |
| `537:24 == -> !=` (done checkbox) | **Test gap**, same shape: assert `- [x]` on a done todo (no test does; the only `- [x]` occurrences are fixtures). |
| `560:23 == -> !=` (`user_gate` action_kind) | **Test gap.** `action_kind=goal_decision` is asserted nowhere in `tests/*.rs`; `compat_projection_contract` only checks `task_class=user_gate`, which the mutation leaves intact. |
| `591:17 == -> !=` and `593:60 delete !` (done-todo evidence) | **Test gap.** Nothing asserts that a done todo with non-empty evidence renders `evidence=…` / `completed_at=…`; and with `593:60` an *empty* evidence string renders a bare `evidence=` — the filter exists to suppress exactly that, and no test covers it either. |
| `172:5 delete_pending_retry -> false`, `172:70 >= -> <` | **Test gap, narrow and Windows-only.** The retry-success path (a delete-pending `ERROR_ACCESS_DENIED` that clears inside the 64-poll budget) is exercised only *probabilistically*, by `concurrent_acquires_all_succeed`'s 150 rounds × 4 threads — neither the primary run nor the re-check landed in it, so the mutation survived both. Remedy: a deterministic test that holds a `FILE_SHARE_DELETE` handle briefly while another acquirer creates the lock, so the transient window is guaranteed inside the budget. |
| `278:23 classify_lock: guard NotFound -> false` | **Latency-only survivor.** With the guard disabled, a missing-but-unreadable lock file is classified `Empty` instead of `Released`: the acquirer waits out `EMPTY_LOCK_WAIT` (250 ms), then removes the (absent) file and acquires. Every path still converges to acquisition — the observable difference is the wait, and no test asserts which branch was taken. Stated as a survivor with that reasoning, not as a proven equivalence. |

### 5. The survivor re-check (wider witness set) — 5 of 23 survivors were witness-set artifacts

All 23 primary survivors were re-tested with the crate's integration targets that
reference `compat` (`misc_drive`, `bughunt_regressions`, `backfill_drive`,
`monitor_poll_contract`, `run_loop_notes_drive`, `console_drive_b`) added to
`--lib` + `compat_projection_contract`. The filter matched **29** mutants — the 21
survivor addresses, plus siblings at the same addresses that the primary run had
already caught (a free consistency check) — and ran 24 min:

```powershell
cargo mutants -p future-loop --file orchestration/loop/src/compat.rs --gitignore true `
    --timeout 300 -j 3 -o mutation\out-compat-recheck -F "compat\.rs:(…21 addresses…)" `
    --cargo-test-arg --lib --cargo-test-arg --test --cargo-test-arg compat_projection_contract `
    --cargo-test-arg --test --cargo-test-arg misc_drive --cargo-test-arg --test --cargo-test-arg bughunt_regressions `
    --cargo-test-arg --test --cargo-test-arg backfill_drive --cargo-test-arg --test --cargo-test-arg monitor_poll_contract `
    --cargo-test-arg --test --cargo-test-arg run_loop_notes_drive --cargo-test-arg --test --cargo-test-arg console_drive_b
# -> 29 mutants tested in 24m: 11 caught, 18 missed   (baseline green: 102 s build + 15 s test)
```

`python mutation/compare-runs.py mutation/out-compat/mutants.out
mutation/out-compat-recheck/mutants.out` → **5 conversions**, 18 still missed, 0
un-re-tested. The conversions are the five rows of §4 Group A. **No survivor was
converted by a test that merely executes the code**: each conversion names the
assertion that failed, and each of the 18 that stayed missed is a survivor *under
both* witness sets — which is what makes the Group B/C/D adjudication above
evidence rather than a hunch.

**One verdict is not stable, and this is new information.** `360:61` was MISSED in
the primary run and CAUGHT in the re-check, by the same witness
(`compat::tests::concurrent_acquires_all_succeed`). That test is a 150-round ×
4-thread race, so whether the mutant dies depends on the interleaving it happens
to hit — the mutant's death is real but not reproducible on demand. It is counted
as **caught (non-deterministically)** and flagged in `uncaught[]`; the honest
reading of the file is 46 + 5 = **51 of 69 viable mutants decided caught, 18
survivors**, with one of the 51 resting on a race witness.

**What this sample says about the loop crate's waivers** (the reason the loop
files were ordered): the lock/projection core is well tested — 46 of 72 mutants
die against a 5-second primary witness set, and the survivors are not a diffuse
coverage hole but a short, individually-named list: `#[cfg]`-gated arms that
cannot be observed on this host (8), five unasserted `todo_line` rendering
branches, the `home_dir` absolute/non-empty filter, one Windows-only retry
window, and one latency-only survivor. Nothing in this file's survivors is
explained by "the tests never exercise it" — the code runs; the *answer* is not
asserted. That is the same class of weakness the `queue.rs` sample found, and the
remedies are named above.
