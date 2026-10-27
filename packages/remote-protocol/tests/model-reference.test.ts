import { describe, test, expect } from "vitest";
import {
  LEGACY_DEFAULT_THINKING_LEVEL,
  THINKING_LEVELS,
  defaultThinkingLevelForModel,
  modelProviderFromReference,
  modelReference,
  modelThinkingLevels,
  resolveModelThinkingLevel,
  type RemoteModel,
} from "../src/types";

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
  test("an older host that declares nothing keeps the legacy six", () => {
    expect(modelThinkingLevels(catalogueModel())).toEqual(THINKING_LEVELS);
    expect(modelThinkingLevels(undefined)).toEqual(THINKING_LEVELS);
    // An older host sends no list at all — not an empty one — and that is the
    // same answer, which is what keeps pre-upgrade hosts unchanged.
    expect(modelThinkingLevels(catalogueModel(null))).toEqual(THINKING_LEVELS);
    expect(modelThinkingLevels(catalogueModel([]))).toEqual(THINKING_LEVELS);
  });

  test("the host's own list is shown exactly as declared, in its order", () => {
    // The agent resolved this list; a client-side vocabulary would drop `max`
    // (and any provider token) from a picker that must offer what the host runs.
    const declared = ["default", "off", "low", "high", "max"];
    expect(modelThinkingLevels(catalogueModel(declared))).toEqual(declared);
    expect(modelThinkingLevels(catalogueModel(["default"]))).toEqual(["default"]);
    expect(modelThinkingLevels(catalogueModel(["default", "turbo"]))).toEqual(["default", "turbo"]);
  });
});

describe("defaultThinkingLevelForModel", () => {
  test("a current host starts at its declared default", () => {
    expect(defaultThinkingLevelForModel(catalogueModel(["default", "off", "high"]))).toBe("default");
  });

  test("a declared starting level wins when the host states one", () => {
    const model: RemoteModel = {
      id: "m",
      thinkingLevels: ["default", "off", "high"],
      defaultThinkingLevel: "high",
    };
    expect(defaultThinkingLevelForModel(model)).toBe("high");
  });

  test("an older host starts at the legacy level, never at the literal default", () => {
    // Sending "default" to a host that predates `thinkingLevels` would be a
    // value it never offered: its conversations start at "medium".
    expect(defaultThinkingLevelForModel(catalogueModel())).toBe(LEGACY_DEFAULT_THINKING_LEVEL);
    expect(defaultThinkingLevelForModel(undefined)).toBe(LEGACY_DEFAULT_THINKING_LEVEL);
    expect(LEGACY_DEFAULT_THINKING_LEVEL).toBe("medium");
    // The legacy vocabulary has no "default": that is exactly why it must never
    // be sent to such a host.
    expect((THINKING_LEVELS as readonly string[]).includes("default")).toBe(false);
  });

  test("falls back to the first declared level when no starting level is known", () => {
    expect(defaultThinkingLevelForModel(catalogueModel(["low", "high"]))).toBe("low");
  });
});

describe("resolveModelThinkingLevel", () => {
  const declared: RemoteModel = {
    id: "m",
    thinkingLevels: ["default", "off", "low", "high"],
    defaultThinkingLevel: "default",
  };

  test("keeps a level the model offers", () => {
    expect(resolveModelThinkingLevel("high", declared)).toBe("high");
    expect(resolveModelThinkingLevel("default", declared)).toBe("default");
  });

  test("shows the model's starting level for one it does not offer", () => {
    // `medium`/`xhigh` were stored by another model, or before this one narrowed
    // its list; the host clamps them, so displaying them as current is wrong.
    expect(resolveModelThinkingLevel("medium", declared)).toBe("default");
    expect(resolveModelThinkingLevel("xhigh", declared)).toBe("default");
    expect(resolveModelThinkingLevel(null, declared)).toBe("default");
  });

  test("a level outside the client's vocabulary is not filtered out", () => {
    const max: RemoteModel = { id: "m", thinkingLevels: ["default", "off", "max"], defaultThinkingLevel: "default" };
    expect(resolveModelThinkingLevel("max", max)).toBe("max");
  });

  test("an older host keeps the level it was given and falls back to medium", () => {
    // Display-side, a host without a list is the unchanged path.
    expect(resolveModelThinkingLevel("xhigh", catalogueModel())).toBe("xhigh");
    expect(resolveModelThinkingLevel("default", catalogueModel())).toBe("medium");
  });

  test("an unknown model has no list to resolve against", () => {
    expect(resolveModelThinkingLevel("high", undefined)).toBeUndefined();
  });
});
