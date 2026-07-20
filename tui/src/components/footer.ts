/**
 * Footer - status bar matching style.
 * Shows: pwd, model, thinking, token stats, cost, context usage.
 */

import { RESET } from "../tui.js";
import { visibleWidth, truncateToWidth } from "../utils.js";
import type { Component } from "../tui.js";

export interface FooterData {
  cwd?: string;
  model?: string;
  thinking?: string;
  streaming?: boolean;
  spinnerFrame?: number;
  pending?: number;
  contextTokens?: number;
  contextWindow?: number;
  contextPercent?: number;
  tokensIn?: number;
  tokensOut?: number;
  tokensCacheR?: number;
  tokensCacheW?: number;
  toolElapsed?: number;
  totalCost?: number;
  autoCompactionEnabled?: boolean;
}

const BASE_FG = 245;
const ACCENT_FG = 252;
const THINKING_FG = 117;
const TOKEN_FG = 71;
const COST_FG = 71;
const GREEN_FG = 71;
const YELLOW_FG = 226;
const RED_FG = 204;
const AUTO_FG = 240;
const SPINNER_FG = 39;

const SPINNER_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

function colorFg(c: number, text: string): string {
  return `\x1b[38;5;${c}m${text}\x1b[38;5;${BASE_FG}m`;
}

export class Footer implements Component {
  private data: FooterData = {};

  constructor(private width = 80) {}

  setData(data: FooterData): void {
    this.data = data;
  }

  handleInput(_data: string): void { /* no-op */ }

  invalidate(): void { /* no cache */ }

  setWidth(_w: number): void {}

  render(width: number): string[] {
    const baseFg = `\x1b[38;5;${BASE_FG}m`;

    // Build left side: [spinner] [pwd] [model] [thinking]
    const leftParts: string[] = [];

    // Spinner when streaming
    if (this.data.streaming) {
      const frameIdx = (this.data.spinnerFrame ?? 0) % SPINNER_FRAMES.length;
      leftParts.push(colorFg(SPINNER_FG, SPINNER_FRAMES[frameIdx]));
    }

    // Tool elapsed time
    if (this.data.toolElapsed !== undefined && this.data.toolElapsed > 0) {
      leftParts.push(colorFg(TOKEN_FG, `${this.data.toolElapsed}s`));
    }

    // PWD — uses default fg (245)
    if (this.data.cwd) {
      const home = process.env.HOME || "";
      const pwd = home && this.data.cwd.startsWith(home)
        ? "~" + this.data.cwd.slice(home.length)
        : this.data.cwd;
      leftParts.push(baseFg + pwd);
    }

    // Model — brighter fg (252), optional thinking level in blue
    if (this.data.model) {
      const modelShort = this.shortenModel(this.data.model);
      const thinking = this.data.thinking && this.data.thinking !== "off"
        ? colorFg(THINKING_FG, ` • ${this.data.thinking}`)
        : "";
      leftParts.push(colorFg(ACCENT_FG, modelShort) + thinking);
    }

    // Build right side: [token stats] [cost] [context usage]
    const rightParts: string[] = [];

    // Token stats: ↑Xk ↓Xk
    const tokenParts: string[] = [];
    if (this.data.tokensIn) tokenParts.push(`↑${this.fmtTokens(this.data.tokensIn)}`);
    if (this.data.tokensOut) tokenParts.push(`↓${this.fmtTokens(this.data.tokensOut)}`);
    if (this.data.tokensCacheR) tokenParts.push(`R${this.fmtTokens(this.data.tokensCacheR)}`);
    if (this.data.tokensCacheW) tokenParts.push(`W${this.fmtTokens(this.data.tokensCacheW)}`);
    if (tokenParts.length > 0) {
      rightParts.push(colorFg(TOKEN_FG, tokenParts.join(" ")));
    }

    // Cost
    if (this.data.totalCost !== undefined && this.data.totalCost > 0) {
      rightParts.push(colorFg(COST_FG, `¥${this.data.totalCost.toFixed(3)}`));
    }

    // Context usage: tokenCount/contextWindow (color based on percent fill)
    if (this.data.contextWindow) {
      const used = this.fmtTokens(this.data.contextTokens ?? 0);
      const win = this.fmtTokens(this.data.contextWindow);
      const pct = this.data.contextPercent ?? 0;
      // Color based on usage level
      const usedColor = pct < 70 ? GREEN_FG   // green < 70%
        : pct < 90 ? YELLOW_FG  // yellow 70-90%
        : RED_FG; // red > 90%
      let usageStr = colorFg(usedColor, used) + baseFg + `/${win}`;
      if (this.data.autoCompactionEnabled) {
        usageStr += colorFg(AUTO_FG, " (auto)");
      }
      rightParts.push(usageStr);
    }

    const left = leftParts.join(baseFg + "  ");
    const right = rightParts.join(baseFg + "  ");

    // Ensure the left part starts with baseFg even if leftParts is empty
    let leftStr = leftParts.length > 0 ? left : baseFg;

    let leftLen = visibleWidth(leftStr);
    const rightLen = visibleWidth(right);
    const avail = width - 1; // reserve 1 for safety margin

    // Both sides must be truncated on overflow: an over-wide line wraps
    // physically and desyncs the diff renderer's row tracking, which assumes
    // one logical line == one terminal row. Share the space — the right side
    // (tokens/cost/context) gets at most half so it stays visible even with
    // a deep cwd or long model name.
    let rightStr = right;
    if (leftLen + rightLen > avail) {
      const maxRight = Math.min(rightLen, Math.max(0, Math.floor(avail / 2)));
      rightStr = truncateToWidth(right, maxRight, { ellipsis: false });
      const maxLeft = Math.max(0, avail - visibleWidth(rightStr) - 1);
      // No ellipsis: the styled left string may end with an ANSI sequence,
      // and truncateToWidth's ellipsis replaces the last byte — which could
      // be the tail of an escape sequence and corrupt it.
      leftStr = truncateToWidth(leftStr, maxLeft, { ellipsis: false });
      leftLen = visibleWidth(leftStr);
    }

    const padding = Math.max(1, width - leftLen - visibleWidth(rightStr) - 1);
    const line = leftStr + baseFg + " ".repeat(padding) + rightStr;

    return [`${line}${RESET}`];
  }

  getHeight(): number {
    return 1;
  }

  private shortenModel(model: string): string {
    const parts = model.split("/");
    return parts[parts.length - 1] ?? model;
  }

  private fmtTokens(n: number): string {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
    if (n >= 1_000) return Math.round(n / 1_000) + "k";
    return String(n);
  }
}
