import { fg, bg, bold } from "../theme.js";

export interface AutocompleteItem {
  value: string;
  label: string;
  description?: string;
}

export class AutocompletePopup {
  private items: AutocompleteItem[] = [];
  private selectedIndex = 0;
  private prefix = "";
  private visible = false;
  private maxVisible = 10;

  show(items: AutocompleteItem[], prefix = ""): void {
    this.items = items;
    this.selectedIndex = 0;
    this.prefix = prefix;
    this.visible = true;
  }

  hide(): void {
    this.visible = false;
    this.items = [];
    this.selectedIndex = 0;
    this.prefix = "";
  }

  isVisible(): boolean {
    return this.visible;
  }

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

  // Render the autocomplete popup, returning array of lines
  render(width: number): string[] {
    if (!this.visible || this.items.length === 0) return [];

    const lines: string[] = [];

    // Header line
    const title = "Completions";
    const titleStr = bold(" " + title + " ");
    const spaces = " ".repeat(Math.max(0, width - title.length - 3));
    lines.push(bg(235, fg(151, titleStr) + fg(244, spaces)));

    // Items
    const startIndex = Math.max(0, this.selectedIndex - this.maxVisible + 1);
    const endIndex = Math.min(this.items.length, startIndex + this.maxVisible);

    for (let i = startIndex; i < endIndex; i++) {
      const item = this.items[i];
      const isSelected = i === this.selectedIndex;

      const label = item.label.length > width - 3
        ? item.label.slice(0, width - 6) + "..."
        : item.label;

      const desc = item.description ? `  ${item.description}` : "";
      const descLen = Math.min(desc.length, width - label.length - 4);

      if (isSelected) {
        const line = fg(0, bg(151, `▶ ${label}`)) + fg(245, desc.slice(0, descLen));
        lines.push(line);
      } else {
        const line = fg(245, `  ${label}`) + fg(240, desc.slice(0, descLen));
        lines.push(line);
      }
    }

    // Footer with hint
    if (this.items.length > this.maxVisible) {
      const footer = fg(240, `  (${this.selectedIndex + 1}/${this.items.length})`);
      lines.push(footer);
    }

    return lines;
  }

  // Get height of popup
  height(): number {
    if (!this.visible || this.items.length === 0) return 0;
    const visibleCount = Math.min(this.items.length, this.maxVisible);
    return 2 + visibleCount + (this.items.length > this.maxVisible ? 1 : 0);
  }
}
