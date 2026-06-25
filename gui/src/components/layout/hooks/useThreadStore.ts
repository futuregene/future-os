import type { Dispatch, SetStateAction } from "react";
import type { StoredRun, StoredThread, StoredWorkspace } from "../../../integrations/storage/threadStore";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  cancelStaleApprovalRequests,
  getOrCreateDefaultChatThread,
  initializeAppStore,
  listRuns,
  listThreads,
  listWorkspaces,
} from "../../../integrations/storage/threadStore";
import { usePolling } from "../../../lib/usePolling";

type ThreadRunStatuses = Record<string, StoredRun["status"] | undefined>;

export interface ThreadStore {
  threads: StoredThread[];
  workspaces: StoredWorkspace[];
  activeThread: StoredThread | null;
  activeWorkspace: StoredWorkspace | null;
  activeThreads: StoredThread[];
  activeThreadId: string | null;
  setActiveThreadId: Dispatch<SetStateAction<string | null>>;
  threadRunStatuses: ThreadRunStatuses;
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
 * derived active thread/workspace, and a 1.5s poll of each active thread's
 * latest run status.
 */
export function useThreadStore(): ThreadStore {
  const [threads, setThreads] = useState<StoredThread[]>([]);
  const [threadRunStatuses, setThreadRunStatuses] = useState<ThreadRunStatuses>({});
  const [workspaces, setWorkspaces] = useState<StoredWorkspace[]>([]);
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);
  const [loadingStore, setLoadingStore] = useState(true);
  const [storeError, setStoreError] = useState<string | null>(null);

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
    const entries = await Promise.all(
      nextThreads.map(async (thread) => {
        const runs = await listRuns(thread.id);
        return [thread.id, runs[0]?.status] as const;
      }),
    );
    if (generation !== runStatusGenRef.current) {
      return;
    }
    setThreadRunStatuses(Object.fromEntries(entries));
  }, []);

  const refreshStore = useCallback(async (nextActiveThreadId?: string) => {
    const [nextThreads, nextWorkspaces] = await Promise.all([listThreads(), listWorkspaces()]);
    const selectableThreads = nextThreads.filter(thread => thread.status === "active");
    setThreads(nextThreads);
    setWorkspaces(nextWorkspaces);
    void refreshThreadRunStatuses(selectableThreads);
    if (nextActiveThreadId && selectableThreads.some(thread => thread.id === nextActiveThreadId)) {
      setActiveThreadId(nextActiveThreadId);
    }
    else if (activeThreadId && selectableThreads.some(thread => thread.id === activeThreadId)) {
      setActiveThreadId(activeThreadId);
    }
    else {
      setActiveThreadId(selectableThreads[0]?.id ?? null);
    }
  }, [activeThreadId, refreshThreadRunStatuses]);

  useEffect(() => {
    let cancelled = false;

    async function bootstrapStore() {
      setLoadingStore(true);
      try {
        await initializeAppStore();
        await cancelStaleApprovalRequests();
        const recentThread = await getOrCreateDefaultChatThread();
        const [nextThreads, nextWorkspaces] = await Promise.all([listThreads(), listWorkspaces()]);
        if (cancelled) {
          return;
        }
        setThreads(nextThreads);
        setWorkspaces(nextWorkspaces);
        void refreshThreadRunStatuses(nextThreads.filter(thread => thread.status === "active"));
        setActiveThreadId(recentThread.id);
        setStoreError(null);
      }
      catch (error) {
        if (!cancelled) {
          setStoreError(error instanceof Error ? error.message : String(error));
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
  }, [refreshThreadRunStatuses]);

  usePolling(() => refreshThreadRunStatuses(activeThreads), 1500, {
    enabled: activeThreads.length > 0,
    deps: [activeThreads, refreshThreadRunStatuses],
  });
  useEffect(() => {
    if (activeThreads.length === 0) {
      setThreadRunStatuses({});
    }
  }, [activeThreads.length]);

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
    threads,
    workspaces,
  };
}
