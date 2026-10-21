// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { clearCommandHandlers, hasCommandHandler } from "../keyboard/commands";
import { resetShortcutScopes } from "../keyboard/scopes";
import { resetAllCommandBindings } from "../keyboard/shortcuts";
import { VibeKeyProbe } from "./VibeKeyProbe";

/**
 * The readout's two promises, which are the whole reason it is allowed inside a panel
 * whose keys are load-bearing:
 *
 * 1. **it consumes nothing** — a press it shows still reaches the app, so opening it
 *    cannot change how `←` or `⏎` behave;
 * 2. **it is not a command** — no id in the table, no handler in the registry, so the
 *    dispatcher's decision order is untouched.
 *
 * Both are asserted here rather than argued in a comment.
 */
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let seenByPage: KeyboardEvent[];

/** Records every press that survives to the page — i.e. that nothing stopped. */
function record(event: KeyboardEvent) {
  seenByPage.push(event);
}

beforeEach(() => {
  clearCommandHandlers();
  resetAllCommandBindings();
  resetShortcutScopes();
  container = document.body.appendChild(document.createElement("div"));
  root = createRoot(container);
  seenByPage = [];
  // A listener registered *after* the readout's own: whatever this one sees, the app's
  // views would also see.
  window.addEventListener("keydown", record);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  window.removeEventListener("keydown", record);
});

function render(enabled: boolean) {
  act(() => root.render(<VibeKeyProbe enabled={enabled} />));
}

/** Hold ⌥⇧ and press K, `times` times, close together. */
function gesture(times = 3) {
  for (let index = 0; index < times; index++) {
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", {
        altKey: true,
        bubbles: true,
        cancelable: true,
        code: "KeyK",
        key: "k",
        shiftKey: true,
      }));
    });
  }
}

function press(code: string, key: string, init: KeyboardEventInit = {}) {
  let notPrevented = false;
  act(() => {
    const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, code, key, ...init });
    notPrevented = window.dispatchEvent(event);
  });
  return notPrevented;
}

function lines() {
  return [...container.querySelectorAll("[data-vibe-key-probe] div.font-mono > div")]
    .map(element => element.textContent ?? "");
}

describe("vibeKeyProbe", () => {
  it("renders nothing when it is not enabled", () => {
    render(false);
    gesture();
    expect(container.querySelector("[data-vibe-key-probe]")).toBeNull();
  });

  it("stays hidden until the gesture is completed", () => {
    render(true);
    gesture(2);
    expect(container.querySelector("[data-vibe-key-probe]")).toBeNull();
    gesture(1);
    expect(container.querySelector("[data-vibe-key-probe]")).not.toBeNull();
  });

  it("prints the key it just saw, quoted, newest first", () => {
    render(true);
    gesture();
    press("ArrowUp", "ArrowUp");
    press("ContextMenu", "ContextMenu");
    const shown = lines();
    expect(shown[0]).toContain(`code="ContextMenu"`);
    expect(shown[1]).toContain(`code="ArrowUp"`);
  });

  it("consumes no press: the page still receives every key it shows", () => {
    // This is the promise that lets it live inside the panel. `dispatchEvent` returns
    // false when a listener called `preventDefault`, so this is the direct test of
    // "nothing here blocks a key" rather than an indirect one.
    render(true);
    gesture();
    expect(press("ArrowLeft", "ArrowLeft")).toBe(true);
    expect(press("Enter", "Enter")).toBe(true);
    expect(press("ContextMenu", "ContextMenu")).toBe(true);
    // …and each of them was recorded, i.e. it also did not stop propagation.
    expect(seenByPage.map(event => event.code)).toEqual(
      expect.arrayContaining(["ArrowLeft", "Enter", "ContextMenu"]),
    );
    expect(lines()).toHaveLength(3);
  });

  it("adds no command to the registry", () => {
    // A readout that could be dispatched would be able to change what a key means.
    render(true);
    gesture();
    for (const id of ["vibe.keyprobe", "vibe.probe", "vibe.key.probe"] as const)
      expect(hasCommandHandler(id as never)).toBe(false);
  });

  it("closes on the gesture again, and forgets the presses", () => {
    render(true);
    gesture();
    press("KeyA", "a");
    expect(lines()).toHaveLength(1);
    gesture();
    expect(container.querySelector("[data-vibe-key-probe]")).toBeNull();
    gesture();
    expect(lines()).toEqual([]);
  });
});
