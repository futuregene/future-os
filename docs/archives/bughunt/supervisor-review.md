# Supervisor review — first remediation checkpoint

## State (2026-09-11)

**PARTIAL, not ready for the single final PR.** Goal `goal_bughunt_fix_20260911`, supervisor session `20260911-122531-184689`. The authorized four-worker segment ran from 04:30:30 to 05:30:30 UTC. The deterministic monitor stopped this goal's workers and created continuation gate `todo_60242d8b0774`. All four worker session projections subsequently report `streaming: false`. No worker is relaunched without renewed user authorization.

The original 224 findings are partitioned as 79 agent, 55 loop/TUI, 61 CLI/channels/RPC, and 29 apps/build findings; duplicates and speculative mechanisms are **not** counts of distinct repaired production defects. Supervisor/verifier additions are tracked separately in worker ledgers.

## Reviewed and integrated: apps/build

Reviewed commits `6ae8f550` and `2d062dcd`; merged into `claude/bughunt-all`. Read the 29-row ledger and source/test diffs. In particular checked approval-finally and editable Escape, error UI boundaries, search-window narrowing, stable output dependencies, removal of dead generation plumbing without removing the actual baseline reconciliation, source-preserving markdown depth fallback, numbered token boundaries, bounded mobile replay retention/retry, upload EOF cancellation, structured error classification, patch header/hunk separation, home-path validation, Windows static-server path rejection, option-terminating macOS opener, profile isolation and installer normalization.

Independent supervisor execution (not merely copied worker claims):

- `git diff --check origin/main...HEAD`: PASS for apps slice.
- Actual `scripts/test-generate-models.py`: PASS, including None/empty sources and preservation of existing catalog. No network calls made by the mocked test.
- Actual `scripts/test-profile-isolated.py`: 2 tests PASS; child home isolation and exit-code propagation; no live agent started.
- Actual `scripts/test-android-config.py`: PASS, four platform/ABI subcases using mock SDK tools.
- `bash -n scripts/start-mobile-android.sh scripts/agent-profile-bench.sh`: PASS.
- Integrated Desktop `tsc --noEmit`: PASS; ESLint over all `src/**/*.{ts,tsx}`: PASS.
- Integrated Desktop Vitest: **98 files / 862 tests PASS**. An earlier targeted pass also passed 10 files / 72 tests. Temporary Vite aliases explicitly pointed shared packages at the integration worktree, not the ancestor main checkout.
- Integrated Mobile targeted Jest: **3 suites / 111 tests PASS** (`syncEngine`, `files`, `ComposerDock`), with worktree-local shared package mappings and the existing Expo-compatible React dependencies.

Worker-reported full Mobile and Tauri passes remain attributed to the worker; they have not been independently re-executed by the supervisor at this checkpoint. Worker documented an initial Tauri timeout and a DatabaseBusy failure before an unchanged full rerun passed; this history is retained in `fix-apps.md`.

Scope accepted locally, **not** a Windows-native or live-service certification. Still outstanding: offline PowerShell installer regression on Windows; native profiling/path checks; platform JWT TTL evidence (w30-3); real journal/publish timing evidence for the mobile replay incident. Correctly escaped UNC parser counterexample and w29-6 permanent-failure refutation are justified; no dangerous path reinterpretation or create_new weakening was integrated.

## Round 2: model identity / modality pipeline / Windows script CI review

User authorized another 120 minutes, deadline epoch1789115561 (16:32:41 local). Reviewed and integrated `6d96e393`, `bbc5efe1`, `56384018`: provider-qualified identities on actual Desktop/Mobile command paths, preserved output-modality schema/generator/reader, text-output default selection, attachment-only image boundary, and offline script tests inside the existing required per-platform CI statuses. Existing job/check names and script-only checkout/skip conditions were inspected; Windows execution still awaits the final PR.

Independent supervisor checks on the integrated source:

