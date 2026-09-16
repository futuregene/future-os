# Composer scroll consistency after long paste (2026-09-14)

## Scope and reproduction

Baseline: `c65c1f8b`, including the prior caret-reveal fix from #591.
Tested the actual `MentionEditor` and product CSS in a bottom-aligned flex
layout with a toolbar, using Playwright on Windows. Engines: WebKit 26.5
(build 2336) and Chromium 151.0.7922.34. No agent, model calls, or production
chat data were used. Paste was a synthetic `ClipboardEvent` carrying plain
text; subsequent typing and Shift+Enter used browser keyboard automation.

This is WebKit-engine evidence, **not native macOS WKWebView validation**.
The macOS system clipboard and real Chinese IME candidate UI remain untested.

Reproduction sequence:

1. Paste `"long text ".repeat(1000)` into an empty editor.
2. Press Shift+Enter, then type enough characters to wrap onto another line.
3. Repeat with one and two trailing newlines in the pasted text.
4. Measure editor `scrollTop`, height/top, and the collapsed selection range's
   rectangle after typing, rather than inferring movement from screenshots.

At a 1000x800 viewport (600px-wide host), the baseline WebKit editor was capped
at 320px. After paste + newline, its scrollTop was 1943px and the caret bottom
was 747.5px. At the first soft wrap it scrolled to 1968px: **25px for a 20px
line**, moving the caret bottom to 742.5px. This exposes a mismatch between
manual glyph-edge scrolling and native caret reveal.

Trailing-newline paste also had incorrect geometry: WebKit measured the prior
line, while Chromium returned no collapsed range rectangle and left scrollTop
at zero until a subsequent edit.

## Correction

- Use full line height (including leading) and editor padding as the caret's
  visible region, rounding fractional corrections outward.
- Apply the same local reveal on native input as on paste/manual newline.
  Changing only the manual reveal is insufficient: an intermediate experiment
  still moved the caret by 5px on native soft wraps.
- Leave active IME composition alone; reveal after composition commits.
- Pad a pasted trailing newline with the editor's existing zero-width-space
  convention, parking the caret before the pad. Serialization removes the pad,
  so the submitted text is unchanged.
- Never scroll ancestors, force the document end, change the live selection
  while measuring, or act on a range/outside-editor selection.

## Results

- WebKit and Chromium: viewport widths 1000px and 500px, each with zero/one/two
  trailing pasted newlines, followed by Shift+Enter and 180 typed characters.
  All 12 cases passed. The caret bottom relative to the editor stayed at
  **315.5px in WebKit / 315px in Chromium**, including repeated soft wraps.
  Editor height/top stayed fixed, scrollTop never moved backwards, and page
  scrollTop stayed zero.
- Middle-of-document paste + typing passed in both engines at both widths;
  later content remained below the viewport instead of jumping to the end.
- `MentionEditor.scroll.test.tsx`: 14 tests passed, covering padding/line-box
  geometry, fractional rounding, native input, trailing-newline serialization,
  composition handling, selection ownership, and the existing reveal cases.
- Desktop checks: `tsc --noEmit`, ESLint over `src/**/*.{ts,tsx}`, and the full
  Vitest suite passed (**110 files / 973 tests**).

Mac acceptance check: paste a long paragraph and a multiline document, continue
with Chinese IME and Latin text, alternate Shift+Enter with soft wrapping, and
edit in the middle. Check at normal and scaled display settings. The text should
follow the caret without extra vertical nudges; candidate selection and sending
must remain unchanged.
