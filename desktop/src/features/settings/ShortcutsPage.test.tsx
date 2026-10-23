// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { resetAllCommandBindings, resolvedBindingText, setCommandBinding } from "../keyboard/shortcuts";
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

/** The section whose heading reads `label`. */
function section(label: string): HTMLElement {
  const found = [...container.querySelectorAll<HTMLElement>("section")]
    .find(candidate => candidate.querySelector("h3")?.textContent?.trim() === label);
  if (!found)
    throw new Error(`no section for ${label}`);
  return found;
}

/** The heading of a section, which folds it open and shut. */
function sectionToggle(label: string): HTMLButtonElement {
  const button = section(label).querySelector<HTMLButtonElement>("button[aria-expanded]");
  if (!button)
    throw new Error(`no toggle in the ${label} section`);
  return button;
}

function search(text: string) {
  const input = container.querySelector<HTMLInputElement>("input[aria-label=\"Search commands\"]")!;
  const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => {
    setValue.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
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
    search("terminal");
    expect(container.textContent).toContain("Show or hide the terminal");
    expect(container.textContent).not.toContain("New conversation");
  });

  it("lists the app's own keys first and the panel's last", () => {
    // Frequency, not taxonomy: the two screens a reader comes here for, then the
    // small groups, then the mode most readers never open.
    const headings = [...container.querySelectorAll("h3")].map(node => node.textContent?.trim());
    expect(headings).toEqual([
      "Conversation",
      "Navigation",
      "Panels and terminal",
      "Search and settings",
      "Vibe panel",
      "Fixed keys",
    ]);
  });

  it("folds the Vibe panel's keys shut, and says how many are behind the fold", () => {
    // Thirty-odd keys that only answer inside one panel: a reader who came to change
    // ⌘N should not scroll past them to reach the row beneath.
    expect(sectionToggle("Vibe panel").getAttribute("aria-expanded")).toBe("false");
    expect(section("Vibe panel").querySelector("button[aria-pressed]")).toBeNull();
    expect(section("Vibe panel").textContent).toMatch(/\d+ keys/);

    act(() => sectionToggle("Vibe panel").click());
    expect(sectionToggle("Vibe panel").getAttribute("aria-expanded")).toBe("true");
    expect(bindingButton("Open or close the Vibe panel").textContent).toBe("Ctrl+Shift+V");

    act(() => sectionToggle("Vibe panel").click());
    expect(section("Vibe panel").querySelector("button[aria-pressed]")).toBeNull();
  });

  it("shows a customisation on a folded section's heading", () => {
    // Folding must not be able to hide the one thing this page exists to report.
    expect(section("Vibe panel").textContent).not.toMatch(/changed/);
    act(() => setCommandBinding("vibe.toggle", "mod+shift+y"));
    expect(section("Vibe panel").textContent).toContain("1 changed");
  });

  it("opens a folded section whose keys a search matches", () => {
    // A match folded away reads as "no matches": the query opens every section.
    expect(container.textContent).not.toContain("Go to the numbered cell");
    search("numbered");
    expect(container.textContent).toContain("Go to the numbered cell");
  });

  it("lists the Vibe panel's keys, and says they only work while it is open", () => {
    // A mode's keys are part of the table like any other, so a reader can see the
    // whole keyboard — including the ones that would be unusable outside it.
    act(() => sectionToggle("Vibe panel").click());
    expect(container.textContent).toContain("Vibe panel");
    expect(container.textContent).toContain("Open or close the Vibe panel");
    expect(bindingButton("Open or close the Vibe panel").textContent).toBe("Ctrl+Shift+V");
    // The panel's own keys are bare, so the row says when they apply instead of
    // pretending they work everywhere.
    expect(container.textContent).toContain("Only while the Vibe panel is open");
    expect(bindingButton("Leave the zoomed conversation").textContent).toBe("Esc");
  });

  it("shows the direct-selection keys as a fixed list instead of a recorder", () => {
    // `1`–`9` is one command with nine keys: recording a single key would ask a
    // question the page cannot answer, so it is shown, not edited.
    act(() => sectionToggle("Vibe panel").click());
    const cellRow = row("Go to the numbered cell");
    expect(cellRow.querySelector("button[aria-pressed]")).toBeNull();
    expect(cellRow.textContent).toContain("1 … 9");
  });

  it("says when nothing matches", () => {
    search("zzzz");
    expect(container.textContent).toContain("No command matches");
  });
});
