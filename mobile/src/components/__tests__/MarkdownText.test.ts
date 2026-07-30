import { blocksFromMarkdown } from "../MarkdownText";

describe("markdown block parser", () => {
  test("parses a GFM table with headers, rows and null aligns", () => {
    expect(blocksFromMarkdown("| A | B |\n|---|---|\n| 1 | 2 |")).toEqual([
      { kind: "table", headers: ["A", "B"], aligns: [null, null], rows: [["1", "2"]] },
    ]);
  });

  test("parses column alignments", () => {
    const [block] = blocksFromMarkdown("| L | C | R |\n|:--|:-:|--:|\n| a | b | c |");
    if (block?.kind !== "table") throw new Error("expected table");
    expect(block.aligns).toEqual(["left", "center", "right"]);
  });

  test("normalises row cell count to the header length", () => {
    const [block] = blocksFromMarkdown("| a | b |\n|---|---|\n| 1 | 2 | 3 |\n| x |");
    if (block?.kind !== "table") throw new Error("expected table");
    expect(block.rows).toEqual([
      ["1", "2"],
      ["x", ""],
    ]);
  });

  test("keeps an escaped pipe inside a cell", () => {
    const [block] = blocksFromMarkdown("| a \\| b | c |\n|---|---|\n| 1 | 2 |");
    if (block?.kind !== "table") throw new Error("expected table");
    expect(block.headers).toEqual(["a | b", "c"]);
  });

  test("does not let a paragraph swallow a table that has no blank line before it", () => {
    const blocks = blocksFromMarkdown("intro\n| h |\n|---|\n| x |");
    expect(blocks[0]).toEqual({ kind: "paragraph", text: "intro" });
    expect(blocks[1]?.kind).toBe("table");
  });

  test("a bare --- line is a rule, not a table separator", () => {
    expect(blocksFromMarkdown("---")).toEqual([{ kind: "rule" }]);
  });

  test("parses a blockquote's inner text", () => {
    expect(blocksFromMarkdown("> one\n> two")).toEqual([{ kind: "quote", text: "one\ntwo" }]);
  });

  test("parses task list items", () => {
    const [block] = blocksFromMarkdown("- [x] done\n- [ ] todo\n- plain");
    if (block?.kind !== "list") throw new Error("expected list");
    expect(block.ordered).toBe(false);
    expect(block.items).toEqual([
      { text: "done", checked: true },
      { text: "todo", checked: false },
      { text: "plain", checked: null },
    ]);
  });

  test("ordered lists never yield task checkboxes", () => {
    const [block] = blocksFromMarkdown("1. [x] not a task");
    if (block?.kind !== "list") throw new Error("expected list");
    expect(block.items).toEqual([{ text: "[x] not a task", checked: null }]);
  });

  test("strikethrough stays inline (not a block) for the renderer", () => {
    expect(blocksFromMarkdown("see ~~old~~ text")).toEqual([
      { kind: "paragraph", text: "see ~~old~~ text" },
    ]);
  });

  test("existing block types still parse", () => {
    const blocks = blocksFromMarkdown("# title\n\n```\ncode\n```\n\n- a\n- b\n\npara");
    expect(blocks.map(block => block.kind)).toEqual(["heading", "code", "list", "paragraph"]);
  });
});
