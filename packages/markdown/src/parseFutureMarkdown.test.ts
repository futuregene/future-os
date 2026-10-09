import type { Root } from "mdast";
import { describe, expect, it } from "vitest";
import {
  collectReferences,
  exceedsNestingLimit,
  parseFutureMarkdown,
  parseMdast,
} from "./parseFutureMarkdown";
import type { InlineNode, MarkdownNode } from "./types";

const text = (value: string): InlineNode => ({ text: value, type: "text" });
const nodeTypes = (nodes: MarkdownNode[]) => nodes.map((node) => node.type);
const parse = (source: string) => parseFutureMarkdown(source, undefined, false);
const first = (source: string) => parse(source).nodes[0]!;

describe("parseFutureMarkdown — block nodes", () => {
  it.each<[string, MarkdownNode["type"]]>([
    ["# heading\n", "heading"],
    ["Title\n=====\n", "heading"],
    ["---\n", "thematicBreak"],
    ["plain paragraph\n", "paragraph"],
    ["> quoted\n", "blockquote"],
    ["```ts\nconst a = 1;\n```\n", "code"],
    ["    indented code\n", "code"],
    ["| a |\n| - |\n| 1 |\n", "table"],
    ["- item\n", "list"],
    ["1. item\n", "list"],
  ])("maps %j to a %s node", (source, expected) => {
    expect(nodeTypes(parse(source).nodes)).toEqual([expected]);
  });

  it("carries heading levels 1 through 6", () => {
    for (let level = 1; level <= 6; level += 1) {
      const heading = first(`${"#".repeat(level)} title\n`);
      expect(heading).toMatchObject({ children: [text("title")], level, type: "heading" });
    }
  });

  it("keeps a code block's language and drops it when absent", () => {
    expect(first("```ts\nconst a = 1;\n```\n")).toEqual({
      code: "const a = 1;",
      language: "ts",
      type: "code",
    });
    expect(first("```\nplain\n```\n")).toEqual({ code: "plain", type: "code" });
  });

  it("turns block-level HTML into a rendered block and drops the definition", () => {
    expect(first("<div>\n  hi\n</div>\n")).toEqual({
      children: [text("hi")],
      type: "paragraph",
    });
    // A link definition contributes no node of its own.
    expect(parse("[x][1]\n\n[1]: https://example.com\n").nodes).toHaveLength(1);
  });

  it.each<[string, "center" | "left" | "right"]>([
    ['<p align="center">hello</p>\n', "center"],
    ['<p align="left">hello</p>\n', "left"],
    ['<p align="right">hello</p>\n', "right"],
  ])('applies a whitelisted <p align="%s">', (source, align) => {
    expect(first(source)).toEqual({
      align,
      children: [text("hello")],
      type: "paragraph",
    });
  });

  it("renders a whitelisted heading with alignment", () => {
    expect(first('<h3 align="center">Title</h3>\n')).toEqual({
      align: "center",
      children: [text("Title")],
      level: 3,
      type: "heading",
    });
  });

  it("reads an <img> width and folds it into a centered paragraph when the wrapper is centered", () => {
    expect(first('<img src="docs/a.png" width="120">\n')).toEqual({
      children: [{ alt: "", src: "docs/a.png", type: "image", width: 120 }],
      type: "paragraph",
    });
    expect(first('<div align="center">\n<img src="docs/a.png">\n</div>\n')).toEqual({
      align: "center",
      children: [{ alt: "", src: "docs/a.png", type: "image" }],
      type: "paragraph",
    });
  });

  it("keeps a <br> as a break, flattens a <details> summary and leaves a script inert", () => {
    expect(first("line1<br>line2\n")).toEqual({
      children: [text("line1"), { type: "break" }, text("line2")],
      type: "paragraph",
    });
    // The summary survives as its own line; the body (a separate markdown
    // block) is untouched; the closing </details> yields nothing.
    expect(parse('<details><summary>more</summary>\n\ninner body\n\n</details>\n').nodes).toEqual([
      { children: [text("more")], type: "paragraph" },
      { children: [text("inner body")], type: "paragraph" },
    ]);
    // Non-whitelisted markup is kept verbatim as inert text (readable, never
    // executed), not restructured.
    expect(first("<script>alert(1)</script>\n")).toEqual({
      children: [text("<script>alert(1)</script>")],
      type: "paragraph",
    });
  });

  it("keeps blockquotes nested", () => {
    const quote = first("> outer\n>\n> > inner\n");
    expect(quote.type).toBe("blockquote");
    expect(nodeTypes((quote as { children: MarkdownNode[] }).children)).toEqual([
      "paragraph",
      "blockquote",
    ]);
  });
});

