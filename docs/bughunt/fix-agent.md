# fix-agent — round-2 handoff and complete finding ledger

Session `20260911-123030-b1012f`; todo `todo_024bdc7eab9e`; branch `claude/bughunt-agent`; exclusive worktree `/Users/geilige/future-os/.worktrees/bughunt-agent`.

**Local implementation/checks delivered for review. Not a claim of native Windows certification, resolution of externally unverified findings, or completion of the global 224-finding goal.** All 79 original assigned IDs are accounted for below. No confirmed remaining agent implementation is silently omitted. Explicit external, latent, delegated and native-validation limitations remain separate from repaired production defects.

Sources: read-only `/Users/geilige/future-os/.future/bughunt/REPORT.md`, `CROSS-MODEL-VERIFICATION.md`, w01–w13, assigned w25 sections, V1/V2/V4, KIMI, GLM-b1/b2/b3/b6. Current source, actual tests and reachable callers override old verifier labels. ROUND2.md authorizes the additional 120-minute window, epochs 1789108361–1789115561; it supersedes round-1 timing instructions.

## commits

Apply this branch's commits in order (none pushed or merged by this worker):

1. `d4781920` — first-round approval/isolation/bounded execution checkpoint.
2. `1f677403` — first-round regression corrections and honest partial evidence.
3. `950742c2` — round-2 persistence, stream cleanup, Windows arguments, model limits, rule diagnostics and regressions.
4. `2ce2fba7` — final round-2 validated backpressure/boundary regressions, native test definitions and reconciled 79-row ledger.

Final deterministic INDEX.json reconciliation after that commit: **79 rows, 79 unique assigned IDs, no missing or extra IDs**. Code matches the successful 16:09:47 full batch. The following documentation-only commit records these immutable commit IDs; no code changed after validation.

The final report replaces stale first-round “compiled/pending” row notes. Historical partial state remains in git, not mixed into current dispositions.

## tests — actual final full-crate result

Executed exactly from this worktree:

`python3 /Users/geilige/future-os/.future/bughunt-fix/check-rust.py future-agent`

The supplied script serialized the entire batch and fixed Rust 1.97.0, CARGO_HOME, RUSTUP_HOME, shared target, isolated HOME/USERPROFILE and fd limit. **PASS at 16:09:47 CST**, including:

- `cargo fmt -p future-agent --check`: PASS.
- `cargo clippy -p future-agent --all-targets -- -D warnings`: PASS.
- `cargo test -p future-agent`: PASS at default parallelism.
- Library: **1749 passed, 0 failed, 1 ignored** (24.50 s).
- `cli_smoke`: **23 passed**, including isolated agent startup/shutdown, singleton, strict invalid-address handling.
- `compaction_persist`: 1 passed; `session_load_test`: 1 passed; `sqlite_startup`: 3 passed; `sqlite_storage`: 14 passed.
- Main/doc test targets: zero cases, successful. Linux-only smoke target: zero cases on macOS. Nine real Seatbelt smoke cases remain explicitly ignored, not counted as executed.
- Total executed successes: **1791**, with 10 ignored cases across lib/Seatbelt. No live user agent disturbed.

An earlier full serialized batch also passed at 15:45:13 (1743 lib successes plus the same integration suites). Earlier development passes were 1738 and 1725 lib successes. Two development failures were addressed, not hidden: (1) remaining old `ls` exemption assertion corrected after intentionally tightening Manual policy; (2) a channel interrupt had already consumed its signal, so new reason selection incorrectly called it a provider error—added explicit interrupted-during-stream state and retained the original interrupt expectation. Final tool-index guard also marks the run incomplete, not just emitting an error.

### Parallel cache-test investigation

