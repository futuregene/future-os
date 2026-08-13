import type { FutureMarkdownDocument, MarkdownNode } from "./futureMarkdownTypes";
import { describe, expect, it } from "vitest";
import { parseFutureMarkdown } from "./parseFutureMarkdown";

function nodeAt(doc: FutureMarkdownDocument, index: number): MarkdownNode {
  const node = doc.nodes[index];
  if (!node)
    throw new Error(`no node at index ${index}`);
  return node;
}

function paragraphAt(doc: FutureMarkdownDocument, index: number) {
  const node = nodeAt(doc, index);
  if (node.type !== "paragraph")
    throw new Error(`node ${index} is ${node.type}, not paragraph`);
  return node;
}

function types(nodes: MarkdownNode[]): string[] {
  return nodes.map(n => n.type);
}

describe("parseFutureMarkdown block constructs", () => {
  it("converts blockquotes, headings, breaks and thematic breaks", () => {
    const doc = parseFutureMarkdown("> quoted *text*\n\n## H2\n\n#### H4\n\nhard  \nbreak\n\n---");
    expect(types(doc.nodes)).toEqual(["blockquote", "heading", "heading", "paragraph", "thematicBreak"]);
    const quote = nodeAt(doc, 0);
    expect(quote.type === "blockquote" && quote.children[0]?.type).toBe("paragraph");
    const h2 = nodeAt(doc, 1);
    expect(h2.type === "heading" && h2.level).toBe(2);
    // Headings clamp to level 3.
    const h4 = nodeAt(doc, 2);
    expect(h4.type === "heading" && h4.level).toBe(3);
    // Hard break inside the paragraph.
    const para = paragraphAt(doc, 3);
    expect(para.children.some(c => c.type === "break")).toBe(true);
  });

  it("converts h1 headings", () => {
    const doc = parseFutureMarkdown("# Top");
    const h1 = nodeAt(doc, 0);
    expect(h1.type === "heading" && h1.level).toBe(1);
  });

  it("converts footnotes, inline html, code and strikethrough", () => {
    const doc = parseFutureMarkdown("note[^a] and <b>html</b> and `code` and ~~gone~~\n\n[^a]: footnote body");
    const para = paragraphAt(doc, 0);
    const kinds = para.children.map(c => c.type);
    expect(kinds).toContain("code");
    expect(kinds).toContain("delete");
    // Footnote reference renders as its literal marker.
    expect(para.children.some(c => c.type === "text" && c.text.includes("[^a]"))).toBe(true);
    // The footnote definition becomes a blockquote block.
    expect(types(doc.nodes)).toContain("blockquote");
  });

  it("converts inline images and reference-style images", () => {
    const doc = parseFutureMarkdown("![alt](https://x/y.png \"title\")\n\n![refalt][img]\n\n[img]: https://x/z.png");
    expect(paragraphAt(doc, 0).children[0]).toMatchObject({
      type: "image",
      alt: "alt",
      src: "https://x/y.png",
      title: "title",
    });
    expect(paragraphAt(doc, 1).children[0]).toMatchObject({
      type: "image",
      alt: "refalt",
      src: "https://x/z.png",
    });
  });

  it("renders unresolved image references as literal text", () => {
    const doc = parseFutureMarkdown("![alt][missing]");
    expect(paragraphAt(doc, 0).children[0]?.type).toBe("text");
  });

  it("renders unresolved link references as literal text", () => {
    // remark leaves a reference link with no definition as literal source text.
    const doc = parseFutureMarkdown("[label][missing]");
    expect(paragraphAt(doc, 0).children[0]).toMatchObject({ type: "text", text: "[label][missing]" });
  });

  it("turns reference-style links with local-path definitions into file references", () => {
    const doc = parseFutureMarkdown("[doc][d]\n\n[d]: /abs/file.md");
    expect(paragraphAt(doc, 0).children[0]).toMatchObject({
      type: "futureReference",
      reference: { targetType: "file", targetId: "/abs/file.md" },
    });
    expect(doc.references).toHaveLength(1);
  });

  it("keeps definition-only blocks out of the output", () => {
    const doc = parseFutureMarkdown("[d]: /abs/file.md");
    expect(doc.nodes).toHaveLength(0);
  });

  it("converts lists with task states and nested blocks", () => {
    const doc = parseFutureMarkdown("1. first\n2. second\n\n- [ ] todo\n- [x] done\n- item\n\n  nested paragraph");
    const ordered = nodeAt(doc, 0);
    expect(ordered.type === "list" && ordered.ordered).toBe(true);
    // Consecutive `-` lists merge into one list node.
    const tasks = nodeAt(doc, 1);
    if (tasks.type !== "list")
      throw new Error("expected a list");
    expect(tasks.items.map(i => i.checked)).toEqual([false, true, undefined]);
    expect(tasks.items[2]?.blocks?.[0]?.type).toBe("paragraph");
  });

  it("pads or truncates table rows to the header width", () => {
    const doc = parseFutureMarkdown("| a | b |\n| --- | --- |\n| 1 |\n| 1 | 2 | 3 |");
    const table = nodeAt(doc, 0);
    if (table.type !== "table")
      throw new Error("expected a table");
    expect(table.rows[0]).toHaveLength(2);
    expect(table.rows[1]).toHaveLength(2);
  });

  it("renders html blocks as safe text paragraphs", () => {
    const doc = parseFutureMarkdown("<div>\n\ntext");
    expect(types(doc.nodes)).toEqual(["paragraph", "paragraph"]);
    expect(paragraphAt(doc, 0).children[0]).toMatchObject({ text: "<div>" });
  });

  it("recognizes futureos-file block embeds with view normalization", () => {
    const doc = parseFutureMarkdown("```futureos-file\nid: /abs/a.md\ntitle: A\nview: summary\n```");
    const embed = nodeAt(doc, 0);
    expect(embed).toMatchObject({
      type: "futureEmbed",
      reference: { targetType: "file", targetId: "/abs/a.md", label: "A", view: "summary" },
    });
    expect(doc.references).toHaveLength(1);
  });

  it("normalizes every embed view variant and defaults to card", () => {
    for (const view of ["chip", "diff-summary", "output-summary", "timeline", "summary"]) {
      const doc = parseFutureMarkdown(`\`\`\`futureos-file\nid: /a.md\nview: ${view}\n\`\`\``);
      const embed = nodeAt(doc, 0);
      expect(embed.type === "futureEmbed" && embed.reference.view).toBe(view);
    }
    const unknown = parseFutureMarkdown("```futureos-file\nid: /a.md\nview: bogus\n```");
    const unknownEmbed = nodeAt(unknown, 0);
    expect(unknownEmbed.type === "futureEmbed" && unknownEmbed.reference.view).toBe("card");
    const missing = parseFutureMarkdown("```futureos-file\nid: /a.md\n```");
    const missingView = nodeAt(missing, 0);
    expect(missingView.type === "futureEmbed" && missingView.reference.view).toBe("card");
  });

  it("falls back to a plain code block when the embed has no id", () => {
    const doc = parseFutureMarkdown("```futureos-file\ntitle: no id here\n```");
    expect(nodeAt(doc, 0).type).toBe("code");
  });

  it("skips directive lines without a key separator", () => {
    const doc = parseFutureMarkdown("```futureos-file\nnoseparator\n: leading\nid: /a.md\n```");
    const embed = nodeAt(doc, 0);
    expect(embed.type === "futureEmbed" && embed.reference.targetId).toBe("/a.md");
  });

  it("collects references from blockquotes, lists, tables and formatted spans", () => {
    const doc = parseFutureMarkdown([
      "> [/q.md]",
      "",
      "- [/l.md]",
      "",
      "| [/h.md] |",
      "| --- |",
      "| [/c.md] |",
      "",
      "**[/s.md]** and *[/i.md]* and ~~[/d.md]~~",
    ].join("\n"));
    const ids = doc.references.map(r => r.targetId);
    expect(ids).toEqual(expect.arrayContaining(["/q.md", "/l.md", "/h.md", "/c.md", "/s.md", "/i.md", "/d.md"]));
  });

  it("merges adjacent text nodes after inline conversions", () => {
    // Inline html converts to a text node adjacent to the surrounding text —
    // compaction merges them into one.
    const doc = parseFutureMarkdown("before <b>bold</b> after");
    const texts = paragraphAt(doc, 0).children.filter(c => c.type === "text");
    expect(texts).toHaveLength(1);
    expect(texts[0]).toMatchObject({ text: "before <b>bold</b> after" });
  });

  it("serves repeat parses from the LRU cache", () => {
    const first = parseFutureMarkdown("cached body");
    const second = parseFutureMarkdown("cached body");
    expect(second).toBe(first);
  });

  it("evicts the oldest parse past the cache cap", () => {
    for (let i = 0; i < 520; i += 1) {
      parseFutureMarkdown(`unique body ${i}`);
    }
    // Still works after eviction churn.
    expect(parseFutureMarkdown("unique body 0").raw).toBe("unique body 0");
  });

  it("keeps futureos:// links inert (minimal link mode) even with bad encodings", () => {
    const doc = parseFutureMarkdown("[x](futureos://run/%E0%A4%A)");
    expect(paragraphAt(doc, 0).children[0]?.type).toBe("link");
  });
});
