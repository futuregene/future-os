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
  function Harness({ sessionId = "a", ids = [] }: { sessionId?: string; ids?: string[] }) {
    current = useTimelinePaging(
      sessionId,
      true,
      false,
      request,
      onScroll,
      ids.map(id => ({ id })),
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

  test("programmatic scroll never loads a page; collision starts request immediately", () => {
    act(() => current.onScroll(scrollEvent(1400)));
    expect(request).not.toHaveBeenCalled();
    collide();
    expect(request).toHaveBeenCalledTimes(1);
    expect(current.pagingActive).toBe(true);
  });

  test("a completed empty page retains the marker for 1500ms from collision", async () => {
    collide();
    await act(async () => {});
    await advance(1499);
    expect(current.pagingActive).toBe(true);
    await advance(1);
    expect(current.showLoadOlderHint).toBe(false);
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

  test("slow response runs concurrently with minimum time, not another 1500ms", async () => {
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
    await advance(1500);
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
    await advance(1500);
    expect(current.pagingActive).toBe(false);
    expect(current.pagingFailed).toBe(true);
    request.mockImplementationOnce(() => {
      throw new Error("disconnected");
    });
    act(() => current.loadOlder());
    await advance(1500);
    expect(current.pagingFailed).toBe(true);
    act(() => current.loadOlder());
    await act(async () => {});
    await advance(1500);
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
