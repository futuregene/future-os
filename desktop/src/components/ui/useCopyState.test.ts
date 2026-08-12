import { act } from "react";
// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";

import { useCopyState } from "./useCopyState";

const copyTextMock = vi.fn<(text: string) => Promise<void>>();

vi.mock("../../lib/clipboard", () => ({
  copyText: (text: string) => copyTextMock(text),
}));

afterEach(() => {
  vi.useRealTimers();
  copyTextMock.mockReset();
});

describe("useCopyState", () => {
  it("flashes the copied key and resets after the timeout", async () => {
    vi.useFakeTimers();
    copyTextMock.mockResolvedValue(undefined);
    const h = renderHook(() => useCopyState());
    await act(async () => {
      await h.current.copy("text");
    });
    expect(h.current.copiedKey).toBe("default");
    act(() => {
      vi.advanceTimersByTime(1400);
    });
    expect(h.current.copiedKey).toBeNull();
    h.unmount();
  });

  it("keys the flag per copy target", async () => {
    copyTextMock.mockResolvedValue(undefined);
    const h = renderHook(() => useCopyState<"path" | "content">());
    await act(async () => {
      await h.current.copy("text", "path");
    });
    expect(h.current.copiedKey).toBe("path");
    h.unmount();
  });

  it("toasts instead of flashing when the copy fails", async () => {
    copyTextMock.mockRejectedValue(new Error("denied"));
    const h = renderHook(() => useCopyState());
    const events: CustomEvent[] = [];
    window.addEventListener("futureos:toast", e => events.push(e as CustomEvent));
    await act(async () => {
      await h.current.copy("text");
    });
    expect(h.current.copiedKey).toBeNull();
    expect(events).toHaveLength(1);
    expect(events[0]?.detail.tone).toBe("error");
    h.unmount();
  });

  it("clears a pending reset timer on unmount", async () => {
    vi.useFakeTimers();
    copyTextMock.mockResolvedValue(undefined);
    const h = renderHook(() => useCopyState());
    await act(async () => {
      await h.current.copy("text");
    });
    h.unmount();
    // Advancing past the reset fires no post-unmount state update.
    act(() => {
      vi.advanceTimersByTime(5000);
    });
  });

  it("re-copying clears the previous timer", async () => {
    vi.useFakeTimers();
    copyTextMock.mockResolvedValue(undefined);
    const h = renderHook(() => useCopyState());
    await act(async () => {
      await h.current.copy("a");
    });
    act(() => {
      vi.advanceTimersByTime(700);
    });
    await act(async () => {
      await h.current.copy("b");
    });
    act(() => {
      vi.advanceTimersByTime(1399);
    });
    expect(h.current.copiedKey).toBe("default");
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(h.current.copiedKey).toBeNull();
    h.unmount();
  });
});
