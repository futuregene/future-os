/**
 * Spacer component — renders empty lines for vertical spacing.
 */

import type { Component } from "../tui.js";

export class Spacer implements Component {
  private lines: number;

  constructor(lines = 1) {
    this.lines = lines;
  }

  setLines(lines: number): void {
    this.lines = lines;
  }

  invalidate(): void {
    // No cached state
  }

  render(_width: number): string[] {
    const result: string[] = [];
    for (let i = 0; i < this.lines; i++) {
      result.push("");
    }
    return result;
  }
}
