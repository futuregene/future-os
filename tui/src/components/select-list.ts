/**
 * SelectList - a list selector with filtering and keyboard navigation.
 * Modeled after pi's SelectList: Component + handleInput, filtering, scroll indicators.
 */

import type { Component } from "../tui.js";
import { CSI, RESET, BOLD } from "../tui.js";
import { visibleWidth, truncateToWidth } from "../utils.js";

export interface SelectItem {
  value: string;
  label: string;
  description?: string;
}

export interface SelectListOptions {
  title: string;
  items: SelectItem[];
  maxVisible?: number;
  theme?: {
    accent: number;
    fg: number;
    dimFg: number;
    selectedFg: number;
    selectedBg: number;
    bg: number;
  };
  onSelect?: (item: SelectItem) => void;
  onCancel?: () => void;
  onSelectionChange?: (item: SelectItem) => void;
  onKey?: (key: string) => boolean;
}

const DEFAULT_THEME = {
  accent: 39,
  fg: 252,
  dimFg: 245,
  selectedFg: 255,
  selectedBg: 38,
  bg: 235,
};

export class SelectList implements Component {
  private items: SelectItem[];
  private filteredItems: SelectItem[];
  private selectedIndex = 0;
  private filter = "";
  private maxVisible: number;
  private theme: typeof DEFAULT_THEME;
  private title: string;
  private onSelect?: (item: SelectItem) => void;
  private onCancel?: () => void;
  private onSelectionChange?: (item: SelectItem) => void;
  private onKey?: (key: string) => boolean;
  private scrollOffset = 0;

  constructor(options: SelectListOptions) {
    this.items = options.items;
    this.filteredItems = options.items;
    this.maxVisible = options.maxVisible ?? 10;
    this.theme = { ...DEFAULT_THEME, ...options.theme };
    this.title = options.title;
    this.onSelect = options.onSelect;
    this.onCancel = options.onCancel;
    this.onSelectionChange = options.onSelectionChange;
    this.onKey = options.onKey;
  }

  getSelectedItem(): SelectItem | null {
    if (this.filteredItems.length === 0) return null;
    return this.filteredItems[this.selectedIndex] ?? null;
  }

  setSelectedIndex(index: number): void {
    this.selectedIndex = Math.max(0, Math.min(index, this.filteredItems.length - 1));
    this.recalcScroll();
  }

  setFilter(filter: string): void {
    this.filter = filter;
    this.selectedIndex = 0;
    this.applyFilter();
  }

  handleInput(data: string): void {
    this.handleKey(data);
  }

  invalidate(): void { /* no cache */ }

  handleKey(key: string): boolean {
    if (this.onKey?.(key)) return true;

    switch (key) {
      case "up":
        if (this.selectedIndex > 0) {
          this.selectedIndex--;
        } else {
          // Wrap to bottom
          this.selectedIndex = this.filteredItems.length - 1;
          this.scrollOffset = Math.max(0, this.selectedIndex - this.maxVisible + 1);
        }
        this.recalcScroll();
        this.notifySelectionChange();
        return true;
      case "down":
        if (this.selectedIndex < this.filteredItems.length - 1) {
          this.selectedIndex++;
        } else {
          // Wrap to top
          this.selectedIndex = 0;
          this.scrollOffset = 0;
        }
        this.recalcScroll();
        this.notifySelectionChange();
        return true;
      case "enter":
        if (this.filteredItems.length > 0) {
          this.onSelect?.(this.filteredItems[this.selectedIndex]);
        }
        return true;
      case "escape":
        this.onCancel?.();
        return true;
      case "backspace":
        this.filter = this.filter.slice(0, -1);
        this.applyFilter();
        return true;
      default:
        if (key.length === 1 && key.charCodeAt(0) >= 32) {
          this.filter += key;
          this.applyFilter();
          return true;
        }
        return false;
    }
  }

