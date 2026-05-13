/**
 * xihu TUI - Main application.
 * Connects the TypeScript TUI to the Go Agent via JSON-RPC.
 */

import * as fs from "node:fs";
import { RpcClient } from "./rpc/client.js";
import { SelectList, type SelectItem } from "./components/select-list.js";
import { Editor } from "./components/editor.js";
import { renderChatMessage, type ChatMessageData } from "./components/markdown.js";
import {
  NodeTerminal,
  CSI,
  RESET,
  CLEAR,
  CLEAR_LINE,
  cursorPos,
  CURSOR_HIDE,
  CURSOR_SHOW,
  ALT_SCREEN_ON,
  ALT_SCREEN_OFF,
  DEFAULT_THEME,
  BOLD,
  type Theme,
} from "./tui.js";

type Overlay =
  | { kind: "select"; title: string; component: SelectList }
  | { kind: "input"; title: string; value: string }
  | null;

interface KeyEvent {
  name: string;
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
}

export class App {
  private terminal: NodeTerminal;
  private client: RpcClient;
  private theme: Theme;
  private editor: Editor;
  private messages: ChatMessageData[] = [];
  private model = "";
  private thinkingLevel = "off";
  private streaming = false;
  private overlay: Overlay = null;
  private running = false;
  private escBuf = "";

  constructor(private serverUrl = "http://localhost:7890") {
    this.terminal = new NodeTerminal();
    this.client = new RpcClient(serverUrl);
    this.theme = DEFAULT_THEME;
    this.editor = new Editor("❯ ", {
      prompt: this.theme.accent,
      text: this.theme.fg,
      cursor: this.theme.accent,
      bg: this.theme.bg,
    }, {
      onSubmit: (v) => this.handleSubmit(v),
    });
  }

  // ─── Lifecycle ────────────────────────────────────────────────────────────

  async start(): Promise<void> {
    // Enter alternate screen, hide cursor
    this.terminal.enterAlternateScreen();
    this.terminal.hideCursor();

    // Raw mode
    process.stdin.resume();
    process.stdin.setRawMode!(true);
    process.stdin.setEncoding("utf-8");

    this.running = true;

    // Key input loop
    process.stdin.on("data", (chunk: string) => {
      for (const char of chunk) {
        this.handleChar(char);
      }
    });

    // Initial state load
    await this.refresh();
    this.render();

    // Keep running
    await new Promise<void>((resolve) => {
      process.stdin.on("SIGINT", () => resolve());
    });
  }

  async stop(): Promise<void> {
    this.running = false;
    process.stdin.setRawMode!(false);
    this.terminal.exitAlternateScreen();
    this.terminal.showCursor();
    this.terminal.clear();
    this.terminal.close();
  }

  // ─── Key Input ──────────────────────────────────────────────────────────

  private handleChar(char: string): void {
    // Escape sequence handling
    if (char === "\x1b") {
      this.escBuf = "\x1b";
      return;
    }

    if (this.escBuf.length > 0) {
      this.escBuf += char;
      const key = this.parseEscSeq(this.escBuf);
      if (key) {
        this.escBuf = "";
        this.handleKey(key);
        return;
      }
      if (this.escBuf.length > 8) {
        // Unknown escape, reset
        this.escBuf = "";
      }
      return;
    }

    // Ctrl+C
    if (char.charCodeAt(0) === 3) {
      this.handleInterrupt();
      return;
    }

    // Overlay key handling
    if (this.overlay?.kind === "select") {
      const key = this.charToKey(char);
      if (key) {
        const handled = this.overlay.component.handleKey(key.name);
        if (handled) this.render();
      }
      return;
    }

    if (this.overlay?.kind === "input") {
      const code = char.charCodeAt(0);
      if (code === 13) {
        this.handleOverlayInputSubmit(this.overlay.value);
      } else if (code === 127) {
        this.overlay.value = this.overlay.value.slice(0, -1);
        this.render();
      } else if (code >= 32) {
        this.overlay.value += char;
        this.render();
      }
      return;
    }

    // Editor key handling
    const key = this.charToKey(char);
    if (key) {
      if (key.name === "escape") {
        if (this.editor.getValue() === "") {
          // Could close overlay or exit
        } else {
          this.editor.setValue("");
        }
      } else if (key.name === "ctrl+l") {
        this.terminal.clear();
        this.render();
      } else if (key.name === "ctrl+p") {
        this.cycleModel();
      } else if (key.name === "ctrl+s") {
        this.showSettings();
      } else if (key.name === "ctrl+r") {
        this.showSessions();
      } else {
        const handled = this.editor.handleKey(key);
        if (handled) this.render();
      }
    } else if (char.charCodeAt(0) >= 32) {
      this.editor.insertText(char);
      this.render();
    }
  }

