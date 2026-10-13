import type { RemoteSession, RemoteWorkspace } from "../remote/types";
import type { MergedSessionRow } from "../remote/mergeSessions";
import { sessionKey } from "../remote/mergeSessions";

export interface SessionNode {
  session: RemoteSession;
  children: SessionNode[];
}

/** Desktop parity: pinned/orphan sessions are roots; break cycles without losing rows. */
export function buildSessionTree(sessions: RemoteSession[]): SessionNode[] {
  const nodes = new Map(
    sessions.map(session => [session.sessionId, { session, children: [] } as SessionNode]),
  );
  const parents = new Map<string, string>();
  for (const session of sessions) {
    const parent = session.parentSessionId;
    if (!session.pinned && parent && parent !== session.sessionId && nodes.has(parent)) {
      parents.set(session.sessionId, parent);
    }
  }
  const resolved = new Set<string>();
  for (const id of nodes.keys()) {
    const path = new Set<string>();
    let current: string | undefined = id;
    while (current && !resolved.has(current)) {
      path.add(current);
      const parent = parents.get(current);
      if (parent && path.has(parent)) {
        parents.delete(current);
        break;
      }
      current = parent;
    }
    for (const visited of path) resolved.add(visited);
  }
  const roots: SessionNode[] = [];
  for (const [id, node] of nodes) {
    const ancestors: string[] = [];
    let parent = parents.get(id);
    while (parent) {
      ancestors.push(parent);
      parent = parents.get(parent);
    }
    const displayParent = ancestors.length > 1 ? ancestors[ancestors.length - 2] : ancestors[0];
    if (displayParent) nodes.get(displayParent)!.children.push(node);
    else roots.push(node);
  }
  return roots;
}

export type CatalogRow =
  | {
    kind: "workspace";
    key: string;
    workspace: RemoteWorkspace;
    count: number;
    /**
     * Which desktop the workspace lives on, in the merged list.
     *
     * Absent for the ordinary single-desktop list, where every heading is the
     * active desktop's and its own group key is the workspace id alone — which
     * is what makes a fold behave the same in both lists.
     */
    desktopId?: string;
    /** The machine's own name, when the list shows more than one. */
    desktopLabel?: string;
  }
  | {
    kind: "session";
    key: string;
    session: RemoteSession;
    depth: number;
    hasChildren: boolean;
    /**
     * What this row's fold is keyed by.
     *
     * A conversation on the machine the phone is talking to folds by its session
     * id — the key the single-machine list already folds by, so one conversation
     * folds the same in both lists. A peer's carries its machine, because a
     * session id is that machine's, not global: two machines can each hold a
     * conversation called `s1`, and one fold must not close both. The same rule
     * the merged list's workspace headings follow, for the same reason.
     */
    foldId: string;
    /**
     * Which desktop the session lives on, when the list is merged.
     *
     * Absent in the ordinary single-desktop list, where every row is the active
     * desktop's and a label would be noise. Present in the merged list, where a
     * row that lost its source would be opened against the wrong machine.
     */
    desktopId?: string;
  };

/** One workspace's rows on one machine, as the merged list files them. */
export interface MergedWorkspaceGroup {
  /** Unique across machines: the same workspace id on two machines is two groups. */
  key: string;
  /** The machine the group belongs to, or null for the chat section. */
  desktopId: string | null;
  kind: "workspace" | "chat";
  /** The workspace's name, from whichever row in the group carried it. */
  label: string | null;
  workspaceId: string | null;
  /** Pinned *on its own machine*, which is what leads the merged list. */
  pinned: boolean;
  /** When the workspace itself was last touched at its source. */
  orderedAt: number | null;
  rows: MergedSessionRow[];
}

/**
 * The merged list's sections: pinned shortcuts, then a group per workspace, then
 * the conversations that live in no workspace.
 *
 * The shape is the desktop app's, on purpose — the same sessions are shown in
 * both, and a phone that filed them differently would make one machine's
 * conversation appear in two places depending on the device holding it. A group
 * is keyed by `(machine, workspace)`: a workspace called "future-os" exists on
 * two machines at once, and merging them would file one machine's rows under the
 * other's heading.
 *
 * Every group fact (name, pin, place) rides on its rows — the host reports them
 * per row — and one row reporting it is the group's answer: a host that predates
 * a field must not blank what a newer row in the same group supplied.
 */
