/**
 * The merged conversation list: this machine's threads and every paired remote
 * desktop's sessions, in one list, ordered by the same rule everywhere.
 *
 * Two properties matter and are the reason this is a pure module rather than
 * inline JSX:
 *
 * 1. **Every row knows its source.** A row that cannot name its host would be
 *    routed to the wrong machine on click, so the source is part of the row
 *    identity, not a presentation detail.
 * 2. **The order must be one order.** Local threads already rank by
 *    `pinned, COALESCE(last_message_at, updated_at, created_at)` on the host;
 *    remote sessions carry `lastMessageAt` from that same coalesce. Ranking
 *    them together with a different rule would make the merged list disagree
 *    with the per-machine lists the user can also see.
 *
 * A remote session without a timestamp (an older host) sorts below every
 * timestamped row rather than at the top: "unknown age" must not masquerade as
 * "just now".
 */

/** A session as the host's `list_sessions` reports it, plus our stamp. */
export interface RemoteSessionRow {
  sessionId: string;
  threadId?: string;
  title: string;
  mode?: "chat" | "workspace";
  workspaceId?: string;
  /** The workspace's name on that host; absent on a host that predates it. */
  workspaceName?: string | null;
  parentSessionId?: string | null;
  pinned?: boolean;
  streaming?: boolean;
  status?: string;
  /** Unix millis. Absent on a host that predates the field. */
  lastMessageAt?: number | null;
}

export interface RemoteCatalog {
  /** The paired host this snapshot came from. */
  desktopId: string;
  sessions: RemoteSessionRow[];
}

/** One row of the merged list. */
export interface MergedConversation {
  /** Unique across machines: the same session id on two hosts is two rows. */
  key: string;
  /** `null` for a local thread. */
  desktopId: string | null;
  /** The local thread id, or the remote session id. */
  id: string;
  /**
   * The *host* thread id for a remote row; `null` for local rows.
   *
   * Distinct from `id` on purpose: a remote row is addressed by its session id
   * for anything session-scoped (prompt, abort, rename, reading history), but
   * the host's pin and delete routes take a **thread** id. Dropping this in the
   * merge would make those two actions unaddressable, and guessing that the two
   * ids are interchangeable would pin or delete the wrong record.
   */
  threadId: string | null;
  title: string;
  pinned: boolean;
  streaming: boolean;
  /** Unix millis, or `null` when the source has no timestamp. */
  lastMessageAt: number | null;
  mode: "chat" | "workspace";
  /** The remote workspace this session belongs to; `null` for local rows. */
  workspaceId: string | null;
  /**
   * The workspace's name, for a group heading.
   *
   * Local rows take it from this machine's workspace store; remote rows from the
   * host's catalogue. Absent when neither knows it — a heading is then the
   * machine itself, which is still truthful, where an id would be noise.
   */
  workspaceName?: string | null;
}

export interface LocalThreadLike {
  id: string;
  title: string;
  mode: "chat" | "workspace";
  pinned: boolean;
  status: "active" | "archived" | "deleted";
  workspaceId: string;
  lastMessageAt?: number | null;
  updatedAt: number;
  createdAt: number;
}

/** Which machines the list shows. `null` desktopId means this machine. */
export type DeviceFilter = { kind: "all" } | { kind: "device"; desktopId: string | null };

export function rowKey(desktopId: string | null, id: string): string {
  return `${desktopId ?? "local"}::${id}`;
}

/**
 * The local thread's timestamp, using the host's own coalesce so a thread that
 * has never been messaged still sorts by when it was last touched.
 */
export function localTimestamp(thread: LocalThreadLike): number {
  return thread.lastMessageAt ?? thread.updatedAt ?? thread.createdAt;
}

/**
 * Build the merged list.
 *
 * `catalogs` may contain several hosts; a host that is not connected simply has
 * no entry, and its rows disappear rather than going stale-but-visible. Rows
 * are de-duplicated by `(desktopId, id)`, so a poll that re-reads the same
 * snapshot cannot double a row.
 */
