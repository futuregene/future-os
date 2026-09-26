# tau-remote — coverage, waivers and dimension evidence

Scope (`verify.py rust-module tau-remote`, group `desktop-tauri`):

```
desktop/src-tauri/src/remote/
desktop/src-tauri/src/remote_host/
desktop/src-tauri/src/future_login.rs
desktop/src-tauri/src/device_identity.rs
desktop/src-tauri/src/auth_store.rs
```

## The measurement

`desktop/src-tauri` is not a workspace member; its package name is `futureos`.
**The crate now compiles and the whole package test suite is green in the
worktree**, so the out-of-tree mirror the previous run needed
(`.future/cov100/tau-m`, two patched files plus 16 Windows failures, and an
all-zero report without `--skip`) is no longer required and was not used:

```
cd desktop/src-tauri
$env:CARGO_TARGET_DIR = "target/cov-w-tau-rem2"
$env:RUST_TEST_THREADS = "4"
cargo llvm-cov -p futureos -j 3 --json `
  --output-path ../../coverage/tauri-tau-remote-report.json `
  -- --test-threads=4
# -> 1504 passed; 0 failed; 3 ignored   (green, report non-zero, no --skip)
```

One atomic invocation (test run + report together), private target dir named
after the agent, `report` still takes no `--target-dir`. The result is the real
line metric (the `Lines` column = LCOV `LH`/`LF`), and the report's file paths
preserve the `desktop/src-tauri/…` segment the gate anchors on.

**Cross-check:** a second `cargo llvm-cov report --lcov` over the same profile
data (`.future/cov100/tau-remote-final.info`) disagrees per file with
cargo-llvm-cov's JSON summary: for the nine files this round targeted, LCOV
reports **21** uncovered lines where the JSON summary reports **40**. The JSON
counts a brace/span end or an error closure that LCOV emits no `DA:` record for
(the same difference the plan warns about when counting from `segments`). The
gate resolves the module from the JSON summary, so the table below uses the JSON
numbers as the primary metric and names the LCOV figure where it differs; no row
depends on the disagreement.

## Before / after

The previous baseline was measured from the mirror, whose `terminal/session.rs`
was 301 lines shorter than the repository's, so the module totals are not
directly comparable — only the per-file figures are.

| | module lines | uncovered | files with gaps |
|---|---|---|---|
| previous run (mirror, 96.8223%) | 15387/15892 | 505 | 28 |
| now (`coverage/tauri-tau-remote-report.json`) | **16678/17108 = 97.4866%** | **430** | **26** |

Per-file, for the nine files this run targeted (JSON summary; the LCOV figure in
brackets):

| file | before | now (JSON) | now (LCOV) |
|---|---|---|---|
| `remote/commands.rs` | 44 | **9** | 10 |
| `remote_host/business/prompt.rs` | 22 | **11** | 7 |
| `remote_host/pairing.rs` | 17 | **9** | 1 |
| `remote/test_support.rs` | 11 | **7** | 2 |
| `remote_host/business/wire_limits.rs` | 5 | **2** | 0 |
| `remote_host/business/settings.rs` | 3 | **0** | 0 |
| `remote_host/business/catalog.rs` | 1 | **0** | 0 |
| `remote_host/business/history.rs` | 1 | **1** | 1 |
| `remote/supervisor/state.rs` | 1 | **1** | 0 |

65 of the 105 lines the previous run's *Residual risk* section named as "a
fixture gap, not the environment" are now covered by tests. The 40 that remain
are registered below, each with the category and the reason — including the ones
this run concluded are **unreachable by construction** and are reported to the
reviewer as deletion candidates rather than waived as "hard".

Two caveats on the numbers. The whole-package `rustfmt --check` is red for 35
hunks in files **this task does not own** (`agent_bridge/stream.rs`,
`remote/secure.rs`, `remote/tests.rs`, `remote/publisher/coalesce.rs`,
`remote_host/files.rs`, `remote_host/read_pages.rs`,
`remote_host/sync_measurement.rs`, `scheduler/mod.rs`, `terminal/server.rs`) —
other workers editing this checkout. The nine files in this write set are
rustfmt-clean; formatting them after the first report moved a handful of
multi-line expressions onto their own lines, which is why the measurement was
re-run and every number here comes from the post-format tree.

