import { modelProviderFromReference, modelReference } from "../types";

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
