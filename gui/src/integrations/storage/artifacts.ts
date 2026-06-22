import type { ReferenceTargetSearchResult, StoredArtifact, StoredResearchResource } from "./types";
import { invoke } from "@tauri-apps/api/core";

// ─── Artifacts ───────────────────────────────────────────────────────────

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

export async function importAttachmentArtifact(input: { threadId: string; path: string }) {
  return invoke<StoredArtifact>("import_attachment_artifact", { input });
}

export async function deleteArtifact(artifactId: string) {
  return invoke<StoredArtifact>("delete_artifact", { artifactId });
}

// ─── Research ─────────────────────────────────────────────────────────────

export async function promoteArtifactToResearch(artifactId: string) {
  return invoke<StoredResearchResource>("promote_artifact_to_research", { artifactId });
}

export async function listResearchResources(workspaceId: string) {
  return invoke<StoredResearchResource[]>("list_research_resources", { workspaceId });
}

// ─── References ──────────────────────────────────────────────────────────

export async function searchReferenceTargets(input: {
  workspaceId: string;
  query?: string | null;
  limit?: number | null;
}) {
  return invoke<ReferenceTargetSearchResult[]>("search_reference_targets", { input });
}
