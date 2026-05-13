/**
 * Footer - status bar matching pi-mono style.
 * Shows: pwd, shortcuts, session info.
 */

import { CSI, RESET } from "../tui.js";
import type { Component } from "../tui.js";
import { fg, dim, type Theme } from "../theme.js";

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

export class Footer implements Component {
  private data: FooterData = {};

  constructor(private width = 80, private theme?: Theme) {}

  setData(data: FooterData): void {
    this.data = data;
  }

  handleInput(_data: string): void { /* no-op */ }

  invalidate(): void { /* no cache */ }

  setWidth(w: number): void {
    // noop for now
  }

  render(width: number): string[] {
    // Build left side: [pwd] [model] [thinking] [streaming]
    const leftParts: string[] = [];

    // PWD
    if (this.data.cwd) {
      const home = process.env.HOME || "";
      const pwd = home && this.data.cwd.startsWith(home)
        ? "~" + this.data.cwd.slice(home.length)
        : this.data.cwd;
      leftParts.push(fg(245, pwd));
    }

    // Model + thinking (e.g., "deepseek-v4-flash • high")
    if (this.data.model) {
      const modelShort = this.shortenModel(this.data.model);
      const thinking = this.data.thinking && this.data.thinking !== "off"
        ? fg(117, ` • ${this.data.thinking}`)
        : "";
      leftParts.push(fg(252, modelShort) + thinking);
    }

    // Build right side: [stats] [shortcuts]
    const rightParts: string[] = [];

    // Token stats: ↑Xk ↓Xk R Xk
    const tokenParts: string[] = [];
    if (this.data.tokensIn) tokenParts.push(fg(71, `\u2191${this.fmtTokens(this.data.tokensIn)}`));
    if (this.data.tokensOut) tokenParts.push(fg(71, `\u2193${this.fmtTokens(this.data.tokensOut)}`));
    if (this.data.tokensCacheR) tokenParts.push(fg(71, `R${this.fmtTokens(this.data.tokensCacheR)}`));
    if (tokenParts.length > 0) {
      rightParts.push(fg(245, tokenParts.join(" ")));
    }

    // Cost
    if (this.data.totalCost !== undefined && this.data.totalCost > 0) {
      rightParts.push(fg(245, `$${this.data.totalCost.toFixed(3)}`));
    }

    // Context usage: X.X%/1.0M (auto)
    if (this.data.contextPercent !== undefined && this.data.contextWindow) {
      const pct = this.data.contextPercent.toFixed(1);
      const win = this.fmtTokens(this.data.contextWindow);
      // Color based on usage level
      const pctColor = this.data.contextPercent < 70 ? fg(71, pct)   // green < 70%
        : this.data.contextPercent < 90 ? fg(226, pct)  // yellow 70-90%
        : fg(204, pct); // red > 90%
      let usageStr = pctColor + fg(245, `%/`) + fg(245, win);
      if (this.data.autoCompactionEnabled) {
        usageStr += fg(240, ` (auto)`);
      }
      rightParts.push(usageStr);
    }

    // Shortcuts
    const left = leftParts.join("  ");
    const right = rightParts.join("  ");

    const leftLen = this.strip(left).length;
    const rightLen = this.strip(right).length;
    const padding = Math.max(1, width - leftLen - rightLen - 1);

    const line = left + " ".repeat(padding) + right;

    return [`${CSI}48;5;235m${CSI}38;5;245m${line}${RESET}`];
  }

  getHeight(): number {
    return 1;
  }

  private strip(text: string): string {
    return text.replace(/\x1b\[[0-9;]*m/g, "");
  }

  private shortenModel(model: string): string {
    // Shorten provider/model: "anthropic/claude-sonnet-4" -> "claude-sonnet-4"
    const parts = model.split("/");
    return parts[parts.length - 1] ?? model;
  }

  private fmtTokens(n: number): string {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
    if (n >= 1_000) return Math.round(n / 1_000) + "k";
    return String(n);
  }
}
