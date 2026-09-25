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

  function Harness({
    sessionId = "s1",
    atLatest = true,
    items = ITEMS,
  }: {
    sessionId?: string;
    atLatest?: boolean;
    items?: TimelineItem[];
  }): null {
    result.current = useQuestionNav({ sessionId, items, listRef, atLatest });
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
    result = { current: undefined as never };
    act(() => {
      renderer = create(createElement(Harness));
    });
  });

  afterEach(() => {
    if (renderer) act(() => renderer!.unmount());
    renderer = null;
  });

  test("offers nothing while the tail is on screen", () => {
    act(() => result.current.onScroll(scrollEvent(0)));
    reportRows(0, 0);
    expect(result.current.visible).toBe(false);
    expect(result.current.previous).toBeNull();
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

  test("retracts the offer when the reader returns to the tail", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    reportRows(2, null);
    expect(result.current.visible).toBe(true);

    act(() => result.current.onScroll(scrollEvent(0)));
    expect(result.current.visible).toBe(false);
  });

  test("↑ aligns the target question's top edge below the viewport's", () => {
    act(() => {
      result.current.onScroll(scrollEvent(600));
    });
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

  test("a jump out of the measured window is approximated, then re-issued", async () => {
    act(() => {
      result.current.onScroll(scrollEvent(600));
    });
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
    act(() => result.current.onScroll(scrollEvent(600)));
    reportRows(2, null);
    expect(result.current.visible).toBe(true);

    act(() => renderer!.update(createElement(Harness, { sessionId: "s2" })));

    expect(result.current.visible).toBe(false);
    expect(result.current.previous).toBeNull();
  });
});
