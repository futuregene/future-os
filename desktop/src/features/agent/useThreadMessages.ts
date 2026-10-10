import type { AgentMessage } from "@future-os/thread-projection";
import type { Dispatch, SetStateAction } from "react";
import type { StoredRun } from "../../integrations/storage/threadStore";
import {
  matchesSettledRun,
} from "@future-os/thread-projection";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import {
  getLatestRun,
} from "../../integrations/storage/threadStore";
import { errorMessage } from "../../lib/errors";
import { useCommittedRef } from "../../lib/useCommittedRef";
import { loadThreadHistory } from "./history/loadThreadHistory";
import { reconcileThreadHistory } from "./reconcileThreadHistory";
import { useSessionMessageEvents } from "./runtime/useSessionMessageEvents";
import {
  getThreadHistoryCursor,
  getThreadMessageSnapshot,
  setThreadMessageSnapshot,
} from "./threadMessageCache";

interface UseThreadMessagesInput {
  threadId: string | null;
  workspaceId?: string | null;
  workspacePath?: string | null;
  agentSessionId?: string | null;
}

// Flash-free loading indicator (mirrors the right-context panel, useContextData):
// a thread load usually resolves in tens of ms, so hold off showing the "loading"
// text until the load has run this long...
const LOADING_INDICATOR_DELAY_MS = 200;
// ...and once shown, keep it visible at least this long so it can't itself flash.
const LOADING_INDICATOR_MIN_MS = 200;

/**
 * Owns a thread's message list + recent-run status: loads messages when the
 * instance mounts and keeps a live run ticking while one is active.
 *
 * AgentThread is keyed by thread id, so each conversation gets its own
 * instance: every state and listener here belongs to exactly one thread, and
 * writers from a conversation the user switched away from are torn down with
 * their instance — no cross-thread guarding is needed. The races left are
 * within this one thread (see `reconcileThreadHistory`'s request-time baseline).
 */
