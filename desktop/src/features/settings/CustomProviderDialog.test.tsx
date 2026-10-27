// @vitest-environment jsdom
import type { CustomProvider } from "../../integrations/agent/providers";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CustomProviderDialog } from "./CustomProviderDialog";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const initial: CustomProvider = {
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

/** The per-model thinking-level chips, in their fixed order. */
const CHIPS = ["Off", "Minimal", "Low", "Medium", "High", "XHigh", "Max"];

function chips(): HTMLButtonElement[] {
  const pressed = [...document.querySelectorAll<HTMLButtonElement>("button[aria-pressed]")];
  expect(pressed).toHaveLength(CHIPS.length);
  return pressed;
}

function pressedLabels(): string[] {
  return chips().filter(chip => chip.getAttribute("aria-pressed") === "true").map(chip => chip.textContent ?? "");
}

function mount(provider: typeof initial) {
  const onSubmit = vi.fn().mockResolvedValue(undefined);
  const container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  act(() => root.render(
    <CustomProviderDialog existing={[]} initial={provider} onClose={vi.fn()} onSubmit={onSubmit} open />,
  ));
  return onSubmit;
}

describe("custom model thinking levels", () => {
  it("offers the known wire values as a multi-select, with default implicit", () => {
    mount(initial);
    act(() => button("+ Add model").click());

    // `default` is always available and never a chip.
    expect([...document.querySelectorAll("button[aria-pressed]")].map(button => button.textContent)).toEqual(CHIPS);
    expect(document.body.textContent).not.toContain("Supports thinking");
  });

  it("starts a new model on off/low/medium/high", async () => {
    const onSubmit = mount(initial);
    act(() => button("+ Add model").click());
    const modelId = document.querySelector<HTMLInputElement>("input[placeholder=\"e.g. qwen\"]")!;
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(modelId, "unlisted-deployment");
      modelId.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(pressedLabels()).toEqual(["Off", "Low", "Medium", "High"]);

    await act(async () => button("Save").click());
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({
      models: [expect.objectContaining({
        id: "unlisted-deployment",
        thinkingLevels: ["off", "low", "medium", "high"],
        // No UI for it anymore, but the legacy flag still round-trips.
        reasoning: true,
      })],
    }));
  });

  it("toggles a level on and off, keeping the declared order", async () => {
    const onSubmit = mount(initial);
    act(() => button("+ Add model").click());
    const modelId = document.querySelector<HTMLInputElement>("input[placeholder=\"e.g. qwen\"]")!;
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(modelId, "qwen");
      modelId.dispatchEvent(new Event("input", { bubbles: true }));
      chips()[6]!.click(); // Max on
      chips()[2]!.click(); // Low off
    });
    expect(pressedLabels()).toEqual(["Off", "Medium", "High", "Max"]);

    await act(async () => button("Save").click());
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({
      models: [expect.objectContaining({ thinkingLevels: ["off", "medium", "high", "max"] })],
    }));
  });

  it("keeps a stored list and its unknown wire values on reopen", async () => {
    const provider = {
      ...initial,
      models: [{
        id: "gpt-5.6-sol",
        name: "Model",
        supportsImages: true,
        contextWindow: 128000,
        maxTokens: 16384,
        reasoning: false,
        thinkingLevels: ["off", "max", "ultra"],
        inputCost: 0,
        outputCost: 0,
        cacheReadCost: 0,
        cacheWriteCost: 0,
      }],
    };
    const onSubmit = mount(provider);

    // The known ones show as selected; the unknown one has no chip of its own...
    expect(pressedLabels()).toEqual(["Off", "Max"]);
    // ...but it is not lost: it rides along in the submitted list.
    await act(async () => button("Save").click());
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({
      models: [expect.objectContaining({
        thinkingLevels: ["off", "max", "ultra"],
        reasoning: false,
        supportsImages: true,
      })],
    }));
  });

  it("sends an empty list when every level is switched off", async () => {
    const provider = {
      ...initial,
      models: [{
        id: "m1",
        name: "M1",
        supportsImages: false,
        contextWindow: 128000,
        maxTokens: 16384,
        reasoning: true,
        thinkingLevels: ["off", "low"],
        inputCost: 0,
        outputCost: 0,
        cacheReadCost: 0,
        cacheWriteCost: 0,
      }],
    };
    const onSubmit = mount(provider);
    act(() => {
      chips()[0]!.click(); // Off off
      chips()[2]!.click(); // Low off
    });
    await act(async () => button("Save").click());
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({
      models: [expect.objectContaining({ thinkingLevels: [] })],
    }));
  });
});
