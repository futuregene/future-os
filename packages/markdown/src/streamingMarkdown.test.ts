import { describe, expect, it } from "vitest";
import { createStreamingMarkdownParser } from "./streamingMarkdown";
import { parseFutureMarkdown } from "./parseFutureMarkdown";

describe("createStreamingMarkdownParser", () => {
  it("uses the canonical parser (and its cache) for a settled segment", () => {
    const parser = createStreamingMarkdownParser();
    const document = parser("settled\n", false);
    expect(document).toBe(parseFutureMarkdown("settled\n"));
    expect(document.nodes).toEqual([{ children: [{ text: "settled", type: "text" }], type: "paragraph" }]);
  });

  it("keeps the frozen prefix object stable across an append", () => {
    const parser = createStreamingMarkdownParser();
    const first = parser("alpha\n\nbeta\n", true);
    const second = parser("alpha\n\nbeta\ncontinues\n", true);
    expect(first.nodes).toHaveLength(2);
    // Only the mutable suffix is re-parsed; the frozen block keeps identity.
    expect(second.nodes[0]).toBe(first.nodes[0]);
    expect(second.nodes[1]).toMatchObject({
      children: [{ text: "beta\ncontinues", type: "text" }],
      type: "paragraph",
    });
    expect(second.raw).toBe("alpha\n\nbeta\ncontinues\n");
  });

  it("returns the identical document while the text is unchanged", () => {
    const parser = createStreamingMarkdownParser();
    const first = parser("alpha\n\nbeta\n", true);
    expect(parser("alpha\n\nbeta\n", true)).toBe(first);
  });

  it("re-parses from scratch when the text is not an extension of the previous one", () => {
    const parser = createStreamingMarkdownParser();
    parser("alpha\n\nbeta\n", true);
    const replaced = parser("zulu\n", true);
    expect(replaced.nodes).toEqual([{ children: [{ text: "zulu", type: "text" }], type: "paragraph" }]);
    expect(replaced.raw).toBe("zulu\n");
  });

  it("returns an empty document for an empty segment", () => {
    const parser = createStreamingMarkdownParser();
    expect(parser("", true)).toEqual({ nodes: [], raw: "", references: [] });
  });

  it("merges a streamed table row incrementally", () => {
    const parser = createStreamingMarkdownParser();
    const header = "| a | b |\n| - | - |\n";
    const first = parser(`${header}| 1 | 2 |\n`, true);
    expect(first.nodes).toHaveLength(1);
    const merged = parser(`${header}| 1 | 2 |\n| 3 | 4 |\n`, true);
    expect(merged.nodes).toHaveLength(1);
    expect(merged.nodes[0]).toMatchObject({
      rows: [[[{ text: "1", type: "text" }], [{ text: "2", type: "text" }]], [[{ text: "3", type: "text" }], [{ text: "4", type: "text" }]]],
      type: "table",
    });
    // A third row reuses the same incremental path: only the last (still
    // mutable) row is replaced by the freshly parsed tail.
    const third = parser(`${header}| 1 | 2 |\n| 3 | 4 |\n| 5 | 6 |\n`, true);
    expect(third.nodes[0]).toMatchObject({
      rows: [
        [[{ text: "1", type: "text" }], [{ text: "2", type: "text" }]],
        [[{ text: "3", type: "text" }], [{ text: "4", type: "text" }]],
        [[{ text: "5", type: "text" }], [{ text: "6", type: "text" }]],
      ],
      type: "table",
    });
  });

  it("collects references from a merged table", () => {
    const parser = createStreamingMarkdownParser();
    const header = "| file |\n| - |\n";
    parser(`${header}| [a](docs/a.md) |\n`, true);
    const merged = parser(`${header}| [a](docs/a.md) |\n| [b](docs/b.md) |\n`, true);
    expect(merged.references.map((reference) => reference.targetId)).toEqual(["docs/a.md", "docs/b.md"]);
  });

  it("falls back to the generic append path when the table no longer parses as one", () => {
    const parser = createStreamingMarkdownParser();
    const header = "| a | b |\n| - | - |\n";
    parser(`${header}| 1 | 2 |\n`, true);
    const fallback = parser(`${header}| 1 | 2 |\n\ntrailing paragraph\n`, true);
    expect(fallback.nodes.map((node) => node.type)).toEqual(["table", "paragraph"]);
  });

  it("drops the incremental checkpoint when a definition appears in the suffix", () => {
    const parser = createStreamingMarkdownParser();
    parser("alpha\n\nbeta\n", true);
    const withDefinition = parser("alpha\n\nbeta\n\n[1]: docs/a.md\n", true);
    expect(withDefinition.references).toEqual([]);
    expect(withDefinition.nodes).toHaveLength(2);
    // The definition also invalidates the prefix on the next plain append.
    const after = parser("alpha\n\nbeta\n\n[1]: docs/a.md\n\ngamma\n", true);
    expect(after.nodes.map((node) => node.type)).toEqual(["paragraph", "paragraph", "paragraph"]);
  });

  it("drops the incremental checkpoint when the suffix nests past the guard", () => {
    const parser = createStreamingMarkdownParser();
    parser("alpha\n\nbeta\n", true);
    const deep = `${"> ".repeat(70)}deep\n`;
    const document = parser(`alpha\n\nbeta\n\n${deep}`, true);
    expect(document.nodes).toEqual([{ children: [{ text: `alpha\n\nbeta\n\n${deep}`, type: "text" }], type: "paragraph" }]);
  });

  it("recomputes references across the appended prefix", () => {
    const parser = createStreamingMarkdownParser();
    const first = parser("see [a.md]\n\nsecond\n", true);
    expect(first.references.map((reference) => reference.targetId)).toEqual(["a.md"]);
    const appended = parser("see [a.md]\n\nsecond\n\nthird\n", true);
    expect(appended.references.map((reference) => reference.targetId)).toEqual(["a.md"]);
    expect(appended.nodes).toHaveLength(3);
  });

  it("does not build a checkpoint for a definition-only segment", () => {
    const parser = createStreamingMarkdownParser();
    expect(parser("[1]: docs/a.md\n", true).nodes).toEqual([]);
    // Nothing was checkpointed, so the following append re-parses from zero.
    expect(parser("[1]: docs/a.md\n\nbody\n", true).nodes).toHaveLength(1);
  });
});
