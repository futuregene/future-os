import type { ReactElement } from "react";
// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { DEFAULT_APP_SETTINGS } from "../../integrations/storage/appSettings";
import { ChoiceCardsProvider } from "./ChoiceCardsContext";
import { MarkdownContent } from "./MarkdownContent";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/** A reply that closes with a card, exactly as the prompt teaches the model. */
const REPLY = `今天的检查都过了。

\`\`\`futureos-choice
title: 接下来做哪一件？
option: forest | 重画森林图 | 按 Dreher 2025 的样本量重算两篇的置信区间
option: flow | 补一张实验流程图 | 把两篇的被试流程画成一张对照图
\`\`\`
`;

/** The same fence with one row: not a card, so it degrades to a code block. */
const MALFORMED = `\`\`\`futureos-choice
title: 只有一行
option: only | 唯一的选项
\`\`\`
`;

function mount(ui: ReactElement) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(ui));
  return {
    container,
    cleanup: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

describe("the choice-card switch", () => {
  it("ships on", () => {
    // The persisted default, before any database row exists: the renderer's own
    // default is not the product's answer, so pin the stored one.
    expect(DEFAULT_APP_SETTINGS.choiceCards).toBe(true);
  });

  it("draws neither the card nor its source when off", () => {
    const { container, cleanup } = mount(
      createElement(ChoiceCardsProvider, { value: false }, createElement(MarkdownContent, { content: REPLY })),
    );
    try {
      // Structural: no card node at all…
      expect(container.querySelectorAll("[data-choice-card]")).toHaveLength(0);
      // …and not one character of the fence, not even as a code block. This is
      // what "根本不显示" means: the block leaves no trace, so a reader who turned
      // cards off never sees the card's markdown in the middle of a reply.
      expect(container.textContent).not.toContain("futureos-choice");
      expect(container.textContent).not.toContain("接下来做哪一件？");
      expect(container.textContent).not.toContain("重画森林图");
      // The prose around it survives — only the card block goes.
      expect(container.textContent).toContain("今天的检查都过了。");
    }
    finally {
      cleanup();
    }
  });

  it("draws nothing for a malformed fence either", () => {
    // A one-row card is not a card (`parseChoiceCard` returns null and the caller
    // normally draws the fenced source). With the switch off that fallback must
    // not resurface the fence as a code block.
    const { container, cleanup } = mount(
      createElement(ChoiceCardsProvider, { value: false }, createElement(MarkdownContent, { content: MALFORMED })),
    );
    try {
      expect(container.querySelectorAll("[data-choice-card]")).toHaveLength(0);
      expect(container.textContent).not.toContain("futureos-choice");
      expect(container.textContent).not.toContain("唯一的选项");
    }
    finally {
      cleanup();
    }
  });

  it("draws the card, and answers it, when on", () => {
    const onSelect = vi.fn();
    const { container, cleanup } = mount(
      createElement(
        ChoiceCardsProvider,
        { value: true },
        createElement(MarkdownContent, {
          choiceAnswer: { onSelect, selectedIds: [] },
          content: REPLY,
        }),
      ),
    );
    try {
      const card = container.querySelector("[data-choice-card]");
      expect(card).not.toBeNull();
      expect(card!.getAttribute("data-choice-state")).toBe("active");
      expect(container.textContent).toContain("接下来做哪一件？");
      const rows = container.querySelectorAll<HTMLElement>("[data-choice-option]");
      expect([...rows].map(row => row.dataset.choiceOption)).toEqual(["forest", "flow"]);
      act(() => rows[0]!.click());
      expect(onSelect).toHaveBeenCalledExactlyOnceWith(
        "forest",
        expect.objectContaining({ title: "接下来做哪一件？", mode: "single" }),
      );
    }
    finally {
      cleanup();
    }
  });

  it("keeps a card read-only when on but unanswered", () => {
    // Behaviour before the switch existed must survive it: without an answer
    // surface every card is inert, which is what an old reply renders as.
    const { container, cleanup } = mount(
      createElement(ChoiceCardsProvider, { value: true }, createElement(MarkdownContent, { content: REPLY })),
    );
    try {
      const card = container.querySelector("[data-choice-card]")!;
      expect(card.getAttribute("data-choice-state")).toBe("readonly");
      expect(container.querySelector("[data-choice-option]")!.getAttribute("tabindex")).toBeNull();
    }
    finally {
      cleanup();
    }
  });
});
