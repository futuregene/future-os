/**
 * SelectList - a list selector with filtering and keyboard navigation.
 * Modeled after pi's SelectList: Component + handleInput, filtering, scroll indicators.
 */
import { CSI, RESET, BOLD } from "../tui.js";
import { truncateToWidth } from "../utils.js";
const DEFAULT_THEME = {
    accent: 39,
    fg: 252,
    dimFg: 245,
    selectedFg: 255,
    selectedBg: 38,
    bg: 235,
};
export class SelectList {
    items;
    filteredItems;
    selectedIndex = 0;
    filter = "";
    maxVisible;
    theme;
    title;
    onSelect;
    onCancel;
    onSelectionChange;
    onKey;
    scrollOffset = 0;
    constructor(options) {
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
    getSelectedItem() {
        if (this.filteredItems.length === 0)
            return null;
        return this.filteredItems[this.selectedIndex] ?? null;
    }
    setSelectedIndex(index) {
        this.selectedIndex = Math.max(0, Math.min(index, this.filteredItems.length - 1));
        this.recalcScroll();
    }
    setFilter(filter) {
        this.filter = filter;
        this.selectedIndex = 0;
        this.applyFilter();
    }
    handleInput(data) {
        this.handleKey(data);
    }
    invalidate() { }
    handleKey(key) {
        if (this.onKey?.(key))
            return true;
        switch (key) {
            case "up":
                // Wrap to bottom when at top (matches pi)
                if (this.selectedIndex > 0) {
                    this.selectedIndex--;
                }
                else {
                    this.selectedIndex = this.filteredItems.length - 1;
                }
                this.recalcScroll();
                this.notifySelectionChange();
                return true;
            case "down":
                // Wrap to top when at bottom (matches pi)
                if (this.selectedIndex < this.filteredItems.length - 1) {
                    this.selectedIndex++;
                }
                else {
                    this.selectedIndex = 0;
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
    applyFilter() {
        if (!this.filter) {
            this.filteredItems = this.items;
        }
        else {
            const q = this.filter.toLowerCase();
            this.filteredItems = this.items.filter((item) => item.value.toLowerCase().startsWith(q));
        }
        if (this.selectedIndex >= this.filteredItems.length) {
            this.selectedIndex = Math.max(0, this.filteredItems.length - 1);
        }
        this.scrollOffset = 0;
        this.notifySelectionChange();
    }
    recalcScroll() {
        if (this.selectedIndex < this.scrollOffset) {
            this.scrollOffset = this.selectedIndex;
        }
        else if (this.selectedIndex >= this.scrollOffset + this.maxVisible) {
            this.scrollOffset = this.selectedIndex - this.maxVisible + 1;
        }
    }
    notifySelectionChange() {
        const item = this.getSelectedItem();
        if (item)
            this.onSelectionChange?.(item);
    }
    render(width) {
        const lines = [];
        const innerW = Math.max(20, width);
        // Width budget: label gets most of the space, description gets the rest
        const maxLabelW = Math.max(10, innerW - 35);
        const maxDescW = Math.max(5, innerW - maxLabelW - 4);
        lines.push(`${CSI}38;5;${this.theme.accent}m${BOLD} ${this.title} ${RESET}`);
        lines.push(`${CSI}2mFilter: ${this.filter}_ ${RESET}`);
        const total = this.filteredItems.length;
        const maxItems = Math.min(total, this.maxVisible);
        // Scroll indicator above
        if (this.scrollOffset > 0) {
            lines.push(`${CSI}38;5;${this.theme.dimFg}m↑ ${this.scrollOffset} more${RESET}`);
        }
        for (let i = 0; i < maxItems; i++) {
            const idx = this.scrollOffset + i;
            const item = this.filteredItems[idx];
            if (!item)
                continue;
            const selected = idx === this.selectedIndex;
            const labelPart = truncateToWidth(item.label, maxLabelW);
            // Normalize multiline descriptions: replace \r\n with space
            const rawDesc = item.description?.replace(/\r\n/g, " ") ?? "";
            const descPart = truncateToWidth(rawDesc, maxDescW);
            if (selected) {
                const prefix = `${CSI}38;5;${this.theme.selectedFg}m${CSI}48;5;${this.theme.selectedBg}m ▶ `;
                const label = `${labelPart}${RESET}`;
                const suffix = descPart
                    ? `${CSI}38;5;${this.theme.selectedFg}m${CSI}48;5;${this.theme.selectedBg}m ${CSI}2m${descPart}${RESET}`
                    : "";
                lines.push(prefix + label + suffix);
            }
            else {
                const label = `${CSI}38;5;${this.theme.fg}m  ${labelPart}${RESET}`;
                const suffix = descPart
                    ? ` ${CSI}38;5;${this.theme.dimFg}m${CSI}2m${descPart}${RESET}`
                    : "";
                lines.push(label + suffix);
            }
        }
        // Scroll indicator below
        if (this.scrollOffset + maxItems < total) {
            const remaining = total - this.scrollOffset - maxItems;
            lines.push(`${CSI}38;5;${this.theme.dimFg}m↓ ${remaining} more${RESET}`);
        }
        if (total === 0) {
            lines.push(`${CSI}2mNo matching items${RESET}`);
        }
        return lines;
    }
}
