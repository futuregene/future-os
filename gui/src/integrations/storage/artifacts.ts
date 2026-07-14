import type { StoredArtifact, WorkspaceFileResult } from "./types";
import { invokeCommand } from "../tauri/invoke";

// ─── Artifacts ───────────────────────────────────────────────────────────

export async function listArtifacts(threadId: string) {
  return invokeCommand<StoredArtifact[]>("list_artifacts", { threadId });
}

export async function importAttachmentArtifact(input: { threadId: string; path: string }) {
  return invokeCommand<StoredArtifact>("import_attachment_artifact", { input });
}

export async function deleteArtifact(artifactId: string) {
  return invokeCommand<StoredArtifact>("delete_artifact", { artifactId });
}

// ─── Workspace file `@`-mention search ─────────────────────────────────────

export async function searchWorkspaceFiles(input: {
  workspaceId: string;
  query?: string | null;
  limit?: number | null;
}) {
  return invokeCommand<WorkspaceFileResult[]>("search_workspace_files", { input });
}
