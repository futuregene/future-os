import type { Mock } from "vitest";
// @vitest-environment jsdom
import type { PaletteCommand, PaletteTarget } from "./paletteItems";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clearCommandHandlers, registerCommand } from "../keyboard/commands";
import { resetAllCommandBindings, setCommandBinding } from "../keyboard/shortcuts";
import { CommandPalette } from "./CommandPalette";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let onClose: Mock<() => void>;

const commands: PaletteCommand[] = [
  { category: "chat", id: "chat.new", label: "New conversation" },
  { category: "settings", id: "settings.open", label: "Open settings" },
];
const openSettings = vi.fn();
const newChat = vi.fn();

function mount(open = true, targets: PaletteTarget[] = []) {
  act(() => root.render(
    <CommandPalette commands={commands} onClose={onClose} open={open} targets={targets} />,
  ));
}

function input(): HTMLInputElement {
  return container.querySelector<HTMLInputElement>("input[data-palette-input]")!;
}

function rows(): HTMLElement[] {
  return [...container.querySelectorAll<HTMLElement>("[data-palette-index]")];
}

function type(text: string) {
  const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => {
    setValue.call(input(), text);
    input().dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function press(key: string, extras: KeyboardEventInit = {}) {
  act(() => {
    input().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key, ...extras }));
  });
}

beforeEach(() => {
  clearCommandHandlers();
  localStorage.clear();
  resetAllCommandBindings();
  onClose = vi.fn<() => void>();
  openSettings.mockClear();
  newChat.mockClear();
  // The palette lists only commands that can run *now* — the same rule the
  // dispatcher applies to a key with no owner.
  registerCommand("settings.open", openSettings);
  registerCommand("chat.new", newChat);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  clearCommandHandlers();
});

describe("command palette", () => {
  it("lists the commands and focuses the input when it opens", () => {
    mount();
    expect(rows().map(row => row.textContent)).toEqual([
      expect.stringContaining("New conversation"),
      expect.stringContaining("Open settings"),
    ]);
    expect(document.activeElement).toBe(input());
  });

  it("runs the selected command and closes", () => {
    mount();
    press("ArrowDown");
    press("Enter");
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(openSettings).toHaveBeenCalledTimes(1);
  });

  it("runs the first row without an arrow press", () => {
    // `↩` straight after typing is the most common use; the selection starts on
    // the first row, so no arrow press is needed.
    mount();
    type("New conversation");
    press("Enter");
    expect(newChat).toHaveBeenCalledTimes(1);
  });

  it("leaves out a command that has no handler right now", () => {
    // "Open settings" is registered, "New conversation" is not: the second one is
    // a row that would do nothing, so it is not a row.
    act(() => {
      clearCommandHandlers();
    });
    registerCommand("settings.open", openSettings);
    mount();
    expect(rows().map(row => row.textContent)).toEqual([expect.stringContaining("Open settings")]);
  });

  it("filters as the user types", () => {
    mount();
    type("settings");
    expect(rows().map(row => row.textContent)).toEqual([expect.stringContaining("Open settings")]);
  });

  it("says so when nothing matches", () => {
    mount();
    type("zzzz");
    expect(rows()).toHaveLength(0);
    expect(container.textContent).toContain("Nothing matches");
  });

  it("wraps the selection at both ends", () => {
    mount();
    // Up from the first row lands on the last …
    press("ArrowUp");
    expect(rows()[1]!.getAttribute("aria-selected")).toBe("true");
    // … and down from the last comes back to the first.
    press("ArrowDown");
    expect(rows()[0]!.getAttribute("aria-selected")).toBe("true");
  });

  it("opens a conversation from the list, through its own action", () => {
    const open = vi.fn();
    mount(true, [{ key: "t1", kind: "conversation", run: open, subtitle: "Workspace A", title: "Compare conclusions" }]);
    type("compare");
    expect(rows().map(row => row.textContent)).toEqual([expect.stringContaining("Compare conclusions")]);
    press("Enter");
    expect(open).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("runs a row's command only when that row is the selected one", () => {
    // One conversation matching the query and no command: `↩` must run the
    // conversation's action and leave the command registry alone.
    const open = vi.fn();
    mount(true, [{ key: "t1", kind: "conversation", run: open, title: "Alpha" }]);
    type("alpha");
    expect(rows()).toHaveLength(1);
    press("Enter");
    expect(open).toHaveBeenCalledTimes(1);
    expect(openSettings).not.toHaveBeenCalled();
  });

  it("closes on the key that opened it", () => {
    // The dialog cannot rely on the dispatcher here: while it is open it *is* the
    // overlay the dispatcher stands behind.
    mount();
    press("p", { ctrlKey: true });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("follows the palette's own rebinding for that key", () => {
    act(() => setCommandBinding("palette.open", "mod+shift+p"));
    mount();
    // The default is no longer the palette's key …
    press("p", { ctrlKey: true });
    expect(onClose).not.toHaveBeenCalled();
    // … and the new one is.
    press("P", { ctrlKey: true, shiftKey: true });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("renders nothing when closed", () => {
    mount(false);
    expect(container.textContent).toBe("");
  });
});