describe("parseFutureMarkdown — lists", () => {
  it("keeps ordered start numbers and task state", () => {
    expect(first("3. three\n")).toMatchObject({ items: [{ children: [text("three")] }], ordered: true, start: 3 });
    expect(first("- [x] done\n- [ ] open\n")).toMatchObject({
      items: [
        { checked: true, children: [text("done")] },
        { checked: false, children: [text("open")] },
      ],
      ordered: false,
    });
  });

  it("keeps `start` undefined for unordered lists", () => {
    const list = first("- a\n");
    expect((list as { start?: number }).start).toBeUndefined();
  });

  it("splits a multi-block item into first-paragraph children plus blocks", () => {
    expect(first("- item\n\n  second paragraph\n\n- simple\n")).toMatchObject({
      items: [
        {
          blocks: [{ children: [text("second paragraph")], type: "paragraph" }],
          children: [text("item")],
        },
        { children: [text("simple")] },
      ],
    });
    // An item whose first block is not a paragraph has no inline children.
    expect(first("- > quoted\n")).toMatchObject({
      items: [
        {
          blocks: [{ children: [{ children: [text("quoted")], type: "paragraph" }], type: "blockquote" }],
          children: [],
        },
      ],
    });
    expect(first("- \n")).toMatchObject({ items: [{ children: [] }] });
  });
});

describe("parseFutureMarkdown — tables", () => {
  it("reads alignments, headers and body rows", () => {
    const table = first("| a | b | c | d |\n| :- | -: | :-: | - |\n| 1 | 2 | 3 | 4 |\n");
    expect(table).toEqual({
      alignments: ["left", "right", "center", null],
      headers: [[text("a")], [text("b")], [text("c")], [text("d")]],
      rows: [[[text("1")], [text("2")], [text("3")], [text("4")]]],
      type: "table",
    });
  });

  it("pads a short row and truncates an over-long one from a supplied tree", () => {
    // remark-gfm always pads/truncates rows to the header width, so the
    // defensive reshape in `tableRowToCells` is only reachable through the
    // public `parsedTree` parameter (worker-supplied trees).
    const cell = (value: string) => ({ children: [{ type: "text", value }], type: "tableCell" });
    const tree = {
      type: "root",
      children: [
        {
          type: "table",
          align: [null, "left"],
          children: [
            { type: "tableRow", children: [cell("h1"), cell("h2")] },
            { type: "tableRow", children: [cell("only")] },
            { type: "tableRow", children: [cell("a"), cell("b"), cell("c")] },
          ],
        },
      ],
    } as unknown as Root;

    expect(parseFutureMarkdown("<ragged>", tree, false).nodes).toEqual([
      {
        alignments: [null, "left"],
        headers: [[text("h1")], [text("h2")]],
        rows: [[[text("only")], []], [[text("a")], [text("b")]]],
        type: "table",
      },
    ]);
  });

  it("falls back to null alignments and no headers on a degenerate tree", () => {
    // remark-gfm always emits `align`, and a table always has a header row, so
    // these defensive arms are only reachable through the `parsedTree` API.
    const noAlign = {
      type: "root",
      children: [
        {
          type: "table",
          align: null,
          children: [
            {
              type: "tableRow",
              children: [{ children: [{ type: "text", value: "h" }], type: "tableCell" }],
            },
          ],
        },
      ],
    } as unknown as Root;
    expect(parseFutureMarkdown("<no-align>", noAlign, false).nodes).toEqual([
      { alignments: [null], headers: [[text("h")]], rows: [], type: "table" },
    ]);

    const empty = {
      type: "root",
      children: [{ type: "table", children: [] }],
    } as unknown as Root;
    expect(parseFutureMarkdown("<empty-table>", empty, false).nodes).toEqual([
      { alignments: [], headers: [], rows: [], type: "table" },
    ]);
  });
});

describe("parseFutureMarkdown — futureos-file embeds", () => {
  it.each([
    ["chip", "chip"],
    ["diff-summary", "diff-summary"],
    ["output-summary", "output-summary"],
    ["timeline", "timeline"],
    ["summary", "summary"],
    ["unknown", "card"],
    [undefined, "card"],
  ])("normalizes view %j to %s", (view, expected) => {
    const directive = [`id: docs/a.md`, `title: Notes`, ...(view ? [`view: ${view}`] : [])].join("\n");
    const node = first(`\`\`\`futureos-file\n${directive}\n\`\`\`\n`);
    expect(node).toEqual({
      reference: {
        label: "Notes",
        source: "block",
        targetId: "docs/a.md",
        targetType: "file",
        view: expected,
      },
      type: "futureEmbed",
    });
  });

  it("ignores directive lines that cannot be fields and trims keys/values", () => {
    const node = first("```futureos-file\nno-colon-here\n: value-without-key\n   : orphaned-value\n   id  :   docs/a.md   \n\n```\n");
    expect(node).toEqual({
      reference: expect.objectContaining({ targetId: "docs/a.md" }),
      type: "futureEmbed",
    });
  });

  it("falls back to a code block when the id is missing or the language is not a file embed", () => {
    expect(first("```futureos-file\nview: chip\n```\n")).toMatchObject({
      code: "view: chip",
      language: "futureos-file",
      type: "code",
    });
    expect(first("```futureos-artifact\nid: 1\n```\n")).toMatchObject({ type: "code" });
  });
});

