import type { DesktopCommands } from "../remote/desktopCommands";
import type { MergedSessionRow } from "../remote/mergeSessions";
import type { RemoteSession } from "../remote/types";

/**
 * One conversation a batch can address: the key a tick refers to, what it is,
 * and the machine that owns it.
 *
 * The key is what a batch holds, not the session id: the merged list shows two
 * machines at once, and the same session id exists on both — keying by id would
 * tick a row on one machine when the user ticked the other's. In the ordinary
 * single-machine list a session row's key *is* its id.
 */
export interface SessionTarget {
  key: string;
  session: RemoteSession;
  desktopId: string | null;
}

/**
 * Every conversation a list holds, folded or not, under the key a batch uses
 * for it.
 *
 * A batch cannot be resolved against the drawn rows: a folded group's rows
 * leave the list while a selection made before the fold still has to be
 * deleted, and a "select all in this workspace" is exactly the action that
 * reaches conversations the reader cannot see. Folded groups are therefore read
 * from the catalogue, not from what is on screen.
 *
 * `merged` is the list's own merged rows when it is drawing several machines,
 * and undefined when it draws the active desktop's tree — the same choice the
 * list itself makes between its two row builders. The screen that renders the
 * list's entry points asks the same question of the same function, so the two
 * cannot disagree about whether a batch has anything to hold.
 */
export function listTargets(
  merged: readonly MergedSessionRow[] | undefined,
  sessions: readonly RemoteSession[],
): SessionTarget[] {
  if (merged !== undefined)
    return merged.map(row => ({
      key: row.key,
      session: row.session,
      desktopId: row.desktopId,
    }));
  return sessions.map(session => ({
    key: session.sessionId,
    session,
    desktopId: null,
  }));
}

/**
 * What a batch may hold: conversations that are not pinned shortcuts, on a
 * machine that can take the command.
 *
 * Pinned sessions are promoted above the groups (and, on the workspace page,
 * out of their own workspace), so they carry no checkbox while selecting and
 * select-all never sweeps them up. A machine that cannot answer is left out for
 * the same reason its menu items are: a checkbox would promise a delete it
 * would refuse.
 */
export function selectableTargets(
  targets: readonly SessionTarget[],
  commandsFor: (desktopId: string | null) => DesktopCommands | null,
): SessionTarget[] {
  return targets.filter(
    target => !target.session.pinned && commandsFor(target.desktopId) !== null,
  );
}
