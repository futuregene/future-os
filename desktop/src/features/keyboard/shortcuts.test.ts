// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";

const STORAGE_KEY = "future.shortcuts.v1";

/**
 * The store is a module singleton that reads localStorage once at import, so each
 * case imports a fresh copy (the same technique `i18n.test.ts` uses).
 */
async function freshStore() {
  vi.resetModules();
  return import("./shortcuts");
}

afterEach(() => {
  localStorage.removeItem(STORAGE_KEY);
});

describe("shortcut store", () => {
  it("starts from the catalog defaults", async () => {
    const store = await freshStore();
    expect(store.resolvedBindingText("chat.new")).toBe("mod+n");
    expect(store.bindingLabel("chat.new", undefined, false)).toBe("Ctrl+N");
    expect(store.bindingLabel("chat.new", undefined, true)).toBe("⌘N");
  });

  it("stores only differences, so an untouched command keeps following its default", async () => {
    const store = await freshStore();
    store.setCommandBinding("chat.new", "ctrl+shift+k");
    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY)!);
    expect(stored.overrides).toEqual({ "chat.new": "ctrl+shift+KeyK" });
    // Everything else stays out of the document.
    expect(Object.keys(stored.overrides)).toHaveLength(1);
  });

  it("drops an override that names the default again", async () => {
    const store = await freshStore();
    store.setCommandBinding("chat.new", "ctrl+shift+k");
    store.setCommandBinding("chat.new", "mod+n");
    // Nothing left to store means nothing stored: the command follows its
    // default again, including if that default changes in a later release.
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
    expect(store.resolvedBindingText("chat.new")).toBe("mod+n");
  });

  it("remembers an explicit unbinding", async () => {
    const store = await freshStore();
    store.setCommandBinding("chat.new", null);
    expect(store.resolvedBindingText("chat.new")).toBeNull();
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!).overrides).toEqual({ "chat.new": null });
  });

  it("keeps a device's key after the chord is rebound or cleared", async () => {
    // `vibe.menu` is the remote control's menu button plus a keyboard chord. The
    // button belongs to the device: rebinding the chord moves the chord only, and
    // clearing it leaves the button alone.
    const store = await freshStore();
    store.setCommandBinding("vibe.menu", "ctrl+shift+m");
    expect(store.resolvedBindingTexts("vibe.menu")).toEqual(["ctrl+shift+KeyM", "contextmenu"]);
    store.setCommandBinding("vibe.menu", null);
    expect(store.resolvedBindingTexts("vibe.menu")).toEqual(["contextmenu"]);
  });

  it("resets one command and all of them", async () => {
    const store = await freshStore();
    store.setCommandBinding("chat.new", "ctrl+shift+k");
    store.setCommandBinding("panel.left", "ctrl+alt+l");
    store.resetCommandBinding("chat.new");
    expect(store.resolvedBindingText("chat.new")).toBe("mod+n");
    expect(store.resolvedBindingText("panel.left")).toBe("ctrl+alt+KeyL");
    store.resetAllCommandBindings();
    expect(store.resolvedBindingText("panel.left")).toBe("mod+d");
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it("finds the command a binding would collide with", async () => {
    const store = await freshStore();
    expect(store.conflictingCommand("mod+f", "chat.new")?.id).toBe("thread.find");
    // A command does not conflict with itself.
    expect(store.conflictingCommand("mod+f", "thread.find")).toBeNull();
    expect(store.conflictingCommand("ctrl+alt+9", "chat.new")).toBeNull();
  });

  it("keeps the accelerator portable when the same text is read on another platform", async () => {
    const store = await freshStore();
    // What the recorder stores for a press of the platform accelerator: `mod`, so
    // the same choice is ⌘⇧K on macOS and Ctrl+Shift+K elsewhere.
    store.setCommandBinding("chat.new", "mod+shift+k");
    expect(store.bindingLabel("chat.new", undefined, true)).toBe("⇧⌘K");
    expect(store.bindingLabel("chat.new", undefined, false)).toBe("Ctrl+Shift+K");
  });

  it("keeps a literal ⌃ distinct from the accelerator", async () => {
    const store = await freshStore();
    // On macOS the recorder stores a pressed ⌃ as `ctrl`, not as `mod`.
    store.setCommandBinding("chat.new", "ctrl+k");
    expect(store.bindingLabel("chat.new", undefined, true)).toBe("⌃K");
    expect(store.bindingLabel("chat.new", undefined, false)).toBe("Ctrl+K");
  });

  it("drops a stored value it cannot read instead of guessing", async () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      version: 1,
      overrides: { "chat.new": "not a binding", "gone.command": "mod+k", "panel.left": 42 },
    }));
    const store = await freshStore();
    expect(store.resolvedBindingText("chat.new")).toBe("mod+n");
    expect(store.resolvedBindingText("panel.left")).toBe("mod+d");
    // Nothing was written back: the unreadable document is simply not consulted.
    expect(store.shortcutOverrides()).toEqual({});
  });

  it("survives storage that is not JSON at all", async () => {
    localStorage.setItem(STORAGE_KEY, "{oops");
    const store = await freshStore();
    expect(store.resolvedBindingText("chat.new")).toBe("mod+n");
  });

  it("notifies subscribers on every write", async () => {
    const store = await freshStore();
    const listener = vi.fn();
    const unsubscribe = store.subscribeShortcuts(listener);
    store.setCommandBinding("chat.new", "ctrl+shift+k");
    store.resetAllCommandBindings();
    expect(listener).toHaveBeenCalledTimes(2);
    unsubscribe();
    store.setCommandBinding("chat.new", "ctrl+shift+k");
    expect(listener).toHaveBeenCalledTimes(2);
  });
});
