import type { TerminalInfo } from "./types";
import {
  createTerminal,
  listTerminals,
  removeTerminal,
} from "./client";
import {
  createRemoteTerminal,
  listRemoteTerminals,
  removeRemoteTerminal,
} from "./remoteTerminal";

/**
 * The three calls the tab controller makes, whichever machine owns the shells.
 *
 * The controller (`useTerminalTabs`) reconciles a stored tab list against the
 * host's, creates shells and closes them; none of that cares whether "the host"
 * is this machine's loopback server or a paired desktop's control plane. What
 * differs is only the calls, so they are the seam.
 *
 * `scope` is the conversation the tabs belong to. A local conversation names it
 * by thread id; a paired one has already bound `desktopId` + `sessionId` here,
 * and the loopback server is the only side that needs to be told which
 * conversation a list is for.
 */
export interface TerminalApi {
  /** Every shell this conversation has, running and retained-exited. */
  list: (scope: string) => Promise<TerminalInfo[]>;
  create: (scope: string, input: { title?: string }) => Promise<TerminalInfo>;
  /** Terminate one shell. A session the host already forgot is not an error. */
  remove: (id: string) => Promise<void>;
}

export const localTerminalApi: TerminalApi = {
  list: scope => listTerminals(scope),
  create: (scope, input) => createTerminal({ threadId: scope, ...input }),
  remove: id => removeTerminal(id).then(() => undefined),
};

/** One conversation's shells on a paired desktop. */
export function remoteTerminalApi(desktopId: string, sessionId: string): TerminalApi {
  return {
    list: () => listRemoteTerminals(desktopId, sessionId),
    create: (_scope, input) => createRemoteTerminal(desktopId, sessionId, input),
    remove: id => removeRemoteTerminal(desktopId, id),
  };
}

/** A stable key for a remote conversation's tab state (`localStorage`). */
export function remoteTerminalScope(desktopId: string, sessionId: string): string {
  // Prefixed so a remote conversation can never collide with a local thread id,
  // which is what the stored map is keyed by today.
  return `remote:${desktopId}:${sessionId}`;
}
