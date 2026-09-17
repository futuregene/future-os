import { parseFutureMarkdown, type TableNode } from "@future-os/markdown";
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
