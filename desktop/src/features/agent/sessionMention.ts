import type { StoredThread, StoredWorkspace } from "../../integrations/storage/types";

/** One selectable conversation in the composer's `#` menu. */
export interface SessionMentionOption {
  /** Agent session id — the payload a reference carries. */
  sessionId: string;
  title: string;
  /**
   * Workspace the conversation is filed under; null for a chat.
   *
   * The menu shows it beside the title rather than filing rows under it: the
   * list's one order is recency, and a heading would cut that order into
   * stretches that each restart at the top. The name is still worth a row: a
   * query matches it (see `filterSessionMentions`), and a match the reader
   * cannot see the reason for reads as random.
   */
  workspace: { id: string; name: string } | null;
}

/**
 * The conversations the `#` menu offers, newest activity first.
 *
 * The menu answers one question — "which conversation do I mean" — and the
 * conversation the reader just used is nearly always the answer, so the order
 * is single and absolute: the most recently active conversation first. It
 * therefore files no rows under their workspaces the way the rail does, and it
 * does not hoist pinned conversations: a pin is about where a conversation sits
 * in the navigation list, and acting on it here would push the one just used
 * down the menu it was opened for. The rail is one keystroke away for someone
 * navigating.
 *
 * "Activity" is the store's own key, `COALESCE(last_message_at, updated_at)` —
 * the same one `list_threads` orders by and the one the host sends the phone as
 * `lastMessageAt`, so the two ends rank the same conversation the same way. A
 * thread that has never been prompted still has `updated_at`, so every row is
 * dated.
 *
 * A conversation is a chat by its `mode`, never by its workspace. Every chat
 * conversation owns a temporary workspace row (`get_or_create_chat_workspace_in`,
 * named "<title> Workspace"), so filing by `workspaceId` gave each chat a group
 * of its own — a wall of workspaces that were really conversations. Only a
 * workspace-mode conversation names its workspace, exactly as the rail scopes
 * threads (`mode === "chat" ? "chat" : workspaceId`).
 *
 * A thread without an agent session id is skipped: that id is the whole point of
 * the reference, and a conversation that was never prompted has none.
 * `currentSessionId` is dropped — offering the conversation you are already in
 * would insert a reference to yourself.
 */
export function sessionMentionOptions(
  threads: StoredThread[],
  workspaces: StoredWorkspace[],
  currentSessionId?: string | null,
): SessionMentionOption[] {
  const byId = new Map(workspaces.map(workspace => [workspace.id, workspace]));
  return threads
    .filter(thread => Boolean(thread.agentSessionId)
      && thread.agentSessionId !== currentSessionId
      && thread.status !== "deleted")
    .map(thread => ({ thread, at: lastActivityAt(thread) }))
    .sort((left, right) => right.at - left.at || compareText(left.thread.id, right.thread.id))
    .map(({ thread }): SessionMentionOption => {
      const scope = threadScope(thread);
      if (!scope)
        return { sessionId: thread.agentSessionId as string, title: thread.title, workspace: null };
      const workspace = byId.get(scope);
      return {
        sessionId: thread.agentSessionId as string,
        title: thread.title,
        workspace: workspace
          ? { id: workspace.id, name: workspace.name }
          : { id: scope, name: "" },
      };
    });
}

/** The store's activity key: `COALESCE(last_message_at, updated_at)`. */
function lastActivityAt(thread: StoredThread): number {
  return thread.lastMessageAt ?? thread.updatedAt;
}

/** The scope a conversation belongs to, or null for a chat (the rail's rule). */
function threadScope(thread: StoredThread): string | null {
  if (thread.mode === "chat")
    return null;
  return thread.workspaceId || null;
}

/**
 * Filter the menu to conversations matching `query`. A title match counts on
 * its own; a conversation filed under a matching workspace comes along, so a
 * user who remembers the workspace but not the title still finds it. The
 * recency order `sessionMentionOptions` returned is preserved.
 */
export function filterSessionMentions(
  options: SessionMentionOption[],
  query: string,
): SessionMentionOption[] {
  const search = query.trim().toLocaleLowerCase();
  if (!search)
    return options;
  return options.filter(option =>
    option.title.toLocaleLowerCase().includes(search)
    || (option.workspace?.name ?? "").toLocaleLowerCase().includes(search));
}

/** A total order on text; `localeCompare` is locale-dependent and not stable. */
function compareText(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}
