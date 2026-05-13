/**
 * ChatArea - the main scrollable chat view.
 * Manages a viewport into the message history.
 */

import { CSI, RESET, CLEAR_LINE } from "../tui.js";

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system" | "tool";
  content: string;
  name?: string;
  tool?: string;
  timestamp?: number;
  thinking?: string;  // for thinking block
}

export interface ChatTheme {
  userPrefix: number;
  assistantPrefix: number;
  systemPrefix: number;
  toolPrefix: number;
  userText: number;
  assistantText: number;
  dimText: number;
  errorText: number;
  codeBg: number;
  codeFg: number;
  accent: number;
}

export class ChatArea {
  private messages: ChatMessage[] = [];
  private viewportTop = 0;      // index into renderedLines
  private viewportHeight = 20;
  private renderedLineCount = 0;
  private renderedLines: RenderedLine[] = [];
  private autoScroll = true;
  private width = 80;

  // Prefix icons and colors per role
  private theme: ChatTheme = {
    userPrefix: 220,
    assistantPrefix: 39,
    systemPrefix: 244,
    toolPrefix: 200,
    userText: 252,
    assistantText: 252,
    dimText: 245,
    errorText: 160,
    codeBg: 236,
    codeFg: 150,
    accent: 39,
  };

  constructor(private maxWidth = 80) {
    this.width = maxWidth;
  }

  // ─── Public API ─────────────────────────────────────────────────────────

  setWidth(w: number): void {
    if (w !== this.width) {
      this.width = w;
      this.rerender();
    }
  }

  setViewportHeight(h: number): void {
    this.viewportHeight = h;
    // Clamp viewport
    if (this.viewportTop + this.viewportHeight > this.renderedLineCount) {
      this.viewportTop = Math.max(0, this.renderedLineCount - this.viewportHeight);
    }
  }

  setAutoScroll(v: boolean): void {
    this.autoScroll = v;
    if (v) this.scrollToBottom();
  }

  addMessage(msg: ChatMessage): void {
    this.messages.push(msg);
    this.rerender();
    if (this.autoScroll) this.scrollToBottom();
  }

  updateLastMessage(content: string): void {
    if (this.messages.length === 0) return;
    const last = this.messages[this.messages.length - 1];
    if (last.role === "assistant") {
      last.content = content;
      this.rerender();
      if (this.autoScroll) this.scrollToBottom();
    }
  }

  clearMessages(): void {
    this.messages = [];
    this.rerender();
  }

  scrollUp(lines = 3): void {
    this.viewportTop = Math.max(0, this.viewportTop - lines);
  }

  scrollDown(lines = 3): void {
    this.viewportTop = Math.min(
      Math.max(0, this.renderedLineCount - this.viewportHeight),
      this.viewportTop + lines
    );
  }

  scrollToBottom(): void {
    this.viewportTop = Math.max(0, this.renderedLineCount - this.viewportHeight);
  }

  // ─── Rendering ────────────────────────────────────────────────────────

  getHeight(): number {
    return Math.min(this.renderedLineCount, this.viewportHeight);
  }

  render(): string[] {
    const visibleLines = this.renderedLines.slice(
      this.viewportTop,
      this.viewportTop + this.viewportHeight
    );
    return visibleLines.map((rl) => rl.text);
  }

  private rerender(): void {
    this.renderedLines = [];
    for (const msg of this.messages) {
      this.renderMessage(msg);
      // Add separator after each message
      this.renderedLines.push({ text: "", dim: true });
    }
    this.renderedLineCount = this.renderedLines.length;
  }

