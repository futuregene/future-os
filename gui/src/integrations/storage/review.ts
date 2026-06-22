import type { GitReview, StoredReviewChangeset, StoredReviewFileChange } from "./types";
import { invoke } from "@tauri-apps/api/core";

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
