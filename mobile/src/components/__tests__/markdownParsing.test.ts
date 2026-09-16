import { createStreamingMarkdownParser, parseFutureMarkdown } from "@future-os/markdown";

describe("Markdown syntax fidelity", () => {
  test.each([
    ["迁移失败处理不合适：", "新工作区字段迁移被放进了可选集合。"],
    ["新旧端兼容未完善：", "新手机端连接旧桌面端时仍显示置顶入口。"],
    ["失效工作区校验不足：", "更新条件排除了已删除记录。"],
  ])("renders a CJK list label ending in punctuation: %s", (label, body) => {
    expect(parseFutureMarkdown(`- **${label}**${body}`).nodes).toEqual([{
      type: "list", ordered: false, start: undefined,
      items: [{ children: [
        { type: "strong", children: [{ type: "text", text: label }] },
        { type: "text", text: body },
      ], blocks: undefined, checked: undefined }],
    }]);
  });

  test.each([
    ["前**“重点”**后", "strong", "“重点”"],
    ["前*（重点）*后", "italic", "（重点）"],
    ["前**注意:**后", "strong", "注意:"],
    ["前**「重要」**です", "strong", "「重要」"],
    ["앞**중요：**뒤", "strong", "중요："],
  ])("supports punctuation at CJK emphasis boundaries: %s", (source, type, text) => {
    expect(parseFutureMarkdown(source).nodes[0]).toMatchObject({
      type: "paragraph", children: [
        { type: "text" }, { type, children: [{ type: "text", text }] }, { type: "text" },
      ],
    });
  });

  test("preserves escapes, code, destinations and ordinary delimiter rules", () => {
    expect(parseFutureMarkdown(String.raw`\*\*注意：\*\*正文`).nodes[0]).toMatchObject({
      children: [{ type: "text", text: "**注意：**正文" }],
    });
    expect(parseFutureMarkdown("`**注意：**正文`").nodes[0]).toMatchObject({
      children: [{ type: "code", code: "**注意：**正文" }],
    });
    expect(parseFutureMarkdown("```md\n**注意：**正文\n```").nodes[0]).toEqual({
      type: "code", language: "md", code: "**注意：**正文",
    });
    expect(parseFutureMarkdown("[链接](https://example.com/**注意：**正文)").nodes[0]).toMatchObject({
      children: [{ type: "link", href: "https://example.com/**注意：**正文" }],
    });
    for (const source of ["**Warning:**text", "foo_bar_baz", "** 注意：**正文", "**注意： **正文", "**注意：正文"]) {
      expect(parseFutureMarkdown(source).nodes[0]).toMatchObject({ children: [{ type: "text", text: source }] });
    }
  });
  test("settled parse cache evicts by charged bytes and bypasses oversized sources", () => {
    const source = "```\n" + "x".repeat(100000) + "\n```";
    const first = parseFutureMarkdown(source);
    expect(parseFutureMarkdown(source)).toBe(first);
    for (let i = 0; i < 30; i++) parseFutureMarkdown(`# ${i}\n\n${source}`);
    expect(parseFutureMarkdown(source)).not.toBe(first);
    const large = source + "x".repeat(40000);
    expect(parseFutureMarkdown(large)).not.toBe(parseFutureMarkdown(large));
  });
  test("collects images inside links and preserves rich local-file labels", () => {
    const document = parseFutureMarkdown("[![chart](assets/a.png)](https://example.com) [![local](assets/b.png)](./report.md) [**bold**](./report.md)");
    expect(document.references.map(ref => ref.targetId)).toEqual(["assets/a.png", "report.md", "assets/b.png", "report.md"]);
    expect(document.references.find(ref => ref.targetId === "report.md")?.label).toBe("local");
    expect(JSON.stringify(document.nodes)).toContain('"type":"image"');
    expect(JSON.stringify(document.nodes)).toContain('"type":"strong"');
  });

  test("preserves all six heading levels", () => {
    const document = parseFutureMarkdown(Array.from({ length: 6 }, (_, i) => `${"#".repeat(i + 1)} Heading`).join("\n\n"));
    expect(document.nodes.map(node => node.type === "heading" ? node.level : null)).toEqual([1, 2, 3, 4, 5, 6]);
  });

  test("preserves zero and non-one ordered starts, including nested lists", () => {
    expect(parseFutureMarkdown("0. zero\n1. one").nodes[0]).toMatchObject({ type: "list", start: 0 });
    expect(parseFutureMarkdown("9. nine\n\n   42. nested\n\n10. ten").nodes[0]).toMatchObject({
      type: "list", start: 9,
      items: [{ blocks: [{ type: "list", start: 42 }] }, {}],
    });
  });

  test("resolves definitions in containers, with the first definition winning", () => {
    const document = parseFutureMarkdown("[doc][ref]\n\n> [ref]: https://example.com/first\n\n[ref]: https://example.com/second");
    expect(document.nodes[0]).toMatchObject({ children: [{ type: "link", href: "https://example.com/first" }] });
  });

  test("supports safe HTML line breaks without enabling arbitrary HTML", () => {
    const document = parseFutureMarkdown("| A |\n|---|\n| first<br>second<BR />third |\n\n<script>alert(1)</script>");
    expect(document.nodes[0]).toMatchObject({ rows: [[[
      { type: "text", text: "first" }, { type: "break" },
      { type: "text", text: "second" }, { type: "break" }, { type: "text", text: "third" },
    ]]] });
    expect(document.nodes[1]).toMatchObject({ children: [{ type: "text", text: "<script>alert(1)</script>" }] });
    expect(parseFutureMarkdown("text<br onclick='bad()'>next").nodes[0]).toMatchObject({
      children: [{ type: "text", text: "text<br onclick='bad()'>next" }],
    });
  });

  test("keeps footnote definitions identifiable rather than anonymous quotes", () => {
    const document = parseFutureMarkdown("One[^a], two[^b].\n\n[^a]: Alpha\n\n[^b]: Beta");
    expect(JSON.stringify(document.nodes[1])).toContain("[^a]");
    expect(JSON.stringify(document.nodes[2])).toContain("[^b]");
  });

  test("link/image/task-list prefixes retain stable blocks and references", () => {
    const project = createStreamingMarkdownParser();
    const prefix = "[link](https://example.com) ![image](./plot.png)\n\n- [x] Done\n\n";
    const before = project(prefix + "tail", true);
    const after = project(prefix + "tail grows", true);
    expect(after.nodes[0]).toBe(before.nodes[0]);
    expect(after.nodes[1]).toBe(before.nodes[1]);
    expect(after).toEqual(parseFutureMarkdown(prefix + "tail grows"));
  });

  test.each([
    "- **迁移失败处理不合适：**新工作区。\n- **新旧端兼容未完善：**新手机端。\n\n下一段",
    "前**“重点”**后，前*（重点）*后。\n\n> **注意：**正文",
    "| 项目 | 说明 |\n|---|---|\n| **注意：**正文 | 前**「重要」**です |",
    "**注意：`code`**正文，**注意：[链接](https://example.com)**正文。",
    "前***“重点”***后，**注意：*重点*。**正文。",
    "[old][ref]\n\nmore\n\n> [ref]: https://example.com",
    "[old][ref]\n\nmore\n\n- [ref]: https://example.com",
    "[old][two words]\n\nmore\n\n[two\nwords]: ./target.md",
    "![img](./a.png)\n\n| A | B |\n|---|---|\n| [link](./b.md) | ![x](./c.png) |\n| next | [later][id] |\n\n[id]: ./d.md",
    "[link](https://example.com)\n\n- [x] done\n- [ ] pending\n\nnext",
    "# Heading\n\n## Other\n\nText **bold *nested* words** ~~old~~ `code`\\\nbreak\n\n---",
    "0. zero\n1. one\n   - [x] checked\n   - [ ] todo\n\n> quote\n>\n> paragraph",
    "| Left | Right |\n|:---|---:|\n| A<br>B | `x` |\n| missing |\n\nAfter",
    "[reference][r]\n\n> [r]: https://example.com\n\n![alt](https://example.com/a.png)",
    "Equation $x^2$ and \\(y_1\\).\n\n\\[\n\\frac{a}{b}\n\\]\n\n```ts\nconst a = 1;\n```",
    "note[^a]\n\n[^a]: body\n\n    more body",
  ])("every streaming prefix matches a fresh parse: %s", source => {
    const project = createStreamingMarkdownParser();
    for (let length = 0; length <= source.length; length++) {
      const prefix = source.slice(0, length);
      expect(project(prefix, true)).toEqual(parseFutureMarkdown(prefix));
    }
    expect(project(source, false)).toEqual(parseFutureMarkdown(source));
    expect(project("replacement", true)).toEqual(parseFutureMarkdown("replacement"));
  });
});
