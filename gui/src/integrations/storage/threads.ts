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

export async function createDefaultChatThread() {
  return invokeCommand<StoredThread>("create_thread", {
    input: {
      mode: "chat",
      title: "New Chat",
    },
  });
}

export async function createThread(input: {
  mode: StoredThread["mode"];
  title?: string | null;
  workspaceId?: string | null;
  workspacePath?: string | null;
  workspaceName?: string | null;
  modelProvider?: string | null;
  modelId?: string | null;
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
