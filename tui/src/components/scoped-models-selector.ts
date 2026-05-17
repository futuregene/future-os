/**
 * ScopedModelsSelector - multi-select model toggle list.
 * Matches pi's ScopedModelsSelectorComponent: shows all models with ✓/✗,
 * Space toggles, Enter saves, Escape cancels.
 */

import type { Component } from "../tui.js";
import { CSI, RESET, BOLD } from "../tui.js";
import { visibleWidth, truncateToWidth } from "../utils.js";
import type { ModelInfo } from "../rpc/types.js";

export interface ScopedModelsSelectorOptions {
  allModels: ModelInfo[];
  enabledModelIds: Set<string>;
  onSave: (enabledIds: string[]) => void;
  onCancel: () => void;
  maxVisible?: number;
}

const THEME = {
  accent: 39,
  fg: 252,
  dimFg: 245,
  selectedFg: 255,
  selectedBg: 38,
  bg: 235,
  success: 40,
  error: 196,
};

export class ScopedModelsSelector implements Component {
  focused = false;
  private models: ModelInfo[];
  private enabledSet: Set<string>;
  private filteredItems: ModelInfo[] = [];
  private selectedIndex = 0;
  private filter = "";
  private maxVisible: number;
  private onSave: (enabledIds: string[]) => void;
  private onCancel: () => void;
  private scrollOffset = 0;
  private originalEnabled: Set<string>; // for discard on cancel

  constructor(options: ScopedModelsSelectorOptions) {
    this.models = options.allModels;
    this.enabledSet = new Set(options.enabledModelIds);
    this.originalEnabled = new Set(options.enabledModelIds);
    this.maxVisible = options.maxVisible ?? 12;
    this.onSave = options.onSave;
    this.onCancel = options.onCancel;
    this.applyFilter();
  }

  invalidate(): void { /* no cache */ }

  handleInput(data: string): void {
    this.handleKey(data);
  }

  handleKey(key: string): boolean {
    if (key === "up") {
      if (this.selectedIndex > 0) {
        this.selectedIndex--;
      } else {
        this.selectedIndex = this.filteredItems.length - 1;
      }
      this.recalcScroll();
      return true;
    }
    if (key === "down") {
      if (this.selectedIndex < this.filteredItems.length - 1) {
        this.selectedIndex++;
      } else {
        this.selectedIndex = 0;
      }
      this.recalcScroll();
      return true;
    }
    if (key === "space") {
      const item = this.filteredItems[this.selectedIndex];
      if (item) {
        if (this.enabledSet.has(item.id)) {
          this.enabledSet.delete(item.id);
        } else {
          this.enabledSet.add(item.id);
        }
      }
      return true;
    }
    if (key === "enter") {
      this.onSave([...this.enabledSet]);
      return true;
    }
    if (key === "escape") {
      this.onCancel();
      return true;
    }
    if (key === "backspace") {
      this.filter = this.filter.slice(0, -1);
      this.applyFilter();
      return true;
    }
    if (key.length === 1 && key.charCodeAt(0) >= 32) {
      this.filter += key;
      this.applyFilter();
      return true;
    }
    return false;
  }

  private applyFilter(): void {
    if (!this.filter) {
      this.filteredItems = this.models;
    } else {
      const q = this.filter.toLowerCase();
      this.filteredItems = this.models.filter(
        (m) => m.id.toLowerCase().includes(q) || m.provider.toLowerCase().includes(q),
      );
    }
    if (this.selectedIndex >= this.filteredItems.length) {
      this.selectedIndex = Math.max(0, this.filteredItems.length - 1);
    }
    this.scrollOffset = 0;
  }

  private recalcScroll(): void {
    if (this.selectedIndex < this.scrollOffset) {
      this.scrollOffset = this.selectedIndex;
    } else if (this.selectedIndex >= this.scrollOffset + this.maxVisible) {
      this.scrollOffset = this.selectedIndex - this.maxVisible + 1;
    }
  }

  render(width: number): string[] {
    const lines: string[] = [];
    const innerW = Math.max(20, width);
    const maxLabelW = Math.max(10, innerW - 35);
    const maxDescW = Math.max(5, innerW - maxLabelW - 8);

    lines.push(`${CSI}38;5;${THEME.accent}m${BOLD} Model Scope ${RESET}`);
    lines.push(`${CSI}38;5;${THEME.dimFg}m${CSI}2m Session-only. Enter to save to settings. ${RESET}`);
    lines.push(`${CSI}2mFilter: ${this.filter}_ ${RESET}`);

    const total = this.filteredItems.length;
    const maxItems = Math.min(total, this.maxVisible);

    if (this.scrollOffset > 0) {
      lines.push(`${CSI}38;5;${THEME.dimFg}m↑ ${this.scrollOffset} more${RESET}`);
    }

    for (let i = 0; i < maxItems; i++) {
      const idx = this.scrollOffset + i;
      const item = this.filteredItems[idx];
      if (!item) continue;

      const selected = idx === this.selectedIndex;
      const isEnabled = this.enabledSet.has(item.id);
      const status = isEnabled
        ? `${CSI}38;5;${THEME.success}m ✓${RESET}`
        : `${CSI}38;5;${THEME.dimFg}m ✗${RESET}`;
      const labelPart = truncateToWidth(item.id, maxLabelW);
      const descPart = truncateToWidth(`[${item.provider}]`, maxDescW);

      if (selected) {
        const prefix = `${CSI}38;5;${THEME.selectedFg}m${CSI}48;5;${THEME.selectedBg}m ▶ ${status} `;
        const label = `${labelPart}${RESET}`;
        const suffix = descPart
          ? `${CSI}38;5;${THEME.selectedFg}m${CSI}48;5;${THEME.selectedBg}m ${CSI}2m${descPart}${RESET}`
          : "";
        lines.push(prefix + label + suffix);
      } else {
        const label = `${CSI}38;5;${THEME.fg}m  ${status} ${labelPart}${RESET}`;
        const suffix = descPart
          ? ` ${CSI}38;5;${THEME.dimFg}m${CSI}2m${descPart}${RESET}`
          : "";
        lines.push(label + suffix);
      }
    }

    if (this.scrollOffset + maxItems < total) {
      const remaining = total - this.scrollOffset - maxItems;
      lines.push(`${CSI}38;5;${THEME.dimFg}m↓ ${remaining} more${RESET}`);
    }

    if (total === 0) {
      lines.push(`${CSI}2mNo matching models${RESET}`);
    }

    const enabledCount = this.enabledSet.size;
    const dirty = !this.setsEqual(this.enabledSet, this.originalEnabled);
    const footer = `  Space toggle · Enter save · Esc cancel · ${enabledCount}/${this.models.length} enabled`;
    lines.push(dirty
      ? `${CSI}38;5;${THEME.dimFg}m${footer}${RESET} ${CSI}38;5;11m(unsaved)${RESET}`
      : `${CSI}38;5;${THEME.dimFg}m${footer}${RESET}`);

    return lines;
  }

  private setsEqual(a: Set<string>, b: Set<string>): boolean {
    if (a.size !== b.size) return false;
    for (const v of a) {
      if (!b.has(v)) return false;
    }
    return true;
  }
}
