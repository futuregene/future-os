import type { AvailableSkill, InstalledSkill } from "../../integrations/skills/skillsClient";
import { describe, expect, it } from "vitest";
import {
  allCategoriesValue,
  matchesAvailableSkill,
  matchesInstalledSkill,
  matchesQuery,
  normalizeSearchText,
  uniqueSorted,
} from "./skillsFilter";

function installed(overrides: Partial<InstalledSkill> = {}): InstalledSkill {
  return { id: "id", name: "Name", description: "Description", nameZh: null, descriptionZh: null, version: "1.0.0", ...overrides };
}

function available(overrides: Partial<AvailableSkill> = {}): AvailableSkill {
  return {
    id: "id",
    name: "Name",
    description: "Description",
    nameZh: "中文名",
    descriptionZh: "中文描述",
    category: "core",
    latestVersion: "1.0.0",
    ...overrides,
  };
}

describe("normalizeSearchText", () => {
  it("trims, lowercases, and coalesces nullish to empty", () => {
    expect(normalizeSearchText("  Foo BAR ")).toBe("foo bar");
    expect(normalizeSearchText(null)).toBe("");
    expect(normalizeSearchText(undefined)).toBe("");
  });
});

describe("matchesQuery", () => {
  it("matches everything when the query is blank", () => {
    expect(matchesQuery("   ", ["anything"])).toBe(true);
  });

  it("matches case-insensitively across any field", () => {
    expect(matchesQuery("GENE", ["rare", "gene-variant"])).toBe(true);
    expect(matchesQuery("missing", ["rare", null, undefined])).toBe(false);
  });
});

describe("uniqueSorted", () => {
  it("dedupes, drops empties, and sorts", () => {
    expect(uniqueSorted(["b", "a", "b", "", null, "a"])).toEqual(["a", "b"]);
  });
});

describe("matchesInstalledSkill", () => {
  it("filters by query only", () => {
    const skill = installed({ name: "Literature", id: "lit" });
    expect(matchesInstalledSkill(skill, { category: allCategoriesValue, query: "lit" })).toBe(true);
    expect(matchesInstalledSkill(skill, { category: "other", query: "" })).toBe(true);
    expect(matchesInstalledSkill(skill, { category: allCategoriesValue, query: "missing" })).toBe(false);
  });
});

describe("matchesAvailableSkill", () => {
  it("filters by the skill's own category and query", () => {
    const skill = available({ category: "core", name: "Core" });
    expect(matchesAvailableSkill(skill, { category: "core", query: "core" })).toBe(true);
    expect(matchesAvailableSkill(skill, { category: "rare", query: "" })).toBe(false);
    expect(matchesAvailableSkill(skill, { category: allCategoriesValue, query: "zzz" })).toBe(false);
  });

  it("matches localized catalogue fields", () => {
    const skill = available({ nameZh: "深度研究", descriptionZh: "多源信息综合" });
    expect(matchesAvailableSkill(skill, { category: allCategoriesValue, query: "深度" })).toBe(true);
    expect(matchesAvailableSkill(skill, { category: allCategoriesValue, query: "综合" })).toBe(true);
  });
});
