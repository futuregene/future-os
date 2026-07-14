import type { GitReview, LastRunReviewData, WorkspaceReviewCapabilities } from "./types";
import { invokeCommand } from "../tauri/invoke";

export type ReviewBase = "custom" | "head" | "merge-base" | "upstream";

export async function getWorkspaceReviewCapabilities(workspaceId: string) {
  return invokeCommand<WorkspaceReviewCapabilities>("get_workspace_review_capabilities", {
    workspaceId,
  });
}

export async function getLastRunReview(threadId: string) {
  return invokeCommand<LastRunReviewData | null>("get_last_run_review", { threadId });
}

export async function retryRunReview(runId: string) {
  return invokeCommand<LastRunReviewData | null>("retry_run_review", { runId });
}

export async function getGitReview(input: {
  workspaceId: string;
  base?: ReviewBase;
  customBase?: string | null;
}) {
  return invokeCommand<GitReview>("get_git_review", {
    base: input.base ?? "head",
    customBase: input.customBase ?? null,
    workspaceId: input.workspaceId,
  });
}
