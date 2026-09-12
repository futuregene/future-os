import { describe, expect, it } from "vitest";
import { isThreadSearchQuery } from "./threadSearchQuery";

describe("thread search input threshold", () => {
  it.each(["", " ", "  ", "a", "a ", " a", " a ", "Z", "1", ".", "✅", "\n\t"])("does not search %j", (query) => {
    expect(isThreadSearchQuery(query)).toBe(false);
  });

  it.each(["ab", " ab ", "中 ", "12", "..", "a1", "中", "字", "123a", "搜索", "a-b", "中文123"])("accepts %j", (query) => {
    expect(isThreadSearchQuery(query)).toBe(true);
  });
});
