# Composer keeps the sent line painted after a send (2026-09-24)

## Scope and reproduction

Reported on the desktop app (macOS 26.6.2, Apple M3 Ultra, Tauri/WKWebView):
after sending a message, the input box kept the last line of the message that
had just been sent, with the empty-state hint drawn above it. The reporter
attached a screenshot of the composer.

The prompt was two lines (Shift+Enter between them) containing a `/skill`
mention; `agent.db` shows the agent received it at 16:38:23.489 (`run-20260924-163823-b18a33`)
and the screenshot was taken ~7 s later, so the composer had had several seconds
to settle before the capture.

Not reproduced in a browser engine: replaying the same sequence against the
shipped frontend in the headless screenshot harness (real Chrome, real key
events) leaves the composer correctly repainted, and the clear path itself is
covered by `Composer.delivery.test.tsx`. The defect is in frame presentation,
not in the app's state.

## What the screenshot shows

Measured from the reporter's PNG (1804×762, 2× window):

- The composer card spans y 533–742; inside it the editor's box is y 550–670.
- Exactly two text rows are painted in that box: the hint at y 562–593 (ink
  `#8a94a6` = `ink-muted`, plus 88 px of ink `#172033` for the caret before the
  first character) and the residual line at y 605–632 (ink `#172033`, plus
  615 px of accent blue `#2563eb` — the pill).
- The residual line's glyph box (x 177–1275) is the same width as that line
  inside the sent bubble above (x 545–1643, 1098 px): the same text, same font,
  at its old position in the editor.
- The editor's DOM was empty at that moment. The hint is rendered only while
  the editor is empty and it is the app's own mark of that state
  (`{empty ? …}` over the `empty` state a MutationObserver keeps in sync), and
  the caret sat at the start of the first line — where a cleared editor's caret
  lands — not at the end of the residual text.
- The previous first line is gone, and its whole extent lies inside the hint's
  mounted box (the hint's text is 336.2 px wide at 1×, the old line ~276 px):
  the hint's mount repainted that rect, while the clear's own repaint never
  reached the editor — the residual second line sits below that rect and
  survived.

## Diagnosis

WKWebView on macOS 26 presents partially updated frames — regions of the new
frame next to regions of the old one, with the DOM and hit-testing live
(tauri-apps/wry#1848). That is how the DOM can be correct while the pixels are
not. The same family is already recorded in the desktop source for the message
hover controls (`MessageBlock.tsx`, the `will-change` note).

A fix therefore cannot rely on the editor's own invalidation being painted; it
has to dirty the whole box from something that always accompanies the reset.

## Correction

- The empty-state hint now covers the editor's whole box (`inset-0` plus the
  editor's own `px-2 py-1`) instead of only the text's own width
  (`left-2 top-1`). Mounting it — which always happens when a clear empties the
  editor — dirties every pixel of the box, and the repaint that follows wipes
  whatever the clear left painted; unmounting it does the same when a draft is
  restored.
- Since the hint now overlays the editor, it has to stay `pointer-events-none`,
  and it keeps the editor's padding so the text does not move.

## Validation (headless harness, not native WKWebView)

- The hint's box equals the editor's box (409,743, 750×56) and its text line is
  unchanged (417,748, 336.2×17).
- A 2× capture of the whole composer card is pixel-identical with the previous
  build: 0 of 2,291,520 pixels differ.
- Hit-testing still reaches the editor through the hint (`elementFromPoint`
  returns `role=textbox`; a click in the lower half of the box focuses it).

## Not validated

- No native WKWebView reproduction: the mechanism rests on the presentation
  defect being region-local and on the hint's mount dirtying the editor's box —
  which the reporter's screenshot shows working for the smaller rect, not for
  the whole box.
- The hint only churns while the editor crosses empty (a clear-in, a
  draft-restore-out). For a reset that replaces text with text, the editor's own
  invalidation is still the only thing asking for the repaint; the next lever
  there is a compositor promotion of the editor, which must be paired with
  dirtying the editor's own layer (a promoted editor no longer paints into the
  hint's layer — see the comment in `MentionEditor.tsx`).
