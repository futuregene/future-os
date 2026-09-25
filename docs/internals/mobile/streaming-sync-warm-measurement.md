# Cached reopen: real-history trace comparison (2026-09-16)

> ([中文](streaming-sync-warm-measurement.zh-CN.md)) The follow-up snapshot
> optimization is complete; see
> [snapshot + incremental and the A/B measurements](streaming-sync-snapshot-optimization.md).
> This report's controlled-cutoff experiment keeps the raw-event bootstrap (the
> measurement entry explicitly disables `preferSnapshot`) to reproduce whether
> the cache is reused; this report's ~12.7-second recovery time is not the new
> default path's time.

## The question to answer

"Part of the content is already displayed — can we read only the events after
it?" splits into two different conditions:

1. The current run's complete projector state, visible entries, contiguous
   cursor, and a trusted baseline still exist: incremental is allowed.
2. Only display data exists, or a baseline-invalidating notice was received:
   the current code must rebuild — "having text" is not "the full prefix has
   been applied".

This round verifies these two actual code paths rather than estimating the full
cost again.

## Method and boundaries

Reuses `streaming-sync-browser-measurement.md`'s isolated Agent, real
DesktopHost, Chrome, and loopback HTTP measurement tooling. The desktop adapter
is still a debug test build; NATS/E2EE, phone networks, Hermes, and native UI
are outside the measurement scope.

The new entry `scripts/measure/measure-sync-warm.ts` reuses the production Mobile
SyncEngine, paging, cursor, and projector. The data comes from the same
111,395-event completed historical run. This database snapshot had 231
completed runs in total.

Unlike the previous round's history-only targeting, this round **explicitly
adds the following scenario controls** — not presented as a packet capture of a
really streaming session:

- `get_state` still goes through real RPC, but the test adapter sets activeRun
  to the chosen historical run.
- First only events 0…111193 are made visible, then the 200 real follow-up
  events 111194…111393 are released; the final `agent_end` is withheld.
- The test adapter filters the cutoff point on real replay pages and sets a
  consistent watermark; no fake tokens, no injected RTT, no fake timers.
- The chosen run's terminal assistant mirror in the current history page is
  excluded so the history terminal state does not override the test's activeRun.
  Other current history rows are kept; this is not a complete history window of
  exact historical-time reconstruction.
- The first page still goes through the real server's no-watermark read path;
  later pages follow the production cursor loop.
- The controlled invalidation scenario explicitly sends a hidden-session
  `resend`; the display-cache scenario keeps the same batch of visible entries
  while deliberately removing cursor/projector/trusted baseline.

Each scenario runs three rounds, timed with real clocks. Every round checks the
request start, total events, final highWater, prefixComplete, streaming, and
projector existence; the full timeline.items SHA-256 verifies that display data
is identical across incremental, empty-increment, and full recovery — session
bodies are not output.

## Three-round results (before the diagnostic fields)

| Scenario | Replay start sinceIdx | Applied events | Replay requests | Sync completion |
| --- | ---: | ---: | ---: | ---: |
| Cold open, prefix established | -1 | 111,194 | 112 | 12.787–12.838 s |
| Complete-cache reopen, 200 new events | 111193 | 200 | 1 | **57.3–65.6 ms** |
| Complete-cache reopen, no new events | 111393 | 0 | 1 | **38.5–41.2 ms** |
| Reopen after explicit invalidation while hidden | -1 | 111,394 | 112 | 12.667–12.723 s |
| Same visible content kept, no recovery state | -1 | 111,394 | 112 | 12.644–12.803 s |

Each round has one additional state request and one history request; no replay
used chunked reads. The +200-event scenario's actual HTTP reply was about
65.8 KB; full recovery about 38.8 MB.

**Byte-count semantics**: HTTP JSON replies are counted before the test cutoff
filter, so they may include tail events the controller withheld. For example,
in the empty-increment scenario the real Agent still returned the terminal
event the test then withheld — the read response was 584 bytes, but 0 events
were applied to SyncEngine. This number must not be read as a real active run's
empty-response size.

This sample's current production cache-budget estimate is about 0.95 MB (not
the whole JS heap). Leaving actually called the default
`pruneCache("another-session")` and no eviction happened. Only this sample and
this cache occupancy were verified; multi-session contention, more than 8
sessions, or other large content could still cause eviction.

