import { useCallback, useLayoutEffect, useMemo, useRef } from "react";
import type { FlatList, LayoutChangeEvent, NativeScrollEvent, NativeSyntheticEvent } from "react-native";
import { layout } from "../theme/tokens";

// Navigation unmounts the list. Keep desktop/tab offsets for this app lifetime,
// but never share them with the transient search-results list.
const savedOffsets = new Map<string, number>();

/**
 * The shortest row this list draws: a conversation's 44pt touch target.
 *
 * Only ever a divisor for a row *count* (see `initialBatchForRestore`), never a
 * measurement. Real rows are at least this tall, so the count it produces can
 * only be too large — and too many rows is a frame of wasted work, while too
 * few is the reader's place missing from the list.
 */
const SHORTEST_ROW_HEIGHT = layout.touchTarget;

/** Rows mounted past the reader's place, so the row they were reading arrives with its neighbours. */
const ROWS_PAST_THE_PLACE = 8;

/**
 * How many rows the list must mount before its first frame for the reader's
 * saved place to exist in it, or `undefined` when there is nothing to restore.
 *
 * Returning to a page is not a scroll: the reader asked for the page they left,
 * not to watch it travel there. A `FlatList` cannot be positioned past rows it
 * has not laid out — the native offset is clamped to what exists — so the batch
 * mounted before the first frame is what decides whether the saved offset can
 * be applied at once. `offset + viewport` is the content the offset needs to be
 * legal at all, and dividing by the shortest row over-counts the rows that
 * content takes, so the batch is enough on any page and any text size.
 */
export function initialBatchForRestore(
  offset: number,
  viewport: number,
  rowCount: number,
): number | undefined {
  if (offset <= 0 || rowCount <= 0) return undefined;
  const needed = Math.ceil((offset + viewport) / SHORTEST_ROW_HEIGHT) + ROWS_PAST_THE_PLACE;
  return Math.min(needed, rowCount);
}

/**
 * Where the reader is, not where the newest row is.
 *
 * The app keeps this list mounted under an open conversation (`App.tsx`), so
 * the native offset survives the visit — but the *rows* do not. Both the
 * recency page and the two tree pages redraw whenever the catalogue moves: a
 * message appended anywhere in the phone's set re-ranks the row it belongs to,
 * a run that ends moves its conversation out of "Active", a peer machine's list
 * is re-read after an action on it. A reader who scrolled to a conversation and
 * came back — or who never left, with rows shuffling under the thumb — finds
 * the same pixel offset over different conversations, which is the "it was not
 * where I left it" report.
 *
 * The native anchor is the remedy for rows moving under a list that is already
 * on screen: when the rows above the first visible one change, the view moves
 * its offset to keep that row where it was. It is the same anchor the transcript
 * uses (`useChatScroll`), with one addition for this page — within zero of the
 * top the list stays at the top instead, because at the head of a page ranked by
 * recency the newest row belongs on screen rather than pushed above it.
 *
 * A constant rather than a literal at the call site: the prop's identity is
 * what the native view re-reads, and a fresh object on every render would look
 * like a new anchor to it on every scroll event.
 */
const SESSION_LIST_ANCHOR = { minIndexForVisible: 0, autoscrollToTopThreshold: 0 };

/**
 * The reader's place on a page they are entering, restored without a scroll.
 *
 * A page is rebuilt when the reader switches to it (the screen keys the list by
 * tab), so the offset has to be put back by hand. Two things make that a
 * placement rather than a journey:
 *
 * - the list mounts enough rows to hold the offset (`initialBatchForRestore`),
 *   so the native view can take the saved position on its first layout;
 * - `restore` refuses to move the list until the content can actually hold that
 *   offset. Scrolling to `min(target, content - viewport)` instead — which is
 *   what this did — walks a half-laid-out list down to its end one measured
 *   batch at a time, and every one of those steps is a scroll the reader gets to
 *   watch. That is the "switching tabs scrolls the page to the bottom again"
 *   report; the reader's own place is not on the way there.
 */
export function useSessionListScroll<T>(key: string | null) {
  const listRef = useRef<FlatList<T>>(null);
  const initialOffset = useMemo(() => ({ x: 0, y: key ? savedOffsets.get(key) ?? 0 : 0 }), [key]);
  const pendingRef = useRef<number | null>(null);
  const geometryRef = useRef({ viewport: 0, content: 0 });
  const interactedRef = useRef(false);

  useLayoutEffect(() => {
    pendingRef.current = initialOffset.y > 0 ? initialOffset.y : null;
    geometryRef.current = { viewport: 0, content: 0 };
    interactedRef.current = false;
  }, [initialOffset]);

  const restore = useCallback(() => {
    const target = pendingRef.current;
    const { viewport, content } = geometryRef.current;
    if (target === null || viewport <= 0 || content <= 0) return;
    // Wait for a list that can hold the offset. A short content is one still
    // laying its rows out, and scrolling it anywhere would be a move the reader
    // did not ask for; the mount batch is sized so this lands in one step. The
    // pixel of slack is for the arithmetic, not for the list: content and
    // viewport are fractional and a reader parked exactly at the end sits on
    // `content - viewport`.
    if (content + 1 < target + viewport) return;
    listRef.current?.scrollToOffset({ offset: target, animated: false });
  }, []);

  const onLayout = useCallback((event: LayoutChangeEvent) => {
    geometryRef.current.viewport = event.nativeEvent.layout.height;
    restore();
  }, [restore]);

  const onContentSizeChange = useCallback((_width: number, height: number) => {
    geometryRef.current.content = height;
    restore();
  }, [restore]);

  const onScrollBeginDrag = useCallback(() => {
    // A deliberate gesture always wins over a pending navigation restoration.
    pendingRef.current = null;
    interactedRef.current = true;
  }, []);

  const onScroll = useCallback((event: NativeSyntheticEvent<NativeScrollEvent>) => {
    const offset = Math.max(0, event.nativeEvent.contentOffset.y);
    const { viewport, content } = geometryRef.current;
    if (pendingRef.current !== null && viewport > 0 && content >= viewport + pendingRef.current &&
        Math.abs(offset - pendingRef.current) < 1) {
      pendingRef.current = null;
    }
    // Initial mount/layout events must not erase the last user position.
    if (key && interactedRef.current) savedOffsets.set(key, offset);
  }, [key]);

  return {
    listRef,
    initialOffset,
    maintainVisibleContentPosition: SESSION_LIST_ANCHOR,
    onLayout,
    onContentSizeChange,
    onScrollBeginDrag,
    onScroll,
  };
}
