/**
 * The panel's order: creation time, plus whatever the user moved.
 *
 * The default is `createdAt` ascending, which is the opposite of the rail's
 * "most recently active first" on purpose. The rail is a timeline; the panel is
 * spatial memory. A cell must not move because someone replied in it — otherwise
 * the place a conversation lives changes every time it is used, and the panel
 * stops being a place at all.
 *
 * The user's own order wins where it exists. That is why this is a *merge* and
 * not a sort:
 *
 * - every key the user has placed, in the order they placed it, comes first —
 *   the algorithm never rewrites a relative order the user chose;
 * - a member the user has not placed yet (a freshly pinned conversation) is
 *   inserted at the first position whose `createdAt` is later, so it appears
 *   where it belongs in time instead of being dumped at the end;
 * - anything in the stored order that is no longer a member is dropped, so
 *   unpinning a conversation moves nobody else;
 * - malformed input (an unknown key, a duplicate) is dropped rather than
 *   guessed at — the stored order is untrusted, like every stored preference.
 *
 * `createdAt` ties are broken by key so two conversations pinned in the same
 * millisecond still have one stable order, on every machine and after every
 * restart.
 */

/** What the order needs to know about one panel member. */
export interface VibeMemberKey {
  /** `local:<threadId>` / `remote:<desktopId>:<sessionId>` — see `vibeKeyOf`. */
  key: string;
  /**
   * Unix millis, or `null` when the source cannot say.
   *
   * A host's catalogue reports no creation time, so a remote member has none. It
   * still has a place — the one the user gave it, or the end of the list — and a
   * `null` never reorders a member that already has one.
   */
  createdAt: number | null;
}

/**
 * A conversation's key in the panel.
 *
 * Namespaced because the panel will hold this machine's conversations and a
 * host's in one list (P1). The key must survive a rename and a reconnect, so it is
 * built from ids, never a title.
 */
export function vibeKeyOfLocal(threadId: string): string {
  return `local:${threadId}`;
}

/**
 * The resolved order of the panel's members: every member exactly once.
 *
 * `preferred` is the stored order as the user left it; it may be stale (a member
 * was unpinned elsewhere), partial (a conversation was pinned after it was
 * written) or empty (the user never reordered). All three are normal inputs.
 */
export function resolveOrder(
  members: readonly VibeMemberKey[],
  preferred: readonly string[],
): string[] {
  const byKey = new Map(members.map(member => [member.key, member]));
  const ordered: string[] = [];
  for (const key of preferred) {
    // Unknown keys are dropped and a repeat keeps its first position: the stored
    // value came from an older build, a hand-edited profile, or a bug, and none of
    // those may duplicate a cell.
    if (byKey.has(key) && !ordered.includes(key))
      ordered.push(key);
  }
  const unplaced = members
    .filter(member => !ordered.includes(member.key))
    .sort((left, right) => timeOf(left) - timeOf(right) || left.key.localeCompare(right.key));
  for (const member of unplaced) {
    // An unknown time never inserts *before* a known one: "I cannot say when this
    // was made" must not claim to be older than everything, and the end of the list
    // is where a reader expects a new member they can then move.
    const at = timeOf(member);
    const index = at === Number.POSITIVE_INFINITY
      ? -1
      : ordered.findIndex(key => timeOf(byKey.get(key)!) > at);
    if (index === -1)
      ordered.push(member.key);
    else
      ordered.splice(index, 0, member.key);
  }
  return ordered;
}

/** A member's time for ordering; unknown sorts last. */
function timeOf(member: VibeMemberKey): number {
  return member.createdAt ?? Number.POSITIVE_INFINITY;
}

/**
 * The order with `key` moved `delta` places (or unchanged at an edge).
 *
 * Used both by the keyboard (`vibe.move.left/right`) and, through the same
 * function, by a drop from a pointer: the panel's order is one list, and there is
 * no second opinion about where a cell went.
 */
export function moveInOrder(
  order: readonly string[],
  key: string,
  delta: number,
): string[] {
  const from = order.indexOf(key);
  if (from === -1 || delta === 0)
    return [...order];
  const to = Math.min(order.length - 1, Math.max(0, from + delta));
  if (to === from)
    return [...order];
  const next = [...order];
  next.splice(from, 1);
  next.splice(to, 0, key);
  return next;
}

/** The order with `key` removed (it is no longer a member). */
export function withoutInOrder(order: readonly string[], key: string): string[] {
  return order.filter(entry => entry !== key);
}
