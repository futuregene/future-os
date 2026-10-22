import type { StoredThread } from "../../integrations/storage/threadStore";
import type { RemoteCatalog, RemoteSessionRow } from "../remote-peer/mergeConversations";

/**
 * What a cell of the panel *is*, before any of them are drawn.
 *
 * One list for both machines, built here so the panel has a single answer to "what
 * is in the grid" and the remote half is not a second code path with its own idea
 * of membership or order. Pure, and separate from the component, so the membership
 * rules are a table of cases rather than something to observe by looking at a grid.
 *
 * Membership is still **the pin**: a local thread with `pinned`, or a host's session
 * its catalogue reports as pinned. A pinned *workspace* contributes nothing (see
 * `VIBE.md` §1).
 */
export interface VibeCell {
  /** `local:<threadId>` / `remote:<desktopId>:<sessionId>` — stable across renames. */
  key: string;
  kind: "local" | "remote";
  title: string;
  /**
   * Unix millis, or `null` when the source cannot say.
   *
   * A host's catalogue carries `lastMessageAt` but no creation time, so a remote cell
   * has none. `null` only affects **where a new member is inserted** (it goes after
   * the ones with a known time); a position the user already chose is never touched.
   */
  createdAt: number | null;
  /** The machine, `null` for this one. */
  desktopId: string | null;
  /** The local thread, for a local cell. */
  thread?: StoredThread;
  /** The host's row, for a remote cell. */
  session?: RemoteSessionRow;
  /** The workspace's name, when the source knows it (a cell's subtitle). */
  workspaceName?: string | null;
}

/** Which machines the panel shows. Mirrors the rail's own filter. */
export type VibeDeviceFilter = { kind: "all" } | { kind: "device"; desktopId: string };

export function vibeKeyOfRemote(desktopId: string, sessionId: string): string {
  return `remote:${desktopId}:${sessionId}`;
}

/**
 * Every pinned conversation the filter lets through, in no particular order.
 *
 * Order is `order.ts`'s job: this answers membership only, because the two questions
 * have different inputs (a pin, a filter) and different rules (nothing vs. merge).
 */
export function vibeCellsOf(
  threads: readonly StoredThread[],
  catalogs: readonly RemoteCatalog[],
  filter: VibeDeviceFilter,
  workspaceNames: ReadonlyMap<string, string>,
): VibeCell[] {
  const cells: VibeCell[] = [];
  if (filter.kind === "all" || filter.desktopId === "local") {
    for (const thread of threads) {
      // Archived and deleted conversations leave the panel as they leave the rail;
      // an unpinned one was never in it.
      if (!thread.pinned || thread.status !== "active")
        continue;
      cells.push({
        createdAt: thread.createdAt,
        desktopId: null,
        key: `local:${thread.id}`,
        kind: "local",
        thread,
        title: thread.title,
        workspaceName: workspaceNames.get(thread.workspaceId) ?? null,
      });
    }
  }
  for (const catalog of catalogs) {
    if (filter.kind === "device" && filter.desktopId !== catalog.desktopId)
      continue;
    for (const session of catalog.sessions) {
      // The pin, and nothing else. A host's row carries no *lifecycle* status: the
      // `status` field on the wire is the last **run**'s (`mergeConversations` reads
      // it into `runStatus`, and `remote_host::catalog` fills it from the run table),
      // so reading it as "is this conversation live" dropped a pinned conversation as
      // soon as it had ever run — which is nearly all of them, and it made the panel
      // look like it only knew this machine's pins. The host's own thread lifecycle
      // is not in the catalogue (deleted threads are already excluded at the source),
      // so an archived conversation of a host's appears until it is unpinned; there is
      // no field to tell it apart, and inventing one here is what caused the bug.
      if (session.pinned !== true)
        continue;
      cells.push({
        createdAt: null,
        desktopId: catalog.desktopId,
        key: vibeKeyOfRemote(catalog.desktopId, session.sessionId),
        kind: "remote",
        session,
        title: session.title,
        workspaceName: session.workspaceName ?? null,
      });
    }
  }
  return cells;
}

/**
 * The machines the menu may filter to, in the order it offers them: this one first,
 * then the paired hosts as the peer list names them.
 *
 * Built from what is *paired*, not from what happens to be in the panel: a filter
 * that offered only machines with pinned conversations would make "show me that
 * machine" disappear exactly when the panel is empty, which is when a reader wants
 * it most.
 */
export function vibeMachines(
  catalogs: readonly RemoteCatalog[],
  peerLabels: ReadonlyMap<string, string>,
): Array<{ id: string; label: string }> {
  const machines = [{ id: "local", label: "" }];
  for (const catalog of catalogs)
    machines.push({ id: catalog.desktopId, label: peerLabels.get(catalog.desktopId) ?? catalog.desktopId });
  return machines;
}
