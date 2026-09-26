import { describe, expect, it } from "vitest";
import { joinSoftBreaks } from "./softBreaks";
import type { FutureMarkdownDocument, InlineNode, MarkdownNode } from "./types";

function doc(nodes: MarkdownNode[]): FutureMarkdownDocument {
  return { nodes, raw: "", references: [] };
}

describe("joinSoftBreaks", () => {
  it("returns the same document when there is no soft break", () => {
    const input = doc([{ type: "paragraph", children: [{ type: "text", text: "no breaks" }] }]);
    expect(joinSoftBreaks(input)).toBe(input);
  });

  it("folds source newlines and surrounding indentation into one space", () => {
    const input = doc([
      { type: "paragraph", children: [{ type: "text", text: "wrapped at a column\n    continues here" }] },
    ]);
    expect(joinSoftBreaks(input).nodes).toEqual([
      { type: "paragraph", children: [{ type: "text", text: "wrapped at a column continues here" }] },
    ]);
    // Unchanged nodes keep identity so memoized renderers can bail out.
    expect(input.nodes[0]).not.toBe(joinSoftBreaks(input).nodes[0]);
  });

  it("does not rewrite a hard break node or other inline kinds", () => {
    const input = doc([
      {
        type: "paragraph",
        children: [
          { type: "text", text: "a" },
          { type: "break" },
          { type: "code", code: "x\n y" },
          { type: "image", alt: "", src: "https://example.com/a.png" },
          { type: "mathInline", code: "a\nb", displayMode: false },
        ],
      },
    ]);
    expect(joinSoftBreaks(input)).toBe(input);
  });

  it("reaches headings, blockquotes, list items and their sub-blocks", () => {
    const input = doc([
      { type: "heading", level: 2, children: [{ type: "text", text: "A\n B" }] },
      {
        type: "blockquote",
        children: [{ type: "paragraph", children: [{ type: "text", text: "q\n1" }] }],
      },
      {
        type: "list",
        ordered: true,
        start: 1,
        items: [
          {
            children: [{ type: "text", text: "item\none" }],
            blocks: [{ type: "paragraph", children: [{ type: "text", text: "sub\nblock" }] }],
          },
          { children: [{ type: "text", text: "plain" }] },
        ],
      },
    ]);
    const joined = joinSoftBreaks(input);
    expect(joined.nodes).toEqual([
      { type: "heading", level: 2, children: [{ type: "text", text: "A B" }] },
      {
        type: "blockquote",
        children: [{ type: "paragraph", children: [{ type: "text", text: "q 1" }] }],
      },
      {
        type: "list",
        ordered: true,
        start: 1,
        items: [
          {
            children: [{ type: "text", text: "item one" }],
            blocks: [{ type: "paragraph", children: [{ type: "text", text: "sub block" }] }],
          },
          { children: [{ type: "text", text: "plain" }] },
        ],
      },
    ]);
  });

  it("recurses through emphasis, links and reference children", () => {
    const input = doc([
      {
        type: "paragraph",
        children: [
          { type: "strong", children: [{ type: "text", text: "s\nt" }] },
          { type: "italic", children: [{ type: "text", text: "i\nj" }] },
          { type: "delete", children: [{ type: "text", text: "d\ne" }] },
          {
            type: "link",
            href: "https://example.com",
            children: [{ type: "text", text: "l\nk" }],
          },
          {
            type: "futureReference",
            reference: { source: "inline", targetId: "a.md", targetType: "file", view: "chip" },
            children: [{ type: "text", text: "r\nf" }],
          },
        ],
      },
    ]);
    expect(joinSoftBreaks(input).nodes).toEqual([
      {
        type: "paragraph",
        children: [
          { type: "strong", children: [{ type: "text", text: "s t" }] },
          { type: "italic", children: [{ type: "text", text: "i j" }] },
          { type: "delete", children: [{ type: "text", text: "d e" }] },
          {
            type: "link",
            href: "https://example.com",
            children: [{ type: "text", text: "l k" }],
          },
          {
            type: "futureReference",
            reference: { source: "inline", targetId: "a.md", targetType: "file", view: "chip" },
            children: [{ type: "text", text: "r f" }],
          },
        ],
      },
    ]);
  });

  it("keeps a childless reference node untouched", () => {
    const childless: InlineNode = {
      type: "futureReference",
      reference: { source: "inline", targetId: "a.md", targetType: "file", view: "chip" },
    };
    const input = doc([{ type: "paragraph", children: [childless] }]);
    expect(joinSoftBreaks(input)).toBe(input);
  });

  it("keeps document and node identity when no descendant needs a change", () => {
    // The unchanged arm of every container/emphasis ternary: a renderer that
    // memoizes on identity must be able to bail out of a whole subtree.
    const input = doc([
      {
        type: "blockquote",
        children: [{ type: "paragraph", children: [{ type: "text", text: "no break here" }] }],
      },
      {
        type: "list",
        ordered: false,
        items: [
          {
            children: [{ type: "text", text: "plain" }],
            blocks: [{ type: "paragraph", children: [{ type: "text", text: "sub" }] }],
          },
        ],
      },
      {
        type: "paragraph",
        children: [
          { type: "strong", children: [{ type: "text", text: "s" }] },
          { type: "italic", children: [{ type: "text", text: "i" }] },
          { type: "delete", children: [{ type: "text", text: "d" }] },
          { type: "link", href: "https://example.com", children: [{ type: "text", text: "l" }] },
          {
            type: "futureReference",
            reference: { source: "inline", targetId: "a.md", targetType: "file", view: "chip" },
            children: [{ type: "text", text: "r" }],
          },
        ],
      },
    ]);
    expect(joinSoftBreaks(input)).toBe(input);
  });

  it("passes non-inline blocks through unchanged", () => {
    const input = doc([
      { type: "code", code: "a\nb" },
      { type: "thematicBreak" },
      { type: "mathBlock", code: "x\ny" },
      {
        type: "futureEmbed",
        reference: { source: "block", targetId: "f.md", targetType: "file", view: "card" },
      },
      {
        type: "table",
        alignments: [null],
        headers: [[{ type: "text", text: "h\n1" }]],
        rows: [[[{ type: "text", text: "c\n2" }]]],
      },
    ]);
    expect(joinSoftBreaks(input)).toBe(input);
  });
});
