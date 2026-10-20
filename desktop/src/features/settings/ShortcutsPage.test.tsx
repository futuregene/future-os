// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { resetAllCommandBindings, resolvedBindingText } from "../keyboard/shortcuts";
import { ShortcutsPage } from "./ShortcutsPage";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  localStorage.clear();
  resetAllCommandBindings();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  act(() => root.render(<ShortcutsPage />));
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

/** The row for one command, found by its visible label. */
function row(label: string): HTMLElement {
  const found = [...container.querySelectorAll<HTMLElement>("[class*=\"py-3.5\"]")]
    .find(candidate => candidate.textContent?.includes(label));
  if (!found)
    throw new Error(`no row for ${label}`);
  return found;
}

function bindingButton(label: string): HTMLButtonElement {
  const button = row(label).querySelector<HTMLButtonElement>("button[aria-pressed]");
  if (!button)
    throw new Error(`no recorder in the ${label} row`);
  return button;
}

function keydown(init: KeyboardEventInit, target: EventTarget = window) {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
  act(() => {
    target.dispatchEvent(event);
  });
  return event;
}

describe("keyboard shortcut settings", () => {
  it("lists every command with its current binding", () => {
    expect(bindingButton("New conversation").textContent).toBe("Ctrl+N");
    expect(bindingButton("Show or hide the terminal").textContent).toBe("Ctrl+J");
  });

  it("records a new combination and shows it immediately", () => {
    act(() => bindingButton("New conversation").click());
    expect(bindingButton("New conversation").textContent).toContain("Press a new combination");

    keydown({ ctrlKey: true, shiftKey: true, key: "K" });

    expect(resolvedBindingText("chat.new")).toBe("mod+shift+KeyK");
    expect(bindingButton("New conversation").textContent).toBe("Ctrl+Shift+K");
    // Disarmed again: the next press is the app's, not the recorder's.
    expect(bindingButton("New conversation").getAttribute("aria-pressed")).toBe("false");
  });

  it("refuses a combination that needs a modifier, and stays armed", () => {
    act(() => bindingButton("New conversation").click());
    keydown({ key: "k" });
    expect(container.querySelector("[role=\"alert\"]")?.textContent).toContain("Include Ctrl or Alt");
    expect(resolvedBindingText("chat.new")).toBe("mod+n");
    expect(bindingButton("New conversation").getAttribute("aria-pressed")).toBe("true");
  });

  it("refuses a combination another command already uses, and names it", () => {
    act(() => bindingButton("New conversation").click());
    // `Ctrl+F` is the find bar's.
    keydown({ ctrlKey: true, key: "f" });
    expect(container.querySelector("[role=\"alert\"]")?.textContent).toContain("Find in this conversation");
    expect(resolvedBindingText("chat.new")).toBe("mod+n");
  });

  it("refuses a reserved combination", () => {
    act(() => bindingButton("New conversation").click());
    keydown({ ctrlKey: true, key: "c" });
    expect(container.querySelector("[role=\"alert\"]")?.textContent).toContain("Reserved by the system");
    expect(resolvedBindingText("chat.new")).toBe("mod+n");
  });

  it("cancels on Escape without touching the binding", () => {
    act(() => bindingButton("New conversation").click());
    keydown({ ctrlKey: true, shiftKey: true, key: "K" });
    expect(resolvedBindingText("chat.new")).toBe("mod+shift+KeyK");
    act(() => bindingButton("New conversation").click());
    keydown({ key: "Escape" });
    expect(resolvedBindingText("chat.new")).toBe("mod+shift+KeyK");
    expect(bindingButton("New conversation").getAttribute("aria-pressed")).toBe("false");
  });

  it("removes a binding with Backspace and says so", () => {
    act(() => bindingButton("New conversation").click());
    keydown({ key: "Backspace" });
    expect(resolvedBindingText("chat.new")).toBeNull();
    expect(bindingButton("New conversation").textContent).toBe("Not set");
  });

  it("offers a per-row restore only once a command is customised", () => {
    const rowButtons = () => row("New conversation").querySelectorAll("button").length;
    const before = rowButtons();
    act(() => bindingButton("New conversation").click());
    keydown({ ctrlKey: true, shiftKey: true, key: "K" });
    expect(rowButtons()).toBe(before + 1);
    const restore = row("New conversation").querySelectorAll("button")[0]!;
    act(() => restore.click());
    expect(resolvedBindingText("chat.new")).toBe("mod+n");
    expect(rowButtons()).toBe(before);
  });

  it("restores every default at once", () => {
    const restoreAll = [...container.querySelectorAll("button")].find(button => button.textContent === "Restore all defaults")!;
    expect(restoreAll.disabled).toBe(true);

    act(() => bindingButton("New conversation").click());
    keydown({ ctrlKey: true, shiftKey: true, key: "K" });
    expect(restoreAll.disabled).toBe(false);
    act(() => restoreAll.click());
    expect(resolvedBindingText("chat.new")).toBe("mod+n");
    expect(restoreAll.disabled).toBe(true);
  });

  it("filters the table by command name", () => {
    const search = container.querySelector<HTMLInputElement>("input[aria-label=\"Search commands\"]")!;
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    act(() => {
      setValue.call(search, "terminal");
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(container.textContent).toContain("Show or hide the terminal");
    expect(container.textContent).not.toContain("New conversation");
  });

  it("says when nothing matches", () => {
    const search = container.querySelector<HTMLInputElement>("input[aria-label=\"Search commands\"]")!;
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    act(() => {
      setValue.call(search, "zzzz");
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(container.textContent).toContain("No command matches");
  });
});
