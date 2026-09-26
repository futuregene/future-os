import { describe, expect, it } from "vitest";
import { referenceKey } from "./types";

describe("referenceKey", () => {
  it("joins target type and id so the same id under two types stays distinct", () => {
    expect(referenceKey({ targetId: "docs/a.md", targetType: "file" })).toBe("file:docs/a.md");
    expect(referenceKey({ targetId: "docs/a.md", targetType: "artifact" })).toBe("artifact:docs/a.md");
    expect(referenceKey({ targetId: "docs/a.md", targetType: "file" }))
      .not.toBe(referenceKey({ targetId: "docs/a.md", targetType: "artifact" }));
  });

  it("is stable for equal inputs (usable as a Map/Set key)", () => {
    const keys = new Set([
      referenceKey({ targetId: "1", targetType: "run" }),
      referenceKey({ targetId: "1", targetType: "run" }),
    ]);
    expect(keys.size).toBe(1);
  });
});
