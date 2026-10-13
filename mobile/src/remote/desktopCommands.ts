/**
 * Everything a list row can ask of one machine, in the shape every machine
 * answers.
 *
 * The list merges the conversations and workspaces of several desktops, so a
 * row's actions have to go to the machine that owns it: rename, pin, delete and
 * title generation are commands to *that* desktop, which is not necessarily the
 * one this phone's active connection is talking to. Both implementations speak
 * the same RPC surface — a desktop answers a paired client with the same
 * commands it answers its own window with — so row code holds one of these and
 * does not branch on which machine it is for.
 *
 * `capabilities` rides here rather than beside it because it is the same
 * question asked of the same machine — what may be offered on its rows — and a
 * second lookup table would be a second thing to keep in step. A machine too old
 * for a command simply does not advertise it.
 *
 * `setSessionPinned` and `deleteSession` name the host's thread as well as its
 * session: the session id alone is the lane a command travels on, not an
 * identity the host can act on. A host that reported no thread id cannot be
 * addressed for either.
 */
export interface DesktopCommands {
  /** What the machine says it supports, from its handshake. */
  capabilities: ReadonlySet<string>;
  renameSession(sessionId: string, name: string): Promise<void>;
  /** Generate a title on that machine, in the language the app is showing. */
  generateTitle(sessionId: string, language: string): Promise<string>;
  setSessionPinned(sessionId: string, threadId: string, pinned: boolean): Promise<void>;
  deleteSession(sessionId: string, threadId: string): Promise<void>;
  /** Rename a workspace the machine owns. The host applies its own name rules. */
  renameWorkspace(workspaceId: string, name: string): Promise<void>;
  setWorkspacePinned(workspaceId: string, pinned: boolean): Promise<void>;
  /** Remove a workspace *and its conversations* on the machine that owns it. */
  deleteWorkspace(workspaceId: string): Promise<void>;
}
