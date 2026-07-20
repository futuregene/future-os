/**
 * ChatArea - scrollable chat view matching style.
 * Renders messages with proper markdown, tool output, and streaming.
 */

import { RESET } from "../tui.js";
import type { Component } from "../tui.js";
import { fg, bold, italic } from "../theme.js";
import type { Theme } from "../theme.js";
import { MarkdownRenderer } from "./markdown.js";
import { applyBackgroundToLine, truncateToWidth, wrapTextWithAnsi } from "../utils.js";

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system" | "tool";
  content: string;
  name?: string;     // tool name
  tool?: string;     // tool call id
  toolArgs?: string; // tool arguments (JSON, for display)
  toolStatus?: "running" | "complete" | "error";
  exitCode?: number;
  timestamp?: number;
  thinking?: string;
  pending?: boolean;  // streaming in progress
  stopped?: boolean;  // generation was interrupted (partial content kept)
  welcome?: boolean;   // skip prefix/icon for welcome/info messages
}

export class ChatArea implements Component {
  private messages: ChatMessage[] = [];
  private viewportTop = 0;

  get lastMessage(): ChatMessage | undefined {
    return this.messages[this.messages.length - 1];
  }
  private viewportHeight = 20;
  private renderedLines: RenderedLine[] = [];
  private autoScroll = true;
  private width = 80;
  private thinkingHidden = false;
  private lastRenderWidth = -1;
  private dirty = false;  // true when messages changed but render() not yet called

