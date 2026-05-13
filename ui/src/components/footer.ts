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
}

export class Footer {
  constructor(private width = 80, private theme?: Theme) {}

  setWidth(w: number): void {
    // noop for now
  }

  render(data: FooterData): string {
    const segments: string[] = [];

    // PWD (home replaced with ~)
    if (data.cwd) {
      const home = process.env.HOME || "";
      const pwd = home && data.cwd.startsWith(home)
        ? "~" + data.cwd.slice(home.length)
        : data.cwd;
      segments.push(dim(pwd));
    }

    // Model
    if (data.model) {
      segments.push(fg(252, this.shortenModel(data.model)));
    }

    // Streaming indicator
    if (data.streaming) {
      segments.push(fg(151, "◐ running"));
    }

    // Pending count
    if (data.pending && data.pending > 0) {
      segments.push(dim(`(${data.pending} queued)`));
    }

    // Session name
    if (data.sessionName) {
      segments.push(dim(data.sessionName));
    }

    // Shortcuts (right-aligned)
    const left = segments.join("  ");
    const right = dim("  [?] Help  [^C] Interrupt");

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
}
