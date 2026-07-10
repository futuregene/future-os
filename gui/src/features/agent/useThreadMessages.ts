import type { StoredRun } from "../../integrations/storage/threadStore";
import type { AgentMessage } from "./agentThreadTypes";
import { useCallback, useEffect, useRef, useState } from "react";
import i18n from "../../i18n";
import { listMessages, listRuns } from "../../integrations/storage/threadStore";
import { errorMessage } from "../../lib/errors";
import { upsertFutureReferenceData } from "../markdown/futureReferenceStore";
import { matchesSettledRun, toAgentMessage } from "./agentMessageFormatters";
import { restoreMessageActivities } from "./threadRunProjection";

interface UseThreadMessagesInput {
  threadId: string | null;
  workspaceId?: string | null;
}

interface ThreadCacheEntry {
  messages: AgentMessage[];
  recentRun: StoredRun | null;
  scrollTop: number;
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
  // Scroll position restored from cache on switch-back. Sticky auto-scroll reads
  // this as a prop and resets to null after applying.
  const [restoredScrollTop, setRestoredScrollTop] = useState<number | null>(null);

  // In-memory cache of recently loaded threads. Switching back to a cached
  // thread restores messages instantly and then refreshes in the background.
  const cacheRef = useRef(new Map<string, ThreadCacheEntry>());
  // LRU order: most recently accessed threadId first.
  const lruRef = useRef<string[]>([]);

  function cachePut(tid: string, entry: ThreadCacheEntry) {
    const cache = cacheRef.current;
    // Evict oldest if at capacity and this thread isn't already cached.
    if (!cache.has(tid) && cache.size >= CACHE_MAX) {
      const oldest = lruRef.current.pop();
      if (oldest)
        cache.delete(oldest);
    }
    // Preserve any previously-saved scroll position.
    const existing = cache.get(tid);
    if (existing && existing.scrollTop > 0 && entry.scrollTop === 0) {
      entry.scrollTop = existing.scrollTop;
    }
    cache.set(tid, entry);
    lruRef.current = [tid, ...lruRef.current.filter(id => id !== tid)];
  }

  function cacheGet(tid: string): ThreadCacheEntry | undefined {
    const entry = cacheRef.current.get(tid);
    if (entry) {
      // Bump to front of LRU.
      lruRef.current = [tid, ...lruRef.current.filter(id => id !== tid)];
    }
    return entry;
  }

  /** Persist scroll position directly into the cache for the given thread. */
  const saveScrollPosition = useCallback((tid: string, scrollTop: number) => {
    const entry = cacheRef.current.get(tid);
    if (entry) {
      entry.scrollTop = scrollTop;
    }
  }, []);

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
      // Run-status refresh is best-effort: a failure here must not blank the
      // thread (it runs alongside listMessages in loadThreadMessages via
      // Promise.all) or abort an in-flight send. Keep the previous recentRun
      // until the next poll. The waiting-approval prompt is rendered separately
      // by AgentThread from `activeApproval`, so no message rewrite is needed.
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
      // Drop the result if the user switched threads while this was in flight —
      // writing it now would paint the old thread's messages into the new view.
      if (targetThreadId !== activeThreadIdRef.current) {
        return;
      }
      setMessages(restoredMessages);
      cachePut(targetThreadId, { messages: restoredMessages, recentRun: null, scrollTop: 0 });
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
        if (cached.scrollTop > 0) setRestoredScrollTop(cached.scrollTop);
        setMessages(cached.messages);
        setRecentRun(cached.recentRun);
        setLoadingThread(false);
        // Background refresh to pick up new messages / run changes.
        try {
          const restored = await loadFromStore(threadId, workspaceId);
          if (!cancelled && threadId === activeThreadIdRef.current) {
            setMessages(restored);
            cachePut(threadId, { messages: restored, recentRun: null, scrollTop: 0 });
          }
        }
        catch {
          // Best-effort: keep the cached version on refresh failure.
        }
        return;
      }

      setRestoredScrollTop(null);
      setLoadingThread(true);
      try {
        const restoredMessages = await loadFromStore(threadId, workspaceId);
        if (!cancelled) {
          setMessages(restoredMessages);
          cachePut(threadId, { messages: restoredMessages, recentRun: null, scrollTop: 0 });
        }
      }
      catch (error) {
        const message = errorMessage(error);
        if (!cancelled) {
          setMessages([
            {
              id: "store_error",
              role: "assistant",
              author: i18n.t("agent:author.system"),
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
  }, [refreshRecentRun, workspaceId, threadId]);

  // Stable flag so the poll effect keys on "is a run active", not on the
  // recentRun object identity — refreshRecentRun replaces recentRun with a fresh
  // object every tick, which would otherwise tear down and rebuild the interval
  // (and re-render) each 1.5s.
  const isRunActive = Boolean(recentRun && !matchesSettledRun(recentRun.status));

  useEffect(() => {
    if (!threadId || !isRunActive)
      return;

    const timer = window.setInterval(() => {
      void refreshRecentRun(threadId, workspaceId);
    }, 1500);

    return () => window.clearInterval(timer);
  }, [isRunActive, refreshRecentRun, workspaceId, threadId]);

  return {
    loadingThread,
    messages,
    recentRun,
    reloadMessagesQuiet,
    refreshRecentRun,
    setMessages,
    setRecentRun,
    restoredScrollTop,
    setRestoredScrollTop,
    saveScrollPosition,
  };
}
