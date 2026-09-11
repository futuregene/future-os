# Bug-hunt remediation — integrated review index

The 2026-09-11 audit's **224 original finding IDs are all accounted for** in the four ledgers below (mechanically reconciled against the original INDEX.json). This is not a claim that there were 224 distinct production bugs: duplicates, refutations, latent unsupported cases and externally unverified assertions have separate dispositions. Confirmed reachable repairs, regression evidence and cross-owner follow-through are recorded individually.

| Scope | Original IDs | Evidence |
|---|---:|---|
| Agent, LLM, sandbox, session/persistence and assigned concurrency | 79 | [Agent ledger](./fix-agent.md) |
| Loop/TUI and supervisor/verifier additions | 55 | [Loop/TUI ledger](./fix-loop-tui.md) |
| CLI/browser/channels/shared RPC and security overlaps | 61 | [CLI ledger](./fix-cli.md) |
| Desktop/mobile/shared frontend packages/scripts | 29 | [Apps ledger](./fix-apps.md) |

[Supervisor review](./supervisor-review.md) retains the chronological failures, partial checkpoints and independent corrections. Earlier checkpoint sections there are historical; this index summarizes the integrated candidate after merging `origin/main` at `1294b628`.

## Integrated changes

- Conservative Manual shell approval, bounded/cancellable shell execution, UTF-8-safe rules/UI/card handling, bounded SSE event memory and linear chunk scanning.
- Queue-budget/snapshot isolation, ordered persistence recovery, partial-stream repair, accurate counters and strict configuration/address boundaries.
- Consistent loop closure/deferred clocks, canonical lease/owner folding, completion release, durable worker attribution, safe WebUI handler encoding and deterministic projections.
- Channel sender admission, safe received filenames, bounded downloads/ingress, ordered starts, origin-bound approvals and error/supersede cleanup.
- CLI error/timeout/input handling, browser transactions/navigation cleanup and additive RPC event metadata with unchanged existing field numbers and event-data dual-write.
- Frontend approval/error/paging/rendering corrections, qualified model identity, mobile replay/upload guards, catalog-generator failure protection and cross-platform scripts.

Independent integration review additionally corrected the Windows TOKEN_USER/SID storage mismatch, an introduced decorated-Unicode input offset panic (actual failing regression before repair), and a test lifecycle race: ambient Tauri observers are now aborted and drained before a new test HOME/database is published. Windows atomic lease folding reads through the already-locked handle instead of reopening a mandatory-locked file. The upstream remote-business extraction is preserved; the model identity repair follows its moved helper.

## Local validation completed

Pinned Rust 1.97.0, isolated test HOME, appropriate fd limit:

- All six workspace crates: `cargo fmt -p … --check`, `cargo clippy -p … --all-targets -- -D warnings`, and full `cargo test -p …` passed, including integration/doc targets. Long combined commands hit tool execution bounds; the incomplete final TUI/loop parts were explicitly rerun to completion, not counted as successful partial runs.
- Tauri backend: fmt/clippy all-targets and full tests passed: **1146 library + 1 binary test**, one doctest ignored. Two earlier full runs exposed SQLiteBusy during fixture initialization; source lifecycle correction, not serial-test flags or weakened assertions, preceded the passing full run.
- Desktop: tsc, ESLint, **101 files / 870 tests** passed.
- Mobile: tsc, ESLint, **49 suites / 699 tests** passed. A stale test fixture was updated for upstream connectionPresentation; worktree-local dependency mappings and the normal generated version prerequisite were supplied without editing main's dependencies.
- Actual-source Node checks: graph geometry 4 fixtures, WebUI HTML/JS attribute contexts 16 cases, console level forwarding.
- Offline Python generator/profile/Android checks passed. Protobuf code regenerated with `make generate-proto`; both checked-in generated files remained identical.

**Workflow changes are deferred at the user's explicit request.** This PR does not modify `.github/workflows/`; all test source files remain. Existing CI still provides its original cross-platform compile checks, but the proposed additional Windows installer, token/PowerShell/process and loop lifecycle test execution is not enabled by this PR. Native execution of those new tests remains unverified and must not be inferred from macOS passes. The proposed workflow patch is preserved separately for later work. Earlier worker/supervisor ledgers mentioning added CI steps describe the implementation history, not this PR's final workflow scope. An optional extra model-based native reviewer stalled/failed at the LLM service and produced no accepted report; it is not counted as a completed review.

## Explicit limitations and operator changes

- Windows localized denial attribution (`w04-5`), upstream cumulative/incremental usage semantics (`w09-4`) and platform JWT lifetime (`w30-3`) still require external evidence. No guessed privileged retry, accounting change or TTL was introduced.
- Dormant/unsupported-producer APIs and refuted assertions are listed with reasons in each ledger; they are not silently counted as fixed production defects.
- Shipped catalog: **158 output-only additions** preserve all 3826 rows/order/other fields and exclude **110 source-confirmed non-chat models**. [Provenance](./model-output-provenance-20260911.json) documents sources; [432 unmatched identities](./model-output-unmatched-20260911.json) remain explicitly unclassified.
- DingTalk now requires `sender_allowlist` (empty denies all; explicit `"*"` means trust all). See channel configuration docs before upgrading. Old Feishu approval cards after bridge restart safely report not delivered rather than binding to another session.
- Default full permission/OS-backed sandbox policy is unchanged; Manual shell now requires approval for external commands/file reads rather than trusting executable basenames. The path-aware read tool remains available.

A single PR carries the integrated code fixes and test sources, excluding the explicitly deferred workflow changes. PR/CI/merge completion is reported from GitHub state, not inferred from these local passes.