## Conclusions

- Normal complete-cache incremental reuse already works: the 111k prefix is not
  re-read, only the 200 follow-up events are backfilled.
- "Same visible content remains" and "safe to continue incrementally" are not
  the same state. Even with identical visible entries, baseline invalidation
  can trigger a full recovery.
- This round reproduced no wrong full replay from normal cached navigation, and
  observed no eviction of this sample by the default cache budget.
- The manually triggered invalidation control explains possible costs, but
  **cannot conclude a user's real slow session experienced this invalidation**.
- Keep the existing complete-cache policy; do not keep expanding the
  chunk/event-page budget, and do not delete baseline validation to fake an
  "instant open".

First opens and display-data-only paths still have real optimization room: the
server returns a restorable run-projection snapshot with a cursor consistent
with it; the client restores and only takes follow-up events. The Agent already
has an in-memory semantic projection implementation to base a later design on;
atomic watermark, tool-argument/thinking/terminal completeness, run switching,
old-desktop compatibility, and snapshot size still need verification. This
round adds no such protocol and does not pass ordinary history text off as such
a snapshot.

## This round's code change: only locatable diagnostics added

The existing `[remote] session timeline sync timing` log gains an optional
`replayPlan`, recorded when the read mode is chosen rather than reverse-derived
from the completed cache:

```json
{
  "mode": "full",
  "sinceIdx": -1,
  "cachedHighWater": 111393,
  "prefixDecision": "baseline-untrusted"
}
```

`prefixDecision` returns the first unmet condition; it is not a complete
lifecycle trace:

| Value | Meaning |
| --- | --- |
| `reused` | complete prefix reused, incremental read |
| `no-cache` | no cached timeline; may be first open, cleared, or evicted — this value alone cannot distinguish them |
| `baseline-untrusted` | content exists but no trusted baseline was established, or the existing baseline was invalidated |
| `run-inactive-or-changed` | the server's activeRun differs from the cache, or the run is no longer active |
| `missing-projector` | no restorable accumulated state |
| `not-streaming` | the cache or projector is no longer streaming |
| `prefix-incomplete` | the cursor prefix is incomplete |
| `missing-assistant` | the visible assistant entry matching the projector does not exist |
| `truncated` | the current run has a truncation marker |
| `reconcile-requires-full` | the current reconcile reason is not one of the open/reconnect that allow prefix reuse |

- Does not change the existing reuse eligibility, cursor, replay, or
  hint-completion conditions.
- Records this field only for the replay chosen after history-refresh; an idle
  that only refreshes history without reading replay has no `replayPlan`, and
  other pure-tail reconciles do not claim a complete-cache decision was made.
- Keeps the existing log throttling: dev builds output every time, release
  builds still only attempts of at least 1 second. No chat bodies, credentials,
  or new transport protocols.
- After adding the fields, one more round of the five browser scenarios: normal
  increment is `reused`; first time is `no-cache`; explicit invalidation and
  display-only cache are `baseline-untrusted`. Event counts, request starts, and
  recovered display content all remain identical. This round is functional
  verification and is not mixed into the first three rounds' performance scope.
- Mobile type-check, lint, 94 test suites / 1,306 tests pass; the browser
  measurement scripts pass TypeScript checks separately.

Raw de-identified metrics are in `streaming-sync-warm-measurement-2026-09-16.json`.

## Reproduction

Use the isolated-launch steps from the previous report, replacing only the
browser bundle entry:

```sh
node_modules/.bin/esbuild scripts/measure/measure-sync-warm.ts --bundle --platform=browser --outfile=target/sync-browser-measurement/bundle.js
python3 scripts/measure/measure-sync-browser.py --test-binary <built ignored test executable>
```

Open the ready.json URL and click the button; three rounds by default; add
`#rounds=1` to the URL for a one-pass five-scenario diagnostic verification.
The sample must have a long enough real prefix, a contiguous cursor, and a
trailing `agent_end`; the test fails without these conditions instead of faking
success.

After measuring, stop this run's runner (not the user's original agent),
confirm its two child processes ended and the private database copy was
deleted. Keep the de-identified metrics and clean up temporary logs and the
bundle.
