import type { ReferenceTargetSearchResult, StoredArtifact, StoredResearchResource } from "./types";
import { invokeCommand } from "../tauri/invoke";

// ─── Artifacts ───────────────────────────────────────────────────────────

export async function listArtifacts(threadId: string) {
  return invokeCommand<StoredArtifact[]>("list_artifacts", { threadId });
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
  return invokeCommand<StoredArtifact>("create_artifact", { input });
}

export async function importAttachmentArtifact(input: { threadId: string; path: string }) {
  return invokeCommand<StoredArtifact>("import_attachment_artifact", { input });
}

export async function deleteArtifact(artifactId: string) {
  return invokeCommand<StoredArtifact>("delete_artifact", { artifactId });
}

// ─── Research ─────────────────────────────────────────────────────────────

export async function promoteArtifactToResearch(artifactId: string) {
  return invokeCommand<StoredResearchResource>("promote_artifact_to_research", { artifactId });
}

export async function listResearchResources(workspaceId: string) {
  return invokeCommand<StoredResearchResource[]>("list_research_resources", { workspaceId });
}

// ─── References ──────────────────────────────────────────────────────────

export async function searchReferenceTargets(input: {
  workspaceId: string;
  query?: string | null;
  limit?: number | null;
}) {
  return invokeCommand<ReferenceTargetSearchResult[]>("search_reference_targets", { input });
}
