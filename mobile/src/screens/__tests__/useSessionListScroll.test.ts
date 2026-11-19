import { createElement, useLayoutEffect } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { LayoutChangeEvent, NativeScrollEvent, NativeSyntheticEvent } from "react-native";
import { initialBatchForRestore, useSessionListScroll } from "../useSessionListScroll";
import { layout } from "../../theme/tokens";

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

test("never walks a half-laid-out list to its end on the way back", () => {
  // VirtualizedList grows its content one batch at a time, so the offset it
  // reports is short of the reader's place for the first frames. Scrolling to
  // what that content can hold is what moved the list to its top and then down
  // to its bottom, one batch per step — the whole of the reported "it scrolls
  // itself back to the end". Nothing may move until the offset is legal.
  remember(1200);
  remount();
  act(() => current.onLayout(layoutEvent(600)));
  act(() => current.onContentSizeChange(320, 500));
  expect(scrollToOffset).not.toHaveBeenCalled();
  act(() => current.onContentSizeChange(320, 900));
  expect(scrollToOffset).not.toHaveBeenCalled();
  act(() => current.onContentSizeChange(320, 1600));
  expect(scrollToOffset).not.toHaveBeenCalled();
  act(() => current.onContentSizeChange(320, 1800));
  expect(scrollToOffset).toHaveBeenLastCalledWith({ offset: 1200, animated: false });
  act(() => current.onScroll(scrollEvent(1200)));
  scrollToOffset.mockClear();
  // The reader's place, and nothing after it: later batches add rows below the
  // offset and must not move the view.
  act(() => current.onContentSizeChange(320, 2400));
  expect(scrollToOffset).not.toHaveBeenCalled();
});

test("a list shorter than the reader's place stays where it is instead of jumping", () => {
  // Rows can go between two visits — a conversation deleted, a page windowed
  // again. The saved offset is then nowhere to be found, and the list says so by
  // not moving at all rather than by travelling down to the new end.
  remember(1200);
  remount();
  act(() => {
    current.onLayout(layoutEvent(600));
    current.onContentSizeChange(320, 1000);
  });
  expect(scrollToOffset).not.toHaveBeenCalled();
});

test("a reader parked at the end comes back to the end, fractional geometry and all", () => {
  // The common case on every page: the reader went to the bottom. Their offset
  // is `content - viewport`, so it is the last position the content can hold —
  // and content sizes and viewports are fractions, not whole pixels.
  remember(1000);
  remount();
  act(() => {
    current.onLayout(layoutEvent(600.4));
    current.onContentSizeChange(320, 1600.2);
  });
  expect(scrollToOffset).toHaveBeenLastCalledWith({ offset: 1000, animated: false });
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

test("hands the native view one anchor that keeps the reader's row in place", () => {
  // Both pages re-rank themselves while the reader sits on them, so the rows
  // above the viewport move without any navigation at all. The anchor is what
  // keeps the first visible row where the reader left it, and it stays at the
  // top only when they are already there — a new conversation then belongs on
  // screen, not pushed above it.
  expect(current.maintainVisibleContentPosition).toEqual({
    minIndexForVisible: 0,
    autoscrollToTopThreshold: 0,
  });
  const anchor = current.maintainVisibleContentPosition;
  remember(1200);
  act(() => current.onScroll(scrollEvent(1240)));
  // One object for the app's lifetime: the native view re-reads this prop, so a
  // fresh object per render would look like a new anchor on every scroll event.
  expect(current.maintainVisibleContentPosition).toBe(anchor);
  remount();
  expect(current.maintainVisibleContentPosition).toBe(anchor);
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

test("the batch a restored page mounts is sized to hold the reader's place", () => {
  // The list cannot be positioned past rows it has not laid out, so the offset
  // is only restorable without a scroll when the first batch already reaches
  // `offset + viewport`. Rows are at least a touch target tall, so dividing by
  // that over-counts — the point, since a short batch is a visible scroll.
  expect(initialBatchForRestore(0, 800, 500)).toBeUndefined();
  expect(initialBatchForRestore(1200, 800, 500))
    .toBe(Math.ceil((1200 + 800) / layout.touchTarget) + 8);
  // A screen's worth of rows is more than a list's viewport, so the same offset
  // on a short phone still asks for at least that many.
  expect(initialBatchForRestore(1200, 400, 500)!).toBeGreaterThan(8);
  // Never more rows than the page holds, and nothing at all for an empty one.
  expect(initialBatchForRestore(1200, 800, 12)).toBe(12);
  expect(initialBatchForRestore(1200, 800, 0)).toBeUndefined();
});
