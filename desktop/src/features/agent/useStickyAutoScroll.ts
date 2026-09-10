import type { RefObject } from "react";
import { useCallback, useLayoutEffect, useRef, useState } from "react";

/** Within this many px of the bottom, auto-follow streaming output. */
const STICK_THRESHOLD_PX = 48;
/** Past this distance from the bottom, reveal the "jump to latest" button. */
const JUMP_BUTTON_THRESHOLD_PX = 240;

interface UseStickyAutoScrollInput {
  scrollRef: RefObject<HTMLElement | null>;
  /** Changing this (e.g. the message list) re-runs the follow effect. */
  contentKey: unknown;
  /** False while the real message content is temporarily not mounted. */
  followEnabled?: boolean;
  /** Extra work to run on every scroll event (e.g. floating-scrollbar visibility). */
  onScroll?: () => void;
  /** Run after a content-driven follow settles (e.g. update floating scrollbar). */
  onContentSettled?: () => void;
}

/**
 * Sticky auto-scroll: follow streaming output only while pinned near the bottom.
 * When the user scrolls up past the threshold we stop following (so they can read
 * history) and offer a "jump to latest" button once they're far. Orthogonal to
 * message/run state — depends only on the scroll container and the two keys.
 */
export function useStickyAutoScroll({
  scrollRef,
  contentKey,
  followEnabled = true,
  onScroll,
  onContentSettled,
}: UseStickyAutoScrollInput) {
  const stickToBottomRef = useRef(true);
  const anchorRef = useRef<Anchor | null>(null);
  const writtenTopRef = useRef<number | null>(null);

  const preserveViewport = useCallback(() => {
    stickToBottomRef.current = false;
    anchorRef.current = captureAnchor(scrollRef.current);
  }, [scrollRef]);
  const [showJumpToLatest, setShowJumpToLatest] = useState(false);
  // Keep the callbacks in refs so the effect/handlers always call the latest
  // without listing them as deps (which would re-run the follow effect on every
  // render when the parent passes inline closures).
  const onScrollRef = useRef(onScroll);
  const onContentSettledRef = useRef(onContentSettled);
  onScrollRef.current = onScroll;
  onContentSettledRef.current = onContentSettled;

  // Compose external scroll handling (e.g. floating scrollbar visibility) with
  // sticky detection: re-derive stickiness from the caret's distance to the
  // bottom. A programmatic scroll-to-bottom lands here too, leaving distance ≈ 0
  // → stays stuck; a user scroll-up grows the distance → unsticks. Two
  // thresholds: a tight one to keep following, a looser one to reveal the button.
  const handleScroll = useCallback(() => {
    onScrollRef.current?.();
    const scrollContainer = scrollRef.current;
    if (scrollContainer) {
      const distance
        = scrollContainer.scrollHeight
          - scrollContainer.clientHeight
          - scrollContainer.scrollTop;
      // Ignore our own anchor correction. All other movement (wheel, keyboard,
      // scrollbar or search navigation) establishes a new reading position.
      if (
        writtenTopRef.current === null
        || Math.abs(scrollContainer.scrollTop - writtenTopRef.current) > 0.5
      ) {
        stickToBottomRef.current = distance <= STICK_THRESHOLD_PX;
        anchorRef.current = stickToBottomRef.current
          ? null
          : captureAnchor(scrollContainer);
      }
      writtenTopRef.current = null;
      setShowJumpToLatest(distance > JUMP_BUTTON_THRESHOLD_PX);
    }
  }, [scrollRef]);

  // Jump straight to the latest message and re-enable auto-follow.
  const scrollToLatest = useCallback(() => {
    const scrollContainer = scrollRef.current;
    if (!scrollContainer)
      return;
    stickToBottomRef.current = true;
    anchorRef.current = null;
    setShowJumpToLatest(false);
    scrollContainer.scrollTop = scrollContainer.scrollHeight;
  }, [scrollRef]);

  const settleViewport = useCallback(() => {
    if (!followEnabled)
      return;
    const container = scrollRef.current;
    if (!container)
      return;
    const before = container.scrollTop;
    if (stickToBottomRef.current) {
      container.scrollTop = container.scrollHeight;
    }
    else {
      const anchor = anchorRef.current;
      const target
        = anchor
          && Array.from(
            container.querySelectorAll<HTMLElement>("[data-message-id]"),
          ).find(element => element.dataset.messageId === anchor.id);
      if (target) {
        const delta
          = target.getBoundingClientRect().top
            - container.getBoundingClientRect().top
            - anchor!.offset;
        if (Math.abs(delta) > 0.5)
          container.scrollTop += delta;
      }
      else {
        // A replaced transcript can lose the old ID. Keep the current position
        // and adopt a surviving message; never jump unconditionally to zero.
        anchorRef.current = captureAnchor(container);
      }
    }
    if (container.scrollTop !== before)
      writtenTopRef.current = container.scrollTop;
    const distance
      = container.scrollHeight - container.clientHeight - container.scrollTop;
    setShowJumpToLatest(
      !stickToBottomRef.current && distance > JUMP_BUTTON_THRESHOLD_PX,
    );
    onContentSettledRef.current?.();
  }, [followEnabled, scrollRef]);

  useLayoutEffect(settleViewport, [contentKey, settleViewport]);

  // Keep the same anchor through deferred image/markdown layout, not merely
  // the first React commit. Observe both viewport and content dimensions.
  useLayoutEffect(() => {
    const container = scrollRef.current;
    if (!container || typeof ResizeObserver === "undefined")
      return;
    const observer = new ResizeObserver(settleViewport);
    observer.observe(container);
    for (const child of container.children) observer.observe(child);
    return () => observer.disconnect();
  }, [contentKey, scrollRef, settleViewport]);

  return { handleScroll, scrollToLatest, showJumpToLatest, preserveViewport };
}

interface Anchor {
  id: string;
  offset: number;
}

function captureAnchor(container: HTMLElement | null): Anchor | null {
  if (!container)
    return null;
  const top = container.getBoundingClientRect().top;
  for (const element of container.querySelectorAll<HTMLElement>(
    "[data-message-id]",
  )) {
    const rect = element.getBoundingClientRect();
    if (rect.bottom > top)
      return { id: element.dataset.messageId!, offset: rect.top - top };
  }
  return null;
}
