// @vitest-environment jsdom
import type { MouseEvent as ReactMouseEvent } from "react";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../../test/renderHook";
import { MIN_CENTER_PANEL_WIDTH, MIN_RIGHT_PANEL_WIDTH } from "./panelGeometry";
import { RIGHT_PANEL_DEFAULT_WIDTH, useRightPanelWidth } from "./useRightPanelWidth";

const STORAGE_KEY = "future.rightPanelWidth";

function setWindowWidth(width: number) {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: width, writable: true });
}

/** A center element whose left edge sits at `left`. */
function center(left: number) {
  const element = document.createElement("main");
  element.getBoundingClientRect = () => ({ left } as DOMRect);
  return { current: element };
}

function mouse(type: string, options: MouseEventInit = {}) {
  return new MouseEvent(type, { bubbles: true, cancelable: true, ...options });
}

/**
 * The resize handle takes a React synthetic event; `startResize` only reads
 * `button` and calls `preventDefault`, so a minimal stand-in is enough.
 */
function handleMouseDown(button = 0) {
  return { button, preventDefault: vi.fn() } as unknown as ReactMouseEvent<Element, globalThis.MouseEvent>;
}

describe("useRightPanelWidth", () => {
  beforeEach(() => {
    sessionStorage.clear();
    setWindowWidth(1600);
  });

  it("defaults to the historical fixed width and persists it", () => {
    const hook = renderHook(() => useRightPanelWidth(center(224)));
    expect(hook.current.width).toBe(RIGHT_PANEL_DEFAULT_WIDTH);
    expect(sessionStorage.getItem(STORAGE_KEY)).toBe(String(RIGHT_PANEL_DEFAULT_WIDTH));
  });

  it("restores a stored width and falls back when it is unreadable", () => {
    sessionStorage.setItem(STORAGE_KEY, "640");
    const stored = renderHook(() => useRightPanelWidth(center(224)));
    expect(stored.current.width).toBe(640);

    sessionStorage.setItem(STORAGE_KEY, "not-a-number");
    const corrupt = renderHook(() => useRightPanelWidth(center(224)));
    expect(corrupt.current.width).toBe(RIGHT_PANEL_DEFAULT_WIDTH);
  });

  it("re-clamps a too-wide stored width on mount so the center keeps its floor", () => {
    // 900px window, rail ending at 224 → only 292px for the right panel, below
    // its own floor; the center's floor wins and the panel takes the minimum.
    setWindowWidth(900);
    sessionStorage.setItem(STORAGE_KEY, "900");
    const hook = renderHook(() => useRightPanelWidth(center(224)));
    expect(hook.current.width).toBe(MIN_RIGHT_PANEL_WIDTH);
  });

  it("re-clamps on window resize", () => {
    sessionStorage.setItem(STORAGE_KEY, "700");
    const hook = renderHook(() => useRightPanelWidth(center(224)));
    expect(hook.current.width).toBe(700);

    setWindowWidth(700);
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(hook.current.width).toBe(MIN_RIGHT_PANEL_WIDTH);
  });

  it("nudges within the clamp range and rounds the result", () => {
    const hook = renderHook(() => useRightPanelWidth(center(224)));
    // 1600 - 224 = 1376 available; the right panel may take up to 992.
    act(() => {
      hook.current.nudge(100);
    });
    expect(hook.current.width).toBe(484);

    act(() => {
      hook.current.nudge(10_000);
    });
    expect(hook.current.width).toBe(1600 - 224 - MIN_CENTER_PANEL_WIDTH);

    act(() => {
      hook.current.nudge(-10_000);
    });
    expect(hook.current.width).toBe(MIN_RIGHT_PANEL_WIDTH);
  });

  it("ignores a non-primary mouse button on the resize handle", () => {
    const hook = renderHook(() => useRightPanelWidth(center(224)));
    const event = handleMouseDown(2);

    act(() => {
      hook.current.startResize(event);
    });
    expect(hook.current.resizing).toBe(false);
    expect(event.preventDefault).not.toHaveBeenCalled();
  });

  it("resizes on drag and stops listening on mouseup", () => {
    const hook = renderHook(() => useRightPanelWidth(center(224)));
    const event = handleMouseDown();

    act(() => {
      hook.current.startResize(event);
    });
    expect(hook.current.resizing).toBe(true);
    expect(event.preventDefault).toHaveBeenCalled();

    // Drag so the pointer sits 700px from the window's right edge.
    act(() => {
      window.dispatchEvent(mouse("mousemove", { clientX: 900 }));
    });
    expect(hook.current.width).toBe(700);

    act(() => {
      window.dispatchEvent(mouse("mouseup"));
    });
    expect(hook.current.resizing).toBe(false);

    // After mouseup the drag is detached: further moves must not resize.
    act(() => {
      window.dispatchEvent(mouse("mousemove", { clientX: 1200 }));
    });
    expect(hook.current.width).toBe(700);
  });

  it("rounds fractional drag positions", () => {
    const hook = renderHook(() => useRightPanelWidth(center(224)));
    act(() => {
      hook.current.startResize(handleMouseDown());
    });
    act(() => {
      window.dispatchEvent(mouse("mousemove", { clientX: 899.4 }));
    });
    expect(hook.current.width).toBe(701);
  });

  it("detaches the drag and resize listeners on unmount", () => {
    const addSpy = vi.spyOn(window, "addEventListener");
    const removeSpy = vi.spyOn(window, "removeEventListener");
    try {
      const hook = renderHook(() => useRightPanelWidth(center(224)));
      act(() => {
        hook.current.startResize(handleMouseDown());
      });
      expect(hook.current.resizing).toBe(true);
      hook.unmount();

      const added = (type: string) => addSpy.mock.calls.filter(([name]) => name === type).length;
      const removed = (type: string) => removeSpy.mock.calls.filter(([name]) => name === type).length;
      expect(added("mousemove")).toBeGreaterThan(0);
      expect(removed("mousemove")).toBe(added("mousemove"));
      expect(removed("mouseup")).toBe(added("mouseup"));
      expect(removed("resize")).toBe(added("resize"));
    }
    finally {
      addSpy.mockRestore();
      removeSpy.mockRestore();
    }
  });

  it("still resizes when the center element is missing", () => {
    const hook = renderHook(() => useRightPanelWidth({ current: null }));
    expect(hook.current.width).toBe(RIGHT_PANEL_DEFAULT_WIDTH);
    act(() => {
      hook.current.startResize(handleMouseDown());
    });
    act(() => {
      window.dispatchEvent(mouse("mousemove", { clientX: 1500 }));
    });
    expect(hook.current.width).toBe(MIN_RIGHT_PANEL_WIDTH);
  });

  it("survives a sessionStorage that is unavailable", () => {
    // Private mode / storage disabled: every touch of `sessionStorage` throws.
    // Without the hook's try/catch the initial read (or the persist effect)
    // would take the whole render down.
    const original = Object.getOwnPropertyDescriptor(window, "sessionStorage");
    Object.defineProperty(window, "sessionStorage", {
      configurable: true,
      get() {
        throw new Error("SecurityError: storage disabled");
      },
    });
    try {
      const hook = renderHook(() => useRightPanelWidth(center(224)));
      expect(hook.current.width).toBe(RIGHT_PANEL_DEFAULT_WIDTH);
      expect(() => act(() => {
        hook.current.nudge(50);
      })).not.toThrow();
      // The in-memory width still applies for this session.
      expect(hook.current.width).toBe(434);
    }
    finally {
      if (original)
        Object.defineProperty(window, "sessionStorage", original);
      else
        Reflect.deleteProperty(window, "sessionStorage");
    }
  });

  it("assumes a zero left edge when the DOMRect has none", () => {
    const element = document.createElement("main");
    element.getBoundingClientRect = () => ({ left: undefined } as unknown as DOMRect);
    const hook = renderHook(() => useRightPanelWidth({ current: element }));

    // 1600 - 0 = 1600 available, so the panel may grow to 1600 - 384.
    act(() => {
      hook.current.nudge(10_000);
    });
    expect(hook.current.width).toBe(1600 - MIN_CENTER_PANEL_WIDTH);
  });
});
