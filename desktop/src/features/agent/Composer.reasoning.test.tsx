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

it("disables the picker for a single-level model, and enables it once the agent publishes more", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onThinkingLevelChange = vi.fn();
  // The agent's effective list for a model without thinking controls: just the
  // implicit default. No client-side `reasoning` reading is involved.
  const model = {
    id: "m",
    provider: "custom",
    label: "Model",
    thinkingLevels: ["default"],
    defaultThinkingLevel: "default",
  };
  const render = async () => {
    await act(async () => root.render(<Composer onSend={vi.fn()} modelId="custom/m" modelOptions={[{ ...model }]} thinkingLevel="high" onThinkingLevelChange={onThinkingLevelChange} />));
  };
  try {
    await render();
    const button = container.querySelector<HTMLButtonElement>("button[aria-label=\"Thinking level\"]")!;
    expect(button).not.toBeNull();
    expect(button.disabled).toBe(true);
    expect(button.textContent).toContain("default");
    expect(button.title).toContain("no thinking-level choices");
    act(() => button.click());
    expect(onThinkingLevelChange).not.toHaveBeenCalled();

    model.thinkingLevels = ["default", "off", "low", "high"];
    await render();
    expect(button.disabled).toBe(false);
    expect(button.textContent).toContain("high");
    act(() => button.click());
    const low = [...document.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent?.includes("low"));
    expect(low).toBeTruthy();
    act(() => low!.click());
    expect(onThinkingLevelChange).toHaveBeenCalledWith("low");
  }
  finally {
    act(() => root.unmount());
    container.remove();
  }
});
