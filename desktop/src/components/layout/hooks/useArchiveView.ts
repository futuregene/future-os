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
  /**
   * The conversations inside each archived workspace the reader has expanded.
   *
   * They are not in the store's pages (the workspace is JOIN-hidden), so the
   * view fetches them per group; the rail merges them into the list a batch
   * selection addresses, which is what lets one be restored or deleted from the
   * same toolbar as everything else.
   */
  workspaceThreads: Record<string, StoredThread[]>;
  loading: boolean;
  hasMoreThreads: boolean;
  /** Read the archived lists once, when the entry is first opened. */
  ensureLoaded: () => void;
  /** Fetch one archived workspace's contents (once per group). */
  ensureWorkspaceThreads: (workspaceId: string) => void;
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
  const [workspaceThreads, setWorkspaceThreads] = useState<Record<string, StoredThread[]>>({});
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
      // Contents of workspaces that are still shelved are re-read for the ones
      // the reader had open; a restored workspace's entry simply disappears
      // with its heading.
      setWorkspaceThreads((current) => {
        const kept: Record<string, StoredThread[]> = {};
        for (const workspace of workspacePage.workspaces) {
          if (current[workspace.id])
            kept[workspace.id] = current[workspace.id]!;
        }
        return kept;
      });
    }
    finally {
      stateRef.current = { loading: false, loaded: true, nextCursor: stateRef.current.nextCursor };
      setLoading(false);
    }
  }, []);

  /**
   * One archived workspace's conversations.
   *
   * Loaded on its group's first expansion and kept with the workspace id, so a
   * collapse/re-expand does not refetch and a batch can address the rows.
   */
  const ensureWorkspaceThreads = useCallback((workspaceId: string) => {
    setWorkspaceThreads((current) => {
      if (current[workspaceId])
        return current;
      void listThreadsPage(`workspace-archived:${workspaceId}`, null, 50)
        .then((page) => {
          setWorkspaceThreads(previous => ({ ...previous, [workspaceId]: page.threads }));
        })
        .catch(() => {
          setWorkspaceThreads(previous => ({ ...previous, [workspaceId]: [] }));
        });
      return current;
    });
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
    workspaceThreads,
    loading,
    hasMoreThreads: nextCursor !== null,
    ensureLoaded,
    ensureWorkspaceThreads,
    loadMoreThreads,
    refresh,
  };
}
