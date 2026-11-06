# Model-assisted UI end-to-end tests: let the model pick elements, not coordinates

A prototype that drives the real desktop renderer (headless Chrome, mock data) using the
one capability the model is actually reliable at: **choosing among discrete, numbered
targets**. It never emits coordinates; geometry comes from the DOM and the click is a real
mouse event dispatched over CDP.

This is a **case-authoring aid**, not a CI substitute — see "Where this fits" at the end.

## Shape of one step

```
CDP enumerates interactive elements        (DOM → label + bounding box)
        ↓  red boxes + numbers drawn over the screenshot (Set-of-Mark)
model sees only the numbers → {"id": N}
        ↓  executor clicks the DOM-reported centre with Input.dispatchMouseEvent
re-screenshot + read DOM text
        ↓  assertion: text lines that were NOT on screen before / new data-testid
```

Because the model produces no coordinates, the classic "clicked 3 px off" failure mode
disappears. What is left is a single question per step: *which element completes the goal?*

## Measured on the desktop app

Subject: `scripts/screenshots/capture.py serve-desktop` (headless Chrome, mock data),
viewport 1280×850, 84 interactive elements on the first screen. 3 runs × 4 scenarios.

| Scenario | Goal | Passed | Mean decision time |
|---|---|---|---|
| S1 | open model picker | 3/3 | 1.46 s |
| S2 | open skills list | 2/3 | 2.56 s |
| S3 | open scheduled tasks | 3/3 | 1.89 s |
| N1 | open *printer settings* (does not exist) | **3/3 correctly refused** | 2.68 s |
| | **total** | **11/12 (92%)** | |

Evidence used by the assertion — text lines added by the click:

- S2 (pass): `Skills` / `管理 Agent 可用的技能：安装、卸载与浏览技能市场。` / `已安装` / `全部`
- S3 (pass): `新建定时任务` / `每周进展周报` / `周一, 周五 10:00`

## Three findings worth keeping

**1. Assert on *added* text, never on "contains a keyword".**
The sidebar buttons are themselves labelled 「模型」「技能」, so `text.includes('技能')`
is true before the click. The first version of this harness used that check and reported
4/4 green — every one of them false. The assertion has to diff the text (or the `data-testid`
set) between before and after, and only then look for the keyword inside the delta.

**2. Give the model an explicit refusal.**
With a system prompt that only allowed "return a number from the image", the negative
scenario picked 「全部设备」 — the closest-looking thing. After adding *"if no element can
complete the goal, return id 0"*, it refused correctly in 3/3 runs. Whether an agent can
decline "the target does not exist" depends on whether you gave it the option.

**3. The remaining failure is "picked the neighbour", not "could not pick".**
S2 failed once in three runs: 「定时任务」 was chosen for "open skills list". Those two
buttons are the same size and vertically adjacent — the same error shape seen in dense lists
elsewhere. Cheap mitigations: pre-filter candidates by label before numbering, or make the
number chips larger relative to the target.

## Where this fits

`harness/` already owns video / console / network / IPC / database / perf channels with
scripted assertions. This layer is **complementary**: it answers "which element", and
everything else stays deterministic.

Recommended usage — **generate, then freeze**: the model appears once while the case is
being written; the element it picked is frozen into a `data-testid` selector and the case
runs in `harness/` like any other. At 1.5–2.7 s per decision, a dozens-of-steps regression
is minutes of pure waiting plus an ~8% pick error, which is exactly what CI cannot absorb.

## Reproducing

```bash
python3 scripts/screenshots/capture.py serve-desktop --port 5399   # desktop renderer
FUTURE_API_KEY=... node harness/scripts/som-element-picker.mjs --url http://localhost:5399/shot/
```

`harness/scripts/som-element-picker.mjs` is the prototype driver: it enumerates elements,
draws the marks, asks the model, clicks, and prints the before/after text delta. It needs a
vision model endpoint and an API key from the environment; it is a template for writing
cases, not something CI calls.

## Judging the recording with a video model

Scripted assertions only know what they encode. A second layer watches the recording:
`harness/scripts/video-judge.mjs` sends the flow video to a video-capable model
(`qwen3.8-max`; the DeepSeek models in the catalogue take images, not video) and asks
whether the run matched the expected flow.

Measured on the desktop renderer, 2026-11-06 (8 fps capture, 6–9 s clips, 152–179 s per
judgement):

- clean run → `pass`, with per-step confirmation (panel titles, 5 installed skills matching
  the sidebar badge, scheduled-task list);
