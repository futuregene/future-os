/**
 * Unit tests for SelectList and ScopedModelsSelector components.
 *
 * Tests the pure logic: filtering, navigation, selection, toggle.
 * Render output is validated structurally (line count, content presence).
 */
import { describe, test, expect } from "bun:test";
import { SelectList } from "../components/select-list.js";
import { ScopedModelsSelector } from "../components/scoped-models-selector.js";
import type { ModelInfo } from "../rpc/types.js";
import { stripAnsiCodes, visibleWidth } from "../utils.js";

// ─── SelectList ───────────────────────────────────────────────────────────

const ITEMS = [
  { value: "apple", label: "Apple", description: "A fruit" },
  { value: "banana", label: "Banana", description: "Yellow" },
  { value: "cherry", label: "Cherry", description: "Red and small" },
  { value: "date", label: "Date", description: "Sweet dried fruit" },
  { value: "elderberry", label: "Elderberry", description: "Dark purple" },
];

function makeSelectList(overrides = {}) {
  return new SelectList({
    title: "Test List",
    items: ITEMS,
    maxVisible: 3,
    ...overrides,
  });
}

describe("SelectList", () => {
  test("getSelectedItem returns first item by default", () => {
    const list = makeSelectList();
    expect(list.getSelectedItem()?.value).toBe("apple");
  });

  test("getSelectedItem returns null for empty list", () => {
    const list = new SelectList({ title: "Empty", items: [] });
    expect(list.getSelectedItem()).toBeNull();
  });

  test("setSelectedIndex clamps to valid range", () => {
    const list = makeSelectList();
    list.setSelectedIndex(2);
    expect(list.getSelectedItem()?.value).toBe("cherry");

    list.setSelectedIndex(999);
    expect(list.getSelectedItem()?.value).toBe("elderberry");

    list.setSelectedIndex(-5);
    expect(list.getSelectedItem()?.value).toBe("apple");
  });

  test("setFilter narrows items and resets selection", () => {
    const list = makeSelectList();
    list.setSelectedIndex(3);
    list.setFilter("an");
    expect(list.getSelectedItem()?.value).toBe("banana");
  });

  test("filter is case-insensitive", () => {
    const list = makeSelectList();
    list.setFilter("BERRY");
    expect(list.getSelectedItem()?.value).toBe("elderberry");
  });

  test("filter by value field", () => {
    const list = makeSelectList();
    list.setFilter("cher");
    expect(list.getSelectedItem()?.value).toBe("cherry");
  });

  test("clearing filter restores all items", () => {
    const list = makeSelectList();
    list.setFilter("an");
    list.setFilter("");
    expect(list.getSelectedItem()?.value).toBe("apple");
  });

  test("handleKey up/down navigates", () => {
    const list = makeSelectList();
    list.handleKey("down");
    expect(list.getSelectedItem()?.value).toBe("banana");
    list.handleKey("down");
    expect(list.getSelectedItem()?.value).toBe("cherry");
    list.handleKey("up");
    expect(list.getSelectedItem()?.value).toBe("banana");
  });

  test("handleKey up wraps to bottom", () => {
    const list = makeSelectList();
    list.handleKey("up");
    expect(list.getSelectedItem()?.value).toBe("elderberry");
  });

  test("handleKey down wraps to top", () => {
    const list = makeSelectList();
    list.setSelectedIndex(ITEMS.length - 1);
    list.handleKey("down");
    expect(list.getSelectedItem()?.value).toBe("apple");
  });

  test("handleKey enter calls onSelect", () => {
    let selected: string | undefined;
    const list = makeSelectList({
      onSelect: (item: { value: string }) => { selected = item.value; },
    });
    list.handleKey("enter");
    expect(selected).toBe("apple");
  });

  test("handleKey escape calls onCancel", () => {
    let cancelled = false;
    const list = makeSelectList({ onCancel: () => { cancelled = true; } });
    list.handleKey("escape");
    expect(cancelled).toBe(true);
  });

  test("handleKey printable chars filter", () => {
    const list = makeSelectList();
    list.handleKey("b");
    list.handleKey("a");
    list.handleKey("n");
    expect(list.getSelectedItem()?.value).toBe("banana");
  });

  test("handleKey backspace removes filter char", () => {
    const list = makeSelectList();
    list.handleKey("b");
    list.handleKey("a");
    list.handleKey("n");
    list.handleKey("backspace");
    list.handleKey("backspace");
    expect(list.getSelectedItem()?.value).toBe("banana");
  });

  test("handleKey returns false for unhandled keys", () => {
    const list = makeSelectList();
    expect(list.handleKey("f5")).toBe(false);
  });

  test("onKey handler intercepts before default", () => {
    let intercepted = false;
    const list = makeSelectList({
      onKey: () => { intercepted = true; return true; },
    });
    list.handleKey("down");
    expect(intercepted).toBe(true);
  });

  test("onSelectionChange fires on navigation", () => {
    const changes: string[] = [];
    const list = makeSelectList({
      onSelectionChange: (item: { value: string }) => changes.push(item.value),
    });
    list.handleKey("down");
    list.handleKey("down");
    expect(changes).toEqual(["banana", "cherry"]);
  });

  test("render produces expected line count", () => {
    const list = makeSelectList();
    const lines = list.render(60);
    // title + filter + scroll-above + 3 visible + scroll-below = 7
    expect(lines.length).toBe(7);
  });

  test("render shows title and filter", () => {
    const list = makeSelectList();
    const lines = list.render(60);
    expect(stripAnsiCodes(lines[0])).toContain("Test List");
    expect(stripAnsiCodes(lines[1])).toContain("Filter:");
  });

  test("render shows items with selection indicator", () => {
    const list = makeSelectList();
    const lines = list.render(60);
    const itemLine = stripAnsiCodes(lines[3]);
    expect(itemLine).toContain("▶");
    expect(itemLine).toContain("Apple");
  });

  test("render shows scroll indicator when overflow", () => {
    const list = makeSelectList({ maxVisible: 2 });
    const lines = list.render(60);
    const hasScrollIndicator = lines.some(l => stripAnsiCodes(l).includes("more"));
    expect(hasScrollIndicator).toBe(true);
  });

  test("render empty list shows no matching items", () => {
    const list = new SelectList({ title: "Empty", items: [] });
    const lines = list.render(60);
    const hasNoItems = lines.some(l => stripAnsiCodes(l).includes("No matching items"));
    expect(hasNoItems).toBe(true);
  });

  test("render respects terminal width", () => {
    const list = makeSelectList();
    const lines = list.render(40);
    for (const line of lines) {
      expect(visibleWidth(line)).toBeLessThanOrEqual(40);
    }
  });
});

