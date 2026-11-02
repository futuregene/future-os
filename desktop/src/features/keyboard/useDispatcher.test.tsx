// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clearCommandHandlers, useCommand } from "./commands";
import { ShortcutHost } from "./ShortcutHost";
import { resetAllCommandBindings, setCommandBinding } from "./shortcuts";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * The window listener end to end: a component registers a command, the host
 * installs the dispatcher, and a real key event travels between them.
 *
 * What is *not* re-tested here is the rule table — `dispatch.test.ts` covers it
 * without a DOM. This file is about the wiring those rules depend on: the event
 * actually reaching the host, a handled key not also reaching the page, and a
 * rebinding taking effect on the next press.
 */
let host: ReturnType<typeof createRoot>;
let hostContainer: HTMLDivElement;
let view: ReturnType<typeof createRoot>;
let viewContainer: HTMLDivElement;

function press(target: EventTarget, init: KeyboardEventInit): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
  act(() => {
    target.dispatchEvent(event);
  });
  return event;
}

function mountApp(onNewChat: () => void) {
  function Recent(): null {
    // A view that registers a command of its own, the way `ActivityRail`,
    // `ThreadSearch` and the composer do.
    useCommand("chat.new", onNewChat);
    return null;
  }
  act(() => {
    view.render(<Recent />);
  });
}

beforeEach(() => {
  localStorage.clear();
  resetAllCommandBindings();
  clearCommandHandlers();
  hostContainer = document.createElement("div");
  viewContainer = document.createElement("div");
  document.body.append(hostContainer, viewContainer);
  host = createRoot(hostContainer);
  view = createRoot(viewContainer);
  act(() => {
    host.render(<ShortcutHost />);
  });
});

afterEach(() => {
  act(() => host.unmount());
  act(() => view.unmount());
  hostContainer.remove();
  viewContainer.remove();
  clearCommandHandlers();
});

describe("shortcut dispatcher", () => {
  it("runs a registered command and consumes the key", () => {
    const onNewChat = vi.fn();
    mountApp(onNewChat);
    const event = press(window, { ctrlKey: true, key: "n" });
    expect(onNewChat).toHaveBeenCalledTimes(1);
    expect(event.defaultPrevented).toBe(true);
  });

  it("keeps the event from reaching the focused view once it is handled", () => {
    mountApp(vi.fn());
    const downstream = vi.fn();
    document.addEventListener("keydown", downstream);
    press(document.body, { ctrlKey: true, key: "n" });
    expect(downstream).not.toHaveBeenCalled();
    document.removeEventListener("keydown", downstream);
  });

  it("leaves an unregistered command's key alone", () => {
    // No view mounted: nothing claims ⌘N, so it stays the platform's own.
    const event = press(window, { ctrlKey: true, key: "n" });
    expect(event.defaultPrevented).toBe(false);
  });

  it("uses a rebinding on the next press, without a reload", () => {
    const onNewChat = vi.fn();
    mountApp(onNewChat);
    act(() => setCommandBinding("chat.new", "ctrl+shift+k"));
    // The old combination is free ...
    expect(press(window, { ctrlKey: true, key: "n" }).defaultPrevented).toBe(false);
    expect(onNewChat).not.toHaveBeenCalled();
    // ... and the new one works immediately.
    expect(press(window, { ctrlKey: true, shiftKey: true, key: "k" }).defaultPrevented).toBe(true);
    expect(onNewChat).toHaveBeenCalledTimes(1);
  });

  it("does nothing for a command the user unbound", () => {
    const onNewChat = vi.fn();
    mountApp(onNewChat);
    act(() => setCommandBinding("chat.new", null));
    expect(press(window, { ctrlKey: true, key: "n" }).defaultPrevented).toBe(false);
    expect(onNewChat).not.toHaveBeenCalled();
  });

  it("leaves the shell's keys alone inside the terminal", () => {
    mountApp(vi.fn());
    const onPanel = vi.fn();
    // Registered on a view that also happens to be mounted: the terminal rule is
    // about where the key was pressed, not about who owns the command.
    function Panel(): null {
      useCommand("panel.left", onPanel);
      return null;
    }
    act(() => {
      view.render(<Panel />);
    });

    // Outside the terminal the app answers Ctrl+D.
    press(document.body, { ctrlKey: true, key: "d" });
    expect(onPanel).toHaveBeenCalledTimes(1);

    // Inside it, `Ctrl+D` is EOF and must reach the shell untouched.
    const terminal = document.createElement("div");
    terminal.dataset.component = "terminal";
    const surface = document.createElement("textarea");
    terminal.append(surface);
    document.body.append(terminal);
    const inside = press(surface, { ctrlKey: true, key: "d" });
    expect(onPanel).toHaveBeenCalledTimes(1);
    expect(inside.defaultPrevented).toBe(false);
    terminal.remove();
  });

  it("does not run while an IME is composing", () => {
    const onNewChat = vi.fn();
    mountApp(onNewChat);
    press(window, { ctrlKey: true, isComposing: true, key: "n" });
    expect(onNewChat).not.toHaveBeenCalled();
  });
});
