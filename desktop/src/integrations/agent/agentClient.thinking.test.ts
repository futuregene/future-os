// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it } from "vitest";
import { defaultThinkingLevel, legacyThinkingLevels, modelThinkingLevel, modelThinkingLevels, normalizeThinkingLevel, rememberLastUsedThinkingLevel, resolveInitialThinkingLevel, resolveThinkingLevel } from "./agentClient";

beforeEach(() => localStorage.clear());
afterEach(() => localStorage.clear());

/** The shape a current agent sends: the effective list, `default` first. */
function currentModel(overrides: Record<string, unknown> = {}) {
  return {
    id: "m",
    provider: "future",
    label: "M",
    thinkingLevels: ["default", "off", "low", "high", "max"],
    defaultThinkingLevel: "default",
    ...overrides,
  };
}

it("defaults to the implicit default level", () => {
  expect(defaultThinkingLevel).toBe("default");
  expect(normalizeThinkingLevel()).toBe("default");
  expect(normalizeThinkingLevel("")).toBe("default");
  expect(normalizeThinkingLevel(null)).toBe("default");
});

it("keeps any non-empty value as a level, since the model decides the vocabulary", () => {
  expect(normalizeThinkingLevel("max")).toBe("max");
  // An arbitrary wire token is a legal level now; only the model's list says
  // whether it will actually run at it.
  expect(normalizeThinkingLevel("ultra")).toBe("ultra");
});

it("falls back to the legacy six and medium when the agent omits the field", () => {
  // An older agent sends a single `thinkingLevel` and no list at all.
  const models = [{ id: "m", provider: "future", label: "M", thinkingLevel: "high" }];
  expect(modelThinkingLevels("future/m", models)).toEqual(legacyThinkingLevels);
  expect(modelThinkingLevel("future/m", models)).toBe("medium");
  expect(resolveInitialThinkingLevel("future/m", models)).toBe("medium");
});

it("treats an empty list like a missing one, keeping the legacy six", () => {
  const models = [{ id: "m", provider: "future", label: "M", thinkingLevels: [] }];
  expect(modelThinkingLevels("future/m", models)).toEqual(legacyThinkingLevels);
  expect(modelThinkingLevel("future/m", models)).toBe("medium");
});

it("shows exactly the list the agent sends, in its order", () => {
  const models = [currentModel()];
  expect(modelThinkingLevels("future/m", models)).toEqual(["default", "off", "low", "high", "max"]);
  expect(modelThinkingLevel("future/m", models)).toBe("default");
});

it("starts at the first level the agent declared when it names no default", () => {
  // A host that sends the list but not `defaultThinkingLevel` (an older current
  // build). The list is authoritative and ordered, so its first entry is the
  // starting point; falling through to `medium` would pick a level the model does
  // not offer.
  const models = [currentModel({ defaultThinkingLevel: undefined, thinkingLevels: ["low", "high"] })];
  expect(modelThinkingLevel("future/m", models)).toBe("low");
  // The same when the declared default is not one of the offered levels.
  const narrowed = [currentModel({ defaultThinkingLevel: "max", thinkingLevels: ["low", "high"] })];
  expect(modelThinkingLevel("future/m", narrowed)).toBe("low");
});

it("does not filter arbitrary values out of the agent's list", () => {
  const models = [currentModel({ thinkingLevels: ["default", "ultra", "max"], defaultThinkingLevel: "default" })];
  expect(modelThinkingLevels("future/m", models)).toEqual(["default", "ultra", "max"]);
});

it("honours a saved level while the model still offers it", () => {
  const models = [currentModel()];
  rememberLastUsedThinkingLevel("max");
  expect(resolveInitialThinkingLevel("future/m", models)).toBe("max");
  rememberLastUsedThinkingLevel("xhigh");
  expect(resolveInitialThinkingLevel("future/m", models)).toBe("default");
});

it("never repeats a saved level the chosen model no longer offers", () => {
  rememberLastUsedThinkingLevel("xhigh");
  const models = [currentModel({
    thinkingLevels: ["default", "off", "low", "high"],
    defaultThinkingLevel: "default",
  })];
  expect(resolveInitialThinkingLevel("future/m", models)).toBe("default");
});

it("starts on the level the agent clamped for the model", () => {
  const models = [currentModel({
    thinkingLevels: ["default", "low", "high"],
    defaultThinkingLevel: "low",
  })];
  expect(modelThinkingLevel("future/m", models)).toBe("low");
  expect(resolveInitialThinkingLevel("future/m", models)).toBe("low");
  expect(resolveThinkingLevel("high", "future/m", models)).toBe("high");
  // A value from another model (or from before this one narrowed its list) is
  // replaced by the model's own starting level.
  expect(resolveThinkingLevel("xhigh", "future/m", models)).toBe("low");
  expect(resolveThinkingLevel(null, "future/m", models)).toBe("low");
});

it("offers no choice when the model's list holds only the implicit default", () => {
  const models = [currentModel({ thinkingLevels: ["default"], defaultThinkingLevel: "default" })];
  expect(modelThinkingLevels("future/m", models)).toEqual(["default"]);
  expect(modelThinkingLevel("future/m", models)).toBe("default");
  // A stored level from another model is shown as `default`, not the stored one.
  expect(resolveThinkingLevel("high", "future/m", models)).toBe("default");
});

it("respects an older agent's own narrowed list without filtering it to six", () => {
  // The field predates the new contract: an old agent may publish a subset that
  // has no `default` entry. It is still authoritative for its model.
  const models = [{
    id: "m",
    provider: "custom",
    label: "M",
    thinkingLevels: ["low", "high"],
    defaultThinkingLevel: "low",
  }];
  expect(modelThinkingLevels("custom/m", models)).toEqual(["low", "high"]);
  expect(modelThinkingLevel("custom/m", models)).toBe("low");
  // The literal `default` is never invented onto a list that does not hold it.
  rememberLastUsedThinkingLevel("default");
  expect(resolveInitialThinkingLevel("custom/m", models)).toBe("low");
});

it("uses medium before the model catalogue loads", () => {
  expect(resolveInitialThinkingLevel("", [])).toBe("medium");
});