describe("parseFutureMarkdown — footnotes", () => {
  it("renders a reference as text and the definition as a labelled blockquote", () => {
    expect(parse("note[^1]\n\n[^1]: body text\n").nodes).toEqual([
      { children: [text("note[^1]")], type: "paragraph" },
      {
        children: [{ children: [text("[^1]: "), text("body text")], type: "paragraph" }],
        type: "blockquote",
      },
    ]);
  });

  it("inserts a label paragraph when the definition does not start with one", () => {
    const tree = {
      type: "root",
      children: [
        {
          type: "footnoteDefinition",
          identifier: "a",
          children: [{ type: "code", value: "block\n" }],
        },
      ],
    } as unknown as Root;
    expect(parseFutureMarkdown("<footnote-code>", tree, false).nodes).toEqual([
      {
        children: [
          { children: [text("[^a]: ")], type: "paragraph" },
          { code: "block\n", type: "code" },
        ],
        type: "blockquote",
      },
    ]);
  });
});

describe("parseFutureMarkdown — math", () => {
  it.each([
    ["\\(x+1\\)\n", "x+1"],
    ["\\[x+1\\]\n", "x+1"],
    ["$$x+1$$\n", "x+1"],
  ])("promotes a lone formula %j to a math block", (source, code) => {
    expect(parse(source).nodes).toEqual([{ code, type: "mathBlock" }]);
  });

  it("keeps an inline formula inline when it shares a paragraph", () => {
    expect(first("before $a+b$ after\n")).toMatchObject({
      children: [text("before "), { code: "a+b", displayMode: false, type: "mathInline" }, text(" after")],
      type: "paragraph",
    });
    expect(first("before \\(a+b\\) after\n")).toMatchObject({
      children: [text("before "), { code: "a+b", displayMode: false, type: "mathInline" }, text(" after")],
    });
  });

  it("leaves an unmatched inline delimiter as ordinary (backslash-escaped) text", () => {
    expect(first("\\(unclosed\n")).toMatchObject({ children: [text("(unclosed")] });
  });

  it("keeps a multi-line display block as one node and trims it", () => {
    expect(parse("\\[\n  a + b\n\\]\n").nodes).toEqual([{ code: "a + b", type: "mathBlock" }]);
  });

  it("keeps a newline inside an inline formula", () => {
    expect(first("text \\(a\nb\\) rest\n")).toEqual({
      children: [
        text("text "),
        { code: "a\nb", displayMode: false, type: "mathInline" },
        text(" rest"),
      ],
      type: "paragraph",
    });
  });

  it("treats an inner backslash that does not close the formula as data", () => {
    expect(first("text \\(a\\b\\) rest\n")).toMatchObject({
      children: [text("text "), { code: "a\\b", type: "mathInline" }, text(" rest")],
    });
    expect(first("\\[a\\b\\]\n")).toEqual({ code: "a\\b", type: "mathBlock" });
  });

  it("allows spaces between a display block's closing delimiter and its line end", () => {
    expect(first("\\[a\\]  \n")).toEqual({ code: "a", type: "mathBlock" });
    // A non-space character after the delimiter keeps the token unclosed, so
    // the rest of the line is data again.
    expect(first("\\[a\\]  x\n")).toEqual({ code: "a\\]  x", type: "mathBlock" });
    expect(first("\\[a\\]x\n")).toEqual({ code: "a\\]x", type: "mathBlock" });
  });

  it("interrupts a paragraph when display math starts a line", () => {
    expect(parse("para\n\\[x\\]\n").nodes).toEqual([
      { children: [text("para")], type: "paragraph" },
      { code: "x", type: "mathBlock" },
    ]);
  });
});

