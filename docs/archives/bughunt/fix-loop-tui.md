# fix-loop-tui — final slice handoff

Session `20260911-123030-76ccff`; task `todo_79f44876e5c3`; goal `goal_bughunt_fix_20260911`.
Exclusive worktree `/Users/geilige/future-os/.worktrees/bughunt-loop-tui`, branch `claude/bughunt-loop-tui`, baseline `fc81c016`.

**Result: assigned slice implemented and locally validated, awaiting supervisor independent review/integration. This is not global goal/PR completion.** All 55 original findings plus SF/NF/V5/V6 additions and the owned WebUI w32-5 boundary have dispositions below. Latent mechanisms and refuted claims are not counted as repaired production defects.

Inputs inspected: read-only `/Users/geilige/future-os/.future/bughunt/{REPORT,CROSS-MODEL-VERIFICATION,w14..w20,w33}` and applicable V2/V3/V5/V6, GLM-b3/b4/b5/b8, KIMI and w32 evidence. Current code/call paths and actual regressions, not historical confirmed labels, determine the dispositions.

## commits

- `d2905a9d` — first-round lifecycle, scheduling, privacy and Unicode repairs, regression files and initial full-scope ledger.
- `736830dd` — preserved controlled-stop edits: manual CLI lease TTL expectation and clippy borrow fix.
- `05ca1f04` — durable run attribution, clock/deferred semantics, list widths and dedicated regressions.
- `3b60af69` — owner/bridge/help/mirror regressions, final legacy test corrections, WebUI attribute encoding, reviewed parity rows.
- A final report-only commit follows this write. No push, PR, branch merge, reset, extra workers, main/integration source edits or live-agent intervention was performed.

Round 3 resumed the clean `3b60af69` tree. The final round-2 check had already passed before the timer interrupted the report write; the cancelled write did not execute. No settled implementation was redone.

## tests

The required command **actually passed**, not a manual completion override:

```text
python3 /Users/geilige/future-os/.future/bughunt-fix/check-rust.py future-loop future-tui
PASS full scoped batch ['future-loop', 'future-tui'] at 16:31:13
```

Date: 2026-09-11. Both crates passed `cargo fmt -p … --check`, `cargo clippy -p … --all-targets -- -D warnings`, and **full `cargo test -p …` including integration and doc-test targets**. Loop: 402 library tests plus every integration binary; TUI: 850 library tests, 7 new integration regressions, 5 CLI smoke tests. Rust 1.97.0; isolated test HOME; fd limit 10240; serialized shared-target batch. These checks covered all source changes in `3b60af69`.

Additional actual executions:
- `node orchestration/loop/tests/graph_layout.mjs`: 4 actual-source geometry fixtures passed.
- `node orchestration/loop/tests/webui_attribute_context.mjs`: 16 actual-template HTML-parser/JavaScript-parser round trips passed (jsdom, no browser or live service).
- Actual `render_parity` executable against corpus and reviewed golden: **97/97 rows byte-identical**. Before editing the reference, exactly 95 rows matched; the two reviewed corrections are documented in `tui/tests/README.md`. No blanket snapshot acceptance.
- `git diff --check`: passed; source tree clean after `3b60af69`.

Meaningful failed/rejected attempts are retained here rather than erased:
- First-round short 120/180/300-second commands timed out around shared Cargo locks/rebuilds. They were not passes; serialized long-timeout batches replaced them.
- Naive `notify_one` retained an initial permit and spuriously cancelled the first subscription (six TUI tests exposed it). Rejected/reverted; the final implementation registers notifications before state reads and uses the existing version counter. All stream tests and the deterministic race test passed.
- Full loop tests exposed old expectations encoding the bugs: latest=oldest, manual CLI claim dies with the CLI, reversed inbox scope, and claim ignoring malformed ledger. These were corrected with explicit source semantics. Atomic claim now fails closed like canonical replay and does not append after corrupt input.
- The new bridge fixture initially expected success after an unsuccessful one-turn budget. Corrected the fixture to require the existing `max-turns reached` nonzero exit, not weaken the budget policy.

## coverage

All paths below are relative to this worktree. `L-reg` means `orchestration/loop/tests/bughunt_regressions.rs`; `T-reg` means `tui/tests/bughunt_regressions.rs`. All named Rust tests ran in the successful full batch.

