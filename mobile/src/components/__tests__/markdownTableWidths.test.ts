import { parseFutureMarkdown, type InlineNode, type TableNode } from "@future-os/markdown";
import { markdownTableWidths } from "../markdownTableWidths";

function table(source: string): TableNode {
  const node = parseFutureMarkdown(source).nodes[0];
  if (node?.type !== "table") throw new Error("Expected a table");
  return node;
}

const experiment = table("| 条件 | 模型输入 | 评分方式 |\n|---|---|---|\n| 闭卷 | 冻结投影 + 原题 | 实验结束后统一评分 |\n| 开卷 | **同一冻结投影 + 同一原题** | 实验结束后统一评分 |");

test("short CJK labels leave room for longer columns on a phone", () => {
  const widths = markdownTableWidths(experiment, 320, 1);
  expect(widths[0]).toBe(64);
  expect(widths[1]).toBeGreaterThan(widths[0]!);
  expect(widths[2]).toBeGreaterThan(widths[0]!);
  expect(widths.reduce((sum, width) => sum + width, 0)).toBeCloseTo(318);
});

test("many long columns retain readable widths and overflow instead of clipping", () => {
  const node = table("| Long heading one | Long heading two | Long heading three | Long heading four |\n|---|---|---|---|\n| a | b | c | d |");
  expect(markdownTableWidths(node, 320, 1)).toEqual([112, 112, 112, 112]);
});

test("system font scaling increases widths without shrinking text", () => {
  const node = table("| 条件 | 模型输入 |\n|---|---|\n| 开卷 | 冻结投影 |");
  expect(markdownTableWidths(node, 1000, 1)).toEqual([64, 73]);
  // Native text scales; the 17dp padding/border does not.
  expect(markdownTableWidths(node, 1000, 2)).toEqual([128, 129]);
  expect(markdownTableWidths(experiment, 320, 2).reduce((sum, width) => sum + width, 0)).toBeGreaterThan(320);
});

test("inline formatting, link URLs and explicit breaks do not inflate widths", () => {
  const formatted = table("| 条件 | 输入 |\n|---|---|\n| **开卷** | [冻结投影](https://example.com/very/long/path)<br>同一原题 |");
  const plain = table("| 条件 | 输入 |\n|---|---|\n| 开卷 | 冻结投影 |");
  expect(markdownTableWidths(formatted, 320, 1)).toEqual(markdownTableWidths(plain, 320, 1));
});

test("body rows beyond the initial virtualized viewport contribute to column widths", () => {
  const node = table("| A | B |\n|---|---|\n" + "| 1 | 2 |\n".repeat(50) + "| 3 | 最后一个很长的单元格内容 |");
  expect(markdownTableWidths(node, 1000, 1)).toEqual([64, 180]);
});

test("empty cells, initial layout and wide orientations remain compact and finite", () => {
  const node = table("| A | B |\n|---|---|\n| missing |");
  expect(markdownTableWidths(node, 0, 1)).toEqual([73, 64]);
  expect(markdownTableWidths(node, 1600, 1)).toEqual([73, 64]);
  expect(markdownTableWidths({ headers: [], rows: [], alignments: [] }, 320, 1)).toEqual([]);
});

/** One column whose single-cell content is exactly `cell`. */
function singleColumn(cell: InlineNode[]): TableNode {
  return { alignments: [null], headers: [cell], rows: [] };
}

describe("what a column is measured from", () => {
  test("inline code counts as its code, not as its syntax", () => {
    expect(markdownTableWidths(singleColumn([{ code: "abcdefgh", type: "code" }]), 1000, 1))
      .toEqual([81]);
  });

  test("an image counts as its alt text, falling back to its file name", () => {
    expect(markdownTableWidths(singleColumn([{ alt: "alt", src: "https://x/y/long-name.png", type: "image" }]), 1000, 1))
      .toEqual([64]);
    // No alt: the reader sees the file name, so the column is sized for it.
    expect(markdownTableWidths(singleColumn([{ alt: "", src: "/a/b/diagram.png", type: "image" }]), 1000, 1))
      .toEqual([105]);
  });

  test("a future reference counts as its label, falling back to the target id", () => {
    const reference = {
      label: "Web", source: "inline" as const, targetId: "future-web",
      targetType: "file" as const, version: null, view: "chip" as const,
    };
    expect(markdownTableWidths(singleColumn([{ reference, type: "futureReference" }]), 1000, 1))
      .toEqual([64]);
    const bare = { ...reference, label: "" };
    expect(markdownTableWidths(singleColumn([{ reference: bare, type: "futureReference" }]), 1000, 1))
      .toEqual([97]);
  });

  test("an empty or unknown inline node contributes nothing rather than NaN", () => {
    expect(markdownTableWidths(singleColumn([]), 1000, 1)).toEqual([64]);
    expect(markdownTableWidths(singleColumn([{ type: "break" }]), 1000, 1)).toEqual([64]);
    expect(markdownTableWidths(singleColumn([{ children: [], type: "strong" }]), 1000, 1)).toEqual([64]);
    // An empty inline wrapper has no children to recurse into and no text of its
    // own: the column must fall back to its minimum, not to NaN.
    expect(markdownTableWidths(singleColumn([{ type: "strong" } as InlineNode]), 1000, 1))
      .toEqual([64]);
  });

  test("a line break starts a new measurement line instead of widening the column", () => {
    const wrapped: InlineNode[] = [
      { text: "x".repeat(20), type: "text" },
      { type: "break" },
      { text: "yy", type: "text" },
    ];
    expect(markdownTableWidths(singleColumn(wrapped), 1000, 1)).toEqual([177]);
  });

  test("content past the 180dp cap stops widening the column", () => {
    expect(markdownTableWidths(singleColumn([{ text: "y".repeat(400), type: "text" }]), 1000, 1))
      .toEqual([180]);
    // A row longer than an already-capped header changes nothing.
    const node: TableNode = {
      alignments: [null],
      headers: [[{ text: "x".repeat(400), type: "text" }]],
      rows: [[[{ text: "y".repeat(400), type: "text" }]]],
    };
    expect(markdownTableWidths(node, 1000, 1)).toEqual([180]);
  });
});
