import type { StoredThread, StoredWorkspace } from "../../integrations/storage/types";

/** One selectable conversation in the composer's `#` menu. */
export interface SessionMentionOption {
  /** Agent session id — the payload a reference carries. */
  sessionId: string;
  title: string;
  /** Workspace the conversation is filed under; null for a chat (no workspace). */
  workspace: { id: string; name: string } | null;
}

/** A workspace's conversations in the `#` menu, or the chats (workspace: null). */
export interface SessionMentionGroup {
  workspace: { id: string; name: string } | null;
  sessions: SessionMentionOption[];
}

/**
 * The conversations the `#` menu offers, in menu order: chats (no workspace)
 * first, then one group per workspace, pinned workspaces leading — the same
 * order the rail and the mobile share sheet use, so the menu does not disagree
 * with the list the user just came from.
 *
 * A thread without an agent session id is skipped: that id is the whole point
 * of the reference, and a conversation that was never prompted has none.
 * `currentSessionId` is dropped — offering the conversation you are already in
 * would insert a reference to yourself.
 */
export function sessionMentionOptions(
  threads: StoredThread[],
  workspaces: StoredWorkspace[],
  currentSessionId?: string | null,
): SessionMentionOption[] {
  const ordered = [
    ...workspaces.filter(workspace => workspace.pinned),
    ...workspaces.filter(workspace => !workspace.pinned),
  ];
  const rank = new Map(ordered.map((workspace, index) => [workspace.id, index]));
  const byId = new Map(workspaces.map(workspace => [workspace.id, workspace]));
  const options = threads
    .filter(thread => Boolean(thread.agentSessionId)
      && thread.agentSessionId !== currentSessionId
      && thread.status !== "deleted")
    .map((thread): SessionMentionOption => {
      const workspace = thread.workspaceId ? byId.get(thread.workspaceId) : undefined;
      return {
        sessionId: thread.agentSessionId as string,
        title: thread.title,
        workspace: workspace ? { id: workspace.id, name: workspace.name } : null,
      };
    });
  // A chat (no workspace) leads; the rest follow their workspace's rank. Ties —
  // two conversations in the same workspace — keep the caller's order, which
  // the rail already sorted (pinned first, then most recent).
  return options
    .map((option, index) => ({ option, index }))
    .sort((left, right) => {
      const rankOf = (option: SessionMentionOption) =>
        option.workspace ? (rank.get(option.workspace.id) ?? ordered.length) : -1;
      return rankOf(left.option) - rankOf(right.option) || left.index - right.index;
    })
    .map(entry => entry.option);
}

/**
 * Filter the menu to conversations matching `query`. A title match counts on
 * its own; a conversation filed under a matching workspace comes along, so a
 * user who remembers the workspace but not the title still finds it.
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

/** Re-group a filtered option list for display; empty groups are dropped. */
export function groupSessionMentions(options: SessionMentionOption[]): SessionMentionGroup[] {
  const groups: SessionMentionGroup[] = [];
  for (const option of options) {
    const last = groups[groups.length - 1];
    if (last && (last.workspace?.id ?? null) === (option.workspace?.id ?? null)) {
      last.sessions.push(option);
      continue;
    }
    groups.push({ workspace: option.workspace, sessions: [option] });
  }
  return groups;
}