- All three actual offline Python regression scripts PASS (generator 3 tests; profiling 2; Android platform fixtures).
- Desktop full `tsc --noEmit` and ESLint PASS; new identity/helper/hook tests **4/4 PASS**.
- Mobile identity/controller/outbox tests **61/61 PASS**. The first attempt failed one suite because this fresh integration worktree lacked generated `src/version.generated.ts`; after running the existing `npm run gen-version` build prerequisite, all three suites passed. This was not a source-code test failure and no assertion was weakened.
- Shared package aliases explicitly point at integration source; scratch configs/dependency links/generated version are removed after checks.

**Additional independent finding: pipeline-only repair does not close w10-BUG6 against the shipped catalog.** Old entries still lack output metadata and therefore receive the compatibility text default. The apps round-2 delivery is recorded as rework for this remaining scope, while the reviewed code is integrated. Follow-up `todo_64d9f49b8ea7` resumes the same worker within the unchanged deadline to obtain public-source provenance and make an output-only metadata repair for existing catalog entries. No wholesale refresh, record insertion/removal/reorder, endpoint/pricing/context changes, model-name inference, credentials or token capture is authorized. Exact unmatched coverage must remain explicit. Final coordination depends on this follow-up as well as all other producing tasks.

## Round 2: agent integration and independent Windows correction

Integrated agent commits through `b418d41a` for cross-crate checking, retaining both model-modality/default-filter changes from apps and the agent's resolver/context/cache-order changes. The worker's automatic full-crate validator passed (1749 library tests plus 42 integration tests, 10 ignored native/other cases). This is not a native Windows pass or independent supervisor re-execution of the whole combined suite. Read the reconciled 79-row ledger and inspected key diffs for shell approval/cancellation, FIFO recovery, bounded SSE, provider snapshots, repair ordering, backpressure and Windows token/rewrite paths; `git diff --check` passed.

**Independent review caught two concrete Windows defects in the delivered patch:**

1. `everyone_sid()` allocated `Vec<usize>` then constructed `OwnedSid(Vec<u32>)`, a cfg(windows)-only compile error. Corrected its SID storage to DWORD words.
2. `current_user_sid()` allocated `Vec<u32>` for `TOKEN_USER`, which contains a pointer and needs machine-word alignment on 64-bit Windows. Corrected the query buffer to `Vec<usize>`. The worker ledger's claim that this was already machine-word-aligned was inaccurate.

Added `native_sid_buffers_are_valid_and_clone_byte_exact`, querying only the test process's TOKEN_QUERY identity and constructing the Everyone SID, with validity/clone checks. Added native Windows token, PowerShell stdin and shell status/descendant regressions to the existing required Windows CI build job. Preserve CARGO_HOME/RUSTUP_HOME before isolated HOME/USERPROFILE; fail immediately after each failing cargo invocation. These checks are **defined, not executed on this Mac**. `cargo fmt -p future-agent --check`, `git diff --check`, CI YAML parse/filter existence and native test-name checks pass after the correction. Combined full-crate/direct-consumer checks remain pending integration of all slices. No PR has been opened and no final native acceptance claimed.

## Other partitions — NOT independently accepted/integrated

- Agent: checkpoint commits `d4781920`, `1f677403`; ledger covers 79 assigned IDs. Worker explicitly reports partial implementation, dedicated-regression gaps, unresolved confirmed findings and native/protocol integration gaps. It reports 1719 library tests passing serially, but an earlier parallel cache-test failure still requires investigation and full crate integration/doc tests are unfinished. A loop delivery marked done is NOT acceptance; requires rework/continuation.
- Loop/TUI: checkpoint commit `d2905a9d`; two later modified files (`compat.rs`, `cli_registry_contract.rs`) remain uncommitted after stop. Ledger covers 55 originals plus SF/NF/verifier additions. It reports 847 TUI library tests and 7 loop regression tests passing, but full checks and several confirmed repairs remain unfinished. Do not reset the uncommitted changes.
- CLI/channels/RPC: 34 modified tracked files plus new regression/report files remain **uncommitted** at stop. Do not discard or claim reviewed. The ledger's initial rows are not up-to-date final dispositions. Need inspect diffs, finish checks and checkpoint commits before integration; do not count code churn as a repaired-finding total.

