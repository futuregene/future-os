// @vitest-environment jsdom
import type { CommandId } from "./catalog";
import type { DispatchContext, ShortcutEvent } from "./dispatch";
import type { ShortcutScope } from "./scopes";
import { describe, expect, it, vi } from "vitest";
import { parseBinding } from "../../lib/keybinding";
import { dispatchShortcut, isBareBinding } from "./dispatch";

/**
 * A context built from binding *text* rather than parsed bindings, so a case can
 * say `{ "chat.new": "mod+n" }` and keep reading like the catalog it mirrors.
 */
function context(
  overrides: Omit<Partial<DispatchContext>, "bindings"> & {
    bindings?: Record<string, string | readonly string[] | null>;
    scopes?: readonly ShortcutScope[];
  } = {},
): DispatchContext {
  const { bindings, ...rest } = overrides;
  const resolved = new Map<CommandId, ReturnType<typeof parseBinding>[]>();
  for (const [id, text] of Object.entries(bindings ?? {})) {
    const texts = text === null ? [] : typeof text === "string" ? [text] : text;
    resolved.set(id as CommandId, texts.map(one => parseBinding(one)!).filter(Boolean));
  }
  return {
    bindings: resolved as DispatchContext["bindings"],
    editable: false,
    hasHandler: () => true,
    hasKeyboardOwner: () => false,
    hasOpenOverlay: () => false,
    inTerminal: false,
    isMac: false,
    scopes: ["global"],
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
    expect(dispatchShortcut(event("n", { code: "KeyN", ctrlKey: true }), context({ bindings: { "chat.new": "mod+n" } }))?.id).toBe("chat.new");
  });

  it("leaves a key nobody bound alone", () => {
    expect(dispatchShortcut(event("q", { code: "KeyQ", ctrlKey: true }), context({ bindings: { "chat.new": "mod+n" } }))).toBeNull();
  });

  it("leaves a bound key alone while nothing owns the command", () => {
    // No conversation open means no find bar, and `⌘F` must stay the platform's
    // own rather than being swallowed for nothing.
    const hit = dispatchShortcut(
      event("f", { code: "KeyF", ctrlKey: true }),
      context({ bindings: { "thread.find": "mod+f" }, hasHandler: () => false }),
    );
    expect(hit).toBeNull();
  });

  it("stands down while an IME composes", () => {
    const bindings = { "chat.new": "mod+n" };
    expect(dispatchShortcut(event("n", { code: "KeyN", ctrlKey: true, isComposing: true }), context({ bindings }))).toBeNull();
    expect(dispatchShortcut(event("n", { code: "KeyN", ctrlKey: true, keyCode: 229 }), context({ bindings }))).toBeNull();
    // WebKitGTK reports the key an input method consumed as "Unidentified".
    expect(dispatchShortcut(event("Unidentified", { code: "", ctrlKey: true }), context({ bindings }))).toBeNull();
  });

  it("treats AltGr as typing, not as Ctrl+Alt", () => {
    const hit = dispatchShortcut(
      event("7", { code: "Digit7", ctrlKey: true, altKey: true, getModifierState: key => key === "AltGraph" }),
      context({ bindings: { "chat.new": "ctrl+alt+7" } }),
    );
    expect(hit).toBeNull();
  });

  it("stands down behind a modal or a floating menu", () => {
    const bindings = { "chat.new": "mod+n" };
    const press = () => event("n", { code: "KeyN", ctrlKey: true });
    expect(dispatchShortcut(press(), context({ bindings, hasOpenOverlay: () => true }))).toBeNull();
    expect(dispatchShortcut(press(), context({ bindings, hasKeyboardOwner: () => true }))).toBeNull();
  });

  it("repeats only the commands that walk a list", () => {
    const bindings = { "chat.new": "mod+n", "thread.next": "mod+bracketright" };
    const held = event("n", { code: "KeyN", ctrlKey: true, repeat: true });
    expect(dispatchShortcut(held, context({ bindings }))).toBeNull();
    const walk = event("]", { code: "BracketRight", ctrlKey: true, repeat: true });
    expect(dispatchShortcut(walk, context({ bindings }))?.id).toBe("thread.next");
  });

  it("leaves the shell's own keys alone inside the terminal", () => {
    const bindings = { "panel.left": "mod+b", "terminal.toggle": "mod+j" };
    const pressCtrlB = event("b", { code: "KeyB", ctrlKey: true });
    // Outside the terminal the app owns Ctrl+B …
    expect(dispatchShortcut(pressCtrlB, context({ bindings }))?.id).toBe("panel.left");
    // … inside it, `Ctrl+B` is readline's "back one character".
    expect(dispatchShortcut(pressCtrlB, context({ bindings, inTerminal: true }))).toBeNull();
    // The terminal's own toggle is the one command that may pass.
    expect(dispatchShortcut(event("j", { code: "KeyJ", ctrlKey: true }), context({ bindings, inTerminal: true }))?.id).toBe("terminal.toggle");
  });

  it("respects a platform's accelerator", () => {
    const bindings = { "chat.new": "mod+n" };
    // ⌘N on macOS, Ctrl+N elsewhere; neither is the other's key.
    expect(dispatchShortcut(event("n", { code: "KeyN", metaKey: true }), context({ bindings, isMac: true }))?.id).toBe("chat.new");
    expect(dispatchShortcut(event("n", { code: "KeyN", ctrlKey: true }), context({ bindings, isMac: true }))).toBeNull();
    expect(dispatchShortcut(event("n", { code: "KeyN", metaKey: true }), context({ bindings, isMac: false }))).toBeNull();
  });

  it("leaves an event an earlier handler consumed", () => {
    const hit = dispatchShortcut(
      event("n", { code: "KeyN", ctrlKey: true, defaultPrevented: true }),
      context({ bindings: { "chat.new": "mod+n" } }),
    );
    expect(hit).toBeNull();
  });

  it("runs a rebinding", () => {
    const bindings = { "chat.new": "ctrl+shift+k" };
    expect(dispatchShortcut(event("k", { code: "KeyK", ctrlKey: true, shiftKey: true }), context({ bindings }))?.id).toBe("chat.new");
    // The old default is free again.
    expect(dispatchShortcut(event("n", { code: "KeyN", ctrlKey: true }), context({ bindings }))).toBeNull();
  });

  it("runs nothing for a command the user unbound", () => {
    const bindings = { "chat.new": null };
    expect(dispatchShortcut(event("n", { code: "KeyN", ctrlKey: true }), context({ bindings }))).toBeNull();
  });

  it("never asks about a handler before a binding matches", () => {
    const hasHandler = vi.fn(() => true);
    dispatchShortcut(event("x", { code: "KeyX" }), context({ bindings: { "chat.new": "mod+n" }, hasHandler }));
    expect(hasHandler).not.toHaveBeenCalled();
  });
});

