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

/** The inset a landed *end* keeps above the viewport's bottom edge. */
const BOTTOM_INSET = 12;

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

  /** Report the topmost row with any part on screen. */
  function reportRows(top: number) {
    const token = (index: number): ViewToken => ({
      item: ITEMS[index],
      key: String(index),
      index,
      isViewable: true,
    });
    const [partial] = result.current.viewabilityConfigCallbackPairs;
    act(() => {
      partial?.onViewableItemsChanged?.({
        viewableItems: [token(top)],
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
    // list is pinned to the tail and no scroll event has been delivered. ↑ is
    // the question that started the turn on screen — the answer's own end is
    // where the reader already is.
    reportRows(0);
    expect(result.current.visible).toBe(true);
    expect(result.current.previous).toEqual({ kind: "question", question: 1 });
    // Every newer rung is already on screen, so ↓ has nothing to offer.
    expect(result.current.next).toBeNull();
  });

  test("offers both directions once the reader is inside a turn", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    // The top edge is inside the newest answer: ↑ still offers the question
    // that started the turn, and ↓ has nothing newer below.
    reportRows(0);
    expect(result.current.visible).toBe(true);
    expect(result.current.previous).toEqual({ kind: "question", question: 1 });
    expect(result.current.next).toBeNull();

    // Further up, the top edge inside the answer to question-1: ↑ offers the
    // previous answer's end and ↓ the question of the turn on screen.
    reportRows(2);
    expect(result.current.previous).toEqual({ kind: "question", question: 3 });
    expect(result.current.next).toEqual({ kind: "question", question: 1 });
  });

  test("a drag as well as an offset marks the reading position", () => {
    // Native reports the drag; the offset never moves (a short list on screen).
    act(() => renderer!.update(createElement(Harness, { atLatest: false })));
    reportRows(2);
    expect(result.current.visible).toBe(true);
  });

  test("↓ retracts when the reader returns to the tail, ↑ does not", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    reportRows(2);
    expect(result.current.next).not.toBeNull();

    act(() => result.current.onScroll(scrollEvent(0)));
    expect(result.current.next).toBeNull();
    expect(result.current.previous).not.toBeNull();
  });

  test("↑ lands the question that started the turn on screen", () => {
    act(() => {
      result.current.onScroll(scrollEvent(600));
    });
    reportRows(0);

    act(() => result.current.goToPrevious());

    expect(list.scrollToIndex).toHaveBeenCalledTimes(1);
    expect(list.scrollToIndex).toHaveBeenCalledWith({
      index: 1,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });
  });

  test("↑ lands the previous answer at its start while the row has no anchor", () => {
    act(() => {
      result.current.onScroll(scrollEvent(600));
    });
    // The top edge is inside the answer to question-2, so answer-1 is above it.
    reportRows(1);
    expect(result.current.previous).toEqual({ kind: "replyTail", index: 2 });

    act(() => result.current.goToPrevious());

    // A reply the bubble has not measured yet has no tail to aim at: the jump
    // falls back to the row's own start, and the measurement that follows
    // re-issues it (see the anchor tests below).
    expect(list.scrollToIndex).toHaveBeenCalledWith({
      index: 2,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });
  });

  test("↑ lands the previous answer on its last output, once it is measured", () => {
    act(() => {
      result.current.onScroll(scrollEvent(600));
    });
    reportRows(1);
    act(() => result.current.onReplyTailAnchor("answer-1", 240));

    act(() => result.current.goToPrevious());

    // The row is lifted by the anchor's own offset, so the reply's last output
    // — not the reasoning rows it opened with — takes the reading edge.
    expect(list.scrollToIndex).toHaveBeenCalledWith({
      index: 2,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET + 240,
      animated: false,
    });
  });

  test("↓ onto a reply is lifted by its anchor the same way", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    // The top edge is at the oldest question: ↓ walks back down the ladder.
    reportRows(3);
    expect(result.current.next).toEqual({ kind: "replyTail", index: 2 });
    act(() => result.current.onReplyTailAnchor("answer-1", 96));

    act(() => result.current.goToNext());

    expect(list.scrollToIndex).toHaveBeenCalledWith({
      index: 2,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET + 96,
      animated: false,
    });
  });

  test("a jump onto an unmeasured reply is re-issued when the row is measured", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    reportRows(1);
    act(() => result.current.goToPrevious());
    expect(list.scrollToIndex).toHaveBeenLastCalledWith({
      index: 2,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });

    act(() => result.current.onReplyTailAnchor("answer-1", 240));

    // The row landed and was measured: the jump is re-issued against the
    // anchor rather than waiting out its realign window.
    expect(list.scrollToIndex).toHaveBeenCalledTimes(2);
    expect(list.scrollToIndex).toHaveBeenLastCalledWith({
      index: 2,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET + 240,
      animated: false,
    });
  });

  test("a measurement for another row does not drag the reader back", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    reportRows(1);
    act(() => result.current.goToPrevious());
    expect(list.scrollToIndex).toHaveBeenCalledTimes(1);

    // A reply elsewhere re-lays out (a late image, a settled slice): that is
    // not this jump's business, and the reader is not pulled off their landing.
    act(() => result.current.onReplyTailAnchor("answer-2", 800));

    expect(list.scrollToIndex).toHaveBeenCalledTimes(1);
  });

  test("a drag ends the window in which a measurement is taken up", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    reportRows(1);
    act(() => result.current.goToPrevious());
    expect(list.scrollToIndex).toHaveBeenCalledTimes(1);

    // The reader took the viewport by hand: their position is theirs again, so
    // the row's own measurement no longer re-issues anything.
    act(() => result.current.onScrollBeginDrag());
    act(() => result.current.onReplyTailAnchor("answer-1", 240));

    expect(list.scrollToIndex).toHaveBeenCalledTimes(1);
  });

  test("the press after a landed rung steps on instead of re-aligning it", () => {
    act(() => {
      result.current.onScroll(scrollEvent(600));
    });
    // A reply that fills the screen reports the same topmost row at its start
    // and at its end, so the walk itself is what has to move: the rung just
    // landed on is left behind, not offered again.
    reportRows(0);
    act(() => result.current.goToPrevious());
    expect(list.scrollToIndex).toHaveBeenLastCalledWith({
      index: 1,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });

    act(() => result.current.goToPrevious());

    expect(list.scrollToIndex).toHaveBeenCalledTimes(2);
    expect(list.scrollToIndex).toHaveBeenLastCalledWith({
      index: 2,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });
  });

  test("↓ walks back down the rungs the walk came up", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    reportRows(0);
    act(() => result.current.goToPrevious());

    act(() => result.current.goToNext());

    // The turn on screen again, by its end: exactly the rung the walk left.
    expect(list.scrollToIndex).toHaveBeenLastCalledWith({
      index: 0,
      viewPosition: 0,
      viewOffset: BOTTOM_INSET,
      animated: false,
    });
  });

  test("a drag re-anchors the walk to what the viewport reports", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    reportRows(0);
    act(() => result.current.goToPrevious());
    act(() => result.current.goToPrevious());
    expect(list.scrollToIndex).toHaveBeenCalledTimes(2);

    // The reader takes the viewport by hand up to the oldest question: the
    // ladder is exhausted there, whatever the landed rung was.
    reportRows(3);
    act(() => result.current.onScrollBeginDrag());
    act(() => result.current.goToPrevious());

    expect(list.scrollToIndex).toHaveBeenCalledTimes(2);
  });

  test("↓ lands the question of the turn on screen", () => {
    act(() => {
      result.current.onScroll(scrollEvent(600));
    });
    reportRows(2);

    act(() => result.current.goToNext());

    expect(list.scrollToIndex).toHaveBeenCalledWith({
      index: 1,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });
  });

  test("entering another session drops the offer", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    reportRows(2);
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
    reportRows(3);
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
    reportRows(3);
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
    reportRows(3);

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
