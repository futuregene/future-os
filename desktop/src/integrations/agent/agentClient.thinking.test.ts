// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it } from "vitest";
import { defaultThinkingLevel, modelThinkingLevel, normalizeThinkingLevel, rememberLastUsedThinkingLevel, resolveInitialThinkingLevel } from "./agentClient";

beforeEach(() => localStorage.clear());
afterEach(() => localStorage.clear());

it.each(["deepseek-v4-pro", "another-model"])("defaults %s to medium despite the agent catalog's high default", (id) => {
  const models = [{ id, provider: "future", label: id, thinkingLevel: "high" }];
  expect(modelThinkingLevel(`future/${id}`, models)).toBe("medium");
  expect(resolveInitialThinkingLevel(`future/${id}`, models)).toBe("medium");
});

it("uses medium before the model catalog loads and for invalid levels", () => {
  expect(defaultThinkingLevel).toBe("medium");
  expect(resolveInitialThinkingLevel("", [])).toBe("medium");
  expect(normalizeThinkingLevel()).toBe("medium");
  expect(normalizeThinkingLevel("invalid")).toBe("medium");
});

it.each(["off", "high", "xhigh"])("preserves an explicit %s choice", (level) => {
  rememberLastUsedThinkingLevel(level);
  expect(resolveInitialThinkingLevel("future/model", [])).toBe(level);
  expect(normalizeThinkingLevel(level)).toBe(level);
});

it("keeps unsupported models off even with a saved choice", () => {
  rememberLastUsedThinkingLevel("high");
  const models = [{ id: "model", provider: "custom", label: "Model", reasoning: false }];
  expect(modelThinkingLevel("custom/model", models)).toBe("off");
  expect(resolveInitialThinkingLevel("custom/model", models)).toBe("off");
});
