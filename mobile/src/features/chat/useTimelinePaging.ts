import { useCallback, useLayoutEffect, useRef, useState } from "react";
import type { NativeScrollEvent, NativeSyntheticEvent } from "react-native";

type ScrollEvent = NativeSyntheticEvent<NativeScrollEvent>;
type PageResult = false | string[];
type Transaction = {
  sessionId: string;
  expectedIds: string[] | null;
  failed: boolean;
  committed: boolean;
  lastLayout: number;
  layoutObserved: boolean;
};

// Native VirtualizedList mounts rows in 32ms batches. Two JS animation frames
// alone can fall entirely between those batches. Require a quiet layout window
// after the requested ids are committed AND a native layout has been observed.
const LAYOUT_QUIET_MS = 100;

/**
 * A gesture starts at most one page transaction. Network completion, React
 * commit and native layout are distinct barriers. Completion follows those
 * barriers, not a fixed spinner duration that delays the next deliberate page.
 */
export function useTimelinePaging(
  sessionId: string,
  canLoadOlder: boolean,
  loadingOlder: boolean,
  requestOlder: () => Promise<PageResult>,
  onScroll: (event: ScrollEvent) => void,
  items: readonly { id: string }[] = [],
) {
  const [presentation, setPresentation] = useState({
    sessionId,
    active: false,
    failed: false,
  });
  if (presentation.sessionId !== sessionId) {
    setPresentation({ sessionId, active: false, failed: false });
  }
  const transactionRef = useRef<Transaction | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const gestureRef = useRef({ active: false, used: false });
  const idsRef = useRef(new Set<string>());
  const itemsRef = useRef(items);
  const checkRef = useRef<() => void>(() => undefined);

  const cancelTimer = useCallback(() => {
    if (timerRef.current !== null) clearTimeout(timerRef.current);
    timerRef.current = null;
  }, []);

  const check = useCallback(() => {
    cancelTimer();
    const tx = transactionRef.current;
    if (!tx || tx.expectedIds === null) return;
    if (!tx.committed) {
      if (!tx.expectedIds.every(id => idsRef.current.has(id))) return;
      tx.committed = true;
      tx.lastLayout = Date.now();
      tx.layoutObserved = false;
      // Empty/failed pages have no new native rows to wait for.
      if (tx.expectedIds.length === 0) tx.layoutObserved = true;
    }
    if (!tx.layoutObserved) return;
    const remaining = tx.lastLayout + LAYOUT_QUIET_MS - Date.now();
    if (remaining > 0) {
      timerRef.current = setTimeout(() => checkRef.current(), remaining);
      return;
    }
    transactionRef.current = null;
    setPresentation({ sessionId: tx.sessionId, active: false, failed: tx.failed });
  }, [cancelTimer]);

  useLayoutEffect(() => {
    checkRef.current = check;
  }, [check]);

  useLayoutEffect(() => {
    transactionRef.current = null;
    gestureRef.current = { active: false, used: false };
    return () => {
      transactionRef.current = null;
      cancelTimer();
    };
  }, [sessionId, cancelTimer]);

  useLayoutEffect(() => {
    itemsRef.current = items;
    // Text-only streaming changes the items array too. There is no reason to
    // rebuild a large identity index when no page transaction is waiting.
    if (transactionRef.current) {
      idsRef.current = new Set(items.map(item => item.id));
      check();
    }
  }, [items, check, sessionId]);

  const loadOlder = useCallback(() => {
    if (!canLoadOlder || loadingOlder || transactionRef.current) return;
    const previousIds = new Set(itemsRef.current.map(item => item.id));
    idsRef.current = previousIds;
    const tx: Transaction = {
      sessionId,
      expectedIds: null,
      failed: false,
      committed: false,
      lastLayout: Date.now(),
      layoutObserved: false,
    };
    transactionRef.current = tx;
    gestureRef.current.used = true;
    setPresentation({ sessionId, active: true, failed: false });
    const complete = (result: PageResult) => {
      // Identity guards every continuation, including failures from old sessions.
      if (transactionRef.current !== tx) return;
      tx.failed = result === false;
      tx.expectedIds = result === false ? [] : result.filter(id => !previousIds.has(id));
      checkRef.current();
    };
    try {
      void requestOlder().then(complete, () => complete(false));
    } catch {
      complete(false);
    }
  }, [canLoadOlder, loadingOlder, requestOlder, sessionId]);

  const onListLayout = useCallback(() => {
    const tx = transactionRef.current;
    if (!tx) return;
    tx.lastLayout = Date.now();
    // Only native layouts after the React commit satisfy the render barrier.
    if (tx.committed) tx.layoutObserved = true;
    checkRef.current();
  }, []);

  const detectCollision = useCallback(
    (event: ScrollEvent) => {
      const gesture = gestureRef.current;
      if (!gesture.active || gesture.used) return;
      const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
      // One page of look-ahead during a deliberate gesture, at most one
      // viewport (600 layout points). The gesture/transaction guards prevent
      // programmatic scrolling or momentum from cascading through history.
      const lookAhead = Math.max(8, Math.min(layoutMeasurement.height, 600));
      if (
        contentSize.height > 0 &&
        contentOffset.y + layoutMeasurement.height >= contentSize.height - lookAhead
      )
        loadOlder();
    },
    [loadOlder],
  );

  const handleScroll = useCallback(
    (event: ScrollEvent) => {
      onScroll(event);
      detectCollision(event);
    },
    [detectCollision, onScroll],
  );

  const onScrollBeginDrag = useCallback(() => {
    gestureRef.current = { active: true, used: transactionRef.current !== null };
  }, []);
  const onScrollEndDrag = useCallback(
    (event: ScrollEvent) => {
      detectCollision(event);
      // Momentum belongs to this same gesture and shares its used flag.
    },
    [detectCollision],
  );
  const onMomentumScrollEnd = useCallback(
    (event: ScrollEvent) => {
      detectCollision(event);
      gestureRef.current.active = false;
    },
    [detectCollision],
  );

  const current = presentation.sessionId === sessionId;
  return {
    showLoadOlderHint: current && (presentation.active || presentation.failed),
    pagingActive: current && presentation.active,
    pagingFailed: current && presentation.failed,
    loadOlder,
    onListLayout,
    onContentSizeChange: onListLayout,
    onScrollBeginDrag,
    onScrollEndDrag,
    onMomentumScrollEnd,
    onScroll: handleScroll,
  };
}