describe("parseFutureMarkdown — $$ display blocks", () => {
  it("keeps the opening line when the fence shares it with the formula", () => {
    // The shape models write: `$$` on the first formula line, the closing `$$`
    // right after the last one. remark-math reads the first line as fence meta
    // and never matches a closing fence, so the block used to swallow every
    // node after it (a following table came back as literal `|` rows).
    const nodes = parse("$$a = b\n= c + d$$\n\n| L |\n| - |\n| 1 |\n").nodes;
    expect(nodes[0]).toEqual({ code: "a = b\n= c + d", type: "mathBlock" });
    expect(nodeTypes(nodes.slice(1))).toEqual(["table"]);
  });

  it("keeps consecutive blocks apart and inside their containers", () => {
    expect(parse("$$a\nb$$\n\n$$c\nd$$\n").nodes).toEqual([
      { code: "a\nb", type: "mathBlock" },
      { code: "c\nd", type: "mathBlock" },
    ]);
    expect(first("> $$a\n> b$$\n")).toEqual({
      children: [{ code: "a\nb", type: "mathBlock" }],
      type: "blockquote",
    });
  });

  it("drops the fence's indentation from the formula", () => {
    expect(parse("  $$\n  a + b\n  $$\n").nodes).toEqual([{ code: "a + b", type: "mathBlock" }]);
    // An indented whitespace-only line is indentation, not formula text.
    expect(parse("  $$\n  a\n  \n  b\n  $$\n").nodes).toEqual([{ code: "a\n\nb", type: "mathBlock" }]);
    // With an unindented fence there is no indentation to strip, so the
    // continuation line's own indentation is left to LaTeX (it ignores it).
    expect(parse("$$\n  a + b\n$$\n").nodes).toEqual([{ code: "  a + b", type: "mathBlock" }]);
  });

  it("keeps a lone dollar inside the formula", () => {
    expect(first("$$a $ b$$\n")).toEqual({ code: "a $ b", type: "mathBlock" });
  });

  it("leaves `$$…$$` to inline math when text follows the closing run", () => {
    // `$$x$$ and more` is not a display block: the fence must end its line.
    expect(first("$$x$$ and more\n")).toEqual({
      children: [{ code: "x", displayMode: false, type: "mathInline" }, text(" and more")],
      type: "paragraph",
    });
  });

  it("keeps a lone dollar at a line start inline", () => {
    expect(first("$x$ alone\n")).toEqual({
      children: [{ code: "x", displayMode: false, type: "mathInline" }, text(" alone")],
      type: "paragraph",
    });
  });

  it("keeps a blank line inside the formula", () => {
    expect(parse("$$\na\n\nb$$\n\nafter\n").nodes).toEqual([
      { code: "a\n\nb", type: "mathBlock" },
      { children: [text("after")], type: "paragraph" },
    ]);
  });

  it("keeps an unterminated fence as a block when the reply ends mid-formula", () => {
    // A streamed reply ends wherever the model stopped, with no closing fence
    // and no trailing newline; what has arrived so far is still display math.
    expect(parse("$$a\nb").nodes).toEqual([{ code: "a\nb", type: "mathBlock" }]);
    expect(parse("$$a").nodes).toEqual([{ code: "a", type: "mathBlock" }]);
  });
});

describe("parseFutureMarkdown — nesting guard", () => {
  function chain(depth: number) {
    let node: { type: string; children?: unknown[] } = { type: "leaf" };
    for (let index = 0; index < depth; index += 1) node = { type: "wrapper", children: [node] };
    return { type: "root", children: [node] };
  }

  it("accepts a tree whose deepest node sits at depth 64 and rejects 65", () => {
    // chain(n) puts its leaf at depth n + 1, so 63 wrappers is the deepest
    // still-accepted tree and 64 trips the guard.
    expect(exceedsNestingLimit(chain(63))).toBe(false);
    expect(exceedsNestingLimit(chain(64))).toBe(true);
  });

  it("keeps the whole source as text instead of dropping the deep tail", () => {
    const source = `${"> ".repeat(66)}deep\n`;
    expect(parse(source).nodes).toEqual([{ children: [text(source)], type: "paragraph" }]);
  });
});