| Finding | Final disposition, evidence, changed paths and regressions |
|---|---|
| w14 BUG-1 | **Fixed.** Manual completion can lack a passed validator receipt; `decision/goal_frontier/terminal.rs` now enumerates `unvalidated_deliveries`. L-reg `deferred_and_unvalidated_work_never_closes` and terminal-validator receipt contracts pass. |
| w14 BUG-2 | **Fixed.** `state::is_terminal` only accepts Done/Superseded, not due Deferred. Same regression and schema/decision contracts. Duplicate w17-1. |
| w14 BUG-3 | **Fixed.** Due Deferred re-enters its own class's open lane, including monitor/gate/blocker, via `state::*_at` and decision code. L-reg `every_due_deferred_class_reenters_its_open_lane_with_one_clock` covers all six classes. Coordination/user-action keep their Open peers' operator/manual semantics; they are not silently promoted to worker advancement. |
| w14 BUG-4 | **Fixed scheduling contract.** Supplied clock reaches packet, identity, lane/frontier, terminal judgement, dependency checks and summaries. L-reg fixed-epoch early/late matrix and injected-clock test pass. Rollout UUID remains nondeterministic metadata, not a scheduling clock. |
| w14 BUG-5 | **Fixed/narrowed.** Stalled monitor now emits `monitor_stalled`. Legacy/advisory variants retained: simplification intentionally removed forced policy exits. L-reg stalled branch and quota wire-code tests pass. |
| w14 BUG-6 | **Refuted as production defect / latent API breadth.** Valid packet constructors satisfy consistency invariants; defensive repair not firing on valid packets is correct. No consumer requires all nine historic dispositions. `arbitration_contract` passes all 20 cases, including actual fail-closed handling of inconsistent contracts. No scheduler rewrite. |
| w14 BUG-7 | **Fixed.** Character-count guard now matches character truncation in `decision::truncate`. L-reg Unicode truncation/metadata regression passes. |
| w14 BUG-8 | **Refuted as demonstrated production defect.** Identity skip already has `ok=false`, `should_run=false` and explicit registration instructions. Goal-level keep-active need not authorize this unregistered worker; no keep_active consumer was established. Identity tests pass. Do not park other registered workers by changing goal liveness speculatively. |
| w14 BUG-9 | **Fixed advisory ordering, production incident not claimed.** Timestamps are captured before append lock, so `decision/oscillation.rs` stably orders observations by timestamp. `append_order_cannot_fabricate_timestamp_order_oscillation` passes. |
| w15 BUG-1 | **Fixed.** `runtime/run_history.rs` selects the first/newest row. Library and `run_lifecycle_contract` fixtures now assert newest `run_recorded`, not oldest `quota_monitor_poll`; both pass. |
| w15 BUG-2 | **Fixed.** `runtime/run_compaction.rs` reports active kept rows, excludes previously archived rows, and does not claim a destination collision was archived. Repeat/collision tests pass. |
| w15 BUG-3 | **Fixed.** Checked cadence multiplication in `scheduler/state.rs`; L-reg overflow and normal-hours cases pass. No arbitrary new duration policy. |
| w15 BUG-4 | **Fixed.** `store.rs` validates writable goal IDs and maps unsafe read IDs to safe components; runtime and backfill read paths share that mapping. L-reg slash/backslash/absolute/dot traversal rejection passes. Unsafe historic IDs are not automatically migrated or followed outside the root. |
| w15 BUG-5 | **Fixed.** Latest-distinct failure cap enforced in merge and load/normalize. L-reg uses nine different targets, not a duplicate-only vacuous cap test; scheduler contracts pass. |
| w16 BUG-1 | **Fixed.** Atomic claim folds canonical ledger events under its exclusive lock, including renewal/owner/completion, instead of a partial lease parser. L-reg expired original claim + live renewal refuses peer theft; lease/store suites pass. |
| w16 BUG-2 | **Fixed.** Manual claims record no short-lived CLI PID; long-lived run uses `try_claim_todo_with_pid`. L-reg, CLI disjoint-frontier/manual-lease contract and owner projection subprocess tests pass. |
| w16 BUG-3 | **Fixed.** Shared private-span detector respects sk-/ak- token boundaries; task-/disk-/risk-/peak- are preserved. `projection/privacy.rs`, L-reg classification/redaction matrix pass. |
| w16 BUG-4 | **Fixed.** Redaction consumes path/token suffix, not just `/Users/`; tests assert username and secret suffix are absent, including Windows-style text. No real secrets read. |
| w16 BUG-5 | **Fixed.** Addressed-only guard corrected. Explicit scope/question/mention matrix and updated `work_items_drive::operator_inbox_kinds` pass. Capturing a chat does not make ordinary statements attention-required. |
| w16 BUG-6 | **Latent, no current production impact.** Empty-marker `contains("")` mechanism holds, but marker/hint APIs have no production consumer; executor uses actual tools/evidence. No speculative wiring or feature added. |
| w16 BUG-7 | **Fixed.** Linux /home and /root markers added; privacy projection no longer consults current HOME. L-reg path classification passes. Separate state-layer boundary diagnostic retains its existing purpose. |
| w16 BUG-8 | **Fixed.** `WorkLeasedToOthers` included in all 17 reason-code wire round-trip/uniqueness assertions. `quota/error_codes.rs` tests pass. |
| w16 BUG-9 | **Fixed.** Bridge receipts and records use the existing goal-wide base offset; bridge run IDs also carry UUIDs. Real two-process stdio bridge regression verifies attribution, distinct turns/run IDs and heartbeat IDs. Existing budget failures remain nonzero. |
| w17 BUG-1 | **Duplicate fixed:** w14-2. Original suggestion merely negating due check rejected: all Deferred work remains nonterminal. |
| w17 BUG-2 | **Fixed.** Optional durable `RunRecord.agent_id` stamped by executor and stdio bridge; webui and lane attribution use it, not run-name prefixes/current claims. Actual executor test plus persisted replay/completion/reassignment/UUID-style records test pass. Legacy records remain honestly unattributed. |
| w17 BUG-3 | **Fixed.** Graph uses actual layer-group heights/cumulative offsets. Four actual-source Node fixtures ensure containment/nonoverlap. Original claim that nodes were physically unreachable even with manual pan was too strong; defect is layout/fit. |
| w17 BUG-4 | **Fixed.** `compat::write_run` adds safe UUID filename suffix. L-reg 16 rapid JSON/MD pairs remain distinct and carry attribution. Authoritative spend ledger is unchanged. |
| w17 BUG-5 | **Latent / current caller reachability absent.** Tiny-budget truncate_evidence panic exists, but production callers use 1600/4096 or enforce >=12. No hypothetical API rewrite. |
| w17 BUG-6 | **Fixed.** Compat anchors encode/decode percent/newline/CR/tab/angle characters for note/resume/evidence. L-reg round trip preserves embedded newline, `-->` and literal `%20`; backfill suites pass. |
| w18 BUG-1 | **Fixed.** List Heading and blockquote raw-text reconstruction preserve heading text in `markdown.rs`. T-reg actual rendering passes. |
| w18 BUG-2 | **Fixed.** Source and decoded text strip C0/C1 controls before styling; OSC URL destinations reject controls. T-reg numeric entity text and actual cmark link destination tests pass; trusted internal Kitty protocol line still passes through. Not a blanket ban on generated terminal protocols. |
| w18 BUG-3 | **Fixed.** Selector saves deterministic sorted enabled IDs; Ctrl+P retains the saved Vec order. Repeated independent HashSet-instance save regression passes. |
| w18 BUG-4 | **Fixed.** List code border width accounts for content indentation/bullet width. T-reg widths 8/24/40/60/80 each produce two unsplit borders without ghost rows. One explicit list-code golden correction documented; 97-row parity passes. |
| w18 BUG-5 | **Fixed.** SelectList descriptions normalize CR/LF. Direct multiline-description test asserts each rendered string is one terminal row; passes. |
| w18 BUG-6 | **Fixed.** Component-aware home stripping with dirs::home_dir and native separator. Existing footer tests plus prefix-cousin regression pass on macOS; native Windows execution remains unclaimed. |
| w18 BUG-7 | **Fixed.** Top-level paragraph leading whitespace restored, excluding nested container indentation. T-reg tests spaces/tabs/nested paragraph and parity check pass. |
| w18 BUG-8 | **Fixed.** Nonempty whitespace-only text yields one empty row; empty string stays empty. T-reg passes. |
| w18 BUG-9 | **Refuted as correctness defect / documented Unicode divergence.** A single Unicode scalar emoji is valid filter input. Reintroducing JS UTF-16 length's accidental astral rejection is undesirable; explicit accepted divergence in `tui/tests/README.md`. |
| w19 BUG-1 | **Fixed.** Input::cursor_byte translates UTF-16 to UTF-8, providers guard byte boundaries. Actual Stdin/Input/provider Unicode regression passes. Duplicate w20-3. |
| w19 BUG-2 | **Fixed.** Visual rows retain source UTF-16 offsets, including discarded wrap spaces and hard newlines. T-reg navigation passes; old test's column-1='w' claim corrected to source position 7 ('o'). |
| w19 BUG-3 | **Fixed.** Indented fence detection conservatively protects stream prefix cache. Actual per-frame eager-vs-streaming nested-fence regression passes. |
| w19 BUG-4 | **Fixed.** Saturating inner widths in every message branch, including the unsafe max-after-subtraction system branch. Width 0..2 user/assistant/thinking regression passes. |
| w19 BUG-5 | **Fixed.** Provider state synchronized from current app cwd before query; file search uses that cwd and attachment subprocess gets current_dir, never process-global chdir. T-reg cwd-switch fixture and existing fd/find stub tests pass. |
| w19 BUG-6 | **Fixed.** Debounced and Tab queries use actual current cursor; slash prefix matching and replacement preserve suffix. Actual App middle-selection and tick tests pass. |
| w20 BUG-1 | **Fixed.** Escape-sequence scan advances only over valid char boundaries. Actual ESC+Chinese/accent/emoji StdinBuffer regression passes. |
| w20 BUG-2 | **Fixed.** Overlap slicing checks char boundary. Actual App Unicode middle-selection regression passes. |
| w20 BUG-3 | **Duplicate fixed:** w19-1. |
| w20 BUG-4 | **Fixed.** Loaded model/session results refresh registered providers; missing slash-argument caches fetch asynchronously and query current input without opening unwanted overlays. App cache-wiring and full RPC/UI tests pass. Nonempty caches refresh through existing list operations; no new live-catalog polling feature promised. |
| w20 BUG-5 | **Fixed.** Pre-register notification before empty-session read; use poke version across subscription. Deterministic barrier test pokes exactly between read and wait; passes with all stream tests. Naive notify_one approach rejected (see history below). |
| w20 BUG-6 | **Fixed ordering; cycle subclaim latent.** BTreeMap stabilizes cwd group order; full App tests pass. Normal fork creation points a fresh session to an existing parent; arbitrary corrupt/imported cyclic metadata was not established as a normal reachable producer. No speculative cycle-recovery algorithm; no claim that malformed external cycles are repaired. |
| w20 BUG-7 | **Fixed/narrowed.** Selection cursor uses UTF-16. Original no-suffix example was not a visible defect because clamp landed at end; actual App regression now includes a suffix and verifies cursor position 7 after Chinese completion. |
| w33 BUG-1 | **Fixed.** Help and console share PathBuf-based project-root resolver, not HOME. L-reg subprocess tests different HOME/current-dir and env override. |
| w33 BUG-2 | **Fixed.** Help ends in newline; same subprocess assertion passes. |
| w33 BUG-3 | **Latent / current production reachability refuted.** Static registry names are distinct; no production cross-group duplicate registration. Avoid unnecessary public API redesign. |
| w33 BUG-4 | **Latent / current impact refuted.** No experimental child setter/true child in current builder. Missing hypothetical gating is not an existing user-visible failure. |
| SF-1 | **Fixed.** Complete clears holder/expiry/PID; guard and agent list ignore terminal holders. Canonical atomic claim refuses completed work. Replay regression, terminal/dead workspace test and real CLI completion→idle projection all pass. |
| SF-2 | **Fixed diagnostics / semantic claim narrowed.** Case-sensitive identities and assignment before onboarding are intentional. Add/update warn with exact registration/reassignment action and casing suggestion; frontier displays unregistered owner. Subprocess regression passes. Quiet wait with another owner is not itself command failure; no forced case normalization or invented exit-error policy. |
| NF-1 | **Fixed.** Owner enforced inside atomic claim's canonical fold under lock. Non-owner rejection regression passes regardless of free/expired lease. |
| NF-2 | **Fixed.** Status exposes owner; frontier exposes pending todo assignments, registration state and leases, plus text owner hints. Real CLI JSON/text-side construction and subprocess tests pass. |
| V5-PID | **Fixed consistency.** Workspace guard/agent list use same pid_alive semantics as claim. Unix dead-PID fixture passes; Windows remains conservative because its existing probe returns alive. Valid manual None-PID leases retain TTL protection. |
| V5-lease-projection | **Fixed.** Status includes claimed_by/lease_expires_at/holder_pid. CLI regression observes active manual lease and completion release. |
| V5-regression-gap | **Closed.** Done-with-live-lease and complete→idle/nonblocking regressions explicitly exercise the formerly missing states. |
| V6-group-index | **Latent public API panic, no current production caller.** Builder uses indices returned by group(); no untrusted group index enters the call graph. Separately tracked, not counted as repaired production bug. |
| V6-line-reference | **Documentation correction.** Original w33 registry line 1267 does not exist; referenced consumer is console::render_command_help. |
| w32 BUG-5 (owned cross-slice) | **Fixed/narrowed.** WebUI encodes JS string then HTML attribute at all four dynamic goal/todo handler sites; actual templates tested through jsdom HTML decoding and JavaScript compilation (16 cases). New goal ID validation separately rejects malicious IDs. Existing/imported data is handled as data, without alleging third-party production exploitation. |

