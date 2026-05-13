/**
 * ChatArea - scrollable chat view matching pi-mono style.
 * Renders messages with proper markdown, tool output, and streaming.
 */

import { CSI, RESET } from "../tui.js";
import type { Component } from "../tui.js";
import { fg, bg, bold, italic } from "../theme.js";
import type { Theme } from "../theme.js";
import { MarkdownRenderer } from "./markdown.js";
import { visibleWidth, applyBackgroundToLine } from "../utils.js";

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

export class ChatArea implements Component {
  private messages: ChatMessage[] = [];
  private viewportTop = 0;
  private viewportHeight = 20;
  private renderedLines: RenderedLine[] = [];
  private autoScroll = true;
  private width = 80;
  private thinkingHidden = false;
  private lastRenderWidth = -1;

  private md = new MarkdownRenderer();
  private theme: Theme;

  constructor(private maxWidth = 80, theme?: Theme) {
    this.theme = theme ?? {
      bg: 235, fg: 252, accent: 109, border: 69,
      selectedBg: 237, selectedFg: 255, dim: 245,
      error: 204, success: 143,
      mdHeading: 221, mdLink: 117, mdCode: 109,
      mdCodeBlock: 143, mdCodeBlockBorder: 244, mdQuote: 244,
      toolPendingBg: 17, toolSuccessBg: 22, toolErrorBg: 52,
      toolTitle: 109, toolOutput: 244,
      thinkingOff: 240, thinkingMinimal: 110, thinkingLow: 68,
      thinkingMedium: 117, thinkingHigh: 182, thinkingXhigh: 213,
      thinkingText: 244,
      userBg: 59, assistantBg: 235,
    };
  }

  // ─── Public API ─────────────────────────────────────────────────

  setWidth(w: number): void {
    if (w !== this.width) {
      this.width = w;
      this.rerender();
    }
  }