  private applyFilter(): void {
    if (!this.filter) {
      this.filteredItems = this.items;
    } else {
      const q = this.filter.toLowerCase();
      this.filteredItems = this.items.filter(
        (item) => item.value.toLowerCase().includes(q)
      );
    }
    if (this.selectedIndex >= this.filteredItems.length) {
      this.selectedIndex = Math.max(0, this.filteredItems.length - 1);
    }
    this.scrollOffset = 0;
    this.notifySelectionChange();
  }

  private recalcScroll(): void {
    if (this.selectedIndex < this.scrollOffset) {
      this.scrollOffset = this.selectedIndex;
    } else if (this.selectedIndex >= this.scrollOffset + this.maxVisible) {
      this.scrollOffset = this.selectedIndex - this.maxVisible + 1;
    }
  }

  private notifySelectionChange(): void {
    const item = this.getSelectedItem();
    if (item) this.onSelectionChange?.(item);
  }

  render(width: number): string[] {
    const lines: string[] = [];
    const innerW = Math.max(20, width);

    // Width budget: label left, description right, both aligned
    const maxLabelW = Math.max(10, Math.floor(innerW * 0.55));
    const maxDescW = Math.max(5, innerW - maxLabelW - 4);

    // Helper: pad line to fill innerW, ensuring each line clears stale content
    const padToWidth = (line: string): string => {
      const visW = visibleWidth(line);
      if (visW < innerW) {
        return line + " ".repeat(innerW - visW) + RESET;
      }
      // Line already at or over width — truncate to avoid compositing overflow
      return truncateToWidth(line, innerW - 1) + RESET;
    };

    lines.push(padToWidth(`${CSI}38;5;${this.theme.accent}m${BOLD} ${this.title}`));
    lines.push(padToWidth(`${CSI}2mFilter: ${this.filter}_`));

    const total = this.filteredItems.length;
    const maxItems = Math.min(total, this.maxVisible);

    // Scroll indicator above (always reserve space for consistent line count)
    if (this.scrollOffset > 0) {
      lines.push(padToWidth(`${CSI}38;5;${this.theme.dimFg}m↑ ${this.scrollOffset} more`));
    } else {
      lines.push(padToWidth(""));
    }

    for (let i = 0; i < this.maxVisible; i++) {
      const idx = this.scrollOffset + i;
      const item = this.filteredItems[idx];
      if (!item) {
        lines.push(padToWidth(""));
        continue;
      }

      const selected = idx === this.selectedIndex;
      const labelPart = truncateToWidth(item.label, maxLabelW);
      // Pad label to fixed width so description column is aligned
      const labelVisW = visibleWidth(labelPart);
      const labelPad = " ".repeat(Math.max(0, maxLabelW - labelVisW));
      // Normalize multiline descriptions: replace \r\n with space
      const rawDesc = item.description?.replace(/\r\n/g, " ") ?? "";
      const descPart = truncateToWidth(rawDesc, maxDescW);

      if (selected) {
        // Single continuous background: no RESET gap between label and suffix
        const bgSeq = `${CSI}48;5;${this.theme.selectedBg}m`;
        const fgSeq = `${CSI}38;5;${this.theme.selectedFg}m`;
        const head = `${fgSeq}${bgSeq} ▶ `;
        const label = labelPart + labelPad;
        const suffix = descPart
          ? ` ${CSI}2m${descPart}`
          : "";
        lines.push(padToWidth(head + label + suffix));
      } else {
        const label = `${CSI}38;5;${this.theme.fg}m  ${labelPart}${labelPad}${RESET}`;
        const suffix = descPart
          ? ` ${CSI}38;5;${this.theme.dimFg}m${CSI}2m${descPart}${RESET}`
          : "";
        lines.push(padToWidth(label + suffix));
      }
    }

    // Scroll indicator below (always reserve space for consistent line count)
    if (this.scrollOffset + maxItems < total) {
      const remaining = total - this.scrollOffset - maxItems;
      lines.push(padToWidth(`${CSI}38;5;${this.theme.dimFg}m↓ ${remaining} more`));
    } else {
      lines.push(padToWidth(""));
    }

    if (total === 0) {
      // Replace one empty slot with the message (line count stays constant)
      lines[2] = padToWidth(`${CSI}2mNo matching items`);
    }

    return lines;
  }
}
