/**
 * future-tui TUI - Main application.
 * Complete terminal UI for FutureAgent.
 */

import { GrpcClient } from "./rpc/index.js";
import { VERSION } from "./version.generated.js";
import type { SessionSummary } from "./rpc/types.js";
import { ChatArea, type ChatMessage } from "./components/chat-area.js";
import { Footer, type FooterData } from "./components/footer.js";
import { SelectList, type SelectItem } from "./components/select-list.js";
import { AutocompletePopup, type AutocompleteItem, AutocompleteManager, SlashCommandProvider, FilePathProvider, AttachmentProvider, type SlashCommand } from "./components/autocomplete.js";
import { ScopedModelsSelector } from "./components/scoped-models-selector.js";
import { Input } from "./components/input.js";
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
import { DARK_THEME, type Theme, fg, bold } from "./theme.js";
import { getCapabilities, isImageLine, setCellDimensions, collectKittyImageIds, deleteKittyImages, extractKittyImageIds } from "./terminal-image.js";
import { parseKey, isKeyRelease, Key } from "./keys.js";
import { KeybindingManager } from "./keybindings.js";
import { extractSegments, visibleWidth, stripAnsiCodes, normalizeTerminalOutput, sliceByColumn, truncateToWidth, wrapTextWithAnsi } from "./utils.js";
import { renderHelp } from "./help-screen.js";
import * as path from "node:path";
import * as os from "node:os";
import * as fs from "node:fs";

// Termux detection: skip full redraw on height changes (keyboard show/hide)
function isTermuxSession(): boolean {
  return Boolean(process.env.TERMUX_VERSION);
}

export class App extends Container {
  private terminal: NodeTerminal;
  private client: GrpcClient;
  private theme: Theme;
  private input: Input;
  private chat: ChatArea;
  private footer: Footer;
  private overlayStack: { component: Component; options?: OverlayOptions; preFocus: Component | null; hidden: boolean; focusOrder: number }[] = [];
  private focusOrderCounter = 0;
  private focusedComponent: Component | null = null;
  private inputListeners = new Set<InputListener>();
  private autocomplete = new AutocompletePopup();
  private acManager = new AutocompleteManager();
  private keybindings = new KeybindingManager();
  private enabledModelIds: string[] | null = null;  // client-side scoped models

  // ── TUI-local settings (persisted to disk, not on agent) ──────────────
  private tuiSettings: { defaultModel?: string; defaultThinkingLevel?: string; defaultPermissionLevel?: string; enabledModelIds?: string[] } = {};
  private tuiSettingsPath = path.join(os.homedir(), ".future", "tui", "settings.json");

  // Slash commands for autocomplete (with model/session arg flags)
  private readonly slashCommands: SlashCommand[] = [
    { value: "/cwd", label: "/cwd", description: "change working directory" },
    { value: "/approve", label: "/approve", description: "approve pending tool execution" },
    { value: "/reject", label: "/reject", description: "reject pending tool execution" },
    { value: "/stop", label: "/stop", description: "stop current generation" },
    { value: "/status", label: "/status", description: "show session and model info" },
    { value: "/model", label: "/model", description: "select model", takesModelArg: true },
    { value: "/sessions", label: "/sessions", description: "browse sessions" },
    { value: "/new", label: "/new", description: "new session" },
    { value: "/clone", label: "/clone", description: "clone session", takesSessionArg: true },
    { value: "/fork", label: "/fork", description: "fork session", takesSessionArg: true },
    { value: "/tree", label: "/tree", description: "session tree" },
    { value: "/name", label: "/name", description: "set session name" },
    { value: "/scoped-models", label: "/scoped-models", description: "configure model scope" },
    { value: "/reload", label: "/reload", description: "reload skills + context" },
    { value: "/help", label: "/help", description: "show help" },
  ];

  // Autocomplete provider callbacks
  private getModels = async (): Promise<string[]> => {
    try {
      const models = await this.client.listModels();
      return models.map((m) => m.provider ? `${m.provider}/${m.id}` : m.id);
    } catch { return []; }
  };

  private getSessions = async (): Promise<string[]> => {
    try {
      const r = await this.client.listSessions();
      return r.sessions.map((s) => s.session_name || s.id);
    } catch { return []; }
  };

  private state = {
    model: "",
    thinking: "off",
    streaming: false,
    spinnerFrame: 0,
    sessionId: "",  // Current session ID
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
    tokensCacheR: 0,
    tokensCacheW: 0,
    totalCost: 0,
    autoCompactionEnabled: true,
    toolStartTime: 0,
    activeToolCount: 0,
    explicitSession: false, // true when --session/--continue/--resume/--fork was used
  };

  private running = false;

  // Diff-based render state 
  private previousLines: string[] = [];
  private cursorRow = 0;
  private hardwareCursorRow = 0;
  private maxLinesRendered = 0;
  private previousViewportTop = 0;
  private clearOnShrink = process.env.PI_CLEAR_ON_SHRINK === "1";
  private forceClearNextRender = false;
  private showHardwareCursor = process.env.PI_HARDWARE_CURSOR === "1";
  private renderRequested = false;
  private renderTimer: ReturnType<typeof setTimeout> | undefined;
  private lastRenderAt = 0;
  private resizeDebounceTimer: ReturnType<typeof setTimeout> | undefined;
  private static readonly MIN_RENDER_INTERVAL_MS = 33;
  private static readonly RESIZE_DEBOUNCE_MS = 150;
  private static readonly SEGMENT_RESET = "\x1b[0m\x1b]8;;\x07";  // SGR reset + OSC 8 close (prevents hyperlink leak)
  private previousWidth = 0;
  private previousHeight = 0;
  private previousKittyImageIds = new Set<number>();
  private fullRedrawCount = 0;
  public onDebug?: () => void;

  getClearOnShrink(): boolean { return this.clearOnShrink; }
  setClearOnShrink(enabled: boolean): void { this.clearOnShrink = enabled; }

  getShowHardwareCursor(): boolean { return this.showHardwareCursor; }
  setShowHardwareCursor(enabled: boolean): void {
    if (this.showHardwareCursor === enabled) return;
    this.showHardwareCursor = enabled;
    if (!enabled) this.terminal.hideCursor();
    this.requestRender();
  }

  getFullRedrawCount(): number { return this.fullRedrawCount; }

