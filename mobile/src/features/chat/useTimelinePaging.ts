import { useCallback, useLayoutEffect, useRef, useState } from "react";
import type { NativeScrollEvent, NativeSyntheticEvent } from "react-native";

type ScrollEvent = NativeSyntheticEvent<NativeScrollEvent>;
type PageResult = false | string[];
type PagePresentation = { deadline: number | null; lastLayout: number | null };
const LAYOUT_QUIET_MS = 100;
const LAYOUT_WAIT_MAX_MS = 1_000;

function nearOlderEdge({
  offset,
  content,
  viewport,
}: {
  offset: number;
  content: number;
  viewport: number;
}) {
  const lookAhead = Math.max(8, Math.min(viewport, 600));
  return (
    content > 0 && viewport > 0 && offset + viewport >= content - lookAhead
  );
}

/** The controller owns fetching and committing a page. This hook owns only
 * user intent: one request per gesture, a manual entry point, and presentation.
 * After a committed page, native layout gets a bounded settling period before
 * another gesture may page. Missing layout events cannot lock history forever.
 */
export function useTimelinePaging(
  sessionId: string,
  canLoadOlder: boolean,
  loadingOlder: boolean,
  requestOlder: () => Promise<PageResult>,
  onScroll: (event: ScrollEvent) => void,
) {
  const [presentation, setPresentation] = useState({
    sessionId,
    active: false,
    failed: false,
    nearOlder: true,
  });
  if (presentation.sessionId !== sessionId) {
    setPresentation({
      sessionId,
      active: false,
      failed: false,
      nearOlder: true,
    });
  }
  const requestRef = useRef<PagePresentation | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const gestureRef = useRef({
    active: false,
    used: false,
    momentumPending: false,
  });
  const geometryRef = useRef({ offset: 0, content: 0, viewport: 0 });

  const clearTimer = useCallback(() => {
    if (timerRef.current !== null) clearTimeout(timerRef.current);
    timerRef.current = null;
  }, []);

  const settle = useCallback(
    function checkLayout() {
      clearTimer();
      const request = requestRef.current;
      if (request?.deadline == null) return;
      const finishAt = Math.min(
        request.deadline,
        request.lastLayout === null
          ? Infinity
          : request.lastLayout + LAYOUT_QUIET_MS,
      );
      const remaining = finishAt - Date.now();
      if (remaining > 0) {
        timerRef.current = setTimeout(checkLayout, remaining);
        return;
      }
      requestRef.current = null;
      setPresentation((previous) => ({ ...previous, active: false }));
    },
    [clearTimer],
  );

  useLayoutEffect(() => {
    requestRef.current = null;
    gestureRef.current = { active: false, used: false, momentumPending: false };
    geometryRef.current = { offset: 0, content: 0, viewport: 0 };
    return () => {
      requestRef.current = null;
      clearTimer();
    };
  }, [sessionId, clearTimer]);

  const updateProximity = useCallback(() => {
    const geometry = geometryRef.current;
    // Keep a manual entry point until the first native measurements arrive.
    if (!geometry.content || !geometry.viewport) return;
    const nearOlder = nearOlderEdge(geometry);
    setPresentation((previous) =>
      previous.sessionId !== sessionId || previous.nearOlder === nearOlder
        ? previous
        : { ...previous, nearOlder },
    );
  }, [sessionId]);

  const loadOlder = useCallback(() => {
    if (!canLoadOlder || loadingOlder || requestRef.current) return;
    const request: PagePresentation = { deadline: null, lastLayout: null };
    requestRef.current = request;
    gestureRef.current.used = true;
    setPresentation((previous) => ({
      ...previous,
      sessionId,
      active: true,
      failed: false,
    }));
    const complete = (result: PageResult) => {
      if (requestRef.current !== request) return;
      if (result !== false && result.length > 0) {
        // The deadline starts after the sync lane acknowledges the page, not
        // when the network request starts. It never declares missing data ready.
        request.deadline = Date.now() + LAYOUT_WAIT_MAX_MS;
        settle();
        return;
      }
      requestRef.current = null;
      setPresentation((previous) => ({
        ...previous,
        sessionId,
        active: false,
        failed: result === false,
      }));
    };
    try {
      void requestOlder().then(complete, () => complete(false));
    } catch {
      complete(false);
    }
  }, [canLoadOlder, loadingOlder, requestOlder, sessionId, settle]);

  const observeScroll = useCallback(
    (event: ScrollEvent) => {
      const { contentOffset, contentSize, layoutMeasurement } =
        event.nativeEvent;
      geometryRef.current = {
        offset: contentOffset.y,
        content: contentSize.height,
        viewport: layoutMeasurement.height,
      };
      updateProximity();
      const gesture = gestureRef.current;
      if (gesture.active && !gesture.used && nearOlderEdge(geometryRef.current))
        loadOlder();
    },
    [loadOlder, updateProximity],
  );

  const handleScroll = useCallback(
    (event: ScrollEvent) => {
      onScroll(event);
      observeScroll(event);
    },
    [observeScroll, onScroll],
  );

  const onScrollBeginDrag = useCallback(
    (event: ScrollEvent) => {
      gestureRef.current = {
        active: true,
        used: requestRef.current !== null || loadingOlder,
        momentumPending: false,
      };
      // An already-clamped edge may not produce another scroll event.
      observeScroll(event);
    },
    [loadingOlder, observeScroll],
  );
  const onScrollEndDrag = useCallback(
    (event: ScrollEvent) => {
      observeScroll(event);
      gestureRef.current.active = false;
      gestureRef.current.momentumPending = true;
    },
    [observeScroll],
  );
  const onMomentumScrollBegin = useCallback(() => {
    gestureRef.current.active = gestureRef.current.momentumPending;
    gestureRef.current.momentumPending = false;
  }, []);
  const onMomentumScrollEnd = useCallback(
    (event: ScrollEvent) => {
      observeScroll(event);
      gestureRef.current.active = false;
      gestureRef.current.momentumPending = false;
    },
    [observeScroll],
  );

  const onRowLayout = useCallback(() => {
    if (requestRef.current?.deadline != null) {
      requestRef.current.lastLayout = Date.now();
      settle();
    }
  }, [settle]);
  const onListLayout = useCallback(
    (height: number) => {
      geometryRef.current.viewport = height;
      updateProximity();
      onRowLayout();
    },
    [onRowLayout, updateProximity],
  );
  const onContentSizeChange = useCallback(
    (_width: number, height: number) => {
      geometryRef.current.content = height;
      updateProximity();
      onRowLayout();
    },
    [onRowLayout, updateProximity],
  );

  const current = presentation.sessionId === sessionId;
  return {
    showLoadOlderHint:
      current &&
      (presentation.active ||
        (canLoadOlder && (presentation.failed || presentation.nearOlder))),
    pagingActive: current && presentation.active,
    pagingFailed: current && presentation.failed,
    loadOlder,
    onRowLayout,
    onListLayout,
    onContentSizeChange,
    onScrollBeginDrag,
    onScrollEndDrag,
    onMomentumScrollBegin,
    onMomentumScrollEnd,
    onScroll: handleScroll,
  };
}
