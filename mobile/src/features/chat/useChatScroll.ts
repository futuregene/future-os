import { useEffect, useRef, useState, type RefObject } from "react";
import {
  FlatList,
  type LayoutChangeEvent,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
} from "react-native";
import type { TimelineItem } from "../../remote/types";

// How close to the bottom counts as "at latest" (px). Shared by the atLatest
// detection and the scroll target so the two never disagree.
const AT_LATEST_THRESHOLD = 32;

export interface ChatScrollApi {
  listRef: RefObject<FlatList<TimelineItem> | null>;
  atLatest: boolean;
  composerHeight: number;
  setComposerHeight: (value: number) => void;
  scrollToLatest: () => void;
  onContentSizeChange: (width: number, height: number) => void;
  onListLayout: (event: LayoutChangeEvent) => void;
  onScroll: (event: NativeSyntheticEvent<NativeScrollEvent>) => void;
}

export function useChatScroll(selectedSessionId: string, transcriptItemCount: number): ChatScrollApi {
  const listRef = useRef<FlatList<TimelineItem>>(null);
  const [atLatest, setAtLatest] = useState(true);
  const [composerHeight, setComposerHeight] = useState(0);
  const contentHeightRef = useRef(0);
  const layoutHeightRef = useRef(0);
  // The first content render snaps to the end without animation; only later
  // appends (streaming, new messages) scroll animated.
  const landedRef = useRef(false);
  // A FlatList emits scroll events while iOS/Android are measuring its first
  // content and viewport. Those are layout side effects, not a user decision
  // to read earlier messages, so they must not disable the initial snap.
  const initialScrollPendingRef = useRef(true);
  const initialScrollFrameRef = useRef<number | null>(null);

  // The bottom-most scroll offset: full content height minus the viewport,
  // never negative. Recomputed from measured sizes so it stays correct as the
  // composer (bottom padding) and content grow.
  const maxScrollOffset = () => Math.max(0, contentHeightRef.current - layoutHeightRef.current);

  // Opening a conversation may lay out the list, its remote history and the
  // floating composer in separate commits. Wait one frame, then repeat on the
  // next frame with the final measurements: this avoids both Android's stale
  // first content size and iOS applying its safe-area/composer inset late.
  const scheduleInitialScroll = () => {
    if (
      !initialScrollPendingRef.current ||
      contentHeightRef.current <= 0 ||
      layoutHeightRef.current <= 0 ||
      composerHeight <= 0
    ) {
      return;
    }
    if (initialScrollFrameRef.current != null) return;
    initialScrollFrameRef.current = requestAnimationFrame(() => {
      initialScrollFrameRef.current = null;
      if (!initialScrollPendingRef.current) return;
      listRef.current?.scrollToOffset({ animated: false, offset: maxScrollOffset() });
      initialScrollFrameRef.current = requestAnimationFrame(() => {
        initialScrollFrameRef.current = null;
        if (!initialScrollPendingRef.current) return;
        listRef.current?.scrollToOffset({ animated: false, offset: maxScrollOffset() });
        initialScrollPendingRef.current = false;
        landedRef.current = true;
        setAtLatest(true);
      });
    });
  };

  // Reset the opening contract if the mounted screen switches between the
  // draft and an established session. In the usual navigation path ChatScreen
  // unmounts, but this also covers a send binding a draft to its new session.
  useEffect(() => {
    landedRef.current = false;
    initialScrollPendingRef.current = true;
    contentHeightRef.current = 0;
    layoutHeightRef.current = 0;
    return () => {
      if (initialScrollFrameRef.current != null) {
        cancelAnimationFrame(initialScrollFrameRef.current);
        initialScrollFrameRef.current = null;
      }
    };
  }, [selectedSessionId]);

  // scrollToEnd is unreliable on Android for the first layout of a large
  // history (it can no-op or use a stale content size, leaving a gap). An
  // explicit offset computed from the measured content size is deterministic;
  // the rAF retry catches content that finishes laying out a frame late.
  const scrollToLatest = () => {
    setAtLatest(true);
    listRef.current?.scrollToOffset({ animated: true, offset: maxScrollOffset() });
    requestAnimationFrame(() =>
      listRef.current?.scrollToOffset({ animated: true, offset: maxScrollOffset() }),
    );
  };

  const onContentSizeChange = (_w: number, h: number) => {
    contentHeightRef.current = h;
    if (initialScrollPendingRef.current) {
      scheduleInitialScroll();
      return;
    }
    if (!atLatest) return;
    if (!landedRef.current && transcriptItemCount === 0) return;
    listRef.current?.scrollToOffset({
      animated: landedRef.current,
      offset: maxScrollOffset(),
    });
    landedRef.current = true;
  };

  const onListLayout = (event: LayoutChangeEvent) => {
    layoutHeightRef.current = event.nativeEvent.layout.height;
    scheduleInitialScroll();
  };

  const onScroll = (event: NativeSyntheticEvent<NativeScrollEvent>) => {
    // Ignore first-layout offsets; the two-frame snap above owns the
    // initial position on both platforms.
    if (initialScrollPendingRef.current) return;
    const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
    setAtLatest(
      contentOffset.y + layoutMeasurement.height >=
        contentSize.height - AT_LATEST_THRESHOLD,
    );
  };

  return {
    listRef,
    atLatest,
    composerHeight,
    setComposerHeight,
    scrollToLatest,
    onContentSizeChange,
    onListLayout,
    onScroll,
  };
}
