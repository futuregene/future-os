import { describe, expect, it } from "vitest";
import { isRecord, singleLine, truncate } from "./objects";

describe("isRecord", () => {
  it("accepts plain objects and rejects null/arrays/primitives", () => {
    expect(isRecord({ a: 1 })).toBe(true);
    expect(isRecord(null)).toBe(false);
    expect(isRecord([1])).toBe(false);
    expect(isRecord("x")).toBe(false);
  });
});

describe("singleLine", () => {
  it("collapses whitespace runs and trims", () => {
    expect(singleLine("  a\n b\t c  ")).toBe("a b c");
  });
});

describe("truncate", () => {
  it("truncates beyond max with an ellipsis", () => {
    expect(truncate("hello world", 5)).toBe("hello...");
  });

  it("returns compacted text within max unchanged", () => {
    expect(truncate("hi\nthere", 20)).toBe("hi there");
  });
});
