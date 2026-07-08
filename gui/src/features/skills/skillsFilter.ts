import type { AvailableSkill, InstalledSkill } from "../../integrations/skills/skillsClient";

/** Sentinel category value meaning "no category filter". */
export const allCategoriesValue = "__all__";

export interface SkillFilters {
  category: string;
  query: string;
}

export function matchesInstalledSkill(skill: InstalledSkill, filters: SkillFilters) {
  return matchesQuery(filters.query, [
    skill.id,
    skill.name,
    skill.description,
    skill.version,
  ]);
}

export function matchesAvailableSkill(skill: AvailableSkill, filters: SkillFilters) {
  if (!matchesCategory(skill.category, filters.category))
    return false;

  return matchesQuery(filters.query, [
    skill.id,
    skill.name,
    skill.nameZh,
    skill.description,
    skill.descriptionZh,
    skill.category,
    skill.latestVersion,
  ]);
}

function matchesCategory(category: string | undefined, selectedCategory: string) {
  return selectedCategory === allCategoriesValue || category === selectedCategory;
}

export function matchesQuery(query: string, values: Array<string | null | undefined>) {
  const normalizedQuery = normalizeSearchText(query);
  if (!normalizedQuery)
    return true;

  return values.some(value => normalizeSearchText(value).includes(normalizedQuery));
}

export function normalizeSearchText(value: string | null | undefined) {
  return (value ?? "").trim().toLowerCase();
}

export function uniqueSorted(values: Array<string | null | undefined>) {
  return Array.from(new Set(values.filter((value): value is string => Boolean(value)))).sort((a, b) => a.localeCompare(b));
}
