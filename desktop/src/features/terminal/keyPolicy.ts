/**
 * Which keys the terminal is allowed to interpret itself.
 *
 * Two owners take precedence over xterm:
 *
 * 1. the app's panel shortcut (`Ctrl+J` / `⌘J`), which `useTerminalPanel` also
 *    listens for — the view releases it so the event can bubble to that
 *    listener;
 * 2. the input method. While an IME is composing, every keystroke belongs to the
 *    IME, so the terminal must stand down instead of treating the key as input.
 *
 * xterm's own composition helper only recognises Chromium's convention for
 * "this key belongs to the IME" — `keyCode === 229` (see
 * `browser/input/CompositionHelper.ts`). WebKit, the engine the desktop webview
 * uses on Linux, reports the keys an IME consumes with their *real* keyCodes: a
 * composing `Backspace` arrives as `keyCode 0` (measured through WebKitWebDriver
 * with ibus-libpinyin). xterm therefore saw it as ordinary terminal input, and
 * before processing any such key it runs `_finalizeComposition(false)`, which
 * commits whatever the textarea holds at that instant.
 *
 * The result, reproduced against the real engine, a real IME and a real shell:
 * typing pinyin, pressing Backspace to fix a letter and then committing sent the
 * Chinese text to the shell **twice** — once on the Backspace, once on the
 * commit — so the command line kept a leftover copy that the user's deletion
 * could not clear.
 */
import type { ShortcutKey } from "./shortcut";
import { isMacOS } from "../../lib/platform";
import { isPanelToggleShortcut } from "./shortcut";

/** The part of a KeyboardEvent this policy reads. */
export interface KeyPolicyEvent extends ShortcutKey {
  /** True while the event is part of an IME composition. */
  isComposing: boolean;
}

/**
 * True when xterm should handle the key; false when the terminal must stand
 * down and let the key reach its real owner (the app or the IME).
 *
 * Standing down does not swallow the key: xterm returns without calling
 * `preventDefault`, so the browser still delivers it to the IME, which is
 * exactly what a composing keystroke needs.
 *
 * The app's own command is released first, so a wedged composition cannot make
 * the panel un-collapsible. Measured caveat: on WebKitGTK + ibus the composing
 * `j` of `Ctrl+J` arrives as `key: "Unidentified"`, i.e. the IME consumes it and
 * cancels the preedit before it can be recognised — the ordering only decides
 * engines that report the key normally.
 */
export function terminalKeyPolicy(event: KeyPolicyEvent, isMac: boolean = isMacOS): boolean {
  return !isPanelToggleShortcut(event, isMac) && !event.isComposing;
}
