import type { ChoiceCardProps } from "./ChoiceCard";
// @vitest-environment jsdom
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

  it("draws the title, the mode label, the hint and one row per option", () => {
    const html = render({ onSelect: vi.fn() });
    expect(html).toContain("接下来做哪一件？");
    expect(html).toContain("Single choice");
    expect(html).toContain("Pick one and it lands in the input; you can type something else instead");
    for (const option of options) {
      expect(html).toContain(option.label);
      expect(html).toContain(option.description ?? "");
    }
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

  it("uses checkbox rows for a multi-choice card and its own hint", () => {
    const html = render({ mode: "multi", onSelect: vi.fn() });
    expect(html).toContain("Multiple choice");
    expect(html).toContain("Pick as many as apply");
    expect(html).not.toContain("Pick one and it lands in the input");
    const card = mount({ mode: "multi", onSelect: vi.fn() });
    expect([...card.querySelectorAll("[data-choice-option]")].every(row => row.getAttribute("role") === "checkbox")).toBe(true);
    // Shape, not just the corner label, says the rows can be picked together.
    expect([...card.querySelectorAll("[data-choice-mark]")].every(mark => mark.getAttribute("data-choice-mark") === "box")).toBe(true);
  });

  it("prefers the model's own hint over the default subtitle", () => {
    const html = render({ hint: "点几个都行", onSelect: vi.fn() });
    expect(html).toContain("点几个都行");
    expect(html).not.toContain("Pick one and it lands in the input");
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
   * The subtitle is the only place a card can promise an action. A card that
   * cannot answer must not print the answering one: "pick one and it lands in
   * the input" over rows that do nothing is a button that looks pressable and
   * isn't — the whole point of the read-only state (§3's tolerance table).
   */
  it("does not promise a pick on a read-only card, in visible text or to a screen reader", () => {
    const card = mount({ hint: "点几个都行" }).querySelector<HTMLElement>("[data-choice-card]")!;
    expect(card.textContent).toContain("An earlier question — this card can no longer be picked");
    expect(card.textContent).not.toContain("Pick one and it lands in the input");
    // The model's own hint describes picking too, so it goes the same way.
    expect(card.textContent).not.toContain("点几个都行");
    // And the same sentence is what a screen reader reads for the group.
    expect(card.getAttribute("aria-disabled")).toBe("true");
    const describedBy = card.getAttribute("aria-describedby");
    expect(describedBy).toBeTruthy();
    expect(document.getElementById(describedBy!)?.textContent)
      .toBe("An earlier question — this card can no longer be picked");
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

  it("prints the answering hint again as soon as it has an answer surface", () => {
    const card = mount({ onSelect: vi.fn() }).querySelector<HTMLElement>("[data-choice-card]")!;
    expect(card.textContent).toContain("Pick one and it lands in the input");
    expect(card.textContent).not.toContain("An earlier question");
    expect(card.getAttribute("aria-disabled")).toBeNull();
  });
});
