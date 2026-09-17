import { basename, type InlineNode, type TableNode } from "@future-os/markdown";

function cellText(nodes: InlineNode[]): string {
  return nodes.map(node => {
    if ("children" in node && node.children) return cellText(node.children);
    if ("text" in node) return node.text;
    if ("code" in node) return node.code;
    if (node.type === "break") return "\n";
    if (node.type === "image") return node.alt || basename(node.src);
    if (node.type === "futureReference") return node.reference.label || basename(node.reference.targetId);
    return "";
  }).join("");
}

/** Estimate native 14pt text, not Markdown syntax. Exact glyph measurement is
 * unnecessary: cells wrap, and the shared widths keep virtualized rows aligned.
 */
function preferredWidth(cell: InlineNode[], fontScale: number): number {
  let line = 0;
  let longest = 0;
  for (const char of cellText(cell)) {
    if (char === "\n") line = 0;
    else line += char.codePointAt(0)! <= 0xff ? 8 : 14;
    longest = Math.max(longest, line);
    if (longest >= 180) break;
  }
  // 8dp padding on each side, plus a possible left border.
  return Math.min(180 * fontScale, Math.max(64 * fontScale, longest * fontScale + 17));
}

export function markdownTableWidths(node: TableNode, viewportWidth: number, fontScale: number): number[] {
  const preferred = node.headers.map((header, column) => {
    let width = preferredWidth(header, fontScale);
    for (const row of node.rows) {
      if (width >= 180 * fontScale) break;
      width = Math.max(width, preferredWidth(row[column] ?? [], fontScale));
    }
    return width;
  });
  // Keep short label/number columns narrow. Long columns may wrap down to
  // 112dp, but never squeeze every column just to eliminate horizontal scroll.
  const minimum = preferred.map(width => Math.min(width, 112 * fontScale));
  const total = preferred.reduce((sum, width) => sum + width, 0);
  const minimumTotal = minimum.reduce((sum, width) => sum + width, 0);
  const available = Math.max(0, viewportWidth - 2); // outer borders
  if (viewportWidth <= 0 || total <= available || total === minimumTotal) return preferred;
  const shrink = Math.min(1, (total - available) / (total - minimumTotal));
  return preferred.map((width, column) => width - (width - minimum[column]!) * shrink);
}
