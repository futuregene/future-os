/**
 * Markdown renderer matching pi-mono style.
 * Supports: headings, bold, italic, code, code blocks, links, quotes, lists.
 */

import { CSI, RESET } from "../tui.js";
import { fg, bg, bold, dim, italic } from "../theme.js";
import type { Theme } from "../theme.js";

export interface MarkdownThemeColors {
  heading: number;
  link: number;
  code: number;
  codeBlock: number;
  codeBlockBorder: number;
  quote: number;
  bold: number;
  italic: number;
  text: number;
  dim: number;
}

const DEFAULT_THEME_COLORS: MarkdownThemeColors = {
  heading: 221,   // gold
  link: 117,      // light blue
  code: 151,      // accent (teal)
  codeBlock: 142,  // green
  codeBlockBorder: 244,
  quote: 244,
  bold: 255,
  italic: 245,
  text: 252,
  dim: 245,
};

export class MarkdownRenderer {
  private colors: MarkdownThemeColors;

  constructor(colors?: Partial<MarkdownThemeColors>) {
    this.colors = { ...DEFAULT_THEME_COLORS, ...colors };
  }

  /**
   * Render markdown text to ANSI-colored lines wrapped to maxWidth.
   */
  render(text: string, maxWidth: number): string[] {
    const lines: string[] = [];
    const rawLines = text.split("\n");

    let inCodeBlock = false;
    let codeLang = "";

    for (const raw of rawLines) {
      // Code block fence
      if (raw.startsWith("```")) {
        if (inCodeBlock) {
          // End code block: bottom border
          lines.push(this.codeBlockBorder(` ${"─".repeat(Math.min(raw.length, maxWidth - 4))} `));
          inCodeBlock = false;
          codeLang = "";
        } else {
          // Start code block: top border
          codeLang = raw.slice(3).trim();
          lines.push(this.codeBlockBorder(` ┌${"─".repeat(Math.min(raw.length, maxWidth - 4))}┐ `));
          if (codeLang) {
            lines.push(this.codeBlock(`${"  " + codeLang} `));
          }
          inCodeBlock = true;
        }
        continue;
      }

      if (inCodeBlock) {
        // Code block line: green text, dim
        lines.push(this.dimCode(raw.slice(0, maxWidth - 2)));
        continue;
      }

      // Blockquote
      if (raw.startsWith("> ")) {
        const content = raw.slice(2);
        lines.push(this.quote(`▌ ${this.processInline(content, maxWidth - 3)}`));
        continue;
      }

      // Heading
      const headingMatch = raw.match(/^(#{1,6})\s+(.*)/);
      if (headingMatch) {
        const level = headingMatch[1].length;
        const content = headingMatch[2];
        const prefix = " ".repeat(Math.max(0, 3 - level));
        lines.push(this.heading(`${prefix}${content}`));
        continue;
      }

      // List item
      const listMatch = raw.match(/^(\s*)[-*]\s+(.*)/);
      if (listMatch) {
        const indent = listMatch[1].length;
        const content = listMatch[2];
        lines.push(`${"  ".repeat(indent)}  ${this.accentBullet("▸")} ${this.processInline(content, maxWidth - indent - 4)}`);
        continue;
      }

      // Regular paragraph
      const processed = this.processInline(raw, maxWidth - 2);
      if (processed.trim()) {
        lines.push(processed);
      } else {
        lines.push("");
      }
    }

    return lines;
  }

  /**
   * Process inline markdown: bold, italic, code, links.
   */
  private processInline(text: string, maxWidth: number): string {
    let result = text;

    // Inline code: `code` — accent on dark background
    result = result.replace(/`([^`]+)`/g, (_, code) => {
      return bg(236, fg(151, code));
    });

    // Bold: **text** or __text__
    result = result.replace(/\*\*([^*]+)\*\*/g, (_, t) => bold(fg(255, t)));
    result = result.replace(/__([^_]+)__/g, (_, t) => bold(fg(255, t)));

    // Italic: *text* or _text_
    result = result.replace(/\*([^*]+)\*/g, (_, t) => italic(fg(245, t)));
    result = result.replace(/_([^_]+)_/g, (_, t) => italic(fg(245, t)));

    // Strikethrough: ~~text~~
    result = result.replace(/~~([^~]+)~~/g, (_, t) => `${CSI}9m${t}${RESET}`);

    // Links: [text](url) → text (dimmed url)
    result = result.replace(/\[([^\]]+)\]\(([^)]+)\)/g, (_, label, _url) => {
      return fg(117, label);
    });

    return result;
  }

  private heading(text: string): string {
    return fg(221, bold(text));
  }

  private accentBullet(text: string): string {
    return fg(151, text);
  }

  private quote(text: string): string {
    return fg(244, italic(text));
  }

  private codeBlock(text: string): string {
    return fg(142, dim(text));
  }

  private codeBlockBorder(text: string): string {
    return fg(244, dim(text));
  }

  private dimCode(text: string): string {
    return fg(142, dim(text));
  }

  /**
   * Strip ANSI codes to get plain text length.
   */
  strip(text: string): string {
    return text.replace(/\x1b\[[0-9;]*m/g, "");
  }

  /**
   * Word-wrap text to maxWidth.
   */
  wrap(text: string, maxWidth: number): string[] {
    if (this.strip(text).length <= maxWidth) return [text];
    const words = this.strip(text).split(" ");
    const lines: string[] = [];
    let line = "";
    for (const word of words) {
      const combined = line ? line + " " + word : word;
      if (combined.length > maxWidth && line) {
        lines.push(line);
        line = word;
      } else {
        line = combined;
      }
    }
    if (line) lines.push(line);
    return lines.length ? lines : [""];
  }
}