describe("parseFutureMarkdown — inline nodes", () => {
  it("maps every inline kind", () => {
    const source =
      "~~del~~ *em* **strong** `code`  \nbreak ![alt](https://x.com/a.png) [t](https://x.com) [l](docs/a.md) [b.md] [^n] <br> <span>raw</span>\n";
    const paragraph = first(source) as { children: InlineNode[] };
    expect(paragraph.children).toEqual([
      { children: [text("del")], type: "delete" },
      text(" "),
      { children: [text("em")], type: "italic" },
      text(" "),
      { children: [text("strong")], type: "strong" },
      text(" "),
      { code: "code", type: "code" },
      // The two trailing spaces are the hard break, so no text node separates
      // the code span from it.
      { type: "break" },
      text("break "),
      { alt: "alt", src: "https://x.com/a.png", type: "image" },
      text(" "),
      { children: [text("t")], href: "https://x.com", type: "link" },
      text(" "),
      {
        children: [text("l")],
        reference: { label: "l", source: "inline", targetId: "docs/a.md", targetType: "file", view: "chip" },
        type: "futureReference",
      },
      text(" "),
      {
        reference: { label: "b.md", source: "inline", targetId: "b.md", targetType: "file", view: "chip" },
        type: "futureReference",
      },
      // The footnote reference is plain text and merges with its neighbours.
      text(" [^n] "),
      { type: "break" },
      text(" <span>raw</span>"),
    ]);
  });

  it("compact-merges adjacent text produced by HTML comments and footnote refs", () => {
    expect(first("<!--c-->x[^1]y\n\n[^1]: n\n")).toEqual({
      children: [text("x[^1]y")],
      type: "paragraph",
    });
  });

  it("resolves reference links and images against definitions, including local paths", () => {
    expect(parse("[ref][1] ![alt][1]\n\n[1]: https://example.com/a.png \"T\"\n").nodes).toEqual([
      {
        children: [
          { children: [text("ref")], href: "https://example.com/a.png", type: "link" },
          text(" "),
          { alt: "alt", src: "https://example.com/a.png", title: "T", type: "image" },
        ],
        type: "paragraph",
      },
    ]);
    expect(parse("[l][1]\n\n[1]: docs/a.md\n").nodes).toEqual([
      {
        children: [
          {
            children: [text("l")],
            reference: { label: "l", source: "inline", targetId: "docs/a.md", targetType: "file", view: "chip" },
            type: "futureReference",
          },
        ],
        type: "paragraph",
      },
    ]);
  });

  it("normalizes an identifier's case and inner whitespace", () => {
    expect(parse("[x][MiXeD   Ref]\n\n[mixed ref]: docs/a.md\n").nodes[0]).toMatchObject({
      children: [{ reference: { targetId: "docs/a.md" }, type: "futureReference" }],
    });
  });

  it("keeps the first definition when an identifier is defined twice", () => {
    // CommonMark resolves a duplicate identifier to the first definition in
    // source order; the later one must not override it.
    expect(parse("[l][1]\n\n[1]: docs/a.md\n\n[1]: docs/b.md\n").nodes[0]).toMatchObject({
      children: [{ reference: { targetId: "docs/a.md" }, type: "futureReference" }],
    });
  });

  it("keeps a disabled futureos:// scheme link inert", () => {
    expect(first("[x](futureos://artifact/1)\n")).toEqual({
      children: [
        { children: [text("x")], href: "futureos://artifact/1", type: "link" },
      ],
      type: "paragraph",
    });
  });

  it("leaves a link with an unparsable destination inert", () => {
    expect(first("[x](not-a-url)\n")).toMatchObject({
      children: [{ children: [text("x")], href: "not-a-url", type: "link" }],
    });
  });

  it("builds a link label from children that carry no text of their own", () => {
    // The label flattens a hard break to nothing, so a local destination still
    // resolves to a file reference labelled `ab`.
    expect(first("[a  \nb](docs/ab.md)\n")).toMatchObject({
      children: [
        {
          children: [{ text: "a", type: "text" }, { type: "break" }, { text: "b", type: "text" }],
          reference: { label: "ab", targetId: "docs/ab.md", targetType: "file", view: "chip" },
          type: "futureReference",
        },
      ],
    });
  });

  it("keeps a link label image's alt text when the image is the label", () => {
    // `mdastText` has a dedicated arm for image nodes; a label that *is* an
    // image must use its alt text rather than an empty string.
    expect(first("[![alt](https://x.com/a.png)](docs/a.md)\n")).toMatchObject({
      children: [
        {
          children: [{ alt: "alt", src: "https://x.com/a.png", type: "image" }],
          reference: { label: "alt", targetId: "docs/a.md", targetType: "file", view: "chip" },
          type: "futureReference",
        },
      ],
    });
  });

  it("only promotes bracketed mentions that name a path", () => {
    expect(first("see [notes.md] and [plain text]\n")).toMatchObject({
      children: [
        text("see "),
        {
          reference: { label: "notes.md", source: "inline", targetId: "notes.md", targetType: "file", view: "chip" },
          type: "futureReference",
        },
        text(" and [plain text]"),
      ],
    });
  });
});

