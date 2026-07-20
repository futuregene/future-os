/**
 * Unit tests for keyboard input parsing (Kitty CSI-u, xterm modifyOtherKeys,
 * legacy sequences). The mapping from raw bytes to key names is the contract
 * every keybinding depends on — regressions here break the whole TUI.
 *
 * Kitty modifier encoding: the parameter is 1 + modifier bits
 * (shift=1, alt=2, ctrl=4), so e.g. ";5" means ctrl.
 */
import { describe, test, expect, beforeEach } from "bun:test";
import {
  parseKey,
  isKeyRelease,
  isKeyRepeat,
  decodeKittyPrintable,
  setKittyProtocolActive,
} from "../keys.js";

// Tests run with Kitty protocol OFF unless a test enables it explicitly.
beforeEach(() => {
  setKittyProtocolActive(false);
});

// ─── Legacy sequences ──────────────────────────────────────────────────────

describe("parseKey: legacy", () => {
  test("single printable chars pass through", () => {
    expect(parseKey("a")).toBe("a");
    expect(parseKey("Z")).toBe("Z");
    expect(parseKey("5")).toBe("5");
  });

  test("control bytes map to ctrl+letter", () => {
    expect(parseKey("\x01")).toBe("ctrl+a");
    expect(parseKey("\x03")).toBe("ctrl+c");
    expect(parseKey("\x1a")).toBe("ctrl+z");
  });

  test("named keys", () => {
    expect(parseKey("\x1b")).toBe("escape");
    expect(parseKey("\t")).toBe("tab");
    expect(parseKey("\r")).toBe("enter");
    expect(parseKey(" ")).toBe("space");
    expect(parseKey("\x7f")).toBe("backspace");
  });

  test("arrow and function keys via CSI/SS3", () => {
    expect(parseKey("\x1b[A")).toBe("up");
    expect(parseKey("\x1b[B")).toBe("down");
    expect(parseKey("\x1b[C")).toBe("right");
    expect(parseKey("\x1b[D")).toBe("left");
    expect(parseKey("\x1bOA")).toBe("up");
    expect(parseKey("\x1bOP")).toBe("f1");
    expect(parseKey("\x1b[15~")).toBe("f5");
  });

  test("modified legacy sequences", () => {
    expect(parseKey("\x1b[Z")).toBe("shift+tab");
    expect(parseKey("\x1b[3~")).toBe("delete");
    expect(parseKey("\x1b[3^")).toBe("ctrl+delete");
    expect(parseKey("\x1b[2~")).toBe("insert");
  });

  test("alt+letter is ESC + letter (kitty off)", () => {
    expect(parseKey("\x1ba")).toBe("alt+a");
    expect(parseKey("\x1b5")).toBe("alt+5");
  });

  test("unrecognized input returns undefined", () => {
    expect(parseKey("\x1b[999~")).toBeUndefined();
  });
});

// ─── Kitty CSI-u ───────────────────────────────────────────────────────────

describe("parseKey: kitty CSI-u", () => {
  test("plain keypress", () => {
    expect(parseKey("\x1b[97u")).toBe("a"); // 'a'
    expect(parseKey("\x1b[13u")).toBe("enter");
    expect(parseKey("\x1b[27u")).toBe("escape");
  });

  test("modifiers", () => {
    expect(parseKey("\x1b[97;5u")).toBe("ctrl+a"); // 5 = 1+ctrl
    expect(parseKey("\x1b[97;2u")).toBe("shift+a"); // 2 = 1+shift
    expect(parseKey("\x1b[97;3u")).toBe("alt+a"); // 3 = 1+alt
    expect(parseKey("\x1b[99;5u")).toBe("ctrl+c");
  });

  test("shift+ctrl combines in order", () => {
    expect(parseKey("\x1b[97;6u")).toBe("shift+ctrl+a"); // 6 = 1+shift+ctrl
  });

  test("functional arrows and navigation keys", () => {
    expect(parseKey("\x1b[57419u")).toBe("up");
    expect(parseKey("\x1b[57420u")).toBe("down");
    expect(parseKey("\x1b[1;5A")).toBe("ctrl+up"); // arrow-with-modifier form
    expect(parseKey("\x1b[1;2H")).toBe("shift+home");
  });

  test("keypad equivalents normalize to base keys", () => {
    // Keypad digits 57399..57408 map back to '0'..'9'.
    expect(parseKey("\x1b[57400u")).toBe("1");
    expect(parseKey("\x1b[57414u")).toBe("enter"); // keypad enter
  });
});

// ─── Event type detection ──────────────────────────────────────────────────

describe("isKeyRelease / isKeyRepeat", () => {
  test("release event (:3 suffix)", () => {
    expect(isKeyRelease("\x1b[97;1:3u")).toBe(true);
    expect(isKeyRelease("\x1b[1;1:3A")).toBe(true);
    expect(isKeyRelease("\x1b[97u")).toBe(false);
  });

  test("repeat event (:2 suffix)", () => {
    expect(isKeyRepeat("\x1b[97;1:2u")).toBe(true);
    expect(isKeyRepeat("\x1b[97u")).toBe(false);
  });

  test("bracketed paste markers are never release/repeat", () => {
    expect(isKeyRelease("\x1b[200~some;1:3u\x1b[201~")).toBe(false);
    expect(isKeyRepeat("\x1b[200~some;1:2u\x1b[201~")).toBe(false);
  });
});

// ─── decodeKittyPrintable ──────────────────────────────────────────────────

describe("decodeKittyPrintable", () => {
  test("decodes plain keypress to its character", () => {
    expect(decodeKittyPrintable("\x1b[97u")).toBe("a");
  });

  test("rejects non-kitty input", () => {
    expect(decodeKittyPrintable("a")).toBeUndefined();
    expect(decodeKittyPrintable("\x1b[A")).toBeUndefined();
  });
});

// ─── Mode-dependent sequences ──────────────────────────────────────────────

describe("kitty-mode-dependent parsing", () => {
  test("ESC+CR is shift+enter under kitty, alt+enter in legacy mode", () => {
    setKittyProtocolActive(true);
    expect(parseKey("\x1b\r")).toBe("shift+enter");

    setKittyProtocolActive(false);
    expect(parseKey("\x1b\r")).toBe("alt+enter");
  });

  test("alt+letter only exists in legacy mode", () => {
    setKittyProtocolActive(false);
    expect(parseKey("\x1bx")).toBe("alt+x");

    // Under kitty, bare ESC+letter is not parsed as alt (kitty sends CSI-u).
    setKittyProtocolActive(true);
    expect(parseKey("\x1bx")).toBeUndefined();
  });
});
