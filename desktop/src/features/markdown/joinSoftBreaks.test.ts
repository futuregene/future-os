import { describe, expect, it } from "vitest";
import { joinSoftBreaks, parseFutureMarkdown } from "./parseFutureMarkdown";

// mdast keeps a paragraph's soft breaks inside `text` values; a document surface
// must join them so prose wrapped at the source's column reflows to the pane.
describe("joinSoftBreaks", () => {
  it("reflows a paragraph wrapped at the file's column", () => {
    const document = joinSoftBreaks(parseFutureMarkdown(
      "投影最小（1 706 tok）、回本最快（46 轮）；压缩请求复用自己的\n基础指令，压缩只要 0.42 元。",
    ));

    expect(document.nodes).toEqual([{
      type: "paragraph",
      children: [{
        type: "text",
        text: "投影最小（1 706 tok）、回本最快（46 轮）；压缩请求复用自己的 基础指令，压缩只要 0.42 元。",
      }],
    }]);
  });

  it("folds indentation around the break and keeps surrounding formatting", () => {
    const document = joinSoftBreaks(parseFutureMarkdown("**加粗**\n  后续正文\n\n> 引用第一行\n> 引用第二行"));

    expect(document.nodes[0]).toEqual({
      type: "paragraph",
      children: [
        { type: "strong", children: [{ type: "text", text: "加粗" }] },
        { type: "text", text: " 后续正文" },
      ],
    });
    expect(document.nodes[1]).toEqual({
      type: "blockquote",
      children: [{
        type: "paragraph",
        children: [{ type: "text", text: "引用第一行 引用第二行" }],
      }],
    });
  });

  it("joins list items and setext headings", () => {
    const document = joinSoftBreaks(parseFutureMarkdown(
      "- 列表第一行\n  列表第二行\n\n标题第一行\n标题第二行\n===",
    ));

    expect(document.nodes[0]).toMatchObject({
      type: "list",
      items: [{ children: [{ type: "text", text: "列表第一行 列表第二行" }] }],
    });
    expect(document.nodes[1]).toMatchObject({
      type: "heading",
      level: 1,
      children: [{ type: "text", text: "标题第一行 标题第二行" }],
    });
  });

  it("keeps hard breaks, which are their own node", () => {
    // `\` before the newline, and two trailing spaces, are hard breaks.
    const document = joinSoftBreaks(parseFutureMarkdown("第一行\\\n第二行\n\n第三行  \n第四行"));

    expect(document.nodes[0]).toEqual({
      type: "paragraph",
      children: [
        { type: "text", text: "第一行" },
        { type: "break" },
        { type: "text", text: "第二行" },
      ],
    });
    expect(document.nodes[1]).toEqual({
      type: "paragraph",
      children: [
        { type: "text", text: "第三行" },
        { type: "break" },
        { type: "text", text: "第四行" },
      ],
    });
  });

  it("leaves code blocks and break-free documents as they are", () => {
    const untouched = parseFutureMarkdown("```\n第一行\n第二行\n```\n\n一段\n\n另一段");
    expect(joinSoftBreaks(untouched)).toBe(untouched);
    expect(joinSoftBreaks(untouched).nodes[0]).toEqual({
      type: "code",
      language: undefined,
      code: "第一行\n第二行",
    });
  });

  it("rebuilds only the blocks that carry a soft break", () => {
    const before = parseFutureMarkdown("一段\n续行\n\n另一段");
    const after = joinSoftBreaks(before);

    expect(after).not.toBe(before);
    expect(after.nodes[0]).not.toBe(before.nodes[0]);
    expect(after.nodes[1]).toBe(before.nodes[1]);
    expect(after.raw).toBe(before.raw);
  });
});
