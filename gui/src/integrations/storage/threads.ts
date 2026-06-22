import type { StoredMessage, StoredThread, StoredWorkspace, ThreadCleanupSummary } from "./types";
import { invoke } from "@tauri-apps/api/core";

// ─── Workspaces ──────────────────────────────────────────────────────────

export async function listWorkspaces() {
  return invoke<StoredWorkspace[]>("list_workspaces");
}

export async function createWorkspace(input: {
  name?: string | null;
  path: string;
  description?: string | null;
  createDirectory?: boolean | null;
}) {
  return invoke<StoredWorkspace>("create_workspace", { input });
}

export async function getOrCreateChatWorkspace(input: { threadId: string; title?: string | null }) {
  return invoke<StoredWorkspace>("get_or_create_chat_workspace", input);
}

export async function ensureWorkspaceGit(workspaceId: string) {
  return invoke<boolean>("ensure_workspace_git", { workspaceId });
}

// ─── Threads ─────────────────────────────────────────────────────────────

export async function getRecentThread() {
  return invoke<StoredThread | null>("get_recent_thread");
}

export async function getThread(threadId: string) {
  return invoke<StoredThread | null>("get_thread", { threadId });
}

export async function listThreads() {
  return invoke<StoredThread[]>("list_threads");
}

export async function createDefaultChatThread() {
  return invoke<StoredThread>("create_thread", {
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
  return invoke<StoredThread>("create_thread", { input });
}

export async function renameThread(input: { threadId: string; title: string }) {
  return invoke<StoredThread>("rename_thread", { input });
}

export async function updateThreadModel(input: {
  threadId: string;
  modelProvider?: string | null;
  modelId?: string | null;
}) {
  return invoke<StoredThread>("update_thread_model", { input });
}

export async function pinThread(input: { threadId: string; pinned: boolean }) {
  return invoke<StoredThread>("pin_thread", { input });
}

export async function archiveThread(threadId: string) {
  return invoke<StoredThread>("archive_thread", { threadId });
}

export async function restoreThread(threadId: string) {
  return invoke<StoredThread>("restore_thread", { threadId });
}

export async function deleteThread(threadId: string) {
  return invoke<StoredThread>("delete_thread", { threadId });
}

export async function getThreadCleanupSummary(threadId: string) {
  return invoke<ThreadCleanupSummary>("get_thread_cleanup_summary", { threadId });
}

// ─── Messages ────────────────────────────────────────────────────────────

export async function listMessages(threadId: string) {
  return invoke<StoredMessage[]>("list_messages", { threadId });
}

export async function appendMessage(input: {
  threadId: string;
  runId?: string | null;
  role: StoredMessage["role"];
  contentType?: StoredMessage["contentType"];
  content: string;
  status?: StoredMessage["status"];
}) {
  return invoke<StoredMessage>("append_message", { input });
}
