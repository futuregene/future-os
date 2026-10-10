import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type RefObject,
} from "react";
import type {
  FlatList,
  NativeScrollEvent,
  NativeSyntheticEvent,
  ViewabilityConfig,
  ViewabilityConfigCallbackPair,
  ViewToken,
} from "react-native";
import type { TimelineItem } from "../../remote/types";
import { spacing } from "../../theme/tokens";
import {
  EMPTY_ROWS,
  buildLadder,
  nextRung,
  previousRung,
  questionIndices,
  viewportRows,
  type QuestionRung,
  type ViewportRows,
} from "./questionNav";

/** The offset `useChatScroll` already treats as "at the latest". */
const AT_LATEST_THRESHOLD_PX = 32;
/**
 * The geometry a question jump aims at: the landed row sits this far below
 * the viewport's top edge.
 *
 * The list is inverted, so `viewPosition: 1` aligns the target's *cell* with
 * that edge — and an inverted cell lays its separator out above its row
 * (`column-reverse`), which is the list's own inter-row gap. Aligning the cell
 * therefore already leaves `ROW_GAP` above the row, so what a jump passes as
 * `viewOffset` is the difference. Exported so a test can pin the wiring; the
 * harness measures the delivered inset itself.
 */
const QUESTION_TOP_INSET = 12;
const ROW_GAP = spacing.md;
export const JUMP_VIEW_OFFSET = ROW_GAP - QUESTION_TOP_INSET;
/**
 * A jump is only as exact as VirtualizedList's knowledge of the target row. A
 * row outside the rendered window is estimated from the average row height, so
 * the jump lands approximately, renders the target, and is re-issued against the
 * freshly measured frame.
 */
const REALIGN_MS = 320;
/** Bounded so a target that never enters the render window cannot loop. */
const MAX_JUMP_ATTEMPTS = 4;

const PARTLY_VISIBLE: ViewabilityConfig = { itemVisiblePercentThreshold: 1 };
const FULLY_VISIBLE: ViewabilityConfig = { itemVisiblePercentThreshold: 100 };

export interface QuestionNavApi {
  /**
   * The ↑ rung (what a press would do), or null. Offered from the tail as
   * well, where it is the reply tail of the turn on screen.
   */
  previous: QuestionRung | null;
  /**
   * The ↓ rung, or null. Only once the reader has left the tail: at the tail
   * there is nothing newer below.
   */
  next: QuestionRung | null;
  /** Whether ↑ has anywhere to go — a rung, or a page of history to pull. */
  hasPrevious: boolean;
  /** Whether the control has anything to offer. */
  visible: boolean;
  goToPrevious: () => void;
  goToNext: () => void;
  onScroll: (event: NativeSyntheticEvent<NativeScrollEvent>) => void;
  onScrollToIndexFailed: (info: {
    index: number;
    averageItemLength: number;
  }) => void;
  viewabilityConfigCallbackPairs: ViewabilityConfigCallbackPair[];
}

/** The topmost (visually highest) row in a viewable set. */
function topOf(viewableItems: ViewToken[]): number | null {
  let top: number | null = null;
  for (const token of viewableItems) {
    const index = token.index;
    if (index == null || !token.isViewable) continue;
    if (top === null || index > top) top = index;
  }
  return top;
}

/**
 * "↑ older / ↓ newer" for a long transcript. Each press walks one rung of the
 * question ladder: the previous answer's last slice, the previous question,
 * the end of that question, the next answer's last slice… — so the reader can
 * stop on the prose at the end of an answer instead of only ever landing on
 * prompts.
 *
 * Every landing is a `scrollToIndex` against the list's own measured frames:
 * a question aligns its *start* below the viewport's top edge, and a tail
 * (a reply's last slice, or a question's own end) aligns the start of the row
 * *below* it the same way. Because the list is a scaleY-1 mirror, that second
 * alignment leaves the tail's bottom edge flush with the viewport's bottom
 * edge — the question geometry delivers the tail without any inverted-math
 * or pixel measuring of its own.
 *
 * The anchor comes from what the list reports as viewable — partly and fully
 * visible rows — rather than from row geometry: a row's `onLayout` `y` is
 * relative to its own parent, the virtualized cell, so it says nothing about
 * where the row sits in the transcript. ↑ is offered whether or not the reader
 * has scrolled: a conversation opens at its tail, where the tail of the turn
 * on screen is still one press away. ↓ is the direction that needs the tail
 * off screen — with the tail on screen every newer rung is already visible,
 * so there is nothing below to go to. That reading state is the union of the
 * native "a drag took the viewport off the tail" signal owned by
 * `useChatScroll` and the measured offset, because a web build never reports a
 * drag: `onScrollBeginDrag` has no equivalent there.
 */
