/**
 * The commands a conversation row issues, in the shape every machine answers.
 *
 * The list merges the conversations of several desktops, so a row's actions
 * have to go to the machine that owns it: rename, pin, delete and title
 * generation are commands to *that* desktop, which is not necessarily the one
 * this phone's active connection is talking to. Both implementations speak the
 * same RPC surface — a desktop answers a paired client with the same commands
 * it answers its own window with — so the row code holds one of these and does
 * not branch on which machine it is for.
 *
 * `setPinned` and `delete` name the host's thread as well as its session: the
 * session id alone is the lane a command travels on, not an identity the host
 * can act on. A host that reported no thread id cannot be addressed for either.
 */
export interface SessionCommands {
  rename(sessionId: string, name: string): Promise<void>;
  /** Generate a title on that machine, in the language the app is showing. */
  generateTitle(sessionId: string, language: string): Promise<string>;
  setPinned(sessionId: string, threadId: string, pinned: boolean): Promise<void>;
  delete(sessionId: string, threadId: string): Promise<void>;
}
