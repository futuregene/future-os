import { beforeEach, describe, expect, it, vi } from "vitest";

import { invokeCommand } from "../tauri/invoke";
import { initializeAppStore, storedTimeToIso } from "./app";
import {
  deleteArtifact,
  importAttachmentArtifact,
  listArtifacts,
  searchWorkspaceFiles,
} from "./artifacts";
import {
  deleteTempAttachment,
  exportArtifactFile,
  generateImageThumbnail,
  importEphemeralImage,
  inspectAttachment,
  listDirectory,
  openExternalUrl,
  openPath,
  readFileBase64,
  readTextFilePreview,
  resolvePreviewLinkPath,
  savePastedImage,
  validateImageAttachment,
} from "./files";
import { resolveMarkdownReferences } from "./markdownReferences";
import {
  getGitReview,
  getLastRunReview,
  getWorkspaceReviewCapabilities,
  retryRunReview,
} from "./review";
import {
  abortRun,
  clearFinishedRuns,
  createRun,
  decideApprovalRequest,
  getLatestRun,
  getRun,
  listApprovalRequests,
  listLatestRunInfos,
  listPendingApprovalRequests,
  listRunEvents,
  listRunEventsBulk,
  listRunEventsSince,
  listRuns,
  listToolCalls,
  listToolCallsBulk,
  listToolOutputs,
  saveApprovalRule,
  updateRunStatus,
} from "./runs";
import {
  batchDeleteThreads,
  createDefaultChatThread,
  createThread,
  createWorkspace,
  deleteThread,
  deleteWorkspace,
  ensureWorkspaceGit,
  forkThread,
  getRecentOrCreateDefaultThread,
  getRecentThread,
  getSessionEntries,
  getThreadCleanupSummary,
  listThreads,
  listWorkspaces,
  pinThread,
  renameThread,
  renameWorkspace,
  restoreThread,
  updateThreadModel,
  updateThreadThinkingLevel,
} from "./threads";

const invokeMock = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(null);
});

describe("invokeCommand error normalization", () => {
  it("passes Error rejections through unchanged", async () => {
    const failure = new Error("native");
    invokeMock.mockRejectedValue(failure);
    await expect(invokeCommand("cmd")).rejects.toBe(failure);
  });

  it("wraps string rejections", async () => {
    invokeMock.mockRejectedValue("plain failure");
    await expect(invokeCommand("cmd")).rejects.toThrow("plain failure");
  });

  it("reads message / error fields from structured rejections", async () => {
    invokeMock.mockRejectedValueOnce({ message: "m" });
    await expect(invokeCommand("cmd")).rejects.toThrow("m");
    invokeMock.mockRejectedValueOnce({ error: "e" });
    await expect(invokeCommand("cmd")).rejects.toThrow("e");
  });

  it("jSON-stringifies other objects and falls back to a generic message", async () => {
    invokeMock.mockRejectedValueOnce({ code: 7 });
    await expect(invokeCommand("cmd")).rejects.toThrow("{\"code\":7}");
    invokeMock.mockRejectedValueOnce(null);
    await expect(invokeCommand("cmd")).rejects.toThrow("Tauri command \"cmd\" failed");
  });

  it("stringifies circular objects via String()", async () => {
    const circular: Record<string, unknown> = {};
    circular.self = circular;
    invokeMock.mockRejectedValue(circular);
    await expect(invokeCommand("cmd")).rejects.toThrow("[object Object]");
  });

  it("stringifies primitive non-string rejections", async () => {
    invokeMock.mockRejectedValue(42);
    await expect(invokeCommand("cmd")).rejects.toThrow("42");
  });
});

