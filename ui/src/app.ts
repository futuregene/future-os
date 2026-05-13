/**
 * xihu TUI - Main application.
 * Complete terminal UI for xihu agent.
 */

import { RpcClient } from "./rpc/client.js";
import { ChatArea, type ChatMessage } from "./components/chat-area.js";
import { Footer, type FooterData } from "./components/footer.js";
import { SelectList, type SelectItem } from "./components/select-list.js";
import { Editor } from "./components/editor.js";
import {
  NodeTerminal,
  CSI,
  RESET,
  CLEAR,
  CLEAR_LINE,
  cursorPos,
  CURSOR_HIDE,
  ALT_SCREEN_ON,
  ALT_SCREEN_OFF,
  BOLD,
} from "./tui.js";
import { DARK_THEME, type Theme, fg, dim, bold } from "./theme.js";

type Overlay =
  | { kind: "select"; title: string; component: SelectList }
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
  private chat: ChatArea;
  private footer: Footer;
  private overlay: Overlay = null;
  private escBuf = "";

  private state = {
    model: "",
    thinking: "off",
    streaming: false,
    sessionName: "",
    cwd: "",
    version: "",
    skills: [] as string[],
    contextFiles: [] as string[],
    extensions: [] as string[],
    contextTokens: 0,
    contextWindow: 0,
    contextPercent: 0,
    thinkingHidden: false,  // true = show "Thinking..." instead of actual thinking
  };

  private running = false;

  constructor(private serverUrl = "http://localhost:7890") {
    this.terminal = new NodeTerminal();
    this.client = new RpcClient(serverUrl);
    this.theme = DARK_THEME;
    this.chat = new ChatArea(this.terminal.getWidth());
    this.footer = new Footer(this.terminal.getWidth());

    this.editor = new Editor("❯ ", {
      prompt: this.theme.accent,
      text: this.theme.fg,
      cursor: this.theme.accent,
      bg: this.theme.bg,
    }, {
      onSubmit: (v) => this.handleSubmit(v),
    });

    // Subscribe to SSE events
    this.client.subscribe((event) => {
      this.handleAgentEvent(event);
    });
  }

  // ─── Lifecycle ────────────────────────────────────────────────────────────

  async start(): Promise<void> {
    this.terminal.enterAlternateScreen();
    this.terminal.hideCursor();

    process.stdin.resume();
    process.stdin.setRawMode!(true);
    process.stdin.setEncoding("utf-8");

    this.running = true;

    process.stdin.on("data", (chunk: string) => {
      for (const char of chunk) {
        this.handleChar(char);
      }
    });

    await this.refresh();
    this.showWelcome();
    this.render();

    await new Promise<void>((resolve) => {
      process.stdin.on("SIGINT", () => resolve());
    });
  }

  async stop(): Promise<void> {
    this.running = false;
    process.stdin.setRawMode!(false);
    this.terminal.exitAlternateScreen();
    this.terminal.write(CLEAR + cursorPos(1, 1) + CURSOR_HIDE);
    this.terminal.close();
  }

  // ─── SSE Events ─────────────────────────────────────────────────────────

  private handleAgentEvent(event: { type: string; [key: string]: unknown }): void {
    switch (event.type) {
      case "text_chunk":
        this.state.streaming = true;
        this.chat.updateLastMessage(
          ((event as { text?: string }).text ?? "")
        );
        break;

      case "agent_end": {
        this.state.streaming = false;
        const e = event as { text?: string };
        if (e.text && this.chat) {
          this.chat.updateLastMessage(e.text);
        }
        break;
      }

      case "agent_start":
        this.state.streaming = true;
        this.chat.addMessage({
          id: crypto.randomUUID(),
          role: "assistant",
          content: "",
        });
        break;

      case "thinking_start":
        this.state.streaming = true;
        this.chat.startThinking();
        break;

      case "thinking_delta": {
        const e = event as { text?: string };
        this.chat.appendThinkingDelta(e.text ?? "");
        break;
      }

      case "thinking_end":
        this.chat.endThinking();
        break;

      case "tool_start": {
        const e = event as { tool_id?: string; tool_name?: string };
        this.chat.addToolStart(e.tool_id ?? "", e.tool_name ?? "");
        break;
      }

      case "tool_delta": {
        const e = event as { tool_id?: string; text?: string };
        this.chat.appendToolDelta(e.tool_id ?? "", e.text ?? "");
        break;
      }

      case "tool_end": {
        const e = event as { tool_id?: string; text?: string };
        this.chat.finishTool(e.tool_id ?? "", e.text);
        break;
      }

      case "error": {
        this.state.streaming = false;
        const e = event as { error_message?: string };
        this.chat.addMessage({
          id: crypto.randomUUID(),
          role: "system",
          content: `Error: ${e.error_message ?? "unknown"}`,
        });
        break;
      }

      default:
        break;
    }
    this.render();
  }

  // ─── Input Handling ─────────────────────────────────────────────────────

  private handleChar(char: string): void {
    const code = char.charCodeAt(0);

    // Escape sequence start
    if (char === "\x1b") {
      this.escBuf = "\x1b";
      return;
    }

    // In escape sequence
    if (this.escBuf.length > 0) {
      this.escBuf += char;
      const key = this.parseEscSeq(this.escBuf);
      if (key) {
        this.escBuf = "";
        this.handleKey(key);
        return;
      }
      if (this.escBuf.length > 8) {
        this.escBuf = "";
      }
      return;
    }

    // Ctrl+C
    if (code === 3) {
      this.handleInterrupt();
      return;
    }

    // Overlay mode
    if (this.overlay?.kind === "select") {
      const key = this.charToKeyEvent(char);
      if (key) {
        this.overlay.component.handleKey(key.name);
        this.render();
      }
      return;
    }

    // Global key
    const key = this.charToKeyEvent(char);
    if (key) {
      this.handleKey(key);
    } else if (code >= 32) {
      this.editor.insertText(char);
      this.render();
    }
  }

  private handleKey(key: KeyEvent): void {
    // Escape - close overlay or clear editor
    if (key.name === "escape") {
      if (this.overlay) {
        this.overlay = null;
        this.render();
      } else {
        this.editor.setValue("");
        this.render();
      }
      return;
    }

    // Overlay keys
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

    // Navigation
    if (key.name === "pageup") {
      this.chat.scrollUp(10);
      this.render();
      return;
    }
    if (key.name === "pagedown") {
      this.chat.scrollDown(10);
      this.render();
      return;
    }
    if (key.name === "up" && !key.ctrl && !key.shift) {
      this.chat.scrollUp(3);
      this.render();
      return;
    }
    if (key.name === "down" && !key.ctrl && !key.shift) {
      this.chat.scrollDown(3);
      this.render();
      return;
    }

    // Ctrl shortcuts
    if (key.ctrl) {
      switch (key.name) {
        case "c":
          this.handleInterrupt();
          break;
        case "l":
          this.chat.clearMessages();
          this.render();
          break;
        case "p":
          this.cycleModel();
          break;
        case "r":
          this.showSessions();
          break;
        case "s":
          this.showSettings();
          break;
        case "t":
          this.cycleThinking();
          break;
        case "o":
          this.toggleThinking();
          break;
        default:
          // Pass to editor
          if (this.editor.handleKey({ name: key.name, ctrl: true, shift: false, alt: false })) {
            this.render();
          }
          break;
      }
      return;
    }

    // Shift+Tab - cycle thinking
    if (key.name === "shift+tab") {
      this.cycleThinking();
      return;
    }

    // Tab - autocomplete (future)
    if (key.name === "tab") {
      return;
    }

    // Enter - submit
    if (key.name === "enter") {
      // handled by editor
    }

    // Editor handles the rest
    if (this.editor.handleKey({ name: key.name, ctrl: key.ctrl, shift: key.shift, alt: key.alt })) {
      this.render();
    }
  }

  private charToKeyEvent(char: string): KeyEvent | null {
    const code = char.charCodeAt(0);
    if (code === 13) return { name: "enter", ctrl: false, shift: false, alt: false };
    if (code === 9)  return { name: "tab", ctrl: false, shift: false, alt: false };
    if (code === 127 || code === 8) return { name: "backspace", ctrl: false, shift: false, alt: false };
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
      // Ctrl+arrows
      case "\x1b[1;5A": return { name: "up", ctrl: true, shift: false, alt: false };
      case "\x1b[1;5B": return { name: "down", ctrl: true, shift: false, alt: false };
      case "\x1b[1;5C": return { name: "right", ctrl: true, shift: false, alt: false };
      case "\x1b[1;5D": return { name: "left", ctrl: true, shift: false, alt: false };
      default: return null;
    }
  }

  private handleInterrupt(): void {
    if (this.state.streaming) {
      this.client.abort();
      this.state.streaming = false;
      this.chat.addMessage({
        id: crypto.randomUUID(),
        role: "system",
        content: "(aborted)",
      });
      this.render();
    } else {
      this.stop();
      process.exit(0);
    }
  }

  // ─── Actions ────────────────────────────────────────────────────────────

  private async handleSubmit(value: string): Promise<void> {
    this.chat.addMessage({
      id: crypto.randomUUID(),
      role: "user",
      content: value,
    });
    this.state.streaming = true;
    this.render();

    try {
      await this.client.prompt(value);
    } catch (err) {
      this.state.streaming = false;
      this.chat.addMessage({
        id: crypto.randomUUID(),
        role: "system",
        content: `Error: ${err}`,
      });
    }
    this.render();
  }

  // Wrap long welcome text to maxWidth characters, applying color to each line.
  private wrapWelcomeLine(text: string, color: (t: string) => string, maxWidth: number): string[] {
    const plain = text.replace(/\x1b\[[0-9;]*m/g, "");
    if (plain.length <= maxWidth) return [color(text)];
    const words = plain.split(" ");
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
    // Apply color to each line
    return lines.map((l) => color(l));
  }

  // ─── Welcome Screen ─────────────────────────────────────────────────

  private showWelcome(): void {
    const dim_ = (t: string) => fg(245, t);
    const sectionHdr = (t: string) => fg(221, t);
    const add = (content: string) => {
      this.chat.addMessage({ id: crypto.randomUUID(), role: "system", welcome: true, content });
    };
    const addColored = (content: string, color: (t: string) => string) => {
      this.chat.addMessage({ id: crypto.randomUUID(), role: "system", welcome: true, content: color(content) });
    };

    // Banner: "xihu vX.X.X"
    const version = this.state.version || "0.3.0";
    addColored(`${fg(151, bold("xihu"))}${fg(245, " v" + version)}`, (t) => t);

    // Shortcuts line
    addColored("escape interrupt · ctrl+c/ctrl+d clear/exit · / commands · ! bash · ctrl+o more", dim_);

    // Expand hint
    addColored("Press ctrl+o to show full startup help and loaded resources.", dim_);

    // Onboarding
    addColored("Xihu can explain its own features and look up its docs. Ask it how to use or extend Xihu.", dim_);

    // Context files
    if (this.state.contextFiles.length > 0) {
      add("");
      addColored("[Context]", sectionHdr);
      const shortNames = this.state.contextFiles.map((f) => {
        const parts = f.split("/");
        return parts[parts.length - 1] ?? f;
      });
      addColored(" " + shortNames.join(", "), dim_);
    }

    // Skills
    if (this.state.skills.length > 0) {
      add("");
      addColored("[Skills]", sectionHdr);
      for (const line of this.wrapWelcomeLine(" " + this.state.skills.join(", "), dim_, 78)) {
        addColored(line, dim_);
      }
    }

    // Extensions
    if (this.state.extensions.length > 0) {
      add("");
      addColored("[Extensions]", sectionHdr);
      for (const line of this.wrapWelcomeLine(" " + this.state.extensions.join(", "), dim_, 78)) {
        addColored(line, dim_);
      }
    }

    // Separator line
    addColored("─".repeat(Math.min(80, 70)), dim_);
  }

  private async refresh(): Promise<void> {
    try {
      const s = await this.client.getState();
      this.state.model = s.model ?? "(no model)";
      this.state.thinking = s.thinkingLevel;
      this.state.sessionName = s.sessionName ?? "";
      this.state.cwd = s.cwd ?? "";
      this.state.version = s.version ?? "";
      this.state.skills = s.skills ?? [];
      this.state.contextFiles = s.contextFiles ?? [];
      this.state.extensions = s.extensions ?? [];
      this.state.contextTokens = s.contextTokens ?? 0;
      this.state.contextWindow = s.contextWindow ?? 0;
      this.state.contextPercent = s.contextPercent ?? 0;
    } catch {
      this.state.model = "(not connected)";
    }
  }

  private async cycleModel(): Promise<void> {
    try {
      const r = await this.client.cycleModel();
      if (r) {
        this.state.model = r.model;
        this.state.thinking = r.thinkingLevel;
      }
    } catch { /* ignore */ }
    this.render();
  }

  private toggleThinking(): void {
    this.state.thinkingHidden = !this.state.thinkingHidden;
    this.chat.setThinkingHidden(this.state.thinkingHidden);
    this.render();
  }

  private async cycleThinking(): Promise<void> {
    try {
      const r = await this.client.cycleThinkingLevel();
      if (r) this.state.thinking = r.level;
    } catch { /* ignore */ }
    this.render();
  }

  // ─── Overlays ───────────────────────────────────────────────────────────

  async showSessions(): Promise<void> {
    let sessions: { id: string; name?: string; model: string; updated_at: string }[] = [];
    try {
      const r = await this.client.listSessions();
      sessions = r.sessions;
    } catch (err) {
      this.chat.addMessage({
        id: crypto.randomUUID(),
        role: "system",
        content: `Failed to load sessions: ${err}`,
      });
      return;
    }

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
        try {
          await this.client.switchSession(item.value);
          await this.refresh();
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Switched to session: ${item.label}`,
          });
        } catch (err) {
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Failed to switch session: ${err}`,
          });
        }
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
  }

  async showSettings(): Promise<void> {
    const items: SelectItem[] = [
      { value: "reload", label: "Reload", description: "Reload settings and restart" },
      { value: "model", label: "Model", description: this.state.model },
      { value: "thinking", label: "Thinking", description: this.state.thinking },
      { value: "sessions", label: "Sessions", description: "Browse sessions" },
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
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: "Settings reloaded",
          });
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
    const footerHeight = 1;
    const editorHeight = 1;
    const chatHeight = H - footerHeight - editorHeight;

    // Update dimensions
    this.chat.setWidth(W);
    this.chat.setViewportHeight(chatHeight);
    this.footer.setWidth(W);

    let out = CLEAR + cursorPos(1, 1);

    // ── Header ──
    out += this.renderHeader(W);

    // ── Chat ──
    const chatLines = this.chat.render();
    for (let i = 0; i < chatLines.length; i++) {
      out += cursorPos(i + 3, 1) + CLEAR_LINE + chatLines[i];
    }

    // ── Overlay ──
    if (this.overlay?.kind === "select") {
      const lines = this.overlay.component.render(W);
      const overlayH = lines.length + 2;
      const top = Math.floor((chatHeight - overlayH) / 2) + 2;
      const left = Math.max(1, Math.floor((W - Math.min(W - 4, 70)) / 2));

      // Dim background
      for (let i = 0; i < overlayH; i++) {
        out += cursorPos(top + i, 1) + CLEAR_LINE;
      }
      // Content
      for (let i = 0; i < lines.length; i++) {
        out += cursorPos(top + 1 + i, left) + lines[i];
      }
    }

    // ── Editor ──
    const editorLine = H - 1;
    out += cursorPos(editorLine, 1) + CLEAR_LINE;
    const editorText = this.editor.render(W);
    if (editorText.length > 0) {
      out += editorText[0];
    }

    // ── Footer ──
    const footerData: FooterData = {
      cwd: this.state.cwd,
      model: this.state.model,
      thinking: this.state.thinking,
      streaming: this.state.streaming,
      sessionName: this.state.sessionName,
      contextTokens: this.state.contextTokens,
      contextWindow: this.state.contextWindow,
      contextPercent: this.state.contextPercent,
    };
    out += cursorPos(H, 1) + CLEAR_LINE + this.footer.render(footerData);

    this.terminal.write(out);
  }

  private renderHeader(W: number): string {
    // Model + thinking (e.g. "claude-sonnet-4  medium")
    const model = fg(252, this.state.model || "(no model)");
    const thinking = this.state.thinking !== "off"
      ? fg(117, this.state.thinking)
      : fg(240, "off");
    const headerText = `${fg(151, BOLD + "xihu")}  ${model}  ${thinking}`;

    // Separator line
    const sep = dim("─".repeat(Math.min(W, 50)));

    return cursorPos(1, 1) + CLEAR_LINE + headerText +
           cursorPos(2, 1) + CLEAR_LINE + sep;
  }
}
