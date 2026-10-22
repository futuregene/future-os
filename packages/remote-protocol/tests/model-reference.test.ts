import { describe, test, expect } from "vitest";
import { modelProviderFromReference, modelReference, modelThinkingLevels, resolveModelThinkingLevel, THINKING_LEVELS, type RemoteModel } from "../src/types";

/**
 * A catalogue row as the mobile pickers receive it. `modelThinkingLevels` reads
 * only `thinkingLevels`, so its parameter is that subset — but every real call
 * site hands it a whole model, and typing the fixture as one keeps these cases
 * written against the shape the app actually passes.
 */
const catalogueModel = (thinkingLevels?: string[] | null): RemoteModel => ({ id: "m", thinkingLevels });

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
    expect(modelThinkingLevels(catalogueModel())).toEqual(THINKING_LEVELS);
    expect(modelThinkingLevels(undefined)).toEqual(THINKING_LEVELS);
    // An older host sends no list at all — not an empty one — and that is the
    // same answer, which is what keeps pre-upgrade hosts unchanged.
    expect(modelThinkingLevels(catalogueModel(null))).toEqual(THINKING_LEVELS);
  });

  test("a declared list is intersected with the canonical vocabulary in canonical order", () => {
    expect(modelThinkingLevels(catalogueModel(["high", "off", "low"]))).toEqual(["off", "low", "high"]);
    // The agent normally normalizes this already; a hand-written list must not
    // smuggle in a level no label or budget arm knows.
    expect(modelThinkingLevels(catalogueModel(["low", "mid", "high"]))).toEqual(["low", "high"]);
    expect(modelThinkingLevels(catalogueModel(["off"]))).toEqual(["off"]);
  });
});

describe("resolveModelThinkingLevel", () => {
  const narrow: RemoteModel = { id: "m", thinkingLevels: ["off", "low", "high"], defaultThinkingLevel: "low" };

  test("keeps a level the model offers", () => {
    expect(resolveModelThinkingLevel("high", narrow)).toBe("high");
  });

  test("shows the model's starting level for one it does not offer", () => {
    // `medium`/`xhigh` were stored by another model, or before this one narrowed
    // its list; the agent clamps them, so displaying them as current is wrong.
    expect(resolveModelThinkingLevel("medium", narrow)).toBe("low");
    expect(resolveModelThinkingLevel("xhigh", narrow)).toBe("low");
    expect(resolveModelThinkingLevel(null, narrow)).toBe("low");
  });

  test("leaves a model that declares nothing alone", () => {
    expect(resolveModelThinkingLevel("xhigh", catalogueModel())).toBe("xhigh");
  });

  test("falls back to the shallowest offered level when no starting level is known", () => {
    expect(resolveModelThinkingLevel("xhigh", catalogueModel(["low", "high"]))).toBe("low");
  });
});
