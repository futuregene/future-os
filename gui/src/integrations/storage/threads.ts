import type { StoredMessage, StoredThread, StoredWorkspace, ThreadCleanupSummary } from "./types";
import { invokeCommand } from "../tauri/invoke";

// ─── Workspaces ──────────────────────────────────────────────────────────

export async function listWorkspaces() {
  return invokeCommand<StoredWorkspace[]>("list_workspaces");
}

export async function createWorkspace(input: {
  name?: string | null;
  path: string;
  description?: string | null;
  createDirectory?: boolean | null;
}) {
  return invokeCommand<StoredWorkspace>("create_workspace", { input });
}

export async function getOrCreateChatWorkspace(input: { threadId: string; title?: string | null }) {
  return invokeCommand<StoredWorkspace>("get_or_create_chat_workspace", {
    threadId: input.threadId,
    title: input.title ?? null,
  });
}

export async function ensureWorkspaceGit(workspaceId: string) {
  return invokeCommand<boolean>("ensure_workspace_git", { workspaceId });
}

export async function renameWorkspace(input: { workspaceId: string; name: string }) {
  return invokeCommand<StoredWorkspace>("rename_workspace", { input });
}

export async function deleteWorkspace(workspaceId: string) {
  return invokeCommand<StoredWorkspace>("delete_workspace", { workspaceId });
}

// ─── Threads ─────────────────────────────────────────────────────────────

export async function getRecentThread() {
  return invokeCommand<StoredThread | null>("get_recent_thread");
}

export async function getThread(threadId: string) {
  return invokeCommand<StoredThread | null>("get_thread", { threadId });
}

export async function listThreads() {
  return invokeCommand<StoredThread[]>("list_threads");
}

// `defaultTitle` is supplied by the UI-layer caller (localized there) so this
// integration module stays free of an i18n dependency.
export async function createDefaultChatThread(defaultTitle: string) {
  return invokeCommand<StoredThread>("create_thread", {
    input: {
      mode: "chat",
      title: defaultTitle,
    },
  });
}

// Bootstrap can run more than once concurrently (React StrictMode double-mounts
// the store hook in dev). Without dedupe, each run sees an empty DB and creates
// its own "New Chat", leaving duplicates. Share one in-flight promise so
// concurrent callers resolve to a single thread; clear it once settled (on
// success and failure) so this only dedupes concurrent bootstraps and never
// hands a later remount a stale (possibly since-deleted) cached thread.
let recentOrDefaultThreadPromise: Promise<StoredThread> | null = null;

export function getRecentOrCreateDefaultThread(defaultTitle: string) {
  recentOrDefaultThreadPromise ??= (async () => (await getRecentThread()) ?? createDefaultChatThread(defaultTitle))()
    .then((thread) => {
      recentOrDefaultThreadPromise = null;
      return thread;
    })
    .catch((error) => {
      recentOrDefaultThreadPromise = null;
      throw error;
    });
  return recentOrDefaultThreadPromise;
}

export async function createThread(input: {
  mode: StoredThread["mode"];
  title?: string | null;
  workspaceId?: string | null;
  workspacePath?: string | null;
  workspaceName?: string | null;
  modelProvider?: string | null;
  modelId?: string | null;
  thinkingLevel?: string | null;
  agentSessionId?: string | null;
}) {
  return invokeCommand<StoredThread>("create_thread", { input });
}

export async function renameThread(input: { threadId: string; title: string }) {
  return invokeCommand<StoredThread>("rename_thread", { input });
}

export async function updateThreadModel(input: {
  threadId: string;
  modelProvider?: string | null;
  modelId?: string | null;
}) {
  return invokeCommand<StoredThread>("update_thread_model", { input });
}

export async function updateThreadThinkingLevel(input: {
  threadId: string;
  thinkingLevel?: string | null;
}) {
  return invokeCommand<StoredThread>("update_thread_thinking_level", { input });
}

export async function pinThread(input: { threadId: string; pinned: boolean }) {
  return invokeCommand<StoredThread>("pin_thread", { input });
}

export async function archiveThread(threadId: string) {
  return invokeCommand<StoredThread>("archive_thread", { threadId });
}

export async function restoreThread(threadId: string) {
  return invokeCommand<StoredThread>("restore_thread", { threadId });
}

export async function deleteThread(threadId: string) {
  return invokeCommand<StoredThread>("delete_thread", { threadId });
}

export async function getThreadCleanupSummary(threadId: string) {
  return invokeCommand<ThreadCleanupSummary>("get_thread_cleanup_summary", { threadId });
}

// ─── Messages ────────────────────────────────────────────────────────────

export async function listMessages(threadId: string) {
  return invokeCommand<StoredMessage[]>("list_messages", { threadId });
}

export async function appendMessage(input: {
  threadId: string;
  runId?: string | null;
  role: StoredMessage["role"];
  contentType?: StoredMessage["contentType"];
  content: string;
  status?: StoredMessage["status"];
}) {
  return invokeCommand<StoredMessage>("append_message", { input });
}

/** Fork the agent session at the given user message, returning the new session id. */
export function forkThread(threadId: string, userMessageContent: string) {
  return invokeCommand<string>("fork_thread", { threadId, userMessageContent });
}

/** Fetch session entries from the agent (primary message source). */
export async function getSessionEntries(threadId: string) {
  return invokeCommand<{ entries: Record<string, unknown>[] }>("get_session_entries", { threadId });
}
