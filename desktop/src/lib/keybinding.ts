/**
 * A keyboard binding as data: parse it, serialize it, match it, label it, judge
 * it.
 *
 * Nothing here touches React, the DOM's event system or the locale bundles —
 * the same rules must hold in the settings page, in the dispatcher and in a
 * test, so they live in one dependency-free module (like `platform` and
 * `fontSize`).
 *
 * Three decisions are worth knowing before changing anything below.
 *
 * **`mod` is not a key.** It is the platform's primary accelerator: `⌘` on
 * macOS, `Ctrl` on Windows and Linux. Storing `mod+n` rather than `⌘N` keeps one
 * default valid on all three platforms, and it makes a user's rebinding portable
 * — "new conversation" stays the platform's accelerator on whatever machine the
 * settings are read on.
 *
 * **Matching is on `event.code`, not `event.key`.** `key` is what the *layout*
 * types: on AZERTY `Shift+,` is `?`, on a German layout `z` and `y` trade places,
 * and a Dvorak user's `Ctrl+Z` would sit on a different physical key. `code` is
 * the physical key, which is what "the same shortcut" means for a hand that
 * remembers a position. `key` is only a fallback for the engines that leave
 * `code` empty (some WebKitGTK builds, synthesised events).
 *
 * **A binding must carry a real modifier.** A bare key is ordinary typing, and
 * two of this app's own keyboard features (the transcript's Page Up/Down, the
 * rail's arrow keys) already exist because taking a bare key over costs the user
 * a keystroke every time. `Shift` alone does not count: `Shift+F` is how everyone
 * types a capital F.
 */

/** The modifiers a binding can name, in canonical serialization order. */
const MODIFIERS = ["mod", "ctrl", "alt", "shift"] as const;
type Modifier = (typeof MODIFIERS)[number];

/** A parsed binding: which modifiers, and which physical key. */
export interface KeyBinding {
  /** The platform accelerator: `⌘` on macOS, `Ctrl` elsewhere. */
  mod: boolean;
  /** A literal Ctrl — only meaningful on macOS, where `mod` is `⌘`. */
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  /** A canonical `KeyboardEvent.code` value, e.g. `"KeyN"` or `"Comma"`. */
  code: string;
}

