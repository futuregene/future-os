import type { StoredThread } from "../../integrations/storage/threadStore";

/** What a tree needs to know about one row of a list. */
export interface TreeItem {
  /** Unique within the list: a local thread id, or a merged row's key. */
  key: string;
  /**
   * The row this one was forked from, or `null`.
   *
   * In the list's own vocabulary — a key, not a parent id — because addressing a
   * parent is the caller's knowledge: a local thread points at a session id, a
   * merged row at the parent's row key.
   */
  parentKey: string | null;
  /** A pinned row is listed in the pinned section, so it leads instead of nesting. */
  pinned: boolean;
}

export interface TreeNode<T> {
  item: T;
  children: TreeNode<T>[];
}

/**
 * Build a tree from a flat list whose rows name their parent.
 *
 * One algorithm for both lists in the rail — this machine's conversations and
 * the merged list's remote ones — because they draw the same thing: a fork as an
 * indented row under the turn it came from, folded by the same `+`/`−` toggle.
 * Only how a row is identified and how it names its parent differ, and both are
 * the caller's arguments.
 *
 * The three rules are the local list's, and are here rather than in either caller
 * so the two cannot drift:
 *
 * - **A pinned row is a root.** It is listed in the pinned section, so hanging it
 *   under its parent would put it in two places or neither.
 * - **A missing parent, or one that is not a row here, makes its child a root** —
 *   never a row that cannot be drawn. A malformed cycle loses one edge instead of
 *   losing every row in it.
 * - **A chain deeper than three levels flattens into the third**, which is as much
 *   indentation as a rail has room for, and loses no conversation.
 *
 * Siblings keep the order they arrive in, which is the order the list is sorted
 * in: the tree decides indentation, never ranking.
 */
export function buildTree<T>(items: T[], shape: (item: T) => TreeItem): TreeNode<T>[] {
  const shapes = new Map(items.map(item => [shape(item).key, shape(item)]));
  const parents = new Map<string, string>();
  for (const { key, parentKey, pinned } of shapes.values()) {
    if (!pinned && parentKey && parentKey !== key && shapes.has(parentKey))
      parents.set(key, parentKey);
  }
  // Break one edge per cycle, including cycles disconnected from every root.
  const resolved = new Set<string>();
  for (const key of shapes.keys()) {
    const path = new Set<string>();
    let id: string | undefined = key;
    while (id && !resolved.has(id)) {
      path.add(id);
      const parent = parents.get(id);
      if (parent && path.has(parent)) {
        parents.delete(id);
        break;
      }
      id = parent;
    }
    for (const visited of path) resolved.add(visited);
  }
  const nodes = new Map(items.map(item => [shape(item).key, { item, children: [] } as TreeNode<T>]));
  const roots: TreeNode<T>[] = [];
  for (const item of items) {
    const node = nodes.get(shape(item).key)!;
    const ancestors: string[] = [];
    let parent = parents.get(shape(item).key);
    while (parent) {
      ancestors.push(parent);
      parent = parents.get(parent);
    }
    // The last ancestor is level 1; attach to level 2 at most.
    const displayParent = ancestors.length > 1 ? ancestors[ancestors.length - 2] : ancestors[0];
    if (displayParent)
      nodes.get(displayParent)!.children.push(node);
    else
      roots.push(node);
  }
  return roots;
}

export interface TreeRow<T> {
  item: T;
  depth: number;
  hasChildren: boolean;
}

/** The rows of a tree that a reader can currently see, parents before children. */
export function visibleTreeRows<T>(
  nodes: TreeNode<T>[],
  key: (item: T) => string,
  expanded: ReadonlySet<string>,
  depth = 0,
): TreeRow<T>[] {
  return nodes.flatMap(node => [
    { item: node.item, depth, hasChildren: node.children.length > 0 },
    ...(expanded.has(key(node.item)) ? visibleTreeRows(node.children, key, expanded, depth + 1) : []),
  ]);
}

/** A conversation of this machine's, as the rail's list draws it. */
export type ThreadTreeNode = TreeNode<StoredThread>;

export interface ThreadTreeRow {
  thread: StoredThread;
  depth: number;
  hasChildren: boolean;
}

/**
 * Which thread each of this machine's threads was forked from, by thread id.
 *
 * A fork names its parent by **agent session** id, and that is resolved here,
 * once: the tree this machine's own list is built from and the parent the merged
 * list hangs a local fork under are the same relation, and two resolutions of it
 * would be two answers to "whose fork is this".
 *
 * A thread whose parent is not in the list (an archived or deleted parent) has no
 * entry: there is no row to hang it under, so it is a root.
 */
export function threadParentIds(threads: StoredThread[]): Map<string, string> {
  const bySession = new Map(
    threads.filter(thread => thread.agentSessionId).map(thread => [thread.agentSessionId, thread]),
  );
  const parents = new Map<string, string>();
  for (const thread of threads) {
    const parentId = thread.parentSessionId ? bySession.get(thread.parentSessionId)?.id : undefined;
    if (parentId && parentId !== thread.id)
      parents.set(thread.id, parentId);
  }
  return parents;
}

/**
 * This machine's conversation tree.
 *
 * Descendants follow their root's section. Pinned children become roots. A
 * missing/archived parent becomes a root too, and a fork is resolved by *agent
 * session* id: a parent in another workspace is still its parent.
 */
export function buildThreadTree(threads: StoredThread[]): ThreadTreeNode[] {
  const parents = threadParentIds(threads);
  return buildTree(threads, thread => ({
    key: thread.id,
    parentKey: parents.get(thread.id) ?? null,
    pinned: thread.pinned,
  }));
}

export function visibleThreadRows(nodes: ThreadTreeNode[], expanded: Set<string>, depth = 0): ThreadTreeRow[] {
  return visibleTreeRows(nodes, thread => thread.id, expanded, depth)
    .map(row => ({ thread: row.item, depth: row.depth, hasChildren: row.hasChildren }));
}