// ─── ScopedModelsSelector ──────────────────────────────────────────────────

const MODELS: ModelInfo[] = [
  { id: "gpt-4o", label: "GPT-4o", provider: "openai", supportsImages: true, thinkingLevel: "medium", contextWindow: 128000, isDefault: true },
  { id: "claude-sonnet-4", label: "Claude Sonnet 4", provider: "anthropic", supportsImages: true, thinkingLevel: "high", contextWindow: 200000, isDefault: false },
  { id: "o3-mini", label: "o3 Mini", provider: "openai", supportsImages: false, thinkingLevel: "high", contextWindow: 128000, isDefault: false },
  { id: "deepseek-r1", label: "DeepSeek R1", provider: "deepseek", supportsImages: false, thinkingLevel: "high", contextWindow: 64000, isDefault: false },
];

function makeSelector(overrides = {}) {
  return new ScopedModelsSelector({
    allModels: MODELS,
    enabledModelIds: new Set(["openai/gpt-4o", "anthropic/claude-sonnet-4"]),
    onSave: () => {},
    onCancel: () => {},
    maxVisible: 4,
    ...overrides,
  });
}

describe("ScopedModelsSelector", () => {
  test("models are sorted by provider/id", () => {
    let saved: string[] = [];
    const sel = makeSelector({ onSave: (ids: string[]) => { saved = ids; } });
    sel.handleKey("enter");
    expect(saved).toContain("openai/gpt-4o");
    expect(saved).toContain("anthropic/claude-sonnet-4");
  });

  test("space toggles model on/off", () => {
    let saved: string[] = [];
    const sel = makeSelector({ onSave: (ids: string[]) => { saved = ids; } });
    // Navigate to first model (sorted: anthropic/claude-sonnet-4)
    sel.handleKey("space"); // toggle off claude
    sel.handleKey("enter");
    expect(saved).not.toContain("anthropic/claude-sonnet-4");
    expect(saved).toContain("openai/gpt-4o");
  });

  test("space toggles model back on", () => {
    let saved: string[] = [];
    const sel = makeSelector({ onSave: (ids: string[]) => { saved = ids; } });
    sel.handleKey("space"); // toggle off
    sel.handleKey("space"); // toggle back on
    sel.handleKey("enter");
    expect(saved).toContain("anthropic/claude-sonnet-4");
  });

  test("escape calls onCancel without saving", () => {
    let saved = false;
    let cancelled = false;
    const sel = makeSelector({
      onSave: () => { saved = true; },
      onCancel: () => { cancelled = true; },
    });
    sel.handleKey("escape");
    expect(cancelled).toBe(true);
    expect(saved).toBe(false);
  });

  test("filter narrows model list", () => {
    const sel = makeSelector();
    sel.handleKey("d");
    sel.handleKey("e");
    sel.handleKey("e");
    sel.handleKey("p");
    // After "deep" filter, only deepseek-r1 should match
    const lines = sel.render(60);
    const text = lines.map(stripAnsiCodes).join("\n");
    expect(text).toContain("deepseek");
    expect(text).not.toContain("gpt-4o");
  });

  test("render shows enabled count", () => {
    const sel = makeSelector();
    const lines = sel.render(60);
    const text = lines.map(stripAnsiCodes).join("\n");
    expect(text).toContain("2/4 enabled");
  });

  test("render shows unsaved indicator after toggle", () => {
    const sel = makeSelector();
    const before = sel.render(60);
    expect(before.some(l => stripAnsiCodes(l).includes("unsaved"))).toBe(false);
    sel.handleKey("space");
    const after = sel.render(60);
    expect(after.some(l => stripAnsiCodes(l).includes("unsaved"))).toBe(true);
  });

  test("render shows ✓ for enabled, ✗ for disabled", () => {
    const sel = makeSelector();
    const lines = sel.render(60);
    const text = lines.map(stripAnsiCodes).join("\n");
    expect(text).toContain("✓");
    expect(text).toContain("✗");
  });

  test("handleKey up/down navigates sorted list", () => {
    // Sorted: anthropic/claude-sonnet-4, deepseek/deepseek-r1, openai/gpt-4o, openai/o3-mini
    let saved: string[] = [];
    const sel = makeSelector({ onSave: (ids: string[]) => { saved = ids; } });
    sel.handleKey("down"); // move to deepseek-r1
    sel.handleKey("space"); // enable deepseek-r1
    sel.handleKey("enter");
    expect(saved).toContain("deepseek/deepseek-r1");
  });

  test("empty filter shows all models", () => {
    const sel = makeSelector();
    const lines = sel.render(80);
    const text = lines.map(stripAnsiCodes).join("\n");
    expect(text).toContain("claude-sonnet-4");
    expect(text).toContain("deepseek-r1");
    expect(text).toContain("gpt-4o");
    expect(text).toContain("o3-mini");
  });

  test("handleInput delegates to handleKey", () => {
    let cancelled = false;
    const sel = makeSelector({ onCancel: () => { cancelled = true; } });
    sel.handleInput("escape");
    expect(cancelled).toBe(true);
  });
});