## gaps and integration notes

- No assigned **confirmed, reachable** repair is intentionally left unimplemented; full required local checks passed. Supervisor must independently review the narrowed/refuted/latent dispositions above and integrate with other workers before global acceptance.
- Native Windows/Linux execution and final CI are not claimed. In particular Windows pid_alive is still conservatively true; this patch aligns consumers rather than inventing native process detection. No real credentials, live LLM/browser services or user agent were used for reproductions.
- Historical run records without agent_id cannot be reliably attributed after ownership changes; they remain unknown. No fabricated migration guesses. `RunRecord.agent_id` is additive JSON (absent legacy -> None); explicit Rust struct fixtures were updated. No protobuf fields changed. Direct root CLI consumer should be checked during integration.
- `AttachmentProvider` is now constructed with `Default`; it carries cwd state. Its existing Windows fallback limitation (fd unavailable and POSIX find fallback) is not recharacterized as a new repaired platform feature.
- Unsafe historical goal IDs are not automatically moved or read outside the configured root. New writable IDs use the documented safe component restriction.
- Concurrent independent run processes may still reserve the same numeric base turn; this pre-existing console-wide mechanism is outside w16-9's sequential bridge-offset bug. UUID run identity is unique. No claim of a new atomic global turn allocator.
- Corrupt/imported session-parent cycles and the explicitly latent APIs above remain potential hardening areas, not demonstrated production defects. No extra worker/todo was created for them.
- Additional JS regressions require existing repo Node dependencies (jsdom); they are explicit commands, not silently represented as cargo tests.

