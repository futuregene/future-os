import type { RemoteSession, RemoteWorkspace } from "../../remote/types";

/** One selectable conversation in the composer's `#` menu. */
export interface SessionMentionOption {
  /** Agent session id — the payload a reference carries. */
  sessionId: string;
  title: string;
  /**
   * Workspace the conversation is filed under; null for a chat.
   *
   * The picker shows it on the row rather than filing rows under it: the list's
   * one order is recency, and a heading would cut that order into stretches that
   * each restart at the top. The name is still worth a row — a query matches it,
   * and a match whose reason is invisible reads as random.
   */
  workspace: { id: string; name: string } | null;
}

/**
 * The conversations the composer's `#` menu offers, newest activity first,
 * narrowed by `query`.
 *
 * The menu answers one question — "which conversation do I mean" — and the one
 * just used is nearly always the answer, so its order is single and absolute:
 * most recently active first. That is the desktop's order too
 * (`sessionMentionOptions` there), on the desktop's own activity key
 * `COALESCE(last_message_at, updated_at)`, which the host sends as
 * `lastMessageAt`; a desktop older than that field sends none, and the
 * conversation's start time stands in — such a row sorts below the ones that
 * can be dated, never as though it were just used.
 *
 * It therefore does **not** file rows under their workspaces the way the session
 * list does, and it does not hoist pinned conversations: a pin is about where a
 * conversation sits in the navigation list, and acting on it here would push the
 * one just used down the menu it was opened for. The session list is one tab
 * away for someone navigating.
 *
 * The conversation being composed in is dropped: offering it would insert a
 * reference to itself. A conversation with no session id is dropped too — that
 * id is the whole payload of the reference.
 */
export function sessionMentionOptions(
  sessions: RemoteSession[],
  workspaces: RemoteWorkspace[],
  query = "",
  currentSessionId?: string | null,
): SessionMentionOption[] {
  const byId = new Map(workspaces.map(workspace => [workspace.id, workspace]));
  const search = query.trim().toLocaleLowerCase();
  return sessions
    .filter(session => session.sessionId !== "" && session.sessionId !== currentSessionId)
    .map(session => ({
      option: toOption(session, byId),
      at: session.lastMessageAt ?? session.createdAt ?? Number.NEGATIVE_INFINITY,
    }))
    .filter(({ option }) => !search
      || option.title.toLocaleLowerCase().includes(search)
      || (option.workspace?.name ?? "").toLocaleLowerCase().includes(search))
    .sort((left, right) => right.at - left.at
      || compareText(left.option.sessionId, right.option.sessionId))
    .map(({ option }) => option);
}

function toOption(
  session: RemoteSession,
  workspaces: Map<string, RemoteWorkspace>,
): SessionMentionOption {
  // A chat is a chat by `mode`, never by its workspace: the desktop stores every
  // chat in a temporary workspace of its own, named after the chat, and naming
  // that here would label a chat with its own title.
  if (session.mode !== "workspace")
    return { sessionId: session.sessionId, title: session.title, workspace: null };
  const id = session.workspaceId ?? "";
  const name = workspaces.get(id)?.name ?? session.workspaceName ?? "";
  return { sessionId: session.sessionId, title: session.title, workspace: { id, name } };
}

/** A total order on text; `localeCompare` is locale-dependent and not stable. */
function compareText(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}