describe("scopes", () => {
  const gridBindings = {
    "vibe.open": "enter",
    "vibe.focus.down": "arrowdown",
  };

  it("does not fire a mode's key while the mode is off", () => {
    expect(dispatchShortcut(event("Enter", { code: "Enter" }), context({ bindings: gridBindings }))).toBeNull();
  });

  it("fires it while the mode's scope is active", () => {
    const hit = dispatchShortcut(
      event("Enter", { code: "Enter" }),
      context({ bindings: gridBindings, scopes: ["vibeGrid", "vibe", "global"] }),
    );
    expect(hit?.id).toBe("vibe.open");
  });

  it("lets a mode shadow a global key, and gives it back when the mode closes", () => {
    // Inside the zoomed cell ⌘[ is "hide the side panel"; outside it the same key
    // is the rail's "previous conversation". One key, two scopes, no conflict.
    const bindings = {
      "thread.previous": "mod+bracketleft",
      "vibe.panel.hide": "mod+bracketleft",
    };
    const press = event("[", { code: "BracketLeft", ctrlKey: true });
    expect(dispatchShortcut(press, context({ bindings, scopes: ["global"] }))?.id).toBe("thread.previous");
    expect(dispatchShortcut(press, context({ bindings, scopes: ["vibeZoom", "vibe", "global"] }))?.id).toBe("vibe.panel.hide");
    // The innermost scope wins when both are on.
    expect(dispatchShortcut(press, context({ bindings, scopes: ["vibe", "global"] }))?.id).toBe("thread.previous");
  });

  it("never runs a bare global key", () => {
    // `bindingProblem` refuses to record one, and this is the other half: even if
    // a table carried it, a modifier-less global command does not fire.
    expect(dispatchShortcut(event("ArrowDown", { code: "ArrowDown" }), context({ bindings: { "thread.next": "arrowdown" } }))).toBeNull();
  });

  it("leaves a bare mode key to the text field that has the caret", () => {
    // The zoomed cell has a composer: `↩` there sends, it does not zoom.
    const bindings = { "vibe.open": "enter" };
    const scopes = ["vibeGrid", "vibe", "global"] as const;
    expect(dispatchShortcut(event("Enter", { code: "Enter" }), context({ bindings, editable: true, scopes }))).toBeNull();
    expect(dispatchShortcut(event("Enter", { code: "Enter" }), context({ bindings, editable: false, scopes }))?.id).toBe("vibe.open");
  });
});

