/**
 * SettingsList component — scrollable settings list with value cycling,
 * optional search filtering, and submenu support.
 */

import type { Component } from "../tui.js";
import { truncateToWidth, visibleWidth, wrapTextWithAnsi } from "../utils.js";

export interface SettingItem {
  id: string;
  label: string;
  description?: string;
  currentValue: string;
  /** If provided, Enter/Space cycles through these values. */
  values?: string[];
  /** If provided, Enter opens this submenu. */
  submenu?: (currentValue: string, done: (selectedValue?: string) => void) => Component;
}

export interface SettingsListTheme {
  label: (text: string, selected: boolean) => string;
  value: (text: string, selected: boolean) => string;
  description: (text: string) => string;
  cursor: string;
  hint: (text: string) => string;
}

export interface SettingsListOptions {
  enableSearch?: boolean;
}

export class SettingsList implements Component {
  private items: SettingItem[];
  private filteredItems: SettingItem[];
  private theme: SettingsListTheme;
  private selectedIndex = 0;
  private maxVisible: number;
  private onChange: (id: string, newValue: string) => void;
  private onCancel: () => void;
  private searchQuery = "";
  private searchEnabled: boolean;

  // Submenu state
  private submenuComponent: Component | null = null;
  private submenuItemIndex: number | null = null;

  constructor(
    items: SettingItem[],
    maxVisible: number,
    theme: SettingsListTheme,
    onChange: (id: string, newValue: string) => void,
    onCancel: () => void,
    options: SettingsListOptions = {},
  ) {
    this.items = items;
    this.filteredItems = items;
    this.maxVisible = maxVisible;
    this.theme = theme;
    this.onChange = onChange;
    this.onCancel = onCancel;
    this.searchEnabled = options.enableSearch ?? false;
  }

  updateValue(id: string, newValue: string): void {
    const item = this.items.find((i) => i.id === id);
    if (item) item.currentValue = newValue;
  }

  invalidate(): void {
    this.submenuComponent?.invalidate?.();
  }

  render(width: number): string[] {
    if (this.submenuComponent) return this.submenuComponent.render(width);
    return this.renderMainList(width);
  }

  private renderMainList(width: number): string[] {
    const lines: string[] = [];

    // Search bar
    if (this.searchEnabled) {
      lines.push(this.theme.hint(`  search: ${this.searchQuery}_`));
      lines.push("");
    }

    if (this.items.length === 0) {
      lines.push(this.theme.hint("  No settings available"));
      this.addHintLine(lines, width);
      return lines;
    }

    const displayItems = this.searchEnabled ? this.filteredItems : this.items;
    if (displayItems.length === 0) {
      lines.push(truncateToWidth(this.theme.hint("  No matching settings"), width));
      this.addHintLine(lines, width);
      return lines;
    }

    // Scroll window
    const start = Math.max(0, Math.min(
      this.selectedIndex - Math.floor(this.maxVisible / 2),
      displayItems.length - this.maxVisible,
    ));
    const end = Math.min(start + this.maxVisible, displayItems.length);

    // Max label width for alignment
    const maxLabelW = Math.min(30, Math.max(...this.items.map((i) => visibleWidth(i.label))));

    for (let i = start; i < end; i++) {
      const item = displayItems[i];
      if (!item) continue;

      const isSelected = i === this.selectedIndex;
      const prefix = isSelected ? this.theme.cursor : "  ";
      const prefixW = visibleWidth(prefix);

      const labelPadded = item.label + " ".repeat(Math.max(0, maxLabelW - visibleWidth(item.label)));
      const labelText = this.theme.label(labelPadded, isSelected);

      const separator = "  ";
      const usedW = prefixW + maxLabelW + visibleWidth(separator);
      const valueMaxW = width - usedW - 2;
      const valueText = this.theme.value(
        truncateToWidth(item.currentValue, Math.max(1, valueMaxW)),
        isSelected,
      );

      lines.push(truncateToWidth(prefix + labelText + separator + valueText, width));
    }

    // Scroll indicator
    if (start > 0 || end < displayItems.length) {
      lines.push(this.theme.hint(
        truncateToWidth(`  (${this.selectedIndex + 1}/${displayItems.length})`, width - 2),
      ));
    }

    // Description
    const selected = displayItems[this.selectedIndex];
    if (selected?.description) {
      lines.push("");
      for (const line of wrapTextWithAnsi(selected.description, width - 4)) {
        lines.push(this.theme.description(`  ${line}`));
      }
    }

    this.addHintLine(lines, width);
    return lines;
  }

  handleInput(data: string): void {
    // Submenu active — delegate input
    if (this.submenuComponent) {
      this.submenuComponent.handleInput?.(data);
      return;
    }

    const displayItems = this.searchEnabled ? this.filteredItems : this.items;

    if (data === "up") {
      if (displayItems.length === 0) return;
      this.selectedIndex = this.selectedIndex === 0
        ? displayItems.length - 1
        : this.selectedIndex - 1;
    } else if (data === "down") {
      if (displayItems.length === 0) return;
      this.selectedIndex = this.selectedIndex === displayItems.length - 1
        ? 0
        : this.selectedIndex + 1;
    } else if (data === "enter" || data === " ") {
      this.activateItem();
    } else if (data === "escape") {
      this.onCancel();
    } else if (this.searchEnabled && data.length === 1 && data.charCodeAt(0) >= 32) {
      // Inline search: append char, filter, reset selection
      this.searchQuery += data;
      this.applyFilter(this.searchQuery);
    } else if (this.searchEnabled && data === "backspace" && this.searchQuery.length > 0) {
      this.searchQuery = this.searchQuery.slice(0, -1);
      this.applyFilter(this.searchQuery);
    }
  }

  private activateItem(): void {
    const displayItems = this.searchEnabled ? this.filteredItems : this.items;
    const item = displayItems[this.selectedIndex];
    if (!item) return;

    if (item.submenu) {
      this.submenuItemIndex = this.selectedIndex;
      this.submenuComponent = item.submenu(item.currentValue, (selectedValue?: string) => {
        if (selectedValue !== undefined) {
          item.currentValue = selectedValue;
          this.onChange(item.id, selectedValue);
        }
        this.closeSubmenu();
      });
    } else if (item.values && item.values.length > 0) {
      const currentIdx = item.values.indexOf(item.currentValue);
      const nextIdx = (currentIdx + 1) % item.values.length;
      const newValue = item.values[nextIdx];
      item.currentValue = newValue;
      this.onChange(item.id, newValue);
    }
  }

  private closeSubmenu(): void {
    this.submenuComponent = null;
    if (this.submenuItemIndex !== null) {
      this.selectedIndex = this.submenuItemIndex;
      this.submenuItemIndex = null;
    }
  }

  private applyFilter(query: string): void {
    const q = query.toLowerCase();
    this.filteredItems = this.items.filter((item) =>
      item.label.toLowerCase().includes(q) ||
      item.currentValue.toLowerCase().includes(q),
    );
    this.selectedIndex = 0;
  }

  private addHintLine(lines: string[], width: number): void {
    lines.push("");
    lines.push(truncateToWidth(
      this.theme.hint(
        this.searchEnabled
          ? "  Type to search · Enter/Space to change · Esc to cancel"
          : "  Enter/Space to change · Esc to cancel",
      ),
      width,
    ));
  }
}
