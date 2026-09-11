import { useCallback, useLayoutEffect, useRef, useState } from "react";
import type { NativeScrollEvent, NativeSyntheticEvent } from "react-native";

type ScrollEvent = NativeSyntheticEvent<NativeScrollEvent>;
type PageResult = false | string[];
type Transaction = {
  sessionId: string;
  deadline: number;
  expectedIds: string[] | null;
  failed: boolean;
  committed: boolean;
  lastLayout: number;
  layoutObserved: boolean;
};

const MIN_HINT_MS = 1_500;
// Native VirtualizedList mounts rows in 32ms batches. Two JS animation frames
// alone can fall entirely between those batches. Require a quiet layout window
// after the requested ids are committed AND a native layout has been observed.
const LAYOUT_QUIET_MS = 100;

/**
 * A gesture starts at most one page transaction. Network completion, React
 * commit and native layout are distinct barriers; the minimum display timer
 * starts at collision and runs concurrently with all three.
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
    const remaining = Math.max(tx.deadline, tx.lastLayout + LAYOUT_QUIET_MS) - Date.now();
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
    idsRef.current = new Set(items.map(item => item.id));
    check();
  }, [items, check, sessionId]);

  const loadOlder = useCallback(() => {
    if (!canLoadOlder || loadingOlder || transactionRef.current) return;
    const previousIds = idsRef.current;
    const tx: Transaction = {
      sessionId,
      deadline: Date.now() + MIN_HINT_MS,
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
      if (
        contentSize.height > 0 &&
        contentOffset.y + layoutMeasurement.height >= contentSize.height - 8
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
