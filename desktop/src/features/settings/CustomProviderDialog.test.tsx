// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CustomProviderDialog } from "./CustomProviderDialog";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const initial = {
  id: "custom",
  name: "Custom",
  api: "openai-responses",
  baseUrl: "https://example.test/v1",
  hasApiKey: true,
  models: [],
};
let root: ReturnType<typeof createRoot>;
afterEach(() => {
  act(() => root.unmount());
  document.body.innerHTML = "";
});
function button(text: string) {
  const found = [...document.querySelectorAll("button")].find(button => button.textContent === text);
  expect(found).toBeTruthy();
  return found!;
}
function reasoningSwitch() {
  const found = document.querySelector<HTMLButtonElement>("[role=\"switch\"][aria-label=\"Supports thinking\"]");
  expect(found).not.toBeNull();
  return found!;
}

describe("custom model thinking capability", () => {
  it("defaults a new model on and saves an explicit false when switched off", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    act(() => root.render(<CustomProviderDialog existing={[]} initial={initial} onClose={vi.fn()} onSubmit={onSubmit} open />));
    act(() => button("+ Add model").click());
    expect(reasoningSwitch().getAttribute("aria-checked")).toBe("true");
    const modelId = document.querySelector<HTMLInputElement>("input[placeholder=\"e.g. qwen\"]")!;
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(modelId, "unlisted-deployment");
      modelId.dispatchEvent(new Event("input", { bubbles: true }));
      reasoningSwitch().click();
    });
    expect(reasoningSwitch().getAttribute("aria-checked")).toBe("false");
    await act(async () => button("Save").click());
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({
      models: [expect.objectContaining({ id: "unlisted-deployment", reasoning: false })],
    }));
  });

  it("keeps a saved false off when reopening and saves true when re-enabled", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const provider = { ...initial, models: [{
      id: "gpt-5.6-sol",
      name: "Model",
      supportsImages: true,
      contextWindow: 128000,
      maxTokens: 16384,
      reasoning: false,
    }] };
    const container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    act(() => root.render(<CustomProviderDialog existing={[]} initial={provider} onClose={vi.fn()} onSubmit={onSubmit} open />));
    expect(reasoningSwitch().getAttribute("aria-checked")).toBe("false");
    act(() => reasoningSwitch().click());
    await act(async () => button("Save").click());
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({
      models: [expect.objectContaining({ reasoning: true })],
    }));
  });
});
