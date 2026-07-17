/**
 * Autocomplete system — provider-based completion with debounce and cancellation.
 *
 */

import { fg, bold } from "../theme.js";
import type { Component } from "../tui.js";
import { visibleWidth } from "../utils.js";

// ─── Types ─────────────────────────────────────────────────────────────────

export interface AutocompleteItem {
  value: string;
  label: string;
  description?: string;
}

export interface AutocompleteContext {
  text: string;
  cursorPos: number;
  /** The token being completed at cursor position. */
  token: string;
  /** Start offset of the token being completed. */
  tokenStart: number;
}

export interface AutocompleteProvider {
  name: string;
  /** Return non-null context if this provider should handle the input. */
  match(text: string, cursorPos: number): AutocompleteContext | null;
  /** Return completion items for the matched context. */
  getCompletions(ctx: AutocompleteContext, signal: AbortSignal): Promise<AutocompleteItem[]>;
}

// ─── Autocomplete Manager ──────────────────────────────────────────────────

const DEBOUNCE_MS = 20;

export class AutocompleteManager {
  private providers: AutocompleteProvider[] = [];
  private abortController: AbortController | null = null;
  private debounceTimer: ReturnType<typeof setTimeout> | null = null;
  private pendingQuery: (() => void) | null = null;
  private lastText = "";
  private lastCursorPos = 0;
  /** The latest matched context (for token-aware completion). */
  activeContext: AutocompleteContext | null = null;

  /** Callback when items are ready (or empty to hide). */
  onItems?: (items: AutocompleteItem[]) => void;

  register(provider: AutocompleteProvider): () => void {
    this.providers.push(provider);
    return () => {
      const idx = this.providers.indexOf(provider);
      if (idx !== -1) this.providers.splice(idx, 1);
    };
  }

  /** Trigger a query for the given text. Debounced; cancels previous inflight queries. */
  query(text: string, cursorPos: number): void {
    this.lastText = text;
    this.lastCursorPos = cursorPos;

    // Cancel previous debounce
    if (this.debounceTimer) {
      clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }

    this.debounceTimer = setTimeout(() => {
      this.debounceTimer = null;
      this.runQuery(text, cursorPos);
    }, DEBOUNCE_MS);
  }

  /** Run query immediately (bypasses debounce). Used for Tab trigger. */
  queryImmediate(text: string, cursorPos: number): void {
    if (this.debounceTimer) {
      clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
    this.runQuery(text, cursorPos);
  }

  /** Re-run last query (for continued typing within same token). */
  refresh(): void {
    this.query(this.lastText, this.lastCursorPos);
  }

  private async runQuery(text: string, cursorPos: number): Promise<void> {
    // Cancel previous inflight request
    if (this.abortController) {
      this.abortController.abort();
    }
    this.abortController = new AbortController();
    const signal = this.abortController.signal;

    // Try each provider in registration order
    for (const provider of this.providers) {
      if (signal.aborted) return;
      const ctx = provider.match(text, cursorPos);
      if (!ctx) continue;

      try {
        const items = await provider.getCompletions(ctx, signal);
        if (signal.aborted) return;
        if (items.length > 0) {
          this.activeContext = ctx;
          this.onItems?.(items);
          return;
        }
      } catch {
        // Provider threw — try next
      }
    }

    // No provider matched or returned items
    this.activeContext = null;
    if (!signal.aborted) {
      this.onItems?.([]);
    }
  }

  destroy(): void {
    if (this.debounceTimer) clearTimeout(this.debounceTimer);
    if (this.abortController) this.abortController.abort();
    this.providers = [];
  }
}

// ─── Slash Command Provider ────────────────────────────────────────────────

export interface SlashCommand {
  value: string;
  label: string;
  description: string;
  /** If true, this command accepts a model name argument. */
  takesModelArg?: boolean;
  /** If true, this command accepts a session ID argument. */
  takesSessionArg?: boolean;
}

export class SlashCommandProvider implements AutocompleteProvider {
  name = "slash-command";

  constructor(
    private commands: SlashCommand[],
    private getModels?: () => Promise<string[]>,
    private getSessions?: () => Promise<string[]>,
  ) {}

  match(text: string, _cursorPos: number): AutocompleteContext | null {
    if (!text.startsWith("/")) return null;

    const spaceIdx = text.indexOf(" ");
    if (spaceIdx === -1) {
      // Typing command name: /mod...
      return { text, cursorPos: _cursorPos, token: text.slice(1), tokenStart: 1 };
    }

    const cmdName = text.slice(1, spaceIdx).toLowerCase();
    const cmd = this.commands.find((c) => c.value.slice(1).toLowerCase() === cmdName);
    if (!cmd) return null;

    const arg = text.slice(spaceIdx + 1);
    if (cmd.takesModelArg || cmd.takesSessionArg) {
      return {
        text,
        cursorPos: _cursorPos,
        token: arg,
        tokenStart: spaceIdx + 1,
      };
    }

    return null;
  }