  private renderMessage(msg: ChatMessage): void {
    const prefix = this.rolePrefix(msg.role);
    const color = this.roleColor(msg.role);

    if (msg.role === "user") {
      // User: compact single-line prefix + first line
      const firstLine = msg.content.split("\n")[0];
      const truncated = this.truncate(firstLine, this.width - 3);
      this.renderedLines.push({
        text: `${prefix} ${this.color(truncated, color)}`,
        dim: false,
      });
      return;
    }

    if (msg.role === "system") {
      const lines = msg.content.split("\n").slice(0, 2);
      for (let i = 0; i < lines.length; i++) {
        this.renderedLines.push({
          text: `${i === 0 ? prefix + " " : "  "}${this.color(lines[i], this.theme.dimText)}`,
          dim: true,
        });
      }
      return;
    }

    if (msg.role === "assistant" || msg.role === "tool") {
      // Tool name badge
      if (msg.tool) {
        this.renderedLines.push({
          text: `${prefix} ${this.color("[" + msg.tool + "]", this.theme.dimText)}`,
          dim: false,
        });
      } else {
        this.renderedLines.push({
          text: `${prefix}`,
          dim: false,
        });
      }

      // Content rendered as markdown
      const mdLines = this.renderMarkdown(msg.content);
      for (const line of mdLines) {
        this.renderedLines.push({ text: `  ${line}`, dim: false });
      }

      // Thinking block (if any)
      if (msg.thinking) {
        this.renderedLines.push({
          text: `  ${this.color("[thinking]", this.theme.dimText)}`,
          dim: true,
        });
      }
    }
  }

  private renderMarkdown(text: string): string[] {
    const lines: string[] = [];
    const rawLines = text.split("\n");

    let inCode = false;
    let codeLang = "";

    for (const raw of rawLines) {
      // Code block fence
      if (raw.startsWith("```")) {
        if (inCode) {
          lines.push(this.dim("-".repeat(Math.min(raw.length, this.width - 2))));
          inCode = false;
          codeLang = "";
        } else {
          codeLang = raw.slice(3).trim();
          lines.push(this.color("\u250C" + "-".repeat(Math.min(raw.length - 3, this.width - 4)) + "\u2510", this.theme.codeFg));
          if (codeLang) lines.push(this.color(` ${codeLang} `, this.theme.codeFg));
          inCode = true;
        }
        continue;
      }

      if (inCode) {
        lines.push(" " + this.color(raw.slice(0, this.width - 2), this.theme.codeFg));
        continue;
      }

      // Inline code: `code`
      let processed = this.processInline(raw);

      // Truncate to width
      processed = this.truncate(processed, this.width - 2);

      lines.push(processed);
    }

    return lines;
  }

  private processInline(text: string): string {
    // Inline code
    text = text.replace(/`([^`]+)`/g, (_, code) =>
      `${this.colorBg(code, this.theme.codeBg, this.theme.codeFg)}`
    );
    // Bold
    text = text.replace(/\*\*([^*]+)\*\*/g, (_, t) => `${RESET}${CSI}1m${t}${CSI}0m`);
    // Italic
    text = text.replace(/\*([^*]+)\*/g, (_, t) => `${CSI}3m${t}${CSI}0m`);
    // Dim
    text = text.replace(/`([^`]+)`/g, (_, code) =>
      `${this.colorBg(code, this.theme.codeBg, this.theme.codeFg)}`
    );
    return text;
  }

  private truncate(text: string, maxWidth: number): string {
    const stripped = this.stripAnsi(text);
    if (stripped.length <= maxWidth) return text;
    return text.slice(0, maxWidth - 1) + "…";
  }

  private stripAnsi(text: string): string {
    return text.replace(/\x1b\[[0-9;]*m/g, "");
  }

  private rolePrefix(role: string): string {
    switch (role) {
      case "user": return "👤";
      case "assistant": return "🤖";
      case "system": return "⚙️";
      case "tool": return "🔧";
      default: return "•";
    }
  }

  private roleColor(role: string): number {
    switch (role) {
      case "user": return this.theme.userText;
      case "assistant": return this.theme.assistantText;
      case "system": return this.theme.dimText;
      case "tool": return this.theme.toolPrefix;
      default: return 252;
    }
  }

  private color(text: string, c: number): string {
    return `${CSI}38;5;${c}m${text}${RESET}`;
  }

  private colorBg(text: string, bg: number, fg: number): string {
    return `${CSI}38;5;${fg}m${CSI}48;5;${bg}m${text}${RESET}`;
  }

  private dim(text: string): string {
    return `${CSI}2m${text}${RESET}`;
  }
}

interface RenderedLine {
  text: string;
  dim: boolean;
}
