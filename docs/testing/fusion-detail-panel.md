# Fusion discussion detail panel verification

Date: 2026-11-02 (Asia/Shanghai). Design: [English](../architecture/fusion.md), [中文](../architecture/fusion.zh-CN.md).

## UI boundary and application exercises

The real Desktop React components ran in the repository screenshot harness against synthetic discussion records. The transport boundary is mocked; no user database, credentials or paid model requests were used. Scenarios live in `scripts/screenshots/scenarios.d/fusion-detail.json`; the existing `fusion-progress` scenario still checks the compact inline activity.

The exercised flows cover opening the detail panel from inline activity, returning to its discussion list, preserving public responses after synthesis failure, joining UTF-8 transcript pages through the server cursor, an active discussion without a premature conclusion, moderator JSON rendered as readable issues/stances, and the dark appearance. The remote exercise opens an existing remote conversation and dispatches the same typed inspection event with the shipping host-bound resolver; the real remote request adapter reads the mock host's discussion into the same detail component. It is a boundary exercise, not a claim of a paired-device run. The Runs scenario checks the focused question and dedicated-panel routing from the original list entry.

Screenshots were visually inspected for normal, dark and remote layouts. A long transcript assertion checks the rendered public response, rather than counting text hidden inside the optional original-record disclosure. Viewing original messages remains opt-in; opening another discussion or conversation cannot reuse the previous resource's data.

## Static checks and CI regressions

Desktop's project ESLint/typecheck and documentation checks cover the implementation. ESLint retains the existing unrelated JSDoc warning in `features/agent/runtime/liveRunProjection.ts`. No Rust, RPC schema or database migration changed.

Regression cases are authored for lazy step reads, byte-cursor paging, failure-material retention without a stale conclusion, late-response rejection after a parent switch, remote-read failure without a local fallback, context-panel routing before the projected tool list arrives, thread-switch isolation, and return from remote discussion details to remote files. Unit tests are not executed locally; they belong to CI under repository policy.

This record does not claim Android/iOS or native Windows/Linux device interaction. Mobile's compact activity and the terminal's original-message queries retain their existing behavior; this change adds a Desktop inspection surface over the same read-only discussion records.

## Compact title ring follow-up

Desktop and Mobile now show a four-segment ring and current stage `x/4` beside the inline Fusion title, including while collapsed. Expanded model rows use icons/colors with accessible state descriptions; stage 2 filters to the moderator, including legacy records without a configuration snapshot. Compact metadata reads continue while collapsed, stop at a terminal discussion, and do not load full child transcripts. Expanding after a failed metadata read allows a new read attempt.

The updated `fusion-progress`, `fusion-progress-partial`, `fusion-progress-running`, `fusion-ring-collapsed` and `fusion-ring-moderator` screenshot scenarios cover completed, reduced-participation and active states, collapsed-title visibility and stage-2 role filtering on both frontends. Mobile exercises use the real React Native components in the Expo web harness with an isolated set of Web-only dependencies; they are not native-device runs.
