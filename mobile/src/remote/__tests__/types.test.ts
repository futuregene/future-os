import { modelProviderFromReference, modelReference } from "../types";

describe("model reference helpers", () => {
  test("modelReference scopes an id with its provider", () => {
    expect(modelReference({ id: "gpt-5", provider: "openai" })).toBe("openai/gpt-5");
  });

  test("modelReference leaves an already-scoped id and provider-less ids alone", () => {
    expect(modelReference({ id: "openai/gpt-5", provider: "openai" })).toBe("openai/gpt-5");
    expect(modelReference({ id: "gpt-5" })).toBe("gpt-5");
  });

  test("modelProviderFromReference extracts the provider segment", () => {
    expect(modelProviderFromReference("openai/gpt-5")).toBe("openai");
  });

  test("modelProviderFromReference is undefined without a separator", () => {
    expect(modelProviderFromReference("gpt-5")).toBeUndefined();
    expect(modelProviderFromReference("/gpt-5")).toBeUndefined();
  });
});