export function mergeConversations(
  threads: LocalThreadLike[],
  catalogs: RemoteCatalog[],
  filter: DeviceFilter,
  /** This machine's workspace names, for the group headings. */
  workspaceNames: ReadonlyMap<string, string> = new Map(),
): MergedConversation[] {
  const rows: MergedConversation[] = [];

  const wantsLocal = filter.kind === "all" || filter.desktopId === null;
  const wantsHost = (desktopId: string) =>
    filter.kind === "all"
    || (filter.kind === "device" && filter.desktopId === desktopId);

  if (wantsLocal) {
    for (const thread of threads) {
      // Deleted threads are not conversations the user can open; archived ones
      // are still listed (the rail has a restore action for them).
      if (thread.status === "deleted")
        continue;
      rows.push({
        key: rowKey(null, thread.id),
        desktopId: null,
        id: thread.id,
        // A local row's thread id *is* its id: the local store addresses
        // threads by that id everywhere, so there is nothing to translate.
        threadId: thread.id,
        title: thread.title,
        pinned: thread.pinned,
        streaming: false,
        lastMessageAt: localTimestamp(thread),
        mode: thread.mode,
        workspaceId: thread.workspaceId,
        workspaceName: workspaceNames.get(thread.workspaceId) ?? null,
      });
    }
  }

  const seen = new Set(rows.map(row => row.key));
  for (const catalog of catalogs) {
    if (!wantsHost(catalog.desktopId))
      continue;
    for (const session of catalog.sessions) {
      const key = rowKey(catalog.desktopId, session.sessionId);
      if (seen.has(key))
        continue;
      seen.add(key);
      rows.push({
        key,
        desktopId: catalog.desktopId,
        id: session.sessionId,
        // Absent only on a host that predates the catalogue's thread id. The
        // row is still readable and promptable; pin and delete stay disabled
        // rather than falling back to the session id, which the host's pin and
        // delete routes do not accept.
        threadId: session.threadId ?? null,
        title: session.title,
        pinned: session.pinned === true,
        streaming: session.streaming === true,
        lastMessageAt: typeof session.lastMessageAt === "number" ? session.lastMessageAt : null,
        mode: session.mode === "workspace" ? "workspace" : "chat",
        workspaceId: session.workspaceId ?? null,
        workspaceName: session.workspaceName ?? null,
      });
    }
  }

  return rows.sort(compareConversations);
}

/**
 * Pinned first, then most recent. Ties break on the row key so the order is
 * stable across polls: a list that reshuffles equal timestamps flickers, and a
 * flickering list is how a user misses the message they were reading.
 */
export function compareConversations(a: MergedConversation, b: MergedConversation): number {
  if (a.pinned !== b.pinned)
    return a.pinned ? -1 : 1;
  const at = a.lastMessageAt ?? Number.NEGATIVE_INFINITY;
  const bt = b.lastMessageAt ?? Number.NEGATIVE_INFINITY;
  if (at !== bt)
    return bt - at;
  return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
}

/** A heading in the merged list, with the rows under it. */
export interface MergedGroup {
  key: string;
  /** `null` for this machine. */
  desktopId: string | null;
  kind: "chat" | "workspace";
  /** The workspace's name, or `null` for the chat section. */
  label: string | null;
  rows: MergedConversation[];
}

/**
 * The merged list's headings: pinned, then workspaces, then chats.
 *
 * The same shape the local list has, because a user who pairs a machine and then
 * cannot find their workspaces has lost something they had before the feature
 * existed. A workspace is identified by `(machine, workspace id)`, not by name: a
 * group called "future-os" exists on two machines at once, and merging them would
 * put another machine's rows under this machine's heading.
 *
 * Groups are ordered by their most recent row, so the list still reads
 * newest-first inside every heading and the headings themselves move with
 * activity rather than being pinned to an arbitrary machine order.
 */
export function groupMergedRows(rows: MergedConversation[]): {
  chats: MergedGroup[];
  pinned: MergedConversation[];
  workspaces: MergedGroup[];
} {
  const pinned = rows.filter(row => row.pinned);
  const rest = rows.filter(row => !row.pinned);

  const byKey = new Map<string, MergedGroup>();
  for (const row of rest) {
    const machine = row.desktopId ?? "local";
    // A chat-mode conversation has no workspace of its own in this list's shape:
    // the local rail shows one chat section for all of them, and a heading per
    // workspace would be a heading for something the row is not in.
    const grouped = row.mode === "workspace";
    // This machine's groups are keyed by the workspace id *alone*, which is the
    // key the local list already folds by: the same workspace then folds and
    // unfolds together in both lists, instead of two fold states disagreeing
    // about one workspace depending on which list is drawn. A remote workspace
    // needs the machine in the key — two machines can hold the same workspace id.
    const key = grouped
      ? (row.desktopId === null ? (row.workspaceId ?? "") : `${machine}::${row.workspaceId ?? ""}`)
      : `${machine}::chat`;
    const existing = byKey.get(key);
    const group: MergedGroup = existing ?? {
      key,
      desktopId: row.desktopId,
      kind: grouped ? "workspace" : "chat",
      label: row.workspaceName ?? null,
      rows: [],
    };
    if (!existing)
      byKey.set(key, group);
    // An older row whose host predates `workspaceName` must not blank a heading
    // a newer row in the same group supplied.
    group.label ??= row.workspaceName ?? null;
    group.rows.push(row);
  }

  const newest = (group: MergedGroup) =>
    group.rows.reduce(
      (best, row) => Math.max(best, row.lastMessageAt ?? Number.NEGATIVE_INFINITY),
      Number.NEGATIVE_INFINITY,
    );
  const groups = [...byKey.values()];
  groups.sort((a, b) => {
    const gap = newest(b) - newest(a);
    if (Number.isFinite(gap) && gap !== 0)
      return gap;
    return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
  });

  return {
    pinned,
    workspaces: groups.filter(group => group.kind === "workspace"),
    // This machine's chat section first: it is the one the local rail labels
    // "chats", and a reader looking for their own conversations should not have
    // to look past another machine's to find them.
    chats: groups
      .filter(group => group.kind === "chat")
      .sort((a, b) => (a.desktopId === null ? -1 : b.desktopId === null ? 1 : 0)),
  };
}
