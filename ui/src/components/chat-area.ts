/**
 * ChatArea - scrollable chat view matching pi-mono style.
 * Renders messages with proper markdown, tool output, and streaming.
 */

import { CSI, RESET } from "../tui.js";
import { fg, bg, dim, bold, italic } from "../theme.js";
import type { Theme } from "../theme.js";
import { MarkdownRenderer } from "./markdown.js";

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system" | "tool";
  content: string;
  name?: string;     // tool name
  tool?: string;     // tool call id
  toolStatus?: "running" | "complete" | "error";
  exitCode?: number;
  timestamp?: number;
  thinking?: string;
  pending?: boolean;  // streaming in progress
  welcome?: boolean;   // skip prefix/icon for welcome/info messages
}

export class ChatArea {
  private messages: ChatMessage[] = [];
  private viewportTop = 0;
  private renderedLines: RenderedLine[] = [];
  private autoScroll = true;
  private width = 80;

  private md = new MarkdownRenderer();
  private theme: Theme;

  constructor(private maxWidth = 80, theme?: Theme) {
    this.theme = theme ?? {
      bg: 235, fg: 252, accent: 151, border: 69,
      selectedBg: 237, selectedFg: 255, dim: 245,
      error: 204, success: 142,
      mdHeading: 221, mdLink: 117, mdCode: 151,
      mdCodeBlock: 142, mdCodeBlockBorder: 244, mdQuote: 244,
      toolPendingBg: 235, toolSuccessBg: 236, toolErrorBg: 237,
      toolTitle: 151, toolOutput: 244,
      thinkingOff: 240, thinkingMinimal: 110, thinkingLow: 68,
      thinkingMedium: 117, thinkingHigh: 182, thinkingXhigh: 213,
      userBg: 239, assistantBg: 235,
    };
  }

  // ─── Public API ─────────────────────────────────────────────────

  setWidth(w: number): void {
    if (w !== this.width) {
      this.width = w;
      this.rerender();
    }
  }

  setViewportHeight(_h: number): void {
    // Clamp viewport
    if (this.viewportTop + 20 > this.renderedLines.length) {
      this.viewportTop = Math.max(0, this.renderedLines.length - 20);
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
      last.pending = true;
      this.rerender();
      if (this.autoScroll) this.scrollToBottom();
    }
  }

  markLastMessageComplete(): void {
    if (this.messages.length === 0) return;
    const last = this.messages[this.messages.length - 1];
    if (last.role === "assistant") {
      last.pending = false;
      this.rerender();
    }
  }

  clearMessages(): void {
    this.messages = [];
    this.rerender();
  }

  scrollUp(lines = 5): void {
    this.viewportTop = Math.max(0, this.viewportTop - lines);
    this.autoScroll = false;
  }

  scrollDown(lines = 5): void {
    this.viewportTop = Math.min(
      Math.max(0, this.renderedLines.length - 20),
      this.viewportTop + lines
    );
  }

  scrollToBottom(): void {
    this.viewportTop = Math.max(0, this.renderedLines.length - 20);
  }

  // ─── Rendering ──────────────────────────────────────────────────

  getHeight(): number {
    return Math.min(this.renderedLines.length, 20);
  }

  render(): string[] {
    const visible = this.renderedLines.slice(
      this.viewportTop,
      this.viewportTop + 20
    );
    return visible.map((rl) => rl.text);
  }

  private rerender(): void {
    this.renderedLines = [];
    for (const msg of this.messages) {
      this.renderMessage(msg);
    }
    this.renderedLines.push({ text: "", dim: true });
  }

  private renderMessage(msg: ChatMessage): void {
    switch (msg.role) {
      case "user":
        this.renderUserMessage(msg);
        break;
      case "assistant":
        this.renderAssistantMessage(msg);
        break;
      case "tool":
        this.renderToolMessage(msg);
        break;
      case "system":
        this.renderSystemMessage(msg);
        break;
    }
  }

  // ─── User message ─────────────────────────────────────────────

  private renderUserMessage(msg: ChatMessage): void {
    const lines = msg.content.split("\n");
    // First line with icon
    const first = lines[0] || "";
    this.renderedLines.push({
      text: `${fg(226, "👤")} ${fg(252, this.md.strip(first).length > this.width - 4 ? this.md.strip(first).slice(0, this.width - 7) + "…" : first)}`,
      dim: false,
    });
    // Continuation lines (dimmed)
    for (const line of lines.slice(1)) {
      if (line.trim()) {
        this.renderedLines.push({
          text: `${fg(244, "  " + line)}`,
          dim: true,
        });
      }
    }
  }

