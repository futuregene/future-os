import { useCallback, useLayoutEffect, useMemo, useRef } from "react";
import type { FlatList, LayoutChangeEvent, NativeScrollEvent, NativeSyntheticEvent } from "react-native";

// Navigation unmounts the list. Keep desktop/tab offsets for this app lifetime,
// but never share them with the transient search-results list.
const savedOffsets = new Map<string, number>();

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

  return { listRef, initialOffset, onLayout, onContentSizeChange, onScrollBeginDrag, onScroll };
}
