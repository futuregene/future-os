// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { Composer } from "./Composer";

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: vi.fn(async (command: string) => command === "list_agent_providers" ? { builtin: [], custom: [] } : []),
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

it("disables thinking levels for a model configured off, and enables them after a capability refresh", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onThinkingLevelChange = vi.fn();
  const model = { id: "m", provider: "custom", label: "Model", thinkingLevel: "high", reasoning: false };
  const render = async () => {
    await act(async () => root.render(<Composer onSend={vi.fn()} modelId="custom/m" modelOptions={[{ ...model }]} thinkingLevel="high" onThinkingLevelChange={onThinkingLevelChange} />));
  };
  try {
    await render();
    const button = container.querySelector<HTMLButtonElement>("button[aria-label=\"Thinking level\"]")!;
    expect(button).not.toBeNull();
    expect(button.disabled).toBe(true);
    expect(button.textContent).toContain("Off");
    expect(button.title).toContain("configured as not supporting thinking controls");
    act(() => button.click());
    expect(onThinkingLevelChange).not.toHaveBeenCalled();
    model.reasoning = true;
    await render();
    expect(button.disabled).toBe(false);
    expect(button.textContent).toContain("High");
    act(() => button.click());
    const low = [...document.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent?.includes("Low"));
    expect(low).toBeTruthy();
    act(() => low!.click());
    expect(onThinkingLevelChange).toHaveBeenCalledWith("low");
  }
  finally {
    act(() => root.unmount());
    container.remove();
  }
});
