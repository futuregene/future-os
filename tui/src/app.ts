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
import { deleteKittyImage, getCapabilities, isImageLine, setCellDimensions } from "./terminal-image.js";
import { parseKey, isKeyRelease, isKeyRepeat, Key } from "./keys.js";
import { KeybindingManager } from "./keybindings.js";
import { visibleWidth, stripAnsiCodes } from "./utils.js";

const KITTY_SEQUENCE_PREFIX = "\x1b_G";

function extractKittyImageIds(line: string): number[] {
  const sequenceStart = line.indexOf(KITTY_SEQUENCE_PREFIX);
  if (sequenceStart === -1) return [];

  const paramsStart = sequenceStart + KITTY_SEQUENCE_PREFIX.length;
  const paramsEnd = line.indexOf(";", paramsStart);
  if (paramsEnd === -1) return [];

  const params = line.slice(paramsStart, paramsEnd);
  for (const param of params.split(",")) {
    const [key, value] = param.split("=", 2);
    if (key !== "i" || value === undefined) continue;
    const id = Number(value);
    if (Number.isInteger(id) && id > 0 && id <= 0xffffffff) {
      return [id];
    }
  }
  return [];
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
  private keybindings = new KeybindingManager();

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
  private cursorRow = 0;
  private hardwareCursorRow = 0;
  private maxLinesRendered = 0;
  private previousViewportTop = 0;
  private clearOnShrink = process.env.PI_CLEAR_ON_SHRINK === "1";
  private showHardwareCursor = process.env.PI_HARDWARE_CURSOR === "1";
  private renderRequested = false;
  private renderTimer: ReturnType<typeof setTimeout> | undefined;
  private lastRenderAt = 0;
  private static readonly MIN_RENDER_INTERVAL_MS = 16;
  private static readonly SEGMENT_RESET = "\x1b[0m";
  private previousWidth = 0;
  private previousHeight = 0;
  private previousKittyImageIds = new Set<number>();

  constructor(private serverUrl = "http://localhost:7890") {
    super();
    this.terminal = new NodeTerminal();
    this.client = new RpcClient(serverUrl);
    this.theme = DARK_THEME;
    this.chat = new ChatArea(this.terminal.columns);
    this.footer = new Footer(this.terminal.columns);

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

    // Register global keybindings
    this.keybindings.add(Key.ctrl_c, () => { this.handleInterrupt(); return true; }, "Interrupt / exit");
    this.keybindings.add(Key.ctrl_l, () => { this.chat.clearMessages(); this.requestRender(); return true; }, "Clear chat");
    this.keybindings.add(Key.ctrl_p, () => { this.cycleModel(); return true; }, "Cycle model");
    this.keybindings.add(Key.ctrl_r, () => { this.showSessions(); return true; }, "Browse sessions");
    this.keybindings.add(Key.ctrl_s, () => { this.showSettings(); return true; }, "Open settings");
    this.keybindings.add(Key.ctrl_o, () => { this.showHelpOverlay(); return true; }, "Show help");
    this.keybindings.add(Key.ctrl_t, () => { this.cycleThinking(); return true; }, "Cycle thinking");
    this.keybindings.add(Key.shift_tab, () => { this.cycleThinking(); return true; }, "Cycle thinking");

    // Subscribe to SSE events
    this.client.subscribe((event) => {
      this.handleAgentEvent(event);
    });
  }

  // ─── Lifecycle ────────────────────────────────────────────────────────────

  async start(): Promise<void> {
    this.terminal.hideCursor();
    this.running = true;
    this.queryCellSize();

    // Terminal manages stdin, emits complete sequences via onInput callback
    this.terminal.start(
      (data: string) => this.handleInput(data),
      () => this.requestRender(),
    );

    await this.refresh();

    // Create a new session if the server auto-generated one (no --session etc).
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

    // Drain stdin to prevent key release leaks, then clean up terminal state
    await this.terminal.drainInput();
    this.terminal.stop();

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

  /**
   * Receives complete sequences from the terminal's StdinBuffer.
   * No char-by-char buffering needed — escape sequences arrive fully assembled.
   */
  private handleInput(data: string): void {
    // Cell size response
    if (this.consumeCellSizeResponse(data)) {
      this.requestRender();
      return;
    }

    // Input listener pipeline
    if (this.inputListeners.size > 0) {
      let d: string | undefined = data;
      for (const listener of this.inputListeners) {
        if (!d) break;
        const result = listener(d);
        if (result?.consume) { d = undefined; break; }
        if (result?.data !== undefined) d = result.data;
      }
      if (!d) return;
      data = d;
    }

    // Bracketed paste
    if (data.startsWith("\x1b[200~")) {
      const endIdx = data.indexOf("\x1b[201~");
      if (endIdx !== -1) {
        const content = data.slice(6, endIdx);
        this.editor.insertText(content);
        this.requestRender();
      }
      return;
    }

    // Ctrl+C (interrupt) — check raw byte before parseKey for responsiveness
    if (data === "\x03") {
      this.handleInterrupt();
      return;
    }

    // Parse key through unified parser (Kitty CSI-u, modifyOtherKeys, legacy)
    const keyName = parseKey(data);
    if (keyName) {
      this.handleKey(keyName);
      return;
    }

    // Fallback: printable character not covered by parseKey
    if (data.length === 1 && data.charCodeAt(0) >= 32) {
      if (this.overlayStack.length > 0) {
        const top = this.getTopOverlay();
        if (top?.handleInput) {
          top.handleInput(data);
          this.requestRender();
        }
        return;
      }
      this.editor.insertText(data);
      this.requestRender();
    }
  }

  private handleKey(key: string): void {
    // Escape - close autocomplete or overlay or clear editor
    if (key === "escape") {
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

    // Overlay mode — dispatch to top overlay via handleInput
    if (this.overlayStack.length > 0) {
      const top = this.getTopOverlay();
      if (top?.handleInput) {
        top.handleInput(key);
      }
      this.requestRender();
      return;
    }

    // Autocomplete navigation takes priority over chat scroll
    if (this.autocomplete.isVisible()) {
      if (key === "up") {
        this.autocomplete.selectPrev();
        this.requestRender();
        return;
      }
      if (key === "down") {
        this.autocomplete.selectNext();
        this.requestRender();
        return;
      }
      if (key === "enter") {
        const item = this.autocomplete.getSelectedItem();
        if (item) {
          this.editor.setValue(item.value);
          this.autocomplete.hide();
          this.requestRender();
          this.handleSubmit(item.value);
        }
        return;
      }
    }

    // Dispatch through keybinding manager (ctrl shortcuts, shift+tab, etc.)
    if (this.keybindings.dispatch(key)) {
      this.requestRender();
      return;
    }

    // Other ctrl+key combos — pass to editor
    if (key.startsWith("ctrl+")) {
      if (this.editor.handleKey(key)) {
        this.requestRender();
      }
      return;
    }

    // Tab - autocomplete
    if (key === "tab") {
      if (this.autocomplete.isVisible()) {
        const item = this.autocomplete.getSelectedItem();
        if (item) {
          this.editor.setValue(item.value);
          this.autocomplete.hide();
          this.requestRender();
          this.handleSubmit(item.value);
        }
        return;
      } else {
        this.triggerAutocomplete();
      }
      return;
    }

    // Enter - handled by editor (falls through)

    // Editor handles the rest
    if (this.editor.handleKey(key)) {
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
        } else if (!h && !entry.options?.nonCapturing && this.isOverlayVisible(entry, this.terminal.columns, this.terminal.rows)) {
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
    if (isImageLine(base)) return base;
    // Simple compositing: pad overlay to width, place at column position
    const overlayClean = stripAnsiCodes(overlay);
    const baseClean = stripAnsiCodes(base);

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
  /**
   * Request a render. Uses dual-phase scheduling (matches pi):
   * - force: process.nextTick for immediate full redraw
   * - ambient: setTimeout-based coalescing at ~60fps
   */
  private requestRender(force = false): void {
    if (force) {
      this.previousLines = [];
      this.previousWidth = -1;  // triggers widthChanged in doRender
      this.previousHeight = -1; // triggers heightChanged in doRender
      this.cursorRow = 0;
      this.hardwareCursorRow = 0;
      this.maxLinesRendered = 0;
      this.previousViewportTop = 0;
      if (this.renderTimer) {
        clearTimeout(this.renderTimer);
        this.renderTimer = undefined;
      }
      this.renderRequested = true;
      process.nextTick(() => {
        if (!this.running || !this.renderRequested) return;
        this.renderRequested = false;
        this.lastRenderAt = performance.now();
        this.doRender();
      });
      return;
    }
    if (this.renderRequested) return;
    this.renderRequested = true;
    process.nextTick(() => this.scheduleRender());
  }

  private scheduleRender(): void {
    if (!this.running || this.renderTimer || !this.renderRequested) return;
    const elapsed = performance.now() - this.lastRenderAt;
    const delay = Math.max(0, App.MIN_RENDER_INTERVAL_MS - elapsed);
    this.renderTimer = setTimeout(() => {
      this.renderTimer = undefined;
      if (!this.running || !this.renderRequested) return;
      this.renderRequested = false;
      this.lastRenderAt = performance.now();
      this.doRender();
      if (this.renderRequested) this.scheduleRender();
    }, delay);
  }

  // ─── Line resets / cursor extraction ───────────────────────────────

  private applyLineResets(lines: string[]): string[] {
    const reset = App.SEGMENT_RESET;
    for (let i = 0; i < lines.length; i++) {
      if (!isImageLine(lines[i])) {
        lines[i] = lines[i] + reset;
      }
    }
    return lines;
  }

  private extractCursorPosition(lines: string[], height: number): { row: number; col: number } | null {
    const viewportTop = Math.max(0, lines.length - height);
    for (let row = lines.length - 1; row >= viewportTop; row--) {
      const line = lines[row];
      const markerIndex = line.indexOf("\x1b_pi:c\x07");
      if (markerIndex !== -1) {
        const beforeMarker = line.slice(0, markerIndex);
        const col = visibleWidth(beforeMarker);
        lines[row] = line.slice(0, markerIndex) + line.slice(markerIndex + 7);
        return { row, col };
      }
    }
    return null;
  }

  private positionHardwareCursor(cursorPos: { row: number; col: number } | null, totalLines: number): void {
    if (!cursorPos || totalLines <= 0) return;
    const targetRow = Math.min(cursorPos.row, totalLines - 1);
    const currentRow = this.hardwareCursorRow;
    if (targetRow > currentRow) {
      this.terminal.write(`\x1b[${targetRow - currentRow}B`);
    } else if (targetRow < currentRow) {
      this.terminal.write(`\x1b[${currentRow - targetRow}A`);
    }
    this.terminal.write(`\x1b[${cursorPos.col}G`);
    this.hardwareCursorRow = targetRow;
    if (this.showHardwareCursor) {
      this.terminal.write("\x1b[?25h");
    }
  }

  private queryCellSize(): void {
    if (!getCapabilities().images) return;
    this.terminal.write("\x1b[16t");
  }

  private consumeCellSizeResponse(data: string): boolean {
    const match = data.match(/^\x1b\[6;(\d+);(\d+)t$/);
    if (!match) return false;
    const heightPx = parseInt(match[1], 10);
    const widthPx = parseInt(match[2], 10);
    if (heightPx <= 0 || widthPx <= 0) return true;
    setCellDimensions({ widthPx, heightPx });
    this.invalidate();
    this.requestRender();
    return true;
  }

  private collectKittyImageIds(lines: string[]): Set<number> {
    const ids = new Set<number>();
    for (const line of lines) {
      for (const id of extractKittyImageIds(line)) {
        ids.add(id);
      }
    }
    return ids;
  }

  private deleteKittyImages(ids: Iterable<number>): string {
    let buffer = "";
    for (const id of ids) {
      buffer += deleteKittyImage(id);
    }
    return buffer;
  }

  private expandLastChangedForKittyImages(firstChanged: number, lastChanged: number): number {
    let expandedLastChanged = lastChanged;
    for (let i = firstChanged; i < this.previousLines.length; i++) {
      if (extractKittyImageIds(this.previousLines[i]).length > 0) {
        expandedLastChanged = Math.max(expandedLastChanged, i);
      }
    }
    return expandedLastChanged;
  }

  private deleteChangedKittyImages(firstChanged: number, lastChanged: number): string {
    if (firstChanged < 0 || lastChanged < firstChanged) return "";

    const ids = new Set<number>();
    const maxLine = Math.min(lastChanged, this.previousLines.length - 1);
    for (let i = firstChanged; i <= maxLine; i++) {
      for (const id of extractKittyImageIds(this.previousLines[i] ?? "")) {
        ids.add(id);
      }
    }

    return this.deleteKittyImages(ids);
  }

  // ─── Main Render Pipeline ──────────────────────────────────────────

  private doRender(): void {
    if (!this.running) return;
    const W = this.terminal.columns;
    const H = this.terminal.rows;
    const widthChanged = this.previousWidth !== 0 && this.previousWidth !== W;
    const heightChanged = this.previousHeight !== 0 && this.previousHeight !== H;
    const previousBufferLength = this.previousHeight > 0 ? this.previousViewportTop + this.previousHeight : H;
    let prevViewportTop = heightChanged ? Math.max(0, previousBufferLength - H) : this.previousViewportTop;
    let viewportTop = prevViewportTop;
    let hardwareCursorRow = this.hardwareCursorRow;
    const computeLineDiff = (targetRow: number): number => {
      const currentScreenRow = hardwareCursorRow - prevViewportTop;
      const targetScreenRow = targetRow - viewportTop;
      return targetScreenRow - currentScreenRow;
    };

    // Render editor first to determine its height (multi-line aware)
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

    const footerLines = this.footer.render(W).length;
    const editorLines = this.editor.render(W);
    const editorHeight = editorLines.length;

    // Set chat viewport based on remaining space
    const chatHeight = H - editorHeight - footerLines;
    this.chat.setViewportHeight(Math.max(1, chatHeight));

    // Build render output: chat + editor + footer
    const chatLines = this.chat.render(W);
    let newLines = [...chatLines, ...editorLines, ...this.footer.render(W)];

    // Composite overlays into rendered lines (before diff compare, matches pi)
    if (this.overlayStack.length > 0) {
      newLines = this.compositeOverlays(newLines, W, H);
    }

    // Autocomplete popup (positioned above editor)
    if (this.autocomplete.isVisible()) {
      const acLines = this.autocomplete.render(W);
      if (acLines.length > 0) {
        const editorIdx = H - 1 - editorHeight;
        let acTop = editorIdx - acLines.length;
        for (const line of acLines) {
          if (acTop >= 0) newLines[acTop] = line;
          acTop++;
        }
      }
    }

    // Extract cursor position before line resets (marker must be found first)
    const cursorPos = this.extractCursorPosition(newLines, H);

    // Apply line resets (prevents ANSI style bleed between lines)
    newLines = this.applyLineResets(newLines);

    // ── Full render helper ──────────────────────────────────────────
    const fullRender = (clear: boolean): void => {
      let buf = SYNC_BEGIN;
      if (clear) {
        buf += this.deleteKittyImages(this.previousKittyImageIds);
        buf += "\x1b[2J\x1b[H\x1b[3J"; // Clear screen, home, clear scrollback
      }
      for (let i = 0; i < newLines.length; i++) {
        if (i > 0) buf += "\r\n";
        buf += newLines[i];
      }
      buf += SYNC_END;
      this.terminal.write(buf);
      this.cursorRow = Math.max(0, newLines.length - 1);
      this.hardwareCursorRow = this.cursorRow;
      if (clear) {
        this.maxLinesRendered = newLines.length;
      } else {
        this.maxLinesRendered = Math.max(this.maxLinesRendered, newLines.length);
      }
      const bufferLength = Math.max(H, newLines.length);
      this.previousViewportTop = Math.max(0, bufferLength - H);
      this.positionHardwareCursor(cursorPos, newLines.length);
      this.previousLines = newLines;
      this.previousKittyImageIds = this.collectKittyImageIds(newLines);
      this.previousWidth = W;
      this.previousHeight = H;
    };

    // First render — output without clearing (assumes clean screen)
    if (this.previousLines.length === 0 && !widthChanged && !heightChanged) {
      fullRender(false);
      return;
    }

    // Width changes always need full re-render (wrapping changes)
    if (widthChanged) {
      fullRender(true);
      return;
    }

    // Height changes normally need full re-render to keep visible viewport aligned
    if (heightChanged) {
      fullRender(true);
      return;
    }

    // Content shrunk — clear empty rows when clearOnShrink enabled
    if (this.clearOnShrink && newLines.length < this.maxLinesRendered && this.overlayStack.length === 0) {
      fullRender(true);
      return;
    }

    // ── Diff: find changed lines ────────────────────────────────────
    let firstChanged = -1;
    let lastChanged = -1;
    const maxLines = Math.max(newLines.length, this.previousLines.length);
    for (let i = 0; i < maxLines; i++) {
      const oldLine = i < this.previousLines.length ? this.previousLines[i] : "";
      const newLine = i < newLines.length ? newLines[i] : "";
      if (oldLine !== newLine) {
        if (firstChanged === -1) firstChanged = i;
        lastChanged = i;
      }
    }

    // Appended lines detection (streaming optimization)
    const appendedLines = newLines.length > this.previousLines.length;
    if (appendedLines) {
      if (firstChanged === -1) firstChanged = this.previousLines.length;
      lastChanged = newLines.length - 1;
    }
    if (firstChanged !== -1) {
      lastChanged = this.expandLastChangedForKittyImages(firstChanged, lastChanged);
    }
    const appendStart = appendedLines && firstChanged === this.previousLines.length && firstChanged > 0;

    // No changes — but still need to update hardware cursor position
    if (firstChanged === -1) {
      this.positionHardwareCursor(cursorPos, newLines.length);
      this.previousViewportTop = prevViewportTop;
      this.previousHeight = H;
      return;
    }

    // ── All changes in deleted lines (content shrunk) ─────────────────
    if (firstChanged >= newLines.length) {
      if (this.previousLines.length > newLines.length) {
        let buf = SYNC_BEGIN;
        buf += this.deleteChangedKittyImages(firstChanged, lastChanged);
        const targetRow = Math.max(0, newLines.length - 1);
        // If viewport moved up (content above viewport removed), full render
        if (targetRow < prevViewportTop) {
          fullRender(true);
          return;
        }
        const lineDiff = computeLineDiff(targetRow);
        if (lineDiff > 0) buf += `\x1b[${lineDiff}B`;
        else if (lineDiff < 0) buf += `\x1b[${-lineDiff}A`;
        buf += "\r";

        const extraLines = this.previousLines.length - newLines.length;
        // If too many lines to clear, full render
        if (extraLines > H) {
          fullRender(true);
          return;
        }
        if (extraLines > 0) buf += "\x1b[1B";
        for (let i = 0; i < extraLines; i++) {
          buf += "\r\x1b[2K";
          if (i < extraLines - 1) buf += "\x1b[1B";
        }
        if (extraLines > 0) buf += `\x1b[${extraLines}A`;
        buf += SYNC_END;
        this.terminal.write(buf);
        this.cursorRow = targetRow;
        this.hardwareCursorRow = targetRow;
      }
      this.positionHardwareCursor(cursorPos, newLines.length);
      this.previousLines = newLines;
      this.previousKittyImageIds = this.collectKittyImageIds(newLines);
      this.previousWidth = W;
      this.previousHeight = H;
      this.previousViewportTop = prevViewportTop;
      return;
    }

    // Differential rendering can only touch what was actually visible.
    // If first changed line is above previous viewport, need a full redraw.
    if (firstChanged < prevViewportTop) {
      fullRender(true);
      return;
    }

    // ── Differential render ──────────────────────────────────────────
    let buf = SYNC_BEGIN;
    buf += this.deleteChangedKittyImages(firstChanged, lastChanged);
    const prevViewportBottom = prevViewportTop + H - 1;
    const moveTargetRow = appendStart ? firstChanged - 1 : firstChanged;
    if (moveTargetRow > prevViewportBottom) {
      const currentScreenRow = Math.max(0, Math.min(H - 1, hardwareCursorRow - prevViewportTop));
      const moveToBottom = H - 1 - currentScreenRow;
      if (moveToBottom > 0) {
        buf += `\x1b[${moveToBottom}B`;
      }
      const scroll = moveTargetRow - prevViewportBottom;
      buf += "\r\n".repeat(scroll);
      prevViewportTop += scroll;
      viewportTop += scroll;
      hardwareCursorRow = moveTargetRow;
    }

    // Move cursor to first changed line
    const lineDiff = computeLineDiff(moveTargetRow);
    if (lineDiff > 0) {
      buf += `\x1b[${lineDiff}B`;
    } else if (lineDiff < 0) {
      buf += `\x1b[${-lineDiff}A`;
    }

    buf += appendStart ? "\r\n" : "\r";

    const renderEnd = Math.min(lastChanged, newLines.length - 1);
    for (let i = firstChanged; i <= renderEnd; i++) {
      if (i > firstChanged) buf += "\r\n";
      buf += "\x1b[2K";
      let line = newLines[i];
      const isImage = isImageLine(line);
      if (!isImage && visibleWidth(line) > W) {
        // Width overflow: strip ANSI and truncate (component should fix this, but don't crash)
        line = stripAnsiCodes(line).slice(0, W);
        newLines[i] = line;
      }
      buf += line;
    }

    let finalCursorRow = renderEnd;

    // Clear extra lines when content shrunk
    if (this.previousLines.length > newLines.length) {
      if (renderEnd < newLines.length - 1) {
        const moveDown = newLines.length - 1 - renderEnd;
        buf += `\x1b[${moveDown}B`;
        finalCursorRow = newLines.length - 1;
      }
      const extraLines = this.previousLines.length - newLines.length;
      for (let i = newLines.length; i < this.previousLines.length; i++) {
        buf += "\r\n\x1b[2K";
      }
      buf += `\x1b[${extraLines}A`;
    }

    buf += SYNC_END;
    this.terminal.write(buf);

    this.cursorRow = Math.max(0, newLines.length - 1);
    this.hardwareCursorRow = finalCursorRow;
    this.maxLinesRendered = Math.max(this.maxLinesRendered, newLines.length);
    this.previousViewportTop = Math.max(prevViewportTop, finalCursorRow - H + 1);

    this.positionHardwareCursor(cursorPos, newLines.length);

    this.previousLines = newLines;
    this.previousKittyImageIds = this.collectKittyImageIds(newLines);
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
      const lPad = colW - visibleWidth(l);
      const rPad = colW - visibleWidth(r);
      lines.push(dim_("\u2502") + "  " + l + " ".repeat(Math.max(1, lPad)) + r + " ".repeat(Math.max(1, rPad)) + dim_("\u2502"));
    }

    lines.push(dim_("\u2514" + "\u2500".repeat(W - 2) + "\u2518"));
    return lines;
  }

}
