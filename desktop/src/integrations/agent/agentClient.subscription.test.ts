// @vitest-environment jsdom
import type { AgentModelOption } from "./agentClient";
import { expect, it } from "vitest";
import { callableModelOptions, isModelLockedBySubscription, modelRequiresSubscription } from "./agentClient";

function model(overrides: Partial<AgentModelOption> = {}): AgentModelOption {
  return { id: "future-free", label: "Free", provider: "future", ...overrides };
}

it("reads the platform's subscriber-only flag off the catalogue entry", () => {
  expect(modelRequiresSubscription(model({ requiresSubscription: true }))).toBe(true);
  // Absent is the normal case: every provider other than Future leaves it unset.
  expect(modelRequiresSubscription(model())).toBe(false);
  expect(modelRequiresSubscription(model({ requiresSubscription: false }))).toBe(false);
});

it("locks a subscriber-only model only when the account is known to have no plan", () => {
  const gated = model({ requiresSubscription: true });
  expect(isModelLockedBySubscription(gated, false)).toBe(true);
  // A live plan, an unknown plan (signed out / community edition / a failed
  // read) and a model the platform does not gate all stay selectable.
  expect(isModelLockedBySubscription(gated, true)).toBe(false);
  expect(isModelLockedBySubscription(gated, null)).toBe(false);
  expect(isModelLockedBySubscription(model({ requiresSubscription: false }), false)).toBe(false);
});

it("never locks a model a plan does not pay for", () => {
  // `deepseek-flash` is inside the plan's model whitelist but is also callable
  // from the balance, so it carries no subscriber-only flag and stays offered.
  const flash = model({ id: "deepseek-flash", label: "Flash" });
  expect(isModelLockedBySubscription(flash, false)).toBe(false);
});

it("narrows the callable list only when the account is known to have no plan", () => {
  const models = [
    model({ id: "deepseek-flash", label: "Flash" }),
    model({ id: "future-free", label: "Free", requiresSubscription: true }),
    model({ id: "gpt-5", label: "GPT", provider: "openai" }),
  ];
  // Catalogue order survives the filter: the picker is the same ring, minus
  // the rows the account cannot pick.
  expect(callableModelOptions(models, false).map(entry => entry.id)).toEqual(["deepseek-flash", "gpt-5"]);
  expect(callableModelOptions(models, true)).toEqual(models);
  // Unknown is not "no plan": the same list comes back untouched.
  expect(callableModelOptions(models, null)).toEqual(models);
});