/** The part of a keyboard event a binding reads; tests need no real event. */
export interface BindingKeyEvent {
  key: string;
  code?: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

/** Punctuation typed with a `key` of one character, mapped to its physical code. */
const PUNCTUATION_CODES: Record<string, string> = {
  ",": "Comma",
  ".": "Period",
  "/": "Slash",
  ";": "Semicolon",
  "'": "Quote",
  "[": "BracketLeft",
  "]": "BracketRight",
  "\\": "Backslash",
  "-": "Minus",
  "=": "Equal",
  "`": "Backquote",
  " ": "Space",
};

/** Code → the character it types on a US layout, for labels. */
const PUNCTUATION_LABELS: Record<string, string> = {
  Comma: ",",
  Period: ".",
  Slash: "/",
  Semicolon: ";",
  Quote: "'",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Minus: "-",
  Equal: "=",
  Backquote: "`",
  Space: "Space",
};

/** Named keys, by the words a person would write and by their own code name. */
const NAMED_CODES: Record<string, string> = (() => {
  const named: Record<string, string> = {
    enter: "Enter",
    return: "Enter",
    escape: "Escape",
    esc: "Escape",
    tab: "Tab",
    space: "Space",
    backspace: "Backspace",
    delete: "Delete",
    del: "Delete",
    insert: "Insert",
    ins: "Insert",
    home: "Home",
    end: "End",
    pageup: "PageUp",
    pagedown: "PageDown",
    up: "ArrowUp",
    down: "ArrowDown",
    left: "ArrowLeft",
    right: "ArrowRight",
    arrowup: "ArrowUp",
    arrowdown: "ArrowDown",
    arrowleft: "ArrowLeft",
    arrowright: "ArrowRight",
  };
  // Punctuation answers to both spellings: the character (`","`) and its code
  // (`"comma"`), so a hand-written or stored binding in either form parses.
  for (const [character, code] of Object.entries(PUNCTUATION_CODES)) {
    if (character === " ")
      continue;
    named[code.toLowerCase()] = code;
    named[character] = code;
  }
  return named;
})();

/** How a non-character key reads in a label, per platform. */
const KEY_LABELS: Record<string, { mac: string; other: string }> = {
  Enter: { mac: "↩", other: "Enter" },
  Escape: { mac: "Esc", other: "Esc" },
  Tab: { mac: "⇥", other: "Tab" },
  Backspace: { mac: "⌫", other: "Backspace" },
  Delete: { mac: "⌦", other: "Delete" },
  Insert: { mac: "Ins", other: "Ins" },
  Home: { mac: "↖", other: "Home" },
  End: { mac: "↘", other: "End" },
  PageUp: { mac: "⇞", other: "PgUp" },
  PageDown: { mac: "⇟", other: "PgDn" },
  ArrowUp: { mac: "↑", other: "Up" },
  ArrowDown: { mac: "↓", other: "Down" },
  ArrowLeft: { mac: "←", other: "Left" },
  ArrowRight: { mac: "→", other: "Right" },
};

/**
 * Combinations an app must not take over: the ones the OS or the platform's own
 * text editing owns, plus the webview's reload and developer tools.
 *
 * They are refused by the recorder rather than silently accepted, because a
 * binding the platform eats is a binding that looks assigned and does nothing.
 */
const RESERVED_BINDINGS: ReadonlyArray<{ binding: string; macOnly?: boolean }> = [
  // Editing: every text field in the app relies on these.
  { binding: "mod+c" },
  { binding: "mod+v" },
  { binding: "mod+x" },
  { binding: "mod+a" },
  { binding: "mod+z" },
  { binding: "mod+shift+z" },
  { binding: "mod+y" },
  // The webview's own reload and developer tools.
  { binding: "mod+r" },
  { binding: "mod+alt+i" },
  // Closing the window from the keyboard.
  { binding: "alt+f4" },
  // macOS: quit, hide, minimise, close, Spotlight, the app and window switchers,
  // and the screenshot chords.
  { binding: "mod+q", macOnly: true },
  { binding: "mod+h", macOnly: true },
  { binding: "mod+m", macOnly: true },
  { binding: "mod+w", macOnly: true },
  { binding: "mod+space", macOnly: true },
  { binding: "mod+tab", macOnly: true },
  { binding: "mod+backquote", macOnly: true },
  { binding: "mod+shift+3", macOnly: true },
  { binding: "mod+shift+4", macOnly: true },
  { binding: "mod+shift+5", macOnly: true },
];

/**
 * Parse a written binding (`"mod+shift+k"`, `"ctrl+,"`, `"alt+F5"`).
 *
 * Returns `null` for anything this module does not understand, so a stored value
 * that a future version cannot read is dropped rather than misread as some other
 * key. The last non-modifier token is the key; a binding needs one.
 */
export function parseBinding(text: string): KeyBinding | null {
  const parts = text.split("+").map(part => part.trim()).filter(part => part.length > 0);
  if (parts.length === 0)
    return null;
  const binding: KeyBinding = { mod: false, ctrl: false, alt: false, shift: false, code: "" };
  let code: string | null = null;
  for (const part of parts) {
    const modifier = modifierOf(part);
    if (modifier) {
      // A named modifier after the key has been read means the text is not a
      // binding at all ("mod+k+shift" would otherwise mean two different things).
      if (code !== null)
        return null;
      binding[modifier] = true;
      continue;
    }
    if (code !== null)
      return null;
    code = codeOf(part);
    if (code === null)
      return null;
  }
  if (code === null)
    return null;
  binding.code = code;
  return binding;
}

function modifierOf(token: string): Modifier | null {
  const lower = token.toLowerCase();
  if (lower === "mod" || lower === "cmd" || lower === "command" || lower === "meta")
    return "mod";
  if (lower === "ctrl" || lower === "control")
    return "ctrl";
  if (lower === "alt" || lower === "option" || lower === "opt")
    return "alt";
  if (lower === "shift")
    return "shift";
  return null;
}

/** The canonical code a key token names, or `null` when it names none. */
function codeOf(token: string): string | null {
  if (/^[a-z]$/i.test(token))
    return `Key${token.toUpperCase()}`;
  if (/^\d$/.test(token))
    return `Digit${token}`;
  if (PUNCTUATION_CODES[token])
    return PUNCTUATION_CODES[token]!;
  if (/^f(?:[1-9]|1\d|2[0-4])$/i.test(token))
    return token.toUpperCase();
  if (/^key[a-z]$/i.test(token))
    return `Key${token.slice(3).toUpperCase()}`;
  if (/^digit\d$/i.test(token))
    return `Digit${token.slice(5)}`;
  return NAMED_CODES[token.toLowerCase()] ?? null;
}

/** The canonical text form: modifiers in a fixed order, then the key. */
export function serializeBinding(binding: KeyBinding): string {
  const parts = MODIFIERS.filter(modifier => binding[modifier]);
  return [...parts, binding.code].join("+");
}

/** Two bindings name the same combination. */
export function sameBinding(left: KeyBinding, right: KeyBinding): boolean {
  return serializeBinding(left) === serializeBinding(right);
}

/**
 * The physical key an event is on, or `null` when the event names none.
 *
 * `code` wins; `key` is the fallback for engines and synthetic events that leave
 * it empty.
 */
export function eventCode(event: BindingKeyEvent): string | null {
  const code = event.code?.trim();
  if (code)
    return code;
  return codeOf(event.key);
}

/** Whether the event is the combination this binding names. */
export function matchesBinding(
  binding: KeyBinding,
  event: BindingKeyEvent,
  isMac: boolean,
): boolean {
  // On Windows and Linux `mod` *is* Ctrl, so an explicit `ctrl` names the same
  // key there; on macOS they are two different keys.
  const expectCtrl = isMac ? binding.ctrl : binding.ctrl || binding.mod;
  const expectMeta = isMac ? binding.mod : false;
  // Strict equality in both directions: an extra modifier means the user pressed
  // something else (`Ctrl+Shift+J` is not `Ctrl+J`).
  if (event.ctrlKey !== expectCtrl || event.metaKey !== expectMeta)
    return false;
  if (event.altKey !== binding.alt || event.shiftKey !== binding.shift)
    return false;
  const code = eventCode(event);
  return code !== null && code === binding.code;
}

/** How one physical key reads in a label. */
export function keyLabel(code: string, isMac: boolean): string {
  if (PUNCTUATION_LABELS[code])
    return PUNCTUATION_LABELS[code]!;
  if (/^Key[A-Z]$/.test(code))
    return code.slice(3);
  if (/^Digit\d$/.test(code))
    return code.slice(5);
  const named = KEY_LABELS[code];
  if (named)
    return isMac ? named.mac : named.other;
  return code;
}

/**
 * A binding in the platform's own notation: `⌘⇧K` on macOS (modifiers in the
 * order Apple prints them, `⌃⌥⇧⌘`), `Ctrl+Shift+K` everywhere else.
 */
export function formatBinding(binding: KeyBinding, isMac: boolean): string {
  const key = keyLabel(binding.code, isMac);
  if (isMac) {
    return `${binding.ctrl ? "⌃" : ""}${binding.alt ? "⌥" : ""}${binding.shift ? "⇧" : ""}${binding.mod ? "⌘" : ""}${key}`;
  }
  const parts: string[] = [];
  if (binding.mod || binding.ctrl)
    parts.push("Ctrl");
  if (binding.alt)
    parts.push("Alt");
  if (binding.shift)
    parts.push("Shift");
  parts.push(key);
  return parts.join("+");
}

/** What is wrong with a candidate binding, or `null` when nothing is. */
export type BindingProblem = "needsModifier" | "reserved";

/**
 * Judge a binding the user is about to assign.
 *
 * `needsModifier` keeps bare keys out of the table (see the module comment);
 * `reserved` refuses the combinations the OS and the platform's text editing
 * own. Conflicts with another command are the caller's business — only it knows
 * which commands exist.
 */
export function bindingProblem(binding: KeyBinding, isMac: boolean): BindingProblem | null {
  if (!binding.mod && !binding.ctrl && !binding.alt)
    return "needsModifier";
  const text = serializeBinding(binding);
  for (const entry of RESERVED_BINDINGS) {
    if (entry.macOnly && !isMac)
      continue;
    const reserved = parseBinding(entry.binding);
    if (reserved && serializeBinding(reserved) === text)
      return "reserved";
  }
  return null;
}

/** The bindings the recorder refuses, for the settings page's read-only list. */
export function reservedBindingLabels(isMac: boolean): string[] {
  return RESERVED_BINDINGS
    .filter(entry => !entry.macOnly || isMac)
    .map((entry) => {
      const binding = parseBinding(entry.binding);
      return binding ? formatBinding(binding, isMac) : entry.binding;
    });
}

/**
 * True for the keys that only ever act as modifiers.
 *
 * The recorder uses this to wait for the *real* key instead of accepting the
 * first `Shift` it sees.
 */
export function isModifierKey(event: { key: string }): boolean {
  return ["Shift", "Control", "Alt", "Meta", "AltGraph", "CapsLock", "Fn", "FnLock", "Hyper", "Super", "Symbol", "SymbolLock", "OS"]
    .includes(event.key);
}

/**
 * The binding an event would store, or `null` while it is not a binding yet.
 *
 * The recorder's whole job: a modifier-only press yields `null` (keep waiting),
 * a real key yields the parsed binding. Modifier state comes from the event, so
 * the stored text says nothing about *how* the key was held — which is what lets
 * the same text describe `⌘N` and `Ctrl+N`.
 */
export function bindingFromEvent(event: BindingKeyEvent, isMac: boolean): KeyBinding | null {
  if (isModifierKey(event))
    return null;
  const code = eventCode(event);
  if (code === null)
    return null;
  return {
    mod: isMac ? event.metaKey : event.ctrlKey,
    // On Windows and Linux the accelerator *is* Ctrl, so `ctrl` would duplicate
    // `mod` in the stored text and make the same key look like two modifiers.
    ctrl: isMac ? event.ctrlKey : false,
    alt: event.altKey,
    shift: event.shiftKey,
    code,
  };
}
