import type { KeyBinding } from "../../lib/keybinding";
/**
 * The app's commands, as one table.
 *
 * A command is what the user means ("start a new conversation"), a *binding* is
 * which keys currently mean it, and this table is the single place that says
 * both — which is what keeps the dispatcher, the settings page and the tooltips
 * from drifting apart. A component registers the *handler*; the metadata lives
 * here.
 *
 * Three rules for this file:
 *
 * - **Only `mod`-style bindings in the global scope.** `mod` is `⌘` on macOS and
 *   `Ctrl` elsewhere (see `lib/keybinding`), so one default serves all three
 *   platforms. A bare key (an arrow, `↩`, a digit) is only ever assigned inside a
 *   scope*, where it means what the mode says and where the text field it would
 *   otherwise be typing into is not on screen.
 * - **`inTerminal` is opt-in and rare.** `Ctrl` is the accelerator on Windows and
 *   Linux, and the terminal is a shell: `Ctrl+B` is "back one character" there.
 *   A command that is not marked runs nowhere inside the terminal, whatever its
 *   binding says, so the shell keeps its line editing.
 * - **A scope is a keymap that exists only while its mode is on** (`scopes.ts`).
 *   Inside it a key may sit on one the global keymap already uses — that is the
 *   point of the mode — and the global meaning comes back when the mode closes.
 */
import type { ShortcutScope } from "./scopes";
import { formatBinding, parseBinding } from "../../lib/keybinding";

/** Groups the settings page lists commands under. */
export type CommandCategory = "chat" | "navigation" | "panels" | "terminal" | "vibe" | "palette" | "settings";