export function groupMergedRows(
  rows: readonly MergedSessionRow[],
): { pinned: MergedSessionRow[]; workspaces: MergedWorkspaceGroup[]; chats: MergedSessionRow[] } {
  const pinned: MergedSessionRow[] = [];
  const byKey = new Map<string, MergedWorkspaceGroup>();
  for (const row of rows) {
    if (row.session.pinned) {
      pinned.push(row);
      continue;
    }
    // A chat-mode conversation has no workspace of its own: the list shows one
    // section for all of them, whatever machine they are on.
    const grouped = row.session.mode === "workspace";
    const workspaceId = row.session.workspaceId ?? "";
    const key = grouped ? `${row.desktopId}::${workspaceId}` : "chat";
    const existing = byKey.get(key);
    const group: MergedWorkspaceGroup = existing ?? {
      key,
      desktopId: grouped ? row.desktopId : null,
      kind: grouped ? "workspace" : "chat",
      label: null,
      workspaceId: grouped ? workspaceId : null,
      pinned: false,
      orderedAt: null,
      rows: [],
    };
    group.label ??= row.session.workspaceName ?? null;
    group.pinned ||= grouped && row.session.workspacePinned === true;
    group.orderedAt ??= row.session.workspaceOrderedAt ?? null;
    group.rows.push(row);
    if (!existing) byKey.set(key, group);
  }
  const groups = [...byKey.values()];
  groups.sort((a, b) => {
    // Pinned groups first, as in the single-machine list; then the workspace's
    // own place, newest first, so a group holds its rank while the reader works
    // in it — a conversation arriving inside a workspace must not move it.
    if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
    const at = a.orderedAt ?? Number.NEGATIVE_INFINITY;
    const bt = b.orderedAt ?? Number.NEGATIVE_INFINITY;
    if (at !== bt) return bt - at;
    if (a.desktopId !== b.desktopId)
      return (a.desktopId ?? "") < (b.desktopId ?? "") ? -1 : 1;
    return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
  });
  const chats = groups.find(group => group.kind === "chat");
  return {
    pinned,
    workspaces: groups.filter(group => group.kind === "workspace"),
    chats: chats ? chats.rows : [],
  };
}

/**
 * The merged list as rows: a heading per workspace, with its conversations under
 * it, and each conversation's forks under *that*.
 *
 * The heading carries the group key as its workspace id for a machine other than
 * the active one: a fold must not make two machines' "future-os" close together,
 * and the active machine keeps its plain id so that folding there behaves
 * exactly as it does in the single-machine list.
 *
 * The tree is the single-machine list's, per machine (see `mergedTree`): a fork
 * is drawn under the conversation it came from, one level in and folded by the
 * same `+`/`−`. A merged list is not a list of roots — the machines' rows carry
 * the same parents here as they do there — and drawing them flat left a fork
 * indistinguishable from a conversation of its own, with no way to fold a
 * conversation that had thirty of them.
 */
