import type {
  StoredApprovalRequest,
  StoredArtifact,
  StoredResearchResource,
  StoredReviewChangeset,
  StoredRun,
  StoredToolCall,
} from "./types";
import { invoke } from "@tauri-apps/api/core";

export type FutureReferenceType = "approval" | "artifact" | "research" | "review" | "run" | "tool";

export interface MarkdownReferenceRequest {
  targetType: FutureReferenceType;
  targetId: string;
}

export interface ResolvedMarkdownReference {
  targetType: FutureReferenceType | string;
  targetId: string;
  status: "resolved" | "missing" | "forbidden" | string;
  data?: StoredApprovalRequest | StoredArtifact | StoredResearchResource | StoredReviewChangeset | StoredRun | StoredToolCall | null;
  error?: string | null;
}

export async function resolveMarkdownReferences(workspaceId: string, references: MarkdownReferenceRequest[]) {
  if (references.length === 0)
    return [];

  return invoke<ResolvedMarkdownReference[]>("resolve_markdown_references", {
    input: { references, workspaceId },
  });
}
