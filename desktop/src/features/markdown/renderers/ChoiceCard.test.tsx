import type { ChoiceCardProps } from "./ChoiceCard";
// @vitest-environment jsdom
import { CHOICE_CONTEXT_VISIBLE_LINES } from "@future-os/markdown";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChoiceCard } from "./ChoiceCard";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const options = [
  { description: "按 Dreher 2025 的样本量重算两篇的置信区间", id: "forest", label: "重画森林图" },
  { description: "把两篇的被试流程画成一张对照图", id: "flow", label: "补一张实验流程图" },
  { description: "Frank 42 / Dreher 186", id: "table", label: "表里加一列样本量" },
];

function render(props: Partial<ChoiceCardProps> = {}) {
  return renderToStaticMarkup(
    createElement(ChoiceCard, { mode: "single", options, title: "接下来做哪一件？", ...props }),
  );
}

describe("choice card rendering", () => {
  let container: HTMLDivElement;
  let root: ReturnType<typeof createRoot>;

  beforeEach(() => {
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  function mount(props: Partial<ChoiceCardProps> = {}) {
    act(() => {
      root.render(createElement(ChoiceCard, { mode: "single", options, title: "接下来做哪一件？", ...props }));
    });
    return container;
  }

  it("draws the title, the mode label and one row per option — and no subtitle of its own", () => {
    const html = render({ onSelect: vi.fn() });
    expect(html).toContain("接下来做哪一件？");
    expect(html).toContain("Single choice");
    for (const option of options) {
      expect(html).toContain(option.label);
      expect(html).toContain(option.description ?? "");
    }
    // 2026-11-18: the default subtitle is gone. "单选" plus the radio rings say
    // one-or-several already, and the old line explained a mechanism (the answer
    // lands in the input) that the composer shows by itself.
    expect(html).not.toContain("lands in the input");
    expect(html).not.toContain("Pick as many as apply");
  });

  it("carries the accessible structure from §9: named group, radio rows, aria-checked", () => {
    const card = mount().querySelector("[data-choice-card]")!;
    expect(card.getAttribute("role")).toBe("group");
    const labelId = card.getAttribute("aria-labelledby");
    expect(labelId).toBeTruthy();
    expect(document.getElementById(labelId!)?.textContent).toBe("接下来做哪一件？");

    const rows = card.querySelectorAll("[data-choice-option]");
    expect(rows).toHaveLength(3);
    expect([...rows].map(row => row.getAttribute("role"))).toEqual(["radio", "radio", "radio"]);
    expect([...rows].map(row => row.getAttribute("aria-checked"))).toEqual(["false", "false", "false"]);
    // A mutually exclusive card wears the radio ring; only multi uses a box.
    expect([...card.querySelectorAll("[data-choice-mark]")].every(mark => mark.getAttribute("data-choice-mark") === "ring")).toBe(true);
  });

  it("draws the context under the title, and nothing when the fence omitted it", () => {
    const html = render({
      context: "森林图已按新样本量重画，图注还挂着旧的 42。\n图例仍压在曲线上。",
      onSelect: vi.fn(),
    });
    expect(html).toContain("data-choice-context");
    // Repeated `context:` lines are paragraphs, so the break is kept.
    expect(html).toContain("图注还挂着旧的 42。\n图例仍压在曲线上。");
    // Order: title, then the situation, then the rows.
    expect(html.indexOf("接下来做哪一件？")).toBeLessThan(html.indexOf("森林图已按新样本量重画"));
    expect(html.indexOf("森林图已按新样本量重画")).toBeLessThan(html.indexOf("重画森林图"));

    expect(render({ onSelect: vi.fn() })).not.toContain("data-choice-context");
  });

  it("keeps the context on a card that cannot be picked", () => {
    // The subtitle turns into "this card cannot be picked" (§2), but the
    // situation is information: a past card still says what it was asking about.
    const html = render({ context: "森林图已重画完了。" });
    expect(html).toContain("森林图已重画完了。");
  });

  it("uses checkbox rows for a multi-choice card", () => {
    const html = render({ mode: "multi", onSelect: vi.fn() });
    expect(html).toContain("Multiple choice");
    // The label and the box shape carry it; there is no "pick several" line.
    expect(html).not.toContain("Pick as many as apply");
    const card = mount({ mode: "multi", onSelect: vi.fn() });
    expect([...card.querySelectorAll("[data-choice-option]")].every(row => row.getAttribute("role") === "checkbox")).toBe(true);
    // Shape, not just the corner label, says the rows can be picked together.
    expect([...card.querySelectorAll("[data-choice-mark]")].every(mark => mark.getAttribute("data-choice-mark") === "box")).toBe(true);
  });

  it("prints the model's own hint, and only when it wrote one", () => {
    const html = render({ hint: "点几个都行", onSelect: vi.fn() });
    expect(html).toContain("点几个都行");
    // Nothing is invented when the fence carried no hint.
    expect(render({ onSelect: vi.fn() })).not.toContain("data-choice-hint");
  });

  it("is read-only without an answer surface: rows out of the tab order, no hover, nothing dimmed", () => {
    const card = mount();
    expect(card.querySelector("[data-choice-card]")?.getAttribute("data-choice-state")).toBe("readonly");
    const rows = [...card.querySelectorAll<HTMLElement>("[data-choice-option]")];
    expect(rows.every(row => row.tabIndex === -1)).toBe(true);
    expect(rows.every(row => row.getAttribute("aria-disabled") === "true")).toBe(true);
    expect(rows.every(row => !row.className.includes("hover:"))).toBe(true);
    // Nothing is dimmed while no answer is known: a uniformly grey card with no
    // marked row reads as broken (see ChoiceOptionRow).
    expect(rows.some(row => row.className.includes("opacity-50"))).toBe(false);
  });

  it("reports a pick through onSelect and is focusable once it has an answer surface", () => {
    const onSelect = vi.fn();
    const card = mount({ onSelect });
    expect(card.querySelector("[data-choice-card]")?.getAttribute("data-choice-state")).toBe("active");
    const rows = [...card.querySelectorAll<HTMLElement>("[data-choice-option]")];
    expect(rows.map(row => row.tabIndex)).toEqual([0, -1, -1]);
    act(() => rows[1]!.click());
    expect(onSelect).toHaveBeenCalledWith("flow");
  });

  it("marks the answer the draft holds and reveals the footer only then", () => {
    expect(render()).not.toContain("In the input — add a note if you want, or send as is");

    const card = mount({ onSelect: vi.fn(), selectedIds: ["forest"] });
    const rows = [...card.querySelectorAll<HTMLElement>("[data-choice-option]")];
    expect(rows.map(row => row.getAttribute("aria-checked"))).toEqual(["true", "false", "false"]);
    expect(card.textContent).toContain("In the input — add a note if you want, or send as is");
    // The chosen row is tinted; no row carries a border (fill-only emphasis, §5.2).
    expect(rows[0]!.className).toContain("bg-accent-soft");
    expect(rows[0]!.className).not.toContain("border");
    // Roving tabindex now starts from the chosen row.
    expect(rows.map(row => row.tabIndex)).toEqual([0, -1, -1]);
  });

  it("dims the other rows on a past card that has a marked answer", () => {
    const card = mount({ selectedIds: ["forest"] });
    const rows = [...card.querySelectorAll("[data-choice-option]")];
    expect(rows[0]!.className).not.toContain("opacity-50");
    expect(rows[1]!.className).toContain("opacity-50");
    expect(rows[2]!.className).toContain("opacity-50");
  });

  it("walks rows with the arrow keys and picks with Space", () => {
    const onSelect = vi.fn();
    const card = mount({ onSelect });
    const rows = [...card.querySelectorAll<HTMLElement>("[data-choice-option]")];
    rows[0]!.focus();
    act(() => {
      rows[0]!.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" }));
    });
    expect(document.activeElement).toBe(rows[1]);
    act(() => {
      rows[1]!.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowUp" }));
    });
    expect(document.activeElement).toBe(rows[0]);
    act(() => {
      rows[0]!.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: " " }));
    });
    expect(onSelect).toHaveBeenCalledWith("forest");
  });

  /**
   * A past card owes the reader one fact the live card does not: the rows are
   * inert. It says so underneath whatever the reply itself wrote, and the model's
   * own line stays — history is re-read, not answered.
   */
  it("keeps what the reply itself said on a read-only card, and adds that it is inert", () => {
    const card = mount({ hint: "点几个都行" }).querySelector<HTMLElement>("[data-choice-card]")!;
    expect(card.textContent).toContain("An earlier question — this card can no longer be picked");
    expect(card.textContent).toContain("点几个都行");
    // Order matters: what was said at the time, then the fact that it is inert.
    expect(card.textContent!.indexOf("点几个都行"))
      .toBeLessThan(card.textContent!.indexOf("An earlier question"));
    expect(card.getAttribute("aria-disabled")).toBe("true");
    // Both sentences describe the group to a screen reader, in the same order.
    const describedBy = card.getAttribute("aria-describedby")!;
    expect(describedBy.split(" ").map(id => document.getElementById(id)?.textContent))
      .toEqual(["点几个都行", "An earlier question — this card can no longer be picked"]);
  });

  it("says only that a past card is inert when the reply carried no hint of its own", () => {
    const card = mount().querySelector<HTMLElement>("[data-choice-card]")!;
    const describedBy = card.getAttribute("aria-describedby")!;
    expect(describedBy.split(" ").map(id => document.getElementById(id)?.textContent))
      .toEqual(["An earlier question — this card can no longer be picked"]);
  });

  it("keeps a read-only card out of the tab order and refuses focus", () => {
    const card = mount();
    const rows = [...card.querySelectorAll<HTMLElement>("[data-choice-option]")];
    for (const row of rows) {
      // `tabIndex` -1 is not enough on its own: a script or a click can still
      // focus such a row, so the element itself must refuse too.
      expect(row.tabIndex).toBe(-1);
      expect(row.getAttribute("aria-disabled")).toBe("true");
    }
    rows[0]!.focus();
    expect(document.activeElement).not.toBe(rows[0]);
  });

  it("drops the inert notice as soon as the card has an answer surface", () => {
    const card = mount({ onSelect: vi.fn() }).querySelector<HTMLElement>("[data-choice-card]")!;
    expect(card.getAttribute("aria-disabled")).toBeNull();
    expect(card.textContent).not.toContain("An earlier question");
    // With no model hint there is nothing to describe the group with at all.
    expect(card.getAttribute("aria-describedby")).toBeNull();
  });

  /**
   * A context that outgrows the card folds down to `CHOICE_CONTEXT_VISIBLE_LINES`
   * lines behind a button; one that fits is left alone, with no button at all — a
   * "show more" over text that is already fully visible is a control that does
   * nothing (the same reason the default subtitle went, §5.1).
   */
  it("leaves a context that fits in the visible lines unclamped and without a button", () => {
    const card = mount({ context: "森林图已重画完了。图例仍压在曲线上。" });
    const context = card.querySelector<HTMLElement>("[data-choice-context]")!;
    expect(context.className).not.toContain("line-clamp");
    expect(context.getAttribute("data-choice-context-folded")).toBeNull();
    expect(card.querySelector("[data-choice-context-toggle]")).toBeNull();
  });

  it("folds a long context and expands it on one click", () => {
    const card = mount({
      context: Array.from({ length: 8 }, (_, index) => `第 ${index + 1} 条：图注还挂着旧的 42。`).join("\n"),
      onSelect: vi.fn(),
    });
    const context = card.querySelector<HTMLElement>("[data-choice-context]")!;
    // The clamp follows the estimate (choiceContext.ts) and mirrors the shared
    // line count: if the constant moves, this reads the new number and fails.
    expect(context.className).toContain(`line-clamp-${CHOICE_CONTEXT_VISIBLE_LINES}`);
    expect(context.getAttribute("data-choice-context-folded")).toBe("true");
    // The whole text stays in the DOM, so the reader and the accessibility tree
    // never lose a word of it — only the first lines are drawn.
    expect(context.textContent).toContain("第 8 条");

    const toggle = card.querySelector<HTMLButtonElement>("[data-choice-context-toggle]")!;
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(toggle.getAttribute("aria-controls")).toBe(context.id);
    expect(toggle.textContent).toContain("Show more");

    act(() => toggle.click());

    const expanded = card.querySelector<HTMLElement>("[data-choice-context]")!;
    expect(expanded.className).not.toContain("line-clamp");
    expect(expanded.getAttribute("data-choice-context-folded")).toBeNull();
    const after = card.querySelector<HTMLButtonElement>("[data-choice-context-toggle]")!;
    expect(after.getAttribute("aria-expanded")).toBe("true");
    expect(after.textContent).toContain("Show less");

    act(() => after.click());
    expect(card.querySelector("[data-choice-context]")!.getAttribute("data-choice-context-folded")).toBe("true");
  });

  it("folds the context on a card that cannot be picked too", () => {
    // Reading is exactly what a past card is for, so the fold must work there:
    // the toggle answers nothing and therefore is not part of the answer surface.
    const card = mount({ context: Array.from({ length: 8 }).fill("图注还挂着旧的 42。").join("\n") });
    const toggle = card.querySelector<HTMLButtonElement>("[data-choice-context-toggle]")!;
    act(() => toggle.click());
    expect(card.querySelector("[data-choice-context]")!.getAttribute("data-choice-context-folded")).toBeNull();
    // No answer surface appeared alongside it.
    expect(card.querySelector("[data-choice-card]")!.getAttribute("aria-disabled")).toBe("true");
  });
});
