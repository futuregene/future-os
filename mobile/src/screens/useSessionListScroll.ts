import { useCallback, useLayoutEffect, useMemo, useRef } from "react";
import type { FlatList, LayoutChangeEvent, NativeScrollEvent, NativeSyntheticEvent } from "react-native";

// Navigation unmounts the list. Keep desktop/tab offsets for this app lifetime,
// but never share them with the transient search-results list.
const savedOffsets = new Map<string, number>();

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
 * The native anchor is the remedy: when the rows above the first visible one
 * change, the view moves its offset to keep that row where it was. It is the
 * same anchor the transcript uses (`useChatScroll`), with one addition for this
 * page — within zero of the top the list stays at the top instead, because at
 * the head of a page ranked by recency the newest row belongs on screen rather
 * than pushed above it.
 *
 * A constant rather than a literal at the call site: the prop's identity is
 * what the native view re-reads, and a fresh object on every render would look
 * like a new anchor to it on every scroll event.
 */
const SESSION_LIST_ANCHOR = { minIndexForVisible: 0, autoscrollToTopThreshold: 0 };

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
    // contentOffset can be clamped to zero before native rows are laid out.
    // Retry as VirtualizedList measures batches, not after an arbitrary delay.
    listRef.current?.scrollToOffset({
      offset: Math.min(target, Math.max(0, content - viewport)),
      animated: false,
    });
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
