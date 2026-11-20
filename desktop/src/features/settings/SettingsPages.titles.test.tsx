// @vitest-environment jsdom
import type { ReactElement } from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { DEFAULT_APP_SETTINGS } from "../../integrations/storage/appSettings";
import { AppearancePage } from "./AppearancePage";
import { ReplyPage } from "./ReplyPage";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function mount(node: ReactElement) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  return {
    container,
    root,
    async render() {
      await act(async () => root.render(node));
    },
    unmount() {
      act(() => root.unmount());
      container.remove();
    },
  };
}

it("enables title generation by default and allows opting out without context compaction", async () => {
  const onToggle = vi.fn();
  const page = mount(
    <ReplyPage
      autoTitleFirstTurn={DEFAULT_APP_SETTINGS.autoTitleFirstTurn}
      choiceCards={DEFAULT_APP_SETTINGS.choiceCards}
      onToggleAutoTitleFirstTurn={onToggle}
      onToggleChoiceCards={() => {}}
    />,
  );
  try {
    await page.render();
    // The reply page carries the two reply-level switches and nothing else: the
    // completion bell is this machine's own notification (see the appearance
    // page below), and thinking output is not a switch at all.
    expect(page.container.querySelectorAll("[role=switch]")).toHaveLength(2);
    expect(page.container.textContent).not.toContain("Completion bell");
    expect(page.container.textContent).not.toContain("Show thinking process");
    expect(DEFAULT_APP_SETTINGS).not.toHaveProperty("showThinking");
    const toggle = page.container.querySelector<HTMLButtonElement>("[role=switch][aria-label='Generate a title after the first answer']");
    expect(toggle).not.toBeNull();
    expect(toggle!.getAttribute("aria-checked")).toBe("true");
    expect(page.container.textContent).toContain("Later answers do not trigger it");
    expect(page.container.textContent).toContain("Conversation context is not compacted");
    await act(async () => toggle!.click());
    expect(onToggle).toHaveBeenCalledExactlyOnceWith(false);
  }
  finally {
    page.unmount();
  }
});

it("keeps the completion bell with this machine's own preferences", async () => {
  const onToggle = vi.fn();
  const page = mount(
    <AppearancePage
      bellOnComplete={DEFAULT_APP_SETTINGS.bellOnComplete}
      onToggleBellOnComplete={onToggle}
    />,
  );
  try {
    await page.render();
    // It plays on this machine and no paired phone offers it, so it is a local
    // preference — the same page as language, theme and text size.
    const rows = [...page.container.querySelectorAll("[role=switch]")];
    expect(rows.map(row => row.getAttribute("aria-label"))).toEqual(["Completion bell"]);
    expect(page.container.textContent).toContain("Alerts");
    expect(DEFAULT_APP_SETTINGS.bellOnComplete).toBe(true);
    await act(async () => page.container.querySelector<HTMLButtonElement>("[role=switch]")!.click());
    expect(onToggle).toHaveBeenCalledExactlyOnceWith(false);
  }
  finally {
    page.unmount();
  }
});