  constructor(private grpcAddr = "localhost:50051", private cliOptions: {
    session?: string | null;
    continue?: boolean;
    resume?: boolean;
    fork?: string | null;
    initialPrompt?: string;
  } = {}) {
    super();
    this.terminal = new NodeTerminal();
    // Always use gRPC
    this.client = new GrpcClient(grpcAddr);
    this.theme = DARK_THEME;
    this.chat = new ChatArea(this.terminal.columns);
    this.chat.setOnChange(() => this.requestRender());
    this.footer = new Footer(this.terminal.columns);

    this.input = new Input();
    this.input.onSubmit = (v) => this.handleSubmit(v);
    this.input.onChange = (v) => this.acManager.query(v, v.length);
    this.input.onEscape = () => { this.input.setValue(""); this.requestRender(); };
    this.input.focused = true;
    this.focusedComponent = this.input;

    // Wire autocomplete manager → popup
    this.acManager.onItems = (items) => {
      if (items.length > 0) {
        this.autocomplete.show(items);
      } else {
        this.autocomplete.hide();
      }
      this.requestRender();
    };

    // Register autocomplete providers
    this.acManager.register(new SlashCommandProvider(this.slashCommands, this.getModels, this.getSessions));
    this.acManager.register(new FilePathProvider(this.state.cwd || process.cwd()));
    this.acManager.register(new AttachmentProvider());

    // Register children with Container 
    this.addChild(this.chat);
    this.addChild(this.input);
    this.addChild(this.footer);

    // Register global keybindings
    this.keybindings.add(Key.ctrl_c, () => { this.handleInterrupt(); return true; }, "Interrupt / exit");
    this.keybindings.add(Key.ctrl_l, () => {
      this.forceClearNextRender = true; this.requestRender(); return true;
    }, "Clear screen / redraw");
    this.keybindings.add(Key.ctrl_p, () => { this.cycleModel(); return true; }, "Cycle model");
    this.keybindings.add(Key.ctrl_r, () => { this.showSessions(); return true; }, "Browse sessions");
    this.keybindings.add(Key.ctrl_t, () => { this.cycleThinking(); return true; }, "Cycle thinking");
    this.keybindings.add(Key.shift_tab, () => { this.cycleThinking(); return true; }, "Cycle thinking");
    this.keybindings.add(Key.pageUp, () => {
      this.chat.scrollUp(this.terminal.rows); this.requestRender(); return true;
    }, "Scroll chat up");
    this.keybindings.add(Key.pageDown, () => {
      this.chat.scrollDown(this.terminal.rows); this.requestRender(); return true;
    }, "Scroll chat down");
    this.keybindings.add(Key.ctrl_up, () => {
      this.chat.scrollUp(3); this.requestRender(); return true;
    }, "Scroll chat up (line)");
    this.keybindings.add(Key.ctrl_down, () => {
      this.chat.scrollDown(3); this.requestRender(); return true;
    }, "Scroll chat down (line)");

    // Event subscription is deferred to start() — subscribing here with an
    // empty currentSessionId would risk receiving events from other sessions
    // (e.g. a GUI session streaming concurrently).
  }

  // ─── Lifecycle ────────────────────────────────────────────────────────────