describe("parseFutureMarkdown — autolink boundaries and CJK emphasis", () => {
  it("cuts a bare URL at CJK punctuation without swallowing the sentence", () => {
    expect(first("see https://x.com/a。next\n")).toEqual({
      children: [
        text("see "),
        { children: [text("https://x.com/a")], href: "https://x.com/a", type: "link" },
        text("。next"),
      ],
      type: "paragraph",
    });
  });

  it("leaves the closing emphasis markers outside the URL", () => {
    expect(first("**https://x.com/b** (x)\n")).toEqual({
      children: [
        { children: [{ children: [text("https://x.com/b")], href: "https://x.com/b", type: "link" }], type: "strong" },
        text(" (x)"),
      ],
      type: "paragraph",
    });
  });

  it("gives a URL glued to a CJK letter its own node", () => {
    expect(first("网页https://y.com/z\n")).toEqual({
      children: [
        text("网页"),
        { children: [text("https://y.com/z")], href: "https://y.com/z", type: "link" },
      ],
      type: "paragraph",
    });
  });

  it("does not end a URL at a single asterisk", () => {
    // Only a `**` run terminates a bare URL (it is the emphasis that opened
    // before it); a lone `*` is a valid URL character. A CJK letter does not
    // terminate it either, so the target keeps running to the node's end.
    expect(first("见 https://x.com/a*下\n")).toEqual({
      children: [
        text("见 "),
        { children: [text("https://x.com/a*下")], href: "https://x.com/a*下", type: "link" },
      ],
      type: "paragraph",
    });
  });

  it("always echoes the original source as `raw` (round-trip invariant)", () => {
    // `raw` is what streaming offsets and the incremental projector slice on, so
    // it must be the byte-identical input for every shape — including the
    // delimiter/punctuation juxtapositions the CJK emphasis plugin rewrites the
    // flanking rules for.
    for (const source of [
      "甲**：乙**丙\n",
      "甲**:乙**丙\n",
      "汉字**，逗号**后\n",
      "**注意：**正文\n",
      "汉字**。**\n",
      "甲*：乙*丙\n",
      "甲**）、乙**\n",
      "甲**、乙**丙\n",
      "復現：**10 轮**\n",
      "甲**（乙**）\n",
      "見**「引号」**後\n",
      "甲**\n",
    ]) {
      expect(parseFutureMarkdown(source, undefined, false).raw).toBe(source);
    }
  });

  it("closes CJK emphasis across full-width punctuation", () => {
    expect(first("**注意：**正文 and 復現：**10 轮**\n")).toEqual({
      children: [
        { children: [text("注意：")], type: "strong" },
        text("正文 and 復現："),
        { children: [text("10 轮")], type: "strong" },
      ],
      type: "paragraph",
    });
  });

  it("opens an emphasis run that is glued to CJK text and full-width punctuation", () => {
    // The `_open` arm: a delimiter run whose outside neighbour is CJK and whose
    // inside neighbour is punctuation (`甲**：乙**丙`) — CommonMark's own rule
    // would leave the closing run unpaired and print the asterisks literally.
    expect(first("甲**：乙**丙\n")).toEqual({
      children: [
        text("甲"),
        { children: [text("：乙")], type: "strong" },
        text("丙"),
      ],
      type: "paragraph",
    });
  });

  it("closes an emphasis run after full-width punctuation", () => {
    // The `_close` arm: punctuation (CJK punctuation included) before the run
    // forces the closing delimiter so the emphasis pairs up instead of printing
    // its asterisks literally.
    expect(first("见「**上**说\n")).toEqual({
      children: [
        text("见「"),
        { children: [text("上")], type: "strong" },
        text("说"),
      ],
      type: "paragraph",
    });
  });

  it("keeps emoji and CJK text intact while still finding a bracketed path mention", () => {
    expect(first("🙂 [长诗.md] 👨👩👧\n")).toEqual({
      children: [
        text("🙂 "),
        {
          reference: { label: "长诗.md", source: "inline", targetId: "长诗.md", targetType: "file", view: "chip" },
          type: "futureReference",
        },
        text(" 👨👩👧"),
      ],
      type: "paragraph",
    });
  });
});