  async getCompletions(ctx: AutocompleteContext, signal: AbortSignal): Promise<AutocompleteItem[]> {
    const text = ctx.text;
    const spaceIdx = text.indexOf(" ");

    if (spaceIdx === -1) {
      // Complete command name, sorted alphabetically
      const prefix = ctx.token.toLowerCase();
      return this.commands
        .filter((c) => c.label.toLowerCase().includes(prefix))
        .sort((a, b) => a.label.localeCompare(b.label))
        .map((c) => ({ value: c.value, label: c.label, description: c.description }));
    }

    // Complete argument
    const cmdName = text.slice(1, spaceIdx).toLowerCase();
    const cmd = this.commands.find((c) => c.value.slice(1).toLowerCase() === cmdName);
    if (!cmd) return [];

    const argPrefix = ctx.token.toLowerCase();

    if (cmd.takesModelArg && this.getModels) {
      if (signal.aborted) return [];
      try {
        const models = await this.getModels();
        return models
          .filter((m) => m.toLowerCase().includes(argPrefix))
          .slice(0, 20)
          .map((m) => ({
            value: `${cmd.value} ${m}`,
            label: m,
            description: "",
          }));
      } catch {
        return [];
      }
    }

    if (cmd.takesSessionArg && this.getSessions) {
      if (signal.aborted) return [];
      try {
        const sessions = await this.getSessions();
        return sessions
          .filter((s) => s.toLowerCase().includes(argPrefix))
          .slice(0, 20)
          .map((s) => ({
            value: `${cmd.value} ${s}`,
            label: s,
            description: "",
          }));
      } catch {
        return [];
      }
    }

    return [];
  }
}

// ─── File Path Provider ────────────────────────────────────────────────────

import * as fs from "fs";
import * as path from "path";

export class FilePathProvider implements AutocompleteProvider {
  name = "file-path";
  private cwd: string;

  constructor(cwd?: string) {
    this.cwd = cwd ?? process.cwd();
  }

  setCwd(cwd: string): void {
    this.cwd = cwd;
  }

  match(text: string, _cursorPos: number): AutocompleteContext | null {
    // Detect file path patterns: starts with . or contains / at cursor
    const prefix = text.slice(0, _cursorPos);
    // Look for the last path-like token
    const pathMatch = prefix.match(/(?:^|\s)([.]?[^\s]*\/[^\s]*|[.][^\s]*)$/);
    if (!pathMatch || !pathMatch[1]) return null;
    const token = pathMatch[1];
    const tokenStart = (pathMatch.index ?? 0) + pathMatch[0].indexOf(token);
    return { text, cursorPos: _cursorPos, token, tokenStart };
  }

  async getCompletions(ctx: AutocompleteContext, signal: AbortSignal): Promise<AutocompleteItem[]> {
    const token = ctx.token;
    // Resolve the partial path
    let dirPath: string;
    let filePrefix: string;

    try {
      const resolved = token.startsWith("~")
        ? path.join(process.env.HOME ?? "/", token.slice(1))
        : path.resolve(this.cwd, token);

      const stat = await fs.promises.stat(resolved).catch(() => null);
      if (stat?.isDirectory()) {
        dirPath = resolved;
        filePrefix = "";
      } else {
        dirPath = path.dirname(resolved);
        filePrefix = path.basename(resolved);
      }
    } catch {
      return [];
    }

    if (signal.aborted) return [];

    let entries: fs.Dirent[];
    try {
      entries = await fs.promises.readdir(dirPath, { withFileTypes: true });
    } catch {
      return [];
    }

    if (signal.aborted) return [];

    const matches = entries
      .filter((e) => e.name.toLowerCase().startsWith(filePrefix.toLowerCase()))
      .filter((e) => !e.name.startsWith(".")) // skip hidden
      .sort((a, b) => {
        // Directories first, then alphabetical
        if (a.isDirectory() && !b.isDirectory()) return -1;
        if (!a.isDirectory() && b.isDirectory()) return 1;
        return a.name.localeCompare(b.name);
      })
      .slice(0, 20)
      .map((e) => {
        const suffix = e.isDirectory() ? "/" : "";
        const full = path.join(dirPath, e.name + suffix);
        // Make path relative to cwd for display
        let display = full;
        if (full.startsWith(this.cwd)) {
          display = full.slice(this.cwd.length);
          if (display.startsWith("/")) display = display.slice(1);
        }
        return {
          value: display,
          label: display,
          description: e.isDirectory() ? "dir" : "",
        };
      });

    return matches;
  }
}

// ─── Attachment Provider ────────────────────────────────────────────────────

/**
 * Attachment provider: triggered by "@" for fuzzy file search.
 * Uses fd (when available) or falls back to find for fast fuzzy matching.
 */
export class AttachmentProvider implements AutocompleteProvider {
  name = "attachment";