export function useQuestionNav({
  sessionId,
  items,
  listRef,
  atLatest,
  onTakeOver,
  hasOlderHistory = false,
  loadingOlder = false,
  loadOlder,
}: {
  sessionId: string;
  /** The rows the list renders, in view order (newest first). */
  items: readonly TimelineItem[];
  listRef: RefObject<FlatList<TimelineItem> | null>;
  atLatest: boolean;
  /**
   * A jump is the reader taking the viewport over from the tail, and it has to
   * say so: while the list is still following the tail, the next streaming
   * commit pins it back to the bottom and the jump is gone. `useChatScroll`
   * owns that handover — the same one the load-older hint performs — and it is
   * required so a host cannot wire a jump that silently undoes itself.
   */
  onTakeOver: () => void;
  /** More transcript pages exist above the loaded slice (remote contract). */
  hasOlderHistory?: boolean;
  /** A page is already being fetched. */
  loadingOlder?: boolean;
  /** Ask for the next page up; resolves to the prepended ids, or false. */
  loadOlder?: () => Promise<false | string[]>;
}): QuestionNavApi {
  const questions = useMemo(() => questionIndices(items), [items]);
  const ladder = useMemo(() => buildLadder(items, questions), [items, questions]);
  const [nav, setNav] = useState<{
    sessionId: string;
    previous: QuestionRung | null;
    next: QuestionRung | null;
  }>({ sessionId, previous: null, next: null });

  const sessionIdRef = useRef(sessionId);
  const itemsRef = useRef(items);
  const ladderRef = useRef(ladder);
  const atLatestRef = useRef(atLatest);
  const hasOlderRef = useRef(hasOlderHistory);
  const loadingOlderRef = useRef(loadingOlder);
  const loadOlderRef = useRef(loadOlder);
  const onTakeOverRef = useRef(onTakeOver);
  const rowsRef = useRef<ViewportRows>(EMPTY_ROWS);
  const partialTopRef = useRef<number | null>(null);
  const fullTopRef = useRef<number | null>(null);
  const offsetRef = useRef(0);
  const jumpRef = useRef<{ index: number; attempts: number } | null>(null);
  const pendingPageRef = useRef(false);
  const realignTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const refreshRows = useCallback(() => {
    rowsRef.current = viewportRows({
      partialTop: partialTopRef.current,
      fullTop: fullTopRef.current,
    });
  }, []);

  /** Resolve what ↑/↓ would do right now and publish it if it changed. */
  const sync = useCallback(() => {
    const session = sessionIdRef.current;
    const reading =
      !atLatestRef.current || offsetRef.current > AT_LATEST_THRESHOLD_PX;
    // ↑ reads the viewport rows alone: the rung above the top edge is one the
    // reader can reach, from the tail as much as from anywhere else. ↓ is the
    // tail-dependent half of the ladder.
    const previous = previousRung(ladderRef.current.rungs, rowsRef.current);
    const next = reading
      ? nextRung(ladderRef.current.rungs, rowsRef.current)
      : null;
    setNav(current =>
      current.sessionId === session &&
      current.previous === previous &&
      current.next === next
        ? current
        : { sessionId: session, previous, next },
    );
  }, []);

  useEffect(() => {
    sessionIdRef.current = sessionId;
    itemsRef.current = items;
    ladderRef.current = ladder;
    atLatestRef.current = atLatest;
    hasOlderRef.current = hasOlderHistory;
    loadingOlderRef.current = loadingOlder;
    loadOlderRef.current = loadOlder;
    onTakeOverRef.current = onTakeOver;
  });

  // Another session opens at its tail: whatever the previous transcript had
  // scrolled to says nothing about this one.
  useEffect(() => {
    offsetRef.current = 0;
    partialTopRef.current = null;
    fullTopRef.current = null;
    rowsRef.current = EMPTY_ROWS;
    pendingPageRef.current = false;
    sync();
  }, [sessionId, sync]);

  // A new turn, a page of history or a session switch all change the answer
  // without any scroll or viewability event.
  useEffect(() => {
    sync();
  }, [sync, ladder, atLatest, sessionId]);

  useEffect(
    () => () => {
      if (realignTimerRef.current !== null) clearTimeout(realignTimerRef.current);
    },
    [],
  );

  const onPartlyVisible = useCallback(
    (info: { viewableItems: ViewToken[] }) => {
      partialTopRef.current = topOf(info.viewableItems);
      refreshRows();
      sync();
    },
    [refreshRows, sync],
  );
  const onFullyVisible = useCallback(
    (info: { viewableItems: ViewToken[] }) => {
      fullTopRef.current = topOf(info.viewableItems);
      refreshRows();
      sync();
    },
    [refreshRows, sync],
  );
  // FlatList forbids replacing this prop on the fly and the callbacks above are
  // stable, so one array owns the two configurations for the list's lifetime.
  // (A ref would be the obvious home, but refs may not be read during render,
  // which is where the list needs this prop.)
  const [viewabilityConfigCallbackPairs] = useState<
    ViewabilityConfigCallbackPair[]
  >(() => [
    {
      viewabilityConfig: PARTLY_VISIBLE,
      onViewableItemsChanged: onPartlyVisible,
    },
    { viewabilityConfig: FULLY_VISIBLE, onViewableItemsChanged: onFullyVisible },
  ]);

  /** Scroll `index` into the question position: start just below the top edge. */
  const align = useCallback(
    (index: number) => {
      listRef.current?.scrollToIndex({
        index,
        viewPosition: 1,
        viewOffset: JUMP_VIEW_OFFSET,
        animated: false,
      });
    },
    [listRef],
  );

  /**
   * Land on a tail: align the row *below* it as if it were a question. The
   * scaleY-1 mirror then puts the tail's bottom edge at the viewport's own
   * bottom edge. The row below is by construction closer to the reader than
   * the tail — the question below for a reply tail, the reply's own start for
   * a question end — so it is always measured and the jump never needs
   * `onScrollToIndexFailed`; and the aligned row plus everything between it
   * and the tail is already on screen, so no re-issue is needed either.
   */
  const alignTail = useCallback(
    (below: number) => {
      listRef.current?.scrollToIndex({
        index: below,
        viewPosition: 1,
        viewOffset: JUMP_VIEW_OFFSET,
        animated: false,
      });
    },
    [listRef],
  );

  /** Where a rung lands, as the row whose start takes the question position. */
  const rungTarget = useCallback(
    (rung: QuestionRung): { target: number; tail: boolean } => {
      const lastIndex = ladderRef.current.lastIndex;
      switch (rung.kind) {
        case "question":
          return { target: rung.question, tail: false };
        case "replyTail": {
          // The row below the tail: the question that ends this turn, or —
          // at the newest turn — the transcript's own last row.
          const below = rung.nextQuestion ?? lastIndex;
          return below >= 0
            ? { target: below, tail: true }
            : // No row sits below the tail: the question position is the
              // best the list can offer.
              { target: rung.index, tail: false };
        }
        case "questionEnd": {
          // The row below the question's tail is the reply's start — or, at
          // the loaded slice's top, the last row there is.
          const reply = rung.question - 1;
          const below = rung.tail !== null && rung.tail < rung.question ? rung.tail : reply;
          return below >= 0
            ? { target: below, tail: true }
            : { target: rung.question, tail: false };
        }
      }
    },
    [],
  );

  const jumpTo = useCallback(
    (rung: QuestionRung | null) => {
      if (rung === null || listRef.current === null) return;
      const { target, tail } = rungTarget(rung);
      if (realignTimerRef.current !== null) clearTimeout(realignTimerRef.current);
      onTakeOverRef.current();
      if (tail) {
        // A tail lands through a measured neighbour: exact in one issue.
        alignTail(target);
        jumpRef.current = null;
        return;
      }
      jumpRef.current = { index: target, attempts: 0 };
      align(target);
      realignTimerRef.current = setTimeout(() => align(target), REALIGN_MS);
    },
    [align, alignTail, listRef, rungTarget],
  );

  /**
   * ↑ from the loaded slice's top: pull the next page of history, then land on
   * the oldest turn it contains — which is exactly where the reader was
   * headed. `loadOlder` resolves to the prepended ids, so the oldest of them
   * is the jump's target without any index math across the prepend.
   */
  const pageUp = useCallback(() => {
    if (pendingPageRef.current || loadingOlderRef.current) return;
    const request = loadOlderRef.current;
    if (!request) return;
    // Keep the reading position through the page: without this the list
    // follows the tail (offset 0) while the fetch is in flight, and the
    // landed jump that follows would read as a teleport back to the newest
    // message instead of one step up the transcript.
    onTakeOverRef.current();
    pendingPageRef.current = true;
    void request()
      .then(ids => {
        pendingPageRef.current = false;
        if (!ids || ids.length === 0) return;
        const oldest = ids[ids.length - 1]!;
        const itemsNow = itemsRef.current;
        const index = itemsNow.findIndex(item => item.id === oldest);
        if (index < 0) return;
        jumpRef.current = { index, attempts: 0 };
        align(index);
        if (realignTimerRef.current !== null) clearTimeout(realignTimerRef.current);
        realignTimerRef.current = setTimeout(() => align(index), REALIGN_MS);
      })
      .catch(() => {
        pendingPageRef.current = false;
      });
  }, [align]);

  const goToPrevious = useCallback(() => {
    if (nav.sessionId !== sessionId) return;
    if (nav.previous !== null) {
      jumpTo(nav.previous);
      return;
    }
    // The ladder ran out at the loaded slice's top: the next rung lives in
    // history, and the button's contract is to fetch it rather than stall.
    if (hasOlderRef.current) pageUp();
  }, [jumpTo, nav, pageUp, sessionId]);
  const goToNext = useCallback(() => {
    jumpTo(nav.sessionId === sessionId ? nav.next : null);
  }, [jumpTo, nav, sessionId]);

  /**
   * The target is outside the measured window: move to where the average row
   * height says it is, then re-issue the jump once it has been rendered and
   * measured. Attempts are capped — `realignTimerRef` already re-issues once, so
   * this only covers jumps of many screens.
   */
  const onScrollToIndexFailed = useCallback(
    ({ index, averageItemLength }: { index: number; averageItemLength: number }) => {
      const jump = jumpRef.current;
      if (jump === null || jump.index !== index || jump.attempts >= MAX_JUMP_ATTEMPTS)
        return;
      jump.attempts += 1;
      listRef.current?.scrollToOffset({
        offset: Math.max(0, averageItemLength * index),
        animated: false,
      });
      if (realignTimerRef.current !== null) clearTimeout(realignTimerRef.current);
      realignTimerRef.current = setTimeout(() => align(index), REALIGN_MS);
    },
    [align, listRef],
  );

  const onScroll = useCallback(
    (event: NativeSyntheticEvent<NativeScrollEvent>) => {
      const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
      // A list that fits its viewport cannot be scrolled away from the tail;
      // whatever offset it reports is layout noise, not reading position.
      const fits = contentSize.height <= layoutMeasurement.height + 1;
      offsetRef.current = fits ? 0 : Math.max(0, contentOffset.y);
      sync();
    },
    [sync],
  );

  const inSession = nav.sessionId === sessionId;
  const current = inSession ? nav : { previous: null, next: null };
  // ↑ stays alive one step past the loaded ladder when history is known to
  // continue: the press pages first and lands where the page begins.
  const hasPrevious =
    inSession && (current.previous !== null || (hasOlderHistory && !loadingOlder));
  return {
    previous: current.previous,
    next: current.next,
    hasPrevious,
    visible: hasPrevious || current.next !== null,
    goToPrevious,
    goToNext,
    onScroll,
    onScrollToIndexFailed,
    viewabilityConfigCallbackPairs,
  };
}