- run with three injected anomalies → `fail`, and all three were named with the right step:
  a `加载失败：无法获取模型列表 (500)` banner, the same banner persisting across later screens
  and occluding content, and a `（此处应显示技能列表，但内容为空）` placeholder contradicting a
  list that was in fact rendered.

Cost and latency rule out per-assertion use: budget ~3 minutes per case and run it
asynchronously after the scripted pass, as a second opinion rather than a gate.

## A zero false-positive rate is only half a metric

Tightening the judge until it stops flagging healthy runs is easy and it is not
progress. Measured on 2026-11-06, same desktop renderer:

| prompt | false positives (green runs judged fail) | catch (recorded defects found) |
|---|---|---|
| v1 "verify every step" | 2/8 (25%) | 3/3 on a clip with a stuck red banner + contradictory empty state |
| v2 "report only what you can point at" | **0/8** | 3/3 on that same clip; **0/9 on fixture-level faults** |

The 0/9 is the useful part. `harness/cli.mjs run --fault {api-500,empty-list,hang}`
injects real fixture faults, but their *symptoms* are not visible in a 4–11 s
recording: `hang` looks like normal startup, `empty-list` still renders workspace
groups, and `api-500` is swallowed by the desktop into an empty sidebar. The judge
called all nine "normal loading", and frame-level checks agreed with it.

So: **report false positives and catch rate together, each with its n.** A judge
that reports nothing scores perfectly on false positives and is worthless; a judge
that reports everything scores perfectly on catch and is unusable. Neither number
means anything alone, and a suite of green runs can only ever measure the first.

### The judge is only as good as the defect's visible symptom

> **A judge's value depends on the defect having a visible symptom; a defect with no
> symptom cannot measure a catch rate.**

Round one injected three fixture faults and the judge caught **0/9**. The faults were
real — the cases failed their own assertions — but nothing was *visible*: `hang` looks
like a normal slow start, `api-500` is swallowed by the desktop into an empty sidebar,
and a list that returns nothing looks like a list that never had anything.

Before that, an injector reported `applied: 148` (148 successful DOM writes over 45 s)
and the recording still showed a clean run: **an injector's own success counter is not
evidence that the symptom reached the video**. Verify with a frame grab and an OCR
question, not with the injector's log.

Giving each fault a painted symptom fixed it:

| fault | symptom | caught |
|---|---|---|
| `api-500` | red banner that survives re-renders | 4/4 |
| `dead-click` | greyed button + 「正在发送，未响应…」 pill | 3/3 |
| `empty-list` | blank rail + 「暂无工作区，暂无会话」 | 1/3 → **2/3** after anchoring the mask on geometry instead of on an element |

The one remaining miss is the judge's, not the fixture's: the recording plainly shows a
blank rail next to a rendered conversation, and it reported both facts while still
returning `pass`. That is the next thing to look at — with a v2/v3 comparison, not by
editing v2 in place.

### Judging policy: recall for the review queue, precision for a gate

Measured on 2026-11-06 over the injected defect set and the green recordings:

1. **`v4` (recall-first) feeds a review queue, never a gate.** Its prompt asks for
   anything that looks wrong and accepts that over-reporting is fine, because a
   missed defect is unrecoverable while a false alarm costs one look. Result:
   **10/10 injected defects caught**, ~260 s per green case, 1–3 observations each.
   Confirmed observations are promoted to the issue store by a human — the judge
   itself files nothing.
2. **Strict `v2`/`v3` are for an actual gate only.** They report only what can be
   pointed at (contradictory content on one screen, a persistent banner, a control
   that never responds) and scored **0/8 false positives**. That is the right
   behaviour once a suite is stable and a gate is wanted.
3. **`v4` must never be a gate.** It flags **100% of green runs**, so gating on it
   blocks every healthy build.

Sample sizes, so nobody reads this as a complete study:

- green recordings: **n = 3 of 8** under `v4` (the remaining five were not run);
- the `v2`/`v3` columns are **partial samples from an interrupted batch** (n = 7 and
  n = 6), not a complete same-batch pass.

The conclusion above does not depend on those gaps — three green runs were already
enough to see that what looks like a false positive is really a review ticket — but
the numbers should be quoted with their n.

### What a green suite does not say

A full pass read **122/122 cases passed** on 2026-11-06. That number is narrower than
it looks: **the cases only assert what someone encoded.** They cannot see a stale
line of copy, a one-frame flash, a colour that drifted, or two halves of a screen
contradicting each other unless a case happens to check exactly that.

