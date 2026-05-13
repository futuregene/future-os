/**
 * SelectList - a list selector with filtering and keyboard navigation.
 * Modeled after @earendil-works/pi-tui SelectList.
 */

import type { Theme } from "../tui.js";
import { DEFAULT_THEME, CSI, RESET, BOLD } from "../tui.js";

export interface SelectItem {
  value: string;
  label: string;
  description?: string;
}

export interface SelectListOptions {
  title: string;
  items: SelectItem[];
  theme?: Theme;
  maxVisible?: number;
  onSelect?: (item: SelectItem) => void;
  onCancel?: () => void;
  onKey?: (key: string) => boolean;
}

const DESCRIPTION_WIDTH = 40;
const PRIMARY_WIDTH = 50;

export class SelectList {
  private items: SelectItem[];
  private filteredItems: SelectItem[];
  private selectedIndex = 0;
  private filter = "";
  private maxVisible: number;
  private theme: Theme;
  private title: string;
  private onSelect?: (item: SelectItem) => void;
  private onCancel?: () => void;
  private onKey?: (key: string) => boolean;

  constructor(options: SelectListOptions) {
    this.items = options.items;
    this.filteredItems = options.items;
    this.maxVisible = options.maxVisible ?? 10;
    this.theme = options.theme ?? DEFAULT_THEME;
    this.title = options.title;
    this.onSelect = options.onSelect;
    this.onCancel = options.onCancel;
    this.onKey = options.onKey;
  }

  getSelectedItem(): SelectItem | null {
    if (this.filteredItems.length === 0) return null;
    return this.filteredItems[this.selectedIndex] ?? null;
  }

  handleKey(key: string): boolean {
    if (this.onKey?.(key)) return true;

    switch (key) {
      case "up":
        if (this.selectedIndex > 0) this.selectedIndex--;
        return true;
      case "down":
        if (this.selectedIndex < this.filteredItems.length - 1) this.selectedIndex++;
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
        (item) =>
          item.label.toLowerCase().includes(q) ||
          (item.description?.toLowerCase().includes(q) ?? false)
      );
    }
    if (this.selectedIndex >= this.filteredItems.length) {
      this.selectedIndex = Math.max(0, this.filteredItems.length - 1);
    }
  }

  getHeight(): number {
    return Math.min(this.filteredItems.length, this.maxVisible) + 4;
  }

  render(width: number): string[] {
    const lines: string[] = [];

    lines.push(`${CSI}38;5;${this.theme.accent}m${BOLD} ${this.title} ${RESET}`);
    lines.push(`${CSI}2mFilter: ${this.filter}_ ${RESET}`);

    const maxItems = Math.min(this.filteredItems.length, this.maxVisible);
    for (let i = 0; i < maxItems; i++) {
      const item = this.filteredItems[i];
      const selected = i === this.selectedIndex;
      const primaryText = item.label.slice(0, PRIMARY_WIDTH - 4);
      const descText = item.description?.slice(0, DESCRIPTION_WIDTH) ?? "";

      if (selected) {
        lines.push(
          `${CSI}38;5;${this.theme.selectedFg}m${CSI}48;5;${this.theme.selectedBg}m ▶ ${primaryText}${CSI}0m`
        );
        if (descText) {
          lines.push(
            `${CSI}38;5;${this.theme.selectedFg}m${CSI}48;5;${this.theme.selectedBg}m   ${CSI}2m${descText}${CSI}0m`
          );
        }
      } else {
        lines.push(`  ${primaryText} ${CSI}2m${descText}${RESET}`);
      }
    }

    if (this.filteredItems.length === 0) {
      lines.push(`${CSI}2mNo matching items${RESET}`);
    }

    return lines;
  }
}
