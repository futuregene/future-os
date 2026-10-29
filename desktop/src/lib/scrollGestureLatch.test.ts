// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { installScrollGestureLatch, scrollContainerFor } from "./scrollGestureLatch";

let clock = 0;
let cleanup: (() => void) | null = null;

beforeEach(() => {
  clock = 1000;
  cleanup = installScrollGestureLatch(window, { now: () => clock });
});

afterEach(() => {
  cleanup?.();
  cleanup = null;
  document.body.innerHTML = "";
});

/** A 200px-tall window over 1000px of content. */
function scroller({ horizontal = false, overflow = "auto" } = {}) {
  const element = document.createElement("div");
  if (horizontal)
    element.style.overflowX = overflow;
  else
    element.style.overflowY = overflow;
  Object.defineProperty(element, "clientHeight", { value: 200 });
  Object.defineProperty(element, "scrollHeight", { value: 1200 });
  Object.defineProperty(element, "clientWidth", { value: 200 });
  Object.defineProperty(element, "scrollWidth", { value: 1200 });
  const row = document.createElement("span");
  element.append(row);
  document.body.append(element);
  return { element, row };
}

function wheel(target: Element, deltaY: number, init: WheelEventInit = {}) {
  const event = new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY, ...init });
  target.dispatchEvent(event);
  return event;
}

/** Move the clock past the latch's gap, ending the gesture. */
function pause() {
  clock += 1000;
}

describe("scrollGestureLatch", () => {
  it("scrolls the container the gesture is in", () => {
    const list = scroller();
    expect(wheel(list.row, 40).defaultPrevented).toBe(false);
    clock += 16;
    expect(wheel(list.row, 40).defaultPrevented).toBe(false);
  });

  it("keeps the gesture out of a container the pointer moved onto", () => {
    const list = scroller();
    const panel = scroller();
    wheel(list.row, 40);
    clock += 16;
    expect(wheel(panel.row, 40).defaultPrevented).toBe(true);
    clock += 16;
    expect(wheel(panel.row, 40).defaultPrevented).toBe(true);
  });

  it("scrolls the other container once the gesture has ended", () => {
    const list = scroller();
    const panel = scroller();
    wheel(list.row, 40);
    pause();
    expect(wheel(panel.row, 40).defaultPrevented).toBe(false);
    clock += 16;
    expect(wheel(panel.row, 40).defaultPrevented).toBe(false);
  });

  it("adopts the first container under a gesture that starts off any list", () => {
    const panel = scroller();
    expect(wheel(document.body, 40).defaultPrevented).toBe(false);
    clock += 16;
    expect(wheel(panel.row, 40).defaultPrevented).toBe(false);
  });

  it("leaves a non-cancelable momentum tail to the engine", () => {
    const list = scroller();
    const panel = scroller();
    wheel(list.row, 40);
    clock += 16;
    // WebKit's momentum tail cannot be cancelled; it must not throw, and the
    // latch must not be handed over to the container it landed on.
    expect(wheel(panel.row, 40, { cancelable: false }).defaultPrevented).toBe(false);
    clock += 16;
    expect(wheel(panel.row, 40).defaultPrevented).toBe(true);
  });

  it("leaves pinch zoom and zero deltas alone", () => {
    const list = scroller();
    const panel = scroller();
    wheel(list.row, 40);
    clock += 16;
    expect(wheel(panel.row, 40, { ctrlKey: true }).defaultPrevented).toBe(false);
    expect(wheel(panel.row, 0).defaultPrevented).toBe(false);
    expect(wheel(panel.row, 40).defaultPrevented).toBe(true);
  });

  it("resolves the innermost scroller under the pointer", () => {
    const outer = scroller();
    const inner = scroller();
    outer.element.append(inner.element);
    expect(scrollContainerFor(inner.row, false)).toBe(inner.element);

    wheel(outer.row, 40);
    clock += 16;
    expect(wheel(inner.row, 40).defaultPrevented).toBe(true);
  });

  it("latches a horizontal wheel between horizontal scrollers", () => {
    const first = scroller({ horizontal: true });
    const second = scroller({ horizontal: true });
    wheel(first.row, 0, { deltaX: 40 });
    clock += 16;
    expect(wheel(second.row, 0, { deltaX: 40 }).defaultPrevented).toBe(true);
  });

  it("walks past a hidden-overflow box to the scroller that really scrolls", () => {
    const list = scroller();
    const clipped = scroller({ overflow: "hidden" });
    list.element.append(clipped.element);
    expect(scrollContainerFor(clipped.row, false)).toBe(list.element);
  });
});
