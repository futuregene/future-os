import type {
  StoredApprovalRequest,
  StoredArtifact,
  StoredFile,
  StoredReviewChangeset,
  StoredRun,
  StoredToolCall,
} from "./types";
import { invokeCommand } from "../tauri/invoke";

export type FutureReferenceType = "approval" | "artifact" | "file" | "review" | "run" | "tool";

export interface MarkdownReferenceRequest {
  targetType: FutureReferenceType;
  targetId: string;
}

export interface ResolvedMarkdownReference {
  targetType: FutureReferenceType | string;
  targetId: string;
  status: "resolved" | "missing" | "forbidden" | string;
  data?: StoredApprovalRequest | StoredArtifact | StoredFile | StoredReviewChangeset | StoredRun | StoredToolCall | null;
  error?: string | null;
}

export async function resolveMarkdownReferences(workspaceId: string, references: MarkdownReferenceRequest[]) {
  if (references.length === 0)
    return [];

  return invokeCommand<ResolvedMarkdownReference[]>("resolve_markdown_references", {
    input: { references, workspaceId },
  });
}
