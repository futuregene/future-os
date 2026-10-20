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

describe("the panel shortcut is a rebindable command", () => {
  // The binding lives in the command table now (`features/keyboard/catalog`), so
  // these cases read it through the store the terminal itself reads.
  it("follows a user's rebinding on the next press", async () => {
    const { setCommandBinding, resetAllCommandBindings } = await import("../keyboard/shortcuts");
    resetAllCommandBindings();
    setCommandBinding("terminal.toggle", "mod+shift+j");
    try {
      expect(isPanelToggleShortcut(key({ ctrlKey: true, shiftKey: true }), false)).toBe(true);
      // The default is free while the command is bound elsewhere.
      expect(isPanelToggleShortcut(key({ ctrlKey: true }), false)).toBe(false);
      expect(panelToggleShortcutLabel(false)).toBe("Ctrl+Shift+J");
      expect(panelToggleShortcutLabel(true)).toBe("⇧⌘J");
    }
    finally {
      resetAllCommandBindings();
    }
  });

  it("says nothing when the user removed the binding", async () => {
    const { setCommandBinding, resetAllCommandBindings } = await import("../keyboard/shortcuts");
    resetAllCommandBindings();
    setCommandBinding("terminal.toggle", null);
    try {
      expect(isPanelToggleShortcut(key({ ctrlKey: true }), false)).toBe(false);
      expect(panelToggleShortcutLabel(false)).toBe("");
    }
    finally {
      resetAllCommandBindings();
    }
  });
});
