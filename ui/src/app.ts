/**
 * xihu TUI - Main application.
 */

import * as readline from "node:readline";
import * as fs from "node:fs";
import { RpcClient } from "./rpc/client.js";
import { SelectList, type SelectItem } from "./components/select-list.js";
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
  | { kind: "select"; component: SelectList }
  | { kind: "input"; prompt: string; value: string }
  | null;

export class App {
  private terminal: NodeTerminal;
  private client: RpcClient;
  private theme: Theme;
  private state: {
    messages: ChatMessage[];
    model: string;
    thinkingLevel: string;
    streaming: boolean;
    overlay: Overlay;
    inputValue: string;
    running: boolean;
  };

  constructor(serverUrl = "http://localhost:7890") {
    this.terminal = new NodeTerminal();
    this.client = new RpcClient(serverUrl);
    this.theme = DEFAULT_THEME;
    this.state = {
      messages: [],
      model: "",
      thinkingLevel: "off",
      streaming: false,
      overlay: null,
      inputValue: "",
      running: false,
    };
  }

  // ─── Lifecycle ────────────────────────────────────────────────────────────

  async start(): Promise<void> {
    this.terminal.enterAlternateScreen();
    this.terminal.hideCursor();
    this.state.running = true;

    // Enable raw mode on stdin
    process.stdin.setRawMode!(true);
    process.stdin.resume();
    process.stdin.setEncoding("utf-8");

    // Read keypresses character by character
    process.stdin.on("data", (chunk: string) => {
      for (const char of chunk) {
        this.handleChar(char);
      }
    });

    await this.refresh();
    this.render();

    // Keep running until stopped
    await new Promise(() => {});
  }

  async stop(): Promise<void> {
    this.state.running = false;
    process.stdin.setRawMode!(false);
    this.terminal.exitAlternateScreen();
    this.terminal.showCursor();
    this.terminal.clear();
    this.terminal.close();
  }

  // ─── Input Handling ─────────────────────────────────────────────────────

  private handleChar(char: string): void {
    const code = char.charCodeAt(0);

    // Handle special keys via escape sequences
    if (char === "\x1b") {
      this.escMode = true;
      this.escBuffer = "\x1b";
      return;
    }

    if (this.escMode) {
      this.escBuffer += char;
      if (this.tryHandleEsc()) {
        this.escMode = false;
        this.escBuffer = "";
      }
      return;
    }

    // Ctrl+C
    if (code === 3) {
      this.handleInterrupt();
      return;
    }

    const overlay = this.state.overlay;
    if (overlay?.kind === "input") {
      if (code === 13) {
        // Enter
        this.handleInputSubmit(overlay.value);
      } else if (code === 127 || code === 8) {
        // Backspace
        overlay.value = overlay.value.slice(0, -1);
        this.render();
      } else if (code === 27) {
        // Escape
        this.state.overlay = null;
        this.render();
      } else if (code >= 32) {
        overlay.value += char;
        this.render();
      }
      return;
    }

    if (overlay?.kind === "select") {
      const key = this.charToKey(char, code);
      if (key) {
        const handled = overlay.component.handleKey(key);
        if (handled) this.render();
      }
      return;
    }

    // Global shortcuts
    const key = this.charToKey(char, code);
    if (key) {
      this.handleGlobalKey(key);
    } else if (code >= 32) {
      // Printable character
      this.state.inputValue += char;
    }
  }

  private escMode = false;
  private escBuffer = "";

  private tryHandleEsc(): boolean {
    const buf = this.escBuffer;
    if (buf === "\x1b[A") { this.handleGlobalKey("up"); return true; }
    if (buf === "\x1b[B") { this.handleGlobalKey("down"); return true; }
    if (buf === "\x1b[C") { this.handleGlobalKey("right"); return true; }
    if (buf === "\x1b[D") { this.handleGlobalKey("left"); return true; }
    if (buf === "\x1b[H") { this.handleGlobalKey("home"); return true; }
    if (buf === "\x1b[F") { this.handleGlobalKey("end"); return true; }
    if (buf === "\x1b[3~") { this.handleGlobalKey("delete"); return true; }
    if (buf === "\x1b") { this.handleGlobalKey("escape"); return true; }
    if (buf.length > 3) { return true; } // Unknown escape, ignore
    return false;
  }

  private charToKey(char: string, code: number): string | null {
    if (code === 13) return "enter";
    if (code === 9) return "tab";
    if (code === 127 || code === 8) return "backspace";
    return null;
  }

  private handleGlobalKey(key: string): void {
    switch (key) {
      case "escape":
        if (this.state.overlay) {
          this.state.overlay = null;
          this.render();
        }
        break;
      case "ctrl+p":
        this.cycleModel();
        break;
      case "shift+tab":
        this.cycleThinking();
        break;
      case "ctrl+l":
        this.terminal.clear();
        this.render();
        break;
      case "up":
      case "down":
        // Navigate chat history
        break;
    }
  }

  private handleInterrupt(): void {
    if (this.state.streaming) {
      this.client.abort();
      this.state.streaming = false;
    } else {
      this.stop();
      process.exit(0);
    }
  }

  private async handleInputSubmit(value: string): Promise<void> {
    this.state.overlay = null;
    if (value.trim()) {
      await this.sendPrompt(value);
    }
    this.render();
  }

  // ─── Actions ─────────────────────────────────────────────────────────────