export function mergedRows(
  rows: readonly MergedSessionRow[],
  query: string,
  activeDesktopId: string | null,
  collapsed: ReadonlySet<string> = new Set(),
  /**
   * Which conversations have their forks on screen, by session id.
   *
   * The same state the single-machine list folds by, and for the same reason:
   * it is the same fork. One conversation therefore opens and closes together
   * whichever list is drawing it, instead of a fold that depends on where the
   * reader happens to be looking.
   */
  expanded: ReadonlySet<string> = new Set(),
): CatalogRow[] {
  const search = query.trim().toLocaleLowerCase();
  const matches = (row: (typeof rows)[number]): boolean =>
    !search ||
    row.session.title.toLocaleLowerCase().includes(search) ||
    (row.session.workspaceName ?? "").toLocaleLowerCase().includes(search);
  const listed = rows.filter(matches);
  const { children, rootOf } = mergedTree(listed);
  const sessionRow = (
    row: (typeof rows)[number],
    depth: number,
    hasChildren: boolean,
  ): Extract<CatalogRow, { kind: "session" }> => ({
    kind: "session" as const,
    key: row.key,
    session: row.session,
    depth,
    hasChildren,
    foldId: row.desktopId === activeDesktopId ? row.session.sessionId : row.key,
    desktopId: row.desktopId,
  });
  /**
   * A conversation and the forks under it, parents before children.
   *
   * The fold is the single-machine list's: a fork is hidden until the `+` in
   * front of the conversation it came from is pressed, and a search ignores the
   * fold — a hidden fork stays discoverable, at the top level, which is how the
   * single-machine list answers the same question.
   */
  const block = (row: (typeof rows)[number], depth = 0): CatalogRow[] => {
    const forks = children.get(row.key) ?? [];
    const head = sessionRow(row, search ? 0 : depth, !search && forks.length > 0);
    if (!search && !expanded.has(head.foldId)) return [head];
    return [head, ...forks.flatMap(fork => block(fork, depth + 1))];
  };
  /** Everything under a conversation, folded ones included — a heading counts them. */
  const subtree = (row: (typeof rows)[number]): number =>
    1 + (children.get(row.key) ?? []).reduce((total, fork) => total + subtree(fork), 0);
  // Sections and their order are decided by the *conversation* each row hangs
  // under — its root — rather than by the fork's own fields, so a fork follows
  // the conversation it came from: the forks of a pinned conversation are part
  // of that shortcut, and one forked inside a workspace is under that heading.
  // A fork is younger than its parent by construction, so filing it by its own
  // fields or time would put it above the row the reader just unfolded.
  const roots = listed.filter(
    row => (rootOf.get(row.key) ?? row).key === row.key,
  );
  const { pinned, workspaces, chats } = groupMergedRows(roots);
  const out: CatalogRow[] = [];
  // Pinned conversations are shortcuts promoted above the groups, exactly as in
  // the single-machine list.
  for (const row of pinned) out.push(...block(row));
  for (const group of workspaces) {
    const workspaceId = group.workspaceId ?? "";
    // This machine's heading folds by the workspace id alone — the key the
    // single-machine list already folds by, so one workspace folds the same in
    // both lists. A peer's needs its machine in the key: two machines can hold
    // the same workspace id, and one fold must not close both.
    const id = group.desktopId === activeDesktopId ? workspaceId : group.key;
    out.push({
      kind: "workspace",
      key: `merged:${group.key}`,
      workspace: { id, name: group.label ?? workspaceId, path: "" },
      count: group.rows.reduce((total, row) => total + subtree(row), 0),
      ...(group.desktopId ? { desktopId: group.desktopId } : {}),
    });
    // A folded group keeps its heading and its count and hides its rows — and a
    // search ignores folds, so a hidden conversation stays discoverable (the
    // same rule the single-machine list follows).
    if (search || !collapsed.has(id)) {
      for (const row of group.rows) out.push(...block(row));
    }
  }
  for (const row of chats) out.push(...block(row));
  return out;
}

/**
 * Where each listed row hangs, keyed by the row's own key.
 *
 * Built one machine at a time, because a parent is named by a session id that
 * is only meaningful on the machine that issued it: two machines can each hold
 * a conversation called `s1`, and one machine's row must not adopt the other's
 * fork — the same reason a merged row is addressed by `(machine, id)` at all.
 *
 * `buildSessionTree` is the single-machine list's own tree, reused rather than
 * restated: "pinned conversations are roots, a parent that is not listed makes
 * its child a root, a cycle loses one edge" is one relation, and a second
 * implementation of it would be a second answer to "whose fork is this". This
 * is only the mapping from its session-shaped nodes back to the merged rows,
 * which are what carry the machine.
 */