## new evidence, rejected approaches, and next check

Beyond the previous attempt: durable execution attribution survives completion/reassignment/replay; every due Deferred class matches its Open lane at an injected clock; real CLI owner/lease views agree; rapid run files stay distinct; list geometry and terminal/HTML encoding have actual-implementation regressions; complete scoped Rust suites now pass.

Rejected approaches: trusting historical confirmed labels; negating just one Deferred predicate; guessing historical worker from current owner or run-name prefix; case-folding worker identity; assigning the short-lived CLI PID to manual leases; naive notify_one (retained initial permit spuriously cancelled subscriptions); preserving old tests that asserted latest=oldest/reversed scope/garbage-ledger acceptance; blanket golden regeneration.

Observed validation history: short first-round commands timed out around shared builds and were not counted as passes. Full checks subsequently exposed and corrected the old latest-row, manual-claim, lane-attribution, reversed-inbox and corrupt-ledger expectations. The new unsuccessful one-turn bridge fixture correctly expects a nonzero max-turns exit. These were deterministic contract corrections, not rerun-until-green flake handling. The final serialized batch passed at 16:31:13. Later report write was interrupted by the checkpoint before execution; this final report replaces stale pending paragraphs.

Next useful check: supervisor cherry-pick/review the four commits, reconcile shared RPC/model consumer changes, run direct-consumer/integration checks and the two Node regressions, then perform the one authorized final PR/CI workflow. Worker handoff declares `--no-follow-up`; supervisor owns any successor task and global closure.