  private async sendPrompt(message: string): Promise<void> {
    this.state.streaming = true;
    this.state.messages.push({ role: "user", content: message });
    this.render();

    try {
      await this.client.prompt(message);
      this.state.streaming = false;
      this.state.messages.push({ role: "assistant", content: "[Response complete]" });
    } catch (err) {
      this.state.streaming = false;
      this.state.messages.push({ role: "system", content: `Error: ${err}` });
    }

    this.render();
  }

  private async cycleModel(): Promise<void> {
    try {
      const result = await this.client.cycleModel();
      if (result) {
        this.state.model = result.model;
        this.state.thinkingLevel = result.thinkingLevel;
      }
    } catch {
      // Ignore
    }
    this.render();
  }

  private async cycleThinking(): Promise<void> {
    try {
      const result = await this.client.cycleThinkingLevel();
      if (result) {
        this.state.thinkingLevel = result.level;
      }
    } catch {
      // Ignore
    }
    this.render();
  }

  private async refresh(): Promise<void> {
    try {
      const state = await this.client.getState();
      this.state.model = state.model ?? "(no model)";
      this.state.thinkingLevel = state.thinkingLevel;
    } catch {
      this.state.model = "(not connected)";
    }
  }

  // ─── Overlays ───────────────────────────────────────────────────────────

  async showSessions(): Promise<void> {
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
        this.state.overlay = null;
        this.render();
      },
      onCancel: () => {
        this.state.overlay = null;
        this.render();
      },
    });

    this.state.overlay = { kind: "select", component: sl };
    this.render();
  }

  async showSettings(): Promise<void> {
    const items: SelectItem[] = [
      { value: "theme", label: "Theme", description: "Change UI colors" },
      { value: "model", label: "Model", description: this.state.model },
      { value: "thinking", label: "Thinking Level", description: this.state.thinkingLevel },
    ];

    const sl = new SelectList({
      title: "Settings",
      items,
      maxVisible: 10,
      onSelect: () => {
        this.state.overlay = null;
        this.render();
      },
      onCancel: () => {
        this.state.overlay = null;
        this.render();
      },
    });

    this.state.overlay = { kind: "select", component: sl };
    this.render();
  }

  // ─── Rendering ───────────────────────────────────────────────────────────

  private render(): void {
    const width = this.terminal.getWidth();
    const height = this.terminal.getHeight();

    let output = CLEAR + cursorPos(1, 1);

    // Chat area
    output += this.renderChat(width);

    // Overlay
    if (this.state.overlay?.kind === "select") {
      const lines = this.state.overlay.component.render(width);
      const top = Math.floor((height - lines.length) / 2);
      for (let i = 0; i < lines.length; i++) {
        output += cursorPos(top + i, 1) + CLEAR_LINE + lines[i];
      }
    } else if (this.state.overlay?.kind === "input") {
      const inputLine = height - 2;
      output += cursorPos(inputLine, 1) + CLEAR_LINE;
      output += `${CSI}38;5;${this.theme.accent}m${this.state.overlay.prompt}: ${RESET}${this.state.overlay.value}_`;
    }

    // Input line (when no overlay)
    if (!this.state.overlay) {
      const inputLine = height - 2;
      output += cursorPos(inputLine, 1) + CLEAR_LINE;
      const prompt = `${CSI}38;5;${this.theme.accent}m❯ ${RESET}`;
      output += prompt + this.state.inputValue + "_";
    }

    // Footer
    output += this.renderFooter(width, height);

    this.terminal.write(output);
  }

  private renderChat(width: number): string {
    const lines: string[] = [];

    // Header
    lines.push(
      `${CSI}38;5;${this.theme.accent}m${BOLD} xihu ${RESET} ${CSI}2m${this.state.model}${RESET}`
    );
    lines.push("─".repeat(Math.min(width, 60)));

    // Messages
    const maxLines = this.terminal.getHeight() - 6;
    const recent = this.state.messages.slice(-maxLines);

    for (const msg of recent) {
      const prefix = msg.role === "user" ? "👤" : msg.role === "assistant" ? "🤖" : "⚙️";
      const content = (msg.content ?? "").split("\n")[0];
      const truncated = content.length > width - 4 ? content.slice(0, width - 7) + "…" : content;
      lines.push(`${prefix} ${CSI}2m${truncated}${RESET}`);
    }

    if (this.state.streaming) {
      lines.push(`${CSI}38;5;${this.theme.accent}m◐ Thinking...${RESET}`);
    }

    let output = "";
    for (let i = 0; i < lines.length; i++) {
      output += cursorPos(i + 1, 1) + CLEAR_LINE + lines[i];
    }
    return output;
  }

  private renderFooter(width: number, height: number): string {
    const model = this.state.model || "(no model)";
    const thinking = this.state.thinkingLevel;
    const streaming = this.state.streaming ? "◐" : "";
    const overlay = this.state.overlay !== null ? "[ESC]" : "";

    const footerText = ` ${model} · thinking: ${thinking} ${streaming} ${overlay} `;
    return cursorPos(height, 1) + CLEAR_LINE +
      `${CSI}48;5;${this.theme.bg}m${CSI}38;5;${this.theme.dimFg}m${footerText}${RESET}`;
  }
}

interface ChatMessage {
  role: "user" | "assistant" | "system" | "tool";
  content?: string;
}
