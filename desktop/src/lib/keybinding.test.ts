import type { BindingKeyEvent, KeyBinding } from "./keybinding";
import { describe, expect, it } from "vitest";
import {
  bindingFromEvent,
  bindingProblem,
  eventCode,
  formatBinding,
  isModifierKey,
  keyLabel,
  matchesBinding,
  parseBinding,
  reservedBindingLabels,
  sameBinding,
  serializeBinding,
} from "./keybinding";

function key(overrides: Partial<BindingKeyEvent> = {}): BindingKeyEvent {
  return { altKey: false, code: "", ctrlKey: false, key: "", metaKey: false, shiftKey: false, ...overrides };
}

function binding(text: string): KeyBinding {
  const parsed = parseBinding(text);
  expect(parsed, text).not.toBeNull();
  return parsed!;
}

describe("parseBinding", () => {
  it("reads letters, digits and named keys", () => {
    expect(binding("mod+n")).toEqual({ alt: false, code: "KeyN", ctrl: false, mod: true, shift: false });
    expect(binding("mod+1").code).toBe("Digit1");
    expect(binding("F5").code).toBe("F5");
    expect(binding("pageup").code).toBe("PageUp");
    expect(binding("ArrowUp").code).toBe("ArrowUp");
  });

  it("reads punctuation as the physical key", () => {
    // Both spellings describe the same key, which is what makes a stored value
    // and a hand-written one interchangeable.
    expect(binding("mod+.").code).toBe("Period");
    expect(binding("mod+period").code).toBe("Period");
    expect(binding("mod+[").code).toBe("BracketLeft");
    expect(binding("mod+bracketleft").code).toBe("BracketLeft");
  });

  it("accepts the aliases people write", () => {
    expect(binding("cmd+k")).toEqual(binding("mod+k"));
    expect(binding("ctrl+k")).toEqual(binding("control+k"));
    expect(binding("opt+k")).toEqual(binding("alt+k"));
  });

  it("reads a remote control's keys", () => {
    // A TV-style remote's back / menu / home buttons are ordinary DOM codes; the
    // panel binds them, so the parser has to know them by name.
    expect(binding("contextmenu").code).toBe("ContextMenu");
    // Some remotes' menu button reports `key: "Menu"` with no usable code.
    expect(binding("menu").code).toBe("ContextMenu");
    expect(binding("browserback").code).toBe("BrowserBack");
    expect(binding("goback").code).toBe("BrowserBack");
    expect(binding("browserhome").code).toBe("BrowserHome");
  });

  it("labels a remote's keys by the word printed on them", () => {
    // Not a glyph: no platform shares one for these buttons.
    expect(formatBinding(binding("contextmenu"), true)).toBe("Menu");
    expect(formatBinding(binding("browserback"), false)).toBe("Back");
    expect(formatBinding(binding("browserhome"), true)).toBe("Home");
  });

  it("refuses text that is not one key with modifiers", () => {
    expect(parseBinding("")).toBeNull();
    expect(parseBinding("mod")).toBeNull();
    expect(parseBinding("mod+n+k")).toBeNull();
    expect(parseBinding("mod+k+shift")).toBeNull();
    expect(parseBinding("mod+nope")).toBeNull();
    expect(parseBinding("mod+F99")).toBeNull();
  });
});

describe("serializeBinding", () => {
  it("writes modifiers in one fixed order", () => {
    const written = serializeBinding(binding("shift+alt+ctrl+mod+k"));
    expect(written).toBe("mod+ctrl+alt+shift+KeyK");
    // Round trip: what is stored parses back to itself.
    expect(serializeBinding(binding(written))).toBe(written);
  });

  it("compares combinations by their canonical text", () => {
    expect(sameBinding(binding("cmd+K"), binding("mod+k"))).toBe(true);
    expect(sameBinding(binding("mod+k"), binding("mod+shift+k"))).toBe(false);
  });
});

describe("matchesBinding", () => {
  const bindingText = binding("mod+shift+f");

  it("treats mod as Ctrl on Windows and Linux", () => {
    expect(matchesBinding(bindingText, key({ code: "KeyF", ctrlKey: true, shiftKey: true }), false)).toBe(true);
    // ⌘ is not the accelerator there, so it is a different (unbound) combination.
    expect(matchesBinding(bindingText, key({ code: "KeyF", metaKey: true, shiftKey: true }), false)).toBe(false);
  });

  it("treats mod as ⌘ on macOS", () => {
    expect(matchesBinding(bindingText, key({ code: "KeyF", metaKey: true, shiftKey: true }), true)).toBe(true);
    expect(matchesBinding(bindingText, key({ code: "KeyF", ctrlKey: true, shiftKey: true }), true)).toBe(false);
  });

  it("requires the modifier set to match exactly", () => {
    const plain = binding("mod+k");
    expect(matchesBinding(plain, key({ code: "KeyK", ctrlKey: true }), false)).toBe(true);
    expect(matchesBinding(plain, key({ code: "KeyK", ctrlKey: true, shiftKey: true }), false)).toBe(false);
    expect(matchesBinding(plain, key({ code: "KeyK", ctrlKey: true, altKey: true }), false)).toBe(false);
    // A bare key is never a shortcut.
    expect(matchesBinding(plain, key({ code: "KeyK" }), false)).toBe(false);
  });

  it("matches the physical key, not the character the layout types", () => {
    // AZERTY: the same key produces "," without Shift and "?" with it. A binding
    // on the key is a binding on the position.
    const comma = binding("mod+,");
    expect(matchesBinding(comma, key({ code: "Comma", ctrlKey: true, key: "," }), false)).toBe(true);
    expect(matchesBinding(comma, key({ code: "Comma", ctrlKey: true, key: "?" }), false)).toBe(true);
  });

  it("falls back to the reported character when the engine sends no code", () => {
    expect(matchesBinding(binding("mod+f"), key({ ctrlKey: true, key: "F" }), false)).toBe(true);
    expect(matchesBinding(binding("mod+."), key({ ctrlKey: true, key: "." }), false)).toBe(true);
    expect(matchesBinding(binding("mod+."), key({ ctrlKey: true, key: "?" }), false)).toBe(false);
  });
});