export type { ShortcutScope };

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
    | "terminal.toggle"
    // Search and help: the two entries that exist to *find* the others.
    | "palette.open"
    | "shortcuts.help"
    // The full-screen panel of pinned conversations ("Vibe"). Scoped: in it the
    // grid answers the bare keys, and a key it binds may shadow a global one.
    | "vibe.toggle"
    | "vibe.focus.up"
    | "vibe.focus.down"
    | "vibe.focus.left"
    | "vibe.focus.right"
    | "vibe.open"
    | "vibe.close"
    | "vibe.back"
    | "vibe.zoom.previous"
    | "vibe.zoom.next"
    | "vibe.cell"
    | "vibe.columns.cycle"
    | "vibe.columns.more"
    | "vibe.columns.fewer"
    | "vibe.columns.auto"
    | "vibe.move.left"
    | "vibe.move.right"
    | "vibe.panel.hide"
    | "vibe.panel.pin"
    | "vibe.device"
    | "vibe.attention"
    | "vibe.add"
    | "vibe.remove"
    | "vibe.fullscreen"
    | "vibe.help"
    // The input layer: the one place the caret is in a text field, so these are
    // the commands that fire *over* an editor, and the ones a microphone-driven
    // conversation is actually operated with.
    | "vibe.input.open"
    | "vibe.input.close"
    // Taking a held sentence back into the box: a change of mind about one message,
    // not about the layer (which is why it is not on `Esc`).
    | "vibe.queue.cancel"
    | "vibe.block.previous"
    | "vibe.block.next"
    | "vibe.menu"
    | "vibe.home";

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
  /**
   * The keymap this command belongs to. `global` (the default) is always on.
   *
   * A scoped command is matched only while its scope is active, and is matched
   * **before** the global keymap — so inside the mode it may take a key the app
   * uses outside it.
   */
  scope?: ShortcutScope;
  /**
   * The command answers to several keys, and `defaultBinding` is the first.
   *
   * Only the direct-selection commands need this (`1`–`9` land on a cell), and
   * they are not editable in v1: recording one key for a nine-key command would
   * mean recording nine. The settings page shows them as a read-only key list.
   */
  bindings?: readonly string[];
  /**
   * Keys the command **always** answers to, on top of whatever it resolves to.
   *
   * These are the device's keys, not the user's: a remote control's back / menu /
   * home buttons must keep working after the user rebinds the keyboard chord, and
   * cannot be handed to another command. So they survive an override and survive
   * a cleared binding, they are not recorded, and a conflict with them is still
   * refused (`conflictingCommand` sees them).
   */
  aliases?: readonly string[];
  /**
   * Whether the command runs with the caret inside a text field.
   *
   * Off for everything but the input layer. The dispatcher leaves a bare key to
   * the field that has the caret (`isBareBinding`), and that rule is what keeps
   * `↩` meaning "send" inside a composer — the input layer is the one place where
   * the panel's own key is the one that sends, so it says so here instead of the
   * dispatcher guessing. A composing key never reaches a command at all, so a
   * dictation's `↩` still commits the input method's candidate.
   */
  overEditable?: boolean;
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
  { id: "palette.open", category: "palette", defaultBinding: "mod+k", inTerminal: false },
  { id: "shortcuts.help", category: "palette", defaultBinding: "mod+slash", inTerminal: false },
  { id: "settings.open", category: "settings", defaultBinding: "mod+comma", inTerminal: false },

  // ---- Vibe panel (full-screen grid of pinned conversations) ----------------
  //
  // `vibe` is held while the panel is on, `vibeGrid` while the grid has the
  // keyboard (no cell zoomed), `vibeZoom` while one cell is filling the screen.
  // The three layers are the design's own: a direction key must have exactly one
  // owner, and the panel's owner is whichever layer is on top.
  //
  // The panel reuses `thread.previous` / `thread.next` for "previous/next cell"
  // rather than adding a second pair: inside a mode a command means what the
  // mode's owner makes it mean, which is what the registry already does — the
  // rail is not mounted to own them there.
  { id: "vibe.toggle", category: "vibe", defaultBinding: "mod+shift+v", inTerminal: false },
  // Each direction answers to its arrow **and** to `h`/`j`/`k`/`l`. Two bindings on one
  // command rather than four more commands: the movement is the same movement, and the
  // table already expresses "one command, several keys" (`vibe.cell`). The letters are
  // fixed (the settings page lists them read-only) because a rebindable pair would let a
  // reader shadow them without noticing what they shadowed.
  { id: "vibe.focus.up", category: "vibe", defaultBinding: "arrowup", bindings: ["arrowup", "k"], inTerminal: false, repeat: true, scope: "vibeGrid" },
  { id: "vibe.focus.down", category: "vibe", defaultBinding: "arrowdown", bindings: ["arrowdown", "j"], inTerminal: false, repeat: true, scope: "vibeGrid" },
  { id: "vibe.focus.left", category: "vibe", defaultBinding: "arrowleft", bindings: ["arrowleft", "h"], inTerminal: false, repeat: true, scope: "vibeGrid" },
  { id: "vibe.focus.right", category: "vibe", defaultBinding: "arrowright", bindings: ["arrowright", "l"], inTerminal: false, repeat: true, scope: "vibeGrid" },
  { id: "vibe.open", category: "vibe", defaultBinding: "enter", inTerminal: false, scope: "vibeGrid" },
  // "Back" leaves the panel from the grid: the same button that steps out of one
  // layer steps out of the last one.
  { id: "vibe.close", category: "vibe", defaultBinding: "escape", aliases: ["browserback"], inTerminal: false, scope: "vibeGrid" },
  { id: "vibe.back", category: "vibe", defaultBinding: "escape", aliases: ["browserback"], inTerminal: false, scope: "vibeZoom" },
  // Inside a zoomed cell the bare arrows switch conversations — the grid's own
  // arrows are not live there, and the caret rule keeps them off a text field.
  // Distinct commands from the grid's because a command has one binding set: the
  // grid moves a selection ring, this changes what the cell shows.
  // `h`/`l` alongside the arrows here too: a reader whose hands are on home row should not
  // have to move them to change conversation, and these two are the same pair of moves as
  // the grid's left/right.
  { id: "vibe.zoom.previous", category: "vibe", defaultBinding: "arrowleft", bindings: ["arrowleft", "h"], inTerminal: false, repeat: true, scope: "vibeZoom" },
  { id: "vibe.zoom.next", category: "vibe", defaultBinding: "arrowright", bindings: ["arrowright", "l"], inTerminal: false, repeat: true, scope: "vibeZoom" },
  // Up/down inside a zoomed cell walk the transcript **by block**, not by pixel:
  // a remote control cannot scroll, so the reading layer moves a block cursor over
  // messages and expands the one it is on. The wheel and the trackpad still scroll
  // normally — this pair is what makes the same view readable from the sofa.
  { id: "vibe.block.previous", category: "vibe", defaultBinding: "arrowup", inTerminal: false, repeat: true, scope: "vibeZoom" },
  { id: "vibe.block.next", category: "vibe", defaultBinding: "arrowdown", inTerminal: false, repeat: true, scope: "vibeZoom" },
  // ---- The input layer -----------------------------------------------------
  //
  // One editable element exists at a time in the panel, and this layer is the
  // only one that runs over it. `Esc` backs out to the reading layer.
  //
  // **There is no send command here, deliberately.** `↩` already sends in the
  // composer (`MentionEditor.onSubmit`, a fixed key — see `FIXED_KEYS`), and it
  // already stands down while an input method composes. Routing the same key
  // through the command table would put the two on one keystroke: the dispatcher
  // listens in the capture phase, so its handler would run first and the editor's
  // submit second, sending from two owners on one press. The panel's send key is
  // therefore the composer's own, and an empty draft closes the layer from there
  // (`VIBE.md` §5.2).
  { id: "vibe.input.open", category: "vibe", defaultBinding: "enter", inTerminal: false, scope: "vibeZoom" },
  // `⌘⇧X`: "take that back". A chord rather than a bare key, because the caret is in
  // the editor when a sentence is queued (`VIBE.md` §5.4).
  { id: "vibe.queue.cancel", category: "vibe", defaultBinding: "mod+shift+x", inTerminal: false, scope: "vibeInput" },
  { id: "vibe.input.close", category: "vibe", defaultBinding: "escape", aliases: ["browserback"], inTerminal: false, overEditable: true, scope: "vibeInput" },
  // ---- Keys the remote control has, and a keyboard does not ----------------
  //
  // The menu and home buttons live in the panel-wide scope: their meaning is the
  // same in all three layers (open the layer's menu; jump to the live end of the
  // conversation), so one command per button is what keeps them from growing a
  // copy per layer. Both run over the editor because reaching the menu while
  // dictating is exactly when they are needed.
  { id: "vibe.menu", category: "vibe", defaultBinding: "mod+shift+k", aliases: ["contextmenu"], inTerminal: false, overEditable: true, scope: "vibe" },
  { id: "vibe.home", category: "vibe", defaultBinding: "browserhome", inTerminal: false, overEditable: true, scope: "vibe" },
  // Direct selection: nine bindings on one command, because a cell is chosen by
  // the digit printed on it, not by nine separate commands.
  {
    id: "vibe.cell",
    category: "vibe",
    defaultBinding: "1",
    bindings: ["1", "2", "3", "4", "5", "6", "7", "8", "9"],
    inTerminal: false,
    scope: "vibeGrid",
  },
  { id: "vibe.columns.cycle", category: "vibe", defaultBinding: "mod+shift+g", inTerminal: false, scope: "vibe" },
  { id: "vibe.columns.more", category: "vibe", defaultBinding: "mod+equal", inTerminal: false, scope: "vibe" },
  { id: "vibe.columns.fewer", category: "vibe", defaultBinding: "mod+minus", inTerminal: false, scope: "vibe" },
  { id: "vibe.columns.auto", category: "vibe", defaultBinding: "mod+0", inTerminal: false, scope: "vibe" },
  { id: "vibe.move.left", category: "vibe", defaultBinding: "mod+alt+arrowleft", inTerminal: false, scope: "vibe" },
  { id: "vibe.move.right", category: "vibe", defaultBinding: "mod+alt+arrowright", inTerminal: false, scope: "vibe" },
  { id: "vibe.panel.hide", category: "vibe", defaultBinding: "mod+backslash", inTerminal: false, scope: "vibeZoom" },
  { id: "vibe.panel.pin", category: "vibe", defaultBinding: "mod+shift+backslash", inTerminal: false, scope: "vibeZoom" },
  { id: "vibe.device", category: "vibe", defaultBinding: "mod+shift+d", inTerminal: false, scope: "vibe" },
  { id: "vibe.attention", category: "vibe", defaultBinding: "mod+shift+a", inTerminal: false, scope: "vibe" },
  { id: "vibe.add", category: "vibe", defaultBinding: "mod+shift+e", inTerminal: false, scope: "vibe" },
  { id: "vibe.remove", category: "vibe", defaultBinding: "mod+shift+backspace", inTerminal: false, scope: "vibe" },
  { id: "vibe.fullscreen", category: "vibe", defaultBinding: "mod+ctrl+f", inTerminal: false },
  // The cheat sheet from inside the panel. It shares `mod+/` with the global
  // `shortcuts.help` on purpose: the panel binds it in its own scope, so inside
  // the panel it is the panel's key and outside it is the app's. One feature, two
  // owners — which is what a scope is for, and why the two may share a key.
  { id: "vibe.help", category: "vibe", defaultBinding: "mod+slash", inTerminal: false, scope: "vibe" },
];

