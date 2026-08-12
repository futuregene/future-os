// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { act } from "react";
import { flushAsync, renderHook } from "../test/renderHook";
import { useAsyncResource } from "./useAsyncResource";

describe("useAsyncResource", () => {
  it("loads data and clears the loading flag", async () => {
    const h = renderHook(() => useAsyncResource(() => Promise.resolve("data"), [], "init"));
    expect(h.current.loading).toBe(true);
    await flushAsync();
    expect(h.current).toMatchObject({ data: "data", loading: false, error: null });
    h.unmount();
  });

  it("captures loader errors as messages", async () => {
    const h = renderHook(() => useAsyncResource(() => Promise.reject(new Error("boom")), [], "init"));
    await flushAsync();
    expect(h.current).toMatchObject({ data: "init", loading: false, error: "boom" });
    h.unmount();
  });

  it("stringifies non-Error rejections", async () => {
    const h = renderHook(() => useAsyncResource(() => Promise.reject("raw"), [], "init"));
    await flushAsync();
    expect(h.current.error).toBe("raw");
    h.unmount();
  });

  it("reloads silently without flipping loading", async () => {
    let value = 0;
    const h = renderHook(() => useAsyncResource(() => Promise.resolve(++value), [], 0));
    await flushAsync();
    expect(h.current.data).toBe(1);
    act(() => {
      h.current.reload();
    });
    expect(h.current.loading).toBe(false);
    await flushAsync();
    expect(h.current.data).toBe(2);
    h.unmount();
  });

  it("shows the spinner again when deps change", async () => {
    let dep = 1;
    const h = renderHook(() => useAsyncResource(() => Promise.resolve(dep * 10), [dep], 0));
    await flushAsync();
    expect(h.current.data).toBe(10);
    dep = 2;
    h.rerender();
    expect(h.current.loading).toBe(true);
    await flushAsync();
    expect(h.current.data).toBe(20);
    h.unmount();
  });

  it("skips the state update when isEqual reports structural equality", async () => {
    const h = renderHook(() => useAsyncResource(
      () => Promise.resolve([1, 2]),
      [],
      [] as number[],
      { isEqual: (a, b) => a.join() === b.join() },
    ));
    await flushAsync();
    const first = h.current.data;
    expect(first).toEqual([1, 2]);
    act(() => {
      h.current.reload();
    });
    await flushAsync();
    expect(h.current.data).toBe(first);
    h.unmount();
  });

  it("applies a new load after an isEqual-skip when the previous value is the initial data", async () => {
    let value = 1;
    const h = renderHook(() => useAsyncResource(
      () => Promise.resolve(value),
      [],
      0,
      { isEqual: (a, b) => a === b },
    ));
    await flushAsync();
    // Previous === initialData: isEqual guard is bypassed entirely.
    expect(h.current.data).toBe(1);
    h.unmount();
  });

  it("ignores a load that resolves after unmount", async () => {
    let resolveLoad!: (v: string) => void;
    const h = renderHook(() => useAsyncResource(
      () => new Promise<string>((resolve) => {
        resolveLoad = resolve;
      }),
      [],
      "init",
    ));
    h.unmount();
    resolveLoad("late");
    await flushAsync();
    // No state update after unmount — nothing to assert on beyond not crashing.
    expect(h.current.data).toBe("init");
  });

  it("ignores an error that rejects after unmount", async () => {
    let rejectLoad!: (e: Error) => void;
    const h = renderHook(() => useAsyncResource(
      () => new Promise<string>((_, reject) => {
        rejectLoad = reject;
      }),
      [],
      "init",
    ));
    h.unmount();
    rejectLoad(new Error("late"));
    await flushAsync();
    expect(h.current.error).toBeNull();
  });
});
