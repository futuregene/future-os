import { createElement, useLayoutEffect } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { LayoutChangeEvent, NativeScrollEvent, NativeSyntheticEvent } from "react-native";
import { useSessionListScroll } from "../useSessionListScroll";

const scrollEvent = (y: number) => ({ nativeEvent: { contentOffset: { y } } }) as NativeSyntheticEvent<NativeScrollEvent>;
const layoutEvent = (height: number) => ({ nativeEvent: { layout: { height } } }) as LayoutChangeEvent;
let tree: ReactTestRenderer;
let current: ReturnType<typeof useSessionListScroll>;
let key: string;
const scrollToOffset = jest.fn();
function Harness({ listKey }: { listKey: string | null }) {
  const result = useSessionListScroll(listKey);
  useLayoutEffect(() => { current = result; });
  return null;
}
function mount(listKey: string | null = key) {
  act(() => { tree = create(createElement(Harness, { listKey })); });
  current.listRef.current = { scrollToOffset } as unknown as NonNullable<typeof current.listRef.current>;
}
function remember(y: number) {
  act(() => {
    current.onScrollBeginDrag();
    current.onScroll(scrollEvent(y));
  });
}
function remount() {
  act(() => tree.unmount());
  mount();
}
beforeEach(() => {
  key = expect.getState().currentTestName!;
  scrollToOffset.mockClear();
  mount();
});
afterEach(() => act(() => tree.unmount()));

test("restores after both viewport and native content exist, ignoring the initial zero event", () => {
  remember(1200);
  remount();
  expect(current.initialOffset).toEqual({ x: 0, y: 1200 });
  act(() => {
    // RN may report the requested prop before measuring, then clamp it to 0.
    current.onScroll(scrollEvent(1200));
    current.onScroll(scrollEvent(0));
    current.onLayout(layoutEvent(600));
  });
  expect(scrollToOffset).not.toHaveBeenCalled();
  act(() => current.onContentSizeChange(320, 3000));
  expect(scrollToOffset).toHaveBeenLastCalledWith({ offset: 1200, animated: false });
  act(() => current.onScroll(scrollEvent(1200)));
  scrollToOffset.mockClear();
  act(() => current.onContentSizeChange(320, 3100));
  expect(scrollToOffset).not.toHaveBeenCalled();
  remount();
  expect(current.initialOffset.y).toBe(1200);
});

test("retries clamped restoration as virtualized batches grow, in either layout order", () => {
  remember(1200);
  remount();
  act(() => {
    current.onContentSizeChange(320, 800);
    current.onLayout(layoutEvent(600));
    current.onScroll(scrollEvent(200));
  });
  expect(scrollToOffset).toHaveBeenLastCalledWith({ offset: 200, animated: false });
  act(() => current.onContentSizeChange(320, 3000));
  expect(scrollToOffset).toHaveBeenLastCalledWith({ offset: 1200, animated: false });
  remount();
  expect(current.initialOffset.y).toBe(1200);
});

test("a deliberate gesture cancels restoration, including when rows were deleted", () => {
  remember(1200);
  remount();
  remember(100);
  act(() => {
    current.onLayout(layoutEvent(600));
    current.onContentSizeChange(320, 3000);
  });
  expect(scrollToOffset).not.toHaveBeenCalled();
  remount();
  expect(current.initialOffset.y).toBe(100);
});

test("separates desktops and tabs and does not replace browse offsets with search offsets", () => {
  remember(1200);
  act(() => tree.update(createElement(Harness, { listKey: `${key}:other-desktop` })));
  expect(current.initialOffset.y).toBe(0);
  remember(400);
  act(() => tree.update(createElement(Harness, { listKey: `${key}:workspace` })));
  expect(current.initialOffset.y).toBe(0);
  remember(600);
  act(() => tree.update(createElement(Harness, { listKey: null })));
  expect(current.initialOffset.y).toBe(0);
  remember(80);
  act(() => tree.update(createElement(Harness, { listKey: key })));
  expect(current.initialOffset.y).toBe(1200);
  act(() => tree.update(createElement(Harness, { listKey: `${key}:workspace` })));
  expect(current.initialOffset.y).toBe(600);
});

test("cold or empty lists do not issue scroll commands or store bounce offsets", () => {
  act(() => {
    current.onLayout(layoutEvent(600));
    current.onContentSizeChange(320, 0);
    current.onScroll(scrollEvent(0));
  });
  expect(scrollToOffset).not.toHaveBeenCalled();
  remember(-30);
  remount();
  expect(current.initialOffset.y).toBe(0);
});
