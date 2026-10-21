// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clearCommandHandlers, registerCommand } from "../keyboard/commands";
import { resetAllCommandBindings, setCommandBinding } from "../keyboard/shortcuts";
import { ShortcutCheatSheet } from "./ShortcutCheatSheet";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

function mount(open = true) {
  act(() => root.render(<ShortcutCheatSheet onClose={() => {}} open={open} />));
}

/** The key shown for a command, found by its label. */
function keysFor(label: string): string | null {
  const rows = [...container.querySelectorAll("dl > div")];
  const row = rows.find(candidate => candidate.querySelector("dt")?.textContent === label);
  return row?.querySelector("dd")?.textContent ?? null;
}

beforeEach(() => {
  localStorage.clear();
  clearCommandHandlers();
  resetAllCommandBindings();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  clearCommandHandlers();
  resetAllCommandBindings();
});

describe("keyboard shortcut cheat sheet", () => {
  it("lists the live keys, grouped by category", () => {
    mount();
    expect(container.textContent).toContain("Conversation");
    expect(container.textContent).toContain("Navigation");
    expect(keysFor("New conversation")).toBe("Ctrl+N");
    expect(keysFor("Show or hide the terminal")).toBe("Ctrl+J");
  });

  it("follows a rebinding while it is open", () => {
    // The reason a cheat sheet exists in an app with editable keys: a card printed
    // from the defaults would be a card of the old keys.
    mount();
    expect(keysFor("New conversation")).toBe("Ctrl+N");

    act(() => setCommandBinding("chat.new", "mod+shift+k"));
    expect(keysFor("New conversation")).toBe("Ctrl+Shift+K");

    // … and back again when the customisation is dropped.
    act(() => resetAllCommandBindings());
    expect(keysFor("New conversation")).toBe("Ctrl+N");
  });

  it("leaves out a command with no key", () => {
    mount();
    act(() => setCommandBinding("chat.new", null));
    expect(keysFor("New conversation")).toBeNull();
  });

  it("shows a multi-key command as its range", () => {
    mount();
    expect(keysFor("Go to the numbered cell")).toBe("1 … 9");
  });

  it("says when a group's keys only work inside a mode", () => {
    mount();
    // The Vibe panel's rows are bare keys, so the group says when they apply.
    expect(container.textContent).toContain("Only while the Vibe panel is open");
  });

  it("sends a reader to the page that edits these keys", () => {
    // The button is the command, not a second way into Settings: a rebinding of
    // `shortcuts.settings` moves the key, and this button still says what it does.
    const onClose = vi.fn();
    const edit = vi.fn();
    const unregister = registerCommand("shortcuts.settings", edit);
    act(() => root.render(<ShortcutCheatSheet onClose={onClose} open />));

    const button = [...container.querySelectorAll("button")].find(node => node.textContent === "Edit shortcuts");
    expect(button).toBeTruthy();
    act(() => button!.click());
    expect(edit).toHaveBeenCalledTimes(1);
    // Closed first, so the settings dialog is the overlay on top.
    expect(onClose).toHaveBeenCalledTimes(1);
    unregister();
  });

  it("offers no edit button while nothing owns that command", () => {
    // On the onboarding gate there is no settings page to reach; a button that
    // does nothing is worse than none.
    mount();
    expect([...container.querySelectorAll("button")].some(node => node.textContent === "Edit shortcuts")).toBe(false);
  });

  it("renders nothing when closed", () => {
    mount(false);
    expect(container.textContent).toBe("");
  });
});
