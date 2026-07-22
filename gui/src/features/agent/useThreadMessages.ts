import type { StoredRun } from "../../integrations/storage/threadStore";
import type { AgentMessage } from "./agentThreadTypes";
import { useCallback, useEffect, useRef, useState } from "react";
import i18n from "../../i18n";
import { getSessionEntries, listRuns } from "../../integrations/storage/threadStore";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { errorMessage } from "../../lib/errors";
import { usePolling } from "../../lib/usePolling";
import { upsertFutureReferenceData } from "../markdown/futureReferenceStore";
import { matchesSettledRun } from "./agentMessageFormatters";
import { entriesToMessages } from "./entryProjection";
import { applyRunMetadata, recoverAbortedTurns } from "./threadRunProjection";

interface UseThreadMessagesInput {
  threadId: string | null;
  workspaceId?: string | null;
  agentSessionId?: string | null;
}

interface ThreadCacheEntry {
  messages: AgentMessage[];
  recentRun: StoredRun | null;
}

type AgentLoadResult
  = | { status: "loaded"; messages: AgentMessage[] }
    | { status: "empty" }
    | { status: "failed"; error: string };

/** Max cached threads before evicting the oldest. */
const CACHE_MAX = 20;

// Flash-free loading indicator (mirrors the right-context panel, useContextData):
// a thread load usually resolves in tens of ms, so hold off showing the "loading"
// text until the load has run this long...
const LOADING_INDICATOR_DELAY_MS = 200;
// ...and once shown, keep it visible at least this long so it can't itself flash.
const LOADING_INDICATOR_MIN_MS = 200;

/**
 * Owns a thread's message list + recent-run status: loads/restores messages on
 * thread switch, keeps a live run polling while one is active, and caches
 * recently-visited threads so switching back is instant.
 */
