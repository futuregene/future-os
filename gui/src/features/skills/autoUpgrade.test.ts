import type { AvailableSkill, InstalledSkill } from "../../integrations/skills/skillsClient";
import { describe, expect, it } from "vitest";
import { computeSkillUpgrades } from "./autoUpgrade";

function installed(overrides: Partial<InstalledSkill> = {}): InstalledSkill {
  return { id: "id", name: "Name", description: "", nameZh: null, descriptionZh: null, version: "1.0.0", ...overrides };
}

function available(overrides: Partial<AvailableSkill> = {}): AvailableSkill {
  return {
    id: "id",
    name: "Name",
    description: "",
    nameZh: "",
    descriptionZh: "",
    category: "core",
    latestVersion: "1.0.0",
    ...overrides,
  };
}

describe("computeSkillUpgrades", () => {
  it("returns skills whose catalogue version is newer", () => {
    const result = computeSkillUpgrades(
      [installed({ id: "a", version: "1.0.0" }), installed({ id: "b", version: "2.1.0" })],
      [available({ id: "a", latestVersion: "1.2.0" }), available({ id: "b", latestVersion: "2.1.0" })],
    );
    expect(result).toEqual([{ id: "a", version: "1.2.0" }]);
  });

  it("ignores skills at or above the catalogue version", () => {
    const result = computeSkillUpgrades(
      [installed({ id: "a", version: "2.0.0" })],
      [available({ id: "a", latestVersion: "1.9.0" })],
    );
    expect(result).toEqual([]);
  });

  it("ignores installed skills missing from the catalogue", () => {
    const result = computeSkillUpgrades(
      [installed({ id: "orphan", version: "1.0.0" })],
      [available({ id: "other", latestVersion: "9.9.9" })],
    );
    expect(result).toEqual([]);
  });

  it("skips skills with a missing installed or catalogue version", () => {
    const result = computeSkillUpgrades(
      [installed({ id: "a", version: null }), installed({ id: "b", version: "1.0.0" })],
      [available({ id: "a", latestVersion: "2.0.0" }), available({ id: "b", latestVersion: null })],
    );
    expect(result).toEqual([]);
  });
});
