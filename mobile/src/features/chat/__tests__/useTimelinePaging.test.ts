import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { NativeScrollEvent, NativeSyntheticEvent } from "react-native";
import { useTimelinePaging } from "../useTimelinePaging";

function scrollEvent(y: number): NativeSyntheticEvent<NativeScrollEvent> {
  return {
    nativeEvent: {
      contentOffset: { x: 0, y },
      contentSize: { width: 320, height: 2000 },
      layoutMeasurement: { width: 320, height: 600 },
    },
  } as NativeSyntheticEvent<NativeScrollEvent>;
}

describe("paging transaction", () => {
  let renderer: ReactTestRenderer;
  let current: ReturnType<typeof useTimelinePaging>;
  const request = jest.fn<Promise<false | string[]>, []>();
  const onScroll = jest.fn();
  function Harness({ sessionId = "a", ids = [], rows }: { sessionId?: string; ids?: string[]; rows?: { id: string }[] }) {
    current = useTimelinePaging(
      sessionId,
      true,
      false,
      request,
      onScroll,
      rows ?? ids.map(id => ({ id })),
    );
    return null;
  }
  const advance = async (ms: number) => {
    await act(async () => {
      jest.advanceTimersByTime(ms);
    });
  };
  const collide = () =>
    act(() => {
      current.onScrollBeginDrag();
      current.onScroll(scrollEvent(1400));
    });
  beforeEach(() => {
    jest.useFakeTimers();
    request.mockResolvedValue([]);
    act(() => {
      renderer = create(createElement(Harness));
    });
  });
  afterEach(() => {
    act(() => renderer.unmount());
    jest.useRealTimers();
    request.mockReset();
    onScroll.mockReset();
  });

  test("idle text updates do not rebuild the full history identity index", async () => {
    const readId = jest.fn((i: number) => `row-${i}`);
    const rows = Array.from({ length: 1000 }, (_, i) => ({ get id() { return readId(i); } }));
    for (let frame = 0; frame < 100; frame++) {
      act(() => renderer.update(createElement(Harness, { rows: [...rows] })));
    }
    expect(readId).not.toHaveBeenCalled();
    collide();
    expect(readId).toHaveBeenCalledTimes(rows.length);
    await act(async () => {});
  });

  test("programmatic scroll never loads a page; collision starts request immediately", () => {
    act(() => current.onScroll(scrollEvent(1400)));
    expect(request).not.toHaveBeenCalled();
    collide();
    expect(request).toHaveBeenCalledTimes(1);
    expect(current.pagingActive).toBe(true);
  });

  test("a deliberate gesture prefetches at most one page within a bounded look-ahead", () => {
    act(() => {
      current.onScrollBeginDrag();
      current.onScroll(scrollEvent(799));
    });
    expect(request).not.toHaveBeenCalled();
    act(() => {
      current.onScroll(scrollEvent(800));
      current.onScroll(scrollEvent(1000));
      current.onScroll(scrollEvent(1400));
    });
    expect(request).toHaveBeenCalledTimes(1);
  });

  test("a completed empty page only waits for the 100ms quiet window", async () => {
    collide();
    await act(async () => {});
    await advance(99);
    expect(current.pagingActive).toBe(true);
    await advance(1);
    expect(current.showLoadOlderHint).toBe(false);
  });

  test("a fast committed page permits another deliberate gesture without a 1500ms cooldown", async () => {
    request.mockResolvedValue(["older"]);
    collide();
    await act(async () => {});
    act(() => renderer.update(createElement(Harness, { ids: ["older"] })));
    act(() => current.onListLayout());
    await advance(99);
    expect(current.pagingActive).toBe(true);
    await advance(1);
    expect(current.pagingActive).toBe(false);
    collide();
    expect(request).toHaveBeenCalledTimes(2);
  });

  test("request completion and unchanged old geometry cannot satisfy a new page", async () => {
    request.mockResolvedValue(["older"]);
    collide();
    await act(async () => {});
    act(() => current.onListLayout());
    await advance(2000);
    expect(current.pagingActive).toBe(true);
    act(() => renderer.update(createElement(Harness, { ids: ["older"] })));
    await advance(200);
    expect(current.pagingActive).toBe(true);
    act(() => current.onListLayout());
    await advance(99);
    expect(current.pagingActive).toBe(true);
    // Late row layout restarts the quiet window.
    act(() => current.onListLayout());
    await advance(100);
    expect(current.showLoadOlderHint).toBe(false);
  });

  test("a slow response only waits for layout quietness after completion", async () => {
    let resolve!: (result: string[]) => void;
    request.mockImplementationOnce(
      () =>
        new Promise(done => {
          resolve = done;
        }),
    );
    collide();
    await advance(2000);
    expect(current.pagingActive).toBe(true);
    await act(async () => resolve([]));
    await advance(100);
    expect(current.showLoadOlderHint).toBe(false);
  });

  test("drag and momentum cannot cascade to another page even after cooldown", async () => {
    collide();
    await act(async () => {});
    await advance(100);
    act(() => {
      current.onScrollEndDrag(scrollEvent(1400));
      current.onMomentumScrollEnd(scrollEvent(1400));
    });
    expect(request).toHaveBeenCalledTimes(1);
    // Another deliberate drag at the clamped edge can load again.
    act(() => {
      current.onScrollBeginDrag();
      current.onScrollEndDrag(scrollEvent(1400));
    });
    expect(request).toHaveBeenCalledTimes(2);
  });

  test("timeout and synchronous failures expose a retry without unhandled rejection", async () => {
    request.mockRejectedValueOnce(new Error("timeout"));
    collide();
    await act(async () => {});
    await advance(100);
    expect(current.pagingActive).toBe(false);
    expect(current.pagingFailed).toBe(true);
    request.mockImplementationOnce(() => {
      throw new Error("disconnected");
    });
    act(() => current.loadOlder());
    await advance(100);
    expect(current.pagingFailed).toBe(true);
    act(() => current.loadOlder());
    await act(async () => {});
    await advance(100);
    expect(current.showLoadOlderHint).toBe(false);
  });

  test("old-session completion cannot change the new-session transaction", async () => {
    let resolveOld!: (result: false) => void;
    request.mockImplementationOnce(
      () =>
        new Promise(done => {
          resolveOld = done;
        }),
    );
    collide();
    act(() => renderer.update(createElement(Harness, { sessionId: "b" })));
    request.mockResolvedValueOnce(["b-page"]);
    collide();
    await act(async () => resolveOld(false));
    await advance(2000);
    expect(current.pagingActive).toBe(true);
    expect(current.pagingFailed).toBe(false);
    act(() => renderer.update(createElement(Harness, { sessionId: "b", ids: ["b-page"] })));
    act(() => current.onListLayout());
    await advance(100);
    expect(current.showLoadOlderHint).toBe(false);
  });

  test("returning to a previous session cannot resurrect its marker", async () => {
    request.mockImplementationOnce(() => new Promise(() => {}));
    collide();
    act(() => renderer.update(createElement(Harness, { sessionId: "b" })));
    act(() => renderer.update(createElement(Harness, { sessionId: "a" })));
    expect(current.showLoadOlderHint).toBe(false);
    expect(current.pagingActive).toBe(false);
  });
});
