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
const CHIPS = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

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
    expect(pressedLabels()).toEqual(["off", "low", "medium", "high"]);

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
      chips()[6]!.click(); // max on
      chips()[2]!.click(); // low off
    });
    expect(pressedLabels()).toEqual(["off", "medium", "high", "max"]);

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
    expect(pressedLabels()).toEqual(["off", "max"]);
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
      chips()[0]!.click(); // off off
      chips()[2]!.click(); // low off
    });
    await act(async () => button("Save").click());
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({
      models: [expect.objectContaining({ thinkingLevels: [] })],
    }));
  });
});

/**
 * What the write carries is a clean list, not whatever was stored.
 *
 * A provider saved by an older build (or edited by hand) can hold the reserved
 * `default`, a blank, or the same level twice. The host applies the same rules
 * and would drop them anyway — this keeps the payload honest and, more to the
 * point, keeps a duplicate from becoming a second chip that cannot be turned
 * off.
 */
it("drops reserved, blank and repeated levels on the way out", async () => {
  const stored = (thinkingLevels: string[]) => ({
    ...initial,
    models: [{
      id: "m1",
      name: "M1",
      supportsImages: false,
      contextWindow: 128000,
      maxTokens: 16384,
      reasoning: true,
      thinkingLevels,
      inputCost: 0,
      outputCost: 0,
      cacheReadCost: 0,
      cacheWriteCost: 0,
    }],
  });
  const onSubmit = mount(stored(["default", " low ", "low", "", "high"]));

  // `default` is implicit and `low` already appeared, so the row shows two.
  expect(pressedLabels()).toEqual(["low", "high"]);
  await act(async () => button("Save").click());
  expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({
    models: [expect.objectContaining({ thinkingLevels: ["low", "high"] })],
  }));
});

/**
 * A level toggled on one row belongs to that row.
 *
 * The rows are one array, so every toggle maps over all of them; a mapping that
 * wrote on every index would toggle the same chip on every model at once.
 */
it("toggles a level on one model only", async () => {
  const onSubmit = mount({
    ...initial,
    models: [
      {
        id: "m1",
        name: "M1",
        supportsImages: false,
        contextWindow: 128000,
        maxTokens: 16384,
        reasoning: true,
        thinkingLevels: ["low"],
        inputCost: 0,
        outputCost: 0,
        cacheReadCost: 0,
        cacheWriteCost: 0,
      },
      {
        id: "m2",
        name: "M2",
        supportsImages: false,
        contextWindow: 128000,
        maxTokens: 16384,
        reasoning: true,
        thinkingLevels: ["medium"],
        inputCost: 0,
        outputCost: 0,
        cacheReadCost: 0,
        cacheWriteCost: 0,
      },
    ],
  });

  // The second row's `high` chip: two rows of seven, in row order.
  const all = [...document.querySelectorAll<HTMLButtonElement>("button[aria-pressed]")];
  expect(all).toHaveLength(CHIPS.length * 2);
  act(() => all[CHIPS.length + CHIPS.indexOf("high")]!.click());
  await act(async () => button("Save").click());

  const submitted = onSubmit.mock.calls[0]![0] as { models: Array<{ id: string; thinkingLevels: string[] }> };
  expect(submitted.models.map(model => [model.id, model.thinkingLevels])).toEqual([
    ["m1", ["low"]],
    ["m2", ["medium", "high"]],
  ]);
});
