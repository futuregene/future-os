import type { AgentMessage } from "@future-os/thread-projection";
import type { Dispatch, SetStateAction } from "react";
import type { StoredRun } from "../../integrations/storage/threadStore";
import {
  entriesToTurns,
  matchesSettledRun,
  turnsToMessages,
  upsertUserMessage,
  userMessageFromEvent,
} from "@future-os/thread-projection";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import i18n from "../../i18n";
import {
  getLatestRun,
  getRun,
  getSessionEntriesPage,
  listRuns,
} from "../../integrations/storage/threadStore";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { errorMessage } from "../../lib/errors";
import { emitFutureEvent } from "../../lib/futureEvents";
import { reconcileThreadHistory } from "./reconcileThreadHistory";
import {
  getThreadHistoryCursor,
  getThreadMessageSnapshot,
  setThreadMessageSnapshot,
} from "./threadMessageCache";
import {
  applyJournalRunOutcomes,
  applyRunMetadata,
  buildStreamingPreview,
  mergeStreamingPreview,
  recoverAbortedTurns,
  recoverFailedRuns,
} from "./threadRunProjection";

interface UseThreadMessagesInput {
  threadId: string | null;
  workspaceId?: string | null;
  workspacePath?: string | null;
  agentSessionId?: string | null;
}

