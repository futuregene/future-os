import type { FutureReference } from "./futureMarkdownTypes";
import { describe, expect, it, vi } from "vitest";

import { resolveFutureReferences } from "./resolveFutureReferences";

const resolveMock = vi.fn<(w: string, refs: unknown[]) => Promise<Array<Record<string, unknown>>>>();

vi.mock("../../integrations/storage/markdownReferences", () => ({
  resolveMarkdownReferences: (w: string, refs: unknown[]) => resolveMock(w, refs),
}));

function ref(targetType: FutureReference["targetType"], targetId: string): FutureReference {
  return { source: "inline", targetId, targetType, view: "chip" };
}

describe("resolveFutureReferences", () => {
  it("dedupes by identity and keys the result map by type:id", async () => {
    resolveMock.mockResolvedValue([
      { targetType: "run", targetId: "r1", status: "resolved" },
      { targetType: "file", targetId: "/a", status: "missing" },
    ]);
    const map = await resolveFutureReferences("w", [ref("run", "r1"), ref("run", "r1"), ref("file", "/a")]);
    expect(resolveMock).toHaveBeenCalledWith("w", [
      { targetType: "run", targetId: "r1" },
      { targetType: "file", targetId: "/a" },
    ]);
    expect(Object.keys(map).sort()).toEqual(["file:/a", "run:r1"]);
    expect(map["run:r1"]).toMatchObject({ status: "resolved" });
  });
});
