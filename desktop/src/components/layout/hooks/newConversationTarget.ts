import type { ConversationMode } from "../../../features/agent/NewConversation";

/**
 * Where the next new conversation should open: the target of the one the user
 * created last.
 *
 * The new-conversation screen asks the same question on every visit — a
 * workspace, or a plain chat — and used to answer it with a plain chat no matter
 * what the user chose last time. The choice is a preference about this machine's
 * UI rather than conversation data, so it is kept here in the webview's storage,
 * beside the rail's other layout state (`future.collapsedWorkspaces`,
 * `future.leftPanelWidth`), and survives a restart.
 */
export interface NewConversationTarget {
  mode: ConversationMode;
  /** The workspace a `workspace` target creates in; null for a plain chat. */
  workspaceId: string | null;
}

const STORAGE_KEY = "future.newConversationTarget";

/**
 * The remembered target, or null when the user has not created a conversation
 * yet (or the stored value cannot be trusted). Storage is best effort, like the
 * rail's other layout state: a webview without it degrades to the old default.
 */
export function readNewConversationTarget(): NewConversationTarget | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === null)
      return null;
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null)
      return null;
    const { mode, workspaceId } = parsed as { mode?: unknown; workspaceId?: unknown };
    // A `workspace` target without its id names no folder, which is not a
    // target at all — read it as "nothing remembered" rather than guessing one.
    if (mode === "chat")
      return { mode, workspaceId: null };
    if (mode === "workspace" && typeof workspaceId === "string" && workspaceId.length > 0)
      return { mode, workspaceId };
    return null;
  }
  catch {
    // Unavailable or corrupt storage — nothing remembered.
    return null;
  }
}

/** Remember the target of a conversation the user just created. */
export function writeNewConversationTarget(target: NewConversationTarget) {
  const normalized: NewConversationTarget = target.mode === "workspace" && target.workspaceId
    ? { mode: "workspace", workspaceId: target.workspaceId }
    : { mode: "chat", workspaceId: null };
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(normalized));
  }
  catch { /* Storage is best effort. */ }
}

/**
 * The target a plain "new conversation" opens on.
 *
 * Nothing remembered → a plain chat, which is what this screen did before the
 * preference existed. A workspace that is no longer in the store is not a
 * target any more: binding the composer to a folder that is gone would put the
 * first message somewhere the user did not choose, so that case falls back to a
 * plain chat as well.
 */
export function defaultNewConversationTarget(
  stored: NewConversationTarget | null,
  hasWorkspace: (workspaceId: string) => boolean,
): NewConversationTarget {
  if (stored?.mode === "workspace" && stored.workspaceId && hasWorkspace(stored.workspaceId))
    return stored;
  return { mode: "chat", workspaceId: null };
}
