# Porting the rescued `wb-mentions` snapshot (2026-10-26)

Not a design document: the record of taking one commit that exists nowhere but a
local branch — `8958cf069`, *wip(tui): preserve the file-mention and permission
work left uncommitted here* — and landing the parts of it that could be shown to
work against the workbench as it stands today.

The snapshot was never a branch anyone could review. Its own message says so: the
branch tip was still the `dev-huichen` snapshot, so all of the work existed only
in one worktree's working tree, and it was committed *as-is* to stop a
`git worktree remove` from deleting it. Nothing in it had been rebased, reviewed
or run against the surface it now had to live in.

Its subject matter — `@`/`#` mentions, a palette with skills in it, a per-goal
to-do board — had also been overtaken: between the snapshot and this port, three
PRs rebuilt the same files. The palette had become data-driven and bilingual, the
draft had become an `EditorState` with undo and paste stacks, the key table had
become rebindable, and `ctrl+p` had been given to the palette. So the port is a
re-implementation against the current contract with the snapshot as the
specification, not a merge.

## 0. The snapshot, by the numbers

| | |
|---|---|
| Commit | `8958cf069` (local branch `feat/wb-mentions`, never pushed, never in a PR) |
| Size | 1926 added / 151 deleted, 13 files, 86 hunks |
| Shape | 10 source files under `tui/src/workbench`, 2 screenshot scripts, 2 PNGs |
| Provenance | a rescued working tree; no review, no tests run at the time, no CI |

## 1. Sorting it: what could land

Each function point was judged on its own: does it have a test that proves the
behaviour, does it compile and run on its own, and does its consumer still exist
in the surface it is landing on?