/** Settings page section order. */
export const CATEGORIES: readonly CommandCategory[] = ["chat", "navigation", "panels", "terminal", "vibe", "palette", "settings"];

const BY_ID = new Map<string, CommandSpec>(COMMANDS.map(spec => [spec.id, spec]));

/** The table entry for an id, or `undefined` for a value this version dropped. */
export function commandSpec(id: string): CommandSpec | undefined {
  return BY_ID.get(id);
}

/** True when `value` is a command this version knows. */
export function isCommandId(value: string): value is CommandId {
  return BY_ID.has(value);
}

/** The keymap a command belongs to. */
export function scopeOf(spec: CommandSpec): ShortcutScope {
  return spec.scope ?? "global";
}

/** Every default text a command answers to (`1`–`9` for direct selection). */
export function defaultBindingTexts(spec: CommandSpec): readonly string[] {
  return spec.bindings ?? [spec.defaultBinding];
}

/**
 * The keys a command always answers to, on top of its binding (`aliases`).
 *
 * A remote control's buttons live here: they are the device's, not the user's, so
 * a rebinding or a cleared binding must not take them away.
 */
export function aliasBindingTexts(spec: CommandSpec): readonly string[] {
  return spec.aliases ?? [];
}

/** Every key a command answers to out of the box, hardware aliases included. */
export function everyDefaultBindingText(spec: CommandSpec): readonly string[] {
  return [...defaultBindingTexts(spec), ...aliasBindingTexts(spec)];
}

/** A command's default bindings, parsed. */
export function defaultBindings(spec: CommandSpec): KeyBinding[] {
  return defaultBindingTexts(spec).map((text) => {
    const binding = parseBinding(text);
    // The defaults are literals in this file and a test parses every one of
    // them; a null here means the table itself was broken by an edit.
    if (!binding)
      throw new Error(`Invalid default binding for ${spec.id}: ${text}`);
    return binding;
  });
}

/** The first default binding, parsed. */
export function defaultBinding(spec: CommandSpec): KeyBinding {
  return defaultBindings(spec)[0]!;
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
