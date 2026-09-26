// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useFloatingScrollbar } from "./useFloatingScrollbar";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * ResizeObserver stub. jsdom has no layout engine, so the test supplies the
 * geometry; the observer only has to exist so the hook's effect can install it
 * (and so the observer-driven recompute path is exercised).
 */
class ResizeObserverStub {
  constructor(private readonly callback: () => void) {}

  observe() {}
  disconnect() {}
  unobserve() {}

  fire() {
    this.callback();
  }
}

let observers: ResizeObserverStub[] = [];

function setGeometry(element: HTMLElement, geometry: { clientHeight: number; scrollHeight: number }) {
  Object.defineProperty(element, "clientHeight", { configurable: true, value: geometry.clientHeight });
  Object.defineProperty(element, "scrollHeight", { configurable: true, value: geometry.scrollHeight });
}

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

beforeEach(() => {
  observers = [];
  vi.stubGlobal("ResizeObserver", class extends ResizeObserverStub {
    constructor(callback: () => void) {
      super(callback);
      observers.push(this);
    }
  });
});

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

/** Mount a probe with the documented wiring (refs + onScroll + thumb drag). */
function mountHarness(options: { attachRefs?: boolean } = {}) {
  const attachRefs = options.attachRefs ?? true;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  let latest!: ReturnType<typeof useFloatingScrollbar>;

  function Probe() {
    latest = useFloatingScrollbar();
    return (
      <div
        ref={attachRefs ? latest.scrollRef : undefined}
        data-testid="viewport"
        onScroll={latest.handleScroll}
      >
        <div ref={latest.contentRef} />
        <div data-testid="thumb" onPointerDown={latest.handleThumbPointerDown} />
      </div>
    );
  }

  act(() => root.render(<Probe />));

  return {
    container,
    get scrollbar() {
      return latest;
    },
    viewport: container.querySelector<HTMLElement>("[data-testid=viewport]")!,
    thumb: container.querySelector<HTMLElement>("[data-testid=thumb]")!,
  };
}

/** Recompute through the observer installed by the hook's own effect. */
function observedRecompute() {
  act(() => {
    for (const observer of observers)
      observer.fire();
  });
}

/** Recompute through the window-resize path (no observer involved). */
function resizeRecompute() {
  act(() => {
    window.dispatchEvent(new Event("resize"));
  });
}

function pointer(type: string, clientY: number) {
  return new MouseEvent(type, { bubbles: true, cancelable: true, clientY });
}

describe("useFloatingScrollbar geometry", () => {
  it("starts with no thumb and observes both the viewport and the content", () => {
    const harness = mountHarness();

    expect(harness.scrollbar.scrollbar).toEqual({ height: 0, top: 1, visible: false });
    // The effect installs exactly one observer and watches the two boxes that can
    // change the ratio (the fixed-height viewport and its content wrapper).
    expect(observers).toHaveLength(1);
  });

  it("stays hidden for content that fits its viewport", () => {
    const harness = mountHarness();
    setGeometry(harness.viewport, { clientHeight: 100, scrollHeight: 100 });

    resizeRecompute();

    expect(harness.scrollbar.scrollbar).toEqual({ height: 0, top: 1, visible: false });
  });

  it("computes the thumb from the viewport ratio and the scroll offset", () => {
    const harness = mountHarness();
    setGeometry(harness.viewport, { clientHeight: 100, scrollHeight: 400 });

    observedRecompute();
    // 100/400 of 98px is below the 36px floor, so the floor wins at the top.
    expect(harness.scrollbar.scrollbar).toEqual({ height: 36, top: 1, visible: false });

    harness.viewport.scrollTop = 300;
    act(() => harness.scrollbar.updateFloatingScrollbar(true));
    // Scrolled to the end: the thumb sits flush against the bottom inset.
    expect(harness.scrollbar.scrollbar).toEqual({ height: 36, top: 63, visible: true });
  });

  it("uses a larger thumb once the content ratio allows it", () => {
    const harness = mountHarness();
    setGeometry(harness.viewport, { clientHeight: 400, scrollHeight: 800 });

    observedRecompute();

    // 400/800 of 398px = 199px, well above the floor.
    expect(harness.scrollbar.scrollbar).toEqual({ height: 199, top: 1, visible: false });
  });

  it("skips the state write when a recompute produces identical geometry", () => {
    const harness = mountHarness();
    setGeometry(harness.viewport, { clientHeight: 100, scrollHeight: 400 });
    observedRecompute();

    const first = harness.scrollbar.scrollbar;

    // A second recompute with unchanged geometry (observer fire, resize event)
    // must return the same object identity, or every streaming push would
    // commit twice.
    observedRecompute();
    resizeRecompute();

    expect(harness.scrollbar.scrollbar).toBe(first);
  });

  it("does nothing when the scroll container is not attached yet", () => {
    const harness = mountHarness({ attachRefs: false });

    expect(harness.scrollbar.scrollRef.current).toBeNull();
    expect(() => harness.scrollbar.updateFloatingScrollbar(true)).not.toThrow();
    // No container means the mount effect returns early, so the initial state is
    // the pristine zero (never touched).
    expect(harness.scrollbar.scrollbar).toEqual({ height: 0, top: 0, visible: false });
  });
});

