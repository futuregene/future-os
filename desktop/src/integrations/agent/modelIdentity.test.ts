// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { modelKey, modelOption, resolveInitialModelId } from "./agentClient";

const models = [
  { id: "deepseek/deepseek-v4-flash", provider: "deepseek", label: "Direct" },
  { id: "deepseek/deepseek-v4-flash", provider: "ambient", label: "Gateway" },
];

afterEach(() => vi.unstubAllGlobals());

describe("provider-qualified model identities", () => {
  it("preserves provider-looking raw ids and resolves the selected provider exactly", () => {
    for (const model of models) {
      const key = modelKey(model);
      expect(key).toBe(`${model.provider}/deepseek/deepseek-v4-flash`);
      expect(modelOption(key, [...models].reverse())).toBe(model);
    }
  });

  it("does not select an arbitrary provider for an ambiguous legacy bare id", () => {
    const bare = models.map(model => ({ ...model, id: "shared" }));
    expect(modelOption("shared", bare)).toBeUndefined();
    expect(modelOption("shared", [...bare].reverse())).toBeUndefined();
    expect(modelOption("deepseek/deepseek-v4-flash", models)).toBeUndefined();
    expect(modelOption("deepseek/deepseek-v4-flash", [...models].reverse())).toBeUndefined();
    expect(modelOption("missing/deepseek-v4-flash", [
      { id: "deepseek-v4-flash", provider: "other", label: "Other" },
    ])).toBeUndefined();
  });

  it("upgrades a unique legacy slash-containing selection before sending it", () => {
    vi.stubGlobal("localStorage", { getItem: () => "deepseek/deepseek-v4-flash" });
    expect(resolveInitialModelId([models[1]!])).toBe("ambient/deepseek/deepseek-v4-flash");
  });
});
