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
 * 5. **A bare key belongs to a mode, and not while the caret is in a field.** The
 *    grid's arrows/`↩`/digits are only bound inside a scope; even there, a press
 *    aimed at a text field is the field's (the panel's zoomed cell has a
 *    composer).
 * 6. **Inside the terminal, only commands marked `inTerminal` fire.** `Ctrl` is
 *    the accelerator on Windows and Linux, and the terminal is a shell: `Ctrl+B`
 *    there is "back one character". A command the shell owns keeps the shell's
 *    meaning, whatever the binding says.
 * 7. **Holding a key repeats only what says it may.** Walking a list wants the
 *    repeat; toggling a panel does not.
 *
 * **Scopes are resolved before all of this**: the innermost active scope is tried
 * first, so a mode's `←` wins over nothing and a mode's `⌘[` wins over the global
 * one — and the global meaning returns the moment the mode closes. Two commands
 * that share a key in different scopes are not a conflict; two in one scope are,
 * and the recorder refuses the second.
 */
import type { CommandId, ShortcutScope } from "./catalog";
import { matchesBinding } from "../../lib/keybinding";
import { COMMANDS, commandSpec, scopeOf } from "./catalog";
import { activeScopes } from "./scopes";

/** The part of a key event the dispatcher reads; tests need no real event. */
export interface ShortcutEvent extends BindingKeyEvent {
  repeat: boolean;
  isComposing?: boolean;
  keyCode?: number;
  defaultPrevented?: boolean;
  getModifierState?: (key: string) => boolean;
}

/** A key press the dispatcher accepted. */
export interface DispatchHit {
  id: CommandId;
  /** The binding that matched — the digit, for a nine-key command. */
  binding: KeyBinding;
}

/** What the dispatcher needs to know about the app around it. */
export interface DispatchContext {
  isMac: boolean;
  /** Every command's live keys (see `shortcuts.ts:resolvedBindings`). */
  bindings: ReadonlyMap<CommandId, readonly KeyBinding[]>;
  /** Whether the command currently has a handler (see `commands.ts`). */
  hasHandler: (id: CommandId) => boolean;
  /** A modal is open (`components/ui/overlayStack`). */
  hasOpenOverlay: () => boolean;
  /** A modal *or* a floating menu holds the keyboard (`lib/keyboardOwners`). */
  hasKeyboardOwner: () => boolean;
  /** The event belongs to the embedded terminal. */
  inTerminal: boolean;
  /** The press is aimed at a text field (`lib/editableTarget`). */
  editable?: boolean;
  /** The active scopes, innermost first. Defaults to the live registry. */
  scopes?: readonly ShortcutScope[];
}

/** Chromium reports "this key belongs to the IME" as keyCode 229. */
const IME_KEY_CODE = 229;

/**
 * Whether a binding has no real modifier — the mode-only keys.
 *
 * `Shift` alone does not count: it is how everyone types a capital letter, and
 * `Shift+Enter` is a newline in the composer, not a chord. A key with only Shift
 * is as much "ordinary typing" as a bare one.
 */
export function isBareBinding(binding: KeyBinding): boolean {
  return !binding.mod && !binding.ctrl && !binding.alt;
}

/**
 * The command this event should run, or `null` when it belongs to someone else.
 *
 * Returning `null` means "leave the event alone": the caller must not call
 * `preventDefault`.
 */
export function dispatchShortcut(event: ShortcutEvent, context: DispatchContext): DispatchHit | null {
  if (event.defaultPrevented)
    return null;
  if (event.isComposing || event.keyCode === IME_KEY_CODE || event.key === "Unidentified")
    return null;
  if (event.getModifierState?.("AltGraph"))
    return null;

  const hit = commandForEvent(event, context);
  if (!hit)
    return null;
  const spec = commandSpec(hit.id);
  if (!spec || !context.hasHandler(hit.id))
    return null;
  if (event.repeat && !spec.repeat)
    return null;
  if (context.hasOpenOverlay() || context.hasKeyboardOwner())
    return null;
  // A modifier-less key is only ever a mode's key, and even inside the mode it
  // belongs to a text field that has the caret.
  if (isBareBinding(hit.binding) && (scopeOf(spec) === "global" || context.editable))
    return null;
  if (context.inTerminal && !spec.inTerminal)
    return null;
  return hit;
}

/**
 * The first command whose binding matches, resolved scope by scope.
 *
 * Innermost scope first: a mode's key shadows the global one for as long as the
 * mode is on. Within one scope the table order decides, which only matters for a
 * table that broke its own uniqueness (a test pins that it does not).
 */
function commandForEvent(event: ShortcutEvent, context: DispatchContext): DispatchHit | null {
  const scopes = context.scopes ?? activeScopes();
  for (const scope of scopes) {
    for (const spec of COMMANDS) {
      if (scopeOf(spec) !== scope)
        continue;
      for (const binding of context.bindings.get(spec.id) ?? []) {
        if (matchesBinding(binding, event, context.isMac))
          return { binding, id: spec.id };
      }
    }
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
