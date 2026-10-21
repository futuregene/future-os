/**
 * What the shortcut recorder accepts, refuses, and how it says so.
 *
 * Kept out of the component so the rules can be tested without a DOM and so the
 * recorder file exports one thing (the React refresh boundary). Everything here
 * reads a `KeyboardEvent`-shaped object and the command table — nothing else.
 */
import type { TFunction } from "i18next";
import type { BindingProblem } from "../../lib/keybinding";
import type { CommandId, CommandSpec } from "../keyboard/catalog";
import { bindingFromEvent, bindingProblem, isModifierKey, serializeBinding } from "../../lib/keybinding";
import { conflictingCommand } from "../keyboard/shortcuts";

/** Why a press was refused, in the vocabulary the strings know. */
export type RecorderRefusal
  = | { problem: BindingProblem }
    | { problem: "conflict"; conflict?: CommandSpec }
    | { problem: "unsupported" };

/** The part of a key event the recorder reads; tests need no real event. */
export interface RecorderKeyEvent {
  key: string;
  code?: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  getModifierState?: (key: string) => boolean;
}

/**
 * Read a press into a stored binding, or say why it cannot be one.
 *
 * `null` means "keep waiting": a modifier on its own is half a combination.
 */
export function interpretPress(
  event: RecorderKeyEvent,
  command: CommandId,
  isMac: boolean,
  /**
   * Whether this command lives in a mode's keymap (`scopeOf(spec) !== "global"`).
   * A mode owns bare keys — its scope is only on when the mode is — so they are
   * recording-worthy there and refused everywhere else.
   */
  allowBare = false,
): { binding: string } | RecorderRefusal | null {
  if (isModifierKey(event))
    return null;
  // AltGr is how Windows and Linux type `{`, `[`, `@` — a command on
  // `Ctrl+Alt+<key>` would fire mid-word.
  if (event.getModifierState?.("AltGraph"))
    return { problem: "unsupported" };
  const binding = bindingFromEvent(event, isMac);
  if (!binding)
    return { problem: "unsupported" };
  const problem = bindingProblem(binding, isMac, { allowBare });
  if (problem)
    return { problem };
  const text = serializeBinding(binding);
  const conflict = conflictingCommand(text, command);
  if (conflict)
    return { problem: "conflict", conflict };
  return { binding: text };
}

/** The modifier glyphs currently held, in the platform's notation. */
export function heldModifiers(
  event: { ctrlKey: boolean; metaKey: boolean; altKey: boolean; shiftKey: boolean },
  isMac: boolean,
): string {
  if (isMac)
    return `${event.ctrlKey ? "⌃" : ""}${event.altKey ? "⌥" : ""}${event.shiftKey ? "⇧" : ""}${event.metaKey ? "⌘" : ""}`;
  const parts: string[] = [];
  if (event.ctrlKey)
    parts.push("Ctrl");
  if (event.altKey)
    parts.push("Alt");
  if (event.shiftKey)
    parts.push("Shift");
  return parts.length > 0 ? `${parts.join("+")}+` : "";
}

/** A refusal in the reader's language. */
export function refusalText(t: TFunction<"shortcuts">, refusal: RecorderRefusal, isMac: boolean): string {
  switch (refusal.problem) {
    case "needsModifier":
      return t(isMac ? "recorder.needsModifierMac" : "recorder.needsModifierOther");
    case "reserved":
      return t("recorder.reserved");
    case "conflict":
      return t("recorder.conflict", {
        command: refusal.conflict ? t(`commands.${refusal.conflict.id}`) : "",
      });
    case "unsupported":
      return t("recorder.unsupported");
  }
}
