import { describe, expect, it } from "vitest";
import { parseFutureMarkdown } from "./parseFutureMarkdown";
import { splitStreamingMarkdown } from "./streamingMarkdownBlocks";

const inline = (code: string) => ({ code, displayMode: false, type: "mathInline" });
const block = (code: string) => ({ code, type: "mathBlock" });

describe("latex math delimiters", () => {
  it("parses parentheses in prose and emphasis without interpreting TeX as Markdown", () => {
    expect(parseFutureMarkdown(String.raw`答案是 **\(\boxed{21}\)**，\(a_b * c\)。`).nodes).toEqual([
      { type: "paragraph", children: [
        { type: "text", text: "答案是 " },
        { type: "strong", children: [inline(String.raw`\boxed{21}`)] },
        { type: "text", text: "，" },
        inline("a_b * c"),
        { type: "text", text: "。" },
      ] },
    ]);
  });

  it.each(["\n", "\r\n"])("parses display equations with %j line endings and blank lines", (eol) => {
    const code = [String.raw`\begin{aligned}`, String.raw`a&=b\\[2pt]`, "", String.raw`c&=d`, String.raw`\end{aligned}`].join(eol);
    const nodes = parseFutureMarkdown(["Before", String.raw`\[`, code, String.raw`\]`, "After"].join(eol)).nodes;
    expect(nodes.map(node => node.type)).toEqual(["paragraph", "mathBlock", "paragraph"]);
    expect(nodes[1]).toEqual(block(code));
  });

  it("supports single-line bracket blocks and existing dollar delimiters", () => {
    expect(parseFutureMarkdown(String.raw`\[\boxed{21}\]`).nodes).toEqual([block(String.raw`\boxed{21}`)]);
    expect(parseFutureMarkdown("$$\nx^2\n$$").nodes).toEqual([block("x^2")]);
    expect(parseFutureMarkdown("$x^2$").nodes).toEqual([block("x^2")]);
  });

  it("supports formulas in list items, blockquotes and table cells", () => {
    expect(parseFutureMarkdown(String.raw`- Value \(a+b\)`).nodes).toMatchObject([
      { type: "list", items: [{ children: [{ type: "text", text: "Value " }, inline("a+b")] }] },
    ]);
    expect(parseFutureMarkdown("- Formula\n\n  \\[\n  a+b\n  \\]").nodes).toMatchObject([
      { type: "list", items: [{ blocks: [block("a+b")] }] },
    ]);
    expect(parseFutureMarkdown(["> \\[", "> x^2", "> \\]", "", "outside"].join("\n")).nodes).toMatchObject([
      { type: "blockquote", children: [block("x^2")] },
      { type: "paragraph" },
    ]);
    expect(parseFutureMarkdown("| Formula |\n| --- |\n| \\(a_b\\) |").nodes).toMatchObject([
      { type: "table", rows: [[[inline("a_b")]]] },
    ]);
  });

  it("leaves inline, fenced and indented code unchanged", () => {
    const code = String.raw`\(x\) \[y\]`;
    expect(parseFutureMarkdown(`\`${code}\``).nodes).toEqual([
      { type: "paragraph", children: [{ type: "code", code }] },
    ]);
    for (const source of [`\`\`\`tex\n${code}\n\`\`\``, `~~~tex\n${code}\n~~~`, `    ${code}`]) {
      expect(parseFutureMarkdown(source).nodes).toMatchObject([{ type: "code", code }]);
    }
  });

  it("does not reinterpret escaped delimiters, link targets, HTML or dollar math", () => {
    const escaped = parseFutureMarkdown(String.raw`\\(x\\) \\[y\\]`).nodes;
    // Existing bracket/file-reference parsing is independent of math syntax.
    expect(JSON.stringify(escaped)).not.toMatch(/mathInline|mathBlock/);
    expect(parseFutureMarkdown(String.raw`[link](https://example.com/\(x\))`).nodes).toMatchObject([
      { type: "paragraph", children: [{ type: "link", href: "https://example.com/(x)" }] },
    ]);
    const html = String.raw`<div>\(x\)</div>`;
    expect(parseFutureMarkdown(html).nodes).toEqual([{ type: "paragraph", children: [{ type: "text", text: html }] }]);
    expect(parseFutureMarkdown(String.raw`$\text{\(literal\)}$`).nodes).toEqual([block(String.raw`\text{\(literal\)}`)]);
  });

  it("preserves escaped backslashes within formulas", () => {
    expect(parseFutureMarkdown(String.raw`Value \(a\\) + b\) end`).nodes).toMatchObject([
      { type: "paragraph", children: [
        { type: "text", text: "Value " },
        inline(String.raw`a\\) + b`),
        { type: "text", text: " end" },
      ] },
    ]);
  });

  it("falls back for unmatched inline delimiters and accepts an unfinished display block", () => {
    expect(parseFutureMarkdown(String.raw`Before \(x+1`).nodes).toEqual([
      { type: "paragraph", children: [{ type: "text", text: "Before (x+1" }] },
    ]);
    expect(parseFutureMarkdown("\\[\nx+1").nodes).toEqual([block("x+1")]);
    expect(parseFutureMarkdown("> \\[\n> x+1\noutside").nodes).toMatchObject([
      { type: "blockquote", children: [block("x+1")] },
      { type: "paragraph" },
    ]);
  });
});

describe("streaming LaTeX blocks", () => {
  it("keeps display math with blank lines in one source slice with exact offsets", () => {
    const first = "First.\n\n";
    const formula = "\\[\na=1\n\nb=2\n\\]\n\n";
    const source = `${first}${formula}Last.`;
    expect(splitStreamingMarkdown(source, true)).toEqual([
      { start: 0, content: first, live: false },
      { start: first.length, content: formula, live: false },
      { start: first.length + formula.length, content: "Last.", live: true },
    ]);
    expect(splitStreamingMarkdown(source, false).every(part => !part.live)).toBe(true);
  });

  it("keeps an unfinished display equation live across blank lines until it closes", () => {
    const first = "First.\n\n";
    const equation = "\\[\na=1\n\nb=2\n\\]";
    for (let length = 2; length <= equation.length; length++) {
      const tail = equation.slice(0, length);
      expect(splitStreamingMarkdown(first + tail, true)).toEqual([
        { start: 0, content: first, live: false },
        { start: first.length, content: tail, live: true },
      ]);
    }
  });
});
