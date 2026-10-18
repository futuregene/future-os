# Review — `module-tauri-appdata` (§S18.5: 378 = 368 WAIVED + 10 OPEN)

**Reviewer unit:** REV-TAURI-APP · **Date:** 2026-10-17 · **Verdict: 4 rows / 11 lines FELL on measurement; 3 more rows / 21 lines fail as registrations; 4 lines UNVERIFIABLE; the rest survive.**

Reviewed record: `docs/testing/module-tauri-appdata.md`, ledger **§S18.5**, pinned report
**`/tmp/appdata-taub-s23-normalized-lcov.info`** (378 uncovered / 27 files over the ten scopes),
gate `reconcile.py --section "S18.5"` → **378 mentioned / MISSING 0** (no STALE block); the
registry gate `staletable.py --gate` reads **STALE 0 / ABSENT 0 / COVERED 0 / PASS** (re-run by
me, §0). The record stopped moving at `42cb69d10` (10:37); the **source did not** — see §0.2.

The ledger is a mixture of kinds, and each kind was attacked differently:

| kind | rows | how attacked |
|---|---|---|
| `unreachable-*` registrations (the claim is "no input reaches it") | the 10 OPEN lines, the `unreachable-by-construction` rows (`run_control.rs:190`, `files.rs:1024`, both `snapshot.rs` blocks), and the concrete-`AppHandle`/`APP_HANDLE` type-barrier rows | a probe that **produces the state** (measured), or a source-level proof that the state cannot exist |
| `attribution-artifact` (the line executed but llvm-cov attributes the region elsewhere) | the client/import/queries/stream/test_support/update_journal/approval/workspaces artifact rows, plus the artifacts inside the 164-line `update.rs` bundle | neighbour counts re-read from the pinned report, plus the source line's construct |
| failure-only / ignored-test / deliberate side-effect exclusions | the `tasks.rs` harness block, `approval_assessments.rs:169`, `workspaces.rs:936`, `update_recovery.rs:180,213` | the policy category is checked, not only the line's status |

## 0. Pairing and the one thing that moved

### 0.1 The declared pairing is clean

`reconcile.py` against the pinned report over the ten scopes: **378 uncovered, 378 mentioned,
MISSING 0**; `staletable.py --gate` on the registry pairing `tauri-appdata` reads **378 claims /
378 uncov / STALE 0 / ABSENT 0 / COVERED 0 / section / PASS**. Every ledger row is `DA:0` in its own
report, and no row claims a line the report shows covered. So none of the falls below is a
pairing artefact, and none of the record's rows was a stale overstatement **against its own
report**.

**Which report is the record's own?** §S28 names the tree instead of a commit: "`ef705360c` plus
§S26's `tasks.rs` test plus this segment's `remote_peer.rs` change". I resolved it by checking the
line table against the candidate trees rather than assuming it: at `42cb69d10` (the commit that
landed those two tests) `run_control.rs:179` is `return Ok(());` in the thread-missing arm, exactly
the construct the ledger names, and `ef705360c`'s copy of that file is byte-identical. **The pinned
report's source revision is `42cb69d10`.** All probes below were built on that revision.

### 0.2 The source moved under the record: 42cb69d10 → HEAD 26fc0a0d3

