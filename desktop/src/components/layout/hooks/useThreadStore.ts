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
import { RECENT_PAGE_SIZE } from "../recentOrder";

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
  /**
   * How many roots the whole scope holds, as its last page reported.
   *
   * A cursor says "there is more", not "how much"; the count behind "show the
   * other N" comes from here, and it counts the scope, not the loaded part.
   */
  totalRoots: number;
  loading: boolean;
}

/**
 * How many roots one request asks for (the rail's page size).
 *
 * Ten, not fifty: a page is a screenful plus a little, and "load more" one
 * click away is cheaper than scrolling past rows the reader did not ask for.
 */
const PAGE_ROOTS = 10;
/** Workspace list page size — one request asks for the host's whole clamp. */
const PAGE_WORKSPACES = 100;

/**
 * The recent view's own stream, spelled the way `list_threads_page` takes it.
 *
 * Its rows are the view's, not the tree's: the tree pages per workspace, the
 * recent view one conversation stream across every workspace (see
 * `docs/design/workspace-thread-archive-and-paging.md` §4.4).
 */
const RECENT_SCOPE = "recent";

/**
 * How many roots one page of `scope` asks for.
 *
 * The recent view pages by the size its own control promises ("show 30 more"),
 * so a page is exactly what a click draws; a workspace group and the chat
 * section are one screenful plus a little, and `PAGE_ROOTS` is what the rail
 * shows before its own "load more".
 */
/**
 * Every workspace row of the *active* list, not one page of it.
 *
 * The rail draws the workspaces the reader owns, all of them: paging that list
 * ten at a time hid most of it behind a "load more" under the last group, which
 * is the same complaint the conversations had. The list holds user workspaces
 * only (temporary ones never appear in it), so reading it whole is one or two
 * requests; the cursor walk is there so a bigger list is still complete.
 */
async function readAllWorkspaces(): Promise<StoredWorkspace[]> {
  const workspaces: StoredWorkspace[] = [];
  let cursor: string | null = null;
  for (let page = 0; page < 100; page += 1) {
    const answered = await listWorkspacesPage(false, cursor, PAGE_WORKSPACES);
    workspaces.push(...answered.workspaces);
    cursor = answered.nextCursor;
    if (cursor === null)
      break;
  }
  return workspaces;
}

