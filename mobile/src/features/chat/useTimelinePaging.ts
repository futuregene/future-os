import { useCallback, useLayoutEffect, useRef, useState } from "react";
import type { NativeScrollEvent, NativeSyntheticEvent } from "react-native";

type ScrollEvent = NativeSyntheticEvent<NativeScrollEvent>;
type PageResult = false | string[];
type PagePresentation = {
  minimumUntil: number;
  completedAt: number | null;
  lastLayout: number | null;
  requiresLayout: boolean;
};
const INDICATOR_MIN_MS = 1_000;
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
  });
  if (presentation.sessionId !== sessionId) {
    setPresentation({
      sessionId,
      active: false,
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
      if (request?.completedAt == null) return;
      const layoutReadyAt = request.requiresLayout
        ? Math.min(
            request.completedAt + LAYOUT_WAIT_MAX_MS,
            request.lastLayout === null
              ? Infinity
              : request.lastLayout + LAYOUT_QUIET_MS,
          )
        : request.completedAt;
      const finishAt = Math.max(
        request.minimumUntil,
        layoutReadyAt,
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

  const loadOlder = useCallback(() => {
    if (!canLoadOlder || loadingOlder || requestRef.current) return;
    const request: PagePresentation = {
      minimumUntil: Date.now() + INDICATOR_MIN_MS,
      completedAt: null,
      lastLayout: null,
      requiresLayout: true,
    };
    requestRef.current = request;
    gestureRef.current.used = true;
    setPresentation((previous) => ({
      ...previous,
      sessionId,
      active: true,
    }));
    timerRef.current = setTimeout(settle, INDICATOR_MIN_MS);
    const complete = (result: PageResult) => {
      if (requestRef.current !== request) return;
      request.completedAt = Date.now();
      // Failed and empty pages add no rows, so there is no native layout event
      // to await. Both still honor the one-second minimum presentation.
      request.requiresLayout = result !== false && result.length > 0;
      settle();
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
      const gesture = gestureRef.current;
      if (gesture.active && !gesture.used && nearOlderEdge(geometryRef.current))
        loadOlder();
    },
    [loadOlder],
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
    if (requestRef.current?.completedAt != null && requestRef.current.requiresLayout) {
      requestRef.current.lastLayout = Date.now();
      settle();
    }
  }, [settle]);
  const onListLayout = useCallback(
    (height: number) => {
      geometryRef.current.viewport = height;
      onRowLayout();
    },
    [onRowLayout],
  );
  const onContentSizeChange = useCallback(
    (_width: number, height: number) => {
      geometryRef.current.content = height;
      onRowLayout();
    },
    [onRowLayout],
  );

  const current = presentation.sessionId === sessionId;
  return {
    showLoadOlderHint: current && presentation.active,
    pagingActive: current && presentation.active,
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