This is a **source-drift finding, not a row defect** (named, not scored as a fall, per the
supervisor's instruction).

* `git diff --stat 42cb69d10..HEAD` over the ten scopes: **16 files, +1171/−134 lines**, driven by
  the 16:23 merge `3662b442b` (PRs #1157, #1152) and `1876ffe78`. `run_control.rs` alone is
  **496 changed lines**; `import.rs` +148, `store/threads.rs` +200.
* Consequence for the ledger: many rows no longer resolve to the construct they name at HEAD.
  `run_control.rs:179` is now a **blank line** (the construct moved to 257); `run_control.rs:190`
  is the `}` of `mark_run_failed_if_active`; `store/threads.rs:1648` is no longer the uncovered
  line (the HEAD zeros are 250, 269, 1840). Full map in §0.3.
* An **indicative** HEAD figure, from an existing full-suite pair measured at HEAD by the
  tauri-shell unit (`/tmp/tauri-shell-land/{gui,headless}.info`, union by max; `gui.info` was
  written 23:33 and `headless.info` 21:44 — the only commit between them, `c953e5671`, touches
  `lib.rs` tests and docs, not the ten scopes): **389 DA:0 lines in the ten scopes / 28 files** —
  not 378. It is *not* a replacement measurement (different legs,
  four known port-8022 failures in the gui leg), but the line-number drift is structural and the
  count difference is consistent with new unwaived code (new `run_control.rs` arms at 40-42 and
  86-88, `store/runs.rs:178`, `store/threads.rs:250/269/1840`).

**Action for the supervisor, not for this review:** the record must be re-pinned to a HEAD
measurement before the final report quotes 378, and the row anchors must be re-read at that
revision. The verdicts below are at the record's own revision (`42cb69d10`), which is the only
revision its report describes; the three measured handler families are line-stable at HEAD
(review.rs 289-293, debug.rs 114/116, update_recovery.rs 36-38 are unchanged), and the
`run_control.rs` fall maps 179 → 257.

### 0.3 Drift map for the ledger's rows (HEAD zeros vs ledger anchors)

| ledger anchor (42cb) | HEAD zero(s) at the same construct | same line? |
|---|---|---|
| `client.rs:215-218, 248-249` | 215-218, 248-249 | **yes** |
| `client.rs:232` | not a HEAD zero (brace reads covered at HEAD) | **no** |
| `client.rs:1016` | 1065 | no (file +49 lines) |
| `import.rs:188, 559, 596` | 188, 456, 591, 628 | partly (188 yes; 591/628 are the moved tails) |
| `run_control.rs:179, 190` | 257 (and new 40-42, 86-88, 273, 283, 287) | no |
| `test_support.rs:547, 554-557, 771` | 555, 562-565, 798 | no |
| `store/threads.rs:1648` | 250, 269, 1840 | no |
| `tasks.rs` 95-line block | 97 zeros, including 217-219/223/306/371/426/436/454-455/466 | rows stable, count 97 |
| `debug.rs`, `review.rs`, `update_recovery.rs`, `update.rs`, `scheduler/`, `snapshot.rs`, `approval_assessments`, `workspaces` | the same line sets | **yes** |

## 1. Method and scratch tree

Scratch checkout from `git archive` (never `git worktree add`), in a **segment-unique** directory:

```bash
R=/tmp/rev-appdata-s1-20261017          # unique to this segment
git archive 42cb69d10 | tar -x -C $R
mkdir -p $R/desktop/src-tauri/binaries
: > $R/desktop/src-tauri/binaries/future-aarch64-apple-darwin   # tauri-build sidecar
cp -Rc target/cov-tauri-shell $R/target                          # APFS clone; deps reused
```

Probes are ordinary tests appended to the **end** of each file's module (so no existing line
number moves), in the scratch tree only. No production line was edited; no file in the campaign
worktree was touched except this report. Toolchain `1.97.0` (the repo pin), `cargo-llvm-cov
0.8.7`, `LLVM_COV`/`LLVM_PROFDATA` pointed at the toolchain's `llvm-tools` binaries.

```bash
export PATH=~/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
B=~/.rustup/toolchains/1.97.0-aarch64-apple-darwin/lib/rustlib/aarch64-apple-darwin/bin
export LLVM_COV=$B/llvm-cov LLVM_PROFDATA=$B/llvm-profdata
cd $R/desktop/src-tauri
CARGO_TARGET_DIR=$R/target cargo llvm-cov --no-report --lib -- rev_pr --test-threads=1
CARGO_TARGET_DIR=$R/target cargo llvm-cov report --lcov --output-path $R/probe.info
```

Result: `test result: ok. 4 passed; 0 failed; 0 ignored; 2105 filtered out; finished in 336.78s`
(log `$R/probe2.log`). The report contains two SF blocks per file — one from stale profraw that was
already in the reused target directory (campaign path `/Users/.../cov100-r2`, i.e. the tauri-shell
unit's run) and one from **this** run (`/private/tmp/rev-appdata-s1-20261017`). **Every AFTER
number quoted below is read from the `/private/tmp` block**; the campaign-path block is ignored
(it is a different run's data and is what makes path-blind reads wrong).

## 2. Verdict summary

| verdict | rows | lines |
|---|---|---|
| **FELL — measured** (a probe produced the state and the `DA` count moved 0 → 1) | 4 | **11** |
| **FELL — registration** (category/mechanism cannot stand as written; line genuinely uncovered) | 3 | **21** |
| UNVERIFIABLE (state not produced; what is missing stated) | 1 | 4 |
| SURVIVED | 35 | 342 |
| **total** | **43** | **378** |

Per-row verdicts are in §5; the measured falls are §3; the registration falls §4.

## 3. The measured falls

All four probes ran in one instrumented lib-test invocation. **Before** = the pinned report at the
same line; **after** = `$R/probe.info`, `/private/tmp` block.

| row | lines | before | after | probe |
|---|---|---|---|---|
| `agent_bridge/review.rs:289-293` | 5 | `289,290,291,292,293 = 0` (neighbours 287=3, 294=3) | **1,1,1,1,1** (294/295 stay 0) | P2 |
| `commands/debug.rs:114, 116` | 2 | `114,116 = 0` (113=2, 117=2) | **114=1, 116=1** (117=0 — the early return ran) | P3 |
| `commands/update_recovery.rs:36-38` | 3 | `36,37,38 = 0` (35=3) | **36=1, 37=1, 38=1** | P4 |
| `agent_bridge/run_control.rs:179` | 1 | `179 = 0` (177=31, 178=31, 181=31) | **179=1** (175=1, 177=1, 178=1; 176=0 — the earlier guard was passed) | P1 |

### 3.1 P1 — `run_control.rs:179`: the FK invariant is a writer rule, not a data rule

The row's mechanism: "needs a Run row whose conversation row is gone, and `schema.rs:53` declares
`thread_id TEXT NOT NULL REFERENCES threads(id)` — measured, a `DELETE FROM threads` for a thread
that still owns a run is refused (§S19.1)". The refutation is one pragma: **SQLite foreign-key
enforcement is per-connection and off by default**, and the app's own test code turns it off to
build exactly this shape (`store/runs.rs:1237`, `store/cleanup.rs:646`,
`store/review_snapshots.rs:547`, `store/db.rs:898`). §S19.1 measured the refusal **with the app's
`CONNECTION_PRAGMAS` (`db.rs:156`) applied**; the same repository demonstrates the other half.

The probe seeds a workspace-mode thread and one run, enables auto-titles, deletes the thread over a
raw connection with `PRAGMA foreign_keys = OFF`, then calls the title pass. The coverage numbers
show the walk: `175=1` (the run list guard passed), `177=1`/`178=1` (the `let … else` header),
`179=1` (the missing-thread return) — and `176=0`, the earlier early return did not run.

```rust
#[cfg(test)]
mod rev_probe {
    use super::*;
    use crate::agent_bridge::test_support::{
        mock_agent, seed_run, seed_thread, seed_workspace, TestHome,
    };

    /// REVIEW PROBE (REV-TAURI-APP): the ledger's `run_control.rs:179`
    /// (at report revision 42cb69d10) claims the thread-missing arm is
    /// `unreachable-by-construction` because `runs.thread_id REFERENCES
    /// threads(id)`. FK enforcement is per-connection and the app's own test
    /// helpers turn it off, so the orphan state is constructible on disk.
    #[tokio::test]
    async fn rev_pr_orphan_run_takes_the_missing_thread_arm() {
        let home = TestHome::new("rev-pr-orphan");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-rev"));
        let run = seed_run(&thread.id);
        crate::store::update_app_settings(crate::store::UpdateAppSettingsInput {
            auto_title_first_turn: Some(true),
            title_language: Some("en".into()),
            ..Default::default()
        })
        .expect("enable titles");
        let db = future_app_settings::app_db_path().expect("db path");
        let conn = rusqlite::Connection::open(db).expect("open db");
        conn.execute_batch("PRAGMA foreign_keys = OFF;")
            .expect("fk off");
        let deleted = conn
            .execute("DELETE FROM threads WHERE id = ?1", [&thread.id])
            .expect("delete thread");
        assert_eq!(deleted, 1, "the thread row must actually be gone");
        drop(conn);
        super::title_first_turn_if_enabled(&run.id)
            .await
            .expect("a run whose thread vanished is a no-op");
        assert!(mock.requests_of("generate_session_title").is_empty());
    }
}
```

sha256 of the appended block: **`51112b8baacb16d18602342afec95772d7ece2c28bff45b4817b627455ec2977`**
(40 lines). The record's own test comment (`run_control.rs:705-710` at this revision) states the
claim this probe breaks; it should be rewritten with the result.

**What this does and does not say:** a *store file* can hold the orphan row and a test can construct
it; no production writer in this tree produces it while FK is ON. The honest category is
`unreachable-by-construction` **for production writers** and "constructible by a raw connection /
legacy DB" for tests — i.e. not a construction proof, and the line is coverable. It is rework, and
the test that lands it must state which pragma it used.

### 3.2 P2 — `review.rs:289-293`: the TOCTOU has an injection seam (the `git` subprocess)

The row (and §S4.2, §S27.1) says "a TOCTOU window with no injection point: the existence check at
285-287 and the return at 289-293 cannot be separated from a test". They can be separated: the
window between the reads and `upsert_run_changeset`'s parent check contains
`shadow_review::materialize`, which runs `git` as a subprocess (`shadow_review/repository.rs:102,
376, 419, 443`). A PATH shim that deletes the Run row on its first invocation and then sleeps is a
deterministic ordering, not a timing race: the delete lands after `get_run`/`resolve`/
`get_review_snapshot` and before the parent check.

The probe asserts the surfaced error is the *deleted-parent* message (line 290's literal), and the
coverage moves **289-293 0 → 1** with `294-295 = 0` (the `Ok(())` was not reached). That is the
whole arm.

```rust
#[cfg(test)]
mod rev_probe {
    use std::fs;

    use crate::store::{self, CreateRunInput, CreateThreadInput, CreateWorkspaceInput};

    const SHIM: &str = r#"#!/bin/sh
if [ -n "$REV_PROBE_SENTINEL" ] && [ ! -f "$REV_PROBE_SENTINEL" ]; then
  : > "$REV_PROBE_SENTINEL"
  python3 -c 'import os,sqlite3;c=sqlite3.connect(os.environ["REV_PROBE_DB"]);c.execute("DELETE FROM runs WHERE id=?",(os.environ["REV_PROBE_RUN"],));c.commit()'
  sleep 1
fi
exec /usr/bin/git "$@"
"#;

    /// REVIEW PROBE (REV-TAURI-APP): `review.rs:289-293` is registered OPEN as
    /// "TOCTOU, no injection seam". The seam is the `git` subprocess that
    /// `materialize` runs: a PATH shim deletes the Run row inside that window,
    /// after `get_run`/`resolve`/`get_review_snapshot` have read it and before
    /// `upsert_run_changeset`'s parent check.
    #[test]
    fn rev_pr_retry_reports_a_run_deleted_during_materialize() {
        use std::os::unix::fs::PermissionsExt;

        let _lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let base = std::env::temp_dir().join(format!("futureos_revretry_{}", std::process::id()));
        let home = base.join("home");
        let ws_dir = base.join("ws");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&ws_dir).unwrap();
        std::env::set_var("HOME", &home);
        store::initialize_app_store().unwrap();
        let ws = store::create_workspace(CreateWorkspaceInput {
            name: Some("rev".into()),
            path: ws_dir.display().to_string(),
            description: None,
            create_directory: Some(false),
        })
        .unwrap();
        let thread = store::create_thread(CreateThreadInput {
            mode: "workspace".into(),
            title: Some("rev".into()),
            workspace_id: Some(ws.id),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .unwrap();
        let run = store::create_run(CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        super::capture_before(&thread.id, &run.id);
        fs::write(ws_dir.join("rev.txt"), "changed\n").unwrap();
        super::finalize_after(&thread.id, &run.id);

        let bin = base.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let shim = bin.join("git");
        fs::write(&shim, SHIM).unwrap();
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
        let sentinel = base.join("armed");
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", bin.display(), old_path));
        std::env::set_var("REV_PROBE_DB", future_app_settings::app_db_path().unwrap());
        std::env::set_var("REV_PROBE_RUN", &run.id);
        std::env::set_var("REV_PROBE_SENTINEL", &sentinel);

        let err = super::retry(&run.id).unwrap_err();
        std::env::set_var("PATH", old_path);
        assert!(
            err.to_string().contains("was deleted"),
            "expected the deleted-parent arm, got: {err}"
        );
        let _ = fs::remove_dir_all(&base);
    }
}
```

sha256 of the appended block: **`c99aa535f119a48e24deabe8c6f969d59e8afbf2c7b152362066aad2dfbde876`**
(83 lines; includes the shim). The probe took **1.70 s** standalone, so a landed version is cheap;
the shim needs the same `TEST_HOME_LOCK` (it does) and must be skipped on Windows (`/bin/sh`
assumption) — the landing test should use a `cfg(unix)` shim or an injected runner.

### 3.3 P3 — `debug.rs:114,116`: `Phase::Ready && !owns_agent()` is a *production* state

The row says the state is "producible only by an externally started agent" and that
`agent_host.rs` has "no test-only phase setter". Both are true and both are irrelevant: the
production `AgentHost::run` **sets `Phase::Ready` with `owned == false` itself** when
`future_rpc::connection::select_agent` finds an external agent (`agent_host.rs:196-204`). The test
harness's mock agent is a **real TCP tonic server** whose address is published in
`FUTURE_AGENT_GRPC_ADDR` (`test_support.rs:288-315`), so `host().start(addr)` walks that branch
without any setter.

The probe asserts `Ready` **and** `!owns_agent()` before the call, then asserts the surfaced error
is the guard's message — which is line 116's literal. Coverage: **114 → 1, 116 → 1**, and `117=0`
(the `desktop_shutdown::stop` after the guard did not run), `113=1` (the condition was evaluated).

```rust
#[cfg(test)]
mod rev_probe {
    use super::*;
    use crate::auth_store::test_support::HomeGuard;

    /// REVIEW PROBE (REV-TAURI-APP): `commands/debug.rs:114,116` claims it needs
    /// `Phase::Ready && !owns_agent()` and that "agent_host.rs has no test-only
    /// phase setter". The production `run()` sets exactly that state when
    /// `select_agent` finds an EXTERNAL agent (agent_host.rs:196-204); the mock
    /// agent is a real TCP service, so the state needs no setter.
    #[test]
    fn rev_pr_external_agent_blocks_the_environment_switch() {
        let _home = HomeGuard::new("cmd-env-external-rev");
        crate::store::initialize_app_store().expect("init store");
        let mock = crate::agent_bridge::test_support::mock_agent();
        let addr = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock agent address");
        crate::agent_host::host()
            .start(addr)
            .expect("an external agent is selected");
        assert_eq!(
            crate::agent_host::host().phase(),
            crate::agent_host::Phase::Ready
        );
        assert!(!crate::agent_host::host().owns_agent());
        let app = tauri::test::mock_app();
        let err = set_future_environment_with(app.handle().clone(), "test", |_| Ok(()))
            .expect_err("an external owner must block the switch");
        assert!(
            err.to_string().contains("Stop the external Agent"),
            "{err}"
        );
        let _ = crate::agent_host::host().stop();
        drop(mock);
    }
}
```

sha256 of the appended block: **`e1d7c561c92f81400d6b1b73408aec637a987f40f66a5f8053c22257cff35e35`**
(35 lines). A landed version must reset the process-global `HOST` (`stop()` is done) and must run
in a `--test-threads=1`-safe position, because `HOST` is a `LazyLock`.

### 3.4 P4 — `update_recovery.rs:36-38`: `capture()` finishes; the category is a cost claim

The row says `capture()` "fingerprints the test binary's own build tree — measured at §S2.6/S3.3 as
**not finishing in 600 s**". Measured here, it **does** finish: run alone under the instrumented
binary it reports **`elapsed=332.252785958s ok=true files=8115`** (log
`$R/capture-alone.log`); the four-test run took **336.78 s total**, of which the other three probes
take ~4.5 s combined (0.01 s, 1.70 s, 0.02 s). So `capture()` walked **8 115 files** of an 8.7 GB
instrumented `deps/` tree and returned `Ok` in **332.25 s**. This is a **completed** fingerprint,
not a partial one: the test asserts `Ok`, line 38's `}` is hit, and `33/34/35` — the
`verify_saved` tail — stay 0, so the run is the `capture()` call and nothing else.

```rust
#[cfg(test)]
mod rev_probe {
    use super::*;

    /// REVIEW PROBE (REV-TAURI-APP): `commands/update_recovery.rs:36-38` claims
    /// `capture()` is `unreachable-in-this-environment` because it fingerprints
    /// the test binary's build tree ("not finishing in 600 s"). Measure it.
    #[test]
    fn rev_pr_capture_fingerprints_the_running_tree() {
        let _home = crate::auth_store::test_support::HomeGuard::new("rev-capture");
        let started = std::time::Instant::now();
        let result = ProgramFingerprint::capture();
        let elapsed = started.elapsed();
        let files = result.as_ref().map(|f| f.files.len()).unwrap_or(0);
        eprintln!(
            "REV_PROBE capture elapsed={elapsed:?} ok={} files={files}",
            result.is_ok()
        );
        assert!(result.is_ok(), "capture failed: {:?}", result.err());
    }
}
```

sha256 of the appended block: **`234e4e567a848365f12f53cf94f8590a850481d98cdf4d6350cf476e300d64c4`**
(21 lines). **This is the campaign's `cli.rs:590` pattern again** (REV-AGENT: "cost-prohibitive"
then covered in ~5 s): 332.25 s is a cost, not an impossibility. The category `unreachable-in-this-
environment` is false; whether a 5.6-minute test belongs in the suite is a *policy* decision for
the supervisor, and it should be registered as such (an explicit exclusion), or landed with a
smaller `current_exe`-shaped fixture. Note the timing is tree-dependent: the hash walks
`target/debug/deps` (macOS `extract_path_from_executable` returns the parent for a non-`.app`
binary), so the number scales with the build directory, not with the code.

## 4. The registration falls (line genuinely uncovered; the tag cannot stand as written)

These are FALLS under REVIEW.md rule 5a: the stated mechanism is weaker than the category it
claims, or the category is not one of the four policy categories at all.

### 4.1 `src/tasks.rs:217-219, 223, 306, 371, 426, 436, 454-455, 466` — 11 lines, no mechanism covers them

The ledger row (`| src/tasks.rs:217-219, …, 1729 | 95 | §S9.6 (the two #[ignore]d paid harnesses, kept) |`)
attributes **the whole 95 lines** to two `#[ignore]`d paid harnesses. But these 11 lines are
**production failure arms** in `tick`/the executor — `Store::open` failure inside the spawned run
(`217-219`), task-execution failure (`223`), `record_cleanup_error` while stopping (`306`), and the
`bind_task_thread` / `update_run` / `get_thread` / `prepare_prompt` error returns (`426, 436,
454-455, 466`). They are not in any `#[ignore]`d test, and the record says so elsewhere:

* §4.2 (`module-tauri-appdata.md:335`) lists these lines as **OPEN — reachable**, with concrete
  fixtures: "fail `Store::open` inside the spawned run (a home that is a file), script a refusing
  mock for `create_new_session`/`bind_task_thread`/thread load";
* §S11.4 (`:3675-3679`) explicitly leaves the grouping unsettled: "the successor should settle it
  by reading `tasks.rs:217-219`";
* §S11.9 (`:3998`) repeats it as the next useful check.

So the row's mechanism does not cover 11 of its 95 lines, and the record's own work list says they
are reachable. I did not land a fixture for them in this segment (the `Store::open`-fails window
is between `tick`'s own successful open at 196 and the spawn; the mock-driven ones at 426/436/454/
466 are the cheap half). **Verdict: FELL for the 11 lines; the other 84 (the two ignored harnesses
plus failure-only/artifact lines) SURVIVED.**

### 4.2 `commands/remote.rs:69-70` — 2 lines, a side-effect exclusion, not unreachability

The row: "the OS opener (`open` resolves to `/usr/bin/open`, so running it opens the developer's
browser)", category `unreachable-in-this-environment`. But `open_url` (67-70) is a plain `pub fn`;
any test that calls `open_url("https://example.com")` executes 69-70 — on a headless host as well
(`that_detached` returns after the spawn, and even a failure still runs the closure). The missing
thing is **the reviewer's/test author's willingness to open a browser**, not a host capability.
That is a deliberate exclusion and must be registered as one (like an `#[ignore]`), not as an
environmental impossibility.

### 4.3 `commands/update.rs:388-395` — 8 lines, `platform-unmeasured` without the platform

The row is a 164-line bundle whose text says "the `platform-unmeasured` arm" and stops there. The
arm is `!platform_allows_updates()` (`update.rs:387`): on macOS it is `true`, so the 8-line status
block is dead **here**. `platform_allows_updates` (`update.rs:250-273`) shows the false branches:
**Linux** where `is_debian_deb_install()` is false, **Windows** where `bundle_type()` is not
`NSIS` (the portable ZIP), and any non-macOS/Windows/Linux OS. So the shape is **(b)** — the line
compiles here, another platform's runtime decides — and the missing condition is **"a Linux host
whose install is not a Debian package, or a Windows portable-ZIP build"**. The category is right;
the registration does not name the platform or the condition, which the ledger's own rule requires.
**Verdict: FELL as a registration; the other 156 lines of the bundle survived inspection** (Wry
`AppHandle` commands, the install modal over `sync_channel(0)`, attribute lines, `)?;`/brace
artifacts, the CDN const).

## 5. Per-row verdicts

`S` = SURVIVED, `F` = FELL (measured), `Fr` = FELL (registration), `U` = UNVERIFIABLE.
Neighbour counts are from `/tmp/appdata-taub-s23-normalized-lcov.info` at the row's line.

| # | row | lines | verdict | basis |
|---|---|---|---|---|
| 1 | `agent_bridge/review.rs:289-293` | 5 | **F** | P2 measured 0→1; `git` shim is a deterministic seam |
| 2 | `commands/debug.rs:114,116` | 2 | **F** | P3 measured 0→1; `External` selection sets the state |
| 3 | `commands/update_recovery.rs:36-38` | 3 | **F** | P4 measured 0→1; `capture()` completes in 332.25 s (8 115 files) |
| 4 | `agent_bridge/client.rs:215-218,248-249` | 6 | S | `JoinError` needs the spawned task to panic/abort; `verify()`/`connect_verified()` have no such input (214=824/219=824, 246=367/250=361) |
| 5 | `agent_bridge/client.rs:232` | 1 | S | artifact: brace after the `!matches!` return (231=3, 235=367; §S26.2's three-run oscillation) |
| 6 | `agent_bridge/client.rs:1016` | 1 | S | artifact: `matches!(` call-site line inside `assert!` (1014=1,1015=1,1017=1) |
| 7 | `agent_bridge/config_observer.rs:97` | 1 | S | `APP_HANDLE` is `OnceLock<AppHandle<Wry>>`; a `MockRuntime` handle is a different type (96=4,98=4) |
| 8 | `agent_bridge/headless.rs:149-150,160-161` | 4 | S | same `JoinError` family for the prompt task (148=19/151=16) |
| 9 | `agent_bridge/import.rs:188,596` | 2 | S | artifacts: fallback brace (186=22,189=2) and `)?;` tail (594=11,595=11,597=11) |
| 10 | `agent_bridge/import.rs:559` | 1 | S | artifact: closing brace of a covered `if let` (558=9,560=9) |
| 11 | `agent_bridge/observer.rs:702` | 1 | S* | `#[cfg(not(test))] drop(task)`; see §6.1 for the correction |
| 12 | `agent_bridge/queries.rs:168` | 1 | S | artifact: the `||` line (167=4,169=2,170=2) |
| 13 | `agent_bridge/run_control.rs:179` | 1 | **F** | P1 measured 0→1 |
| 14 | `agent_bridge/run_control.rs:190` | 1 | S | `get_session_entries` builds `json!({"entries": entries})` itself (`queries.rs:235`), so the key is always an array; second call to the same guard is impossible |
| 15 | `agent_bridge/stream.rs:439` | 1 | S | artifact: brace between executed lines (438=39,440=39) |
| 16 | `agent_bridge/test_support.rs:547` | 1 | S | artifact: `match result {` header; the `_ => break` arm at 558 is covered (220) |
| 17 | `agent_bridge/test_support.rs:554-557` | 4 | U | the `DatabaseBusy` retry arm; the record's "no test can … without a concurrent task" is weaker than it reads (a test can spawn the task); see §7 |
| 18 | `agent_bridge/test_support.rs:771` | 1 | S* | test cleanup `Some(saved)` arm; `saved` is captured **after** `remove_var("HOME")` (761-762), so it is always `None`; mechanism now supplied |
| 19 | `agent_events.rs:54` | 1 | S | `APP_HANDLE` Wry barrier; None arm covered (52=61,55=61) |
| 20 | `commands/agent.rs:5` | 1 | S | `#[tauri::command]` attribute line; body is covered (6=1) |
| 21 | `commands/agent.rs:78` | 1 | S | same-poll `try_recv` needs `replica_lease::collect` to complete in one poll; the fast-completion test leaves it 0, no seam in another module |
| 22 | `commands/debug.rs:15,17-19,97,99-101` | 8 | S | concrete `tauri::AppHandle` (Wry) in `clear_app_data`/`set_future_environment`: `generate_handler!` cannot instantiate for `MockRuntime` |
| 23 | `commands/files.rs:283-288` | 6 | S | `prepare_image_preview(app: tauri::AppHandle)` — same concrete-handle barrier |
| 24 | `commands/files.rs:1024` | 1 | S | by construction: `old_home` is read at 1017 **after** HOME is removed at 1016 → always `None` |
| 25 | `commands/login.rs:9` | 1 | S | attribute line of an `AppHandle<R>`-only command (10=2,11=2,12=2) |
| 26 | `commands/login.rs:205,221` | 2 | S | artifacts: `matches!(` call-site lines in asserts (204/206, 220/222 covered) |
| 27 | `commands/remote.rs:69-70` | 2 | **Fr** | side-effect exclusion; `open_url` is directly callable (§4.2) |
| 28 | `commands/remote_peer.rs:81` | 1 | S | attribute line of an `AppHandle<R>`-only command (82=1,83=1,84=1) |
| 29 | `commands/remote_peer.rs:301` | 1 | S | artifact, proven by renumbering (298→301, same construct, same 0; 296-300=2, 302=2) |
| 30 | `commands/skills.rs:96` | 1 | S | attribute line of an `AppHandle<R>`-only command; the IPC harness fails its own assertion (94=2,98=1) |
| 31 | `commands/update.rs` bundle | 164 | S×156 / **Fr**×8 | platform arm 388-395 (§4.3); the rest inspected as Wry/install/artifact |
| 32 | `commands/update_journal.rs:150` | 1 | S | artifact: `)?;` of a covered `write_json_atomic` (148=1,149=1) |
| 33 | `scheduler/mod.rs:205-211,215-220,223-231,513` | 23 | S | job bodies sleep one full interval on the process runtime; `start()` is inert (the record's own test at 533-548); 513 is an assert-message artifact |
| 34 | `shadow_review/snapshot.rs:81-91` | 11 | S | by construction: `capture` filters at 39 and `stage` filters the same list again at 224 with the same pure function → `paths.len() != staged.len()` cannot hold (record: 66 `Ok` runs, block never entered) |
| 35 | `shadow_review/snapshot.rs:234-245` | 12 | S | same double-filter: the retry's `filter_ignored` returns `newly_ignored = 0` (224), so 235-245 is dead |
| 36 | `store/approval_assessments.rs:60` | 1 | S | artifact: `)?;` tail of the covered owner query (59=1,61=1) |
| 37 | `store/approval_assessments.rs:169` | 1 | S | failure-only test assertion; a hostile store fires it and fails the test (§S17.2 measured) |
| 38 | `store/threads.rs:1648` | 1 | S | test-only `remove_dir_all(…).expect(…)` in a cleanup step; failure-only. **At HEAD this line is not the uncovered one** (250/269/1840) — see §0.3 |
| 39 | `store/workspaces.rs:60,936` | 2 | S | 60 is a `)?)` artifact (59=27,61=27); 936 is a test's failure-only `panic!` (935=2) |
| 40 | `src/tasks.rs` bundle | 95 | **Fr**×11 / S×84 | §4.1; the 84 = two `#[ignore]`d paid harnesses + accepted failure-only/artifact lines |
| 41 | `src/tasks.rs:1878` | 1 | S | failure-only `panic!`, measured firing (§S15.5b) |
| 42 | `src/tasks.rs:1939` | 1 | S | failure-only `panic!` (1938/1940 covered) |
| 43 | `commands/update_recovery.rs:180,213` | 2 | S | failure-only assertions (§S20.2): 180 runs only if a FIFO is wrongly accepted; 213 only if the assert fails |

\* survived with a mechanism correction (§6).

**Counts.** Measured falls **4 rows / 11 lines**. Registration falls **3 rows / 21 lines**
(11 + 2 + 8). Unverifiable **1 row / 4 lines**. Survived **35 rows / 342 lines**.

### 5.1 The token that would invalidate each surviving family

A waiver that cannot be invalidated is a restatement, so each surviving mechanism names the change
that would revive the line:

| family | invalidation token |
|---|---|
| `client.rs` / `headless.rs` `JoinError` arms | an `abort()`/`JoinHandle::abort` on those spawned connection tasks, or a runtime shutdown path in a test — either makes `JoinError` producible |
| concrete-`AppHandle` / `APP_HANDLE<Wry>` barriers (`debug`, `files`, `login`, `agent`, `remote_peer`, `skills`, `config_observer`, `agent_events`) | widening the parameter to `AppHandle<R>`/`impl Runtime` (the generic split the record already uses for `prepare_image_preview_with`), or a `MockRuntime` that the IPC harness can drive |
| `files.rs:1024` | moving `old_home`'s capture above `remove_var("HOME")` (then the arm runs) |
| `run_control.rs:190` | a `get_session_entries` variant whose `entries` is not a `Vec` (e.g. a lazy/lean page) — the guard defends a shape the *helper* rules out today, not one the protocol rules out |
| `snapshot.rs:81-91, 234-245` | removing the first `filter_ignored` at `capture:39`, or making `filter_ignored` non-pure (time/state dependent) — both would make `stage`'s second filter find a difference |
| `scheduler` job bodies | running the jobs on a runtime the test can advance (`tokio::time::pause/advance`), i.e. replacing the `Instant::from_std`+`crate::runtime::spawn` pair |
| `agent.rs:78` | a seam in `replica_lease::collect` that answers synchronously (or removing that await) |
| `test_support.rs:554-557` | a `cfg(test)` fault hook in `initialize_app_store`, or a way to hold `app.db`'s write lock inside the 536→545 window (see §7) |
| `remote.rs:69-70` | nothing — it is reachable now; only the side-effect decision stands in the way |

## 6. Record corrections (line survives; the stated reason is wrong or incomplete)

1. **`observer.rs:702`.** The mechanism says the line "does not exist in a coverage build at all",
   yet the pinned report **has** a `DA:702` record (0). The precise fact is that the *lib-test*
   compilation takes the `#[cfg(test)]` sibling (697-700, 172 hits) while the **bin/integration
   compilations** read the `#[cfg(not(test))]` line — the record's own retraction at `:1889-1894`
   already says this, but the §S18.5 row text still carries the stronger claim.
2. **`test_support.rs:771`** has no mechanism in any section; the mechanism is that `saved` is
   captured after `remove_var("HOME")`, so the `Some` arm is dead by construction.
3. **`test_support.rs:554-557`.** §S16.3b's falsifiable prediction — "no test can make
   `initialize_app_store` return `DatabaseBusy` for a `TestHome::new` path **without a concurrently
   running task**" — is weaker than it reads: a test **can** spawn the concurrent task (the window
   is `remove_dir_all` at 536 → `initialize_app_store` at 545, and a thread that opens the new
   `app.db` and takes a write lock fits in it). It is a race with a real (if narrow) injection
   surface, so the category should be re-argued (or a probe attempted) rather than called
   `unreachable-in-this-environment` on the strength of "no test can schedule it".
4. **`scheduler/mod.rs:205-231`.** The mechanism ("calling `start` runs the production jobs on
   their real clocks") is right for the *network* actions, but the precise reason the closures are
   dead is `sleep_until(job.next_due())` at `:111` plus `note_explicit_trigger` not waking a
   pending sleep (`:113-116`) — i.e. the missing thing is **time advancement on the process
   runtime**, not the clock itself.
5. **`commands/remote.rs:69-70`** should read as a deliberate side-effect exclusion, not an
   environment property (§4.2).
6. **§S18.5's `tasks.rs` row text** is a one-line summary of a 95-line bundle with at least three
   distinct mechanisms (ignored harnesses, production failure arms, failure-only assertions); the
   bundle should be split into three rows (§4.1).

## 7. What I did not reach, and what it would need

* **`test_support.rs:554-557` (4 lines).** I did not get a `DatabaseBusy` hit. To settle it: a
   thread that polls the minted `TestHome` path (pid + tag + process-local counter) and opens
  `app.db` with `BEGIN IMMEDIATE` as soon as it exists, repeated across `TestHome::new` calls; or a
  `cfg(test)` fault hook in `initialize_app_store`. I am **not** claiming it falls — I am claiming
  the record's category is not established by its own argument.
* **`commands/update.rs`'s 156 non-platform lines.** Inspected, not individually probed: the
  install block's `sync_channel(0)`/`recv` needs a native confirm modal; several lines are
  `#[tauri::command]` attribute lines of concrete-`AppHandle` commands. A successor should attack
  `acknowledge_update_repair` (the record's own first question: "can a mock app answer
  `blocking_show`?") first.
* **`scheduler`'s three job bodies.** A `tokio::time::pause()`/`advance()` probe would need the
  jobs to run on the test runtime instead of `crate::runtime::spawn` (a seam in `runtime`, not in
  `scheduler`).
* **The 84 surviving `tasks.rs` lines.** The ignored harnesses are a policy choice (paid model /
  live agent) and I did not run them; the failure-only lines were accepted on the record's
  measured firing, not re-measured.

**Flakes relied on: none.** P1-P3 are deterministic (a pragma, a shim, a mock service); P4 is a
single timed call. No before/after rate from `.future/cov100/flake-rate.sh` is needed. The one
timing-sensitive number (332.25 s) is a *duration*, not a pass/fail.

## 8. What the numbers become

At the record's revision (`42cb69d10`), 378 = 368 WAIVED + 10 OPEN:

* **10 OPEN → 10 FELL** (5 + 2 + 3), all measured, plus `run_control.rs:179` (1) — all **11 are
  coverable**. A unit that lands those probes moves the residual to **378 − 11 = 367**;
* the 21 registration lines stay *uncovered* — a relabel does not change the count — but they
  cannot stay as written: the 11 `tasks.rs` lines are **reachable work** (§4.1), `remote.rs:69-70`
  is a **deliberate exclusion** (§4.2), and `update.rs:388-395` is `platform-unmeasured` **with
  the platform named** (§4.3);
* if the 11 `tasks.rs` lines are then covered, the residual becomes **367 − 11 = 356**, with 8
  platform lines and 2 exclusion lines still accounted for;
* do **not** quote 378 for HEAD: at HEAD the same scopes show **389** indicative DA:0 lines over a
  changed tree (§0.2). Re-pin first, then quote.

## 9. Provenance

| item | path / value |
|---|---|
| scratch tree | `/tmp/rev-appdata-s1-20261017` (kept for verification) |
| base revision | `42cb69d10`; HEAD at review time `26fc0a0d3` |
| instrumented run | `$R/probe2.log` — 4 passed, 0 failed, **336.78 s** |
| lcov for the AFTER numbers | `/tmp/rev-appdata-s1-20261017/probe.info` (`/private/tmp` block) |
| pinned BEFORE report | `/tmp/appdata-taub-s23-normalized-lcov.info` |
| probe hashes | run_control `51112b8b…`, review `c99aa535…`, debug `e1d7c561…`, update_recovery `234e4e56…` |
| write target of this review | `docs/testing/review-tauri-appdata.md` only; no record/source edit |

## 10. Handoff for the next unit

* **Artifacts:** this report; scratch tree `/tmp/rev-appdata-s1-20261017` (probe sources + hashes
  above, `probe.info`, `probe2.log`, `capture-alone.log`); the pinned BEFORE report is untouched.
* **New evidence vs the previous attempt:** four measured 0→1 transitions on the exact lines the
  ledger called unreachable (11 lines), and three registrations that the record itself contradicts
  (11 tasks lines were already listed OPEN with fixtures in §4.2; §S11.4 says the grouping is
  unsettled). Nothing here restates a prior review: `review-tauri-appdata.md` did not exist before
  this unit.
* **Rejected approaches:** (a) reviewing at HEAD instead of `42cb69d10` — the pinned report is a
  42cb line table, so HEAD verdicts would be unfair to the rows; the drift is reported separately
  (§0.2) instead; (b) reading coverage from the reused target dir's profraw without checking the
  file path — it carries another run's records and a path-blind read would attribute them to this
  probe; (c) a timing-based delete for P2 — replaced by the `git` shim, which is deterministic;
  (d) manufacturing a `Phase` setter for P3 — the production `External` branch already produces the
  state.
* **Next useful check:** (1) re-pin the record to a HEAD measurement and re-read every anchor
  (§0.3); (2) a `DatabaseBusy` probe for `test_support.rs:554-557`; (3) the 11 `tasks.rs` lines,
  starting with the mock-driven `426/436/454/466` (the `Store::open`-fails window at 217-219 is
  much narrower than §4.2 assumed); (4) decide, as a policy question, whether a 5.6-minute
  `capture()` test is acceptable or the line becomes a documented exclusion.