describe("parseFutureMarkdown — links with an unclosed angle destination", () => {
  it("recovers `](<path)` as a file chip instead of leaking the destination", () => {
    // The model drops the `>` far more often than not: without a repair the
    // bracketed label becomes a chip and the rest renders literally, so the
    // reader sees the path twice — `bench/REPORT.md(<./bench/REPORT.md)`.
    expect(first("结论：[bench/REPORT.md](<./bench/REPORT.md)（脚本：[out.py](<./out.py)）\n"))
      .toEqual({
        children: [
          text("结论："),
          {
            reference: { label: "bench/REPORT.md", source: "inline", targetId: "bench/REPORT.md", targetType: "file", view: "chip" },
            children: [text("bench/REPORT.md")],
            type: "futureReference",
          },
          text("（脚本："),
          {
            reference: { label: "out.py", source: "inline", targetId: "out.py", targetType: "file", view: "chip" },
            children: [text("out.py")],
            type: "futureReference",
          },
          text("）"),
        ],
        type: "paragraph",
      });
  });

  it("leaves a well-formed angle destination alone", () => {
    expect(first("[bench/REPORT.md](<./bench/REPORT.md>)\n")).toEqual({
      children: [{
        reference: { label: "bench/REPORT.md", source: "inline", targetId: "bench/REPORT.md", targetType: "file", view: "chip" },
        children: [text("bench/REPORT.md")],
        type: "futureReference",
      }],
      type: "paragraph",
    });
    // The repair only manufactures a file link; an ordinary destination parses
    // through the normal link path.
    expect(first("[site](<https://x.com/a>)\n")).toEqual({
      children: [{ children: [text("site")], href: "https://x.com/a", type: "link" }],
      type: "paragraph",
    });
  });

  it("does not reach across a closing parenthesis or a line break", () => {
    // The destination cannot contain `)`, `>` or whitespace, so a stray `(<`
    // that is not a broken link stays literal text.
    expect(first("a (<not a link) b\n")).toEqual({
      children: [text("a (<not a link) b")],
      type: "paragraph",
    });
  });
});

describe("collectReferences", () => {
  it("collects embeds and inline references in document order across containers", () => {
    const document = parse(
      [
        "```futureos-file",
        "id: block.md",
        "```",
        "",
        "| [c](docs/c.md) | ![d](docs/d.png) |",
        "| - | - |",
        "| **[e](docs/e.md)** | [f](https://x.com) |",
        "",
        "> - [g](docs/g.md)",
        "",
        "## [h.md]",
        "",
      ].join("\n"),
    );
    expect(document.references).toEqual([
      { label: undefined, source: "block", targetId: "block.md", targetType: "file", view: "card" },
      { label: "c", source: "inline", targetId: "docs/c.md", targetType: "file", view: "chip" },
      { label: "d", source: "inline", targetId: "docs/d.png", targetType: "file", view: "chip" },
      { label: "e", source: "inline", targetId: "docs/e.md", targetType: "file", view: "chip" },
      { label: "g", source: "inline", targetId: "docs/g.md", targetType: "file", view: "chip" },
      { label: "h.md", source: "inline", targetId: "h.md", targetType: "file", view: "chip" },
    ]);
  });

  it("walks reference children and skips remote images", () => {
    const nodes: MarkdownNode[] = [
      { type: "thematicBreak" },
      {
        type: "paragraph",
        children: [
          { alt: "remote", src: "https://x.com/a.png", type: "image" },
          { alt: "local", src: "docs/l.png", type: "image" },
        ],
      },
      {
        type: "table",
        alignments: [null],
        headers: [[{ alt: "", src: "h.png", type: "image" }]],
        rows: [[[{ type: "break" }]]],
      },
      {
        type: "list",
        ordered: false,
        items: [
          {
            children: [
              {
                children: [
                  {
                    children: [text("inner")],
                    reference: { source: "inline", targetId: "docs/r.md", targetType: "file", view: "chip" },
                    type: "futureReference",
                  },
                ],
                href: "https://x.com",
                type: "link",
              },
            ],
            blocks: [{ children: [{ alt: "b", src: "docs/b.png", type: "image" }], type: "paragraph" }],
          },
        ],
      },
    ];
    expect(collectReferences(nodes).map((reference) => reference.targetId)).toEqual([
      "docs/l.png",
      "h.png",
      "docs/r.md",
      "docs/b.png",
    ]);
  });
});

