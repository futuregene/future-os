/**
 * ChatArea - scrollable chat view matching pi-mono style.
 * Renders messages with proper markdown, tool output, and streaming.
 */

import { CSI, RESET } from "../tui.js";
import type { Component } from "../tui.js";
import { fg, bg, bold, italic } from "../theme.js";
import type { Theme } from "../theme.js";
import { MarkdownRenderer } from "./markdown.js";
import { visibleWidth, applyBackgroundToLine, truncateToWidth, wrapTextWithAnsi } from "../utils.js";

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
  private dirty = false;  // true when messages changed but render() not yet called

  private md = new MarkdownRenderer();
  private theme: Theme;


  constructor(private maxWidth = 80, theme?: Theme) {
    this.theme = theme ?? {
      bg: -1, fg: 252, accent: 39, border: 240,
      selectedBg: 38, selectedFg: 255, dim: 245,
      error: 204, success: 143,
      mdHeading: 221, mdLink: 117, mdCode: 109,
      mdCodeBlock: 143, mdCodeBlockBorder: 244, mdQuote: 244,
      toolPendingBg: 236, toolSuccessBg: 236, toolErrorBg: 52,
      toolTitle: 109, toolOutput: 244,
      thinkingOff: 240, thinkingMinimal: 110, thinkingLow: 68,
      thinkingMedium: 117, thinkingHigh: 182, thinkingXhigh: 213,
      thinkingText: 244,
      userBg: 59, assistantBg: -1,
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

  addToolStart(toolId: string, toolName: string, toolArgs?: string): void {
    const msg: ChatMessage = {
      id: crypto.randomUUID(),
      role: "tool",
      content: "",
      name: toolName,
      tool: toolId,
      toolStatus: "running",
    };
    if (toolArgs !== undefined) {
      (msg as { toolArgs?: string }).toolArgs = toolArgs;
    }
    this.messages.push(msg);
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

  finishTool(toolId: string, _output?: string): void {
    for (let i = this.messages.length - 1; i >= 0; i--) {
      const msg = this.messages[i];
      if (msg.role === "tool" && msg.tool === toolId) {
        msg.toolStatus = "complete";
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
    const maxTop = Math.max(0, this.renderedLines.length - this.viewportHeight);
    this.viewportTop = Math.min(maxTop, this.viewportTop + lines);
    // Re-enable auto-scroll when scrolled to bottom
    if (this.viewportTop >= maxTop) {
      this.autoScroll = true;
    }
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
    if (width !== this.lastRenderWidth || this.dirty) {
      this.lastRenderWidth = width;
      this.width = width;
      this.dirty = false;
      this.rerender();
    }
    const visible = this.renderedLines.slice(
      this.viewportTop,
      this.viewportTop + this.viewportHeight
    );
    return visible.map((rl) => rl.text);
  }

  private rerender(): void {
    // Defer until first render() has set the correct terminal width.
    if (this.lastRenderWidth === -1) {
      this.dirty = true;
      return;
    }
    this.dirty = false;

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
      // Use applyBackgroundToLine to properly handle inner RESET codes
      // from markdown (links, code, etc.) — they would otherwise clear the bg.
      const bgLine = applyBackgroundToLine(text, this.width, this.theme.userBg);
      this.renderedLines.push({
        text: bgLine,
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
        const thinkPrefix = `\x1b[3m\x1b[38;5;${this.theme.thinkingText}m`;
        for (const line of thinkingLines) {
          if (line === "") {
            this.renderedLines.push({ text: "", dim: true });
          } else {
            // Re-apply thinking style after every ANSI reset within the line,
            // so that markdown bold/code/link codes don't clear the thinking color.
            const styled = ` ${line}`.replace(/\x1b\[0m/g, `\x1b[0m${thinkPrefix}`);
            this.renderedLines.push({
              text: thinkPrefix + styled + RESET,
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

  // ─── Tool message (single-line summary) ─

  private renderToolMessage(msg: ChatMessage): void {
    const toolName = msg.name || msg.tool || "tool";
    const status = msg.toolStatus || "running";

    // Background color based on status (subtle dark gray, red for errors)
    const bgColor = status === "error" ? this.theme.toolErrorBg
      : status !== "running" ? this.theme.toolSuccessBg
      : this.theme.toolPendingBg;

    // Single-line header: tool name + key args (file path for read/write/edit)
    const toolArgs = (msg as { toolArgs?: string }).toolArgs;
    const commandLine = " " + this.formatToolCall(toolName, toolArgs);
    this.renderedLines.push({
      text: applyBackgroundToLine(commandLine, this.width, bgColor),
      dim: false,
    });
  }

  /** Format tool call display per tool type (matches pi's per-tool renderCall). */
  private formatToolCall(toolName: string, toolArgs?: string): string {
    // Total line: " " + prefix + " " + content, must fit within this.width
    // Available for content = this.width - 1 (leading space) - visibleWidth(prefix) - 1 (separator)
    const maxFor = (prefixLen: number) => Math.max(10, this.width - 2 - prefixLen);
    if (!toolArgs) {
      return fg(this.theme.toolTitle, bold(truncateToWidth(toolName, maxFor(toolName.length))));
    }

    try {
      const args = JSON.parse(toolArgs);

      switch (toolName) {
        case "bash": {
          const cmd = typeof args.command === "string" ? args.command : "";
          const firstLine = cmd.split("\n")[0] ?? "";
          const cmdText = firstLine ? firstLine : "...";
          return `${fg(this.theme.toolTitle, bold("$"))} ${truncateToWidth(cmdText, maxFor(1))}`;
        }
        case "read": {
          const filePath = typeof args.path === "string" ? args.path : "";
          let rangeInfo = "";
          if (args.offset !== undefined) {
            const start = args.offset ?? 1;
            const end = args.limit !== undefined ? start + args.limit - 1 : "";
            rangeInfo = `:${start}${end ? `-${end}` : ""}`;
          }
          const maxPath = Math.max(5, maxFor(4) - rangeInfo.length);
          const pathDisplay = filePath
            ? fg(this.theme.accent, truncateToWidth(filePath, maxPath))
            : fg(this.theme.toolOutput, "...");
          return `${fg(this.theme.toolTitle, bold("read"))} ${pathDisplay}${fg(this.theme.error, rangeInfo)}`;
        }
        case "write": {
          const filePath = typeof args.path === "string" ? args.path : "";
          const pathDisplay = filePath ? fg(this.theme.accent, truncateToWidth(filePath, maxFor(5))) : fg(this.theme.toolOutput, "...");
          return `${fg(this.theme.toolTitle, bold("write"))} ${pathDisplay}`;
        }
        case "edit": {
          const filePath = typeof args.path === "string" ? args.path : "";
          const pathDisplay = filePath ? fg(this.theme.accent, truncateToWidth(filePath, maxFor(4))) : fg(this.theme.toolOutput, "...");
          return `${fg(this.theme.toolTitle, bold("edit"))} ${pathDisplay}`;
        }
        case "grep": {
          const pattern = typeof args.pattern === "string" ? args.pattern : "";
          const filePath = typeof args.path === "string" ? args.path : "";
          const patternDisplay = pattern ? fg(this.theme.toolOutput, truncateToWidth(pattern, maxFor(4))) : "...";
          const pathDisplay = filePath ? ` ${fg(this.theme.accent, truncateToWidth(filePath, Math.max(5, maxFor(4) - (pattern ? pattern.length + 1 : 0))))}` : "";
          return `${fg(this.theme.toolTitle, bold("grep"))} ${patternDisplay}${pathDisplay}`;
        }
        case "ls": {
          const filePath = typeof args.path === "string" ? args.path : "";
          const pathDisplay = filePath ? fg(this.theme.accent, truncateToWidth(filePath, maxFor(2))) : fg(this.theme.toolOutput, "...");
          return `${fg(this.theme.toolTitle, bold("ls"))} ${pathDisplay}`;
        }
        default: {
          const argSummary = JSON.stringify(args);
          const truncated = truncateToWidth(argSummary, maxFor(toolName.length));
          return `${fg(this.theme.toolTitle, bold(toolName))} ${fg(this.theme.toolOutput, truncated)}`;
        }
      }
    } catch {
      const displayArgs = truncateToWidth(toolArgs, maxFor(toolName.length));
      return `${fg(this.theme.toolTitle, bold(toolName))} ${fg(this.theme.toolOutput, displayArgs)}`;
    }
  }

  // ─── System message ──────────────────────────────────────────

  private renderSystemMessage(msg: ChatMessage): void {
    if (msg.welcome) {
      const wrapWidth = Math.max(1, this.width - 2);
      const lines = msg.content.split("\n");
      for (const line of lines) {
        if (!line.trim()) {
          this.renderedLines.push({ text: "", dim: true });
        } else {
          const wrapped = wrapTextWithAnsi(line, wrapWidth);
          for (const wl of wrapped) {
            this.renderedLines.push({ text: wl, dim: true });
          }
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
