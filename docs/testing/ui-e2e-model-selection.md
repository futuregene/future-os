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
