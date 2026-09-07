// @vitest-environment jsdom
import type { PointerEvent as ReactPointerEvent } from "react";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../../test/renderHook";
import { useLeftPanelWidth } from "./useLeftPanelWidth";

beforeEach(() => {
  localStorage.clear();
  Object.defineProperty(window, "innerWidth", { value: 1440, writable: true, configurable: true });
});

function pointer(type: string, clientX: number, pointerId = 1) {
  const event = new MouseEvent(type, { clientX, button: 0 });
  Object.defineProperty(event, "pointerId", { value: pointerId });
  return event;
}

describe("left panel resizing", () => {
  it("persists keyboard changes, clamps both bounds, and restores the preference after a narrow window", () => {
    const h = renderHook(() => useLeftPanelWidth(true));
    expect(h.current.width).toBe(288);
    act(() => h.current.nudge(1000));
    expect(h.current.width).toBe(480);
    act(() => {
      window.innerWidth = 1024;
      window.dispatchEvent(new Event("resize"));
    });
    expect(h.current.width).toBe(256);
    act(() => {
      window.innerWidth = 1440;
      window.dispatchEvent(new Event("resize"));
    });
    expect(h.current.width).toBe(480);
    h.unmount();
    const restored = renderHook(() => useLeftPanelWidth(false));
    expect(restored.current.width).toBe(480);
    act(() => restored.current.nudge(-1000));
    expect(restored.current.width).toBe(180);
    restored.unmount();
  });

  it("resizes by pointer delta, ignores other pointers, and cancels cleanly", () => {
    const h = renderHook(() => useLeftPanelWidth(false));
    act(() => h.current.startResize(pointer("pointerdown", 288) as unknown as ReactPointerEvent));
    expect(h.current.resizing).toBe(true);
    act(() => window.dispatchEvent(pointer("pointermove", 350, 2)));
    expect(h.current.width).toBe(288);
    act(() => window.dispatchEvent(pointer("pointermove", 350)));
    expect(h.current.width).toBe(350);
    act(() => window.dispatchEvent(pointer("pointercancel", 350)));
    expect(h.current.resizing).toBe(false);
    act(() => window.dispatchEvent(pointer("pointermove", 400)));
    expect(h.current.width).toBe(350);
    act(() => h.current.startResize(pointer("pointerdown", 350) as unknown as ReactPointerEvent));
    h.unmount();
    window.dispatchEvent(pointer("pointermove", 480));
    expect(localStorage.getItem("future.leftPanelWidth")).toBe("350");
  });

  it("tolerates unavailable storage and corrupt values", () => {
    localStorage.setItem("future.leftPanelWidth", "NaN");
    const h = renderHook(() => useLeftPanelWidth(false));
    expect(h.current.width).toBe(288);
    h.unmount();
    const get = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    const set = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    const unavailable = renderHook(() => useLeftPanelWidth(false));
    expect(unavailable.current.width).toBe(288);
    unavailable.unmount();
    get.mockRestore();
    set.mockRestore();
  });
});
