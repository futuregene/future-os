import type { RemoteSession } from "./types";

/**
 * The merged conversation list: every paired desktop's sessions in one list,
 * ordered by one rule.
 *
 * The rule has to match the desktop app's, because the same sessions are shown
 * there: `pinned` first, then most recent, using the host's own
 * `lastMessageAt` (the agent's `COALESCE(last_message_at, updated_at,
 * created_at)`). Ranking them differently on the phone would make one machine's
 * conversation appear in two different places depending on the device holding
 * it.
 *
 * A session without a timestamp (a desktop older than that field) sorts *below*
 * every dated row: "unknown age" must not masquerade as "just now". Ties break
 * on a stable key so a poll cannot reshuffle the list under the reader.
 */

/** Which desktops the list shows. `null` desktopId means the active one only. */
export type DesktopFilter = { kind: "all" } | { kind: "desktop"; desktopId: string };

export interface CatalogEntry {
  desktopId: string;
  /** The desktop's display label (its `name`, else its id). */
  label: string;
  sessions: RemoteSession[];
}

export interface MergedSessionRow {
  /** Unique across desktops: the same session id on two machines is two rows. */
  key: string;
  desktopId: string;
  session: RemoteSession;
  /** Unix millis, or null when the source has no timestamp. */
  lastMessageAt: number | null;
}

export function sessionKey(desktopId: string, sessionId: string): string {
  return `${desktopId}::${sessionId}`;
}

export function mergeSessions(
  catalogs: CatalogEntry[],
  filter: DesktopFilter,
): MergedSessionRow[] {
  const rows: MergedSessionRow[] = [];
  const seen = new Set<string>();
  for (const catalog of catalogs) {
    if (filter.kind === "desktop" && filter.desktopId !== catalog.desktopId) continue;
    for (const session of catalog.sessions) {
      const key = sessionKey(catalog.desktopId, session.sessionId);
      if (seen.has(key)) continue;
      seen.add(key);
      rows.push({
        key,
        desktopId: catalog.desktopId,
        session,
        lastMessageAt: typeof session.lastMessageAt === "number" ? session.lastMessageAt : null,
      });
    }
  }
  return rows.sort(compareSessions);
}

export function compareSessions(a: MergedSessionRow, b: MergedSessionRow): number {
  const pinnedA = a.session.pinned === true;
  const pinnedB = b.session.pinned === true;
  if (pinnedA !== pinnedB) return pinnedA ? -1 : 1;
  const at = a.lastMessageAt ?? Number.NEGATIVE_INFINITY;
  const bt = b.lastMessageAt ?? Number.NEGATIVE_INFINITY;
  if (at !== bt) return bt - at;
  return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
}

/**
 * How many conversations the filter hides.
 *
 * Shown next to the picker so "one desktop" does not read as "my sessions
 * disappeared" — the same reassurance the desktop app gives in the same spot.
 */
export function hiddenSessionCount(catalogs: CatalogEntry[], filter: DesktopFilter): number {
  const total = mergeSessions(catalogs, { kind: "all" }).length;
  return total - mergeSessions(catalogs, filter).length;
}
