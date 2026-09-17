import type { Terminal } from "@xterm/xterm";

/**
 * Work around xterm 6.0's stale keydown flag in macOS WebKit.
 *
 * With an IME input source, WebKit can deliver beforeinput/input BEFORE the
 * character's keydown. If another key is still held, xterm's `_keyDownSeen`
 * suppresses that input as a duplicate. The subsequent keydown (often 229) does
 * not send it either. This loses overlapping keystrokes and shifted characters.
 * https://github.com/xtermjs/xterm.js/issues/5374
 *
 * Reset only that guard before committed text, leaving xterm responsible for
 * sending it through onData. In particular, keep its keypress deduplication and
 * composition handling intact; never guess missing input using a timer.
 *
 * This is intentionally isolated private-API compatibility code for our pinned
 * xterm version. The real-xterm regression tests must pass on dependency updates.
 */
export function installMacInputWorkaround(terminal: Terminal): () => void {
  const ua = navigator.userAgent;
  if (!/Macintosh|Mac OS X/i.test(ua) || !/AppleWebKit/i.test(ua) || /Chrome|Chromium|Edg\//i.test(ua)) {
    return () => {};
  }
  const textarea = terminal.textarea;
  if (!textarea) {
    return () => {};
  }
  const core = (terminal as unknown as { _core?: { _keyDownSeen?: boolean } })._core;
  const beforeInput = (event: InputEvent) => {
    if (
      event.composed
      && event.data
      && event.inputType === "insertText"
      && !event.isComposing
      && !terminal.options.screenReaderMode
      && typeof core?._keyDownSeen === "boolean"
    ) {
      core._keyDownSeen = false;
    }
  };
  textarea.addEventListener("beforeinput", beforeInput);
  return () => textarea.removeEventListener("beforeinput", beforeInput);
}
