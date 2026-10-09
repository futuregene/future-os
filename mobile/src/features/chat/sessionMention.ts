import type { RemoteSession, RemoteWorkspace } from "../../remote/types";
import { shareSessionGroups } from "../../share/shareSessionGroups";

/**
 * The conversations the composer's `#` menu offers for a typed query, filed the
 * way the session list files them — the share sheet's own grouping
 * (`shareSessionGroups`): conversations with no workspace first, then one group
 * per workspace with its conversations under it, pinned workspaces leading. A
 * query matches a title or the name of the workspace a conversation is filed
 * under, which is how the menu narrows to one workspace.
 *
 * The conversation being composed in is dropped: offering it would insert a
 * reference to itself. A conversation with no session id is dropped too — that
 * id is the whole payload of the reference.
 */
export function sessionMentionGroups(
  sessions: RemoteSession[],
  workspaces: RemoteWorkspace[],
  query = "",
  currentSessionId?: string | null,
): ReturnType<typeof shareSessionGroups> {
  const candidates = sessions.filter(
    session => session.sessionId !== "" && session.sessionId !== currentSessionId,
  );
  return shareSessionGroups(candidates, workspaces, query);
}
