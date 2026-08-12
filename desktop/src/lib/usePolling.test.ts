// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { renderHook } from "../test/renderHook";
import { usePolling } from "./usePolling";

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("usePolling", () => {
  it("runs immediately and then on the interval while enabled", () => {
    const callback = vi.fn();
    const h = renderHook(() => usePolling(callback, 1000));
    expect(callback).toHaveBeenCalledTimes(1);
    act(() => {
      vi.advanceTimersByTime(3000);
    });
    expect(callback).toHaveBeenCalledTimes(4);
    h.unmount();
  });

  it("never installs a timer when disabled", () => {
    const callback = vi.fn();
    const h = renderHook(() => usePolling(callback, 1000, { enabled: false }));
    act(() => {
      vi.advanceTimersByTime(5000);
    });
    expect(callback).not.toHaveBeenCalled();
    h.unmount();
  });

  it("stops ticking after unmount", () => {
    const callback = vi.fn();
    const h = renderHook(() => usePolling(callback, 1000));
    h.unmount();
    act(() => {
      vi.advanceTimersByTime(3000);
    });
    expect(callback).toHaveBeenCalledTimes(1);
  });

  it("restarts the poll when deps change", () => {
    const callback = vi.fn();
    let dep = "a";
    const h = renderHook(() => usePolling(callback, 1000, { deps: [dep] }));
    expect(callback).toHaveBeenCalledTimes(1);
    dep = "b";
    h.rerender();
    expect(callback).toHaveBeenCalledTimes(2);
    h.unmount();
  });
});