  private md = new MarkdownRenderer();
  private theme: Theme;
  private onChange?: () => void;
  private messageLineRanges: { start: number; end: number }[] = [];

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
    if (this.lastRenderWidth === -1) {
      this.rerender();
    } else {
      this.appendLastMessage();
    }
    if (this.autoScroll) this.scrollToBottom();
    this.onChange?.();
  }

  setOnChange(cb: () => void): void {
    this.onChange = cb;
  }

  updateLastMessage(content: string): void {
    const idx = this.findAssistantIndex();
    if (idx >= 0) {
      this.messages[idx].content = content;
      this.messages[idx].pending = true;
      this.rerenderMessage(idx);
      if (this.autoScroll) this.scrollToBottom();
    }
  }

  appendToLastMessage(delta: string): void {
    const idx = this.findAssistantIndex();
    if (idx >= 0) {
      this.messages[idx].content += delta;
      this.messages[idx].pending = true;
      this.rerenderMessage(idx);
      if (this.autoScroll) this.scrollToBottom();
    }
  }

  /** Mark the streaming assistant message as interrupted (stopped).
   *  Partial content (text, thinking, tool calls) stays visible. */
  markLastAssistantStopped(): void {
    const idx = this.findAssistantIndex();
    if (idx >= 0) {
      const msg = this.messages[idx];
      msg.pending = false;
      msg.stopped = true;
      this.rerenderMessage(idx);
    }
  }

  markLastMessageComplete(): void {
    const idx = this.findAssistantIndex();
    if (idx >= 0) {
      this.messages[idx].pending = false;
      this.rerenderMessage(idx);
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
    if (this.lastRenderWidth === -1) {
      this.rerender();
    } else {
      this.appendLastMessage();
    }
    if (this.autoScroll) this.scrollToBottom();
  }

  appendToolDelta(toolId: string, text: string): void {
    const idx = this.findToolIndex(toolId);
    if (idx >= 0) {
      this.messages[idx].content += text;
      this.rerenderMessage(idx);
      if (this.autoScroll) this.scrollToBottom();
    }
  }

  finishTool(toolId: string, _output?: string): void {
    const idx = this.findToolIndex(toolId);
    if (idx >= 0) {
      this.messages[idx].toolStatus = "complete";
      this.rerenderMessage(idx);
    }
  }

  // ─── Thinking management ────────────────────────────────────────

  private newId(): string {
    return Math.random().toString(36).slice(2, 10);
  }

  startThinking(): void {
    if (this.messages.length === 0) {
      this.messages.push({ id: this.newId(), role: "assistant", content: "", thinking: "" });
      if (this.lastRenderWidth === -1) {
        this.rerender();
      } else {
        this.appendLastMessage();
      }
      return;
    }
    const lastIdx = this.messages.length - 1;
    const last = this.messages[lastIdx];
    if (last.role === "assistant") {
      // Subsequent thinking blocks in the same turn are concatenated
      // directly (no injected separator) into one thinking section.
      if (last.thinking === undefined) {
        last.thinking = "";
      }
      this.rerenderMessage(lastIdx);
    } else {
      this.messages.push({ id: this.newId(), role: "assistant", content: "", thinking: "" });
      if (this.lastRenderWidth === -1) {
        this.rerender();
      } else {
        this.appendLastMessage();
      }
    }
  }

  appendThinkingDelta(text: string): void {
    if (this.messages.length === 0) return;
    const lastIdx = this.messages.length - 1;
    const last = this.messages[lastIdx];
    if (last.role === "assistant" && last.thinking !== undefined) {
      last.thinking += text;
      this.rerenderMessage(lastIdx);
    }
  }

  endThinking(): void {
    if (this.messages.length === 0) return;
    const lastIdx = this.messages.length - 1;
    const last = this.messages[lastIdx];
    if (last.role === "assistant" && last.thinking !== undefined) {
      this.rerenderMessage(lastIdx);
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

  scrollUp(lines = 5): boolean {
    if (this.viewportTop <= 0) return false;
    this.viewportTop = Math.max(0, this.viewportTop - lines);
    this.autoScroll = false;
    return true;
  }

  scrollDown(lines = 5): boolean {
    const maxTop = Math.max(0, this.renderedLines.length - this.viewportHeight);
    if (this.viewportTop >= maxTop) return false;
    this.viewportTop = Math.min(maxTop, this.viewportTop + lines);
    if (this.viewportTop >= maxTop) {
      this.autoScroll = true;
    }
    return true;
  }

  isAtTop(): boolean {
    return this.viewportTop <= 0;
  }

  isAtBottom(): boolean {
    const maxTop = Math.max(0, this.renderedLines.length - this.viewportHeight);
    return this.viewportTop >= maxTop;
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

  /** Render ALL lines (bypass viewport) — used to seed terminal scrollback with full chat history. */
  renderAll(width: number): string[] {
    if (width !== this.lastRenderWidth || this.dirty) {
      this.lastRenderWidth = width;
      this.width = width;
      this.dirty = false;
      this.rerender();
    }
    return this.renderedLines.map((rl) => rl.text);
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
    this.messageLineRanges = [];
    for (let i = 0; i < this.messages.length; i++) {
      if (i > 0) {
        this.renderedLines.push({ text: "", dim: true });
      }
      const start = this.renderedLines.length;
      this.renderMessage(this.messages[i]);
      this.messageLineRanges.push({ start, end: this.renderedLines.length - 1 });
    }
    this.renderedLines.push({ text: "", dim: true });
  }

  /** Re-render only the message at msgIdx, splicing its lines in-place. */
  private rerenderMessage(msgIdx: number): void {
    if (this.lastRenderWidth === -1 || msgIdx >= this.messageLineRanges.length) {
      this.rerender();
      return;
    }
    const range = this.messageLineRanges[msgIdx];
    const oldLen = range.end - range.start + 1;

    // Render into a temp array via swap (avoids threading out params everywhere)
    const saved = this.renderedLines;
    this.renderedLines = [];
    this.renderMessage(this.messages[msgIdx]);
    const newLines = this.renderedLines;
    this.renderedLines = saved;

    this.renderedLines.splice(range.start, oldLen, ...newLines);
    const delta = newLines.length - oldLen;
    range.end = range.start + newLines.length - 1;
    for (let i = msgIdx + 1; i < this.messageLineRanges.length; i++) {
      this.messageLineRanges[i].start += delta;
      this.messageLineRanges[i].end += delta;
    }
    if (this.autoScroll) this.scrollToBottom();
    this.onChange?.();
  }

  /** Append the last message in this.messages to renderedLines (assumes msg already pushed). */
  private appendLastMessage(): void {
    // Remove trailing spacer
    this.renderedLines.pop();
    if (this.messages.length > 1) {
      this.renderedLines.push({ text: "", dim: true });
    }
    const start = this.renderedLines.length;
    this.renderMessage(this.messages[this.messages.length - 1]);
    this.messageLineRanges.push({ start, end: this.renderedLines.length - 1 });
    this.renderedLines.push({ text: "", dim: true });
    if (this.autoScroll) this.scrollToBottom();
    this.onChange?.();
  }

  private findAssistantIndex(): number {
    for (let i = this.messages.length - 1; i >= 0; i--) {
      if (this.messages[i].role === "assistant") return i;
    }
    return -1;
  }

  private findToolIndex(toolId: string): number {
    for (let i = this.messages.length - 1; i >= 0; i--) {
      const msg = this.messages[i];
      if (msg.role === "tool" && msg.tool === toolId) return i;
    }
    return -1;
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

  // ─── User message (markdown + full-width background Box) ─

  private renderUserMessage(msg: ChatMessage): void {
    // Render through markdown for proper text wrapping.
    // Blank lines are kept (with the bubble background) so multi-paragraph
    // user messages don't lose their paragraph separation.
    const rendered = this.md.render(msg.content, this.width - 2);
    for (const line of rendered) {
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

  // ─── Assistant message (mardown via marked, thinking first) ─

  private renderAssistantMessage(msg: ChatMessage): void {
    const hasThinking = msg.thinking && msg.thinking.trim();

    // Render thinking block FIRST (before content).
    // Render through the markdown renderer so long lines wrap correctly
    // .
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
            // Re-apply thinking style after EVERY ANSI reset within the line,
            // so markdown bold/code/link styles don't leak default-colored
            // text into the gray thinking block. Must match both "\x1b[0m"
            // and "\x1b[m" — the theme helpers reset with the latter, and
            // matching only the former left trailing segments unstyled
            // (rendered in body color instead of gray).
            const styled = ` ${line}`.replace(/\x1b\[0?m/g, `\x1b[0m${thinkPrefix}`);
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

    // Interrupted generation: show a subtle marker so the user can tell the
    // reply was aborted mid-stream rather than completed.
    if (msg.stopped) {
      this.renderedLines.push({
        text: fg(this.theme.thinkingText, italic(" ■ interrupted")),
        dim: true,
      });
    }
  }

  // ─── Tool message (single-line header only, matches streaming style) ─

  private renderToolMessage(msg: ChatMessage): void {
    const toolName = msg.name || msg.tool || "tool";
    const status = msg.toolStatus || "running";

    const bgColor = status === "error" ? this.theme.toolErrorBg
      : status !== "running" ? this.theme.toolSuccessBg
      : this.theme.toolPendingBg;

    // Single header line only — during streaming, tool output is never
    // shown inline (it arrives in subsequent assistant messages).
    const toolArgs = (msg as { toolArgs?: string }).toolArgs;
    const line = " " + this.formatToolCall(toolName, toolArgs);

    this.renderedLines.push({
      text: applyBackgroundToLine(line, this.width, bgColor),
      dim: status === "complete",
    });
  }

  /** Format tool call display per tool type . */
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
        case "shell": {
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
    const wrapWidth = Math.max(1, this.width - 2);
    const lines = msg.content.split("\n");
    for (let i = 0; i < lines.length; i++) {
      if (!lines[i].trim()) continue;
      const isError = lines[i].toLowerCase().includes("error") || lines[i].toLowerCase().includes("failed");
      const color = isError ? fg(this.theme.error, lines[i]) : fg(this.theme.dim, lines[i]);
      const wrapped = wrapTextWithAnsi(color, wrapWidth);
      for (const wl of wrapped) {
        this.renderedLines.push({ text: ` ${wl}`, dim: true });
      }
    }
  }
}

interface RenderedLine {
  text: string;
  dim: boolean;
}