  match(text: string, cursorPos: number): AutocompleteContext | null {
    const prefix = text.slice(0, cursorPos);
    // Match "@" at word boundary, possibly followed by partial filename
    const atMatch = prefix.match(/(?:^|\s)@([^\s]*)$/);
    if (!atMatch) return null;
    const token = atMatch[1] ?? "";
    const tokenStart = (atMatch.index ?? 0) + (atMatch[0].indexOf("@") >= 0 ? atMatch[0].indexOf("@") : 0) + 1;
    return { text, cursorPos, token, tokenStart };
  }

  async getCompletions(ctx: AutocompleteContext, signal: AbortSignal): Promise<AutocompleteItem[]> {
    const pattern = ctx.token.toLowerCase();
    if (pattern.length === 0) return [];

    let results: string[] = [];

    // Try fd first (fast, respects .gitignore)
    try {
      const cp = await import("node:child_process");
      const args = ["--hidden", "--type", "f", "--max-results", "50"];
      if (pattern.length > 0) args.push(pattern);
      const stdout = await new Promise<string>((resolve, reject) => {
        cp.execFile("fd", args, {
          cwd: process.cwd(),
          encoding: "utf-8",
          timeout: 3000,
          maxBuffer: 1024 * 1024,
        }, (err, stdout) => {
          if (err) reject(err);
          else resolve(stdout);
        });
      });
      results = stdout.trim().split("\n").filter(Boolean);
    } catch {
      // fd not available — fall back to native find / Get-ChildItem
      try {
        const cp = await import("node:child_process");
        const isWin = process.platform === "win32";
        const cmd = isWin ? "powershell" : "find";
        const args = isWin
          ? ["-NoProfile", "-Command", `Get-ChildItem -Path '.' -Recurse -File -Name '${pattern}*' | Select-Object -First 50`]
          : [".", "-name", `${pattern}*`, "-type", "f", "-maxdepth", "5"];
        const stdout = await new Promise<string>((resolve, reject) => {
          cp.execFile(cmd, args, {
            cwd: process.cwd(),
            encoding: "utf-8",
            timeout: 5000,
            maxBuffer: 1024 * 1024,
          }, (err, stdout) => {
            if (err) reject(err);
            else resolve(stdout);
          });
        });
        results = stdout.trim().split("\n").filter(Boolean);
      } catch {
        // Neither fd nor fallback available — no autocomplete results
      }
    }

    if (signal.aborted) return [];

    return results.map((filePath) => ({
      value: `@${filePath}`,
      label: filePath,
      description: "",
    }));
  }
}


export class AutocompletePopup implements Component {
  private items: AutocompleteItem[] = [];
  private selectedIndex = 0;
  private visible = false;
  private maxVisible = 10;

  show(items: AutocompleteItem[]): void {
    this.items = items;
    this.selectedIndex = 0;
    this.visible = true;
  }

  hide(): void {
    this.visible = false;
  }

  isVisible(): boolean {
    return this.visible;
  }

  handleInput(data: string): void {
    if (data === "up") this.selectPrev();
    else if (data === "down") this.selectNext();
  }

  invalidate(): void { /* no cache */ }

  getSelectedItem(): AutocompleteItem | null {
    if (!this.visible || this.items.length === 0) return null;
    return this.items[this.selectedIndex] ?? null;
  }

  selectNext(): void {
    if (this.items.length === 0) return;
    this.selectedIndex = (this.selectedIndex + 1) % this.items.length;
  }

  selectPrev(): void {
    if (this.items.length === 0) return;
    this.selectedIndex = this.selectedIndex === 0 ? this.items.length - 1 : this.selectedIndex - 1;
  }

  setMaxVisible(n: number): void {
    this.maxVisible = n;
  }

  render(width: number): string[] {
    if (!this.visible || this.items.length === 0) return [];

    const popupWidth = Math.min(width - 4, 48);
    const lines: string[] = [];

    lines.push(fg(244, "┌") + fg(239, "─".repeat(popupWidth)) + fg(244, "┐"));

    const start = Math.max(0, this.selectedIndex - this.maxVisible + 1);
    const end = Math.min(this.items.length, start + this.maxVisible);

    for (let i = start; i < end; i++) {
      const item = this.items[i];
      const isSelected = i === this.selectedIndex;
      const desc = item.description ? fg(245, ` ${item.description}`) : "";
      const label = (item.label + desc).slice(0, popupWidth - 4);

      if (isSelected) {
        const content = fg(151, bold("▶")) + " " + fg(252, label);
        const pad = popupWidth - 2 - visibleWidth(content);
        lines.push(fg(244, "│") + " " + content + " ".repeat(Math.max(0, pad)) + fg(244, "│"));
      } else {
        const content = "  " + label;
        const pad = popupWidth - 2 - visibleWidth(content);
        lines.push(fg(244, "│") + fg(245, content) + " ".repeat(Math.max(0, pad)) + fg(244, "│"));
      }
    }

    lines.push(fg(244, "└") + fg(239, "─".repeat(popupWidth)) + fg(244, "┘"));

    return lines;
  }

  height(): number {
    if (!this.visible || this.items.length === 0) return 0;
    return 2 + Math.min(this.items.length, this.maxVisible);
  }
}
