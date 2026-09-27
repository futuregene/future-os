# Archived testing records — `docs/testing/mutation-report.md`

Moved out of `docs/testing/mutation-report.md` on 2026-09-27 during the
historical-pollution cleanup (task T4b, goal `goal_39567c2a6b22`,
worktree `docs-testing`). The text below is verbatim, with its original
dates; the live record keeps a one-line pointer where each block was
removed.

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

These make **`out-policy/outcomes.json` unusable as a detection score**.
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

**Consequence for the gate.** Scoring `scripts/measure/mutation/summary.json` from
`out-policy/outcomes.json` as it stands would report 6/8 = 75 % for this file and
75 %+ elsewhere, mostly from port collisions. The honest numbers for these eight
mutants are **0/8 genuinely caught before this change** and, for the five
`default_*` mutants, **5/5 caught after it** (verified in §4/§3). A re-run must
either quarantine the flaky Telegram-webhook / ws-timing tests (the environment
class the plan already waives) or run with `--test-threads` low enough to stop
the port collisions, and it must be re-measured on the post-fix commit — the
recorded `out-policy` verdicts predate this fix and must not be carried forward.

### 7. Durable artifacts

* `scripts/measure/mutation/policy-defaults-finding.md` — the extracted raw evidence for §1–§6
  (verdict/log cross-reference, verbatim failing-test lines), kept so it survives
  a rewrite of this shared file.
* `scripts/measure/mutation/summary.json` — **not** written by this item; it is the mutation
  job's roll-up and must not be synthesized from the contaminated verdicts above.

---

## The mutation roll-up — `scripts/measure/mutation/summary.json`, its scope, and why the file needs one clean re-run

*Item `todo_c354929ff539` · agent `w-mut` · appended 2026-09-26 · raw artifacts under the runners' `out-*/` trees*

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

| | mutated tree of run 1 (`out-policy/`) | file on disk now |
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

`scripts/measure/mutation/summary.json`: `caught = 6`, `missed = 0`, over **6 viable evaluated
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
MUT=scripts/measure/mutation   # the runners and their kept evidence live here
# 1. attest the revision, and make sure no other worker is writing the crate
git -C . status --short channels/src/policy.rs ; sha256sum channels/src/policy.rs
# 2. NO CARGO_TARGET_DIR  (this is the defect in §3) and -j 1 for attribution
cargo mutants -p future-channel --file channels/src/policy.rs --gitignore true \
  --timeout 400 -o "$MUT/out-clean-<sha>" -- --lib
# 3. re-check the hash afterwards; accept a verdict only if a failing test can
#    plausibly observe the mutated line
```

Then, and only then, does `scripts/measure/mutation/summary.json` acquire a `caught`/`missed`
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

* `scripts/measure/mutation/summary.json` — the roll-up (schema `future-cov100-mutation-summary-v1`).
* `out-policy-recheck2/mutants.out/` (next to the runners) — the deciding run on
  `sha256 9E746D4F…` (`caught.txt` 2, `unviable.txt` 1, `missed.txt` empty).
* `out-policy-recheck/mutants.out/` — the earlier recheck (interrupted at
  the time box; `caught.txt` 4, killed mid-flight after mutant `90`).
* `out-policy/mutants.out/` — run 1, **stale revision, reference only**.
* `timing-probe/mutants.out/missed.txt` — the pre-fix probe
  (`60:5` MISSED).

---

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
in `scripts/measure/mutation/summary.json` are still the owner's to re-run and re-issue.

**A finding worth carrying forward:** 9 of 11 survivors sat behind one 4-line
predicate whose *victim choice* nothing asserted. Line coverage of
`evict_idle` was already 100 % — the suite executed the filter on every eviction
and never checked its answer. That is exactly the class of weakness this sample
exists to expose, and it is invisible to a coverage number.
