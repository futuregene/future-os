/**
 * Footer - status bar matching pi-mono style.
 * Shows: pwd, shortcuts, session info.
 */

import { CSI, RESET } from "../tui.js";
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

export class Footer {
  constructor(private width = 80, private theme?: Theme) {}

  setWidth(w: number): void {
    // noop for now
  }

  render(data: FooterData): string {
    // Build left side: [pwd] [model] [thinking] [streaming]
    const leftParts: string[] = [];

    // PWD
    if (data.cwd) {
      const home = process.env.HOME || "";
      const pwd = home && data.cwd.startsWith(home)
        ? "~" + data.cwd.slice(home.length)
        : data.cwd;
      leftParts.push(fg(245, pwd));
    }

    // Model + thinking (e.g., "deepseek-v4-flash • high")
    if (data.model) {
      const modelShort = this.shortenModel(data.model);
      const thinking = data.thinking && data.thinking !== "off"
        ? fg(117, ` • ${data.thinking}`)
        : "";
      leftParts.push(fg(252, modelShort) + thinking);
    }

    // Build right side: [stats] [shortcuts]
    const rightParts: string[] = [];

    // Token stats: ↑Xk ↓Xk R Xk
    const tokenParts: string[] = [];
    if (data.tokensIn) tokenParts.push(fg(71, `\u2191${this.fmtTokens(data.tokensIn)}`));
    if (data.tokensOut) tokenParts.push(fg(71, `\u2193${this.fmtTokens(data.tokensOut)}`));
    if (data.tokensCacheR) tokenParts.push(fg(71, `R${this.fmtTokens(data.tokensCacheR)}`));
    if (tokenParts.length > 0) {
      rightParts.push(fg(245, tokenParts.join(" ")));
    }

    // Cost
    if (data.totalCost !== undefined && data.totalCost > 0) {
      rightParts.push(fg(245, `$${data.totalCost.toFixed(3)}`));
    }

    // Context usage: X.X%/1.0M (auto)
    if (data.contextPercent !== undefined && data.contextWindow) {
      const pct = data.contextPercent.toFixed(1);
      const win = this.fmtTokens(data.contextWindow);
      // Color based on usage level
      const pctColor = data.contextPercent < 70 ? fg(71, pct)   // green < 70%
        : data.contextPercent < 90 ? fg(226, pct)  // yellow 70-90%
        : fg(204, pct); // red > 90%
      let usageStr = pctColor + fg(245, `%/`) + fg(245, win);
      if (data.autoCompactionEnabled) {
        usageStr += fg(240, ` (auto)`);
      }
      rightParts.push(usageStr);
    }

    // Shortcuts
    rightParts.push(fg(240, "[?] Help  [^C] Interrupt"));

    const left = leftParts.join("  ");
    const right = rightParts.join("  ");

    const leftLen = this.strip(left).length;
    const rightLen = this.strip(right).length;
    const padding = Math.max(1, this.width - leftLen - rightLen - 1);

    const line = left + " ".repeat(padding) + right;

    return `${CSI}48;5;235m${CSI}38;5;245m${line}${RESET}`;
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