## WAIVED

Lines-100-or-waived, per file, with the category and the reason. Each row names
the repo-relative path, so the gate can match it.

| file | uncovered | category | reason |
|---|---|---|---|
| `desktop/src-tauri/src/remote/supervisor/start.rs` | 175 | unreachable-in-this-environment | 164 of these are the `#[cfg(not(test))]` bodies of `spawn_start_retry`, `spawn_runtime_reconnect`, `GenerationWatch::run`, `spawn_runtime_supervisor`, `spawn_web_reconnect` and `require_tls`. The test binary compiles the `#[cfg(test)] return;` arm beside each one, so **no test can execute the other arm** — it is absent from the instrumented binary. Reaching them needs a *new failure-injection seam* (a transport that refuses readiness, a broker that drops mid-handshake) or a real host; both are design changes this task was explicitly told not to make. The residual 11 are `start_generation`'s early returns, unchanged from the previous run. |
| `desktop/src-tauri/src/remote_host/sync_measurement.rs` | 68 | unreachable-in-this-environment | 40 are the `#[ignore]`d `serve_real_snapshot` harness, which needs an isolated Agent, a DB snapshot, a browser-driven client and its 120 s timeout arm (`scripts/measure-sync-browser.py`). The ~28 in `handle()` are socket-teardown arms: a peer that disconnects mid-response, a truncated body, a missing `origin`. |
| `desktop/src-tauri/src/remote/transport.rs` | 35 | unreachable-in-this-environment | The credential-refresh failure paths: a refreshed credential the broker *rejects* (needs `FakeNats` to refuse `CONNECT`), the `CREDENTIAL_EPISODE` connect failure, and a generation/readiness mismatch that needs `stop()` to land between two awaits of the refresh. The `require_tls(true)` line is `#[cfg(not(test))]`. |
| `desktop/src-tauri/src/future_login.rs` | 25 | unreachable-in-this-environment | The auth-state error arms and the retryable-poll transport error need the platform to **drop the connection mid-poll**, and 477 needs an `error` with no `source()` chain. The scripted `MockPlatform` answers every request it accepts; it cannot abort one halfway. |
| `desktop/src-tauri/src/remote/supervisor/shutdown.rs` | 20 | unreachable-in-this-environment | The mobile-notice arms need a live pair-scoped publish at the exact instant of `unpair`/`stop`, plus a `start_once` scripted to fail so `handle_system_resume` takes its retryable and `Err` arms. The `serde_json::to_vec(Value)` `.ok()?` arms underneath them are additionally `unreachable-by-construction`. |
| `desktop/src-tauri/src/remote/secure.rs` | 16 | unreachable-by-construction | The residual lines are `map_err` closures over crypto calls that cannot fail behind the guards that precede them (`noise.write`/`finish`/`remote_public_key` after a successful `read`, `serde_json::to_vec` on a `json!` value, `Reply::open`'s `error("channel")` after the channel was just matched). |
| `desktop/src-tauri/src/remote_host/business/prompt.rs` | 11 | unreachable-in-this-environment | Lines 145–151 are the "Future Agent did not acknowledge the prompt" reply (LCOV counts exactly those seven). It fires only when `accepted_rx` is dropped without a send, i.e. when `agent_bridge::headless::run_prepared_prompt_inner` returns between its two awaits — which happens only if the spawned prompt task *panics or is aborted*: `prompt.await` is the only early return, and a `tokio::spawn`ed task cannot be aborted from outside. That lives in `agent_bridge/headless.rs`, outside this module's write set and with no seam. The other four JSON lines are the multi-line `.ok_or_else(...)` spans of `validate_continue_source`/`remote_prompt_receipt` (`attribution-artifact`): `a_retried_continuation_is_answered_from_its_stored_receipt` and `a_receipt_for_a_thread_that_no_longer_exists_reads_as_no_receipt` execute those expressions. |
| `desktop/src-tauri/src/remote/commands.rs` | 9 | unreachable-by-construction | See the *Dead arms* report below: 276, 303, 432, 758, 837, 840, 843, 858, 861 are `if`/`else` edges that the guards three lines above them make impossible (or `#[cfg(test)]`-erased); 873 needs the connection handler to die between a successful `publish` and its `flush`; and 531 is the log line inside the unpair arm's spawned cleanup, which runs on the process-global runtime that `crate::runtime::set` points at, so a test cannot schedule it (`unreachable-in-this-environment`). LCOV counts 10 here (the JSON 9). The previously waived 34 lines are now covered (encrypted lane, access epoch, identity ladder, challenge cap, publish failure). |
| `desktop/src-tauri/src/remote_host/pairing.rs` | 9 | attribution-artifact | LCOV counts **one** line, 129: the `)?)` of `create_pairing`'s `code_expires_at.ok_or_else(...)?`. `a_pairing_code_without_an_expiry_is_refused_by_name` asserts that arm's refusal (`pairing_expiry_required`), so the expression provably runs and the line is a span end. The other eight JSON lines are the `map_err` closures of `http_client()`/`jwt_expiry` chains, the `?` on `config_io` reads, and `#[should_panic]` arms in test fixtures — all reached by the same tests (LCOV emits no `DA:` record for them). |
| `desktop/src-tauri/src/remote_host/files.rs` | 9 | unreachable-by-construction | Line 355 (`images > MAX_IMAGES`) is a dead duplicate of the earlier `references.len() > MAX_ATTACHMENTS` guard because `MAX_ATTACHMENTS == MAX_IMAGES == 10`; `cached_image_preview`'s residual arms and the upload-claim rollback detail need a filesystem that fails a hard link in the same directory (1133) or a `rename` after a successful copy (1171) — a TOCTOU race Windows will not produce on demand. |
| `desktop/src-tauri/src/remote_host/availability.rs` | 8 | unreachable-in-this-environment | The warning branch of `monitor()` needs the real loop to observe an available→unavailable *transition* across its hard-coded 3 s tick, and the probe result is not injectable. The durable fix is to inject the probe; until then the branch cannot be driven. |
| `desktop/src-tauri/src/remote/test_support.rs` | 7 | platform-unmeasured | LCOV counts **two** lines: the `cfg!(target_os = "macos")` arms of the mock's `probe_sandbox` default answer (`"available"`, `"macos_seatbelt"`). The measurement host is Windows, and the mock must report what its own host has — the whole point of the answer. `the_mock_defaults_satisfy_the_agent_readiness_handshake` asserts the Windows arm and that the code is a machine-readable reason. The other five JSON lines are brace ends of `set_session_entries`, `default_answer` and the publish helpers (LCOV: no record). |
| `desktop/src-tauri/src/remote_host/business/providers.rs` | 7 | unreachable-by-construction | `reply_view`'s `Err(_)` arm for `serde_json::to_value(ProvidersView)`. The view is a tree of `String`/`Vec`/`bool`/`Option`/`BTreeMap<String, _>` with string keys, so `to_value` cannot fail for it. Should be deleted. |
| `desktop/src-tauri/src/remote/transfer.rs` | 7 | unreachable-in-this-environment | A `queue_subscribe` the fake broker refuses, the seal failure on the reply lane, and the `pull` bad-index arm — all need either a broker that refuses a subscription or a secure channel that is mid-swap at the moment the reply is sealed. |
| `desktop/src-tauri/src/remote_host/session_files.rs` | 4 | platform-unmeasured | The directory-entry skips need a **dangling symlink**, a TOCTOU removal, and a FIFO/socket — Unix-only objects. The measurement host is Windows. A `#[cfg(unix)]` test is the follow-up and the lines are live on a Unix runner. |
| `desktop/src-tauri/src/remote/publisher.rs` | 4 | unreachable-in-this-environment | The catalog publisher's missing-snapshot arms (the store read returning `None` mid-tick) and the `Ok(false)` no-key reply path — both need the secure channel to lose its key between two ticks, which needs a credential refresh to land inside the tick. |
| `desktop/src-tauri/src/device_identity.rs` | 4 | unreachable-in-this-environment | The `HOME/USERPROFILE is not set` arm and the `PoisonError::into_inner` arms. Reaching the first means blanking or removing **both** process-wide while the store, the device-id table and every sibling test resolve storage through them, so it cannot be isolated under `TEST_HOME_LOCK`; the second needs a poisoned mutex, which requires a prior panic in a shared static. |
| `desktop/src-tauri/src/remote_host/business/wire_limits.rs` | 2 | attribution-artifact | LCOV counts **zero**: the two JSON lines are brace ends inside `paginate_events`/`truncated_event_data` whose statements the tests below execute (`the_byte_budget_is_what_keeps_a_page_inside_one_reply`, `an_oversized_event_has_its_data_cut_or_marked_missing`, `an_item_that_fits_once_its_tool_arguments_are_reduced_keeps_its_identity`, `a_value_that_is_not_an_item_object_has_nothing_to_rewrite`). The two stages this file's waiver used to name — the second `serialized_len(item) <= cap` early return and the non-object arm — are now covered. |
| `desktop/src-tauri/src/remote/lifecycle.rs` | 2 | unreachable-by-construction | `cancelled()`'s `changed() == Err` arm needs every `watch::Sender` dropped while the receiver is borrowed from the same `AccessEpoch` — the type system makes that unproducible. The second line is in `CandidateTasks`. |
| `desktop/src-tauri/src/auth_store.rs` | 2 | unreachable-by-construction | The `PoisonError::into_inner` closures of the `HomeGuard` fixture. A poisoned lock requires a prior panic while holding the same process-global `TEST_HOME_LOCK`, which would poison it for every later test in the binary. |
| `desktop/src-tauri/src/remote_host/read_pages.rs` | 1 | unreachable-by-construction | The `else { break }` of the cache-eviction loop. `insert` rejects `bytes.len() > MAX_SNAPSHOT_BYTES` (16 MiB) three lines earlier and `MAX_CACHE_BYTES` is 32 MiB, so with an empty cache the loop condition is false and `min_by_key` always yields `Some`. The guard should be deleted rather than kept. |
| `desktop/src-tauri/src/remote_host/business/transfers.rs` | 1 | unreachable-in-this-environment | `spawn_blocking`'s `JoinError`: the blocking task must panic or be cancelled. Needs a seam, or a panic inside `session_files::list`. |
| `desktop/src-tauri/src/remote_host/business/history.rs` | 1 | unreachable-by-construction | Line 221 is the `else` edge of `if let Some(events) = data["events"].as_array_mut()`. The only producer of `data` in this handler is `agent_bridge::queries::read_events_since`, which returns a **typed** `EventsSincePayload` and serializes it: `events: Vec<ReplayEventPayload>` has no `skip_serializing_if`, so the field is always a JSON array. The guard is defensive against a producer that cannot exist. Should be deleted (reported). |
| `desktop/src-tauri/src/remote/web_server.rs` | 1 | unreachable-in-this-environment | The body of `web_client_is_enabled` is behind `#[cfg(not(test))]`; the test build takes the `#[cfg(test)]` arm beside it. |
| `desktop/src-tauri/src/remote/supervisor/state.rs` | 1 | attribution-artifact | LCOV counts **zero**. The previously waived line was `shared_runtime`'s `pairing_confirmed` latch, and `a_confirmed_pairing_latches_onto_the_reused_runtime` now covers it and asserts the latch, the retained reply slots and the rotated bridge id. The residual JSON line is the closing brace of the reuse branch, whose statements that test executes. |
| `desktop/src-tauri/src/remote/publisher/coalesce.rs` | 1 | attribution-artifact | The residual line is the closing span of the `#[ignore]`d real-journal harness; `the_measurement_harnesses_hold_over_a_synthetic_journal` is the named test that proves the surrounding code runs. |

`remote_host/business/settings.rs` and `remote_host/business/catalog.rs` were
waived with 3 and 1 lines respectively and are now **fully covered**, so their
rows are gone; the crate's file count with gaps dropped from 28 to 26.

## NOT MEASURED — no executable lines

llvm-cov omits a file that compiles to **zero instrumented regions**, so these
never appear in the report and cannot be 0%-covered. Named here so the gate's
"absent from the report" check is explained rather than unexplained.

| file | why it has no data |
|---|---|
| `desktop/src-tauri/src/remote/services.rs` | only `trait` declarations (`ReplySink`, `BusinessHost`, `PairingHost`, `StateHost`, `FileHost`) and two type aliases — no function bodies to instrument. |
| `desktop/src-tauri/src/remote/supervisor/mod.rs` | module declarations and `use` re-exports only — no bodies. |
| `desktop/src-tauri/src/remote/tests.rs` | the integration-style test module itself; exercised by definition, and its own lines are not production coverage. |

## Dead arms — reported, not deleted

The task forbids changing production code to move a number, and requires that a
line believed to be truly dead be *argued*, not removed. These four are
impossible given the code above them; each should be deleted by the owner, and
none of them is counted as "reachable but untested":

1. **`commands.rs:303` and `commands.rs:758` — `None => return`.** Both follow an
   `access_current()` check with **no `await` in between** (the only awaits in
   between are none: `handshake.secure.handshake`, the transcript, the signature
   and the store calls are all synchronous), and `access_current()` and
   `commit()` compare the very same `AccessEpoch` value. `access_current() == true`
   therefore implies `commit(epoch, ..) == Some(..)`. The two `Option` arms exist
   only because `commit` returns an `Option`.
2. **`commands.rs:276` — the `else` of `if matches!(activated, Some(Ok(())))`.**
   `activated` is `None` only for a stale epoch, which line 235 already refused,
   or `Some(Err(..))` from `Transport::activate`, which accepts exactly the
   channels `open` can hand back (the current channel, or a candidate younger
   than 30 s) and is a no-op for the current one.
3. **`commands.rs:432` — the `else` of `if inserted`'s expiry task guard.**
   `slots.get(&id).is_some_and(|current| Arc::ptr_eq(current, &expected))` is the
   expiry task's own comparison; the only remover of a slot is that same task for
   that same id, so no concurrent removal can make it false.
4. **`commands.rs:837`/`840`/`843` — the `return plain` backstops in
   `encode_reply_payload_with_gzip`.** `837`/`840` are `io::Write`/`finish` on a
   `GzEncoder<Vec<u8>>`, whose sink cannot fail. `843` needs gzip to *not* shrink
   a JSON body of at least 32 KiB (`REMOTE_JSON_GZIP_THRESHOLD_BYTES`); measured
   on this host, the least compressible JSON text available (uniform ASCII 95,
   the highest-entropy alphabet JSON can carry) is 0.833× its input at
   `Compression::fast`, and every structured construction tested compresses
   better. The guard is a backstop against a future encoder change, not live code.

Also reported: `commands.rs:858`/`861` are unreachable **in the test build** —
`Reply::seal` has a `#[cfg(test)] if self.channel.is_none() { return Ok(...) }`
shortcut, so the `Ok(Err(_))` arm (a secure reply lane that cannot seal) can only
be reached in production, where the same state cannot arise. Line 861 is the
`#[cfg(not(test))] return;` itself (an `attribution-artifact` span). And
`commands.rs:873` (flush failure) needs the connection handler to die *between* a
successful `publish` and its `flush`; when the handler is gone, `publish` already
fails first (`a_reply_that_cannot_be_published_is_not_reported_as_delivered`
covers that arm), so the flush arm needs a race the harness cannot schedule.

`remote_host/business/history.rs:221` and `remote_host/business/providers.rs`'s
`to_value` `Err` arm are the other two deletion candidates argued above.

`commands.rs:531` is not dead — it is the failure arm of the unpair cleanup,
and `an_unpair_that_cannot_persist_is_still_acknowledged` *asserts* the failure
it logs (`assert!(crate::remote::unpair().await.is_err())` under an unset HOME)
and that the phone is acknowledged anyway. What it cannot schedule is the
spawned task itself: `crate::runtime::spawn` hands the future to the
process-global handle installed by `crate::runtime::set`, which belongs to
whichever test owns it, so the log line's execution is not observable from the
test that provokes it.

## New tests (this run)

Whole run: `cargo llvm-cov -p futureos … -- --test-threads=4` →
**1504 passed, 0 failed, 3 ignored** (no `--skip`).

| file | tests added |
|---|---|
| `remote/commands.rs` | `a_pairing_id_that_cannot_form_a_queue_group_returns_instead_of_waiting`, `a_paired_phone_gets_its_commands_answered_over_the_encrypted_lane`, `an_unopenable_secure_record_is_dropped_without_a_reply`, `a_handshake_sealed_inside_a_secure_record_is_never_a_business_command`, `a_channel_that_has_not_declared_readiness_is_not_yet_active`, `a_handshake_opening_without_a_reply_subject_sends_nothing`, `a_secure_connection_admitted_under_a_stale_epoch_answers_nothing`, `a_command_admitted_under_a_stale_epoch_never_reaches_the_host`, `a_stale_epoch_drops_the_command_before_it_is_decoded`, `an_epoch_revoked_while_a_retry_waited_for_its_slot_drops_the_retry`, `a_handshake_that_disagrees_on_one_identity_field_is_refused`, `the_challenge_table_stops_accepting_a_flood`, `a_confirm_commits_only_under_its_own_access_epoch`, `a_reply_that_cannot_be_published_is_not_reported_as_delivered`, `an_unpair_that_cannot_persist_is_still_acknowledged`, `an_unpair_whose_epoch_was_revoked_leaves_the_pairing_alone` |
| `remote/test_support.rs` | `await_publish_matching_times_out_when_no_publish_matches`, `wait_until_panics_when_the_condition_never_holds`, `set_session_entries_shape_skips_non_object_entries`, `the_mock_defaults_satisfy_the_agent_readiness_handshake`, plus `open_secure_channel` split out of `secure_pair` |
| `remote/supervisor/state.rs` | `a_confirmed_pairing_latches_onto_the_reused_runtime` |
| `remote_host/business/prompt.rs` | `a_prompt_whose_receipt_cannot_be_checked_is_refused`, `a_receipt_for_a_thread_that_no_longer_exists_reads_as_no_receipt`, `a_retried_continuation_is_answered_from_its_stored_receipt` |
| `remote_host/business/settings.rs` | `an_unreadable_store_is_reported_not_defaulted`, `a_catalogue_without_a_models_array_is_not_reported_as_all_hidden` |
| `remote_host/business/catalog.rs` | `a_pinned_workspace_whose_snapshot_cannot_be_read_is_not_reported_as_pinned` |
| `remote_host/business/wire_limits.rs` | `an_item_that_fits_once_its_tool_arguments_are_reduced_keeps_its_identity`, `a_value_that_is_not_an_item_object_has_nothing_to_rewrite`, `a_parseable_tool_call_keeps_its_target_and_drops_its_content` |
| `remote_host/pairing.rs` | `a_pairing_code_without_an_expiry_is_refused_by_name`, `pending_revokes_are_skipped_blocked_or_retried_by_error_class` |

No production line was changed to make a number move: the only non-test edits in
this run are the two `#[cfg(test)]`-module refactors named under *weak-tests-fixed*.

## dimensions

| dimension | evidence |
|---|---|
| boundary | `a_pairing_id_that_cannot_form_a_queue_group_returns_instead_of_waiting` (a pairing id NATS refuses), `the_challenge_table_stops_accepting_a_flood` (32 accepted, the 33rd refused and no challenge displaced), `a_handshake_that_disagrees_on_one_identity_field_is_refused` (six fields one at a time), `a_value_that_is_not_an_item_object_has_nothing_to_rewrite` (an oversized row that is not an item object at all), `an_item_that_fits_once_its_tool_arguments_are_reduced_keeps_its_identity` (the cap reached only after the second reduction stage), `a_catalogue_without_a_models_array_is_not_reported_as_all_hidden`, `an_opening_flood_is_refused_once_the_challenge_table_is_full` (16-challenge crypto cap) |
| error-path | `a_prompt_whose_receipt_cannot_be_checked_is_refused` and `an_unreadable_store_is_reported_not_defaulted` (a store that cannot answer is a failure, never a default), `a_pinned_workspace_whose_snapshot_cannot_be_read_is_not_reported_as_pinned` ("cannot read" ≠ "you have none"), `a_pairing_code_without_an_expiry_is_refused_by_name`, `pending_revokes_are_skipped_blocked_or_retried_by_error_class` (a 4xx blocks the retry, 429 does not, a foreign platform is never called), `a_reply_that_cannot_be_published_is_not_reported_as_delivered`, `an_unpair_that_cannot_persist_is_still_acknowledged`, `an_unopenable_secure_record_is_dropped_without_a_reply` |
| concurrency | `an_epoch_revoked_while_a_retry_waited_for_its_slot_drops_the_retry` (one poll parks the retry on the in-flight reply slot, the epoch is revoked while it waits — no sleep), `an_unpair_whose_epoch_was_revoked_leaves_the_pairing_alone` (the spawn runs after the revocation; the live half proves the same path does the work), `a_confirm_commits_only_under_its_own_access_epoch`, `a_command_admitted_under_a_stale_epoch_never_reaches_the_host` (a counting host proves the work did not happen), `an_epoch_revoked_while_a_retry_waited_for_its_slot_drops_the_retry` vs. `a_prompt_id_is_the_identity_of_its_run` (retry inside the slot window vs. after it) |
| property | `a_confirmed_pairing_latches_onto_the_reused_runtime` (the confirmation survives a generation swap while the bridge id rotates), `a_paired_phone_gets_its_commands_answered_over_the_encrypted_lane` (the reply is only readable through the channel that proved the invitation), `a_channel_that_has_not_declared_readiness_is_not_yet_active` (readiness is a commit, not a consequence), `the_challenge_table_stops_accepting_a_flood` (the cap never displaces an issued challenge), `a_retried_continuation_is_answered_from_its_stored_receipt` (no second run for one command id) |
| serialization | `a_handshake_sealed_inside_a_secure_record_is_never_a_business_command` (a sealed `pair_handshake`/`pair_handshake_confirm` is dropped, never dispatched), `an_unopenable_secure_record_is_dropped_without_a_reply` (a malformed `FRE2` record never falls through to the plaintext factory), `a_handshake_opening_without_a_reply_subject_sends_nothing` (a one-way opening publishes nothing at all), `a_receipt_for_a_thread_that_no_longer_exists_reads_as_no_receipt` (null vs. object), `a_prompt_receipt_is_null_for_an_unknown_id` |
| platform-cfg | The `cfg(unix)`/`cfg(macos)` branches in this subtree (`session_files.rs` symlink/FIFO skips, the macOS-only `product_sandbox_available`/`probe_sandbox` arms) cannot execute on the Windows measurement host and are registered `platform-unmeasured`. The `cfg(not(test))`/`cfg(test)` pairs are exercised in the sense that the test arm is what runs; the production arm is the largest single waiver (`start.rs`, `web_server.rs`, and the `publish_reply_payload` backstop). |

## weak-tests-fixed

1. **A test that could not fail was replaced, not kept.**
   `command_loop_subscribe_failure_logs_and_returns` killed the broker and then
   asserted nothing: async-nats accepts a subscribe *optimistically*, so
   `queue_subscribe` returned `Ok` and the branch the test was named for never
   ran (the coverage report agreed — all five lines were uncovered). It is now
   `a_pairing_id_that_cannot_form_a_queue_group_returns_instead_of_waiting`, which
   drives the real refusal and asserts the loop is deaf afterwards.
2. **A wrong-premise assertion was corrected.**
   `unparsable_tool_arguments_are_marked_not_dropped` asserted that a rebuilt
   `tool_args` is "empty **or** carries a `truncated` key". The second disjunct
   can never hold: the rebuilt value is always an object built from the four
   target keys. It now asserts the real contract (the object holds only target
   keys, and a non-JSON input recovers none of them), and a new test covers the
   parseable half.
3. **Two duplicated poll loops were replaced by the module's own `wait_until`.**
   Both copied a `while run.status == "running" { assert!(now < deadline); sleep }`
   loop whose body could not run in a green test (the run settles before the
   first poll) — six lines, never executed, in two tests. They now call
   `wait_until`, which panics with its own message, and that helper has a
   `#[should_panic]` test of its own.
4. **A test that only passed because of another test was made self-contained.**
   `mock_platform_handles_broken_requests` built a `reqwest::Client` without
   installing the rustls provider, so it passed or failed on test scheduling
   (it panics when run alone). It now installs the provider itself.
5. **A test's own cleanup is now asserted rather than half-executed.**
   `the_gzip_override_is_an_operator_switch` restored the environment variable
   through a `match previous { Some => set, None => remove }` whose `Some` arm
   never ran. Both arms are now exercised and asserted before the operator's own
   value is restored.
6. **A predicate that could never run was replaced by a value comparison.**
   After `rustfmt` split it, `unparsable_tool_arguments_are_marked_not_dropped`'s
   shape assertion exposed what it had hidden: `.all(|key| …)` never calls its
   closure for an empty object, so the assertion's own line was uncovered. It and
   `a_parseable_tool_call_keeps_its_target_and_drops_its_content` now compare a
   sorted key list (`argument_keys`), which is both stronger (it names the whole
   shape) and has no unexecuted body.
7. **A test premise is now checked, not assumed.**
   `an_unpair_that_cannot_persist_is_still_acknowledged` removed HOME and asserted
   only the acknowledgement; it now first asserts that `unpair()` really fails in
   that state, so the test fails loudly instead of silently proving nothing if the
   premise stops holding.
8. **Assertion-free tests introduced this run: none.** Every test above asserts
   something the phone can observe: a named refusal, silence where a reply used to
   be, "the host never ran", "the credential was not written", "no publish at all".

## Residual risk — rows a reviewer should scrutinise

1. **`start.rs`'s 164 lines.** Genuinely absent from the instrumented binary (the
   `#[cfg(not(test))]` bodies). The needed seam is a failure-injection point in
   `spawn_start_retry`/`GenerationWatch::run`/`spawn_runtime_supervisor` (a
   transport whose readiness never commits, a broker that dies mid-handshake) or a
   real host; this task was told not to add one.
