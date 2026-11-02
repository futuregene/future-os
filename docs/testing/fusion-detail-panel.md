# Fusion discussion detail panel verification

Date: 2026-11-02 (Asia/Shanghai). Design: [English](../architecture/fusion.md), [中文](../architecture/fusion.zh-CN.md).

## UI boundary and application exercises

The real Desktop React components ran in the repository screenshot harness against synthetic discussion records. The transport boundary is mocked; no user database, credentials or paid model requests were used. Scenarios live in `scripts/screenshots/scenarios.d/fusion-detail.json`; the existing `fusion-progress` scenario still checks the compact inline activity.

The exercised flows cover opening the detail panel from inline activity, returning to the Runs tool list, preserving public responses after synthesis failure, joining UTF-8 transcript pages through the server cursor, an active discussion without a premature conclusion, moderator JSON rendered as readable issues/stances, and the dark appearance. The remote exercise opens an existing remote conversation and dispatches the same typed inspection event with the shipping host-bound resolver; the real remote request adapter reads the mock host's discussion into the same detail component. It is a boundary exercise, not a claim of a paired-device run. The Runs scenario checks the focused question and dedicated-panel routing from the original list entry.

Screenshots were visually inspected for normal, dark and remote layouts. A long transcript assertion checks the rendered public response. The inspector no longer exposes an original-message/tool-evidence disclosure and shows only the last report rather than earlier drafts; raw records remain available through terminal queries. Opening another discussion or conversation cannot reuse the previous resource's data.

## Static checks and CI regressions

Desktop's project ESLint/typecheck and documentation checks cover the implementation. ESLint retains the existing unrelated JSDoc warning in `features/agent/runtime/liveRunProjection.ts`. No Rust, RPC schema or database migration changed.

Regression cases are authored for lazy step reads, byte-cursor paging, failure-material retention without a stale conclusion, late-response rejection after a parent switch, remote-read failure without a local fallback, context-panel routing before the projected tool list arrives, thread-switch isolation, and return from remote discussion details to remote files. Unit tests are not executed locally; they belong to CI under repository policy.

This record does not claim Android/iOS or native Windows/Linux device interaction. Mobile's compact activity and the terminal's original-message queries retain their existing behavior; this change adds a Desktop inspection surface over the same read-only discussion records.

## Compact title ring follow-up

Desktop and Mobile now show a 14px four-segment ring without center text beside the inline Fusion title, including while collapsed. The expanded discussion topic precedes model rows, which use icons/colors with accessible state descriptions; stages 2 and 4 filter to the moderator, including legacy records without a configuration snapshot. Compact metadata reads continue while collapsed, stop at a terminal discussion, and do not load full child transcripts. Expanding after a failed metadata read allows a new read attempt.

The updated `fusion-progress`, `fusion-progress-partial`, `fusion-progress-running`, `fusion-ring-collapsed` and `fusion-ring-moderator` screenshot scenarios cover completed, reduced-participation and active states, collapsed-title visibility, topic-before-model ordering, stage-2 role filtering, and active synthesis role filtering (`fusion-ring-summary`) on both frontends. Mobile exercises use the real React Native components in the Expo web harness with an isolated set of Web-only dependencies; they are not native-device runs.

Fusion details are inspected within Runs, without a separate panel type. Internal moderator preflight remains recorded but is omitted from user-facing details; inline activity calls this preparation rather than exposing readiness checks.


## Ordinary report presentation (2026-11-02)

The detail header now groups status/completeness on one row. The duplicate conclusion block is removed; users read the final moderator report under tab four. Expanding an entry fetches its complete paged transcript internally, selects the final public report and renders the existing Markdown/structured-report view. The UI does not expose Read more or a truncated JSON preview. Cancellation stops follow-up pages after collapse or selection changes; read failures retain a localized retry action and remote reads remain host-bound.

The real React screenshot harness passed four scenarios: `fusion-detail`, `fusion-detail-paging`, `fusion-detail-review-report` and `fusion-detail-final-report`. The long review fixture crosses the 32 KiB transport boundary, renders its current answer without JSON field names and retains its final marker without manual paging. Remote-source and responsive-tab scenarios also passed. Screenshots of the long review, final-tab report and responsive layout were visually inspected. Harness data is synthetic; no model calls or user databases were used. Desktop ESLint/typecheck passed with the same pre-existing unrelated JSDoc warning noted above. Unit tests remain assigned to CI.