  // ─── Assistant message ────────────────────────────────────────

  private renderAssistantMessage(msg: ChatMessage): void {
    // Prefix line with icon
    const prefix = msg.pending
      ? `${fg(151, "🤖")} ${fg(245, "◐")}`
      : `${fg(69, "🤖")}`;

    const lines = msg.content.split("\n");
    const first = lines[0] || "";

    if (first) {
      const mdLines = this.md.render(first, this.width - 3);
      if (mdLines.length > 0) {
        this.renderedLines.push({ text: `${prefix} ${mdLines[0]}`, dim: false });
        for (const line of mdLines.slice(1)) {
          this.renderedLines.push({ text: `  ${line}`, dim: false });
        }
      }
    } else {
      this.renderedLines.push({ text: prefix, dim: false });
    }

    // Rest of lines
    for (const line of lines.slice(1)) {
      if (line.trim()) {
        const mdLines = this.md.render(line, this.width - 3);
        for (const l of mdLines) {
          this.renderedLines.push({ text: `  ${l}`, dim: false });
        }
      }
    }

    // Streaming cursor
    if (msg.pending) {
      this.renderedLines.push({ text: `${fg(151, "  ◐")}`, dim: false });
    }
  }

  // ─── Tool message ─────────────────────────────────────────────

  private renderToolMessage(msg: ChatMessage): void {
    const toolName = msg.name || msg.tool || "tool";
    const status = msg.toolStatus || "running";

    // Background color based on status
    const bgColor = status === "error" ? this.theme.toolErrorBg
      : status === "complete" ? this.theme.toolSuccessBg
      : this.theme.toolPendingBg;

    const borderColor = fg(244, "─".repeat(Math.min(toolName.length + 4, this.width - 4)));
    const titleColor = fg(151, bold(`$ ${toolName}`));

    // Top border
    this.renderedLines.push({
      text: bg(bgColor, fg(244, "┌" + "─".repeat(Math.min(toolName.length + 4, this.width - 4)) + "┐")),
      dim: false,
    });

    // Command line
    this.renderedLines.push({
      text: bg(bgColor, ` ${titleColor}`),
      dim: false,
    });

    // Output
    const outputLines = msg.content.split("\n");
    for (const line of outputLines.slice(0, 30)) {
      const stripped = this.md.strip(line);
      if (stripped) {
        const truncated = stripped.length > this.width - 4
          ? stripped.slice(0, this.width - 7) + "…"
          : stripped;
        this.renderedLines.push({
          text: bg(bgColor, ` ${fg(244, truncated)}`),
          dim: true,
        });
      }
    }

    if (outputLines.length > 30) {
      this.renderedLines.push({
        text: bg(bgColor, fg(244, ` ${"[... ${outputLines.length - 30} more lines]"}`)),
        dim: true,
      });
    }

    // Exit code for completed tools
    if (status === "complete" && msg.exitCode !== undefined) {
      const exitColor = msg.exitCode === 0 ? fg(142, `[exit ${msg.exitCode}]`) : fg(204, `[exit ${msg.exitCode}]`);
      this.renderedLines.push({
        text: bg(bgColor, ` ${exitColor}`),
        dim: true,
      });
    }

    // Bottom border
    this.renderedLines.push({
      text: bg(bgColor, fg(244, "└" + "─".repeat(Math.min(toolName.length + 4, this.width - 4)) + "┘")),
      dim: false,
    });
  }

  // ─── System message ──────────────────────────────────────────

  private renderSystemMessage(msg: ChatMessage): void {
    if (msg.welcome) {
      // Welcome/info messages: no prefix, just colored text
      const lines = msg.content.split("\n");
      for (const line of lines) {
        if (!line.trim()) {
          this.renderedLines.push({ text: "", dim: true });
        } else {
          this.renderedLines.push({ text: line, dim: true });
        }
      }
      return;
    }
    // Real system messages: ⚙️ prefix
    const lines = msg.content.split("\n");
    for (let i = 0; i < lines.length; i++) {
      if (!lines[i].trim()) continue;
      const isError = lines[i].toLowerCase().includes("error") || lines[i].toLowerCase().includes("failed");
      const color = isError ? fg(204, lines[i]) : fg(244, lines[i]);
      this.renderedLines.push({ text: `${fg(244, "⚙️")} ${color}`, dim: true });
    }
  }
}

interface RenderedLine {
  text: string;
  dim: boolean;
}
