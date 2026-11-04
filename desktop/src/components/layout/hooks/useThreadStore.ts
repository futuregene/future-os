import type { Dispatch, SetStateAction } from "react";
import type { ThreadActivityUpdate, ThreadRuntimeUpdate, ThreadRuntimeUpdateBatch } from "../../../integrations/agent/runtimeEvents";
import type { StoredRun, StoredThread, StoredWorkspace } from "../../../integrations/storage/threadStore";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import i18n from "../../../i18n";
import { listStreamingThreadIds, prefetchAgentState } from "../../../integrations/agent/agentStateCache";
import {
  getRecentOrCreateDefaultThread,
  initializeAppStore,
  listActiveThreads,
  listLatestRunInfos,
  listPinnedThreads,
  listPinnedWorkspaces,
  listThreadsPage,
  listWorkspacesPage,
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

interface ThreadStreamingUpdate {
  revision: number;
  threadIds: string[];
}

/** One conversation list the rail draws, and its own cursor. */
export type ThreadScopeKey = string;

interface ScopeState {
  /** Whether the first page of this scope has been requested. */
  started: boolean;
  /** Roots fetched so far, for the "load more" affordance. */
  loadedRoots: number;
  /** Cursor for the next page; null once the scope is exhausted. */
  nextCursor: string | null;
  loading: boolean;
}

/**
 * How many roots one request asks for (the rail's page size).
 *
 * Ten, not fifty: a page is a screenful plus a little, and "load more" one
 * click away is cheaper than scrolling past rows the reader did not ask for.
 */
const PAGE_ROOTS = 10;
/** Workspace list page size — the same ten, for the same reason. */
const PAGE_WORKSPACES = 10;

export function reduceThreadRunStatus(
  previous: ThreadRunStatuses,
  update: ThreadRuntimeUpdate,
  endedAt = Date.now(),
): ThreadRunStatuses {
  const current = previous[update.threadId];
  if (current?.revision !== undefined && update.revision <= current.revision)
    return previous;
  const terminal = ["completed", "failed", "cancelled"].includes(update.status);
  const status = terminal ? update.status as ThreadRunInfo["status"] : "running";
  // A run's terminal state is final. The abort path emits the terminal push
  // (run-row CAS) while the collector is still draining the stream, so its
  // trailing "finalizing" push arrives AFTER with a higher revision — it must
  // not regress the projection back to "running". Only a new run (different
  // runId) or a terminal status replaces the settled entry.
  if (
    current
    && current.runId === update.runId
    && !terminal
    && ["completed", "failed", "cancelled"].includes(current.status)
  ) {
    return previous;
  }
  // Bail out when the projection's meaning is unchanged. During streaming the
  // revision climbs on every coalesced push (~25/s) while status/runId/endedAt
  // stay constant — returning a fresh object each time re-rendered the whole
  // AppShell tree at push frequency (H2). The stored revision not advancing
  // during skipped pushes is safe: pushes don't duplicate, and the comparison
  // is idempotent, so a genuinely different status still lands. A duplicate
  // terminal push keeps the first observed endedAt (the truthful one).
  if (
    current
    && current.status === status
    && current.runId === update.runId
    && (terminal ? current.endedAt !== null : current.endedAt === null)
  ) {
    return previous;
  }
  return {
    ...previous,
    [update.threadId]: {
      runId: update.runId,
      revision: update.revision,
      status,
      endedAt: terminal ? endedAt : null,
    },
  };
}

function streamingStatuses(threadIds: string[]): ThreadStreamingStatuses {
  return Object.fromEntries(threadIds.map(threadId => [threadId, true]));
}

/**
 * Apply one backend activity push to the store's rows.
 *
 * A row patch, not a reload: the backend advances a running conversation's
 * stamp about once a second, and re-listing the whole store that often would
 * replace the array (and re-read SQLite) while the reader is typing or
 * scrolling in it. The stamp itself is taken from the push — never invented
 * here — and only a newer value for a row this list holds is written; anything
 * else returns the same array so React skips the render.
 */
export function applyThreadActivity(
  previous: StoredThread[],
  threadId: string,
  lastMessageAt: number,
): StoredThread[] {
  const index = previous.findIndex(thread => thread.id === threadId);
  if (index < 0)
    return previous;
  const current = previous[index]!;
  if (current.lastMessageAt !== null && current.lastMessageAt !== undefined && current.lastMessageAt >= lastMessageAt)
    return previous;
  const next = [...previous];
  next[index] = { ...current, lastMessageAt };
  return next;
}

/**
 * Same-set guard for streaming statuses: while any thread streams, the push
 * repeats the unchanged id set at push frequency — writing a fresh object each
 * time re-rendered AppShell (and with it the whole rail) for nothing.
 */
function mergeStreamingStatuses(previous: ThreadStreamingStatuses, threadIds: string[]): ThreadStreamingStatuses {
  if (threadIds.length === Object.keys(previous).length && threadIds.every(threadId => previous[threadId]))
    return previous;
  return streamingStatuses(threadIds);
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
  /** Which scopes have more roots than the rail currently holds. */
  scopeHasMore: (scope: string) => boolean;
  /** Whether the workspace list has another page. */
  workspacesHasMore: boolean;
  /** Fetch the next page of the workspace list. */
  loadMoreWorkspaces: () => Promise<void>;
  /** Whether a scope's first page is still in flight. */
  scopeLoading: (scope: string) => boolean;
  /** Fetch the next page of `scope` (or its first page). */
  loadMore: (scope: string) => Promise<void>;
  /** Make sure `scope` has been requested at least once (lazy workspace groups). */
  ensureScope: (scope: string) => void;
  /**
   * Make a conversation the open one, merging it into the store first.
   *
   * The row may not be in any loaded page — an archived detail, or a task's
   * conversation the tick loop just created — so the store holds it before the
   * shell's `activeThread` lookup can find it.
   */
  selectThread: (thread: StoredThread) => void;
  /**
   * Reload the scopes already in the store, keeping their loaded depth — the
   * mutation path (archive/restore/delete) and the `threads-updated` push.
   */
  refreshStore: (nextActiveThreadId?: string) => Promise<void>;
}

/**
 * Owns the local thread/workspace store.
 *
 * **Paged by scope**, one paged stream per list the rail draws: `chat` (the
 * chat section), `workspace:<id>` (one group), `recent` (the recent view's
 * global stream) and `archived` (the archived view). A page is a set of complete
 * fork trees, so the rail can build its tree from what it has without ever
 * holding half a fork.
 *
 * Bootstrap loads only what the first screen needs: pinned conversations,
 * pinned workspaces, one page of the workspace list, and one page of the chat
 * section. A workspace group's conversations load when the group is first
 * expanded; the archived view loads when the entry is opened.
 *
 * Run status is driven primarily by the `thread-runtime-updated` push event; a
 * low-frequency (30s) reconciliation pass backstops lost events / backend
 * restarts. Cross-client streaming status is also pushed after one initial
 * snapshot.
 */
export function useThreadStore(): ThreadStore {
  const [threads, setThreads] = useState<StoredThread[]>([]);
  const [threadRunStatuses, setThreadRunStatuses] = useState<ThreadRunStatuses>({});
  const [threadStreamingStatuses, setThreadStreamingStatuses] = useState<ThreadStreamingStatuses>({});
  const [workspaces, setWorkspaces] = useState<StoredWorkspace[]>([]);
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);
  const [loadingStore, setLoadingStore] = useState(true);
  const [storeError, setStoreError] = useState<string | null>(null);
  const [scopes, setScopes] = useState<Record<ThreadScopeKey, ScopeState>>({});
  const [workspacesMore, setWorkspacesMore] = useState(false);

  // Mirror the active id into a ref so `refreshStore` can read the latest value
  // without listing it as a dependency — that kept it stable-free, recreating
  // it (and cascading to consumers) on every thread selection (B-14b).
  const activeThreadIdRef = useRef(activeThreadId);
  activeThreadIdRef.current = activeThreadId;
  // The scopes the reader has actually opened, read by refreshStore so a
  // reload costs exactly what is on screen.
  const scopesRef = useRef(scopes);
  scopesRef.current = scopes;
  // The rows currently held, for the active conversation that survives a
  // refresh even when its scope no longer lists it (an archived detail).
  const threadsRef = useRef(threads);
  threadsRef.current = threads;

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

  /** Merge fetched rows into the store, newest copy winning. */
  const mergeThreads = useCallback((incoming: StoredThread[]) => {
    if (incoming.length === 0)
      return;
    setThreads((previous) => {
      const byId = new Map(previous.map(thread => [thread.id, thread] as const));
      for (const thread of incoming) byId.set(thread.id, thread);
      const next = [...byId.values()];
      // A row that arrived with a newer state may change order; the rail sorts
      // its own lists, so the array order only has to be stable.
      return next;
    });
  }, []);

  /**
   * Fetch one page of `scope` and merge it.
   *
   * `reset` re-reads the scope from its first page (mutation path); otherwise
   * the scope's cursor advances. Both keep `nextCursor` truthful, so "load
   * more" appears exactly while the scope has more.
   */
  const fetchPage = useCallback(async (scope: ThreadScopeKey, reset: boolean) => {
    const current = scopesRef.current[scope];
    if (!reset && current && (!current.started || current.nextCursor === null))
      return;
    if (!reset && current?.loading)
      return;
    setScopes(previous => ({
      ...previous,
      [scope]: {
        started: true,
        loadedRoots: reset ? 0 : (previous[scope]?.loadedRoots ?? 0),
        nextCursor: reset ? null : (previous[scope]?.nextCursor ?? null),
        loading: true,
      },
    }));
    try {
      const cursor = reset ? null : (scopesRef.current[scope]?.nextCursor ?? null);
      const page = await listThreadsPage(scope, cursor, PAGE_ROOTS);
      mergeThreads(page.threads);
      setScopes(previous => ({
        ...previous,
        [scope]: {
          started: true,
          loadedRoots: (previous[scope]?.loadedRoots ?? 0) + PAGE_ROOTS,
          nextCursor: page.nextCursor,
          loading: false,
        },
      }));
    }
    catch (error) {
      setScopes(previous => ({
        ...previous,
        [scope]: {
          started: previous[scope]?.started ?? false,
          loadedRoots: previous[scope]?.loadedRoots ?? 0,
          nextCursor: previous[scope]?.nextCursor ?? null,
          loading: false,
        },
      }));
      throw error;
    }
  }, [mergeThreads]);

  const loadMore = useCallback(async (scope: string) => {
    await fetchPage(scope, false);
  }, [fetchPage]);

  const ensureScope = useCallback((scope: string) => {
    const state = scopesRef.current[scope];
    if (state?.started)
      return;
    void fetchPage(scope, false).catch(() => {
      // A failed first page leaves the scope unstarted; the next expansion or
      // the next `threads-updated` retries. The rail shows its own spinner.
    });
  }, [fetchPage]);

  const selectThread = useCallback((thread: StoredThread) => {
    mergeThreads([thread]);
    setActiveThreadId(thread.id);
  }, [mergeThreads]);

  const scopeHasMore = useCallback(
    (scope: string) => {
      const state = scopes[scope];
      return Boolean(state?.started && state.nextCursor !== null);
    },
    [scopes],
  );

  const scopeLoading = useCallback((scope: string) => scopes[scope]?.loading ?? false, [scopes]);

  /** The workspace list's next page (the first one is loaded at bootstrap). */
  const workspaceCursorRef = useRef<string | null>(null);
  const loadMoreWorkspaces = useCallback(async () => {
    const cursor = workspaceCursorRef.current;
    if (cursor === null)
      return;
    const page = await listWorkspacesPage(false, cursor, PAGE_WORKSPACES);
    setWorkspaces(previous => [...previous, ...page.workspaces]);
    workspaceCursorRef.current = page.nextCursor;
    setWorkspacesMore(page.nextCursor !== null);
  }, []);

  // Guard against overlapping refreshes (rapid deletes/creates, each calling
  // refreshStore): a slow page landing after a newer one would revive a stale
  // list — briefly resurrecting a just-deleted thread and possibly
  // re-selecting it. Newest call wins.
  const refreshStoreGenRef = useRef(0);

  /**
   * Re-read the scopes the reader has opened, at the depth they were loaded.
   *
   * Not a full reload: each open scope is re-requested page by page until it
   * reaches the roots it already held (or runs dry), so a restart-ordered list
   * converges without paging a reader back to row one. The active id is
   * re-derived the same way the unpaged store did it.
   */
  const refreshStore = useCallback(async (nextActiveThreadId?: string) => {
    const generation = ++refreshStoreGenRef.current;
    try {
      const [pinned, pinnedWs, firstWorkspaces, active] = await Promise.all([
        listPinnedThreads("recent"),
        listPinnedWorkspaces(),
        listWorkspacesPage(false, null, PAGE_WORKSPACES),
        listActiveThreads(),
      ]);
      if (generation !== refreshStoreGenRef.current)
        return;
      setWorkspaces([...pinnedWs, ...firstWorkspaces.workspaces]);
      workspaceCursorRef.current = firstWorkspaces.nextCursor;
      setWorkspacesMore(firstWorkspaces.nextCursor !== null);

      // Re-read every scope the reader has opened at the depth it held, and
      // rebuild the row set from the answers instead of merging into it: a row
      // that a page no longer returns (archived, deleted, moved) must leave the
      // store, which merging alone can never express.
      const rows: StoredThread[] = [...pinned, ...active];
      const open = Object.entries(scopesRef.current).filter(([, state]) => state.started);
      for (const [scope, state] of open) {
        const wanted = Math.max(state.loadedRoots, PAGE_ROOTS);
        let cursor: string | null = null;
        let roots = 0;
        for (;;) {
          const page = await listThreadsPage(scope, cursor, PAGE_ROOTS);
          if (generation !== refreshStoreGenRef.current)
            return;
          rows.push(...page.threads);
          roots += PAGE_ROOTS;
          cursor = page.nextCursor;
          if (cursor === null || roots >= wanted)
            break;
        }
        setScopes(previous => ({
          ...previous,
          [scope]: { started: true, loadedRoots: roots, nextCursor: cursor, loading: false },
        }));
      }

      const byId = new Map(rows.map(thread => [thread.id, thread] as const));
      // The conversation on screen stays addressable even when its scope no
      // longer returns it (a just-archived detail stays open, read-only).
      const currentActive = byId.get(activeThreadIdRef.current ?? "");
      if (!currentActive && activeThreadIdRef.current) {
        const stillKnown = threadsRef.current.find(thread => thread.id === activeThreadIdRef.current);
        if (stillKnown)
          byId.set(stillKnown.id, stillKnown);
      }
      setThreads([...byId.values()]);

      const selectable = nextActiveThreadId ?? activeThreadIdRef.current;
      if (selectable && byId.has(selectable)) {
        setActiveThreadId(selectable);
      }
    }
    catch (error) {
      if (generation === refreshStoreGenRef.current)
        setStoreError(errorMessage(error));
    }
  }, []);

  // Keep the start-time ordering honest: a conversation that becomes active
  // from another client is inserted by `listActiveThreads`, so the recent
  // view's active stretch is complete even when its page does not hold it yet.
  const refreshActiveThreads = useCallback(async () => {
    try {
      const active = await listActiveThreads();
      mergeThreads(active);
    }
    catch {
      // Transient — the next reconciliation pass retries.
    }
  }, [mergeThreads]);

  // Run-status reconciliation is driven solely by the low-frequency pass
  // below; `usePolling` doesn't cancel in-flight async, so a generation guard
  // keeps a slow snapshot from overwriting a newer one.
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
              // `revision` is a process-global monotonic watermark, independent
              // of which run it was last stamped by — never lower it on a runId
              // change. Otherwise a run rollover (A done, B created) drops the
              // watermark to undefined and a delayed push for A (within the 40ms
              // coalesce window) slips past the reducer's guard, regressing the
              // sidebar to A's status until B's first push or the next pass.
              revision: current?.revision,
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

  useEffect(() => {
    // Hand-rolled cancel guard (not useAsyncResource): a multi-step bootstrap
    // (init store → recent thread → first pages) writing several states, not a
    // single resource. See desktop/CLAUDE.md §4.
    // Stale-run convergence intentionally does NOT happen here: it lives in
    // the backend's setup (once per process). Bootstrap re-runs on every
    // webview reload, where the backend may still own live runs.
    let cancelled = false;

    async function bootstrapStore() {
      setLoadingStore(true);
      try {
        await initializeAppStore();
        const recentThread = await getRecentOrCreateDefaultThread(i18n.t("common:newChat"));
        const [pinned, pinnedWs, firstWorkspaces, firstChat, active] = await Promise.all([
          listPinnedThreads("recent"),
          listPinnedWorkspaces(),
          listWorkspacesPage(false, null, PAGE_WORKSPACES),
          listThreadsPage("chat", null, PAGE_ROOTS),
          listActiveThreads(),
        ]);
        if (cancelled) {
          return;
        }
        setWorkspaces([...pinnedWs, ...firstWorkspaces.workspaces]);
        workspaceCursorRef.current = firstWorkspaces.nextCursor;
        setWorkspacesMore(firstWorkspaces.nextCursor !== null);
        setScopes({
          chat: {
            started: true,
            loadedRoots: PAGE_ROOTS,
            nextCursor: firstChat.nextCursor,
            loading: false,
          },
        });
        const byId = new Map<string, StoredThread>();
        for (const thread of [...pinned, ...firstChat.threads, ...active]) {
          byId.set(thread.id, thread);
        }
        // A restored session may live under a workspace or a chat page the
        // reader will not open first; make sure the one we select is present.
        if (!byId.has(recentThread.id)) {
          byId.set(recentThread.id, recentThread);
        }
        setThreads([...byId.values()]);
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
  // The recent view's active stretch must be whole even when its page does not
  // hold a conversation another client just started.
  usePolling(() => refreshActiveThreads(), 30_000, { deps: [refreshActiveThreads] });
  useEffect(() => {
    const unlisten = listen<ThreadRuntimeUpdateBatch>("thread-runtime-updated", (event) => {
      // A push is newer than any reconciliation query already in flight.
      // Invalidate that query before reducing the event so its stale snapshot
      // cannot revert this status when it eventually resolves.
      runStatusGenRef.current += 1;
      setThreadRunStatuses(previous => event.payload.updates.reduce(
        (statuses, update) => reduceThreadRunStatus(statuses, update),
        previous,
      ));
    });
    return () => {
      void unlisten.then(stop => stop());
    };
  }, []);
  // A conversation's activity stamp moved. Externally-driven runs (the CLI, a
  // scheduler, another machine) arrive this way and nowhere else: their row's
  // age label and its place in the list are read from `lastMessageAt`, which
  // used to stay at the value from before the run for the whole reply. Patch
  // the one row — a full `refreshStore` at push frequency would reload the
  // list out from under the reader (see `applyThreadActivity`).
  useEffect(() => {
    const unlisten = listen<ThreadActivityUpdate>("thread-activity-updated", (event) => {
      setThreads(previous => applyThreadActivity(
        previous,
        event.payload.threadId,
        event.payload.lastMessageAt,
      ));
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
      setThreadStreamingStatuses(previous => mergeStreamingStatuses(previous, event.payload.threadIds));
    });
    void unlisten.then(async () => {
      // Register first, then read the initial snapshot. If a newer push arrives
      // while the snapshot is in flight, discard the snapshot instead of
      // reverting to stale cross-client status.
      const threadIds = await listStreamingThreadIds();
      if (!cancelled && !sawPush)
        setThreadStreamingStatuses(previous => mergeStreamingStatuses(previous, threadIds));
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

  // Pre-fetch agent state when a thread becomes active: seeds the cache so
  // model/thinking/title render without a network delay, and revalidates the
  // entry (TTL-throttled inside getAgentState) on every switch back. Between
  // activations the push events (model_changed / thinking_level_changed /
  // session_name_changed / config_reloaded) keep the entry current — no
  // timer needed. Synchronous reads are stale-while-revalidate, so a late
  // revalidation can never flash the composer back to the draft model.
  useEffect(() => {
    // A fresh thread has no Agent session until its first prompt. Its cached
    // model/thinking values are the user's explicit draft selections; fetching
    // the honest "no session" payload here would replace them with nulls and
    // make the composer fall back to the model default before sending.
    if (!activeThread?.agentSessionId)
      return;
    prefetchAgentState(activeThread.id);
  }, [activeThread?.id, activeThread?.agentSessionId]);

  return {
    activeThread,
    activeThreadId,
    activeThreads,
    activeWorkspace,
    ensureScope,
    loadMore,
    loadMoreWorkspaces,
    loadingStore,
    refreshStore,
    scopeHasMore,
    scopeLoading,
    selectThread,
    setActiveThreadId,
    storeError,
    threadRunStatuses,
    threadStreamingStatuses,
    threads,
    workspaces,
    workspacesHasMore: workspacesMore,
  };
}
