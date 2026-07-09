import type { PointerEvent as ReactPointerEvent } from "react";
import { useCallback, useEffect, useRef, useState } from "react";

export interface FloatingScrollbarState {
  /** Thumb height in px (0 when the content doesn't overflow). */
  height: number;
  /** Thumb offset from the top of the track, in px. */
  top: number;
  visible: boolean;
}

/** Gap between the thumb and the track edges. */
const INSET = 4;
/** Smallest thumb so it stays grabbable on long content. */
const MIN_THUMB = 36;
/** How long the thumb lingers after the last scroll. */
const HIDE_DELAY_MS = 1200;

/**
 * A custom overlay scrollbar shared across scroll regions (chat transcript, the
 * left conversation list). The native bar is hidden via `.floating-scrollbar`;
 * this drives a thin thumb (see {@link ../components/ui/FloatingScrollbar}) that
 * shows on scroll/hover and can be dragged.
 *
 * Usage: put `scrollRef` on a `.floating-scrollbar overflow-y-auto` element,
 * wire `onScroll={handleScroll}`, wrap it in a `group relative` container, and
 * render `<FloatingScrollbar scrollbar={scrollbar} onPointerDown={handleThumbPointerDown} />`.
 */
export function useFloatingScrollbar() {
  const scrollRef = useRef<HTMLDivElement>(null);
  const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Detach for an in-progress thumb drag, so unmounting mid-drag (e.g. switching
  // threads while dragging) removes the document listeners instead of leaking
  // them and calling setState on the unmounted hook.
  const dragCleanupRef = useRef<(() => void) | null>(null);
  const [scrollbar, setScrollbar] = useState<FloatingScrollbarState>({ height: 0, top: 0, visible: false });

  const updateFloatingScrollbar = useCallback((visible: boolean) => {
    const scrollContainer = scrollRef.current;
    if (!scrollContainer)
      return;

    const { clientHeight, scrollHeight, scrollTop } = scrollContainer;
    const canScroll = scrollHeight > clientHeight;
    const height = canScroll
      ? Math.max(MIN_THUMB, (clientHeight / scrollHeight) * (clientHeight - INSET * 2))
      : 0;
    const maxTop = clientHeight - INSET * 2 - height;
    const top = canScroll ? INSET + (scrollTop / (scrollHeight - clientHeight)) * maxTop : INSET;

    setScrollbar({ height, top, visible: visible && canScroll });
  }, []);

  const handleScroll = useCallback(() => {
    updateFloatingScrollbar(true);
    if (hideTimerRef.current)
      clearTimeout(hideTimerRef.current);
    hideTimerRef.current = setTimeout(updateFloatingScrollbar, HIDE_DELAY_MS, false);
  }, [updateFloatingScrollbar]);

  const handleThumbPointerDown = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    const scrollContainer = scrollRef.current;
    if (!scrollContainer)
      return;
    event.preventDefault();

    const { clientHeight, scrollHeight } = scrollContainer;
    const scrollable = scrollHeight - clientHeight;
    const thumbHeight = Math.max(MIN_THUMB, (clientHeight / scrollHeight) * (clientHeight - INSET * 2));
    const maxTop = clientHeight - INSET * 2 - thumbHeight;
    if (scrollable <= 0 || maxTop <= 0)
      return;

    const startY = event.clientY;
    const startScrollTop = scrollContainer.scrollTop;

    // Map thumb travel back to scrollTop; the resulting scroll re-fires
    // handleScroll to keep the thumb visible.
    const onMove = (moveEvent: PointerEvent) => {
      scrollContainer.scrollTop = startScrollTop + ((moveEvent.clientY - startY) / maxTop) * scrollable;
      updateFloatingScrollbar(true);
    };
    const onUp = () => {
      document.removeEventListener("pointermove", onMove);
      document.removeEventListener("pointerup", onUp);
      dragCleanupRef.current = null;
    };
    document.addEventListener("pointermove", onMove);
    document.addEventListener("pointerup", onUp);
    dragCleanupRef.current = onUp;
  }, [updateFloatingScrollbar]);

  useEffect(() => {
    const container = scrollRef.current;
    updateFloatingScrollbar(false);

    // Recompute thumb geometry (silently — don't force the thumb visible) when
    // the viewport changes without a scroll: window resize, left-panel drag
    // (container box changes), or content grow/shrink (list add/remove changes
    // scrollHeight without resizing the fixed-height container, so also watch
    // its content wrapper). Otherwise a hover reveals a stale-sized thumb.
    const recompute = () => updateFloatingScrollbar(false);
    const observer = new ResizeObserver(recompute);
    if (container) {
      observer.observe(container);
      if (container.firstElementChild)
        observer.observe(container.firstElementChild);
    }
    window.addEventListener("resize", recompute);

    return () => {
      observer.disconnect();
      window.removeEventListener("resize", recompute);
      if (hideTimerRef.current)
        clearTimeout(hideTimerRef.current);
    };
  }, [updateFloatingScrollbar]);

  // Detach any in-progress drag on unmount (the normal path detaches on pointerup).
  useEffect(() => () => dragCleanupRef.current?.(), []);

  return { scrollRef, scrollbar, updateFloatingScrollbar, handleScroll, handleThumbPointerDown };
}