| # | Function point | Added lines (approx.) | Verdict | Landed as |
|---|---|---:|---|---|
| 1 | `@` file and `#` conversation completion menu (state, token, rows, insert, keys, click, painted box, file listing) | ~520 | **complete** | `feat/wb-mention` (`8d8567114`) |
| 2 | `@path` inlined into the prompt on submit (`<file name=…>` blocks, the CLI's own shape) | ~160 | **complete** | `feat/wb-mention-inline` (`e85fa9216`) |
| 3 | `/` palette lists installed skills; `tab` completes a row into the draft | ~300 | **complete** | `feat/wb-palette-skills` (`c1b7e04b2`) |
| 4 | 待办 panel names the goal it belongs to; `/todo` focuses it | ~220 | **complete** | `feat/wb-todo-goal-label` (`8d7f48e2f`) |
| 5 | one terminal read carrying several keys is split into keys | ~100 | **complete** | `fix/wb-key-chunk-split` (`873b2468b`) |
| 6 | the capture harness writes scripted input completely (`write_all`) | 18 | **complete** | `feat/wb-capture-steps` (`245824ff8`) |
| 7 | `/models` scope picker (a multi-select over the catalogue) | ~215 | **half** | not landed — see §2.1 |
| 8 | `/permission` picker | ~60 | **redundant** | not landed — see §2.2 |
| 9 | `ctrl+p` / `ctrl+t` / `ctrl+y` / `ctrl+m` cycle keys | ~70 | **conflicts** | not landed — see §3 |
| 10 | status-bar segments as click targets | ~46 | **half** | not landed — see §2.3 |
| 11 | `/thinking` alias, `/models` and `/todo` verbs, `HELP_KEYS` rewrite | ~25 | **mixed** | only `/todo` landed (with #4) |
| 12 | frame de-duplication, `?2026` synchronized update, tick→workbench | ~51 | **superseded** | not landed — see §2.4 |
| 13 | paste routed into an overlay's query field | ~20 | **half** | not landed — see §2.5 |
| 14 | the rail opens on 会话 instead of 待办 | 1 | **preference** | not landed — see §2.6 |
| 15 | two screenshots of the new menus | 2 PNGs | **stale** | not landed — see §5 |
| 16 | capture scenarios for the two new menus | ~19 | **needs a real cwd** | not landed — see §5 |

About **480 of the snapshot's 1926 added lines (25%) did not land**, and the
"half" core of it is item 7: roughly **215 lines of a picker whose only consumer
is gone** (§2.1). The line counts above are whole-hunk additions attributed to
the function point the hunk implements; the landed column counts what the six
units actually added, which is larger than the snapshot's because the port
re-expressed every sentence through the i18n table and added tests at the
current surface (1711 added / 117 deleted in total).

## 2. What did not land, and why

### 2.1 `/models` scope picker — a full control with no reader left

The snapshot added `PickerKind::Scope`, a `marked` set on the picker, `space` to
toggle a row, `SetModelScope`, and persisted the set to `settings.enabledModelIds`
— and nothing else: a scope exists only to bound *which models a keystroke walks*,
and that keystroke was `ctrl+p` (item 9), which the current surface has given to
the palette. Turning the picker on would put a menu on screen whose only effect
is a preference nothing reads: the definition of the half-baked bucket. Rejected
rather than merged with a guessed consumer.

### 2.2 `/permission` picker — already a command and a chip

`live.rs` already has `set_permission_level`, `cycle_permission` (the launchpad's
access chip and `/permission <level>`) and the level ladder `all → workspace →
none`. A picker over the same three values adds a second road to a value that
already has two, and its rows would have to be re-expressed as i18n keys to be
drawn at all. No new capability, so not ported.

### 2.3 Status-bar segments as click targets

Two thirds of that change point at the pickers it wants to open (`status.model`
and `status.effort` → the model picker, `status.permission` → the permission
picker of §2.2, which is not landing). The row is also `fit()`-truncated on a
narrow terminal, so a segment's rectangle can be placed past the text that was
painted — a click target over empty cells. Not ported: it would need a
narrow-terminal pass of its own, and its permission segment has no picker.

### 2.4 Frame de-duplication, `?2026`, tick dispatch — solved differently

The snapshot's `previous_frame` cache answers "did the bytes change?"; the driver
now owns that question properly through a `TickClock` (`tui-event-loop-cost`),
and `?2026` (a synchronized update) is a *different* defect the snapshot bundled
in — no test, unrelated to the mentions, and it belongs with the loop's own cost
work rather than in a port of a mention menu. The tick dispatch existed only to
open the `@` menu after a paste; the menu is now synced when the frame is
composed, which needs no tick at all.

### 2.5 Paste into an overlay's query field

A real gap, but the snapshot's version covers two of the surface's eight
overlays (palette, picker) — the current surface also has preferences,
keybindings, queue, approval, diff, history, tree, fork and worktree. Landing a
two-eighths fix reads as "paste works", which it would not. Recorded here as a
defect worth its own pass.

### 2.6 The rail's default panel

`panel: PanelId::Sessions` instead of `Todo` is a preference, not a feature, and
the current default has tests asserting the todo board is what the rail opens on.
Not a port: a different product decision, and not this record's to make.

## 3. Keybinding collisions (the reason item 9 is out)

| Keystroke | What the snapshot wanted | What the surface already owns | Resolution |
|---|---|---|---|
| `ctrl+p` | cycle to the next model | `preferences::Action::Palette` — a *rebindable* action, `/preferences` shows it | existing wins; model cycling stays on `/model` |
| `ctrl+r` | open the sessions picker | `preferences::Action::Sessions` — same thing already | nothing to port |
| `ctrl+t` | cycle the reasoning level | free, but its only use was the model scope (§2.1) | not landed |
| `ctrl+y` | cycle the permission level | the draft editor's `ctrl+y` (yank) | existing wins |
| `ctrl+m` | open the model picker | unused, but identical traffic to `ctrl+enter`/`Enter` on many terminals | not landed |

The `@` / `#` / `/` menus this record does land take no global chords: `@` and
`#` are *draft tokens* (a token that starts with the sigil opens a menu; every
other key still edits the draft), and `/` was already the palette's key.

## 4. The two hardest conflicts, and which side won

**A. `palette_entries()` was `(command, description)` pairs; the snapshot wanted
a row type.** The current palette builds its descriptions from the i18n table and
registers one hit region per painted row, with the row's ordinal equal to its
painted line. Adding skills inserts a *heading* line, which breaks that identity.
Resolution: take the current structure (`Vec<(&str, Key)>` + `t(lang, key)` +
per-row regions) and add the row kind on top of it — `PaletteRow::{Command,Skill}`,
one heading printed at each kind change, and the hit-region line counter advanced
inside the same loop that paints, so a click cannot land one row off after a
heading. The snapshot's separate `paint` loop was not taken: two loops that must
agree is exactly the bug the current code avoids.

**B. The draft is an `EditorState`, not `InputView { text, cursor }`.** Every
mention edit in the snapshot is a pair of byte offsets into a plain string, and
the snapshot's own `WbAction::FillDraft` did not exist in the current surface.
Resolution: keep the current editor and express both the insert and the palette's
completion as the existing `WbAction::EditInput { text, cursor }`, which
`Live::set_draft` applies through `EditorState::set`. What was *not* taken is the
snapshot's placement of the menu's sync in the tick handler: the menu is now
re-derived in `handle()` **and** in `render()`, so it cannot depend on a tick
arriving — which also removed the need for the loop change in item 12.

Both are cases of the same rule: where the two sides disagree about a *structure*
the new feature has to live inside, the current structure wins and the feature is
adapted to it; only the feature's own lines were taken as-is.

## 5. Screenshots: not updated, deliberately

The snapshot's two PNGs were rendered from the surface *as it was in that
worktree* — before the bilingual rebuild, with a different rail, status row and
composer. They are not evidence about the code that landed, so they were dropped
rather than refreshed from a different build.

A fresh still was attempted and rejected for a reason worth recording: the
scripted capture harness runs the demo agent, whose session cwd is the placeholder
`/Users/you/future-os`, so the `@` menu there lists **nothing** and the frame says
`没有匹配`. That is an honest frame, but a poor picture of the feature — the
listing is real only when the session's cwd exists. The route for a real shot is
the harness's `--addr` path against a live session in a real checkout:

```bash
cargo build -p future-cli --bin future
python3 scripts/screenshots/capture.py --out docs/tui/assets tui mention.png \
  --scenario chat --addr <a live agent's addr>   # then type `@` in the frame
```

`docs/tui/workbench-spec.md` §10 therefore still lists the previous set. Its
table is not wrong; it is simply older than this feature.

## 6. i18n

Every sentence the new chrome draws is a row in `workbench/i18n.rs` — 14 new
keys, `ALL_KEYS` 197 → 211: 5 for the mention menu (two titles, two hints, the
empty line), 2 for the palette's headings, 2 for the help table, 4 for the to-do
board's own states, 1 for `/todo`. The
guard (`tui/tests/i18n_guard`) runs green with no new file exemption and no new
allowlist row.

The one exemption this record raised is in `BUDGET` (35 → 37), for the two
`@path` inlining notes in `live.rs` (`[{path}: 300 KB, not inlined …]`). They are
**prompt text for the model** — never drawn — so they are not chrome the table
should carry; they are marked in place with `// i18n: prompt text for the model`
rather than added to the allowlist.

## 7. Verification

Run in this branch's worktree on the pinned toolchain (`1.99.1`).

| Check | Result |
|---|---|
| `cargo test -p future-tui` | **1249 lib tests + 21 integration tests, 0 failed** (integration: `cli_smoke` 6, `i18n_guard` 8, `perf_loop_probe` 1, `signal_shutdown` 6) |
| `cargo test -p future-tui --test i18n_guard` | 8 passed; 37 exemptions against `BUDGET` 37 |
| `cargo clippy -p future-tui --all-targets -- -D warnings` | clean |
| `cargo fmt -p future-tui --check` | clean |
| `cargo check -p future-tui --all-targets` (examples included) | clean — `tui/examples/workbench_semantics.rs` updated for the palette's new row type |
| the portrait frames | hand-checked at 100×30 and 120×38: the mention box ends one row above the composer's top edge and never covers the draft; the palette prints each heading once |

New tests, by unit: 7 for the mention menu (token boundaries, file and session
lists, the insert that replaces exactly the token, `esc` that closes without
touching the draft, an unmatched list that still submits, the painted box above
the composer, the two listing roads), 3 for the inlining (block shape, the three
"cannot inline" notes, size limit and punctuation), 4 for the palette (two kinds
and their order, filtering to a skill, `enter` filling rather than running, `tab`
completion, painted headings with a click that stays aligned), 3 for the board
(named from the registry, absent goal, `/todo`), 3 for the key-chunk split plus
the incomplete-sequence case, and 2 for the working-directory listing.

## 8. Known gaps this record leaves behind

- **Paste into an overlay's query line** (§2.5): still goes to the draft when a
  picker or the palette is up.
- **`@` in a demo/live-mismatched cwd** lists nothing, by design: the menu reads
  the *session's* directory. A session attached to a cwd that does not exist on
  this machine therefore shows an empty menu rather than a stale list.
- **`ctrl+alt+[` and other two-byte escapes** are still read whole by the key
  parser; the split in item 5 refuses to guess and leaves a chunk it cannot
  decompose as text.
- **The model scope** has no home until something walks models with a key that
  is not `ctrl+p`; the persisted `enabledModelIds` setting stays unread.
