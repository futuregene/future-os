/**
 * Text component — multi-line text with word wrapping, padding, and background.
 */

import type { Component } from "../tui.js";
import { applyBackgroundToLine, visibleWidth, wrapTextWithAnsi } from "../utils.js";

export class Text implements Component {
  private text: string;
  private paddingX: number;
  private paddingY: number;
  private bgColor?: number;

  private cachedText?: string;
  private cachedWidth?: number;
  private cachedLines?: string[];

  constructor(text = "", paddingX = 1, paddingY = 0, bgColor?: number) {
    this.text = text;
    this.paddingX = paddingX;
    this.paddingY = paddingY;
    this.bgColor = bgColor;
  }

  setText(text: string): void {
    this.text = text;
    this.invalidate();
  }

  setBgColor(bgColor?: number): void {
    this.bgColor = bgColor;
    this.invalidate();
  }

  invalidate(): void {
    this.cachedText = undefined;
    this.cachedWidth = undefined;
    this.cachedLines = undefined;
  }

  render(width: number): string[] {
    if (this.cachedLines && this.cachedText === this.text && this.cachedWidth === width) {
      return this.cachedLines;
    }

    if (!this.text || this.text.trim() === "") {
      return (this.cachedText = this.text, this.cachedWidth = width, this.cachedLines = [], []);
    }

    const normalized = this.text.replace(/\t/g, "   ");
    const contentWidth = Math.max(1, width - this.paddingX * 2);
    const wrapped = wrapTextWithAnsi(normalized, contentWidth);

    const leftMargin = " ".repeat(this.paddingX);
    const rightMargin = " ".repeat(this.paddingX);
    const contentLines: string[] = [];

    for (const line of wrapped) {
      const withMargins = leftMargin + line + rightMargin;
      if (this.bgColor !== undefined) {
        contentLines.push(applyBackgroundToLine(withMargins, width, this.bgColor));
      } else {
        const pad = Math.max(0, width - visibleWidth(withMargins));
        contentLines.push(withMargins + " ".repeat(pad));
      }
    }

    const emptyLine = " ".repeat(width);
    const emptyLines: string[] = [];
    for (let i = 0; i < this.paddingY; i++) {
      emptyLines.push(this.bgColor !== undefined ? applyBackgroundToLine(emptyLine, width, this.bgColor) : emptyLine);
    }

    const result = [...emptyLines, ...contentLines, ...emptyLines];

    this.cachedText = this.text;
    this.cachedWidth = width;
    this.cachedLines = result;
    return result.length > 0 ? result : [""];
  }
}
