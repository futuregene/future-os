/**
 * Footer - the status bar at the bottom of the screen.
 * Shows model, thinking level, streaming status, shortcuts, etc.
 */

import { CSI, RESET } from "../tui.js";

export interface FooterData {
  model?: string;
  thinking?: string;
  streaming?: boolean;
  sessionName?: string;
  cwd?: string;
  pending?: number;
}

export interface FooterTheme {
  bg: number;
  fg: number;
  accent: number;
  dim: number;
}

export class Footer {
  private theme: FooterTheme = {
    bg: 235,
    fg: 252,
    accent: 39,
    dim: 245,
  };

  constructor(private width = 80) {}

  setWidth(w: number): void {
    this.width = w;
  }

  render(data: FooterData): string {
    const segments: string[] = [];

    // Model name
    if (data.model) {
      segments.push(this.accent(this.shortenModel(data.model)));
    }

    // Thinking level
    if (data.thinking && data.thinking !== "off") {
      segments.push(this.dim(`💭 ${data.thinking}`));
    }

    // Streaming indicator
    if (data.streaming) {
      segments.push(this.accent("◐"));
    }

    // Pending count
    if (data.pending && data.pending > 0) {
      segments.push(this.dim(`(${data.pending} queued)`));
    }

    // Session name
    if (data.sessionName) {
      segments.push(this.dim(data.sessionName));
    }

    // Build footer line
    const left = segments.join("  ");
    const right = this.dim("[?] Help");

    const used = this.stripAnsi(left).length + this.stripAnsi(right).length;
    const padding = Math.max(1, this.width - used - 1);
    const line = left + " ".repeat(padding) + right;

    return `${CSI}48;5;${this.theme.bg}m${CSI}38;5;${this.theme.fg}m${line}${RESET}`;
  }

  getHeight(): number {
    return 1;
  }

  private accent(text: string): string {
    return `${CSI}38;5;${this.theme.accent}m${text}${CSI}0m`;
  }

  private dim(text: string): string {
    return `${CSI}38;5;${this.theme.dim}m${text}${CSI}0m`;
  }

  private shortenModel(model: string): string {
    // Shorten provider/model format: "anthropic/claude-sonnet-4" -> "claude-sonnet-4"
    const parts = model.split("/");
    return parts[parts.length - 1] ?? model;
  }

  private stripAnsi(text: string): string {
    return text.replace(/\x1b\[[0-9;]*m/g, "");
  }
}
