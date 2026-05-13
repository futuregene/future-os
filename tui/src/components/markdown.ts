/**
 * Markdown renderer using marked for parsing, matching pi-mono style.
 * Supports: headings, bold, italic, code, code blocks, links, quotes, lists, tables.
 */

import { Marked, type Token, type Tokens } from "marked";
import { fg, bold, dim, italic } from "../theme.js";
import type { Theme } from "../theme.js";
import { visibleWidth } from "../utils.js";

export interface MarkdownTheme {
  heading: (text: string) => string;
  link: (text: string) => string;
  code: (text: string) => string;
  codeBlock: (text: string) => string;
  codeBlockBorder: (text: string) => string;
  quote: (text: string) => string;
  bold: (text: string) => string;
  italic: (text: string) => string;
}

const markdownParser = new Marked();

export class MarkdownRenderer {
  private theme: MarkdownTheme;

  constructor(theme?: Partial<MarkdownTheme>) {
    this.theme = {
      heading: (t) => fg(221, bold(t)),
      link: (t) => fg(117, t),
      code: (t) => fg(151, t),
      codeBlock: (t) => fg(143, dim(t)),
      codeBlockBorder: (t) => fg(244, dim(t)),
      quote: (t) => fg(244, italic(t)),
      bold: (t) => bold(t),
      italic: (t) => italic(t),
      ...theme,
    };
  }

  render(text: string, maxWidth: number): string[] {
    const lines: string[] = [];
    const tokens = markdownParser.lexer(text);

    for (let i = 0; i < tokens.length; i++) {
      const token = tokens[i];
      const nextType = tokens[i + 1]?.type;
      lines.push(...this.renderToken(token, maxWidth, nextType));
    }
    return lines;
  }

  private renderToken(token: Token, width: number, nextType?: string): string[] {
    const lines: string[] = [];

    switch (token.type) {
      case "heading": {
        const h = token as Tokens.Heading;
        const text = this.renderInline(h.tokens);
        if (h.depth === 1) {
          lines.push(this.theme.heading(this.theme.bold(text)));
        } else {
          lines.push(this.theme.heading(text));
        }
        if (nextType && nextType !== "space") lines.push("");
        break;
      }

      case "paragraph": {
        const p = token as Tokens.Paragraph;
        const text = this.renderInline(p.tokens);
        lines.push(text);
        if (nextType && nextType !== "list" && nextType !== "space") lines.push("");
        break;
      }

      case "code": {
        const c = token as Tokens.Code;
        lines.push(this.theme.codeBlockBorder("```" + (c.lang || "")));
        for (const codeLine of c.text.split("\n")) {
          lines.push("  " + this.theme.codeBlock(codeLine));
        }
        lines.push(this.theme.codeBlockBorder("```"));
        if (nextType && nextType !== "space") lines.push("");
        break;
      }

      case "blockquote": {
        const q = token as Tokens.Blockquote;
        for (const qt of q.tokens) {
          const inner = this.renderToken(qt, width - 2);
          for (const innerLine of inner) {
            if (innerLine === "") {
              lines.push(this.theme.quote("│ "));
            } else {
              lines.push(this.theme.quote("│ ") + this.theme.quote(innerLine));
            }
          }
        }
        if (nextType && nextType !== "space") lines.push("");
        break;
      }

      case "list": {
        lines.push(...this.renderList(token as Tokens.List, 0));
        break;
      }

      case "hr":
        lines.push(fg(244, "─".repeat(Math.min(width, 80))));
        if (nextType && nextType !== "space") lines.push("");
        break;

      case "space":
        lines.push("");
        break;

      case "table": {
        lines.push(...this.renderTable(token as Tokens.Table, width));
        if (nextType && nextType !== "space") lines.push("");
        break;
      }

      default:
        if ("text" in token && typeof token.text === "string") {
          lines.push(token.text);
        }
    }

    return lines;
  }

