// @vitest-environment jsdom
import type { CommandId } from "./catalog";
import type { DispatchContext, ShortcutEvent } from "./dispatch";
import { describe, expect, it, vi } from "vitest";
import { parseBinding, serializeBinding } from "../../lib/keybinding";
import { dispatchShortcut } from "./dispatch";

/**
 * A context built from binding *text* rather than parsed bindings, so a case can
 * say `{ "chat.new": "mod+n" }` and keep reading like the catalog it mirrors.
 */
function context(overrides: Omit<Partial<DispatchContext>, "bindings"> & { bindings?: Record<string, string | null> } = {}): DispatchContext {
  const { bindings, ...rest } = overrides;
  const resolved = new Map<CommandId, ReturnType<typeof parseBinding>>();
  const source = bindings ?? {};
  for (const [id, text] of Object.entries(source))
    resolved.set(id as CommandId, text === null ? null : parseBinding(text));
  return {
    isMac: false,
    bindings: resolved,
    hasHandler: () => true,
    hasOpenOverlay: () => false,
    hasKeyboardOwner: () => false,
    inTerminal: false,
    ...rest,
  };
}

function event(key: string, overrides: Partial<ShortcutEvent> = {}): ShortcutEvent {
  return {
    altKey: false,
    code: "",
    ctrlKey: false,
    key,
    metaKey: false,
    repeat: false,
    shiftKey: false,
    ...overrides,
  };
}

describe("dispatchShortcut", () => {
  it("runs the command whose binding matches", () => {
    expect(dispatchShortcut(event("n", { code: "KeyN", ctrlKey: true }), context({ bindings: { "chat.new": "mod+n" } }))).toBe("chat.new");
  });

  it("leaves a key nobody bound alone", () => {
    expect(dispatchShortcut(event("q", { code: "KeyQ", ctrlKey: true }), context({ bindings: { "chat.new": "mod+n" } }))).toBeNull();
  });

  it("leaves a bound key alone while nothing owns the command", () => {
    // No conversation open means no find bar, and `⌘F` must stay the platform's
    // own rather than being swallowed for nothing.
    const id = dispatchShortcut(
      event("f", { code: "KeyF", ctrlKey: true }),
      context({ bindings: { "thread.find": "mod+f" }, hasHandler: () => false }),
    );
    expect(id).toBeNull();
  });

  it("stands down while an IME composes", () => {
    const bindings = { "chat.new": "mod+n" };
    expect(dispatchShortcut(event("n", { code: "KeyN", ctrlKey: true, isComposing: true }), context({ bindings }))).toBeNull();
    expect(dispatchShortcut(event("n", { code: "KeyN", ctrlKey: true, keyCode: 229 }), context({ bindings }))).toBeNull();
    // WebKitGTK reports the key an input method consumed as "Unidentified".
    expect(dispatchShortcut(event("Unidentified", { code: "", ctrlKey: true }), context({ bindings }))).toBeNull();
  });

  it("treats AltGr as typing, not as Ctrl+Alt", () => {
    const id = dispatchShortcut(
      event("7", { code: "Digit7", ctrlKey: true, altKey: true, getModifierState: key => key === "AltGraph" }),
      context({ bindings: { "chat.new": "ctrl+alt+7" } }),
    );
    expect(id).toBeNull();
  });

  it("stands down behind a modal or a floating menu", () => {
    const bindings = { "chat.new": "mod+n" };
    const press = () => event("n", { code: "KeyN", ctrlKey: true });
    expect(dispatchShortcut(press(), context({ bindings, hasOpenOverlay: () => true }))).toBeNull();
    expect(dispatchShortcut(press(), context({ bindings, hasKeyboardOwner: () => true }))).toBeNull();
  });

  it("repeats only the commands that walk a list", () => {
    const bindings = { "chat.new": "mod+n", "thread.next": "mod+shift+bracketright" };
    const held = event("n", { code: "KeyN", ctrlKey: true, repeat: true });
    expect(dispatchShortcut(held, context({ bindings }))).toBeNull();
    const walk = event("]", { code: "BracketRight", ctrlKey: true, repeat: true, shiftKey: true });
    expect(dispatchShortcut(walk, context({ bindings }))).toBe("thread.next");
  });

  it("leaves the shell's own keys alone inside the terminal", () => {
    const bindings = { "panel.left": "mod+b", "terminal.toggle": "mod+j" };
    const pressCtrlB = event("b", { code: "KeyB", ctrlKey: true });
    // Outside the terminal the app owns Ctrl+B …
    expect(dispatchShortcut(pressCtrlB, context({ bindings }))).toBe("panel.left");
    // … inside it, `Ctrl+B` is readline's "back one character".
    expect(dispatchShortcut(pressCtrlB, context({ bindings, inTerminal: true }))).toBeNull();
    // The terminal's own toggle is the one command that may pass.
    expect(dispatchShortcut(event("j", { code: "KeyJ", ctrlKey: true }), context({ bindings, inTerminal: true }))).toBe("terminal.toggle");
  });

  it("respects a platform's accelerator", () => {
    const bindings = { "chat.new": "mod+n" };
    // ⌘N on macOS, Ctrl+N elsewhere; neither is the other's key.
    expect(dispatchShortcut(event("n", { code: "KeyN", metaKey: true }), context({ bindings, isMac: true }))).toBe("chat.new");
    expect(dispatchShortcut(event("n", { code: "KeyN", ctrlKey: true }), context({ bindings, isMac: true }))).toBeNull();
    expect(dispatchShortcut(event("n", { code: "KeyN", metaKey: true }), context({ bindings, isMac: false }))).toBeNull();
  });

  it("leaves an event an earlier handler consumed", () => {
    const id = dispatchShortcut(
      event("n", { code: "KeyN", ctrlKey: true, defaultPrevented: true }),
      context({ bindings: { "chat.new": "mod+n" } }),
    );
    expect(id).toBeNull();
  });

  it("runs a rebinding", () => {
    const bindings = { "chat.new": "ctrl+shift+k" };
    expect(serializeBinding(parseBinding(bindings["chat.new"])!)).toBe("ctrl+shift+KeyK");
    expect(dispatchShortcut(event("k", { code: "KeyK", ctrlKey: true, shiftKey: true }), context({ bindings }))).toBe("chat.new");
    // The old default is free again.
    expect(dispatchShortcut(event("n", { code: "KeyN", ctrlKey: true }), context({ bindings }))).toBeNull();
  });

  it("runs nothing for a command the user unbound", () => {
    const bindings = { "chat.new": null };
    expect(dispatchShortcut(event("n", { code: "KeyN", ctrlKey: true }), context({ bindings }))).toBeNull();
  });

  it("never runs a command without a matching binding", () => {
    const hasHandler = vi.fn(() => true);
    dispatchShortcut(event("x", { code: "KeyX" }), context({ bindings: { "chat.new": "mod+n" }, hasHandler }));
    expect(hasHandler).not.toHaveBeenCalled();
  });
});