function mergedTree(listed: readonly MergedSessionRow[]): {
  /** A conversation's forks, in the order the catalogue lists them. */
  children: Map<string, MergedSessionRow[]>;
  /** The conversation each row hangs under, itself when it hangs under none. */
  rootOf: Map<string, MergedSessionRow>;
} {
  const byKey = new Map(listed.map(row => [row.key, row]));
  const byDesktop = new Map<string, MergedSessionRow[]>();
  for (const row of listed) {
    const machine = byDesktop.get(row.desktopId);
    if (machine) machine.push(row);
    else byDesktop.set(row.desktopId, [row]);
  }
  const children = new Map<string, MergedSessionRow[]>();
  const rootOf = new Map<string, MergedSessionRow>();
  for (const [desktopId, machineRows] of byDesktop) {
    const walk = (nodes: SessionNode[], root: MergedSessionRow | null): void => {
      for (const node of nodes) {
        const row = byKey.get(sessionKey(desktopId, node.session.sessionId));
        if (!row) continue;
        const block = root ?? row;
        rootOf.set(row.key, block);
        const forks = node.children
          .map(child => byKey.get(sessionKey(desktopId, child.session.sessionId)))
          .filter((fork): fork is MergedSessionRow => fork !== undefined);
        if (forks.length > 0) children.set(row.key, forks);
        walk(node.children, block);
      }
    };
    walk(buildSessionTree(machineRows.map(row => row.session)), null);
  }
  return { children, rootOf };
}

export function catalogRows(
  sessions: RemoteSession[],
  workspaces: RemoteWorkspace[],
  tab: "chat" | "workspace",
  collapsed: Set<string>,
  expanded: Set<string>,
  query: string,
): CatalogRow[] {
  const search = query.trim().toLocaleLowerCase();
  const roots = buildSessionTree(sessions);
  const rows: CatalogRow[] = [];
  const append = (nodes: SessionNode[], depth: number, workspaceMatches = false) => {
    for (const node of nodes) {
      // Search is flat and ignores folds, so a hidden child is still discoverable.
      const matches =
        !search || workspaceMatches || node.session.title.toLocaleLowerCase().includes(search);
      if (matches)
        rows.push({
          kind: "session",
          key: node.session.sessionId,
          session: node.session,
          depth: search ? 0 : depth,
          hasChildren: !search && node.children.length > 0,
          // One machine's list: the session id is the whole identity.
          foldId: node.session.sessionId,
        });
      if (search || expanded.has(node.session.sessionId))
        append(node.children, depth + 1, workspaceMatches);
    }
  };
  if (tab === "chat") {
    append(
      roots.filter(node => node.session.mode !== "workspace"),
      0,
    );
    return rows;
  }
  const workspaceRoots = roots.filter(node => node.session.mode === "workspace");
  const groups = [...workspaces];
  // Retain sessions whose workspace disappeared rather than silently hiding them.
  for (const node of workspaceRoots) {
    const id = node.session.workspaceId ?? "";
    if (!groups.some(workspace => workspace.id === id)) groups.push({ id, name: id, path: "" });
  }
  const matchesWorkspace = (workspace: RemoteWorkspace): boolean =>
    !!search && `${workspace.name} ${workspace.path}`.toLocaleLowerCase().includes(search);
  // The catalog already supplies recency order. Keep that order among pins,
  // but promote them (with their expandable children) above every workspace.
  // Workspace folds must not hide these shortcuts.
  for (const node of workspaceRoots) {
    if (!node.session.pinned) continue;
    const workspace = groups.find(group => group.id === (node.session.workspaceId ?? ""))!;
    append([node], 0, matchesWorkspace(workspace));
  }
  // Pinned workspaces follow those pinned conversations and lead the rest of
  // the groups; within each half the desktop's recency order is kept.
  const orderedGroups = [
    ...groups.filter(workspace => workspace.pinned),
    ...groups.filter(workspace => !workspace.pinned),
  ];
  const countNodes = (nodes: SessionNode[]): number =>
    nodes.reduce((total, node) => total + 1 + countNodes(node.children), 0);
  for (const workspace of orderedGroups) {
    const children = workspaceRoots.filter(
      node => (node.session.workspaceId ?? "") === workspace.id,
    );
    const workspaceMatches = matchesWorkspace(workspace);
    const header: CatalogRow = {
      kind: "workspace",
      key: `workspace:${workspace.id}`,
      workspace,
      count: countNodes(children),
    };
    const start = rows.length;
    rows.push(header);
    // Counts and workspace actions still include promoted pins; display them
    // only once, at the page top, rather than duplicating them in the group.
    if (search || !collapsed.has(workspace.id))
      append(children.filter(node => !node.session.pinned), 0, workspaceMatches);
    if (search && !workspaceMatches && rows.length === start + 1) rows.pop();
  }
  return rows;
}
