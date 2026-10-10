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

// View order, newest first: answer 2, question 2, answer 1, question 1.
const ITEMS = [
  { id: "answer-2", kind: "message", role: "assistant", text: "…" },
  { id: "question-2", kind: "message", role: "user", text: "…" },
  { id: "answer-1", kind: "message", role: "assistant", text: "…" },
  { id: "question-1", kind: "message", role: "user", text: "…" },
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
    hasOlderHistory = false,
    loadingOlder = false,
    loadOlder,
  }: {
    sessionId?: string;
    atLatest?: boolean;
    items?: TimelineItem[];
    hasOlderHistory?: boolean;
    loadingOlder?: boolean;
    loadOlder?: () => Promise<false | string[]>;
  }): null {
    result.current = useQuestionNav({
      sessionId,
      items,
      listRef,
      atLatest,
      onTakeOver,
      hasOlderHistory,
      loadingOlder,
      loadOlder,
    });
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
    // list is pinned to the tail and no scroll event has been delivered. ↑
    // is the reply tail of the turn on screen.
    reportRows(0, null);
    expect(result.current.visible).toBe(true);
    expect(result.current.previous).toEqual({
      kind: "replyTail",
      index: 0,
      nextQuestion: null,
    });
    // Every newer rung is already on screen, so ↓ has nothing to offer.
    expect(result.current.next).toBeNull();
  });

  test("offers both directions once the reader is inside a turn", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    // The top edge is inside the newest answer, which is not fully visible:
    // ↑ offers the newest reply's tail first.
    reportRows(0, null);
    expect(result.current.visible).toBe(true);
    expect(result.current.previous).toEqual({
      kind: "replyTail",
      index: 0,
      nextQuestion: null,
    });
    expect(result.current.next).toBeNull();

    // Further up: the top edge cuts the first answer, so ↑ offers its tail
    // and ↓ the newest turn's own question end.
    reportRows(2, null);
    expect(result.current.previous).toEqual({
      kind: "replyTail",
      index: 2,
      nextQuestion: 1,
    });
    expect(result.current.next).toEqual({
      kind: "questionEnd",
      question: 1,
      tail: 0,
    });
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
    expect(result.current.next).not.toBeNull();

    act(() => result.current.onScroll(scrollEvent(0)));
    expect(result.current.next).toBeNull();
    expect(result.current.previous).not.toBeNull();
  });

  test("↑ aligns the reply tail's end by landing on the row below it", () => {
    act(() => {
      result.current.onScroll(scrollEvent(600));
    });
    // Reading at question-2's start, whole: ↑ offers answer-1's tail, which
    // lands through question-2 — the row below the tail.
    reportRows(1, 1);

    act(() => result.current.goToPrevious());

    expect(list.scrollToIndex).toHaveBeenCalledTimes(1);
    expect(list.scrollToIndex).toHaveBeenCalledWith({
      index: 1,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });
  });

  test("↑ lands a reply tail by aligning the row below it", () => {
    act(() => {
      result.current.onScroll(scrollEvent(600));
    });
    // Inside answer-2, cut at the top: the press offers its own tail, which
    // lands through the transcript's last row.
    reportRows(0, null);

    act(() => result.current.goToPrevious());

    expect(list.scrollToIndex).toHaveBeenCalledWith({
      index: 3,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });
  });

  test("a reply tail in a later turn aligns the question below it", () => {
    act(() => {
      result.current.onScroll(scrollEvent(600));
    });
    // Top edge at question-1, whole: ↓ offers question-1's own end, which
    // aligns answer-1 — the row below the question's tail.
    reportRows(3, 3);

    act(() => result.current.goToNext());

    expect(list.scrollToIndex).toHaveBeenCalledWith({
      index: 1,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });
  });

  test("a question end lands through its reply's start", () => {
    act(() => {
      result.current.onScroll(scrollEvent(600));
    });
    // Top edge inside answer-1, cut: ↓ walks to question-2's own end, which
    // aligns answer-2 — the row below the question's tail.
    reportRows(2, null);
    expect(result.current.next).toEqual({
      kind: "questionEnd",
      question: 1,
      tail: 0,
    });

    act(() => result.current.goToNext());

    expect(list.scrollToIndex).toHaveBeenCalledWith({
      index: 0,
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

  test("↑ at the loaded top with more history pages first, then jumps", async () => {
    const loadOlder = jest.fn(async (): Promise<false | string[]> => ["older-2", "older-1"]);
    act(() => {
      renderer!.update(createElement(Harness, { hasOlderHistory: true, loadOlder }));
      result.current.onScroll(scrollEvent(600));
    });
    // The top edge is at the oldest loaded row: the ladder is exhausted.
    reportRows(3, null);
    expect(result.current.previous).toBeNull();
    expect(result.current.hasPrevious).toBe(true);

    act(() => result.current.goToPrevious());
    expect(loadOlder).toHaveBeenCalledTimes(1);
    // Paging hands the viewport to the reader first: without it the list
    // would follow the tail while the fetch is in flight, and the jump would
    // read as a teleport to the newest message.
    expect(onTakeOver).toHaveBeenCalledTimes(1);
    // The page lands the hook's jump on the oldest id it returned, once the
    // new items are visible. The harness keeps ITEMS, so the id lookup
    // misses and nothing scrolls — paging itself is what this pins.
    await act(async () => {});
    expect(list.scrollToIndex).not.toHaveBeenCalled();
  });

  test("↑ stays inert at the loaded top while a page is in flight", () => {
    act(() => {
      renderer!.update(
        createElement(Harness, { hasOlderHistory: true, loadingOlder: true }),
      );
      result.current.onScroll(scrollEvent(600));
    });
    reportRows(3, null);
    expect(result.current.hasPrevious).toBe(false);

    act(() => result.current.goToPrevious());
    expect(list.scrollToIndex).not.toHaveBeenCalled();
    expect(list.scrollToOffset).not.toHaveBeenCalled();
  });

  test("a paged ↑ lands on the oldest row the page prepended", async () => {
    const olderItems = [
      { id: "older-1", kind: "message", role: "user", text: "…" },
      ...ITEMS,
    ] as unknown as TimelineItem[];
    const loadOlder = jest.fn(async (): Promise<false | string[]> => ["older-1"]);
    act(() => {
      renderer!.update(createElement(Harness, { hasOlderHistory: true, loadOlder }));
      result.current.onScroll(scrollEvent(600));
    });
    reportRows(3, null);

    act(() => result.current.goToPrevious());
    // The page commits: the hook sees the new items and lands on the oldest
    // id the page prepended.
    act(() => {
      renderer!.update(
        createElement(Harness, { hasOlderHistory: true, loadOlder, items: olderItems }),
      );
    });
    await act(async () => {});

    expect(list.scrollToIndex).toHaveBeenCalledWith({
      index: 0,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });
  });
});
