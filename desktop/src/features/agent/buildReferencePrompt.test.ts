import { beforeEach, describe, expect, it, vi } from "vitest";

import { buildReferenceContext } from "./buildReferencePrompt";

const resolveMock = vi.fn<(w: string, refs: unknown[]) => Promise<Array<Record<string, unknown>>>>();

vi.mock("../../integrations/storage/markdownReferences", () => ({
  resolveMarkdownReferences: (w: string, refs: unknown[]) => resolveMock(w, refs),
}));

beforeEach(() => {
  resolveMock.mockReset();
});

describe("buildReferenceContext", () => {
  it("returns no context when there are no references", async () => {
    await expect(buildReferenceContext("w", "plain text")).resolves.toBe("");
    expect(resolveMock).not.toHaveBeenCalled();
  });

  it("returns no context when resolution fails", async () => {
    resolveMock.mockRejectedValue(new Error("ipc"));
    await expect(buildReferenceContext("w", "[r](/abs/a.md)")).resolves.toBe("");
  });

  it("marks unresolved references as unavailable", async () => {
    resolveMock.mockResolvedValue([
      { targetType: "file", targetId: "/abs/a.md", status: "missing" },
    ]);
    const out = await buildReferenceContext("w", "[r](/abs/a.md)");
    expect(out).toContain("file:/abs/a.md - unavailable");
    expect(out).toContain("Referenced FutureOS objects");
  });

  it("marks resolutions without data as unavailable", async () => {
    resolveMock.mockResolvedValue([
      { targetType: "file", targetId: "/abs/a.md", status: "resolved", data: null },
    ]);
    const out = await buildReferenceContext("w", "[r](/abs/a.md)");
    expect(out).toContain("unavailable");
  });

  it("summarizes resolved file references with the default arm", async () => {
    resolveMock.mockResolvedValue([
      { targetType: "file", targetId: "/abs/a.md", status: "resolved", data: { path: "/abs/a.md", name: "a.md", insideWorkspace: false } },
    ]);
    const out = await buildReferenceContext("w", "[a](/abs/a.md) and [b](/abs/b.md)");
    expect(out).toContain("1. file:/abs/a.md");
    expect(out).toContain("2. file:/abs/b.md - unavailable");
    expect(out).toMatch(/^Referenced FutureOS objects/);
  });
});