Round 1 had a real parallel failure in `registry_injects_future_models_from_disk_cache`; serial success did not refute it. Source inspection found its cache reset occurred **before waiting for TestHome's global HOME lock**. Other registry tests could fill the shared memory cache during that wait, so the “cold-cache” assertion read another test's catalog. Moved reset after HOME acquisition, matching other cache tests' ordering. Default-parallel development runs and both full serialized batches pass after this change. This repairs the identified test-isolation window; it does not claim to prove every possible global-cache race impossible.

## coverage — all 79 assigned original findings

All paths below are worktree-relative. `Fixed` means implementation and listed regression passed in the final macOS batch unless explicitly labeled native-only. `Native pending` is not a native pass. `Latent`/`not established` are not counted as repaired production bugs. No SF/NF IDs were assigned to this partition.

| ID | Final disposition | Evidence, implementation, regression and remaining limitation |
|---|---|---|
| w01-BUG-1 | Fixed; native gap | `rpc/session.rs` checks deadline until process AND readers finish; bounds captured output; cancellation generation terminates direct shell RPCs. `commands/settings.rs` releases session lock before execution. Passed `execute_shell_bounds_inherited_pipe_lifetime`, `execute_shell_snapshot_can_be_cancelled_without_session_lock`, existing group-kill test, and actual `shell_rpc_releases_session_lock_and_abort_stops_process`. Escaped descendants can retain detached reader threads until EOF; no unbounded join remains. |
| w01-BUG-2 | Fixed | Session retains injected `GlobalQueueBudget`; restore uses it instead of unlimited queue. `hydrated_scheduler_keeps_injected_global_budget` loads real saved metadata and proves the second queued request hits the injected global capacity 1. |
| w01-BUG-3 | Fixed | `get_session_stats` uses real counters/cost/SQLite path. `session_stats_empty` now also checks nonzero input/output total 168, cost 0.75, actual database path. Cached input is not added a second time to total. |
| w01-BUG-4 | Refuted as current production panic | Setters and loop-lock holders are serialized by the enclosing session lock in current reachable callers. No competing holder outside that lock found. No change to silently drop settings on try-lock failure. Latent lock-order fragility is not a production panic. |
| w01-BUG-5 | Fixed | Folded projection carries latest event_id/timestamp/run_sequence with idx. `projection_preserves_semantic_order_while_coalescing_deltas` checks identity suffix against folded cursor as well as text/order. |
| w01-BUG-6 | Fixed without data loss | Bounded event queue now applies blocking backpressure rather than marking transient Full as permanent persistence failure. Writer thread does not acquire broadcaster/session locks; disconnected writer/storage errors remain fail-closed. `transient_full_event_queue_backpressures_without_failing_health` exercises actual append with a full queue, verifies both ordered events and healthy state. Existing journal failure/durable batching tests pass. Slow storage can still stall a producer, as at existing durability boundaries; no delta dropping introduced. |
| w02-BUG-1 | Fixed | Removed program-basename exemptions. env/wrappers always ask in Manual. Passed shell command-table and gate tests; no command executed to demonstrate bypass. |
| w02-BUG-2 | Fixed | Arbitrary shell file reads ask; no guessed shell-token secret parser. Secret/absolute/parent/variable/PowerShell reads covered by command table. Path-aware read tool remains available. No real secrets read. |
| w02-BUG-3 | Fixed | Manual git commands ask; branch/tag/remote/reflog mutation cases covered. Avoid incomplete per-option blacklists and git helper execution assumptions. |
| w02-BUG-4 | Fixed | sort/uniq/find/date/hostname/yes/seq are not exempt. External `ls` also asks: its basename can be PATH-shadowed. Only exact builtin `pwd` remains automatic; configured shell startup/environment remains trusted. Explicit full permission and actual OS-wrapped behavior unchanged. |
| w02-BUG-5 | Fixed | Gate ignores explicit null optional permissions, matching handler Option semantics. Existing empty-write test now explicitly tests null through `ApprovalGate::request`. Invalid non-null payloads still fail. |
| w02-BUG-6 | Fixed | `shorten_home` uses component-aware strip_prefix. Existing home test plus sibling-prefix assertion ensures `<home>-sibling/file` is not abbreviated into a misleading persistence suggestion. |
| w02-BUG-7 | Fixed | HTML metadata is escaped before interpolation. `generate_session_html_escapes_content` now supplies malicious title/model/cwd as well as message content and forbids raw script/img markup. |
| w03-BUG-1 | Fixed | Enqueue waits while scheduler.active survives control/task-slot cleanup. Deterministic `enqueue_prompt_defers_until_scheduler_completion_delivery` passes, preserving new request instead of cancelling it on ActiveRunExists. |
| w03-BUG-2 | Fixed | System prompt takes accepted model/thinking, not live session fields. `queued_run_prompt_uses_accepted_model_and_thinking` changes live settings after enqueue and inspects actual outbound ModelRequest. |
| w03-BUG-3 | Fixed; consumer integration note | `LLMProvider::snapshot` is additive with an immutable-provider default. Production Client clones mutable generation locks while retaining live model-registry authority. Loop snapshots consume it. Both private-generation mutation test and actual HTTP `provider_snapshot_sends_frozen_thinking_on_the_real_http_path` pass; queued prompt test covers admission path. Custom mutable providers must implement snapshot. Direct-consumer checks belong to supervisor integration. |
| w03-BUG-4 | Fixed | Lexical normalization preserves unresolved relative parents and never climbs above an absolute root. Regression covers ../x, ../../x and a/../../x through actual helper. |
| w04-BUG-1 | Fixed | Glob compiler iterates chars rather than UTF-8 bytes. Actual RuleSet test uses CJK workspace/rule names and verifies secret Read/Write Ask, user Deny and one-character wildcard semantics. |
| w04-BUG-2 | Fixed; native pending | Corrected V1 root cause: native Path components prevent duplicated absolute remainder/prefix; Windows regex pattern/target separators and case handling agree. Shared RuleSet regression passes on macOS; native Windows execution remains necessary. |
| w04-BUG-3 | Fixed | Quote stripping requires at least two bytes. Pure implementation is compiled for tests on every host; `malformed_powershell_wrapper_quotes_never_panic` covers both lone quotes and wrapper forms. No PowerShell process needed to establish slice safety. |
| w04-BUG-4 | Fixed | Rule parsing normalizes case/edge whitespace and rejects unknown fields without broadening access. Per-rule diagnostics reach RuleSet resolution_errors while valid rules survive. Actual-file `invalid_rule_fields_are_diagnosed_without_losing_valid_rules` verifies READ/deny-space applies to reads only and both unknown rules are diagnosed. |
| w04-BUG-5 | Unclear external/native behavior; no patch | Actual localized Windows denial text/cause not observed. Nonzero status alone cannot authorize an unsandboxed retry, and access-denied can be ordinary ACL failure. Do not inject a false sandbox marker or label every error a sandbox violation. Need native controlled denial evidence before defining classifier behavior. |
| w04-BUG-6 | Fixed | Tilde joins trim leading platform separators so ~//x stays under home. Actual helper regression passes. |
| w04-BUG-7 | Fixed; native pending | Platform separator predicate handles ~\\x on Windows, preserves literal backslash on Unix. Windows assertion exists in cross-platform helper test but was not executed on macOS. |
| w04-BUG-8 | Fixed | Seatbelt uses selected `unix_shell()`; tool description defers to host-shell prompt, no false bash claim. `seatbelt_uses_the_same_interpreter_as_plain_shell` verifies real prepared argv. |
| w05-BUG-1 | Fixed (a); (b) not established | Windows plan omits lower literal subtrees entirely covered by an earlier literal write rule, so shadowed denies cannot become unconditional ACEs. Exact/child carveout regression passes in platform-neutral plan tests. Original (b) assumes listing a fallback writable root overrides a higher user deny; the rule engine does not promise that. Physical inherited-ACL behavior still requires native verification; no blanket deny removal applied. |
| w05-BUG-2 | Fixed | Shared MAX_BWRAP_ARGS=9000; MAX_MOUNTS uses worst-case four args per opaque-directory mount plus 32 fixed-argument headroom, not the report's incorrect universal three-arg arithmetic. `oversized_literal_mount_plan_fails_before_helper_execution` builds actual filesystem/rule input and gets MountLimit before helper launch. |
| w05-BUG-3 | Fixed | Failed Linux probes cached for only 5 seconds, keyed by PATH/workspace/cwd; context changes bypass cache, expiry retries. Fake-host regression drives actual cache/probe implementation and proves no subprocess calls while cached, then successful retry. Failure is not cached permanently. |
| w05-BUG-4 | Latent, no production patch | No current producer of GRANT + FILE_GENERIC_WRITE + INHERIT_ONLY for the same capability/object. Existing child-only ACE grants DELETE, not WRITE. Mask observation alone does not establish a currently reachable denial bug. |
| w05-BUG-5 | Unsafe-contract hardening fixed; native pending | TOKEN_USER uses machine-word-aligned storage; OwnedSid uses u32-aligned words across clone/everyone/derived paths, preserving exact SID bytes. Updated Windows byte-layout/alignment assertion exists. No actual allocator-induced crash claimed; native Windows compile/test outstanding. |
| w06-BUG-1 | Latent public-API case; no production patch | SQL NULL timestamp mechanism exists for an empty low-level save, but current RPC persistence supplies dated session/run entries and empty legacy imports are rejected. No production creator of this state established. Kept distinct from malformed-data resilience improvements. |
| w06-BUG-2 | Fixed | Revalidation classifies source_changed/source_unreadable as a per-session skipped import instead of aborting unrelated startup. Never imports stale bytes; destination transaction errors still propagate. Actual-file disappearance/change regression and full SQLite import/startup suites pass. |
| w06-BUG-3 | Fixed at reachable RPC boundary | cmd_fork checks requested entry exists in loaded parent before creating a child. RPC regression tries nonexistent point, checks failure/no new session, then valid fork succeeds. Legacy public fork helper retains its documented fallback; no reachable RPC uses that fallback silently. |
| w06-BUG-4 | Fixed | Recovery is a FIFO worker command, with acknowledgement; only successful ordered recovery clears prior error. Held-worker regression verifies accepted append precedes terminal; existing degraded/close/commit tests pass. |
| w06-BUG-5 | Latent low-level diagnostic case | Missing-id assistant/tool can produce SQL error through malformed public raw values; production serialization and import validation supply IDs. Not an observed production disclosure; no speculative rewrite. |
| w07-BUG-1 | Fixed | Repair relocates late real tool result into the calling assistant's response window, preserving entry identity; does not merely remove an adjacent placeholder and leave invalid ordering. Stops at later declaration of same call ID and respects known run ownership. Actual Manager save/load regression verifies exactly one real result directly after assistant; shared reconciliation tests pass. |
| w07-BUG-2 | Duplicate / latent | Same NULL timestamp mechanism as w06-BUG-1; not a second repaired production defect. |
| w07-BUG-3 | Fixed | Uses existing unicode-width 0.2.2 scalar-width table; regression covers ✅, 🚀 and halfwidth Katakana. Approximate scalar preview is documented, not claimed to be a full grapheme layout engine. |
| w07-BUG-4 | Latent / unestablished upstream input | Cross-run call-ID reuse not established for current providers/call paths. No global dedupe semantic rewrite based only on a hypothetical malformed transcript. |
| w08-BUG-1 | Fixed | Decoder bounds complete unfinished event bytes including line overhead, resets per event. Actual multiline regression passes; many empty data lines cannot bypass memory accounting. |
| w08-BUG-2 | Fixed narrowly | Repeated finish still delivers newly supplied usage; no duplicate Finish, no read beyond authoritative [DONE]. Idempotence/usage regression passes. Trailing nonstandard cost after DONE is not grounds to keep a completed transport open. |
| w08-BUG-3 | Fixed | Responses accepts integer/float/string token fields (including detail counts), preserves credit_cost, derives missing total. Actual adapter regression passes. USD cost/estimated_cost are deliberately not treated as platform-credit aliases. |
| w08-BUG-4 | Fixed | Consumer clears provider-owned id/item_id on pending (not completed) tool calls before partial-history finalization. Preserves unrelated metadata and visible content. Actual interrupted stream regression verifies persisted history has no unfinished provider identity. Rejected producer-only finish-after-drop placebo. |
| w08-BUG-5 | Fixed blocking-runtime issue | Image preparation runs in spawn_blocking, outside async runtime workers; existing real image/HTTP tests pass. Repeated path reads are retained intentionally because attachments are live references; no unbounded or stale image cache introduced. Individual decode remains subject to existing input/allocation bounds, not claimed cancellable midway. |
| w08-BUG-6 | Fixed | Anthropic EOF drains open blocks in deterministic index order before Incomplete. Incomplete thinking does not gain replay authority from a partial signature. Explicit open text/reasoning/tool regression and existing transport tests pass. |
| w09-BUG-1 | Fixed | Each independent loop gets its own checkpoint cell; only explicit run/session sharing remains. Isolation pointer test and session/run compaction tests pass. |
| w09-BUG-2 | Fixed | Engine constructor applies configured max_turns to Loop. Config regression asserts 7 and existing runtime turn-limit tests pass. |
| w09-BUG-3 | Fixed | All consumer tool-event indices >=256 rejected before UI forwarding/resize; error also marks run incomplete with invalid_tool_index. Actual scripted usize::MAX/256 regression verifies no tool history and correct terminal classification. |
| w09-BUG-4 | Unclear external semantics; no accounting patch | No authoritative multi-usage token fixture establishing cumulative versus incremental values. Cost semantics do not prove token semantics. Do not silently change billing/accounting based on inference. |
| w09-BUG-5 | Fixed | Exhausted disconnect retries produce connection-interruption reason and final tool-end, not stale “retrying” or user-interrupt text. Six-disconnect actual loop regression and channel/flag interruption cases pass. |
| w10-BUG-1 | Agent fixed; caller work delegated | Qualified lookup remains authoritative, then bare exact IDs may contain slashes. Regression distinguishes two providers sharing family/model and unique bare slash fallback. Ambiguous caller references must be fully qualified; fix-apps owns desktop/mobile correction and reports it delivered. Integration must combine both branches. |
| w10-BUG-2 | Fixed | Canonical agent/auth.json precedes legacy agent-app; canonical empty object does not resurrect keys. Isolated-HOME regression passes. Both directory-layout language documents updated. Unloadable canonical file retains documented legacy fallback. |
| w10-BUG-3 | Fixed | Validate exactly what is written; reject leading/trailing whitespace IDs rather than validate trimmed/write untrimmed. Provider validation regressions pass. No unauthorized edits to users' preexisting config files. |
| w10-BUG-4 | Fixed | Loader distinguishes omission (0 sentinel) from explicit 128000; enrichment fills only absent limit and gives unknown models normal fallback. Explicit-128k regression and loader/catalog tests pass. |
| w10-BUG-5 | Fixed | Auth-write failure restores models only when this call changed it. Dedicated Unix failure-injection test proves unchanged models inode survives failed auth deletion; existing rollback tests pass. |
| w10-BUG-6 | Delegated to fix-apps | ROUND2 assigns generator, builtin output schema/conversion and default-selection filtering to fix-apps. Read its updated delivery ledger; exact overlap is `agent/src/models/mod.rs` builtin conversion/default selection/tests plus `models/builtin/mod.rs`. Do not count this branch as implementing that patch. Supervisor must integrate/recheck. |
| w10-BUG-7 | Fixed | Malformed auth entries warn without including decoder/credential values. Log-capture regression verifies warning/provider name, retained valid entry, no secret sentinel leakage. |
| w10-BUG-8 | Fixed | Checked positive i32 conversion; actual conversion regression covers negative, zero, 3 billion and valid 64000. |
| w11-BUG-1 | Fixed; native pending | PowerShell-safe ASCII base64 literal decoded as UTF-8 and piped to correctly spaced CLI --stdin; suffix preserved; doubled single quotes supported; compound programs not rewritten. Actual rewrite/decode regression preserves Unicode/apostrophes/commas and flags. Native PowerShell execution test exists but not run on macOS. Windows command-line length limits remain platform constraints. |
| w11-BUG-2 | Fixed; native pending | Windows shell waits for real process exit inside original timeout; real status returned; timed-out partial output is signal failure, not exit 0. Added native exit-7/timeout/descendant test; not executed on this host. |
| w11-BUG-3 | Fixed | Edit tolerates read's LF view of CRLF and preserves untouched bytes/CRLF insertion. Actual read+edit handler regression passes. Mixed-line-ending insertion uses existing CRLF presence; no whole-file normalization. |
| w11-BUG-4 | Fixed | Absolute home comparison is platform-aware before recursive-rm root matching, including Windows drive paths. Pure guard test uses resolved home without running rm. Existing allowed project-target tests pass. |
| w11-BUG-5 | Fixed | Top-level frontmatter extraction ignores nested keys. Explicit nested name/version regression and skill suite pass. |
| w11-BUG-6 | Fixed | MIME comes from decoder's actual recognized format, not guessed suffix/default PNG. Real BMP bytes stored as attachment.bin produce image/bmp in regression. |
| w11-BUG-7 | Fixed | Single and batch modes share first-occurrence replacement. Repeated old text/CRLF batch regression verifies first match and intact later occurrence. |
| w11-BUG-8 | Fixed by eliminating temp files | Argument rewrite now carries data in memory; no creation path or cleanup timing to leak a JSON file on success/error/cancellation. Rewrite regression asserts absence of temp-file/redirection form. |
| w12-BUG-1 | Fixed | Watchdog retains scheduled run_sequence. Paused-clock test uses begin_scheduled(7), reaches CancellationStuck as intended. |
| w12-BUG-2 | Fixed | Shared per-character quarter-token cost drives both context estimator and summary chunk splitting. CJK/Cyrillic/emoji/mixed-text regression verifies same units and bounded, lossless chunks. |
| w12-BUG-3 | Refuted proposed invariant; boundary documented | For window<=1 no integer satisfies reserve>0 and reserve<window. Current result does not invent capacity; added explicit [-1,0,1,2] boundary regression/documentation while preserving valid-window tests. Semantic summary-budget admission rejects unusably small capacity; no demonstrated repeated paid compaction loop. |
| w12-BUG-4 | Latent constructor inconsistency | Current production image caller filters empty URL before new_user. No reachable malformed provider request established; no speculative data-shape change. |
| w13-BUG-1 | Fixed shell/approval shutdown path | Unlocked cancellable shell RPC and shutdown approval cancellation remove the demonstrated lock deadlock. Actual RPC concurrency/abort regression and isolated CLI SIGINT smoke pass. These are compositional evidence, not a new end-to-end SIGINT-during-long-shell capture. Unrelated blocked filesystem operations can still delay runtime shutdown. |
| w13-BUG-2 | Fixed | Fallible IP/SocketAddr validation; bracketed IPv6 supported, hostnames rejected with actionable error rather than panic. Unit plus actual CLI invalid-address tests pass. |
| w13-BUG-3 | Fixed | Invalid/overflow/missing port cannot fall back to 50051. Replaced weak smoke test accepting exit 0 OR 1 with exact exit 1 + invalid-address diagnostic + no listening log for five invalid forms. |
| w13-BUG-4 | Fixed | --verbose enables debug default filter; explicit RUST_LOG remains authoritative. Actual tracing enabled-event regression verifies debug disabled/enabled by default mode. |
| w13-BUG-5 | Fixed | Present invalid sandbox tier returns tonic InvalidArgument before policy conversion. Wire-boundary regression covers strict, mixed case and empty value. Legacy internal parser untouched. |
| w13-BUG-6 | Delegated protocol contract | ROUND2 assigns Base64/data-URI comment/encoding contract to fix-cli and caller review to fix-apps. No guessed image/* MIME or unrelated RPC schema change in this branch. Supervisor must reconcile delegated result. |
| w25-BUG-1 | Fixed; native pending | Job disarms only after normal completed wait; timeout/read failure retains kill-on-close. Native test checks nonzero footer and only its own spawned descendant's termination. Not a macOS-native pass. |
| w25-BUG-3 | Fixed | abort_retry cancels approval senders after abort. Existing RPC test now has a real pending approval receiver and verifies Cancelled delivery plus empty pending list. |
| w25-BUG-6 | Fixed mechanism; native pending | Mutex serializes get/probe/set, preserving retry on transient failure rather than caching it forever. No native concurrent host-probe execution performed; source lock/OnceLock identity reviewed. |

## gaps and integration contract

1. **External evidence still genuinely absent:** w04-5 Windows denial attribution/localization and w09-4 progressive token semantics. These are not claimed repaired production defects. w05-1(b), w06-1/5, w07-4 and w12-4 have the narrower reachability/invariant dispositions above, not blanket “confirmed”.
2. **Native Windows validation outstanding:** run agent native tests, especially `powershell_executes_rewritten_stdin_with_unicode`, `windows_shell_reports_exit_status_and_timeout_kills_descendants`, SID layout/alignment, glob/tilde and host-probe paths. macOS clippy cannot type-check cfg(windows) bodies. No heavy tools installed or real credentials used to fabricate native evidence.
3. **Cross-owner integration:** fix-apps owns full-qualified desktop/mobile model references and w10-6 generator/builtin reader/default filter. This branch owns resolver, missing-window sentinel and cache-test ordering in the same models.rs file; merge carefully. fix-cli owns w13-6 and packages/rpc. No proto field numbers changed here.
4. **Direct consumers:** additive LLMProvider::snapshot default avoids forcing immutable mocks to change, but mutable custom providers must implement it. Supervisor should test CLI/desktop consumers after integration. Cargo.lock change is only agent's dependency edge to already-locked unicode-width 0.2.2.
5. **Scope limits retained:** scalar-width previews are not full grapheme layout; individual image decode cannot be cancelled midway; direct shell escaped descendants may retain detached reader threads; bounded journal backpressure can wait on slow storage. None is concealed as a verified cure for every conceivable mechanism.

## New evidence vs round 1 / rejected approaches

- Round 1 stopped with confirmed implementations and integration checks missing. Round 2 supplies real full-crate parallel passes, targeted regressions, source-supported narrower dispositions, and working implementations of the remaining confirmed in-scope defects.
- Did not silently drop journal deltas, guess USD/credit aliases or token accumulation semantics, infer sandbox denial from arbitrary failures, call producer cleanup after its consumer is gone, or globally dedupe after inserting placeholders while leaving invalid message ordering.
- Did not fix Windows string spacing alone while leaving unsupported PowerShell `<` redirection or permanent temp-file leakage.
- Did not classify serial cache-test success as proof of a baseline flake; fixed its identified reset/HOME-lock ordering and verified default-parallel runs.
- No push, PR, branch merge/reset, extra worker, other-worktree edit or user-agent disruption. Only supervisor owns independent acceptance, integration, final PR/auto-merge/CI and cleanup.

Next useful check: independent review of this branch plus delegated models/proto integration, followed by native Windows execution and direct-consumer tests. Full local Rust batch is reproducible with the exact command above; external semantics need evidence rather than additional blind implementation.
