test: raise per-module coverage to its measurable residual, with a reviewed waiver ledger

## What this is

A coverage campaign over the whole workspace, run under one rule (**口径 A**): a scope is
done when its per-line `lcov DA:0` count is zero, **or** every remaining uncovered line is
registered in `docs/testing/waiver-ledger.md` with a category and a mechanism verified in the
source. 27 module records now account for their residual line by line.

The second half of the work is adversarial: **nine independent reviews tried to falsify the
waivers** by constructing the inputs the tags said could not exist. Roughly half to two-thirds
of author-assigned waivers fell, and the reviews overturned rulings of my own. That calibration
is the most important result here — a waiver that has not been attacked is a hypothesis.

## Figures (each module self-reported against its own pinned artifact; NOT summable)

| module | uncovered | note |
|---|---|---|
| goal / tasks / packages | 77 | |
| agent-core (AG-B) | 213 | |
| agent-rpc (AG-A) | 202 | |
| agent-llm (slice of agent-core) | 115 | its headline 396 was `summary.lines`, which this campaign forbids |
| channels | 8 | |
| cli | 52 | |
| cli-mcp | 4 | disjoint from cli by its own declaration |
| desktop frontend | 59 | |
| mobile | 13 | 13/13 survived review |
| tui | 13 | |
| tauri-appdata | 378 | |
| tauri-remote | 1205 = 353 product + 852 `#[cfg(test)]` harness lines | the harness is `#[ignore]`d for want of a real broker |
| tauri-headless | 359 | 27 previously-unclassified lines now crosswalked |
| tauri-shell | see its record | 90 of 217 waiver rows were falsified by review |

**Do not sum these.** `tauri-headless` and `agent-llm` are slices of a parent scope; adding
them double-counts. The ledger carries a DO NOT SUM note. The objective is per-scope, which is
what the module records state.

## Flakiness

Every flake found in this campaign has a named mechanism and a fix resting on a control, not on
a rate: the src-tauri lib-suite deadlock (lock-order, now enforced by a source-scanning test),
the transfer-lane starvation family, the hand-off degradation (now loud and discardable), the
`persistence.rs` hand-off ordering (armed before `barrier()`), 64-burner reproductions where a
rate was the only evidence available, and one deliberate withdrawal — a 60 s test tolerance that
made the failure worse and was reverted with its reasoning left in the source.

## Real network

`REAL-NET` exercised the remote stack as two OS processes against a real `nats-server` with real
Noise/NKey: pairing, a 1.2 MB upload and a 960 KB download byte-identical, a prompt through a
real Agent, plus failure paths (socket close, broker killed mid-transfer with a 160 MB retry,
peer restart with the same identity). Its first report claimed a real defect — journal coalescing
growing the lane at the production 100 ms window — and **that claim was wrong**: measured with the
marker printed before the assertions, the lane was byte-identical (nothing merged, because that
journal's chunks were 100.9-110 ms apart against a 100 ms window, and `Coalescer::flush` returns a
lone fragment unchanged). The defect was the measurement's strict `coalesced < today`, which no
journal that cannot merge can satisfy. Recorded as a correction in `real-network.md` §3.3a and
`module-tauri-remote.md` §9, with a burst-paced journal added to show the window does merge 2.30x
when it can. No production behaviour changed; no assertion weakened.

## Review findings that are part of the deliverable

- `module-tauri-shell`: 90 of 217 rows falsified; 43 of them are uncovered in the healthy
  artifact too, so the published 150 overstated the coverable residual.
- `module-tauri-llm`: headline metric corrected (396 -> 115).
- `module-cli`: a `#[cfg(not(test))]` class rule, plus the doctor EnvGuard conversions.
- `module-agent-*`: two waiver rows overturned, one of them a supervisor ruling.

## Verification

`docs/testing/` holds a record per module plus nine review documents; `docs/testing/waiver-ledger.md`
is the per-module ledger. Every figure above is re-derivable with the campaign's tools, and the
final report states the numbers each record owns rather than a single repository percentage.

## Platform-scoped tests (stated, not hidden)

Five test cases and one harness-level target are restricted to macOS, each because the precondition
cannot be built on the other platform — not because the assertion is inconvenient. The mechanisms are
recorded in the source at each site:

| what | why it cannot run elsewhere |
|---|---|
| `gui::startup_paths_tests::an_exit_requested_from_the_mock_loop_takes_the_guarded_quit_arm` | On the headless Linux runner `on_close_requested()` takes the CONFIRM path (no session for the dialog), so the Proceed branch that publishes the stop never runs |
| `scheduler::tests::an_explicit_update_check_uses_the_endpoint_it_is_given` | `platform_allows_updates()` is `is_debian_deb_install()` on Linux and an NSIS bundle check on Windows — both false under `cargo test`, so the function correctly short-circuits before reading an endpoint |
| `rpc … an_unclassifiable_local_endpoint_failure_never_authorizes_a_start` | Needs Darwin's `ENOTSOCK` for a connect to a regular file; Linux answers `ECONNREFUSED`, which the client classifies as "absent" |
| `supervision_watchdog_stall_drive` (whole target) | The fixture needs a window between the parent's spawn and the watchdog's `exec`: macOS spawn costs tens of ms (5/5 delays land), Linux ~1 ms (0/5) |
| `re_pair_entry_points::the_pty_launcher_forwards_to_a_running_desktop_and_prints_the_answer` | Needs a REAL `futureos` process to bind the single-instance socket; on the headless runner the Desktop cannot start (tao: "Failed to initialize GTK"), so the socket is never bound (§S11.10.2's host-state precondition) |
| `tests/gui_main_thread_drive.rs` target | A `harness = false` binary whose purpose is muda's main-thread requirement; on Linux it panicked in tao with "Failed to initialize GTK". It now returns with one line off macOS |