export function useThreadMessages({ threadId, workspaceId, agentSessionId }: UseThreadMessagesInput) {
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  // Truthful data-loading state: gates pendingPrompt delivery (useAgentThreadState)
  // and must flip the instant a load starts/ends. The UI reads the debounced
  // `loadingIndicator` below instead, so this can stay honest without flashing.
  const [loadingThread, setLoadingThread] = useState(true);
  // Debounced projection of `loadingThread` for the "loading" indicator: only
  // turns on if a load outlasts the delay, and once on stays for a minimum so a
  // fast switch-back can't flash it. Purely presentational.
  const [loadingIndicator, setLoadingIndicator] = useState(false);
  const indicatorShownAtRef = useRef<number | null>(null);
  const [recentRun, setRecentRun] = useState<StoredRun | null>(null);

  // In-memory cache of recently loaded threads. Switching back to a cached
  // thread restores messages instantly and then refreshes in the background.
  const cacheRef = useRef(new Map<string, ThreadCacheEntry>());
  // LRU order: most recently accessed threadId first.
  const lruRef = useRef<string[]>([]);

  function cachePut(tid: string, entry: ThreadCacheEntry) {
    const cache = cacheRef.current;
    if (!cache.has(tid) && cache.size >= CACHE_MAX) {
      const oldest = lruRef.current.pop();
      if (oldest)
        cache.delete(oldest);
    }
    cache.set(tid, entry);
    lruRef.current = [tid, ...lruRef.current.filter(id => id !== tid)];
  }

  function cacheGet(tid: string): ThreadCacheEntry | undefined {
    const entry = cacheRef.current.get(tid);
    if (entry) {
      lruRef.current = [tid, ...lruRef.current.filter(id => id !== tid)];
    }
    return entry;
  }

  // Tracks the thread this view currently shows. Since AgentThread is not keyed
  // by threadId (it stays mounted across thread switches), an async write from a
  // background reload must verify its target is still active before touching
  // state — otherwise a slow load for thread A can overwrite thread B's view.
  const activeThreadIdRef = useRef(threadId);
  activeThreadIdRef.current = threadId;

  // Guard against overlapping refreshes (poll tick, send, thread switch) where a
  // slow response lands after a newer one and writes stale run state — e.g. a
  // previous thread's run after switching. Newest call wins.
  const recentRunGenRef = useRef(0);
  const refreshRecentRun = useCallback(async (targetThreadId: string, targetWorkspaceId?: string | null) => {
    const generation = ++recentRunGenRef.current;
    try {
      const runs = await listRuns(targetThreadId);
      if (generation !== recentRunGenRef.current) {
        return;
      }
      const latestRun = runs[0] ?? null;
      if (targetThreadId === activeThreadIdRef.current) {
        setRecentRun(latestRun);
      }
      if (latestRun) {
        upsertFutureReferenceData(targetWorkspaceId, "run", latestRun.id, latestRun);
      }
    }
    catch {
      // Run-status refresh is best-effort.
    }
  }, []);

  // Reload a thread's messages from the agent session (the sole source of truth)
  // without flipping the full-screen loading state — used to swap a synthetic
  // streaming bubble for the persisted assistant message once a background run
  // settles. Keeps the current messages if the agent has nothing (never blanks).
  const reloadMessagesQuiet = useCallback(async (targetThreadId: string) => {
    const result = await loadFromAgent(targetThreadId);
    if (result.status !== "loaded" || targetThreadId !== activeThreadIdRef.current)
      return;
    setMessages(result.messages);
    cachePut(targetThreadId, { messages: result.messages, recentRun: null });
    // loadFromAgent is a hoisted inner function; this reload fires only on
    // explicit call, so it's intentionally excluded from the deps.
    // eslint-disable-next-line react/exhaustive-deps
  }, []);

  // Reconstruct a thread's messages from the agent session JSONL
  // (get_session_entries) — the only message store (the SQLite messages table
  // was removed). Empty and failed loads stay distinct so a transient Agent
  // error never masquerades as an empty conversation and clears valid cache.
  async function loadFromAgent(tid: string, wid?: string | null): Promise<AgentLoadResult> {
    try {
      const result = await getSessionEntries(tid);
      if (!result?.entries?.length)
        return { status: "empty" };
      const messages = entriesToMessages(result.entries as unknown as import("./entryProjection").SessionEntry[]);
      if (!messages.length)
        return { status: "empty" };
      // Agent JSONL doesn't record a run's GUI-side outcome (failed/cancelled/
      // model) — backfill it from the SQLite `runs` table so a reload keeps the
      // Retry/Continue button, the "stopped" marker, and the model badge.
      const runs = await listRuns(tid).catch(() => [] as StoredRun[]);
      const withRunMeta = applyRunMetadata(messages, runs);
      // An aborted turn has no reply in the session JSONL — recover the partial
      // text the model streamed (persisted as run events) so it isn't lost.
      const recovered = await recoverAbortedTurns(withRunMeta);
      await refreshRecentRun(tid, wid).catch(() => {});
      return { status: "loaded", messages: recovered };
    }
    catch (error) {
      return { status: "failed", error: errorMessage(error) };
    }
  }

  useEffect(() => {
    let cancelled = false;

    async function loadThreadMessages() {
      if (!threadId) {
        setMessages([]);
        setLoadingThread(false);
        return;
      }

      // Check cache first — restore instantly if available, then refresh.
      const cached = cacheGet(threadId);
      if (cached) {
        setMessages(cached.messages);
        setRecentRun(cached.recentRun);
        setLoadingThread(false);
        // Background refresh from the agent session (empty when it has none).
        const result = await loadFromAgent(threadId, workspaceId);
        if (!cancelled && threadId === activeThreadIdRef.current && result.status !== "failed") {
          const restored = result.status === "loaded" ? result.messages : [];
          setMessages(restored);
          cachePut(threadId, { messages: restored, recentRun: null });
        }
        return;
      }

      setLoadingThread(true);
      const result = await loadFromAgent(threadId, workspaceId);
      if (!cancelled) {
        if (result.status === "failed") {
          setMessages([
            {
              id: "store_error",
              role: "assistant",
              authorKey: "author.system",
              content: i18n.t("agent:thread.messagesLoadFailed", { message: result.error }),
              createdAt: new Date().toISOString(),
            },
          ]);
        }
        else {
          const restoredMessages = result.status === "loaded" ? result.messages : [];
          setMessages(restoredMessages);
          cachePut(threadId, { messages: restoredMessages, recentRun: null });
        }
        setLoadingThread(false);
      }
    }

    void loadThreadMessages();

    return () => {
      cancelled = true;
    };
    // loadFromAgent is an unstable inner function; the reload must fire on
    // thread/workspace change only, not on every render, so it's excluded.
    // eslint-disable-next-line react/exhaustive-deps
  }, [refreshRecentRun, workspaceId, threadId]);

  // Derive the flash-free indicator from the truthful `loadingThread`: show it
  // only if loading outlasts LOADING_INDICATOR_DELAY_MS, and once shown hold it
  // for at least LOADING_INDICATOR_MIN_MS so it can't flash off immediately.
  useEffect(() => {
    if (loadingThread) {
      const showTimer = setTimeout(() => {
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
    const remaining = LOADING_INDICATOR_MIN_MS - (performance.now() - indicatorShownAtRef.current);
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

  const isRunActive = Boolean(recentRun && !matchesSettledRun(recentRun.status));

  // Poll the run's status while it's in flight so a background settle is picked
  // up. `refreshRecentRun` guards its own state (generation + active-thread ref),
  // so the immediate tick and any thread-switch overlap are race-safe.
  usePolling(
    () => {
      if (threadId)
        void refreshRecentRun(threadId, workspaceId);
    },
    1500,
    { enabled: Boolean(threadId) && isRunActive, deps: [threadId, workspaceId, refreshRecentRun] },
  );

  // When another client (TUI, CLI, phone) is streaming on this thread's
  // session, ask the Tauri backend to create a synthetic run and subscribe to
  // the agent's live event stream.  Events are persisted locally, so the
  // existing reattach machinery (refreshRecentRun → useRunReattach) picks up
  // the streaming bubble automatically.  No local StoredRun existed before.
  //
  // Guards:
  //   attachedRef — don't re-attach while the same streaming session is active
  //   isRunActive — don't attach while a local run (incl. our own synthetic
  //     one) is still in flight; the existing reattach poll handles it
  const attachedRef = useRef(false);
  usePolling(
    async () => {
      if (!threadId || isRunActive)
        return;
      // Per-thread streaming check for the OPEN thread only (reattach).
      // Deliberately uncached: attach decisions need fresh truth, unlike
      // the thread-list indicator which uses the bulk poll.
      let streaming = false;
      try {
        const raw = await invokeCommand<Record<string, unknown>>("get_thread_agent_state", { threadId });
        streaming = raw.isStreaming === true;
      }
      catch {
        // Agent unreachable — treat as not streaming; retry next tick.
      }
      if (streaming && !attachedRef.current) {
        try {
          const result = await invokeCommand<{ runId?: string }>("attach_remote_stream", { threadId });
          // An empty runId means a run was already recently settled — don't
          // retry until the agent confirms streaming has stopped.
          attachedRef.current = true;
          if (result?.runId) {
            // Reload agent entries so user message + history are visible,
            // then kick refreshRecentRun so useRunReattach picks up the
            // synthetic run and starts the streaming bubble on its own.
            await reloadMessagesQuiet(threadId);
            await refreshRecentRun(threadId, workspaceId);
          }
        }
        catch {
          // Agent unreachable — will retry next tick.
        }
      }
      else if (!streaming) {
        attachedRef.current = false;
      }
    },
    2000,
    {
      enabled: Boolean(threadId),
      deps: [threadId, refreshRecentRun, reloadMessagesQuiet, workspaceId, isRunActive],
    },
  );

  // ── Real-time user_message from StreamEvents observer ────────────
  // Inserts the user message directly from the Tauri event stream
  // for zero-latency display.  All other events (text_chunk, thinking,
  // tools, agent_end) continue through the synthetic run → useRunReattach
  // path to avoid conflicting with the existing streaming bubble logic.
  useEffect(() => {
    if (!threadId || !agentSessionId)
      return;
    const handler = (ev: Event) => {
      const detail = (ev as CustomEvent).detail as {
        sessionId: string;
        eventType: string;
        payload: Record<string, unknown>;
      } | undefined;
      if (!detail || detail.sessionId !== agentSessionId)
        return;
      if (detail.eventType !== "user_message")
        return;

      const text = typeof detail.payload.text === "string" ? detail.payload.text : "";
      if (!text)
        return;
      setMessages((prev) => {
        // Dedup: skip if the last user message has identical text.
        // Checking only the last message avoids suppressing legitimate
        // repeated prompts (e.g. sending "continue" twice).
        const userMsgs = prev.filter(m => m.role === "user");
        const lastUser = userMsgs[userMsgs.length - 1];
        if (lastUser && lastUser.content === text)
          return prev;
        return [...prev, {
          id: `user_${Date.now()}`,
          role: "user",
          authorKey: "author.you",
          content: text,
          status: "complete",
          createdAt: new Date().toISOString(),
        } satisfies AgentMessage];
      });
    };
    window.addEventListener("future:agent-event", handler);
    return () => window.removeEventListener("future:agent-event", handler);
  }, [threadId, agentSessionId]);

  return {
    loadingThread,
    loadingIndicator,
    messages,
    recentRun,
    reloadMessagesQuiet,
    refreshRecentRun,
    setMessages,
    setRecentRun,
  };
}
