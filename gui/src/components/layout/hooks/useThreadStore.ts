import type { Dispatch, SetStateAction } from "react";
import type { StoredRun, StoredThread, StoredWorkspace } from "../../../integrations/storage/threadStore";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import i18n from "../../../i18n";
import { listStreamingThreadIds, prefetchAgentState } from "../../../integrations/agent/agentStateCache";
import {
  getRecentOrCreateDefaultThread,
  initializeAppStore,
  listLatestRunInfos,
  listThreads,
  listWorkspaces,
} from "../../../integrations/storage/threadStore";
import { errorMessage } from "../../../lib/errors";
import { usePolling } from "../../../lib/usePolling";

export interface ThreadRunInfo {
  status: StoredRun["status"];
  endedAt: number | null;
  /** Latest StoredRun represented by this thread-level projection. */
  runId?: string;
  /** Process-global Tauri runtime revision; absent on reconciliation snapshots. */
  revision?: number;
}

type ThreadRunStatuses = Record<string, ThreadRunInfo | undefined>;
type ThreadStreamingStatuses = Record<string, boolean>;

interface ThreadRuntimeUpdate {
  threadId: string;
  runId: string;
  revision: number;
  status: string;
  resetProjection: boolean;
}

interface ThreadStreamingUpdate {
  revision: number;
  threadIds: string[];
}

export function reduceThreadRunStatus(
  previous: ThreadRunStatuses,
  update: ThreadRuntimeUpdate,
  endedAt = Date.now(),
): ThreadRunStatuses {
  const current = previous[update.threadId];
  if (current?.revision !== undefined && update.revision <= current.revision)
    return previous;
  const terminal = ["completed", "failed", "cancelled"].includes(update.status);
  return {
    ...previous,
    [update.threadId]: {
      runId: update.runId,
      revision: update.revision,
      status: terminal ? update.status as ThreadRunInfo["status"] : "running",
      endedAt: terminal ? endedAt : null,
    },
  };
}

function streamingStatuses(threadIds: string[]): ThreadStreamingStatuses {
  return Object.fromEntries(threadIds.map(threadId => [threadId, true]));
}

export interface ThreadStore {
  threads: StoredThread[];
  workspaces: StoredWorkspace[];
  activeThread: StoredThread | null;
  activeWorkspace: StoredWorkspace | null;
  activeThreads: StoredThread[];
  activeThreadId: string | null;
  setActiveThreadId: Dispatch<SetStateAction<string | null>>;
  threadRunStatuses: ThreadRunStatuses;
  threadStreamingStatuses: ThreadStreamingStatuses;
  loadingStore: boolean;
  storeError: string | null;
  /**
   * Reload threads + workspaces and reconcile the active thread (prefer the
   * given id, else keep the current one if still selectable, else the first).
   */
  refreshStore: (nextActiveThreadId?: string) => Promise<void>;
}

/**
 * Owns the local thread/workspace store: bootstrap (init + stale-approval
 * cleanup + recent/default thread), the threads/workspaces lists and the
 * derived active thread/workspace. Run status is driven primarily by the
 * `thread-runtime-updated` push event; a low-frequency (30s) reconciliation
 * pass backstops lost events / backend restarts. Cross-client streaming status
 * is also pushed after one initial snapshot.
 */