  async start(): Promise<void> {
    this.loadTuiSettings();
    this.terminal.hideCursor();
    this.running = true;
    this.queryCellSize();

    // Terminal manages stdin, emits complete sequences via onInput callback
    this.terminal.start(
      (data: string) => this.handleInput(data),
      () => this.requestResizeRender(),
    );

    // Handle CLI session options
    if (this.cliOptions.session) {
      // --session: switch to specific session
      this.state.explicitSession = true;
      try {
        await this.client.switchSession(this.cliOptions.session);
        await this.refresh();
        await this.loadSessionMessages();
      } catch (err) {
        this.chat.addMessage({
          id: crypto.randomUUID(),
          role: "system",
          content: `Failed to switch to session ${this.cliOptions.session}: ${err}`,
        });
      }
    } else if (this.cliOptions.continue) {
      // --continue: find most recent session and continue
      this.state.explicitSession = true;
      try {
        const { sessions } = await this.client.listSessions();
        if (sessions.length > 0) {
          sessions.sort((a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime());
          await this.client.switchSession(sessions[0].id);
          await this.refresh();
          await this.loadSessionMessages();
        }
      } catch (err) {
        this.chat.addMessage({
          id: crypto.randomUUID(),
          role: "system",
          content: `Failed to continue session: ${err}`,
        });
      }
    } else if (this.cliOptions.fork) {
      // --fork: fork from specific session
      this.state.explicitSession = true;
      try {
        await this.client.fork(this.cliOptions.fork);
        await this.refresh();
      } catch (err) {
        this.chat.addMessage({
          id: crypto.randomUUID(),
          role: "system",
          content: `Failed to fork session ${this.cliOptions.fork}: ${err}`,
        });
      }
    } else if (this.cliOptions.resume) {
      // --resume: show session picker (handled by showSessions)
      this.state.explicitSession = true;
      await this.refresh();
      this.showSessions();
    } else {
      // No explicit session option, create new session.
      // Reload skills first so getState returns the latest list.
      try { await this.client.reloadConfig(); } catch { /* ok if agent doesn't support it */ }
      await this.refresh();
      if (!this.state.explicitSession) {
        try {
          const result = await this.client.newSession();
          if (result.sessionId) {
            // Server created a new session with an ID, use it
            await this.refresh();
          }
        } catch {
          // Server may not support new_session — continue with current session
        }
      }
    }

    // Handle initial prompt (non-empty messages from CLI without -p flag)
    if (this.cliOptions.initialPrompt) {
      // Wait a bit for the TUI to render, then send the prompt
      setTimeout(() => {
        this.client.prompt(this.cliOptions.initialPrompt!);
      }, 100);
    }

    // Subscribe to events only after session is established — prevents
    // cross-session event leakage (e.g. GUI streaming bleeding into TUI).
    this.client.subscribe((event) => {
      // If we were showing "not connected", refresh state on first event
      if (this.state.model === "(not connected)") {
        this.refresh().catch(() => {});
      }
      this.handleAgentEvent(event);
    });

    await this.applyTuiDefaults();
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
    if (this.resizeDebounceTimer) {
      clearTimeout(this.resizeDebounceTimer);
      this.resizeDebounceTimer = undefined;
    }
    this.renderRequested = false;

    // Drain stdin to prevent key release leaks, then clean up terminal state
    await this.terminal.drainInput();
    this.terminal.stop();

    // Move cursor to end of content )
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
      case "user_message": {
        const e = event as { text?: string };
        const text = e.text ?? "";
        // Dedup: the sender TUI already added this message locally before
        // sending the RPC, so its own broadcast would create a duplicate.
        // Observing TUIs (different client, same session) see it for the
        // first time — without it they would only get the assistant reply.
        const last = this.chat.lastMessage;
        if (last?.role === "user" && last.content === text) return;
        this.chat.addMessage({
          id: crypto.randomUUID(),
          role: "user",
          content: text,
        });
        break;
      }

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
        // Refresh state to update context percentage, token totals, etc.
        this.refresh().then(() => this.requestRender());
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
        const e = event as { tool_id?: string; tool_name?: string; tool_args?: unknown };
        const toolArgs = typeof e.tool_args === "string" ? e.tool_args
          : typeof e.tool_args === "object" ? JSON.stringify(e.tool_args)
          : undefined;
        this.chat.addToolStart(e.tool_id ?? "", e.tool_name ?? "", toolArgs);
        if (this.state.activeToolCount === 0) this.state.toolStartTime = performance.now();
        this.state.activeToolCount++;
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
        this.state.activeToolCount = Math.max(0, this.state.activeToolCount - 1);
        if (this.state.activeToolCount === 0) this.state.toolStartTime = 0;
        // Each LLM turn may have its own cost (which was finalised before
        // tools executed).  Pull the latest cumulative cost/token totals so
        // the footer updates after every tool call, not just at agent_end.
        this.refresh().then(() => this.requestRender());
        break;
      }

      case "approval_request": {
        const e = event as {
          approval_request_id?: string;
          tool_id?: string;
          tool_name?: string;
          kind?: string;
          risk_level?: string;
          title?: string;
          summary?: string;
          requested_action?: unknown;
        };
        this.showApprovalOverlay({
          requestId: e.approval_request_id ?? "",
          toolId: e.tool_id ?? "",
          toolName: e.tool_name ?? "",
          kind: e.kind ?? "",
          riskLevel: e.risk_level ?? "",
          title: e.title ?? "Approve tool execution",
          summary: e.summary ?? "",
          requestedAction: e.requested_action,
        });
        break;
      }

      case "error": {
        this.state.streaming = false;
        const e = event as { error?: string; error_message?: string };
        const msg = e.error ?? e.error_message ?? "unknown error";
        this.chat.addMessage({
          id: crypto.randomUUID(),
          role: "system",
          content: `Error: ${msg}`,
        });
        break;
      }

      case "usage": {
        const e = event as { usage?: { prompt_tokens?: number; completion_tokens?: number; cache_read_tokens?: number; cache_write_tokens?: number; total_tokens?: number } };
        if (e.usage?.prompt_tokens !== undefined) this.state.tokensIn += e.usage.prompt_tokens;
        if (e.usage?.completion_tokens !== undefined) this.state.tokensOut += e.usage.completion_tokens;
        if (e.usage?.cache_read_tokens !== undefined) this.state.tokensCacheR += e.usage.cache_read_tokens;
        if (e.usage?.cache_write_tokens !== undefined) this.state.tokensCacheW += e.usage.cache_write_tokens;
        this.state.contextTokens = (e.usage?.prompt_tokens ?? 0) + (e.usage?.completion_tokens ?? 0);
        // Pull latest cumulative cost/token totals from the agent so the
        // footer updates after every model call — a single user turn often
        // spans multiple thinking blocks and tool calls, and previously the
        // cost only refreshed once at agent_end. The agent updates its
        // cumulative cost right before emitting this event, so the state
        // read reflects the call that just finished.
        this.refresh().then(() => this.requestRender());
        break;
      }

      // ── Settings-change events (broadcast by agent so other clients stay in sync) ──

      case "model_changed": {
        const e = event as { model?: string };
        if (e.model) this.state.model = e.model;
        break;
      }
      case "thinking_level_changed": {
        const e = event as { level?: string };
        if (e.level) this.state.thinking = e.level;
        break;
      }
      case "permission_level_changed": {
        // Reflected in /status only — no footer field, but refresh to stay accurate.
        this.refresh().catch(() => {});
        break;
      }
      case "cwd_changed": {
        const e = event as { cwd?: string };
        if (e.cwd) this.state.cwd = e.cwd;
        break;
      }
      case "session_name_changed": {
        // No immediate TUI action needed — name is shown in session list.
        break;
      }
      case "auto_compaction_changed": {
        const e = event as { enabled?: boolean };
        if (typeof e.enabled === "boolean") this.state.autoCompactionEnabled = e.enabled;
        break;
      }
      case "tools_changed":
      case "steering_mode_changed":
      case "follow_up_mode_changed":
      case "sandbox_policy_changed":
      case "ephemeral_changed": {
        // Reflected in /status; refresh to keep accurate.
        this.refresh().catch(() => {});
        break;
      }
      case "config_reloaded": {
        const e = event as { skills?: string[]; contextFiles?: string[] };
        if (e.skills) this.state.skills = e.skills.slice().sort((a, b) => a.localeCompare(b));
        if (e.contextFiles) this.state.contextFiles = e.contextFiles;
        this.chat.addMessage({
          id: crypto.randomUUID(),
          role: "system",
          content: `Config reloaded: ${e.skills?.length ?? 0} skills, ${e.contextFiles?.join(", ") || "no context files"}`,
        });
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

    // Filter key release events unless focused component explicitly wants them 
    if (isKeyRelease(data)) {
      const focused = this.focusedComponent;
      if (!focused?.wantsKeyRelease) return;
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
        // Route to the focused overlay if one is open — matches the
        // printable-character branch below. Pasting into the background
        // input while an overlay has focus would surface unexpected text
        // when the overlay closes.
        if (this.overlayStack.length > 0) {
          const top = this.getTopOverlay();
          if (top?.handleInput) top.handleInput(content);
        } else {
          this.input.insertText(content);
        }
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
      // If focused component is a now-invisible overlay, redirect focus 
      const focusedOverlay = this.overlayStack.find((o) => o.component === this.focusedComponent);
      if (focusedOverlay && !this.isOverlayVisible(focusedOverlay, this.terminal.columns, this.terminal.rows)) {
        const top = this.getTopOverlay();
        if (top) {
          this.setFocus(top);
        } else {
          this.setFocus(this.input);
        }
      }
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
      this.input.insertText(data);
      this.requestRender();
    }
  }

  private handleKey(key: string): void {
    // Shift+Ctrl+D — trigger debug callback 
    if (key === "shift+ctrl+d" && this.onDebug) {
      this.onDebug();
      return;
    }

    // Escape - close autocomplete or overlay or clear editor
    if (key === "escape") {
      if (this.autocomplete.isVisible()) {
        this.autocomplete.hide();
        this.requestRender();
      } else if (this.overlayStack.length > 0) {
        this.hideOverlay();
        this.requestRender();
      } else {
        this.input.setValue("");
        this.autocomplete.hide();
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
        this.applyAutocompleteSelection();
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
      if (this.input.handleKey(key)) {
        this.requestRender();
      }
      return;
    }

    // Tab - autocomplete
    if (key === "tab") {
      if (this.autocomplete.isVisible()) {
        // Accept the highlighted completion into the input only —
        // do NOT submit (Tab is completion, not confirmation).
        this.applyAutocompleteSelection();
      } else {
        this.triggerAutocomplete();
      }
      return;
    }

    // Enter - handled by editor (falls through)

    // Editor handles the rest
    if (this.input.handleKey(key)) {
      this.requestRender();
    }
  }

  // ─── Approval Overlay ────────────────────────────────────────────

  private pendingApproval: {
    requestId: string;
    toolName: string;
    title: string;
    summary: string;
    riskLevel: string;
    requestedAction?: unknown;
  } | null = null;

  private showApprovalOverlay(req: {
    requestId: string;
    toolId: string;
    toolName: string;
    kind: string;
    riskLevel: string;
    title: string;
    summary: string;
    requestedAction?: unknown;
  }): void {
    // Store pending approval
    this.pendingApproval = {
      requestId: req.requestId,
      toolName: req.toolName,
      title: req.title,
      summary: req.summary,
      riskLevel: req.riskLevel,
      requestedAction: req.requestedAction,
    };

    // Show as a chat message with instructions
    const actionPreview = req.requestedAction
      ? (typeof req.requestedAction === "string"
          ? req.requestedAction
          : JSON.stringify(req.requestedAction, null, 2))
      : "";
    this.chat.addMessage({
      id: crypto.randomUUID(),
      role: "system",
      content: [
        `⚠️ **Approval Required** [${req.riskLevel.toUpperCase()} RISK]`,
        `**${req.title}**`,
        req.summary,
        actionPreview ? `\`\`\`\n${actionPreview.slice(0, 500)}\n\`\`\`` : "",
        "",
        `Type **/approve ${req.requestId}** to allow or **/reject ${req.requestId}** to deny.`,
      ].join("\n"),
    });

    // Auto-fill the input with the approve command
    this.input.setValue(`/approve ${req.requestId}`);
    this.requestRender();
  }

  // ─── Autocomplete ───────────────────────────────────────────────

  private triggerAutocomplete(): void {
    const text = this.input.getValue();
    this.acManager.queryImmediate(text, text.length);
  }

  /** Insert the currently highlighted autocomplete item into the input. */
  private applyAutocompleteSelection(): void {
    const item = this.autocomplete.getSelectedItem();
    if (!item) return;
    const ctx = this.acManager.activeContext;
    if (ctx?.token) {
      // Replace only the token portion, preserving the prefix.
      // Strip the longest suffix of `before` that is also a prefix of
      // the item value, so trigger characters aren't duplicated —
      // e.g. slash command "/model" with before="/", or attachment
      // "@file" with before="check @" (previously produced "@@file").
      const before = ctx.text.slice(0, ctx.tokenStart);
      const after = ctx.text.slice(ctx.tokenStart + ctx.token.length);
      let value = item.value;
      const maxOverlap = Math.min(before.length, value.length);
      for (let len = maxOverlap; len > 0; len--) {
        if (before.endsWith(value.slice(0, len))) {
          value = value.slice(len);
          break;
        }
      }
      this.input.setValue(before + value + after, (before + value).length);
    } else {
      this.input.setValue(item.value);
    }
    this.autocomplete.hide();
    this.requestRender();
  }

  private handleInterrupt(): void {
    if (this.state.streaming) {
      this.client.abort();
      this.state.streaming = false;
      // Mark the in-progress assistant message as stopped so the partial
      // content (thinking, text, tool calls) is preserved and visible —
      // matching the GUI's behaviour of keeping the aborted reply.
      this.chat.markLastAssistantStopped();
      this.requestRender();
      return;
    }
    // Not streaming: exit the app
    this.running = false;
    setImmediate(async () => {
      await this.stop();
      process.exit(0);
    });
  }

  // ─── Actions ────────────────────────────────────────────────────────────

  private async handleSubmit(value: string): Promise<void> {
    if (!value.trim()) return;

    this.input.setValue("");
    this.requestRender();

    // Handle slash commands locally (don't send to LLM)
    if (value.startsWith("/")) {
      const parts = value.slice(1).split(/\s+/);
      const cmd = parts[0].toLowerCase();
      const arg = parts.slice(1).join(" ");

      if (cmd === "model") {
        if (arg) {
          // Set model directly — agent resolves and stores provider/id
          try {
            await this.client.setModel(arg);
            await this.refresh();
            this.tuiSettings.defaultModel = this.state.model;
            this.saveTuiSettings();
            this.chat.addMessage({
              id: crypto.randomUUID(),
              role: "system",
              content: `Model: ${this.state.model}`,
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

      if (cmd === "help") {
        this.showHelpOverlay();
        return;
      }

      if (cmd === "reload") {
        try {
          const result = await this.client.reloadConfig();
          await this.refresh();
          const skillList = result.skills?.length
            ? result.skills.length + " skills loaded"
            : "no skills found";
          const ctxList = result.contextFiles?.length
            ? ", " + result.contextFiles.join(", ")
            : "";
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Reloaded: ${skillList}${ctxList}`,
          });
        } catch (err) {
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Reload failed: ${err}`,
          });
        }
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
          content: "Session export is not available in the TUI.",
        });
        return;
      }

      if (cmd === "import") {
        this.chat.addMessage({
          id: crypto.randomUUID(),
          role: "system",
          content: "Session import is not available in the TUI.",
        });
        return;
      }

      if (cmd === "clone") {
        try {
          const result = await this.client.clone();
          if (!result.cancelled) {
            await this.refresh();
            await this.loadSessionMessages();
            this.chat.addMessage({
              id: crypto.randomUUID(),
              role: "system",
              content: "Session cloned — continue in new branch.",
            });
          }
        } catch (err) {
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Failed to clone session: ${err}`,
          });
        }
        return;
      }

      if (cmd === "fork") {
        try {
          const result = await this.client.getForkMessages();
          const messages = (result as { messages?: Array<{ id: string; role: string; content: string; timestamp: string }> }).messages || [];
          if (messages.length === 0) {
            this.chat.addMessage({
              id: crypto.randomUUID(),
              role: "system",
              content: "No user messages to fork from.",
            });
            return;
          }
          const items: SelectItem[] = messages.map((m, i) => ({
            value: m.id,
            label: `#${i + 1}  ${m.timestamp || ""}`,
            description: (m.content || "").substring(0, 70),
          }));
          const sl = new SelectList({
            title: "Fork from message",
            items,
            maxVisible: 15,
            onSelect: async (item) => {
              try {
                const r = await this.client.fork(item.value);
                if (!r.cancelled) {
                  await this.refresh();
                  await this.loadSessionMessages();
                  this.chat.addMessage({
                    id: crypto.randomUUID(),
                    role: "system",
                    content: `Forked from ${item.label}.`,
                  });
                }
              } catch (err) {
                this.chat.addMessage({
                  id: crypto.randomUUID(),
                  role: "system",
                  content: `Failed to fork: ${err}`,
                });
              }
              this.hideOverlay();
            },
            onCancel: () => {
              this.hideOverlay();
            },
          });
          this.showOverlay(sl);
        } catch (err) {
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Failed to load fork messages: ${err}`,
          });
        }
        return;
      }

      if (cmd === "tree") {
        try {
          const r = await this.client.listSessions();
          const sessions = r.sessions;
          if (sessions.length === 0) {
            this.chat.addMessage({
              id: crypto.randomUUID(),
              role: "system",
              content: "No sessions found.",
            });
            return;
          }
          // Group sessions by cwd, build tree from parent_session_id
          const grouped = new Map<string, typeof sessions>();
          for (const s of sessions) {
            const cwd = s.cwd || "";
            if (!grouped.has(cwd)) grouped.set(cwd, []);
            grouped.get(cwd)!.push(s);
          }
          // Build flat tree lines (indented by parent relationship)
          const items: SelectItem[] = [];
          for (const [, group] of grouped) {
            // Build parent->children map
            const children = new Map<string, typeof sessions>();
            const roots: typeof sessions = [];
            for (const s of group) {
              const parentId = s.parent_session_id || "";
              if (parentId && group.some((g) => g.id === parentId)) {
                if (!children.has(parentId)) children.set(parentId, []);
                children.get(parentId)!.push(s);
              } else {
                roots.push(s);
              }
            }
            // Flatten recursively, tracking "last child" at each depth for
            // correct tree-drawing (│ for ancestors with more children after).
            const flatten = (list: typeof sessions, depth: number, ancestorsLast: boolean[]) => {
              list.sort((a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime());
              for (let i = 0; i < list.length; i++) {
                const s = list[i];
                const isLast = i === list.length - 1;
                const hasChildren = children.has(s.id) && (children.get(s.id)?.length ?? 0) > 0;
                // Build prefix: ancestor lines + connector
                let prefix = "";
                for (let d = 0; d < depth; d++) {
                  prefix += ancestorsLast[d] ? "  " : "│ ";
                }
                if (depth > 0) {
                  prefix += isLast ? "└─ " : "├─ ";
                }
                const currentMarker = s.id === this.state.sessionId ? "▶ " : "  ";
                const streamingMark = (s as any).is_streaming ? "● " : "";
                const label = `${currentMarker}${streamingMark}${prefix}${s.session_name || (s as any).first_message || s.id}`;
                items.push({
                  value: s.id,
                  label,
                  description: `${s.model} · ${(s as any).query_count ?? "?"}Q · ${new Date(s.updated_at).toLocaleString()}`,
                });
                if (hasChildren) {
                  flatten(children.get(s.id)!, depth + 1, [...ancestorsLast, isLast]);
                }
              }
            };
            flatten(roots, 0, []);
          }
          const treeList = new SelectList({
            title: "Session Tree",
            items,
            maxVisible: 20,
            onSelect: async (item) => {
              try {
                if (item.value !== this.state.sessionId) {
                  await this.client.switchSession(item.value);
                  await this.refresh();
                  await this.loadSessionMessages();
                  this.chat.addMessage({
                    id: crypto.randomUUID(),
                    role: "system",
                    content: `Switched to: ${item.label}`,
                  });
                }
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
          this.showOverlay(treeList);
        } catch (err) {
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Failed to load session tree: ${err}`,
          });
        }
        return;
      }