describe("storage invoke wrappers", () => {
  it("app store", async () => {
    await initializeAppStore();
    expect(invokeMock).toHaveBeenCalledWith("initialize_app_store", undefined);
    expect(storedTimeToIso(0)).toBe(new Date(0).toISOString());
  });

  it("artifacts", async () => {
    await listArtifacts("t1");
    expect(invokeMock).toHaveBeenLastCalledWith("list_artifacts", { threadId: "t1" });
    await importAttachmentArtifact({ threadId: "t1", path: "/p" });
    expect(invokeMock).toHaveBeenLastCalledWith("import_attachment_artifact", { input: { threadId: "t1", path: "/p" } });
    await deleteArtifact("a1");
    expect(invokeMock).toHaveBeenLastCalledWith("delete_artifact", { artifactId: "a1" });
    await searchWorkspaceFiles({ workspaceId: "w1", query: "q", limit: 5 });
    expect(invokeMock).toHaveBeenLastCalledWith("search_workspace_files", { input: { workspaceId: "w1", query: "q", limit: 5 } });
  });

  it("files", async () => {
    await openPath("/p");
    expect(invokeMock).toHaveBeenLastCalledWith("open_path", { path: "/p" });
    await listDirectory("/d");
    expect(invokeMock).toHaveBeenLastCalledWith("list_directory", { path: "/d" });
    await openExternalUrl("https://x");
    expect(invokeMock).toHaveBeenLastCalledWith("open_external_url", { url: "https://x" });
    await resolvePreviewLinkPath("/b.md", "t.md");
    expect(invokeMock).toHaveBeenLastCalledWith("resolve_preview_link_path", { baseFile: "/b.md", target: "t.md" });
    await readTextFilePreview({ path: "/p", maxBytes: 10 });
    expect(invokeMock).toHaveBeenLastCalledWith("read_text_file_preview", { maxBytes: 10, path: "/p" });
    await exportArtifactFile({ destinationPath: "/t", sourcePath: "/s" });
    expect(invokeMock).toHaveBeenLastCalledWith("export_artifact_file", { content: null, destinationPath: "/t", sourcePath: "/s" });
    await savePastedImage({ bytes: [1], extension: "png" });
    expect(invokeMock).toHaveBeenLastCalledWith("save_pasted_image", { bytes: [1], extension: "png" });
    await inspectAttachment("/a");
    expect(invokeMock).toHaveBeenLastCalledWith("inspect_attachment", { path: "/a" });
    await validateImageAttachment("/i");
    expect(invokeMock).toHaveBeenLastCalledWith("validate_image_attachment", { path: "/i" });
    await readFileBase64({ path: "/f", maxBytes: 8 });
    expect(invokeMock).toHaveBeenLastCalledWith("read_file_base64", { maxBytes: 8, path: "/f" });
    await readFileBase64({ path: "/f" });
    expect(invokeMock).toHaveBeenLastCalledWith("read_file_base64", { maxBytes: null, path: "/f" });
    await generateImageThumbnail({ threadId: "t", sourcePath: "/s" });
    expect(invokeMock).toHaveBeenLastCalledWith("generate_image_thumbnail", { sourcePath: "/s", threadId: "t" });
    await importEphemeralImage({ threadId: "t", path: "/s", name: "n" });
    expect(invokeMock).toHaveBeenLastCalledWith("import_ephemeral_image", { name: "n", sourcePath: "/s", threadId: "t" });
    await deleteTempAttachment("/tmp");
    expect(invokeMock).toHaveBeenLastCalledWith("delete_temp_attachment", { path: "/tmp" });
  });

  it("markdown references", async () => {
    await expect(resolveMarkdownReferences("w", [])).resolves.toEqual([]);
    expect(invokeMock).not.toHaveBeenCalled();
    await resolveMarkdownReferences("w", [{ targetType: "run", targetId: "r" }]);
    expect(invokeMock).toHaveBeenCalledWith("resolve_markdown_references", {
      input: { references: [{ targetType: "run", targetId: "r" }], workspaceId: "w" },
    });
  });

  it("review", async () => {
    await getWorkspaceReviewCapabilities("w");
    expect(invokeMock).toHaveBeenLastCalledWith("get_workspace_review_capabilities", { workspaceId: "w" });
    await getLastRunReview("t");
    expect(invokeMock).toHaveBeenLastCalledWith("get_last_run_review", { threadId: "t" });
    await retryRunReview("r");
    expect(invokeMock).toHaveBeenLastCalledWith("retry_run_review", { runId: "r" });
    await getGitReview({ workspaceId: "w" });
    expect(invokeMock).toHaveBeenLastCalledWith("get_git_review", { base: "head", customBase: null, workspaceId: "w" });
    await getGitReview({ workspaceId: "w", base: "custom", customBase: "abc" });
    expect(invokeMock).toHaveBeenLastCalledWith("get_git_review", { base: "custom", customBase: "abc", workspaceId: "w" });
  });

  it("runs", async () => {
    await createRun({ threadId: "t", triggerMessageId: "m" });
    expect(invokeMock).toHaveBeenLastCalledWith("create_run", { input: { threadId: "t", triggerMessageId: "m" } });
    await listRuns("t");
    expect(invokeMock).toHaveBeenLastCalledWith("list_runs", { threadId: "t" });
    await getLatestRun("t");
    expect(invokeMock).toHaveBeenLastCalledWith("get_latest_run", { threadId: "t" });
    await getRun("r");
    expect(invokeMock).toHaveBeenLastCalledWith("get_run", { runId: "r" });
    await listLatestRunInfos(["t1", "t2"]);
    expect(invokeMock).toHaveBeenLastCalledWith("list_latest_run_infos", { threadIds: ["t1", "t2"] });
    await updateRunStatus({ runId: "r", status: "completed" });
    expect(invokeMock).toHaveBeenLastCalledWith("update_run_status", { input: { runId: "r", status: "completed" } });
    await abortRun({ threadId: "t", runId: "r" });
    expect(invokeMock).toHaveBeenLastCalledWith("abort_run", { threadId: "t", runId: "r" });
    await clearFinishedRuns("t");
    expect(invokeMock).toHaveBeenLastCalledWith("clear_finished_runs", { threadId: "t" });
    await listRunEvents("r");
    expect(invokeMock).toHaveBeenLastCalledWith("list_run_events", { runId: "r" });
    await listRunEventsSince("r", 5);
    expect(invokeMock).toHaveBeenLastCalledWith("list_run_events_since", { runId: "r", sinceSequence: 5 });
    await listRunEventsBulk(["r1"]);
    expect(invokeMock).toHaveBeenLastCalledWith("list_run_events_bulk", { runIds: ["r1"] });
    await listToolCalls("r");
    expect(invokeMock).toHaveBeenLastCalledWith("list_tool_calls", { runId: "r" });
    await listToolCallsBulk(["r1"]);
    expect(invokeMock).toHaveBeenLastCalledWith("list_tool_calls_bulk", { runIds: ["r1"] });
    await listToolOutputs("r", "tc");
    expect(invokeMock).toHaveBeenLastCalledWith("list_tool_outputs", { runId: "r", toolCallId: "tc" });
    await listApprovalRequests("t");
    expect(invokeMock).toHaveBeenLastCalledWith("list_approval_requests", { threadId: "t" });
    await listPendingApprovalRequests();
    expect(invokeMock).toHaveBeenLastCalledWith("list_pending_approval_requests", undefined);
    await decideApprovalRequest({ approvalRequestId: "a", status: "approved" });
    expect(invokeMock).toHaveBeenLastCalledWith("decide_approval_request", { input: { approvalRequestId: "a", status: "approved" } });
    await saveApprovalRule({ threadId: "t", path: "/p", access: "read" });
    expect(invokeMock).toHaveBeenLastCalledWith("save_approval_rule", { input: { threadId: "t", path: "/p", access: "read" } });
  });

  it("threads and workspaces", async () => {
    await listWorkspaces();
    expect(invokeMock).toHaveBeenLastCalledWith("list_workspaces", undefined);
    await createWorkspace({ path: "/w" });
    expect(invokeMock).toHaveBeenLastCalledWith("create_workspace", { input: { path: "/w" } });
    await ensureWorkspaceGit("w");
    expect(invokeMock).toHaveBeenLastCalledWith("ensure_workspace_git", { workspaceId: "w" });
    await renameWorkspace({ workspaceId: "w", name: "n" });
    expect(invokeMock).toHaveBeenLastCalledWith("rename_workspace", { input: { workspaceId: "w", name: "n" } });
    await deleteWorkspace("w");
    expect(invokeMock).toHaveBeenLastCalledWith("delete_workspace", { workspaceId: "w" });
    await getRecentThread();
    expect(invokeMock).toHaveBeenLastCalledWith("get_recent_thread", undefined);
    await listThreads();
    expect(invokeMock).toHaveBeenLastCalledWith("list_threads", undefined);
    await createDefaultChatThread("New Chat");
    expect(invokeMock).toHaveBeenLastCalledWith("create_thread", { input: { mode: "chat", title: "New Chat" } });
    await createThread({ mode: "chat", title: "t" });
    expect(invokeMock).toHaveBeenLastCalledWith("create_thread", { input: { mode: "chat", title: "t" } });
    await renameThread({ threadId: "t", title: "n" });
    expect(invokeMock).toHaveBeenLastCalledWith("rename_thread", { input: { threadId: "t", title: "n" } });
    await updateThreadModel({ threadId: "t", modelId: "m" });
    expect(invokeMock).toHaveBeenLastCalledWith("update_thread_model", { input: { threadId: "t", modelId: "m" } });
    await updateThreadThinkingLevel({ threadId: "t", thinkingLevel: "high" });
    expect(invokeMock).toHaveBeenLastCalledWith("update_thread_thinking_level", { input: { threadId: "t", thinkingLevel: "high" } });
    await pinThread({ threadId: "t", pinned: true });
    expect(invokeMock).toHaveBeenLastCalledWith("pin_thread", { input: { threadId: "t", pinned: true } });
    await restoreThread("t");
    expect(invokeMock).toHaveBeenLastCalledWith("restore_thread", { threadId: "t" });
    await deleteThread({ threadId: "t", deleteFiles: true });
    expect(invokeMock).toHaveBeenLastCalledWith("delete_thread", { input: { threadId: "t", deleteFiles: true } });
    await batchDeleteThreads({ threadIds: ["t"], deleteFiles: false });
    expect(invokeMock).toHaveBeenLastCalledWith("batch_delete_threads", { input: { threadIds: ["t"], deleteFiles: false } });
    await getThreadCleanupSummary("t");
    expect(invokeMock).toHaveBeenLastCalledWith("get_thread_cleanup_summary", { threadId: "t" });
    await forkThread("t", "content", 2);
    expect(invokeMock).toHaveBeenLastCalledWith("fork_thread", { threadId: "t", userMessageContent: "content", userMessageIndex: 2 });
    await getSessionEntries("t");
    expect(invokeMock).toHaveBeenLastCalledWith("get_session_entries", { threadId: "t" });
  });
});