  private renderInline(tokens: Token[]): string {
    let result = "";
    for (const token of tokens) {
      switch (token.type) {
        case "text":
          result += token.text;
          break;
        case "strong":
          result += this.theme.bold(this.renderInline(token.tokens || []));
          break;
        case "em":
          result += this.theme.italic(this.renderInline(token.tokens || []));
          break;
        case "codespan":
          result += fg(151, token.text);
          break;
        case "link": {
          const t = token as Tokens.Link;
          const label = this.renderInline(t.tokens || []);
          result += this.theme.link(label);
          break;
        }
        case "del":
          result += fg(244, this.renderInline(token.tokens || []));
          break;
        case "br":
          result += "\n";
          break;
        case "html":
        case "image":
          break;
        default:
          if ("text" in token && typeof token.text === "string") {
            result += token.text;
          }
      }
    }
    return result;
  }

  private renderList(list: Tokens.List, depth: number): string[] {
    const lines: string[] = [];
    const indent = "  ".repeat(depth);
    let idx = list.start || 1;

    for (const item of list.items) {
      const bullet = list.ordered ? `${idx}. ` : "- ";
      idx++;

      // Collect all inline text from the item
      let itemText = "";
      let hasNestedList = false;
      const nestedLines: string[] = [];

      for (const itemToken of item.tokens) {
        if (itemToken.type === "list") {
          hasNestedList = true;
          nestedLines.push(...this.renderList(itemToken as Tokens.List, depth + 1));
        } else if (itemToken.type === "text" || itemToken.type === "paragraph") {
          const t = itemToken as Tokens.Text | Tokens.Paragraph;
          itemText += this.renderInline(t.tokens || []);
        }
      }

      if (itemText.trim() || !hasNestedList) {
        lines.push(indent + fg(151, bullet) + itemText);
      }
      if (hasNestedList) {
        for (const nl of nestedLines) {
          lines.push(nl);
        }
      }
    }

    return lines;
  }

  private renderTable(table: Tokens.Table, width: number): string[] {
    const lines: string[] = [];
    const ncols = table.header.length;
    if (ncols === 0) return lines;

    // Collect text per cell
    const headerCells = table.header.map((h) => this.renderInline(h.tokens));
    const rowCells = table.rows.map((row) =>
      row.map((cell) => this.renderInline(cell.tokens))
    );

    // Calculate column widths
    const colWidths: number[] = new Array(ncols).fill(3);
    for (let i = 0; i < ncols; i++) {
      colWidths[i] = Math.max(colWidths[i], visibleWidth(headerCells[i]));
      for (const row of rowCells) {
        colWidths[i] = Math.max(colWidths[i], visibleWidth(row[i]));
      }
    }

    // Clamp to fit
    const borderOverhead = 3 * ncols + 1;
    const avail = width - borderOverhead;
    const total = colWidths.reduce((a, b) => a + b, 0);
    if (total > avail) {
      const scale = avail / total;
      for (let i = 0; i < ncols; i++) {
        colWidths[i] = Math.max(3, Math.floor(colWidths[i] * scale));
      }
    }

    const pad = (text: string, w: number) =>
      text + " ".repeat(Math.max(0, w - visibleWidth(text)));

    // Top border
    lines.push("┌─" + colWidths.map((w) => "─".repeat(w)).join("─┬─") + "─┐");

    // Header
    lines.push("│ " + headerCells.map((c, i) => bold(pad(c, colWidths[i]))).join(" │ ") + " │");

    // Separator
    lines.push("├─" + colWidths.map((w) => "─".repeat(w)).join("─┼─") + "─┤");

    // Rows
    for (const row of rowCells) {
      lines.push("│ " + row.map((c, i) => pad(c, colWidths[i])).join(" │ ") + " │");
    }

    // Bottom border
    lines.push("└─" + colWidths.map((w) => "─".repeat(w)).join("─┴─") + "─┘");

    return lines;
  }
}