      if (cmd === "new") {
        try {
          const result = await this.client.newSession();
          if (result.sessionId) {
            await this.refresh();
            this.chat.addMessage({
              id: crypto.randomUUID(),
              role: "system",
              content: "New session started.",
            });
          }
        } catch {
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: "Not connected to agent.",
          });
        }
        return;
      }

      if (cmd === "name") {
        if (!arg) {
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: "Usage: /name <session name>",
          });
        } else {
          try {
            await this.client.setSessionName(arg);
            this.chat.addMessage({
              id: crypto.randomUUID(),
              role: "system",
              content: `Session name set to: ${arg}`,
            });
          } catch (err) {
            this.chat.addMessage({
              id: crypto.randomUUID(),
              role: "system",
              content: `Failed to set session name: ${err}`,
            });
          }
        }
        return;
      }

      if (cmd === "scoped-models") {
        try {
          const models = await this.client.listModels();
          const enabledSet = new Set(this.enabledModelIds ?? models.map((m) => m.provider ? `${m.provider}/${m.id}` : m.id));
          const selector = new ScopedModelsSelector({
            allModels: models,
            enabledModelIds: enabledSet,
            onSave: async (enabledIds) => {
              try {
                this.enabledModelIds = enabledIds;
                this.saveTuiSettings();
                this.chat.addMessage({
                  id: crypto.randomUUID(),
                  role: "system",
                  content: `Model scope saved (${enabledIds.length}/${models.length} enabled)`,
                });
              } catch (err) {
                this.chat.addMessage({
                  id: crypto.randomUUID(),
                  role: "system",
                  content: `Failed to save model scope: ${err}`,
                });
              }
              this.hideOverlay();
            },
            onCancel: () => {
              this.hideOverlay();
            },
          });
          this.showOverlay(selector);
        } catch (err) {
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Failed to load models: ${err}`,
          });
        }
        return;
      }

      if (cmd === "cwd" && arg) {
        try {
          let resolved = arg;
          if (resolved === "~") {
            resolved = os.homedir();
          } else if (resolved.startsWith("~/")) {
            resolved = path.join(os.homedir(), resolved.slice(2));
          } else if (!path.isAbsolute(resolved)) {
            resolved = path.resolve(this.state.cwd ?? os.homedir(), resolved);
          }
          await this.client.setCwd(resolved);
          this.state.cwd = resolved;
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Working directory: ${resolved}`,
          });
        } catch (err) {
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Failed to change directory: ${err}`,
          });
        }
        return;
      }

      if (cmd === "approve" && arg) {
        try {
          await this.client.approvalDecision(arg, true);
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Approved request: ${arg}`,
          });
        } catch (err) {
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Failed to approve: ${err}`,
          });
        }
        return;
      }

      if (cmd === "reject" && arg) {
        try {
          await this.client.approvalDecision(arg, false, "rejected by user");
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Rejected request: ${arg}`,
          });
        } catch (err) {
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Failed to reject: ${err}`,
          });
        }
        return;
      }

      if (cmd === "stop") {
        try {
          await this.client.abort();
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: "Stopped current generation.",
          });
        } catch (err) {
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Failed to stop: ${err}`,
          });
        }
        return;
      }

      if (cmd === "status") {
        try {
          const s = await this.client.getState();
          const models = await this.client.listModels();
          const currentModel = models.find((m) => m.id === s.model);
          const modelInfo = currentModel
            ? [
                `**Model:** ${currentModel.label} (\`${currentModel.id}\`)`,
                `**Provider:** ${currentModel.provider}`,
                `**Image support:** ${currentModel.supportsImages ? "yes" : "no"}`,
                `**Context window:** ${(currentModel.contextWindow / 1000).toFixed(0)}K`,
              ]
            : [`**Model:** ${s.model || "(unknown)"}`];
          const lines = [
            ...modelInfo,
            "",
            `**Session:** ${s.sessionId || "(none)"}`,
            `**CWD:** ${s.cwd || "(none)"}`,
            `**Thinking:** ${s.thinkingLevel}`,
            `**Permission:** ${s.permissionLevel ?? "all"}`,
            `**Queries:** ${s.queryCount}`,
            `**Auto compaction:** ${s.autoCompactionEnabled ? "on" : "off"}`,
            `**Streaming:** ${s.isStreaming ? "yes" : "no"}`,
            "",
            `**Context:** ${s.contextTokens ?? 0} / ${s.contextWindow ?? 0} (${(s.contextPercent ?? 0).toFixed(1)}%)`,
            `**Tokens:** ${s.tokensIn ?? 0} in / ${s.tokensOut ?? 0} out`,
            `**Cost:** ¥${(s.totalCost ?? 0).toFixed(4)}`,
          ];
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: lines.join("\n"),
          });
        } catch (err) {
          this.chat.addMessage({
            id: crypto.randomUUID(),
            role: "system",
            content: `Failed to get status: ${err}`,
          });
        }
        return;
      }
    }

    // Regular prompt - send to server
    this.chat.addMessage({
      id: crypto.randomUUID(),
      role: "user",
      content: value,
    });

    if (this.state.streaming) {
      // Already streaming — queue as follow-up, processed after current turn finishes
      try {
        await this.client.followUp(value);
      } catch {
        // Ignore followUp errors; if agent is unreachable prompt would also fail
      }
      this.requestRender();
      return;
    }

    this.state.streaming = true;
    this.requestRender();

    try {
      await this.client.prompt(value);
    } catch (err: any) {
      this.state.streaming = false;
      const msg = err?.message || String(err);
      // Transport errors: prompt may have reached the agent anyway.
      // Stream events will arrive once the connection recovers.
      if (!msg.includes("transport") && !msg.includes("14 UNAVAILABLE")
          && !msg.includes("Connect Failed") && !msg.includes("ECONNREFUSED")) {
        this.chat.addMessage({
          id: crypto.randomUUID(),
          role: "system",
          content: "Not connected to agent. Start the agent or check the gRPC connection.",
        });
      }
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

    // Banner: "future-tui vX.X.X". Prefer the agent's reported version (gRPC
    // handshake); fall back to this binary's injected version.
    const version = this.state.version || VERSION;
    addColored(`${fg(151, bold("future-tui"))}${fg(245, " v" + version)}`, (t) => t);

    // Shortcuts line (truncate to fit terminal width)
    const termW = this.terminal.columns;
    const shortcuts = truncateToWidth("ctrl+c interrupt · ctrl+p model · ctrl+t thinking · / commands", termW - 4);
    addColored(shortcuts, dim_);

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

    // Skills (wrap to fit terminal width)
    if (this.state.skills.length > 0) {
      const skillsList = "[skills] " + this.state.skills.join(", ");
      const lines = wrapTextWithAnsi(dim_(skillsList), this.terminal.columns - 4);
      add(lines.join("\n"));
    }

    // Extensions (truncate to fit terminal width)
    if (this.state.extensions.length > 0) {
      add("");
      addColored("[Extensions]", sectionHdr);
      const extList = " " + this.state.extensions.join(", ");
      addColored(extList, dim_);
    }
  }

  private async loadSessionMessages(): Promise<void> {
    try {
      const result = await this.client.getMessages();
      const messages = (result as { messages?: Record<string, unknown>[] }).messages || [];

      this.chat.clearMessages();

      for (const msg of messages) {
        const role = (msg.role as string) || "";
        // Only render user, assistant, and tool messages
        if (!["user", "assistant", "tool"].includes(role)) continue;

        let content = "";
        const raw = msg.content;
        if (typeof raw === "string") {
          content = raw;
        } else if (Array.isArray(raw)) {
          content = (raw as Array<Record<string, unknown>>)
            .map((block) => {
              // TextBlock uses "text", ToolResultBlock uses "content"
              if (typeof block.text === "string") return block.text;
              if (typeof block.content === "string") return block.content;
              return "";
            })
            .filter(Boolean)
            .join("");
        }

        const toolCalls = msg.tool_calls as Array<Record<string, unknown>> | undefined;
        if (!content && !toolCalls?.length) continue;

        this.chat.addMessage({
          id: (msg.id as string) || crypto.randomUUID(),
          role: role as ChatMessage["role"],
          content,
          name: (msg.name as string) || undefined,
          tool: (msg.tool_call_id as string) || undefined,
          toolArgs: (msg.tool_args as string) || undefined,
          thinking: (msg.reasoning_content as string) || undefined,
          // Historical tool messages: check content for error prefix
          toolStatus: role === "tool"
            ? (content.startsWith("Error:") ? "error" as const : "complete" as const)
            : undefined,
        });
      }

      this.requestRender(true);
    } catch {
      // Session messages not available — proceed with empty chat
    }
  }

  private loadTuiSettings(): void {
    try {
      const data = fs.readFileSync(this.tuiSettingsPath, "utf-8");
      this.tuiSettings = JSON.parse(data);
    } catch { /* file doesn't exist yet — use defaults */ }
    if (this.tuiSettings.enabledModelIds) {
      this.enabledModelIds = this.tuiSettings.enabledModelIds;
    }
  }

  private saveTuiSettings(): void {
    this.tuiSettings.enabledModelIds = this.enabledModelIds ?? undefined;
    try {
      fs.mkdirSync(path.dirname(this.tuiSettingsPath), { recursive: true });
      fs.writeFileSync(this.tuiSettingsPath, JSON.stringify(this.tuiSettings, null, 2));
    } catch { /* ignore write errors */ }
  }

  private async applyTuiDefaults(): Promise<void> {
    const s = this.tuiSettings;
    try {
      if (s.defaultModel) {
        await this.client.setModel(s.defaultModel);
      }
      if (s.defaultThinkingLevel) {
        await this.client.setThinkingLevel(s.defaultThinkingLevel as "off" | "minimal" | "low" | "medium" | "high" | "xhigh");
        this.state.thinking = s.defaultThinkingLevel;
      }
      if (s.defaultPermissionLevel) {
        await this.client.setPermissionLevel(s.defaultPermissionLevel as "all" | "workspace" | "none");
      }
      // Re-read agent state so footer reflects changes
      await this.refresh();
    } catch { /* ok if agent doesn't support a command yet */ }
  }

  private async refresh(): Promise<void> {
    try {
      const s = await this.client.getState();
      this.state.model = s.model ?? "(no model)";
      this.state.thinking = s.thinkingLevel;
      this.state.sessionId = s.sessionId ?? this.state.sessionId;
      this.state.cwd = s.cwd ?? "";
      this.state.version = s.version ?? "";
      this.state.skills = (s.skills ?? []).slice().sort((a, b) => a.localeCompare(b));
      this.state.contextFiles = s.contextFiles ?? [];
      this.state.extensions = s.extensions ?? [];
      this.state.contextTokens = s.contextTokens ?? 0;
      this.state.contextWindow = s.contextWindow ?? 0;
      this.state.contextPercent = s.contextPercent ?? 0;
      this.state.tokensIn = s.tokensIn ?? 0;
      this.state.tokensOut = s.tokensOut ?? 0;
      this.state.tokensCacheR = s.tokensCacheR ?? 0;
      this.state.tokensCacheW = s.tokensCacheW ?? 0;
      this.state.totalCost = s.totalCost ?? 0;
      this.state.explicitSession = s.explicitSession ?? false;
      this.state.autoCompactionEnabled = s.autoCompactionEnabled ?? true;
      
      // Update client's session ID if server returned a different one.
      // Compare against the client's subscribed session (not state.sessionId,
      // which was already updated above — that comparison was always false,
      // leaving the event stream stuck on the old session).
      if (s.sessionId && s.sessionId !== this.client.getCurrentSessionId()) {
        this.client.setCurrentSessionId(s.sessionId);
        this.client.connectEvents();
      }
    } catch {
      // Keep last known model; footer briefly showing "(not connected)" is
      // confusing during transient reconnects.
      if (!this.state.model || this.state.model === "(no model)") {
        this.state.model = "(not connected)";
      }
    }
  }

  async showModelSelector(): Promise<void> {
    let models: string[] = [];
    try {
      const allModels = await this.client.listModels();
      // /model shows all models (scoping only applies to ctrl+p cycling).
      models = allModels
        .map((m) => m.provider ? `${m.provider}/${m.id}` : m.id)
        .sort((a, b) => a.localeCompare(b));
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
          this.tuiSettings.defaultModel = item.value;
          this.saveTuiSettings();
          await this.refresh();
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
      // If scoped models are set, cycle within them locally.
      if (this.enabledModelIds && this.enabledModelIds.length > 0) {
        const current = this.state.model;
        const idx = this.enabledModelIds.indexOf(current);
        const nextIdx = idx < 0 ? 0 : (idx + 1) % this.enabledModelIds.length;
        const nextModel = this.enabledModelIds[nextIdx];
        await this.client.setModel(nextModel);
        this.state.model = nextModel;
        this.tuiSettings.defaultModel = nextModel;
        this.saveTuiSettings();
      } else {
        await this.client.cycleModel();
        await this.refresh();
      }
    } catch { /* ignore */ }
    this.requestRender();
  }

  private async cycleThinking(): Promise<void> {
    try {
      const r = await this.client.cycleThinkingLevel();
      if (r) {
        this.state.thinking = r.level;
        this.tuiSettings.defaultThinkingLevel = r.level;
        this.saveTuiSettings();
      }
    } catch { /* ignore */ }
    this.requestRender();
  }

  // ─── Overlays ───────────────────────────────────────────────────────────

  private showHelpOverlay(): void {
    const helpComponent: Component = {
      render: (width: number) => renderHelp(width),
      invalidate: () => {},
    };
    this.showOverlay(helpComponent);
  }

  showOverlay(component: Component, options?: OverlayOptions): OverlayHandle {
    const preFocus = this.focusedComponent;
    const focusOrder = ++this.focusOrderCounter;
    const entry = { component, options, preFocus, hidden: false, focusOrder };
    this.overlayStack.push(entry);

    // Auto-focus unless nonCapturing 
    if (!options?.nonCapturing) {
      this.setFocus(component);
    }
    this.terminal.hideCursor();
    this.requestRender(true);

    return {
      hide: () => {
        const index = this.overlayStack.indexOf(entry);
        if (index !== -1) {
          this.overlayStack.splice(index, 1);
          if (this.focusedComponent === component) {
            this.restoreFocus(entry);
          }
          if (this.overlayStack.length === 0) this.terminal.hideCursor();
          this.requestRender(true);
        }
      },
      setHidden: (h: boolean) => {
        if (entry.hidden === h) return;
        entry.hidden = h;
        if (h && this.focusedComponent === component) {
          // If hiding this overlay, move focus to next visible overlay
          const top = this.getTopOverlay();
          if (top) {
            this.setFocus(top);
          } else {
            this.restoreFocus(entry);
          }
          this.terminal.hideCursor();
        } else if (!h && !entry.options?.nonCapturing && this.isOverlayVisible(entry, this.terminal.columns, this.terminal.rows)) {
          entry.focusOrder = ++this.focusOrderCounter;
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
      this.setFocus(this.input);
    }
  }

  hideOverlay(): void {
    const entry = this.overlayStack.pop();
    if (!entry) return;
    if (this.focusedComponent === entry.component) {
      this.restoreFocus(entry);
    }
    this.requestRender(true);
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

  private compositeLineAt(base: string, overlay: string, col: number, overlayWidth: number, totalWidth?: number): string {
    if (isImageLine(base)) return base;

    const tw = totalWidth ?? this.terminal.columns;
    const afterStart = col + overlayWidth;
    const baseSegs = extractSegments(base, col, afterStart, tw - afterStart, true);

    // Extract overlay with width tracking
    const overlayClean = stripAnsiCodes(overlay);
    const overlayVisWidth = visibleWidth(overlayClean);

    // Pad segments to target widths
    const beforePad = Math.max(0, col - baseSegs.beforeWidth);
    const overlayPad = Math.max(0, overlayWidth - overlayVisWidth);
    const actualBeforeWidth = Math.max(col, baseSegs.beforeWidth);
    const actualOverlayWidth = Math.max(overlayWidth, overlayVisWidth);
    const afterTarget = Math.max(0, tw - actualBeforeWidth - actualOverlayWidth);
    const afterPad = Math.max(0, afterTarget - baseSegs.afterWidth);

    // Compose result with reset marker between segments
    const reset = "\x1b[0m";
    const result =
      baseSegs.before +
      " ".repeat(beforePad) +
      reset +
      overlay +
      " ".repeat(overlayPad) +
      reset +
      baseSegs.after +
      " ".repeat(afterPad);

    // Final safeguard: verify and truncate to terminal width
    const resultWidth = visibleWidth(result);
    if (resultWidth <= tw) return result;
    return sliceByColumn(result, 0, tw);
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
    let sessions: { id: string; session_name?: string; first_message?: string; query_count?: number; model: string; updated_at: string; is_streaming?: boolean }[] = [];
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
      label: s.session_name || (s as any).first_message || s.id,
      description: `${s.is_streaming ? "● " : ""}${s.model} · ${s.query_count ?? "?"}Q · ${new Date(s.updated_at).toLocaleString()}`,
    }));

    const sl = new SelectList({
      title: "Sessions",
      items,
      maxVisible: 15,
      onSelect: async (item) => {
        try {
          await this.client.switchSession(item.value);
          await this.refresh();
          await this.loadSessionMessages();
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

  // ─── Rendering (differential with synchronized output) ──────────

  /**
   * Request a render. Uses dual-phase scheduling:
   * - force: process.nextTick for an immediate full redraw
   * - ambient: setTimeout-based coalescing at ~30fps (MIN_RENDER_INTERVAL_MS)
   */
  private requestResizeRender(): void {
    if (this.resizeDebounceTimer) clearTimeout(this.resizeDebounceTimer);
    this.resizeDebounceTimer = setTimeout(() => {
      this.resizeDebounceTimer = undefined;
      this.requestRender();
    }, App.RESIZE_DEBOUNCE_MS);
  }

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
    if (!this.running || !this.renderRequested) return;
    // Clear any pending timer so we re-evaluate with current state.
    if (this.renderTimer) {
      clearTimeout(this.renderTimer);
      this.renderTimer = undefined;
    }
    const elapsed = performance.now() - this.lastRenderAt;
    const delay = Math.max(0, App.MIN_RENDER_INTERVAL_MS - elapsed);
    this.renderTimer = setTimeout(() => {
      this.renderTimer = undefined;
      if (!this.running || !this.renderRequested) return;
      this.renderRequested = false;
      this.lastRenderAt = performance.now();
      this.doRender();
      if (this.renderRequested) this.scheduleRender();
      else if (this.state.streaming) this.requestRender();
    }, delay);
  }

  // ─── Line resets / cursor extraction ───────────────────────────────

  private applyLineResets(lines: string[]): string[] {
    const reset = App.SEGMENT_RESET;
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      if (!line) continue;
      if (!isImageLine(line)) {
        lines[i] = normalizeTerminalOutput(line) + reset;
      }
    }
    return lines;
  }

  private extractCursorPosition(lines: string[], height: number): { row: number; col: number } | null {
    const viewportTop = Math.max(0, lines.length - height);
    for (let row = lines.length - 1; row >= viewportTop; row--) {
      const line = lines[row];
      if (!line) continue;
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
    this.terminal.write(`\x1b[${cursorPos.col + 1}G`);
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

    return deleteKittyImages(ids);
  }

  // ─── Main Render Pipeline ──────────────────────────────────────────

  private doRender(): void {
    if (!this.running) return;
    if (this.state.streaming) this.state.spinnerFrame++;
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
      spinnerFrame: this.state.spinnerFrame,
      contextTokens: this.state.contextTokens,
      contextWindow: this.state.contextWindow,
      contextPercent: this.state.contextPercent,
      tokensIn: this.state.tokensIn,
      tokensOut: this.state.tokensOut,
      tokensCacheR: this.state.tokensCacheR,
      tokensCacheW: this.state.tokensCacheW,
      totalCost: this.state.totalCost,
      autoCompactionEnabled: this.state.autoCompactionEnabled,
      toolElapsed: this.state.toolStartTime > 0
        ? Math.floor((performance.now() - this.state.toolStartTime) / 1000)
        : 0,
    };
    this.footer.setData(footerData);

    const footerRendered = this.footer.render(W);
    const footerLines = footerRendered.length;
    const editorLines = this.input.render(W);
    const editorHeight = editorLines.length;

    // Set chat viewport based on remaining space
    const chatHeight = H - editorHeight - footerLines;
    this.chat.setViewportHeight(Math.max(1, chatHeight));

    // Build render output: chat + editor + footer
    const chatLines = this.chat.render(W);
    let newLines = [...chatLines, ...editorLines, ...footerRendered];
    // Filter out undefined entries (can happen with certain input sequences)
    newLines = newLines.map((l) => l ?? "");

    // Composite overlays into rendered lines (before diff compare)
    if (this.overlayStack.length > 0) {
      newLines = this.compositeOverlays(newLines, W, H);
    }

    // Autocomplete popup (positioned above editor)
    if (this.autocomplete.isVisible()) {
      const acLines = this.autocomplete.render(W);
      if (acLines.length > 0) {
        // Position relative to the actual content length, NOT the terminal
        // height: when the chat doesn't fill the screen the editor sits above
        // row H, and H-based indexing would detach the popup from the input
        // and punch holes (undefined entries) into newLines.
        const editorIdx = newLines.length - footerLines - editorHeight;
        // Keep bottom-aligned with the editor (clip at the top when the popup
        // is taller than the space above); guard both bounds so writes never
        // create sparse holes or extend the array.
        let acTop = editorIdx - acLines.length;
        for (const line of acLines) {
          if (acTop >= 0 && acTop < editorIdx) newLines[acTop] = line;
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
      // PI_TUI_DEBUG: dump full render state to /tmp/tui/
      if (process.env.PI_TUI_DEBUG === "1") {
        try {
          const debugDir = path.join(os.tmpdir(), "tui");
          fs.mkdirSync(debugDir, { recursive: true });
          const ts = Date.now();
          const renderLines = [...newLines];
          const debugLines = [
            `=== RENDER ${ts} ===`,
            `reason: ${clear ? "clear" : "full"}, W=${W}, H=${H}`,
            `previousLines.length=${this.previousLines.length}`,
            `newLines.length=${newLines.length}`,
            `overlayStack.length=${this.overlayStack.length}`,
            `cursorPos=${cursorPos ? `${cursorPos.row}:${cursorPos.col}` : "null"}`,
            "--- lines ---",
          ];
          for (const line of renderLines) {
            debugLines.push(line.replace(/\x1b/g, "\\x1b"));
          }
          debugLines.push("--- end ---");
          fs.writeFileSync(path.join(debugDir, `render-${ts}.log`), debugLines.join("\\n"));
        } catch {}
      }

      let buf = SYNC_BEGIN;
      if (clear) {
        buf += deleteKittyImages(this.previousKittyImageIds);
        buf += "\x1b[H\x1b[2J"; // Home, clear screen (never clear scrollback — terminal scrollback holds TUI + bash history)
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
      this.previousKittyImageIds = collectKittyImageIds(newLines);
      this.previousWidth = W;
      this.previousHeight = H;
    };

    // Debug redraw logging 
    const debugRedraw = process.env.PI_DEBUG_REDRAW === "1";
    const logRedraw = (reason: string): void => {
      if (!debugRedraw) return;
      const logPath = path.join(os.homedir(), ".future", "tui", "debug.log");
      const msg = `[${new Date().toISOString()}] fullRender: ${reason} (prev=${this.previousLines.length}, new=${newLines.length}, w=${W}, h=${H})\n`;
      fs.appendFileSync(logPath, msg);
    };

    // First render — output without clearing (assumes clean screen)
    if (this.previousLines.length === 0 && !widthChanged && !heightChanged) {
      logRedraw("first render");
      fullRender(false);
      return;
    }

    // Width changes always need full re-render (wrapping changes)
    if (widthChanged) {
      logRedraw(`terminal width changed (${this.previousWidth} -> ${W})`);
      this.fullRedrawCount++;
      fullRender(true);
      return;
    }

    // Height changes normally need full re-render to keep visible viewport aligned,
    // but Termux changes height when the software keyboard shows or hides.
    // In that environment a full redraw would be wasteful — fall through to the
    // differential path instead. (Returning here without updating previousHeight
    // would freeze the TUI permanently: every later render also sees heightChanged.)
    if (heightChanged && !isTermuxSession()) {
      logRedraw(`terminal height changed (${this.previousHeight} -> ${H})`);
      this.fullRedrawCount++;
      fullRender(true);
      return;
    }

    // Content shrunk — clear empty rows when clearOnShrink enabled
    if (this.clearOnShrink && newLines.length < this.maxLinesRendered && this.overlayStack.length === 0) {
      logRedraw(`clearOnShrink (maxLinesRendered=${this.maxLinesRendered})`);
      this.fullRedrawCount++;
      fullRender(true);
      return;
    }

    // Ctrl+L forced clear screen
    if (this.forceClearNextRender) {
      this.forceClearNextRender = false;
      logRedraw("force clear (Ctrl+L)");
      this.fullRedrawCount++;
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
          logRedraw(`deleted lines moved viewport up (${targetRow} < ${prevViewportTop})`);
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
          logRedraw(`too many lines to clear (extraLines=${extraLines} > H=${H})`);
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
      this.previousKittyImageIds = collectKittyImageIds(newLines);
      this.previousWidth = W;
      this.previousHeight = H;
      this.previousViewportTop = prevViewportTop;
      return;
    }

    // Differential rendering can only touch what was actually visible.
    // If first changed line is above previous viewport, need a full redraw.
    if (firstChanged < prevViewportTop) {
      logRedraw(`first changed line above viewport (${firstChanged} < ${prevViewportTop})`);
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
      if (!line) continue;
      const isImage = isImageLine(line);
      if (!isImage && visibleWidth(line) > W) {
        // Truncate instead of crashing — graceful degradation
        line = truncateToWidth(line, W - 1);
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
    this.previousKittyImageIds = collectKittyImageIds(newLines);
    this.previousWidth = W;
    this.previousHeight = H;
  }

}