type AgentLoadResult
  = | {
    status: "loaded";
    messages: AgentMessage[];
    hasMore: boolean;
    nextOffset: number;
  }
  | { status: "empty" }
  | { status: "failed"; error: string };

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
  const [source, setSource] = useState({ id: normalizedAgentSessionId, version: 0, owner: 0 });
  if (source.id !== normalizedAgentSessionId) {
    setSource({ id: normalizedAgentSessionId, version: source.version + 1, owner: source.owner + (source.id === null ? 0 : 1) });
  }
  const sourceRef = useRef(source);
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
  const ownerChanged = source.owner !== sourceRef.current.owner;
  const loadingThread = blockingLoad || ownerChanged;
  const loadingRef = useRef(loadingThread);
  loadingRef.current = loadingThread;
  // Debounced projection of `loadingThread` for the "loading" indicator: only
  // comes on if a load outlasts the delay, and once on stays for a minimum so a
  // fast switch-back can't flash it. Purely presentational.
  const [loadingIndicator, setLoadingIndicator] = useState(false);
  const indicatorShownAtRef = useRef<number | null>(null);
  const [recentRunState, setRecentRunState] = useState<StoredRun | null>(null);
  const recentRun = ownerChanged ? null : recentRunState;
  const setRecentRun: Dispatch<SetStateAction<StoredRun | null>> = useCallback((value) => {
    if (aliveRef.current && sourceRef.current.owner === source.owner)
      setRecentRunState(value);
  }, [source.owner]);
  const [hasOlderHistory, setHasOlderHistory] = useState(
    initialCursor?.hasMore ?? false,
  );
  const [historyError, setHistoryError] = useState<string | null>(null);
  const historyCursorRef = useRef<number | null>(initialCursor?.before ?? null);
  const hasOlderHistoryRef = useRef(hasOlderHistory);
  hasOlderHistoryRef.current = hasOlderHistory;
  const olderInFlightRef = useRef<object | null>(null);
  const historyEpochRef = useRef(0);
  const messagesRef = useRef(messages);
  messagesRef.current = messages;
  const setMessages: Dispatch<SetStateAction<AgentMessage[]>> = useCallback((value) => {
    if (!aliveRef.current || sourceRef.current.owner !== source.owner)
      return;
    const next = typeof value === "function" ? value(messagesRef.current) : value;
    messagesRef.current = next;
    setMessagesState(next);
  }, [source.owner]);
  useLayoutEffect(() => {
    if (sourceRef.current.version === source.version)
      return;
    const replacing = sourceRef.current.owner !== source.owner;
    sourceRef.current = source;
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
    async (targetThreadId: string, _targetWorkspaceId?: string | null) => {
      if (!aliveRef.current || sourceRef.current.version !== source.version)
        return;
      const generation = ++recentRunGenRef.current;
      try {
        // One row, not the thread's whole run history. Invoked on push events
        // (thread-runtime-updated terminal / remote-activity) and loads — there is
        // no longer a periodic timer driving it.
        const latestRun = await getLatestRun(targetThreadId);
        if (!aliveRef.current || sourceRef.current.version !== source.version || generation !== recentRunGenRef.current) {
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
    [source.version, setRecentRun],
  );

  // Reconstruct the thread's messages from the agent Agent transcript
  // (get_session_entries). Empty and failed loads stay distinct so a transient Agent
  // error never masquerades as an empty conversation.
  const loadFromAgent = useCallback(async (
    tid: string,
    _wid?: string | null,
    activeRunId?: string | null,
    activeRunStartedAt?: number | null,
    before: number | null = null,
  ): Promise<AgentLoadResult> => {
    try {
      const result = await getSessionEntriesPage(tid, before);
      const entries = result?.entries ?? [];
      if (!entries.length) {
        return {
          status: "loaded",
          messages: [],
          hasMore: result.hasMore,
          nextOffset: result.nextOffset,
        };
      }
      const turns = entriesToTurns(entries as unknown as import("@future-os/thread-projection").SessionEntry[]);
      // The shared projection intentionally leaves run identity off user
      // bubbles. Desktop reconciliation needs both halves of an exchange;
      // take identity from the canonical turn (never guess by text/time).
      const userRuns = new Map(turns.flatMap(node =>
        node.kind === "turn" && node.turn.runId
          ? [[node.turn.user.id, node.turn.runId] as const]
          : [],
      ));
      const messages = applyJournalRunOutcomes(turnsToMessages(turns));
      if (!messages.length) {
        return {
          status: "loaded",
          messages: [],
          hasMore: result.hasMore,
          nextOffset: result.nextOffset,
        };
      }
      // Agent transcript doesn't record a run's GUI-side outcome (failed/cancelled/
      // model) — backfill it from the SQLite `runs` table so a reload keeps the
      // Retry/Continue button, the "stopped" marker, and the model badge.
      const allRuns = await listRuns(tid).catch(() => [] as StoredRun[]);
      // Do not recover failures or events from unloaded history into this page.
      const firstTime = Date.parse(messages[0]!.createdAt);
      const lastTime
        = before === null
          ? Infinity
          : Date.parse(messages[messages.length - 1]!.createdAt);
      const runs = allRuns.filter((run) => {
        const time = run.startedAt ?? run.createdAt;
        return (
          (!result.hasMore || (run.endedAt ?? run.updatedAt) >= firstTime)
          && time <= lastTime
        );
      });
      const withRunMeta = applyRunMetadata(messages, runs);
      // An aborted exchange has no reply in the Agent transcript — recover the partial
      // text the model streamed (persisted as run events) so it isn't lost.
      const recovered = await recoverAbortedTurns(withRunMeta);
      // A run that failed before any assistant entry was saved (e.g. the model
      // API rejected the first call) leaves no trace in the Agent transcript —
      // rebuild its failure bubble from the run record so the error survives a
      // thread switch instead of silently disappearing.
      const withFailures = recoverFailedRuns(recovered, runs);
      // An in-flight run is folded into the SAME array here: history and live
      // bubble land in one setMessages, so opening an active conversation
      // paints both in a single frame instead of history then bubble. The fold
      // also dedups a mid-run snapshot the agent's save_callback may already
      // have persisted for this exchange (mergeStreamingPreview). Verified
      // against the run row so a settle that raced the reload never resurrects
      // a bubble for a finished run.
      const liveBubble = activeRunId
        ? await getRun(activeRunId)
            .then(run =>
              run && !matchesSettledRun(run.status)
                ? buildStreamingPreview(activeRunId, activeRunStartedAt ?? null)
                : null,
            )
            .catch(() => null)
        : null;
      const finalMessages = liveBubble
        ? mergeStreamingPreview(withFailures, liveBubble)
        : withFailures;
      return {
        status: "loaded",
        messages: finalMessages.map(message =>
          message.role === "user" && userRuns.has(message.id)
            ? { ...message, runId: userRuns.get(message.id) }
            : message,
        ),
        hasMore: result.hasMore,
        nextOffset: result.nextOffset,
      };
    }
    catch (error) {
      return { status: "failed", error: errorMessage(error) };
    }
  }, []);

  // All tail reads share one request order. Settling may replace a streaming
  // snapshot, but never bypasses source ownership or overwrites newer writes.
  const reloadMessagesQuiet = useCallback(
    async (targetThreadId: string, settle = false) => {
      if (!aliveRef.current || sourceRef.current.version !== source.version)
        return;
      const request = ++tailRequestRef.current;
      const baseline = messagesRef.current;
      await refreshRecentRun(targetThreadId);
      if (!aliveRef.current || request !== tailRequestRef.current || sourceRef.current.version !== source.version)
        return;
      const result = await loadFromAgent(
        targetThreadId,
        undefined,
        activeRunRef.current.runId,
        activeRunRef.current.startedAt,
      );
      if (!aliveRef.current || request !== tailRequestRef.current || sourceRef.current.version !== source.version)
        return;
      if (result.status !== "loaded") {
        setHistoryError(result.status === "failed" ? result.error : i18n.t("agent:thread.messagesLoadFailed"));
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
    [loadFromAgent, refreshRecentRun, source.version],
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
    void reloadMessagesQuiet(threadId, true);
  }, [reloadMessagesQuiet, threadId, workspaceId]);

  const loadOlderHistory = useCallback(
    async (beforeCommit?: (messages: AgentMessage[]) => void) => {
      if (
        !threadId
        || !aliveRef.current
        || sourceRef.current.version !== source.version
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
        const result = await loadFromAgent(
          threadId,
          undefined,
          null,
          null,
          before,
        );
        if (epoch !== historyEpochRef.current)
          return;
        if (result.status === "failed")
          throw new Error(result.error);
        if (result.status === "empty") {
          throw new Error(
            "History page returned no entries before its cursor.",
          );
        }
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
    [threadId, loadFromAgent, setMessages, source.version],
  );

  // Search fills the message cache without expanding the rendered window.
  // Reuse the normal page reader and its source/epoch ownership checks.
  const loadAllHistoryForSearch = useCallback(async (signal: AbortSignal) => {
    const assertCurrent = () => {
      if (signal.aborted || !aliveRef.current
        || sourceRef.current.version !== source.version) {
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
  }, [loadOlderHistory, source.version]);

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

  // Remote runs are discovered from the already-open session event stream.
  // This replaces the old per-thread 2s get_state poll.
  const attachedRef = useRef(false);

  // ── Real-time user_message from StreamEvents observer ────────────
  // Inserts the user message directly from the Tauri event stream
  // for zero-latency display.  All other events (text_chunk, thinking,
  // tools, agent_end) continue through the synthetic run → useRunReattach
  // path to avoid conflicting with the existing streaming bubble logic.
  useEffect(() => {
    if (!threadId || !normalizedAgentSessionId)
      return;
    const handler = (ev: Event) => {
      const detail = (ev as CustomEvent).detail as
        | {
          threadId: string;
          sessionId: string;
          eventType: string;
          payload: Record<string, unknown>;
        }
        | undefined;
      // Only this conversation's events apply to this instance — other
      // conversations live on their own keyed AgentThread instances.
      if (
        !detail
        || detail.threadId !== threadId
        || detail.sessionId !== normalizedAgentSessionId
      ) {
        return;
      }
      if (detail.eventType === "agent_end") {
        attachedRef.current = false;
        emitFutureEvent("agent_end", undefined);
        return;
      }
      if (detail.eventType === "agent_start") {
        if (isRunActive || attachedRef.current)
          return;
        attachedRef.current = true;
        void invokeCommand<{ runId?: string }>("attach_remote_stream", {
          threadId,
        })
          .then(async (result) => {
            if (!result?.runId)
              return;
            await reloadMessagesQuiet(threadId);
            await refreshRecentRun(threadId, workspaceId);
          })
          .catch(() => {
            attachedRef.current = false;
          });
        return;
      }
      if (
        detail.eventType === "compaction_started"
        || detail.eventType === "compaction_committed"
        || detail.eventType === "compaction_failed"
      ) {
        // Run-scoped compaction is already projected from the persisted run
        // event log. This direct path is for standalone/manual and model-switch
        // compaction, which has no active run bubble to host its status.
        if (isRunActive || attachedRef.current)
          return;
        const operationId
          = typeof detail.payload.operation_id === "string"
            ? detail.payload.operation_id
            : `session_${Date.now()}`;
        const messageId = `compaction_${operationId}`;
        const checkpointId
          = typeof detail.payload.checkpoint_id === "string"
            ? detail.payload.checkpoint_id
            : operationId;
        const tokensBefore
          = typeof detail.payload.tokens_before === "number"
            ? detail.payload.tokens_before
            : undefined;
        const error
          = typeof detail.payload.error === "string"
            ? detail.payload.error
            : undefined;
        const trigger
          = typeof detail.payload.trigger === "string"
            ? detail.payload.trigger
            : undefined;
        const status
          = detail.eventType === "compaction_started"
            ? ("running" as const)
            : detail.eventType === "compaction_failed"
              ? ("failed" as const)
              : ("completed" as const);
        setMessages((prev) => {
          const segment = {
            id: checkpointId,
            kind: "compaction" as const,
            ...(tokensBefore != null && tokensBefore > 0
              ? { tokensBefore }
              : {}),
            ...(trigger ? { trigger } : {}),
            ...(status !== "completed" ? { status } : {}),
            ...(error ? { error } : {}),
          };
          const existing = prev.findIndex(
            message => message.id === messageId,
          );
          const message: AgentMessage = {
            id: messageId,
            role: "assistant",
            authorKey: "author.researchCopilot",
            content: "",
            status: "complete",
            createdAt: new Date().toISOString(),
            segments: [segment],
          };
          if (existing < 0)
            return [...prev, message];
          const next = [...prev];
          next[existing] = message;
          return next;
        });
        return;
      }
      if (detail.eventType !== "user_message")
        return;

      // A user_message that lands while this thread's load is still in flight
      // would append onto the not-yet-committed base — dropping it is
      // lossless, the Agent history load carries the persisted entry.
      if (loadingRef.current)
        return;

      const user = userMessageFromEvent(detail.payload);
      if (!user) {
        // An identity-less event can only invalidate history; text is not a key.
        void reloadMessagesQuiet(threadId);
        return;
      }
      setMessages(prev => upsertUserMessage(prev, user));
    };
    window.addEventListener("future:agent-event", handler);
    return () => window.removeEventListener("future:agent-event", handler);
  }, [
    normalizedAgentSessionId,
    isRunActive,
    refreshRecentRun,
    reloadMessagesQuiet,
    setMessages,
    threadId,
    workspaceId,
  ]);

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
    reloadMessagesQuiet,
    refreshRecentRun,
    setMessages,
    setRecentRun,
  };
}
