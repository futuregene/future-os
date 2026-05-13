/**
 * Keyboard input handling for terminal applications.
 *
 * Supports: legacy terminal sequences, Kitty CSI-u protocol, xterm modifyOtherKeys.
 * Ported from pi-mono for xihu TUI.
 */

// ─── Global Kitty Protocol State ─────────────────────────────────────────

let _kittyProtocolActive = false;

export function setKittyProtocolActive(active: boolean): void {
  _kittyProtocolActive = active;
}

export function isKittyProtocolActive(): boolean {
  return _kittyProtocolActive;
}

// ─── Constants ───────────────────────────────────────────────────────────

const SYMBOL_KEYS = new Set([
  "`", "-", "=", "[", "]", "\\", ";", "'", ",", ".", "/",
  "!", "@", "#", "$", "%", "^", "&", "*", "(", ")",
  "_", "+", "|", "~", "{", "}", ":", "<", ">", "?",
]);

const MODIFIERS = {
  shift: 1,
  alt: 2,
  ctrl: 4,
  super: 8,
} as const;

const LOCK_MASK = 64 + 128;

const CODEPOINTS = {
  escape: 27,
  tab: 9,
  enter: 13,
  space: 32,
  backspace: 127,
  kpEnter: 57414,
  delete: 57425,
} as const;

const ARROW_CODEPOINTS = {
  up: -1,
  down: -2,
  right: -3,
  left: -4,
} as const;

const FUNCTIONAL_CODEPOINTS = {
  delete: -10,
  insert: -11,
  pageUp: -12,
  pageDown: -13,
  home: -14,
  end: -15,
} as const;

const KITTY_FUNCTIONAL_KEY_EQUIVALENTS = new Map<number, number>([
  [57399, 48], [57400, 49], [57401, 50], [57402, 51], [57403, 52],
  [57404, 53], [57405, 54], [57406, 55], [57407, 56], [57408, 57],
  [57409, 46], [57410, 47], [57411, 42], [57412, 45], [57413, 43],
  [57415, 61], [57416, 44],
  [57417, ARROW_CODEPOINTS.left], [57418, ARROW_CODEPOINTS.right],
  [57419, ARROW_CODEPOINTS.up], [57420, ARROW_CODEPOINTS.down],
  [57421, FUNCTIONAL_CODEPOINTS.pageUp], [57422, FUNCTIONAL_CODEPOINTS.pageDown],
  [57423, FUNCTIONAL_CODEPOINTS.home], [57424, FUNCTIONAL_CODEPOINTS.end],
  [57425, FUNCTIONAL_CODEPOINTS.insert], [57426, FUNCTIONAL_CODEPOINTS.delete],
]);

function normalizeKittyFunctionalCodepoint(codepoint: number): number {
  return KITTY_FUNCTIONAL_KEY_EQUIVALENTS.get(codepoint) ?? codepoint;
}

function normalizeShiftedLetterIdentityCodepoint(codepoint: number, modifier: number): number {
  const effectiveModifier = modifier & ~LOCK_MASK;
  if ((effectiveModifier & MODIFIERS.shift) !== 0 && codepoint >= 65 && codepoint <= 90) {
    return codepoint + 32;
  }
  return codepoint;
}

// ─── Legacy Sequence Mappings ────────────────────────────────────────────

const LEGACY_SEQUENCE_KEY_IDS: Record<string, string> = {
  "\x1bOA": "up", "\x1bOB": "down", "\x1bOC": "right", "\x1bOD": "left",
  "\x1bOH": "home", "\x1bOF": "end",
  "\x1b[E": "clear", "\x1bOE": "clear",
  "\x1bOe": "ctrl+clear", "\x1b[e": "shift+clear",
  "\x1b[2~": "insert", "\x1b[2$": "shift+insert", "\x1b[2^": "ctrl+insert",
  "\x1b[3~": "delete", "\x1b[3$": "shift+delete", "\x1b[3^": "ctrl+delete",
  "\x1b[[5~": "pageUp", "\x1b[[6~": "pageDown",
  "\x1b[a": "shift+up", "\x1b[b": "shift+down", "\x1b[c": "shift+right", "\x1b[d": "shift+left",
  "\x1bOa": "ctrl+up", "\x1bOb": "ctrl+down", "\x1bOc": "ctrl+right", "\x1bOd": "ctrl+left",
  "\x1b[5$": "shift+pageUp", "\x1b[6$": "shift+pageDown",
  "\x1b[7$": "shift+home", "\x1b[8$": "shift+end",
  "\x1b[5^": "ctrl+pageUp", "\x1b[6^": "ctrl+pageDown",
  "\x1b[7^": "ctrl+home", "\x1b[8^": "ctrl+end",
  "\x1bOP": "f1", "\x1bOQ": "f2", "\x1bOR": "f3", "\x1bOS": "f4",
  "\x1b[11~": "f1", "\x1b[12~": "f2", "\x1b[13~": "f3", "\x1b[14~": "f4",
  "\x1b[[A": "f1", "\x1b[[B": "f2", "\x1b[[C": "f3", "\x1b[[D": "f4", "\x1b[[E": "f5",
  "\x1b[15~": "f5", "\x1b[17~": "f6", "\x1b[18~": "f7",
  "\x1b[19~": "f8", "\x1b[20~": "f9", "\x1b[21~": "f10",
  "\x1b[23~": "f11", "\x1b[24~": "f12",
  "\x1bb": "alt+left", "\x1bf": "alt+right", "\x1bp": "alt+up", "\x1bn": "alt+down",
};

