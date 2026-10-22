// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it } from "vitest";
import { defaultThinkingLevel, modelThinkingLevel, modelThinkingLevels, normalizeThinkingLevel, rememberLastUsedThinkingLevel, resolveInitialThinkingLevel, resolveThinkingLevel } from "./agentClient";

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

it("offers only the levels a model declares", () => {
  const models = [{ id: "narrow", provider: "custom", label: "Narrow", thinkingLevels: ["low", "high"] }];
  expect(modelThinkingLevels("custom/narrow", models)).toEqual(["low", "high"]);
});

it("offers all six when the model declares nothing or the agent is older", () => {
  // The unchanged path: no list on the row (an agent before this field) or an
  // explicit empty one both mean "this model constrains nothing".
  for (const row of [
    { id: "plain", provider: "future", label: "Plain" },
    { id: "plain", provider: "future", label: "Plain", thinkingLevels: [] },
  ]) {
    expect(modelThinkingLevels("future/plain", [row])).toEqual([
      "off",
      "minimal",
      "low",
      "medium",
      "high",
      "xhigh",
    ]);
  }
});

it("starts a narrow model on the starting level its agent computed", () => {
  // The agent sends its own clamp of the app default (`medium`). Composing that
  // here would be a second copy of the snapping rule; the field is the rule.
  const models = [{
    id: "narrow",
    provider: "custom",
    label: "Narrow",
    thinkingLevels: ["low", "high"],
    defaultThinkingLevel: "low",
  }];
  expect(modelThinkingLevel("custom/narrow", models)).toBe("low");
  expect(resolveInitialThinkingLevel("custom/narrow", models)).toBe("low");
});

it("does not repeat a saved level the chosen model no longer offers", () => {
  rememberLastUsedThinkingLevel("xhigh");
  const models = [{
    id: "narrow",
    provider: "custom",
    label: "Narrow",
    thinkingLevels: ["off", "low", "high"],
    defaultThinkingLevel: "low",
  }];
  expect(resolveInitialThinkingLevel("custom/narrow", models)).toBe("low");
});

it("shows a stored level only while the model still offers it", () => {
  const models = [{
    id: "narrow",
    provider: "custom",
    label: "Narrow",
    thinkingLevels: ["off", "low", "high"],
    defaultThinkingLevel: "low",
  }];
  expect(resolveThinkingLevel("high", "custom/narrow", models)).toBe("high");
  // A value from another model (or from before this one narrowed its list) is
  // replaced by the model's own starting level.
  expect(resolveThinkingLevel("xhigh", "custom/narrow", models)).toBe("low");
  expect(resolveThinkingLevel(null, "custom/narrow", models)).toBe("low");
});
