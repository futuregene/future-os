import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { NativeScrollEvent, NativeSyntheticEvent } from "react-native";
import { useTimelinePaging } from "../useTimelinePaging";

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

describe("timeline paging on an inverted list", () => {
  let renderer: ReactTestRenderer | null = null;
  let result: { current: ReturnType<typeof useTimelinePaging> };
  const forwarded = jest.fn();
  const requestOlder = jest.fn<Promise<void>, []>();

  function Harness({
    sessionId = "s1",
    canLoadOlder = true,
    loadingOlder = false,
  }: {
    sessionId?: string;
    canLoadOlder?: boolean;
    loadingOlder?: boolean;
  }): null {
    result.current = useTimelinePaging(
      sessionId,
      canLoadOlder,
      loadingOlder,
      requestOlder,
      forwarded,
    );
    return null;
  }

  beforeEach(() => {
    jest.useFakeTimers();
    requestOlder.mockResolvedValue(undefined);
    result = { current: undefined as never };
    act(() => {
      renderer = create(React.createElement(Harness));
    });
  });

  afterEach(() => {
    if (renderer) act(() => renderer!.unmount());
    renderer = null;
    jest.useRealTimers();
    forwarded.mockReset();
    requestOlder.mockReset();
  });

  test("latest offset is not mistaken for the older-history edge", () => {
    act(() => result.current.onScroll(scrollEvent(0)));
    act(() => jest.advanceTimersByTime(350));

    expect(forwarded).toHaveBeenCalledTimes(1);
    expect(result.current.showLoadOlderHint).toBe(false);
    expect(requestOlder).not.toHaveBeenCalled();
  });

  test("automatically requests once after settling at the visual top", async () => {
    act(() => result.current.onScroll(scrollEvent(1_400)));
    expect(result.current.showLoadOlderHint).toBe(true);
    await act(async () => jest.advanceTimersByTime(350));
    expect(requestOlder).toHaveBeenCalledTimes(1);

    // Remaining at the same edge cannot cascade through every page. Leaving
    // and returning is the explicit gesture that re-arms one automatic page.
    act(() => result.current.onScroll(scrollEvent(1_400)));
    await act(async () => jest.advanceTimersByTime(350));
    expect(requestOlder).toHaveBeenCalledTimes(1);
    act(() => result.current.onScroll(scrollEvent(100)));
    act(() => result.current.onScroll(scrollEvent(1_400)));
    await act(async () => jest.advanceTimersByTime(350));
    expect(requestOlder).toHaveBeenCalledTimes(2);
  });

  test("leaving the older edge cancels the delayed automatic load", () => {
    act(() => result.current.onScroll(scrollEvent(1_400)));
    act(() => result.current.onScroll(scrollEvent(100)));
    act(() => jest.advanceTimersByTime(350));

    expect(result.current.showLoadOlderHint).toBe(false);
    expect(requestOlder).not.toHaveBeenCalled();
  });

  test("does not request while loading or when no older page exists", () => {
    act(() => renderer!.update(React.createElement(Harness, { loadingOlder: true })));
    act(() => result.current.loadOlder());
    act(() => renderer!.update(React.createElement(Harness, { canLoadOlder: false })));
    act(() => result.current.loadOlder());
    expect(requestOlder).not.toHaveBeenCalled();
  });

  test("switching sessions clears a pending edge load", () => {
    act(() => result.current.onScroll(scrollEvent(1_400)));
    expect(result.current.showLoadOlderHint).toBe(true);
    act(() => renderer!.update(React.createElement(Harness, { sessionId: "s2" })));
    expect(result.current.showLoadOlderHint).toBe(false);
    act(() => jest.advanceTimersByTime(350));
    expect(requestOlder).not.toHaveBeenCalled();
  });
});
