import type { Mock } from "vitest";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clearCommandHandlers, useCommand } from "./commands";
import { resetShortcutScopes, useShortcutScope } from "./scopes";
import { ShortcutHost } from "./ShortcutHost";
import { resetAllCommandBindings, setCommandBinding } from "./shortcuts";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * A mode's keymap, end to end: one physical key running two different commands
 * depending on whether the mode is on, and a mode's bare keys firing only inside
 * it and never over a text field.
 *
 * This is the infrastructure the full-screen Vibe panel stands on — inside it `←`
 * moves between cells and `Esc` backs out, and `⌘[` can mean "hide the side
 * panel" while outside it is the rail's "previous conversation". Two commands on
 * one key is not a conflict when they live in different scopes; it is a conflict
 * when they share one, and the recorder refuses that.
 */
let host: ReturnType<typeof createRoot>;
let view: ReturnType<typeof createRoot>;
let panelHandler: Mock<() => void>;
let railHandler: Mock<() => void>;

function mountMode(open: boolean) {
  function App() {
    // What the panel will hold: its scopes while it is on, and handlers for the
    // commands those scopes carry.
    useShortcutScope("vibe", open);
    useShortcutScope("vibeZoom", open);
    useCommand("vibe.back", panelHandler, open);
    useCommand("thread.previous", railHandler);
    return null;
  }
  act(() => view.render(<App />));
}

function press(target: EventTarget, init: KeyboardEventInit): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
  act(() => target.dispatchEvent(event));
  return event;
}

beforeEach(() => {
  localStorage.clear();
  resetAllCommandBindings();
  clearCommandHandlers();
  resetShortcutScopes();
  panelHandler = vi.fn<() => void>();
  railHandler = vi.fn<() => void>();
  host = createRoot(document.body.appendChild(document.createElement("div")));
  act(() => host.render(<ShortcutHost />));
  view = createRoot(document.body.appendChild(document.createElement("div")));
});

afterEach(() => {
  act(() => host.unmount());
  act(() => view.unmount());
  clearCommandHandlers();
  resetShortcutScopes();
});

describe("mode keymaps", () => {
  it("runs a mode's bare key only while the mode is on", () => {
    // The panel binds `Esc` to "leave the zoomed conversation". Outside the panel
    // nothing binds it, so it stays the platform's.
    mountMode(false);
    expect(press(document.body, { key: "Escape", keyCode: 27 }).defaultPrevented).toBe(false);
    expect(panelHandler).not.toHaveBeenCalled();

    mountMode(true);
    expect(press(document.body, { key: "Escape", keyCode: 27 }).defaultPrevented).toBe(true);
    expect(panelHandler).toHaveBeenCalledTimes(1);
  });

  it("lets the mode win inside and gives the key back outside", () => {
    // The same key in two scopes: the mode's copy is what fires while it is on,
    // the rail's the moment it closes. Neither shadows the other outside its own
    // keymap, so the app never has to choose at launch.
    act(() => setCommandBinding("vibe.back", "mod+bracketleft"));

    mountMode(true);
    press(document.body, { code: "BracketLeft", ctrlKey: true, key: "[" });
    expect(panelHandler).toHaveBeenCalledTimes(1);
    expect(railHandler).not.toHaveBeenCalled();

    mountMode(false);
    press(document.body, { code: "BracketLeft", ctrlKey: true, key: "[" });
    expect(railHandler).toHaveBeenCalledTimes(1);
    expect(panelHandler).toHaveBeenCalledTimes(1);
  });

  it("leaves a bare mode key to the field that has the caret", () => {
    // The zoomed cell has a composer: `Esc` there belongs to the field's own
    // handling, and the panel must not back out while the user is typing.
    mountMode(true);
    const input = document.body.appendChild(document.createElement("input"));

    expect(press(input, { key: "Escape", keyCode: 27 }).defaultPrevented).toBe(false);
    expect(panelHandler).not.toHaveBeenCalled();

    // Outside a field the same key belongs to the mode again.
    expect(press(document.body, { key: "Escape", keyCode: 27 }).defaultPrevented).toBe(true);
    expect(panelHandler).toHaveBeenCalledTimes(1);
    input.remove();
  });
});
