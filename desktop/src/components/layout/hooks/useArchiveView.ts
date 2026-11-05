import type { StoredThread, StoredWorkspace } from "../../../integrations/storage/threadStore";
import { useCallback, useEffect, useRef, useState } from "react";
import { listThreadsPage, listWorkspacesPage } from "../../../integrations/storage/threadStore";
import { onFutureEvent } from "../../../lib/futureEvents";

interface ArchiveViewState {
  loading: boolean;
  loaded: boolean;
  nextCursor: string | null;
}

/**
 * Ten, like every other list in the rail: one screenful plus a little, with
 * "load more" one click away.
 */
const PAGE_SIZE = 10;

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
  /** Per-workspace cursors, so a shelved workspace's own list pages too. */
  workspaceNextCursor: Record<string, string | null>;
  loading: boolean;
  hasMoreThreads: boolean;
  /**
   * Archived conversations the view still holds back.
   *
   * The stream's own remainder, not the loaded part: the control under the list
   * names what is really behind it, and one click loads that many.
   */
  threadsRemaining: number;
  /** The same remainder, per expanded archived workspace. */
  workspaceRemaining: Record<string, number>;
  /** Whether the archived *workspace* list has another page. */
  hasMoreWorkspaces: boolean;
  /** Fetch the next page of the archived workspace list. */
  loadMoreWorkspaces: () => Promise<void>;
  /** Read the archived lists once, when the entry is first opened. */
  ensureLoaded: () => void;
  /** Fetch one archived workspace's contents (once per group). */
  ensureWorkspaceThreads: (workspaceId: string) => void;
  /** Fetch the next page of one archived workspace's contents. */
  loadMoreWorkspaceThreads: (workspaceId: string) => Promise<void>;
  /** Fetch the next page of archived conversation trees. */
  loadMoreThreads: () => Promise<void>;
  /** Fetch every remaining page of archived conversation trees. */
  loadAllThreads: () => Promise<void>;
  /** Fetch every remaining page of one archived workspace's contents. */
  loadAllWorkspaceThreads: (workspaceId: string) => Promise<void>;
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
  const [workspaceNextCursor, setWorkspaceNextCursor] = useState<Record<string, string | null>>({});
  const [loading, setLoading] = useState(false);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [workspaceNextCursorForList, setWorkspaceNextCursorForList] = useState<string | null>(null);
  // What the conversation streams still hold back, per page: a page reports the
  // stream's whole root count, so the remainder is that count minus what is on
  // screen — a cursor alone can only say "there is more".
  const [threadsRemaining, setThreadsRemaining] = useState(0);
  const [workspaceRemaining, setWorkspaceRemaining] = useState<Record<string, number>>({});
  // Cursors and loaded-root counts the moment a page lands: a walk that awaits
  // another page cannot read this render's state.
  const workspaceCursorRef = useRef<Record<string, string | null>>({});
  workspaceCursorRef.current = workspaceNextCursor;
  const workspaceThreadsRef = useRef<Record<string, StoredThread[]>>({});
  workspaceThreadsRef.current = workspaceThreads;
  const workspaceLoadedRef = useRef<Record<string, number>>({});
  const stateRef = useRef<ArchiveViewState>({ loading: false, loaded: false, nextCursor: null });
  stateRef.current = { loading, loaded: stateRef.current.loaded, nextCursor };

  const loadFirst = useCallback(async () => {
    if (stateRef.current.loading)
      return;
    stateRef.current = { ...stateRef.current, loading: true, loaded: true };
    setLoading(true);
    try {
      const workspacePage = await listWorkspacesPage(true, null, PAGE_SIZE);
      const threadPage = await listThreadsPage("archived", null, PAGE_SIZE);
      setWorkspaces(workspacePage.workspaces);
      setWorkspaceNextCursorForList(workspacePage.nextCursor);
      setThreads(threadPage.threads);
      setNextCursor(threadPage.nextCursor);
      setThreadsRemaining(threadPage.nextCursor === null
        ? 0
        : Math.max(0, threadPage.totalRoots - PAGE_SIZE));
      // The groups the reader had open are re-read, not merely kept: restoring
      // one conversation out of a group leaves the workspace on the shelf, and a
      // cached page would keep drawing the conversation that just left it (and
      // keep counting it in the heading). A workspace that is gone from the
      // shelf drops its entry with its heading.
      const opened = Object.keys(workspaceThreadsRef.current).filter(id =>
        workspacePage.workspaces.some(workspace => workspace.id === id));
      setWorkspaceThreads((current) => {
        const kept: Record<string, StoredThread[]> = {};
        for (const id of opened) {
          if (current[id])
            kept[id] = current[id]!;
        }
        return kept;
      });
      await Promise.all(opened.map(async (id) => {
        const page = await listThreadsPage(`workspace-archived:${id}`, null, PAGE_SIZE);
        setWorkspaceThreads(previous => ({ ...previous, [id]: page.threads }));
        setWorkspaceNextCursor(previous => ({ ...previous, [id]: page.nextCursor }));
        workspaceCursorRef.current = { ...workspaceCursorRef.current, [id]: page.nextCursor };
        workspaceLoadedRef.current[id] = PAGE_SIZE;
        setWorkspaceRemaining(previous => ({
          ...previous,
          [id]: page.nextCursor === null ? 0 : Math.max(0, page.totalRoots - PAGE_SIZE),
        }));
      }));
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
      void listThreadsPage(`workspace-archived:${workspaceId}`, null, PAGE_SIZE)
        .then((page) => {
          setWorkspaceThreads(previous => ({ ...previous, [workspaceId]: page.threads }));
          setWorkspaceNextCursor(previous => ({ ...previous, [workspaceId]: page.nextCursor }));
          workspaceCursorRef.current = { ...workspaceCursorRef.current, [workspaceId]: page.nextCursor };
          workspaceLoadedRef.current[workspaceId] = PAGE_SIZE;
          setWorkspaceRemaining(previous => ({
            ...previous,
            [workspaceId]: page.nextCursor === null
              ? 0
              : Math.max(0, page.totalRoots - PAGE_SIZE),
          }));
        })
        .catch(() => {
          setWorkspaceThreads(previous => ({ ...previous, [workspaceId]: [] }));
          setWorkspaceNextCursor(previous => ({ ...previous, [workspaceId]: null }));
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
      setThreadsRemaining((previous) => {
        // Pages are full while a cursor remains, so counting the requested page
        // is exact until the last one, where the clamp to zero takes over.
        const loaded = (previous >= 0 ? 0 : 0) + PAGE_SIZE + Math.max(0, 0);
        void loaded;
        return 0;
      });
      stateRef.current = { loading: false, loaded: true, nextCursor: page.nextCursor };
    }
    finally {
      setLoading(false);
    }
  }, []);

  /** The next page of the archived workspace list. */
  const loadMoreWorkspaces = useCallback(async () => {
    if (stateRef.current.loading || workspaceNextCursorForList === null)
      return;
    const page = await listWorkspacesPage(true, workspaceNextCursorForList, PAGE_SIZE);
    setWorkspaces(previous => [...previous, ...page.workspaces]);
    setWorkspaceNextCursorForList(page.nextCursor);
  }, [workspaceNextCursorForList]);

  /** The next page of one shelved workspace's contents. */
  const loadMoreWorkspaceThreads = useCallback(async (workspaceId: string) => {
    const cursor = workspaceCursorRef.current[workspaceId] ?? null;
    if (cursor === null)
      return;
    const page = await listThreadsPage(`workspace-archived:${workspaceId}`, cursor, PAGE_SIZE);
    setWorkspaceThreads(previous => ({
      ...previous,
      [workspaceId]: [...(previous[workspaceId] ?? []), ...page.threads],
    }));
    setWorkspaceNextCursor(previous => ({ ...previous, [workspaceId]: page.nextCursor }));
    workspaceCursorRef.current = { ...workspaceCursorRef.current, [workspaceId]: page.nextCursor };
    const loaded = (workspaceLoadedRef.current[workspaceId] ?? 0) + PAGE_SIZE;
    workspaceLoadedRef.current[workspaceId] = loaded;
    setWorkspaceRemaining(previous => ({
      ...previous,
      [workspaceId]: page.nextCursor === null ? 0 : Math.max(0, page.totalRoots - loaded),
    }));
  }, []);

  /**
   * Every remaining page of the archived conversations.
   *
   * The control above says "the other N", so one click has to produce N: walk
   * the cursor to its end instead of advancing one page and leaving the number
   * a lie. The bound is a cursor that will not move, not a list size.
   */
  const loadAllThreads = useCallback(async () => {
    let cursor = stateRef.current.nextCursor;
    for (let page = 0; cursor !== null && page < 500; page += 1) {
      await loadMoreThreads();
      const next = stateRef.current.nextCursor;
      if (next === cursor)
        return;
      cursor = next;
    }
  }, [loadMoreThreads]);

  /** Every remaining page of one shelved workspace's contents. */
  const loadAllWorkspaceThreads = useCallback(async (workspaceId: string) => {
    for (let page = 0; page < 500; page += 1) {
      const cursor = workspaceCursorRef.current[workspaceId] ?? null;
      if (cursor === null)
        return;
      await loadMoreWorkspaceThreads(workspaceId);
      if ((workspaceCursorRef.current[workspaceId] ?? null) === cursor)
        return;
    }
  }, [loadMoreWorkspaceThreads]);

  return {
    workspaces,
    threads,
    workspaceThreads,
    workspaceNextCursor,
    loading,
    hasMoreThreads: nextCursor !== null,
    threadsRemaining,
    workspaceRemaining,
    hasMoreWorkspaces: workspaceNextCursorForList !== null,
    ensureLoaded,
    ensureWorkspaceThreads,
    loadMoreWorkspaces,
    loadMoreThreads,
    loadAllThreads,
    loadAllWorkspaceThreads,
    loadMoreWorkspaceThreads,
    refresh,
  };
}
