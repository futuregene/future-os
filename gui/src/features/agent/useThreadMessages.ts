import type { StoredRun } from "../../integrations/storage/threadStore";
import type { AgentMessage } from "./agentThreadTypes";
import { useCallback, useEffect, useRef, useState } from "react";
import i18n from "../../i18n";
import { getSessionEntries, listRuns } from "../../integrations/storage/threadStore";
import { errorMessage } from "../../lib/errors";
import { usePolling } from "../../lib/usePolling";
import { upsertFutureReferenceData } from "../markdown/futureReferenceStore";
import { matchesSettledRun } from "./agentMessageFormatters";
import { entriesToMessages } from "./entryProjection";
import { applyRunMetadata, recoverAbortedTurns } from "./threadRunProjection";

interface UseThreadMessagesInput {
  threadId: string | null;
  workspaceId?: string | null;
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

/**
 * Owns a thread's message list + recent-run status: loads/restores messages on
 * thread switch, keeps a live run polling while one is active, and caches
 * recently-visited threads so switching back is instant.
 */
export function useThreadMessages({ threadId, workspaceId }: UseThreadMessagesInput) {
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [loadingThread, setLoadingThread] = useState(true);
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

  return {
    loadingThread,
    messages,
    recentRun,
    reloadMessagesQuiet,
    refreshRecentRun,
    setMessages,
    setRecentRun,
  };
}
