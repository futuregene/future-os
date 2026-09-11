import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { NativeScrollEvent, NativeSyntheticEvent } from "react-native";
import { useChatScroll } from "../useChatScroll";

function scrollEvent(y: number): NativeSyntheticEvent<NativeScrollEvent> {
  return {
    nativeEvent: {
      contentOffset: { x: 0, y },
      contentInset: { top: 0, left: 0, bottom: 0, right: 0 },
      contentSize: { width: 320, height: 2_000 },
      layoutMeasurement: { width: 320, height: 600 },
      zoomScale: 1,
    },
  } as NativeSyntheticEvent<NativeScrollEvent>;
}

describe("useChatScroll inverted-list model", () => {
  let renderer: ReactTestRenderer | null = null;
  let result: { current: ReturnType<typeof useChatScroll> };

  function Harness({
    sessionId = "s1",
    itemCount = 0,
  }: {
    sessionId?: string;
    itemCount?: number;
  }): null {
    result.current = useChatScroll(sessionId, itemCount);
    return null;
  }

  beforeEach(() => {
    result = { current: undefined as never };
    act(() => {
      renderer = create(createElement(Harness));
    });
  });

  afterEach(() => {
    if (renderer) act(() => renderer!.unmount());
    renderer = null;
  });

  test("treats the invariant offset zero as latest", () => {
    expect(result.current.atLatest).toBe(true);

    act(() => {
      result.current.onScrollBeginDrag();
      result.current.onScroll(scrollEvent(200));
    });
    expect(result.current.atLatest).toBe(false);

    act(() => result.current.onScroll(scrollEvent(0)));
    expect(result.current.atLatest).toBe(true);
  });

  test("short-list relayout cannot steal initial following or enable native compensation", () => {
    const scrollToOffset = jest.fn();
    (result.current.listRef as { current: unknown }).current = { scrollToOffset };
    act(() => {
      result.current.onScroll(scrollEvent(200));
      result.current.onContentSizeChange();
      result.current.onLayout();
    });
    expect(result.current.atLatest).toBe(true);
    expect(result.current.maintainVisibleContentPosition).toBeUndefined();
    expect(scrollToOffset).toHaveBeenLastCalledWith({ offset: 0, animated: false });
  });

  test("native anchoring owns late layout only while reading history", () => {
    const scrollToOffset = jest.fn();
    (result.current.listRef as { current: unknown }).current = { scrollToOffset };
    act(() => {
      result.current.onScrollBeginDrag();
      result.current.onScroll(scrollEvent(600));
      result.current.onContentSizeChange();
      result.current.onLayout();
    });
    expect(scrollToOffset).not.toHaveBeenCalled();
    expect(result.current.maintainVisibleContentPosition).toEqual({ minIndexForVisible: 0 });
    act(() => result.current.scrollToLatest());
    expect(result.current.maintainVisibleContentPosition).toBeUndefined();
  });

  test("back to latest has one deterministic target", () => {
    const scrollToOffset = jest.fn();
    (result.current.listRef as { current: unknown }).current = { scrollToOffset };
    act(() => {
      result.current.onScrollBeginDrag();
      result.current.onScroll(scrollEvent(200));
    });

    act(() => result.current.scrollToLatest());

    expect(scrollToOffset).toHaveBeenCalledTimes(1);
    expect(scrollToOffset).toHaveBeenCalledWith({ animated: false, offset: 0 });
    expect(result.current.atLatest).toBe(true);
  });

  test("a newly entered session starts at latest without a measurement race", () => {
    act(() => {
      result.current.onScrollBeginDrag();
      result.current.onScroll(scrollEvent(200));
    });
    expect(result.current.atLatest).toBe(false);

    act(() => renderer!.update(createElement(Harness, { sessionId: "s2" })));

    expect(result.current.atLatest).toBe(true);
  });

  test("positions the first real page at latest without animation", () => {
    const scrollToOffset = jest.fn();
    (result.current.listRef as { current: unknown }).current = { scrollToOffset };

    act(() => renderer!.update(createElement(Harness, { itemCount: 20 })));

    expect(scrollToOffset).toHaveBeenCalledTimes(1);
    expect(scrollToOffset).toHaveBeenCalledWith({ animated: false, offset: 0 });
    expect(result.current.atLatest).toBe(true);
  });

  test("does not reset to latest when an older page changes the item count", () => {
    const scrollToOffset = jest.fn();
    (result.current.listRef as { current: unknown }).current = { scrollToOffset };
    act(() => renderer!.update(createElement(Harness, { itemCount: 20 })));
    scrollToOffset.mockClear();
    act(() => {
      result.current.onScrollBeginDrag();
      result.current.onScroll(scrollEvent(600));
    });

    act(() => renderer!.update(createElement(Harness, { itemCount: 40 })));

    expect(scrollToOffset).not.toHaveBeenCalled();
    expect(result.current.atLatest).toBe(false);
  });
});
