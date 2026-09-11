import { useCallback, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { FlatList, type NativeScrollEvent, type NativeSyntheticEvent } from "react-native";
import type { TimelineItem } from "../../remote/types";

const AT_LATEST_THRESHOLD_PX = 32;
const READING_ANCHOR = { minIndexForVisible: 0 };

export interface ChatScrollApi {
  listRef: RefObject<FlatList<TimelineItem> | null>;
  atLatest: boolean;
  maintainVisibleContentPosition: typeof READING_ANCHOR | undefined;
  scrollToLatest: () => void;
  onScrollBeginDrag: () => void;
  onLayout: () => void;
  onContentSizeChange: () => void;
  onScroll: (event: NativeSyntheticEvent<NativeScrollEvent>) => void;
}

/**
 * Exactly one owner of the inverted viewport:
 * - following: physical offset zero, including late native layouts;
 * - reading: native maintainVisibleContentPosition preserves a message anchor.
 * Never enable native anchor compensation while following. Underfilled lists
 * reposition their rows as they grow, which otherwise produces invalid offsets.
 */
export function useChatScroll(sessionId: string, itemCount: number): ChatScrollApi {
  const listRef = useRef<FlatList<TimelineItem>>(null);
  const followingRef = useRef(true);
  const manualScrollRef = useRef(false);
  const [state, setState] = useState({ sessionId, following: true });
  const atLatest = state.sessionId !== sessionId || state.following;
  if (state.sessionId !== sessionId) setState({ sessionId, following: true });

  const setFollowing = useCallback(
    (following: boolean) => {
      followingRef.current = following;
      setState(previous =>
        previous.sessionId === sessionId && previous.following === following
          ? previous
          : { sessionId, following },
      );
    },
    [sessionId],
  );

  const pinLatest = useCallback(() => {
    if (followingRef.current) {
      listRef.current?.scrollToOffset({ animated: false, offset: 0 });
    }
  }, []);

  useLayoutEffect(() => {
    manualScrollRef.current = false;
    followingRef.current = true;
  }, [sessionId]);

  // A commit is only an early attempt. Native content/viewport layout callbacks
  // repeat this after the actual geometry changes, including keyboard/composer.
  useLayoutEffect(() => {
    if (itemCount > 0) pinLatest();
  }, [sessionId, itemCount, pinLatest]);

  const scrollToLatest = useCallback(() => {
    manualScrollRef.current = false;
    setFollowing(true);
    listRef.current?.scrollToOffset({ animated: false, offset: 0 });
  }, [setFollowing]);

  const onScrollBeginDrag = useCallback(() => {
    manualScrollRef.current = true;
    // Transfer ownership before the next native layout can follow the tail.
    setFollowing(false);
  }, [setFollowing]);

  const onScroll = useCallback(
    (event: NativeSyntheticEvent<NativeScrollEvent>) => {
      if (!manualScrollRef.current) return;
      const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
      const fits = contentSize.height <= layoutMeasurement.height + 1;
      setFollowing(fits || Math.max(0, contentOffset.y) <= AT_LATEST_THRESHOLD_PX);
    },
    [setFollowing],
  );

  return {
    listRef,
    atLatest,
    maintainVisibleContentPosition: atLatest ? undefined : READING_ANCHOR,
    scrollToLatest,
    onScrollBeginDrag,
    onLayout: pinLatest,
    onContentSizeChange: pinLatest,
    onScroll,
  };
}