Two probes make the gap concrete.

**A "failing" case can be the harness, not the product.** `42-workspace-delete` was
failing because the confirmation copy had been deliberately split into four variants
(active-only / archived-only / mixed / all-archived). The product got *more* precise;
the case still matched one sentence. Fixed by asserting the contract — workspace
leaves the sidebar, its conversations go with it — instead of the wording.

**A judge quote is not evidence until it is checked.** A `v4` scan of two of the most
interactive recordings produced eight observations; the most alarming one quoted the
sidebar as reading 「暂无会话 yet 工作区/对话」. The source has neither that sentence
nor the word "yet": it has two separate strings, `暂无工作区对话` and `暂无对话`.
The judge had merged and paraphrased them. **Verify the quoted text against the frame
(or the locale file) before opening anything** — otherwise the review queue fills
with ghosts.

Cost of that probe, for planning: two recordings, 170 s and 263 s, 11.3k + 6.1k prompt
tokens and 6.9k + 12.7k completion tokens — **0.38 and 0.53 credits**, ~0.9 credits for
two. Budget roughly half a credit per recording and ~4 minutes each; a 15-case sweep
is ~7 credits and an hour, not a CI step.

### What `v4` is actually for

Two probes over eleven real, *passing* recordings (2026-11-06): **nine were judged
`fail`, and not one of them held up.** Four were refuted outright —

- a red Delete button read as *enabled* when the component sets
  `disabled={selectedCount === 0}`; the destructive variant is red either way;
- 「设置弹窗只打开了 4 个页签」 on a case whose 12/12 checks passed, i.e. every tab
  did open and the recording simply switched fast;
- 「余额 0 credits」 — the fixture's fake account balance, not a defect;
- 「暂无会话 yet 工作区/对话」 — text the app does not contain; the model merged two
  locale strings (`暂无工作区对话` and `暂无对话`).

So `v4`'s use on real green runs is **"where to look", never "what is broken"**. Its
characteristic error is **inferring state from appearance** (red ⇒ enabled), followed
by **paraphrasing or splicing adjacent copy**.

Two constraints now enforce that in `harness/scripts/video-judge.mjs`, each with a
regression in `video-judge.test.mjs` (wired into `make test-ui-harness`):

1. **A transport failure is not a verdict.** One retry, and the result records
   `retried`/`attempts`; two failures exit 2 instead of becoming a pass or a fail. Two
   of thirteen recordings were previously lost to a 300 s HTTP/2 stream timeout.
2. **Every issue must quote what it saw, verbatim.** No quote, a quote containing
   words that are in neither the app's locale bundles nor the case's captured text, or
   an interactivity claim whose quote carries no state evidence — all are dropped, and
   if nothing survives the verdict becomes `pass`.

### The one surviving "layout overlap" was not a defect

`batch-select-all-cancel` was reported as having the multi-select toolbar drawn over
the list rows. Two independent checks say otherwise:

- **Structure**: `ActivityRailSelectionToolbar` is an ordinary in-flow block
  (`border-t shrink-0 …`) rendered as a sibling of the list inside a flex column, and
  `globals.css` contains no `position: absolute|fixed` rule that could reach it.
- **Frames**: in the 35 frames covering 6.5–10.0 s, the ink density of the left rail
  stays within ±1% of its median; overlapping double-rendered text would spike it.

Verdict: a misread, not a layout bug. No issue was opened.

### Fixtures are static; bugs are dynamic

Replaying 100 historical UI fixes against the case library caught **57%** of them, and the
misses were not random. What separates a caught bug from a missed one is usually whether
its **condition exists in the fixture**:

- caught: a hint that should be sized to its text, a row whose order changes, a panel that
  must stay reachable in a narrow window — all visible in a single static state;
- missed: control moving from another device (the page reads a copy once), an admission
  refusal that never happens, an ordering that only changes after an event.

Two fixture capabilities change that, both additive and inert unless a case asks:

- `?peers=held` — a paired machine held by another device, so the row hint renders at all
  (without it, `remote_peer_list` returns `[]` and the surface is untestable);
- `?peers=transfer` — the same, but control returns to this machine after five seconds, so
  a case can watch a row **follow** control instead of a snapshot that never moves.

Caveat learned the hard way: a transfer assertion that is green both before and after
reverting the fix does not guard that fix — it only guards "the row never updates at all".
Add the capability, then **prove the assertion goes red on the reverted commit**; otherwise
it is a test of nothing.
