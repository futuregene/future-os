import type { MarkdownNode } from "./futureMarkdownTypes";
import { describe, expect, it } from "vitest";
import { parseFutureMarkdown } from "./parseFutureMarkdown";

describe("parseFutureMarkdown", () => {
  it("does not treat futureos app-object links or fences as references (minimal link mode)", () => {
    const document = parseFutureMarkdown([
      "See [run:build](futureos://run/run_123?view=timeline).",
      "",
      "```futureos-artifact",
      "id: artifact_456",
      "view: card",
      "title: Build Report",
      "```",
    ].join("\n"));

    // App objects are disabled: nothing is collected as a future reference.
    expect(document.references).toEqual([]);

    // The inline link degrades to a plain link node...
    const paragraph = document.nodes[0];
    const link = paragraph?.type === "paragraph"
      ? paragraph.children.find(node => node.type === "link")
      : undefined;
    expect(link).toMatchObject({ href: "futureos://run/run_123?view=timeline", type: "link" });

    // ...and the fenced embed degrades to a plain code block.
    const code = findBlock(document.nodes, "code");
    expect(code?.type === "code" ? code.language : null).toBe("futureos-artifact");
  });

  it("turns a plain absolute-path link into a file reference", () => {
    const document = parseFutureMarkdown(
      "Wrote [test.txt](/Users/tao/app/test.txt).",
    );

    expect(document.references).toEqual([
      {
        label: "test.txt",
        source: "inline",
        targetId: "/Users/tao/app/test.txt",
        targetType: "file",
        view: "chip",
      },
    ]);
  });

  it("turns an angle-bracketed path (spaces) into a file reference", () => {
    const document = parseFutureMarkdown(
      "Wrote [note.txt](</Users/tao/My Docs/note.txt>).",
    );

    expect(document.references).toEqual([
      {
        label: "note.txt",
        source: "inline",
        targetId: "/Users/tao/My Docs/note.txt",
        targetType: "file",
        view: "chip",
      },
    ]);
  });

  it("turns a ./relative path link into a file reference, stripping ./", () => {
    const document = parseFutureMarkdown("Saved [poem.txt](./poem.txt).");

    expect(document.references).toEqual([
      {
        label: "poem.txt",
        source: "inline",
        targetId: "poem.txt",
        targetType: "file",
        view: "chip",
      },
    ]);
  });

  it("turns a bare [/abs/path] shortcut into an abbreviated file reference", () => {
    const document = parseFutureMarkdown("See [/Users/tao/Desktop/poem2.txt].");

    expect(document.references).toEqual([
      {
        label: "/Users/tao/Desktop/poem2.txt",
        source: "inline",
        targetId: "/Users/tao/Desktop/poem2.txt",
        targetType: "file",
        view: "chip",
      },
    ]);
  });

  it("leaves a non-file link (https) as an ordinary link, not a reference", () => {
    const document = parseFutureMarkdown("Docs at [site](https://example.com/page).");

    expect(document.references).toEqual([]);
  });

  it("resolves reference-style links and images through markdown definitions", () => {
    const document = parseFutureMarkdown([
      "[**Docs**][docs] and [artifact][artifact-ref]",
      "",
      "![Chart][chart]",
      "",
      "[docs]: https://example.com/docs",
      "[artifact-ref]: futureos://artifact/artifact_ref?view=summary",
      "[chart]: https://example.com/chart.png \"Chart Title\"",
    ].join("\n"));

    const paragraph = document.nodes[0];
    expect(paragraph?.type).toBe("paragraph");
    const paragraphChildren = paragraph?.type === "paragraph" ? paragraph.children : [];
    const links = paragraphChildren.filter(node => node.type === "link");

    expect(links[0]).toMatchObject({
      href: "https://example.com/docs",
      type: "link",
    });
    expect(links[0]?.type === "link" ? links[0].children[0] : null).toMatchObject({ type: "strong" });

    // Minimal link mode: the futureos:// reference-style link is no longer a
    // futureReference — it degrades to a plain link.
    expect(paragraphChildren.some(node => node.type === "futureReference")).toBe(false);
    expect(links[1]).toMatchObject({
      href: "futureos://artifact/artifact_ref?view=summary",
      type: "link",
    });

    const imageParagraph = document.nodes[1];
    expect(imageParagraph?.type).toBe("paragraph");
    expect(imageParagraph?.type === "paragraph" ? imageParagraph.children[0] : null).toMatchObject({
      alt: "Chart",
      src: "https://example.com/chart.png",
      title: "Chart Title",
      type: "image",
    });
  });

  it("preserves GFM tables, task lists, nested lists, and strikethrough", () => {
    const document = parseFutureMarkdown([
      "| Task | State |",
      "| :--- | ---: |",
      "| ~~old~~ | [tool](futureos://tool/tool_1) |",
      "",
      "- [x] Done",
      "  - Nested item",
    ].join("\n"));

    const table = findBlock(document.nodes, "table");
    expect(table?.type).toBe("table");
    expect(table?.type === "table" ? table.alignments : null).toEqual(["left", "right"]);
    const deletedCell = table?.type === "table" ? table.rows[0]?.[0]?.[0] : null;
    expect(deletedCell).toMatchObject({ type: "delete" });
    // Minimal link mode: the futureos:// tool link degrades to a plain link.
    const tableCell = table?.type === "table" ? table.rows[0]?.[1]?.[0] : null;
    expect(tableCell).toMatchObject({ href: "futureos://tool/tool_1", type: "link" });

    const list = findBlock(document.nodes, "list");
    expect(list?.type).toBe("list");
    const firstItem = list?.type === "list" ? list.items[0] : null;
    expect(firstItem?.checked).toBe(true);
    expect(firstItem?.blocks?.[0]?.type).toBe("list");
  });

  it("keeps raw HTML as safe text rather than renderable markup", () => {
    const document = parseFutureMarkdown("<script>alert(1)</script>");
    const paragraph = document.nodes[0];
    expect(paragraph?.type).toBe("paragraph");
    expect(paragraph?.type === "paragraph" ? paragraph.children : null).toEqual([
      { text: "<script>alert(1)</script>", type: "text" },
    ]);
  });
});

function findBlock<T extends MarkdownNode["type"]>(
  nodes: MarkdownNode[],
  type: T,
): Extract<MarkdownNode, { type: T }> | undefined {
  return nodes.find((node): node is Extract<MarkdownNode, { type: T }> => node.type === type);
}