## End of the additional 120-minute window (16:32:41 local)

The deterministic second checkpoint stopped the remaining CLI and loop/TUI workers and created user continuation gate `todo_070dc3d252e1`. All four projections now report streaming=false. No PR exists and no automatic extension/relaunch is authorized.

Shipped-model follow-up `fd16677e` was independently inspected and integrated. Supervisor reran its exact offline generator validator (**4 tests PASS**) and mechanically compared old/new JSON: **3826 ordered records, all non-output fields unchanged, 158 output additions, 110 classified non-chat entries**. Seven inspected public-source fixtures, retrieval timestamps/hashes and **432 unmatched identities** remain explicit provenance/limits. The actual cycle_model output filter and real catalog/RPC regression were reviewed. Worker handoff was rejected for omitting the literal `commits` acceptance token despite the real commit existing. Supervisor manually repaired the evidence with the verified commit, coverage, executed tests and gaps; criteria were unchanged and no worker was restarted. Follow-up delivery `todo_64d9f49b8ea7` is verified for this local scope.

Attempting to change the older apps pipeline delivery from rework to verified was rejected by the loop API (`delivery is already resolved as rework`); that historical disposition is preserved, with the separately verified follow-up documenting resolution. Do not retry the state transition or edit ledger events.

Remaining at stop:
- Loop/TUI clean branch through `3b60af69` (after `05ca1f04`, `736830dd`, `d2905a9d`), not integrated/reviewed yet. Full final validation/ledger reconciliation must be checked; partial earlier passes are not enough.
- CLI/channels/RPC commits `dc642044`, `d9bff2ab`, `7e621a5c`, plus 14 modified files preserved uncommitted, including configure/auth/run, browser state, channels, decode and docs. Ledger records full RPC/channels success, CLI clippy success but a 719-pass/3-fail lib run; subsequent corrections and full integration/doc suite require verification. Do not discard dirty files or label these known failures resolved without evidence.
- Shared RPC additive proto changes require combined direct-consumer builds/codegen checks after integration.
- Cross-owner w32-BUG4 SSE per-line suffix movement is still explicitly handed to supervisor for repair; w32-BUG5 WebUI handler hardening may be in the final loop commit and must be inspected.
- Native Windows execution, integrated full checks and a single final PR with actual green required CI remain outstanding. Agent local passes do not cover the independent Windows correction introduced at integration.

## Resource accounting

Read-only session metadata for these four exact worker session IDs reports input token totals 23,504,426 + 27,730,610 + 23,470,340 + 24,781,031 and output totals 38,700 + 40,575 + 43,172 + 41,284. Inputs include repeatedly supplied/cached context; they are not unique source tokens. All four stored total_cost values are 0.0, and live usage also reports zero. **Actual Azure charges are not established by these counters; zero here must not be described as free execution.** Supervisor usage is not included in these four-worker totals.

## Next authorized segment should prioritize

1. Preserve/checkpoint CLI and the remaining loop edits; reconcile every disposition with current source and tests.
2. Close the explicitly unimplemented confirmed issues and dedicated-regression gaps, including cross-owner model identity/output modality, loop worker attribution/list-width fixes, browser transaction/lifecycle items and remaining protocol contracts.
3. Verify Manual shell policy changes carefully (do not reintroduce basename/option bypass), then independently check security/termination/lease fixes and cross-crate consumers.
4. Run all affected full crate/TS/mobile checks in a stable build environment. The first parallel segment repeatedly waited on/invalidated a shared Cargo target; schedule expensive builds deliberately rather than launching redundant compilations.
5. Add/run a Windows validation path for the offline installer test, without claiming ordinary cross-platform cargo check executes Windows-specific regression tests.
6. Only after coverage and acceptance gaps are reconciled: sync origin/main, create ONE PR, enable auto-merge immediately, block on required CI, resolve BEHIND/failures and clean only this task's worktrees after actual merge. No PR exists at this checkpoint.
