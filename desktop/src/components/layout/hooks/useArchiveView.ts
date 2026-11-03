import type { StoredThread, StoredWorkspace } from "../../../integrations/storage/threadStore";
import { useCallback, useEffect, useRef, useState } from "react";
import { listThreadsPage, listWorkspacesPage } from "../../../integrations/storage/threadStore";
import { onFutureEvent } from "../../../lib/futureEvents";

interface ArchiveViewState {
  loading: boolean;
  loaded: boolean;
  nextCursor: string | null;
}

const PAGE_SIZE = 50;

export interface ArchiveView {
  workspaces: StoredWorkspace[];
  threads: StoredThread[];
  loading: boolean;
  hasMoreThreads: boolean;
  /** Read the archived lists once, when the entry is first opened. */
  ensureLoaded: () => void;
  /** Fetch the next page of archived conversation trees. */
  loadMoreThreads: () => Promise<void>;
  /** Re-read the archived lists (after an archive/restore/delete). */
  refresh: () => Promise<void>;
}

/**
 * The activity rail's archived view: shelved workspaces and shelved
 * conversations, loaded on demand.
 *
 * Its own stream rather than a filter over the active lists — with paging the
 * active lists hold only what the tree has expanded, so the archived rows are
 * simply not there to filter. Two pages: the archived workspaces (whose contents
 * are browsable through their own scope) and the archived conversation trees.
 */
export function useArchiveView(): ArchiveView {
  const [workspaces, setWorkspaces] = useState<StoredWorkspace[]>([]);
  const [threads, setThreads] = useState<StoredThread[]>([]);
  const [loading, setLoading] = useState(false);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const stateRef = useRef<ArchiveViewState>({ loading: false, loaded: false, nextCursor: null });
  stateRef.current = { loading, loaded: stateRef.current.loaded, nextCursor };

  const loadFirst = useCallback(async () => {
    if (stateRef.current.loading)
      return;
    stateRef.current = { ...stateRef.current, loading: true, loaded: true };
    setLoading(true);
    try {
      const [workspacePage, threadPage] = await Promise.all([
        listWorkspacesPage(true, null, PAGE_SIZE),
        listThreadsPage("archived", null, PAGE_SIZE),
      ]);
      setWorkspaces(workspacePage.workspaces);
      setThreads(threadPage.threads);
      setNextCursor(threadPage.nextCursor);
    }
    finally {
      stateRef.current = { loading: false, loaded: true, nextCursor: stateRef.current.nextCursor };
      setLoading(false);
    }
  }, []);

  const ensureLoaded = useCallback(() => {
    if (!stateRef.current.loaded && !stateRef.current.loading)
      void loadFirst();
  }, [loadFirst]);

  const refresh = useCallback(async () => {
    if (!stateRef.current.loaded)
      return;
    await loadFirst();
  }, [loadFirst]);

  // Any archive/restore/delete elsewhere in the shell invalidates the lists;
  // a stream that is not loaded yet needs nothing (its first page reads then).
  useEffect(() => onFutureEvent("archive-changed", () => {
    void refresh();
  }), [refresh]);

  const loadMoreThreads = useCallback(async () => {
    if (stateRef.current.loading || stateRef.current.nextCursor === null)
      return;
    const cursor = stateRef.current.nextCursor;
    stateRef.current = { ...stateRef.current, loading: true };
    setLoading(true);
    try {
      const page = await listThreadsPage("archived", cursor, PAGE_SIZE);
      setThreads((previous) => {
        const byId = new Map(previous.map(thread => [thread.id, thread] as const));
        for (const thread of page.threads) byId.set(thread.id, thread);
        return [...byId.values()];
      });
      setNextCursor(page.nextCursor);
      stateRef.current = { loading: false, loaded: true, nextCursor: page.nextCursor };
    }
    finally {
      setLoading(false);
    }
  }, []);

  return {
    workspaces,
    threads,
    loading,
    hasMoreThreads: nextCursor !== null,
    ensureLoaded,
    loadMoreThreads,
    refresh,
  };
}