describe("parseFutureMarkdown — parse cache", () => {
  it("returns the identical cached document for a repeated source", () => {
    const source = "cached paragraph\n";
    expect(parseFutureMarkdown(source)).toBe(parseFutureMarkdown(source));
  });

  it("bypasses the cache when asked (streaming fragments)", () => {
    const source = "uncached paragraph\n";
    expect(parseFutureMarkdown(source, undefined, false)).not.toBe(parseFutureMarkdown(source, undefined, false));
  });

  it("does not cache a source larger than the cache's source limit", () => {
    const big = `${"x".repeat(129 * 1024)}\n`;
    expect(parseFutureMarkdown(big, undefined, true)).not.toBe(parseFutureMarkdown(big, undefined, true));
    // Two uncached parses of a 132 KB source: generous budget so CPU contention
    // from other work on the machine cannot turn this into a false failure.
  }, 60_000);

  it("refuses to cache a document whose charged size exceeds the byte budget", () => {
    // Probed candidate shapes: `"a\n\n"` repeated is the densest AST per source
    // character (~65 charged bytes/char). 43 690 repetitions stay inside the
    // 128 KiB source limit while charging ~8.56 MB — past the 8 MiB cache
    // budget — so the document is returned but not retained.
    const source = "a\n\n".repeat(43_690);
    expect(source.length).toBeLessThanOrEqual(128 * 1024);
    const oversized = parseFutureMarkdown(source);
    expect(oversized.raw).toBe(source);
    expect(oversized.nodes).toHaveLength(43_690);

    // The oversized entry must not poison the accounting: an ordinary document
    // parsed afterwards is still cached and served by identity.
    const small = parseFutureMarkdown("small after oversized\n");
    expect(parseFutureMarkdown("small after oversized\n")).toBe(small);
  }, 60_000);

  it("evicts the least-recently-used entry once the cache is full", () => {
    const firstDocument = parseFutureMarkdown("eviction-0\n");
    for (let index = 1; index <= 600; index += 1) parseFutureMarkdown(`eviction-${index}\n`);
    expect(parseFutureMarkdown("eviction-0\n")).not.toBe(firstDocument);
    // 600 parses — budget for load spikes, not for a tighter wall clock.
  }, 60_000);

  it("keeps a re-touched entry when the cache pressure comes later", () => {
    const firstDocument = parseFutureMarkdown("touch-0\n");
    // A hit re-inserts at the newest position, so filling the cache afterwards
    // evicts the untouched entries first.
    expect(parseFutureMarkdown("touch-0\n")).toBe(firstDocument);
    for (let index = 1; index <= 511; index += 1) parseFutureMarkdown(`touch-${index}\n`);
    expect(parseFutureMarkdown("touch-0\n")).toBe(firstDocument);
  }, 60_000);
});

describe("parseMdast", () => {
  it("returns the mdast tree the converter consumes", () => {
    const tree = parseMdast("para$1$\n");
    expect(tree.type).toBe("root");
    expect(tree.children[0]).toMatchObject({ type: "paragraph" });
  });
});

describe("parseFutureMarkdown — worker-supplied trees", () => {
  const cell = (value: string) => ({ children: [{ type: "text", value }], type: "tableCell" });

  it("defends against optional fields a worker-supplied tree may omit", () => {
    // `parsedTree` is a public entry point (workers reuse whole trees or block
    // subtrees), so the converter must tolerate shapes remark itself never emits:
    // a missing `align`, a null alt/title, an ordered list without `start`, and
    // resolved/unresolved reference nodes.
    const tree = {
      type: "root",
      children: [
        {
          type: "table",
          align: null,
          children: [{ type: "tableRow", children: [cell("h")] }],
        },
        // Definitions are document-wide and produce no node of their own.
        { type: "definition", identifier: "img", label: "img", url: "img.png" },
        {
          type: "paragraph",
          children: [
            { type: "image", alt: null, title: null, url: "docs/a.png" },
            { type: "text", value: " " },
            { type: "imageReference", identifier: "img", alt: null, label: "img", title: null },
            { type: "text", value: " " },
            { type: "imageReference", identifier: "missing", alt: null, label: "missing" },
            {
              type: "link",
              url: "docs/b.md",
              title: null,
              children: [{ type: "image", alt: null, title: null, url: "x.png" }],
            },
          ],
        },
        {
          type: "list",
          ordered: true,
          start: null,
          children: [
            { type: "listItem", children: [{ type: "paragraph", children: [{ type: "text", value: "x" }] }] },
          ],
        },
      ],
    } as unknown as Root;

    const nodes = parseFutureMarkdown("<worker-tree>", tree, false).nodes;
    expect(nodes).toMatchObject([
      { alignments: [null], headers: [[text("h")]], rows: [], type: "table" },
      {
        children: [
          // A missing alt falls back to an empty string, a missing title to undefined.
          { alt: "", src: "docs/a.png", title: undefined, type: "image" },
          text(" "),
          // The reference resolves against the definition above.
          { alt: "", src: "img.png", title: undefined, type: "image" },
          // An unresolved reference degrades to its (empty) label text and is
          // then merged with the whitespace around it.
          text(" "),
          // A link whose label is an alt-less image: the label text is empty and
          // the destination is still a local file reference.
          {
            children: [{ alt: "", src: "x.png", title: undefined, type: "image" }],
            reference: { label: "", source: "inline", targetId: "docs/b.md", targetType: "file", view: "chip" },
            type: "futureReference",
          },
        ],
        type: "paragraph",
      },
      {
        items: [{ children: [text("x")] }],
        ordered: true,
        // `start` is optional in the wire/mdast types; an ordered list without
        // one starts at 1.
        start: 1,
        type: "list",
      },
    ]);
  });
});
