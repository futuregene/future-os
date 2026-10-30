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
 * Four rules for this file:
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
 * - **A chord both surfaces have keeps its letter.** The terminal workbench
 *   (`tui/src/workbench/preferences.rs`) binds the same verbs on `ctrl+…`, and
 *   `mod` *is* `Ctrl` on Windows and Linux — so `mod+p` here is the very key the
 *   terminal binds, and only macOS reads it as `⌘` where the terminal is `⌃`.
 *   The letters that are shared on purpose are `p` (the palette), `d` (the rail)
 *   and `o` / `t` / `s` (the composer's three pickers); the pairs that stay
 *   apart are listed with their reasons in
 *   `docs/internals/desktop/KEYBINDINGS.md` ("Agreement with the TUI").
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
    | "composer.model.previous"
    | "composer.model.next"
    | "composer.thinking.previous"
    | "composer.thinking.next"
    | "composer.approval.previous"
    | "composer.approval.next"
    | "thread.find"
    | "thread.copyReply"
    | "thread.latest"
    | "thread.usage"
    | "composer.attach"
    | "group.previous"
    | "group.next"
    | "thread.previous"
    | "thread.next"
    | "rail.search"
    | "rail.view.recent"
    | "rail.view.workspace"
    | "rail.device.next"
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
    | "shortcuts.settings"
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
    | "vibe.page.previous"
    | "vibe.page.next"
    | "vibe.scroll.up"
    | "vibe.scroll.down"
    | "vibe.zoom.scroll.up"
    | "vibe.zoom.scroll.down"
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
  // Reading a long reply on a small screen: the answer people quote is the one the
  // turn ended on, and reaching for its hover button means finding the row first.
  // The same prose the button beside it copies — see `replyCopyText`.
  { id: "thread.copyReply", category: "chat", defaultBinding: "mod+alt+c", inTerminal: false },
  // Down to the live end of the conversation. `⌘⌥G` is what OpenCode binds to
  // "last message"; the jump button the corner shows when the reader is scrolled
  // away has always existed, and this is it from the keyboard. `Page Up`/`Down`
  // step between questions (a fixed key), so this is the coarse move.
  { id: "thread.latest", category: "chat", defaultBinding: "mod+alt+g", inTerminal: false },
  // The conversation's own account book: the same dialog the header's receipt
  // opens, for a reader whose hands are on the keys.
  { id: "thread.usage", category: "chat", defaultBinding: "mod+alt+u", inTerminal: false },
  // The composer's fourth control beside the three pickers: a paperclip is a
  // pointer target, and attaching is the one composer action with no key at all.
  // `⌘⌥A` is the chord the approval picker gave up when it moved to `⌘S`; `a` is
  // the attachment, and this is the one key in the composer's row that has no
  // counterpart to match (the terminal's door to the same thing is `/paste` and
  // the clipboard, which is a key of its own there).
  { id: "composer.attach", category: "chat", defaultBinding: "mod+alt+a", inTerminal: false },
  // The composer's three pickers, from the keyboard: model, thinking level and
  // who approves a tool call. Stepping the value rather than opening the menu —
  // a `SelectMenu` has no roving focus, so opening it would only get a reader to
  // a pointer target. One pair per picker so ⇧ walks back instead of making the
  // reader go the long way round a ring of six thinking levels.
  //
  // **The letters are the workbench's** (`tui/src/workbench/preferences.rs`):
  // `ctrl+o` the model, `ctrl+t` the thinking level, `ctrl+s` the approval mode.
  // `mod` is `Ctrl` on Windows and Linux, so the chord is the same physical key
  // in both surfaces, and only macOS reads it as `⌘` where the terminal reads
  // `⌃` — one vocabulary for the two places this reader works, rather than three
  // letters learnt twice. Each picker keeps its `⇧` reverse for the reason
  // above.
  { id: "composer.model.previous", category: "chat", defaultBinding: "mod+shift+o", inTerminal: false },
  { id: "composer.model.next", category: "chat", defaultBinding: "mod+o", inTerminal: false },
  { id: "composer.thinking.previous", category: "chat", defaultBinding: "mod+shift+t", inTerminal: false },
  { id: "composer.thinking.next", category: "chat", defaultBinding: "mod+t", inTerminal: false },
  { id: "composer.approval.previous", category: "chat", defaultBinding: "mod+shift+s", inTerminal: false },
  { id: "composer.approval.next", category: "chat", defaultBinding: "mod+s", inTerminal: false },
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
  // Which list the rail draws, one key per answer rather than one toggle: a key
  // labelled "the recent ones" must mean the recent ones on every press, and a
  // toggle's second press would mean the opposite of what it says. `mod+alt+`
  // rather than `mod+shift+`: the sections above are `mod+1`…`mod+5`, and the
  // view is the layer *inside* a section, so it sits one notch deeper.
  { id: "rail.view.recent", category: "navigation", defaultBinding: "mod+alt+1", inTerminal: false },
  { id: "rail.view.workspace", category: "navigation", defaultBinding: "mod+alt+2", inTerminal: false },
  // The device filter, as one ring: all machines → this one → each paired
  // machine. `mod+shift+d` is the Vibe panel's own "which machine" key
  // (`vibe.device`, a scoped command); outside the panel the same keys ask the
  // same question of the conversation list, which is what a scope is for.
  { id: "rail.device.next", category: "navigation", defaultBinding: "mod+shift+d", inTerminal: false, repeat: true },
  { id: "section.chat", category: "navigation", defaultBinding: "mod+1", inTerminal: false },
  { id: "section.workspace", category: "navigation", defaultBinding: "mod+2", inTerminal: false },
  { id: "section.skill", category: "navigation", defaultBinding: "mod+3", inTerminal: false },
  { id: "section.tasks", category: "navigation", defaultBinding: "mod+4", inTerminal: false },
  { id: "section.remote", category: "navigation", defaultBinding: "mod+5", inTerminal: false },
  // The rail, on the workbench's own letter (`ctrl+d` there — `d` is the dock it
  // slides in and out of). `⌘⇧D` sits beside it as the device filter, so the pair
  // reads as one family. Being `inTerminal: false` is what keeps `Ctrl+D` (EOF)
  // the shell's inside the terminal panel.
  { id: "panel.left", category: "panels", defaultBinding: "mod+d", inTerminal: false },
  { id: "panel.right", category: "panels", defaultBinding: "mod+shift+b", inTerminal: false },
  // The terminal panel's key, not the shell's: in the workbench's composer
  // `ctrl+j` inserts a newline, but the panel is what the chord means in both
  // frames and VSCode taught it first. Inside a shell it is handed to the panel
  // deliberately (`features/terminal/shortcut.ts`).
  { id: "terminal.toggle", category: "terminal", defaultBinding: "mod+j", inTerminal: true },
  // `⌘P`, the workbench's `ctrl+p`: the palette is the same door on both
  // surfaces, so it answers to the same letter. It moved off `⌘K`, which the
  // terminal surface cannot use at all — there `ctrl+k` is readline's kill-line.
  { id: "palette.open", category: "palette", defaultBinding: "mod+p", inTerminal: false },
  { id: "shortcuts.help", category: "palette", defaultBinding: "mod+slash", inTerminal: false },
  { id: "settings.open", category: "settings", defaultBinding: "mod+comma", inTerminal: false },
  // Settings, but straight to the page that edits these bindings — the one place
  // `mod+/` sends a reader who has just seen a key they want to change. `mod+,`
  // keeps opening Settings on its first page: this is an addition, not a move.
  { id: "shortcuts.settings", category: "settings", defaultBinding: "mod+shift+comma", inTerminal: false },

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
  // Scrolling a transcript that is **not** the one you are typing in: the grid's cells are
  // small, and reading one of them means moving its text, not moving the ring. `PageUp` /
  // `PageDown` in the grid, and the same keys (plus `↑`/`↓`, which the reading layer has
  // nothing else for) in the zoomed view.
  //
  // `PageUp`/`PageDown` rather than `↑`/`↓` in the grid because the arrows are the ring:
  // one key cannot both "go to the cell above" and "scroll this cell up", and the ring is
  // the gesture a reader makes constantly.
  // Turning the grid's page, past twelve pinned conversations (`VIBE.md` §4). `⌥←`/`⌥→`
  // rather than the page keys, which now scroll the selected cell, and rather than
  // `⌘⌥←`/`→`, which move a cell within its page.
  { id: "vibe.page.previous", category: "vibe", defaultBinding: "alt+arrowleft", inTerminal: false, scope: "vibeGrid" },
  { id: "vibe.page.next", category: "vibe", defaultBinding: "alt+arrowright", inTerminal: false, scope: "vibeGrid" },
  { id: "vibe.scroll.up", category: "vibe", defaultBinding: "pageup", inTerminal: false, repeat: true, scope: "vibeGrid" },
  { id: "vibe.scroll.down", category: "vibe", defaultBinding: "pagedown", inTerminal: false, repeat: true, scope: "vibeGrid" },
  { id: "vibe.zoom.scroll.up", category: "vibe", defaultBinding: "arrowup", bindings: ["arrowup", "pageup"], inTerminal: false, repeat: true, scope: "vibeZoom" },
  { id: "vibe.zoom.scroll.down", category: "vibe", defaultBinding: "arrowdown", bindings: ["arrowdown", "pagedown"], inTerminal: false, repeat: true, scope: "vibeZoom" },
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

/**
 * One block of the settings page and the cheat sheet.
 *
 * A **category** says what a command *is* — the palette labels a result with it
 * and the app menu groups by it. A **section** says how it is *listed*: how much
 * of the keyboard a reader takes in at one glance. They are deliberately not the
 * same thing, so that grouping can follow use rather than taxonomy.
 */
export interface CommandSection {
  id: CommandSectionId;
  /** The categories listed under this heading, in order. */
  categories: readonly CommandCategory[];
  /**
   * Start closed.
   *
   * For the keys of a *mode*: a reader who opened this page to change `⌘N` does
   * not need thirty panel keys between them and the row they came for.
   */
  startsCollapsed?: boolean;
}

export type CommandSectionId = "conversation" | "navigation" | "layout" | "search" | "vibe";

/**
 * The page's blocks, most-used first.
 *
 * Ordering is frequency, not classification: `chat` and `navigation` are what a
 * reader opens this page for, so they are the first two screens. The small
 * groups are listed beside each other rather than each owning a one-row card
 * (`panels` + `terminal` are one "show or hide a pane" idea; the palette and
 * Settings are one "get me somewhere" idea), and the Vibe panel — a full-screen
 * mode most readers never open — is last and closed.
 */
export const SECTIONS: readonly CommandSection[] = [
  { id: "conversation", categories: ["chat"] },
  { id: "navigation", categories: ["navigation"] },
  { id: "layout", categories: ["panels", "terminal"] },
  { id: "search", categories: ["palette", "settings"] },
  { id: "vibe", categories: ["vibe"], startsCollapsed: true },
];

/** Every category, in the order the sections list it. */
export const CATEGORIES: readonly CommandCategory[] = SECTIONS.flatMap(section => section.categories);

/** The commands one section lists, in the table's order. */
export function sectionCommands(section: CommandSection): CommandSpec[] {
  return COMMANDS.filter(spec => section.categories.includes(spec.category));
}

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
  // The wider answer to the same question: approve, and stop asking here.
  { labelKey: "fixed.approveAlways", keys: { mac: "⇧⌘↩", other: "Ctrl+Shift+Enter" } },
  { labelKey: "fixed.transcript", keys: { mac: "⇞ / ⇟", other: "Page Up / Page Down" } },
  { labelKey: "fixed.menu", keys: { mac: "↑ ↓ ↩", other: "Up Down Enter" } },
];