// ─── Kitty CSI-u Parsing ─────────────────────────────────────────────────

export type KeyEventType = "press" | "repeat" | "release";

interface ParsedKittySequence {
  codepoint: number;
  shiftedKey?: number;
  baseLayoutKey?: number;
  modifier: number;
  eventType: KeyEventType;
}

interface ParsedModifyOtherKeysSequence {
  codepoint: number;
  modifier: number;
}

function parseEventType(eventTypeStr: string | undefined): KeyEventType {
  if (!eventTypeStr) return "press";
  const eventType = parseInt(eventTypeStr, 10);
  if (eventType === 2) return "repeat";
  if (eventType === 3) return "release";
  return "press";
}

function parseKittySequence(data: string): ParsedKittySequence | null {
  // CSI u: \x1b[<codepoint>;<mod>u or \x1b[<codepoint>:<shifted>:<base>;<mod>:<event>u
  const csiUMatch = data.match(/^\x1b\[(\d+)(?::(\d*))?(?::(\d+))?(?:;(\d+))?(?::(\d+))?u$/);
  if (csiUMatch) {
    const codepoint = parseInt(csiUMatch[1]!, 10);
    const shiftedKey = csiUMatch[2] && csiUMatch[2].length > 0 ? parseInt(csiUMatch[2], 10) : undefined;
    const baseLayoutKey = csiUMatch[3] ? parseInt(csiUMatch[3], 10) : undefined;
    const modValue = csiUMatch[4] ? parseInt(csiUMatch[4], 10) : 1;
    const eventType = parseEventType(csiUMatch[5]);
    return { codepoint, shiftedKey, baseLayoutKey, modifier: modValue - 1, eventType };
  }

  // Arrow keys: \x1b[1;<mod>A/B/C/D or \x1b[1;<mod>:<event>A/B/C/D
  const arrowMatch = data.match(/^\x1b\[1;(\d+)(?::(\d+))?([ABCD])$/);
  if (arrowMatch) {
    const modValue = parseInt(arrowMatch[1]!, 10);
    const eventType = parseEventType(arrowMatch[2]);
    const arrowCodes: Record<string, number> = { A: -1, B: -2, C: -3, D: -4 };
    return { codepoint: arrowCodes[arrowMatch[3]!]!, modifier: modValue - 1, eventType };
  }

  // Home/End: \x1b[<codepoint>;<mod>H/F or \x1b[<codepoint>;<mod>:<event>H/F
  const homeEndMatch = data.match(/^\x1b\[(\d+);(\d+)(?::(\d+))?([HF])$/);
  if (homeEndMatch) {
    const codepoint = parseInt(homeEndMatch[1]!, 10);
    const modValue = parseInt(homeEndMatch[2]!, 10);
    const eventType = parseEventType(homeEndMatch[3]);
    const normalizedCodepoint = homeEndMatch[4] === "H" ? -14 : -15; // home : end
    return { codepoint: normalizedCodepoint, modifier: modValue - 1, eventType };
  }

  return null;
}

// ─── modifyOtherKeys Parsing ─────────────────────────────────────────────

function parseModifyOtherKeysSequence(data: string): ParsedModifyOtherKeysSequence | null {
  const match = data.match(/^\x1b\[27;(\d+);(\d+)~$/);
  if (!match) return null;
  const modValue = parseInt(match[1]!, 10);
  const codepoint = parseInt(match[2]!, 10);
  return { codepoint, modifier: modValue - 1 };
}

// ─── Event Type Detection ────────────────────────────────────────────────

export function isKeyRelease(data: string): boolean {
  if (data.includes("\x1b[200~")) return false;
  return /:3[u~ABCDHF]/.test(data);
}

export function isKeyRepeat(data: string): boolean {
  if (data.includes("\x1b[200~")) return false;
  return /:2[u~ABCDHF]/.test(data);
}

// ─── Key Name Formatting ─────────────────────────────────────────────────

function isWindowsTerminalSession(): boolean {
  return Boolean(process.env.WT_SESSION) && !process.env.SSH_CONNECTION
    && !process.env.SSH_CLIENT && !process.env.SSH_TTY;
}

function formatKeyNameWithModifiers(keyName: string, modifier: number): string | undefined {
  const mods: string[] = [];
  const effectiveMod = modifier & ~LOCK_MASK;
  const supported = MODIFIERS.shift | MODIFIERS.ctrl | MODIFIERS.alt | MODIFIERS.super;
  if ((effectiveMod & ~supported) !== 0) return undefined;
  if (effectiveMod & MODIFIERS.shift) mods.push("shift");
  if (effectiveMod & MODIFIERS.ctrl) mods.push("ctrl");
  if (effectiveMod & MODIFIERS.alt) mods.push("alt");
  if (effectiveMod & MODIFIERS.super) mods.push("super");
  return mods.length > 0 ? `${mods.join("+")}+${keyName}` : keyName;
}

function formatParsedKey(codepoint: number, modifier: number, baseLayoutKey?: number): string | undefined {
  const normalizedCodepoint = normalizeKittyFunctionalCodepoint(codepoint);
  const identityCodepoint = normalizeShiftedLetterIdentityCodepoint(normalizedCodepoint, modifier);

  const isLatinLetter = identityCodepoint >= 97 && identityCodepoint <= 122;
  const isDigit = identityCodepoint >= 48 && identityCodepoint <= 57;
  const isKnownSymbol = SYMBOL_KEYS.has(String.fromCharCode(identityCodepoint));
  const effectiveCodepoint =
    isLatinLetter || isDigit || isKnownSymbol ? identityCodepoint : (baseLayoutKey ?? identityCodepoint);

  let keyName: string | undefined;
  if (effectiveCodepoint === CODEPOINTS.escape) keyName = "escape";
  else if (effectiveCodepoint === CODEPOINTS.tab) keyName = "tab";
  else if (effectiveCodepoint === CODEPOINTS.enter || effectiveCodepoint === CODEPOINTS.kpEnter) keyName = "enter";
  else if (effectiveCodepoint === CODEPOINTS.space) keyName = "space";
  else if (effectiveCodepoint === CODEPOINTS.backspace) keyName = "backspace";
  else if (effectiveCodepoint === FUNCTIONAL_CODEPOINTS.delete) keyName = "delete";
  else if (effectiveCodepoint === FUNCTIONAL_CODEPOINTS.insert) keyName = "insert";
  else if (effectiveCodepoint === FUNCTIONAL_CODEPOINTS.home) keyName = "home";
  else if (effectiveCodepoint === FUNCTIONAL_CODEPOINTS.end) keyName = "end";
  else if (effectiveCodepoint === FUNCTIONAL_CODEPOINTS.pageUp) keyName = "pageUp";
  else if (effectiveCodepoint === FUNCTIONAL_CODEPOINTS.pageDown) keyName = "pageDown";
  else if (effectiveCodepoint === ARROW_CODEPOINTS.up) keyName = "up";
  else if (effectiveCodepoint === ARROW_CODEPOINTS.down) keyName = "down";
  else if (effectiveCodepoint === ARROW_CODEPOINTS.left) keyName = "left";
  else if (effectiveCodepoint === ARROW_CODEPOINTS.right) keyName = "right";
  else if (effectiveCodepoint >= 48 && effectiveCodepoint <= 57) keyName = String.fromCharCode(effectiveCodepoint);
  else if (effectiveCodepoint >= 97 && effectiveCodepoint <= 122) keyName = String.fromCharCode(effectiveCodepoint);
  else if (SYMBOL_KEYS.has(String.fromCharCode(effectiveCodepoint))) keyName = String.fromCharCode(effectiveCodepoint);

  if (!keyName) return undefined;
  return formatKeyNameWithModifiers(keyName, modifier);
}

// ─── Main Parse Function ─────────────────────────────────────────────────

/**
 * Parse raw terminal input into a normalized key identifier string.
 * Handles Kitty CSI-u, xterm modifyOtherKeys, and legacy escape sequences.
 *
 * Returns a string like "ctrl+c", "shift+tab", "up", "enter", "a", etc.
 * Returns undefined for unrecognized input.
 */
export function parseKey(data: string): string | undefined {
  // Kitty CSI-u protocol
  const kitty = parseKittySequence(data);
  if (kitty) {
    return formatParsedKey(kitty.codepoint, kitty.modifier, kitty.baseLayoutKey);
  }

  // xterm modifyOtherKeys
  const modifyOtherKeys = parseModifyOtherKeysSequence(data);
  if (modifyOtherKeys) {
    return formatParsedKey(modifyOtherKeys.codepoint, modifyOtherKeys.modifier);
  }

  // Mode-aware legacy sequences
  if (_kittyProtocolActive) {
    if (data === "\x1b\r" || data === "\n") return "shift+enter";
  }

  const legacyKeyId = LEGACY_SEQUENCE_KEY_IDS[data];
  if (legacyKeyId) return legacyKeyId;

  // Individual legacy sequences
  if (data === "\x1b") return "escape";
  if (data === "\x1c") return "ctrl+\\";
  if (data === "\x1d") return "ctrl+]";
  if (data === "\x1f") return "ctrl+-";
  if (data === "\x1b\x1b") return "ctrl+alt+[";
  if (data === "\x1b\x1c") return "ctrl+alt+\\";
  if (data === "\x1b\x1d") return "ctrl+alt+]";
  if (data === "\x1b\x1f") return "ctrl+alt+-";
  if (data === "\t") return "tab";
  if (data === "\r" || (!_kittyProtocolActive && data === "\n") || data === "\x1bOM") return "enter";
  if (data === "\x00") return "ctrl+space";
  if (data === " ") return "space";
  if (data === "\x7f") return "backspace";
  if (data === "\x08") return isWindowsTerminalSession() ? "ctrl+backspace" : "backspace";
  if (data === "\x1b[Z") return "shift+tab";
  if (!_kittyProtocolActive && data === "\x1b\r") return "alt+enter";
  if (data === "\x1b\x7f" || data === "\x1b\b") return "alt+backspace";
  if (!_kittyProtocolActive && data === "\x1b ") return "alt+space";

  // Legacy alt+letter/digit (ESC followed by key)
  if (!_kittyProtocolActive && data.length === 2 && data[0] === "\x1b") {
    const code = data.charCodeAt(1);
    if (code >= 1 && code <= 26) {
      return `ctrl+alt+${String.fromCharCode(code + 96)}`;
    }
    if ((code >= 97 && code <= 122) || (code >= 48 && code <= 57)) {
      return `alt+${String.fromCharCode(code)}`;
    }
  }

  // Raw Ctrl+letter
  if (data.length === 1) {
    const code = data.charCodeAt(0);
    if (code >= 1 && code <= 26) {
      return `ctrl+${String.fromCharCode(code + 96)}`;
    }
    if (code >= 32 && code <= 126) {
      return data;
    }
  }

  return undefined;
}

// ─── Printable Key Decoding ──────────────────────────────────────────────

/**
 * Decode a Kitty CSI-u sequence into a printable character.
 */
export function decodeKittyPrintable(data: string): string | undefined {
  const match = data.match(/^\x1b\[(\d+)(?::(\d*))?(?::(\d+))?(?:;(\d+))?(?::(\d+))?u$/);
  if (!match) return undefined;

  const codepoint = Number.parseInt(match[1] ?? "", 10);
  if (!Number.isFinite(codepoint)) return undefined;

  const shiftedKey = match[2] && match[2].length > 0 ? Number.parseInt(match[2], 10) : undefined;
  const modValue = match[4] ? Number.parseInt(match[4], 10) : 1;
  const modifier = Number.isFinite(modValue) ? modValue - 1 : 0;

  if ((modifier & ~(MODIFIERS.shift | LOCK_MASK)) !== 0) return undefined;
  if (modifier & (MODIFIERS.alt | MODIFIERS.ctrl)) return undefined;

  let effectiveCodepoint = codepoint;
  if (modifier & MODIFIERS.shift && typeof shiftedKey === "number") {
    effectiveCodepoint = shiftedKey;
  }
  effectiveCodepoint = normalizeKittyFunctionalCodepoint(effectiveCodepoint);
  if (!Number.isFinite(effectiveCodepoint) || effectiveCodepoint < 32) return undefined;

  try {
    return String.fromCodePoint(effectiveCodepoint);
  } catch {
    return undefined;
  }
}

function decodeModifyOtherKeysPrintable(data: string): string | undefined {
  const parsed = parseModifyOtherKeysSequence(data);
  if (!parsed) return undefined;
  const modifier = parsed.modifier & ~LOCK_MASK;
  if ((modifier & ~MODIFIERS.shift) !== 0) return undefined;
  if (!Number.isFinite(parsed.codepoint) || parsed.codepoint < 32) return undefined;
  try {
    return String.fromCodePoint(parsed.codepoint);
  } catch {
    return undefined;
  }
}

export function decodePrintableKey(data: string): string | undefined {
  return decodeKittyPrintable(data) ?? decodeModifyOtherKeysPrintable(data);
}

// ─── KeyId Type & Key Helper ──────────────────────────────────────────────

/** All possible normalized key identifiers returned by parseKey(). */
export type KeyId = string;

function k<T extends string>(s: T): T { return s; }

/**
 * Key helper object providing autocomplete for common key identifiers.
 * Usage: `Key.escape`, `Key.up`, `Key.ctrl_c`, `Key.shift_tab`
 */
export const Key = {
  // Named keys
  escape: k("escape"),
  tab: k("tab"),
  enter: k("enter"),
  space: k("space"),
  backspace: k("backspace"),
  delete: k("delete"),
  insert: k("insert"),
  home: k("home"),
  end: k("end"),
  pageUp: k("pageUp"),
  pageDown: k("pageDown"),
  up: k("up"),
  down: k("down"),
  left: k("left"),
  right: k("right"),
  clear: k("clear"),

  // Function keys
  f1: k("f1"), f2: k("f2"), f3: k("f3"), f4: k("f4"),
  f5: k("f5"), f6: k("f6"), f7: k("f7"), f8: k("f8"),
  f9: k("f9"), f10: k("f10"), f11: k("f11"), f12: k("f12"),

  // Ctrl shortcuts
  ctrl_a: k("ctrl+a"), ctrl_b: k("ctrl+b"), ctrl_c: k("ctrl+c"),
  ctrl_d: k("ctrl+d"), ctrl_e: k("ctrl+e"), ctrl_f: k("ctrl+f"),
  ctrl_g: k("ctrl+g"), ctrl_h: k("ctrl+h"), ctrl_i: k("ctrl+i"),
  ctrl_j: k("ctrl+j"), ctrl_k: k("ctrl+k"), ctrl_l: k("ctrl+l"),
  ctrl_m: k("ctrl+m"), ctrl_n: k("ctrl+n"), ctrl_o: k("ctrl+o"),
  ctrl_p: k("ctrl+p"), ctrl_q: k("ctrl+q"), ctrl_r: k("ctrl+r"),
  ctrl_s: k("ctrl+s"), ctrl_t: k("ctrl+t"), ctrl_u: k("ctrl+u"),
  ctrl_v: k("ctrl+v"), ctrl_w: k("ctrl+w"), ctrl_x: k("ctrl+x"),
  ctrl_y: k("ctrl+y"), ctrl_z: k("ctrl+z"),

  // Ctrl + named keys
  ctrl_up: k("ctrl+up"), ctrl_down: k("ctrl+down"),
  ctrl_left: k("ctrl+left"), ctrl_right: k("ctrl+right"),
  ctrl_home: k("ctrl+home"), ctrl_end: k("ctrl+end"),
  ctrl_pageUp: k("ctrl+pageUp"), ctrl_pageDown: k("ctrl+pageDown"),
  ctrl_backspace: k("ctrl+backspace"), ctrl_delete: k("ctrl+delete"),
  ctrl_space: k("ctrl+space"), ctrl_enter: k("ctrl+enter"),

  // Shift modifiers
  shift_tab: k("shift+tab"), shift_enter: k("shift+enter"),
  shift_up: k("shift+up"), shift_down: k("shift+down"),
  shift_left: k("shift+left"), shift_right: k("shift+right"),

  // Alt modifiers
  alt_up: k("alt+up"), alt_down: k("alt+down"),
  alt_left: k("alt+left"), alt_right: k("alt+right"),
  alt_enter: k("alt+enter"), alt_backspace: k("alt+backspace"),
  alt_space: k("alt+space"),
} as const;

export type KeyConstant = (typeof Key)[keyof typeof Key];

/** Build a modified key string: `modifiedKey("ctrl", "c")` → `"ctrl+c"` */
export function modifiedKey(mod: string, key: string): KeyId {
  return `${mod}+${key}`;
}

/** Build a ctrl+key string: `ctrlKey("c")` → `"ctrl+c"` */
export function ctrlKey(key: string): KeyId {
  return `ctrl+${key}`;
}
