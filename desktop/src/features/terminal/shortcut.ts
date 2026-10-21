/**
 * The show/hide shortcut for the terminal panel, defined once for its three
 * consumers.
 *
 * The **binding** is the command table's, not this module's: `terminal.toggle` is
 * an ordinary rebindable command (Settings → Keyboard shortcuts), stored like
 * every other one. What lives here is the part that is the terminal's own — how
 * the key reaches the panel at all from inside the shell.
 *
 * `useTerminalPanel` listens on the window, so it sees the key everywhere except
 * inside the terminal: xterm cancels (`preventDefault` + `stopPropagation`) every
 * key it handles, and `Ctrl+J` is one of them (it maps to a line feed). The view
 * therefore asks this predicate first and hands the event back to the DOM,
 * exactly like opencode's terminal component does for its toggle keybind.
 */
import type { KeyBinding } from "../../lib/keybinding";
import { formatBinding, matchesBinding } from "../../lib/keybinding";
import { isMacOS } from "../../lib/platform";
import { resolvedBinding } from "../keyboard/shortcuts";

/** The part of a KeyboardEvent this predicate reads; tests need no real event. */
export interface ShortcutKey {
  key: string;
  /** The physical key; see `lib/keybinding` for why this is what we match on. */
  code?: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

/** The panel's live binding — the default is `Ctrl+J` / `⌘J`, and it is rebindable. */
export function panelToggleBinding(): KeyBinding | null {
  return resolvedBinding("terminal.toggle");
}

/**
 * Whether this key press asks for the panel.
 *
 * Read at event time rather than captured once, so a rebinding takes effect on
 * the next press — including while a terminal already has focus.
 */
export function isPanelToggleShortcut(event: ShortcutKey, isMac: boolean = isMacOS): boolean {
  const binding = panelToggleBinding();
  return binding !== null && matchesBinding(binding, event, isMac);
}

/**
 * Shortcut label for tooltips, in the platform's own notation.
 *
 * `""` when the user unbound the command: a tooltip saying "Show terminal ()" is
 * worse than one that says nothing.
 */
export function panelToggleShortcutLabel(isMac: boolean = isMacOS): string {
  const binding = panelToggleBinding();
  return binding ? formatBinding(binding, isMac) : "";
}
