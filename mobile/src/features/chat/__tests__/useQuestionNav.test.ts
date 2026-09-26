import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type {
  FlatList,
  NativeScrollEvent,
  NativeSyntheticEvent,
  ViewToken,
} from "react-native";
import type { TimelineItem } from "../../../remote/types";
import { JUMP_VIEW_OFFSET, useQuestionNav } from "../useQuestionNav";

const scrollEvent = (y: number): NativeSyntheticEvent<NativeScrollEvent> =>
  ({
    nativeEvent: {
      contentOffset: { x: 0, y },
      contentInset: { top: 0, left: 0, bottom: 0, right: 0 },
      contentSize: { width: 390, height: 2_000 },
      layoutMeasurement: { width: 390, height: 700 },
      zoomScale: 1,
    },
  }) as NativeSyntheticEvent<NativeScrollEvent>;

// View order: the newest question is index 1, the oldest index 3.
const ITEMS = [
  { id: "answer-2", kind: "message", role: "assistant" },
  { id: "question-2", kind: "message", role: "user" },
  { id: "answer-1", kind: "message", role: "assistant" },
  { id: "question-1", kind: "message", role: "user" },
] as unknown as TimelineItem[];

describe("useQuestionNav", () => {
  let renderer: ReactTestRenderer | null = null;
  let result: { current: ReturnType<typeof useQuestionNav> };
  const list = {
    scrollToIndex: jest.fn(),
    scrollToOffset: jest.fn(),
  };
  const listRef = { current: list as unknown as FlatList<TimelineItem> };
  const onTakeOver = jest.fn();

  function Harness({
    sessionId = "s1",
    atLatest = true,
    items = ITEMS,
  }: {
    sessionId?: string;
    atLatest?: boolean;
    items?: TimelineItem[];
  }): null {
    result.current = useQuestionNav({ sessionId, items, listRef, atLatest, onTakeOver });
    return null;
  }

  /** Report rows as viewable: `top` partly visible, `fullTop` fully visible. */
  function reportRows(top: number, fullTop: number | null) {
    const token = (index: number): ViewToken => ({
      item: ITEMS[index],
      key: String(index),
      index,
      isViewable: true,
    });
    const [partial, full] = result.current.viewabilityConfigCallbackPairs;
    act(() => {
      partial?.onViewableItemsChanged?.({
        viewableItems: [token(top)],
        changed: [],
      });
      full?.onViewableItemsChanged?.({
        viewableItems: fullTop === null ? [] : [token(fullTop)],
        changed: [],
      });
    });
  }

  beforeEach(() => {
    list.scrollToIndex.mockClear();
    list.scrollToOffset.mockClear();
    onTakeOver.mockClear();
    result = { current: undefined as never };
    act(() => {
      renderer = create(createElement(Harness));
    });
  });

  afterEach(() => {
    if (renderer) act(() => renderer!.unmount());
    renderer = null;
  });

  test("offers ↑ from the tail, before the reader has scrolled at all", () => {
    // The viewability report is all a freshly opened conversation gets: the
    // list is pinned to the tail and no scroll event has been delivered.
    reportRows(0, null);
    expect(result.current.visible).toBe(true);
    expect(result.current.previous).toBe(1);
    // Every newer question is already on screen, so ↓ has nothing to offer.
    expect(result.current.next).toBeNull();
  });

  test("offers both directions once the reader is inside a turn", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    // The top edge is inside the newest answer, which is not fully visible.
    reportRows(0, null);
    expect(result.current.visible).toBe(true);
    expect(result.current.previous).toBe(1);
    expect(result.current.next).toBeNull();

    // Further up: the top edge is inside the first answer.
    reportRows(2, null);
    expect(result.current.previous).toBe(3);
    expect(result.current.next).toBe(1);
  });

  test("a drag as well as an offset marks the reading position", () => {
    // Native reports the drag; the offset never moves (a short list on screen).
    act(() => renderer!.update(createElement(Harness, { atLatest: false })));
    reportRows(2, null);
    expect(result.current.visible).toBe(true);
  });

  test("↓ retracts when the reader returns to the tail, ↑ does not", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    reportRows(2, null);
    expect(result.current.next).toBe(1);

    act(() => result.current.onScroll(scrollEvent(0)));
    expect(result.current.next).toBeNull();
    expect(result.current.previous).toBe(3);
  });

  test("offers nothing when no question is left above the reader", () => {
    // The oldest question is the topmost row on screen: the ladder runs out.
    reportRows(3, 3);
    expect(result.current.visible).toBe(false);
    expect(result.current.previous).toBeNull();
  });

  test("↑ aligns the target question's top edge below the viewport's", () => {
    reportRows(2, null);

    act(() => result.current.goToPrevious());

    expect(list.scrollToIndex).toHaveBeenCalledTimes(1);
    expect(list.scrollToIndex).toHaveBeenCalledWith({
      index: 3,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });
  });

  test("a jump hands the viewport over, so the tail cannot pull it back", () => {
    reportRows(2, null);

    act(() => result.current.goToPrevious());
    expect(onTakeOver).toHaveBeenCalledTimes(1);

    // Nothing to go to: a disabled direction must not move the reader either.
    act(() => result.current.goToNext());
    expect(onTakeOver).toHaveBeenCalledTimes(1);
    expect(list.scrollToIndex).toHaveBeenCalledTimes(1);
  });

  test("a jump out of the measured window is approximated, then re-issued", async () => {
    reportRows(2, null);
    act(() => result.current.goToPrevious());
    list.scrollToIndex.mockClear();

    act(() => result.current.onScrollToIndexFailed({ index: 3, averageItemLength: 80 }));
    expect(list.scrollToOffset).toHaveBeenCalledWith({ offset: 240, animated: false });

    // The re-issue waits for the target row to be laid out, so the test waits
    // for it in real time rather than driving a fake clock.
    await act(() => new Promise(resolve => setTimeout(resolve, 400)));
    expect(list.scrollToIndex).toHaveBeenCalledWith({
      index: 3,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });
  });

  test("entering another session drops the offer", () => {
    reportRows(2, null);
    expect(result.current.visible).toBe(true);

    act(() => renderer!.update(createElement(Harness, { sessionId: "s2" })));

    expect(result.current.visible).toBe(false);
    expect(result.current.previous).toBeNull();
  });

  test("a viewability report ignores rows the list has not indexed or has scrolled past", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    const [partial] = result.current.viewabilityConfigCallbackPairs;
    // Native reports both of these: an unmounted cell has no index, and a cell
    // leaving the window is present-but-not-viewable. Neither may move the
    // reading position, or the arrows would chase off-screen rows.
    act(() => {
      partial?.onViewableItemsChanged?.({
        viewableItems: [
          { item: ITEMS[3], key: "3", index: null, isViewable: true } as unknown as ViewToken,
          { item: ITEMS[1], key: "1", index: 1, isViewable: false },
          { item: ITEMS[2], key: "2", index: 2, isViewable: true },
        ],
        changed: [],
      });
    });
    expect(result.current.previous).toBe(3);
    expect(result.current.next).toBe(1);
  });

  test("a viewability report with nothing viewable leaves the position alone", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    const [partial] = result.current.viewabilityConfigCallbackPairs;
    act(() => {
      partial?.onViewableItemsChanged?.({ viewableItems: [], changed: [] });
    });
    // Nothing was measured, so the ladder is unchanged rather than reset.
    expect(result.current.visible).toBe(false);
    expect(result.current.previous).toBeNull();
  });

  test("a second jump replaces the pending re-align instead of stacking timers", () => {
    reportRows(2, null);
    act(() => result.current.goToPrevious());
    // A second jump while the first is still re-aligning must clear its timer:
    // two pending realigns would fight over the same viewport.
    act(() => result.current.goToPrevious());
    expect(list.scrollToIndex).toHaveBeenCalledTimes(2);
    expect(list.scrollToIndex).toHaveBeenLastCalledWith(expect.objectContaining({ index: 3 }));
    expect(onTakeOver).toHaveBeenCalledTimes(2);
  });

  test("a failed scroll with no jump in flight is ignored", () => {
    reportRows(2, null);
    act(() => result.current.onScrollToIndexFailed({ index: 3, averageItemLength: 80 }));
    expect(list.scrollToOffset).not.toHaveBeenCalled();
  });

  test("a failed scroll for a different index than the pending jump is ignored", async () => {
    reportRows(2, null);
    act(() => result.current.goToPrevious());
    list.scrollToOffset.mockClear();
    // The list retries an older, unrelated index; following it would drag the
    // reader away from the question they asked for.
    act(() => result.current.onScrollToIndexFailed({ index: 1, averageItemLength: 80 }));
    expect(list.scrollToOffset).not.toHaveBeenCalled();
  });

  test("a jump re-aligns once after the list settles", async () => {
    jest.useFakeTimers();
    try {
      reportRows(2, null);
      expect(result.current.previous).toBe(3);
      list.scrollToIndex.mockClear();
      act(() => result.current.goToPrevious());
      expect(list.scrollToIndex).toHaveBeenCalledTimes(1);
      // The first attempt can land short while the list still measures rows;
      // one delayed re-align is what puts the question at its resting offset.
      await act(async () => { jest.advanceTimersByTimeAsync(320); });
      expect(list.scrollToIndex).toHaveBeenCalledTimes(2);
      expect(list.scrollToIndex.mock.calls.at(-1)![0]).toMatchObject({ index: 3 });
      // It is a single correction, not a repeating timer.
      await act(async () => { jest.advanceTimersByTimeAsync(5_000); });
      expect(list.scrollToIndex).toHaveBeenCalledTimes(2);
    } finally { jest.useRealTimers(); }
  });

  test("the re-align retries are capped so a long jump cannot loop forever", async () => {
    reportRows(2, null);
    act(() => result.current.goToPrevious());
    // Each failure re-issues the jump; MAX_JUMP_ATTEMPTS of them are honoured,
    // and the next is dropped rather than scheduling another timer.
    for (const attempt of [1, 2, 3, 4, 5, 6]) {
      act(() => result.current.onScrollToIndexFailed({ index: 3, averageItemLength: 80 }));
      void attempt;
    }
    list.scrollToOffset.mockClear();
    act(() => result.current.onScrollToIndexFailed({ index: 3, averageItemLength: 80 }));
    expect(list.scrollToOffset).not.toHaveBeenCalled();
  });
});

