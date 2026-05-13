import { fg, bold, DARK_THEME } from "../theme.js";
import type { Component } from "../tui.js";
import { visibleWidth } from "../utils.js";

export interface AutocompleteItem {
  value: string;
  label: string;
  description?: string;
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

    // Top border: ┌────┐
    lines.push(fg(244, "┌") + fg(239, "─".repeat(popupWidth)) + fg(244, "┐"));

    // Items (only visible slice)
    const start = Math.max(0, this.selectedIndex - this.maxVisible + 1);
    const end = Math.min(this.items.length, start + this.maxVisible);

    for (let i = start; i < end; i++) {
      const item = this.items[i];
      const isSelected = i === this.selectedIndex;
      const label = item.label.slice(0, popupWidth - 4);

      if (isSelected) {
        // Selected: accent color, bold arrow
        const content = fg(151, bold("▶")) + " " + fg(252, label);
        const pad = popupWidth - 2 - visibleWidth(content);
        lines.push(fg(244, "│") + " " + content + " ".repeat(Math.max(0, pad)) + fg(244, "│"));
      } else {
        // Not selected: dim color
        const content = "  " + label;
        const pad = popupWidth - 2 - visibleWidth(content);
        lines.push(fg(244, "│") + fg(245, content) + " ".repeat(Math.max(0, pad)) + fg(244, "│"));
      }
    }

    // Bottom border
    lines.push(fg(244, "└") + fg(239, "─".repeat(popupWidth)) + fg(244, "┘"));

    return lines;
  }

  height(): number {
    if (!this.visible || this.items.length === 0) return 0;
    return 2 + Math.min(this.items.length, this.maxVisible);
  }
}
