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
function reasoningCheckbox() {
  const label = [...document.querySelectorAll("label")].find(label => label.textContent === "Supports thinking");
  const found = label?.querySelector<HTMLInputElement>("input[type=checkbox]");
  expect(found).toBeTruthy();
  return found!;
}

describe("custom model thinking capability", () => {
  it("checks a new model by default and saves false when its label is clicked", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    act(() => root.render(<CustomProviderDialog existing={[]} initial={initial} onClose={vi.fn()} onSubmit={onSubmit} open />));
    act(() => button("+ Add model").click());
    expect(reasoningCheckbox().checked).toBe(true);
    const modelId = document.querySelector<HTMLInputElement>("input[placeholder=\"e.g. qwen\"]")!;
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(modelId, "unlisted-deployment");
      modelId.dispatchEvent(new Event("input", { bubbles: true }));
      reasoningCheckbox().closest("label")!.click();
    });
    expect(reasoningCheckbox().checked).toBe(false);
    await act(async () => button("Save").click());
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({
      models: [expect.objectContaining({ id: "unlisted-deployment", reasoning: false, supportsImages: false })],
    }));
  });

  it("keeps a saved false unchecked when reopening and saves true when checked", async () => {
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
    expect(reasoningCheckbox().checked).toBe(false);
    act(() => reasoningCheckbox().click());
    await act(async () => button("Save").click());
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({
      models: [expect.objectContaining({ reasoning: true, supportsImages: true })],
    }));
  });
});
