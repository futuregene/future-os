import { describe, expect, it } from "vitest";
import { parseFutureMarkdown } from "./parseFutureMarkdown";

const parse = (source: string) => parseFutureMarkdown(source, undefined, false);
const first = (source: string) => parse(source).nodes[0]!;

/**
 * The native `\(…\)` / `\[…\]` math construct. These cases target the
 * tokenizer's less-travelled states: a display block continued lazily, an
 * escaped character at the very end of the input, and an escaped line ending.
 */
describe("remarkLatexMath — tokenizer edge states", () => {
  it("ends an unclosed display block at its container boundary", () => {
    // The unindented continuation line is not part of the list item, so the
    // display block must not swallow it: the math ends and the remaining text is
    // an ordinary paragraph rather than being pulled into the formula.
    const nodes = parse("- \\[a\n\noutside\n").nodes;
    expect(nodes.map((node) => node.type)).toEqual(["list", "paragraph"]);
    expect(nodes[1]).toEqual({ children: [{ text: "outside", type: "text" }], type: "paragraph" });
  });

  it("refuses a lazy continuation line instead of swallowing it into the formula", () => {
    // The guard this covers: when display math sits inside a container and the
    // next line is an *unprefixed* (lazy) continuation, the tokenizer must end
    // the formula rather than pull that line in. Without the guard the lazy line
    // and the stray closing bracket would become part of the formula.
    expect(parse("> \\[x\ny\n\\]\n").nodes).toEqual([
      { children: [{ code: "x", type: "mathBlock" }], type: "blockquote" },
      { children: [{ text: "y\n]", type: "text" }], type: "paragraph" },
    ]);
  });

  it("refuses a lazy continuation after a paragraph inside the same container", () => {
    expect(parse("> a\n> \\[x\ny\n\\]\n").nodes).toEqual([
      {
        children: [
          { children: [{ text: "a", type: "text" }], type: "paragraph" },
          { code: "x", type: "mathBlock" },
        ],
        type: "blockquote",
      },
      { children: [{ text: "y\n]", type: "text" }], type: "paragraph" },
    ]);
  });

  it("keeps a properly prefixed continuation inside the container", () => {
    // The contrast that makes the two cases above meaningful: prefix the
    // continuation line and the formula does continue (only the delimiters are
    // consumed, so the closing bracket's line leaves its own text behind).
    expect(parse("> \\[x\n> y\n\\]\n").nodes).toEqual([
      { children: [{ code: "x\ny", type: "mathBlock" }], type: "blockquote" },
      { children: [{ text: "]", type: "text" }], type: "paragraph" },
    ]);
  });

  it("splits a display block that is not continued inside its container", () => {
    // Same contract without the blank separator: the display block opened inside
    // the list item ends there, and the leftover formula text becomes an
    // ordinary root paragraph rather than being pulled into the formula.
    const nodes = parse("- \\[a\nb\n\\]\n").nodes;
    expect(nodes.map((node) => node.type)).toEqual(["list", "paragraph"]);
    expect(nodes[0]).toMatchObject({ items: [{ blocks: [{ code: "a", type: "mathBlock" }] }] });
    expect(nodes[1]).toEqual({ children: [{ text: "b\n]", type: "text" }], type: "paragraph" });
  });

  it("keeps an indented continuation inside its container", () => {
    expect(parse("- \\[a\n  b\n  \\]\n").nodes).toEqual([
      { items: [{ blocks: [{ code: "a\nb", type: "mathBlock" }], children: [] }], ordered: false, type: "list" },
    ]);
    expect(parse("> \\[a\n> b\n> \\]\n").nodes).toEqual([
      { children: [{ code: "a\nb", type: "mathBlock" }], type: "blockquote" },
    ]);
  });

  it("keeps an escaped character that runs to the end of the input", () => {
    // `\(a\` with no closing delimiter: the escape consumes what it can, then
    // the state machine sees end-of-input. The unmatched inline delimiter falls
    // back to ordinary Markdown rather than swallowing the rest.
    expect(first("text \\(a\\")).toMatchObject({
      children: [{ text: expect.stringContaining("text ") }],
    });
  });

  it("keeps an escaped line ending inside an inline formula", () => {
    const paragraph = first("x \\(a\\\nb\\)\n") as { children: Array<{ code?: string; text?: string; type: string }> };
    // The backslash before the newline is data, not a delimiter; the formula runs
    // to the end of the line, so no trailing text node is produced.
    expect(paragraph.children).toEqual([
      { text: "x ", type: "text" },
      { code: "a\\\nb", displayMode: false, type: "mathInline" },
    ]);
  });

  it("trims a display block but not an inline formula", () => {
    expect(parse("\\[\n spaced \n\\]\n").nodes).toEqual([{ code: "spaced", type: "mathBlock" }]);
    expect(first("x \\( spaced \\)\n")).toMatchObject({
      children: [{ text: "x ", type: "text" }, { code: " spaced ", displayMode: false, type: "mathInline" }],
    });
  });
});
