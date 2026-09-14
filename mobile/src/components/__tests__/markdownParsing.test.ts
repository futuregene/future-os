import { createStreamingMarkdownParser, parseFutureMarkdown } from "@future-os/markdown";

describe("Markdown syntax fidelity", () => {
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

  test.each([
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