  private charToKey(char: string): KeyEvent | null {
    const code = char.charCodeAt(0);
    if (code === 13) return { name: "enter", ctrl: false, shift: false, alt: false };
    if (code === 9)  return { name: "tab", ctrl: false, shift: false, alt: false };
    if (code === 127 || code === 8) return { name: "backspace", ctrl: false, shift: false, alt: false };
    if (code === 27)  return { name: "escape", ctrl: false, shift: false, alt: false };
    return null;
  }

  private parseEscSeq(seq: string): KeyEvent | null {
    switch (seq) {
      case "\x1b[A": return { name: "up", ctrl: false, shift: false, alt: false };
      case "\x1b[B": return { name: "down", ctrl: false, shift: false, alt: false };
      case "\x1b[C": return { name: "right", ctrl: false, shift: false, alt: false };
      case "\x1b[D": return { name: "left", ctrl: false, shift: false, alt: false };
      case "\x1b[H": return { name: "home", ctrl: false, shift: false, alt: false };
      case "\x1b[F": return { name: "end", ctrl: false, shift: false, alt: false };
      case "\x1b[3~": return { name: "delete", ctrl: false, shift: false, alt: false };
      case "\x1b[5~": return { name: "pageup", ctrl: false, shift: false, alt: false };
      case "\x1b[6~": return { name: "pagedown", ctrl: false, shift: false, alt: false };
      case "\x1b":   return { name: "escape", ctrl: false, shift: false, alt: false };
      case "\x1b[Z": return { name: "shift+tab", ctrl: false, shift: true, alt: false };
      default: {
        // Shift+arrows (TERMINFO-style)
        if (seq === "\x1b[1;2A") return { name: "shift+up", ctrl: false, shift: true, alt: false };
        if (seq === "\x1b[1;2B") return { name: "shift+down", ctrl: false, shift: true, alt: false };
        if (seq === "\x1b[1;2C") return { name: "shift+right", ctrl: false, shift: true, alt: false };
        if (seq === "\x1b[1;2D") return { name: "shift+left", ctrl: false, shift: true, alt: false };
        // Ctrl+arrows
        if (seq === "\x1b[1;5A") return { name: "ctrl+up", ctrl: true, shift: false, alt: false };
        if (seq === "\x1b[1;5B") return { name: "ctrl+down", ctrl: true, shift: false, alt: false };
        if (seq === "\x1b[1;5C") return { name: "ctrl+right", ctrl: true, shift: false, alt: false };
        if (seq === "\x1b[1;5D") return { name: "ctrl+left", ctrl: true, shift: false, alt: false };
        return null;
      }
    }
  }

  private handleKey(key: KeyEvent): void {
    // Escape closes overlay
    if (key.name === "escape") {
      if (this.overlay) {
        this.overlay = null;
        this.render();
      }
      return;
    }

    if (this.overlay?.kind === "select") {
      if (key.name === "enter") {
        const item = this.overlay.component.getSelectedItem();
        if (item) this.overlay.component.handleKey("enter");
      } else {
        this.overlay.component.handleKey(key.name);
      }
      this.render();
      return;
    }
  }

  private handleInterrupt(): void {
    if (this.streaming) {
      this.client.abort();
      this.streaming = false;
      this.messages.push({ role: "system", content: "(aborted)" });
      this.render();
    } else {
      this.stop();
      process.exit(0);
    }
  }

  // ─── Submit ─────────────────────────────────────────────────────────────

  private async handleSubmit(value: string): Promise<void> {
    this.messages.push({ role: "user", content: value });
    this.streaming = true;
    this.render();

    try {
      await this.client.prompt(value);
      this.messages.push({ role: "assistant", content: "(response complete)" });
    } catch (err) {
      this.messages.push({ role: "system", content: `Error: ${err}` });
    }

    this.streaming = false;
    this.render();
  }

  private handleOverlayInputSubmit(value: string): void {
    this.overlay = null;
    this.render();
  }

  // ─── Actions ─────────────────────────────────────────────────────────────

  private async refresh(): Promise<void> {
    try {
      const state = await this.client.getState();
      this.model = state.model ?? "(no model)";
      this.thinkingLevel = state.thinkingLevel;
    } catch {
      this.model = "(not connected)";
    }
  }

  private async cycleModel(): Promise<void> {
    try {
      const r = await this.client.cycleModel();
      if (r) {
        this.model = r.model;
        this.thinkingLevel = r.thinkingLevel;
      }
    } catch { /* ignore */ }
    this.render();
  }

