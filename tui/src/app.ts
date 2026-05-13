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
  SYNC_BEGIN,
  SYNC_END,
  Container,
  type Component,
  type OverlayHandle,
  type OverlayOptions,
  type InputListener,
  type OverlayLayout,
  resolveOverlayLayout,
  isFocusable,
} from "./tui.js";
import { DARK_THEME, type Theme, fg, dim, bold } from "./theme.js";

interface KeyEvent {
  name: string;
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
}

export class App extends Container {
  private terminal: NodeTerminal;
  private client: RpcClient;
  private theme: Theme;
  private editor: Editor;
  private chat: ChatArea;
  private footer: Footer;
  private overlayStack: { component: Component; options?: OverlayOptions; preFocus: Component | null; hidden: boolean; focusOrder: number }[] = [];
  private focusOrderCounter = 0;
  private focusedComponent: Component | null = null;
  private inputListeners = new Set<InputListener>();
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
    explicitSession: false, // true when --session/--continue/--resume/--fork was used
  };

  private running = false;

  // Diff-based render state (matches pi's doRender approach)
  private previousLines: string[] = [];
  private hardwareCursorRow = 0;
  private renderRequested = false;
  private renderTimer: ReturnType<typeof setTimeout> | undefined;
  private lastRenderAt = 0;
  private static readonly MIN_RENDER_INTERVAL_MS = 16;
  private previousWidth = 0;
  private previousHeight = 0;

  constructor(private serverUrl = "http://localhost:7890") {
    super();
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

    // Register children with Container (matches pi's TUI extends Container)
    this.addChild(this.chat);
    this.addChild(this.editor);
    this.addChild(this.footer);

    // Subscribe to SSE events
    this.client.subscribe((event) => {
      this.handleAgentEvent(event);
    });
  }

  // ─── Lifecycle ────────────────────────────────────────────────────────────

  async start(): Promise<void> {
    // Hide cursor immediately (clear + render positioning handled by doRender)
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

    // Trigger re-render on terminal resize so footer stays at bottom
    process.stdout.on("resize", () => {
      this.requestRender();
    });

    await this.refresh();

    // Create a new session if the server auto-generated one (no --session etc).
    // If the user explicitly requested a session via CLI flags, preserve it.
    if (!this.state.explicitSession) {
      try {
        await this.client.newSession();
        await this.refresh();
      } catch {
        // Server may not support new_session — continue with current session
      }
    }

    this.showWelcome();
    this.requestRender();

    await new Promise<void>((resolve) => {
      process.stdin.on("SIGINT", () => resolve());
    });
  }

  async stop(): Promise<void> {
    this.running = false;
    if (this.renderTimer) {
      clearTimeout(this.renderTimer);
      this.renderTimer = undefined;
    }
    this.renderRequested = false;
    process.stdin.setRawMode!(false);
    // Move cursor to end of content (matches pi's stop())
    if (this.previousLines.length > 0) {
      const targetRow = this.previousLines.length;
      const lineDiff = targetRow - this.hardwareCursorRow;
      if (lineDiff > 0) {
        this.terminal.write(`\x1b[${lineDiff}B`);
      } else if (lineDiff < 0) {
        this.terminal.write(`\x1b[${-lineDiff}A`);
      }
      this.terminal.write("\r\n");
    }
    this.terminal.showCursor();
    this.terminal.close();
  }

  // ─── SSE Events ─────────────────────────────────────────────────────────

  private handleAgentEvent(event: { type: string; [key: string]: unknown }): void {
    switch (event.type) {
      case "text_chunk":
        this.state.streaming = true;
        this.chat.appendToLastMessage(
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
    this.requestRender();
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

    // Input listener pipeline
    if (this.inputListeners.size > 0) {
      let data: string | undefined = char;
      for (const listener of this.inputListeners) {
        if (!data) break;
        const result = listener(data);
        if (result?.consume) { data = undefined; break; }
        if (result?.data !== undefined) data = result.data;
      }
      if (!data) return;
    }

    // Overlay mode — dispatch to top overlay
    if (this.overlayStack.length > 0) {
      const top = this.getTopOverlay();
      if (top?.handleInput) {
        const key = this.charToKeyEvent(char);
        if (key) {
          top.handleInput(key.name);
          this.requestRender();
        }
      }
      return;
    }

    // Global key
    const key = this.charToKeyEvent(char);
    if (key) {
      this.handleKey(key);
    } else if (code >= 32) {
      this.editor.insertText(char);
      this.requestRender();
    }
  }

  private handleKey(key: KeyEvent): void {
    // Escape - close autocomplete or overlay or clear editor
    if (key.name === "escape") {
      if (this.autocomplete.isVisible()) {
        this.autocomplete.hide();
        this.requestRender();
      } else if (this.overlayStack.length > 0) {
        this.hideOverlay();
        this.requestRender();
      } else {
        this.editor.setValue("");
        this.requestRender();
      }
      return;
    }

    // Overlay keys — dispatch to top overlay via handleInput
    if (this.overlayStack.length > 0) {
      const top = this.getTopOverlay();
      if (top?.handleInput) {
        top.handleInput(key.name);
      }
      this.requestRender();
      return;
    }

    // Autocomplete navigation takes priority over chat scroll
    if (this.autocomplete.isVisible()) {
      if (key.name === "up") {
        this.autocomplete.selectPrev();
        this.requestRender();
        return;
      }
      if (key.name === "down") {
        this.autocomplete.selectNext();
        this.requestRender();
        return;
      }
      if (key.name === "enter") {
        const item = this.autocomplete.getSelectedItem();
        if (item) {
          this.editor.setValue(item.value);
          this.autocomplete.hide();
          this.requestRender();
          // Submit the command (don't await - let it run async)
          this.handleSubmit(item.value);
        }
        return;
      }
    }

    // Ctrl shortcuts
    if (key.ctrl) {
      switch (key.name) {
        case "c":
          this.handleInterrupt();
          break;
        case "l":
          this.chat.clearMessages();
          this.requestRender();
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
          this.showHelpOverlay();
          break;
        case "t":
          this.cycleThinking();
          break;
        default:
          // Pass to editor
          if (this.editor.handleKey(key)) {
            this.requestRender();
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
          this.requestRender();
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
      this.requestRender();
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

  private handleInterrupt(): void {
    if (this.state.streaming) {
      this.client.abort();
      this.state.streaming = false;
      this.chat.addMessage({
        id: crypto.randomUUID(),
        role: "system",
        content: "(aborted)",
      });
      this.requestRender();
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
        this.showHelpOverlay();
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
    this.requestRender();

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
    this.requestRender();
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
      addColored(" " + this.state.skills.join(", "), dim_);
    }

    // Extensions
    if (this.state.extensions.length > 0) {
      add("");
      addColored("[Extensions]", sectionHdr);
      addColored(" " + this.state.extensions.join(", "), dim_);
    }
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
      this.state.explicitSession = s.explicitSession ?? false;
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
        this.hideOverlay();
      },
      onCancel: () => {
        this.hideOverlay();
      },
    });

    this.showOverlay(sl);
  }

  private async cycleModel(): Promise<void> {
    try {
      const r = await this.client.cycleModel();
      if (r) {
        this.state.model = r.model;
        this.state.thinking = r.thinkingLevel;
      }
    } catch { /* ignore */ }
    this.requestRender();
  }

  private toggleThinking(): void {
    this.state.thinkingHidden = !this.state.thinkingHidden;
    this.chat.setThinkingHidden(this.state.thinkingHidden);
    this.requestRender();
  }

  private async cycleThinking(): Promise<void> {
    try {
      const r = await this.client.cycleThinkingLevel();
      if (r) this.state.thinking = r.level;
    } catch { /* ignore */ }
    this.requestRender();
  }

  // ─── Overlays ───────────────────────────────────────────────────────────

  private showHelpOverlay(): void {
    const helpComponent: Component = {
      render: (width: number) => this.renderHelp(width),
      invalidate: () => {},
    };
    this.showOverlay(helpComponent);
  }

  showOverlay(component: Component, options?: OverlayOptions): OverlayHandle {
    const preFocus = this.focusedComponent;
    const focusOrder = ++this.focusOrderCounter;
    const entry = { component, options, preFocus, hidden: false, focusOrder };
    this.overlayStack.push(entry);

    // Auto-focus unless nonCapturing (matches pi)
    if (!options?.nonCapturing) {
      this.setFocus(component);
    }
    this.requestRender();

    return {
      hide: () => {
        entry.hidden = true;
        if (this.focusedComponent === component) {
          this.restoreFocus(entry);
        }
        this.requestRender();
      },
      setHidden: (h: boolean) => {
        entry.hidden = h;
        if (h && this.focusedComponent === component) {
          this.restoreFocus(entry);
        } else if (!h && !entry.options?.nonCapturing && this.isOverlayVisible(entry, this.terminal.getWidth(), this.terminal.getHeight())) {
          this.setFocus(component);
        }
        this.requestRender();
      },
      isHidden: () => entry.hidden,
      focus: () => {
        entry.focusOrder = ++this.focusOrderCounter;
        this.setFocus(component);
        this.requestRender();
      },
      unfocus: () => {
        if (this.focusedComponent === component) {
          this.restoreFocus(entry);
          this.requestRender();
        }
      },
      isFocused: () => this.focusedComponent === component,
    };
  }

  private restoreFocus(entry: { preFocus: Component | null }): void {
    // Try next visible overlay, then preFocus, then editor
    const top = this.getTopOverlay();
    if (top) {
      this.setFocus(top);
    } else if (entry.preFocus) {
      this.setFocus(entry.preFocus);
    } else {
      this.setFocus(this.editor);
    }
  }

  hideOverlay(): void {
    const entry = this.overlayStack.pop();
    if (!entry) return;
    if (this.focusedComponent === entry.component) {
      this.restoreFocus(entry);
    }
    this.requestRender();
  }

  private getTopOverlay(): Component | null {
    for (let i = this.overlayStack.length - 1; i >= 0; i--) {
      if (!this.overlayStack[i].hidden) return this.overlayStack[i].component;
    }
    return null;
  }

  private compositeOverlays(base: string[], termW: number, termH: number): string[] {
    // Filter visible, sort by focusOrder (ascending = later overlays on top)
    const visible = this.overlayStack
      .filter((e) => !e.hidden && this.isOverlayVisible(e, termW, termH))
      .sort((a, b) => a.focusOrder - b.focusOrder);

    if (visible.length === 0) return base;

    // Pad base to at least termH for stable screen-relative overlay positioning
    const lines = base.length < termH
      ? [...base, ...new Array(termH - base.length).fill("")]
      : base;

    for (const entry of visible) {
      const overlayLines = entry.component.render(termW);
      if (overlayLines.length === 0) continue;
      const layout = resolveOverlayLayout(termW, termH, overlayLines.length, entry.options);

      // Blank out the overlay area first, then composite lines
      const maxRows = Math.min(overlayLines.length, layout.maxHeight, termH - layout.row);
      for (let i = 0; i < maxRows; i++) {
        const targetRow = layout.row + i;
        if (targetRow >= 0 && targetRow < lines.length) {
          lines[targetRow] = this.compositeLineAt(
            lines[targetRow], overlayLines[i], layout.col, layout.width,
          );
        }
      }
    }

    return lines;
  }

  private compositeLineAt(base: string, overlay: string, col: number, width: number): string {
    // Simple compositing: pad overlay to width, place at column position
    const stripAnsi = (s: string) => s.replace(/\x1b\[[0-9;]*m/g, "");
    const overlayClean = stripAnsi(overlay);
    const baseClean = stripAnsi(base);

    // Build: base up to col + overlay (clipped to width) + base after col+overlay
    const before = baseClean.slice(0, Math.min(col, baseClean.length));
    const overlayPart = overlayClean.slice(0, width);
    const after = baseClean.slice(Math.min(col + overlayPart.length, baseClean.length));

    const padOverlay = overlayPart.length < width ? overlayPart + " ".repeat(width - overlayPart.length) : overlayPart;

    // Preserve ANSI codes by re-applying: base background + overlay content
    // For simplicity, use the overlay text directly (it carries its own ANSI codes)
    const beforePadded = before.length < col ? before + " ".repeat(col - before.length) : before;

    return beforePadded + overlay + " ".repeat(Math.max(0, width - overlayClean.length)) + after;
  }

  private isOverlayVisible(
    entry: { hidden: boolean; options?: OverlayOptions },
    termW: number,
    termH: number,
  ): boolean {
    if (entry.hidden) return false;
    if (entry.options?.visible) return entry.options.visible(termW, termH);
    return true;
  }

  private setFocus(component: Component): void {
    if (this.focusedComponent && isFocusable(this.focusedComponent)) {
      this.focusedComponent.focused = false;
    }
    this.focusedComponent = component;
    if (isFocusable(component)) {
      component.focused = true;
    }
  }

  addInputListener(listener: InputListener): () => void {
    this.inputListeners.add(listener);
    return () => { this.inputListeners.delete(listener); };
  }

  removeInputListener(listener: InputListener): void {
    this.inputListeners.delete(listener);
  }

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
        this.hideOverlay();
      },
      onCancel: () => {
        this.hideOverlay();
      },
    });

    this.showOverlay(sl);
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
        this.hideOverlay();
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
        this.requestRender();
      },
      onCancel: () => {
        this.hideOverlay();
      },
    });

    this.showOverlay(sl);
  }

  // ─── Rendering (pi-style differential with synchronized output) ──────────

  /**
   * Request a render. If called rapidly, calls are coalesced so doRender()
   * fires at most every MIN_RENDER_INTERVAL_MS (16ms ≈ 60fps).
   */
  private requestRender(force = false): void {
    if (force) {
      this.previousLines = [];
      this.hardwareCursorRow = 0;
      this.previousWidth = 0;
      this.previousHeight = 0;
      if (this.renderTimer) {
        clearTimeout(this.renderTimer);
        this.renderTimer = undefined;
      }
      this.renderRequested = true;
      setTimeout(() => this.flushRender(), 0);
      return;
    }
    if (this.renderRequested) return;
    this.renderRequested = true;
    const elapsed = performance.now() - this.lastRenderAt;
    const delay = Math.max(0, App.MIN_RENDER_INTERVAL_MS - elapsed);
    if (delay === 0) {
      setTimeout(() => this.flushRender(), 0);
    } else {
      this.renderTimer = setTimeout(() => {
        this.renderTimer = undefined;
        this.flushRender();
      }, delay);
    }
  }

  private flushRender(): void {
    if (!this.running || !this.renderRequested) return;
    this.renderRequested = false;
    this.lastRenderAt = performance.now();
    this.doRender();
  }

  private doRender(): void {
    const W = this.terminal.getWidth();
    const H = this.terminal.getHeight();
    const footerHeight = 1;
    const editorHeight = 1;
    const chatHeight = H - footerHeight - editorHeight;
    const widthChanged = this.previousWidth !== 0 && this.previousWidth !== W;
    const heightChanged = this.previousHeight !== 0 && this.previousHeight !== H;
    const firstRender = this.previousLines.length === 0;

    // Update dimensions
    this.chat.setViewportHeight(chatHeight);

    // Set footer data before render (Container.render calls footer.render)
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
    this.footer.setData(footerData);

    // Base content via Container.render (matches pi: TUI extends Container)
    // Children are registered in order: chat, editor, footer
    let next: string[] = this.render(W);

    // Pad or truncate to terminal height
    if (next.length < H) {
      const padded = new Array(H).fill("");
      for (let i = 0; i < next.length; i++) padded[i] = next[i];
      next = padded;
    } else if (next.length > H) {
      next = next.slice(0, H);
    }

    // ── Composite overlays ──
    next = this.compositeOverlays(next, W, H);

    // ── Autocomplete popup (positioned above editor, below overlays) ──
    if (this.autocomplete.isVisible()) {
      const acLines = this.autocomplete.render(W);
      if (acLines.length > 0) {
        const editorIdx = H - 2;
        let acTop = editorIdx - acLines.length;
        for (const line of acLines) {
          if (acTop >= 0) next[acTop] = line;
          acTop++;
        }
      }
    }

    // Full re-render on size change, height change, or first render.
    // Matches pi's fullRender logic exactly:
    //   - First render: write without clearing (assumes clean screen)
    //   - Width/height change: clear screen + scrollback
    if (widthChanged || heightChanged || firstRender) {
      const clear = widthChanged || heightChanged;
      let buf = SYNC_BEGIN;
      if (clear) {
        buf += "\x1b[2J\x1b[H"; // no \x1b[3J to preserve scrollback
      }
      for (let i = 0; i < H; i++) {
        if (i > 0) buf += "\r\n";
        buf += next[i];
      }
      buf += SYNC_END;
      this.terminal.write(buf);
      this.hardwareCursorRow = H - 1;
      this.previousLines = next;
      this.previousWidth = W;
      this.previousHeight = H;
      return;
    }

    // Find first and last changed lines
    let firstChanged = -1;
    let lastChanged = -1;
    for (let i = 0; i < H; i++) {
      if (this.previousLines[i] !== next[i]) {
        if (firstChanged === -1) firstChanged = i;
        lastChanged = i;
      }
    }

    if (firstChanged === -1) {
      this.previousWidth = W;
      this.previousHeight = H;
      return;
    }

    // Differential render: move cursor to first changed line, then rewrite
    let buf = SYNC_BEGIN;

    const lineDiff = firstChanged - this.hardwareCursorRow;
    if (lineDiff > 0) {
      buf += `\x1b[${lineDiff}B`;
    } else if (lineDiff < 0) {
      buf += `\x1b[${-lineDiff}A`;
    }

    // Carriage return ensures we start at column 0 (relative moves preserve column)
    buf += "\r";

    for (let i = firstChanged; i <= lastChanged!; i++) {
      if (i > firstChanged) buf += "\r\n";
      buf += "\x1b[2K" + next[i];
    }

    buf += SYNC_END;
    this.terminal.write(buf);
    this.hardwareCursorRow = lastChanged!;
    this.previousLines = next;
    this.previousWidth = W;
    this.previousHeight = H;
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
