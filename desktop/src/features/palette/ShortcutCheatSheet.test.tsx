// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
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
  resetAllCommandBindings();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
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

  it("renders nothing when closed", () => {
    mount(false);
    expect(container.textContent).toBe("");
  });
});