describe("eventCode", () => {
  it("prefers the physical code", () => {
    expect(eventCode(key({ code: "KeyN", key: "z" }))).toBe("KeyN");
  });

  it("derives one from the character when there is none", () => {
    expect(eventCode(key({ key: "n" }))).toBe("KeyN");
    expect(eventCode(key({ key: "7" }))).toBe("Digit7");
    expect(eventCode(key({ key: "Enter" }))).toBe("Enter");
    expect(eventCode(key({ key: "Unidentified" }))).toBeNull();
  });

  it("reads a code the event spells out in `key`", () => {
    // Some engines send the physical code as the key (a synthetic event, or a
    // WebKitGTK build with no `code`). `digit3` and `Digit3` both name `Digit3`.
    expect(eventCode(key({ key: "Digit3" }))).toBe("Digit3");
    expect(eventCode(key({ key: "digit3" }))).toBe("Digit3");
  });
});

describe("formatBinding", () => {
  it("prints macOS in Apple's order and symbols", () => {
    expect(formatBinding(binding("shift+alt+mod+k"), true)).toBe("⌥⇧⌘K");
    expect(formatBinding(binding("ctrl+mod+a"), true)).toBe("⌃⌘A");
  });

  it("prints Windows and Linux with names", () => {
    expect(formatBinding(binding("mod+shift+f"), false)).toBe("Ctrl+Shift+F");
    expect(formatBinding(binding("mod+,"), false)).toBe("Ctrl+,");
  });

  it("labels named keys per platform", () => {
    expect(keyLabel("PageUp", true)).toBe("⇞");
    expect(keyLabel("PageUp", false)).toBe("PgUp");
    expect(keyLabel("Enter", true)).toBe("↩");
    expect(keyLabel("Enter", false)).toBe("Enter");
  });
});

describe("bindingProblem", () => {
  it("requires a real modifier", () => {
    expect(bindingProblem(binding("shift+k"), false)).toBe("needsModifier");
    expect(bindingProblem(binding("mod+k"), false)).toBeNull();
    expect(bindingProblem(binding("alt+k"), false)).toBeNull();
    // On macOS a literal ⌃ is a modifier of its own.
    expect(bindingProblem(binding("ctrl+k"), true)).toBeNull();
  });

  it("refuses what the platform and its text editing own", () => {
    expect(bindingProblem(binding("mod+c"), false)).toBe("reserved");
    expect(bindingProblem(binding("mod+v"), false)).toBe("reserved");
    expect(bindingProblem(binding("mod+shift+z"), false)).toBe("reserved");
    expect(bindingProblem(binding("mod+r"), false)).toBe("reserved");
    expect(bindingProblem(binding("alt+f4"), false)).toBe("reserved");
    // Quit/hide/minimise are macOS conventions, so they are free elsewhere.
    expect(bindingProblem(binding("mod+q"), true)).toBe("reserved");
    expect(bindingProblem(binding("mod+q"), false)).toBeNull();
  });

  it("lists the refusals in the platform's notation", () => {
    expect(reservedBindingLabels(false)).toContain("Ctrl+C");
    expect(reservedBindingLabels(true)).toContain("⌘Q");
    expect(reservedBindingLabels(false)).not.toContain("⌘Q");
  });
});

describe("isModifierKey", () => {
  it("recognises a modifier on its own", () => {
    for (const name of ["Shift", "Control", "Alt", "Meta", "AltGraph", "CapsLock"])
      expect(isModifierKey({ key: name })).toBe(true);
    expect(isModifierKey({ key: "k" })).toBe(false);
  });
});

describe("bindingFromEvent", () => {
  it("records the platform accelerator as mod", () => {
    expect(serializeBinding(bindingFromEvent(key({ code: "KeyK", ctrlKey: true }), false)!)).toBe("mod+KeyK");
    expect(serializeBinding(bindingFromEvent(key({ code: "KeyK", metaKey: true }), true)!)).toBe("mod+KeyK");
  });

  it("keeps macOS ⌃ distinct from ⌘", () => {
    expect(serializeBinding(bindingFromEvent(key({ code: "KeyK", ctrlKey: true }), true)!)).toBe("ctrl+KeyK");
  });

  it("waits for a real key", () => {
    expect(bindingFromEvent(key({ key: "Shift", shiftKey: true }), false)).toBeNull();
  });

  it("waits when the event names no key at all", () => {
    // A dead key or an unidentified press: there is nothing to record yet, and
    // guessing a code would store a binding the user never pressed.
    expect(bindingFromEvent(key({ key: "Unidentified", ctrlKey: true }), false)).toBeNull();
    expect(bindingFromEvent(key({ ctrlKey: true }), false)).toBeNull();
  });
});
