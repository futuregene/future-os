import { useCallback, useRef, useState, type RefObject } from "react";
import { FlatList, type NativeScrollEvent, type NativeSyntheticEvent } from "react-native";
import type { TimelineItem } from "../../remote/types";

const AT_LATEST_THRESHOLD_PX = 32;

export interface ChatScrollApi {
  listRef: RefObject<FlatList<TimelineItem> | null>;
  atLatest: boolean;
  composerHeight: number;
  setComposerHeight: (value: number) => void;
  scrollToLatest: () => void;
  onScroll: (event: NativeSyntheticEvent<NativeScrollEvent>) => void;
}

/**
 * Scroll ownership for the inverted transcript.
 *
 * ChatScreen presents newest-first data through an inverted FlatList, making
 * offset zero the single, stable definition of "latest". This hook therefore
 * never derives a bottom offset from asynchronously measured Markdown or tries
 * to compensate for pages changing the content height.
 */
export function useChatScroll(selectedSessionId: string): ChatScrollApi {
  const listRef = useRef<FlatList<TimelineItem>>(null);
  const [latestState, setLatestState] = useState({ sessionId: selectedSessionId, value: true });
  const atLatest = latestState.sessionId === selectedSessionId ? latestState.value : true;
  const [composerHeight, setComposerHeight] = useState(0);

  const setAtLatest = useCallback(
    (value: boolean) => setLatestState({ sessionId: selectedSessionId, value }),
    [selectedSessionId],
  );

  const scrollToLatest = useCallback(() => {
    setAtLatest(true);
    listRef.current?.scrollToOffset({ animated: true, offset: 0 });
  }, [setAtLatest]);

  const onScroll = useCallback(
    (event: NativeSyntheticEvent<NativeScrollEvent>) => {
      setAtLatest(Math.max(0, event.nativeEvent.contentOffset.y) <= AT_LATEST_THRESHOLD_PX);
    },
    [setAtLatest],
  );

  return {
    listRef,
    atLatest,
    composerHeight,
    setComposerHeight,
    scrollToLatest,
    onScroll,
  };
}