  // ─── Overlays ───────────────────────────────────────────────────────────

  async showSessions(): Promise<void> {
    try {
      const { sessions } = await this.client.listSessions();
      const items: SelectItem[] = sessions.map((s) => ({
        value: s.id,
        label: s.name ?? s.id,
        description: `${s.model} · ${new Date(s.updated_at).toLocaleString()}`,
      }));

      const sl = new SelectList({
        title: "Sessions",
        items,
        maxVisible: 15,
        onSelect: async (item) => {
          await this.client.switchSession(item.value);
          await this.refresh();
          this.overlay = null;
          this.render();
        },
        onCancel: () => {
          this.overlay = null;
          this.render();
        },
      });

      this.overlay = { kind: "select", title: "Sessions", component: sl };
      this.render();
    } catch (err) {
      this.messages.push({ role: "system", content: `Failed to load sessions: ${err}` });
      this.render();
    }
  }

  async showSettings(): Promise<void> {
    const items: SelectItem[] = [
      { value: "model", label: "Model", description: this.model },
      { value: "thinking", label: "Thinking", description: this.thinkingLevel },
      { value: "sessions", label: "Sessions", description: "Browse sessions" },
      { value: "reload", label: "Reload", description: "Reload settings" },
    ];

    const sl = new SelectList({
      title: "Settings",
      items,
      maxVisible: 10,
      onSelect: async (item) => {
        this.overlay = null;
        if (item.value === "sessions") {
          await this.showSessions();
        } else if (item.value === "reload") {
          await this.refresh();
          this.messages.push({ role: "system", content: "Settings reloaded" });
        }
        this.render();
      },
      onCancel: () => {
        this.overlay = null;
        this.render();
      },
    });

    this.overlay = { kind: "select", title: "Settings", component: sl };
    this.render();
  }

  // ─── Rendering ───────────────────────────────────────────────────────────

  private render(): void {
    const W = this.terminal.getWidth();
    const H = this.terminal.getHeight();
    const CHAT_HEIGHT = H - 3; // footer + editor

    let out = CLEAR + cursorPos(1, 1);

    // Chat area (scrollable)
    out += this.renderChat(W, CHAT_HEIGHT);

    // Overlay (if any)
    if (this.overlay?.kind === "select") {
      const lines = this.overlay.component.render(W);
      const top = Math.floor((H - lines.length) / 2);
      const left = Math.floor((W - 70) / 2);
      for (let i = 0; i < lines.length; i++) {
        out += cursorPos(top + i, Math.max(1, left)) + CLEAR_LINE + lines[i];
      }
    }

    // Editor / input line
    out += cursorPos(H - 1, 1) + CLEAR_LINE;
    const editorLines = this.editor.render(W);
    if (editorLines.length > 0) {
      out += editorLines[0];
    }

    // Footer
    out += this.renderFooter(W, H);

    this.terminal.write(out);
  }

  private renderChat(W: number, maxHeight: number): string {
    const lines: string[] = [];

    // Header
    lines.push(
      `${CSI}38;5;${this.theme.accent}m${BOLD} xihu${RESET}` +
      `  ${CSI}2m${this.model}${RESET}` +
      `  thinking: ${CSI}2m${this.thinkingLevel}${RESET}`
    );
    lines.push("─".repeat(Math.min(W, 60)));

    // Messages (most recent last)
    const recent = this.messages.slice(-(maxHeight - 4));
    for (const msg of recent) {
      const msgLines = renderChatMessage(msg, W - 2);
      lines.push(...msgLines);
    }

    if (this.streaming) {
      lines.push(`${CSI}38;5;${this.theme.accent}m◐ Thinking...${RESET}`);
    }

    // Truncate if too tall
    const displayLines = lines.slice(0, maxHeight);
    let out = "";
    for (let i = 0; i < displayLines.length; i++) {
      out += cursorPos(i + 1, 1) + CLEAR_LINE + displayLines[i];
    }
    return out;
  }

  private renderFooter(W: number, H: number): string {
    const streaming = this.streaming ? "◐" : "";
    const overlay = this.overlay ? `[ESC] ` : "";

    const left = ` ${this.model} · ${this.thinkingLevel} ${streaming}`;
    const right = `${overlay}`;
    const footer = left + " ".repeat(Math.max(1, W - left.length - right.length - 1)) + right;

    return cursorPos(H, 1) + CLEAR_LINE +
      `${CSI}48;5;${this.theme.bg}m${CSI}38;5;${this.theme.dimFg}m${footer}${RESET}`;
  }
}
