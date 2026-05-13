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
  private dimGray(text: string): string { return fg(244, text); }

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

  // ─── Tool call management ───────────────────────────────────────

  addToolStart(toolId: string, toolName: string): void {
    this.messages.push({
      id: crypto.randomUUID(),
      role: "tool",
      content: "",  // holds args delta during streaming
      name: toolName,
      tool: toolId,
      toolStatus: "running",
    });
    this.rerender();
    if (this.autoScroll) this.scrollToBottom();
  }

  appendToolDelta(toolId: string, text: string): void {
    for (let i = this.messages.length - 1; i >= 0; i--) {
      const msg = this.messages[i];
      if (msg.role === "tool" && msg.tool === toolId) {
        msg.content += text;
        this.rerender();
        if (this.autoScroll) this.scrollToBottom();
        return;
      }
    }
  }

  finishTool(toolId: string, toolArgs?: string): void {
    for (let i = this.messages.length - 1; i >= 0; i--) {
      const msg = this.messages[i];
      if (msg.role === "tool" && msg.tool === toolId) {
        msg.toolStatus = "complete";
        if (toolArgs !== undefined) {
          (msg as { toolArgs?: string }).toolArgs = toolArgs;
        }
        this.rerender();
        return;
      }
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
    const prefix = msg.pending
      ? `${fg(151, "🤖")} ${fg(245, "◐")}`
      : `${fg(69, "🤖")}`;

    // Split content into paragraphs (separated by blank lines)
    const paragraphs = msg.content.split(/\n\n+/);
    const streaming = msg.pending;

    for (let p = 0; p < paragraphs.length; p++) {
      const para = paragraphs[p];
      const lines = para.split("\n");
      const isLastPara = p === paragraphs.length - 1;

      for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        const isLastLine = isLastPara && i === lines.length - 1;
        const isFirstLine = p === 0 && i === 0;

        if (!line.trim()) continue;

        const mdLines = this.md.render(line, this.width - 3);
        for (let j = 0; j < mdLines.length; j++) {
          const isLastMdLine = isLastLine && j === mdLines.length - 1;

          let text: string;
          if (isFirstLine && j === 0) {
            text = `${prefix} ${mdLines[j]}`;
          } else {
            text = `  ${mdLines[j]}`;
          }

          // Append streaming cursor to the very last content line
          if (streaming && isLastMdLine) {
            text += fg(151, " ◐");
          }

          this.renderedLines.push({ text, dim: false });
        }
      }
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

    // Command line: $ toolname args (toolArgs holds the final args after tool_end)
    const toolArgs = (msg as { toolArgs?: string }).toolArgs;
    const commandLine = toolArgs !== undefined && toolArgs !== ""
      ? `${fg(151, bold("$"))} ${fg(151, toolName)} ${this.dimGray(toolArgs)}`
      : `${fg(151, bold("$"))} ${fg(151, toolName)}`;

    // Top border
    const boxWidth = Math.min(this.width - 4, 76);
    const borderLine = "─".repeat(boxWidth);
    this.renderedLines.push({
      text: bg(bgColor, fg(244, "┌" + borderLine + "┐")),
      dim: false,
    });

    // Command line with args
    this.renderedLines.push({
      text: bg(bgColor, ` ${commandLine}`),
      dim: false,
    });

    // Streaming indicator when running
    if (status === "running") {
      this.renderedLines.push({
        text: bg(bgColor, fg(245, "   ◐")),
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
      text: bg(bgColor, fg(244, "└" + borderLine + "┘")),
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
