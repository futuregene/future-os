// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TimezoneField } from "./TimezoneField";

/**
 * The timezone picker inside a form.
 *
 * Its one non-obvious rule is the Escape key: the list is a popup over a dialog, and
 * both listen for Escape. The field stops the key at its own root so the press closes
 * the list rather than discarding the whole form — a reader who meant "never mind
 * this list" must not lose what they had typed. Every other key has to pass through,
 * or typing in the search box would be swallowed here.
 */
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  // Two levels on purpose: React attaches its delegated listeners to the root
  // container, so a listener there would fire alongside the field's own no matter
  // how it stops propagation. `host` sits above it, where stopping is visible.
  host = document.createElement("div");
  container = document.createElement("div");
  host.append(container);
  document.body.append(host);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

function mount(onChange = vi.fn()) {
  act(() => root.render(<TimezoneField locale="en" onChange={onChange} value="" />));
  return onChange;
}

/** The list's popup, open or not. */
function listNode(): Element | null {
  return container.querySelector("[role='listbox']");
}

function openList(): void {
  act(() => container.querySelector<HTMLButtonElement>("button[aria-haspopup='listbox']")!.click());
}

function press(target: Element, key: string): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key });
  act(() => {
    target.dispatchEvent(event);
  });
  return event;
}

describe("picking a zone", () => {
  it("reports the row's zone and closes the list", () => {
    const onChange = mount();
    openList();
    const rows = [...container.querySelectorAll<HTMLElement>("[role='option']")];
    expect(rows.length).toBeGreaterThan(1);
    const chosen = rows.find(row => row.getAttribute("aria-selected") !== "true")!;
    const zone = chosen.getAttribute("data-zone") ?? chosen.textContent;

    act(() => chosen.click());

    expect(onChange).toHaveBeenCalledTimes(1);
    // The value the form stores is the zone id, not the sentence the row shows.
    expect(String(onChange.mock.calls[0]![0])).toContain(String(zone).split(" ")[0]!);
    expect(listNode()).toBeNull();
  });
});

describe("the timezone list's Escape", () => {
  it("closes the list and keeps the press from the dialog behind it", () => {
    mount();
    openList();
    expect(listNode()).not.toBeNull();
    // What a dialog enclosing this form would see: the same key, arriving at an
    // ancestor. `stopPropagation` is the mechanism, so this is what to observe.
    const enclosing = vi.fn();
    host.addEventListener("keydown", enclosing);

    press(container.firstElementChild!, "Escape");

    expect(listNode()).toBeNull();
    expect(enclosing).not.toHaveBeenCalled();
    host.removeEventListener("keydown", enclosing);
  });

  it("also clears what was typed into the list's search box", () => {
    mount();
    openList();
    const search = container.querySelector<HTMLInputElement>("input")!;
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    act(() => {
      setValue.call(search, "tok");
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });

    press(container.firstElementChild!, "Escape");
    openList();

    // Reopening starts from the whole list rather than the abandoned search.
    expect(container.querySelector<HTMLInputElement>("input")!.value).toBe("");
  });

  it("leaves every other key to the field it was typed in", () => {
    mount();
    openList();

    const enclosing = vi.fn();
    host.addEventListener("keydown", enclosing);

    press(container.firstElementChild!, "a");

    expect(enclosing).toHaveBeenCalledTimes(1);
    // Still open: only Escape closes it from the keyboard.
    expect(listNode()).not.toBeNull();
    host.removeEventListener("keydown", enclosing);
  });
});
