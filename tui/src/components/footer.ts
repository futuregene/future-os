/**
 * Footer - status bar matching pi-mono style.
 * Shows: pwd, model, thinking, token stats, cost, context usage.
 */

import { CSI, RESET } from "../tui.js";
import { visibleWidth } from "../utils.js";
import type { Component } from "../tui.js";

export interface FooterData {
  cwd?: string;
  model?: string;
  thinking?: string;
  streaming?: boolean;
  sessionName?: string;
  pending?: number;
  contextTokens?: number;
  contextWindow?: number;
  contextPercent?: number;
  tokensIn?: number;
  tokensOut?: number;
  tokensCacheR?: number;
  tokensCacheW?: number;
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

    // Build left side: [pwd] [model] [thinking]
    const leftParts: string[] = [];

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
      rightParts.push(colorFg(COST_FG, `$${this.data.totalCost.toFixed(3)}`));
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
    const leftStr = leftParts.length > 0 ? left : baseFg;

    const leftLen = visibleWidth(leftStr);
    const rightLen = visibleWidth(right);
    const padding = Math.max(1, width - leftLen - rightLen - 1);

    // Use terminal default background (no explicit bg color)
    const line = leftStr + baseFg + " ".repeat(padding) + right;

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
