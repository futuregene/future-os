/**
 * The app's commands, as one table.
 *
 * A command is what the user means ("start a new conversation"), a *binding* is
 * which keys currently mean it, and this table is the single place that says
 * both — which is what keeps the dispatcher, the settings page and the tooltips
 * from drifting apart. A component registers the *handler*; the metadata lives
 * here.
 *
 * Two rules for this file:
 *
 * - **Only `mod`-style bindings by default.** `mod` is `⌘` on macOS and `Ctrl`
 *   elsewhere (see `lib/keybinding`), so one default serves all three platforms.
 * - **`inTerminal` is opt-in and rare.** `Ctrl` is the accelerator on Windows and
 *   Linux, and the terminal is a shell: `Ctrl+B` is "back one character" there.
 *   A command that is not marked runs nowhere inside the terminal, whatever its
 *   binding says, so the shell keeps its line editing.
 */
import type { KeyBinding } from "../../lib/keybinding";
import { formatBinding, parseBinding } from "../../lib/keybinding";

/** Groups the settings page lists commands under. */
export type CommandCategory = "chat" | "navigation" | "panels" | "terminal" | "settings";

export type CommandId
  = | "chat.new"
    | "chat.stop"
    | "composer.focus"
    | "thread.find"
    | "group.previous"
    | "group.next"
    | "thread.previous"
    | "thread.next"
    | "rail.search"
    | "panel.left"
    | "panel.right"
    | "section.chat"
    | "section.workspace"
    | "section.skill"
    | "section.tasks"
    | "section.remote"
    | "settings.open"
    | "terminal.toggle";

export interface CommandSpec {
  id: CommandId;
  category: CommandCategory;
  /**
   * The portable default. `mod` is the platform accelerator; a user's rebinding
   * is stored as text in the same notation (see `shortcuts.ts`).
   */
  defaultBinding: string;
  /**
   * Whether the command may fire while the terminal has the keyboard. Off for
   * everything but the terminal's own toggle: inside a shell, `Ctrl+B` and
   * friends belong to the shell.
   */
  inTerminal: boolean;
  /** Whether holding the key repeats the command (walking a list). */
  repeat?: boolean;
}

/**
 * Every command, in the order the settings page lists them.
 *
 * The ids are stable and used as storage keys and i18n keys: a label is
 * `shortcuts:commands.<id>`, so adding a command means adding one entry here and
 * one string per language (a test enforces that).
 */
export const COMMANDS: readonly CommandSpec[] = [
  { id: "chat.new", category: "chat", defaultBinding: "mod+n", inTerminal: false },
  { id: "chat.stop", category: "chat", defaultBinding: "mod+period", inTerminal: false },
  { id: "composer.focus", category: "chat", defaultBinding: "mod+l", inTerminal: false },
  { id: "thread.find", category: "chat", defaultBinding: "mod+f", inTerminal: false },
  // Two levels, because the list is two levels: a workspace holds conversations,
  // a conversation holds its forks. `thread.*` walks the conversation rows;
  // `group.*` steps between the top-level items (each workspace, each standalone
  // conversation).
  //
  // **One key family, one modifier each, ⇧ for the bigger step.** `mod+[` /
  // `mod+]` is the pair every browser already uses to go back and forward, so it
  // is learned rather than memorised, and the coarse move is the same keys held
  // one notch harder. An earlier version put the fine move on `mod+alt+↑/↓` to
  // keep clear of text-editing chords; the keys were accurate and the reach was
  // wrong — three fingers for the move people make most often.
  { id: "thread.previous", category: "navigation", defaultBinding: "mod+bracketleft", inTerminal: false, repeat: true },
  { id: "thread.next", category: "navigation", defaultBinding: "mod+bracketright", inTerminal: false, repeat: true },
  { id: "group.previous", category: "navigation", defaultBinding: "mod+shift+bracketleft", inTerminal: false, repeat: true },
  { id: "group.next", category: "navigation", defaultBinding: "mod+shift+bracketright", inTerminal: false, repeat: true },
  { id: "rail.search", category: "navigation", defaultBinding: "mod+shift+f", inTerminal: false },
  { id: "section.chat", category: "navigation", defaultBinding: "mod+1", inTerminal: false },
  { id: "section.workspace", category: "navigation", defaultBinding: "mod+2", inTerminal: false },
  { id: "section.skill", category: "navigation", defaultBinding: "mod+3", inTerminal: false },
  { id: "section.tasks", category: "navigation", defaultBinding: "mod+4", inTerminal: false },
  { id: "section.remote", category: "navigation", defaultBinding: "mod+5", inTerminal: false },
  { id: "panel.left", category: "panels", defaultBinding: "mod+b", inTerminal: false },
  { id: "panel.right", category: "panels", defaultBinding: "mod+shift+b", inTerminal: false },
  { id: "terminal.toggle", category: "terminal", defaultBinding: "mod+j", inTerminal: true },
  { id: "settings.open", category: "settings", defaultBinding: "mod+comma", inTerminal: false },
];

/** Settings page section order. */
export const CATEGORIES: readonly CommandCategory[] = ["chat", "navigation", "panels", "terminal", "settings"];

const BY_ID = new Map<string, CommandSpec>(COMMANDS.map(spec => [spec.id, spec]));

/** The table entry for an id, or `undefined` for a value this version dropped. */
export function commandSpec(id: string): CommandSpec | undefined {
  return BY_ID.get(id);
}

/** True when `value` is a command this version knows. */
export function isCommandId(value: string): value is CommandId {
  return BY_ID.has(value);
}

/** A command's default binding, parsed. */
export function defaultBinding(spec: CommandSpec): KeyBinding {
  const binding = parseBinding(spec.defaultBinding);
  // The defaults are literals in this file and a test parses every one of them;
  // a null here means the table itself was broken by an edit.
  if (!binding)
    throw new Error(`Invalid default binding for ${spec.id}: ${spec.defaultBinding}`);
  return binding;
}

/** A command's default binding, in the platform's notation. */
export function defaultBindingLabel(spec: CommandSpec, isMac: boolean): string {
  return formatBinding(defaultBinding(spec), isMac);
}

/**
 * Keys that keep their meaning everywhere and are deliberately **not**
 * assignable: they are what a text field, a dialog and the transcript already
 * answer, and routing them through the command table would take them away from
 * the thing that needs them.
 *
 * Listed in the settings page so a reader sees the whole keyboard, not just the
 * part they can change.
 */
export const FIXED_KEYS: ReadonlyArray<{ labelKey: string; keys: { mac: string; other: string } }> = [
  { labelKey: "fixed.send", keys: { mac: "↩", other: "Enter" } },
  { labelKey: "fixed.newline", keys: { mac: "⇧↩", other: "Shift+Enter" } },
  { labelKey: "fixed.escape", keys: { mac: "Esc", other: "Esc" } },
  { labelKey: "fixed.approve", keys: { mac: "⌘↩", other: "Ctrl+Enter" } },
  { labelKey: "fixed.transcript", keys: { mac: "⇞ / ⇟", other: "Page Up / Page Down" } },
  { labelKey: "fixed.menu", keys: { mac: "↑ ↓ ↩", other: "Up Down Enter" } },
];