function pageSizeFor(scope: ThreadScopeKey): number {
  return scope === RECENT_SCOPE ? RECENT_PAGE_SIZE : PAGE_ROOTS;
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
 */function mergeStreamingStatuses(previous: ThreadStreamingStatuses, threadIds: string[]): ThreadStreamingStatuses {
  if (threadIds.length === Object.keys(previous).length && threadIds.every(threadId => previous[threadId]))
    return previous;
  return streamingStatuses(threadIds);
}

/**
 * What a refresh knows beyond "re-read the lists".
 *
 * A refreshed page that no longer returns a row is ambiguous on its own: the
 * conversation was archived, moved to another workspace, or simply paged out.
 * Only the caller that made the change can say which it was, and the store holds
 * the open conversation through exactly that ambiguity (an archived detail stays
 * open, read-only) — so a mutation that ends the conversation's place on screen
 * has to say so.
 */
export interface RefreshStoreOptions {
  /** Conversations that no longer exist — deleted, and never to be revived. */
  removedThreadIds?: readonly string[];
  /**
   * Conversations the caller just archived — off the active list, still stored.
   *
   * They leave the open column when one of them is on it: the row is no longer
   * in the list the reader is reading, and holding it there (the read-only
   * archived-detail case) would draw a shelved conversation as the open one
   * while its sidebar row is gone. The store still keeps the row when a scope
   * returns it, with the archived status that keeps it out of the active list.
   */
  archivedThreadIds?: readonly string[];
  /**
   * A workspace the caller just archived — its conversations leave with it.
   *
   * Archiving a workspace writes nothing on its members: every one of them keeps
   * its own status, and it is the container that goes off the lists, so no
   * listing can confirm that a row inside it left. Naming the workspace is what
   * says so, and every conversation filed under it is then treated as off the
   * list (the open one included) without being marked shelved — restoring the
   * workspace re-lists them exactly as they were.
   */
  archivedWorkspaceIds?: readonly string[];
  /** A workspace the caller just deleted — its conversations are gone with it. */
  removedWorkspaceIds?: readonly string[];
}

/** When a conversation last did anything — the stamp its place in a list reads. */
function activityStamp(thread: StoredThread): number {
  return thread.lastMessageAt ?? thread.createdAt;
}

/**
 * The conversation the column falls back to once the open one is gone.
 *
 * Its own workspace first: a reader who just deleted (or shelved) a conversation
 * is still working where that conversation lived, so the next one should come
 * from the same group rather than from across the rail. Then the most recently
 * active row the store holds, because "somewhere else" still beats a blank
 * column. A shelved row is never a candidate — it is not in the list the reader
 * is looking at, and swapping one in for a deleted conversation would read as a
 * jump, not a fallback. `null` means the store has nothing left to open, which
 * the shell turns into its new-conversation screen.
 */
function replacementAfterRemoval(
  byId: ReadonlyMap<string, StoredThread>,
  workspaceId: string | undefined,
): string | null {
  let inWorkspace: StoredThread | null = null;
  let anywhere: StoredThread | null = null;
  for (const row of byId.values()) {
    if (row.status !== "active")
      continue;
    if (!anywhere || activityStamp(row) > activityStamp(anywhere))
      anywhere = row;
    if (workspaceId && row.workspaceId === workspaceId && (!inWorkspace || activityStamp(row) > activityStamp(inWorkspace)))
      inWorkspace = row;
  }
  return (inWorkspace ?? anywhere)?.id ?? null;
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
   * The conversations the recent view's own paged stream holds.
   *
   * The view draws these, not every row any scope has loaded: its page is what
   * the reader asked for when they opened it, and it advances on its own
   * "show more" (`loadMore("recent")`) rather than inheriting whatever a
   * workspace group happened to fetch. `undefined` until that stream has been
   * started at all (the view asks for it when it is first shown), so the list is
   * never emptied by a stream nobody requested.
   */
  recentThreadIds: ReadonlySet<string> | undefined;
  /** Which scopes have more roots than the rail currently holds. */
  scopeHasMore: (scope: string) => boolean;
  /** How many roots `scope` still holds back; 0 when it has none or is unknown. */
  scopeRemaining: (scope: string) => number;
  /** Whether a scope's first page is still in flight. */
  scopeLoading: (scope: string) => boolean;
  /**
   * Fetch every remaining page of `scope`.
   *
   * The other half of `scopeRemaining`, and the only way a list pages now: every
   * control that offers more names its remainder, so it must load that many —
   * not the next page of 30.
   */
  loadRemaining: (scope: string) => Promise<void>;
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
   *
   * `nextActiveThreadId` is the conversation to keep open: absent keeps the one
   * on screen, `null` leaves nothing open. A selection that `options` reports as
   * removed is replaced by the nearest remaining conversation instead of being
   * kept, so deleting the open one cannot leave the column on a row the sidebar
   * has already dropped.
   */
  refreshStore: (nextActiveThreadId?: string | null, options?: RefreshStoreOptions) => Promise<void>;
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
 * expanded; the archived view loads when the entry is opened; the recent view
 * loads its own `recent` stream (30 conversations a page) when that view is
 * opened, so it ranks the whole store page by page instead of only what other
 * scopes happen to hold.
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
  // The recent view's own rows, by id: rows are merged into `threads` (one
  // array, so a row never has two copies), but which of them the recent view
  // draws is a fact about that stream, not about the store's contents.
  const [recentIds, setRecentIds] = useState<ReadonlySet<string>>(() => new Set<string>());

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
   * Request one page of `scope` from `cursor` (null = the first page) and merge
   * it.
   *
   * Returns the page so a caller walking a cursor keeps its own copy of it: the
   * scope state is read on the next render, and a loop that awaited a render to
   * learn "the next cursor" could re-ask for a page it already has.
   */
  const requestPage = useCallback(async (scope: ThreadScopeKey, cursor: string | null) => {
    const size = pageSizeFor(scope);
    setScopes(previous => ({
      ...previous,
      [scope]: {
        started: true,
        loadedRoots: previous[scope]?.loadedRoots ?? 0,
        nextCursor: previous[scope]?.nextCursor ?? null,
        totalRoots: previous[scope]?.totalRoots ?? 0,
        loading: true,
      },
    }));
    try {
      const page = await listThreadsPage(scope, cursor, size);
      mergeThreads(page.threads);
      if (scope === RECENT_SCOPE) {
        setRecentIds(previous => new Set([
          ...previous,
          ...page.threads.map(thread => thread.id),
        ]));
      }
      setScopes(previous => ({
        ...previous,
        [scope]: {
          started: true,
          loadedRoots: (previous[scope]?.loadedRoots ?? 0) + size,
          nextCursor: page.nextCursor,
          totalRoots: page.totalRoots,
          loading: false,
        },
      }));
      return page;
    }
    catch (error) {
      // The cursor is not advanced, so the next attempt asks for the page that
      // failed rather than skipping it.
      setScopes(previous => ({
        ...previous,
        [scope]: {
          started: previous[scope]?.started ?? true,
          loadedRoots: previous[scope]?.loadedRoots ?? 0,
          nextCursor: cursor,
          totalRoots: previous[scope]?.totalRoots ?? 0,
          loading: false,
        },
      }));
      throw error;
    }
  }, [mergeThreads]);

  /**
   * Every remaining page of `scope`, for a control that names a count.
   *
   * "Show the other 476" must produce 476 conversations, so this walks the
   * cursor to its end instead of advancing one page and leaving the label a lie.
   * The bound is a cursor that refuses to move, not an expected list size.
   */
  const loadRemaining = useCallback(async (scope: string) => {
    const state = scopesRef.current[scope];
    if (!state?.started || state.nextCursor === null || state.loading)
      return;
    let cursor: string | null = state.nextCursor;
    for (let page = 0; cursor !== null && page < 500; page += 1) {
      const answered = await requestPage(scope, cursor);
      if (answered.nextCursor === cursor)
        return;
      cursor = answered.nextCursor;
    }
  }, [requestPage]);

  /**
   * Whether bootstrap has finished, so a page can be read and kept.
   *
   * A view can ask for its scope at mount — a window restored on the recent
   * view does — which is before `initialize_app_store` has opened the database
   * that page is read from and before bootstrap writes its own first pages.
   * Asking then either fails against a schema that does not exist yet or lands
   * under that write, so the request waits here instead of being dropped: the
   * view asks once per mount, and losing it left the list on the chat page's
   * rows until the reader switched views and asked again.
   */
  const scopesReadyRef = useRef(false);
  const waitingScopesRef = useRef<string[]>([]);

  const ensureScope = useCallback((scope: string) => {
    if (!scopesReadyRef.current) {
      if (!waitingScopesRef.current.includes(scope))
        waitingScopesRef.current.push(scope);
      return;
    }
    const state = scopesRef.current[scope];
    if (state?.started)
      return;
    void requestPage(scope, null).catch(() => {
      // A failed first page leaves the scope unstarted; the next expansion or
      // the next `threads-updated` retries. The rail shows its own spinner.
    });
  }, [requestPage]);

  // Read by bootstrap's tail, which runs once and must not re-run when a
  // consumer re-creates these callbacks.
  const ensureScopeRef = useRef(ensureScope);
  ensureScopeRef.current = ensureScope;

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

  /**
   * How many roots `scope` still holds back.
   *
   * Read from the scope's own count, never from "rows I happen to hold": a
   * control offering to show the rest must name the list's remainder, and the
   * loaded part is exactly the wrong number to quote. Clamped at 0 because
   * `loadedRoots` counts requested pages.
   */
  const scopeRemaining = useCallback(
    (scope: string) => {
      const state = scopes[scope];
      if (!state?.started)
        return 0;
      return Math.max(0, state.totalRoots - state.loadedRoots);
    },
    [scopes],
  );

  const scopeLoading = useCallback((scope: string) => scopes[scope]?.loading ?? false, [scopes]);

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
   * re-derived the same way the unpaged store did it — except that a row the
   * caller reports as removed or archived is never kept, so the delete and
   * archive paths end with the row gone and the selection moved on (see
   * `RefreshStoreOptions`).
   */
  const refreshStore = useCallback(async (nextActiveThreadId?: string | null, options?: RefreshStoreOptions) => {
    const generation = ++refreshStoreGenRef.current;
    const removed = new Set(options?.removedThreadIds ?? []);
    const archived = new Set(options?.archivedThreadIds ?? []);
    // A workspace that left takes its conversations with it. Archived and
    // deleted read the same here: either way the container is off every list,
    // and no member row carries a status saying so.
    const leftWorkspaces = new Set([
      ...(options?.archivedWorkspaceIds ?? []),
      ...(options?.removedWorkspaceIds ?? []),
    ]);
    try {
      const [pinned, pinnedWs, allWorkspaces, active] = await Promise.all([
        listPinnedThreads("recent"),
        listPinnedWorkspaces(),
        readAllWorkspaces(),
        listActiveThreads(),
      ]);
      if (generation !== refreshStoreGenRef.current)
        return;
      setWorkspaces([...pinnedWs, ...allWorkspaces]);

      // Re-read every scope the reader has opened at the depth it held, and
      // rebuild the row set from the answers instead of merging into it: a row
      // that a page no longer returns (archived, deleted, moved) must leave the
      // store, which merging alone can never express.
      const rows: StoredThread[] = [...pinned, ...active];
      const open = Object.entries(scopesRef.current).filter(([, state]) => state.started);
      let recentRows: Set<string> | null = null;
      for (const [scope, state] of open) {
        const size = pageSizeFor(scope);
        const wanted = Math.max(state.loadedRoots, size);
        const streamed = new Set<string>();
        let cursor: string | null = null;
        let roots = 0;
        // The scope's own count comes back with every page; the last one read is
        // the scope's size, not the page's.
        let totalRoots = state.totalRoots;
        for (;;) {
          const page = await listThreadsPage(scope, cursor, size);
          if (generation !== refreshStoreGenRef.current)
            return;
          rows.push(...page.threads);
          for (const thread of page.threads) streamed.add(thread.id);
          roots += size;
          totalRoots = page.totalRoots;
          cursor = page.nextCursor;
          if (cursor === null || roots >= wanted)
            break;
        }
        if (scope === RECENT_SCOPE)
          recentRows = streamed;
        setScopes(previous => ({
          ...previous,
          [scope]: {
            started: true,
            loadedRoots: roots,
            nextCursor: cursor,
            totalRoots,
            loading: false,
          },
        }));
      }
      // The recent stream is rebuilt, not merged: a conversation it no longer
      // returns (archived or deleted elsewhere) must leave the view with it.
      if (recentRows)
        setRecentIds(recentRows);

      const byId = new Map(rows.map(thread => [thread.id, thread] as const));
      // A removed conversation leaves the store outright, whatever a page
      // racing this read brought in: the delete is the one fact no later
      // listing can contradict.
      for (const id of removed) byId.delete(id);
      // A workspace the caller just archived (or deleted) takes its
      // conversations out the same way. Without this they are the one case the
      // rescue below cannot tell apart: their rows disappear from every list
      // while their own status is untouched, so "not on this page" was read as
      // "still open, just off this page" and the shelved workspace kept the
      // column while its rail entry was gone.
      if (leftWorkspaces.size > 0) {
        for (const [id, row] of byId) {
          if (row.workspaceId && leftWorkspaces.has(row.workspaceId))
            byId.delete(id);
        }
      }
      // The conversation on screen stays addressable even when its scope no
      // longer returns it (an archived detail opened from the archived view
      // stays open, read-only) — but never one the caller just deleted or
      // archived, or the row survives the very reload that was meant to drop
      // it, and a shelved conversation keeps drawing as the open one.
      const currentId = activeThreadIdRef.current;
      const leftTheList = (id: string) => {
        if (removed.has(id) || archived.has(id))
          return true;
        if (leftWorkspaces.size === 0)
          return false;
        // The row is very often already out of `byId` — that is what this
        // refresh just did — so the store's held rows answer which workspace it
        // belonged to.
        const workspaceId = byId.get(id)?.workspaceId
          ?? threadsRef.current.find(thread => thread.id === id)?.workspaceId;
        return Boolean(workspaceId && leftWorkspaces.has(workspaceId));
      };
      if (currentId && !byId.has(currentId) && !leftTheList(currentId)) {
        const stillKnown = threadsRef.current.find(thread => thread.id === currentId);
        if (stillKnown)
          byId.set(stillKnown.id, stillKnown);
      }
      setThreads([...byId.values()]);

      // `null` is "nothing is open any more", which is not the same request as
      // "keep whatever is on screen" (`undefined`).
      if (nextActiveThreadId === null) {
        setActiveThreadId(null);
        return;
      }
      const selectable = nextActiveThreadId ?? currentId;
      if (selectable && byId.has(selectable) && !leftTheList(selectable)) {
        setActiveThreadId(selectable);
      }
      else if (selectable && leftTheList(selectable)) {
        // The open conversation was deleted or archived: the column falls back
        // to the nearest one that remains, never to the id that just left. A row
        // a listing still returns (an archived conversation with a live run) is
        // no exception — it is off the list the reader is reading.
        setActiveThreadId(replacementAfterRemoval(
          byId,
          threadsRef.current.find(thread => thread.id === selectable)?.workspaceId,
        ));
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
        const [pinned, pinnedWs, allWorkspaces, firstChat, active] = await Promise.all([
          listPinnedThreads("recent"),
          listPinnedWorkspaces(),
          readAllWorkspaces(),
          listThreadsPage("chat", null, PAGE_ROOTS),
          listActiveThreads(),
        ]);
        if (cancelled) {
          return;
        }
        setWorkspaces([...pinnedWs, ...allWorkspaces]);
        // Bootstrap contributes its own scope rather than replacing the record:
        // the scope record is shared state, and bootstrap is not its only writer
        // (a mutation or a push refreshes scopes too).
        setScopes(previous => ({
          ...previous,
          chat: {
            started: true,
            loadedRoots: PAGE_ROOTS,
            nextCursor: firstChat.nextCursor,
            totalRoots: firstChat.totalRoots,
            loading: false,
          },
        }));
        setThreads((previous) => {
          // Merge, for the same reason: a `threads-updated` push lands during
          // startup and refreshes the store, and replacing the rows here would
          // drop what that refresh brought in. Bootstrap's own rows win where
          // both hold one.
          const byId = new Map(previous.map(thread => [thread.id, thread] as const));
          for (const thread of [...pinned, ...firstChat.threads, ...active]) {
            byId.set(thread.id, thread);
          }
          // A restored session may live under a workspace or a chat page the
          // reader will not open first; make sure the one we select is present.
          if (!byId.has(recentThread.id)) {
            byId.set(recentThread.id, recentThread);
          }
          return [...byId.values()];
        });
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
          // The store is open: issue the scopes the rail asked for while this
          // was still running (it can ask only once per mount, so a request
          // dropped here would leave that list on the chat page until the
          // reader switched views).
          scopesReadyRef.current = true;
          const waiting = waitingScopesRef.current;
          waitingScopesRef.current = [];
          for (const scope of waiting) ensureScopeRef.current(scope);
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
    loadRemaining,
    loadingStore,
    recentThreadIds: scopes[RECENT_SCOPE]?.started ? recentIds : undefined,
    refreshStore,
    scopeHasMore,
    scopeLoading,
    scopeRemaining,
    selectThread,
    setActiveThreadId,
    storeError,
    threadRunStatuses,
    threadStreamingStatuses,
    threads,
    workspaces,
  };
}
