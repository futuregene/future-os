/**
 * Simple markdown renderer for assistant messages.
 * Supports: bold, italic, inline code, code blocks, links.
 */

import { CSI, RESET, BOLD, DIM } from "../tui.js";

export interface MarkdownTheme {
  codeBg: number;
  codeFg: number;
  link: number;
  bold: number;
  italic: number;
  fg: number;
}

export class Markdown {
  private text: string;
  private theme: MarkdownTheme;
  private fg: number;

  constructor(text: string, fg = 252, theme?: Partial<MarkdownTheme>) {
    this.text = text;
    this.fg = fg;
    this.theme = {
      codeBg: 236,
      codeFg: 150,
      link: 39,
      bold: 255,
      italic: 245,
      fg,
      ...theme,
    };
  }

  /**
   * Render markdown text to ANSI-colored lines wrapped to maxWidth.
   */
  render(maxWidth: number): string[] {
    const lines: string[] = [];
    const rawLines = this.text.split("\n");

    let inCodeBlock = false;
    let inInlineCode = false;
    let codeBlockLang = "";

    for (const raw of rawLines) {
      // Code block fence
      if (raw.startsWith("```")) {
        if (inCodeBlock) {
          // End code block
          lines.push(`${CSI}2m${"─".repeat(Math.min(raw.length, maxWidth))}${RESET}`);
          inCodeBlock = false;
          codeBlockLang = "";
        } else {
          // Start code block
          codeBlockLang = raw.slice(3).trim();
          lines.push(`${CSI}38;5;${this.theme.codeFg}m${"┌" + "─".repeat(Math.min(raw.length - 3, maxWidth - 2)) + "┐"}${RESET}`);
          if (codeBlockLang) {
            lines.push(`${CSI}38;5;${this.theme.codeFg}m ${codeBlockLang} ${RESET}`);
          }
          inCodeBlock = true;
        }
        continue;
      }

      if (inCodeBlock) {
        // Render code line with dim color
        const trimmed = raw.slice(0, maxWidth - 1);
        lines.push(`${CSI}38;5;${this.theme.codeFg}m ${trimmed}${RESET}`);
        continue;
      }

      // Process inline elements
      const processed = this.processInline(raw);
      const wrapped = this.wrapText(processed, maxWidth);
      lines.push(...wrapped);
    }

    return lines;
  }

  private processInline(text: string): string {
    // Process in order: code > bold > italic > links
    let result = text;

    // Inline code: `code`
    result = result.replace(/`([^`]+)`/g, (_, code) => {
      return `${CSI}38;5;${this.theme.codeFg}m${CSI}48;5;${this.theme.codeBg}m${code}${CSI}0m`;
    });

    // Bold: **text**
    result = result.replace(/\*\*([^*]+)\*\*/g, (_, t) => {
      return `${CSI}1m${t}${CSI}0m`;
    });

    // Italic: *text* or _text_
    result = result.replace(/\*([^*]+)\*/g, (_, t) => {
      return `${CSI}3m${t}${CSI}0m`;
    });
    result = result.replace(/_([^_]+)_/g, (_, t) => {
      return `${CSI}3m${t}${CSI}0m`;
    });

    return result;
  }

  private wrapText(text: string, maxWidth: number): string[] {
    if (!text) return [""];
    const lines: string[] = [];
    const stripped = this.stripAnsi(text);
    if (stripped.length <= maxWidth) return [text];

    // Simple word wrap
    const words = stripped.split(" ");
    let currentLine = "";
    let currentAnsi = "";

    for (const word of words) {
      if (currentLine.length + word.length + 1 > maxWidth) {
        if (currentLine) {
          lines.push(currentLine);
          currentLine = "";
        }
      }
      if (!currentLine) {
        currentLine = word;
      } else {
        currentLine += " " + word;
      }
    }
    if (currentLine) lines.push(currentLine);
    return lines.length ? lines : [""];
  }

  private stripAnsi(text: string): string {
    return text.replace(/\x1b\[[0-9;]*m/g, "");
  }
}

/**
 * Render a chat message (user, assistant, tool, system) as formatted lines.
 */
export interface ChatMessageData {
  role: "user" | "assistant" | "system" | "tool";
  content?: string;
  name?: string;
  tool?: string;
}

export interface ChatMessageTheme {
  userPrefix: number;
  assistantPrefix: number;
  systemPrefix: number;
  toolPrefix: number;
  fg: number;
  dimFg: number;
}

export function renderChatMessage(msg: ChatMessageData, maxWidth: number, theme?: Partial<ChatMessageTheme>): string[] {
  const t: ChatMessageTheme = {
    userPrefix: 220,    // yellow
    assistantPrefix: 39, // blue
    systemPrefix: 244,   // gray
    toolPrefix: 200,    // red/pink
    fg: 252,
    dimFg: 245,
    ...theme,
  };

  const prefixMap = {
    user: "👤",
    assistant: "🤖",
    system: "⚙️",
    tool: "🔧",
  };
  const colorMap = {
    user: t.userPrefix,
    assistant: t.assistantPrefix,
    system: t.systemPrefix,
    tool: t.toolPrefix,
  };

  const prefix = prefixMap[msg.role];
  const color = colorMap[msg.role];

  if (!msg.content) return [];

  if (msg.role === "user") {
    // User messages: simple one-liner for compact display
    const content = msg.content.split("\n")[0];
    const truncated = content.length > maxWidth - 3 ? content.slice(0, maxWidth - 6) + "…" : content;
    return [`${prefix} ${CSI}38;5;${color}m${truncated}${RESET}`];
  }

  if (msg.role === "system") {
    const lines = msg.content.split("\n").slice(0, 2);
    return lines.map((l, i) =>
      i === 0
        ? `${CSI}38;5;${color}m${prefix} ${l}${RESET}`
        : `${CSI}38;5;${t.dimFg}m${l}${RESET}`
    );
  }

  // Assistant or tool message: render as markdown
  const md = new Markdown(msg.content, t.fg);
  const mdLines = md.render(maxWidth);

  if (msg.role === "assistant") {
    const first = mdLines[0];
    const rest = mdLines.slice(1);
    return [
      `${CSI}38;5;${color}m${prefix} ${first}${RESET}`,
      ...rest.map((l) => `  ${l}`),
    ];
  }

  // Tool message
  const toolName = msg.tool ? `[${msg.tool}] ` : "";
  return [
    `${CSI}38;5;${color}m${prefix} ${toolName}${RESET}${mdLines[0] || ""}`,
    ...mdLines.slice(1).map((l) => `  ${l}`),
  ];
}
