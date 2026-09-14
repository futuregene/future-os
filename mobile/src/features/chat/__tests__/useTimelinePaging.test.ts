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

describe("paging intent and bounded layout settling", () => {
  let renderer: ReactTestRenderer;
  let current: ReturnType<typeof useTimelinePaging>;
  const request = jest.fn<Promise<false | string[]>, []>();
  const onScroll = jest.fn();
  function Harness({
    sessionId = "a",
    canLoadOlder = true,
  }: {
    sessionId?: string;
    canLoadOlder?: boolean;
  }) {
    current = useTimelinePaging(
      sessionId,
      canLoadOlder,
      false,
      request,
      onScroll,
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
      current.onScrollBeginDrag(scrollEvent(0));
      current.onScroll(scrollEvent(1400));
    });
  beforeEach(() => {
    jest.useFakeTimers();
    request.mockResolvedValue(["older"]);
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

  test("programmatic scroll changes button visibility without fetching", () => {
    act(() => current.onScroll(scrollEvent(0)));
    expect(current.showLoadOlderHint).toBe(false);
    act(() => {
      current.onMomentumScrollBegin();
      current.onScroll(scrollEvent(1400));
      current.onMomentumScrollEnd(scrollEvent(1400));
    });
    expect(current.showLoadOlderHint).toBe(true);
    expect(request).not.toHaveBeenCalled();
    collide();
    expect(request).toHaveBeenCalledTimes(1);
    expect(current.pagingActive).toBe(true);
  });

  test("one gesture prefetches at most one page within one viewport", () => {
    act(() => {
      current.onScrollBeginDrag(scrollEvent(0));
      current.onScroll(scrollEvent(799));
    });
    expect(request).not.toHaveBeenCalled();
    act(() => {
      current.onScroll(scrollEvent(800));
      current.onScroll(scrollEvent(1400));
    });
    expect(request).toHaveBeenCalledTimes(1);
  });

  test("a clamped edge and a short list both have a usable entry point", async () => {
    act(() => {
      current.onListLayout(600);
      current.onContentSizeChange(320, 200);
    });
    expect(current.showLoadOlderHint).toBe(true);
    act(() => {
      current.loadOlder();
      current.loadOlder();
    });
    expect(request).toHaveBeenCalledTimes(1);
    await act(async () => {});
    await advance(1000);
    act(() => current.onScrollBeginDrag(scrollEvent(1400)));
    expect(request).toHaveBeenCalledTimes(2);
  });

  test("a committed page finishes after 100ms of native layout quietness", async () => {
    collide();
    await act(async () => {});
    act(() => current.onContentSizeChange(320, 2500));
    await advance(99);
    expect(current.pagingActive).toBe(true);
    // A late row layout restarts quietness, but not the overall deadline.
    act(() => current.onRowLayout());
    await advance(99);
    expect(current.pagingActive).toBe(true);
    await advance(1);
    expect(current.pagingActive).toBe(false);
    collide();
    expect(request).toHaveBeenCalledTimes(2);
  });

  test("missing native layout events release paging at exactly 1s", async () => {
    collide();
    await act(async () => {});
    await advance(999);
    expect(current.pagingActive).toBe(true);
    await advance(1);
    expect(current.pagingActive).toBe(false);
    expect(current.pagingFailed).toBe(false);
    act(() => current.loadOlder());
    expect(request).toHaveBeenCalledTimes(2);
  });

  test("continuous layouts cannot extend the 1s deadline", async () => {
    collide();
    await act(async () => {});
    for (let i = 0; i < 19; i++) {
      await advance(50);
      act(() => current.onContentSizeChange(320, 2000 + i));
    }
    expect(current.pagingActive).toBe(true);
    await advance(50);
    expect(current.pagingActive).toBe(false);
  });

  test("layout before commit never starts the 1s deadline", async () => {
    let resolve!: (result: string[]) => void;
    request.mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    collide();
    act(() => current.onContentSizeChange(320, 2500));
    await advance(5000);
    expect(current.pagingActive).toBe(true);
    await act(async () => resolve(["older"]));
    await advance(999);
    expect(current.pagingActive).toBe(true);
    await advance(1);
    expect(current.pagingActive).toBe(false);
  });

  test("momentum shares the gesture budget even after paging finishes", async () => {
    collide();
    await act(async () => {});
    await advance(1000);
    act(() => {
      current.onScrollEndDrag(scrollEvent(1400));
      current.onMomentumScrollBegin();
      current.onScroll(scrollEvent(1400));
      current.onMomentumScrollEnd(scrollEvent(1400));
    });
    expect(request).toHaveBeenCalledTimes(1);
    collide();
    expect(request).toHaveBeenCalledTimes(2);
  });

  test("unconsumed drag may prefetch during its momentum", () => {
    act(() => {
      current.onScrollBeginDrag(scrollEvent(0));
      current.onScrollEndDrag(scrollEvent(700));
      current.onMomentumScrollBegin();
      current.onScroll(scrollEvent(800));
    });
    expect(request).toHaveBeenCalledTimes(1);
  });

  test("ended drag without momentum cannot turn later layout scrolling into paging", () => {
    act(() => {
      current.onScrollBeginDrag(scrollEvent(0));
      current.onScrollEndDrag(scrollEvent(700));
      current.onContentSizeChange(320, 2400);
      current.onScroll(scrollEvent(1400));
    });
    expect(request).not.toHaveBeenCalled();
  });

  test("empty pages and errors need no layout wait; failures allow immediate retry", async () => {
    request.mockRejectedValueOnce(new Error("timeout"));
    collide();
    await act(async () => {});
    expect(current.pagingActive).toBe(false);
    expect(current.pagingFailed).toBe(true);
    request.mockImplementationOnce(() => {
      throw new Error("disconnected");
    });
    act(() => current.loadOlder());
    expect(current.pagingFailed).toBe(true);
    request.mockResolvedValueOnce([]);
    await act(async () => current.loadOlder());
    expect(current.pagingActive).toBe(false);
    expect(current.pagingFailed).toBe(false);
  });

  test("old-session completion and layout timers cannot unlock a new request", async () => {
    let resolveOld!: (result: false) => void;
    request.mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolveOld = done;
        }),
    );
    collide();
    act(() => renderer.update(createElement(Harness, { sessionId: "b" })));
    request.mockImplementationOnce(() => new Promise(() => {}));
    collide();
    await act(async () => resolveOld(false));
    await advance(2000);
    expect(current.pagingActive).toBe(true);
    expect(current.pagingFailed).toBe(false);
    act(() => renderer.update(createElement(Harness, { sessionId: "c" })));
    expect(current.pagingActive).toBe(false);
  });

  test("switching during settling clears the timer and exhausted history hides the button", async () => {
    collide();
    await act(async () => {});
    act(() => renderer.update(createElement(Harness, { sessionId: "b" })));
    request.mockImplementationOnce(() => new Promise(() => {}));
    collide();
    await advance(1000);
    expect(current.pagingActive).toBe(true);
    act(() =>
      renderer.update(
        createElement(Harness, { sessionId: "c", canLoadOlder: false }),
      ),
    );
    expect(current.showLoadOlderHint).toBe(false);
    expect(current.pagingActive).toBe(false);
  });
});