describe("several keys on one command", () => {
  it("reports which key matched", () => {
    const bindings = { "vibe.cell": ["1", "2", "3"] };
    const scopes = ["vibeGrid", "vibe", "global"] as const;
    const third = dispatchShortcut(
      event("3", { code: "Digit3" }),
      context({ bindings, scopes }),
    );
    expect(third?.id).toBe("vibe.cell");
    // The handler needs the digit: nine bindings, one command.
    expect(third?.binding.code).toBe("Digit3");
  });
});

describe("the input layer", () => {
  // The one place a bare key runs *over* a text field: `↩` sends, where every
  // other layer's `↩` belongs to the editor.
  const inputBindings = { "vibe.input.close": "escape", "vibe.input.send": "enter" };
  const scopes = ["vibeInput", "vibeZoom", "vibe", "global"] as const;

  it("sends with ↩ while the caret is in the editor", () => {
    const hit = dispatchShortcut(
      event("Enter", { code: "Enter" }),
      context({ bindings: inputBindings, editable: true, scopes }),
    );
    expect(hit?.id).toBe("vibe.input.send");
  });

  it("leaves a composing ↩ to the input method", () => {
    // This is what makes dictation safe: the Enter that commits a candidate (or
    // ends a phrase) is never the Enter that sends.
    const press = (overrides: Partial<ShortcutEvent>) => event("Enter", { code: "Enter", ...overrides });
    expect(dispatchShortcut(press({ isComposing: true }), context({ bindings: inputBindings, editable: true, scopes }))).toBeNull();
    expect(dispatchShortcut(press({ keyCode: 229 }), context({ bindings: inputBindings, editable: true, scopes }))).toBeNull();
  });

  it("leaves another layer's bare key to the editor", () => {
    // One layer out, `↓` walks message blocks and `↩` zooms. Neither may reach
    // over the field that has the caret — that is the difference between
    // `overEditable` and an ordinary scoped command.
    const bindings = { "vibe.block.next": "arrowdown", "vibe.open": "enter" };
    const outer = ["vibeZoom", "vibe", "global"] as const;
    expect(dispatchShortcut(event("ArrowDown", { code: "ArrowDown" }), context({ bindings, editable: true, scopes: outer }))).toBeNull();
    expect(dispatchShortcut(event("Enter", { code: "Enter" }), context({ bindings, editable: true, scopes: outer }))).toBeNull();
    // Without the caret they run normally.
    expect(dispatchShortcut(event("ArrowDown", { code: "ArrowDown" }), context({ bindings, editable: false, scopes: outer }))?.id).toBe("vibe.block.next");
  });

  it("keeps the remote's own buttons live over the editor", () => {
    // Reaching the menu mid-dictation is exactly when the menu button is needed,
    // and the home button jumps back to the live end of the conversation.
    const bindings = { "vibe.menu": ["mod+shift+k", "contextmenu"], "vibe.home": "browserhome" };
    const input = context({ bindings, editable: true, scopes });
    expect(dispatchShortcut(event("ContextMenu", { code: "ContextMenu" }), input)?.id).toBe("vibe.menu");
    expect(dispatchShortcut(event("BrowserHome", { code: "BrowserHome" }), input)?.id).toBe("vibe.home");
  });
});

describe("isBareBinding", () => {
  it("is true without a real modifier, Shift included", () => {
    expect(isBareBinding(parseBinding("arrowdown")!)).toBe(true);
    // Shift alone does not make a chord: it is how a capital letter is typed, and
    // `Shift+Enter` is a newline in the composer.
    expect(isBareBinding(parseBinding("shift+enter")!)).toBe(true);
    expect(isBareBinding(parseBinding("mod+k")!)).toBe(false);
    expect(isBareBinding(parseBinding("alt+k")!)).toBe(false);
  });
});
