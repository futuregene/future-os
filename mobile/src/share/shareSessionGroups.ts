import type { RemoteSession, RemoteWorkspace } from "../remote/types";

/** A workspace's destinations in the share sheet, or — with `workspace: null` —
 * the workspace-less conversations, which are listed as roots above the groups
 * exactly as the session list lists them. */
export interface ShareSessionGroup {
  workspace: RemoteWorkspace | null;
  sessions: RemoteSession[];
}

/** A title matches on its own; a session filed under a matching workspace comes along. */
function matches(session: RemoteSession, workspaceName: string, search: string): boolean {
  if (!search) return true;
  if (session.title.toLocaleLowerCase().includes(search)) return true;
  return workspaceName.toLocaleLowerCase().includes(search);
}

/**
 * Destinations for the share sheet's "existing conversation" step, filed the way
 * the session list files them: conversations with no workspace first (they are
 * roots there too), then one group per workspace with its sessions under it.
 * A workspace the desktop no longer lists keeps its sessions in a group of its
 * own instead of hiding them; the caller names that group generically, since
 * the catalogue has no name to show.
 *
 * A query matches a title or the name of the workspace a session is filed
 * under. Matching sessions stay in their group, so the caller can either browse
 * the tree or flatten the result and drop the headings.
 */
export function shareSessionGroups(
  sessions: RemoteSession[],
  workspaces: RemoteWorkspace[],
  query = "",
): ShareSessionGroup[] {
  const search = query.trim().toLocaleLowerCase();
  const listed = new Set(workspaces.map(workspace => workspace.id));
  // A pinned workspace leads the groups, the way it leads them in the session list.
  const ordered = [
    ...workspaces.filter(workspace => workspace.pinned),
    ...workspaces.filter(workspace => !workspace.pinned),
  ];
  for (const session of sessions) {
    if (session.mode !== "workspace" || listed.has(session.workspaceId ?? "")) continue;
    listed.add(session.workspaceId ?? "");
    ordered.push({ id: session.workspaceId ?? "", name: "", path: "" });
  }
  const groups: ShareSessionGroup[] = [];
  const chats = sessions.filter(
    session => session.mode !== "workspace" && matches(session, "", search),
  );
  if (chats.length > 0) groups.push({ workspace: null, sessions: chats });
  for (const workspace of ordered) {
    const filed = sessions.filter(
      session => session.mode === "workspace"
        && (session.workspaceId ?? "") === workspace.id
        && matches(session, workspace.name, search),
    );
    if (filed.length > 0) groups.push({ workspace, sessions: filed });
  }
  return groups;
}
