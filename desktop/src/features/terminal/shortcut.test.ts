import type { ShortcutKey } from "./shortcut";
import { describe, expect, it } from "vitest";
import { isPanelToggleShortcut, panelToggleShortcutLabel } from "./shortcut";

function key(overrides: Partial<ShortcutKey> = {}): ShortcutKey {
  return { key: "j", ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...overrides };
}

describe("isPanelToggleShortcut", () => {
  it("is Ctrl+J on Windows and Linux", () => {
    expect(isPanelToggleShortcut(key({ ctrlKey: true }), false)).toBe(true);
    // The macOS convention is not also bound: one platform, one shortcut.
    expect(isPanelToggleShortcut(key({ metaKey: true }), false)).toBe(false);
  });

  it("is Cmd+J on macOS", () => {
    expect(isPanelToggleShortcut(key({ metaKey: true }), true)).toBe(true);
    expect(isPanelToggleShortcut(key({ ctrlKey: true }), true)).toBe(false);
  });

  it("ignores extra modifiers", () => {
    expect(isPanelToggleShortcut(key({ ctrlKey: true, shiftKey: true }), false)).toBe(false);
    expect(isPanelToggleShortcut(key({ ctrlKey: true, altKey: true }), false)).toBe(false);
  });

  it("leaves every other key to the terminal", () => {
    // A bare "j" is a keystroke, not a command.
    expect(isPanelToggleShortcut(key(), false)).toBe(false);
    expect(isPanelToggleShortcut(key({ key: "k", ctrlKey: true }), false)).toBe(false);
    // Ctrl+C must keep reaching the shell.
    expect(isPanelToggleShortcut(key({ key: "c", ctrlKey: true }), false)).toBe(false);
  });

  it("matches the letter regardless of case", () => {
    expect(isPanelToggleShortcut(key({ key: "J", ctrlKey: true }), false)).toBe(true);
  });
});

describe("panelToggleShortcutLabel", () => {
  it("uses the platform's notation", () => {
    expect(panelToggleShortcutLabel(false)).toBe("Ctrl+J");
    expect(panelToggleShortcutLabel(true)).toBe("⌘J");
  });
});