describe("useFloatingScrollbar visibility", () => {
  it("reveals the thumb on scroll and hides it again after the linger delay", () => {
    const harness = mountHarness();
    setGeometry(harness.viewport, { clientHeight: 100, scrollHeight: 400 });
    resizeRecompute();
    vi.useFakeTimers();

    act(() => {
      harness.viewport.scrollTop = 50;
      harness.viewport.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    expect(harness.scrollbar.scrollbar.visible).toBe(true);

    // A second scroll before the timer fires replaces the pending one instead of
    // stacking a second hide.
    act(() => {
      harness.viewport.scrollTop = 60;
      harness.viewport.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    act(() => {
      vi.advanceTimersByTime(1199);
    });
    expect(harness.scrollbar.scrollbar.visible).toBe(true);

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(harness.scrollbar.scrollbar.visible).toBe(false);
  });

  it("keeps the thumb hidden while the content fits, even when scrolled", () => {
    const harness = mountHarness();
    setGeometry(harness.viewport, { clientHeight: 100, scrollHeight: 100 });

    act(() => {
      harness.viewport.dispatchEvent(new Event("scroll", { bubbles: true }));
    });

    expect(harness.scrollbar.scrollbar.visible).toBe(false);
  });

  it("clears its pending hide timer on unmount", () => {
    const harness = mountHarness();
    setGeometry(harness.viewport, { clientHeight: 100, scrollHeight: 400 });
    resizeRecompute();
    vi.useFakeTimers();

    act(() => {
      harness.viewport.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    act(() => roots[0]!.root.unmount());

    // The timer would otherwise fire into a detached component.
    expect(() => vi.advanceTimersByTime(5000)).not.toThrow();
  });
});

describe("useFloatingScrollbar thumb drag", () => {
  it("maps pointer travel onto the scroll offset and detaches on pointerup", () => {
    const harness = mountHarness();
    setGeometry(harness.viewport, { clientHeight: 100, scrollHeight: 400 });
    resizeRecompute();
    harness.viewport.scrollTop = 0;

    // Thumb height 36, insets 1: maxTop = 62, scrollable = 300.
    act(() => {
      harness.thumb.dispatchEvent(pointer("pointerdown", 10));
    });
    act(() => {
      document.dispatchEvent(pointer("pointermove", 41));
    });

    // (41-10)/62 * 300 = 150.
    expect(harness.viewport.scrollTop).toBeCloseTo(150);

    act(() => {
      document.dispatchEvent(pointer("pointermove", 72));
    });
    expect(harness.viewport.scrollTop).toBeCloseTo(300);

    act(() => {
      document.dispatchEvent(pointer("pointerup", 72));
    });
    act(() => {
      document.dispatchEvent(pointer("pointermove", 0));
    });
    // Detached: a stray move after the drag must not move the content.
    expect(harness.viewport.scrollTop).toBeCloseTo(300);
  });

  it("preventDefaults the pointerdown so a drag does not start a text selection", () => {
    const harness = mountHarness();
    setGeometry(harness.viewport, { clientHeight: 100, scrollHeight: 400 });
    resizeRecompute();

    const event = pointer("pointerdown", 10);
    const prevented = vi.fn();
    event.preventDefault = prevented;
    act(() => {
      harness.thumb.dispatchEvent(event);
    });

    expect(prevented).toHaveBeenCalledTimes(1);
  });

  it("ignores a drag on content that cannot scroll", () => {
    const harness = mountHarness();
    setGeometry(harness.viewport, { clientHeight: 100, scrollHeight: 100 });
    resizeRecompute();
    harness.viewport.scrollTop = 0;

    act(() => {
      harness.thumb.dispatchEvent(pointer("pointerdown", 10));
    });
    act(() => {
      document.dispatchEvent(pointer("pointermove", 90));
    });

    expect(harness.viewport.scrollTop).toBe(0);
  });

  it("ignores a drag when the thumb fills the track (no travel left)", () => {
    const harness = mountHarness();
    // A 36px thumb in a 30px viewport leaves no travel to map onto the offset.
    setGeometry(harness.viewport, { clientHeight: 30, scrollHeight: 1000 });
    resizeRecompute();
    harness.viewport.scrollTop = 0;

    act(() => {
      harness.thumb.dispatchEvent(pointer("pointerdown", 10));
    });
    act(() => {
      document.dispatchEvent(pointer("pointermove", 400));
    });

    expect(harness.viewport.scrollTop).toBe(0);
  });

  it("detaches a drag that is still in progress when the component unmounts", () => {
    const harness = mountHarness();
    setGeometry(harness.viewport, { clientHeight: 100, scrollHeight: 400 });
    resizeRecompute();
    harness.viewport.scrollTop = 0;

    act(() => {
      harness.thumb.dispatchEvent(pointer("pointerdown", 10));
    });
    act(() => {
      document.dispatchEvent(pointer("pointermove", 41));
    });
    const parked = harness.viewport.scrollTop;

    // Switching threads mid-drag unmounts the scroll region.
    act(() => roots[0]!.root.unmount());
    act(() => {
      document.dispatchEvent(pointer("pointermove", 72));
    });

    expect(harness.viewport.scrollTop).toBe(parked);
  });

  it("does nothing when the thumb is pressed before the ref is attached", () => {
    const harness = mountHarness({ attachRefs: false });

    const event = pointer("pointerdown", 10);
    const prevented = vi.fn();
    event.preventDefault = prevented;
    act(() => {
      harness.thumb.dispatchEvent(event);
    });

    expect(prevented).not.toHaveBeenCalled();
  });
});
