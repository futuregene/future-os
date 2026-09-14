/**
 * The show/hide shortcut for the terminal panel, defined once for its two
 * consumers.
 *
 * `useTerminalPanel` listens on the window, so it sees the key everywhere except
 * inside the terminal: xterm cancels (`preventDefault` + `stopPropagation`)
 * every key it handles, and `Ctrl+J` is one of them (it maps to a line feed). The
 * view therefore asks this predicate first and hands the event back to the DOM,
 * exactly like opencode's terminal component does for its toggle keybind.
 */
import { isMacOS } from "../../lib/platform";

/** The part of a KeyboardEvent this predicate reads; tests need no real event. */
export interface ShortcutKey {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

/**
 * `Ctrl+J` on Windows/Linux, `Cmd+J` on macOS, with no other modifier.
 *
 * The combination is reserved by the app: it never reaches the shell (Enter
 * submits a command line). Alt/Shift variants are ordinary terminal input, and
 * every other key belongs to the terminal.
 */
export function isPanelToggleShortcut(event: ShortcutKey, isMac: boolean = isMacOS): boolean {
  if (event.key.toLowerCase() !== "j")
    return false;
  if (event.altKey || event.shiftKey)
    return false;
  return isMac ? event.metaKey : event.ctrlKey;
}

/** Shortcut label for tooltips, in the platform's own notation. */
export function panelToggleShortcutLabel(isMac: boolean = isMacOS): string {
  return isMac ? "⌘J" : "Ctrl+J";
}
