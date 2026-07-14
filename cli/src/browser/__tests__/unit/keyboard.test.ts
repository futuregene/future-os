/**
 * Unit tests for keyboard parser.
 */
import { describe, test, expect } from "bun:test";
import { parseKey, Modifiers } from "../../input/keyboard.js";

describe("parseKey", () => {
  test("simple key returns keyDown + keyUp", () => {
    const keys = parseKey("Enter");
    expect(keys.length).toBe(2);
    expect(keys[0]!.type).toBe("keyDown");
    expect(keys[0]!.key).toBe("Enter");
    expect(keys[0]!.code).toBe("Enter");
    expect(keys[0]!.modifiers).toBe(Modifiers.None);
    expect(keys[1]!.type).toBe("keyUp");
  });

  test("Tab produces correct key/up pair", () => {
    const keys = parseKey("Tab");
    expect(keys[0]!.key).toBe("Tab");
    expect(keys[0]!.windowsVirtualKeyCode).toBe(9);
  });

  test("Escape produces correct key/up pair", () => {
    const keys = parseKey("Escape");
    expect(keys[0]!.key).toBe("Escape");
  });

  test("Backspace produces correct key/up pair", () => {
    const keys = parseKey("Backspace");
    expect(keys[0]!.key).toBe("Backspace");
  });

  test("Space produces space character", () => {
    const keys = parseKey("Space");
    expect(keys[0]!.key).toBe(" ");
  });

  test("Arrow keys work", () => {
    for (const key of ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"]) {
      const keys = parseKey(key);
      expect(keys[0]!.key).toBe(key);
      expect(keys[0]!.code).toBe(key);
    }
  });

  test("Control+A modifier combination", () => {
    const keys = parseKey("Control+A");
    expect(keys[0]!.modifiers).toBe(Modifiers.Control);
    expect(keys[0]!.key).toBe("a");
    expect(keys[0]!.code).toBe("KeyA");
  });

  test("Meta+C modifier combination", () => {
    const keys = parseKey("Meta+C");
    expect(keys[0]!.modifiers).toBe(Modifiers.Meta);
  });

  test("Control+Shift+Tab combo", () => {
    const keys = parseKey("Control+Shift+Tab");
    expect(keys[0]!.modifiers).toBe(Modifiers.Control | Modifiers.Shift);
    expect(keys[0]!.key).toBe("Tab");
  });

  test("unknown key throws", () => {
    expect(() => parseKey("F23")).toThrow("Unknown key");
  });

  test("Home/End/PageUp/PageDown work", () => {
    for (const key of ["Home", "End", "PageUp", "PageDown"]) {
      expect(() => parseKey(key)).not.toThrow();
    }
  });
});