describe("getRecentOrCreateDefaultThread", () => {
  it("returns the recent thread when one exists", async () => {
    invokeMock.mockResolvedValue({ id: "existing" });
    await expect(getRecentOrCreateDefaultThread("New Chat")).resolves.toEqual({ id: "existing" });
    expect(invokeMock).toHaveBeenCalledWith("get_recent_thread", undefined);
  });

  it("creates a default thread when none exists and dedupes concurrent calls", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_recent_thread")
        return Promise.resolve(null);
      return Promise.resolve({ id: "created" });
    });
    const [a, b] = await Promise.all([
      getRecentOrCreateDefaultThread("New Chat"),
      getRecentOrCreateDefaultThread("New Chat"),
    ]);
    expect(a).toBe(b);
    expect(invokeMock.mock.calls.filter(([cmd]) => cmd === "get_recent_thread")).toHaveLength(1);
  });

  it("clears the in-flight promise on failure so a retry can run", async () => {
    invokeMock.mockRejectedValueOnce(new Error("db down"));
    await expect(getRecentOrCreateDefaultThread("New Chat")).rejects.toThrow("db down");
    invokeMock.mockResolvedValue({ id: "existing" });
    await expect(getRecentOrCreateDefaultThread("New Chat")).resolves.toEqual({ id: "existing" });
  });
});
