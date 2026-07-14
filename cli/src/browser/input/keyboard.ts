/**
 * Keyboard helper — maps human-readable key names to CDP Input.dispatchKeyEvent parameters.
 *
 * Covers the keys used by the current browser-tools.ts press command.
 */
export interface ParsedKey {
  key: string;
  code: string;
  text: string;
  windowsVirtualKeyCode: number;
  nativeVirtualKeyCode: number;
  modifiers: number;
  type: "keyDown" | "keyUp" | "char";
}

/** CDP Input.dispatchKeyEvent modifier bitmask. */
export const Modifiers = {
  None: 0,
  Alt: 1,
  Control: 2,
  Meta: 4,
  Shift: 8,
} as const;

// ── Key table ───────────────────────────────────────────────────────

const KEY_TABLE: Record<string, {
  key: string;
  code: string;
  windowsVirtualKeyCode: number;
  nativeVirtualKeyCode: number;
}> = {
  "enter":     { key: "Enter",     code: "Enter",     windowsVirtualKeyCode: 13,  nativeVirtualKeyCode: 36 },
  "tab":       { key: "Tab",       code: "Tab",       windowsVirtualKeyCode: 9,   nativeVirtualKeyCode: 48 },
  "escape":    { key: "Escape",    code: "Escape",    windowsVirtualKeyCode: 27,  nativeVirtualKeyCode: 53 },
  "backspace": { key: "Backspace", code: "Backspace", windowsVirtualKeyCode: 8,   nativeVirtualKeyCode: 51 },
  "delete":    { key: "Delete",    code: "Delete",    windowsVirtualKeyCode: 46,  nativeVirtualKeyCode: 117 },
  "space":     { key: " ",         code: "Space",     windowsVirtualKeyCode: 32,  nativeVirtualKeyCode: 49 },
  "arrowup":   { key: "ArrowUp",   code: "ArrowUp",   windowsVirtualKeyCode: 38,  nativeVirtualKeyCode: 126 },
  "arrowdown": { key: "ArrowDown", code: "ArrowDown", windowsVirtualKeyCode: 40,  nativeVirtualKeyCode: 125 },
  "arrowleft": { key: "ArrowLeft", code: "ArrowLeft", windowsVirtualKeyCode: 37,  nativeVirtualKeyCode: 123 },
  "arrowright":{ key: "ArrowRight",code: "ArrowRight",windowsVirtualKeyCode: 39,  nativeVirtualKeyCode: 124 },
  "home":      { key: "Home",      code: "Home",      windowsVirtualKeyCode: 36,  nativeVirtualKeyCode: 115 },
  "end":       { key: "End",       code: "End",       windowsVirtualKeyCode: 35,  nativeVirtualKeyCode: 119 },
  "pageup":    { key: "PageUp",    code: "PageUp",    windowsVirtualKeyCode: 33,  nativeVirtualKeyCode: 116 },
  "pagedown":  { key: "PageDown",  code: "PageDown",  windowsVirtualKeyCode: 34,  nativeVirtualKeyCode: 121 },
  "shift":     { key: "Shift",     code: "ShiftLeft",  windowsVirtualKeyCode: 16,  nativeVirtualKeyCode: 56 },
  "control":   { key: "Control",   code: "ControlLeft",windowsVirtualKeyCode: 17,  nativeVirtualKeyCode: 59 },
  "alt":       { key: "Alt",       code: "AltLeft",    windowsVirtualKeyCode: 18,  nativeVirtualKeyCode: 58 },
  "meta":      { key: "Meta",      code: "MetaLeft",   windowsVirtualKeyCode: 91,  nativeVirtualKeyCode: 55 },
  "a": { key: "a", code: "KeyA", windowsVirtualKeyCode: 65, nativeVirtualKeyCode: 0 },
  "c": { key: "c", code: "KeyC", windowsVirtualKeyCode: 67, nativeVirtualKeyCode: 8 },
  "v": { key: "v", code: "KeyV", windowsVirtualKeyCode: 86, nativeVirtualKeyCode: 9 },
  "x": { key: "x", code: "KeyX", windowsVirtualKeyCode: 88, nativeVirtualKeyCode: 7 },
};

/**
 * Parse a human-readable key expression like "Enter" or "Control+A".
 */
export function parseKey(raw: string): ParsedKey[] {
  const parts = raw.split("+");
  if (parts.length === 1) {
    const entry = KEY_TABLE[raw.toLowerCase()];
    if (!entry) throw new Error(`Unknown key: "${raw}"`);
    return [
      {
        ...entry,
        text: entry.key.length === 1 ? entry.key : "",
        modifiers: Modifiers.None,
        type: "keyDown",
      },
      {
        ...entry,
        text: entry.key.length === 1 ? entry.key : "",
        modifiers: Modifiers.None,
        type: "keyUp",
      },
    ];
  }

  // Combo key: e.g. Control+A, Meta+C
  const modifiers = parts.slice(0, -1).map(p => p.trim());
  const finalKey = parts[parts.length - 1]!.trim();

  let modifierMask = Modifiers.None;
  for (const mod of modifiers) {
    const lower = mod.toLowerCase();
    if (lower === "control" || lower === "ctrl") modifierMask |= Modifiers.Control;
    else if (lower === "shift") modifierMask |= Modifiers.Shift;
    else if (lower === "alt" || lower === "option") modifierMask |= Modifiers.Alt;
    else if (lower === "meta" || lower === "command" || lower === "cmd") modifierMask |= Modifiers.Meta;
  }

  const entry = KEY_TABLE[finalKey.toLowerCase()];
  if (!entry) throw new Error(`Unknown key in combo: "${finalKey}"`);

  return [
    {
      ...entry,
      text: entry.key.length === 1 ? entry.key : "",
      modifiers: modifierMask,
      type: "keyDown",
    },
    {
      ...entry,
      text: entry.key.length === 1 ? entry.key : "",
      modifiers: modifierMask,
      type: "keyUp",
    },
  ];
}

/**
 * Characters that need a basic keyDown/keyUp without a modifier table entry.
 * For simple typing, use Input.insertText instead.
 */
export function charKeyDownUp(char: string): ParsedKey[] {
  const key = char.length === 1 ? char : char;
  return [
    {
      key,
      code: "",
      text: char,
      windowsVirtualKeyCode: char.charCodeAt(0),
      nativeVirtualKeyCode: 0,
      modifiers: Modifiers.None,
      type: "keyDown",
    },
    {
      key,
      code: "",
      text: char,
      windowsVirtualKeyCode: char.charCodeAt(0),
      nativeVirtualKeyCode: 0,
      modifiers: Modifiers.None,
      type: "keyUp",
    },
  ];
}
