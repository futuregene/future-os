/**
 * xihu TUI - Main application.
 * Complete terminal UI for xihu agent.
 */

import { RpcClient } from "./rpc/client.js";
import { ChatArea, type ChatMessage } from "./components/chat-area.js";
import { Footer, type FooterData } from "./components/footer.js";
import { SelectList, type SelectItem } from "./components/select-list.js";
import { AutocompletePopup, type AutocompleteItem } from "./components/autocomplete.js";
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
  MOUSE_ON,
  MOUSE_OFF,
  parseMouseEvent,
} from "./tui.js";
import { DARK_THEME, type Theme, fg, dim, bold } from "./theme.js";

type Overlay =
  | { kind: "select"; title: string; component: SelectList }
  | { kind: "help" }
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
  private autocomplete = new AutocompletePopup();
  private escBuf = "";

  // Slash commands for autocomplete
  private readonly slashCommands: AutocompleteItem[] = [
    { value: "/model", label: "/model", description: "select model" },
    { value: "/sessions", label: "/sessions", description: "browse sessions" },
    { value: "/settings", label: "/settings", description: "open settings" },
    { value: "/new", label: "/new", description: "new session" },
    { value: "/clone", label: "/clone", description: "clone session" },
    { value: "/fork", label: "/fork", description: "fork session" },
    { value: "/tree", label: "/tree", description: "session tree" },
    { value: "/thinking", label: "/thinking", description: "toggle thinking level" },
    { value: "/name", label: "/name", description: "set session name" },
    { value: "/scoped-models", label: "/scoped-models", description: "manage scoped models" },
    { value: "/theme", label: "/theme", description: "change theme" },
    { value: "/changelog", label: "/changelog", description: "show changelog" },
    { value: "/help", label: "/help", description: "show help" },
    { value: "/hotkeys", label: "/hotkeys", description: "show shortcuts" },
    { value: "/quit", label: "/quit", description: "quit xihu" },
  ];

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
    tokensIn: 0,
    tokensOut: 0,
    totalCost: 0,
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
      onChange: (v) => this.triggerAutocomplete(),
    });

    // Subscribe to SSE events
    this.client.subscribe((event) => {
      this.handleAgentEvent(event);
    });
  }

  // ─── Lifecycle ────────────────────────────────────────────────────────────

  async start(): Promise<void> {
    this.terminal.enterAlternateScreen();
    this.terminal.write(MOUSE_ON);
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
    this.terminal.write(MOUSE_OFF);
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

      case "usage": {
        const e = event as { input_tokens?: number; output_tokens?: number };
        if (e.input_tokens !== undefined) this.state.tokensIn += e.input_tokens;
        if (e.output_tokens !== undefined) this.state.tokensOut += e.output_tokens;
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
      // Set a timeout - if no more chars arrive, treat as standalone Escape
      setTimeout(() => {
        if (this.escBuf === "\x1b") {
          this.escBuf = "";
          this.handleKey({ name: "escape", ctrl: false, shift: false, alt: false });
        }
      }, 50);
      return;
    }

    // In escape sequence
    if (this.escBuf.length > 0) {
      this.escBuf += char;
      // Check for mouse event: ESC [ M <btn> <x> <y>
      if (this.escBuf.startsWith("\x1b[M")) {
        if (this.escBuf.length >= 6) {
          const mouse = parseMouseEvent(this.escBuf);
          this.escBuf = "";
          if (mouse) this.handleMouseEvent(mouse);
          return;
        }
        if (this.escBuf.length > 10) this.escBuf = "";
        return;
      }
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
    // Escape - close overlay or autocomplete or clear editor
    if (key.name === "escape") {
      if (this.autocomplete.isVisible()) {
        this.autocomplete.hide();
        this.render();
      } else if (this.overlay) {
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

    // Autocomplete navigation takes priority over chat scroll
    if (this.autocomplete.isVisible()) {
      if (key.name === "up") {
        this.autocomplete.selectPrev();
        this.render();
        return;
      }
      if (key.name === "down") {
        this.autocomplete.selectNext();
        this.render();
        return;
      }
      if (key.name === "enter") {
        const item = this.autocomplete.getSelectedItem();
        if (item) {
          this.editor.setValue(item.value);
          this.autocomplete.hide();
          this.render();
          // Submit the command (don't await - let it run async)
          this.handleSubmit(item.value);
        }
        return;
      }
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
        case "o":
          this.overlay = { kind: "help" };
          this.render();
          break;
        case "t":
          this.cycleThinking();
          break;
        default:
          // Pass to editor
          if (this.editor.handleKey(key)) {
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

    // Tab - autocomplete
    if (key.name === "tab") {
      if (this.autocomplete.isVisible()) {
        // Accept selected autocomplete
        const item = this.autocomplete.getSelectedItem();
        if (item) {
          this.editor.setValue(item.value);
          this.autocomplete.hide();
          this.render();
          // Submit the command
          this.handleSubmit(item.value);
        }
        return;
      } else {
        // Trigger autocomplete for slash commands
        this.triggerAutocomplete();
      }
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

  // ─── Autocomplete ───────────────────────────────────────────────

  private triggerAutocomplete(): void {
    const text = this.editor.getValue();

    // Detect slash command prefix
    if (text.startsWith("/")) {
      const prefix = text.slice(1).toLowerCase();
      const filtered = this.slashCommands.filter((cmd) =>
        cmd.label.toLowerCase().includes(prefix)
      );
      if (filtered.length > 0) {
        this.autocomplete.show(filtered);
      } else {
        this.autocomplete.hide();
      }
    } else {
      this.autocomplete.hide();
    }
  }

  private charToKeyEvent(char: string): KeyEvent | null {
    const code = char.charCodeAt(0);
    if (code === 13) return { name: "enter", ctrl: false, shift: false, alt: false };
    if (code === 9)  return { name: "tab", ctrl: false, shift: false, alt: false };
    if (code === 127 || code === 8) return { name: "backspace", ctrl: false, shift: false, alt: false };
    // Ctrl shortcuts
    if (code === 1) return { name: "a", ctrl: true, shift: false, alt: false };
    if (code === 5) return { name: "e", ctrl: true, shift: false, alt: false };
    if (code === 15) return { name: "o", ctrl: true, shift: false, alt: false };
    if (code === 21) return { name: "u", ctrl: true, shift: false, alt: false };
    if (code === 23) return { name: "w", ctrl: true, shift: false, alt: false };
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

  private handleMouseEvent(event: { button: number; x: number; y: number }): void {
    // Wheel up = button 64, wheel down = button 65
    if (event.button === 64) {
      this.chat.scrollUp(3);
      this.render();
    } else if (event.button === 65) {
      this.chat.scrollDown(3);
      this.render();
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
    // Handle slash commands locally (don't send to LLM)
    if (value.startsWith("/")) {
      const parts = value.slice(1).split(/\s+/);
      const cmd = parts[0].toLowerCase();
      const arg = parts.slice(1).join(" ");

      if (cmd === "model") {
        if (arg) {
          // Set model directly
          try {
            await this.client.setModel(arg);
            this.state.model = arg;
            this.chat.addMessage({
              id: crypto.randomUUID(),
              role: "system",
              content: `Model: ${arg}`,
            });
          } catch (err) {
            this.chat.addMessage({
              id: crypto.randomUUID(),
              role: "system",
              content: `Failed to set model: ${err}`,
            });
          }
        } else {
          // Show model selector
          await this.showModelSelector();
        }
        return;
      }

      if (cmd === "sessions") {
        await this.showSessions();
        return;
      }

      if (cmd === "help" || cmd === "hotkeys") {
        this.overlay = { kind: "help" };
        this.render();
        return;
      }

      if (cmd === "compact") {
        try {
          await this.client.compact();
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: "Context compacted",
          });
        } catch (err) {
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: "Compact failed: " + err,
          });
        }
        return;
      }

      if (cmd === "export") {
        this.chat.addMessage({
          id: crypto.randomUUID(),
          role: "system",
          content: "Export: use /settings or Ctrl+E to export session",
        });
        return;
      }

      if (cmd === "import") {
        this.chat.addMessage({
          id: crypto.randomUUID(),
          role: "system",
          content: "Import: use /settings or Ctrl+I to import session",
        });
        return;
      }
    }

    // Regular prompt - send to server
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
      this.state.tokensIn = s.tokensIn ?? 0;
      this.state.tokensOut = s.tokensOut ?? 0;
      this.state.totalCost = s.totalCost ?? 0;
    } catch {
      this.state.model = "(not connected)";
    }
  }

  async showModelSelector(): Promise<void> {
    let models: string[] = [];
    try {
      const r = await this.client.getAvailableModels();
      models = r.models;
    } catch (err) {
      this.chat.addMessage({
        id: crypto.randomUUID(),
        role: "system",
        content: `Failed to load models: ${err}`,
      });
      return;
    }

    const items: SelectItem[] = models.map((m) => ({
      value: m,
      label: m,
      description: m === this.state.model ? "current" : "",
    }));

    const sl = new SelectList({
      title: "Select Model",
      items,
      maxVisible: 15,
      onSelect: async (item) => {
        try {
          await this.client.setModel(item.value);
          this.state.model = item.value;
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Model: ${item.label}`,
          });
        } catch (err) {
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Failed to set model: ${err}`,
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

    this.overlay = { kind: "select", title: "Select Model", component: sl };
    this.render();
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

    // ── Chat ──
    const chatLines = this.chat.render();
    for (let i = 0; i < chatLines.length; i++) {
      out += cursorPos(i + 1, 1) + CLEAR_LINE + chatLines[i];
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

    // Help overlay
    if (this.overlay?.kind === "help") {
      const helpLines = this.renderHelp(W);
      const helpH = helpLines.length + 4;
      const top = Math.floor((chatHeight - helpH) / 2) + 2;

      for (let i = 0; i < helpH; i++) {
        out += cursorPos(top + i, 1) + CLEAR_LINE;
      }
      for (let i = 0; i < helpLines.length; i++) {
        out += cursorPos(top + 2 + i, 2) + helpLines[i];
      }
    }

    // ── Editor ──
    const editorLine = H - 1;
    out += cursorPos(editorLine, 1) + CLEAR_LINE;
    const editorText = this.editor.render(W);
    if (editorText.length > 0) {
      out += editorText[0];
    }

    // ── Autocomplete popup ──
    if (this.autocomplete.isVisible()) {
      const autocompleteLines = this.autocomplete.render(W);
      if (autocompleteLines.length > 0) {
        let acTop = editorLine - this.autocomplete.height() - 1;
        for (const line of autocompleteLines) {
          out += cursorPos(acTop, 1) + CLEAR_LINE + line;
          acTop++;
        }
      }
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
      tokensIn: this.state.tokensIn,
      tokensOut: this.state.tokensOut,
      totalCost: this.state.totalCost,
      autoCompactionEnabled: true,
    };
    out += cursorPos(H, 1) + CLEAR_LINE + this.footer.render(footerData);

    this.terminal.write(out);
  }

  private renderHelp(W: number): string[] {
    const lines: string[] = [];
    const dim_ = (t: string) => fg(245, t);
    const acc = (t: string) => fg(151, t);
    const bold_ = (t: string) => fg(252, bold(t));

    // Two-column layout fitting terminal width W
    const innerW = W - 4;
    const leftCol = [
      acc("Shortcuts:"),
      dim_("  ctrl+o  show this help"),
      dim_("  ctrl+c  interrupt"),
      dim_("  ctrl+d  clear / exit"),
      dim_("  ctrl+l  clear screen"),
      dim_("  ctrl+p  cycle model"),
      dim_("  ctrl+r  browse sessions"),
      dim_("  ctrl+t  cycle thinking"),
      dim_("  tab     autocomplete"),
      dim_("  \u2191\u2193    scroll / navigate"),
      dim_("  enter   submit / accept"),
      dim_("  escape  close popup"),
    ];
    const rightCol = [
      acc("/commands:"),
      dim_("  /model [name]  select model"),
      dim_("  /sessions   browse sessions"),
      dim_("  /new       new session"),
      dim_("  /settings  open settings"),
      dim_("  /compact   compact context"),
      dim_("  /clone     clone session"),
      dim_("  /fork      fork session"),
      dim_("  /tree      session tree"),
      dim_("  /thinking  toggle thinking"),
      dim_("  /name [n]  set session name"),
      dim_("  /help /hotkeys /quit"),
    ];

    const colW = Math.floor(innerW / 2);
    const maxRows = Math.max(leftCol.length, rightCol.length);

    lines.push(dim_("\u250c" + "\u2500".repeat(W - 2) + "\u2510"));
    lines.push(dim_("\u2502") + "  " + bold_("xihu") + "  " + dim_("Terminal UI Help") + " ".repeat(Math.max(0, W - 24)) + dim_("\u2502"));
    lines.push(dim_("\u251c" + "\u2500".repeat(W - 2) + "\u2524"));

    for (let i = 0; i < maxRows; i++) {
      const l = leftCol[i] || "";
      const r = rightCol[i] || "";
      const ls = l.replace(/\x1b\[[0-9;]*m/g, "");
      const rs = r.replace(/\x1b\[[0-9;]*m/g, "");
      const lPad = colW - ls.length;
      const rPad = colW - rs.length;
      lines.push(dim_("\u2502") + "  " + l + " ".repeat(Math.max(1, lPad)) + r + " ".repeat(Math.max(1, rPad)) + dim_("\u2502"));
    }

    lines.push(dim_("\u2514" + "\u2500".repeat(W - 2) + "\u2518"));
    return lines;
  }

}
