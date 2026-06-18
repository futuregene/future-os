import type {
  GitReview,
  ReferenceTargetSearchResult,
  StoredApprovalRequest,
  StoredArtifact,
  StoredMessage,
  StoredResearchResource,
  StoredReviewChangeset,
  StoredReviewFileChange,
  StoredRun,
  StoredRunEvent,
  StoredThread,
  StoredToolCall,
  StoredToolOutput,
  StoredWorkspace,
  ThreadCleanupSummary,
} from "./types";
import { invoke } from "@tauri-apps/api/core";

export type {
  GitReview,
  ReferenceTargetSearchResult,
  StoredApprovalRequest,
  StoredArtifact,
  StoredMessage,
  StoredResearchResource,
  StoredReviewChangeset,
  StoredReviewFileChange,
  StoredRun,
  StoredRunEvent,
  StoredThread,
  StoredToolCall,
  StoredToolOutput,
  StoredWorkspace,
  ThreadCleanupSummary,
} from "./types";

export async function initializeAppStore() {
  await invoke("initialize_app_store");
}

export async function openPath(path: string) {
  return invoke<void>("open_path", { path });
}

export async function readTextFilePreview(input: {
  path: string;
  maxBytes?: number | null;
}) {
  return invoke<{ content: string; size: number; truncated: boolean }>("read_text_file_preview", {
    maxBytes: input.maxBytes ?? null,
    path: input.path,
  });
}

export async function exportArtifactFile(input: {
  destinationPath: string;
  sourcePath?: string | null;
  content?: string | null;
}) {
  return invoke<void>("export_artifact_file", {
    content: input.content ?? null,
    destinationPath: input.destinationPath,
    sourcePath: input.sourcePath ?? null,
  });
}

export async function cancelStaleApprovalRequests() {
  return invoke<number>("cancel_stale_approval_requests");
}

export async function clearFinishedRuns(threadId: string) {
  return invoke<number>("clear_finished_runs", { threadId });
}

export async function getRecentThread() {
  return invoke<StoredThread | null>("get_recent_thread");
}

export async function getThread(threadId: string) {
  return invoke<StoredThread | null>("get_thread", { threadId });
}

export async function listThreads() {
  return invoke<StoredThread[]>("list_threads");
}

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

export async function createRun(input: {
  threadId: string;
  triggerMessageId?: string | null;
  modelProvider?: string | null;
  modelId?: string | null;
}) {
  return invoke<StoredRun>("create_run", { input });
}

export async function listRuns(threadId: string) {
  return invoke<StoredRun[]>("list_runs", { threadId });
}

export async function updateRunStatus(input: {
  runId: string;
  status: StoredRun["status"];
  errorMessage?: string | null;
}) {
  return invoke<StoredRun>("update_run_status", { input });
}

export async function abortRun(input: {
  threadId: string;
  runId: string;
}) {
  return invoke<StoredRun>("abort_run", input);
}

export async function listRunEvents(runId: string) {
  return invoke<StoredRunEvent[]>("list_run_events", { runId });
}

export async function listToolCalls(runId: string) {
  return invoke<StoredToolCall[]>("list_tool_calls", { runId });
}

export async function listToolOutputs(toolCallId: string) {
  return invoke<StoredToolOutput[]>("list_tool_outputs", { toolCallId });
}

export async function listApprovalRequests(threadId: string) {
  return invoke<StoredApprovalRequest[]>("list_approval_requests", { threadId });
}

export async function decideApprovalRequest(input: {
  approvalRequestId: string;
  status: "approved" | "rejected" | "cancelled";
  decisionNote?: string | null;
}) {
  return invoke<StoredApprovalRequest>("decide_approval_request", { input });
}

export async function listReviewChangesets(threadId: string) {
  return invoke<StoredReviewChangeset[]>("list_review_changesets", { threadId });
}

export async function updateReviewChangesetStatus(input: {
  changesetId: string;
  status: "applied" | "discarded" | "pending";
}) {
  return invoke<StoredReviewChangeset>("update_review_changeset_status", { input });
}

export async function listReviewFileChanges(changesetId: string) {
  return invoke<StoredReviewFileChange[]>("list_review_file_changes", { changesetId });
}

export async function getGitReview(input: {
  workspaceId: string;
  base?: "custom" | "head" | "merge-base" | "upstream";
  customBase?: string | null;
}) {
  return invoke<GitReview>("get_git_review", {
    base: input.base ?? "head",
    customBase: input.customBase ?? null,
    workspaceId: input.workspaceId,
  });
}

export async function listArtifacts(threadId: string) {
  return invoke<StoredArtifact[]>("list_artifacts", { threadId });
}

export async function createArtifact(input: {
  workspaceId: string;
  threadId?: string | null;
  runId?: string | null;
  title: string;
  artifactType: string;
  path?: string | null;
  content?: string | null;
  contentStorage?: string | null;
  summary?: string | null;
}) {
  return invoke<StoredArtifact>("create_artifact", { input });
}

export async function importAttachmentArtifact(input: {
  threadId: string;
  path: string;
}) {
  return invoke<StoredArtifact>("import_attachment_artifact", { input });
}

export async function deleteArtifact(artifactId: string) {
  return invoke<StoredArtifact>("delete_artifact", { artifactId });
}

export async function promoteArtifactToResearch(artifactId: string) {
  return invoke<StoredResearchResource>("promote_artifact_to_research", { artifactId });
}

export async function listResearchResources(workspaceId: string) {
  return invoke<StoredResearchResource[]>("list_research_resources", { workspaceId });
}

export async function searchReferenceTargets(input: {
  workspaceId: string;
  query?: string | null;
  limit?: number | null;
}) {
  return invoke<ReferenceTargetSearchResult[]>("search_reference_targets", { input });
}

export function storedTimeToIso(value: number) {
  return new Date(value).toISOString();
}
