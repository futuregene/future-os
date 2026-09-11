import { describe, expect, it } from "vitest";
import { tokenizeJsonLine } from "../../../../packages/json-preview/src/index";
import { parseFutureMarkdown } from "../../../../packages/markdown/src/parseFutureMarkdown";

describe("unc markdown escaping", () => {
  it("preserves a correctly CommonMark-escaped UNC target", () => {
    const path = String.raw`\\server\share\a.md`;
    const escaped = path.split("\\").join("\\\\");
    expect(parseFutureMarkdown(`[log](${escaped})`).references[0]?.targetId).toBe(path);
  });
});

describe("untrusted markdown nesting", () => {
  it("preserves deeply nested source without overflowing conversion or renderers", () => {
    const raw = `${">".repeat(2000)} payload`;
    expect(parseFutureMarkdown(raw)).toEqual({
      raw,
      references: [],
      nodes: [{ type: "paragraph", children: [{ type: "text", text: raw }] }],
    });
  });

  it("keeps ordinary nested quotes structured", () => {
    expect(parseFutureMarkdown("> > text").nodes[0]?.type).toBe("blockquote");
  });
});

describe("json number token boundaries", () => {
  it("does not color numeric suffixes inside invalid JSON identifiers", () => {
    for (const text of ["error_404", "user2", "123abc", "abc123 xyz"]) {
      const tokens = tokenizeJsonLine(text);
      expect(tokens.every(token => token.kind === "plain")).toBe(true);
      expect(tokens.map(token => token.text).join("")).toBe(text);
    }
  });

  it("still recognizes JSON integers, fractions and exponents", () => {
    const tokens = tokenizeJsonLine("[0, -12, 3.5, 1e+4]");
    expect(tokens.filter(token => token.kind === "number").map(token => token.text)).toEqual(["0", "-12", "3.5", "1e+4"]);
  });
});
