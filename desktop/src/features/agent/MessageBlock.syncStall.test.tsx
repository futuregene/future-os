// @vitest-environment jsdom
import type { AgentMessage, PreviewSyncStall } from "@future-os/thread-projection";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { MessageBlock } from "./MessageBlock";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const STALL: PreviewSyncStall = {
  stage: "read",
  reason: "read: get_events_since: event batch commit failed",
  attempts: 12,
  since: Date.parse("2026-11-03T12:00:00.000Z"),
};

it.each(["en", "zh"])("shows a pausable sync note on a stuck reply in %s, and withdraws it on recovery", async (language) => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const base: AgentMessage = {
    id: "stalled-message",
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: "The",
    createdAt: new Date().toISOString(),
    status: "streaming",
    runId: "run-stalled",
  };
  const render = async (patch: Partial<AgentMessage>) => {
    await act(async () => root.render(<MessageBlock message={{ ...base, ...patch }} hovered={false} onHover={vi.fn()} onLeave={vi.fn()} />));
  };
  const label = language === "zh" ? "同步暂缓" : "Sync paused";
  const readStage = language === "zh" ? "读取这个回复的事件流暂缓" : "Reading this reply's events is paused";
  const attempts = language === "zh" ? "连续 12 次未成功" : "12 attempts without success";
  const note = () => container.querySelector<HTMLButtonElement>(`button[aria-expanded]`);

  try {
    await act(async () => {
      await i18n.changeLanguage(language);
    });

    // One short line while collapsed: the reason stays out of the way until asked.
    await render({ previewStalled: STALL });
    expect(note()?.textContent).toBe(label);
    expect(note()?.getAttribute("aria-expanded")).toBe("false");
    expect(container.textContent).not.toContain(STALL.reason);
    // ...and it is not an error: no warning divider, no alert role.
    expect(container.querySelector("[role=\"alert\"]")).toBeNull();
    expect(container.textContent).not.toContain(i18n.t("agent:thread.responseIncomplete"));

    // Opening it names the stage, the streak and the raw diagnostic.
    await act(async () => note()!.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(note()?.getAttribute("aria-expanded")).toBe("true");
    expect(container.textContent).toContain(readStage);
    expect(container.textContent).toContain(attempts);
    expect(container.textContent).toContain(STALL.reason);

    // Asking again folds it back.
    await act(async () => note()!.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(note()?.getAttribute("aria-expanded")).toBe("false");

    // The first successful read clears the field and the note goes with it.
    await render({ previewStalled: undefined, content: "The reply resumed" });
    expect(note()).toBeNull();
    expect(container.textContent).not.toContain(label);
    expect(container.textContent).toContain("The reply resumed");

    // A note left on a settled reply is stale presentation: only a streaming
    // row may carry it, so it never shows on a finished one.
    await render({ status: "complete", previewStalled: STALL });
    expect(note()).toBeNull();
  }
  finally {
    act(() => root.unmount());
    container.remove();
    await i18n.changeLanguage("en");
  }
});
