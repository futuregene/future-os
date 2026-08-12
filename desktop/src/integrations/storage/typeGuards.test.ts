import { describe, expect, it } from "vitest";
import {
  isStoredApproval,
  isStoredArtifact,
  isStoredFile,
  isStoredReview,
  isStoredRun,
} from "./typeGuards";

describe("storage type guards", () => {
  it("isStoredArtifact", () => {
    expect(isStoredArtifact({
      id: "a",
      workspaceId: "w",
      title: "t",
      artifactType: "report",
      createdAt: 1,
      updatedAt: 2,
    })).toBe(true);
    expect(isStoredArtifact({ id: "a" })).toBe(false);
    expect(isStoredArtifact("nope")).toBe(false);
  });

  it("isStoredFile", () => {
    expect(isStoredFile({ path: "/p", name: "n", insideWorkspace: true })).toBe(true);
    expect(isStoredFile({ path: "/p", name: "n" })).toBe(false);
  });

  it("isStoredRun", () => {
    expect(isStoredRun({ id: "r", threadId: "t", status: "running", createdAt: 1, updatedAt: 2 })).toBe(true);
    expect(isStoredRun({ id: "r", threadId: "t" })).toBe(false);
  });

  it("isStoredApproval", () => {
    expect(isStoredApproval({ id: "a", threadId: "t", kind: "k", status: "pending", title: "x" })).toBe(true);
    expect(isStoredApproval({ id: "a", threadId: "t" })).toBe(false);
  });

  it("isStoredReview", () => {
    expect(isStoredReview({ id: "r", threadId: "t", title: "x", status: "open", filesChanged: 3 })).toBe(true);
    expect(isStoredReview({ id: "r", threadId: "t" })).toBe(false);
  });
});