export function useThreadStore(): ThreadStore {
  const [threads, setThreads] = useState<StoredThread[]>([]);
  const [threadRunStatuses, setThreadRunStatuses] = useState<ThreadRunStatuses>({});
  const [threadStreamingStatuses, setThreadStreamingStatuses] = useState<ThreadStreamingStatuses>({});
  const [workspaces, setWorkspaces] = useState<StoredWorkspace[]>([]);
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);
  const [loadingStore, setLoadingStore] = useState(true);
  const [storeError, setStoreError] = useState<string | null>(null);

  // Mirror the active id into a ref so `refreshStore` can read the latest value
  // without listing it as a dependency — that kept it stable-free, recreating
  // it (and cascading to consumers) on every thread selection (B-14b).
  const activeThreadIdRef = useRef(activeThreadId);
  activeThreadIdRef.current = activeThreadId;

  const activeThread = useMemo(
    () => threads.find(thread => thread.id === activeThreadId) ?? null,
    [activeThreadId, threads],
  );
  const activeWorkspace = useMemo(
    () =>
      workspaces.find(workspace => workspace.id === activeThread?.workspaceId)
      ?? workspaces.find(workspace => workspace.kind === "user")
      ?? null,
    [activeThread?.workspaceId, workspaces],
  );
  const activeThreads = useMemo(
    () => threads.filter(thread => thread.status === "active"),
    [threads],
  );

  // usePolling doesn't cancel in-flight async, and refreshStore can overlap a
  // poll tick — so guard against a slow run-status fetch landing after a newer
  // one and overwriting it with stale data (incl. removed threads).
  const runStatusGenRef = useRef(0);
  const refreshThreadRunStatuses = useCallback(async (nextThreads: StoredThread[]) => {
    const generation = ++runStatusGenRef.current;
    // Per-thread catch (not a bare Promise.all): one thread's failed listRuns
    // must not reject the whole batch — that would blank every thread's run
    // indicator and surface an unhandled rejection. A failed thread keeps
    // its previous status and self-heals on the next reconciliation pass.
    const ids = nextThreads.map(t => t.id);
    let infos: Array<{ threadId: string; runId: string; status: string; endedAt: number | null }> = [];
    try {
      infos = await listLatestRunInfos(ids);
    }
    catch {
      // Transient error — keep previous statuses, retry next tick.
    }
    if (generation !== runStatusGenRef.current) {
      return;
    }
    const infoMap = new Map(infos.map(info => [info.threadId, info]));
    setThreadRunStatuses((previous) => {
      let changed = false;
      const next: ThreadRunStatuses = {};
      for (const thread of nextThreads) {
        const info = infoMap.get(thread.id);
        const current = previous[thread.id];
        const value: ThreadRunInfo | undefined = info
          ? {
              endedAt: info.endedAt ?? null,
              runId: info.runId,
              revision: current?.runId === info.runId ? current.revision : undefined,
              status: info.status as ThreadRunInfo["status"],
            }
          : current; // keep old if no new info
        if (!changed) {
          const prev = previous[thread.id];
          changed = prev?.status !== value?.status
            || prev?.endedAt !== value?.endedAt
            || prev?.runId !== value?.runId;
        }
        next[thread.id] = value;
      }
      return changed ? next : previous;
    });
  }, []);

  // Guard against overlapping refreshes (rapid deletes/creates, each calling
  // refreshStore): a slow listThreads landing after a newer one would revive a
  // stale list — briefly resurrecting a just-deleted thread and possibly
  // re-selecting it. Newest call wins.
  const refreshStoreGenRef = useRef(0);
  const refreshStore = useCallback(async (nextActiveThreadId?: string) => {
    const generation = ++refreshStoreGenRef.current;
    const [nextThreads, nextWorkspaces] = await Promise.all([listThreads(), listWorkspaces()]);
    if (generation !== refreshStoreGenRef.current) {
      return;
    }
    const selectableThreads = nextThreads.filter(thread => thread.status === "active");
    setThreads(nextThreads);
    setWorkspaces(nextWorkspaces);
    // Run-status reconciliation is driven solely by the low-frequency pass
    // below. Kicking it off here too would duplicate every fetch.
    const currentActiveThreadId = activeThreadIdRef.current;
    if (nextActiveThreadId && selectableThreads.some(thread => thread.id === nextActiveThreadId)) {
      setActiveThreadId(nextActiveThreadId);
    }
    else if (currentActiveThreadId && selectableThreads.some(thread => thread.id === currentActiveThreadId)) {
      setActiveThreadId(currentActiveThreadId);
    }
    else {
      setActiveThreadId(selectableThreads[0]?.id ?? null);
    }
  }, []);

  useEffect(() => {
    // Hand-rolled cancel guard (not useAsyncResource): a multi-step bootstrap
    // (init store → recent thread → threads+workspaces) writing several
    // states, not a single resource. See gui/CLAUDE.md §4.
    // Stale-run convergence intentionally does NOT happen here: it lives in
    // the backend's setup (once per process). Bootstrap re-runs on every
    // webview reload, where the backend may still own live runs.
    let cancelled = false;

    async function bootstrapStore() {
      setLoadingStore(true);
      try {
        await initializeAppStore();
        const recentThread = await getRecentOrCreateDefaultThread(i18n.t("common:newChat"));
        const [nextThreads, nextWorkspaces] = await Promise.all([listThreads(), listWorkspaces()]);
        if (cancelled) {
          return;
        }
        setThreads(nextThreads);
        setWorkspaces(nextWorkspaces);
        // The reconciliation pass below ticks as soon as `activeThreads`
        // becomes non-empty, so bootstrap needs no duplicate status fetch.
        setActiveThreadId(recentThread.id);
        setStoreError(null);
      }
      catch (error) {
        if (!cancelled) {
          setStoreError(errorMessage(error));
        }
      }
      finally {
        if (!cancelled) {
          setLoadingStore(false);
        }
      }
    }

    void bootstrapStore();

    return () => {
      cancelled = true;
    };
  }, []);

  // Push updates are the primary path; this low-frequency pass is only
  // reconciliation for a lost desktop event or backend restart.
  usePolling(() => refreshThreadRunStatuses(activeThreads), 30_000, {
    enabled: activeThreads.length > 0,
    deps: [activeThreads, refreshThreadRunStatuses],
  });
  useEffect(() => {
    const unlisten = listen<ThreadRuntimeUpdate>("thread-runtime-updated", (event) => {
      // A push is newer than any reconciliation query already in flight.
      // Invalidate that query before reducing the event so its stale snapshot
      // cannot revert this status when it eventually resolves.
      runStatusGenRef.current += 1;
      setThreadRunStatuses(previous => reduceThreadRunStatus(previous, event.payload));
    });
    return () => {
      void unlisten.then(stop => stop());
    };
  }, []);
  useEffect(() => {
    let cancelled = false;
    let sawPush = false;
    let lastRevision = -1;
    const unlisten = listen<ThreadStreamingUpdate>("thread-streaming-updated", (event) => {
      if (event.payload.revision <= lastRevision)
        return;
      lastRevision = event.payload.revision;
      sawPush = true;
      setThreadStreamingStatuses(streamingStatuses(event.payload.threadIds));
    });
    void unlisten.then(async () => {
      // Register first, then read the initial snapshot. If a newer push arrives
      // while the snapshot is in flight, discard the snapshot instead of
      // reverting to stale cross-client status.
      const threadIds = await listStreamingThreadIds();
      if (!cancelled && !sawPush)
        setThreadStreamingStatuses(streamingStatuses(threadIds));
    });
    return () => {
      cancelled = true;
      void unlisten.then(stop => stop());
    };
  }, []);
  useEffect(() => {
    if (activeThreads.length === 0) {
      setThreadRunStatuses({});
      setThreadStreamingStatuses({});
    }
  }, [activeThreads.length]);

  // Pre-fetch agent state for the active thread so model/thinking/title
  // are available from cache without a network delay on first render.
  // Keep refreshing on an interval shorter than the cache TTL (30s): the
  // cached entry must never expire while the thread is being viewed — an
  // expired snapshot dropped the composer back to the global draft
  // model/thinking level mid-view. getAgentState dedupes via the TTL and
  // its in-flight map, so a 10s tick costs at most one fetch per 30s.
  useEffect(() => {
    if (!activeThreadId)
      return;
    prefetchAgentState(activeThreadId);
    const timer = window.setInterval(prefetchAgentState, 10_000, activeThreadId);
    return () => window.clearInterval(timer);
  }, [activeThreadId]);

  return {
    activeThread,
    activeThreadId,
    activeThreads,
    activeWorkspace,
    loadingStore,
    refreshStore,
    setActiveThreadId,
    storeError,
    threadRunStatuses,
    threadStreamingStatuses,
    threads,
    workspaces,
  };
}
