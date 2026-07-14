import type {
  StoredApprovalRequest,
  StoredArtifact,
  StoredFile,
  StoredReviewChangeset,
  StoredRun,
} from "./types";
import { isRecord } from "../../lib/objects";

/**
 * Runtime guards validating the storage payload contract: `resolve_markdown_references`
 * returns `data` as an untyped JSON blob, so each embed narrows it before use.
 * Kept next to `types.ts` so the guards move in lockstep with the interfaces
 * they check.
 */

export function isStoredArtifact(value: unknown): value is StoredArtifact {
  return isRecord(value)
    && typeof value.id === "string"
    && typeof value.workspaceId === "string"
    && typeof value.title === "string"
    && typeof value.artifactType === "string"
    && typeof value.createdAt === "number"
    && typeof value.updatedAt === "number";
}

export function isStoredFile(value: unknown): value is StoredFile {
  return isRecord(value)
    && typeof value.path === "string"
    && typeof value.name === "string"
    && typeof value.insideWorkspace === "boolean";
}

export function isStoredRun(value: unknown): value is StoredRun {
  return isRecord(value)
    && typeof value.id === "string"
    && typeof value.threadId === "string"
    && typeof value.status === "string"
    && typeof value.createdAt === "number"
    && typeof value.updatedAt === "number";
}

export function isStoredApproval(value: unknown): value is StoredApprovalRequest {
  return isRecord(value)
    && typeof value.id === "string"
    && typeof value.threadId === "string"
    && typeof value.kind === "string"
    && typeof value.status === "string"
    && typeof value.title === "string";
}

export function isStoredReview(value: unknown): value is StoredReviewChangeset {
  return isRecord(value)
    && typeof value.id === "string"
    && typeof value.threadId === "string"
    && typeof value.title === "string"
    && typeof value.status === "string"
    && typeof value.filesChanged === "number";
}
