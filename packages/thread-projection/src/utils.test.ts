import { describe, expect, it } from "vitest";
import { isRecord, pathBasename, pathExtension, singleLine, truncate } from "./utils";

describe("isRecord", () => {
  it.each<[unknown, boolean]>([
    [{}, true],
    [{ a: 1 }, true],
    [Object.create(null), true],
    [null, false],
    [undefined, false],
    [[], false],
    [[1, 2], false],
    ["text", false],
    [42, false],
    [true, false],
  ])("isRecord(%j) === %s", (value, expected) => {
    expect(isRecord(value)).toBe(expected);
  });
});

describe("singleLine", () => {
  it("collapses every whitespace run and trims", () => {
    expect(singleLine("  a \t b\n\nc  ")).toBe("a b c");
    expect(singleLine("no-change")).toBe("no-change");
    expect(singleLine("   ")).toBe("");
  });
});

describe("truncate", () => {
  it("returns the compacted value when it already fits", () => {
    expect(truncate("  a   b  ", 20)).toBe("a b");
    // Exactly the limit is not truncated.
    expect(truncate("abcde", 5)).toBe("abcde");
  });

  it("cut at the limit and appends an ellipsis one past it", () => {
    expect(truncate("abcdef", 5)).toBe("abcde...");
    expect(truncate("a\nb\nc", 3)).toBe("a b...");
  });
});

describe("pathBasename", () => {
  it.each([
    ["a/b/c.md", "c.md"],
    ["C:\\dir\\file.txt", "file.txt"],
    ["mixed/dir\\file", "file"],
    ["trailing/", "trailing"],
    ["single", "single"],
    ["/", ""],
    ["", ""],
  ])("pathBasename(%j) === %j", (path, expected) => {
    expect(pathBasename(path)).toBe(expected);
  });
});

describe("pathExtension", () => {
  it.each([
    ["a/b/c.MD", "md"],
    ["a.tar.gz", "gz"],
    // A leading-dot name has no extension, and a dot in a parent directory
    // never leaks into the result.
    [".bashrc", ""],
    ["dir.d/file", ""],
    ["noext", ""],
    ["trailing/", ""],
    ["weird.", ""],
  ])("pathExtension(%j) === %j", (path, expected) => {
    expect(pathExtension(path)).toBe(expected);
  });
});
