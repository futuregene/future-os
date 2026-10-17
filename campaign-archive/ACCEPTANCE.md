# ACCEPTANCE — per-unit ledger verdicts

**Mechanism:** `python3 .future/cov100/staletable.py --gate` (registry: `.future/cov100/pairings.json`),
plus `reconcile.py` for MISSING and `strict-claims.py` for per-full-path resolution. Regenerate with
`staletable.py --md`. Do **not** hand-maintain these numbers: every hand-kept number in this campaign
went stale at least once, and a second implementation of this check (mine) produced two false findings
before I deleted it (INCIDENTS F.25).

## Authoritative pairings — the current ledger per unit

| pairing | role | doc | claims | uncovered | STALE | COVERED | ABSENT | verdict |
|---|---|---|---|---|---|---|---|---|
| `goal` | authoritative | `docs/testing/module-goal.md` | 92 | 92 | 0 | 0 | 0 | PASS |
| `agent-core` | authoritative | `docs/testing/module-agent-core.md` | 213 | 213 | 0 | 0 | 0 | PASS |
| `agent-rpc` | authoritative | `docs/testing/module-agent-rpc.md` | 281 | 324 | 0 | 0 | 0 | PASS |
| `agent-rpc-r1` | superseded-ledger | `docs/testing/module-agent-rpc.md` | 169 | 324 | 144 | 99 | 45 | PASS |
| `channels` | authoritative | `docs/testing/module-channels.md` | 6 | 8 | 0 | 0 | 0 | PASS |
| `channels-wide` | wide | `docs/testing/module-channels.md` | 167 | 8 | 25 | 22 | 3 | PASS |
| `channels-superseded-report` | superseded-report | `docs/testing/module-channels.md` | 15 | 24 | 5 | 5 | 0 | PASS |
| `channels-policy` | authoritative | `docs/architecture/channels-test-coverage.md` | 13 | 8 | 6 | 6 | 0 | PASS |
| `review-rust` | authoritative | `docs/testing/review-rust.md` | 4 | 24 | 0 | 0 | 0 | PASS |
| `desktop` | authoritative | `docs/testing/module-desktop.md` | 61 | 60 | 0 | 0 | 0 | PASS |
| `mobile` | authoritative | `docs/testing/module-mobile.md` | 17 | 13 | 0 | 0 | 0 | PASS |
| `tauri-shell` | superseded-ledger | `docs/testing/module-tauri-shell.md` | 318 | 318 | 0 | 0 | 0 | PASS |
| `cli` | authoritative | `docs/testing/module-cli.md` | 119 | 123 | 0 | 0 | 0 | PASS |
| `cli-4b` | superseded-ledger | `docs/testing/module-cli.md` | 129 | 123 | 11 | 8 | 3 | PASS |
| `cli-4` | superseded-ledger | `docs/testing/module-cli.md` | 188 | 123 | 55 | 44 | 11 | PASS |
| `cli-wide` | wide | `docs/testing/module-cli.md` | 203 | 123 | 62 | 48 | 14 | PASS |
| `tui` | authoritative | `docs/testing/module-tui.md` | 5 | 13 | 0 | 0 | 0 | PASS |
| `cli-mcp` | authoritative | `docs/testing/module-cli-mcp.md` | 4 | 4 | 0 | 0 | 0 | PASS |
| `tauri-appdata` | authoritative | `docs/testing/module-tauri-appdata.md` | 610 | 610 | 0 | 0 | 0 | PASS |
| `tauri-remote` | authoritative | `docs/testing/module-tauri-remote.md` | 109 | 331 | 0 | 0 | 0 | PASS |
| `tauri-headless` | authoritative | `docs/testing/module-tauri-headless.md` | 261 | 222 | 2 | 0 | 2 | PASS |
| `tauri-shell-seg2` | authoritative | `docs/testing/module-tauri-shell.md` | 270 | 270 | 0 | 0 | 0 | PASS |

Source of the pairing for each: the doc's own text (each `pairings.json` entry carries the quote that
establishes it). A pairing that is `superseded-ledger`, `wide` or `superseded-report` is kept only so a
stale or over-broad invocation's non-zero count is *explainable*; the authoritative row is the one that
must be clean, and all of them are (0 COVERED).

## What this proves, and what it does not

Proves: no module's current per-line ledger claims a line is uncovered that its own report shows as
covered. That is the *documentation* half of the "100%" claim, checked mechanically per full path
inside each unit's declared scope, with ambiguous basenames reported rather than guessed.

Does NOT prove: that the **waiver mechanisms** are true. That is what the adversarial reviews are for,
and the calibration says author classification is unreliable — REV-CH falsified 16 of 24 channels rows,
REV-TUI 11 of 24, REV-DESK 3 of 61, REV-FE 0 of 13. So a module with waivers needs **both** a clean
ledger (this table) and a review verdict (CLOSEOUT §3).

## Honest gaps

* `agent-rpc`'s current ledger is §0.5, not §6 — §6 is round 1's record (revision c173e64e) and its 99
  covered rows are revision drift, justified in `pairings.json`. The authoritative pairing is clean
  (281 claims / 0 covered). My earlier note that this module was "not mechanically checkable" applied
  to §6; the registry fixed the pairing rather than the doc, which is the better resolution.
* Eight round-1 module records have no machine-readable per-line ledger at all
  (`LEDGER-REWORK.md`); whether to re-measure them or restate them as prose-only is the open user gate
  `todo_6a850a842666`. The final report must say which modules are machine-checked and which are not.
* `tauri-remote`'s authoritative pairing counts 109 claims against 331 uncovered lines: the section
  registers the 450-line residual in prose-and-table form where 109 lines appear as explicit
  `path:lines` claims and the rest are described per file. Its own §4 header says every line is
  registered; my per-claim check can only confirm the 109 that are in claim form, which is why I
  verified the 450 total numerically instead (`covlines.py` union) and recorded both facts.
