import { useCallback, useEffect, useRef, useState } from "react";
import type { NativeScrollEvent, NativeSyntheticEvent } from "react-native";

const OLDER_EDGE_THRESHOLD_PX = 8;
const OLDER_EDGE_SETTLE_MS = 350;

interface TimelinePagingApi {
  showLoadOlderHint: boolean;
  loadOlder: () => void;
  onScroll: (event: NativeSyntheticEvent<NativeScrollEvent>) => void;
}

/**
 * Loads one older page after the user settles at the visual top of the
 * inverted list. Older rows are appended to the inverted data, so pagination
 * cannot shift the existing rows and no height/offset compensation is needed.
 */
export function useTimelinePaging(
  sessionId: string,
  canLoadOlder: boolean,
  loadingOlder: boolean,
  requestOlder: () => void | Promise<void>,
  onScroll: (event: NativeSyntheticEvent<NativeScrollEvent>) => void,
): TimelinePagingApi {
  const [edgeState, setEdgeState] = useState({ sessionId, atOlderEdge: false });
  const settleTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const requestPendingRef = useRef(false);
  const autoLoadAttemptedRef = useRef(false);

  const clearSettleTimer = useCallback(() => {
    if (settleTimerRef.current == null) return;
    clearTimeout(settleTimerRef.current);
    settleTimerRef.current = null;
  }, []);

  useEffect(() => {
    clearSettleTimer();
    requestPendingRef.current = false;
    autoLoadAttemptedRef.current = false;
  }, [clearSettleTimer, sessionId]);

  useEffect(() => {
    if (loadingOlder || !canLoadOlder) clearSettleTimer();
  }, [canLoadOlder, clearSettleTimer, loadingOlder]);

  useEffect(() => () => clearSettleTimer(), [clearSettleTimer]);

  const atOlderEdge = edgeState.sessionId === sessionId && edgeState.atOlderEdge;
  const showLoadOlderHint = canLoadOlder && atOlderEdge && !loadingOlder;

  const loadOlder = useCallback(() => {
    if (!canLoadOlder || loadingOlder || requestPendingRef.current) return;
    requestPendingRef.current = true;
    clearSettleTimer();
    try {
      void Promise.resolve(requestOlder()).finally(() => {
        requestPendingRef.current = false;
      });
    } catch (error) {
      requestPendingRef.current = false;
      throw error;
    }
  }, [canLoadOlder, clearSettleTimer, loadingOlder, requestOlder]);

  const handleScroll = useCallback(
    (event: NativeSyntheticEvent<NativeScrollEvent>) => {
      onScroll(event);
      const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
      const isAtOlderEdge =
        contentOffset.y + layoutMeasurement.height >= contentSize.height - OLDER_EDGE_THRESHOLD_PX;
      setEdgeState({ sessionId, atOlderEdge: isAtOlderEdge });

      if (!isAtOlderEdge || !canLoadOlder || loadingOlder) {
        clearSettleTimer();
        if (!isAtOlderEdge) autoLoadAttemptedRef.current = false;
        return;
      }
      if (
        settleTimerRef.current != null ||
        requestPendingRef.current ||
        autoLoadAttemptedRef.current
      ) {
        return;
      }
      settleTimerRef.current = setTimeout(() => {
        settleTimerRef.current = null;
        autoLoadAttemptedRef.current = true;
        loadOlder();
      }, OLDER_EDGE_SETTLE_MS);
    },
    [canLoadOlder, clearSettleTimer, loadOlder, loadingOlder, onScroll, sessionId],
  );

  return { showLoadOlderHint, loadOlder, onScroll: handleScroll };
}
