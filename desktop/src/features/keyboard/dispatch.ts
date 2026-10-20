import type { BindingKeyEvent, KeyBinding } from "../../lib/keybinding";
/**
 * The one place a key press is turned into a command.
 *
 * Every global shortcut goes through `dispatchShortcut`, which answers exactly
 * one question: *should this event run a command, and which one?* It never
 * executes anything itself, so the rules below are testable without React and the
 * caller keeps the choice of what "running" means.
 *
 * The rules, in order, and why each exists:
 *
 * 1. **An IME owns the keyboard while it composes.** `isComposing`, Chromium's
 *    `229`, and WebKitGTK's `"Unidentified"` all mean "this key is mid-word".
 *    Acting on it would fire a command while the user is typing Chinese.
 * 2. **AltGr is typing, not a shortcut.** On Windows and Linux `AltGr` arrives as
 *    `Ctrl+Alt`, so a binding on `Ctrl+Alt+…` would fire whenever the user types
 *    `{`, `[` or `@` on a layout where those need a third level.
 * 3. **A binding nobody registered is not handled.** No conversation open means
 *    no `Mod+F`; swallowing it for nothing is how a user loses a keystroke.
 * 4. **A modal or a floating menu holds the keyboard.** `overlayStack` decides
 *    what Escape closes, `keyboardOwners` decides who is using the arrows; both
 *    mean the app must not act behind them (see `lib/keyboardOwners` for why this
 *    is by presence and not focus).
 * 5. **Inside the terminal, only commands marked `inTerminal` fire.** `Ctrl` is
 *    the accelerator on Windows and Linux, and the terminal is a shell: `Ctrl+B`
 *    there is "back one character". A command the shell owns keeps the shell's
 *    meaning, whatever the binding says.
 * 6. **Holding a key repeats only what says it may.** Walking a list wants the
 *    repeat; toggling a panel does not.
 *
 * Order matters: the terminal rule is checked last so a command that is blocked
 * there reports "not mine" without any other rule having to know about terminals.
 */
import type { CommandId } from "./catalog";
import { matchesBinding } from "../../lib/keybinding";
import { COMMANDS, commandSpec } from "./catalog";

/** The part of a key event the dispatcher reads; tests need no real event. */
export interface ShortcutEvent extends BindingKeyEvent {
  repeat: boolean;
  isComposing?: boolean;
  keyCode?: number;
  defaultPrevented?: boolean;
  getModifierState?: (key: string) => boolean;
}

/** What the dispatcher needs to know about the app around it. */
export interface DispatchContext {
  isMac: boolean;
  /** Every command's live binding (see `shortcuts.ts:resolvedBindings`). */
  bindings: ReadonlyMap<CommandId, KeyBinding | null>;
  /** Whether the command currently has a handler (see `commands.ts`). */
  hasHandler: (id: CommandId) => boolean;
  /** A modal is open (`components/ui/overlayStack`). */
  hasOpenOverlay: () => boolean;
  /** A modal *or* a floating menu holds the keyboard (`lib/keyboardOwners`). */
  hasKeyboardOwner: () => boolean;
  /** The event belongs to the embedded terminal. */
  inTerminal: boolean;
}

/** Chromium reports "this key belongs to the IME" as keyCode 229. */
const IME_KEY_CODE = 229;

/**
 * The command this event should run, or `null` when it belongs to someone else.
 *
 * Returning `null` means "leave the event alone": the caller must not call
 * `preventDefault`.
 */
export function dispatchShortcut(event: ShortcutEvent, context: DispatchContext): CommandId | null {
  if (event.defaultPrevented)
    return null;
  if (event.isComposing || event.keyCode === IME_KEY_CODE || event.key === "Unidentified")
    return null;
  if (event.getModifierState?.("AltGraph"))
    return null;

  const id = commandForEvent(event, context);
  if (!id)
    return null;
  const spec = commandSpec(id);
  if (!spec || !context.hasHandler(id))
    return null;
  if (event.repeat && !spec.repeat)
    return null;
  if (context.hasOpenOverlay() || context.hasKeyboardOwner())
    return null;
  if (context.inTerminal && !spec.inTerminal)
    return null;
  return id;
}

/** The first command whose binding matches — the pure half of the rules above. */
function commandForEvent(event: ShortcutEvent, context: DispatchContext): CommandId | null {
  for (const spec of COMMANDS) {
    const binding = context.bindings.get(spec.id);
    if (binding && matchesBinding(binding, event, context.isMac))
      return spec.id;
  }
  return null;
}

/**
 * Whether an element belongs to the embedded terminal.
 *
 * The panel and the xterm surface are both marked: a press on the panel's tab
 * strip is still "in the terminal" as far as the shell is concerned, and the
 * surface is what actually holds focus when the user types.
 */
export const TERMINAL_SURFACE_SELECTOR = "[data-component=\"terminal\"], [data-component=\"terminal-panel\"]";