  setViewportHeight(h: number): void {
    this.viewportHeight = h;
    if (this.viewportTop + this.viewportHeight > this.renderedLines.length) {
      this.viewportTop = Math.max(0, this.renderedLines.length - this.viewportHeight);
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

  appendToLastMessage(delta: string): void {
    if (this.messages.length === 0) return;
    const last = this.messages[this.messages.length - 1];
    if (last.role === "assistant") {
      last.content += delta;
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
      content: "",
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

  // ─── Thinking management ────────────────────────────────────────

  private newId(): string {
    return Math.random().toString(36).slice(2, 10);
  }

  startThinking(): void {
    if (this.messages.length === 0) {
      this.messages.push({ id: this.newId(), role: "assistant", content: "", thinking: "" });
      this.rerender();
      return;
    }
    const last = this.messages[this.messages.length - 1];
    if (last.role === "assistant") {
      last.thinking = "";
      this.rerender();
    } else {
      this.messages.push({ id: this.newId(), role: "assistant", content: "", thinking: "" });
      this.rerender();
    }
  }

  appendThinkingDelta(text: string): void {
    if (this.messages.length === 0) return;
    const last = this.messages[this.messages.length - 1];
    if (last.role === "assistant" && last.thinking !== undefined) {
      last.thinking += text;
      this.rerender();
    }
  }

  endThinking(): void {
    if (this.messages.length === 0) return;
    const last = this.messages[this.messages.length - 1];
    if (last.role === "assistant" && last.thinking !== undefined) {
      this.rerender();
    }
  }

  setThinkingHidden(hidden: boolean): void {
    if (this.thinkingHidden !== hidden) {
      this.thinkingHidden = hidden;
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
      Math.max(0, this.renderedLines.length - this.viewportHeight),
      this.viewportTop + lines
    );
  }

  scrollToBottom(): void {
    this.viewportTop = Math.max(0, this.renderedLines.length - this.viewportHeight);
  }

  // ─── Rendering ──────────────────────────────────────────────────

  getHeight(): number {
    return Math.min(this.renderedLines.length, this.viewportHeight);
  }

  handleInput(_data: string): void { /* no-op */ }

  invalidate(): void {
    this.lastRenderWidth = -1;
  }

  render(width: number): string[] {
    if (width !== this.lastRenderWidth) {
      this.lastRenderWidth = width;
      this.width = width;
      this.rerender();
    }
    const visible = this.renderedLines.slice(
      this.viewportTop,
      this.viewportTop + this.viewportHeight
    );
    return visible.map((rl) => rl.text);
  }

  private rerender(): void {
    this.renderedLines = [];
    for (let i = 0; i < this.messages.length; i++) {
      if (i > 0) {
        // Add spacing between messages (matches pi's Spacer(1) between components)
        this.renderedLines.push({ text: "", dim: true });
      }
      this.renderMessage(this.messages[i]);
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

  // ─── User message (pi style: markdown + full-width background Box) ─

  private renderUserMessage(msg: ChatMessage): void {
    // Render through markdown for proper text wrapping (matches pi: Markdown inside Box)
    const rendered = this.md.render(msg.content, this.width - 2);
    for (const line of rendered) {
      if (line === "") continue;
      const text = ` ${line}`;
      const pad = Math.max(0, this.width - visibleWidth(text));
      const padded = text + " ".repeat(pad);
      this.renderedLines.push({
        text: bg(this.theme.userBg, padded),
        dim: false,
      });
    }
  }

  // ─── Assistant message (pi style: mardown via marked, thinking first) ─

  private renderAssistantMessage(msg: ChatMessage): void {
    const hasThinking = msg.thinking && msg.thinking.trim();

    // Render thinking block FIRST (before content).
    // Render through the markdown renderer so long lines wrap correctly
    // (matches pi passing thinking through its Markdown component).
    if (hasThinking) {
      if (this.thinkingHidden) {
        this.renderedLines.push({
          text: fg(this.theme.thinkingText, italic(" Thinking...")),
          dim: true,
        });
      } else {
        const thinkingLines = this.md.render(msg.thinking!, this.width - 2);
        for (const line of thinkingLines) {
          if (line === "") {
            this.renderedLines.push({ text: "", dim: true });
          } else {
            this.renderedLines.push({
              text: fg(this.theme.thinkingText, italic(` ${line}`)),
              dim: true,
            });
          }
        }
      }
    }

    // Spacer between thinking and content
    if (hasThinking && msg.content.trim()) {
      this.renderedLines.push({ text: "", dim: true });
    }

    // Render markdown content with marked parser
    const contentWidth = this.width - 2;
    const rendered = this.md.render(msg.content, contentWidth);
    for (const line of rendered) {
      if (line === "") {
        this.renderedLines.push({ text: "", dim: true });
      } else {
        this.renderedLines.push({ text: ` ${line}`, dim: false });
      }
    }
  }

  // ─── Tool message (pi style: background color only, no borders) ─

  private renderToolMessage(msg: ChatMessage): void {
    const toolName = msg.name || msg.tool || "tool";
    const status = msg.toolStatus || "running";

    // Background color based on status
    const bgColor = status === "error" ? this.theme.toolErrorBg
      : status === "complete" ? this.theme.toolSuccessBg
      : this.theme.toolPendingBg;

    // Command line: $ toolname args
    const toolArgs = (msg as { toolArgs?: string }).toolArgs;
    const commandLine = toolArgs !== undefined && toolArgs !== ""
      ? `${fg(this.theme.toolTitle, bold("$"))} ${fg(this.theme.toolTitle, toolName)} ${fg(this.theme.toolOutput, toolArgs)}`
      : `${fg(this.theme.toolTitle, bold("$"))} ${fg(this.theme.toolTitle, toolName)}`;

    // Full-width bg line with command.
    // fg()/bold() emit RESET codes that would clear the background, so we
    // re-apply the background after every inner RESET before the final one.
    const text = " " + commandLine;
    const bgLine = applyBackgroundToLine(text, this.width, bgColor);
    this.renderedLines.push({
      text: bgLine,
      dim: false,
    });

    // Exit code for completed tools
    if (status === "complete" && msg.exitCode !== undefined) {
      const exitColor = msg.exitCode === 0 ? this.theme.success : this.theme.error;
      const exitText = ` ${fg(exitColor, `[exit ${msg.exitCode}]`)}`;
      const exitBgLine = applyBackgroundToLine(exitText, this.width, bgColor);
      this.renderedLines.push({
        text: exitBgLine,
        dim: true,
      });
    }
  }

  // ─── System message ──────────────────────────────────────────

  private renderSystemMessage(msg: ChatMessage): void {
    if (msg.welcome) {
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
    const lines = msg.content.split("\n");
    for (let i = 0; i < lines.length; i++) {
      if (!lines[i].trim()) continue;
      const isError = lines[i].toLowerCase().includes("error") || lines[i].toLowerCase().includes("failed");
      const color = isError ? fg(this.theme.error, lines[i]) : fg(this.theme.dim, lines[i]);
      this.renderedLines.push({ text: ` ${color}`, dim: true });
    }
  }
}

interface RenderedLine {
  text: string;
  dim: boolean;
}
