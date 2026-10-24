import type { RemoteCatalog } from "./mergeConversations";

/**
 * What an open conversation is called, remembered for as long as it is open.
 *
 * Its name comes from the *host's* catalogue: a conversation is called what the
 * machine that owns it calls it, so a rename there is what the reader should
 * see here, and a row opened from somewhere that knows only ids (a task run, a
 * notification) has no title of its own to offer. That is also the one source
 * that can go away while the conversation is still open, and it does:
 *
 * - when another paired device takes the machine over, the host refuses this
 *   client's business commands (`superseded`) and sends it no more snapshots, so
 *   every row of that host leaves the catalogue — `useRemotePeers` drops them,
 *   and has to, because a standby device is never sent one;
 * - a catalogue read can fail for reasons that say nothing about the host: a
 *   store busy with a page of history, one round trip that timed out.
 *
 * The transcript survives both — it was read while this client still had the
 * machine — so the title alone fell back to the session id, and a conversation
 * the reader was in the middle of turned into `20261022-164713-ea5b…` in the
 * header. The last title and workspace name seen are therefore kept, per
 * desktop *and* session (the same session id on two hosts is two
 * conversations), and read back when the catalogue no longer carries them.
 *
 * Display only. Nothing here is an authority: the catalogue wins whenever it has
 * a row, and a conversation this client has never seen still falls back to its
 * session id rather than to an empty header. Keeping it is not an exit either —
 * the open conversation stays open, which is the whole point.
 */
export interface RemoteConversationIdentity {
  title: string;
  /** The workspace's name on that host, for a conversation that lives in one. */
  workspaceName: string | null;
}

/**
 * The last identity seen for each conversation, keyed by
 * [`remoteIdentityKey`].
 *
 * Held by the caller (the shell's own ref) rather than as module state: it is a
 * picture of what *this* window has read, and the window that read it is the one
 * that may draw from it.
 */
export type RemoteIdentityMemory = Map<string, RemoteConversationIdentity>;

/** One conversation on one host: the same session id elsewhere is another one. */
export function remoteIdentityKey(desktopId: string, sessionId: string): string {
  return `${desktopId}::${sessionId}`;
}

/**
 * Remember every row the host's catalogues carry.
 *
 * Whole catalogues rather than the open conversation alone: the rows are the
 * same snapshot, so remembering them costs nothing extra, and the next
 * conversation opened on a host that has just gone quiet is described the same
 * way. Idempotent — writing the same titles twice is what a re-render does —
 * so a caller may run it from render.
 */
export function rememberRemoteIdentities(
  catalogs: readonly RemoteCatalog[],
  memory: RemoteIdentityMemory,
): void {
  for (const catalog of catalogs) {
    for (const row of catalog.sessions) {
      memory.set(remoteIdentityKey(catalog.desktopId, row.sessionId), {
        title: row.title,
        workspaceName: identityWorkspace(row.mode, row.workspaceName),
      });
    }
  }
}

/**
 * One conversation's name and workspace, from the catalogue or from memory.
 *
 * The catalogue's row first — it is the truth, and it is where a rename lands —
 * then what was last seen, then the session id, which says which conversation
 * this is to a reader who has never had a title for it.
 */
export function remoteConversationIdentity(
  catalogs: readonly RemoteCatalog[],
  target: { desktopId: string; sessionId: string },
  memory: RemoteIdentityMemory,
): RemoteConversationIdentity {
  const row = catalogs
    .find(catalog => catalog.desktopId === target.desktopId)
    ?.sessions
    .find(session => session.sessionId === target.sessionId);
  if (row) {
    return {
      title: row.title,
      workspaceName: identityWorkspace(row.mode, row.workspaceName),
    };
  }
  return memory.get(remoteIdentityKey(target.desktopId, target.sessionId))
    ?? { title: target.sessionId, workspaceName: null };
}

/**
 * A workspace name, or none.
 *
 * A chat conversation has no workspace of its own, and a host too old to report
 * a name leaves the qualifier off rather than drawing an id: a qualifier the
 * header cannot state truthfully is worse than a missing one.
 */
function identityWorkspace(
  mode: "chat" | "workspace" | undefined,
  workspaceName: string | null | undefined,
): string | null {
  return mode === "workspace" ? (workspaceName ?? null) : null;
}