2. **The JSON/LCOV disagreement.** Nineteen of the 40 remaining target-file
   lines are `DA`-less in LCOV (brace ends, error closures on infallible `?`
   chains, and `#[should_panic]` arms). I argued each one from the source rather
   than from a failed attempt, and named the test that proves the expression
   runs. A reviewer who can execute one should replace the row with a test.
3. **`secure.rs`'s remaining closures** and the `files.rs` TOCTOU arms are
   readings from the source, not from a failed attempt, as in the previous run.

## Post-merge registrations — files `origin/main` brought into this group's scope (2026-09-26)

The branch merged `origin/main`, and re-measuring on the merged tree moved this group from 97.4866%
to **94.7059%** (18068/19078 across 42 files, 1010 uncovered in 29 files). The drop is not a
regression in the tests: it is that the merged tree contains code the previous report did not cover.
Three files are new to this ledger, and each is registered by what its code actually is:

| file | uncovered | category | reason |
|---|---|---|---|
| `desktop/src-tauri/src/remote/verify_e2e.rs` | 321 | `unreachable-in-this-environment` | End-to-end byte verification of the lean lane over the REAL cryptography. Its uncovered lines are the bodies of three `#[ignore]`d tests that need a real broker and a measurement run: `VERIFY_E2E_NATS_URL` (a live NATS server), plus `VERIFY_E2E_JOURNAL`/`VERIFY_E2E_SESSION`/`VERIFY_E2E_RUN`/`VERIFY_E2E_ENTRIES` produced by `scripts/measure/verify-e2e-bytes.py`. `origin/main` added this file with those gates intact. |
| `desktop/src-tauri/src/remote_host/lean.rs` | 156 | `unreachable-in-this-environment` | Fixture generation and a measurement instrument, gated by `LEAN_RENDER_FIXTURES` and `LEAN_HISTORY_ENTRIES` (two `#[ignore]`d tests). The module's ordinary test module runs and covers the non-instrument code; the uncovered lines are the gated paths. |
| `desktop/src-tauri/src/remote_host/business/catalog.rs` | **0** | covered | CLOSED by a test, not waived: the three `Err(error) => reply(..)` arms of the store calls in `set_session_pinned` and `delete_session` are now driven, and the file reports **193/193 lines, no uncovered lines**. The test is `a_failed_lookup_is_not_reported_as_already_deleted` (see the row above). This row previously said "3 | being covered" — that was accurate when written and is superseded here, because leaving it would describe uncovered lines the report no longer contains. |