export function useThreadMessages({
  threadId,
  workspaceId,
  workspacePath,
  agentSessionId,
}: UseThreadMessagesInput) {
  const normalizedAgentSessionId = agentSessionId?.trim() || null;
  const recentRunGenRef = useRef(0);
  const activeRunRef = useRef<{ runId: string | null; startedAt: number | null }>({ runId: null, startedAt: null });
  // AgentThread is keyed by thread id and therefore remounts on every switch.
  // Seed the new instance from a small process-local LRU, then revalidate from
  // the authoritative Agent journal in the background. This preserves the
  // isolation benefit of keyed instances without flashing a loading placeholder
  // every time the user revisits a conversation.
  const [initialSnapshot] = useState(() =>
    threadId
      ? getThreadMessageSnapshot(threadId, normalizedAgentSessionId)
      : null,
  );
  const [initialCursor] = useState(() =>
    threadId
      ? getThreadHistoryCursor(threadId, normalizedAgentSessionId)
      : null,
  );
  const hasWarmSnapshotRef = useRef(initialSnapshot !== null);
  const cacheEligibleRef = useRef(initialSnapshot !== null);
  // A source revision cancels reads. A new owner also cancels writers; first
  // binding preserves ownership so the original send pipeline can finish.
  // Derive this during render so effects never see a new source with old ownership.
  const [source, setSource] = useState({ id: normalizedAgentSessionId, sourceRevision: 0, writeEpoch: 0 });
  if (source.id !== normalizedAgentSessionId) {
    setSource({ id: normalizedAgentSessionId, sourceRevision: source.sourceRevision + 1, writeEpoch: source.writeEpoch + (source.id === null ? 0 : 1) });
  }
  const sourceRef = useRef(source);
  const [committedWriteEpoch, setCommittedWriteEpoch] = useState(source.writeEpoch);
  const aliveRef = useRef(true);
  const replacingRef = useRef(false);
  const tailRequestRef = useRef(0);
  const [sessionChanged, setSessionChanged] = useState(false);
  const [messagesState, setMessagesState] = useState<AgentMessage[]>(
    initialSnapshot ?? [],
  );
  const messages = messagesState;
  // Only cold loads and real source replacements block interaction. Background
  // revalidation (including first binding) does not change composer availability.
  const [blockingLoad, setBlockingLoad] = useState(initialSnapshot === null);
  const ownerChanged = source.writeEpoch !== committedWriteEpoch;
  const loadingThread = blockingLoad || ownerChanged;
  const loadingRef = useCommittedRef(loadingThread);
  // Debounced projection of `loadingThread` for the "loading" indicator: only
  // comes on if a load outlasts the delay, and once on stays for a minimum so a
  // fast switch-back can't flash it. Purely presentational.
  const [loadingIndicator, setLoadingIndicator] = useState(false);
  const indicatorShownAtRef = useRef<number | null>(null);
  const [recentRunState, setRecentRunState] = useState<StoredRun | null>(null);
  const recentRun = ownerChanged ? null : recentRunState;
  const setRecentRun: Dispatch<SetStateAction<StoredRun | null>> = useCallback((value) => {
    if (aliveRef.current && sourceRef.current.writeEpoch === source.writeEpoch)
      setRecentRunState(value);
  }, [source.writeEpoch]);
  const [hasOlderHistory, setHasOlderHistory] = useState(
    initialCursor?.hasMore ?? false,
  );
  const [historyError, setHistoryError] = useState<string | null>(null);
  const historyCursorRef = useRef<number | null>(initialCursor?.before ?? null);
  const hasOlderHistoryRef = useCommittedRef(hasOlderHistory);
  const olderInFlightRef = useRef<object | null>(null);
  const historyEpochRef = useRef(0);
  const messagesRef = useCommittedRef(messages);
  const setMessages: Dispatch<SetStateAction<AgentMessage[]>> = useCallback((value) => {
    if (!aliveRef.current || sourceRef.current.writeEpoch !== source.writeEpoch)
      return;
    const next = typeof value === "function" ? value(messagesRef.current) : value;
    messagesRef.current = next;
    setMessagesState(next);
  }, [messagesRef, source.writeEpoch]);
  useLayoutEffect(() => {
    if (sourceRef.current.sourceRevision === source.sourceRevision)
      return;
    const replacing = sourceRef.current.writeEpoch !== source.writeEpoch;
    sourceRef.current = source;
    setCommittedWriteEpoch(source.writeEpoch);
    replacingRef.current = replacingRef.current || replacing;
    tailRequestRef.current += 1;
    recentRunGenRef.current += 1;
    historyEpochRef.current += 1;
    olderInFlightRef.current = null;
    historyCursorRef.current = null;
    cacheEligibleRef.current = false;
    setHasOlderHistory(false);
    setHistoryError(null);
    if (replacing) {
      setSessionChanged(true);
      setBlockingLoad(true);
      setRecentRunState(null);
      activeRunRef.current = { runId: null, startedAt: null };
    }
  }, [source]);
  useLayoutEffect(
    () => {
      aliveRef.current = true;
      return () => {
        aliveRef.current = false;
        tailRequestRef.current += 1;
        recentRunGenRef.current += 1;
        historyEpochRef.current += 1;
      };
    },
    [],
  );

  // Keep the latest committed immutable message array warm for a future keyed
  // remount. Failed cold loads are deliberately not cached as conversation data.
  useLayoutEffect(() => {
    if (threadId && cacheEligibleRef.current) {
      setThreadMessageSnapshot(
        threadId,
        normalizedAgentSessionId,
        messages,
        historyCursorRef.current === null
          ? undefined
          : { before: historyCursorRef.current, hasMore: hasOlderHistory },
      );
    }
  }, [messages, normalizedAgentSessionId, threadId, hasOlderHistory]);

  const refreshRecentRun = useCallback(
    async (targetThreadId: string) => {
      // First session binding preserves the send owner. Allow that send's
      // captured callback to start a fresh read, but invalidate a read already
      // in flight when the source changes.
      if (!aliveRef.current || sourceRef.current.writeEpoch !== source.writeEpoch)
        return;
      const version = sourceRef.current.sourceRevision;
      const generation = ++recentRunGenRef.current;
      try {
        // One row, not the thread's whole run history. Invoked on push events
        // (thread-runtime-updated terminal / remote-activity) and loads — there is
        // no longer a periodic timer driving it.
        const latestRun = await getLatestRun(targetThreadId);
        if (!aliveRef.current || sourceRef.current.sourceRevision !== version || generation !== recentRunGenRef.current) {
          return;
        }
        // Mirror the in-flight run for the load path (see activeRunRef).
        activeRunRef.current = {
          runId:
            latestRun && !matchesSettledRun(latestRun.status)
              ? latestRun.id
              : null,
          startedAt: latestRun?.startedAt ?? latestRun?.createdAt ?? null,
        };
        setRecentRun(latestRun);
      }
      catch {
        // Run-status refresh is best-effort.
      }
    },
    [source.writeEpoch, setRecentRun],
  );

  // Reconstruct the thread's messages from the agent Agent transcript
  // (get_session_entries). Empty and failed loads stay distinct so a transient Agent
  // error never masquerades as an empty conversation.

  // All tail reads share one request order. Settling may replace a streaming
  // snapshot, but never bypasses source ownership or overwrites newer writes.
  const reloadThreadHistory = useCallback(
    async (targetThreadId: string, settle = false) => {
      if (!aliveRef.current || sourceRef.current.sourceRevision !== source.sourceRevision)
        return;
      const request = ++tailRequestRef.current;
      const baseline = messagesRef.current;
      await refreshRecentRun(targetThreadId);
      if (!aliveRef.current || request !== tailRequestRef.current || sourceRef.current.sourceRevision !== source.sourceRevision)
        return;
      const result = await loadThreadHistory(
        targetThreadId,
        activeRunRef.current.runId,
        activeRunRef.current.startedAt,
      );
      if (!aliveRef.current || request !== tailRequestRef.current || sourceRef.current.sourceRevision !== source.sourceRevision)
        return;
      if (result.status !== "loaded") {
        setHistoryError(result.error);
        if (!replacingRef.current)
          setBlockingLoad(false);
        return;
      }
      const merged = replacingRef.current
        ? { messages: result.messages, keptOlder: false }
        : reconcileThreadHistory(messagesRef.current, result.messages, baseline, historyCursorRef.current !== null, settle);
      historyEpochRef.current += 1;
      olderInFlightRef.current = null;
      if (!merged.keptOlder || historyCursorRef.current === null) {
        historyCursorRef.current = result.nextOffset;
        hasOlderHistoryRef.current = result.hasMore;
        setHasOlderHistory(result.hasMore);
      }
      replacingRef.current = false;
      cacheEligibleRef.current = true;
      hasWarmSnapshotRef.current = true;
      messagesRef.current = merged.messages;
      setMessagesState(merged.messages);
      setHistoryError(null);
      setBlockingLoad(false);
    },
    [hasOlderHistoryRef, messagesRef, refreshRecentRun, source.sourceRevision],
  );

  useEffect(() => {
    if (!threadId) {
      setBlockingLoad(false);
      return;
    }
    // A remount may restore a streaming snapshot whose run finished while
    // another thread was open. Revalidate that cache, including terminal
    // status; the request-time baseline still protects writes made during
    // this read. Otherwise there is no active-run transition to settle it.
    void reloadThreadHistory(threadId, true);
  }, [reloadThreadHistory, threadId, workspaceId]);

  const loadOlderHistory = useCallback(
    async (beforeCommit?: (messages: AgentMessage[]) => void) => {
      if (
        !threadId
        || !aliveRef.current
        || sourceRef.current.sourceRevision !== source.sourceRevision
        || olderInFlightRef.current
        || historyCursorRef.current === null
        || historyCursorRef.current <= 0
      ) {
        return;
      }
      const request = {};
      olderInFlightRef.current = request;
      const epoch = historyEpochRef.current;
      const before = historyCursorRef.current;
      try {
        const result = await loadThreadHistory(
          threadId,
          null,
          null,
          before,
        );
        if (epoch !== historyEpochRef.current)
          return;
        if (result.status === "failed")
          throw new Error(result.error);
        if (result.nextOffset >= before)
          throw new Error("History cursor did not advance.");
        beforeCommit?.(result.messages);
        historyCursorRef.current = result.nextOffset;
        hasOlderHistoryRef.current = result.hasMore;
        setHasOlderHistory(result.hasMore);
        setHistoryError(null);
        setMessages((current) => {
          const ids = new Set(current.map(message => message.id));
          return [
            ...result.messages.filter(message => !ids.has(message.id)),
            ...current,
          ];
        });
      }
      catch (error) {
        if (epoch === historyEpochRef.current)
          setHistoryError(errorMessage(error));
      }
      finally {
        if (olderInFlightRef.current === request)
          olderInFlightRef.current = null;
      }
    },
    [threadId, source.sourceRevision, hasOlderHistoryRef, setMessages],
  );

  // Search fills the message cache without expanding the rendered window.
  // Reuse the normal page reader and its source/epoch ownership checks.
  const loadAllHistoryForSearch = useCallback(async (signal: AbortSignal) => {
    const assertCurrent = () => {
      if (signal.aborted || !aliveRef.current
        || sourceRef.current.sourceRevision !== source.sourceRevision) {
        throw new DOMException("Search cancelled", "AbortError");
      }
    };
    assertCurrent();
    while (loadingRef.current) {
      await new Promise(resolve => window.setTimeout(resolve, 32));
      assertCurrent();
    }
    if (!cacheEligibleRef.current)
      throw new Error("Thread history is unavailable for search.");
    let restarts = 0;
    while (hasOlderHistoryRef.current) {
      assertCurrent();
      if (olderInFlightRef.current) {
        await new Promise(resolve => window.setTimeout(resolve, 32));
        continue;
      }
      const epoch = historyEpochRef.current;
      const before = historyCursorRef.current;
      await loadOlderHistory();
      assertCurrent();
      if (epoch !== historyEpochRef.current && restarts++ < 3)
        continue;
      if (historyCursorRef.current === before)
        throw new Error("History search could not advance its cursor.");
    }
    return messagesRef.current;
  }, [hasOlderHistoryRef, loadOlderHistory, loadingRef, messagesRef, source.sourceRevision]);

  // Derive the flash-free indicator from the truthful `loadingThread`: show it
  // only if loading outlasts LOADING_INDICATOR_DELAY_MS, and once shown hold it
  // for at least LOADING_INDICATOR_MIN_MS so it can't flash off immediately.
  useEffect(() => {
    if (loadingThread) {
      // Keep the old display visible during an atomic source replacement.
      if (hasWarmSnapshotRef.current) {
        indicatorShownAtRef.current = null;
        setLoadingIndicator(false);
        return;
      }
      const showTimer = setTimeout(() => {
        if (hasWarmSnapshotRef.current)
          return;
        indicatorShownAtRef.current = performance.now();
        setLoadingIndicator(true);
      }, LOADING_INDICATOR_DELAY_MS);
      return () => clearTimeout(showTimer);
    }
    // Loading finished. If the indicator never appeared, just keep it hidden.
    if (indicatorShownAtRef.current === null) {
      setLoadingIndicator(false);
      return;
    }
    // It's showing — hold it for the remainder of its minimum visible duration.
    const remaining
      = LOADING_INDICATOR_MIN_MS
        - (performance.now() - indicatorShownAtRef.current);
    if (remaining <= 0) {
      indicatorShownAtRef.current = null;
      setLoadingIndicator(false);
      return;
    }
    const hideTimer = setTimeout(() => {
      indicatorShownAtRef.current = null;
      setLoadingIndicator(false);
    }, remaining);
    return () => clearTimeout(hideTimer);
  }, [loadingThread]);

  const isRunActive = Boolean(
    recentRun && !matchesSettledRun(recentRun.status),
  );

  useSessionMessageEvents({
    threadId,
    agentSessionId: normalizedAgentSessionId,
    isRunActive,
    loadingRef,
    refreshRecentRun,
    reloadThreadHistory,
    setMessages,
  });

  // The message tree renders against this thread's own workspace, so file
  // links always resolve against the right root — with the view keyed by
  // thread, messages and workspace can no longer disagree.
  const renderWorkspace = {
    workspaceId: workspaceId ?? null,
    workspacePath: workspacePath ?? null,
  };

  return {
    loadingThread,
    loadingIndicator,
    hasOlderHistory,
    loadOlderHistory,
    loadAllHistoryForSearch,
    historyError,
    sessionChanged,
    messages,
    recentRun,
    renderWorkspace,
    reloadThreadHistory,
    refreshRecentRun,
    setMessages,
    setRecentRun,
  };
}
