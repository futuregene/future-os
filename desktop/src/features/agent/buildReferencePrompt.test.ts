import { beforeEach, describe, expect, it, vi } from "vitest";

const resolveMock = vi.fn<(w: string, refs: unknown[]) => Promise<Array<Record<string, unknown>>>>();

vi.mock("../../integrations/storage/markdownReferences", () => ({
  resolveMarkdownReferences: (w: string, refs: unknown[]) => resolveMock(w, refs),
}));

import { buildReferencePrompt } from "./buildReferencePrompt";

beforeEach(() => {
  resolveMock.mockReset();
});

describe("buildReferencePrompt", () => {
  it("returns the prompt unchanged when there are no references", async () => {
    await expect(buildReferencePrompt("w", "plain text", "do it")).resolves.toBe("do it");
    expect(resolveMock).not.toHaveBeenCalled();
  });

  it("returns the prompt unchanged when resolution fails", async () => {
    resolveMock.mockRejectedValue(new Error("ipc"));
    await expect(buildReferencePrompt("w", "[r](/abs/a.md)", "do it")).resolves.toBe("do it");
  });

  it("marks unresolved references as unavailable", async () => {
    resolveMock.mockResolvedValue([
      { targetType: "file", targetId: "/abs/a.md", status: "missing" },
    ]);
    const out = await buildReferencePrompt("w", "[r](/abs/a.md)", "do it");
    expect(out).toContain("file:/abs/a.md - unavailable");
    expect(out).toContain("Referenced FutureOS objects");
  });

  it("marks resolutions without data as unavailable", async () => {
    resolveMock.mockResolvedValue([
      { targetType: "file", targetId: "/abs/a.md", status: "resolved", data: null },
    ]);
    const out = await buildReferencePrompt("w", "[r](/abs/a.md)", "do it");
    expect(out).toContain("unavailable");
  });

  it("summarizes resolved file references with the default arm", async () => {
    resolveMock.mockResolvedValue([
      { targetType: "file", targetId: "/abs/a.md", status: "resolved", data: { path: "/abs/a.md", name: "a.md", insideWorkspace: false } },
    ]);
    const out = await buildReferencePrompt("w", "[a](/abs/a.md) and [b](/abs/b.md)", "do it");
    expect(out).toContain("1. file:/abs/a.md");
    expect(out).toContain("2. file:/abs/b.md - unavailable");
    expect(out).toMatch(/^do it\n\nReferenced FutureOS objects/);
  });
});
