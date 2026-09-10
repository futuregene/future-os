// @vitest-environment jsdom
import type { AgentMessage } from "@future-os/thread-projection";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { MessageBlock } from "./MessageBlock";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

it.each(["en", "zh"])("shows reconnect attempts visibly in %s and clears on resume or termination", async (language) => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const base: AgentMessage = {
    id: "retry-message",
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: "",
    createdAt: new Date().toISOString(),
    status: "streaming",
  };
  const render = async (patch: Partial<AgentMessage>) => {
    await act(async () => root.render(<MessageBlock message={{ ...base, ...patch }} hovered={false} onHover={vi.fn()} onLeave={vi.fn()} />));
  };
  try {
    await act(async () => {
      await i18n.changeLanguage(language);
    });
    for (const attempt of [1, 5]) {
      await render({ reconnecting: { attempt, maxRetries: 5, delayMs: 2000 } });
      const label = language === "zh" ? `正在重连 ${attempt}/5` : `Reconnecting ${attempt}/5`;
      const status = container.querySelector("[role=\"status\"]");
      expect(status?.textContent).toBe(label);
      expect(status?.getAttribute("aria-label")).toBe(label);
    }
    await render({});
    expect(container.querySelector("[role=\"status\"]")?.textContent).toBe("");
    expect(container.querySelector("[role=\"status\"]")?.getAttribute("aria-label")).toBe(i18n.t("agent:message.generating"));
    for (const status of ["complete", "failed"] as const) {
      await render({ status, reconnecting: { attempt: 5, maxRetries: 5, delayMs: 32000 } });
      const reconnectLabel = language === "zh" ? "正在重连 5/5" : "Reconnecting 5/5";
      expect(container.textContent).not.toContain(reconnectLabel);
      expect(container.querySelector(`[aria-label="${reconnectLabel}"]`)).toBeNull();
    }
  }
  finally {
    act(() => root.unmount());
    container.remove();
    await i18n.changeLanguage("en");
  }
});
