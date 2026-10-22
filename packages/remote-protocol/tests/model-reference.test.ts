import { describe, test, expect } from "vitest";
import { modelProviderFromReference, modelReference, modelThinkingLevels, THINKING_LEVELS } from "../src/types";

describe("model reference helpers", () => {
  test("modelReference scopes an id with its provider", () => {
    expect(modelReference({ id: "gpt-5", provider: "openai" })).toBe("openai/gpt-5");
  });

  test("modelReference does not confuse a provider-looking raw id with a qualified reference", () => {
    expect(modelReference({ id: "openai/gpt-5", provider: "openai" })).toBe("openai/openai/gpt-5");
    expect(modelReference({ id: "openrouter/auto", provider: "openrouter" })).toBe("openrouter/openrouter/auto");
    expect(modelReference({ id: "gpt-5" })).toBe("gpt-5");
  });

  test("two providers sharing one raw id retain distinct identities", () => {
    const id = "deepseek/deepseek-v4-flash";
    expect(modelReference({ id, provider: "deepseek" })).toBe("deepseek/deepseek/deepseek-v4-flash");
    expect(modelReference({ id, provider: "ambient" })).toBe("ambient/deepseek/deepseek-v4-flash");
  });

  test("modelProviderFromReference extracts the provider segment", () => {
    expect(modelProviderFromReference("openai/gpt-5")).toBe("openai");
  });

  test("modelProviderFromReference is undefined without a separator", () => {
    expect(modelProviderFromReference("gpt-5")).toBeUndefined();
    expect(modelProviderFromReference("/gpt-5")).toBeUndefined();
  });
});

describe("modelThinkingLevels", () => {
  test("a model that declares nothing offers every level", () => {
    expect(modelThinkingLevels({ id: "m" })).toEqual(THINKING_LEVELS);
    expect(modelThinkingLevels(undefined)).toEqual(THINKING_LEVELS);
    // An older host sends no list at all — not an empty one — and that is the
    // same answer, which is what keeps pre-upgrade hosts unchanged.
    expect(modelThinkingLevels({ id: "m", thinkingLevels: null })).toEqual(THINKING_LEVELS);
  });

  test("a declared list is intersected with the canonical vocabulary in canonical order", () => {
    expect(modelThinkingLevels({ id: "m", thinkingLevels: ["high", "off", "low"] })).toEqual(["off", "low", "high"]);
    // The agent normally normalizes this already; a hand-written list must not
    // smuggle in a level no label or budget arm knows.
    expect(modelThinkingLevels({ id: "m", thinkingLevels: ["low", "mid", "high"] })).toEqual(["low", "high"]);
    expect(modelThinkingLevels({ id: "m", thinkingLevels: ["off"] })).toEqual(["off"]);
  });
});
