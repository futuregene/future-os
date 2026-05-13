/**
 * Box component — container that applies padding and background to children.
 */

import type { Component } from "../tui.js";
import { applyBackgroundToLine, visibleWidth } from "../utils.js";

type RenderCache = {
  childLines: string[];
  width: number;
  bgColor: number | undefined;
  lines: string[];
};

export class Box implements Component {
  children: Component[] = [];
  private paddingX: number;
  private paddingY: number;
  private bgColor?: number;
  private cache?: RenderCache;

  constructor(paddingX = 1, paddingY = 1, bgColor?: number) {
    this.paddingX = paddingX;
    this.paddingY = paddingY;
    this.bgColor = bgColor;
  }

  addChild(component: Component): void {
    this.children.push(component);
    this.cache = undefined;
  }

  removeChild(component: Component): void {
    const idx = this.children.indexOf(component);
    if (idx !== -1) {
      this.children.splice(idx, 1);
      this.cache = undefined;
    }
  }

  clear(): void {
    this.children = [];
    this.cache = undefined;
  }

  setBgColor(bgColor?: number): void {
    this.bgColor = bgColor;
  }

  invalidate(): void {
    this.cache = undefined;
    for (const child of this.children) {
      child.invalidate?.();
    }
  }

  render(width: number): string[] {
    if (this.children.length === 0) return [];

    const contentWidth = Math.max(1, width - this.paddingX * 2);
    const leftPad = " ".repeat(this.paddingX);

    const childLines: string[] = [];
    for (const child of this.children) {
      for (const line of child.render(contentWidth)) {
        childLines.push(leftPad + line);
      }
    }

    if (childLines.length === 0) return [];

    // Check cache
    if (
      this.cache &&
      this.cache.width === width &&
      this.cache.bgColor === this.bgColor &&
      this.cache.childLines.length === childLines.length &&
      this.cache.childLines.every((l, i) => l === childLines[i])
    ) {
      return this.cache.lines;
    }

    const result: string[] = [];

    // Top padding
    for (let i = 0; i < this.paddingY; i++) {
      result.push(this.applyBg("", width));
    }

    // Content
    for (const line of childLines) {
      result.push(this.applyBg(line, width));
    }

    // Bottom padding
    for (let i = 0; i < this.paddingY; i++) {
      result.push(this.applyBg("", width));
    }

    this.cache = { childLines, width, bgColor: this.bgColor, lines: result };
    return result;
  }

  private applyBg(line: string, width: number): string {
    const pad = Math.max(0, width - visibleWidth(line));
    const padded = line + " ".repeat(pad);
    if (this.bgColor !== undefined) return applyBackgroundToLine(padded, width, this.bgColor);
    return padded;
  }
}
