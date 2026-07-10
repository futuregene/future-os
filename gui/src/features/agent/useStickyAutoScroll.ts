import type { RefObject } from "react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

/** Within this many px of the bottom, auto-follow streaming output. */
const STICK_THRESHOLD_PX = 48;
/** Past this distance from the bottom, reveal the "jump to latest" button. */
const JUMP_BUTTON_THRESHOLD_PX = 240;

interface UseStickyAutoScrollInput {
  scrollRef: RefObject<HTMLElement | null>;
  /** Changing this (e.g. the active thread id) re-pins to the latest message. */
  resetKey: unknown;
  /** Changing this (e.g. the message list) re-runs the follow effect. */
  contentKey: unknown;
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
  resetKey,
  contentKey,
  onScroll,
  onContentSettled,
}: UseStickyAutoScrollInput) {
  const stickToBottomRef = useRef(true);
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
      const distance = scrollContainer.scrollHeight - scrollContainer.clientHeight - scrollContainer.scrollTop;
      stickToBottomRef.current = distance <= STICK_THRESHOLD_PX;
      setShowJumpToLatest(distance > JUMP_BUTTON_THRESHOLD_PX);
    }
  }, [scrollRef]);

  // Jump straight to the latest message and re-enable auto-follow.
  const scrollToLatest = useCallback(() => {
    const scrollContainer = scrollRef.current;
    if (!scrollContainer)
      return;
    stickToBottomRef.current = true;
    setShowJumpToLatest(false);
    scrollContainer.scrollTop = scrollContainer.scrollHeight;
  }, [scrollRef]);

  // Opening/switching a thread starts pinned to the latest message.
  useEffect(() => {
    stickToBottomRef.current = true;
    setShowJumpToLatest(false);
  }, [resetKey]);

  // useLayoutEffect so the scroll-to-bottom happens before the browser paints,
  // avoiding a visible "flash at top → jump to bottom" when switching threads.
  useLayoutEffect(() => {
    const scrollContainer = scrollRef.current;
    if (!scrollContainer)
      return;

    // Only follow new/streamed content while pinned to the bottom; if the user
    // scrolled up, leave their position but surface the jump button once the
    // still-growing content pushes them far enough from the bottom.
    if (stickToBottomRef.current) {
      scrollContainer.scrollTop = scrollContainer.scrollHeight;
    }
    else {
      const distance = scrollContainer.scrollHeight - scrollContainer.clientHeight - scrollContainer.scrollTop;
      setShowJumpToLatest(distance > JUMP_BUTTON_THRESHOLD_PX);
    }
    onContentSettledRef.current?.();
  }, [contentKey, scrollRef]);

  return { handleScroll, scrollToLatest, showJumpToLatest };
}
