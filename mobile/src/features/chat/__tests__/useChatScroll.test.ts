import React from "react";
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

  function Harness({ sessionId = "s1" }: { sessionId?: string }): null {
    result.current = useChatScroll(sessionId);
    return null;
  }

  beforeEach(() => {
    result = { current: undefined as never };
    act(() => {
      renderer = create(React.createElement(Harness));
    });
  });

  afterEach(() => {
    if (renderer) act(() => renderer!.unmount());
    renderer = null;
  });

  test("treats the invariant offset zero as latest", () => {
    expect(result.current.atLatest).toBe(true);

    act(() => result.current.onScroll(scrollEvent(200)));
    expect(result.current.atLatest).toBe(false);

    act(() => result.current.onScroll(scrollEvent(0)));
    expect(result.current.atLatest).toBe(true);
  });

  test("back to latest has one deterministic target", () => {
    const scrollToOffset = jest.fn();
    (result.current.listRef as { current: unknown }).current = { scrollToOffset };
    act(() => result.current.onScroll(scrollEvent(200)));

    act(() => result.current.scrollToLatest());

    expect(scrollToOffset).toHaveBeenCalledTimes(1);
    expect(scrollToOffset).toHaveBeenCalledWith({ animated: true, offset: 0 });
    expect(result.current.atLatest).toBe(true);
  });

  test("a newly entered session starts at latest without a measurement race", () => {
    act(() => result.current.onScroll(scrollEvent(200)));
    expect(result.current.atLatest).toBe(false);

    act(() => renderer!.update(React.createElement(Harness, { sessionId: "s2" })));

    expect(result.current.atLatest).toBe(true);
  });
});
