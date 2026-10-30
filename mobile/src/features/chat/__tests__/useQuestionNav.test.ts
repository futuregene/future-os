import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type {
  FlatList,
  NativeScrollEvent,
  NativeSyntheticEvent,
} from "react-native";
import type { TimelineItem } from "../../../remote/types";
import { JUMP_VIEW_OFFSET, useQuestionNav, type ViewToken } from "../useQuestionNav";

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

// View order, newest first: answer 2, question 2, answer 1, question 1. The
// walk's own list is the two prompts, in view space: [1, 3].
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
    // the prompt that started the turn on screen — the newest reply's own end
    // is where the reader already is, and is never an ↑ destination.
    reportRows(0);
    expect(result.current.visible).toBe(true);
    expect(result.current.previous).toBe(1);
    // Every newer row is already on screen, so ↓ has nothing to offer.
    expect(result.current.next).toBeNull();
  });

  test("offers both directions once the reader is inside a turn", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    // The top edge is inside the newest answer: ↑ still offers the prompt that
    // started the turn, and ↓ has nothing newer below.
    reportRows(0);
    expect(result.current.visible).toBe(true);
    expect(result.current.previous).toBe(1);
    expect(result.current.next).toBeNull();

    // Further up, the top edge inside the answer to question-1: ↑ offers the
    // prompt above the reader and ↓ the prompt of the turn on screen.
    reportRows(2);
    expect(result.current.previous).toBe(3);
    expect(result.current.next).toBe(1);
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

  test("↑ steps over the answer between two prompts", () => {
    act(() => {
      result.current.onScroll(scrollEvent(600));
    });
    // The top edge is at question-2: the answer to question-1 is the next thing
    // up the transcript, and ↑ passes it by — the press is for prompts. ↓ from
    // here has nothing *newer* that is a prompt.
    reportRows(1);
    expect(result.current.previous).toBe(3);
    expect(result.current.next).toBeNull();

    act(() => result.current.goToPrevious());

    expect(list.scrollToIndex).toHaveBeenCalledTimes(1);
    expect(list.scrollToIndex).toHaveBeenCalledWith({
      index: 3,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });
  });

  test("↓ lands the next prompt, skipping the answer between them", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    // The top edge is at the oldest question: ↓ walks up towards newer prompts.
    reportRows(3);
    expect(result.current.next).toBe(1);

    act(() => result.current.goToNext());

    // The answer to question-1 sits between the two prompts and is passed in
    // this one press: the landing is the next prompt's own start.
    expect(list.scrollToIndex).toHaveBeenCalledWith({
      index: 1,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });
  });

  test("the press after a landed rung walks on instead of re-aligning it", () => {
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
    // The next prompt up, not the answer in between.
    expect(list.scrollToIndex).toHaveBeenLastCalledWith({
      index: 3,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });
  });

  test("↓ then ↑ returns to the prompt the reader came from", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    // Up the transcript first, one press per prompt…
    reportRows(0);
    act(() => result.current.goToPrevious());
    act(() => result.current.goToPrevious());
    expect(list.scrollToIndex).toHaveBeenLastCalledWith({
      index: 3,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });

    // …then down and back: the pair is one walk, so ↓ reaches the prompt it came
    // past and ↑ returns to the one below it.
    act(() => result.current.goToNext());
    expect(list.scrollToIndex).toHaveBeenLastCalledWith({
      index: 1,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });
    act(() => result.current.goToPrevious());
    expect(list.scrollToIndex).toHaveBeenLastCalledWith({
      index: 3,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });
  });

  test("↓ from a landed prompt reaches the next prompt, not the answer below", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    reportRows(3);
    act(() => result.current.goToNext());

    // One press up from the oldest prompt: the answer between them is passed.
    expect(list.scrollToIndex).toHaveBeenLastCalledWith({
      index: 1,
      viewPosition: 1,
      viewOffset: JUMP_VIEW_OFFSET,
      animated: false,
    });
  });

  test("a drag re-anchors the walk to what the viewport reports", () => {
    act(() => result.current.onScroll(scrollEvent(600)));
    reportRows(0);
    act(() => result.current.goToPrevious());
    act(() => result.current.goToPrevious());
    expect(list.scrollToIndex).toHaveBeenCalledTimes(2);

    // The reader takes the viewport by hand up to the oldest question: the walk
    // is exhausted there, whatever the landed row was.
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
    // The page lands the hook's jump on the first id it returned, once the new
    // items are visible. The harness keeps ITEMS, so the id lookup misses and
    // nothing scrolls — paging itself is what this pins.
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

  test("a paged ↑ lands on the oldest turn the page prepended", async () => {
    const olderItems = [
      { id: "older-q", kind: "message", role: "user", text: "…" },
      { id: "older-a", kind: "message", role: "assistant", text: "…" },
      ...ITEMS,
    ] as unknown as TimelineItem[];
    const loadOlder = jest.fn(async (): Promise<false | string[]> => ["older-q", "older-a"]);
    act(() => {
      renderer!.update(createElement(Harness, { hasOlderHistory: true, loadOlder }));
      result.current.onScroll(scrollEvent(600));
    });
    reportRows(3);

    act(() => result.current.goToPrevious());
    // The page commits: the hook sees the new items and lands on the first id
    // the page prepended — the oldest turn, not the newest one it added.
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

  test("a page that fails releases the reader's ↑ instead of wedging it", async () => {
    const loadOlder = jest.fn(async (): Promise<false | string[]> => {
      throw new Error("history_unavailable");
    });
    act(() => {
      renderer!.update(createElement(Harness, { hasOlderHistory: true, loadOlder }));
      result.current.onScroll(scrollEvent(600));
    });
    reportRows(3);

    act(() => result.current.goToPrevious());
    await act(async () => {});
    expect(loadOlder).toHaveBeenCalledTimes(1);

    // The pull is not left in flight: the next press asks again rather than
    // being swallowed by a flag nobody clears.
    act(() => result.current.goToPrevious());
    expect(loadOlder).toHaveBeenCalledTimes(2);
  });

  test("a landing outside the measured window is estimated, then re-issued while it settles", () => {
    jest.useFakeTimers();
    try {
      act(() => result.current.onScroll(scrollEvent(600)));
      reportRows(3);
      act(() => result.current.goToNext());
      const target = list.scrollToIndex.mock.calls.at(-1)![0] as { index: number };
      expect(list.scrollToIndex).toHaveBeenCalledTimes(1);

      // The list cannot scroll to a row it has not laid out, so it says so and
      // the hook moves to where the average row height puts the target.
      act(() => result.current.onScrollToIndexFailed({
        index: target.index, averageItemLength: 120,
      }));
      expect(list.scrollToOffset).toHaveBeenCalledWith({ offset: 120 * target.index, animated: false });
      // The row is still unmeasured, so the window in which its measurement is
      // taken up restarts: one more landing pass, then no more.
      act(() => { jest.advanceTimersByTime(320); });
      const settleCalls = list.scrollToIndex.mock.calls.length;
      expect(settleCalls).toBeGreaterThan(1);

      // A failure for a row that is not the jump's target is not this jump's
      // business: nothing moves and nothing is re-armed.
      act(() => result.current.onScrollToIndexFailed({
        index: target.index + 1, averageItemLength: 120,
      }));
      expect(list.scrollToIndex).toHaveBeenCalledTimes(settleCalls);
      expect(list.scrollToOffset).toHaveBeenCalledTimes(1);
    } finally {
      jest.useRealTimers();
    }
  });

  test("an endless run of failed scrolls stops asking for more landings", () => {
    jest.useFakeTimers();
    try {
      act(() => result.current.onScroll(scrollEvent(600)));
      reportRows(3);
      act(() => result.current.goToNext());
      const target = list.scrollToIndex.mock.calls.at(-1)![0] as { index: number };

      // A row that never measures (a very long reply) would otherwise re-issue
      // forever; the attempts are capped and the walk gives up.
      for (let attempt = 0; attempt < 8; attempt += 1) {
        act(() => result.current.onScrollToIndexFailed({
          index: target.index, averageItemLength: 120,
        }));
        act(() => { jest.advanceTimersByTime(320); });
      }
      const settled = list.scrollToIndex.mock.calls.length;
      act(() => result.current.onScrollToIndexFailed({
        index: target.index, averageItemLength: 120,
      }));
      expect(list.scrollToIndex).toHaveBeenCalledTimes(settled);
    } finally {
      jest.useRealTimers();
    }
  });
});
