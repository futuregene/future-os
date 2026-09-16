# Composer caret visibility after paste and newline

## Confirmed defect and scope

Baseline: `ae7c959b` (the relevant edit handlers are unchanged from the earlier
`c0c4d8e4` verification). Windows Chrome 153, actual desktop Composer,
MentionEditor and product CSS; Tauri IPC/send callbacks stubbed. No model calls
or access to production chat data. This does **not** establish macOS WKWebView
behavior or resolve the separate report of submitted input remaining visible.

Before the fix, native clipboard paste of 100 lines left the editor's viewport
at the top although the selection was at the end. Shift+Enter did not reveal
the next line either. Typing a character then jumped to the selection. The same
behavior was seen at 1280x800 and 720x800. `paste.isTrusted` was true; the
product handler prevented the native edit. A native textarea control followed
the selection immediately. This is a confirmed delayed caret-follow defect,
not proof that the original user's report of repeated macOS jitter has been
reproduced.

## Fix

After a manual paste/newline, measure the caret line and adjust **only the
editor's** scrollTop by the amount required to expose it. Do not scroll ancestor
containers, force the document bottom, insert measurement markers, or change
IME handling. Anchor a pasted selection inside its text node so its rectangle
is measurable. For a newline, also recognize the empty trailing text node that
Range.insertNode can produce, preserving the existing zero-width padding used
to render an empty last line (stripped during serialization).

## Verification

- Regression tests failed before the correction: pasted caret below/above the
  viewport and both Shift+Enter/Ctrl+Enter did not scroll.
- A separate regression test failed for a trailing newline followed by an empty
  split text node; it passes after the padding correction.
- Native Windows clipboard + Ctrl+V browser checks passed at both viewport
  widths. Original clipboard formats were backed up in memory and restored.
- With 100 lines / 9091 characters, editor height was 320px. Paste now scrolls
  immediately to approximately 1683.33px. Five successive newline operations
  move it to 1703.33, 1723.33, 1743.33, 1763.33 and 1783.33px. Subsequent typing
  stays at 1783.33px: **no delayed jump**.
- Shift+Enter and Ctrl+Enter, short typed input followed by newline, and native
  paste in the middle of existing long text all keep the caret visible.
- Middle insertion leaves later content below the viewport rather than forcing
  the editor to the bottom. Page scrollTop remains zero. Sending still clears
  both the editor and its saved draft.
- jsdom tests cover minimal upward/downward scrolling, an already-visible caret,
  live selection/ancestor preservation, empty geometry, and the IME guard.

Native macOS desktop and actual system IME candidate selection remain untested;
no claim is made that the separate macOS residual-input/jitter reports are fixed.
