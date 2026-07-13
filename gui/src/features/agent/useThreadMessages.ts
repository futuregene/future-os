import type { StoredRun } from "../../integrations/storage/threadStore";
import type { AgentMessage } from "./agentThreadTypes";
import { useCallback, useEffect, useRef, useState } from "react";
import i18n from "../../i18n";
import { getSessionEntries, listMessages, listRuns } from "../../integrations/storage/threadStore";
import { errorMessage } from "../../lib/errors";
import { usePolling } from "../../lib/usePolling";
import { upsertFutureReferenceData } from "../markdown/futureReferenceStore";
import { matchesSettledRun, toAgentMessage } from "./agentMessageFormatters";
import { entriesToMessages } from "./entryProjection";
import { applyRunMetadata, restoreMessageActivities } from "./threadRunProjection";

interface UseThreadMessagesInput {
  threadId: string | null;
  workspaceId?: string | null;
}

interface ThreadCacheEntry {
  messages: AgentMessage[];
  recentRun: StoredRun | null;
}

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

  // Reload a thread's messages from the store without flipping the full-screen
  // loading state — used to swap a synthetic streaming bubble for the persisted
  // assistant message once a background run settles.
  const reloadMessagesQuiet = useCallback(async (targetThreadId: string) => {
    try {
      const storedMessages = await listMessages(targetThreadId);
      const agentMessages = storedMessages.map(toAgentMessage);
      const restoredMessages = await restoreMessageActivities(agentMessages, targetThreadId);
      if (targetThreadId !== activeThreadIdRef.current) {
        return;
      }
      setMessages(restoredMessages);
      cachePut(targetThreadId, { messages: restoredMessages, recentRun: null });
    }
    catch {
      // Best-effort refresh: keep the current messages on failure.
    }
  }, []);

  async function loadFromStore(tid: string, wid?: string | null) {
    const [storedMessages] = await Promise.all([listMessages(tid), refreshRecentRun(tid, wid)]);
    const agentMessages = storedMessages.map(toAgentMessage);
    const restoredMessages = await restoreMessageActivities(agentMessages, tid);
    return restoredMessages;
  }

  // Try loading messages from the agent session first; fall back to SQLite.
  async function loadFromAgent(tid: string, wid?: string | null) {
    try {
      const result = await getSessionEntries(tid);
      if (!result?.entries?.length)
        return null;
      const messages = entriesToMessages(result.entries as unknown as import("./entryProjection").SessionEntry[]);
      if (!messages.length)
        return null;
      // Agent JSONL doesn't record a run's GUI-side outcome (failed/cancelled/
      // model) — backfill it from the SQLite `runs` table so a reload keeps the
      // Retry/Continue button, the "stopped" marker, and the model badge.
      const runs = await listRuns(tid).catch(() => [] as StoredRun[]);
      const withRunMeta = applyRunMetadata(messages, runs);
      await refreshRecentRun(tid, wid).catch(() => {});
      return withRunMeta;
    }
    catch {
      return null;
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
        // Background refresh: try agent first, fall back to store.
        try {
          const restored = await loadFromAgent(threadId, workspaceId)
            ?? await loadFromStore(threadId, workspaceId);
          if (!cancelled && threadId === activeThreadIdRef.current) {
            setMessages(restored);
            cachePut(threadId, { messages: restored, recentRun: null });
          }
        }
        catch {
          // Best-effort: keep the cached version on refresh failure.
        }
        return;
      }

      setLoadingThread(true);
      try {
        const restoredMessages = await loadFromAgent(threadId, workspaceId)
          ?? await loadFromStore(threadId, workspaceId);
        if (!cancelled) {
          setMessages(restoredMessages);
          cachePut(threadId, { messages: restoredMessages, recentRun: null });
        }
      }
      catch (error) {
        const message = errorMessage(error);
        if (!cancelled) {
          setMessages([
            {
              id: "store_error",
              role: "assistant",
              authorKey: "author.system",
              content: i18n.t("agent:thread.messagesLoadFailed", { message }),
              createdAt: new Date().toISOString(),
            },
          ]);
        }
      }
      finally {
        if (!cancelled) {
          setLoadingThread(false);
        }
      }
    }

    void loadThreadMessages();

    return () => {
      cancelled = true;
    };
    // loadFromStore is an unstable inner function; the reload must fire on
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
