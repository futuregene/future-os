import type { CatalogDeltaPacket, CatalogWorkspace, PresenceSession, RemoteSession } from "./types";

/**
 * Applying a catalogue delta, without React, without a transport.
 *
 * `catalog_delta_v1` promises the host something specific: a packet may carry
 * only the rows that changed (`full: false`), and the client is the half that
 * keeps the rest. The safety of the whole lane rests on one rule, so it lives
 * here in one function rather than in a reducer somewhere:
 *
 *   **a row's absence is not a deletion.** Removals arrive in `removed`; a row
 *   this client already holds and the packet does not mention is kept.
 *
 * The other half is ordering. A delta names no order — the host's list order is
 * `pinned DESC, activity DESC`, which every row now carries as `sortKey`, so a
 * reader reproduces the host's ranking instead of trusting arrival order.
 */

/** Above every real timestamp, so a pinned row outranks all unpinned ones. */
const PINNED_FLOOR = 1_000_000_000_000_000;

/**
 * The host's ranking, reproduced from fields the row already carries.
 *
 * The host orders by `pinned DESC, COALESCE(last_message_at, updated_at,
 * created_at) DESC`, and a row's `lastMessageAt` is already that coalesce down to
 * `updated_at` — so `pinned ? floor + activity : activity` is the same ranking,
 * without a number per row on the wire (which cost more than the workspace
 * dictionary saved once bodies are compressed).
 */
function hostRank(row: RemoteSession): number {
  const activity =
    typeof row.lastMessageAt === "number"
      ? row.lastMessageAt
      : typeof row.createdAt === "number"
        ? row.createdAt
        : 0;
  return activity + (row.pinned ? PINNED_FLOOR : 0);
}

function byHostOrder(a: RemoteSession, b: RemoteSession): number {
  const delta = hostRank(b) - hostRank(a);
  if (delta !== 0) return delta;
  // The host's own order is unspecified for equal keys; an id tie-break at least
  // makes two clients agree with each other.
  return a.sessionId < b.sessionId ? -1 : a.sessionId > b.sessionId ? 1 : 0;
}

/**
 * One row, with the workspace fields this lane leaves to the dictionary.
 *
 * A packet carries the dictionary only when it moved — a delta that names one
 * changed conversation says nothing about workspaces, and saying nothing is not
 * saying "this workspace has no name". Replacing the row wholesale lost the name
 * the row already had, which the list then drew as its generic "workspace"
 * heading until the next packet carried the dictionary again: the heading
 * flickered between the real name and the fallback, once per streamed row.
 */
function withWorkspace(
  row: RemoteSession,
  dictionary: Map<string, CatalogWorkspace>,
  previous?: RemoteSession,
): RemoteSession {
  const workspace = row.workspaceId ? dictionary.get(row.workspaceId) : undefined;
  if (workspace) {
    return {
      ...row,
      workspaceName: workspace.name ?? null,
      workspacePinned: workspace.pinned ?? null,
      workspaceOrderedAt: workspace.orderedAt ?? null,
    };
  }
  if (!previous) return row;
  return {
    ...row,
    ...(previous.workspaceName !== undefined ? { workspaceName: previous.workspaceName } : {}),
    ...(previous.workspacePinned !== undefined
      ? { workspacePinned: previous.workspacePinned }
      : {}),
    ...(previous.workspaceOrderedAt !== undefined
      ? { workspaceOrderedAt: previous.workspaceOrderedAt }
      : {}),
  };
}

function toRemoteSession(row: PresenceSession): RemoteSession {
  return { ...row } as RemoteSession;
}

/**
 * Merge one packet into the list this client holds.
 *
 * Returns the list to render. Pure: the caller owns the state, the version gate
 * and everything that hangs off a row changing status.
 */
export function applyCatalogDelta(
  current: readonly RemoteSession[],
  packet: CatalogDeltaPacket,
): RemoteSession[] {
  const dictionary = new Map<string, CatalogWorkspace>(
    (packet.workspaces ?? []).map((workspace) => [workspace.id, workspace]),
  );
  const merged = new Map<string, RemoteSession>();
  if (!packet.full) {
    for (const row of current) merged.set(row.sessionId, row);
  }
  for (const incoming of packet.rows ?? []) {
    const previous = merged.get(incoming.sessionId);
    const row = withWorkspace(toRemoteSession(incoming), dictionary, previous);
    // A delta packet may repeat a row this client already holds (the host only
    // guarantees changed rows are *included*), so the packet's copy wins — over
    // the fields it carries. What it does not carry stays.
    merged.set(row.sessionId, row);
  }
  if (packet.workspaces) {
    // A workspace was renamed, re-pinned or moved: every row that names it reads
    // the dictionary's copy, because this lane's rows do not carry their own.
    for (const [id, row] of merged) merged.set(id, withWorkspace(row, dictionary));
  }
  for (const id of packet.removed ?? []) merged.delete(id);
  return [...merged.values()].sort(byHostOrder);
}
